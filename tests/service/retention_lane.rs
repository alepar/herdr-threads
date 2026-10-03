//! The retention lane (nested spec D4): idle commits, backoff, cancellation,
//! backlog shutdown, kick isolation and Health reporting. The store is real;
//! the clock is fake, so a tick is one `advance`.
use super::*;
use crate::{
    ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
        HostObservation, HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance,
        SnapshotHeader, StructuralOccupancy,
    },
    protocol::{
        ids::{HostBootId, HostCallId, HostTargetId},
        time::{MonoInstant, UtcMillis},
    },
    service::kicks::{CommitKicks, LaneSet},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicI64, AtomicU64},
    time::Instant,
};

const TICK_MS: u64 = 60_000;
const HOUR_MS: i64 = 60 * 60 * 1000;

struct LaneClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl Clock for LaneClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

struct Records(Mutex<Vec<(Lane, ApiError)>>);
impl LaneErrorLog for Records {
    fn record(&self, lane: Lane, error: &ApiError) {
        self.0.lock().unwrap().push((lane, error.clone()));
    }
}

/// A store, a fake clock and the pieces of one retention lane; the lane
/// thread starts with `start`.
struct LaneFx {
    _iso: TestIsolation,
    path: PathBuf,
    clock: Arc<LaneClock>,
    store: Arc<SqliteStore>,
    kicks: Arc<CommitKicks>,
    writer: Arc<FairWriter>,
    pacer: Arc<Pacer>,
    status: Arc<WorkerStatus>,
    cancel: Cancellation,
    handle: Option<thread::JoinHandle<()>>,
}

impl LaneFx {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let path = iso.state_root().join("store.db");
        let clock = Arc::new(LaneClock {
            mono: AtomicU64::new(1),
            utc: AtomicI64::new(1_000 * HOUR_MS),
        });
        let kicks = Arc::new(CommitKicks::default());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), clock.clone()),
                "i",
                StoreSettings::default(),
            )
            .unwrap()
            .with_commit_kicks(Arc::clone(&kicks)),
        );
        let cancel = Cancellation::default();
        let pacer = Arc::new(Pacer::new("retention", clock.clone(), cancel.clone()));
        Self {
            _iso: iso,
            path,
            clock,
            store,
            kicks,
            writer: Arc::new(FairWriter::new(32)),
            pacer,
            status: Arc::new(WorkerStatus::default()),
            cancel,
            handle: None,
        }
    }

    fn start(&mut self) {
        self.kicks
            .register(Lane::Retention, Arc::clone(&self.pacer));
        self.handle = Some(
            start_retention_worker(
                self.store.clone(),
                Arc::clone(&self.writer),
                Arc::clone(&self.pacer),
                self.cancel.clone(),
                Arc::clone(&self.status),
            )
            .unwrap(),
        );
    }

    /// Moves the fake clock forward and wakes the lane's pacer.
    fn advance(&self, mono_ms: u64) {
        self.clock.mono.fetch_add(mono_ms, Ordering::SeqCst);
        self.clock.utc.fetch_add(mono_ms as i64, Ordering::SeqCst);
        self.pacer.clock_advanced();
    }

    /// Waits until the lane has entered its `n`-th wait (finished `n` passes
    /// or backoff waits).
    fn wait_idle(&self, n: u64) {
        wait_until("the lane to wait", || self.pacer.idle_events() >= n);
    }

    fn retention_commits(&self) -> u64 {
        self.store.commit_counts()["retention"]
    }

    fn raw(&self) -> Connection {
        let db = Connection::open(&self.path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    fn generations(&self) -> i64 {
        self.raw()
            .query_row("SELECT count(*) FROM snapshot_generations", [], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

impl Drop for LaneFx {
    fn drop(&mut self) {
        self.stop();
    }
}

fn observation(sequence: u64) -> HostObservation {
    HostObservation {
        target: HostTargetId::new("pane"),
        focused: false,
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::CoherentEnumeration,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("capture-{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    }
}

/// One observation publish of a single pane, as the observation lane does it.
fn publish(store: &SqliteStore, sequence: u64) {
    let admission = StorePort::begin_host_observation(store, "i", &budget()).unwrap();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: vec![observation(sequence)],
    };
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = StorePort::begin_snapshot_stage(store, header, &budget()).unwrap();
    StorePort::stage_snapshot_targets(
        store,
        &stage.id,
        0,
        &snapshot.targets,
        DurableWorkAdmission::new(16).unwrap(),
        &budget(),
    )
    .unwrap();
    StorePort::seal_snapshot_stage(store, &stage.id, &budget()).unwrap();
    StorePort::publish_snapshot_stage(store, &stage.id, &budget()).unwrap();
}

/// Raw discarded generations with `targets` targets each, from another
/// connection (a prunable backlog).
fn seed_backlog(db: &Connection, generations: i64, targets: usize) {
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0) ON CONFLICT(id) DO NOTHING",
        [],
    )
    .unwrap();
    db.execute_batch("BEGIN").unwrap();
    let mut generation = db
        .prepare("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,admission_sequence,created_at) VALUES (?1,'i','b',1,?2,'inc',?3,?3,'discarded',0,0,?2,0)")
        .unwrap();
    let mut target = db
        .prepare("INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES (?1,?2,1,1,'unknown','unknown',0)")
        .unwrap();
    for n in 1..=generations {
        let id = format!("g{n}");
        generation
            .execute(rusqlite::params![id, n, targets as i64])
            .unwrap();
        for t in 0..targets {
            target
                .execute(rusqlite::params![id, format!("t{t}")])
                .unwrap();
        }
    }
    drop((generation, target));
    db.execute_batch("COMMIT").unwrap();
}

#[test]
fn idle_zero_retention_commits_over_two_ticks() {
    let mut fx = LaneFx::new("lane-idle");
    fx.start();
    fx.wait_idle(1);
    for tick in 1..=2u64 {
        fx.advance(TICK_MS);
        fx.wait_idle(1 + tick);
    }
    assert_eq!(fx.pacer.idle_events(), 3, "one pass per tick, no spinning");
    assert_eq!(
        fx.retention_commits(),
        0,
        "no publication and nothing due: no Retention-origin commit"
    );
    assert!(fx.status.last_tick().is_some(), "each pass is a good pass");
    assert!(fx.status.health().is_none());
}

#[test]
fn at_most_one_commit_per_tick_while_observation_publishes() {
    let mut fx = LaneFx::new("lane-publishing");
    for sequence in 1..=3 {
        publish(&fx.store, sequence);
    }
    fx.start();
    fx.wait_idle(1);
    assert_eq!(fx.generations(), 3, "active, previous and baseline");
    for tick in 1..=5u64 {
        let before = fx.retention_commits();
        publish(&fx.store, 3 + tick);
        fx.advance(TICK_MS);
        fx.wait_idle(1 + tick);
        let commits = fx.retention_commits() - before;
        assert_eq!(
            commits, 1,
            "tick {tick}: one prune commit for one superseded generation"
        );
        assert_eq!(fx.generations(), 3, "tick {tick}");
    }
}

/// The store failure every failure test injects: deleting a generation aborts.
fn inject_failure(db: &Connection) {
    db.execute_batch(
        "CREATE TRIGGER inject_prune_failure BEFORE DELETE ON snapshot_generations BEGIN SELECT RAISE(ABORT,'injected'); END",
    )
    .unwrap();
}

fn clear_failure(db: &Connection) {
    db.execute_batch("DROP TRIGGER inject_prune_failure")
        .unwrap();
}

fn seed_one_superseded(fx: &LaneFx) {
    for sequence in 1..=4 {
        publish(&fx.store, sequence);
    }
    assert_eq!(fx.generations(), 4);
}

#[test]
fn store_failure_backs_off_and_resets() {
    let mut fx = LaneFx::new("lane-backoff");
    seed_one_superseded(&fx);
    inject_failure(&fx.raw());
    fx.start();
    let mut idle = 1;
    for attempt in 1..=11u32 {
        fx.wait_idle(idle);
        wait_until("the failure to be counted", || {
            fx.pacer.attempts() == attempt
        });
        let delay = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
        let nominal = (100u64 << (attempt - 1)).min(30_000);
        let low = (nominal as f64 * 0.8) as u64;
        let high = ((nominal as f64 * 1.2) as u64).min(30_000);
        assert!(
            (low..=high).contains(&delay),
            "attempt {attempt}: retry in {delay} ms, expected {low}..={high}"
        );
        // The pass fails again only once the backoff is due.
        let before = fx.pacer.idle_events();
        thread::sleep(Duration::from_millis(15));
        assert_eq!(fx.pacer.idle_events(), before, "not retried before due");
        idle = before + 1;
        if attempt < 11 {
            fx.advance(delay);
        }
    }
    clear_failure(&fx.raw());
    // Now the retry succeeds, resets the backoff and prunes.
    let due = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
    fx.advance(due);
    // The worker resets the pacer before `record_success` clears Health.
    wait_until("the retry to succeed and clear the failure", || {
        fx.pacer.attempts() == 0 && fx.status.health().is_none()
    });
    assert!(fx.status.retry().is_none());
    wait_until("the pruned generation", || fx.generations() == 3);
}

#[test]
fn failure_reaches_lane_error_log_and_health_retry_suffix() {
    let mut fx = LaneFx::new("lane-health");
    seed_one_superseded(&fx);
    let log = Arc::new(Records(Mutex::default()));
    fx.status.set_error_log(log.clone());
    inject_failure(&fx.raw());
    fx.start();
    fx.wait_idle(1);
    // The pacer counts the failure before `record_failure` publishes it to
    // Health and then to the error log.
    wait_until("the failure", || {
        fx.pacer.attempts() == 1 && !log.0.lock().unwrap().is_empty()
    });
    {
        let records = log.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].0, Lane::Retention);
    }
    let summary = fx
        .status
        .health()
        .expect("a failed pass degrades")
        .summary();
    assert!(summary.starts_with("lane retention failed:"), "{summary}");
    assert!(summary.contains("; retrying (attempt 1,"), "{summary}");
    assert!(!summary.contains('\n'), "Health keeps one line: {summary}");
    assert!(!summary.contains("injected"), "no store detail: {summary}");
    clear_failure(&fx.raw());
    let due = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
    fx.advance(due);
    // `record_success` clears Health, then advances the tick, after the
    // pacer reset; a success that skips either times this wait out.
    wait_until("the retry to succeed", || {
        fx.pacer.attempts() == 0 && fx.status.last_tick().is_some()
    });
    assert!(fx.status.health().is_none());
}

#[test]
fn cancellation_stops_blocked_lane_within_20ms() {
    let mut fx = LaneFx::new("lane-cancel");
    fx.start();
    fx.wait_idle(1);
    let started = Instant::now();
    fx.cancel.cancel();
    let handle = fx.handle.take().unwrap();
    wait_until("the lane to stop", || handle.is_finished());
    let stopped_in = started.elapsed();
    handle.join().unwrap();
    assert!(
        stopped_in < Duration::from_millis(20),
        "a blocked lane took {stopped_in:?} to stop"
    );
    assert!(
        !fx.status.lane_dead(),
        "a requested stop is not a dead lane"
    );
}

#[test]
fn shutdown_during_backlog_stops_after_current_batch() {
    let mut fx = LaneFx::new("lane-shutdown");
    // 10^4 generations of 10 targets: about 430 batches of at most 256 rows.
    seed_backlog(&fx.raw(), 10_000, 10);
    fx.start();
    wait_until("the drain to begin", || fx.retention_commits() >= 3);
    let at_cancel = fx.retention_commits();
    fx.stop();
    let at_stop = fx.retention_commits();
    assert!(
        at_stop - at_cancel <= 1,
        "{} batches ran after cancellation",
        at_stop - at_cancel
    );
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fx.retention_commits(),
        at_stop,
        "no transaction starts after the stop"
    );
    let remaining = fx.generations();
    assert!(
        remaining > 0 && remaining < 10_000,
        "stopped mid-backlog: {remaining} generations remain"
    );
    assert!(!fx.status.lane_dead());
}

#[test]
fn retention_prune_commit_kicks_no_lane() {
    type Flushed = Arc<Mutex<Vec<(LaneSet, Option<Lane>)>>>;
    let flushed: Flushed = Arc::default();
    let record = |fx: &LaneFx| {
        let flushed = Arc::clone(&flushed);
        fx.store.set_kick_sink(Box::new(move |lanes, origin| {
            flushed.lock().unwrap().push((lanes, origin));
        }));
    };
    let old_job = |db: &Connection| {
        db.execute(
            "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) VALUES ('w','send_attention','w',0,'complete',1)",
            [],
        )
        .unwrap();
    };

    // Control: a prune that deletes a work job commits a table the deadline
    // lane watches. Run as request origin it kicks that lane.
    let control = LaneFx::new("lane-kicks-control");
    record(&control);
    old_job(&control.raw());
    let mut seen = StorePort::prune_retention(&*control.store, &budget()).unwrap();
    assert_eq!(seen.jobs, 1);
    assert_eq!(
        flushed.lock().unwrap().as_slice(),
        [(LaneSet::EMPTY.with(Lane::Deadlines), None)],
        "the control commit does kick the deadline lane"
    );
    flushed.lock().unwrap().clear();

    // The retention lane's commits (generations, targets and a job) kick nothing.
    let mut fx = LaneFx::new("lane-kicks");
    record(&fx);
    seed_backlog(&fx.raw(), 5, 3);
    old_job(&fx.raw());
    fx.start();
    wait_until("the prune", || fx.generations() == 0);
    fx.wait_idle(1);
    seen = StorePort::prune_retention(&*fx.store, &budget()).unwrap();
    assert_eq!(seen, crate::ports::PruneProgress::default());
    assert!(fx.retention_commits() >= 1);
    assert!(
        flushed.lock().unwrap().is_empty(),
        "retention-origin commits kicked {:?}",
        flushed.lock().unwrap()
    );
}

#[test]
fn spawned_by_production_startup_and_prunes_within_one_tick() {
    let mut fx = LaneFx::new("lane-production");
    for sequence in 1..=4 {
        publish(&fx.store, sequence);
    }
    // The helper the elected factory calls: it registers the Pacer with the
    // commit-kick registry and attaches it to the lane's WorkerStatus.
    assert!(!fx.kicks.registered(Lane::Retention));
    let status = Arc::new(WorkerStatus::default());
    let production_cancel = Cancellation::default();
    let handle = crate::app::start_retention_lane(
        &fx.kicks,
        fx.clock.clone(),
        production_cancel.clone(),
        fx.store.clone(),
        Arc::clone(&fx.writer),
        Arc::clone(&status),
    )
    .unwrap();
    assert!(fx.kicks.registered(Lane::Retention));
    // The startup pass prunes the superseded generation.
    wait_until("the startup pass", || fx.generations() == 3);
    wait_until("the first good pass", || status.last_tick().is_some());
    // One tick later (fake clock) a new publication's predecessor is pruned.
    publish(&fx.store, 5);
    assert_eq!(fx.generations(), 4);
    fx.clock.mono.fetch_add(TICK_MS, Ordering::SeqCst);
    fx.kicks.kick(LaneSet::EMPTY.with(Lane::Retention));
    wait_until("the tick pass", || fx.generations() == 3);
    // The registered Pacer is the one attached to the status: an injected
    // failure shows as a backoff on it, and a kick after the due time retries.
    inject_failure(&fx.raw());
    publish(&fx.store, 6);
    fx.clock.mono.fetch_add(TICK_MS, Ordering::SeqCst);
    fx.kicks.kick(LaneSet::EMPTY.with(Lane::Retention));
    wait_until("the backoff", || status.retry().is_some());
    let (attempts, next) = status.retry().unwrap();
    assert_eq!(attempts, 1);
    clear_failure(&fx.raw());
    fx.clock.mono.store(next.0, Ordering::SeqCst);
    fx.kicks.kick(LaneSet::EMPTY.with(Lane::Retention));
    wait_until("the retry", || status.retry().is_none());
    wait_until("the pruned generation", || fx.generations() == 3);
    // Shutdown ends the lane without marking it dead.
    production_cancel.cancel();
    handle.join().unwrap();
    assert!(!status.lane_dead());
    fx.stop();
}
