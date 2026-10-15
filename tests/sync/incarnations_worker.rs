//! IncarnationsWorker tests: real `SqliteStore`s, a hand-driven sink and a
//! manual (or paused tokio) clock; no coordinator.

use super::*;

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::daemon::logs::LaneErrorLog;
use crate::node::NodeIdentity;
use crate::store::StoreSettings;
use crate::store::connection::{DecisionInstant, StoreContext};
use crate::store::journal;
use crate::sync::EntityKey;
use crate::sync::coordinator::{CloneKind, CloneReport, ForkSink};
use crate::sync::facts::Predecessor;
use crate::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry, GenesisPayload};
use crate::sync::journal_port::IngestOutcome;
use crate::test_support::isolation::TestIsolation;

const SEED_A: [u8; 32] = [1; 32];
const SEED_C: [u8; 32] = [3; 32];
const T0: i64 = 1_700_000_000_000;
const OPERATOR: &str = "operator:local-user:501";

#[derive(Serialize, Deserialize)]
struct Put {
    id: String,
}

struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(_: &Put) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        Ok(())
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<PutKind>())
}

/// A clock the test moves.
struct ManualClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl ManualClock {
    fn new(utc: i64) -> Arc<Self> {
        Arc::new(Self {
            mono: AtomicU64::new(1_000),
            utc: AtomicI64::new(utc),
        })
    }
    fn advance(&self, ms: u64) {
        self.mono.fetch_add(ms, Ordering::SeqCst);
        self.utc
            .fetch_add(i64::try_from(ms).unwrap(), Ordering::SeqCst);
    }
    fn set_utc(&self, utc: i64) {
        self.utc.store(utc, Ordering::SeqCst);
    }
}
impl Clock for ManualClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

/// A clock that follows tokio's (paused) time.
struct PausedClock {
    base: tokio::time::Instant,
}
impl PausedClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            base: tokio::time::Instant::now(),
        })
    }
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_millis()).unwrap()
    }
}
impl Clock for PausedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(T0 + i64::try_from(self.elapsed_ms()).unwrap())
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1_000 + self.elapsed_ms())
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn peer(n: u8) -> NodeUuid {
    NodeUuid::from_bytes([n; 16])
}

fn hex(hash: &[u8; 32]) -> String {
    hex_encode(hash)
}

/// One node: a real store, a sink and the worker's other collaborators.
struct Node {
    _isolation: TestIsolation,
    store: Arc<SqliteStore>,
    clock: Arc<dyn Clock>,
    manual: Option<Arc<ManualClock>>,
    sink: Arc<IncarnationsSink>,
    lines: Arc<Mutex<Vec<String>>>,
    status: Arc<WorkerStatus>,
    hold: Arc<OwnHistoryHold>,
    signals: Arc<SyncSignals>,
    writer: Arc<FairWriter>,
    cancel: Cancellation,
}

impl Node {
    fn new(seed: [u8; 32], incarnation_seed: u64, clock: Arc<ManualClock>) -> Self {
        let mut node = Self::with_clock(seed, incarnation_seed, clock.clone());
        node.manual = Some(clock);
        node
    }

    fn with_clock(seed: [u8; 32], incarnation_seed: u64, clock: Arc<dyn Clock>) -> Self {
        let isolation = TestIsolation::new("incarnations-worker");
        let ctx = StoreContext::new(isolation.state_root().join("store.db"), clock.clone())
            .with_journal_seed(seed);
        ctx.seed_incarnations(incarnation_seed);
        let id = Arc::new(NodeIdentity::from_seed(seed));
        {
            let mut writer = ctx.open_writer().expect("open a writer");
            journal::open(&ctx, &mut writer, id.clone(), "host", registry())
                .expect("open the journal");
        }
        let store = Arc::new(
            SqliteStore::new(ctx, id.uuid().to_string(), StoreSettings::default())
                .expect("open the store"),
        );
        let hold = store
            .context()
            .sync()
            .expect("journal open")
            .own_history_hold()
            .clone();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Self::new_sink(&store, &clock, &lines);
        Self {
            _isolation: isolation,
            store,
            clock,
            manual: None,
            sink,
            lines,
            status: Arc::new(WorkerStatus::default()),
            hold,
            signals: Arc::new(SyncSignals::new()),
            writer: Arc::new(FairWriter::new(64)),
            cancel: Cancellation::default(),
        }
    }

    fn new_sink(
        store: &Arc<SqliteStore>,
        clock: &Arc<dyn Clock>,
        lines: &Arc<Mutex<Vec<String>>>,
    ) -> Arc<IncarnationsSink> {
        let lines = lines.clone();
        let log: LogFn = Arc::new(move |line| lines.lock().unwrap().push(line.to_owned()));
        Arc::new(IncarnationsSink::new(store.clone(), clock.clone(), log))
    }

    /// A fresh sink, as after a daemon restart (in-memory state is empty).
    fn restart_sink(&mut self) {
        self.sink = Self::new_sink(&self.store, &self.clock, &self.lines);
    }

    fn deps(&self) -> WorkerDeps {
        let lines = self.lines.clone();
        WorkerDeps {
            sink: self.sink.clone(),
            journal: self.store.clone(),
            store: self.store.clone(),
            writer: self.writer.clone(),
            status: self.status.clone(),
            own_history: self.hold.clone(),
            signals: self.signals.clone(),
            sync_pacer: None,
            clock: self.clock.clone(),
            log: Arc::new(move |line| lines.lock().unwrap().push(line.to_owned())),
            cancel: self.cancel.clone(),
        }
    }

    fn core(&self) -> Core {
        Core::new(self.deps())
    }

    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn advance(&self, ms: u64) {
        self.manual.as_ref().expect("a manual clock").advance(ms);
    }

    fn now(&self) -> MonoInstant {
        self.clock.monotonic_now()
    }

    fn put(&self, id: &str) {
        let ctx = self.store.context();
        let mut conn = ctx.open_writer().expect("open a writer");
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| journal::author::<PutKind>(tx, ctx, d, &Put { id: id.into() }),
        )
        .expect("author a put");
    }

    fn rotate_to(&self, incarnation: u32, predecessor: Option<Predecessor>) {
        let ctx = self.store.context();
        let mut conn = ctx.open_writer().expect("open a writer");
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| journal::begin_incarnation(tx, ctx, d, incarnation, predecessor, "host"),
        )
        .expect("begin an incarnation");
    }

    fn active_log(&self) -> LogKey {
        let db = self.store.context().open_query(budget()).unwrap();
        let (node, incarnation): (String, u32) = db
            .query_row(
                "SELECT l.node_uuid,l.incarnation FROM local_node n \
                 JOIN sync_logs l ON l.log_ix=n.active_log_ix",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        LogKey {
            node: node.parse().unwrap(),
            incarnation,
        }
    }

    fn entries(&self, log: &LogKey, from: u64) -> Vec<Envelope> {
        self.store
            .serve(log, from, BatchLimits::default(), &budget())
            .unwrap()
    }

    fn ingest(&self, from: &Node, log: &LogKey, first: u64) {
        let entries = from.entries(log, first);
        match self.store.ingest(log, entries, &budget()).unwrap() {
            IngestOutcome::Stored { .. } => {}
            other => panic!("ingest of {log:?}: {other:?}"),
        }
    }

    fn count(&self, sql: &str) -> i64 {
        let db = self.store.context().open_query(budget()).unwrap();
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn reissues(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM sync_reissues")
    }

    fn evidence(&self, kind: &str) -> i64 {
        self.count(&format!(
            "SELECT COUNT(*) FROM sync_clone_evidence WHERE kind='{kind}'"
        ))
    }

    fn held_lines(&self) -> usize {
        self.lines().iter().filter(|l| *l == HELD_LINE).count()
    }

    /// One repair function as a writer quantum outside the worker (the
    /// operator's path, or state the worker is started on later).
    fn quantum<R>(
        &self,
        f: impl FnOnce(&Transaction<'_>, &StoreContext, DecisionInstant) -> Result<R, ApiError>,
    ) -> R {
        self.store.incarnations_quantum(&budget(), f).unwrap()
    }

    fn genesis_predecessor(&self, log: &LogKey) -> Option<Predecessor> {
        let db = self.store.context().open_query(budget()).unwrap();
        let payload: Vec<u8> = db
            .query_row(
                "SELECT j.payload FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=1",
                rusqlite::params![log.node.to_string(), log.incarnation],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_slice::<GenesisPayload>(&payload)
            .unwrap()
            .predecessor
    }
}

/// Node A (the owner) with a log `I` holding puts at counters 2 and 3, a
/// clone `b` of A that wrote a different entry at counter 3 (and one more at
/// 4), and a third node `c` that holds A's chain.
struct World {
    a: Node,
    c: Node,
    log: LogKey,
    a3: Envelope,
    b3: Envelope,
    b4: Envelope,
}

fn world() -> World {
    let clock = ManualClock::new(T0);
    world_on(Node::new(SEED_A, 600, clock))
}

fn world_on(a: Node) -> World {
    a.put("x");
    a.put("y");
    let b = Node::new(SEED_A, 600, ManualClock::new(T0));
    b.put("x");
    b.put("z");
    b.put("w");
    let log = a.active_log();
    assert_eq!(log, b.active_log(), "the clone holds the same log");
    let a3 = a.entries(&log, 3).remove(0);
    let b3 = b.entries(&log, 3).remove(0);
    let b4 = b.entries(&log, 4).remove(0);
    assert_ne!(a3.hash, b3.hash);
    assert_eq!(a3.prev_hash, b3.prev_hash, "one parent: a fork at 3");
    let c = Node::new(SEED_C, 77, ManualClock::new(T0));
    c.ingest(&a, &log, 1);
    World {
        a,
        c,
        log,
        a3,
        b3,
        b4,
    }
}

impl World {
    fn proof(&self) -> Proof {
        Proof {
            log: self.log,
            x: 3,
            held: self.a3.clone(),
            conflicting: self.b3.clone(),
        }
    }

    fn owner_fork(&self, source: NodeUuid) -> Job {
        Job::OwnerFork {
            proof: Box::new(self.proof()),
            source: Some(source),
        }
    }

    fn have(&self, counter: u64, head: &[u8; 32]) -> Vec<HaveLog> {
        vec![HaveLog {
            node: self.log.node,
            inc: self.log.incarnation,
            counter,
            head: hex(head),
            frozen_at: None,
        }]
    }

    fn b3_reply(&self) -> BranchReply {
        BranchReply::Entries {
            node: self.log.node,
            inc: self.log.incarnation,
            tip_hash: hex(&self.b3.hash),
            entries: vec![WireEnvelope::from(&self.b3)],
            more: false,
        }
    }

    fn b3_want(&self) -> BranchWant {
        BranchWant {
            node: self.log.node,
            inc: self.log.incarnation,
            tip: TipRef {
                counter: 3,
                hash: hex(&self.b3.hash),
            },
            from: 3,
        }
    }

    fn clone_report(&self) -> CloneReport {
        CloneReport {
            subject: self.log.node,
            kind: CloneKind::Handshake,
            boots: Vec::new(),
            source: peer(11),
        }
    }
}

// ---- Queue policy and timers ---------------------------------------------------

// Kills: a queue that runs every Have, coalesces across sources, or keeps the
// older Have of a source.
#[test]
fn have_jobs_coalesce_per_source() {
    let w = world();
    let mut core = w.a.core();
    let (s1, s2) = (peer(11), peer(12));
    w.a.sink.on_have(s1, true, &w.have(2, &w.a3.prev_hash), &[]);
    w.a.sink.on_have(s2, true, &w.have(3, &w.a3.hash), &[]);
    w.a.sink.on_have(s1, true, &w.have(3, &w.b3.hash), &[]);
    core.intake();
    assert_eq!(core.pending.len(), 2, "one Have per source");
    let heads: Vec<(NodeUuid, String)> = core
        .pending
        .iter()
        .filter_map(|q| match &q.job {
            Job::Have { source, logs, .. } => Some((*source, logs[0].head.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        heads,
        vec![(s1, hex(&w.b3.hash)), (s2, hex(&w.a3.hash))],
        "the latest Have of s1 won, in s1's place"
    );
}

/// A 3 MiB `branch_entries` answer for a fake branch of a log nobody holds.
fn big_reply(byte: u8) -> (BranchWant, BranchReply) {
    let node = peer(77);
    let tip = TipRef {
        counter: 9,
        hash: hex(&[byte; 32]),
    };
    let want = BranchWant {
        node,
        inc: 5,
        tip: tip.clone(),
        from: 4,
    };
    let reply = BranchReply::Entries {
        node,
        inc: 5,
        tip_hash: tip.hash,
        entries: vec![big_wire(node, 5)],
        more: false,
    };
    (want, reply)
}

fn big_wire(node: NodeUuid, inc: u32) -> WireEnvelope {
    let payload = format!("\"{}\"", "x".repeat(3 << 20));
    WireEnvelope {
        node: node.to_string(),
        inc,
        counter: 4,
        prev: hex(&[0; 32]),
        hlc: 1,
        kind: "test.put".into(),
        ver: 1,
        deps: String::new(),
        creates: None,
        needs: Vec::new(),
        payload: serde_json::value::RawValue::from_string(payload).unwrap(),
        hash: hex(&[1; 32]),
        sig: hex(&[2; 32]),
    }
}

/// Asks `source` for `want` and delivers `reply` as its answer.
fn answer(w: &World, source: NodeUuid, want: &BranchWant, reply: BranchReply) {
    w.a.sink
        .note_tip_source(BranchKey::of(want).log, &want.tip.hash, source);
    assert_eq!(w.a.sink.request_branch(want.clone()), Some(source));
    w.a.sink.on_branch(source, reply);
}

// Kills: no cap, a cap that drops the newest job, a log line per drop instead
// of per minute, and a dropped branch that is never asked for again.
#[test]
fn cap_8mib_drops_oldest_coalescible_logs_once_per_minute_rerequests() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    // The real pulling branch: the owner rotates, with `s` as the source.
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert_eq!(
        w.a.sink.outbox().sends,
        vec![(s, w.b3_want())],
        "the branch was asked for"
    );

    // Its answer is the oldest queued job; two more 3 MiB answers follow.
    w.a.sink.on_branch(
        s,
        BranchReply::Entries {
            node: w.log.node,
            inc: w.log.incarnation,
            tip_hash: hex(&w.b3.hash),
            entries: vec![big_wire(w.log.node, w.log.incarnation)],
            more: false,
        },
    );
    for byte in [8, 9] {
        let (want, reply) = big_reply(byte);
        answer(&w, s, &want, reply);
    }
    core.intake();
    assert_eq!(core.pending.len(), 2, "the oldest was dropped");
    assert!(core.pending_bytes <= CAP_BYTES);
    assert!(
        core.pending.iter().all(
            |q| !matches!(&q.job, Job::Branch { want, .. } if want.tip.hash == hex(&w.b3.hash))
        ),
        "the dropped job was the oldest, the real branch's answer"
    );
    let overflow_lines = |w: &World| {
        w.a.lines()
            .iter()
            .filter(|l| l.contains("queue over 8 MiB"))
            .count()
    };
    assert_eq!(overflow_lines(&w), 1);

    // More overflow inside the minute writes no second line.
    for byte in [10, 11] {
        let (want, reply) = big_reply(byte);
        answer(&w, s, &want, reply);
    }
    core.intake();
    assert_eq!(overflow_lines(&w), 1, "one line per minute");
    // After the minute it writes one more.
    w.a.advance(60_000);
    for byte in [12, 13] {
        let (want, reply) = big_reply(byte);
        answer(&w, s, &want, reply);
    }
    core.intake();
    assert_eq!(overflow_lines(&w), 2);

    // The next sweep asks for the dropped branch again.
    core.tick();
    let again = w.a.sink.outbox().sends;
    assert!(again.contains(&(s, w.b3_want())), "re-requested: {again:?}");
}

// Kills: a timer that never fails a request over, one that fails over early
// (before 30 s) and a sweep driven by anything but the worker's own 1 s timer.
#[tokio::test(start_paused = true)]
async fn timer_fails_over_unanswered_request_after_30s() {
    let clock = PausedClock::new();
    let w = world_on(Node::with_clock(SEED_A, 600, clock.clone()));
    let (s1, s2) = (peer(11), peer(12));
    w.a.sink.note_tip_source(w.log, &hex(&w.b3.hash), s2);
    let mut core = w.a.core();
    w.a.sink.enqueue(w.owner_fork(s1));
    core.work();
    let first = w.a.sink.outbox().sends;
    assert_eq!(first.len(), 1, "{first:?}");
    let (asked, want) = first[0].clone();
    let other = if asked == s1 { s2 } else { s1 };

    let task = tokio::spawn(run(core));
    tokio::time::sleep(Duration::from_secs(29)).await;
    assert!(
        w.a.sink.outbox().sends.is_empty(),
        "no failover before 30 s"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        w.a.sink.outbox().sends,
        vec![(other, want)],
        "the unanswered request moved to the other source"
    );
    w.a.cancel.cancel();
    task.await.unwrap();
}

struct CountingLog(AtomicU64);
impl LaneErrorLog for CountingLog {
    fn record(&self, lane: Lane, _: &ApiError) {
        assert_eq!(lane, Lane::Sync);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// Kills: a failure that is not recorded on Lane::Sync, a retry before the
// backoff, a backoff that does not double or exceeds 60 s, and a recovery that
// leaves Health failed.
#[test]
fn failing_job_records_sync_failure_and_backs_off_1_to_60s() {
    let clock = ManualClock::new(T0);
    let a = Node::new(SEED_A, 600, clock.clone());
    let failures = Arc::new(CountingLog(AtomicU64::new(0)));
    a.status.set_error_log(failures.clone());
    let failed = |a: &Node| a.status.health().is_some_and(|h| h.failure.is_some());
    let mut core = a.core();
    a.store.set_lane_fault(Some(Arc::new(|lane| {
        (lane == Some(Lane::Sync)).then(|| ApiError::new(ErrorCode::StoreBusy, "injected"))
    })));
    let report = CloneReport {
        subject: peer(1),
        kind: CloneKind::Handshake,
        boots: Vec::new(),
        source: peer(2),
    };
    a.sink.enqueue(Job::Clone(report));
    core.work();
    assert_eq!(failures.0.load(Ordering::SeqCst), 1);
    assert!(failed(&a), "Health shows the lane failure");

    let mut delays = Vec::new();
    for round in 2..=8_u64 {
        let delay = core.next_job_retry().unwrap().0 - a.now().0;
        delays.push(delay);
        clock.advance(delay - 1);
        core.work();
        assert_eq!(failures.0.load(Ordering::SeqCst), round - 1, "too early");
        clock.advance(1);
        core.work();
        assert_eq!(
            failures.0.load(Ordering::SeqCst),
            round,
            "retried at {delay}"
        );
    }
    assert_eq!(
        delays,
        vec![1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 60_000],
        "1 s doubling to the 60 s cap"
    );
    assert_eq!(
        core.next_job_retry().unwrap().0 - a.now().0,
        60_000,
        "the cap holds"
    );

    // The fault clears: the kept job runs, and Health recovers.
    a.store.set_lane_fault(None);
    clock.advance(60_000);
    core.work();
    assert_eq!(a.evidence("handshake"), 1, "the job was kept and ran");
    assert!(!failed(&a), "recovery clears the failure");
    assert_eq!(core.next_job_retry(), None);
    assert!(core.pending.is_empty());
}

// Kills: a retryable failure of one job that blocks every other job behind a
// worker-wide backoff.
#[test]
fn one_failing_job_does_not_block_the_others() {
    let clock = ManualClock::new(T0);
    let a = Node::new(SEED_A, 600, clock.clone());
    let mut core = a.core();
    let bad_dir = a._isolation.path("not-a-dir");
    std::fs::write(&bad_dir, b"").unwrap();
    a.sink.enqueue(Job::Salvage {
        backup_db: a._isolation.path("no-backup.db"),
        node_dir: bad_dir,
    });
    let clone = |n: u8| {
        Job::Clone(CloneReport {
            subject: peer(n),
            kind: CloneKind::Handshake,
            boots: Vec::new(),
            source: peer(2),
        })
    };
    a.sink.enqueue(clone(1));
    core.work();
    assert_eq!(a.evidence("handshake"), 1, "the clone job ran");
    assert!(
        core.next_job_retry().is_some(),
        "the salvage job is pending with its own backoff"
    );
    assert!(a.status.health().is_some_and(|h| h.failure.is_some()));

    clock.advance(1_000);
    a.sink.enqueue(clone(3));
    core.work();
    assert_eq!(a.evidence("handshake"), 2, "a later job still completes");
}

// Kills: a salvage that removes the marker after a Headroom stop, a stop that
// is not reported or not retried, and a retry that does not finish the rebuild.
#[test]
fn salvage_out_of_headroom_keeps_marker_and_retries_until_it_fits() {
    let clock = ManualClock::new(T0);
    let a = Node::new(SEED_A, 600, clock.clone());
    let c = Node::new(SEED_C, 77, ManualClock::new(T0));
    c.put("x");
    c.put("y");
    let dir = a._isolation.state_root().join("node-dir");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join(crate::sync::resync::MARKER_FILE);
    std::fs::write(&marker, b"{}").unwrap();
    let c_uuid = NodeIdentity::from_seed(SEED_C).uuid().to_string();
    let held = format!("SELECT COUNT(*) FROM sync_logs WHERE node_uuid='{c_uuid}'");
    let failed = |a: &Node| a.status.health().is_some_and(|h| h.failure.is_some());

    a.store.context().set_ingest_headroom(u64::MAX);
    let mut core = a.core();
    a.sink.enqueue(Job::Salvage {
        backup_db: c._isolation.state_root().join("store.db"),
        node_dir: dir.clone(),
    });
    core.work();
    assert!(marker.exists(), "the marker stays");
    assert!(
        a.lines()
            .iter()
            .any(|l| l.contains("marker kept: no disk headroom")),
        "{:?}",
        a.lines()
    );
    assert!(failed(&a), "Health shows the stop");
    assert_eq!(a.count(&held), 0, "nothing was stored");

    a.store.context().set_ingest_headroom(0);
    clock.advance(60_000);
    core.work();
    assert!(!marker.exists(), "the retry finished the rebuild");
    assert!(a.lines().iter().any(|l| l.contains("marker removed")));
    assert_eq!(a.count(&held), 1, "c's log is read back");
}

// ---- Third-party and owner paths ------------------------------------------------

// Kills: a conflict that is not recorded, a freeze at the wrong counter, a log
// left paused, and no refs_changed for the coordinator's delta.
#[test]
fn conflict_job_records_fork_resumes_sets_refs_changed() {
    let w = world();
    let mut core = w.c.core();
    w.c.sink.enqueue(Job::Fork {
        log: w.log,
        held: Box::new(w.a3.clone()),
        conflicting: Box::new(w.b3.clone()),
        source: peer(11),
    });
    core.work();
    assert_eq!(w.c.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(
        w.c.count(&format!(
            "SELECT frozen_at_counter FROM sync_logs WHERE incarnation={}",
            w.log.incarnation
        )),
        2,
        "frozen at x - 1"
    );
    assert_eq!(w.c.sink.resumed(), vec![w.log], "the log is resumed");
    let out = w.c.sink.outbox();
    assert!(out.refs_changed, "a new fork is advertised");
    let refs = w.c.sink.fork_refs();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].conflicting.hash, hex(&w.b3.hash));
    assert_eq!(refs[0].frozen_at, 2);
}

// Kills: a Have mismatch that fetches more than the one entry, a missing
// request, and a matching reply that records nothing.
#[test]
fn have_head_mismatch_single_entry_branch_want_then_fork_recorded() {
    let w = world();
    let mut core = w.c.core();
    let s = peer(11);
    w.c.sink.on_have(s, true, &w.have(3, &w.b3.hash), &[]);
    core.work();
    assert_eq!(
        w.c.sink.outbox().sends,
        vec![(s, w.b3_want())],
        "one branch_want for the single differing entry"
    );
    assert_eq!(w.c.count("SELECT COUNT(*) FROM sync_forks"), 0);

    w.c.sink.on_branch(s, w.b3_reply());
    core.work();
    assert_eq!(w.c.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(w.c.count("SELECT COUNT(*) FROM journal_forked"), 1);
    assert_eq!(w.c.sink.resumed(), vec![w.log]);

    // The memo keeps the same Have from asking again.
    w.c.sink.on_have(s, true, &w.have(3, &w.b3.hash), &[]);
    core.work();
    assert!(w.c.sink.outbox().sends.is_empty(), "memoized");
}

// Kills: a forwarded fork reference that is ignored, one that asks for the side
// the node already holds, and a pair that is recorded without both sides.
#[test]
fn forwarded_fork_ref_fetches_missing_side_then_records() {
    let w = world();
    let mut core = w.c.core();
    let s = peer(11);
    let fork = ForkRef {
        node: w.log.node,
        inc: w.log.incarnation,
        held: TipRef {
            counter: 3,
            hash: hex(&w.a3.hash),
        },
        conflicting: TipRef {
            counter: 3,
            hash: hex(&w.b3.hash),
        },
        frozen_at: 2,
    };
    w.c.sink.on_have(s, true, &[], &[fork]);
    core.work();
    assert_eq!(
        w.c.sink.outbox().sends,
        vec![(s, w.b3_want())],
        "only the missing side is requested"
    );
    w.c.sink.on_branch(s, w.b3_reply());
    core.work();
    assert_eq!(w.c.count("SELECT COUNT(*) FROM sync_forks"), 1);
}

// Kills: an owner proof that records instead of rotating, a freeze at the
// wrong counter, no branch pull, and a re-issue that waits for the pull.
#[test]
fn own_log_proof_owner_rotate_schedules_pulls() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    let now_active = w.a.active_log();
    assert_ne!(now_active, w.log, "rotated away from the forked log");
    assert_eq!(
        w.a.genesis_predecessor(&now_active),
        Some(Predecessor {
            incarnation: w.log.incarnation,
            frozen_at: Some(2)
        })
    );
    assert_eq!(w.a.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(
        w.a.count("SELECT COUNT(*) FROM sync_branches WHERE state='pulling'"),
        1
    );
    assert_eq!(
        w.a.sink.outbox().sends,
        vec![(s, w.b3_want())],
        "the conflicting branch is pulled from the proof's source"
    );
    assert_eq!(w.a.sink.resumed(), vec![w.log]);
    assert_eq!(
        w.a.reissues(),
        2,
        "the owner's tail and the proof's other entry re-issue at once"
    );
}

// Kills: a re-issue that does not run after a pull commit, one that re-issues
// the other branch twice, and a branch left pulling.
#[test]
fn reissue_runs_after_pull_commits() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert_eq!(w.a.reissues(), 2, "a3 and the proof's b3");
    // The source's head is further along its branch: a tip beyond the proof.
    w.a.sink.on_have(s, true, &w.have(4, &w.b4.hash), &[]);
    core.work();
    assert_eq!(
        w.a.sink.outbox().sends.len(),
        2,
        "the proof's tip was asked first, then the longer branch"
    );
    let reply = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&w.b4.hash),
        entries: vec![WireEnvelope::from(&w.b3), WireEnvelope::from(&w.b4)],
        more: false,
    };
    w.a.sink.on_branch(s, reply);
    core.work();
    assert_eq!(w.a.reissues(), 3, "b4 re-issues after the pull committed");
    assert_eq!(w.a.count("SELECT COUNT(*) FROM journal_forked"), 2);
    assert_eq!(
        w.a.count("SELECT COUNT(*) FROM sync_branches WHERE state='complete' AND tip_counter=4"),
        1
    );
}

// Kills: a worker that commits a rotation and re-issues under its `Lane::Sync`
// origin without telling the coordinator: that origin keeps a commit from kicking
// its own lane, so peers would not hear of the new incarnation's entries until
// some other commit happened.
#[test]
fn quanta_kick_the_sync_pacer() {
    let w = world();
    let pacer = Arc::new(crate::service::pacer::Pacer::new(
        "sync",
        w.a.clock.clone(),
        Cancellation::default(),
    ));
    let mut deps = w.a.deps();
    deps.sync_pacer = Some(pacer.clone());
    let mut core = Core::new(deps);
    w.a.sink.enqueue(w.owner_fork(peer(11)));
    core.work();
    assert!(w.a.reissues() > 0, "the owner rotated and re-issued");
    // The kick is latched: the wait ends at once. (Without it the manual clock
    // never reaches the pacer's deadline, so the real-time bound ends the wait.)
    let wake = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(tokio::time::timeout(
            Duration::from_secs(5),
            pacer.wait(Duration::from_secs(60)),
        ));
    assert_eq!(wake.ok(), Some(crate::service::pacer::Wake::Kicked));
}

// Kills: a reply to a request the branch row has moved past judged against the
// row's new position: the 1 s timer asks again for the same `from` while the
// first reply still waits in the queue, the first reply moves the row on, and
// the second then read as corrupt and penalized the only source.
#[test]
fn stale_branch_reply_is_dropped_not_penalized() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    w.a.sink.on_have(s, true, &w.have(4, &w.b4.hash), &[]);
    core.work();
    let (_, want) =
        w.a.sink
            .outbox()
            .sends
            .last()
            .cloned()
            .expect("the longer branch was requested");
    assert_eq!((want.tip.counter, want.from), (4, 3));
    // The first part of the branch, with more to come.
    let reply = || BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&w.b4.hash),
        entries: vec![WireEnvelope::from(&w.b3)],
        more: true,
    };
    w.a.sink.on_branch(s, reply());
    // The timer's pull runs before the worker saw the reply: same `from` again.
    w.a.sink.note_tip_source(w.log, &hex(&w.b4.hash), s);
    assert_eq!(w.a.sink.request_branch(want.clone()), Some(s));
    w.a.sink.on_branch(s, reply());
    core.work();

    assert!(
        w.a.lines().iter().all(|l| !l.contains("rejected")),
        "{:?}",
        w.a.lines()
    );
    let asked = w.a.sink.outbox().sends;
    assert!(
        asked
            .iter()
            .any(|(source, ask)| *source == s && ask.from == 4),
        "the rest is asked of the same source: {asked:?}"
    );
}

// Kills: a Branches flag that never sets, one set for a completed branch and
// one that stays set after completion.
#[test]
fn branches_flag_set_while_pulling_cleared_on_completion() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    assert!(!w.a.hold.active(w.a.now()));
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert!(
        w.a.hold.active(w.a.now()),
        "set while the branch is pulling"
    );
    w.a.sink.on_branch(s, w.b3_reply());
    core.work();
    assert!(!w.a.hold.active(w.a.now()), "cleared on completion");
}

// Kills: a restart that forgets pulling branches or unfinished re-issues.
#[test]
fn restart_resumes_pulling_and_unfinished_reissues() {
    let mut w = world();
    let s = peer(11);
    // State a crash left: rotated and recorded, the tail not re-issued yet.
    let proof = w.proof();
    w.a.quantum(|tx, ctx, d| store::owner_rotate(tx, ctx, d, &proof, Some(s)));
    assert_eq!(w.a.reissues(), 0);
    assert!(!w.a.hold.active(w.a.now()));

    w.a.restart_sink();
    let mut core = w.a.core();
    core.start();
    assert_eq!(w.a.reissues(), 2, "the unfinished re-issue ran at start");
    assert!(
        w.a.hold.active(w.a.now()),
        "the pulling branch holds the own history again"
    );
    // The first Have naming the tip gives the sink a source again.
    w.a.sink.on_have(s, true, &w.have(3, &w.b3.hash), &[]);
    core.work();
    assert_eq!(w.a.sink.outbox().sends, vec![(s, w.b3_want())]);
}

// Kills: a worker thread that ignores cancellation.
#[test]
fn stops_on_cancel() {
    let a = Node::new(SEED_A, 600, ManualClock::new(T0));
    let handle = IncarnationsWorker::start(a.deps()).expect("spawn the worker");
    assert_eq!(handle.thread().name(), Some("herdr-incarnations"));
    a.cancel.cancel();
    for _ in 0..100 {
        if handle.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(handle.is_finished(), "the worker thread stopped");
    handle.join().expect("the worker did not panic");
}

// ---- Clone gate -----------------------------------------------------------------

// Kills: an answered request that stops holding its branch before the worker
// handled the reply (any pull in between repeats the same `from`, and the
// duplicate reply is then rejected as corrupt, penalizing the only source).
#[test]
fn answered_request_holds_the_branch_until_the_reply_is_handled() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert_eq!(w.a.sink.outbox().sends, vec![(s, w.b3_want())]);

    // The reply is in the sink (answered) and waits in the job queue.
    w.a.sink.on_branch(s, w.b3_reply());
    core.schedule_pulls().expect("schedule pulls");
    assert!(
        w.a.sink.outbox().sends.is_empty(),
        "no second request for the branch while its reply is unhandled"
    );

    core.work();
    assert_eq!(w.a.reissues(), 2, "the reply was handled once");
    assert!(w.a.lines().iter().all(|l| !l.contains("rejected")));
}

// Kills: a source that holds less than the tip being treated as done (the
// pull never moves to another source).
#[test]
fn short_branch_reply_moves_to_another_source() {
    let w = world();
    let mut core = w.a.core();
    let (s1, s2) = (peer(11), peer(12));
    w.a.sink.enqueue(w.owner_fork(s1));
    core.work();
    w.a.sink.on_branch(s1, w.b3_reply());
    core.work();
    w.a.sink.note_tip_source(w.log, &hex(&w.b4.hash), s1);
    w.a.sink.note_tip_source(w.log, &hex(&w.b4.hash), s2);
    w.a.sink.on_have(s1, true, &w.have(4, &w.b4.hash), &[]);
    core.work();
    let sends = w.a.sink.outbox().sends;
    let (asked, want) = sends.last().cloned().expect("the b4 branch was requested");
    assert_eq!(want.tip.counter, 4, "{sends:?}");
    let other = if asked == s1 { s2 } else { s1 };

    let reply = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&w.b4.hash),
        entries: vec![WireEnvelope::from(&w.b3)],
        more: false,
    };
    w.a.sink.on_branch(asked, reply);
    core.work();
    assert_eq!(
        w.a.sink.outbox().sends,
        vec![(other, BranchWant { from: 4, ..want })],
        "the rest is asked of the other source"
    );
    assert_eq!(
        w.a.count("SELECT COUNT(*) FROM sync_branches WHERE state='pulling'"),
        1
    );
}

// Kills: a Clone job that is not recorded and an owner rotation that runs on a
// clone suspect, or logs more than once an hour.
#[test]
fn clone_job_records_evidence_and_owner_fork_rotates_nothing() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(Job::Clone(w.clone_report()));
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert_eq!(w.a.evidence("handshake"), 1);
    assert_eq!(w.a.active_log(), w.log, "no rotation");
    assert_eq!(w.a.count("SELECT COUNT(*) FROM sync_forks"), 0);
    assert_eq!(w.a.held_lines(), 1);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    assert_eq!(w.a.held_lines(), 1, "one line per hour");
    assert_eq!(
        w.a.sink.resumed(),
        Vec::<LogKey>::new(),
        "the log stays paused"
    );
}

// Kills: a clearance that does not re-run the re-issue pass, keeps the Have
// memos or never asks for a round.
#[test]
fn after_clear_reissue_and_request_round() {
    let mut w = world();
    let s = peer(11);
    let proof = w.proof();
    w.a.quantum(|tx, ctx, d| store::owner_rotate(tx, ctx, d, &proof, Some(s)));
    let report = w.clone_report();
    w.a.quantum(|tx, _ctx, _d| store::record_clone_evidence(tx, &report, UtcMillis(T0)));

    w.a.restart_sink();
    let mut core = w.a.core();
    core.start();
    assert_eq!(w.a.reissues(), 0, "the gate holds the re-issue");
    assert!(core.clone_held);
    assert_eq!(w.a.held_lines(), 1);

    w.a.sink.set_have_memo(s, w.log, 3, hex(&w.b3.hash));
    assert!(w.a.signals.take_request().is_none());
    w.a.quantum(|tx, _ctx, _d| store::clear_clone_evidence(tx, OPERATOR, UtcMillis(T0)));
    w.a.sink.notify_clone_cleared();
    core.work();
    assert_eq!(
        w.a.reissues(),
        2,
        "the re-issue pass ran after the clearance"
    );
    assert_eq!(w.a.sink.have_memo(s, w.log), None, "memos forgotten");
    assert!(
        w.a.signals.take_request().is_some(),
        "a round was requested"
    );
}

fn restored_with_unknown_newer() -> (Node, Node) {
    let orig = Node::new(SEED_A, 600, ManualClock::new(T0));
    orig.advance(1_000);
    orig.rotate_to(501, None);
    let restored = Node::new(SEED_A, 600, ManualClock::new(T0));
    let newer = LogKey {
        node: orig.active_log().node,
        incarnation: 501,
    };
    restored.ingest(&orig, &newer, 1);
    // The restored node moved on after the rotation: no restore successor.
    restored.manual.as_ref().unwrap().set_utc(T0 + 100_000);
    restored.put("late");
    (orig, restored)
}

// Kills: no evidence for an own log nothing names, or evidence only at start.
#[test]
fn own_unknown_newer_recorded_after_have_job() {
    let (_orig, restored) = restored_with_unknown_newer();
    let mut core = restored.core();
    assert_eq!(restored.evidence("own_unknown_newer"), 0);
    restored.sink.on_have(peer(11), true, &[], &[]);
    core.work();
    assert_eq!(restored.evidence("own_unknown_newer"), 1);
}

// ---- Restore older than a rotation ----------------------------------------------

struct RestoreWorld {
    orig: Node,
    restored: Node,
    i1: LogKey,
    i2: LogKey,
    tail: Envelope,
    src: NodeUuid,
}

fn restore_world() -> RestoreWorld {
    let orig = Node::new(SEED_A, 600, ManualClock::new(T0));
    orig.put("p");
    let i1 = orig.active_log();
    orig.advance(1_000);
    orig.rotate_to(
        501,
        Some(Predecessor {
            incarnation: i1.incarnation,
            frozen_at: None,
        }),
    );
    let i2 = orig.active_log();
    assert_ne!(i1, i2);
    let restored = Node::new(SEED_A, 600, ManualClock::new(T0));
    assert_eq!(restored.active_log(), i1, "the backup holds the first log");
    let tail = orig.entries(&i1, 2).remove(0);
    RestoreWorld {
        orig,
        restored,
        i1,
        i2,
        tail,
        src: peer(11),
    }
}

impl RestoreWorld {
    /// The peer advertises I1's tail and I2, and I1's restore entry is queued.
    fn advertise_and_queue(&self) {
        let genesis2 = self.orig.entries(&self.i2, 1).remove(0);
        let logs = vec![
            HaveLog {
                node: self.i1.node,
                inc: self.i1.incarnation,
                counter: 2,
                head: hex(&self.tail.hash),
                frozen_at: None,
            },
            HaveLog {
                node: self.i2.node,
                inc: self.i2.incarnation,
                counter: 1,
                head: hex(&genesis2.hash),
                frozen_at: None,
            },
        ];
        self.restored.sink.on_have(self.src, true, &logs, &[]);
        self.restored.sink.enqueue(Job::Restore {
            log: self.i1,
            source: self.src,
            entry: Box::new(self.tail.clone()),
        });
    }
}

// Kills: a Restore that rotates before the advertised I2 genesis arrives, one
// that names I1 instead of the successor I2, and clone evidence for the
// ordinary restore-older-than-a-rotation case.
#[test]
fn restore_waits_for_i2_genesis_then_names_i2_no_evidence() {
    let w = restore_world();
    let mut core = w.restored.core();
    w.advertise_and_queue();
    core.work();
    assert_eq!(
        w.restored.active_log(),
        w.i1,
        "I2's genesis is not here yet"
    );
    assert_eq!(core.pending.len(), 1, "the Restore waits");

    w.restored.ingest(&w.orig, &w.i2, 1);
    w.restored.advance(1_000);
    core.tick();
    let active = w.restored.active_log();
    assert_ne!(active, w.i1, "rotated");
    assert_eq!(
        w.restored.genesis_predecessor(&active),
        Some(Predecessor {
            incarnation: w.i2.incarnation,
            frozen_at: None
        }),
        "the new log names the newest log of the lineage"
    );
    assert_eq!(
        w.restored.count("SELECT COUNT(*) FROM sync_clone_evidence"),
        0
    );
    assert_eq!(w.restored.sink.resumed(), vec![w.i1]);
}

// Kills: a Restore that waits forever for a genesis nobody serves, and the
// single-copy case: I2 arriving later must leave lineage-branch evidence.
#[test]
fn restore_runs_at_5_min_naming_i1_then_lineage_branch_recorded() {
    let w = restore_world();
    let mut core = w.restored.core();
    w.advertise_and_queue();
    core.work();
    w.restored.advance(299_000);
    core.tick();
    assert_eq!(w.restored.active_log(), w.i1, "still waiting at 299 s");
    w.restored.advance(1_000);
    core.tick();
    let active = w.restored.active_log();
    assert_ne!(active, w.i1, "ran at 5 minutes");
    assert_eq!(
        w.restored.genesis_predecessor(&active),
        Some(Predecessor {
            incarnation: w.i1.incarnation,
            frozen_at: None
        }),
        "no successor was known: it names I1"
    );
    assert_eq!(
        w.restored.count("SELECT COUNT(*) FROM sync_clone_evidence"),
        0
    );

    // I2's genesis arrives late: two geneses now name I1.
    w.restored.ingest(&w.orig, &w.i2, 1);
    core.tick();
    assert_eq!(w.restored.evidence("lineage_branch"), 1);
}
