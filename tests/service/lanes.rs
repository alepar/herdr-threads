use herdr_threads::{
    ports::{DuePhaseProgress, DueScanProgress, DueScanState},
    protocol::results::BoundedError,
    protocol::{
        results::ErrorCode,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    scheduler::deadlines::DriveOutcome,
    service::workers::{BoundedLane, FairWriter, WorkerStatus},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

#[test]
fn background_gets_a_turn_after_eight_foreground_decisions_then_foreground_resumes() {
    let lane = Arc::new(FairWriter::new(32));
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    for _ in 0..7 {
        drop(lane.enter_foreground(&budget, &FixedClock).unwrap());
    }
    let active = lane.enter_foreground(&budget, &FixedClock).unwrap();
    let (events, received) = std::sync::mpsc::channel();
    let background = {
        let lane = Arc::clone(&lane);
        let events = events.clone();
        let budget = budget.clone();
        std::thread::spawn(move || {
            let _guard = lane.enter_background(&budget, &FixedClock).unwrap();
            events.send("background").unwrap();
        })
    };
    let until = Instant::now() + Duration::from_secs(2);
    while lane.waiting() != (0, 1) && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(lane.waiting(), (0, 1));
    let foreground = {
        let lane = Arc::clone(&lane);
        let budget = budget.clone();
        std::thread::spawn(move || {
            let _guard = lane.enter_foreground(&budget, &FixedClock).unwrap();
            events.send("foreground").unwrap();
        })
    };
    while lane.waiting() != (1, 1) && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(lane.waiting(), (1, 1));
    drop(active);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        "background"
    );
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        "foreground"
    );
    background.join().unwrap();
    foreground.join().unwrap();
}

#[test]
fn search_lane_caps_queued_work_and_releases_all_waiters() {
    let lane = Arc::new(BoundedLane::new(1, 4));
    let clock = FixedClock;
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    let held = lane.enter(&budget, &clock).unwrap();
    let mut workers = Vec::new();
    for _ in 0..4 {
        let lane = Arc::clone(&lane);
        let budget = budget.clone();
        workers.push(std::thread::spawn(move || {
            let _guard = lane.enter(&budget, &FixedClock).unwrap();
        }));
    }
    let until = Instant::now() + Duration::from_secs(2);
    while lane.queued() != 4 && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(lane.queued(), 4);
    assert_eq!(
        lane.enter(&budget, &clock).unwrap_err().code,
        ErrorCode::StoreBusy
    );
    drop(held);
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(lane.queued(), 0);
}

#[test]
fn writer_foreground_admission_preserves_arrival_order() {
    let lane = Arc::new(FairWriter::new(32));
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    let active = lane.enter_background(&budget, &FixedClock).unwrap();
    let (events, received) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for label in ["first", "second"] {
        let worker_lane = Arc::clone(&lane);
        let events = events.clone();
        let budget = budget.clone();
        workers.push(std::thread::spawn(move || {
            let _guard = worker_lane.enter_foreground(&budget, &FixedClock).unwrap();
            events.send(label).unwrap();
        }));
        let until = Instant::now() + Duration::from_secs(2);
        while lane.waiting().0 != workers.len() && Instant::now() < until {
            std::thread::yield_now();
        }
        assert_eq!(lane.waiting().0, workers.len());
    }
    drop(active);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        "first"
    );
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        "second"
    );
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn cancelled_search_waiter_releases_capacity_without_entering() {
    let lane = Arc::new(BoundedLane::new(1, 1));
    let held = lane
        .enter(
            &CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            &FixedClock,
        )
        .unwrap();
    let cancellation = Cancellation::default();
    let waiter = {
        let lane = Arc::clone(&lane);
        let cancellation = cancellation.clone();
        std::thread::spawn(move || {
            lane.enter(
                &CallBudget {
                    deadline: MonoInstant(10_000),
                    cancellation,
                },
                &FixedClock,
            )
            .map(|_| ())
        })
    };
    let until = Instant::now() + Duration::from_secs(2);
    while lane.queued() != 1 && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(lane.queued(), 1);
    cancellation.cancel();
    assert_eq!(
        waiter.join().unwrap().unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(lane.queued(), 0);
    drop(held);
}

#[test]
fn deadline_worker_keeps_a_failed_due_phase_visible_during_retry_cooldown() {
    let status = WorkerStatus::default();
    status.observe(&DriveOutcome {
        due_progress: Some(DueScanProgress {
            state: DueScanState::default(),
            examined_candidates: 0,
            warnings_added: 0,
            invitations: DuePhaseProgress::Failed(
                ErrorCode::StoreBusy,
                BoundedError::parse("invitation scan failed").unwrap(),
            ),
            receipts: DuePhaseProgress::Complete,
            has_more: true,
        }),
        ..DriveOutcome::default()
    });
    assert_eq!(
        status.last_error().as_deref(),
        Some("invitation due scan failed: StoreBusy")
    );
    status.observe(&DriveOutcome::default());
    assert_eq!(
        status.last_error().as_deref(),
        Some("invitation due scan failed: StoreBusy")
    );
}

#[test]
fn deadline_health_recovers_only_the_successful_failed_phase() {
    let status = WorkerStatus::default();
    let progress = |invitations, receipts| DriveOutcome {
        due_progress: Some(DueScanProgress {
            state: DueScanState::default(),
            examined_candidates: 0,
            warnings_added: 0,
            invitations,
            receipts,
            has_more: false,
        }),
        ..DriveOutcome::default()
    };
    status.observe(&progress(
        DuePhaseProgress::Failed(
            ErrorCode::StoreBusy,
            BoundedError::parse("invitation failure").unwrap(),
        ),
        DuePhaseProgress::Failed(
            ErrorCode::Conflict,
            BoundedError::parse("receipt failure").unwrap(),
        ),
    ));
    status.observe(&DriveOutcome::default());
    status.observe(&progress(DuePhaseProgress::More, DuePhaseProgress::Skipped));
    assert!(status.last_error().is_some());
    status.observe(&progress(
        DuePhaseProgress::Complete,
        DuePhaseProgress::Skipped,
    ));
    assert_eq!(
        status.last_error().as_deref(),
        Some("receipt due scan failed: Conflict")
    );
    status.observe(&progress(
        DuePhaseProgress::Skipped,
        DuePhaseProgress::Complete,
    ));
    assert_eq!(status.last_error(), None);
}

#[test]
fn deadline_health_does_not_clear_work_failure_on_unrelated_job_success() {
    use herdr_threads::protocol::results::ApiError;
    let status = WorkerStatus::default();
    status.observe(&DriveOutcome {
        work_job: Some("failed-job".into()),
        work_error: Some(ApiError {
            code: ErrorCode::StoreBusy,
            detail: "committed prefix failure".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
        ..DriveOutcome::default()
    });
    status.observe(&DriveOutcome {
        work_job: Some("other-job".into()),
        work_complete: true,
        ..DriveOutcome::default()
    });
    assert_eq!(
        status.last_error().as_deref(),
        Some("work job failed: StoreBusy")
    );
    status.observe(&DriveOutcome {
        work_job: Some("failed-job".into()),
        ..DriveOutcome::default()
    });
    assert_eq!(
        status.last_error().as_deref(),
        Some("work job failed: StoreBusy")
    );
    status.observe(&DriveOutcome {
        work_job: Some("failed-job".into()),
        work_complete: true,
        ..DriveOutcome::default()
    });
    assert_eq!(status.last_error(), None);
}

struct HealthDeadlinePort {
    mono: std::sync::atomic::AtomicU64,
    jobs: std::sync::Mutex<Vec<bool>>,
    mode: std::sync::atomic::AtomicU8,
    invitation_failed: std::sync::atomic::AtomicBool,
}
impl Clock for HealthDeadlinePort {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(std::sync::atomic::Ordering::SeqCst))
    }
}
fn health_page<T>(
    items: Vec<T>,
    next: Option<String>,
    high: u64,
) -> herdr_threads::protocol::pagination::Page<T> {
    use herdr_threads::protocol::pagination::{Consistency, Page, StopReason};
    Page {
        items,
        has_more: next.is_some(),
        next_cursor: next,
        next_argv: None,
        high_water_ordinal: high,
        scope_revision: None,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}
impl herdr_threads::scheduler::deadlines::DeadlinePort for HealthDeadlinePort {
    fn clock(&self) -> &dyn Clock {
        self
    }
    fn due_obligations(
        &self,
        request: herdr_threads::ports::DueScanRequest,
        _: &CallBudget,
    ) -> Result<DueScanProgress, herdr_threads::protocol::results::ApiError> {
        Ok(DueScanProgress {
            state: request.state,
            examined_candidates: 0,
            warnings_added: 0,
            invitations: if !request.run_invitations {
                DuePhaseProgress::Skipped
            } else if self
                .invitation_failed
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                DuePhaseProgress::Failed(
                    ErrorCode::StoreBusy,
                    BoundedError::parse("peer invitation failure").unwrap(),
                )
            } else {
                DuePhaseProgress::Complete
            },
            receipts: if request.run_receipts {
                DuePhaseProgress::Complete
            } else {
                DuePhaseProgress::Skipped
            },
            has_more: false,
        })
    }
    fn pending_retirement_jobs(
        &self,
        _: herdr_threads::protocol::pagination::PageRequest,
        _: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<
            herdr_threads::protocol::results::RetirementStatus,
        >,
        herdr_threads::protocol::results::ApiError,
    > {
        Ok(health_page(vec![], None, 0))
    }
    fn advance_retirement(
        &self,
        _: herdr_threads::protocol::ids::RetirementJobId,
        _: herdr_threads::ports::WorkAdmission,
        _: &CallBudget,
    ) -> Result<herdr_threads::ports::RetirementProgress, herdr_threads::protocol::results::ApiError>
    {
        unreachable!()
    }
    fn pending_work(
        &self,
        page: herdr_threads::protocol::pagination::PageRequest,
        _: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<herdr_threads::ports::WorkCandidate>,
        herdr_threads::protocol::results::ApiError,
    > {
        if self
            .mode
            .compare_exchange(
                3,
                1,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            return Err(herdr_threads::protocol::results::ApiError {
                code: ErrorCode::StoreBusy,
                detail: "discovery failed".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
        }
        let jobs = self.jobs.lock().unwrap();
        let after: usize = page.cursor.as_deref().unwrap_or("0").parse().unwrap();
        let high = jobs.len(); // Fixed throughout this fixture; no job removal.
        let next_job = (after..high).find(|&i| !jobs[i]);
        let Some(i) = next_job else {
            return Ok(health_page(vec![], None, high as u64));
        };
        Ok(health_page(
            vec![herdr_threads::ports::WorkCandidate {
                id: format!("job-{i}"),
                kind: herdr_threads::ports::WorkKind::SendAttention,
                position: 0,
                high_water: 1,
                has_more: true,
            }],
            (i + 1 < high).then(|| (i + 1).to_string()),
            high as u64,
        ))
    }
    fn advance_work(
        &self,
        job: &str,
        _: herdr_threads::ports::DurableWorkAdmission,
        _: &CallBudget,
    ) -> Result<herdr_threads::ports::WorkProgress, herdr_threads::protocol::results::ApiError>
    {
        let i: usize = job.strip_prefix("job-").unwrap().parse().unwrap();
        let mode = self.mode.load(std::sync::atomic::Ordering::SeqCst);
        if mode == 2 {
            self.jobs.lock().unwrap()[i] = true;
        }
        Ok(herdr_threads::ports::WorkProgress {
            completed_units: 1,
            processed_this_turn: 1,
            next_position: u64::from(mode == 2),
            has_more: mode != 2,
            last_error: (mode == 0).then(|| "committed prefix failure".into()),
        })
    }
}

#[test]
fn deadline_health_saturation_requires_a_later_complete_successful_retry_sweep() {
    use herdr_threads::scheduler::deadlines::DeadlineDriver;
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
    let port = HealthDeadlinePort {
        mono: AtomicU64::new(0),
        jobs: std::sync::Mutex::new(vec![false; 65]),
        mode: AtomicU8::new(0),
        invitation_failed: AtomicBool::new(true),
    };
    let mut driver = DeadlineDriver::new(&port);
    let status = WorkerStatus::default();
    let step = |driver: &mut DeadlineDriver<'_, HealthDeadlinePort>| {
        port.mono.fetch_add(1_000, Ordering::SeqCst);
        let outcome = driver
            .drive(&CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            })
            .unwrap();
        status.observe(&outcome);
    };
    for _ in 0..65 {
        step(&mut driver);
    }
    assert!(status.last_error().unwrap().contains("tracking gap"));
    // Successful committed prefixes are incomplete, so a full sweep is insufficient.
    port.mode.store(1, Ordering::SeqCst);
    for _ in 0..65 {
        step(&mut driver);
    }
    assert!(status.last_error().unwrap().contains("tracking gap"));
    status.observe(&DriveOutcome::default());
    assert!(status.last_error().unwrap().contains("tracking gap"));
    port.mode.store(3, Ordering::SeqCst);
    step(&mut driver);
    assert!(status.last_error().unwrap().contains("tracking gap"));
    let exhausted = driver.drive(&CallBudget {
        deadline: port.monotonic_now(),
        cancellation: Cancellation::default(),
    });
    assert_eq!(
        exhausted.as_ref().unwrap_err().code,
        ErrorCode::DeadlineExceeded
    );
    status.observe_drive(&exhausted);
    assert!(status.last_error().unwrap().contains("tracking gap"));
    // Discovery cooldown does not count as coverage; finish an incomplete
    // sweep after the cooldown before beginning the successful fresh sweep.
    loop {
        port.mono.fetch_add(1_000, Ordering::SeqCst);
        let outcome = driver
            .drive(&CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            })
            .unwrap();
        let end = outcome
            .work_discovery
            .as_ref()
            .is_some_and(|page| page.reached_end);
        status.observe(&outcome);
        if end {
            break;
        }
    }
    port.mode.store(2, Ordering::SeqCst);
    for _ in 0..10 {
        step(&mut driver);
    }
    assert!(status.last_error().unwrap().contains("tracking gap"));
    for _ in 10..65 {
        step(&mut driver);
    }
    assert_eq!(
        status.last_error().as_deref(),
        Some("invitation due scan failed: StoreBusy")
    );
    port.invitation_failed.store(false, Ordering::SeqCst);
    for _ in 0..5 {
        step(&mut driver);
    }
    assert_eq!(status.last_error(), None);
}

struct HealthTestDir(std::path::PathBuf);
impl HealthTestDir {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!("herdr-health-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for HealthTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct HealthWakeFixture {
    store: herdr_threads::store::SqliteStore,
    fail: std::sync::atomic::AtomicU8,
    completion_hold: Option<Arc<HealthCompletionHold>>,
}
#[derive(Default)]
struct HealthCompletionState {
    attempt: Option<String>,
    entered: bool,
    released: bool,
}
#[derive(Default)]
struct HealthCompletionHold {
    state: std::sync::Mutex<HealthCompletionState>,
    changed: std::sync::Condvar,
}
impl HealthCompletionHold {
    fn arm(&self, attempt: String) {
        self.state.lock().unwrap().attempt = Some(attempt);
    }
    fn pause(&self, attempt: &str) -> Result<(), herdr_threads::protocol::results::ApiError> {
        let mut state = self.state.lock().unwrap();
        if state.attempt.as_deref() != Some(attempt) || state.released {
            return Ok(());
        }
        state.entered = true;
        self.changed.notify_all();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.released)
            .unwrap();
        if !state.released {
            return Err(herdr_threads::protocol::results::ApiError {
                code: ErrorCode::DeadlineExceeded,
                detail: "bounded health completion fixture wait expired".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
        }
        Ok(())
    }
    fn wait_until_claimed(&self) {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| !state.entered)
            .unwrap();
        assert!(
            state.entered,
            "completion retry did not reach the bounded fixture hold"
        );
    }
    fn release(&self) {
        self.state.lock().unwrap().released = true;
        self.changed.notify_all();
    }
}
struct HealthCompletionRelease(Arc<HealthCompletionHold>);
impl Drop for HealthCompletionRelease {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[test]
fn wake_health_completion_hold_releases_and_joins_on_assertion_unwind() {
    let hold = Arc::new(HealthCompletionHold::default());
    hold.arm("held-attempt".into());
    // A healthy peer must not wait behind the exact claimed attempt.
    hold.pause("healthy-peer").unwrap();
    assert!(!hold.state.lock().unwrap().entered);
    let (finished, result) = std::sync::mpsc::channel();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            let _release = HealthCompletionRelease(hold.clone());
            scope.spawn(|| {
                finished.send(hold.pause("held-attempt")).unwrap();
            });
            hold.wait_until_claimed();
            panic!("exercise assertion cleanup");
        });
    }));
    assert!(unwind.is_err());
    // The scoped thread was joined while unwinding, after the guard released.
    assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
    assert!(hold.state.lock().unwrap().released);
}

impl HealthWakeFixture {
    fn result<T>(
        &self,
        phase: u8,
        operation: impl FnOnce() -> Result<T, herdr_threads::protocol::results::ApiError>,
    ) -> Result<T, herdr_threads::protocol::results::ApiError> {
        if self
            .fail
            .compare_exchange(
                phase,
                0,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            Err(herdr_threads::protocol::results::ApiError {
                code: ErrorCode::StoreBusy,
                detail: "injected wake operation failure".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            })
        } else {
            operation()
        }
    }
}
impl herdr_threads::scheduler::WakePort for HealthWakeFixture {
    fn clock(&self) -> &dyn Clock {
        herdr_threads::ports::StorePort::clock(&self.store)
    }
    fn wake_candidates(
        &self,
        page: herdr_threads::protocol::pagination::PageRequest,
        budget: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<herdr_threads::ports::WakeCandidate>,
        herdr_threads::protocol::results::ApiError,
    > {
        self.result(3, || {
            herdr_threads::ports::StorePort::wake_candidates(&self.store, page, budget)
        })
    }
    fn reserve_wake(
        &self,
        candidate: &herdr_threads::ports::WakeCandidate,
        budget: &CallBudget,
    ) -> Result<
        Option<herdr_threads::ports::WakeReservation>,
        herdr_threads::protocol::results::ApiError,
    > {
        self.result(4, || {
            herdr_threads::ports::StorePort::reserve_wake(&self.store, candidate, budget)
        })
    }
    fn complete_wake(
        &self,
        attempt: herdr_threads::protocol::ids::WakeAttemptId,
        outcome: herdr_threads::ports::WakeOutcome,
        budget: &CallBudget,
    ) -> Result<(), herdr_threads::protocol::results::ApiError> {
        self.result(5, || {
            if let Some(hold) = &self.completion_hold {
                hold.pause(attempt.as_str())?;
            }
            herdr_threads::ports::StorePort::complete_wake(&self.store, attempt, outcome, budget)
        })
    }
    fn wake_recovery_candidates(
        &self,
        page: herdr_threads::protocol::pagination::PageRequest,
        budget: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<herdr_threads::ports::WakeRecoveryCandidate>,
        herdr_threads::protocol::results::ApiError,
    > {
        self.result(1, || {
            herdr_threads::ports::StorePort::wake_recovery_candidates(&self.store, page, budget)
        })
    }
    fn recover_wake_reservation(
        &self,
        request: herdr_threads::ports::WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<herdr_threads::ports::WakeRecoveryOutcome, herdr_threads::protocol::results::ApiError>
    {
        self.result(2, || {
            herdr_threads::ports::StorePort::recover_wake_reservation(&self.store, request, budget)
        })
    }
}
struct HealthNotifier;
impl herdr_threads::ports::NotificationPort for HealthNotifier {
    fn attempt_wake(
        &self,
        _: herdr_threads::ports::WakeReservation,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::WakeOutcome, herdr_threads::protocol::results::ApiError> {
        Ok(herdr_threads::ports::WakeOutcome::Submitted)
    }
}
fn health_wake_store(
    path: &std::path::Path,
    clock: Arc<dyn Clock>,
    boot: uuid::Uuid,
) -> herdr_threads::store::SqliteStore {
    let context = herdr_threads::store::connection::StoreContext::new(path.into(), clock);
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);").unwrap();
    for seat in ["a", "b"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [seat]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'host',1,1,0,'fresh','occupied','idle','execution',1,'term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?1,'host',1,'codex','session','execution','fresh',0,0,'term-'||?1,'inc')", [seat]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'invited')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES (?1,'t',?1,1,'pending',0,1000000,1000000,1)", [seat]).unwrap();
    }
    db.execute_batch("INSERT INTO seats(id,instance_id,state,role,generation,target_generation,created_at) VALUES ('old','i','unresolved','native',1,1,0);").unwrap();
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,retry_step,reservation_id,reservation_boot,last_reservation_id,last_reservation_boot,minimum_delay_ms,effective_delay_ms) VALUES ('old',0,0,'old-attempt',?1,'old-attempt',?1,30000,30000)", [uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap().to_string()]).unwrap();
    drop(db);
    herdr_threads::store::SqliteStore::new(
        context,
        "i",
        herdr_threads::store::StoreSettings {
            daemon_boot: Some(boot),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn wake_health_recovers_only_after_actual_failed_store_operation_retry() {
    use herdr_threads::{
        notification::policy::RetryConfig, scheduler::Scheduler, service::workers::ObservedWakePort,
    };
    let mut retained = Vec::new();
    for phase in 1..=4 {
        let dir = HealthTestDir::new();
        let boot = uuid::Uuid::new_v4();
        let clock = Arc::new(HealthDeadlinePort {
            mono: std::sync::atomic::AtomicU64::new(0),
            jobs: std::sync::Mutex::new(vec![]),
            mode: std::sync::atomic::AtomicU8::new(2),
            invitation_failed: std::sync::atomic::AtomicBool::new(false),
        });
        let fixture = HealthWakeFixture {
            store: health_wake_store(&dir.path().join("db"), clock.clone(), boot),
            fail: std::sync::atomic::AtomicU8::new(phase),
            completion_hold: None,
        };
        let status = WorkerStatus::default();
        let observed = ObservedWakePort::new(&fixture, &status);
        let scheduler = Scheduler::new(
            "i".into(),
            clock.as_ref(),
            &observed,
            &HealthNotifier,
            RetryConfig::default(),
            boot,
        );
        let budget = CallBudget {
            deadline: MonoInstant(10000),
            cancellation: Cancellation::default(),
        };
        observed.begin_drive(&budget);
        let first = scheduler.drive_wakes(&budget);
        assert!(first.is_err(), "phase {phase}");
        observed.observe_drive(&first);
        assert!(status.last_error().is_some());
        observed.begin_drive(&budget);
        let second = scheduler.drive_wakes(&budget);
        assert!(second.is_ok(), "phase {phase}: {second:?}");
        observed.observe_drive(&second);
        if let Some(error) = status.last_error() {
            retained.push((phase, error));
        }
        assert!(status.last_diagnostic().is_some());
    }
    assert!(
        retained.is_empty(),
        "recovered operations remain degraded: {retained:?}"
    );
}

#[test]
fn wake_health_keeps_unsettled_completion_through_local_noop_and_healthy_peer() {
    use herdr_threads::{
        notification::policy::RetryConfig, scheduler::Scheduler, service::workers::ObservedWakePort,
    };
    let dir = HealthTestDir::new();
    let boot = uuid::Uuid::new_v4();
    let clock = Arc::new(HealthDeadlinePort {
        mono: std::sync::atomic::AtomicU64::new(0),
        jobs: std::sync::Mutex::new(vec![]),
        mode: std::sync::atomic::AtomicU8::new(2),
        invitation_failed: std::sync::atomic::AtomicBool::new(false),
    });
    let path = dir.path().join("db");
    let hold = Arc::new(HealthCompletionHold::default());
    let fixture = HealthWakeFixture {
        store: health_wake_store(&path, clock.clone(), boot),
        fail: std::sync::atomic::AtomicU8::new(5),
        completion_hold: Some(hold.clone()),
    };
    let status = WorkerStatus::default();
    let observed = ObservedWakePort::new(&fixture, &status);
    let scheduler = Scheduler::new(
        "i".into(),
        clock.as_ref(),
        &observed,
        &HealthNotifier,
        RetryConfig::default(),
        boot,
    );
    let budget = CallBudget {
        deadline: MonoInstant(10000),
        cancellation: Cancellation::default(),
    };
    observed.begin_drive(&budget);
    let first = scheduler.drive_wakes(&budget);
    assert!(first.is_err());
    observed.observe_drive(&first);
    let first_error = status.last_error().unwrap();
    assert!(
        first_error.contains("unsettled durable wake completion"),
        "{first_error}"
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let attempt: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    hold.arm(attempt.clone());
    std::thread::scope(|scope| {
        // The release guard runs before the scope joins, even if an assertion
        // panics. The hold itself also has a finite timeout.
        let release = HealthCompletionRelease(hold.clone());
        let retry = scope.spawn(|| scheduler.drive_wakes(&budget));
        hold.wait_until_claimed();
        observed.begin_drive(&budget);
        let peer = scheduler.drive_wakes(&budget).unwrap();
        assert_eq!((peer.examined, peer.attempted), (3, 1)); // Failed a is a local no-op; b is committed; old is unresolved.
        assert_eq!(status.last_error().as_deref(), Some(first_error.as_str()));
        let db = rusqlite::Connection::open(&path).unwrap();
        let unsettled: i64 = db
            .query_row(
                "SELECT count(*) FROM wake_work WHERE reservation_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unsettled, 1);
        assert_eq!(
            db.query_row(
                "SELECT reservation_id FROM wake_work WHERE seat_id='a'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            attempt
        );
        assert!(peer.has_more);
        drop(release);
        let recovered = retry.join().unwrap();
        assert!(
            recovered.is_ok(),
            "held completion did not recover: {recovered:?}"
        );
        observed.observe_drive(&recovered);
        assert_eq!(status.last_error(), None);
        let reservation: Option<String> = db
            .query_row(
                "SELECT reservation_id FROM wake_work WHERE seat_id='a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reservation, None);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM wake_work WHERE reservation_id IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    });
}

#[test]
fn deadline_health_retirement_jobs_recover_independently_of_discovery() {
    use herdr_threads::protocol::{ids::RetirementJobId, results::ApiError};
    use herdr_threads::scheduler::deadlines::DiscoveryProgress;
    let status = WorkerStatus::default();
    for (job, code) in [("a", ErrorCode::StoreBusy), ("b", ErrorCode::Conflict)] {
        status.observe(&DriveOutcome {
            retirement_job: Some(RetirementJobId::new(job)),
            retirement_error: Some(ApiError {
                code,
                detail: format!("private retirement {job} failed"),
                restart_argv: None,
                required_minimum_bytes: None,
            }),
            ..Default::default()
        });
    }
    status.observe(&DriveOutcome {
        retirement_discovery: Some(DiscoveryProgress {
            fresh_start: true,
            reached_end: true,
            returned_job: false,
        }),
        ..Default::default()
    });
    // Health sees only the typed class and code, never the private detail.
    assert_eq!(
        status.last_error().as_deref(),
        Some("retirement cleanup failed: StoreBusy")
    );
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("a")),
        retirement_complete: true,
        ..Default::default()
    });
    assert_eq!(
        status.last_error().as_deref(),
        Some("retirement cleanup failed: Conflict")
    );
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("b")),
        ..Default::default()
    });
    assert!(status.last_error().is_some());
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("b")),
        retirement_complete: true,
        ..Default::default()
    });
    assert_eq!(status.last_error(), None);
    // Health never carried the text; the lane's private diagnostic tail
    // keeps the latest exact detail (top-level job errors are persisted
    // nowhere else).
    assert_eq!(
        status.last_diagnostic().as_deref(),
        Some("retirement job: Conflict: private retirement b failed")
    );
}

#[test]
fn deadline_health_redacts_retirement_discovery_failure() {
    // Kills: recording `format!("{:?}: {}", code, detail)` for
    // `FailureKey::RetirementDiscovery` (no job returned by discovery) as
    // the Health failure; and (fix1) `job_failure` dropping the exact detail
    // from the private diagnostic tail.
    use herdr_threads::protocol::results::ApiError;
    use herdr_threads::scheduler::deadlines::DiscoveryProgress;
    let status = WorkerStatus::default();
    status.observe(&DriveOutcome {
        retirement_error: Some(ApiError {
            code: ErrorCode::StoreBusy,
            detail: "SQLite: private discovery failure".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
        ..Default::default()
    });
    assert_eq!(
        status.last_error().as_deref(),
        Some("retirement discovery failed: StoreBusy")
    );
    // The exact detail is kept only in the private diagnostic tail.
    assert_eq!(
        status.last_diagnostic().as_deref(),
        Some("retirement discovery: StoreBusy: SQLite: private discovery failure")
    );
    status.observe(&DriveOutcome {
        retirement_discovery: Some(DiscoveryProgress {
            fresh_start: false,
            reached_end: false,
            returned_job: false,
        }),
        ..Default::default()
    });
    assert_eq!(status.last_error(), None);
}

#[test]
fn deadline_health_clears_retirement_failure_on_committed_partial_progress() {
    // Kills: clearing `FailureKey::Retirement` only on `retirement_complete`
    // (ignoring `retirement_progressed`), and clearing it on an idle,
    // non-progressing observation of the same job.
    use herdr_threads::protocol::{ids::RetirementJobId, results::ApiError};
    let status = WorkerStatus::default();
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("a")),
        retirement_error: Some(ApiError {
            code: ErrorCode::Conflict,
            detail: "SQLite: private failure".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
        ..Default::default()
    });
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("a")),
        ..Default::default()
    });
    assert!(status.last_error().is_some(), "idle observation cleared it");
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("b")),
        retirement_progressed: true,
        ..Default::default()
    });
    assert!(
        status.last_error().is_some(),
        "other job's progress cleared it"
    );
    status.observe(&DriveOutcome {
        retirement_job: Some(RetirementJobId::new("a")),
        retirement_progressed: true,
        ..Default::default()
    });
    assert_eq!(status.last_error(), None);
}

#[test]
fn deadline_health_real_driver_cooldown_skip_does_not_recover_failed_job() {
    use herdr_threads::scheduler::deadlines::DeadlineDriver;
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
    let port = HealthDeadlinePort {
        mono: AtomicU64::new(0),
        jobs: std::sync::Mutex::new(vec![false]),
        mode: AtomicU8::new(0),
        invitation_failed: AtomicBool::new(false),
    };
    let mut driver = DeadlineDriver::new(&port);
    let status = WorkerStatus::default();
    let budget = CallBudget {
        deadline: MonoInstant(10000),
        cancellation: Cancellation::default(),
    };
    status.observe_drive(&driver.drive(&budget));
    assert_eq!(
        status.last_error().as_deref(),
        Some("work job failed: StoreBusy")
    );
    port.mode.store(2, Ordering::SeqCst);
    port.mono.store(1000, Ordering::SeqCst);
    let skipped = driver.drive(&budget).unwrap();
    assert!(skipped.work_discovery.as_ref().unwrap().returned_job);
    assert!(skipped.work_job.is_none());
    status.observe(&skipped);
    assert!(status.last_error().is_some());
    port.mono.store(5000, Ordering::SeqCst);
    let recovered = driver.drive(&budget).unwrap();
    assert!(recovered.work_complete);
    status.observe(&recovered);
    assert_eq!(status.last_error(), None);
    assert!(status.last_diagnostic().is_some());
}

#[test]
fn wake_health_recovers_exhausted_ingress_budget_on_fresh_valid_admission() {
    use herdr_threads::{
        notification::policy::RetryConfig, scheduler::Scheduler, service::workers::ObservedWakePort,
    };
    let dir = HealthTestDir::new();
    let boot = uuid::Uuid::new_v4();
    let clock = Arc::new(HealthDeadlinePort {
        mono: std::sync::atomic::AtomicU64::new(0),
        jobs: std::sync::Mutex::new(vec![]),
        mode: std::sync::atomic::AtomicU8::new(2),
        invitation_failed: std::sync::atomic::AtomicBool::new(false),
    });
    let fixture = HealthWakeFixture {
        store: health_wake_store(&dir.path().join("db"), clock.clone(), boot),
        fail: std::sync::atomic::AtomicU8::new(0),
        completion_hold: None,
    };
    let status = WorkerStatus::default();
    let observed = ObservedWakePort::new(&fixture, &status);
    let scheduler = Scheduler::new(
        "i".into(),
        clock.as_ref(),
        &observed,
        &HealthNotifier,
        RetryConfig::default(),
        boot,
    );
    let exhausted = CallBudget {
        deadline: MonoInstant(0),
        cancellation: Cancellation::default(),
    };
    observed.begin_drive(&exhausted);
    let first = scheduler.drive_wakes(&exhausted);
    assert_eq!(
        first.as_ref().unwrap_err().code,
        ErrorCode::DeadlineExceeded
    );
    observed.observe_drive(&first);
    assert!(status.last_error().is_some());
    let valid = CallBudget {
        deadline: MonoInstant(10000),
        cancellation: Cancellation::default(),
    };
    observed.begin_drive(&valid);
    let next = scheduler.drive_wakes(&valid);
    assert_eq!(next.as_ref().unwrap().attempted, 2);
    observed.observe_drive(&next);
    assert_eq!(status.last_error(), None);
}

#[test]
fn deadline_health_recovered_call_is_not_blocked_by_unrelated_incomplete_job() {
    use herdr_threads::scheduler::deadlines::DeadlineDriver;
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
    let port = HealthDeadlinePort {
        mono: AtomicU64::new(0),
        jobs: std::sync::Mutex::new(vec![false]),
        mode: AtomicU8::new(1),
        invitation_failed: AtomicBool::new(false),
    };
    let mut driver = DeadlineDriver::new(&port);
    let status = WorkerStatus::default();
    let first = driver.drive(&CallBudget {
        deadline: MonoInstant(0),
        cancellation: Cancellation::default(),
    });
    assert_eq!(
        first.as_ref().unwrap_err().code,
        ErrorCode::DeadlineExceeded
    );
    status.observe_drive(&first);
    assert!(status.last_error().is_some());
    let next = driver
        .drive(&CallBudget {
            deadline: MonoInstant(10000),
            cancellation: Cancellation::default(),
        })
        .unwrap();
    assert!(!next.work_complete);
    assert!(next.work_job.is_some());
    assert!(next.work_error.is_none());
    status.observe(&next);
    assert_eq!(status.last_error(), None);
}

struct PartialHealthPort {
    base: HealthDeadlinePort,
    retirement_present: bool,
    retirement_fail: std::sync::atomic::AtomicBool,
    retirement_complete: std::sync::atomic::AtomicBool,
    due_fail: std::sync::atomic::AtomicBool,
    work_top_error: std::sync::atomic::AtomicBool,
    retirement_calls: std::sync::atomic::AtomicU64,
    work_calls: std::sync::atomic::AtomicU64,
    work_prefix: std::sync::Mutex<Option<herdr_threads::ports::WorkProgress>>,
    callbacks: std::sync::Mutex<Vec<&'static str>>,
}
impl PartialHealthPort {
    fn new(retirement_present: bool) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
        Self {
            base: HealthDeadlinePort {
                mono: AtomicU64::new(0),
                jobs: std::sync::Mutex::new(vec![]),
                mode: AtomicU8::new(2),
                invitation_failed: AtomicBool::new(false),
            },
            retirement_present,
            retirement_fail: AtomicBool::new(false),
            retirement_complete: AtomicBool::new(false),
            due_fail: AtomicBool::new(false),
            work_top_error: AtomicBool::new(false),
            retirement_calls: AtomicU64::new(0),
            work_calls: AtomicU64::new(0),
            work_prefix: std::sync::Mutex::new(None),
            callbacks: std::sync::Mutex::new(vec![]),
        }
    }
    fn error(detail: &str) -> herdr_threads::protocol::results::ApiError {
        herdr_threads::protocol::results::ApiError {
            code: ErrorCode::StoreBusy,
            detail: detail.into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }
    }
}
impl herdr_threads::scheduler::deadlines::DeadlinePort for PartialHealthPort {
    fn clock(&self) -> &dyn Clock {
        &self.base
    }
    fn due_obligations(
        &self,
        request: herdr_threads::ports::DueScanRequest,
        budget: &CallBudget,
    ) -> Result<DueScanProgress, herdr_threads::protocol::results::ApiError> {
        self.callbacks.lock().unwrap().push("due");
        if self
            .due_fail
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            Err(Self::error("later due callback failed"))
        } else {
            herdr_threads::scheduler::deadlines::DeadlinePort::due_obligations(
                &self.base, request, budget,
            )
        }
    }
    fn pending_retirement_jobs(
        &self,
        _: herdr_threads::protocol::pagination::PageRequest,
        _: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<
            herdr_threads::protocol::results::RetirementStatus,
        >,
        herdr_threads::protocol::results::ApiError,
    > {
        use herdr_threads::protocol::{
            ids::{RetirementJobId, SeatId},
            results::{CleanupState, RetirementStatus},
        };
        Ok(health_page(
            if self.retirement_present {
                vec![RetirementStatus {
                    job: RetirementJobId::new("retirement"),
                    seat: SeatId::new("seat"),
                    effective_retired: true,
                    retired_at: UtcMillis(0),
                    cleanup_state: if self
                        .retirement_complete
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        CleanupState::Complete
                    } else {
                        CleanupState::Pending
                    },
                    warning_history_complete: self
                        .retirement_complete
                        .load(std::sync::atomic::Ordering::SeqCst),
                    phase: "warnings".into(),
                    processed_units: 0,
                    remaining_estimate: None,
                    last_error: None,
                }]
            } else {
                vec![]
            },
            None,
            u64::from(self.retirement_present),
        ))
    }
    fn advance_retirement(
        &self,
        job: herdr_threads::protocol::ids::RetirementJobId,
        _: herdr_threads::ports::WorkAdmission,
        _: &CallBudget,
    ) -> Result<herdr_threads::ports::RetirementProgress, herdr_threads::protocol::results::ApiError>
    {
        assert_eq!(job.as_str(), "retirement");
        self.callbacks.lock().unwrap().push("retirement");
        self.retirement_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .retirement_fail
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            Err(Self::error("earlier retirement callback failed"))
        } else {
            let was_complete = self
                .retirement_complete
                .swap(true, std::sync::atomic::Ordering::SeqCst);
            Ok(herdr_threads::ports::RetirementProgress {
                job,
                processed_this_turn: u8::from(!was_complete),
                processed_total: 1,
                complete: true,
                warning_history_complete: true,
                last_error: None,
            })
        }
    }
    fn pending_work(
        &self,
        page: herdr_threads::protocol::pagination::PageRequest,
        budget: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::pagination::Page<herdr_threads::ports::WorkCandidate>,
        herdr_threads::protocol::results::ApiError,
    > {
        let mut result = herdr_threads::scheduler::deadlines::DeadlinePort::pending_work(
            &self.base, page, budget,
        )?;
        for job in &mut result.items {
            job.high_water = 2;
            job.position = self
                .work_prefix
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |prefix| prefix.next_position);
        }
        Ok(result)
    }
    fn advance_work(
        &self,
        job: &str,
        admission: herdr_threads::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<herdr_threads::ports::WorkProgress, herdr_threads::protocol::results::ApiError>
    {
        assert_eq!(job, "job-0");
        self.callbacks.lock().unwrap().push("work");
        self.work_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .work_top_error
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Self::error("earlier top-level work callback failed"));
        }
        let prior = self
            .work_prefix
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |prefix| prefix.next_position);
        let mut result = herdr_threads::scheduler::deadlines::DeadlinePort::advance_work(
            &self.base, job, admission, budget,
        )?;
        result.next_position = if result.has_more { 1 } else { 2 };
        result.completed_units = result.next_position;
        // Units this call actually committed past the durable position.
        result.processed_this_turn = u8::try_from(result.next_position.saturating_sub(prior))
            .expect("fixture positions are small");
        *self.work_prefix.lock().unwrap() = Some(result.clone());
        Ok(result)
    }
}
fn observe_partial_driver(
    driver: &mut herdr_threads::scheduler::deadlines::DeadlineDriver<'_, PartialHealthPort>,
    status: &WorkerStatus,
    budget: &CallBudget,
) -> Result<DriveOutcome, herdr_threads::protocol::results::ApiError> {
    status.drive_deadlines(driver, budget)
}

#[test]
fn deadline_health_preserves_retirement_failure_before_due_error_through_cooldown() {
    use herdr_threads::scheduler::deadlines::DeadlineDriver;
    use std::sync::atomic::Ordering;
    let port = PartialHealthPort::new(true);
    port.retirement_fail.store(true, Ordering::SeqCst);
    port.due_fail.store(true, Ordering::SeqCst);
    let status = WorkerStatus::default();
    let mut driver = DeadlineDriver::new(&port);
    let budget = CallBudget {
        deadline: MonoInstant(20000),
        cancellation: Cancellation::default(),
    };
    let first = observe_partial_driver(&mut driver, &status, &budget);
    assert_eq!(first.unwrap_err().detail, "later due callback failed");
    assert_eq!(*port.callbacks.lock().unwrap(), ["retirement", "due"]);
    assert_eq!(port.retirement_calls.load(Ordering::SeqCst), 1);
    port.base.mono.store(1000, Ordering::SeqCst);
    let retry = observe_partial_driver(&mut driver, &status, &budget).unwrap();
    assert!(matches!(
        retry.due_progress.unwrap().invitations,
        DuePhaseProgress::Complete
    ));
    assert!(retry.retirement_job.is_none());
    assert_eq!(port.retirement_calls.load(Ordering::SeqCst), 1);
    assert!(
        status
            .last_error()
            .as_deref()
            .is_some_and(|error| error == "retirement cleanup failed: StoreBusy"),
        "known failure vanished before actual retry: {:?}",
        status.last_error()
    );
    port.base.mono.store(5000, Ordering::SeqCst);
    port.due_fail.store(true, Ordering::SeqCst);
    assert_eq!(
        observe_partial_driver(&mut driver, &status, &budget)
            .unwrap_err()
            .detail,
        "later due callback failed"
    );
    assert_eq!(port.retirement_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        status.last_error().as_deref(),
        Some("deadline drive failed: StoreBusy")
    );
    port.base.mono.store(6000, Ordering::SeqCst);
    observe_partial_driver(&mut driver, &status, &budget).unwrap();
    assert_eq!(status.last_error(), None);
    assert!(status.last_diagnostic().is_some());
}

#[test]
fn deadline_health_preserves_work_first_prefix_and_top_level_errors_before_due_error() {
    use herdr_threads::scheduler::deadlines::DeadlineDriver;
    use std::sync::atomic::Ordering;
    for (top_error, peer_retirement) in [(false, false), (true, false), (false, true)] {
        let port = PartialHealthPort::new(peer_retirement);
        let status = WorkerStatus::default();
        let mut driver = DeadlineDriver::new(&port);
        let budget = CallBudget {
            deadline: MonoInstant(20000),
            cancellation: Cancellation::default(),
        };
        observe_partial_driver(&mut driver, &status, &budget).unwrap();
        port.base.mono.store(1000, Ordering::SeqCst);
        observe_partial_driver(&mut driver, &status, &budget).unwrap();
        *port.base.jobs.lock().unwrap() = vec![false];
        port.base
            .mode
            .store(if top_error { 2 } else { 0 }, Ordering::SeqCst);
        port.work_top_error.store(top_error, Ordering::SeqCst);
        port.retirement_fail
            .store(peer_retirement, Ordering::SeqCst);
        port.due_fail.store(true, Ordering::SeqCst);
        port.callbacks.lock().unwrap().clear();
        port.base.mono.store(2000, Ordering::SeqCst);
        assert_eq!(
            observe_partial_driver(&mut driver, &status, &budget)
                .unwrap_err()
                .detail,
            "later due callback failed"
        );
        let calls = port.callbacks.lock().unwrap().clone();
        assert_eq!(
            calls,
            if peer_retirement {
                vec!["work", "retirement", "due"]
            } else {
                vec!["work", "due"]
            }
        );
        assert_eq!(port.work_calls.load(Ordering::SeqCst), 1);
        if !top_error {
            let prefix = port.work_prefix.lock().unwrap().clone().unwrap();
            assert_eq!(
                (
                    prefix.completed_units,
                    prefix.next_position,
                    prefix.has_more
                ),
                (1, 1, true)
            );
            assert!(prefix.last_error.is_some());
        }
        port.base.mono.store(3000, Ordering::SeqCst);
        observe_partial_driver(&mut driver, &status, &budget).unwrap();
        assert_eq!(port.work_calls.load(Ordering::SeqCst), 1);
        assert!(
            status.last_error().is_some(),
            "work failure vanished during cooldown: top={top_error}, peer={peer_retirement}"
        );
        port.base
            .mode
            .store(if peer_retirement { 1 } else { 2 }, Ordering::SeqCst);
        port.base.mono.store(7000, Ordering::SeqCst);
        observe_partial_driver(&mut driver, &status, &budget).unwrap();
        if peer_retirement {
            assert_eq!(
                status.last_error().as_deref(),
                Some("work job failed: StoreBusy")
            );
            port.base.mode.store(2, Ordering::SeqCst);
            port.base.mono.store(8000, Ordering::SeqCst);
            observe_partial_driver(&mut driver, &status, &budget).unwrap();
        }
        assert_eq!(status.last_error(), None);
    }
}
