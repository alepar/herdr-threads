//! The incarnations sink: the non-blocking front of fork repair.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::requests::{BranchKey, Failover, RequestTable};
use super::{AheadClass, Proof, classify_ahead_of_self};
use crate::node::NodeUuid;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::sync::coordinator::{
    BranchReply, CloneReport, ForkAck, ForkOutbox, ForkSink, LogFn, uuid8,
};
use crate::sync::envelope::StoredEntry;
use crate::sync::frame::{BranchWant, ForkRef, HaveLog, MAX_HAVE_FORKS};
use crate::sync::journal_port::{ForkEvidence, JournalPort};
use crate::sync::{BatchLimits, LogKey};

/// Slots in the job queue.
pub const JOB_QUEUE_SLOTS: usize = 256;
/// The no-`fork.v1` line and the store-error line are logged at most once per
/// log per this many milliseconds.
const LOG_EVERY_MS: u64 = 3_600_000;
/// The queue-overflow line is logged at most once per this many milliseconds.
const OVERFLOW_LOG_EVERY_MS: u64 = 60_000;
/// Most branches the `tip -> sources` map tracks.
const MAX_TIPS: usize = 1024;
/// Most advertising sources kept per tip.
const MAX_SOURCES_PER_TIP: usize = 16;
/// Most `(source, log)` Have memos kept; the memo is cleared when full.
const MAX_HAVE_MEMOS: usize = 4096;
/// Most entries in a rate-limit map before stale ones are pruned.
const MAX_LOGGED: usize = 1024;

/// One unit of repair work for the worker thread.
///
/// A closed set: a new repair trigger is a new variant here, handled by the
/// worker's exhaustive `match`.
#[derive(Debug)]
pub enum Job {
    /// A conflicting pair for one position, from ingest.
    Fork {
        log: LogKey,
        held: Box<StoredEntry>,
        conflicting: Box<StoredEntry>,
        source: NodeUuid,
    },
    /// A peer's `have` (coalesced per source by the worker).
    Have {
        source: NodeUuid,
        fork_cap: bool,
        logs: Vec<HaveLog>,
        forks: Vec<ForkRef>,
    },
    /// An answered branch request.
    Branch {
        source: NodeUuid,
        want: BranchWant,
        reply: BranchReply,
    },
    /// A proven fork of an own log: rotate and pull the other branch.
    OwnerFork {
        proof: Box<Proof>,
        source: Option<NodeUuid>,
    },
    /// An own log continued elsewhere (a restore): `entry` is the peer's
    /// entry that extends this node's head, which `restore_rotate` re-checks.
    Restore {
        log: LogKey,
        source: NodeUuid,
        entry: Box<StoredEntry>,
    },
    /// Read the backup of a `sync resync` rebuild back into the new store;
    /// the marker in `node_dir` is deleted when the pass ends.
    Salvage {
        backup_db: PathBuf,
        node_dir: PathBuf,
    },
    /// A clone suspicion to record.
    Clone(CloneReport),
    /// Re-run the re-issue pass (after a commit, at start, every 5 s while
    /// waiting).
    Reissue,
}

impl Job {
    fn kind(&self) -> &'static str {
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

/// The queue is full; the job was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;

/// A bounded FIFO of [`Job`]s.
///
/// Owns: the [`JOB_QUEUE_SLOTS`] bound and the overflow report. Must not:
/// coalesce, cap bytes or block; those are the worker's.
#[derive(Debug, Default)]
pub struct JobQueue {
    jobs: VecDeque<Job>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `job`, or report [`Overflow`] (the job is dropped) when full.
    pub fn push(&mut self, job: Job) -> Result<(), Overflow> {
        if self.jobs.len() >= JOB_QUEUE_SLOTS {
            return Err(Overflow);
        }
        self.jobs.push_back(job);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Job> {
        self.jobs.pop_front()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

#[derive(Default)]
struct State {
    resumed: BTreeSet<LogKey>,
    sends: Vec<(NodeUuid, BranchWant)>,
    refs_changed: bool,
    /// Newest first.
    fork_refs: VecDeque<ForkRef>,
    sources: BTreeMap<BranchKey, Vec<NodeUuid>>,
    have_memo: BTreeMap<(NodeUuid, LogKey), (u64, String)>,
    requests: RequestTable,
    no_cap_logged: BTreeMap<LogKey, MonoInstant>,
    read_failed_logged: BTreeMap<LogKey, MonoInstant>,
    overflow_logged: Option<MonoInstant>,
}

/// Responsibility (IncarnationsSink / IncarnationsWorker)
///
/// The pair is the incarnations runtime. This type is the sink: the
/// [`ForkSink`] implementation and the non-blocking front. The
/// [`IncarnationsWorker`](super::IncarnationsWorker) is the
/// `herdr-incarnations` std thread that pops [`Job`]s, applies the queue
/// policy (per-source `Have` coalescing, the 8 MiB branch-entry cap) and
/// runs repair one writer quantum at a time. The sink only wakes it
/// ([`jobs_ready`](Self::jobs_ready)) and carries the clone-clearance signal
/// ([`notify_clone_cleared`](Self::notify_clone_cleared)).
/// - Owns: the bounded [`Job`] queue, and one `Mutex`-guarded shared state:
///   the resumed set, the outbox, the `fork_refs`, the `tip -> sources` map,
///   the per-`(source, log)` Have memo and the [`RequestTable`] (one
///   outstanding request per branch, a 30 s timeout with failover, 10-minute
///   `(source, log)` penalties, oldest-last-request preference).
/// - Hides: all fork policy from the coordinator, and when and from whom
///   branches are pulled.
/// - Must not: block in any [`ForkSink`] method (the one synchronous
///   signature check and its two quick journal reads in `on_fork` excepted),
///   write SQL, send frames (the coordinator drains [`ForkSink::outbox`]), or
///   share its pull penalties with the coordinator's rx-source penalties.
/// - Role: chosen once in `CoordinatorDeps`, replacing `LogOnlyForkSink`.
///   There is no `IncarnationsPort`: `ForkSink` and `JournalPort` are the two
///   seams. Its in-memory state starts empty; the worker re-derives from the
///   store.
///
/// Errors: `ForkAck::Corrupt` (the coordinator penalizes and reselects) is
/// kept apart from `Paused`. A job dropped on overflow is logged (once a
/// minute) and recovered by timeout or by the next `Have`; a log whose
/// evidence could not be queued or read is marked resumed so the evidence
/// arrives again.
pub struct IncarnationsSink {
    journal: Arc<dyn JournalPort>,
    clock: Arc<dyn Clock>,
    log: LogFn,
    jobs: Mutex<JobQueue>,
    state: Mutex<State>,
    /// Wakes the worker when a job is queued.
    job_ready: tokio::sync::Notify,
    /// Set by the clone-clearing command path, taken by the worker.
    clone_cleared: AtomicBool,
}

impl IncarnationsSink {
    pub fn new(journal: Arc<dyn JournalPort>, clock: Arc<dyn Clock>, log: LogFn) -> Self {
        Self {
            journal,
            clock,
            log,
            jobs: Mutex::new(JobQueue::new()),
            state: Mutex::new(State::default()),
            job_ready: tokio::sync::Notify::new(),
            clone_cleared: AtomicBool::new(false),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> MonoInstant {
        self.clock.monotonic_now()
    }

    /// Queue `job`; whether it was queued. On overflow the job is dropped and
    /// one line per minute says so.
    pub fn enqueue(&self, job: Job) -> bool {
        let kind = job.kind();
        let pushed = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(job);
        if pushed.is_ok() {
            self.job_ready.notify_one();
            return true;
        }
        let now = self.now();
        let due = {
            let mut st = self.state();
            let due = st
                .overflow_logged
                .is_none_or(|at| now.0.saturating_sub(at.0) >= OVERFLOW_LOG_EVERY_MS);
            if due {
                st.overflow_logged = Some(now);
            }
            due
        };
        if due {
            (self.log)(&format!(
                "sync: incarnations job queue full; dropped {kind}"
            ));
        }
        false
    }

    /// The oldest queued job, for the worker.
    pub fn pop_job(&self) -> Option<Job> {
        self.jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }

    /// Resolves when a job was queued since the worker last waited.
    pub async fn jobs_ready(&self) {
        self.job_ready.notified().await;
    }

    /// Tells the worker that `clear_clone_evidence` committed (called by the
    /// operator command path, ht-5fr.8): it re-runs the re-issue pass, forgets
    /// the Have memos and asks for a sync round.
    pub fn notify_clone_cleared(&self) {
        self.clone_cleared.store(true, Ordering::Release);
        self.job_ready.notify_one();
    }

    /// Whether a clone clearance was signalled since the last call.
    pub fn take_clone_cleared(&self) -> bool {
        self.clone_cleared.swap(false, Ordering::AcqRel)
    }

    /// Let the coordinator want `log` again after its fork is handled.
    pub fn mark_resumed(&self, log: LogKey) {
        self.state().resumed.insert(log);
    }

    /// Add a fork record to the advertised refs (newest first, at most
    /// [`MAX_HAVE_FORKS`]); a ref already advertised changes nothing. Returns
    /// whether it was new, which also sets `refs_changed`.
    pub fn add_fork_ref(&self, fork: ForkRef) -> bool {
        let mut st = self.state();
        let same = |r: &ForkRef| {
            r.node == fork.node
                && r.inc == fork.inc
                && r.held == fork.held
                && r.conflicting == fork.conflicting
        };
        if st.fork_refs.iter().any(same) {
            return false;
        }
        st.fork_refs.push_front(fork);
        st.fork_refs.truncate(MAX_HAVE_FORKS);
        st.refs_changed = true;
        true
    }

    /// Record that `source` advertises the branch of `log` ending at
    /// `tip_hash`.
    pub fn note_tip_source(&self, log: LogKey, tip_hash: &str, source: NodeUuid) {
        note_source(&mut self.state(), log, tip_hash, source);
    }

    /// The sources known to advertise `key`.
    pub fn tip_sources(&self, key: &BranchKey) -> Vec<NodeUuid> {
        self.state().sources.get(key).cloned().unwrap_or_default()
    }

    /// Ask for `want` from one advertising source that holds no live
    /// penalty for the log, preferring the oldest last request. The send is
    /// queued in the outbox; the chosen source is returned. `None` when the
    /// branch already has a request outstanding or no source is usable.
    pub fn request_branch(&self, want: BranchWant) -> Option<NodeUuid> {
        let now = self.now();
        let mut st = self.state();
        let key = BranchKey::of(&want);
        let sources = st.sources.get(&key).cloned().unwrap_or_default();
        let source = st.requests.request(want.clone(), &sources, now)?;
        st.sends.push((source, want));
        Some(source)
    }

    /// The request of `key` failed (`branch_none`): move it to another
    /// source, queueing the new send.
    pub fn fail_branch(&self, key: &BranchKey) -> Failover {
        let now = self.now();
        let mut st = self.state();
        let sources = st.sources.get(key).cloned().unwrap_or_default();
        let outcome = st.requests.fail(key, &sources, now);
        if let Failover::Resent(source, want) = &outcome {
            st.sends.push((*source, want.clone()));
        }
        outcome
    }

    /// Forget the outstanding request of `key`, so a later pull may ask
    /// again.
    pub fn clear_request(&self, key: &BranchKey) -> bool {
        self.state().requests.clear(key)
    }

    /// Avoid `(source, log)` for 10 minutes after a bad reply.
    pub fn penalize(&self, source: NodeUuid, log: LogKey) {
        let now = self.now();
        self.state().requests.penalize(source, log, now);
    }

    /// Whether `(source, log)` is avoided for branch pulls right now.
    pub fn is_penalized(&self, source: NodeUuid, log: LogKey) -> bool {
        let now = self.now();
        self.state().requests.is_penalized(source, log, now)
    }

    /// The worker's 1 s timer: fail over requests unanswered for 30 s,
    /// queueing the new sends. Returns the branches left with no source.
    pub fn sweep(&self) -> Vec<BranchKey> {
        let now = self.now();
        let mut st = self.state();
        let State {
            requests,
            sources,
            sends,
            ..
        } = &mut *st;
        let swept = requests.sweep(now, &|key| sources.get(key).cloned().unwrap_or_default());
        sends.extend(swept.sends);
        swept.exhausted
    }

    /// The last `(counter, head)` checked for `(source, log)`.
    pub fn have_memo(&self, source: NodeUuid, log: LogKey) -> Option<(u64, String)> {
        self.state().have_memo.get(&(source, log)).cloned()
    }

    pub fn set_have_memo(&self, source: NodeUuid, log: LogKey, counter: u64, head: String) {
        let mut st = self.state();
        if st.have_memo.len() >= MAX_HAVE_MEMOS && !st.have_memo.contains_key(&(source, log)) {
            st.have_memo.clear();
        }
        st.have_memo.insert((source, log), (counter, head));
    }

    /// Forget every Have memo (after a clone clearance), so the next `Have`s
    /// re-trigger detection.
    pub fn clear_have_memo(&self) {
        self.state().have_memo.clear();
    }

    /// A peer without `fork.v1` holds a different entry at a counter this
    /// node holds: one line per log per hour, and nothing else.
    pub fn report_head_mismatch_no_cap(&self, log: LogKey, counter: u64, source: NodeUuid) {
        let now = self.now();
        if !due_once_per_hour(&mut self.state().no_cap_logged, log, now) {
            return;
        }
        (self.log)(&format!(
            "sync: log {}/{} head differs at {} from {} (no fork.v1)",
            uuid8(&log.node),
            log.incarnation,
            counter,
            uuid8(&source)
        ));
    }

    /// A journal read in `on_fork` failed or the log is unknown: one line per
    /// log per hour, and the log is resumed so the evidence arrives again.
    fn read_failed(&self, log: LogKey, source: NodeUuid, why: &str) {
        let now = self.now();
        let due = {
            let mut st = self.state();
            st.resumed.insert(log);
            due_once_per_hour(&mut st.read_failed_logged, log, now)
        };
        if due {
            (self.log)(&format!(
                "sync: fork check on {}/{} from {} not possible: {why}",
                uuid8(&log.node),
                log.incarnation,
                uuid8(&source)
            ));
        }
    }

    /// Queue `job`; when it is dropped, resume `log` so the evidence is
    /// offered again.
    fn enqueue_or_resume(&self, job: Job, log: LogKey) {
        if !self.enqueue(job) {
            self.mark_resumed(log);
        }
    }

    fn on_ahead_of_self(&self, log: LogKey, entry: &StoredEntry, source: NodeUuid) -> ForkAck {
        let budget = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        };
        let meta = match self.journal.log_meta(&log, &budget) {
            Ok(Some(meta)) => meta,
            Ok(None) => {
                self.read_failed(log, source, "log unknown");
                return ForkAck::Paused;
            }
            Err(e) => {
                self.read_failed(log, source, &format!("{:?}", e.code));
                return ForkAck::Paused;
            }
        };
        let read_error = std::cell::Cell::new(false);
        let entry_at = |counter: u64| {
            let one = BatchLimits {
                entries: 1,
                bytes: usize::MAX,
            };
            match self.journal.serve(&log, counter, one, &budget) {
                Ok(batch) => batch.into_iter().next().filter(|e| e.counter() == counter),
                Err(_) => {
                    read_error.set(true);
                    None
                }
            }
        };
        let class =
            classify_ahead_of_self((meta.contiguous, meta.head), entry_at, &meta.pubkey, entry);
        match class {
            AheadClass::Corrupt(_) => return ForkAck::Corrupt,
            AheadClass::Restore => {
                let job = Job::Restore {
                    log,
                    source,
                    entry: Box::new(entry.clone()),
                };
                self.enqueue_or_resume(job, log);
            }
            AheadClass::Fork(proof) => {
                let job = Job::OwnerFork {
                    proof,
                    source: Some(source),
                };
                self.enqueue_or_resume(job, log);
            }
            AheadClass::NotFork if read_error.get() => {
                self.read_failed(log, source, "stored entry unreadable");
            }
            AheadClass::NotFork => self.mark_resumed(log),
        }
        ForkAck::Paused
    }
}

fn note_source(st: &mut State, log: LogKey, tip_hash: &str, source: NodeUuid) {
    let key = BranchKey {
        log,
        tip_hash: tip_hash.to_owned(),
    };
    if !st.sources.contains_key(&key) && st.sources.len() >= MAX_TIPS {
        return;
    }
    let sources = st.sources.entry(key).or_default();
    if !sources.contains(&source) && sources.len() < MAX_SOURCES_PER_TIP {
        sources.push(source);
    }
}

/// Whether `log` has not been logged in the last hour; records `now` when so.
fn due_once_per_hour(
    logged: &mut BTreeMap<LogKey, MonoInstant>,
    log: LogKey,
    now: MonoInstant,
) -> bool {
    let due = logged
        .get(&log)
        .is_none_or(|at| now.0.saturating_sub(at.0) >= LOG_EVERY_MS);
    if due {
        if logged.len() >= MAX_LOGGED {
            logged.retain(|_, at| now.0.saturating_sub(at.0) < LOG_EVERY_MS);
        }
        logged.insert(log, now);
    }
    due
}

impl ForkSink for IncarnationsSink {
    fn on_fork(&self, log: LogKey, evidence: ForkEvidence, source: NodeUuid) -> ForkAck {
        match evidence {
            ForkEvidence::AheadOfSelf { entry, .. } => self.on_ahead_of_self(log, &entry, source),
            ForkEvidence::Conflict { held, conflicting } => {
                let job = Job::Fork {
                    log,
                    held,
                    conflicting,
                    source,
                };
                self.enqueue_or_resume(job, log);
                ForkAck::Paused
            }
        }
    }

    fn resumed(&self) -> Vec<LogKey> {
        std::mem::take(&mut self.state().resumed)
            .into_iter()
            .collect()
    }

    fn on_have(&self, source: NodeUuid, fork_cap: bool, logs: &[HaveLog], forks: &[ForkRef]) {
        let forks: &[ForkRef] = if fork_cap { forks } else { &[] };
        {
            let mut st = self.state();
            for fork in forks {
                let log = LogKey {
                    node: fork.node,
                    incarnation: fork.inc,
                };
                note_source(&mut st, log, &fork.held.hash, source);
                note_source(&mut st, log, &fork.conflicting.hash, source);
            }
        }
        self.enqueue(Job::Have {
            source,
            fork_cap,
            logs: logs.to_vec(),
            forks: forks.to_vec(),
        });
    }

    fn on_branch(&self, source: NodeUuid, reply: BranchReply) {
        let (node, inc, tip_hash) = match &reply {
            BranchReply::Entries {
                node,
                inc,
                tip_hash,
                ..
            }
            | BranchReply::None {
                node,
                inc,
                tip_hash,
            } => (*node, *inc, tip_hash.clone()),
        };
        let key = BranchKey {
            log: LogKey {
                node,
                incarnation: inc,
            },
            tip_hash,
        };
        let Some(answered) = self.state().requests.answer(source, &key) else {
            return;
        };
        self.enqueue(Job::Branch {
            source,
            want: answered.want,
            reply,
        });
    }

    fn outbox(&self) -> ForkOutbox {
        let mut st = self.state();
        ForkOutbox {
            sends: std::mem::take(&mut st.sends),
            refs_changed: std::mem::take(&mut st.refs_changed),
        }
    }

    fn fork_refs(&self) -> Vec<ForkRef> {
        self.state().fork_refs.iter().cloned().collect()
    }

    fn on_clone(&self, report: CloneReport) {
        self.enqueue(Job::Clone(report));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::node::NodeIdentity;
    use crate::protocol::time::UtcMillis;
    use crate::sync::envelope::{Deps, Envelope, UnsignedEnvelope};
    use crate::sync::frame::TipRef;
    use crate::sync::hlc::HlcStamp;
    use crate::sync::memory_journal::MemoryJournal;
    use crate::sync::{LogHead, LogState};

    /// A clock that moves only when the test says so.
    #[derive(Default)]
    struct TestClock(AtomicU64);
    impl TestClock {
        fn advance(&self, ms: u64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }

    struct Rig {
        id: Arc<NodeIdentity>,
        journal: Arc<MemoryJournal>,
        clock: Arc<TestClock>,
        lines: Arc<Mutex<Vec<String>>>,
        sink: IncarnationsSink,
    }

    fn rig() -> Rig {
        let id = Arc::new(NodeIdentity::from_seed([1; 32]));
        let journal = Arc::new(MemoryJournal::new(id.clone()));
        // Counters 1 (genesis), 2 and 3.
        journal
            .author_raw("test.kind", br#"{"t":"m2"}"#.to_vec())
            .unwrap();
        journal
            .author_raw("test.kind", br#"{"t":"m3"}"#.to_vec())
            .unwrap();
        let clock = Arc::new(TestClock::default());
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink_lines = lines.clone();
        let log: LogFn = Arc::new(move |line| sink_lines.lock().unwrap().push(line.to_owned()));
        let sink = IncarnationsSink::new(journal.clone(), clock.clone(), log);
        Rig {
            id,
            journal,
            clock,
            lines,
            sink,
        }
    }

    impl Rig {
        fn entry(&self, counter: u64, prev: [u8; 32], tag: &str) -> Envelope {
            UnsignedEnvelope {
                log: self.journal.self_log(),
                counter,
                prev_hash: prev,
                hlc: HlcStamp::from_parts(1_700_000_000_000 + counter, 0),
                kind: "test.kind".into(),
                kind_version: 1,
                deps: Deps::empty(),
                creates: None,
                needs: vec![],
                payload: format!(r#"{{"t":"{tag}"}}"#).into_bytes(),
            }
            .seal(&self.id)
        }

        fn stored(&self, counter: u64) -> Envelope {
            let budget = CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            };
            self.journal
                .serve(
                    &self.journal.self_log(),
                    counter,
                    BatchLimits::default(),
                    &budget,
                )
                .unwrap()
                .remove(0)
                .as_known()
                .cloned()
                .expect("a known entry")
        }

        fn ahead(&self, entry: Envelope) -> ForkEvidence {
            ForkEvidence::AheadOfSelf {
                log: self.journal.self_log(),
                my_head: LogHead {
                    counter: 3,
                    head_hash: self.stored(3).hash,
                    state: LogState::Active,
                    frozen_at: None,
                },
                entry: Box::new(StoredEntry::from(entry)),
            }
        }

        fn drain_jobs(&self) -> Vec<Job> {
            std::iter::from_fn(|| self.sink.pop_job()).collect()
        }

        fn lines(&self) -> Vec<String> {
            self.lines.lock().unwrap().clone()
        }
    }

    fn peer(n: u8) -> NodeUuid {
        NodeUuid::from_bytes([n; 16])
    }

    fn hex(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn tip(counter: u64, n: u8) -> TipRef {
        TipRef {
            counter,
            hash: hex(n),
        }
    }

    fn want_for(log: LogKey, tip_ref: TipRef, from: u64) -> BranchWant {
        BranchWant {
            node: log.node,
            inc: log.incarnation,
            tip: tip_ref,
            from,
        }
    }

    #[test]
    fn on_fork_bad_ahead_entry_corrupt() {
        let rig = rig();
        let mut bad = rig.entry(4, rig.stored(3).hash, "m4");
        bad.signature[0] ^= 1;
        let log = rig.journal.self_log();
        assert_eq!(
            rig.sink.on_fork(log, rig.ahead(bad), peer(7)),
            ForkAck::Corrupt
        );
        assert!(rig.drain_jobs().is_empty());
        assert!(rig.sink.resumed().is_empty());
    }

    #[test]
    fn on_fork_restore_ownerfork_fork_queue_and_paused() {
        let rig = rig();
        let log = rig.journal.self_log();
        let source = peer(7);

        // Extends my head: a restore.
        let restore = rig.entry(4, rig.stored(3).hash, "m4");
        assert_eq!(
            rig.sink.on_fork(log, rig.ahead(restore), source),
            ForkAck::Paused
        );
        // Contradicts my entry at counter 3: an owner fork.
        let theirs = rig.entry(3, rig.stored(2).hash, "other");
        assert_eq!(
            rig.sink.on_fork(log, rig.ahead(theirs.clone()), source),
            ForkAck::Paused
        );
        // Two entries for one position at a third party: a plain fork job.
        let held = rig.entry(2, rig.stored(1).hash, "x");
        let conflicting = rig.entry(2, rig.stored(1).hash, "y");
        let ev = ForkEvidence::Conflict {
            held: Box::new(StoredEntry::from(held.clone())),
            conflicting: Box::new(StoredEntry::from(conflicting.clone())),
        };
        assert_eq!(rig.sink.on_fork(log, ev, source), ForkAck::Paused);

        let jobs = rig.drain_jobs();
        assert_eq!(jobs.len(), 3, "{jobs:?}");
        assert!(
            matches!(&jobs[0], Job::Restore { log: l, source: s, .. } if *l == log && *s == source)
        );
        match &jobs[1] {
            Job::OwnerFork { proof, source: s } => {
                assert_eq!(*s, Some(source));
                assert_eq!(
                    (proof.x, proof.held.clone(), proof.conflicting.clone()),
                    (3, rig.stored(3).into(), theirs.clone().into())
                );
            }
            other => panic!("expected OwnerFork, got {other:?}"),
        }
        match &jobs[2] {
            Job::Fork {
                held: h,
                conflicting: c,
                source: s,
                ..
            } => assert_eq!(
                (&**h, &**c, *s),
                (
                    &StoredEntry::from(held.clone()),
                    &StoredEntry::from(conflicting.clone()),
                    source
                )
            ),
            other => panic!("expected Fork, got {other:?}"),
        }
        // A real fork or restore is resumed by the worker, not by the sink.
        assert!(rig.sink.resumed().is_empty());
    }

    #[test]
    fn on_fork_not_fork_pauses_and_resumes_the_log() {
        let rig = rig();
        let log = rig.journal.self_log();
        // The entry I already hold: not a fork.
        let same = rig.stored(2);
        assert_eq!(
            rig.sink.on_fork(log, rig.ahead(same), peer(7)),
            ForkAck::Paused
        );
        assert!(rig.drain_jobs().is_empty());
        assert_eq!(rig.sink.resumed(), vec![log]);
        // Unknown log: cannot classify; paused, resumed for a retry, one line.
        let unknown = LogKey {
            node: peer(5),
            incarnation: 1,
        };
        let ev = ForkEvidence::AheadOfSelf {
            log: unknown,
            my_head: LogHead {
                counter: 1,
                head_hash: [0; 32],
                state: LogState::Active,
                frozen_at: None,
            },
            entry: Box::new(StoredEntry::from(rig.stored(2))),
        };
        assert_eq!(rig.sink.on_fork(unknown, ev, peer(7)), ForkAck::Paused);
        assert_eq!(rig.sink.resumed(), vec![unknown]);
        assert_eq!(rig.lines().len(), 1);
    }

    fn salvage_job() -> Job {
        Job::Salvage {
            backup_db: PathBuf::from("backup.db"),
            node_dir: PathBuf::from("node"),
        }
    }

    #[test]
    fn full_job_queue_reports_overflow() {
        let mut queue = JobQueue::new();
        for _ in 0..JOB_QUEUE_SLOTS {
            queue.push(Job::Reissue).unwrap();
        }
        assert_eq!(queue.push(Job::Reissue), Err(Overflow));
        assert_eq!(queue.len(), JOB_QUEUE_SLOTS);
        assert!(queue.pop().is_some());
        assert_eq!(queue.push(salvage_job()), Ok(()));

        // Through the sink: the job is dropped, one line a minute, and a
        // dropped fork resumes its log.
        let rig = rig();
        for _ in 0..JOB_QUEUE_SLOTS {
            assert!(rig.sink.enqueue(Job::Reissue));
        }
        assert!(!rig.sink.enqueue(Job::Reissue));
        assert!(!rig.sink.enqueue(salvage_job()));
        assert_eq!(rig.lines().len(), 1, "{:?}", rig.lines());
        rig.clock.advance(60_000);
        assert!(!rig.sink.enqueue(Job::Reissue));
        assert_eq!(rig.lines().len(), 2);
        let log = rig.journal.self_log();
        let ev = ForkEvidence::Conflict {
            held: Box::new(StoredEntry::from(rig.stored(2))),
            conflicting: Box::new(StoredEntry::from(rig.stored(3))),
        };
        assert_eq!(rig.sink.on_fork(log, ev, peer(7)), ForkAck::Paused);
        assert_eq!(rig.sink.resumed(), vec![log]);
    }

    /// A fork with an opaque side is queued like any other: the worker
    /// classifies it on prefix fields (`Paused`, one `Job::Fork` carrying the
    /// entries as stored, no line from the sink).
    #[test]
    fn opaque_fork_side_is_queued_as_stored() {
        let _registry = crate::sync::envelope::hold_layout_registry();
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};

        let rig = rig();
        let signer = NodeIdentity::from_seed([5; 32]);
        let log = LogKey {
            node: peer(9),
            incarnation: 2,
        };
        let mut unsigned = synthetic_unsigned(&signer, 2, 2, [7; 32]);
        unsigned.log = log;
        let opaque = StoredEntry::from(synthetic_entry(&signer, &unsigned));
        let known = StoredEntry::from(rig.entry(2, [7; 32], "k"));
        let source = peer(7);

        for (held, conflicting) in [(&opaque, &known), (&known, &opaque)] {
            let ev = ForkEvidence::Conflict {
                held: Box::new(held.clone()),
                conflicting: Box::new(conflicting.clone()),
            };
            assert_eq!(rig.sink.on_fork(log, ev, source), ForkAck::Paused);
        }
        let jobs = rig.drain_jobs();
        assert_eq!(jobs.len(), 2, "{jobs:?}");
        for (job, (want_held, want_conflicting)) in
            jobs.iter().zip([(&opaque, &known), (&known, &opaque)])
        {
            let Job::Fork {
                log: l,
                held,
                conflicting,
                source: s,
            } = job
            else {
                panic!("expected Fork, got {job:?}");
            };
            assert_eq!(
                (*l, &**held, &**conflicting, *s),
                (log, want_held, want_conflicting, source)
            );
        }
        assert!(rig.lines().is_empty(), "the worker reports, not the sink");
    }

    #[test]
    fn on_branch_drops_unmatched() {
        let rig = rig();
        let log = LogKey {
            node: peer(9),
            incarnation: 2,
        };
        let (a, b) = (peer(1), peer(2));
        let none = |n: u8| BranchReply::None {
            node: log.node,
            inc: log.incarnation,
            tip_hash: hex(n),
        };
        // Nothing asked: dropped.
        rig.sink.on_branch(a, none(5));
        assert!(rig.drain_jobs().is_empty());

        rig.sink.note_tip_source(log, &hex(5), a);
        let want = want_for(log, tip(8, 5), 4);
        assert_eq!(rig.sink.request_branch(want.clone()), Some(a));
        // Wrong source, wrong tip: dropped, and the request stays.
        rig.sink.on_branch(b, none(5));
        rig.sink.on_branch(a, none(6));
        assert!(rig.drain_jobs().is_empty());
        // The right one is matched, once.
        rig.sink.on_branch(a, none(5));
        rig.sink.on_branch(a, none(5));
        let jobs = rig.drain_jobs();
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        assert!(matches!(
            &jobs[0],
            Job::Branch { source, want: w, reply: BranchReply::None { .. } }
                if *source == a && *w == want
        ));
    }

    #[test]
    fn on_clone_queues_one_job() {
        let rig = rig();
        let report = CloneReport {
            subject: peer(3),
            kind: crate::sync::coordinator::CloneKind::PeerClose,
            boots: vec![],
            source: peer(4),
        };
        rig.sink.on_clone(report.clone());
        let jobs = rig.drain_jobs();
        assert_eq!(jobs.len(), 1);
        assert!(matches!(&jobs[0], Job::Clone(r) if *r == report));
    }

    #[test]
    fn on_have_queues_job_and_records_fork_ref_sources() {
        let rig = rig();
        let log = LogKey {
            node: peer(9),
            incarnation: 1,
        };
        let fork = ForkRef {
            node: log.node,
            inc: log.incarnation,
            held: tip(5, 1),
            conflicting: tip(6, 2),
            frozen_at: 4,
        };
        // Without the cap the refs are ignored but the job still runs.
        rig.sink
            .on_have(peer(1), false, &[], std::slice::from_ref(&fork));
        assert!(
            rig.sink
                .tip_sources(&BranchKey {
                    log,
                    tip_hash: hex(2)
                })
                .is_empty()
        );
        rig.sink
            .on_have(peer(2), true, &[], std::slice::from_ref(&fork));
        rig.sink
            .on_have(peer(3), true, &[], std::slice::from_ref(&fork));
        for n in [1, 2] {
            let key = BranchKey {
                log,
                tip_hash: hex(n),
            };
            assert_eq!(rig.sink.tip_sources(&key), vec![peer(2), peer(3)]);
        }
        let jobs = rig.drain_jobs();
        assert_eq!(jobs.len(), 3);
        assert!(matches!(&jobs[0], Job::Have { forks, fork_cap: false, .. } if forks.is_empty()));
        assert!(matches!(&jobs[1], Job::Have { forks, fork_cap: true, .. } if forks.len() == 1));
    }

    #[test]
    fn outbox_drains_once_refs_changed_once() {
        let rig = rig();
        let log = LogKey {
            node: peer(9),
            incarnation: 1,
        };
        let a = peer(1);
        rig.sink.note_tip_source(log, &hex(5), a);
        let want = want_for(log, tip(8, 5), 4);
        assert_eq!(rig.sink.request_branch(want.clone()), Some(a));
        assert!(rig.sink.add_fork_ref(ForkRef {
            node: log.node,
            inc: log.incarnation,
            held: tip(5, 1),
            conflicting: tip(8, 5),
            frozen_at: 4,
        }));
        let out = rig.sink.outbox();
        assert_eq!(out.sends, vec![(a, want)]);
        assert!(out.refs_changed);
        let again = rig.sink.outbox();
        assert!(again.sends.is_empty());
        assert!(!again.refs_changed);
        // A ref already advertised does not flag a change.
        assert!(!rig.sink.add_fork_ref(rig.sink.fork_refs()[0].clone()));
        assert!(!rig.sink.outbox().refs_changed);
    }

    #[test]
    fn fork_refs_capped_64_newest_first() {
        let rig = rig();
        for i in 0..70u64 {
            rig.sink.add_fork_ref(ForkRef {
                node: peer(9),
                inc: 1,
                held: tip(i + 1, 1),
                conflicting: tip(i + 1, 2),
                frozen_at: i,
            });
        }
        let refs = rig.sink.fork_refs();
        assert_eq!(refs.len(), MAX_HAVE_FORKS);
        assert_eq!(refs[0].frozen_at, 69);
        assert_eq!(refs[63].frozen_at, 6);
        assert!(refs.windows(2).all(|w| w[0].frozen_at > w[1].frozen_at));
    }

    #[test]
    fn resumed_returns_each_log_once() {
        let rig = rig();
        let log = rig.journal.self_log();
        rig.sink.mark_resumed(log);
        rig.sink.mark_resumed(log);
        assert_eq!(rig.sink.resumed(), vec![log]);
        assert!(rig.sink.resumed().is_empty());
    }

    #[test]
    fn sink_sweep_fails_over_and_penalty_keeps_source_out() {
        let rig = rig();
        let log = LogKey {
            node: peer(9),
            incarnation: 1,
        };
        let (a, b) = (peer(1), peer(2));
        rig.sink.note_tip_source(log, &hex(5), a);
        rig.sink.note_tip_source(log, &hex(5), b);
        let want = want_for(log, tip(8, 5), 4);
        assert_eq!(rig.sink.request_branch(want.clone()), Some(a));
        rig.sink.outbox();
        rig.clock.advance(30_000);
        assert!(rig.sink.sweep().is_empty());
        assert_eq!(rig.sink.outbox().sends, vec![(b, want.clone())]);
        // A reply from b answers it, and the branch stays held until the
        // worker has handled the reply (`clear_request`); a penalty on b then
        // leaves only a.
        rig.sink.on_branch(
            b,
            BranchReply::None {
                node: log.node,
                inc: log.incarnation,
                tip_hash: hex(5),
            },
        );
        assert_eq!(rig.sink.request_branch(want.clone()), None);
        rig.sink.clear_request(&BranchKey::of(&want));
        rig.sink.penalize(b, log);
        assert_eq!(rig.sink.request_branch(want.clone()), Some(a));
        rig.sink.clear_request(&BranchKey::of(&want));
        rig.sink.penalize(a, log);
        assert_eq!(rig.sink.request_branch(want), None);
    }

    #[test]
    fn no_fork_cap_line_once_per_log_per_hour() {
        let rig = rig();
        let log = LogKey {
            node: peer(9),
            incarnation: 3,
        };
        let other = LogKey {
            node: peer(9),
            incarnation: 4,
        };
        rig.sink.report_head_mismatch_no_cap(log, 7, peer(1));
        rig.sink.report_head_mismatch_no_cap(log, 8, peer(2));
        rig.clock.advance(3_599_999);
        rig.sink.report_head_mismatch_no_cap(log, 9, peer(1));
        assert_eq!(
            rig.lines(),
            vec!["sync: log 09090909/3 head differs at 7 from 01010101 (no fork.v1)"]
        );
        // Another log has its own hour.
        rig.sink.report_head_mismatch_no_cap(other, 2, peer(1));
        assert_eq!(rig.lines().len(), 2);
        rig.clock.advance(1);
        rig.sink.report_head_mismatch_no_cap(log, 10, peer(2));
        let lines = rig.lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[2],
            "sync: log 09090909/3 head differs at 10 from 02020202 (no fork.v1)"
        );
    }
}
