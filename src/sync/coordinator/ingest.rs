//! The coordinator's ingest half: received `entries` frames, the one
//! sequential ingest worker and the handling of every [`IngestOutcome`]
//! (peer-link design, sections 9 and 10).
//!
//! Responsibility (ingest)
//! - Owns: the unsolicited-`entries` rule (dropped, credit granted, counted),
//!   the FIFO of batches headed for the worker, the worker task (one batch at
//!   a time, so per-log order holds across a source switch) and the one
//!   exhaustive `match` on [`IngestOutcome`] with the penalty, pause and
//!   retry bookkeeping each outcome needs.
//! - Hides: the `spawn_blocking`, `Lane::Sync` origin and `FairWriter`
//!   background turn around `JournalPort::ingest`.
//! - Must not: decide fork policy (that is a [`ForkSink`]'s), touch SQLite, or
//!   keep a batch's credit: the frame's bytes go back to the sender as soon
//!   as its outcome is known, whatever it is.
//! - Role: the data-moving half of the receive side; routing (`reselect`)
//!   stays the actor's.
//!
//! Core and harness-neutral.

use super::*;
use crate::service::kicks::{Lane, enter_lane};

/// How long a source that served corrupt bytes is unwantable for that log (ms).
pub(super) const CORRUPT_PENALTY_MS: u64 = 600_000;
/// How long every want is paused after `Headroom` (ms).
pub(super) const HEADROOM_PAUSE_MS: u64 = 60_000;
/// How long the ingest worker waits for its `FairWriter` background turn (ms).
const INGEST_TURN_MS: u64 = 5_000;

/// A batch for the worker.
pub(super) struct IngestJob {
    pub link: u64,
    /// The node on the other end of `link`, for fork reports.
    pub source: NodeUuid,
    pub log: LogKey,
    pub entries: Vec<Envelope>,
    /// The frame's wire length: the credit to return once the batch is done.
    pub len: u32,
}

/// A worker result, joined by the actor.
pub(super) struct IngestDone {
    pub link: u64,
    pub source: NodeUuid,
    pub log: LogKey,
    pub len: u32,
    pub result: Result<IngestOutcome, ApiError>,
}

/// What the worker needs; none of it is actor state.
pub(super) struct WorkerDeps {
    pub journal: Arc<dyn JournalPort>,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    pub cancel: Cancellation,
}

/// Start the worker task. It ends when `jobs` closes, on cancellation or when
/// the actor stops listening.
pub(super) fn spawn_worker(
    deps: WorkerDeps,
    mut jobs: mpsc::UnboundedReceiver<IngestJob>,
    done: mpsc::Sender<IngestDone>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let job = tokio::select! {
                job = jobs.recv() => job,
                () = deps.cancel.cancelled() => None,
            };
            let Some(IngestJob {
                link,
                source,
                log,
                entries,
                len,
            }) = job
            else {
                break;
            };
            let journal = deps.journal.clone();
            let writer = deps.writer.clone();
            let clock = deps.clock.clone();
            let cancel = deps.cancel.clone();
            let result = tokio::task::spawn_blocking(move || {
                ingest_batch(
                    journal.as_ref(),
                    &writer,
                    clock.as_ref(),
                    cancel,
                    &log,
                    entries,
                )
            })
            .await
            .unwrap_or_else(|e| {
                Err(ApiError::new(
                    ErrorCode::StoreBusy,
                    format!("ingest task failed: {e}"),
                ))
            });
            let finished = IngestDone {
                link,
                source,
                log,
                len,
                result,
            };
            if done.send(finished).await.is_err() {
                break;
            }
        }
    })
}

/// One batch on a blocking thread: `Lane::Sync` origin, a `FairWriter`
/// background turn, then `JournalPort::ingest` under the turn.
fn ingest_batch(
    journal: &dyn JournalPort,
    writer: &FairWriter,
    clock: &dyn Clock,
    cancel: Cancellation,
    log: &LogKey,
    entries: Vec<Envelope>,
) -> Result<IngestOutcome, ApiError> {
    let _origin = enter_lane(Lane::Sync);
    let turn_budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(INGEST_TURN_MS)),
        cancellation: cancel.clone(),
    };
    let _turn = writer.enter_background(&turn_budget, clock)?;
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: cancel,
    };
    journal.ingest(log, entries, &budget)
}

impl Actor {
    /// An `entries` frame arrived on `link_id`. Not from the current rx source
    /// of the log: dropped, its credit granted and counted. Otherwise decoded
    /// (a shape error is `Corrupt` for the log) and queued for the worker.
    pub(super) fn on_entries(
        &mut self,
        link_id: u64,
        node: NodeUuid,
        inc: u32,
        entries: Vec<WireEnvelope>,
        wire_len: usize,
    ) {
        let log = LogKey {
            node,
            incarnation: inc,
        };
        let len = u32::try_from(wire_len).unwrap_or(u32::MAX);
        let routed = self.rx.get(&log).is_some_and(|src| src.link == link_id)
            && self
                .links
                .get(&link_id)
                .is_some_and(|l| l.mode == LinkMode::Active && !l.closing);
        if !routed {
            self.grant(link_id, len);
            if let Some(entry) = self.links.get_mut(&link_id) {
                entry.unsolicited += 1;
            }
            return;
        }
        let decoded: Result<Vec<Envelope>, _> =
            entries.into_iter().map(Envelope::try_from).collect();
        let Ok(decoded) = decoded else {
            self.grant(link_id, len);
            self.on_corrupt(link_id, log, "malformed entry");
            return;
        };
        if decoded.is_empty() {
            self.grant(link_id, len);
            return;
        }
        let source = self
            .links
            .get(&link_id)
            .map_or(node, |l| l.handle.peer().node);
        let job = IngestJob {
            link: link_id,
            source,
            log,
            entries: decoded,
            len,
        };
        if self.ingest_jobs.send(job).is_err() {
            self.grant(link_id, len);
        }
    }

    /// Return `len` bytes of credit to the sender on `link_id`.
    pub(super) fn grant(&self, link_id: u64, len: u32) {
        if let Some(entry) = self.links.get(&link_id) {
            let _ = entry.handle.grant(len);
        }
    }

    /// A worker result: the credit goes back first, then the outcome is acted
    /// on.
    pub(super) fn on_ingested(&mut self, done: IngestDone) {
        let IngestDone {
            link,
            source,
            log,
            len,
            result,
        } = done;
        self.grant(link, len);
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(e) => {
                self.on_ingest_error(log, &e);
                return;
            }
        };
        // New outcomes are handled here; fork policy goes in a `ForkSink`
        // implementation, never in this match.
        match outcome {
            IngestOutcome::Stored { head, .. } => self.on_stored(link, log, head),
            IngestOutcome::Gap { expected, .. } => self.on_gap(link, log, expected),
            IngestOutcome::Fork { evidence, .. } => {
                match self.deps.fork_sink.on_fork(log, evidence, source) {
                    ForkAck::Paused => self.pause_log(log),
                    ForkAck::Corrupt => self.on_corrupt(link, log, "fork evidence not proven"),
                }
            }
            IngestOutcome::Frozen { .. } => {
                // The log froze: stop pulling past it. The refreshed local
                // vector says how far `wantable` still allows.
                self.drop_rx(log);
                self.request_local();
            }
            IngestOutcome::Corrupt {
                counter, reason, ..
            } => {
                self.on_corrupt(link, log, &format!("counter {counter}: {reason:?}"));
            }
            IngestOutcome::Headroom => self.on_headroom(),
        }
    }

    /// `Unwant` the log on its rx source and forget the source.
    fn drop_rx(&mut self, log: LogKey) {
        if let Some(src) = self.rx.remove(&log) {
            self.send_unwant(src.link, &log);
        }
    }

    fn on_stored(&mut self, link_id: u64, log: LogKey, head: LogHead) {
        let now = self.mono();
        let counter = head.counter;
        let newer = self.local.0.get(&log).is_none_or(|h| h.counter <= counter);
        if newer {
            self.local.0.insert(log, head);
        }
        if let Some(src) = self.rx.get_mut(&log)
            && src.link == link_id
        {
            src.last_progress = now;
        }
        self.retry_logs.remove(&log);
        if self.retry_logs.is_empty() && self.deps.pacer.attempts() > 0 {
            self.deps.pacer.on_success();
        }
        self.deps.status.record_success(self.deps.clock.utc_now());
        // Relay to the other subscribers of the log.
        let subscribers: Vec<u64> = self
            .links
            .iter()
            .filter(|(id, l)| **id != link_id && l.subs_out.contains_key(&log))
            .map(|(id, _)| *id)
            .collect();
        for id in subscribers {
            self.push_link(id);
        }
        self.reselect_log(log);
        self.update_own_history();
        self.flush_have_deltas(now);
    }

    /// The batch skipped ahead: ask the same source again from `expected`,
    /// unless that is exactly what was last asked (a stale batch of an
    /// earlier subscription).
    fn on_gap(&mut self, link_id: u64, log: LogKey, expected: u64) {
        let Some(src) = self.rx.get_mut(&log) else {
            return;
        };
        if src.link != link_id || src.requested_from == expected {
            return;
        }
        src.requested_from = expected;
        src.last_progress = self.deps.clock.monotonic_now();
        if let Some(entry) = self.links.get(&link_id) {
            let _ = entry.handle.send(PeerFrame::Want {
                node: log.node,
                inc: log.incarnation,
                from: expected,
            });
        }
    }

    /// Penalize `(link, log)` for [`CORRUPT_PENALTY_MS`], stop pulling the log
    /// from it and pick another source. With none, the same source is
    /// re-wanted when the penalty expires.
    pub(super) fn on_corrupt(&mut self, link_id: u64, log: LogKey, why: &str) {
        let now = self.mono();
        (self.deps.log)(&format!(
            "sync: corrupt {}/{} from link {link_id} ({why}); source penalized",
            uuid8(&log.node),
            log.incarnation
        ));
        if let Some(entry) = self.links.get_mut(&link_id) {
            entry
                .errors
                .insert(log, MonoInstant(now.0 + CORRUPT_PENALTY_MS));
        }
        if self.rx.get(&log).is_some_and(|src| src.link == link_id) {
            self.drop_rx(log);
        }
        self.reselect_log(log);
    }

    /// The sink holds the fork: no `Want` for the log until it reports the
    /// log resumed.
    fn pause_log(&mut self, log: LogKey) {
        self.paused.insert(log);
        self.drop_rx(log);
    }

    /// No disk headroom: `Unwant` every rx subscription and pause all wants.
    /// The housekeeping tick re-picks when the pause is over.
    fn on_headroom(&mut self) {
        let now = self.mono();
        self.headroom_until = Some(MonoInstant(now.0 + HEADROOM_PAUSE_MS));
        let sources = std::mem::take(&mut self.rx);
        for (log, src) in sources {
            self.send_unwant(src.link, &log);
        }
    }

    /// A store error: report it on the sync lane and ask again after the
    /// pacer's backoff (see [`Actor::on_retry_due`]).
    fn on_ingest_error(&mut self, log: LogKey, error: &ApiError) {
        self.deps.status.record_failure(Lane::Sync, error);
        self.retry_logs.insert(log);
        if self.deps.pacer.attempts() == 0 {
            self.deps.pacer.on_failure();
        }
    }

    /// The pacer's backoff is due: ask again for every log whose batch failed
    /// to store. While any remain, the next step is scheduled at once (an
    /// ingest `Stored` clears it), so a due backoff never spins.
    pub(super) fn on_retry_due(&mut self) {
        let logs = std::mem::take(&mut self.retry_logs);
        if logs.is_empty() {
            self.deps.pacer.on_success();
            return;
        }
        for log in logs {
            match self.rx.get(&log).map(|src| src.link) {
                Some(link) => self.start_rx(log, link),
                None => self.reselect_log(log),
            }
        }
        self.deps.pacer.on_failure();
    }

    /// Resume the logs the fork sink reports handled, and end an expired
    /// headroom pause.
    pub(super) fn lift_pauses(&mut self, now: MonoInstant) {
        let mut lifted = false;
        for log in self.deps.fork_sink.resumed() {
            if self.paused.remove(&log) {
                lifted = true;
                self.reselect_log(log);
            }
        }
        if lifted {
            self.request_local();
        }
        if self.headroom_until.is_some_and(|until| until.0 <= now.0) {
            self.headroom_until = None;
            self.reselect_all();
        }
    }
}
