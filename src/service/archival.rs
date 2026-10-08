//! Cancellation-owned channel lifecycle lane. All host/filesystem I/O is outside writer ownership.
use crate::{
    archival_legacy::Source,
    ports::{HostPort, StorePort},
    protocol::{
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, MonoInstant},
    },
    service::{
        fair_writer::FairWriter,
        host_reachability::HostReachability,
        kicks::{self, Lane},
        pacer::{Pacer, Wake},
        workers::WorkerStatus,
    },
    store::archival::Runtime,
};
use std::{sync::Arc, time::Duration};
pub struct ArchivalWorker {
    pub store: Arc<dyn StorePort>,
    pub host: Arc<dyn HostPort>,
    pub writer: Arc<FairWriter>,
    pub reachability: Arc<HostReachability>,
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
    fn deciding<R>(
        &self,
        budget: &CallBudget,
        source: Option<String>,
        work: impl FnOnce(&Runtime) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.reachability
            .with_archival_state(self.store.clock().monotonic_now().0, |state| {
                let rt = Runtime {
                    boot: self.boot.clone(),
                    mono: 0,
                    utc: self.store.clock().utc_now(),
                    after_ms: i64::try_from(self.after_ms).map_err(|_| {
                        ApiError::new(ErrorCode::InvalidRequest, "archive grace range")
                    })?,
                    host_generation: i64::try_from(state.generation).map_err(|_| {
                        ApiError::new(ErrorCode::InvalidRequest, "host generation range")
                    })?,
                    coherent: state.coherent,
                    valid_until_mono: state
                        .published_at
                        .and_then(|at| i64::try_from(at).ok())
                        .map(|at| at.saturating_add(crate::store::archival::MAX_GAP_MS)),
                    legacy_source: source,
                };
                work(&rt)
            })
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
            let state = self
                .reachability
                .archival_state(self.store.clock().monotonic_now().0);
            if !state.coherent {
                return Ok(has_more);
            }
            let coverage = self.source.filter_coverage(coverage);
            let work = self.deciding(&budget, coverage.clone(), |rt| {
                self.store.archival_next(rt, &budget)
            })?;
            has_more |= work.has_more;
            if let Some(ticket) = work.ticket {
                let context = ticket.host_context(budget.clone());
                let sample = self
                    .host
                    .observe_current_target_for_archival(ticket.target(), &context);
                // A host failure breaks global continuity. A successful nonidle
                // observation vetoes only that seat's channels in the deciding write.
                // This lane is single-owner: no subsequent archive decision can run
                // until that write succeeds, or the failure below installs uncertainty.
                if sample.is_err() {
                    self.reachability.mark_archival_uncertain();
                }
                let coverage = self.source.filter_coverage(coverage);
                let persisted = self.deciding(&budget, coverage, |rt| {
                    let mut rt = rt.clone();
                    // A transient outage/recovery entirely inside the host call also
                    // breaks continuity, even if the current reachability is back up.
                    rt.coherent &= rt.host_generation as u64 == state.generation;
                    self.store
                        .archival_sample(&rt, &ticket, sample.as_ref().ok(), &budget)
                });
                if persisted.is_err() {
                    self.reachability.mark_archival_uncertain();
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
                if !more
                    && pacer.wait_blocking(Duration::from_millis(
                        crate::store::archival::CADENCE_MS as u64,
                    )) == Wake::Cancelled
                {
                    break;
                }
            }
        })
}
