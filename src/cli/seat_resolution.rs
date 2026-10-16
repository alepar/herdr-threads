//! The seat resolution stage (spec D1, D2, D4-D8): the one place a raw seat argument becomes a
//! canonical seat ID, before any journal intent or dispatch.
//!
//! Owns: the D2 order per slot (exact seat ID, nickname `PREFIX/SEAT_ID`, direct pane ID, name
//! path), the lazy once-per-invocation Herdr topology and live-caller reads, `--space`/`--tab`
//! narrowing, conflict seat annotation, the `SeatMapping` dispatch (lookup or the shared journaled
//! allocation), freezing through `SeatSlot`, the post-freeze ID checks and the `resolved:` stderr
//! lines.
//!
//! Must not: match names (`seat_selector` does), know command kinds (`SeatSlot` does), write any
//! journal intent other than the shared allocation, or run on `retry` (a retry carries no slots).
//! A new seat-argument form goes into the classification in `resolve_one`.

use super::{
    LazyConnection, RunError,
    commands::{
        CliAction, MutationSpec, ParsedCli, SEAT_SELECTOR_PLACEHOLDER, SeatMapping, SeatNarrowing,
        SeatSlot, check_direct_ack, check_unique_recipients,
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
    paths: &'a InstancePaths,
    connection: &'a LazyConnection<C, F>,
    clock: &'a Arc<dyn Clock>,
    stderr: &'a mut W,
    topology: Once<HostTopology, T>,
    caller: Once<Option<HostTargetId>, L>,
    raw_narrowing: Option<SeatNarrowing>,
    narrowing: OnceCell<Narrowing>,
    gates_done: bool,
}

/// Freeze every seat argument of `parsed` to a canonical seat ID (spec D2), or fail the command.
///
/// Per slot, in order: (1) an exact seat ID known to the daemon (retired included) is used as is,
/// reading nothing from Herdr unless `--space`/`--tab` narrow it; (2) `PREFIX/SEAT_ID` whose last
/// part is a known seat is verified against the seat's live location; (3) a pane ID with no
/// narrowing is the pane itself; (4) anything else is a `SPACE/TAB/PANE` name matched by
/// `seat_selector` against the live topology. Steps 3 and 4 map the pane to a seat by the slot's
/// `SeatMapping`: a lookup never allocates; an allocation is the shared journaled `ResolveSeat`,
/// run only after the gates that precede allocation today.
///
/// The topology and the live caller are read lazily, once. A failed read for a nickname, name or
/// narrowed slot is `host_unavailable`. A caller read that fails for any other reason is a stale
/// `HERDR_PANE_ID` (root D3): no caller, every candidate in tier `any`. Exact IDs without
/// narrowing never read Herdr. Each distinct name argument writes one `resolved:` line to
/// `stderr` before any command output. The stage ends by checking that no slot still holds the
/// placeholder: the production guarantee that no raw seat string reaches a journal or dispatch.
pub(crate) fn resolve_cli_seats<C, F, W>(
    parsed: &mut ParsedCli,
    paths: &InstancePaths,
    connection: &LazyConnection<C, F>,
    clock: &Arc<dyn Clock>,
    topology: impl FnOnce() -> Result<HostTopology, ApiError>,
    live_caller: impl FnOnce() -> Result<Option<HostTargetId>, ApiError>,
    stderr: &mut W,
) -> Result<(), RunError>
where
    C: LocalClient,
    F: Fn() -> Result<(uuid::Uuid, C), RunError>,
    W: Write,
{
    if parsed.seat_selectors.is_empty() {
        parsed.seat_narrowing = None;
        return Ok(());
    }
    let slots = std::mem::take(&mut parsed.seat_selectors);
    let mut stage = Stage {
        paths,
        connection,
        clock,
        stderr,
        topology: Once::new(topology),
        caller: Once::new(live_caller),
        raw_narrowing: parsed.seat_narrowing.take(),
        narrowing: OnceCell::new(),
        gates_done: false,
    };
    let mut memo: HashMap<(String, SeatMapping), SeatId> = HashMap::new();
    for arg in &slots {
        let key = (arg.raw.clone(), arg.slot.mapping());
        let seat = match memo.get(&key) {
            Some(seat) => seat.clone(),
            None => {
                let seat = stage.resolve_one(parsed, arg.slot, &arg.raw)?;
                memo.insert(key, seat.clone());
                seat
            }
        };
        #[cfg(test)]
        if SKIP_FIRST_FREEZE.with(|flag| flag.replace(false)) {
            continue;
        }
        arg.slot.set(&mut parsed.action, seat).map_err(run_error)?;
    }
    for arg in &slots {
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
    ) -> Result<SeatId, RunError> {
        // 1. Exact seat ID: only an existing seat takes this step.
        if let Ok(seat) = SeatId::parse(raw)
            && let Some(inspection) = self.inspect(&seat)?
        {
            self.verify_narrowed(raw, &seat, &inspection)?;
            return Ok(seat);
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
            return Ok(seat);
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
        let seat = self.map_pane(parsed, slot, candidate.pane.clone())?;
        let line = format!(
            "resolved: {} {} seat={} (tier: {})\n",
            candidate.path,
            candidate.pane.as_str(),
            seat.as_str(),
            candidate.tier.as_str()
        );
        #[cfg(test)]
        RESOLVED_LINES.with(|lines| lines.borrow_mut().push(line.clone()));
        self.stderr.write_all(line.as_bytes())?;
        Ok(seat)
    }

    /// Steps 3 and 4: the pane's seat by the slot's D4 class.
    fn map_pane(
        &mut self,
        parsed: &ParsedCli,
        slot: SeatSlot,
        pane: HostTargetId,
    ) -> Result<SeatId, RunError> {
        let (instance, client) = self.connection.get()?;
        let clock = self.clock.as_ref();
        match slot.mapping() {
            SeatMapping::Lookup => super::pane_seat(client, &parsed.output, &pane, clock)?
                .ok_or_else(|| super::no_seat_for_pane(&pane)),
            SeatMapping::Allocate => {
                if !self.gates_done {
                    run_gates(parsed, client, clock)?;
                    self.gates_done = true;
                }
                super::allocate_recipient_seat(
                    self.paths,
                    instance,
                    client,
                    clock,
                    &parsed.output,
                    pane,
                )
            }
        }
    }
}

/// The gates that precede recipient allocation today, under the same conditions (allocation is
/// journaled, so a refused command must not leave a `ResolveSeat` intent).
fn run_gates<C: LocalClient + ?Sized>(
    parsed: &ParsedCli,
    client: &C,
    clock: &dyn Clock,
) -> Result<(), RunError> {
    use crate::cli::actor_route::InvocationActor;
    // The agent-without-session refusal, only where `run_caller_scoped` applies it.
    let operator_branch = matches!(&parsed.action,
        CliAction::Mutation(mutation) if super::operator_semantic(mutation).is_some());
    if !operator_branch
        && parsed.actor != InvocationActor::Human
        && super::caller_needs_identity(&parsed.action)
        && let Some(super::caller::CallerOrigin::Refused(refusal)) = super::caller::derive(parsed)
    {
        return Err(super::caller::refusal_error(&refusal));
    }
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
