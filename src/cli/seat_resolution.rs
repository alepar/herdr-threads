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
//! actor refusal), behind one gate point (`run_gates`). Refusals that run later (the selection and
//! mapping checks, the direct-send claim and caller-binding checks) can still leave an idempotent
//! guarded seat for a real pane; the stage does not promise that a refused command allocates
//! nothing.
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
use crate::sync::frame::{AddrOutcome, AddrTarget};
use crate::{
    daemon::paths::{NodePaths, RuntimeContext},
    host::observation::HostTopology,
    ports::LocalClient,
    protocol::{
        capabilities::INVITATION_CONFIRMATION_V1,
        commands::{Command, SeatInspectQuery},
        ids::{HostTargetId, SeatId},
        pagination::PageRequest,
        results::{ApiError, CommandResult, ContinuityStatus, ErrorCode, SeatInspection},
        sync_admin::{AddressReply, RemoteAddress, SyncCommand, SyncReply},
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
    F: Fn() -> Result<(crate::node::NodeUuid, C), RunError>,
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
    F: Fn() -> Result<(crate::node::NodeUuid, C), RunError>,
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
/// place `run_gates` runs, so those refusals journal no `ResolveSeat`. Refusals that run later
/// (selection and mapping checks, the direct-send claim and caller-binding checks) can still
/// leave an idempotent guarded seat for a real pane.
pub(crate) fn allocate_deferred_seats<C, F, W>(
    deferred: DeferredSeats,
    parsed: &mut ParsedCli,
    context: &RuntimeContext,
    paths: &NodePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    stderr: &mut W,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(crate::node::NodeUuid, C), RunError>,
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
                    context,
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
    write_resolved_text(stderr, path, pane.as_str(), seat, tier)
}

/// [`write_resolved_line`] with the pane as display text (`-` for a remote seat with none).
fn write_resolved_text<W: Write>(
    stderr: &mut W,
    path: &str,
    pane: &str,
    seat: &SeatId,
    tier: &str,
) -> std::io::Result<()> {
    let line = format!(
        "resolved: {path} {pane} seat={} (tier: {tier})\n",
        seat.as_str()
    );
    #[cfg(test)]
    RESOLVED_LINES.with(|lines| lines.borrow_mut().push(line.clone()));
    stderr.write_all(line.as_bytes())
}

impl<C, F, W, T, L> Stage<'_, C, F, W, T, L>
where
    C: LocalClient,
    F: Fn() -> Result<(crate::node::NodeUuid, C), RunError>,
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
        // An `@MACHINE` suffix (remote addresses): the home node resolves the location; the whole
        // string keeps its local reading.
        if let Some((location, machine)) = seat_selector::split_machine(raw)
            && let Some(resolved) = self.resolve_at(parsed, slot, raw, location, machine)?
        {
            return Ok(resolved);
        }
        self.resolve_local(parsed, slot, raw)
    }

    /// Steps 1-4 of `raw` against this node only.
    fn resolve_local(
        &mut self,
        parsed: &ParsedCli,
        slot: SeatSlot,
        raw: &str,
    ) -> Result<Resolved, RunError> {
        if let Ok(seat) = SeatId::parse(raw)
            && let Some(inspection) = self.inspect(&seat)?
        {
            self.verify_narrowed(raw, &seat, &inspection)?;
            return Ok(Resolved::Seat(seat));
        }
        let selector = seat_selector::Selector::new(raw);
        // 2. Nickname PREFIX/SEAT_ID: the last part is an existing seat; the prefix must name
        // the seat's live location.
        let nicknames = selector.nicknames();
        for nickname in &nicknames {
            let Ok(seat) = SeatId::parse(nickname.last) else {
                continue;
            };
            let Some(inspection) = self.inspect(&seat)? else {
                continue;
            };
            let topology = self.topology().map_err(run_error)?;
            if !nickname.applies(topology) {
                continue;
            }
            nickname
                .verify(topology, live_target(&inspection))
                .into_result(raw, &seat)
                .map_err(run_error)?;
            self.verify_narrowed(raw, &seat, &inspection)?;
            let narrowing = self
                .narrowing()
                .map_err(run_error)?
                .cloned()
                .unwrap_or_default();
            if let Some(collision) = selector.nickname_collision(
                nickname,
                topology,
                &narrowing,
                live_target(&inspection),
            ) {
                self.candidate(collision, raw)?;
            }
            // Escaping can change the final opaque ID (for example seat\/tail versus tail).
            // Preserve the legacy nickname's refusal/precedence, but refuse a second valid
            // decoded nickname at a different target before returning its canonical seat.
            if nickname.is_legacy_path() {
                for decoded in nicknames
                    .iter()
                    .filter(|n| !n.is_legacy_path() && n.last != nickname.last)
                {
                    let Ok(decoded_seat) = SeatId::parse(decoded.last) else {
                        continue;
                    };
                    let Some(decoded_inspection) = self.inspect(&decoded_seat)? else {
                        continue;
                    };
                    let decoded_target = live_target(&decoded_inspection);
                    if decoded.verify(topology, decoded_target)
                        == seat_selector::LocationCheck::Match
                        && seat_selector::verify_location(topology, &narrowing, decoded_target)
                            == seat_selector::LocationCheck::Match
                        && let Some(collision) = nickname.conflict_with(
                            decoded,
                            topology,
                            live_target(&inspection),
                            decoded_target,
                        )
                    {
                        self.candidate(collision, raw)?;
                    }
                }
            }
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
        let candidate = self.candidate(
            selector.resolve_with(topology, caller, &narrowing, slot.tiering()),
            raw,
        )?;
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

    /// Whether `raw` names a live local pane (its whole-string reading): a read failure is no hit.
    fn local_hit(&self, raw: &str) -> bool {
        let Ok(topology) = self.topology() else {
            return false;
        };
        let caller = self.caller().ok().flatten();
        let narrowing = self.narrowing().ok().flatten().cloned().unwrap_or_default();
        !matches!(
            seat_selector::Selector::new(raw).resolve_with(
                topology,
                caller,
                &narrowing,
                seat_selector::Tiering::Global
            ),
            seat_selector::Outcome::NoMatch | seat_selector::Outcome::InvalidPath
        )
    }

    /// Ask the daemon to resolve `location` on `machine` (spec 4.3). The outer error is a
    /// connection failure; the inner one the daemon's refusal.
    fn address(
        &self,
        machine: &str,
        location: &str,
    ) -> Result<Result<AddressReply, ApiError>, RunError> {
        let (_, client) = self.connection.get()?;
        let budget = crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(
                self.clock
                    .monotonic_now()
                    .0
                    .saturating_add(REMOTE_ADDRESS_BUDGET_MS),
            ),
            cancellation: crate::protocol::time::Cancellation::default(),
        };
        Ok(
            match client.call(
                Command::Sync(SyncCommand::ResolveAddress {
                    machine: machine.to_owned(),
                    target: AddrTarget::Seat {
                        location: location.to_owned(),
                    },
                }),
                &budget,
            ) {
                Ok(CommandResult::Sync(SyncReply::Address(reply))) => Ok(reply),
                Ok(_) => Err(ApiError::invalid_request("unexpected address result")),
                Err(error) => Err(error),
            },
        )
    }

    /// `location@machine` (spec D2): `Ok(None)` when only the whole string's local reading can
    /// apply (the suffix names no known machine), else the one resolution or a refusal. A
    /// suffix naming a machine that cannot answer is refused, never resolved locally (D7).
    fn resolve_at(
        &mut self,
        parsed: &ParsedCli,
        slot: SeatSlot,
        raw: &str,
        location: &str,
        machine: &str,
    ) -> Result<Option<Resolved>, RunError> {
        let reply = match self.address(machine, location)? {
            Ok(reply) => reply,
            // Only a daemon that cannot resolve machines (it has no such command) knows none;
            // any other failure leaves the address refused (D7, N14).
            Err(error) if cannot_resolve_machines(&error) => {
                return if self.local_hit(raw) {
                    Ok(None)
                } else {
                    Err(run_error(remote_error::unknown_machine(machine, &[])))
                };
            }
            Err(error) => return Err(run_error(error)),
        };
        let (node, answer) = match reply {
            AddressReply::UnknownMachine { known } => {
                return if self.local_hit(raw) {
                    Ok(None)
                } else {
                    Err(run_error(remote_error::unknown_machine(machine, &known)))
                };
            }
            AddressReply::AmbiguousMachine { matches } => {
                return Err(run_error(remote_error::ambiguous_machine(
                    machine, &matches,
                )));
            }
            AddressReply::Unknown => {
                return Err(run_error(remote_error::unevaluable(
                    raw,
                    location,
                    machine,
                    "the daemon answered in an unknown form",
                    self.local_hit(raw),
                )));
            }
            AddressReply::SelfNode => {
                if let Ok(seat) = SeatId::parse(location)
                    && let Some(inspection) = self.inspect(&seat)?
                    && inspection.summary.continuity == ContinuityStatus::Remote
                {
                    return Err(run_error(ApiError::invalid_request(format!(
                        "seat {} lives on another machine, not {}",
                        seat.as_str(),
                        machine
                    ))));
                }
                let hit = self.local_hit(raw);
                let at_self = self.resolve_local(parsed, slot, location);
                if !hit {
                    return at_self.map(Some);
                }
                // Both readings name a local pane: the same one is fine, two are a conflict.
                let Ok(at_self) = at_self else {
                    return self.resolve_local(parsed, slot, raw).map(Some);
                };
                return match self.resolve_local(parsed, slot, raw) {
                    Ok(whole) if same_resolution(&at_self, &whole) => Ok(Some(whole)),
                    _ => Err(run_error(remote_error::both_readings(
                        raw,
                        &[raw.to_owned(), location.to_owned()],
                    ))),
                };
            }
            AddressReply::Remote { node, answer } => (node, answer),
        };
        if self.raw_narrowing.is_some() {
            return Err(run_error(ApiError::invalid_request(
                "--space/--tab cannot narrow a remote address; qualify its path instead",
            )));
        }
        let hit = self.local_hit(raw);
        let label = remote_error::machine_label(&node);
        let reason = match answer {
            RemoteAddress::Answered { outcome } => match outcome {
                AddrOutcome::Seat { seat, path, pane } => {
                    if hit {
                        return Err(run_error(remote_error::both_readings(
                            raw,
                            &[raw.to_owned(), format!("{path}@{}", node.tag)],
                        )));
                    }
                    let seat = SeatId::parse(&seat).map_err(|_| {
                        run_error(ApiError::invalid_request(
                            "machine answered a malformed seat",
                        ))
                    })?;
                    let pane_shown = if pane.is_empty() {
                        "-".to_owned()
                    } else {
                        remote_error::one_line(&pane)
                    };
                    let shown = if path.is_empty() {
                        format!("@{}", node.tag)
                    } else {
                        format!("{path}@{}", node.tag)
                    };
                    write_resolved_text(self.stderr, &shown, &pane_shown, &seat, "any")?;
                    return Ok(Some(Resolved::Seat(seat)));
                }
                AddrOutcome::NoSeat { path, pane } if !hit => {
                    return Err(run_error(remote_error::no_seat(&path, &pane, &node)));
                }
                AddrOutcome::NoMatch if hit => return Ok(None),
                AddrOutcome::NoMatch => {
                    return Err(run_error(ApiError::not_found(format!(
                        "seat {location}@{}: neither an existing seat ID nor a live space/tab/pane \
                         name on {label}",
                        node.tag
                    ))));
                }
                AddrOutcome::Ambiguous {
                    candidates,
                    omitted,
                } => {
                    return Err(run_error(remote_error::ambiguous(
                        raw,
                        &node,
                        &candidates,
                        omitted,
                        hit,
                    )));
                }
                AddrOutcome::NoSeat { path, .. } => {
                    return Err(run_error(remote_error::both_readings(
                        raw,
                        &[raw.to_owned(), format!("{path}@{}", node.tag)],
                    )));
                }
                AddrOutcome::HostUnavailable => {
                    format!("{label}: seat names need its live Herdr topology")
                }
                // Request-shaped refusals never succeed on retry.
                AddrOutcome::Refused { detail } => {
                    return Err(run_error(ApiError::invalid_request(format!(
                        "{label} refused {location}: {}",
                        remote_error::one_line(&detail)
                    ))));
                }
                AddrOutcome::Unknown => {
                    return Err(run_error(ApiError::invalid_request(format!(
                        "{label} answered in an unknown form"
                    ))));
                }
            },
            RemoteAddress::Unreachable { last_contact_at } => match last_contact_at {
                Some(at) => format!(
                    "machine {label} is unreachable (last contact {}m ago); pass an exact seat ID",
                    self.clock.utc_now().0.saturating_sub(at).max(0) / 60_000
                ),
                None => format!("machine {label} is unreachable; pass an exact seat ID"),
            },
            RemoteAddress::NoCap { version } => format!(
                "machine {label} cannot resolve names (needs addr.resolve.v1; version {})",
                version.as_deref().unwrap_or("unknown")
            ),
            RemoteAddress::Timeout { secs } => {
                format!("machine {label} did not answer in {secs}s; pass an exact seat ID")
            }
            RemoteAddress::NotReplicated { seat } => {
                format!("seat {seat} on {label} has not replicated here yet; retry")
            }
            RemoteAddress::Unknown => format!("{label} answered in an unknown form"),
        };
        Err(run_error(remote_error::unevaluable(
            raw, location, machine, &reason, hit,
        )))
    }

    fn candidate(
        &self,
        outcome: seat_selector::Outcome,
        raw: &str,
    ) -> Result<seat_selector::Candidate, RunError> {
        let (_, client) = self.connection.get()?;
        let clock = self.clock.as_ref();
        outcome
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
            .map_err(run_error)
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

/// A daemon too old for `resolve address` refuses the command itself.
fn cannot_resolve_machines(error: &ApiError) -> bool {
    matches!(
        error.code,
        ErrorCode::Unsupported | ErrorCode::UnknownWireVersion
    )
}

/// Whether two readings resolved the same seat or pane.
fn same_resolution(a: &Resolved, b: &Resolved) -> bool {
    match (a, b) {
        (Resolved::Seat(a), Resolved::Seat(b)) => a == b,
        (Resolved::Pane { pane: a, .. }, Resolved::Pane { pane: b, .. }) => a == b,
        _ => false,
    }
}

/// The CLI's budget for one `@MACHINE` resolution: the daemon's wait for the home node and for
/// the seat to replicate, plus margin, so the daemon's typed answer arrives first (spec 4.3).
const REMOTE_ADDRESS_BUDGET_MS: u64 =
    crate::service::sync_admin::REMOTE_ADDRESS_DAEMON_BOUND_MS + 3_000;

/// Caller-visible refusals of remote addresses (spec 4.4).
mod remote_error {
    use crate::protocol::results::{ApiError, ErrorCode};
    use crate::protocol::sync_admin::MachineRow;
    use crate::sync::frame::AddrCandidate;

    /// Known machines an unknown-machine refusal names.
    const KNOWN_LISTED: usize = 8;

    pub(super) fn one_line(text: &str) -> String {
        crate::view::escape::escape_for_terminal(text, crate::view::escape::Context::SingleLine)
            .into_owned()
    }

    /// `<tag> <host>`, or the tag alone.
    pub(super) fn machine_label(row: &MachineRow) -> String {
        match &row.host {
            Some(host) => format!("{} {}", row.tag, one_line(host)),
            None => row.tag.clone(),
        }
    }

    pub(super) fn unknown_machine(machine: &str, known: &[MachineRow]) -> ApiError {
        let names: Vec<String> = known.iter().take(KNOWN_LISTED).map(machine_label).collect();
        ApiError::not_found(format!(
            "no machine {}; known: {}",
            one_line(machine),
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        ))
    }

    pub(super) fn ambiguous_machine(machine: &str, matches: &[MachineRow]) -> ApiError {
        let mut lines = vec![format!(
            "ambiguous machine {}: {} matches",
            one_line(machine),
            matches.len()
        )];
        lines.extend(
            matches
                .iter()
                .map(|row| format!("  {} {}", row.uuid, machine_label(row))),
        );
        lines.push("pass @<uuid>".to_owned());
        ApiError::new(ErrorCode::Conflict, lines.join("\n"))
    }

    pub(super) fn no_seat(path: &str, pane: &str, node: &MachineRow) -> ApiError {
        ApiError::not_found(format!(
            "no seat at {}@{} (pane {}); a seat appears once an agent there checks in",
            one_line(path),
            node.tag,
            one_line(pane)
        ))
    }

    pub(super) fn ambiguous(
        raw: &str,
        node: &MachineRow,
        candidates: &[AddrCandidate],
        omitted: u32,
        local_hit: bool,
    ) -> ApiError {
        let total =
            candidates.len() + usize::try_from(omitted).unwrap_or(0) + usize::from(local_hit);
        let mut lines = vec![format!(
            "ambiguous seat {}: {total} candidates (tier: any)",
            one_line(raw)
        )];
        if local_hit {
            lines.push(format!("  {} (this machine)", one_line(raw)));
        }
        for c in candidates {
            lines.push(format!(
                "  {}@{} {} seat={}",
                one_line(&c.path),
                node.tag,
                one_line(&c.pane),
                c.seat.as_deref().map_or("none".to_owned(), one_line)
            ));
        }
        if omitted > 0 {
            lines.push(format!("  ({omitted} omitted)"));
        }
        lines.push("qualify with a longer SPACE/TAB/PANE path, or an exact seat ID".to_owned());
        ApiError::new(ErrorCode::Conflict, lines.join("\n"))
    }

    /// The whole string names a local pane and its split names a remote (or another local)
    /// one.
    pub(super) fn both_readings(raw: &str, readings: &[String]) -> ApiError {
        let mut lines = vec![format!(
            "ambiguous seat {}: {} readings",
            one_line(raw),
            readings.len()
        )];
        lines.extend(readings.iter().map(|r| format!("  {}", one_line(r))));
        lines.push(r"for the local name escape the @: 'path:...\@...'".to_owned());
        ApiError::new(ErrorCode::Conflict, lines.join("\n"))
    }

    /// The machine cannot answer: refused, with the escape hint when the whole string also names
    /// a local pane (spec D2).
    pub(super) fn unevaluable(
        raw: &str,
        location: &str,
        machine: &str,
        reason: &str,
        local_hit: bool,
    ) -> ApiError {
        let code = ErrorCode::HostUnavailable;
        if local_hit {
            ApiError::new(
                code,
                format!(
                    r"cannot check {}@{}: {reason}; for the local name use 'path:...\@...' ({})",
                    one_line(location),
                    one_line(machine),
                    one_line(raw)
                ),
            )
        } else {
            ApiError::new(code, reason.to_owned())
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
