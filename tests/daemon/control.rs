use super::*;
use crate::daemon::health::{ComponentStatus, HealthInputs, RetirementHealth};
use crate::daemon::logs::RotatingLogSink;
use crate::daemon::ownership::EndpointDescriptor;
use crate::ports::{LocalClient, LocalService};
use crate::protocol::authority::PeerIdentity;
use crate::protocol::results::{CapabilityState, HealthState};
use crate::protocol::time::Cancellation;
use crate::protocol::{
    commands::{Command, SeatInspectQuery, StopRequest},
    ids::SeatId,
    pagination::PageRequest,
    results::{CommandResult, ErrorCode, StopAccepted},
    time::{CallBudget, MonoInstant},
};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use uuid::Uuid;

// These fixtures build Health through the production elected provider, so the
// retirement read runs under the request budget; only host/harness authority,
// which production cannot verify in-process, is overlaid by the double.
struct RetirementClock;
impl crate::protocol::time::Clock for RetirementClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

struct RetirementDomain(std::sync::Arc<crate::store::SqliteStore>, String);
impl LocalService for RetirementDomain {
    fn handle(
        &self,
        command: Command,
        _peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        crate::ports::StorePort::query(
            self.0.as_ref(),
            &command,
            &crate::ports::ReadContext {
                instance: self.1.clone(),
                output: crate::protocol::output::OutputSpec::default(),
                operation_scope: None,
            },
            budget,
        )
    }
}

fn retirement_fixture() -> (std::path::PathBuf, String, crate::ports::RetirementJob) {
    use crate::ports::{ClosureEvidence, StorePort};
    use crate::protocol::ids::{HostBootId, HostTargetId};
    let path = std::env::temp_dir().join(format!("herdr-retirement-control-{}.db", Uuid::new_v4()));
    let instance = Uuid::new_v4().to_string();
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(RetirementClock),
    );
    let db = context.open_writer().unwrap();
    db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES (?1,0,'host-boot',1,30)", [&instance]).unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('retiring',?1,'resolved','native','pane',1,1,0)", [&instance]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES (?1,'pane','host-boot',1,1,0,'fresh')", [&instance]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('thread',?1,'topic','goal',0,0,25)", [&instance]).unwrap();
    db.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('thread','retiring','joined',0)", []).unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('thread','retiring',1,1)", []).unwrap();
    for n in 1..=24 {
        let id = format!("receipt-{n}");
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,?2,'thread',?3,'ordinary','body',?3,0)", rusqlite::params![id, instance, n]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'thread','retiring','pending',90,0,90)", [&id]).unwrap();
    }
    drop(db);
    let store = crate::store::SqliteStore::new(
        context,
        instance.clone(),
        crate::store::StoreSettings::default(),
    )
    .unwrap();
    let job = StorePort::begin_retirement(
        &store,
        SeatId::new("retiring"),
        ClosureEvidence {
            host_boot: HostBootId::new("host-boot"),
            epoch: 1,
            target: HostTargetId::new("pane"),
            generation: 1,
        },
        &CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap();
    drop(store);
    (path, instance, job)
}

fn retirement_control(
    path: &std::path::Path,
    instance: &str,
    boot: Uuid,
    shutdown: Cancellation,
) -> (
    ControlService<impl Fn(&CallBudget) -> HealthInputs + Send + Sync, RetirementDomain>,
    std::sync::Arc<crate::store::SqliteStore>,
) {
    use crate::ports::StorePort;
    let store = std::sync::Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(
                path.to_path_buf(),
                std::sync::Arc::new(RetirementClock),
            ),
            instance.to_owned(),
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let health_store = store.clone();
    let instance_id = Uuid::parse_str(instance).unwrap();
    // The shared production Health builder: the retirement read receives
    // `health_summary_budget(request)`, never a budget the double mints.
    let production = crate::app::elected_health_provider(
        instance_id,
        boot,
        crate::service::config::ServiceConfig::default().health_settings(),
        std::sync::Arc::new(RetirementClock),
        health_store as std::sync::Arc<dyn StorePort>,
        [Default::default(), Default::default(), Default::default()],
        // No observation lane runs here: no recorded host evidence, under the
        // macOS adapter's witness (Unknown until a capture is observed).
        crate::app::ElectedHostEvidence {
            status: Default::default(),
            incarnation_witness: crate::protocol::results::CapabilityState::Unknown,
            safe_prompt: crate::protocol::results::CapabilityState::Unsupported,
            harnesses: Default::default(),
        },
    );
    let service = ControlService::new(
        StopController::new(instance_id, boot, shutdown),
        move |request: &CallBudget| {
            // Database, schema, scheduler and retirement come from the
            // production builder under the request budget. Only the
            // host/harness authority that production cannot verify here is
            // overlaid, so the fixture can still observe a Healthy state.
            let mut inputs = production(request);
            inputs.host = ComponentStatus::Ready;
            inputs.current_execution = CapabilityState::Supported;
            inputs.coherent_enumeration = CapabilityState::Supported;
            inputs.safe_prompt = CapabilityState::Supported;
            inputs.receipt_registration = CapabilityState::Supported;
            inputs.codex = crate::daemon::health::HarnessStatus::Supported("native".into());
            inputs.claude = crate::daemon::health::HarnessStatus::Supported("native".into());
            inputs
        },
        RetirementDomain(store.clone(), instance.to_owned()),
    );
    (service, store)
}

fn inspect_retirement(
    service: &impl LocalService,
    budget: &CallBudget,
) -> crate::protocol::results::SeatInspection {
    let CommandResult::SeatInspect(inspection) = service
        .handle(
            Command::SeatInspect(SeatInspectQuery {
                seat: SeatId::new("retiring"),
                page: PageRequest::default(),
            }),
            PeerIdentity::from_kernel(501),
            budget,
        )
        .unwrap()
    else {
        panic!("expected exact seat inspection")
    };
    inspection
}

fn receipt_state(path: &std::path::Path, message: &str) -> String {
    let db = rusqlite::Connection::open(path).unwrap();
    db.query_row(
        "SELECT state FROM receipts WHERE message_id=?1 AND seat_id='retiring'",
        [message],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn retirement_health_and_exact_inspect_survive_stop_between_quanta_and_reopen() {
    use crate::ports::{StorePort, WorkAdmission};
    use crate::protocol::results::{CleanupState, ContinuityStatus};
    let (path, instance, job) = retirement_fixture();
    let boot = Uuid::new_v4();
    let shutdown = Cancellation::default();
    let (service, store) = retirement_control(&path, &instance, boot, shutdown.clone());
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let initial = inspect_retirement(&service, &budget);
    assert_eq!(initial.summary.continuity, ContinuityStatus::Retired);
    let initial = initial.retirement.as_ref().unwrap();
    assert!(initial.effective_retired);
    assert_eq!(initial.cleanup_state, CleanupState::Pending);
    assert_eq!(initial.processed_units, 0);
    assert!(!initial.warning_history_complete);
    assert_eq!(receipt_state(&path, "receipt-1"), "pending");
    assert_eq!(receipt_state(&path, "receipt-24"), "pending");
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health
            .limitations
            .iter()
            .any(|detail| detail.contains("retirement cleanup pending"))
    );
    assert_eq!(health.retirement_pending, None);

    let first = StorePort::advance_retirement(
        store.as_ref(),
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap();
    assert!(!first.complete);
    assert!(first.processed_this_turn > 0 && first.processed_this_turn <= 16);
    let running = inspect_retirement(&service, &budget);
    assert_eq!(running.summary.continuity, ContinuityStatus::Retired);
    let running = running.retirement.as_ref().unwrap();
    assert_eq!(running.cleanup_state, CleanupState::Running);
    assert_eq!(running.processed_units, first.processed_total);
    assert!(!running.warning_history_complete);
    assert_eq!(receipt_state(&path, "receipt-24"), "pending");
    let db = rusqlite::Connection::open(&path).unwrap();
    let durable: (String, i64) = db
        .query_row(
            "SELECT status,processed_units FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(durable, ("pending".into(), first.processed_total as i64));
    drop(db);
    let CommandResult::StopAccepted(accepted) = service
        .handle(
            Command::Stop(StopRequest {
                expected_boot: boot.to_string(),
            }),
            PeerIdentity::from_kernel(501),
            &budget,
        )
        .unwrap()
    else {
        panic!("expected stop acknowledgement")
    };
    assert_eq!(accepted.boot_id, boot.to_string());
    assert!(shutdown.is_cancelled());
    drop(service);
    drop(store);

    let (service, store) =
        retirement_control(&path, &instance, Uuid::new_v4(), Cancellation::default());
    assert_eq!(
        inspect_retirement(&service, &budget)
            .retirement
            .unwrap()
            .processed_units,
        first.processed_total
    );
    let mut turns = 1;
    loop {
        let progress = StorePort::advance_retirement(
            store.as_ref(),
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 10);
    }
    assert!(turns > 1);
    let complete = inspect_retirement(&service, &budget);
    assert_eq!(complete.summary.continuity, ContinuityStatus::Retired);
    let complete = complete.retirement.as_ref().unwrap();
    assert_eq!(complete.cleanup_state, CleanupState::Complete);
    assert!(complete.warning_history_complete);
    assert_eq!(complete.retired_at.0, 100);
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert_eq!(health.state, HealthState::Healthy);
    drop(service);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn retirement_control_health_double_honors_request_budget() {
    // The `retirement_control` Health double must read retirement status
    // under the *request* budget (through the production builder's
    // `health_summary_budget`), exactly like the elected daemon.
    //
    // Kills: the double minting its own detached budget (a fresh
    // `Cancellation::default()` and fixed deadline) for the retirement read
    // instead of passing the request budget through. A cancelled or expired
    // request then still reports a successful retirement read, so the double
    // would mask a detached budget in the fixture tests that use it.
    let (path, instance, _job) = retirement_fixture();
    let (service, store) =
        retirement_control(&path, &instance, Uuid::new_v4(), Cancellation::default());
    let health_for = |budget: &CallBudget| {
        let CommandResult::Health(health) = service
            .handle(Command::Health, PeerIdentity::from_kernel(501), budget)
            .unwrap()
        else {
            panic!("expected health")
        };
        health
    };
    let unavailable = |health: &crate::protocol::results::Health| {
        health
            .limitations
            .iter()
            .any(|item| item.contains("retirement status unavailable"))
    };

    // Positive control: a live request budget reads the store.
    let live = health_for(&CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    });
    assert!(
        !unavailable(&live),
        "live budget must read retirement status"
    );
    assert!(
        live.limitations
            .iter()
            .any(|item| item.contains("retirement cleanup pending"))
    );

    // A cancelled request (client disconnect, shutdown) stops the read.
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let health = health_for(&CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: cancelled,
    });
    assert_eq!(health.state, HealthState::Degraded);
    assert!(unavailable(&health), "cancelled request must not be read");
    assert!(
        !health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup pending")),
        "a cancelled read must not report retirement status"
    );

    // An already-expired request deadline (clock is at MonoInstant(1)).
    let health = health_for(&CallBudget {
        deadline: MonoInstant(0),
        cancellation: Cancellation::default(),
    });
    assert_eq!(health.state, HealthState::Degraded);
    assert!(unavailable(&health), "expired request must not be read");

    drop(service);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn retained_retirement_error_is_visible_only_in_exact_inspect() {
    use crate::protocol::results::CleanupState;
    let (path, instance, job) = retirement_fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE retirements SET last_error='injected storage failure' WHERE id=?1",
        [job.id.as_str()],
    )
    .unwrap();
    drop(db);
    let (service, store) =
        retirement_control(&path, &instance, Uuid::new_v4(), Cancellation::default());
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let detail = inspect_retirement(&service, &budget);
    let detail = detail.retirement.as_ref().unwrap();
    assert!(detail.effective_retired);
    assert_eq!(detail.cleanup_state, CleanupState::Failed);
    assert_eq!(
        detail.last_error.as_ref().unwrap().as_str(),
        "injected storage failure"
    );
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup degraded"))
    );
    assert!(
        !serde_json::to_string(&health)
            .unwrap()
            .contains("injected storage failure")
    );
    drop(service);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn production_retirement_failure_degrades_health_and_recovery_clears_it() {
    use crate::ports::{StorePort, WorkAdmission};
    use crate::protocol::results::CleanupState;
    let (path, instance, job) = retirement_fixture();
    // Real SQLite abort inside a cleanup unit; last_error is never seeded.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_retire_receipt BEFORE UPDATE ON receipts WHEN NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'private receipt storage failure'); END;").unwrap();
    drop(db);
    let (service, store) =
        retirement_control(&path, &instance, Uuid::new_v4(), Cancellation::default());
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let failed = StorePort::advance_retirement(
        store.as_ref(),
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap_err();
    assert_eq!(failed.code, ErrorCode::Conflict);
    let detail = inspect_retirement(&service, &budget);
    let detail = detail.retirement.as_ref().unwrap();
    assert!(detail.effective_retired);
    assert_eq!(detail.cleanup_state, CleanupState::Failed);
    assert!(!detail.warning_history_complete);
    assert_eq!(detail.processed_units, 0);
    assert_eq!(
        detail.last_error.as_ref().map(|e| e.as_str()),
        Some("SQLite: private receipt storage failure")
    );
    assert_eq!(receipt_state(&path, "receipt-1"), "pending");
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup degraded"))
    );
    let redacted = serde_json::to_string(&health).unwrap();
    assert!(!redacted.contains("private receipt storage failure"));
    assert!(!redacted.contains("SQLite"));

    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    drop(db);
    let progressed = StorePort::advance_retirement(
        store.as_ref(),
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap();
    assert!(!progressed.complete);
    assert!(progressed.last_error.is_none());
    let running = inspect_retirement(&service, &budget);
    let running = running.retirement.as_ref().unwrap();
    assert_eq!(running.cleanup_state, CleanupState::Running);
    assert!(running.last_error.is_none());
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup pending"))
    );
    assert!(
        !health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup degraded"))
    );
    let mut turns = 0;
    loop {
        let progress = StorePort::advance_retirement(
            store.as_ref(),
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.last_error.is_none());
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 10);
    }
    let complete = inspect_retirement(&service, &budget);
    let complete = complete.retirement.as_ref().unwrap();
    assert_eq!(complete.cleanup_state, CleanupState::Complete);
    assert!(complete.last_error.is_none());
    assert_eq!(receipt_state(&path, "receipt-24"), "recipient_retired");
    let CommandResult::Health(health) = service
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget)
        .unwrap()
    else {
        panic!("expected health")
    };
    assert_eq!(health.state, HealthState::Healthy);
    drop(service);
    drop(store);
    fs::remove_file(path).unwrap();
}

/// Monotonic time the test advances past the driver tick and retry backoff.
struct DriverClock(std::sync::atomic::AtomicU64);
impl crate::protocol::time::Clock for DriverClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}

#[test]
fn production_health_builder_redacts_retirement_failure_and_clears_on_partial_progress() {
    // Production seam: the real `DeadlineDriver` over a real `SqliteStore`
    // feeds the real `WorkerStatus` through `WorkerStatus::drive_deadlines`
    // (the deadline worker's call), and Health is built by the shared
    // production builder `app::elected_health_provider`.
    //
    // Kills (redaction): `WorkerStatus::observe` recording
    // `format!("{:?}: {}", error.code, error.detail)` for
    // `FailureKey::Retirement` (the pre-fix behaviour): the private SQLite
    // text then reaches the scheduler component and serialized Health.
    //
    // Kills (clear-after-partial-progress): the worker clearing
    // `FailureKey::Retirement` only on `retirement_complete` (ignoring
    // `DriveOutcome::retirement_progressed`), and the driver never setting
    // `retirement_progressed`: the scheduler then stays Degraded after a
    // committed, non-completing quantum.
    use crate::ports::StorePort;
    use crate::scheduler::deadlines::DeadlineDriver;
    use crate::service::workers::WorkerStatus;
    use std::sync::{Arc, atomic::Ordering};
    let (path, instance, job) = retirement_fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_retire_receipt BEFORE UPDATE ON receipts WHEN NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'private receipt storage failure'); END;").unwrap();
    drop(db);
    let clock = Arc::new(DriverClock(std::sync::atomic::AtomicU64::new(10_000)));
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(path.clone(), clock.clone()),
            instance.clone(),
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let status = Arc::new(WorkerStatus::default());
    let instance_id = Uuid::parse_str(&instance).unwrap();
    let boot = Uuid::new_v4();
    let health = crate::app::elected_health_provider(
        instance_id,
        boot,
        crate::service::config::ServiceConfig::default().health_settings(),
        clock.clone(),
        store.clone() as Arc<dyn StorePort>,
        [status.clone(), Default::default(), Default::default()],
        // No observation lane runs here: no recorded host evidence, under the
        // macOS adapter's witness (Unknown until a capture is observed).
        crate::app::ElectedHostEvidence {
            status: Default::default(),
            incarnation_witness: crate::protocol::results::CapabilityState::Unknown,
            safe_prompt: crate::protocol::results::CapabilityState::Unsupported,
            harnesses: Default::default(),
        },
    );
    let budget = |clock: &DriverClock, span: u64| CallBudget {
        deadline: MonoInstant(clock.0.load(Ordering::SeqCst) + span),
        cancellation: Cancellation::default(),
    };
    let mut driver = DeadlineDriver::new(store.as_ref());
    let drive = |driver: &mut DeadlineDriver<'_, crate::store::SqliteStore>| {
        // Past the one-second tick and the five-second retirement backoff.
        clock.0.fetch_add(10_000, Ordering::SeqCst);
        status
            .drive_deadlines(driver, &budget(&clock, 500))
            .unwrap()
    };

    let failed = drive(&mut driver);
    assert_eq!(failed.retirement_job.as_ref(), Some(&job.id));
    assert_eq!(
        failed.retirement_error.as_ref().map(|error| &error.code),
        Some(&ErrorCode::Conflict)
    );
    assert!(!failed.retirement_progressed);
    let inputs = health(&budget(&clock, 1_000));
    assert_eq!(
        inputs.scheduler,
        ComponentStatus::Degraded("retirement cleanup failed: Conflict".into())
    );
    assert!(inputs.retirement.degraded);
    let serialized = serde_json::to_string(&inputs.assemble()).unwrap();
    assert!(serialized.contains("scheduler degraded: retirement cleanup failed: Conflict"));
    assert!(!serialized.contains("private receipt storage failure"));
    assert!(!serialized.contains("SQLite"));
    // The exact diagnostic stays durable for seat inspect only.
    let retained: Option<String> = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT last_error FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        retained.as_deref(),
        Some("SQLite: private receipt storage failure")
    );

    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    drop(db);
    // Discovery first finishes its page past the job (no job returned) and
    // then wraps; a turn without progress on the job must not clear it.
    let mut progressed = drive(&mut driver);
    for _ in 0..2 {
        if progressed.retirement_job.is_some() {
            break;
        }
        assert_eq!(
            health(&budget(&clock, 1_000)).scheduler,
            ComponentStatus::Degraded("retirement cleanup failed: Conflict".into())
        );
        progressed = drive(&mut driver);
    }
    assert_eq!(progressed.retirement_job.as_ref(), Some(&job.id));
    assert!(progressed.retirement_error.is_none());
    assert!(!progressed.retirement_complete, "quantum must be partial");
    assert!(progressed.retirement_progressed);
    let inputs = health(&budget(&clock, 1_000));
    assert_eq!(inputs.scheduler, ComponentStatus::Ready);
    assert!(inputs.retirement.pending);
    assert!(!inputs.retirement.degraded);
    let health = inputs.assemble();
    assert!(
        !health
            .limitations
            .iter()
            .any(|item| item.starts_with("scheduler "))
    );
    drop(driver);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn production_health_builder_pins_every_elected_field() {
    // Production seam: Health built by the shared production builder
    // `app::elected_health_provider` (the one `run_elected` uses) over a real
    // `SqliteStore`, with healthy workers.
    //
    // Kills: `ElectedHealth::inputs` over-claiming any field, in particular
    // `inputs.codex = crate::daemon::health::HarnessStatus::Supported("native".into());` in place of
    // `codex::DECLARATION.health_capability()` (wave-1 merge seam review N1)
    // and `inputs.claude = crate::daemon::health::HarnessStatus::Supported("native".into());`. The destructuring
    // is exhaustive, so a new `HealthInputs` field fails to compile here
    // until it is pinned too.
    use crate::ports::StorePort;
    use std::sync::Arc;
    let (path, instance, _job) = retirement_fixture();
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(path.clone(), Arc::new(RetirementClock)),
            instance.clone(),
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let instance_id = Uuid::parse_str(&instance).unwrap();
    let boot = Uuid::new_v4();
    let settings = crate::service::config::ServiceConfig::default().health_settings();
    let health = crate::app::elected_health_provider(
        instance_id,
        boot,
        settings.clone(),
        Arc::new(RetirementClock),
        store.clone() as Arc<dyn StorePort>,
        [Default::default(), Default::default(), Default::default()],
        // No observation lane runs here: no recorded host evidence, under the
        // macOS adapter's witness (Unknown until a capture is observed).
        crate::app::ElectedHostEvidence {
            status: Default::default(),
            incarnation_witness: crate::protocol::results::CapabilityState::Unknown,
            safe_prompt: crate::protocol::results::CapabilityState::Unsupported,
            harnesses: Default::default(),
        },
    );
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let HealthInputs {
        instance: got_instance,
        boot: got_boot,
        database,
        schema,
        host,
        host_version,
        current_execution,
        coherent_enumeration,
        safe_prompt,
        receipt_registration,
        scheduler,
        last_scheduler_tick_at,
        codex,
        claude,
        retirement,
        settings: got_settings,
        last_reconciliation_at,
        binding_evidence,
        unresolved,
    } = health(&budget);
    assert_eq!(got_instance, instance_id);
    assert_eq!(got_boot, boot);
    assert_eq!(database, ComponentStatus::Ready);
    assert_eq!(schema, ComponentStatus::Ready);
    // Host fields come only from observed host evidence (real-host seats):
    // with no capture observed under an Unknown witness they are Unknown,
    // never asserted Ready/Supported, and no reconciliation has completed.
    assert_eq!(host, ComponentStatus::Unknown);
    assert_eq!(host_version, None);
    assert_eq!(current_execution, CapabilityState::Unsupported);
    assert_eq!(coherent_enumeration, CapabilityState::Unknown);
    assert_eq!(last_reconciliation_at, None);
    assert_eq!(safe_prompt, CapabilityState::Unsupported);
    assert_eq!(receipt_registration, CapabilityState::Unsupported);
    assert_eq!(scheduler, ComponentStatus::Ready);
    // No deadline pass ran under these default workers.
    assert_eq!(last_scheduler_tick_at, None);
    // No harness observation was published into this provider's slot:
    // Unknown, never asserted cooperative or supported.
    assert_eq!(codex, crate::daemon::health::HarnessStatus::Unknown);
    assert_eq!(claude, crate::daemon::health::HarnessStatus::Unknown);
    assert_eq!(
        retirement,
        RetirementHealth {
            pending: true,
            degraded: false,
        }
    );
    assert_eq!(got_settings, Some(settings));
    // The production store's writer reports its startup verification; this
    // fixture has nothing to backfill and nothing lacking.
    assert_eq!(
        binding_evidence,
        Some(crate::ports::BindingEvidenceStartup {
            backfilled: 0,
            still_lacking: 0,
        })
    );
    // Unresolved seats are observed (the fixture's one seat is resolved),
    // so Health reports a reliable zero, not None.
    assert_eq!(
        unresolved,
        Some(crate::ports::UnresolvedSeatSummary::default())
    );

    // The serialized Health carries the same harness capabilities.
    let assembled = health(&budget).assemble();
    assert_eq!(
        assembled.harness.codex,
        crate::protocol::results::HarnessState::Unknown
    );
    assert_eq!(
        assembled.harness.claude,
        crate::protocol::results::HarnessState::Unknown
    );
    assert_eq!(assembled.state, HealthState::Degraded);
    drop(health);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn stop_requires_exact_instance_and_boot_before_cancellation() {
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let cancellation = Cancellation::default();
    let control = StopController::new(instance, boot, cancellation.clone());
    assert!(control.request_stop(Uuid::new_v4(), boot).is_err());
    assert!(!cancellation.is_cancelled());
    assert!(control.request_stop(instance, Uuid::new_v4()).is_err());
    assert!(!cancellation.is_cancelled());
    assert!(control.request_stop(instance, boot).is_ok());
    assert!(cancellation.is_cancelled());
}

#[test]
fn oversized_log_fragment_rotates_to_two_bounded_private_files() {
    let directory = std::env::temp_dir().join(format!("herdr-control-test-{}", Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let history = directory.join("history.sqlite3");
    fs::write(&history, b"domain-history-sentinel").unwrap();
    let mut sink = RotatingLogSink::new(directory.clone());
    sink.install().unwrap();
    sink.write(DiagnosticSource::Daemon, &vec![b'A'; 3 * 1024 * 1024 + 17])
        .unwrap();
    sink.drain().unwrap();
    sink.close().unwrap();
    assert_eq!(fs::read(&history).unwrap(), b"domain-history-sentinel");
    let files = [directory.join("daemon.log"), directory.join("daemon.log.1")];
    for file in files {
        let meta = fs::metadata(file).unwrap();
        assert!(meta.len() <= 1024 * 1024, "file exceeds 1 MiB");
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn missing_harness_support_and_retirement_error_degrade_health_without_counting_jobs() {
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let mut inputs = HealthInputs::unknown(instance, boot);
    inputs.database = ComponentStatus::Ready;
    inputs.schema = ComponentStatus::Ready;
    inputs.host = ComponentStatus::Ready;
    inputs.scheduler = ComponentStatus::Ready;
    inputs.current_execution = CapabilityState::Unsupported;
    inputs.codex =
        crate::daemon::health::HarnessStatus::Refused("current execution unavailable".into());
    inputs.retirement = RetirementHealth {
        pending: true,
        degraded: true,
    };
    let health = inputs.assemble();
    assert_eq!(health.software_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(health.boot_id, boot.to_string());
    assert_eq!(health.instance_id, instance.to_string());
    assert_eq!(health.retirement_pending, None);
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.contains("current execution unavailable"))
    );
    assert!(
        health
            .limitations
            .iter()
            .any(|item| item.contains("retirement cleanup degraded"))
    );
}

#[test]
fn health_bounds_large_ascii_unicode_and_escaped_details_before_wire_encoding() {
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.database = ComponentStatus::Degraded("A".repeat(4_000_000));
    inputs.schema = ComponentStatus::Unavailable("界".repeat(1_000_000));
    inputs.host = ComponentStatus::Degraded("\n".repeat(3_000_000));
    inputs.host_version = Some("界".repeat(1_000_000));
    inputs.codex = crate::daemon::health::HarnessStatus::Refused("B".repeat(4_000_000));
    inputs.claude = crate::daemon::health::HarnessStatus::NotInstalled("C".repeat(4_000_000));
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(
        health.database.state,
        crate::protocol::results::ComponentState::Degraded
    );
    assert_eq!(
        health.schema.state,
        crate::protocol::results::ComponentState::Unavailable
    );
    assert_eq!(health.retirement_pending, None);
    assert!(health.limitations.iter().all(|s| s.len() <= 256));
    assert!(health.database.detail.as_ref().unwrap().len() <= 256);
    assert!(health.schema.detail.as_ref().unwrap().len() <= 256);
    assert!(health.host.version.as_ref().unwrap().len() <= 128);
    assert!(health.validate().is_ok());
    let encoded = serde_json::to_vec(&health).unwrap();
    assert!(encoded.len() < 16_384);
    assert_eq!(
        serde_json::from_slice::<crate::protocol::results::Health>(&encoded).unwrap(),
        health
    );
    let response = crate::protocol::wire::WireResponse {
        version: crate::protocol::wire::PROTOCOL_VERSION,
        request_id: "health-bounded".into(),
        instance: health.instance_id.clone(),
        daemon_boot: health.boot_id.clone(),
        result: Ok(CommandResult::Health(health.clone())),
    };
    let frame = crate::protocol::wire::encode_wire_response(&response).unwrap();
    let decoded: crate::protocol::wire::WireResponse = serde_json::from_slice(&frame[4..]).unwrap();
    assert_eq!(decoded, response);
}

#[test]
fn health_without_provider_observations_stays_typed_unknown_and_degraded() {
    let health = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4()).assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(
        health.database.state,
        crate::protocol::results::ComponentState::Unknown
    );
    assert_eq!(
        health.schema.state,
        crate::protocol::results::ComponentState::Unknown
    );
    assert_eq!(
        health.host.reachability,
        crate::protocol::results::ComponentState::Unknown
    );
    assert_eq!(health.retirement_pending, None);
}

/// Health carries the Codex admission line — a schema-matched,
/// live-unverified one as one bounded limitation (it still counts as
/// cooperative) — and the production provider forwards the daemon's boot
/// observation slot.
/// Kills: dropping the codex detail from Health, an unbounded detail line,
/// demoting the schema-matched flag to a note, and a provider that ignores
/// the observed admission.
#[test]
fn health_reports_the_codex_admission_detail_as_a_bounded_limitation() {
    use crate::daemon::health::HarnessStatus;
    use crate::protocol::results::HarnessState;
    let line = "codex 0.159.2: schema-matched, live-unverified: recipe codex-hooks-v1 hook \
                schemas sha256:86858f2456c999030224a92d8dfb535183fe0edf8601690d8941978fadbb066d; \
                binary sha256 16593cc2f422d5f3";
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.codex = HarnessStatus::Cooperative {
        detail: line.into(),
        live_unverified: true,
    };
    let health = inputs.assemble();
    assert_eq!(health.harness.codex, HarnessState::Cooperative);
    assert!(
        health
            .limitations
            .contains(&format!("harness codex: {line}")),
        "{:?}",
        health.limitations
    );
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.codex = HarnessStatus::Cooperative {
        detail: "x".repeat(10_000),
        live_unverified: true,
    };
    let health = inputs.assemble();
    assert!(health.limitations.iter().all(|s| s.len() <= 256));
    assert!(health.validate().is_ok());
    assert!(
        !HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4())
            .assemble()
            .limitations
            .iter()
            .any(|s| s.starts_with("harness codex:"))
    );

    use crate::ports::StorePort;
    use std::sync::Arc;
    let (path, instance, _job) = retirement_fixture();
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(path.clone(), Arc::new(RetirementClock)),
            instance.clone(),
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let slot: Arc<std::sync::Mutex<crate::app::HarnessObservations>> = Default::default();
    let provider = crate::app::elected_health_provider(
        Uuid::parse_str(&instance).unwrap(),
        Uuid::new_v4(),
        crate::service::config::ServiceConfig::default().health_settings(),
        Arc::new(RetirementClock),
        store as Arc<dyn StorePort>,
        [Default::default(), Default::default(), Default::default()],
        crate::app::ElectedHostEvidence {
            status: Default::default(),
            incarnation_witness: crate::protocol::results::CapabilityState::Unknown,
            safe_prompt: crate::protocol::results::CapabilityState::Unsupported,
            harnesses: Arc::clone(&slot),
        },
    );
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    };
    assert_eq!(provider(&budget).codex, HarnessStatus::Unknown);
    let observed = HarnessStatus::Cooperative {
        detail: line.into(),
        live_unverified: true,
    };
    slot.lock().unwrap().codex = observed.clone();
    slot.lock().unwrap().claude = HarnessStatus::NotInstalled("absent".into());
    assert_eq!(provider(&budget).codex, observed);
    assert_eq!(
        provider(&budget).claude,
        HarnessStatus::NotInstalled("absent".into())
    );
    let _ = fs::remove_file(path);
}

struct FakeService;
impl LocalService for FakeService {
    fn handle(
        &self,
        _command: Command,
        _peer: PeerIdentity,
        _budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(ApiError {
            code: ErrorCode::NotFound,
            detail: "exact seat absent".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        })
    }
}

#[test]
fn control_service_routes_stop_and_health_without_native_authority() {
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let shutdown = Cancellation::default();
    let service = ControlService::new(
        StopController::new(instance, boot, shutdown.clone()),
        move |_: &CallBudget| {
            let mut health = HealthInputs::unknown(instance, boot);
            health.database = ComponentStatus::Ready;
            health.schema = ComponentStatus::Ready;
            health.host = ComponentStatus::Ready;
            health.scheduler = ComponentStatus::Ready;
            health.current_execution = CapabilityState::Supported;
            health.coherent_enumeration = CapabilityState::Supported;
            health.safe_prompt = CapabilityState::Supported;
            health.receipt_registration = CapabilityState::Supported;
            health.codex = crate::daemon::health::HarnessStatus::Supported("native".into());
            health.claude = crate::daemon::health::HarnessStatus::Supported("native".into());
            health
        },
        FakeService,
    );
    let budget = CallBudget {
        deadline: MonoInstant(100),
        cancellation: Cancellation::default(),
    };
    let peer = PeerIdentity::from_kernel(501);
    let health = service.handle(Command::Health, peer, &budget).unwrap();
    assert!(matches!(health, CommandResult::Health(value) if value.state == HealthState::Healthy));
    let inspection = service.handle(
        Command::SeatInspect(SeatInspectQuery {
            seat: SeatId::new("exact-seat"),
            page: PageRequest::default(),
        }),
        peer,
        &budget,
    );
    assert_eq!(inspection.unwrap_err().code, ErrorCode::NotFound);
    let wrong = service.handle(
        Command::Stop(StopRequest {
            expected_boot: Uuid::new_v4().to_string(),
        }),
        peer,
        &budget,
    );
    assert_eq!(wrong.unwrap_err().code, ErrorCode::InstanceMismatch);
    assert!(!shutdown.is_cancelled());
    let accepted = service
        .handle(
            Command::Stop(StopRequest {
                expected_boot: boot.to_string(),
            }),
            peer,
            &budget,
        )
        .unwrap();
    assert_eq!(
        accepted,
        CommandResult::StopAccepted(StopAccepted {
            boot_id: boot.to_string()
        })
    );
    assert!(shutdown.is_cancelled());
}

struct FakeClient {
    result: Result<CommandResult, ApiError>,
}
impl LocalClient for FakeClient {
    fn call(&self, command: Command, _budget: &CallBudget) -> Result<CommandResult, ApiError> {
        assert!(
            matches!(command, Command::Stop(StopRequest { expected_boot }) if Uuid::parse_str(&expected_boot).is_ok())
        );
        self.result.clone()
    }
}

#[test]
fn compatible_old_software_stops_and_unresponsive_client_fails_without_pid_action() {
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let descriptor = EndpointDescriptor {
        software_version: "0.0.1".into(),
        protocol_version: crate::protocol::wire::PROTOCOL_VERSION,
        instance_uuid: instance,
        boot_id: boot,
        endpoint: "/does/not/matter".into(),
        pid: std::process::id(),
        socket_device: 1,
        socket_inode: 1,
    };
    let budget = CallBudget {
        deadline: MonoInstant(100),
        cancellation: Cancellation::default(),
    };
    let success = FakeClient {
        result: Ok(CommandResult::StopAccepted(StopAccepted {
            boot_id: boot.to_string(),
        })),
    };
    assert!(request_stop(&success, &descriptor, &budget).is_ok());
    let timeout = FakeClient {
        result: Err(ApiError {
            code: ErrorCode::HostUnavailable,
            detail: "unresponsive".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
    };
    assert_eq!(
        request_stop(&timeout, &descriptor, &budget)
            .unwrap_err()
            .code,
        ErrorCode::HostUnavailable
    );
    let incompatible = EndpointDescriptor {
        protocol_version: crate::protocol::wire::PROTOCOL_VERSION + 1,
        ..descriptor
    };
    assert_eq!(
        request_stop(&success, &incompatible, &budget)
            .unwrap_err()
            .code,
        ErrorCode::UnknownWireVersion
    );
}

#[test]
fn accepted_stop_waits_only_a_bounded_time_for_owner_exit() {
    let start = std::time::Instant::now();
    let result =
        wait_for_stop_with_probe(Duration::from_millis(25), &Cancellation::default(), || {
            Ok(false)
        });
    assert_eq!(result.unwrap_err().code, ErrorCode::DeadlineExceeded);
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(
        wait_for_stop_with_probe(Duration::from_millis(25), &Cancellation::default(), || Ok(
            true
        ))
        .is_ok()
    );
}

struct StopClock;
impl crate::protocol::time::Clock for StopClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

#[test]
fn stop_completion_waits_for_real_owner_lock_after_socket_and_descriptor_disappear() {
    use crate::daemon::ownership::OwnerLock;
    use crate::daemon::paths::{InstancePaths, RuntimeContext};
    let root = std::env::temp_dir().join(format!("herdr-stop-owner-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let paths = InstancePaths::resolve(
        &RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap(),
    )
    .unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let listener = owner.bind_socket().unwrap();
    let descriptor = owner
        .publish_endpoint(
            &listener,
            "old-compatible",
            crate::protocol::wire::PROTOCOL_VERSION,
        )
        .unwrap();
    let client = FakeClient {
        result: Ok(CommandResult::StopAccepted(StopAccepted {
            boot_id: descriptor.boot_id.to_string(),
        })),
    };
    let budget = CallBudget {
        deadline: MonoInstant(500),
        cancellation: Cancellation::default(),
    };
    let worker_paths = paths.clone();
    let worker_descriptor = descriptor.clone();
    let waiter = std::thread::spawn(move || {
        stop_and_wait(
            &client,
            &worker_paths,
            &worker_descriptor,
            &StopClock,
            &budget,
        )
    });
    std::thread::sleep(Duration::from_millis(30));
    fs::remove_file(&descriptor.endpoint).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        !waiter.is_finished(),
        "missing socket cannot prove owner exit"
    );
    fs::remove_file(&paths.descriptor_path).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        !waiter.is_finished(),
        "missing descriptor cannot prove owner exit"
    );
    drop(listener);
    assert!(
        !waiter.is_finished(),
        "owner still retains the final lock lease"
    );
    drop(owner);
    assert!(waiter.join().unwrap().is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_stop_wait_does_not_report_completion() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error =
        wait_for_stop_with_probe(Duration::from_secs(1), &cancellation, || Ok(true)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
}

#[test]
fn expired_stop_budget_cannot_report_completion_from_a_late_probe() {
    let error = wait_for_stop_with_probe(Duration::ZERO, &Cancellation::default(), || Ok(true))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DeadlineExceeded);
}

/// S1 (wave-2 fix1): a binding that still lacks reconfirmation evidence
/// after the startup verification degrades an otherwise healthy daemon and
/// names the repair; a backfill-only report is informational.
/// Kills: `HealthInputs::assemble` ignoring `still_lacking` for the state
/// (every elected daemon is Degraded today for unrelated reasons, so only an
/// otherwise-healthy input can observe it), and dropping the report line.
#[test]
fn startup_binding_evidence_lacking_degrades_otherwise_healthy_health() {
    let healthy = || {
        let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
        inputs.database = ComponentStatus::Ready;
        inputs.schema = ComponentStatus::Ready;
        inputs.host = ComponentStatus::Ready;
        inputs.scheduler = ComponentStatus::Ready;
        inputs.current_execution = CapabilityState::Supported;
        inputs.coherent_enumeration = CapabilityState::Supported;
        inputs.safe_prompt = CapabilityState::Supported;
        inputs.receipt_registration = CapabilityState::Supported;
        inputs.codex = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs.claude = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs
    };
    let report = |backfilled, still_lacking| crate::ports::BindingEvidenceStartup {
        backfilled,
        still_lacking,
    };
    let mut clean = healthy();
    clean.binding_evidence = Some(report(0, 0));
    let clean = clean.assemble();
    assert_eq!(clean.state, HealthState::Healthy);
    assert!(clean.limitations.is_empty(), "{:?}", clean.limitations);

    let mut backfilled = healthy();
    backfilled.binding_evidence = Some(report(3, 0));
    let backfilled = backfilled.assemble();
    assert_eq!(backfilled.state, HealthState::Healthy);
    assert_eq!(
        backfilled.limitations,
        vec!["binding evidence: backfilled 3 at store startup, still lacking 0 now".to_string()]
    );

    let mut lacking = healthy();
    lacking.binding_evidence = Some(report(1, 2));
    let lacking = lacking.assemble();
    assert_eq!(lacking.state, HealthState::Degraded);
    assert_eq!(
        lacking.limitations,
        vec![
            "binding evidence: backfilled 1 at store startup, still lacking 2 now; after a host \
             invalidation those seats stay unresolved until their agent registers again or \
             `seat rebind SEAT --pane PANE --operator`"
                .to_string()
        ]
    );
    // Every count keeps the line inside Health's 256-byte limitation bound.
    let mut maximal = healthy();
    maximal.binding_evidence = Some(report(u64::MAX, u64::MAX));
    let maximal = maximal.assemble();
    assert_eq!(maximal.validate(), Ok(()));
    assert!(
        !maximal.limitations[0].ends_with('…'),
        "{:?}",
        maximal.limitations
    );
}

/// Wave-2 fix2 (b): an unresolved seat is never silent. Health reports the
/// reliable unresolved count, the repair and the oldest unresolved seats in
/// bounded lines, and is Degraded while any seat is unresolved.
/// Kills: `assemble` not copying the count into `unresolved_seats`, not
/// naming the seats, ignoring unresolved seats for the state, and any line
/// or line count that breaks Health's bounds (an invalid Health is dropped
/// by the transport, so the whole Health read would fail).
#[test]
fn unresolved_seats_are_counted_named_and_degrade_otherwise_healthy_health() {
    use crate::ports::{UnresolvedReason, UnresolvedSeatSample, UnresolvedSeatSummary};
    use crate::protocol::ids::{HostTargetId, SeatId};
    let healthy = || {
        let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
        inputs.database = ComponentStatus::Ready;
        inputs.schema = ComponentStatus::Ready;
        inputs.host = ComponentStatus::Ready;
        inputs.scheduler = ComponentStatus::Ready;
        inputs.current_execution = CapabilityState::Supported;
        inputs.coherent_enumeration = CapabilityState::Supported;
        inputs.safe_prompt = CapabilityState::Supported;
        inputs.receipt_registration = CapabilityState::Supported;
        inputs.codex = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs.claude = crate::daemon::health::HarnessStatus::Supported("native".into());
        inputs
    };
    let unknown = healthy().assemble();
    assert_eq!(unknown.unresolved_seats, None);
    let mut none = healthy();
    none.unresolved = Some(UnresolvedSeatSummary::default());
    let none = none.assemble();
    assert_eq!(none.state, HealthState::Healthy);
    assert_eq!(none.unresolved_seats, Some(0));
    assert!(none.limitations.is_empty(), "{:?}", none.limitations);

    let mut some = healthy();
    some.unresolved = Some(UnresolvedSeatSummary {
        count: 4,
        sample: vec![
            UnresolvedSeatSample {
                seat: SeatId::new("seat-u"),
                target: Some(HostTargetId::new("w4:p11")),
                reason: Some(UnresolvedReason::HostInvalidation),
            },
            UnresolvedSeatSample {
                seat: SeatId::new("seat-v"),
                target: None,
                reason: Some(UnresolvedReason::Other),
            },
        ],
    });
    let some = some.assemble();
    assert_eq!(some.state, HealthState::Degraded);
    assert_eq!(some.unresolved_seats, Some(4));
    assert_eq!(
        some.limitations,
        vec![
            "unresolved seats: 4; their lifecycle check-ins are refused until resolved; a \
             host-invalidated seat reconfirms when a coherent snapshot shows its pane again; \
             repair one that stays unresolved: `seat rebind SEAT --pane PANE --operator`"
                .to_string(),
            "unresolved seat seat-u on w4:p11 (host_invalidation)".to_string(),
            "unresolved seat seat-v on no pane (other)".to_string(),
        ]
    );
    assert_eq!(some.validate(), Ok(()));
    // Maximal ids and counts, with every other limitation source present,
    // stay inside Health's bounds (16 lines of at most 256 bytes): the
    // guidance line is never truncated and extra samples are not listed.
    let mut long = healthy();
    long.database = ComponentStatus::Degraded("d".repeat(300));
    long.schema = ComponentStatus::Degraded("s".repeat(300));
    long.host = ComponentStatus::Unavailable("h".repeat(300));
    long.scheduler = ComponentStatus::Degraded("w".repeat(300));
    long.codex = crate::daemon::health::HarnessStatus::Refused("n".repeat(300));
    long.retirement = RetirementHealth {
        pending: true,
        degraded: true,
    };
    long.binding_evidence = Some(crate::ports::BindingEvidenceStartup {
        backfilled: u64::MAX,
        still_lacking: u64::MAX,
    });
    long.unresolved = Some(UnresolvedSeatSummary {
        count: u64::MAX,
        sample: (0..8)
            .map(|n| UnresolvedSeatSample {
                seat: SeatId::new(format!("seat-{n}-{}", "x".repeat(120))),
                target: Some(HostTargetId::new("p".repeat(128))),
                reason: None,
            })
            .collect(),
    });
    let long = long.assemble();
    assert_eq!(long.validate(), Ok(()));
    let guidance = long
        .limitations
        .iter()
        .find(|line| line.starts_with("unresolved seats: "))
        .expect("guidance line");
    assert!(
        guidance.ends_with("`seat rebind SEAT --pane PANE --operator`"),
        "{guidance}"
    );
    assert_eq!(
        long.limitations
            .iter()
            .filter(|line| line.starts_with("unresolved seat seat-"))
            .count(),
        crate::ports::UNRESOLVED_SEAT_SAMPLE
    );
}

/// The designed cooperative mode, as the live install reports it: host
/// reachable with coherent enumeration and the cooperative wake prompt, no
/// native current execution or receipt registration, both harnesses
/// admitted by recipes (Codex schema-matched).
fn cooperative_inputs() -> HealthInputs {
    use crate::daemon::health::HarnessStatus;
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.database = ComponentStatus::Ready;
    inputs.schema = ComponentStatus::Ready;
    inputs.host = ComponentStatus::Ready;
    inputs.scheduler = ComponentStatus::Ready;
    inputs.current_execution = CapabilityState::Unsupported;
    inputs.coherent_enumeration = CapabilityState::Supported;
    inputs.safe_prompt = CapabilityState::Supported;
    inputs.receipt_registration = CapabilityState::Unsupported;
    inputs.claude = HarnessStatus::Cooperative {
        detail: "claude 2.1.286: listed: recipe claude-hooks-2.1.283".into(),
        live_unverified: false,
    };
    inputs.codex = HarnessStatus::Cooperative {
        detail: "codex 0.159.3: schema-matched, live-unverified: recipe codex-hooks-v1".into(),
        live_unverified: true,
    };
    inputs.unresolved = Some(crate::ports::UnresolvedSeatSummary::default());
    inputs
}

/// User report (ht-4is.8.16): a fully healthy cooperative install read
/// `degraded`. The cooperative mode is the designed one: Health is
/// `healthy`, the harnesses are `cooperative` (not `unsupported`), the
/// cooperative receipt and wake facts are notes, and only the
/// schema-matched Codex admission stays a limitation.
/// Kills: requiring native-verified harnesses, native current execution or
/// host receipt registration for `healthy`; reporting cooperative facts as
/// limitations; dropping the schema-matched flag.
#[test]
fn cooperative_mode_is_healthy_with_notes() {
    use crate::daemon::health::{COOPERATIVE_RECEIPT_LINE, COOPERATIVE_WAKE_LINE};
    use crate::protocol::results::HarnessState;
    let health = cooperative_inputs().assemble();
    assert_eq!(health.state, HealthState::Healthy, "{health:?}");
    assert!(health.validate().is_ok());
    assert_eq!(health.harness.claude, HarnessState::Cooperative);
    assert_eq!(health.harness.codex, HarnessState::Cooperative);
    assert_eq!(
        health.limitations,
        vec![
            "harness codex: codex 0.159.3: schema-matched, live-unverified: recipe codex-hooks-v1"
                .to_owned()
        ]
    );
    assert_eq!(
        health.notes,
        vec![
            "harness claude: claude 2.1.286: listed: recipe claude-hooks-2.1.283".to_owned(),
            COOPERATIVE_RECEIPT_LINE.to_owned(),
            COOPERATIVE_WAKE_LINE.to_owned(),
        ]
    );
    assert!(COOPERATIVE_RECEIPT_LINE.len() <= 256 && COOPERATIVE_WAKE_LINE.len() <= 256);
    let json = serde_json::to_value(&health).unwrap();
    assert_eq!(json["harness"]["claude"], "cooperative");
    assert_eq!(json["state"], "healthy");
}

/// A harness absent from the daemon's PATH is a note; real problems still
/// degrade: a refused version, an unobserved harness, an unreachable host,
/// no wake path, missing coherent enumeration, a worker failure and
/// retirement degradation.
#[test]
fn cooperative_health_degrades_only_on_real_problems() {
    use crate::daemon::health::{HarnessStatus, NO_WAKE_LINE};
    use crate::protocol::results::HarnessState;
    let mut inputs = cooperative_inputs();
    inputs.codex = HarnessStatus::NotInstalled("no executable `codex` on the daemon's PATH".into());
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy, "{health:?}");
    assert_eq!(health.harness.codex, HarnessState::Unsupported);
    assert!(health.limitations.is_empty(), "{:?}", health.limitations);
    assert!(health.notes.contains(
        &"harness codex not installed: no executable `codex` on the daemon's PATH".to_owned()
    ));

    let degraded = |change: &dyn Fn(&mut HealthInputs), expect: &str| {
        let mut inputs = cooperative_inputs();
        change(&mut inputs);
        let health = inputs.assemble();
        assert_eq!(health.state, HealthState::Degraded, "{expect}: {health:?}");
        assert!(health.validate().is_ok());
        assert!(
            health.limitations.iter().any(|line| line.contains(expect)),
            "{expect}: {:?}",
            health.limitations
        );
    };
    degraded(
        &|inputs| inputs.claude = HarnessStatus::Refused("claude 2.1.287: no recipe".into()),
        "harness claude unsupported: claude 2.1.287",
    );
    degraded(
        &|inputs| inputs.codex = HarnessStatus::Unknown,
        "harness codex unknown",
    );
    degraded(
        &|inputs| inputs.host = ComponentStatus::Unavailable("socket missing".into()),
        "host unavailable: socket missing",
    );
    degraded(
        &|inputs| inputs.safe_prompt = CapabilityState::Unsupported,
        NO_WAKE_LINE,
    );
    degraded(
        &|inputs| inputs.scheduler = ComponentStatus::Degraded("capture HostUnavailable".into()),
        "scheduler degraded",
    );
    degraded(
        &|inputs| {
            inputs.retirement = RetirementHealth {
                pending: false,
                degraded: true,
            }
        },
        "retirement cleanup degraded",
    );
    let mut inputs = cooperative_inputs();
    inputs.coherent_enumeration = CapabilityState::Unknown;
    assert_eq!(inputs.assemble().state, HealthState::Degraded);
}

/// `notes` is additive on the wire: an older daemon's Health (no `notes`,
/// `unsupported` harnesses) still decodes; `healthy` cannot claim a host
/// with neither native current execution nor the cooperative wake prompt;
/// notes are bounded like limitations.
#[test]
fn health_notes_decode_from_an_older_daemon_and_bound_validation() {
    let mut health = cooperative_inputs().assemble();
    let mut json = serde_json::to_value(&health).unwrap();
    json.as_object_mut().unwrap().remove("notes");
    json["harness"]["claude"] = "unsupported".into();
    let decoded: crate::protocol::results::Health = serde_json::from_value(json).unwrap();
    assert!(decoded.notes.is_empty());
    assert!(health.validate().is_ok());
    health.host.safe_prompt = CapabilityState::Unsupported;
    assert!(health.validate().is_err());
    let mut health = cooperative_inputs().assemble();
    health.notes = vec!["n".repeat(257)];
    assert!(health.validate().is_err());
}

/// The daemon classifies the installed `claude` it observed.
#[test]
fn claude_observation_classifies_cooperative_refused_and_absent() {
    use crate::daemon::health::HarnessStatus;
    use crate::harness::codex::VersionError;
    let unsupported = CapabilityState::Unsupported;
    assert_eq!(
        crate::app::claude_status(Some(Ok("2.1.286".into())), unsupported),
        HarnessStatus::Cooperative {
            detail: "claude 2.1.286: listed: recipe claude-hooks-2.1.283".into(),
            live_unverified: false,
        }
    );
    assert!(matches!(
        crate::app::claude_status(Some(Ok("2.1.286".into())), CapabilityState::Supported),
        HarnessStatus::Supported(_)
    ));
    assert!(matches!(
        crate::app::claude_status(Some(Err(VersionError::Unsupported("2.1.287".into()))), unsupported),
        HarnessStatus::Refused(detail) if detail.starts_with("claude 2.1.287: no recipe admits it")
    ));
    assert!(matches!(
        crate::app::claude_status(Some(Err(VersionError::Unavailable)), unsupported),
        HarnessStatus::Refused(_)
    ));
    assert!(matches!(
        crate::app::claude_status(None, unsupported),
        HarnessStatus::NotInstalled(_)
    ));
}
