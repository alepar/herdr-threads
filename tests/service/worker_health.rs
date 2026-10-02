//! Every `WorkerStatus` feeder reaches production Health only as a typed
//! redacted class and code. Health here is always built by the shared
//! production builder `app::elected_health_provider` (the one `run_elected`
//! uses), over a real `SqliteStore`, with the lane statuses in production
//! order: deadline, wake, observation.
use super::*;
use crate::daemon::health::{ComponentStatus, HealthInputs};
use crate::ports::{
    DuePhaseProgress, DueScanProgress, DueScanRequest, RetirementProgress, StorePort,
    WorkCandidate, WorkKind,
};
use crate::protocol::{
    pagination::{Consistency, Page, PageRequest, StopReason},
    results::{ApiError, BoundedError, CapabilityState, ErrorCode, RetirementStatus},
    time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
};
use crate::scheduler::deadlines::{DeadlineDriver, DeadlinePort};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

/// A private scratch directory for one real SQLite store.
struct ScratchDir(std::path::PathBuf);
impl ScratchDir {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path =
            std::env::temp_dir().join(format!("herdr-worker-health-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn api_error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}

fn page<T>(items: Vec<T>) -> Page<T> {
    Page {
        items,
        has_more: false,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 1,
        scope_revision: None,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

/// The production Health builder over a real, empty SQLite store.
struct Production {
    _dir: ScratchDir,
    clock: Arc<TestClock>,
    instance: uuid::Uuid,
    store: Arc<dyn StorePort>,
    health: Box<dyn Fn(&CallBudget) -> HealthInputs>,
}
impl Production {
    fn new(workers: [Arc<WorkerStatus>; 3]) -> Self {
        let dir = ScratchDir::new();
        let clock = Arc::new(TestClock(AtomicU64::new(10_000)));
        let instance = uuid::Uuid::new_v4();
        let store: Arc<dyn StorePort> = Arc::new(
            crate::store::SqliteStore::new(
                crate::store::connection::StoreContext::new(dir.0.join("db"), clock.clone()),
                instance.to_string(),
                crate::store::StoreSettings::default(),
            )
            .unwrap(),
        );
        let health = crate::app::elected_health_provider(
            instance,
            uuid::Uuid::new_v4(),
            crate::service::config::ServiceConfig::default().health_settings(),
            clock.clone(),
            store.clone(),
            workers,
            crate::app::ElectedHostEvidence {
                status: Default::default(),
                incarnation_witness: CapabilityState::Unknown,
                safe_prompt: CapabilityState::Unsupported,
                harnesses: Default::default(),
            },
        );
        Self {
            _dir: dir,
            clock,
            instance,
            store,
            health: Box::new(health),
        }
    }
    fn scheduler(&self) -> ComponentStatus {
        (self.health)(&self.budget()).scheduler
    }
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.0.load(Ordering::SeqCst) + 1_000),
            cancellation: Cancellation::default(),
        }
    }
    /// The scheduler component carries exactly `expected`, and neither it
    /// nor the serialized Health carries any private text.
    fn assert_redacted(&self, expected: &str, private: &[&str]) {
        let inputs = (self.health)(&self.budget());
        assert_eq!(
            inputs.scheduler,
            ComponentStatus::Degraded(expected.into()),
            "production Health scheduler component"
        );
        let serialized = serde_json::to_string(&inputs.assemble()).unwrap();
        assert!(
            serialized.contains(&format!("scheduler degraded: {expected}")),
            "{serialized}"
        );
        for text in private {
            assert!(
                !serialized.contains(text),
                "private text {text:?} reached Health: {serialized}"
            );
        }
    }
}

/// Deadline port whose due, retirement discovery, retirement quantum, work
/// discovery and work quantum results are scripted per call. An empty script
/// is a complete due scan and empty discovery.
#[derive(Default)]
struct ScriptedDeadlines {
    clock: Arc<TestClock>,
    due: Mutex<Vec<Result<(DuePhaseProgress, DuePhaseProgress), ApiError>>>,
    retirement_discovery: Mutex<Vec<Result<bool, ApiError>>>,
    retirement: Mutex<Vec<Result<RetirementProgress, ApiError>>>,
    discovery: Mutex<Vec<Result<bool, ApiError>>>,
    work: Mutex<Vec<Result<crate::ports::WorkProgress, ApiError>>>,
}
const PRIVATE_RETIREMENT_JOB: &str = "retirement-private-7c2d";
impl Default for TestClock {
    fn default() -> Self {
        Self(AtomicU64::new(10_000))
    }
}
impl DeadlinePort for ScriptedDeadlines {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        _: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        let mut due = self.due.lock().unwrap();
        let (invitations, receipts) = if due.is_empty() {
            (DuePhaseProgress::Complete, DuePhaseProgress::Complete)
        } else {
            due.remove(0)?
        };
        Ok(DueScanProgress {
            state: request.state,
            examined_candidates: 0,
            warnings_added: 0,
            invitations: if request.run_invitations {
                invitations
            } else {
                DuePhaseProgress::Skipped
            },
            receipts: if request.run_receipts {
                receipts
            } else {
                DuePhaseProgress::Skipped
            },
            has_more: false,
        })
    }
    fn pending_retirement_jobs(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError> {
        let mut discovery = self.retirement_discovery.lock().unwrap();
        let returned = if discovery.is_empty() {
            false
        } else {
            discovery.remove(0)?
        };
        Ok(page(if returned {
            vec![RetirementStatus {
                job: crate::protocol::ids::RetirementJobId::new(PRIVATE_RETIREMENT_JOB),
                seat: crate::protocol::ids::SeatId::new("seat-private-7c2d"),
                effective_retired: true,
                retired_at: UtcMillis(1),
                cleanup_state: crate::protocol::results::CleanupState::Pending,
                warning_history_complete: false,
                phase: "cleanup".into(),
                processed_units: 0,
                remaining_estimate: None,
                last_error: None,
            }]
        } else {
            vec![]
        }))
    }
    fn advance_retirement(
        &self,
        _: crate::protocol::ids::RetirementJobId,
        _: crate::ports::WorkAdmission,
        _: &CallBudget,
    ) -> Result<RetirementProgress, ApiError> {
        self.retirement.lock().unwrap().remove(0)
    }
    fn pending_work(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WorkCandidate>, ApiError> {
        let mut discovery = self.discovery.lock().unwrap();
        let returned = if discovery.is_empty() {
            false
        } else {
            discovery.remove(0)?
        };
        Ok(page(if returned {
            vec![WorkCandidate {
                id: "work:cleanup:prep-private-5e1b".into(),
                kind: WorkKind::SendAttention,
                position: 0,
                high_water: 64,
                has_more: true,
            }]
        } else {
            vec![]
        }))
    }
    fn advance_work(
        &self,
        _: &str,
        _: crate::ports::DurableWorkAdmission,
        _: &CallBudget,
    ) -> Result<crate::ports::WorkProgress, ApiError> {
        self.work.lock().unwrap().remove(0)
    }
}

/// One deadline worker turn past the tick and every retry backoff, through
/// `WorkerStatus::drive_deadlines` (the deadline worker's own call).
fn deadline_turn(
    port: &ScriptedDeadlines,
    driver: &mut DeadlineDriver<'_, ScriptedDeadlines>,
    status: &WorkerStatus,
) -> Result<crate::scheduler::deadlines::DriveOutcome, ApiError> {
    port.clock.0.fetch_add(10_000, Ordering::SeqCst);
    status.drive_deadlines(
        driver,
        &CallBudget {
            deadline: MonoInstant(port.clock.0.load(Ordering::SeqCst) + 500),
            cancellation: Cancellation::default(),
        },
    )
}

#[test]
fn production_health_redacts_work_failures_and_clears_work_job_on_committed_progress() {
    // Kills (private exact detail): `job_failure` returning only the redacted
    // `failure.summary()` as its diagnostic (the pre-fix1 shape): a
    // discovery or top-level `advance_work` Err then keeps its exact detail
    // nowhere, and the `last_diagnostic` equalities fail.
    //
    // Kills (redaction): `job_failure` rendering the pre-fix
    // `format!("{:?}: {}", error.code, error.detail)` for `FailureKey::Work`
    // or `FailureKey::WorkDiscovery`: the private store text and the work
    // job identity then reach the scheduler component and serialized Health.
    //
    // Kills (clear-on-committed-progress, work lane): the deadline driver
    // never setting `DriveOutcome::work_progressed`, or `WorkerStatus::observe`
    // passing `false` as the work lane's `progressed` (the pre-fix
    // behaviour): the scheduler then stays Degraded after a committed,
    // non-completing work quantum.
    let deadline = Arc::new(WorkerStatus::default());
    let production = Production::new([deadline.clone(), Default::default(), Default::default()]);
    let port = ScriptedDeadlines {
        clock: production.clock.clone(),
        ..Default::default()
    };
    *port.discovery.lock().unwrap() = vec![
        Err(api_error(
            ErrorCode::StoreBusy,
            "SQLite: private work discovery failure",
        )),
        Ok(true),
        Ok(true),
        Ok(true),
        Ok(true),
    ];
    *port.work.lock().unwrap() = vec![
        Ok(crate::ports::WorkProgress {
            completed_units: 1,
            processed_this_turn: 1,
            has_more: true,
            next_position: 1,
            last_error: Some("SQLite: private work unit failure".into()),
        }),
        // A top-level `advance_work` Err persists nothing on the work row.
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "SQLite: private top-level work transaction begin failure",
        )),
        // Cooldown-free idle: the job is offered but commits nothing.
        Ok(crate::ports::WorkProgress {
            completed_units: 1,
            processed_this_turn: 0,
            has_more: true,
            next_position: 1,
            last_error: None,
        }),
        Ok(crate::ports::WorkProgress {
            completed_units: 5,
            processed_this_turn: 4,
            has_more: true,
            next_position: 5,
            last_error: None,
        }),
    ];
    let mut driver = DeadlineDriver::new(&port);
    let private = ["private", "prep-private-5e1b", "SQLite"];

    let failed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(failed.work_error.is_some() && failed.work_job.is_none());
    production.assert_redacted("work discovery failed: StoreBusy", &private);
    // Discovery errors are persisted nowhere: the exact detail is kept only
    // in the lane's private diagnostic tail.
    assert_eq!(
        deadline.last_diagnostic().unwrap(),
        "work discovery: StoreBusy: SQLite: private work discovery failure"
    );

    let failed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert_eq!(
        failed.work_job.as_deref(),
        Some("work:cleanup:prep-private-5e1b")
    );
    production.assert_redacted("work job failed: StoreBusy", &private);
    assert_eq!(
        deadline.last_diagnostic().unwrap(),
        "work job: StoreBusy: SQLite: private work unit failure"
    );

    // Top-level job Err: Health keeps the class, the tail the exact detail.
    let failed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert_eq!(
        failed.work_error.as_ref().unwrap().code,
        ErrorCode::StoreCorrupt
    );
    production.assert_redacted("work job failed: StoreCorrupt", &private);
    assert_eq!(
        deadline.last_diagnostic().unwrap(),
        "work job: StoreCorrupt: SQLite: private top-level work transaction begin failure"
    );

    let idle = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(idle.work_job.is_some() && !idle.work_progressed);
    production.assert_redacted("work job failed: StoreCorrupt", &private);

    let progressed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(progressed.work_progressed, "committed quantum");
    assert!(!progressed.work_complete, "quantum must be partial");
    assert_eq!(production.scheduler(), ComponentStatus::Ready);
}

#[test]
fn production_health_redacts_deadline_drive_and_due_phase_failures() {
    // Kills (due phase): `WorkerStatus::observe` recording the pre-fix
    // `Failure::detail(error.as_str())` for `FailureKey::Invitations` /
    // `FailureKey::Receipts`: the bounded store text reaches Health.
    //
    // Kills (general drive): `WorkerStatus::record_drive_error` recording the
    // pre-fix `format!("{:?}: {}", error.code, error.detail)` for
    // `FailureKey::General`.
    let deadline = Arc::new(WorkerStatus::default());
    let production = Production::new([deadline.clone(), Default::default(), Default::default()]);
    let port = ScriptedDeadlines {
        clock: production.clock.clone(),
        ..Default::default()
    };
    *port.due.lock().unwrap() = vec![
        Ok((
            DuePhaseProgress::Failed(
                ErrorCode::StoreBusy,
                BoundedError::parse("StoreBusy: private invitation scan text").unwrap(),
            ),
            DuePhaseProgress::Complete,
        )),
        Ok((DuePhaseProgress::Complete, DuePhaseProgress::Complete)),
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "SQLite: private due call failure",
        )),
    ];
    let mut driver = DeadlineDriver::new(&port);
    let private = ["private", "SQLite"];

    deadline_turn(&port, &mut driver, &deadline).unwrap();
    production.assert_redacted("invitation due scan failed: StoreBusy", &private);
    // The exact detail stays in the lane's private diagnostic tail.
    assert!(
        deadline
            .last_diagnostic()
            .unwrap()
            .contains("private invitation scan text")
    );

    deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert_eq!(production.scheduler(), ComponentStatus::Ready);

    assert!(deadline_turn(&port, &mut driver, &deadline).is_err());
    production.assert_redacted("deadline drive failed: StoreCorrupt", &private);
    assert!(
        deadline
            .last_diagnostic()
            .unwrap()
            .contains("private due call failure")
    );

    deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert_eq!(production.scheduler(), ComponentStatus::Ready);
}

/// A real store behind the production `ObservedWakePort`, failing one chosen
/// callback once with private detail.
struct FailingWakes {
    store: crate::store::SqliteStore,
    fail: std::sync::atomic::AtomicU8,
}
const FAIL_RESERVE: u8 = 1;
const FAIL_COMPLETE: u8 = 2;
impl FailingWakes {
    fn result<T>(
        &self,
        phase: u8,
        run: impl FnOnce() -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        if self
            .fail
            .compare_exchange(phase, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Err(api_error(
                ErrorCode::StoreBusy,
                "SQLite: private wake callback failure",
            ))
        } else {
            run()
        }
    }
}
impl crate::scheduler::WakePort for FailingWakes {
    fn clock(&self) -> &dyn Clock {
        StorePort::clock(&self.store)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<crate::ports::WakeCandidate>, ApiError> {
        StorePort::wake_candidates(&self.store, page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &crate::ports::WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::WakeReservation>, ApiError> {
        self.result(FAIL_RESERVE, || {
            StorePort::reserve_wake(&self.store, candidate, budget)
        })
    }
    fn complete_wake(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: crate::ports::WakeOutcome,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.result(FAIL_COMPLETE, || {
            StorePort::complete_wake(&self.store, attempt, outcome, budget)
        })
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<crate::ports::WakeRecoveryCandidate>, ApiError> {
        StorePort::wake_recovery_candidates(&self.store, page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: crate::ports::WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<crate::ports::WakeRecoveryOutcome, ApiError> {
        StorePort::recover_wake_reservation(&self.store, request, budget)
    }
}
struct SubmittedNotifier;
impl crate::ports::NotificationPort for SubmittedNotifier {
    fn attempt_wake(
        &self,
        _: crate::ports::WakeReservation,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::WakeOutcome, ApiError> {
        Ok(crate::ports::WakeOutcome::Submitted)
    }
}
const WAKE_SEAT: &str = "seat-private-7f3a";

fn wake_store(path: &std::path::Path, clock: Arc<TestClock>, boot: uuid::Uuid) -> FailingWakes {
    let context = crate::store::connection::StoreContext::new(path.into(), clock);
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);").unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [WAKE_SEAT]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant) VALUES ('i',?1,'host',1,1,0,'fresh','occupied','idle','execution',1)", [WAKE_SEAT]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?1,'host',1,'codex','session','execution','fresh',0,0,'term-'||?1,'inc')", [WAKE_SEAT]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'invited')",
        [WAKE_SEAT],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES (?1,'t',?1,1,'pending',0,1000000,1000000,1)", [WAKE_SEAT]).unwrap();
    drop(db);
    FailingWakes {
        store: crate::store::SqliteStore::new(
            context,
            "i",
            crate::store::StoreSettings {
                daemon_boot: Some(boot),
                ..Default::default()
            },
        )
        .unwrap(),
        fail: std::sync::atomic::AtomicU8::new(0),
    }
}

#[test]
fn production_health_redacts_wake_callback_identity_and_detail() {
    // Kills (wake callbacks): `WorkerStatus::health` rendering the wake
    // failure's private diagnostic (the pre-fix
    // `format!("{phase} {identity:?}: {:?}: {}", ...)` detail): the seat id
    // (reservation), the attempt id (completion) and the store text then
    // reach Health.
    //
    // Kills (wake driver): `ObservedWakePort::observe_drive` recording the
    // pre-fix `format!("unverified wake driver recovery: {:?}: {}", ...)`.
    use crate::scheduler::Scheduler;
    let wake = Arc::new(WorkerStatus::default());
    let production = Production::new([Default::default(), wake.clone(), Default::default()]);
    let boot = uuid::Uuid::new_v4();
    let fixture = wake_store(
        &production._dir.0.join("wakes"),
        production.clock.clone(),
        boot,
    );
    let observed = ObservedWakePort::new(&fixture, &wake);
    let deadlines = ScriptedDeadlines::default();
    let scheduler = Scheduler::new(
        "i".into(),
        &deadlines,
        &observed,
        &SubmittedNotifier,
        crate::notification::policy::RetryConfig::default(),
        boot,
    );
    let drive = || {
        let budget = CallBudget {
            deadline: MonoInstant(production.clock.0.load(Ordering::SeqCst) + 5_000),
            cancellation: Cancellation::default(),
        };
        observed.begin_drive(&budget);
        let result = scheduler.drive_wakes(&budget);
        observed.observe_drive(&result);
        result
    };

    fixture.fail.store(FAIL_RESERVE, Ordering::SeqCst);
    assert!(drive().is_err());
    production.assert_redacted(
        "wake reservation failed: StoreBusy",
        &[WAKE_SEAT, "seat-private", "private wake", "SQLite"],
    );
    // The exact detail, identity included, stays in the private tail.
    let tail = wake.last_diagnostic().unwrap();
    assert!(tail.contains(WAKE_SEAT) && tail.contains("private wake callback failure"));

    fixture.fail.store(FAIL_COMPLETE, Ordering::SeqCst);
    assert!(drive().is_err());
    let attempt: String = rusqlite::Connection::open(production._dir.0.join("wakes"))
        .unwrap()
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id=?1",
            [WAKE_SEAT],
            |r| r.get(0),
        )
        .unwrap();
    production.assert_redacted(
        "unsettled durable wake completion failed: StoreBusy",
        &[&attempt, WAKE_SEAT, "private wake", "SQLite"],
    );
    assert!(wake.last_diagnostic().unwrap().contains(&attempt));

    // The matching completion retry settles it.
    assert!(drive().is_ok());
    assert_eq!(production.scheduler(), ComponentStatus::Ready);

    // A driver failure no callback observed stays current, redacted.
    observed.observe_drive(&Err(api_error(
        ErrorCode::StoreCorrupt,
        "private wake scan lock detail",
    )));
    production.assert_redacted(
        "unverified wake driver recovery: StoreCorrupt",
        &["private wake scan lock detail"],
    );
}

#[test]
fn production_health_redacts_observation_capture_invalidation_and_reconciliation() {
    // Feeder-level only: this calls the typed feeders directly. The worker
    // loop's two call sites are covered by
    // `observation_worker_loop_feeds_production_health_at_both_call_sites`.
    //
    // Kills (capture): the observation worker recording the pre-fix
    // `format!("capture {:?}: {}", error.code, error.detail)` or
    // `format!("capture {reason:?}: {cause:?}")` free text: the host error
    // detail then reaches Health.
    //
    // Kills (reconciliation): recording the pre-fix
    // `format!("observation reconciliation {:?}: {}", ...)`, and
    // `observe_reconciliation` never clearing on a committed page (the
    // failure then outlives the page that recovered it).
    use crate::identity::reconcile::ObservationOutcome;
    use crate::ports::{HostInvalidationFence, HostInvalidationReason, HostObservationAdmission};
    let observation = Arc::new(WorkerStatus::default());
    let production = Production::new([Default::default(), Default::default(), observation.clone()]);
    let private = ["private", "/Users/", "socket"];

    observation.observe_capture(&Err(api_error(
        ErrorCode::HostUnavailable,
        "private host socket /Users/someone/.herdr/sock gone",
    )));
    production.assert_redacted("host observation capture failed: HostUnavailable", &private);
    assert!(
        observation
            .last_diagnostic()
            .unwrap()
            .contains("private host socket")
    );

    // A capture that is not due changes nothing.
    observation.observe_capture(&Ok(None));
    production.assert_redacted("host observation capture failed: HostUnavailable", &private);

    let fence = HostInvalidationFence {
        admission: HostObservationAdmission {
            instance: "i".into(),
            sequence: 1,
            expected_active: None,
            expected_boot: None,
            expected_epoch: 1,
            lifecycle_revision: 0,
            invalidation_revision: 0,
        },
        invalidation_revision: 1,
    };
    observation.observe_capture(&Ok(Some(ObservationOutcome::Invalidated {
        reason: HostInvalidationReason::PartialEnumeration,
        fence,
        cause: Some(api_error(
            ErrorCode::HostUnavailable,
            "private partial enumeration socket detail",
        )),
    })));
    production.assert_redacted("host observation invalidated: PartialEnumeration", &private);

    observation.observe_capture(&Ok(Some(ObservationOutcome::Superseded)));
    assert_eq!(production.scheduler(), ComponentStatus::Ready);

    observation.observe_reconciliation::<()>(&Err(api_error(
        ErrorCode::StoreBusy,
        "SQLite: private reconciliation failure",
    )));
    production.assert_redacted(
        "host observation reconciliation failed: StoreBusy",
        &["private", "SQLite"],
    );
    observation.observe_reconciliation(&Ok(()));
    assert_eq!(production.scheduler(), ComponentStatus::Ready);
}

/// A fake Herdr host for the real observation worker loop: each snapshot is a
/// coherent verified enumeration of one empty pane, or (while `fail` is set)
/// an `Err` carrying private host detail.
struct ObservedHost {
    clock: Arc<TestClock>,
    sequence: AtomicU64,
    fail: std::sync::atomic::AtomicBool,
    snapshots: AtomicU64,
}
impl ObservedHost {
    fn observation(&self, sequence: u64) -> crate::ports::HostObservation {
        use crate::ports::*;
        use crate::protocol::ids::*;
        let at = self.clock.monotonic_now();
        HostObservation {
            target: HostTargetId::new("pane"),
            host_boot: HostBootId::new("host"),
            epoch: 1,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: at,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new("terminal")),
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: at,
            completed_at_mono: at,
        }
    }
}
impl crate::ports::HostPort for ObservedHost {
    fn native_launch_capability(&self) -> crate::ports::NativeLaunchCapability {
        crate::ports::NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::HostObservation, ApiError> {
        unreachable!("the observation worker only enumerates")
    }
    fn enumerate_targets(
        &self,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::HostSnapshot, ApiError> {
        use crate::ports::*;
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(api_error(
                ErrorCode::HostUnavailable,
                "private host socket /Users/someone/.herdr/sock refused",
            ));
        }
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        Ok(HostSnapshot {
            boot: crate::protocol::ids::HostBootId::new("host"),
            epoch: 1,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: vec![self.observation(sequence)],
        })
    }
    fn subscribe_lifecycle(
        &self,
        _: &crate::ports::HostCallContext,
    ) -> Result<Box<dyn crate::ports::HostLifecycleSubscription>, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        _: &crate::protocol::ids::SeatId,
        _: &crate::ports::HostObservation,
    ) -> Option<crate::ports::SafeWakeTarget> {
        unreachable!()
    }
    fn submit_prompt(
        &self,
        _: &crate::ports::SafeWakeTarget,
        _: &str,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::PromptOutcome, ApiError> {
        unreachable!()
    }
    fn launch_native(
        &self,
        _: crate::ports::NativeLaunchRequest,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
}

/// The production `ScheduledStore` (real SQLite behind the fair writer),
/// failing admission or the published-snapshot reconciliation page with
/// private store detail while the matching switch is set. Every other call
/// is the production call.
struct FaultyObservationStore {
    inner: ScheduledStore,
    fail_admission: std::sync::atomic::AtomicBool,
    fail_page: std::sync::atomic::AtomicBool,
    pages_ok: AtomicU64,
    pass_records: AtomicU64,
    /// While set, every published page carries one synthetic saved seat whose
    /// planned transition the store refuses (`Stale`).
    refuse_transitions: std::sync::atomic::AtomicBool,
}
impl crate::identity::reconcile::observation_store::ObservationStore
    for Arc<FaultyObservationStore>
{
    fn clock(&self) -> &dyn Clock {
        crate::identity::reconcile::observation_store::ObservationStore::clock(&self.inner)
    }
    fn begin_host_observation(
        &self,
        instance: &str,
        budget: &CallBudget,
    ) -> Result<crate::ports::HostObservationAdmission, ApiError> {
        if self.fail_admission.load(Ordering::SeqCst) {
            return Err(api_error(
                ErrorCode::StoreBusy,
                "SQLite: private admission lock detail",
            ));
        }
        self.inner.begin_host_observation(instance, budget)
    }
    fn invalidate_host_observation(
        &self,
        admission: &crate::ports::HostObservationAdmission,
        reason: crate::ports::HostInvalidationReason,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::HostInvalidationFence>, ApiError> {
        self.inner
            .invalidate_host_observation(admission, reason, budget)
    }
    fn mark_unresolved_from_invalidation(
        &self,
        transition: crate::ports::GuardedInvalidationTransition,
        budget: &CallBudget,
    ) -> Result<crate::ports::ReconciliationOutcome, ApiError> {
        self.inner
            .mark_unresolved_from_invalidation(transition, budget)
    }
    fn saved_seats_page_for_invalidation(
        &self,
        fence: &crate::ports::HostInvalidationFence,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<crate::ports::InvalidationSeatPage, ApiError> {
        self.inner.saved_seats_page_for_invalidation(
            fence,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )
    }
    fn begin_snapshot_stage(
        &self,
        header: crate::ports::SnapshotHeader,
        budget: &CallBudget,
    ) -> Result<crate::ports::SnapshotStage, ApiError> {
        self.inner.begin_snapshot_stage(header, budget)
    }
    fn stage_snapshot_targets(
        &self,
        stage: &crate::ports::SnapshotGenerationId,
        offset: u64,
        targets: &[crate::ports::HostObservation],
        admission: crate::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<crate::ports::SnapshotStageProgress, ApiError> {
        self.inner
            .stage_snapshot_targets(stage, offset, targets, admission, budget)
    }
    fn seal_snapshot_stage(
        &self,
        stage: &crate::ports::SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<crate::ports::SnapshotStage, ApiError> {
        self.inner.seal_snapshot_stage(stage, budget)
    }
    fn publish_snapshot_stage(
        &self,
        stage: &crate::ports::SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<crate::ports::PublishedSnapshot, ApiError> {
        self.inner.publish_snapshot_stage(stage, budget)
    }
    fn discard_snapshot_stage(
        &self,
        stage: &crate::ports::SnapshotGenerationId,
        admission: crate::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<crate::ports::SnapshotCleanupProgress, ApiError> {
        self.inner.discard_snapshot_stage(stage, admission, budget)
    }
    fn saved_seats_page(
        &self,
        published: &crate::ports::SnapshotGenerationId,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<crate::ports::SnapshotSeatPage, ApiError> {
        if self.fail_page.load(Ordering::SeqCst) {
            return Err(api_error(
                ErrorCode::StoreBusy,
                "SQLite: private reconciliation page detail",
            ));
        }
        let mut page = self.inner.saved_seats_page(
            published,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )?;
        if self.refuse_transitions.load(Ordering::SeqCst) {
            use crate::ports::*;
            use crate::protocol::ids::*;
            // A resolved seat whose target is absent from the publication
            // plans a retirement; `apply_reconciliation_transition` refuses it.
            page.seats.push(SnapshotSavedSeat {
                ordinal: page.high_water_ordinal + 1,
                seat: SeatId::new("synthetic-seat"),
                state: SeatState::Resolved,
                unresolved_reason: None,
                prior_published_observation: None,
                structural_proof: None,
                target: Some(HostTargetId::new("absent-pane")),
                terminal: None,
                binding_generation: 1,
                binding_execution: None,
                active_binding_execution: None,
                bound_epoch: None,
                bound_boot: None,
                bound_incarnation: None,
                latest_binding_evidence: None,
                observed_match: None,
            });
            page.visited += 1;
        }
        self.pages_ok.fetch_add(1, Ordering::SeqCst);
        Ok(page)
    }
    fn apply_reconciliation_transition(
        &self,
        transition: crate::ports::GuardedSeatTransition,
        budget: &CallBudget,
    ) -> Result<crate::ports::ReconciliationOutcome, ApiError> {
        if self.refuse_transitions.load(Ordering::SeqCst) {
            return Ok(crate::ports::ReconciliationOutcome::Stale);
        }
        self.inner
            .apply_reconciliation_transition(transition, budget)
    }
    fn record_reconciliation_pass(
        &self,
        published: &crate::ports::PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.pass_records.fetch_add(1, Ordering::SeqCst);
        self.inner.record_reconciliation_pass(published, budget)
    }
}

/// Cancels and joins the observation worker thread. Tests call
/// [`StopWorker::stop`] explicitly, which asserts the worker exited without
/// panicking (production treats a panicked worker as a shutdown failure).
/// `Drop` remains only for the panicking path, where it still cancels and
/// joins but does not add a second panic.
struct StopWorker(Cancellation, Option<std::thread::JoinHandle<()>>);
impl StopWorker {
    fn stop(mut self) {
        self.0.cancel();
        let handle = self.1.take().expect("worker joined once");
        assert!(
            handle.join().is_ok(),
            "observation worker panicked instead of exiting on cancellation"
        );
    }
}
impl Drop for StopWorker {
    fn drop(&mut self) {
        self.0.cancel();
        if let Some(handle) = self.1.take() {
            let joined = handle.join();
            if !std::thread::panicking() {
                assert!(
                    joined.is_ok(),
                    "observation worker panicked instead of exiting on cancellation"
                );
            }
        }
    }
}

#[test]
fn observation_worker_loop_feeds_production_health_at_both_call_sites() {
    // Drives the real observation worker loop (`spawn_observation_loop`,
    // which `start_observation_worker` calls with the production
    // `ScheduledStore`) with a real `OrdinaryIdentity` over a real SQLite
    // store. Health is read only through `app::elected_health_provider`.
    //
    // Kills (capture call site): deleting `status.observe_capture(&captured)`
    // from the worker loop: neither the errored capture nor the invalidation
    // ever reaches Health (phases A and B time out).
    //
    // Kills (reconciliation call site): deleting
    // `status.observe_reconciliation(&page)` from the worker loop: the failed
    // published-snapshot page never reaches Health (phase C times out).
    //
    // Kills (worker panic on cancellation): a `panic!` after the loop's
    // `while`: `StopWorker::stop` joins explicitly and asserts success.
    //
    // Kills (redaction at the Health builder): `ElectedHealth::inputs`
    // rendering a lane's `last_diagnostic()` instead of
    // `health().summary()`: the private host and store text then reaches
    // the scheduler component (`assert_redacted`).
    let observation = Arc::new(WorkerStatus::default());
    let production = Production::new([Default::default(), Default::default(), observation.clone()]);
    let clock = production.clock.clone();
    let host = Arc::new(ObservedHost {
        clock: clock.clone(),
        sequence: AtomicU64::new(1),
        fail: Default::default(),
        snapshots: AtomicU64::new(0),
    });
    let writer = Arc::new(FairWriter::new(32));
    let identity = Arc::new(crate::identity::repair::OrdinaryIdentity::new(
        production.instance.to_string(),
        production.store.clone(),
        host.clone(),
        clock.clone(),
        writer.clone(),
    ));
    let port = Arc::new(FaultyObservationStore {
        inner: ScheduledStore {
            store: production.store.clone(),
            writer,
        },
        fail_admission: Default::default(),
        fail_page: Default::default(),
        pages_ok: AtomicU64::new(0),
        pass_records: AtomicU64::new(0),
        refuse_transitions: Default::default(),
    });
    let private = ["private", "/Users/", "SQLite", "socket"];
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !done() {
            assert!(
                std::time::Instant::now() < until,
                "{what}: the observation worker never fed Health: {:?}",
                production.scheduler()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let degraded =
        |expected: &str| production.scheduler() == ComponentStatus::Degraded(expected.into());
    // A new capture is due once the 5 s snapshot cadence has elapsed.
    let next_capture = || {
        clock.0.fetch_add(5_000, Ordering::SeqCst);
    };

    // Phase A: the capture ends in `Err` (admission fails in the store).
    port.fail_admission.store(true, Ordering::SeqCst);
    let cancellation = Cancellation::default();
    let worker = StopWorker(
        cancellation.clone(),
        Some(
            spawn_observation_loop(
                identity,
                port.clone(),
                cancellation,
                observation.clone(),
                Arc::new(Default::default()),
            )
            .unwrap(),
        ),
    );
    wait_for("errored capture", &|| {
        degraded("host observation capture failed: StoreBusy")
    });
    production.assert_redacted("host observation capture failed: StoreBusy", &private);
    assert!(
        observation
            .last_diagnostic()
            .unwrap()
            .contains("private admission lock detail"),
        "exact capture detail kept privately"
    );
    assert_eq!(host.snapshots.load(Ordering::SeqCst), 0);

    // Phase B: admission succeeds but the host read fails: a durable
    // `Invalidated(HostUnavailable)` replaces the capture error.
    host.fail.store(true, Ordering::SeqCst);
    port.fail_admission.store(false, Ordering::SeqCst);
    next_capture();
    wait_for("invalidated capture", &|| {
        degraded("host observation invalidated: HostUnavailable")
    });
    production.assert_redacted("host observation invalidated: HostUnavailable", &private);
    assert!(host.snapshots.load(Ordering::SeqCst) >= 1);

    // Phase C: a verified capture publishes (clearing the capture failure),
    // then the published-snapshot reconciliation page fails.
    host.fail.store(false, Ordering::SeqCst);
    port.fail_page.store(true, Ordering::SeqCst);
    next_capture();
    wait_for("failed reconciliation page", &|| {
        degraded("host observation reconciliation failed: StoreBusy")
    });
    production.assert_redacted(
        "host observation reconciliation failed: StoreBusy",
        &private,
    );
    assert!(
        observation
            .last_diagnostic()
            .unwrap()
            .contains("private reconciliation page detail"),
        "exact reconciliation detail kept privately"
    );
    assert_eq!(port.pages_ok.load(Ordering::SeqCst), 0);

    // Phase D: the next published capture's committed page clears it.
    port.fail_page.store(false, Ordering::SeqCst);
    next_capture();
    wait_for("recovered", &|| {
        production.scheduler() == ComponentStatus::Ready
    });
    assert!(port.pages_ok.load(Ordering::SeqCst) >= 1);
    worker.stop();
}

#[test]
fn production_health_redacts_retirement_failures_and_keeps_exact_detail_private() {
    // Kills (redaction at the Health builder): `ElectedHealth::inputs`
    // rendering the lane's `last_diagnostic()` (which now carries the exact
    // detail) instead of `health().summary()`: the private store text then
    // reaches the scheduler component and serialized Health.
    //
    // Kills (private exact detail): `job_failure` returning only the redacted
    // `failure.summary()` as its diagnostic (the pre-fix1 shape): the
    // top-level `advance_retirement` Err and the retirement discovery Err are
    // persisted nowhere, so their exact detail would be lost.
    //
    // Kills (clear on completion): `WorkerStatus::observe` not clearing
    // `FailureKey::Retirement(job)` on a completed quantum.
    let deadline = Arc::new(WorkerStatus::default());
    let production = Production::new([deadline.clone(), Default::default(), Default::default()]);
    let port = ScriptedDeadlines {
        clock: production.clock.clone(),
        ..Default::default()
    };
    *port.retirement_discovery.lock().unwrap() = vec![
        Err(api_error(
            ErrorCode::StoreBusy,
            "SQLite: private retirement discovery failure",
        )),
        Ok(true),
        Ok(true),
    ];
    *port.retirement.lock().unwrap() = vec![
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "SQLite: private retirement transaction begin failure",
        )),
        Ok(RetirementProgress {
            job: crate::protocol::ids::RetirementJobId::new(PRIVATE_RETIREMENT_JOB),
            processed_this_turn: 1,
            processed_total: 1,
            complete: true,
            warning_history_complete: true,
            last_error: None,
        }),
    ];
    let mut driver = DeadlineDriver::new(&port);
    let private = ["private", PRIVATE_RETIREMENT_JOB, "SQLite"];

    let failed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(failed.retirement_error.is_some() && failed.retirement_job.is_none());
    production.assert_redacted("retirement discovery failed: StoreBusy", &private);
    assert_eq!(
        deadline.last_diagnostic().unwrap(),
        "retirement discovery: StoreBusy: SQLite: private retirement discovery failure"
    );

    let failed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert_eq!(
        failed.retirement_job.as_ref().map(|job| job.as_str()),
        Some(PRIVATE_RETIREMENT_JOB)
    );
    production.assert_redacted("retirement cleanup failed: StoreCorrupt", &private);
    assert_eq!(
        deadline.last_diagnostic().unwrap(),
        "retirement job: StoreCorrupt: SQLite: private retirement transaction begin failure"
    );

    let completed = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(completed.retirement_complete);
    assert_eq!(production.scheduler(), ComponentStatus::Ready);
}

/// `last_scheduler_tick_at` was never wired (null on every running daemon).
/// It is the completion time of the deadline worker's last error-free tick,
/// read by the production Health builder from the deadline lane's status.
/// Kills: Health leaving the field None after a tick, recording a skipped
/// (pre-gate) call as a tick, and recording a failed pass.
#[test]
fn production_health_reports_the_last_completed_scheduler_tick() {
    let deadline = Arc::new(WorkerStatus::default());
    let production = Production::new([deadline.clone(), Default::default(), Default::default()]);
    let port = ScriptedDeadlines {
        clock: production.clock.clone(),
        ..Default::default()
    };
    let mut driver = DeadlineDriver::new(&port);
    assert_eq!(
        (production.health)(&production.budget()).last_scheduler_tick_at,
        None
    );
    let outcome = deadline_turn(&port, &mut driver, &deadline).unwrap();
    assert!(outcome.ticked);
    let inputs = (production.health)(&production.budget());
    assert_eq!(inputs.last_scheduler_tick_at, Some(UtcMillis(100)));
    assert_eq!(
        inputs.assemble().last_scheduler_tick_at,
        Some(UtcMillis(100))
    );
    // A call before the next tick is due is a skip, not a tick.
    let skipped = Arc::new(WorkerStatus::default());
    let budget = CallBudget {
        deadline: MonoInstant(port.clock.0.load(Ordering::SeqCst) + 500),
        cancellation: Cancellation::default(),
    };
    let outcome = skipped.drive_deadlines(&mut driver, &budget).unwrap();
    assert!(!outcome.ticked);
    assert_eq!(skipped.last_tick(), None);
    // A failed pass records no tick.
    let failing = Arc::new(WorkerStatus::default());
    let exhausted = CallBudget {
        deadline: MonoInstant(0),
        cancellation: Cancellation::default(),
    };
    assert!(failing.drive_deadlines(&mut driver, &exhausted).is_err());
    assert_eq!(failing.last_tick(), None);
}

fn page_progress(
    next: Option<u64>,
    refused: u8,
) -> crate::identity::reconcile::ReconcilePageProgress {
    crate::identity::reconcile::ReconcilePageProgress {
        next_after_ordinal: next,
        high_water_ordinal: 9,
        transition_count: 1,
        retirements_started: 0,
        transitions_refused: refused,
    }
}

// B5 (ht-rzi.1): the marker is recorded only by a refusal-free full pass.
// Kills: a pass_complete that forgets a refusal from an earlier page (the
// last clean page would then record the marker), or that reports done while a
// next page remains.
#[test]
fn pass_with_refused_transition_leaves_marker_behind() {
    assert_eq!(
        pass_complete(false, &page_progress(Some(4), 0)),
        (false, false)
    );
    assert_eq!(
        pass_complete(false, &page_progress(Some(4), 1)),
        (false, true)
    );
    // The refusal on an earlier page survives a clean final page.
    assert_eq!(pass_complete(true, &page_progress(None, 0)), (true, true));
    assert_eq!(pass_complete(false, &page_progress(None, 2)), (true, true));
    assert_eq!(pass_complete(false, &page_progress(None, 0)), (true, false));
}

// Kills: a worker loop that never calls record_reconciliation_pass, or calls
// it again on every turn instead of once per completed published pass.
#[test]
fn refusal_free_pass_records_marker_once() {
    let observation = Arc::new(WorkerStatus::default());
    let production = Production::new([Default::default(), Default::default(), observation.clone()]);
    let clock = production.clock.clone();
    let host = Arc::new(ObservedHost {
        clock: clock.clone(),
        sequence: AtomicU64::new(1),
        fail: Default::default(),
        snapshots: AtomicU64::new(0),
    });
    let writer = Arc::new(FairWriter::new(32));
    let identity = Arc::new(crate::identity::repair::OrdinaryIdentity::new(
        production.instance.to_string(),
        production.store.clone(),
        host.clone(),
        clock.clone(),
        writer.clone(),
    ));
    let port = Arc::new(FaultyObservationStore {
        inner: ScheduledStore {
            store: production.store.clone(),
            writer,
        },
        fail_admission: Default::default(),
        fail_page: Default::default(),
        pages_ok: AtomicU64::new(0),
        pass_records: AtomicU64::new(0),
        refuse_transitions: Default::default(),
    });
    let cancellation = Cancellation::default();
    let worker = StopWorker(
        cancellation.clone(),
        Some(
            spawn_observation_loop(
                identity,
                port.clone(),
                cancellation,
                observation,
                Arc::new(Default::default()),
            )
            .unwrap(),
        ),
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while port.pass_records.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until, "no pass was recorded");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // No further capture is due (the clock has not advanced): still once.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(port.pass_records.load(Ordering::SeqCst), 1);
    worker.stop();
}

// Kills: a worker loop that records the reconciliation marker after a pass in
// which the store refused a transition (TRUST-POLICY C2: a refused pass leaves
// the marker behind so readers keep treating the instance as not reconciled).
#[test]
fn stale_refused_pass_skips_reconciliation_marker() {
    let observation = Arc::new(WorkerStatus::default());
    let production = Production::new([Default::default(), Default::default(), observation.clone()]);
    let clock = production.clock.clone();
    let host = Arc::new(ObservedHost {
        clock: clock.clone(),
        sequence: AtomicU64::new(1),
        fail: Default::default(),
        snapshots: AtomicU64::new(0),
    });
    let writer = Arc::new(FairWriter::new(32));
    let identity = Arc::new(crate::identity::repair::OrdinaryIdentity::new(
        production.instance.to_string(),
        production.store.clone(),
        host.clone(),
        clock.clone(),
        writer.clone(),
    ));
    let port = Arc::new(FaultyObservationStore {
        inner: ScheduledStore {
            store: production.store.clone(),
            writer,
        },
        fail_admission: Default::default(),
        fail_page: Default::default(),
        pages_ok: AtomicU64::new(0),
        pass_records: AtomicU64::new(0),
        refuse_transitions: std::sync::atomic::AtomicBool::new(true),
    });
    let cancellation = Cancellation::default();
    let worker = StopWorker(
        cancellation.clone(),
        Some(
            spawn_observation_loop(
                identity,
                port.clone(),
                cancellation,
                observation,
                Arc::new(Default::default()),
            )
            .unwrap(),
        ),
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    // The refused pass completed: its page was read and the loop moved on.
    while port.pages_ok.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until, "no page was reconciled");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        port.pass_records.load(Ordering::SeqCst),
        0,
        "a Stale-refused pass must not record the marker"
    );
    // A clean pass (next capture due) records it once.
    port.refuse_transitions.store(false, Ordering::SeqCst);
    clock.0.fetch_add(5_000, Ordering::SeqCst);
    while port.pass_records.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < until,
            "no clean pass was recorded"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(port.pass_records.load(Ordering::SeqCst), 1);
    worker.stop();
}
