//! The incarnations worker: the `herdr-incarnations` thread that turns sink
//! jobs into writer quanta.
//!
//! Responsibility (IncarnationsWorker)
//! - Owns: the queue policy over the sink's [`Job`] queue (per-source `Have`
//!   coalescing, the 8 MiB cap on queued branch-entry bytes, the deferred
//!   `Restore` gate), the 1 s timer that sweeps the request table and sends
//!   due pulls, the re-issue driver, the clone gate's reactions, the
//!   own-history `Branches` flag and the failure path (`Lane::Sync` status
//!   plus a 1 s to 60 s backoff).
//! - Hides: which store function a trigger needs and when; every job is one
//!   `FairWriter` background turn (5 s) around one `store::incarnations`
//!   function, run through `execute_budgeted_decision`. A log's public key
//!   and ownership come from [`JournalPort::log_meta`] and
//!   [`JournalPort::stored_hash`].
//! - Must not: write SQL (it names no table, connection or transaction; the
//!   store functions are the only writers), send frames (the coordinator
//!   drains the sink's outbox) or block the sink's front. A
//!   [`Job::Salvage`] runs the resync salvage reader
//!   (`sync::resync::run_salvage`), one ingest batch per writer turn.
//! - Relationships: reads jobs and shared state from [`IncarnationsSink`];
//!   composition (`app.rs`, ht-5fr.6.15) builds [`WorkerDeps`], starts the
//!   thread before the coordinator and cancels it after. The operator's
//!   `sync clone-cleared` path (ht-5fr.8) commits `clear_clone_evidence` and
//!   then calls [`IncarnationsSink::notify_clone_cleared`].
//! - Lifecycle: all in-memory state starts empty; at start the worker
//!   re-derives its pulls and re-issues from the store rows.
//! - Errors: a store error records `Lane::Sync` health, keeps the job and
//!   retries it after its own backoff (1 s doubling to 60 s; other jobs keep
//!   running); a job that can never succeed
//!   (`InvalidRequest`, `NotFound`, `Conflict`) is recorded and dropped.
//!
//! The timer is a 1 s tokio interval timer on a current-thread runtime
//! inside the thread, so tests run it on paused time.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::requests::BranchKey;
use super::sink::{IncarnationsSink, Job};
use super::{Classified, Proof, classify_pair};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::fair_writer::{FairWriter, WriterGuard};
use crate::service::kicks::{Lane, LaneOrigin, enter_lane};
use crate::service::pacer::Pacer;
use crate::service::workers::WorkerStatus;
use crate::store::SqliteStore;
use crate::store::connection::{HoldSource, OwnHistoryHold};
use crate::store::incarnations::{self as store, BranchVerdict, ReissueLimits, Repair, Rotation};
use crate::sync::coordinator::{BranchReply, LogFn, uuid8};
use crate::sync::envelope::{Envelope, WireEnvelope, hex_decode, hex_encode};
use crate::sync::frame::{BranchWant, ForkRef, HaveLog, TipRef};
use crate::sync::journal_port::{JournalPort, LogMeta};
use crate::sync::signals::SyncSignals;
use crate::sync::{BatchLimits, LogKey, LogState};

/// Queued branch-entry bytes allowed after `Have` coalescing.
const CAP_BYTES: usize = 8 << 20;
/// The writer-turn budget of one job.
const QUANTUM_MS: u64 = 5_000;
const BACKOFF_BASE_MS: u64 = 1_000;
const BACKOFF_MAX_MS: u64 = 60_000;
/// Re-issue passes repeat this often while candidates wait on a predecessor.
const REISSUE_WAITING_MS: u64 = 5_000;
/// A `Restore` runs after this long even if an advertised own genesis is
/// still missing (the own-history hold's cap).
const RESTORE_WAIT_MS: u64 = 300_000;
const OVERFLOW_LOG_EVERY_MS: u64 = 60_000;
const HELD_LOG_EVERY_MS: u64 = 3_600_000;
/// Most outstanding single-entry probes remembered.
const MAX_PROBES: usize = 1024;
/// Most fork references remembered as handled.
const MAX_SEEN_FORKS: usize = 4096;
/// Most own logs remembered per advertising source, and sources.
const MAX_ADVERTISED_LOGS: usize = 64;
const MAX_ADVERTISED_SOURCES: usize = 256;
/// Most peer heads of the active self log remembered.
const MAX_ACTIVE_HEADS: usize = 64;

/// Most stopped logs a salvage summary line names.
const SALVAGE_STOPS_LOGGED: usize = 4;

/// The log line for a repair dropped by the clone gate (one per hour).
pub const HELD_LINE: &str = "sync: repair held: clone evidence for this node; see sync status";

/// What the worker thread is built from.
pub struct WorkerDeps {
    pub sink: Arc<IncarnationsSink>,
    pub journal: Arc<dyn JournalPort>,
    pub store: Arc<SqliteStore>,
    pub writer: Arc<FairWriter>,
    pub status: Arc<WorkerStatus>,
    pub own_history: Arc<OwnHistoryHold>,
    /// `SyncHandle::signals()`: the worker asks for a round after a clone
    /// clearance.
    pub signals: Arc<SyncSignals>,
    /// The `Lane::Sync` pacer. The worker commits under a `Lane::Sync` origin,
    /// which keeps a commit from kicking its own lane, so after each quantum it
    /// kicks the coordinator itself: a rotation or a re-issued entry is a local
    /// vector change that peers must hear about. `None` where no coordinator
    /// runs (tests of the worker alone).
    pub sync_pacer: Option<Arc<Pacer>>,
    pub clock: Arc<dyn Clock>,
    pub log: LogFn,
    pub cancel: Cancellation,
}

/// The `herdr-incarnations` thread. See the module documentation.
pub struct IncarnationsWorker;

impl IncarnationsWorker {
    /// Spawns the thread. It ends when `deps.cancel` is cancelled.
    pub fn start(deps: WorkerDeps) -> std::io::Result<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("herdr-incarnations".into())
            .spawn(move || {
                let _alive = deps.status.lane_guard(&deps.cancel);
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build();
                let runtime = match runtime {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        (deps.log)(&format!("sync: incarnations worker runtime failed: {e}"));
                        return;
                    }
                };
                let _origin = enter_lane(Lane::Sync);
                runtime.block_on(run(Core::new(deps)));
            })
    }
}

async fn run(mut core: Core) {
    let cancel = core.d.cancel.clone();
    let sink = Arc::clone(&core.d.sink);
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    core.start();
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            _ = timer.tick() => core.tick(),
            () = sink.jobs_ready() => core.work(),
        }
    }
}

/// A job in the worker's pending list.
struct Queued {
    job: Job,
    queued_at: MonoInstant,
    /// Branch-entry bytes it counts against the cap.
    bytes: usize,
    /// The job's own backoff: it does not run before this.
    retry_at: Option<MonoInstant>,
    /// The next backoff delay of this job.
    delay_ms: u64,
}

/// A single-entry (or restore-entry) `branch_want` the worker sent, and what
/// its reply is for. Replies for a branch with no probe are pull replies.
#[derive(Clone)]
enum Probe {
    /// A peer's head differs from the stored hash at its counter.
    Mismatch,
    /// One side of a forwarded [`ForkRef`]; `got` holds the sides fetched so
    /// far.
    ForkSide { fork: ForkRef, got: Vec<Envelope> },
    /// The peer's entry that extends this node's active head.
    RestoreEntry,
}

struct Core {
    d: WorkerDeps,
    pending: VecDeque<Queued>,
    pending_bytes: usize,
    probes: BTreeMap<BranchKey, Probe>,
    seen_forks: BTreeSet<(LogKey, [u8; 32], [u8; 32])>,
    /// Own logs each source advertised.
    advertised_own: BTreeMap<NodeUuid, BTreeSet<LogKey>>,
    /// Advertised own logs whose genesis has not been seen ingested yet.
    watch_own: BTreeSet<LogKey>,
    /// Peers' heads of the active self log beyond this node's head.
    active_heads: BTreeMap<(NodeUuid, LogKey), (u64, [u8; 32])>,
    reissue_due: bool,
    reissue_waiting: usize,
    reissue_next_at: u64,
    evidence_due: bool,
    restore_derive_due: bool,
    clone_held: bool,
    /// Logs whose repair job the clone gate dropped (resumed on clearance).
    held_logs: BTreeSet<LogKey>,
    held_logged: Option<MonoInstant>,
    overflow_logged: Option<MonoInstant>,
    failing: bool,
    delay_ms: u64,
    retry_at: Option<MonoInstant>,
    own: Option<NodeUuid>,
}

/// Runs `$body` (a closure `|tx, ctx, decision|` calling one
/// `store::incarnations` function) as one writer quantum: a `Lane::Sync`
/// origin, a `FairWriter` background turn and one budgeted decision. Written
/// as a macro so this file names no transaction type.
macro_rules! quantum {
    ($self:expr, $body:expr) => {{
        let (_origin, _turn, budget) = $self.turn()?;
        let done = $self.d.store.incarnations_quantum(&budget, $body)?;
        $self.kick_sync();
        done
    }};
}

impl Core {
    fn new(d: WorkerDeps) -> Self {
        Self {
            d,
            pending: VecDeque::new(),
            pending_bytes: 0,
            probes: BTreeMap::new(),
            seen_forks: BTreeSet::new(),
            advertised_own: BTreeMap::new(),
            watch_own: BTreeSet::new(),
            active_heads: BTreeMap::new(),
            reissue_due: false,
            reissue_waiting: 0,
            reissue_next_at: 0,
            evidence_due: false,
            restore_derive_due: false,
            clone_held: false,
            held_logs: BTreeSet::new(),
            held_logged: None,
            overflow_logged: None,
            failing: false,
            delay_ms: BACKOFF_BASE_MS,
            retry_at: None,
            own: None,
        }
    }

    fn now(&self) -> MonoInstant {
        self.d.clock.monotonic_now()
    }

    fn utc(&self) -> UtcMillis {
        self.d.clock.utc_now()
    }

    fn log(&self, line: &str) {
        (self.d.log)(line);
    }

    fn read_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: self.d.cancel.clone(),
        }
    }

    fn own_node(&mut self) -> Result<NodeUuid, ApiError> {
        if let Some(node) = self.own {
            return Ok(node);
        }
        let node = self.d.store.context().sync()?.identity.uuid();
        self.own = Some(node);
        Ok(node)
    }

    /// Tells the coordinator the journal may have moved (see
    /// [`WorkerDeps::sync_pacer`]); a kick on a quantum that wrote nothing
    /// costs one vector read.
    fn kick_sync(&self) {
        if let Some(pacer) = &self.d.sync_pacer {
            pacer.kick();
        }
    }

    /// The lane origin, a writer turn and the budget of one quantum.
    fn turn(&self) -> Result<(LaneOrigin, WriterGuard<'_>, CallBudget), ApiError> {
        let origin = enter_lane(Lane::Sync);
        let turn_budget = CallBudget {
            deadline: MonoInstant(self.now().0.saturating_add(QUANTUM_MS)),
            cancellation: self.d.cancel.clone(),
        };
        let turn = self
            .d
            .writer
            .enter_background(&turn_budget, self.d.clock.as_ref())?;
        let budget = CallBudget {
            deadline: MonoInstant(self.now().0.saturating_add(QUANTUM_MS)),
            cancellation: self.d.cancel.clone(),
        };
        Ok((origin, turn, budget))
    }

    fn meta(&self, log: &LogKey) -> Result<Option<LogMeta>, ApiError> {
        self.d.journal.log_meta(log, &self.read_budget())
    }

    // ---- Start, timer and the work loop ------------------------------------

    /// The worker's first pass: re-run the re-issue, re-derive the evidence
    /// and the pulls from the store.
    fn start(&mut self) {
        self.reissue_due = true;
        self.evidence_due = true;
        self.restore_derive_due = true;
        let r = self
            .refresh_branches_flag()
            .and_then(|()| self.schedule_pulls());
        self.soft(r);
        self.work();
    }

    /// The 1 s timer: fail over unanswered requests, send due pulls, notice
    /// own geneses that arrived, re-arm the re-issue while candidates wait.
    fn tick(&mut self) {
        for key in self.d.sink.sweep() {
            self.probes.remove(&key);
        }
        let r = self.poll_own_genesis().and_then(|()| self.schedule_pulls());
        self.soft(r);
        if self.reissue_waiting > 0 && self.now().0 >= self.reissue_next_at {
            self.reissue_due = true;
        }
        self.work();
    }

    /// Runs everything that is due, until nothing is or the backoff holds.
    fn work(&mut self) {
        loop {
            if self.d.cancel.is_cancelled() {
                return;
            }
            self.poll_clone_cleared();
            self.intake();
            // Due jobs first: a failing job backs off on its own and never
            // blocks the others behind the worker-wide gate.
            if let Some(item) = self.next_runnable() {
                self.run_item(item);
                continue;
            }
            if self.retry_at.is_some_and(|at| self.now() < at) {
                return;
            }
            if self.reissue_due && !self.clone_held {
                self.run_reissue();
                continue;
            }
            if self.evidence_due {
                self.evidence_due = false;
                let r = self.run_evidence();
                if self.settle(r) {
                    self.evidence_due = true;
                }
                continue;
            }
            if self.restore_derive_due {
                self.restore_derive_due = false;
                let r = self.derive_restore();
                if self.settle(r) {
                    self.restore_derive_due = true;
                }
                continue;
            }
            return;
        }
    }

    // ---- Failure path ------------------------------------------------------

    /// Records a store failure on the sync lane, except one that shutdown
    /// itself caused (the cancelled call is not a lane fault).
    fn record_failure(&self, error: &ApiError) {
        if !self.d.cancel.is_cancelled() {
            self.d.status.record_failure(Lane::Sync, error);
        }
    }

    /// A store failure of a non-job unit (re-issue, evidence, restore
    /// derivation): record it, back the worker off 1 s doubling to 60 s. A
    /// failing job backs off on its own (`run_item`).
    fn fail(&mut self, error: &ApiError) {
        self.record_failure(error);
        self.failing = true;
        let at = self.now().0.saturating_add(self.delay_ms);
        self.retry_at = Some(MonoInstant(at));
        self.delay_ms = self.delay_ms.saturating_mul(2).min(BACKOFF_MAX_MS);
    }

    /// Records recovery (unless a job is still backing off) and clears the
    /// worker-wide backoff; per-job backoffs are untouched.
    fn succeed(&mut self) {
        // Health stays failed while a job is still in its own backoff.
        if self.failing && self.pending.iter().all(|q| q.retry_at.is_none()) {
            self.d.status.record_success(self.utc());
            self.failing = false;
        }
        self.delay_ms = BACKOFF_BASE_MS;
        self.retry_at = None;
    }

    /// Accounts a unit of work: true when it failed and should be retried.
    fn settle(&mut self, result: Result<(), ApiError>) -> bool {
        match result {
            Ok(()) => {
                self.succeed();
                false
            }
            Err(e) => {
                self.fail(&e);
                true
            }
        }
    }

    /// Like [`settle`](Self::settle) for background reads whose work is simply
    /// repeated by the next tick.
    fn soft(&mut self, result: Result<(), ApiError>) {
        self.settle(result);
    }

    fn retryable(error: &ApiError) -> bool {
        !matches!(
            error.code,
            ErrorCode::InvalidRequest | ErrorCode::NotFound | ErrorCode::Conflict
        )
    }

    // ---- Queue policy ------------------------------------------------------

    /// Moves jobs from the sink's queue into the pending list: `Have` jobs
    /// coalesce per source, a second `Restore` for a log is dropped, and the
    /// branch-entry cap drops the oldest coalescible job.
    fn intake(&mut self) {
        while self.pending.len() < super::JOB_QUEUE_SLOTS {
            let Some(job) = self.d.sink.pop_job() else {
                break;
            };
            self.admit(job);
        }
        self.enforce_cap();
    }

    fn admit(&mut self, job: Job) {
        let queued_at = self.now();
        match &job {
            Job::Have { source, .. } => {
                let source = *source;
                if let Some(slot) = self
                    .pending
                    .iter_mut()
                    .find(|q| matches!(&q.job, Job::Have { source: s, .. } if *s == source))
                {
                    // The slot keeps its backoff so a stream of `Have`s cannot
                    // defeat it.
                    slot.job = job;
                    slot.queued_at = queued_at;
                    return;
                }
            }
            Job::Restore { log, .. } => {
                let log = *log;
                if self
                    .pending
                    .iter()
                    .any(|q| matches!(&q.job, Job::Restore { log: l, .. } if *l == log))
                {
                    return;
                }
            }
            _ => {}
        }
        let bytes = branch_bytes(&job);
        self.pending_bytes += bytes;
        self.pending.push_back(Queued {
            job,
            queued_at,
            bytes,
            retry_at: None,
            delay_ms: BACKOFF_BASE_MS,
        });
    }

    fn enforce_cap(&mut self) {
        while self.pending_bytes > CAP_BYTES {
            let Some(index) = self
                .pending
                .iter()
                .position(|q| matches!(q.job, Job::Have { .. } | Job::Branch { .. }))
            else {
                return;
            };
            let Some(dropped) = self.pending.remove(index) else {
                return;
            };
            self.pending_bytes = self.pending_bytes.saturating_sub(dropped.bytes);
            if let Job::Branch { want, .. } = &dropped.job {
                self.d.sink.clear_request(&BranchKey::of(want));
                self.probes.remove(&BranchKey::of(want));
            }
            let now = self.now();
            let due = self
                .overflow_logged
                .is_none_or(|at| now.0.saturating_sub(at.0) >= OVERFLOW_LOG_EVERY_MS);
            if due {
                self.overflow_logged = Some(now);
                self.log(&format!(
                    "sync: incarnations queue over {} MiB; dropped a {} job",
                    CAP_BYTES >> 20,
                    dropped.job.kind_name()
                ));
            }
        }
    }

    /// The first pending job that may run now: a `Restore` waits for the
    /// advertised own geneses (or its 5 minutes).
    fn next_runnable(&mut self) -> Option<Queued> {
        let mut index = 0;
        let now = self.now();
        while index < self.pending.len() {
            if self.pending[index].retry_at.is_some_and(|at| now < at) {
                index += 1;
                continue;
            }
            let ready = match &self.pending[index].job {
                Job::Restore { .. } => {
                    let queued_at = self.pending[index].queued_at;
                    match self.restore_ready(queued_at) {
                        Ok(ready) => ready,
                        Err(e) => {
                            self.fail(&e);
                            false
                        }
                    }
                }
                _ => true,
            };
            if ready {
                let item = self.pending.remove(index)?;
                self.pending_bytes = self.pending_bytes.saturating_sub(item.bytes);
                return Some(item);
            }
            index += 1;
        }
        None
    }

    /// The earliest backoff deadline among pending jobs.
    #[cfg(test)]
    fn next_job_retry(&self) -> Option<MonoInstant> {
        self.pending.iter().filter_map(|q| q.retry_at).min()
    }

    /// Whether a `Restore` queued at `queued_at` may run: every own log an
    /// active link advertises has its genesis ingested, or 5 minutes passed.
    fn restore_ready(&self, queued_at: MonoInstant) -> Result<bool, ApiError> {
        if self.now().0.saturating_sub(queued_at.0) >= RESTORE_WAIT_MS {
            return Ok(true);
        }
        let advertised: BTreeSet<LogKey> = self
            .advertised_own
            .values()
            .flat_map(|logs| logs.iter().copied())
            .collect();
        for log in advertised {
            match self.meta(&log)? {
                Some(meta) if meta.contiguous >= 1 => {}
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn poll_clone_cleared(&mut self) {
        if !self.d.sink.take_clone_cleared() {
            return;
        }
        self.clone_held = false;
        self.held_logged = None;
        self.reissue_due = true;
        self.evidence_due = true;
        self.restore_derive_due = true;
        self.d.sink.clear_have_memo();
        self.seen_forks.clear();
        for log in std::mem::take(&mut self.held_logs) {
            self.d.sink.mark_resumed(log);
        }
        self.d.signals.request_round(self.now());
    }

    // ---- Running one job ---------------------------------------------------

    fn run_item(&mut self, mut item: Queued) {
        match self.dispatch(&item.job, item.queued_at) {
            Ok(()) => self.succeed(),
            Err(e) if Self::retryable(&e) => {
                self.record_failure(&e);
                self.failing = true;
                item.retry_at = Some(MonoInstant(self.now().0.saturating_add(item.delay_ms)));
                item.delay_ms = item.delay_ms.saturating_mul(2).min(BACKOFF_MAX_MS);
                self.pending_bytes += item.bytes;
                self.pending.push_back(item);
            }
            Err(e) => {
                if let Job::Branch { want, .. } = &item.job {
                    self.d.sink.clear_request(&BranchKey::of(want));
                }
                self.record_failure(&e);
                self.failing = true;
                self.log(&format!(
                    "sync: dropped a {} repair job: {}",
                    item.job.kind_name(),
                    e.detail
                ));
            }
        }
    }

    fn dispatch(&mut self, job: &Job, _queued_at: MonoInstant) -> Result<(), ApiError> {
        match job {
            Job::Fork {
                log,
                held,
                conflicting,
                source,
            } => self.handle_fork(*log, held, conflicting, *source),
            Job::Have {
                source,
                fork_cap,
                logs,
                forks,
            } => self.handle_have(*source, *fork_cap, logs, forks),
            Job::Branch {
                source,
                want,
                reply,
            } => self.handle_branch(*source, want, reply),
            Job::OwnerFork { proof, source } => self.owner_fork(proof, *source),
            Job::Restore { log, source, entry } => self.handle_restore(*log, *source, entry),
            Job::Salvage {
                backup_db,
                node_dir,
            } => self.handle_salvage(backup_db, node_dir),
            Job::Clone(report) => {
                let now = self.utc();
                quantum!(self, |tx, _ctx, _decision| store::record_clone_evidence(
                    tx, report, now
                ));
                Ok(())
            }
            Job::Reissue => {
                self.reissue_due = true;
                Ok(())
            }
        }
    }

    /// The clone gate held a repair: the driver idles and the log line is
    /// written once per hour.
    fn note_held(&mut self, log: Option<LogKey>) {
        self.clone_held = true;
        self.reissue_due = false;
        if let Some(log) = log {
            self.held_logs.insert(log);
        }
        let now = self.now();
        let due = self
            .held_logged
            .is_none_or(|at| now.0.saturating_sub(at.0) >= HELD_LOG_EVERY_MS);
        if due {
            self.held_logged = Some(now);
            self.log(HELD_LINE);
        }
    }

    // ---- Resync salvage ------------------------------------------------------

    /// Reads the backup of a `sync resync` rebuild back into the store, one
    /// ingest batch per writer turn (`JournalPort::ingest` verifies every
    /// entry), then ends the rebuild by deleting the marker. A store error
    /// keeps the job for the backoff retry; ingest is idempotent.
    fn handle_salvage(&mut self, backup_db: &Path, node_dir: &Path) -> Result<(), ApiError> {
        let cancel = self.d.cancel.clone();
        let this = &*self;
        let mut ingest = |log: &LogKey, entries: Vec<Envelope>| {
            let (_origin, _turn, _budget) = this.turn()?;
            this.d.journal.ingest(log, entries, &this.read_budget())
        };
        let report = crate::sync::resync::run_salvage(node_dir, backup_db, &mut ingest, &|| {
            cancel.is_cancelled()
        })?;
        let mut line = format!(
            "sync: resync salvage: {} logs, {} entries read back",
            report.logs, report.entries
        );
        if let Some(why) = &report.unreadable {
            line.push_str(&format!("; backup unreadable: {why}"));
        }
        for stop in report.stopped.iter().take(SALVAGE_STOPS_LOGGED) {
            line.push_str(&format!(
                "; log {} stopped: {}",
                uuid8(&stop.log.node),
                stop.why
            ));
        }
        if report.stopped.len() > SALVAGE_STOPS_LOGGED {
            line.push_str(&format!(
                "; {} more logs stopped",
                report.stopped.len() - SALVAGE_STOPS_LOGGED
            ));
        }
        line.push_str(if report.interrupted {
            "; interrupted, marker kept"
        } else if report.headroom {
            "; marker kept: no disk headroom"
        } else {
            "; marker removed"
        });
        self.log(&line);
        if report.headroom {
            // Retryable: the job stays queued and runs again; ingest is
            // idempotent, so the repeat continues where headroom ran out.
            return Err(ApiError::new(
                ErrorCode::StoreFull,
                "resync salvage stopped: no disk headroom",
            ));
        }
        Ok(())
    }

    // ---- Third-party and owner paths ---------------------------------------

    /// An ingest conflict: classify the pair, then record it (or rotate, when
    /// the log is this node's), and let the coordinator want the log again.
    fn handle_fork(
        &mut self,
        log: LogKey,
        held: &Envelope,
        conflicting: &Envelope,
        source: NodeUuid,
    ) -> Result<(), ApiError> {
        if let Some(meta) = self.meta(&log)? {
            self.act_on_pair(log, &meta, held, conflicting, source)?;
        }
        if !self.held_logs.contains(&log) {
            self.d.sink.mark_resumed(log);
        }
        Ok(())
    }

    /// Classifies two entries of `log` and acts on a proof.
    fn act_on_pair(
        &mut self,
        log: LogKey,
        meta: &LogMeta,
        held: &Envelope,
        other: &Envelope,
        source: NodeUuid,
    ) -> Result<(), ApiError> {
        match classify_pair(&meta.pubkey, held, other) {
            Classified::Proof(proof) if meta.own => self.owner_fork(&proof, Some(source)),
            Classified::Proof(proof) => self.third_party_fork(&proof, source),
            Classified::Same | Classified::NotProof => Ok(()),
            Classified::Corrupt { which, err } => {
                self.d.sink.penalize(source, log);
                self.log(&format!(
                    "sync: fork evidence on {}/{} from {} is corrupt ({which:?}: {err})",
                    uuid8(&log.node),
                    log.incarnation,
                    uuid8(&source)
                ));
                Ok(())
            }
        }
    }

    fn third_party_fork(&mut self, proof: &Proof, source: NodeUuid) -> Result<(), ApiError> {
        let record = quantum!(self, |tx, _ctx, _decision| store::record_fork(
            tx,
            proof,
            Some(source)
        ));
        self.d.sink.mark_resumed(proof.log);
        if record.new {
            self.d.sink.add_fork_ref(fork_ref(proof, record.frozen_at));
        }
        self.reissue_due = true;
        Ok(())
    }

    /// A proven fork of an own log: rotate (when it is the active one), record
    /// it and queue the other branch for a pull.
    fn owner_fork(&mut self, proof: &Proof, source: Option<NodeUuid>) -> Result<(), ApiError> {
        let rotation = quantum!(self, |tx, ctx, decision| store::owner_rotate(
            tx, ctx, decision, proof, source
        ));
        match rotation {
            Repair::Held { .. } => {
                self.note_held(Some(proof.log));
                Ok(())
            }
            Repair::Done(rotation) => {
                self.after_rotation(proof.log, rotation, source, Some(proof), &proof.conflicting)
            }
        }
    }

    fn handle_restore(
        &mut self,
        log: LogKey,
        source: NodeUuid,
        entry: &Envelope,
    ) -> Result<(), ApiError> {
        let rotation = quantum!(self, |tx, ctx, decision| store::restore_rotate(
            tx,
            ctx,
            decision,
            entry,
            Some(source)
        ));
        match rotation {
            Repair::Held { .. } => {
                self.note_held(Some(log));
                Ok(())
            }
            Repair::Done(rotation) => self.after_rotation(log, rotation, Some(source), None, entry),
        }
    }

    /// Post-commit work of an owner or restore rotation.
    fn after_rotation(
        &mut self,
        log: LogKey,
        rotation: Rotation,
        source: Option<NodeUuid>,
        proof: Option<&Proof>,
        conflicting: &Envelope,
    ) -> Result<(), ApiError> {
        match rotation {
            Rotation::Forked { fork, .. } => {
                if let Some(source) = source {
                    self.d
                        .sink
                        .note_tip_source(log, &hex_encode(&conflicting.hash), source);
                }
                self.d.sink.mark_resumed(log);
                if fork.new
                    && let Some(proof) = proof
                {
                    self.d.sink.add_fork_ref(fork_ref(proof, fork.frozen_at));
                }
                self.reissue_due = true;
                self.evidence_due = true;
                self.refresh_branches_flag()?;
                self.schedule_pulls()?;
            }
            Rotation::Restored { .. } => {
                // The old log is an ordinary own log now: the coordinator
                // wants it as soon as it is resumed.
                self.d.sink.mark_resumed(log);
                self.evidence_due = true;
            }
            Rotation::Stale => self.d.sink.mark_resumed(log),
        }
        Ok(())
    }

    // ---- Have-time detection and forwarded evidence ------------------------

    fn handle_have(
        &mut self,
        source: NodeUuid,
        fork_cap: bool,
        logs: &[HaveLog],
        forks: &[ForkRef],
    ) -> Result<(), ApiError> {
        let own = self.own_node()?;
        let budget = self.read_budget();
        for have in logs {
            let log = LogKey {
                node: have.node,
                incarnation: have.inc,
            };
            if have.node == own {
                self.note_advertised(source, log);
            }
            let Some(head) = hex_decode::<32>(&have.head) else {
                continue;
            };
            if have.counter == 0 {
                continue;
            }
            let memo = (have.counter, have.head.clone());
            if self.d.sink.have_memo(source, log).as_ref() == Some(&memo) {
                continue;
            }
            let Some(meta) = self.d.journal.log_meta(&log, &budget)? else {
                continue;
            };
            if meta.active {
                if have.counter > meta.contiguous {
                    if self.active_heads.len() < MAX_ACTIVE_HEADS {
                        self.active_heads
                            .insert((source, log), (have.counter, head));
                    }
                } else {
                    self.active_heads.remove(&(source, log));
                }
            }
            let stored = if have.counter <= meta.contiguous {
                self.d.journal.stored_hash(&log, have.counter, &budget)?
            } else {
                None
            };
            if stored == Some(head) {
                self.d.sink.set_have_memo(source, log, memo.0, memo.1);
                continue;
            }
            if meta.own && meta.state == LogState::Frozen {
                // A frozen own log: the peer's head is a branch tip.
                self.d.sink.note_tip_source(log, &have.head, source);
                quantum!(self, |tx, _ctx, _decision| store::upsert_branch_tip(
                    tx,
                    &log,
                    have.counter,
                    head,
                    Some(source)
                ));
                self.refresh_branches_flag()?;
                self.schedule_pulls()?;
                self.d.sink.set_have_memo(source, log, memo.0, memo.1);
                continue;
            }
            if have.counter > meta.contiguous {
                // The peer is ahead: its entries arrive through ingest.
                continue;
            }
            if !fork_cap {
                self.d
                    .sink
                    .report_head_mismatch_no_cap(log, have.counter, source);
                self.d.sink.set_have_memo(source, log, memo.0, memo.1);
                continue;
            }
            let want = BranchWant {
                node: log.node,
                inc: log.incarnation,
                tip: TipRef {
                    counter: have.counter,
                    hash: have.head.clone(),
                },
                from: have.counter,
            };
            if self.send_probe(source, want, Probe::Mismatch) {
                self.d.sink.set_have_memo(source, log, memo.0, memo.1);
            }
        }
        if fork_cap {
            for fork in forks {
                let log = LogKey {
                    node: fork.node,
                    incarnation: fork.inc,
                };
                let (Some(held), Some(conflicting)) = (
                    hex_decode::<32>(&fork.held.hash),
                    hex_decode::<32>(&fork.conflicting.hash),
                ) else {
                    continue;
                };
                if self.seen_forks.contains(&(log, held, conflicting)) {
                    continue;
                }
                self.resolve_fork(source, fork, Vec::new())?;
                if self.seen_forks.len() >= MAX_SEEN_FORKS {
                    self.seen_forks.clear();
                }
                self.seen_forks.insert((log, held, conflicting));
            }
        }
        self.evidence_due = true;
        self.restore_derive_due = true;
        Ok(())
    }

    fn note_advertised(&mut self, source: NodeUuid, log: LogKey) {
        if self.advertised_own.len() >= MAX_ADVERTISED_SOURCES
            && !self.advertised_own.contains_key(&source)
        {
            return;
        }
        let logs = self.advertised_own.entry(source).or_default();
        if logs.len() < MAX_ADVERTISED_LOGS
            && logs.insert(log)
            && self.watch_own.len() < MAX_ADVERTISED_SOURCES
        {
            self.watch_own.insert(log);
        }
    }

    /// Sends a probe `branch_want` and remembers what its reply is for;
    /// whether a request went out.
    fn send_probe(&mut self, source: NodeUuid, want: BranchWant, probe: Probe) -> bool {
        let key = BranchKey::of(&want);
        self.d.sink.note_tip_source(key.log, &key.tip_hash, source);
        if self.probes.len() >= MAX_PROBES {
            self.probes.clear();
        }
        if self.d.sink.request_branch(want).is_none() {
            return false;
        }
        self.probes.insert(key, probe);
        true
    }

    /// This store's entry of `log` at `counter` with `hash`, from the main
    /// chain or a recorded fork branch.
    fn local_entry(
        &self,
        log: &LogKey,
        counter: u64,
        hash: [u8; 32],
    ) -> Result<Option<Envelope>, ApiError> {
        let one = BatchLimits {
            entries: 1,
            bytes: usize::MAX,
        };
        let served =
            self.d
                .journal
                .serve_branch(log, (counter, hash), counter, one, &self.read_budget())?;
        Ok(served
            .and_then(|batch| batch.into_iter().next())
            .filter(|e| e.counter == counter && e.hash == hash))
    }

    /// Obtains both sides of a forwarded fork reference (asking the advertiser
    /// for the one side this node lacks, one request at a time) and acts on
    /// the pair.
    fn resolve_fork(
        &mut self,
        source: NodeUuid,
        fork: &ForkRef,
        got: Vec<Envelope>,
    ) -> Result<(), ApiError> {
        let log = LogKey {
            node: fork.node,
            incarnation: fork.inc,
        };
        let Some(meta) = self.meta(&log)? else {
            return Ok(());
        };
        let mut sides = Vec::with_capacity(2);
        for side in [&fork.held, &fork.conflicting] {
            let Some(hash) = hex_decode::<32>(&side.hash) else {
                return Ok(());
            };
            if let Some(entry) = got
                .iter()
                .find(|e| e.counter == side.counter && e.hash == hash)
            {
                sides.push(entry.clone());
            } else if let Some(entry) = self.local_entry(&log, side.counter, hash)? {
                sides.push(entry);
            } else {
                let want = BranchWant {
                    node: fork.node,
                    inc: fork.inc,
                    tip: side.clone(),
                    from: side.counter,
                };
                self.send_probe(
                    source,
                    want,
                    Probe::ForkSide {
                        fork: fork.clone(),
                        got,
                    },
                );
                return Ok(());
            }
        }
        let [first, second] = <[Envelope; 2]>::try_from(sides).map_err(|_| {
            ApiError::new(ErrorCode::StoreCorrupt, "fork reference resolved to a side")
        })?;
        // The held entry is the one this store keeps in `journal`; with
        // neither, the lower (counter, hash).
        let budget = self.read_budget();
        let in_journal = |e: &Envelope| -> Result<bool, ApiError> {
            Ok(self.d.journal.stored_hash(&log, e.counter, &budget)? == Some(e.hash))
        };
        let swap = if in_journal(&first)? {
            false
        } else if in_journal(&second)? {
            true
        } else {
            (second.counter, second.hash) < (first.counter, first.hash)
        };
        let (held, other) = if swap {
            (second, first)
        } else {
            (first, second)
        };
        self.act_on_pair(log, &meta, &held, &other, source)
    }

    // ---- Branch replies ----------------------------------------------------

    fn handle_branch(
        &mut self,
        source: NodeUuid,
        want: &BranchWant,
        reply: &BranchReply,
    ) -> Result<(), ApiError> {
        let key = BranchKey::of(want);
        let (entries, more) = match reply {
            BranchReply::None { .. } => {
                // The source no longer holds the branch: avoid it for the log;
                // the next pull goes to another source.
                self.d.sink.penalize(source, key.log);
                self.probes.remove(&key);
                self.d.sink.clear_request(&key);
                return self.schedule_pulls();
            }
            BranchReply::Entries { entries, more, .. } => (entries, *more),
        };
        let decoded: Result<Vec<Envelope>, _> = entries.iter().map(decode_wire).collect();
        let Ok(entries) = decoded else {
            self.d.sink.penalize(source, key.log);
            self.probes.remove(&key);
            self.log(&format!(
                "sync: branch reply for {}/{} from {} is malformed",
                uuid8(&key.log.node),
                key.log.incarnation,
                uuid8(&source)
            ));
            self.d.sink.clear_request(&key);
            return self.schedule_pulls();
        };
        match self.probes.get(&key).cloned() {
            Some(probe) => {
                self.on_probe(probe, source, want, &key, &entries)?;
                self.probes.remove(&key);
                // The reply is handled: the branch may be requested again.
                self.d.sink.clear_request(&key);
                Ok(())
            }
            None => self.on_pull_reply(source, &key, want.from, &entries, more),
        }
    }

    fn on_probe(
        &mut self,
        probe: Probe,
        source: NodeUuid,
        want: &BranchWant,
        key: &BranchKey,
        entries: &[Envelope],
    ) -> Result<(), ApiError> {
        let log = key.log;
        let Some(tip_hash) = hex_decode::<32>(&key.tip_hash) else {
            return Ok(());
        };
        match probe {
            Probe::RestoreEntry => {
                if let Some(entry) = entries.first().filter(|e| e.counter == want.from) {
                    let job = Job::Restore {
                        log,
                        source,
                        entry: Box::new(entry.clone()),
                    };
                    self.admit(job);
                }
                Ok(())
            }
            Probe::Mismatch | Probe::ForkSide { .. } => {
                let Some(theirs) = entries
                    .iter()
                    .find(|e| e.counter == want.tip.counter && e.hash == tip_hash)
                else {
                    self.d.sink.penalize(source, log);
                    return Ok(());
                };
                match probe {
                    Probe::ForkSide { fork, mut got } => {
                        got.push(theirs.clone());
                        self.resolve_fork(source, &fork, got)
                    }
                    _ => {
                        let Some(meta) = self.meta(&log)? else {
                            return Ok(());
                        };
                        let Some(mine) = self.local_main_entry(&log, want.tip.counter)? else {
                            return Ok(());
                        };
                        self.act_on_pair(log, &meta, &mine, theirs, source)
                    }
                }
            }
        }
    }

    /// This store's main-chain entry of `log` at `counter`.
    fn local_main_entry(&self, log: &LogKey, counter: u64) -> Result<Option<Envelope>, ApiError> {
        let one = BatchLimits {
            entries: 1,
            bytes: usize::MAX,
        };
        let batch = self
            .d
            .journal
            .serve(log, counter, one, &self.read_budget())?;
        Ok(batch.into_iter().find(|e| e.counter == counter))
    }

    /// A reply to a pull of an own log's branch.
    fn on_pull_reply(
        &mut self,
        source: NodeUuid,
        key: &BranchKey,
        asked_from: u64,
        entries: &[Envelope],
        more: bool,
    ) -> Result<(), ApiError> {
        let Some((branch_ix, next_from)) = self.find_branch(key)? else {
            self.d.sink.clear_request(key);
            return Ok(());
        };
        if asked_from != next_from {
            // A reply to a request the row has moved past: the 1 s timer can
            // ask again for the same `from` while the first reply still waits
            // in the queue, and its verdict (an earlier `next_from`) then
            // makes the second reply look like a bad one. Neither the source
            // nor the row is at fault: drop it; the next pull asks from
            // `next_from`.
            self.d.sink.clear_request(key);
            return self.schedule_pulls();
        }
        let verdict = quantum!(self, |tx, _ctx, _decision| store::branch_progress(
            tx, branch_ix, source, entries, more
        ));
        match verdict {
            BranchVerdict::Progress { short, .. } => {
                self.reissue_due = true;
                if short {
                    // The source held less than the tip: ask another one.
                    self.d.sink.penalize(source, key.log);
                }
            }
            BranchVerdict::NeedsEarlier { .. } => {}
            BranchVerdict::Corrupt { reason } => {
                self.d.sink.penalize(source, key.log);
                self.log(&format!(
                    "sync: branch reply for {}/{} from {} rejected: {reason}",
                    uuid8(&key.log.node),
                    key.log.incarnation,
                    uuid8(&source)
                ));
            }
        }
        self.refresh_branches_flag()?;
        // The row is updated: only now may the branch be requested again
        // (before, a pull would repeat the `from` this reply answered).
        self.d.sink.clear_request(key);
        self.schedule_pulls()
    }

    /// The `pulling` branch row of `key`, if any.
    fn find_branch(&self, key: &BranchKey) -> Result<Option<(i64, u64)>, ApiError> {
        let db = self.d.store.context().open_query(self.read_budget())?;
        Ok(store::due_branches(&db, UtcMillis(i64::MAX))?
            .into_iter()
            .find(|b| b.log == key.log && hex_encode(&b.tip.1) == key.tip_hash)
            .map(|b| (b.branch_ix, b.next_from)))
    }

    // ---- Pulls and the Branches flag ---------------------------------------

    /// Sends a `branch_want` for every due `pulling` branch that has no
    /// request outstanding and a usable source.
    fn schedule_pulls(&mut self) -> Result<(), ApiError> {
        let db = self.d.store.context().open_query(self.read_budget())?;
        let due = store::due_branches(&db, self.utc())?;
        drop(db);
        for branch in due {
            debug_assert!(
                (1..=branch.tip.0).contains(&branch.next_from),
                "a pulling branch has 1 <= next_from <= tip"
            );
            let want = BranchWant {
                node: branch.log.node,
                inc: branch.log.incarnation,
                tip: TipRef {
                    counter: branch.tip.0,
                    hash: hex_encode(&branch.tip.1),
                },
                from: branch.next_from,
            };
            self.d.sink.request_branch(want);
        }
        Ok(())
    }

    /// Sets the own-history `Branches` flag to whether any branch is still
    /// being pulled.
    fn refresh_branches_flag(&mut self) -> Result<(), ApiError> {
        let db = self.d.store.context().open_query(self.read_budget())?;
        let pulling = !store::due_branches(&db, UtcMillis(i64::MAX))?.is_empty();
        drop(db);
        self.d
            .own_history
            .set(HoldSource::Branches, pulling, self.now());
        Ok(())
    }

    // ---- Re-issue, clone evidence and restore derivation -------------------

    fn run_reissue(&mut self) {
        let result = (|| -> Result<Repair<store::ReissueProgress>, ApiError> {
            Ok(quantum!(self, |tx, ctx, decision| store::reissue_pass(
                tx,
                ctx,
                decision,
                ReissueLimits::default()
            )))
        })();
        match result {
            Ok(Repair::Held { .. }) => {
                self.note_held(None);
                self.succeed();
            }
            Ok(Repair::Done(progress)) => {
                self.reissue_waiting = progress.waiting;
                if !progress.has_more {
                    self.reissue_due = false;
                }
                self.reissue_next_at = self.now().0.saturating_add(REISSUE_WAITING_MS);
                self.succeed();
            }
            Err(e) => self.fail(&e),
        }
    }

    /// Records evidence for every `own_unknown_newer` log and lineage branch
    /// that no row names yet.
    fn run_evidence(&mut self) -> Result<(), ApiError> {
        let now = self.utc();
        let recorded = quantum!(self, |tx, ctx, _decision| store::record_own_unknown_newer(
            tx, ctx, now
        ));
        if recorded > 0 {
            self.log(&format!(
                "sync: recorded {recorded} clone evidence row(s) about this node; see sync status"
            ));
        }
        Ok(())
    }

    /// Own geneses that arrived since the last tick: evidence and restore
    /// derivation are re-run for them.
    fn poll_own_genesis(&mut self) -> Result<(), ApiError> {
        let mut arrived = false;
        for log in self.watch_own.clone() {
            if self.meta(&log)?.is_some_and(|meta| meta.contiguous >= 1) {
                self.watch_own.remove(&log);
                arrived = true;
            }
        }
        if arrived {
            self.evidence_due = true;
            self.restore_derive_due = true;
        }
        Ok(())
    }

    /// When the store knows a restore successor and a peer holds entries of
    /// the active log beyond this node's head, fetch the entry that extends
    /// the head so the restore rotation can run (a `Restore` job the sink's
    /// ingest path did not queue, or lost).
    fn derive_restore(&mut self) -> Result<(), ApiError> {
        if self.active_heads.is_empty() {
            return Ok(());
        }
        let db = self.d.store.context().open_query(self.read_budget())?;
        let successor = store::restore_successor(&db, self.d.store.context())?;
        drop(db);
        if successor.is_none() {
            return Ok(());
        }
        for ((source, log), (counter, head)) in self.active_heads.clone() {
            if self
                .pending
                .iter()
                .any(|q| matches!(&q.job, Job::Restore { log: l, .. } if *l == log))
            {
                continue;
            }
            let Some(meta) = self.meta(&log)? else {
                continue;
            };
            if !meta.active || counter <= meta.contiguous {
                continue;
            }
            let want = BranchWant {
                node: log.node,
                inc: log.incarnation,
                tip: TipRef {
                    counter,
                    hash: hex_encode(&head),
                },
                from: meta.contiguous + 1,
            };
            self.send_probe(source, want, Probe::RestoreEntry);
        }
        Ok(())
    }
}

impl Job {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Fork { .. } => "fork",
            Self::Have { .. } => "have",
            Self::Branch { .. } => "branch",
            Self::OwnerFork { .. } => "owner_fork",
            Self::Restore { .. } => "restore",
            Self::Salvage { .. } => "salvage",
            Self::Clone(_) => "clone",
            Self::Reissue => "reissue",
        }
    }
}

/// The branch-entry bytes a job holds in the queue.
fn branch_bytes(job: &Job) -> usize {
    let Job::Branch {
        reply: BranchReply::Entries { entries, .. },
        ..
    } = job
    else {
        return 0;
    };
    entries
        .iter()
        .map(|e| {
            e.payload.get().len()
                + e.node.len()
                + e.prev.len()
                + e.kind.len()
                + e.deps.len()
                + e.hash.len()
                + e.sig.len()
        })
        .sum()
}

/// Decodes a received wire entry without consuming it.
fn decode_wire(wire: &WireEnvelope) -> Result<Envelope, crate::sync::EnvelopeError> {
    Envelope::try_from(WireEnvelope {
        node: wire.node.clone(),
        inc: wire.inc,
        counter: wire.counter,
        prev: wire.prev.clone(),
        hlc: wire.hlc,
        kind: wire.kind.clone(),
        ver: wire.ver,
        deps: wire.deps.clone(),
        creates: wire.creates.clone(),
        needs: wire.needs.clone(),
        payload: wire.payload.clone(),
        hash: wire.hash.clone(),
        sig: wire.sig.clone(),
    })
}

fn fork_ref(proof: &Proof, frozen_at: u64) -> ForkRef {
    let tip = |e: &Envelope| TipRef {
        counter: e.counter,
        hash: hex_encode(&e.hash),
    };
    ForkRef {
        node: proof.log.node,
        inc: proof.log.incarnation,
        held: tip(&proof.held),
        conflicting: tip(&proof.conflicting),
        frozen_at,
    }
}

#[cfg(test)]
#[path = "../../../tests/sync/incarnations_worker.rs"]
mod tests;
