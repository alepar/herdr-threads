use herdr_threads::{
    ports::*,
    protocol::{
        ids::*,
        results::{ApiError, ErrorCode},
        time::*,
    },
    service::{
        archival::ArchivalWorker, fair_writer::FairWriter, host_reachability::HostReachability,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Host {
    path: std::path::PathBuf,
    writer: Arc<FairWriter>,
    calls: AtomicUsize,
    cancel: Cancellation,
    cancel_on_read: std::sync::atomic::AtomicBool,
    negative_failure: AtomicUsize,
}
impl HostPort for Host {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        panic!("plain observation cannot qualify archival")
    }
    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            context.expected_boot.as_ref().map(HostBootId::as_str),
            Some("b")
        );
        assert_eq!(context.expected_epoch, Some(1));
        let probe_clock = RunningClock(std::time::Instant::now());
        let probe_budget = CallBudget {
            deadline: MonoInstant(200),
            cancellation: context.budget.cancellation.clone(),
        };
        let _turn = self
            .writer
            .enter_foreground(&probe_budget, &probe_clock)
            .expect("composer read must not own FairWriter");
        let db = rusqlite::Connection::open(&self.path).unwrap();
        db.busy_timeout(std::time::Duration::ZERO).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; ROLLBACK")
            .expect("composer read must not own SQLite writer");
        let negative = self.negative_failure.load(Ordering::SeqCst);
        if negative == 3 {
            return Ok(super::channel_archival::sample(
                context.budget.deadline.0.saturating_sub(5_000) as i64,
                HostUiState::ActiveTurn,
            ));
        }
        if negative != 0 {
            db.execute_batch("CREATE TRIGGER fail_archival_sample BEFORE UPDATE OF idle_mono ON seat_archival BEGIN SELECT RAISE(ABORT,'injected sample write failure'); END").unwrap();
            if negative == 2 {
                return Err(ApiError::new(ErrorCode::NotFound, "target vanished"));
            }
            return Ok(super::channel_archival::sample(
                100,
                HostUiState::HumanInput,
            ));
        }
        if self.cancel_on_read.load(Ordering::SeqCst) {
            self.cancel.cancel();
            Err(ApiError::new(
                ErrorCode::Cancelled,
                "owned host read cancelled",
            ))
        } else {
            Err(ApiError::new(
                ErrorCode::Unsupported,
                "unsupported composer",
            ))
        }
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        unreachable!()
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        unreachable!()
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        unreachable!()
    }
}
struct RunningClock(std::time::Instant);
impl Clock for RunningClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.elapsed().as_millis() as u64)
    }
}
fn worker_fixture() -> (
    ArchivalWorker,
    Arc<Host>,
    rusqlite::Connection,
    super::handoff_fences::Directory,
) {
    worker_fixture_from_store(super::handoff_fences::store_fixture())
}
fn worker_fixture_from_store(
    fixture: super::handoff_fences::StoreFixture,
) -> (
    ArchivalWorker,
    Arc<Host>,
    rusqlite::Connection,
    super::handoff_fences::Directory,
) {
    let writer = Arc::new(FairWriter::new(32));
    let cancellation = Cancellation::default();
    let host = Arc::new(Host {
        path: fixture._directory.0.join("store.db"),
        writer: writer.clone(),
        calls: AtomicUsize::new(0),
        cancel: cancellation.clone(),
        cancel_on_read: true.into(),
        negative_failure: AtomicUsize::new(0),
    });
    let context = herdr_threads::daemon::paths::RuntimeContext::explicit(
        fixture._directory.0.clone(),
        fixture._directory.0.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = herdr_threads::daemon::paths::InstancePaths::resolve_read_only(&context).unwrap();
    std::fs::create_dir_all(&paths.instance_dir).unwrap();
    let source =
        herdr_threads::archival_legacy::Source::new(&paths, "i".into(), Default::default());
    let reachability = Arc::new(HostReachability::default());
    reachability.mark_archival_published(100);
    let worker = ArchivalWorker {
        store: Arc::new(fixture.store),
        host: host.clone(),
        writer,
        reachability,
        source,
        boot: "worker".into(),
        after_ms: 3_600_000,
        cancellation,
    };
    (worker, host, fixture.db, fixture._directory)
}
#[test]
fn archival_worker_host_read_releases_both_writers_and_shutdown_owns_cancellation() {
    let (mut worker, host, db, _directory) = worker_fixture();
    assert_eq!(worker.run_page().unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM seat_archival WHERE idle_mono IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn archival_worker_initial_unknown_does_not_read_host_and_parked_lane_stops() {
    let (mut worker, host, _db, _directory) = worker_fixture();
    worker.reachability.mark_archival_uncertain();
    assert!(!worker.run_page().unwrap());
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    worker.after_ms = 0;
    let cancellation = worker.cancellation.clone();
    let status = Arc::new(herdr_threads::service::workers::WorkerStatus::default());
    let pacer = Arc::new(herdr_threads::service::pacer::Pacer::new(
        "archival",
        Arc::new(RunningClock(std::time::Instant::now())),
        cancellation.clone(),
    ));
    let handle = herdr_threads::service::archival::start(worker, pacer, status.clone()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while status.last_tick().is_none() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    cancellation.cancel();
    handle.join().unwrap();
    assert!(status.last_tick().is_some());
    assert!(!status.lane_dead());
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn archival_worker_lets_a_commit_burst_settle_before_its_pass() {
    use herdr_threads::service::archival::KICK_SETTLE;
    use std::{sync::mpsc, time::Duration};
    let (mut worker, host, _db, _directory) = worker_fixture();
    // A pass that touches neither the host nor the store: only its timing matters.
    worker.after_ms = 0;
    let cancellation = worker.cancellation.clone();
    let status = Arc::new(herdr_threads::service::workers::WorkerStatus::default());
    let pacer = Arc::new(herdr_threads::service::pacer::Pacer::new(
        "archival",
        Arc::new(RunningClock(std::time::Instant::now())),
        cancellation.clone(),
    ));
    let (idle_tx, idle_rx) = mpsc::channel();
    pacer.set_idle_hook(Box::new(move |_| {
        let _ = idle_tx.send(std::time::Instant::now());
    }));
    let handle =
        herdr_threads::service::archival::start(worker, pacer.clone(), status.clone()).unwrap();
    idle_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("boot pass parks the lane");
    for _ in 0..2 {
        let kicked = std::time::Instant::now();
        pacer.kick();
        let parked = idle_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("a kicked lane runs its pass");
        assert!(
            parked.duration_since(kicked) >= KICK_SETTLE,
            "the pass ran {:?} after the kick, inside the settle window",
            parked.duration_since(kicked)
        );
        assert!(
            idle_rx.recv_timeout(KICK_SETTLE * 2).is_err(),
            "one kick runs one pass"
        );
    }
    pacer.kick();
    std::thread::sleep(Duration::from_millis(20));
    let cancelled = std::time::Instant::now();
    cancellation.cancel();
    handle.join().unwrap();
    assert!(
        cancelled.elapsed() < KICK_SETTLE,
        "cancellation ends the settle wait"
    );
    assert!(!status.lane_dead());
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn archival_worker_before_host_admission_waits_for_safety_tick() {
    use herdr_threads::{
        service::{
            kicks::{CommitKicks, Lane},
            pacer::Pacer,
            workers::WorkerStatus,
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use std::{
        sync::{atomic::AtomicU64, mpsc},
        time::{Duration, Instant},
    };

    struct ManualClock(AtomicU64);
    impl Clock for ManualClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0.load(Ordering::SeqCst) as i64)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }
    struct OwnedLane(Cancellation, Option<std::thread::JoinHandle<()>>);
    impl Drop for OwnedLane {
        fn drop(&mut self) {
            self.0.cancel();
            self.1.take().unwrap().join().unwrap();
        }
    }

    let directory = super::handoff_fences::Directory(
        std::env::temp_dir().join(format!("archival-startup-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let clock = Arc::new(ManualClock(AtomicU64::new(100)));
    let cancellation = Cancellation::default();
    let pacer = Arc::new(Pacer::new("archival", clock.clone(), cancellation.clone()));
    let kicks = Arc::new(CommitKicks::default());
    kicks.register(Lane::Archival, pacer.clone());
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings::default(),
    )
    .unwrap()
    .with_commit_kicks(kicks);
    let db = rusqlite::Connection::open(path).unwrap();
    let (mut worker, host, db, _directory) =
        worker_fixture_from_store(super::handoff_fences::StoreFixture {
            store,
            db,
            _directory: directory,
        });
    worker.cancellation = cancellation.clone();
    worker.reachability.mark_archival_uncertain();
    let store = worker.store.clone();
    let (idle_tx, idle_rx) = mpsc::channel();
    pacer.set_idle_hook(Box::new(move |n| {
        let _ = idle_tx.send(n);
    }));
    let status = Arc::new(WorkerStatus::default());
    let lane = OwnedLane(
        cancellation.clone(),
        Some(
            herdr_threads::service::archival::start(worker, pacer.clone(), status.clone()).unwrap(),
        ),
    );
    idle_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let count = || {
        db.query_row("SELECT count(*) FROM archival_instances", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    assert_eq!(count(), 0, "boot pass precedes canonical host admission");
    store
        .begin_host_observation(
            "i",
            &CallBudget {
                deadline: MonoInstant(10_000),
                cancellation,
            },
        )
        .unwrap();
    let evaluations = pacer.evaluations();
    clock.0.store(5_100, Ordering::SeqCst);
    pacer.clock_advanced();
    let until = Instant::now() + Duration::from_secs(2);
    while pacer.evaluations() <= evaluations {
        assert!(
            Instant::now() < until,
            "lane did not evaluate advanced clock"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        count(),
        0,
        "host admission does not kick the parked archival lane"
    );
    assert_eq!(pacer.wakes(), 0, "five seconds is before the safety tick");
    clock.0.store(60_100, Ordering::SeqCst);
    pacer.clock_advanced();
    idle_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        count(),
        1,
        "safety tick initializes after real host admission"
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    assert!(status.health().is_none());
    drop(lane);
}
#[test]
fn archival_worker_failure_reports_health_and_cancellation_ends_backoff() {
    let (worker, host, db, _directory) = worker_fixture();
    host.cancel_on_read.store(false, Ordering::SeqCst);
    let cancellation = worker.cancellation.clone();
    let status = Arc::new(herdr_threads::service::workers::WorkerStatus::default());
    let pacer = Arc::new(herdr_threads::service::pacer::Pacer::new(
        "archival",
        Arc::new(RunningClock(std::time::Instant::now())),
        cancellation.clone(),
    ));
    let handle = herdr_threads::service::archival::start(worker, pacer, status.clone()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while status.health().is_none() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    cancellation.cancel();
    handle.join().unwrap();
    assert!(status.health().is_some());
    assert!(!status.lane_dead());
    assert!(host.calls.load(Ordering::SeqCst) > 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM seat_archival WHERE idle_mono IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn archival_worker_negative_survives_failed_persistence_before_next_final_decision() {
    use herdr_threads::store::archival::{self, Runtime};
    for negative in [1, 2] {
        let (mut worker, host, mut db, _directory) = worker_fixture();
        worker.after_ms = 100;
        let mut rt = Runtime {
            boot: "worker".into(),
            mono: 0,
            utc: UtcMillis(0),
            after_ms: 100,
            host_generation: 1,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered".into()),
        };
        archival::advance(&db, "i", &rt).unwrap();
        for at in [0, 100] {
            rt.mono = at;
            rt.utc = UtcMillis(at);
            let ticket = archival::observation_ticket(&db, "i", "s", &rt)
                .unwrap()
                .unwrap();
            let mut sample = super::channel_archival::sample(at, HostUiState::Idle);
            sample.0.observation_sequence = at as u64 + 2;
            assert!(archival::record_sample(&mut db, &ticket, &rt, &sample).unwrap());
        }
        for _ in 0..10 {
            let phase: i64 = db
                .query_row(
                    "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            if phase == 4 {
                break;
            }
            archival::advance(&db, "i", &rt).unwrap();
        }
        assert_eq!(
            db.query_row(
                "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4
        );
        db.execute(
            "UPDATE seat_archival SET next_mono=100 WHERE seat_id='s'",
            [],
        )
        .unwrap();
        host.negative_failure.store(negative, Ordering::SeqCst);
        let error = worker.run_page().unwrap_err();
        assert_ne!(
            error.code,
            ErrorCode::NotFound,
            "sample persistence must be the injected failure"
        );
        assert_eq!(
            db.query_row(
                "SELECT idle_mono,last_mono,samples FROM seat_archival WHERE seat_id='s'",
                [],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?
                ))
            )
            .unwrap(),
            (0, 100, 2),
            "failed deciding write must actually leave the old positive sample durable"
        );
        assert_eq!(
            db.query_row(
                "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            5
        );
        db.execute_batch("DROP TRIGGER fail_archival_sample")
            .unwrap();
        host.negative_failure.store(0, Ordering::SeqCst);
        host.cancel_on_read.store(false, Ordering::SeqCst);
        // Even a coherent observer publication between turns must not erase the negative generation.
        worker.reachability.mark_archival_published(100);
        let _ = worker.run_page();
        assert!(
            !db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
                .get::<_, bool>(0))
                .unwrap(),
            "negative {negative} was lost across failed write"
        );
    }
}

#[test]
fn archival_worker_negative_recovery_cycles_do_not_rediscover_retained_history() {
    use herdr_threads::store::archival::{self, Runtime};
    struct TestClock(std::sync::atomic::AtomicU64);
    impl Clock for TestClock {
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.monotonic_now().0 as i64)
        }
    }
    fn measure(size: usize) -> (u64, i64) {
        let clock = Arc::new(TestClock(100.into()));
        let fixture = super::handoff_fences::store_fixture_with_clock(clock.clone());
        let (mut worker, host, db, _directory) = worker_fixture_from_store(fixture);
        host.negative_failure.store(3, Ordering::SeqCst);
        db.execute_batch("BEGIN").unwrap();
        for n in 0..size {
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at) VALUES(?1,'i','retired','native',1,0,0)",[format!("history{n:08}")]).unwrap();
            db.execute("INSERT INTO threads(id,instance_id,topic,goal,archived,created_at,updated_at) VALUES(?1,'i','history','done',1,0,0)",[format!("history{n:08}")]).unwrap();
        }
        db.execute_batch("COMMIT").unwrap();
        let rt = Runtime {
            boot: "worker".into(),
            mono: 100,
            utc: UtcMillis(100),
            after_ms: 3_600_000,
            host_generation: 1,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered".into()),
        };
        for _ in 0..size / archival::PAGE + 3 {
            archival::advance(&db, "i", &rt).unwrap();
        }
        let original_epoch: i64 = db
            .query_row(
                "SELECT evidence_epoch FROM seat_archival WHERE seat_id='history00000000'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut turns = 0;
        for cycle in 1..=3 {
            let now = 100 + cycle * 60_000;
            clock.0.store(now, Ordering::SeqCst);
            worker.reachability.mark_archival_published(now);
            // One stable working read followed by committed negative/recovery and
            // all advertised continuation passes, through the actual worker.
            let before = host.calls.load(Ordering::SeqCst);
            for _ in 0..size / archival::PAGE + 8 {
                turns += 1;
                if !worker.run_page().unwrap() {
                    break;
                }
            }
            assert_eq!(host.calls.load(Ordering::SeqCst), before + 1);
            worker.reachability.mark_archival_published(now);
            for _ in 0..size / archival::PAGE + 8 {
                turns += 1;
                if !worker.run_page().unwrap() {
                    break;
                }
            }
            assert!(
                !db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
                    .get::<_, bool>(0))
                    .unwrap()
            );
        }
        let touched:i64=db.query_row("SELECT (SELECT count(*) FROM seat_archival WHERE seat_id LIKE 'history%' AND evidence_epoch!=?1)+(SELECT count(*) FROM channel_archival WHERE thread_id LIKE 'history%' AND evidence_epoch!=?1)",[original_epoch],|r|r.get(0)).unwrap();
        (turns, touched)
    }
    let small = measure(64);
    let large = measure(10_000);
    eprintln!(
        "full worker negative/recovery cycles (turns,rewritten history rows): {small:?} -> {large:?}"
    );
    assert!(
        large.0 <= small.0 + 2 && large.1 <= small.1 + 8,
        "recurring worker work must exclude retained history"
    );
}

#[test]
fn archival_worker_busy_member_does_not_reset_unrelated_all_left_grace() {
    struct TestClock(std::sync::atomic::AtomicU64);
    impl Clock for TestClock {
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.monotonic_now().0 as i64)
        }
    }
    let clock = Arc::new(TestClock(100.into()));
    let fixture = super::handoff_fences::store_fixture_with_clock(clock.clone());
    let (mut worker, host, db, _directory) = worker_fixture_from_store(fixture);
    host.negative_failure.store(3, Ordering::SeqCst);
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('quiet','i','quiet','done',0,0)", []).unwrap();
    for minute in 0..=60 {
        let now = 100 + minute * 60_000;
        clock.0.store(now, Ordering::SeqCst);
        worker.reachability.mark_archival_published(now);
        for turn in 0..32 {
            if !worker.run_page().unwrap() {
                break;
            }
            assert!(turn < 31, "bounded continuations must settle");
        }
        assert!(
            !db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
                .get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(
            db.query_row("SELECT archived FROM threads WHERE id='quiet'", [], |r| {
                r.get::<_, bool>(0)
            })
            .unwrap(),
            minute == 60,
            "unrelated working member must not restart all-left grace at minute {minute}"
        );
    }
    assert_eq!(host.calls.load(Ordering::SeqCst), 61);
}

struct JournalClock;
impl Clock for JournalClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
type JournalWorkerFixture = (
    ArchivalWorker,
    Arc<Host>,
    rusqlite::Connection,
    super::archival_legacy::Temp,
    herdr_threads::daemon::paths::InstancePaths,
);
fn journal_worker_fixture() -> JournalWorkerFixture {
    let (directory, paths, source) = super::archival_legacy::source();
    let (_store, db) = super::archival_legacy::scanner_store(&paths);
    let store = herdr_threads::store::SqliteStore::new(
        herdr_threads::store::connection::StoreContext::new(
            paths.database_path.clone(),
            Arc::new(JournalClock),
        ),
        "i",
        Default::default(),
    )
    .unwrap();
    let writer = Arc::new(FairWriter::new(32));
    let cancellation = Cancellation::default();
    let host = Arc::new(Host {
        path: paths.database_path.clone(),
        writer: writer.clone(),
        calls: AtomicUsize::new(0),
        cancel: cancellation.clone(),
        cancel_on_read: false.into(),
        negative_failure: AtomicUsize::new(3),
    });
    let worker = ArchivalWorker {
        store: Arc::new(store),
        host: host.clone(),
        writer,
        reachability: Arc::new(HostReachability::default()),
        source,
        boot: "worker".into(),
        after_ms: 60000,
        cancellation,
    };
    (worker, host, db, directory, paths)
}
fn distinct_journals(paths: &herdr_threads::daemon::paths::InstancePaths) {
    use herdr_threads::cli::journal::{Journal, SemanticMutation};
    let reference = super::archival_legacy::compound(paths);
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let pending = journal.load(&reference).unwrap();
    let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
        panic!("frozen")
    };
    let SemanticMutation::Handoff(plan) = *mutation else {
        panic!("handoff")
    };
    for n in 1..37 {
        let mut plan = plan.clone();
        plan.create_key = OperationId::new(format!("outer-{n}-create"));
        plan.invite_key = OperationId::new(format!("outer-{n}-invite"));
        plan.send_key = OperationId::new(format!("outer-{n}-send"));
        journal
            .record(
                pending.header.scope.clone(),
                SemanticMutation::freeze(SemanticMutation::Handoff(plan), claim.clone()).unwrap(),
                0,
            )
            .unwrap();
    }
}
fn drain_worker_traversal(worker: &mut ArchivalWorker) {
    for _ in 0..30 {
        if !worker.run_page().unwrap() {
            return;
        }
    }
    panic!("actual worker traversal did not finish");
}
fn legacy_veto(db: &rusqlite::Connection) -> bool {
    db.query_row(
        "SELECT bootstrap_veto FROM archival_instances WHERE instance_id='i'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
#[test]
fn archival_worker_consumed_outer_admission_failure_vetoes_same_traversal_and_fresh_recovers() {
    let (mut worker, host, db, _directory, paths) = journal_worker_fixture();
    distinct_journals(&paths);
    worker.writer = Arc::new(FairWriter::new(0));
    let error = worker.run_page().unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "outer admission failed before any deciding import"
    );
    worker.writer = host.writer.clone();
    drain_worker_traversal(&mut worker);
    let imported: i64 = db
        .query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        (1..37).contains(&imported),
        "first pending journal page was consumed and not imported: {imported}"
    );
    assert!(
        legacy_veto(&db),
        "same Source EOF must not accept coverage after dropped admission page"
    );
    drain_worker_traversal(&mut worker);
    assert!(
        !legacy_veto(&db),
        "new successful traversal rechecks every original record"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM channel_handoff_fences WHERE origin='legacy_local_journal_hint'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        37
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn archival_worker_deciding_bootstrap_veto_reaches_next_consumer() {
    let (mut worker, host, db, _directory, paths) = journal_worker_fixture();
    worker.run_page().unwrap(); // Seed the actual observation work before parking it.
    super::archival_legacy::modern_compound(&paths, true, false);
    worker.reachability.mark_archival_published(100);
    db.execute("UPDATE seat_archival SET next_mono=999999", [])
        .unwrap();
    assert!(!worker.run_page().unwrap());
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    assert!(
        legacy_veto(&db),
        "archival_next must not restore the pre-pass coverage after deciding absent-parent veto"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn archival_worker_deciding_bootstrap_veto_reaches_sample_consumer() {
    let (mut worker, host, db, _directory, paths) = journal_worker_fixture();
    super::archival_legacy::modern_compound(&paths, true, false);
    worker.reachability.mark_archival_published(100);
    worker.run_page().unwrap();
    assert_eq!(
        host.calls.load(Ordering::SeqCst),
        1,
        "actual archival host sample/persistence path reached"
    );
    assert!(
        legacy_veto(&db),
        "archival_sample must not restore the pre-pass coverage after deciding absent-parent veto"
    );
    assert!(
        !db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
            .get::<_, bool>(0))
            .unwrap()
    );
}

struct AdmissionClock {
    calls: AtomicUsize,
    cancel: Option<Cancellation>,
}
impl Clock for AdmissionClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0
            && let Some(cancel) = &self.cancel
        {
            cancel.cancel();
        }
        MonoInstant(if call == 1 && self.cancel.is_none() {
            5100
        } else {
            100
        })
    }
}
#[test]
fn archival_worker_consumed_page_preserves_deadline_cancel_runtime_and_rollback_errors() {
    for mode in ["deadline", "cancel", "runtime", "rollback"] {
        let (mut worker, host, db, _directory, paths) = journal_worker_fixture();
        distinct_journals(&paths);
        let expected = match mode {
            "deadline" | "cancel" => {
                let clock = Arc::new(AdmissionClock {
                    calls: AtomicUsize::new(0),
                    cancel: (mode == "cancel").then(|| worker.cancellation.clone()),
                });
                worker.store = Arc::new(
                    herdr_threads::store::SqliteStore::new(
                        herdr_threads::store::connection::StoreContext::new(
                            paths.database_path.clone(),
                            clock,
                        ),
                        "i",
                        Default::default(),
                    )
                    .unwrap(),
                );
                if mode == "cancel" {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::DeadlineExceeded
                }
            }
            "runtime" => {
                worker.after_ms = u64::MAX;
                ErrorCode::InvalidRequest
            }
            "rollback" => {
                db.execute_batch("CREATE TRIGGER fail_owned_archival_hint BEFORE INSERT ON channel_handoff_fences BEGIN SELECT RAISE(ABORT,'owned archival hint rollback'); END").unwrap();
                ErrorCode::Conflict
            }
            _ => unreachable!(),
        };
        assert_eq!(
            worker.run_page().unwrap_err().code,
            expected,
            "original {mode} error must be preserved"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        if mode == "rollback" {
            db.execute_batch("DROP TRIGGER fail_owned_archival_hint")
                .unwrap();
        }
        // Cancellation normally ends the lane. A replacement token models a new
        // owner using the retained cursor; it cannot erase the consumed-page veto.
        worker.cancellation = Cancellation::default();
        worker.after_ms = 60000;
        worker.store = Arc::new(
            herdr_threads::store::SqliteStore::new(
                herdr_threads::store::connection::StoreContext::new(
                    paths.database_path.clone(),
                    Arc::new(JournalClock),
                ),
                "i",
                Default::default(),
            )
            .unwrap(),
        );
        drain_worker_traversal(&mut worker);
        let imported: i64 = db
            .query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            (1..37).contains(&imported),
            "{mode} failed after first pending page consumption: {imported}"
        );
        assert!(
            legacy_veto(&db),
            "consumed {mode} error must veto same traversal EOF"
        );
        drain_worker_traversal(&mut worker);
        assert!(
            !legacy_veto(&db),
            "fresh successful traversal recovers after {mode}"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            37
        );
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn archival_worker_fresh_absence_recovers_after_consumed_runtime_failure() {
    let (mut worker, host, db, _directory, paths) = journal_worker_fixture();
    assert!(!paths.instance_dir.join("intents").exists());
    worker.after_ms = u64::MAX;
    assert_eq!(
        worker.run_page().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    worker.after_ms = 60000;
    drain_worker_traversal(&mut worker);
    assert!(
        !legacy_veto(&db),
        "every absent-root scan is a complete fresh traversal, not a permanent old error veto"
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
}
