use herdr_threads::{
    app::SystemClock,
    client::local::LocalSocketClient,
    daemon::{
        ownership::{OwnerLock, read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    ports::LocalClient,
    protocol::{
        commands::{Command, SeatInspectQuery},
        ids::SeatId,
        pagination::PageRequest,
        results::{CommandResult, ComponentState},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    store::connection::StoreContext,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    process::{Child, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct TestChild(Child);
impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn elected_health_sees_late_retirement_and_keeps_exact_error_on_seat_inspect() {
    let root = std::env::temp_dir().join(format!("herdr-retirement-health-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid();
    drop(owner);
    let setup = StoreContext::new(paths.database_path.clone(), Arc::new(SystemClock::new()));
    let db = setup.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
        [instance.to_string()],
    )
    .unwrap();
    for n in 0..25 {
        let seat = format!("old-retired-seat-{n}");
        let job = format!("old-retirement-job-{n}");
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) VALUES (?1,?2,'retired','native',1,0,1,1)",rusqlite::params![seat,instance.to_string()]).unwrap();
        db.execute("INSERT INTO retirements(id,seat_id,cutover_at,closure_boot,closure_epoch,closure_target,closure_generation,status) VALUES (?1,?2,1,'boot',1,'target',1,'complete')",rusqlite::params![job,seat]).unwrap();
    }
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) VALUES ('late-retired-seat',?1,'retired','native',1,0,1,1)",[instance.to_string()]).unwrap();
    db.execute("INSERT INTO retirements(id,seat_id,cutover_at,closure_boot,closure_epoch,closure_target,closure_generation,last_error) VALUES ('late-retirement-job','late-retired-seat',1,'boot',1,'target',1,'private cleanup failure')",[]).unwrap();
    // Keep the worker from settling this row before Health observes it.
    db.execute_batch("CREATE TRIGGER hold_retirement_health BEFORE UPDATE ON retirements BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    drop(db);

    let child = TestChild(
        std::process::Command::new(env!("CARGO_BIN_EXE_herdr-threads"))
            .args([
                "daemon",
                "run",
                "--state-dir",
                root.join("state").to_str().unwrap(),
                "--host-endpoint",
                root.join("host.sock").to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let descriptor = loop {
        if let Some(instance) = read_existing_namespace(&paths).unwrap()
            && let Ok(descriptor) = read_descriptor(&paths, instance)
        {
            break descriptor;
        }
        assert!(Instant::now() < deadline, "daemon did not publish endpoint");
        std::thread::sleep(Duration::from_millis(10));
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = LocalSocketClient::new(
        descriptor.endpoint,
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 3_000),
        cancellation: Cancellation::default(),
    };
    let CommandResult::Health(health) = client.call(Command::Health, &budget()).unwrap() else {
        panic!("missing Health")
    };
    assert!(
        health
            .limitations
            .iter()
            .any(|detail| detail.contains("retirement cleanup pending")),
        "{:?}",
        health.limitations
    );
    assert!(
        health
            .limitations
            .iter()
            .any(|detail| detail.contains("retirement cleanup degraded")),
        "{:?}",
        health.limitations
    );
    assert!(!format!("{health:?}").contains("private cleanup failure"));
    let CommandResult::SeatInspect(inspect) = client
        .call(
            Command::SeatInspect(SeatInspectQuery {
                seat: SeatId::new("late-retired-seat"),
                page: PageRequest::default(),
            }),
            &budget(),
        )
        .unwrap()
    else {
        panic!("missing seat inspect")
    };
    assert_eq!(
        inspect.retirement.unwrap().last_error.unwrap().as_str(),
        "private cleanup failure"
    );
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    db.execute_batch("DROP TRIGGER hold_retirement_health; UPDATE retirements SET last_error=NULL,status='complete' WHERE id='late-retirement-job';").unwrap();
    let CommandResult::Health(cleared) = client.call(Command::Health, &budget()).unwrap() else {
        panic!("missing Health")
    };
    assert!(
        !cleared
            .limitations
            .iter()
            .any(|detail| detail.contains("retirement cleanup")),
        "{:?}",
        cleared.limitations
    );
    db.execute_batch("DROP INDEX retirements_failed_pending")
        .unwrap();
    let CommandResult::Health(failed_read) = client.call(Command::Health, &budget()).unwrap()
    else {
        panic!("missing Health")
    };
    assert_eq!(failed_read.database.state, ComponentState::Degraded);
    assert_eq!(
        failed_read.database.detail.as_deref(),
        Some("retirement status unavailable")
    );
    assert!(!format!("{failed_read:?}").contains("retirements_failed_pending"));
    drop(child);
    fs::remove_dir_all(root).unwrap();
}
