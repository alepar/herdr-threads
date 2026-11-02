//! The daemon side of the launch gate (`Command::AuthorizeLaunchGate`, `LAUNCH_GATE_V1`).
//!
//! Responsibility
//! - Owns: deciding whether the terminal a gate shim runs in may `exec` its harness: the
//!   shim's `(core, tN)` must equal the bootstrap's `CreatedTarget` witness, a fresh
//!   `terminal.get` through the host's observer must confirm it, and the bootstrap Send must
//!   be published (the harness never runs before its first message is staged; a peer
//!   launch's or a plain local launch's gate-only record releases on its recorded creation
//!   instead, N15 and H3).
//!   A plain local launch's gate-only record (`local_launch_gates`, H3) releases the same way.
//! - Also owns: `allocate_seat`, the seat of a gate-only launch (peer or local), allocated
//!   before its gate opens.
//! - Hides: the polling of the canonical view; the wait holds no store turn and no writer
//!   lane (each poll is one short read).
//! - Must not: write in the decision (`allocate_seat` is the launchers' shared write, and that
//!   function alone enters the writer), authorize on a label or a client-supplied path (A2: the canonical view
//!   and a same-response read decide; the shim's socket only orders the observers, creation
//!   core first, and the witness comparison still decides), or wait past the request's
//!   `timeout_ms`. A host that is unavailable is retried until the deadline and then `TimedOut`,
//!   never `WitnessMismatch`.

use std::sync::Arc;
use std::time::Duration;

use crate::harness::registry::OccupantHarness;
use crate::service::{fair_writer::FairWriter, host_sessions::HostSession};

use crate::{
    ports::{
        BootstrapObserver, CreatedTarget, HostCallContext, HostWitness, IncarnationEvidence,
        LaunchGateView, ProcessInfo, StorePort,
    },
    protocol::{
        authority::LivenessFingerprint,
        commands::{RecordManagedLaunch, ResolveSeat},
        handoff::{AuthorizeLaunchGate, LaunchGateRefusal, LaunchGateReply},
        ids::{OperationId, SeatId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Clock, MonoInstant},
    },
};

/// Budget of the seat allocation of a gate-only launch (ms), under the gate shim's own wait
/// (`launch_gate::GATE_TIMEOUT_MS`) so the shim is still waiting when the gate opens.
pub const GATE_SEAT_BUDGET_MS: u64 = 6_000;
/// How long the daemon's request bound outlasts a gate handler's declared wait, so its answer
/// is written before the shim's own call gives up (`launch_gate::CALL_SLACK_MS`, 2 s).
pub const GATE_REPLY_SLACK: Duration = Duration::from_secs(1);
/// How often a stale host observation is retried within that budget.
const GATE_SEAT_RETRY: Duration = Duration::from_millis(100);

/// Time between two reads of the canonical view while the gate waits.
const POLL: Duration = Duration::from_millis(20);

/// What the gate reads of the world: the canonical view of the bootstrap and the observers of
/// every attached host that creates its own targets.
pub struct GateWorld<'a> {
    pub view: &'a dyn Fn(&CallBudget) -> Result<Option<LaunchGateView>, ApiError>,
    pub observers: &'a [Arc<dyn BootstrapObserver>],
}

fn refused(reason: LaunchGateRefusal) -> LaunchGateReply {
    LaunchGateReply::Refused { reason }
}

/// The creation names exactly the shim's terminal on exactly its core.
fn witness_matches(created: &CreatedTarget, request: &AuthorizeLaunchGate) -> bool {
    matches!(&created.witness, HostWitness::HcCore { core_id, .. } if *core_id == request.core_id)
        && created.terminal.as_str() == request.terminal
}

/// What the observers said of the creation's terminal.
enum Confirmed {
    Yes,
    /// Reachable, and not the creation's terminal.
    No,
    /// No observer could say (host unavailable, or the gate's time ran out): poll again.
    Unavailable,
}

/// A same-response `terminal.get` on the creation's terminal, on the creation's core and
/// serving process. The observers come creation core first; the gate's deadline is re-checked
/// before each, and each read is budgeted to what the gate has left.
fn terminal_confirms(
    created: &CreatedTarget,
    observers: &[Arc<dyn BootstrapObserver>],
    budget: &CallBudget,
    clock: &dyn Clock,
    deadline: MonoInstant,
) -> Confirmed {
    let context = HostCallContext {
        budget: CallBudget {
            deadline,
            cancellation: budget.cancellation.clone(),
        },
        expected_boot: Some(created.observed_boot()),
        expected_epoch: None,
    };
    let mut unavailable = false;
    for observer in observers {
        if clock.monotonic_now() >= deadline {
            return Confirmed::Unavailable;
        }
        match observer.observe_bootstrap_target(&created.target, &context) {
            Ok(seen) if seen.host_witness() == created.witness => return Confirmed::Yes,
            Err(error) if error.code == ErrorCode::HostUnavailable => unavailable = true,
            _ => {}
        }
    }
    if unavailable {
        Confirmed::Unavailable
    } else {
        Confirmed::No
    }
}

/// Decide one authorization, waiting at most `request.timeout_ms` (and the call's budget) for
/// the creation to be recorded and the bootstrap Send to be staged.
pub fn authorize(
    request: &AuthorizeLaunchGate,
    world: &GateWorld<'_>,
    clock: &dyn Clock,
    budget: &CallBudget,
) -> Result<LaunchGateReply, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let deadline = MonoInstant(
        clock
            .monotonic_now()
            .0
            .saturating_add(u64::from(request.timeout_ms)),
    )
    .min(budget.deadline);
    loop {
        if budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("launch gate"));
        }
        let view = (world.view)(budget)?;
        let Some(view) = view else {
            return Ok(refused(LaunchGateRefusal::UnknownBootstrap));
        };
        if view.cancelled {
            return Ok(refused(LaunchGateRefusal::StaleAttempt));
        }
        if !view.current_attempt {
            return Ok(refused(LaunchGateRefusal::StaleAttempt));
        }
        if let Some(created) = &view.created {
            if !witness_matches(created, request) {
                return Ok(refused(LaunchGateRefusal::WitnessMismatch));
            }
            if view.send_staged {
                match terminal_confirms(created, world.observers, budget, clock, deadline) {
                    Confirmed::Yes => return Ok(LaunchGateReply::Authorized),
                    Confirmed::No => return Ok(refused(LaunchGateRefusal::WitnessMismatch)),
                    Confirmed::Unavailable => {}
                }
            }
        }
        if clock.monotonic_now() >= deadline {
            return Ok(refused(LaunchGateRefusal::TimedOut));
        }
        std::thread::sleep(POLL);
    }
}

/// What a gate-only launch's seat is allocated from.
pub struct GateSeat<'a> {
    pub session: &'a HostSession,
    pub store: &'a dyn StorePort,
    pub writer: &'a FairWriter,
    pub clock: &'a dyn Clock,
    /// The request's budget: the allocation ends with it and stops when its caller is gone.
    pub budget: &'a CallBudget,
    pub harness: OccupantHarness,
    pub operation: OperationId,
}

/// The seat of a gate-only launch, allocated before its gate opens: the target's ordinary
/// guarded resolution (no caller, as `Command::ResolveSeat` runs it), then the
/// `managed_launch` binding on the process the gate shim is, which `exec` keeps (L2). No
/// pid or process record is an error: the gate never opens without its placeholder.
pub fn allocate_seat(seat: &GateSeat<'_>, created: &CreatedTarget) -> Result<SeatId, ApiError> {
    let budget = CallBudget {
        deadline: MonoInstant(
            (seat.clock.monotonic_now().0 + GATE_SEAT_BUDGET_MS).min(seat.budget.deadline.0),
        ),
        cancellation: seat.budget.cancellation.clone(),
    };
    let allocated = seat.session.identity().resolve(
        ResolveSeat {
            target: created.target.clone(),
            host: seat.session.locator().clone(),
            operation: seat.operation.clone(),
            locator_hint: None,
            tmux_hint: None,
        },
        &budget,
    )?;
    let context = crate::ports::HostCallContext {
        budget: budget.clone(),
        expected_boot: None,
        expected_epoch: None,
    };
    let port = seat.session.port();
    let observation = port.observe_current_target(&created.target, &context)?;
    let evidence = |detail: &'static str| ApiError::new(ErrorCode::InvalidRequest, detail);
    let terminal = observation
        .terminal
        .clone()
        .ok_or_else(|| evidence("the launched terminal reports no terminal id"))?;
    let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
        return Err(evidence(
            "the launched terminal has no verified incarnation",
        ));
    };
    let pid = port
        .target_root_process(&created.target, &context)?
        .ok_or_else(|| evidence("the launched terminal reports no process"))?;
    let record = crate::host::process_table::HostProcessTable
        .process(pid)
        .ok_or_else(|| evidence("the launched terminal's process is not observable"))?;
    let command = RecordManagedLaunch {
        seat: allocated.clone(),
        target: created.target.clone(),
        harness: seat.harness,
        terminal,
        incarnation: identity.clone(),
        host_boot: observation.host_boot.clone(),
        target_generation: observation.generation,
        fingerprint: LivenessFingerprint::of(&record),
    };
    loop {
        let attempt = seat
            .writer
            .enter_background(&budget, seat.clock)
            .and_then(|_turn| seat.store.record_managed_launch(command.clone(), &budget));
        match attempt {
            Ok(_) => return Ok(allocated),
            // The store decides on its effective observation, which can lag the fresh read.
            Err(error)
                if error.code == ErrorCode::StaleHostObservation
                    && seat.clock.monotonic_now() < budget.deadline =>
            {
                std::thread::sleep(GATE_SEAT_RETRY);
            }
            Err(error) => return Err(error),
        }
    }
}
