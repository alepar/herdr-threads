//! Cancellation-owned channel lifecycle lane. All host/filesystem I/O is outside writer ownership.
use crate::{
    archival_legacy::Source,
    ports::{HostId, HostPort, StorePort},
    protocol::{
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, MonoInstant},
    },
    service::{
        fair_writer::FairWriter,
        host_reachability::HostReachability,
        host_sessions::HostSessions,
        kicks::{self, Lane},
        pacer::{Pacer, Wake},
        workers::WorkerStatus,
    },
    store::archival::{HostArchivalEvidence, Runtime},
};
use std::{sync::Arc, time::Duration};

/// One attached Herdr server as the archival lane sees it: the port it samples
/// through and the reachability that says whether its evidence is coherent.
#[derive(Clone)]
pub struct ArchivalHost {
    pub host_id: HostId,
    pub port: Arc<dyn HostPort>,
    pub reachability: Arc<HostReachability>,
}
/// The hosts the archival lane may sample and take coherence evidence from.
/// Production is `HostSessions`; `attached` is in `HostId` order so the lane
/// always takes the hosts' evidence locks in one order.
pub trait ArchivalHosts: Send + Sync {
    fn attached(&self) -> Vec<ArchivalHost>;
    fn get(&self, id: HostId) -> Option<ArchivalHost>;
}
fn archival_host(session: &crate::service::host_sessions::HostSession) -> ArchivalHost {
    ArchivalHost {
        host_id: session.host_id(),
        port: Arc::clone(session.port()),
        reachability: Arc::clone(session.reachability()),
    }
}
impl ArchivalHosts for HostSessions {
    fn attached(&self) -> Vec<ArchivalHost> {
        HostSessions::attached(self)
            .iter()
            .map(|session| archival_host(session))
            .collect()
    }
    fn get(&self, id: HostId) -> Option<ArchivalHost> {
        HostSessions::get(self, id).map(|session| archival_host(&session))
    }
}
/// A fixed host set (tests, and a daemon composed with one fixed host).
pub struct StaticHosts(pub Vec<ArchivalHost>);
impl ArchivalHosts for StaticHosts {
    fn attached(&self) -> Vec<ArchivalHost> {
        self.0.clone()
    }
    fn get(&self, id: HostId) -> Option<ArchivalHost> {
        self.0.iter().find(|h| h.host_id == id).cloned()
    }
}

type EvidenceWork<'a, R> = Box<dyn FnOnce(&[HostArchivalEvidence]) -> R + 'a>;

/// Runs `work` with every host's evidence lock held (in `hosts` order), so no
/// reachability mark can interleave with the deciding store transaction.
fn with_host_evidence<'a, R>(
    hosts: &[ArchivalHost],
    now: u64,
    evidence: &mut Vec<HostArchivalEvidence>,
    work: EvidenceWork<'a, R>,
) -> R {
    let Some((host, rest)) = hosts.split_first() else {
        return work(evidence);
    };
    host.reachability.with_archival_state(now, |state| {
        evidence.push(HostArchivalEvidence {
            host_id: host.host_id,
            generation: i64::try_from(state.generation).unwrap_or(i64::MAX),
            coherent: state.coherent,
            valid_until_mono: state
                .published_at
                .and_then(|at| i64::try_from(at).ok())
                .map(|at| at.saturating_add(crate::store::archival::MAX_GAP_MS)),
        });
        with_host_evidence(rest, now, evidence, work)
    })
}

/// The node-wide archival lane. Its evidence and sampling are per host
/// (`HostArchivalEvidence`); the decision stays node-wide.
pub struct ArchivalWorker {
    pub store: Arc<dyn StorePort>,
    pub hosts: Arc<dyn ArchivalHosts>,
    pub writer: Arc<FairWriter>,
    pub source: Source,
    pub boot: String,
    pub after_ms: u64,
    pub cancellation: Cancellation,
}
impl ArchivalWorker {
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.store.clock().monotonic_now().0.saturating_add(5_000)),
            cancellation: self.cancellation.clone(),
        }
    }
    /// Every attached host's archival evidence at `now`.
    fn evidence(&self) -> Vec<HostArchivalEvidence> {
        let now = self.store.clock().monotonic_now().0;
        let hosts = self.hosts.attached();
        let mut evidence = Vec::with_capacity(hosts.len());
        with_host_evidence(&hosts, now, &mut evidence, Box::new(|_| ()));
        evidence
    }
    fn deciding<R>(
        &self,
        budget: &CallBudget,
        source: Option<String>,
        work: impl FnOnce(&Runtime) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        let now = self.store.clock().monotonic_now().0;
        let hosts = self.hosts.attached();
        let after_ms = i64::try_from(self.after_ms)
            .map_err(|_| ApiError::new(ErrorCode::InvalidRequest, "archive grace range"))?;
        let mut evidence = Vec::with_capacity(hosts.len());
        with_host_evidence(
            &hosts,
            now,
            &mut evidence,
            Box::new(|hosts: &[HostArchivalEvidence]| {
                let rt = Runtime {
                    boot: self.boot.clone(),
                    mono: 0,
                    utc: self.store.clock().utc_now(),
                    after_ms,
                    hosts: hosts.to_vec(),
                    legacy_source: source,
                };
                work(&rt)
            }),
        )
    }
    pub fn run_page(&mut self) -> Result<bool, ApiError> {
        if self.cancellation.is_cancelled() {
            return Err(ApiError::new(
                ErrorCode::Cancelled,
                "archival lane cancelled",
            ));
        }
        if self.after_ms == 0 {
            return Ok(false);
        }
        let scan = self.source.scan(|| self.cancellation.is_cancelled());
        let scan_error = scan.as_ref().err().map(|e| {
            ApiError::new(
                if e.kind() == std::io::ErrorKind::Interrupted {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::StoreCorrupt
                },
                "legacy journal coverage unavailable",
            )
        });
        let scan = scan.unwrap_or_default();
        let result = (|| {
            // Source validation is filesystem I/O and deliberately precedes writer
            // admission. New protocol compounds are protected canonically before effects.
            let coverage = self.source.filter_coverage(scan.coverage);
            let budget = self.budget();
            let progress = self.deciding(&budget, coverage.clone(), |rt| {
                self.store.archival_pass(rt, &scan.hints, &budget)
            })?;
            if let Some(error) = scan_error {
                return Err(error);
            }
            let mut has_more = scan.pending || progress.has_more;
            let before = self.evidence();
            if !before.iter().any(|h| h.coherent) {
                return Ok(has_more);
            }
            let coverage = self.source.filter_coverage(coverage);
            let work = self.deciding(&budget, coverage.clone(), |rt| {
                self.store.archival_next(rt, &budget)
            })?;
            has_more |= work.has_more;
            if let Some(ticket) = work.ticket {
                let host = self.hosts.get(ticket.host_id());
                let context = ticket.host_context(budget.clone());
                let sample = match &host {
                    Some(host) => host
                        .port
                        .observe_current_target_for_archival(ticket.target(), &context),
                    None => Err(ApiError::new(
                        ErrorCode::HostUnavailable,
                        "no session for the seat's Herdr server",
                    )),
                };
                // A host failure breaks that host's continuity only. A successful
                // nonidle observation vetoes only that seat's channels in the
                // deciding write. This lane is single-owner: no subsequent archive
                // decision can run until that write succeeds, or the failure below
                // installs uncertainty. A host with no session has no reachability
                // to mark and already counts as incoherent.
                let mark_uncertain = || {
                    if let Some(host) = &host {
                        host.reachability.mark_archival_uncertain();
                    }
                };
                if sample.is_err() {
                    mark_uncertain();
                }
                let before_generation = before
                    .iter()
                    .find(|h| h.host_id == ticket.host_id())
                    .map(|h| h.generation);
                let coverage = self.source.filter_coverage(coverage);
                let persisted = self.deciding(&budget, coverage, |rt| {
                    let mut rt = rt.clone();
                    // A transient outage/recovery of the ticket's host entirely inside
                    // the host call also breaks its continuity, even if its current
                    // reachability is back up.
                    for host in &mut rt.hosts {
                        if host.host_id == ticket.host_id() {
                            host.coherent &= Some(host.generation) == before_generation;
                        }
                    }
                    self.store
                        .archival_sample(&rt, &ticket, sample.as_ref().ok(), &budget)
                });
                if persisted.is_err() {
                    mark_uncertain();
                }
                persisted?;
                sample?;
            }
            Ok(has_more)
        })();
        // The source cursor advanced before outer writer admission. Every error
        // after consumption must keep that traversal uncertain, including errors
        // that never call the store. Preserve the original error and cursor.
        if result.is_err() {
            self.source.veto_traversal();
        }
        result
    }
}
/// How long the lane waits after a commit kick before its next pass.
pub const KICK_SETTLE: Duration = Duration::from_millis(250);

pub fn start(
    mut worker: ArchivalWorker,
    pacer: Arc<Pacer>,
    status: Arc<WorkerStatus>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    status.attach_pacer(pacer.clone());
    std::thread::Builder::new()
        .name("herdr-archival".into())
        .spawn(move || {
            let _origin = kicks::enter_lane(Lane::Archival);
            let _guard = status.lane_guard(&worker.cancellation);
            while !worker.cancellation.is_cancelled() {
                let more = match worker.run_page() {
                    Ok(more) => {
                        pacer.on_success();
                        status.record_success(worker.store.clock().utc_now());
                        more
                    }
                    Err(_) if worker.cancellation.is_cancelled() => break,
                    Err(error) => {
                        pacer.on_failure();
                        status.record_failure(Lane::Archival, &error);
                        false
                    }
                };
                if more {
                    continue;
                }
                match pacer.wait_blocking(Duration::from_millis(
                    crate::store::archival::CADENCE_MS as u64,
                )) {
                    Wake::Cancelled => break,
                    // A send's publication, materialization and wake
                    // reservation each kick this lane. Archival is never
                    // urgent and its decisions recheck the canonical revision,
                    // so let the burst settle instead of taking writer turns
                    // (and fsyncs) between those commits. Kicks during the
                    // wait stay latched in the Pacer.
                    Wake::Kicked => {
                        if worker.cancellation.wait_blocking(KICK_SETTLE) {
                            break;
                        }
                    }
                    Wake::Tick | Wake::RetryDue => {}
                }
            }
        })
}
