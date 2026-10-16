//! The seat resolution stage (spec D1, D2, D4-D8): the one place a raw seat argument becomes a
//! canonical seat ID, before any journal intent or dispatch.
//!
//! Owns: the D2 order per slot (exact seat ID, nickname `PREFIX/SEAT_ID`, direct pane ID, name
//! path), the lazy once-per-invocation Herdr topology and live-caller reads, `--space`/`--tab`
//! narrowing, conflict seat annotation, the `SeatMapping` dispatch (lookup or the shared journaled
//! allocation), freezing through `SeatSlot`, the post-freeze ID checks and the `resolved:` stderr
//! lines.
//!
//! Two passes. Pass 1 (`resolve_cli_seats`) resolves every slot to an exact seat or a pane and
//! writes no journal intent: lookups and exact IDs freeze there, an `Allocate` slot that resolved
//! to a pane waits in the returned `DeferredSeats`. Pass 2 (`allocate_deferred_seats`) runs the
//! journaled `ResolveSeat` for those panes, and the caller runs it only after every slot has
//! resolved and after the refusals that can follow (the caller derivation, the `--operator`
//! actor refusal), behind one gate point (`run_gates`), so a refused command leaves no allocation.
//!
//! Must not: match names (`seat_selector` does), know command kinds (`SeatSlot` does), write any
//! journal intent other than the shared allocation, or run on `retry` (a retry carries no slots).
//! A new seat-argument form goes into the classification in `resolve_one`.

use super::{
    LazyConnection, RunError,
    commands::{
        CliAction, MutationSpec, ParsedCli, SEAT_SELECTOR_PLACEHOLDER, SeatMapping, SeatNarrowing,
        SeatSelectorArg, SeatSlot, check_direct_ack, check_unique_recipients,
    },
    panes::{looks_like_pane_id, resolve_parents},
    seat_selector::{self, Narrowing},
};
use crate::{
    daemon::paths::InstancePaths,
    host::observation::HostTopology,
    ports::LocalClient,
    protocol::{
        capabilities::INVITATION_CONFIRMATION_V1,
        commands::{Command, SeatInspectQuery},
        ids::{HostTargetId, SeatId},
        pagination::PageRequest,
        results::{ApiError, CommandResult, ContinuityStatus, ErrorCode, SeatInspection},
        time::Clock,
    },
};
use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    io::Write,
    sync::Arc,
};

#[cfg(test)]
thread_local! {
    static RESOLVED_LINES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static SKIP_FIRST_FREEZE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test hook: the `resolved:` lines this thread's stage wrote since the last call.
#[cfg(test)]
pub(crate) fn take_resolved_lines_for_test() -> Vec<String> {
    RESOLVED_LINES.with(|lines| std::mem::take(&mut *lines.borrow_mut()))
}

/// Test hook: the stage skips freezing its first slot while the guard lives, so the end-of-stage
/// check fires through the production entry point.
#[cfg(test)]
pub(crate) fn skip_first_freeze_for_test() -> impl Drop {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            SKIP_FIRST_FREEZE.with(|flag| flag.set(false));
        }
    }
    SKIP_FIRST_FREEZE.with(|flag| flag.set(true));
    Reset
}

/// A host read run at most once, its outcome (success or failure) shared by every slot.
struct Once<T, F> {
    read: RefCell<Option<F>>,
    value: OnceCell<Result<T, ApiError>>,
}

impl<T, F: FnOnce() -> Result<T, ApiError>> Once<T, F> {
    fn new(read: F) -> Self {
        Self {
            read: RefCell::new(Some(read)),
            value: OnceCell::new(),
        }
    }

    fn get(&self) -> Result<&T, ApiError> {
        self.value
            .get_or_init(|| match self.read.borrow_mut().take() {
                Some(read) => read(),
                None => Err(ApiError::invalid_request("host read repeated")),
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Wrap a topology or caller read failure for a nickname, name or narrowed slot: an error that is
/// already `host_unavailable` stays; any other becomes one (names are never passed through).
fn host_unavailable(error: ApiError) -> ApiError {
    if error.code == ErrorCode::HostUnavailable {
        error
    } else {
        ApiError::new(
            ErrorCode::HostUnavailable,
            format!("seat names need the live Herdr topology: {}", error.detail),
        )
    }
}

/// The inspected seat's live mapped target, `None` when retired, mid-retirement, unresolved or
/// unmapped (so its location cannot be verified).
fn live_target(inspection: &SeatInspection) -> Option<&HostTargetId> {
    if inspection.summary.retired_at.is_some()
        || inspection.retirement.is_some()
        || inspection.mapping.state != ContinuityStatus::Resolved
    {
        return None;
    }
    inspection.mapping.target.as_ref()
}

fn run_error(error: ApiError) -> RunError {
    RunError::Api(error)
}

/// The stage's reads and memo for one invocation.
struct Stage<'a, C, F, W, T, L>
where
    T: FnOnce() -> Result<HostTopology, ApiError>,
    L: FnOnce() -> Result<Option<HostTargetId>, ApiError>,
{
    connection: &'a LazyConnection<C, F>,
    clock: &'a Arc<dyn Clock>,
    stderr: &'a mut W,
    topology: Once<HostTopology, T>,
    caller: Once<Option<HostTargetId>, L>,
    raw_narrowing: Option<SeatNarrowing>,
    narrowing: OnceCell<Narrowing>,
}

/// One slot's pass-1 outcome: a frozen seat, or a pane whose seat is allocated in pass 2.
#[derive(Clone)]
enum Resolved {
    Seat(SeatId),
    Pane {
        pane: HostTargetId,
        /// `Some((path, tier))` for a name (D2 step 4): pass 2 prints its `resolved:` line after
        /// allocating; `None` for a direct pane ID (D6: no line).
        line: Option<(String, String)>,
    },
}

/// Allocate slots whose pane is known but whose journaled `ResolveSeat` waits for pass 2.
#[must_use = "pass 2 (allocate_deferred_seats) must run before dispatch"]
pub(crate) struct DeferredSeats {
    entries: Vec<Deferred>,
    /// Every original slot, for pass 2's end-of-stage placeholder check.
    slots: Vec<SeatSelectorArg>,
}

struct Deferred {
    slot: SeatSlot,
    pane: HostTargetId,
    /// Set on the first slot of a distinct raw name only, so a repeated name prints once.
    line: Option<(String, String)>,
}

impl DeferredSeats {
    /// No deferred allocation (a command with no allocate slot).
    pub(crate) fn none() -> Self {
        Self {
            entries: Vec::new(),
            slots: Vec::new(),
        }
    }
}

/// Pass 1: resolve every seat argument of `parsed` to a canonical seat or a pane (spec D2), or
/// fail the command. No journal intent is written here.
///
/// Per slot, in order: (1) an exact seat ID known to the daemon (retired included) is used as is,
/// reading nothing from Herdr unless `--space`/`--tab` narrow it; (2) `PREFIX/SEAT_ID` whose last
/// part is a known seat is verified against the seat's live location; (3) a pane ID with no
/// narrowing is the pane itself; (4) anything else is a `SPACE/TAB/PANE` name matched by
/// `seat_selector` against the live topology. Steps 3 and 4 map the pane to a seat by the slot's
/// `SeatMapping`: a lookup never allocates and freezes here; an allocation is deferred. The
/// returned `DeferredSeats` holds each such pane, to be allocated by `allocate_deferred_seats`
/// (pass 2) once the caller derivation and the command's capability refusals have run. Before
/// returning, a direct send's ACK recipients are compared with its target by seat or pane ID, so a
/// mismatch is refused before anything is allocated.
///
/// The topology and the live caller are read lazily, once. A failed read for a nickname, name or
/// narrowed slot is `host_unavailable`. A caller read that fails for any other reason is a stale
/// `HERDR_PANE_ID` (root D3): no caller, every candidate in tier `any`. Exact IDs without
/// narrowing never read Herdr. Each distinct lookup name argument writes one `resolved:` line to
/// `stderr` before any command output (allocate names write theirs in pass 2). The frozen slots
/// are checked for the placeholder: the production guarantee that no raw seat string reaches a
/// journal or dispatch.
pub(crate) fn resolve_cli_seats<C, F, W>(
    parsed: &mut ParsedCli,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    topology: impl FnOnce() -> Result<HostTopology, ApiError>,
    live_caller: impl FnOnce() -> Result<Option<HostTargetId>, ApiError>,
    stderr: &mut W,
) -> Result<DeferredSeats, RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    if parsed.seat_selectors.is_empty() {
        parsed.seat_narrowing = None;
        return Ok(DeferredSeats::none());
    }
    let slots = std::mem::take(&mut parsed.seat_selectors);
    let mut stage = Stage {
        connection,
        clock,
        stderr,
        topology: Once::new(topology),
        caller: Once::new(live_caller),
        raw_narrowing: parsed.seat_narrowing.take(),
        narrowing: OnceCell::new(),
    };
    let mut memo: HashMap<(String, SeatMapping), Resolved> = HashMap::new();
    let mut resolutions: Vec<(SeatSlot, Resolved)> = Vec::new();
    let mut entries: Vec<Deferred> = Vec::new();
    let mut deferred_slot = vec![false; slots.len()];
    for (index, arg) in slots.iter().enumerate() {
        let key = (arg.raw.clone(), arg.slot.mapping());
        let (resolved, first) = match memo.get(&key) {
            Some(resolved) => (resolved.clone(), false),
            None => {
                let resolved = stage.resolve_one(parsed, arg.slot, &arg.raw)?;
                memo.insert(key, resolved.clone());
                (resolved, true)
            }
        };
        resolutions.push((arg.slot, resolved.clone()));
        #[cfg(test)]
        if SKIP_FIRST_FREEZE.with(|flag| flag.replace(false)) {
            continue;
        }
        match resolved {
            Resolved::Seat(seat) => arg.slot.set(&mut parsed.action, seat).map_err(run_error)?,
            Resolved::Pane { pane, line } => {
                deferred_slot[index] = true;
                entries.push(Deferred {
                    slot: arg.slot,
                    pane,
                    line: line.filter(|_| first),
                });
            }
        }
    }
    check_unfrozen(&slots, &deferred_slot, &parsed.action)?;
    if entries.is_empty() {
        return post_freeze_checks(&mut parsed.action)
            .map(|()| DeferredSeats::none())
            .map_err(run_error);
    }
    check_direct_ack_before_allocation(parsed, &resolutions, connection, clock)?;
    Ok(DeferredSeats { entries, slots })
}

/// The placeholder check over the slots pass 1 froze (every slot not deferred).
fn check_unfrozen(
    slots: &[SeatSelectorArg],
    deferred_slot: &[bool],
    action: &CliAction,
) -> Result<(), RunError> {
    for (arg, deferred) in slots.iter().zip(deferred_slot) {
        if *deferred {
            continue;
        }
        match arg.slot.current(action) {
            Some(value) if value != SEAT_SELECTOR_PLACEHOLDER => {}
            _ => {
                return Err(run_error(ApiError::invalid_request(
                    "seat selector was not resolved before dispatch",
                )));
            }
        }
    }
    Ok(())
}

/// A direct ACK may name only the target (spec D7). Before pass 2 allocates, compare each ACK
/// recipient with the target by seat ID or pane ID; a mixed pair looks up the pane's existing
/// seat (read-only: an unseated pane would get a new seat, so it differs from any ID).
fn check_direct_ack_before_allocation<C, F>(
    parsed: &ParsedCli,
    resolutions: &[(SeatSlot, Resolved)],
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
{
    if !matches!(
        &parsed.action,
        CliAction::Mutation(MutationSpec::DirectSend(_))
    ) {
        return Ok(());
    }
    let Some((_, target)) = resolutions
        .iter()
        .find(|(slot, _)| *slot == SeatSlot::DirectSendTarget)
    else {
        return Ok(());
    };
    for (slot, recipient) in resolutions {
        if !matches!(slot, SeatSlot::RequireAck(_)) {
            continue;
        }
        let same = match (target, recipient) {
            (Resolved::Seat(a), Resolved::Seat(b)) => a == b,
            (Resolved::Pane { pane: a, .. }, Resolved::Pane { pane: b, .. }) => a == b,
            (Resolved::Seat(seat), Resolved::Pane { pane, .. })
            | (Resolved::Pane { pane, .. }, Resolved::Seat(seat)) => {
                let (_, client) = connection.get()?;
                super::pane_seat(client, &parsed.output, pane, clock.as_ref())?
                    .is_some_and(|existing| existing == *seat)
            }
        };
        if !same {
            // `check_direct_ack` owns the refusal text; two distinct IDs trip it.
            let refusal = check_direct_ack(&SeatId::new("target"), &mut vec![SeatId::new("other")])
                .expect_err("distinct seats are refused");
            return Err(run_error(refusal));
        }
    }
    Ok(())
}

/// Pass 2: run the journaled `ResolveSeat` for each deferred Allocate slot, freeze the result and
/// write the name slots' `resolved:` lines. The caller runs this only after every slot resolved
/// and after the caller derivation (or, for `--operator`, the non-Human refusal); it is the one
/// place `run_gates` runs, so a refused command journals no `ResolveSeat`.
pub(crate) fn allocate_deferred_seats<C, F, W>(
    deferred: DeferredSeats,
    parsed: &mut ParsedCli,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    stderr: &mut W,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    if deferred.entries.is_empty() {
        return Ok(());
    }
    let (instance, client) = connection.get()?;
    run_gates(parsed, client, clock.as_ref())?;
    let mut allocated: HashMap<HostTargetId, SeatId> = HashMap::new();
    for entry in &deferred.entries {
        let seat = match allocated.get(&entry.pane) {
            Some(seat) => seat.clone(),
            None => {
                let seat = super::allocate_recipient_seat(
                    paths,
                    instance,
                    client,
                    clock.as_ref(),
                    &parsed.output,
                    entry.pane.clone(),
                )?;
                allocated.insert(entry.pane.clone(), seat.clone());
                seat
            }
        };
        entry
            .slot
            .set(&mut parsed.action, seat.clone())
            .map_err(run_error)?;
        if let Some((path, tier)) = &entry.line {
            write_resolved_line(stderr, path, &entry.pane, &seat, tier)?;
        }
    }
    for arg in &deferred.slots {
        match arg.slot.current(&parsed.action) {
            Some(value) if value != SEAT_SELECTOR_PLACEHOLDER => {}
            _ => {
                return Err(run_error(ApiError::invalid_request(
                    "seat selector was not resolved before dispatch",
                )));
            }
        }
    }
    post_freeze_checks(&mut parsed.action).map_err(run_error)
}

fn write_resolved_line<W: Write>(
    stderr: &mut W,
    path: &str,
    pane: &HostTargetId,
    seat: &SeatId,
    tier: &str,
) -> std::io::Result<()> {
    let line = format!(
        "resolved: {path} {} seat={} (tier: {tier})\n",
        pane.as_str(),
        seat.as_str()
    );
    #[cfg(test)]
    RESOLVED_LINES.with(|lines| lines.borrow_mut().push(line.clone()));
    stderr.write_all(line.as_bytes())
}

impl<C, F, W, T, L> Stage<'_, C, F, W, T, L>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
    T: FnOnce() -> Result<HostTopology, ApiError>,
    L: FnOnce() -> Result<Option<HostTargetId>, ApiError>,
{
    fn topology(&self) -> Result<&HostTopology, ApiError> {
        self.topology.get().map_err(host_unavailable)
    }

    /// The live caller pane: a failed read is `host_unavailable` only when Herdr itself is
    /// unavailable; any other failure is a stale pane ID (no caller).
    fn caller(&self) -> Result<Option<&HostTargetId>, ApiError> {
        match self.caller.get() {
            Ok(caller) => Ok(caller.as_ref()),
            Err(error) if error.code == ErrorCode::HostUnavailable => Err(error),
            Err(_) => Ok(None),
        }
    }

    /// The host-ID narrowing of `--space`/`--tab`, resolved once through the parent helper the
    /// `--pane` selector uses; `None` without narrowing.
    fn narrowing(&self) -> Result<Option<&Narrowing>, ApiError> {
        let Some(raw) = &self.raw_narrowing else {
            return Ok(None);
        };
        if let Some(narrowing) = self.narrowing.get() {
            return Ok(Some(narrowing));
        }
        let topology = self.topology()?;
        let caller_space = if raw.space.is_none() {
            self.caller()?.and_then(|caller| {
                topology
                    .panes
                    .iter()
                    .find(|pane| pane.target == *caller)
                    .map(|pane| pane.space.as_str())
            })
        } else {
            None
        };
        let parents = resolve_parents(
            raw.space.as_deref(),
            raw.tab.as_deref(),
            topology,
            None,
            caller_space,
        )?;
        let resolved = Narrowing {
            space: Some(parents.space.id.clone()),
            tab: parents.tab.map(|tab| tab.id.clone()),
        };
        Ok(Some(self.narrowing.get_or_init(|| resolved)))
    }

    /// `SeatInspect` of an exact seat ID: `Some` when the daemon knows it (retired included),
    /// `None` on `not_found`; any other error propagates.
    fn inspect(&self, seat: &SeatId) -> Result<Option<SeatInspection>, RunError> {
        let (_, client) = self.connection.get()?;
        match client.call(
            Command::SeatInspect(SeatInspectQuery {
                seat: seat.clone(),
                page: PageRequest {
                    limit: 1,
                    ..Default::default()
                },
            }),
            &super::cooperative_budget(self.clock.as_ref()),
        ) {
            Ok(CommandResult::SeatInspect(inspection)) if inspection.summary.seat == *seat => {
                Ok(Some(inspection))
            }
            Ok(CommandResult::SeatInspect(_)) => Ok(None),
            Ok(_) => Err(run_error(ApiError::invalid_request(
                "unexpected seat inspection result",
            ))),
            Err(error) if error.code == ErrorCode::NotFound => Ok(None),
            Err(error) => Err(run_error(error)),
        }
    }

    /// With narrowing, the exact or nickname seat must lie inside it (spec D2).
    fn verify_narrowed(
        &self,
        raw: &str,
        seat: &SeatId,
        inspection: &SeatInspection,
    ) -> Result<(), RunError> {
        if let Some(narrowing) = self.narrowing().map_err(run_error)? {
            let topology = self.topology().map_err(run_error)?;
            seat_selector::verify_location(topology, narrowing, live_target(inspection))
                .into_result(raw, seat)
                .map_err(run_error)?;
        }
        Ok(())
    }

    /// One slot's canonical seat (spec D2 order).
    fn resolve_one(
        &mut self,
        parsed: &ParsedCli,
        slot: SeatSlot,
        raw: &str,
    ) -> Result<Resolved, RunError> {
        // 1. Exact seat ID: only an existing seat takes this step.
        if let Ok(seat) = SeatId::parse(raw)
            && let Some(inspection) = self.inspect(&seat)?
        {
            self.verify_narrowed(raw, &seat, &inspection)?;
            return Ok(Resolved::Seat(seat));
        }
        // 2. Nickname PREFIX/SEAT_ID: the last part is an existing seat; the prefix must name
        // the seat's live location.
        if let Some((prefix, last)) = seat_selector::split_nickname(raw)
            && let Ok(seat) = SeatId::parse(last)
            && let Some(inspection) = self.inspect(&seat)?
        {
            let topology = self.topology().map_err(run_error)?;
            seat_selector::verify_nickname(prefix, topology, live_target(&inspection))
                .into_result(raw, &seat)
                .map_err(run_error)?;
            self.verify_narrowed(raw, &seat, &inspection)?;
            return Ok(Resolved::Seat(seat));
        }
        // 3. A direct pane ID needs no topology unless narrowing applies.
        if looks_like_pane_id(raw) && self.raw_narrowing.is_none() {
            let pane = HostTargetId::parse(raw.to_owned()).map_err(|detail| {
                run_error(ApiError::invalid_request(format!("pane ID: {detail}")))
            })?;
            return self.map_pane(parsed, slot, pane);
        }
        // 4. Name path against the live topology.
        let narrowing = self.narrowing().map_err(run_error)?.cloned();
        let narrowing = narrowing.unwrap_or_default();
        let topology = self.topology().map_err(run_error)?;
        let caller = self.caller().map_err(run_error)?;
        let (_, client) = self.connection.get()?;
        let clock = self.clock.as_ref();
        let candidate = seat_selector::resolve(raw, topology, caller, &narrowing)
            .into_result(raw, |pane| {
                match super::collect_pane_seats(pane, |command| {
                    client.call(command, &super::cooperative_budget(clock))
                }) {
                    Ok(seats) => match seats.resolved.as_slice() {
                        [one] => Ok(Some(one.seat.clone())),
                        [] => Ok(None),
                        _ => Err(()),
                    },
                    Err(_) => Err(()),
                }
            })
            .map_err(run_error)?;
        let path = candidate.path.clone();
        let tier = candidate.tier.as_str().to_owned();
        match self.map_pane(parsed, slot, candidate.pane.clone())? {
            Resolved::Seat(seat) => {
                write_resolved_line(self.stderr, &path, &candidate.pane, &seat, &tier)?;
                Ok(Resolved::Seat(seat))
            }
            Resolved::Pane { pane, .. } => Ok(Resolved::Pane {
                pane,
                line: Some((path, tier)),
            }),
        }
    }

    /// Steps 3 and 4: the pane's seat by the slot's D4 class. A lookup resolves now; an
    /// allocation is deferred to pass 2 (no journal write here).
    fn map_pane(
        &mut self,
        parsed: &ParsedCli,
        slot: SeatSlot,
        pane: HostTargetId,
    ) -> Result<Resolved, RunError> {
        match slot.mapping() {
            SeatMapping::Lookup => {
                let (_, client) = self.connection.get()?;
                super::pane_seat(client, &parsed.output, &pane, self.clock.as_ref())?
                    .map(Resolved::Seat)
                    .ok_or_else(|| super::no_seat_for_pane(&pane))
            }
            SeatMapping::Allocate => Ok(Resolved::Pane { pane, line: None }),
        }
    }
}

/// The one gate point before the first journaled allocation (pass 2 runs it once): p0 sends
/// need the send-time receipts capability, confirmed non-operator invites the invitation
/// confirmation capability, and a direct send the direct-thread capabilities `start` demands.
/// The caller derivation and the agent-without-session refusal have already run by then.
fn run_gates<C: LocalClient + ?Sized>(
    parsed: &ParsedCli,
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    let p0 = match &parsed.action {
        CliAction::Mutation(MutationSpec::Send { priority, .. }) => priority.get() == 0,
        CliAction::Mutation(MutationSpec::DirectSend(request)) => {
            request.priority.is_some_and(|priority| priority.get() == 0)
        }
        _ => false,
    };
    if p0 {
        super::require_send_time_receipts_capability(client, clock)?;
    }
    if let CliAction::Mutation(MutationSpec::DirectSend(request)) = &parsed.action {
        super::direct_send::require_capabilities(request, client, clock)?;
    }
    if matches!(
        &parsed.action,
        CliAction::Mutation(MutationSpec::Invite {
            confirmation: Some(_),
            operator: false,
            ..
        })
    ) {
        super::require_mutation_capability(
            client,
            clock,
            INVITATION_CONFIRMATION_V1,
            "requesting invitation confirmation policy",
        )?;
    }
    Ok(())
}

/// Checks over frozen IDs (a name and an ID can spell the same seat): a direct ACK names only the
/// target; explicit recipients are deduplicated keeping the first occurrence. A repeated or
/// name/ID-equivalent recipient is deduplicated, not refused, so the daemon's own duplicate
/// refusal never sees one.
pub(crate) fn post_freeze_checks(action: &mut CliAction) -> Result<(), ApiError> {
    match action {
        CliAction::Mutation(MutationSpec::DirectSend(request)) => {
            check_direct_ack(&request.target, &mut request.require_ack)
        }
        CliAction::Mutation(MutationSpec::Send { require_ack, .. }) => {
            let mut seen = HashSet::new();
            require_ack.retain(|seat| seen.insert(seat.clone()));
            check_unique_recipients(require_ack)
        }
        _ => Ok(()),
    }
}
