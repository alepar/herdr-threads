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
use crate::sync::envelope::{Envelope, WireEnvelope};
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
            .into_iter()
            .map(|entry| entry.as_known().cloned().expect("a known entry"))
            .collect()
    }

    fn ingest(&self, from: &Node, log: &LogKey, first: u64) {
        let entries = from.entries(log, first);
        let entries = entries.into_iter().map(StoredEntry::from).collect();
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
            held: self.a3.clone().into(),
            conflicting: self.b3.clone().into(),
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
            entries: vec![WireItem::from(&self.b3)],
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
        entries: vec![big_wire(node, 5).into()],
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

/// The peer answers the `TipProof` probe of a `Have` that named b4 with the
/// signed b4 entry, and the worker handles the reply.
fn prove_b4_tip(w: &World, core: &mut Core, source: NodeUuid) {
    w.a.sink.on_branch(
        source,
        BranchReply::Entries {
            node: w.log.node,
            inc: w.log.incarnation,
            tip_hash: hex(&w.b4.hash),
            entries: vec![WireItem::from(&w.b4)],
            more: false,
        },
    );
    core.work();
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
            entries: vec![big_wire(w.log.node, w.log.incarnation).into()],
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
        held: Box::new(w.a3.clone().into()),
        conflicting: Box::new(w.b3.clone().into()),
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
        "the proof's tip was asked first, then the unsigned tip is probed alone"
    );
    prove_b4_tip(&w, &mut core, s);
    assert_eq!(
        w.a.sink.outbox().sends.len(),
        1,
        "the longer branch is pulled once its tip is proven"
    );
    let reply = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&w.b4.hash),
        entries: vec![WireItem::from(&w.b3), WireItem::from(&w.b4)],
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
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(5), pacer.wait(Duration::from_secs(60))).await
        });
    assert_eq!(wake.ok(), Some(crate::service::pacer::Wake::Kicked));
}

// Kills: a daemon that never installs the own-history hold's release kick on
// the Apply lane: the entries the hold held would then wait for the lane's 60 s
// safety tick (after a restore, a minute of redelivery-free but unapplied history).
#[test]
fn own_history_release_kicks_the_apply_lane() {
    let node = Node::new(SEED_A, 600, ManualClock::new(T0));
    let stop = Cancellation::default();
    let kicks = crate::service::kicks::CommitKicks::default();
    let lane = crate::app::start_apply_lane(
        &kicks,
        node.clock.clone(),
        stop.clone(),
        node.store.clone(),
        Box::new(|_| {}),
        node.writer.clone(),
        Arc::new(WorkerStatus::default()),
    )
    .expect("start the apply lane");
    let sync = node.store.context().sync().expect("journal open");
    let hold = sync.own_history_hold();
    // An idle event follows the complete startup drain. Startup must finish
    // before the release sets its hint, or startup could consume that hint
    // even if the release kick were missing.
    let pacer = kicks
        .pacer(Lane::Apply)
        .expect("the apply pacer is registered");
    let startup_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while pacer.idle_events() == 0 && std::time::Instant::now() < startup_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let startup_idle = pacer.idle_events();
    hold.set(HoldSource::Links, true, node.clock.monotonic_now());
    hold.set(HoldSource::Links, false, node.clock.monotonic_now());
    // The release asks for a pass; the lane takes the hint.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while (sync.apply_hint.load(Ordering::Acquire) || pacer.idle_events() <= startup_idle)
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let taken = !sync.apply_hint.load(Ordering::Acquire) && pacer.idle_events() > startup_idle;
    stop.cancel();
    kicks.kick(crate::service::kicks::LaneSet::EMPTY.with(Lane::Apply));
    lane.join().expect("the apply lane stops");
    assert!(
        startup_idle > 0,
        "the startup drain finished before release"
    );
    assert!(taken, "the lane ran a pass after the hold lifted");
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
    prove_b4_tip(&w, &mut core, s);
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
        entries: vec![WireItem::from(&w.b3)],
        more: true,
    };
    w.a.sink.on_branch(s, reply());
    // The answered request now holds the branch, so the sink refuses the
    // timer's second ask; a duplicate reply is queued directly (the stale
    // path stays a defence against one that still arrives).
    w.a.sink.note_tip_source(w.log, &hex(&w.b4.hash), s);
    assert_eq!(w.a.sink.request_branch(want.clone()), None);
    w.a.sink.enqueue(Job::Branch {
        source: s,
        want: want.clone(),
        reply: reply(),
    });
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
    let probed =
        w.a.sink
            .outbox()
            .sends
            .last()
            .expect("the tip was probed")
            .0;
    prove_b4_tip(&w, &mut core, probed);
    let sends = w.a.sink.outbox().sends;
    let (asked, want) = sends.last().cloned().expect("the b4 branch was requested");
    assert_eq!(want.tip.counter, 4, "{sends:?}");
    let other = if asked == s1 { s2 } else { s1 };

    let reply = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&w.b4.hash),
        entries: vec![WireItem::from(&w.b3)],
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
            entry: Box::new(self.tail.clone().into()),
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

// Kills: a Restore that waits for a genesis nobody serves beyond the 10 s proof
// window of an unsigned claim (it used to wait 5 minutes), and the
// single-copy case: I2 arriving later must leave lineage-branch evidence.
#[test]
fn restore_runs_after_10s_naming_i1_then_lineage_branch_recorded() {
    let w = restore_world();
    let mut core = w.restored.core();
    w.advertise_and_queue();
    core.work();
    w.restored.advance(9_000);
    core.tick();
    assert_eq!(w.restored.active_log(), w.i1, "still waiting at 9 s");
    w.restored.advance(1_000);
    core.tick();
    let active = w.restored.active_log();
    assert_ne!(active, w.i1, "ran when the unproven claim's 10 s ended");
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

// ---- Unsigned Have claims (ht-5fr.42) ----------------------------------------------

fn none_reply(log: &LogKey, tip: &[u8; 32]) -> BranchReply {
    BranchReply::None {
        node: log.node,
        inc: log.incarnation,
        tip_hash: hex(tip),
    }
}

// Kills: a Have-advertised tip of a frozen own log that creates a `pulling`
// branch row (and with it the own-history hold) before any signed entry shows
// it exists; a probe reply that is trusted without checking counter, hash and
// signature; a lie that is not penalized; an honest tip that never records.
#[test]
fn have_claiming_a_higher_frozen_own_tip_persists_nothing() {
    let w = world();
    let mut core = w.a.core();
    let s = peer(11);
    w.a.sink.enqueue(w.owner_fork(s));
    core.work();
    w.a.sink.on_branch(s, w.b3_reply());
    core.work();
    let rows = || w.a.count("SELECT COUNT(*) FROM sync_branches");
    let before = rows();
    assert!(!w.a.hold.active(w.a.now()));
    w.a.sink.outbox();

    // Lie 1: a tip nobody can serve (branch_none).
    let lie = [9u8; 32];
    w.a.sink.on_have(s, true, &w.have(4, &lie), &[]);
    core.work();
    assert_eq!(
        w.a.sink.outbox().sends,
        vec![(
            s,
            BranchWant {
                node: w.log.node,
                inc: w.log.incarnation,
                tip: TipRef {
                    counter: 4,
                    hash: hex(&lie)
                },
                from: 4,
            }
        )],
        "the tip alone is probed"
    );
    assert_eq!(rows(), before, "nothing is persisted before the proof");
    assert!(!w.a.hold.active(w.a.now()), "no own-history hold");
    w.a.sink.on_branch(s, none_reply(&w.log, &lie));
    core.work();
    assert_eq!(rows(), before);
    assert!(!w.a.hold.active(w.a.now()));
    assert!(w.a.sink.is_penalized(s, w.log), "the liar is penalized");
    w.a.sink.outbox();

    // Lie 2: an entry that is not at the claimed (counter, hash).
    let lie2 = [8u8; 32];
    w.a.sink.on_have(s, true, &w.have(4, &lie2), &[]);
    core.work();
    w.a.sink.outbox();
    let wrong = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&lie2),
        entries: vec![WireItem::from(&w.b3)],
        more: false,
    };
    w.a.sink.on_branch(s, wrong);
    core.work();
    assert_eq!(rows(), before);
    assert!(!w.a.hold.active(w.a.now()));

    // Lie 3: the right counter and hash, a forged signature.
    let mut forged = w.b4.clone();
    forged.signature[0] ^= 1;
    w.a.sink.on_have(s, true, &w.have(4, &forged.hash), &[]);
    core.work();
    w.a.sink.outbox();
    let reply = BranchReply::Entries {
        node: w.log.node,
        inc: w.log.incarnation,
        tip_hash: hex(&forged.hash),
        entries: vec![WireItem::from(&forged)],
        more: false,
    };
    w.a.sink.on_branch(s, reply);
    core.work();
    assert_eq!(rows(), before, "an unsigned entry proves nothing");
    assert!(!w.a.hold.active(w.a.now()));

    // The honest tip: a signed entry at exactly (4, b4).
    w.a.advance(61_000);
    let t = peer(12);
    w.a.sink.on_have(t, true, &w.have(4, &w.b4.hash), &[]);
    core.work();
    w.a.sink.outbox();
    prove_b4_tip(&w, &mut core, t);
    assert_eq!(rows(), before + 1, "the signed tip is recorded");
    assert!(w.a.hold.active(w.a.now()), "and its pull holds the history");
}

// Kills: clone evidence, a fork row, a branch row or a clone gate driven by
// Haves that name unknown own logs, foreign logs or wild heads.
#[test]
fn have_claims_record_no_clone_evidence() {
    let w = restore_world();
    let mut core = w.restored.core();
    let own = w.i1.node;
    let wild = |node: NodeUuid, inc: u32, counter: u64| HaveLog {
        node,
        inc,
        counter,
        head: hex(&[7; 32]),
        frozen_at: None,
    };
    let logs = vec![
        wild(own, 12_345, 50),
        wild(own, w.i1.incarnation, 40),
        wild(peer(5), 1, 9),
        wild(peer(6), 3, 1_000),
    ];
    w.restored.sink.on_have(w.src, true, &logs, &[]);
    core.work();
    core.tick();
    for table in [
        "sync_clone_evidence",
        "sync_forks",
        "sync_branches",
        "sync_reissues",
    ] {
        assert_eq!(
            w.restored.count(&format!("SELECT COUNT(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert!(!core.clone_held, "no clone gate");
    assert_eq!(w.restored.held_lines(), 0);
    assert!(!w.restored.hold.active(w.restored.now()));
    assert_eq!(w.restored.active_log(), w.i1);
}

// Kills: a restore rotation driven by a peer's claim of entries beyond the
// head rather than by a signed entry that extends it.
#[test]
fn lying_have_triggers_no_restore_rotation() {
    for forged_entry in [false, true] {
        let w = restore_world();
        let mut core = w.restored.core();
        // A restore successor is known: I2's genesis is here.
        w.restored.ingest(&w.orig, &w.i2, 1);
        let logs_before = w.restored.count("SELECT COUNT(*) FROM sync_logs");
        let fake = [7u8; 32];
        let lie = vec![HaveLog {
            node: w.i1.node,
            inc: w.i1.incarnation,
            counter: 2,
            head: hex(&fake),
            frozen_at: None,
        }];
        w.restored.sink.on_have(w.src, true, &lie, &[]);
        core.work();
        let sends = w.restored.sink.outbox().sends;
        assert!(
            sends
                .iter()
                .any(|(s, want)| *s == w.src && want.tip.hash == hex(&fake) && want.from == 2),
            "the claimed entry is asked for: {sends:?}"
        );
        let reply = if forged_entry {
            let mut bad = w.tail.clone();
            bad.signature[0] ^= 1;
            BranchReply::Entries {
                node: w.i1.node,
                inc: w.i1.incarnation,
                tip_hash: hex(&fake),
                entries: vec![WireItem::from(&bad)],
                more: false,
            }
        } else {
            none_reply(&w.i1, &fake)
        };
        w.restored.sink.on_branch(w.src, reply);
        core.work();
        w.restored.advance(1_000);
        core.tick();
        assert_eq!(
            w.restored.active_log(),
            w.i1,
            "no rotation (forged: {forged_entry})"
        );
        assert_eq!(
            w.restored.count("SELECT COUNT(*) FROM sync_logs"),
            logs_before
        );
        assert_eq!(core.pending.len(), 0, "no Restore stays queued");
        assert_eq!(
            w.restored.count("SELECT COUNT(*) FROM sync_clone_evidence"),
            0
        );
    }
}

// Kills: a per-source memory that one peer can fill, and a table that stops
// updating the heads it already holds once it is full.
#[test]
fn one_source_cannot_fill_active_heads() {
    let w = restore_world();
    let mut core = w.restored.core();
    let greedy = peer(11);
    for inc in 0..20u32 {
        let log = LogKey {
            node: w.i1.node,
            incarnation: 1_000 + inc,
        };
        core.note_active_head(greedy, log, 9, [1; 32]);
    }
    let held = |core: &Core, s: NodeUuid| core.active_heads.keys().filter(|(k, _)| *k == s).count();
    assert_eq!(held(&core, greedy), MAX_ACTIVE_HEADS_PER_SOURCE);
    // Others still have room, up to the global cap.
    for n in 0..(MAX_ACTIVE_HEADS as u8) {
        core.note_active_head(peer(100 + n), w.i1, 9, [2; 32]);
    }
    assert_eq!(core.active_heads.len(), MAX_ACTIVE_HEADS);
    // A head already held is still updated at the cap.
    core.note_active_head(peer(100), w.i1, 12, [3; 32]);
    assert_eq!(core.active_heads[&(peer(100), w.i1)], (12, [3; 32]));
}

// Kills: a Restore held back for 5 minutes by an advertised own log whose
// genesis nobody can serve.
#[test]
fn restore_ready_ignores_unproven_advertised_own_logs() {
    let w = restore_world();
    let mut core = w.restored.core();
    let phantom = HaveLog {
        node: w.i1.node,
        inc: 777,
        counter: 5,
        head: hex(&[4; 32]),
        frozen_at: None,
    };
    w.restored.sink.on_have(w.src, true, &[phantom], &[]);
    w.restored.sink.enqueue(Job::Restore {
        log: w.i1,
        source: w.src,
        entry: Box::new(w.tail.clone().into()),
    });
    core.work();
    w.restored.advance(9_000);
    core.tick();
    assert_eq!(w.restored.active_log(), w.i1, "waits inside the window");
    assert_eq!(core.pending.len(), 1);
    w.restored.advance(1_000);
    core.tick();
    assert_ne!(w.restored.active_log(), w.i1, "rotates after 10 s");
}

// ---- Opaque sides of a fork (ht-j87z.10) ----------------------------------------

/// An opaque entry of `log` signed by node A's key (the clone's key too).
fn opaque_of_a(log: LogKey, counter: u64, prev: [u8; 32], tag: &str) -> StoredEntry {
    use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
    let identity = NodeIdentity::from_seed(SEED_A);
    let mut unsigned = synthetic_unsigned(&identity, log.incarnation, counter, prev);
    unsigned.payload = tag.as_bytes().to_vec();
    StoredEntry::from(synthetic_entry(&identity, &unsigned))
}

/// A third node holding `known` entries of the world's log, in order, then
/// `extra` stored as given.
fn holder(w: &World, known: &[Envelope], extra: Vec<StoredEntry>) -> Node {
    let node = Node::new(SEED_C, 78, ManualClock::new(T0));
    let entries: Vec<StoredEntry> = known.iter().cloned().map(StoredEntry::from).collect();
    let entries = entries.into_iter().chain(extra).collect();
    match node.store.ingest(&w.log, entries, &budget()).unwrap() {
        IngestOutcome::Stored { .. } => {}
        other => panic!("holder ingest: {other:?}"),
    }
    node
}

// Kills: an opaque side that is held instead of repaired, a freeze at the
// wrong counter, the known rival lost, the opaque main-chain row moved or
// dropped, and a fork reference built from anything but counter and hash.
#[test]
fn fork_opaque_vs_known_same_counter_freezes() {
    let w = world();
    let prefix = w.a.entries(&w.log, 1);
    let opaque = opaque_of_a(w.log, 3, w.a3.prev_hash, "held");
    let c = holder(&w, &prefix[..2], vec![opaque.clone()]);
    let mut core = c.core();
    c.sink.enqueue(Job::Fork {
        log: w.log,
        held: Box::new(opaque.clone()),
        conflicting: Box::new(w.b3.clone().into()),
        source: peer(11),
    });
    core.work();
    assert_eq!(c.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(
        c.count(&format!(
            "SELECT frozen_at_counter FROM sync_logs WHERE incarnation={}",
            w.log.incarnation
        )),
        2,
        "frozen at x - 1, like a known pair"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal_forked WHERE counter=3"),
        1,
        "the known rival is the off-chain row"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal WHERE kind IS NULL AND counter=3"),
        1,
        "the opaque side stays the main-chain row"
    );
    assert_eq!(c.sink.resumed(), vec![w.log]);
    let refs = c.sink.fork_refs();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].held.hash, hex(&opaque.hash()));
    assert_eq!(refs[0].conflicting.hash, hex(&w.b3.hash));
    assert_eq!(refs[0].frozen_at, 2);
}

// Kills: the opposite direction handled differently: a known held side with an
// opaque rival must put the rival in journal_forked with an empty projection.
#[test]
fn fork_known_vs_opaque_rival_stores_branch_row() {
    let w = world();
    let prefix = w.a.entries(&w.log, 1);
    let c = holder(&w, &prefix, vec![]);
    let rival = opaque_of_a(w.log, 3, w.a3.prev_hash, "rival");
    let mut core = c.core();
    c.sink.enqueue(Job::Fork {
        log: w.log,
        held: Box::new(w.a3.clone().into()),
        conflicting: Box::new(rival.clone()),
        source: peer(11),
    });
    core.work();
    assert_eq!(c.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal_forked WHERE kind IS NULL AND counter=3"),
        1,
        "the opaque rival is an off-chain row"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal_forked WHERE kind IS NOT NULL"),
        0
    );
    assert_eq!(
        c.count(&format!(
            "SELECT frozen_at_counter FROM sync_logs WHERE incarnation={}",
            w.log.incarnation
        )),
        2
    );
}

// Kills: a prev-link break at the next counter that an opaque entry escapes
// (it must freeze like a known one), and a break signed by another key that
// would freeze a healthy log.
#[test]
fn fork_opaque_prev_link_break_freezes() {
    let w = world();
    let prefix = w.a.entries(&w.log, 1);
    let c = holder(&w, &prefix, vec![]);
    let breaking = opaque_of_a(w.log, 4, [0x42; 32], "break");
    let mut core = c.core();
    c.sink.enqueue(Job::Fork {
        log: w.log,
        held: Box::new(w.a3.clone().into()),
        conflicting: Box::new(breaking.clone()),
        source: peer(11),
    });
    core.work();
    assert_eq!(c.count("SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(
        c.count(&format!(
            "SELECT frozen_at_counter FROM sync_logs WHERE incarnation={}",
            w.log.incarnation
        )),
        2,
        "frozen below the entry the break proves"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal_forked WHERE kind IS NULL AND counter=4"),
        1
    );

    // The same break signed by another key is damage: nothing is recorded.
    let c2 = holder(&w, &prefix, vec![]);
    let mut core2 = c2.core();
    let mut forged = crate::test_support::opaque_entry::synthetic_unsigned(
        &NodeIdentity::from_seed([9; 32]),
        w.log.incarnation,
        4,
        [0x43; 32],
    );
    forged.log = w.log;
    let forged = StoredEntry::from(crate::test_support::opaque_entry::synthetic_entry(
        &NodeIdentity::from_seed([9; 32]),
        &forged,
    ));
    c2.sink.enqueue(Job::Fork {
        log: w.log,
        held: Box::new(w.a3.clone().into()),
        conflicting: Box::new(forged),
        source: peer(11),
    });
    core2.work();
    assert_eq!(c2.count("SELECT COUNT(*) FROM sync_forks"), 0);
    assert!(
        c2.lines().iter().any(|l| l.contains("is corrupt")),
        "{:?}",
        c2.lines()
    );
}

// Kills: an own-log restore entry of an unknown layout that rotates (it cannot
// be copied) or is dropped without a trace; the log must stay held and exactly
// one line must say why.
#[test]
fn restore_of_opaque_own_entry_is_reported() {
    use crate::sync::journal_port::ForkEvidence;
    use crate::sync::{LogHead, LogState};
    let w = restore_world();
    let mut core = w.restored.core();
    let entry = opaque_of_a(w.i1, 2, w.tail.prev_hash, "restore");
    let evidence = ForkEvidence::AheadOfSelf {
        log: w.i1,
        my_head: LogHead {
            counter: 1,
            head_hash: w.tail.prev_hash,
            state: LogState::Active,
            frozen_at: None,
        },
        entry: Box::new(entry),
    };
    assert_eq!(
        w.restored.sink.on_fork(w.i1, evidence, w.src),
        crate::sync::coordinator::ForkAck::Paused
    );
    core.work();
    assert_eq!(w.restored.active_log(), w.i1, "no restore rotation");
    assert_eq!(
        w.restored.lines(),
        vec![format!(
            "sync: own log {}/{} entry 2 has an unknown layout; cannot re-issue",
            uuid8(&w.i1.node),
            w.i1.incarnation
        )]
    );
    assert!(w.restored.sink.resumed().is_empty(), "the log stays held");
    assert_eq!(w.restored.reissues(), 0);
}

// Kills: an opaque entry on an own frozen log that is silently absent from the
// re-issue pass (it must be named once per change), one that is re-issued, and
// a repeated line on every pass.
#[test]
fn reissue_pass_reports_opaque_entries_once() {
    let w = world();
    let stray = opaque_of_a(w.log, 4, w.a3.hash, "stray");
    w.a.quantum(|tx, _ctx, _decision| {
        journal::store_forked_entries(tx, &w.log, std::slice::from_ref(&stray), None)
    });
    let mut core = w.a.core();
    w.a.sink.enqueue(w.owner_fork(peer(11)));
    core.work();
    assert_eq!(w.a.reissues(), 2, "the known entries re-issue as before");
    let line = format!(
        "sync: re-issue: 1 opaque entries of {}/{} cannot be re-issued",
        uuid8(&w.log.node),
        w.log.incarnation
    );
    assert_eq!(w.a.lines(), vec![line.clone()]);

    w.a.sink.enqueue(Job::Reissue);
    core.work();
    assert_eq!(
        w.a.lines(),
        vec![line],
        "an unchanged count is not repeated"
    );
    assert_eq!(w.a.reissues(), 2, "the opaque entry is never re-issued");
}
