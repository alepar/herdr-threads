//! The liveness worker (spec §3 Becoming disconnected, decision 9): re-checks the process
//! fingerprints of connected bindings on a cadence and before wake decisions.
//!
//! A binding disconnects `ProcessExit` when its fingerprinted process is gone (or the pid now
//! belongs to a process with another start stamp), or when it is connected with no
//! fingerprint (decision 9: no zombie bindings).
//! Among connected top-level bindings sharing one process (same instance, harness, pid and start)
//! the latest lifecycle claim stays connected; the rest, plain-call-only bindings included,
//! disconnect `Superseded`. A plain-call-only binding or a `managed_launch` placeholder ranks
//! below every lifecycle-claimed session (it can lose, never win), and a group with no
//! lifecycle-claimed session is left as it is. Every write
//! goes through `bindings::disconnect`, the sole transition: the worker never ends a binding,
//! retires, moves or merges (TRUST-POLICY: liveness evidence is structural). Human bindings and
//! seats that are unresolved, retired or under a recovery hold are never judged
//! (`bindings::liveness_candidates`), so a Herdr restart cannot disconnect through here.
//! An idle pass (every process alive) is one read and no write.
use crate::{
    ports::{ProcessInfo, StorePort},
    protocol::{
        ids::SeatId,
        results::ApiError,
        time::{CallBudget, Cancellation, MonoInstant},
    },
    service::{
        fair_writer::FairWriter,
        pacer::{Pacer, Wake},
    },
    store::bindings::{
        BindingRole, BindingRow, DisconnectReason, LivenessCandidate, LivenessDecision,
    },
};
use std::{collections::BTreeMap, sync::Arc};

/// What one pass saw and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassOutcome {
    pub checked: usize,
    pub disconnected: usize,
}

pub struct LivenessWorker {
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    processes: Arc<dyn ProcessInfo>,
}

/// Judges candidates against the process table. Pure: no store access.
pub fn classify(
    candidates: &[LivenessCandidate],
    processes: &dyn ProcessInfo,
) -> Vec<LivenessDecision> {
    let mut decisions = Vec::new();
    // (instance, harness, pid, start, seat when the process spans seats) -> alive top-level
    // claimants.
    type Group = (String, String, u32, u64, Option<String>);
    let mut sharing: BTreeMap<Group, Vec<&LivenessCandidate>> = BTreeMap::new();
    for candidate in candidates {
        let binding = &candidate.binding;
        match binding.fingerprint {
            Some(fingerprint) if fingerprint.is_alive(processes) => {
                // Every alive top-level binding shares its process, plain-call-only ones and
                // `managed_launch` placeholders included: they rank below lifecycle-claimed
                // sessions and so can lose, never win (ht-54r.87). Keep in step with `store::bindings::is_superseded_now` and
                // its `supersede_rank`.
                if binding.role == BindingRole::TopLevel && !binding.concurrent_session {
                    sharing
                        .entry((
                            candidate.instance.clone(),
                            binding.harness.clone(),
                            fingerprint.pid,
                            fingerprint.start,
                            crate::store::bindings::process_spans_seats(&binding.harness)
                                .then(|| binding.seat.as_str().to_owned()),
                        ))
                        .or_default()
                        .push(candidate);
                }
            }
            // Exited, or the pid now belongs to a process with another start.
            Some(_) => decisions.push(decision(binding, DisconnectReason::ProcessExit)),
            // A connected agent binding with no fingerprint has no process to vouch for it
            // (decision 9: no zombie bindings). The migrated 24 h grace is archival only.
            None => decisions.push(decision(binding, DisconnectReason::ProcessExit)),
        }
    }
    for group in sharing.values().filter(|group| group.len() > 1) {
        let rank = |c: &&&LivenessCandidate| {
            (
                c.binding.may_win_same_process(),
                c.claimed_at,
                c.binding.ordinal,
            )
        };
        let Some(winner) = group.iter().max_by_key(rank) else {
            continue;
        };
        // No lifecycle-claimed session in the group: a plain-call-only binding or a
        // `managed_launch` placeholder never wins.
        if !winner.binding.may_win_same_process() {
            continue;
        }
        decisions.extend(
            group
                .iter()
                .filter(|c| c.binding.binding_id != winner.binding.binding_id)
                .map(|c| decision(&c.binding, DisconnectReason::Superseded)),
        );
    }
    decisions
}

fn decision(binding: &BindingRow, reason: DisconnectReason) -> LivenessDecision {
    LivenessDecision {
        binding: binding.binding_id.clone(),
        fingerprint: binding.fingerprint,
        reason,
    }
}

impl LivenessWorker {
    pub fn new(
        store: Arc<dyn StorePort>,
        writer: Arc<FairWriter>,
        processes: Arc<dyn ProcessInfo>,
    ) -> Self {
        Self {
            store,
            writer,
            processes,
        }
    }

    /// One pass: a read of the connected candidates, classification in memory, and one write
    /// transaction only when something changed.
    pub fn pass(&self, budget: &CallBudget) -> Result<PassOutcome, ApiError> {
        self.check(None, budget)
    }

    /// The same check limited to one seat; the scheduler calls it before a wake decision.
    pub fn recheck_seat(&self, seat: &SeatId, budget: &CallBudget) -> Result<(), ApiError> {
        self.check(Some(seat), budget).map(|_| ())
    }

    fn check(&self, seat: Option<&SeatId>, budget: &CallBudget) -> Result<PassOutcome, ApiError> {
        let candidates = self.store.liveness_candidates(seat, budget)?;
        let decisions = classify(&candidates, self.processes.as_ref());
        if decisions.is_empty() {
            return Ok(PassOutcome {
                checked: candidates.len(),
                disconnected: 0,
            });
        }
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        let disconnected = self.store.apply_liveness(&decisions, budget)?;
        Ok(PassOutcome {
            checked: candidates.len(),
            disconnected,
        })
    }
}

/// The liveness thread: one pass per `interval` ([`crate::scheduler::config::LIVENESS_RECHECK_INTERVAL`] in production; see
/// `ServiceConfig::lane_tick`) on its own `Pacer`. It is not
/// a store lane (no kick set, no Health entry); a failed pass backs off on the Pacer and is
/// logged (first failure, then every 60th). Its commits carry no lane origin, so a disconnect
/// kicks the lanes that read presence (wakes, deadlines).
pub fn start_liveness_worker(
    worker: Arc<LivenessWorker>,
    pacer: Arc<Pacer>,
    cancellation: Cancellation,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    interval: std::time::Duration,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("herdr-liveness".into())
        .spawn(move || {
            let mut failures = 0u64;
            while !cancellation.is_cancelled() {
                let budget = CallBudget {
                    deadline: MonoInstant(
                        worker.store.clock().monotonic_now().0.saturating_add(5_000),
                    ),
                    cancellation: cancellation.clone(),
                };
                match worker.pass(&budget) {
                    Ok(_) => {
                        pacer.on_success();
                        failures = 0;
                    }
                    Err(_) if cancellation.is_cancelled() => break,
                    Err(error) => {
                        pacer.on_failure();
                        if failures.is_multiple_of(60) {
                            log(&format!(
                                "liveness pass failed: {:?}: {}",
                                error.code, error.detail
                            ));
                        }
                        failures += 1;
                    }
                }
                if pacer.wait_blocking(interval) == Wake::Cancelled {
                    break;
                }
            }
        })
}

#[cfg(test)]
#[path = "../../tests/service/liveness_worker.rs"]
mod liveness_worker_tests;
