//! The liveness worker (spec §3 Becoming disconnected, decision 9): re-checks the process
//! fingerprints of connected bindings on a cadence and before wake decisions.
//!
//! A binding disconnects `ProcessExit` when its fingerprinted process is gone (or the pid now
//! belongs to a process with another start stamp), or when it is connected with no
//! fingerprint (decision 9: no zombie bindings).
//! Among connected top-level bindings sharing one process (same instance, harness, pid and start,
//! and seat when the harness's process spans seats)
//! the latest lifecycle claim stays connected; the rest, plain-call-only bindings included,
//! disconnect `Superseded`. A plain-call-only binding or a `managed_launch` placeholder ranks
//! below every lifecycle-claimed session (it can lose, never win), and a group with no
//! lifecycle-claimed session is left as it is. Every write
//! goes through `bindings::disconnect`, the sole transition: the worker never ends a binding,
//! retires, moves or merges (TRUST-POLICY: liveness evidence is structural). Human bindings,
//! seats that are unresolved or retired, and seats of a host under a recovery hold until its
//! reconciliation marker names its current recovery boot/epoch are never judged
//! (`bindings::liveness_candidates`), so a Herdr restart cannot disconnect through here.
//! A process-exit verdict is applied only once the seat's host has answered since the process
//! was read dead (`LivenessHosts`, ht-hrbx): a Herdr restart ends its panes' processes before the
//! new boot is published, and judging them in that window would end bindings with the wrong
//! reason. The verdict is kept pending and applied on a later pass if the host published a
//! coherent capture in between and is not down; a new boot's publication moves the recovery
//! boot, so the reconciliation marker is stale and the candidate read drops the seat first.
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
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

/// What the seat's host says about applying a process-exit verdict now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLiveness {
    /// The host never ends its targets' processes (the named host): apply at once.
    Open,
    /// The host is attached and up: its capture tokens (`HostReachability`).
    Published(CaptureMarks),
    /// The host is down, or not attached: apply nothing, keep nothing pending.
    Closed,
}

/// Where an up host's captures stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureMarks {
    /// Capture attempts begun so far.
    pub started: u64,
    /// The newest capture token that published coherently.
    pub published: u64,
    /// Down-to-up transitions so far: a change means an outage in between.
    pub recoveries: u64,
}

/// Per-host evidence for the liveness worker's publication gate. Production is the daemon's
/// `HostSessions`.
pub trait LivenessHosts: Send + Sync {
    fn liveness(&self, host: crate::ports::HostId) -> HostLiveness;
}

impl LivenessHosts for std::sync::Weak<crate::service::host_sessions::HostSessions> {
    fn liveness(&self, host: crate::ports::HostId) -> HostLiveness {
        let Some(session) = self.upgrade().and_then(|sessions| sessions.get_any(host)) else {
            return HostLiveness::Closed;
        };
        if !session.port().processes_end_on_restart() {
            HostLiveness::Open
        } else if session.reachability().is_down() {
            HostLiveness::Closed
        } else {
            let reachability = session.reachability();
            HostLiveness::Published(CaptureMarks {
                started: reachability.captures_started(),
                published: reachability.published_capture(),
                recoveries: reachability.recoveries(),
            })
        }
    }
}

/// A process-exit verdict waiting for its host to answer: the host's capture marks when the
/// process was read dead. It applies once a capture begun after that read published, with no
/// outage in between.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    decision: LivenessDecision,
    host: crate::ports::HostId,
    read_at: CaptureMarks,
}

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
    /// The publication gate; unset, every verdict applies at once (unit fixtures with no host).
    hosts: OnceLock<Arc<dyn LivenessHosts>>,
    pending: Mutex<HashMap<crate::protocol::ids::BindingId, Pending>>,
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
            hosts: OnceLock::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Installs the publication gate (once; the daemon does it when its host registry exists).
    pub fn set_hosts(&self, hosts: Arc<dyn LivenessHosts>) {
        let _ = self.hosts.set(hosts);
    }

    /// The verdicts to apply now; process-exit verdicts wait for their host (module doc). A full
    /// pass forgets pending verdicts it no longer reaches.
    fn gate(
        &self,
        candidates: &[LivenessCandidate],
        decisions: Vec<LivenessDecision>,
        full_pass: bool,
    ) -> Result<Vec<LivenessDecision>, ApiError> {
        let Some(hosts) = self.hosts.get() else {
            return Ok(decisions);
        };
        let host_of: HashMap<_, _> = candidates
            .iter()
            .map(|c| (&c.binding.binding_id, c.host))
            .collect();
        let mut pending = self.pending.lock().map_err(|_| {
            ApiError::new(
                crate::protocol::results::ErrorCode::StoreCorrupt,
                "liveness pending verdicts poisoned",
            )
        })?;
        let mut apply = Vec::new();
        let mut reached = Vec::new();
        for decision in decisions {
            if decision.reason != DisconnectReason::ProcessExit {
                apply.push(decision);
                continue;
            }
            let Some(&host) = host_of.get(&decision.binding) else {
                continue;
            };
            reached.push(decision.binding.clone());
            match hosts.liveness(host) {
                HostLiveness::Open => {
                    pending.remove(&decision.binding);
                    apply.push(decision);
                }
                HostLiveness::Closed => {
                    pending.remove(&decision.binding);
                }
                HostLiveness::Published(now) => match pending.get(&decision.binding) {
                    Some(seen)
                        if seen.decision == decision
                            && seen.host == host
                            && seen.read_at.recoveries == now.recoveries =>
                    {
                        if now.published > seen.read_at.started {
                            pending.remove(&decision.binding);
                            apply.push(decision);
                        }
                    }
                    _ => {
                        pending.insert(
                            decision.binding.clone(),
                            Pending {
                                decision,
                                host,
                                read_at: now,
                            },
                        );
                    }
                },
            }
        }
        if full_pass {
            pending.retain(|binding, _| reached.contains(binding));
        }
        Ok(apply)
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
        let decisions = self.gate(&candidates, decisions, seat.is_none())?;
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
