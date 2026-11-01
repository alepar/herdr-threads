//! The daemon side of the launch gate (`Command::AuthorizeLaunchGate`, `LAUNCH_GATE_V1`).
//!
//! Responsibility
//! - Owns: deciding whether the terminal a gate shim runs in may `exec` its harness: the
//!   shim's `(core, tN)` must equal the bootstrap's `CreatedTarget` witness, a fresh
//!   `terminal.get` through the host's observer must confirm it, and the bootstrap Send must
//!   be published (the harness never runs before its first message is staged).
//! - Hides: the polling of the canonical view; the wait holds no store turn and no writer
//!   lane (each poll is one short read).
//! - Must not: write, authorize on a label or a client-supplied path (A2: the canonical view
//!   and a same-response read decide), or wait past the request's `timeout_ms`.

use std::sync::Arc;
use std::time::Duration;

use crate::{
    ports::{BootstrapObserver, CreatedTarget, HostCallContext, HostWitness, LaunchGateView},
    protocol::{
        handoff::{AuthorizeLaunchGate, LaunchGateRefusal, LaunchGateReply},
        results::ApiError,
        time::{CallBudget, Clock, MonoInstant},
    },
};

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

/// A same-response `terminal.get` on the creation's terminal, on the creation's core and
/// serving process.
fn terminal_confirms(
    created: &CreatedTarget,
    observers: &[Arc<dyn BootstrapObserver>],
    budget: &CallBudget,
) -> bool {
    let context = HostCallContext {
        budget: budget.clone(),
        expected_boot: Some(created.observed_boot()),
        expected_epoch: None,
    };
    observers.iter().any(|observer| {
        observer
            .observe_bootstrap_target(&created.target, &context)
            .is_ok_and(|seen| seen.host_witness() == created.witness)
    })
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
                return Ok(if terminal_confirms(created, world.observers, budget) {
                    LaunchGateReply::Authorized
                } else {
                    refused(LaunchGateRefusal::WitnessMismatch)
                });
            }
        }
        if clock.monotonic_now() >= deadline {
            return Ok(refused(LaunchGateRefusal::TimedOut));
        }
        std::thread::sleep(POLL);
    }
}
