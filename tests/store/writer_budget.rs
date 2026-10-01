use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const BOOT: &str = "00000000-0000-0000-0000-000000000001";
struct LiveClock(Instant);
impl Clock for LiveClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.elapsed().as_millis() as u64)
    }
}
/// `wake_work` columns in `Fixture::row`'s SELECT order.
type WakeWorkColumns = (
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    String,
    i64,
    i64,
);

struct Fixture {
    store: Arc<SqliteStore>,
    clock: Arc<LiveClock>,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("herdr-writer-budget-{}.db", uuid::Uuid::new_v4()));
        let clock = Arc::new(LiveClock(Instant::now()));
        let context = StoreContext::new(path.clone(), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);").unwrap();
        db.execute("INSERT INTO wake_work(seat_id,binding_generation,reservation_id,reservation_boot,reserved_at_utc,last_reservation_id,last_reservation_boot,last_reserved_at_utc,minimum_delay_ms,effective_delay_ms) VALUES ('s',1,'attempt',?1,10,'attempt',?1,10,30000,30000)", [BOOT]).unwrap();
        drop(db);
        let store = Arc::new(
            SqliteStore::new(
                context,
                "i",
                StoreSettings {
                    daemon_boot: Some(uuid::Uuid::parse_str(BOOT).unwrap()),
                    ..StoreSettings::default()
                },
            )
            .unwrap(),
        );
        Self { store, clock, path }
    }
    fn budget(&self, millis: u64) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + millis),
            cancellation: Cancellation::default(),
        }
    }
    fn row(&self) -> WakeWorkColumns {
        Connection::open(&self.path).unwrap().query_row("SELECT reservation_id,reservation_boot,completed_at_utc,last_outcome,last_reservation_id,last_reserved_at_utc,minimum_delay_ms FROM wake_work WHERE seat_id='s'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap()
    }
    fn assert_reserved(&self) {
        assert_eq!(
            self.row(),
            (
                Some("attempt".into()),
                Some(BOOT.into()),
                None,
                None,
                "attempt".into(),
                10,
                30000
            )
        );
    }
    fn assert_settled(&self) {
        assert_eq!(
            self.row(),
            (
                None,
                None,
                Some(100),
                Some("submitted".into()),
                "attempt".into(),
                10,
                30000
            )
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
    }
}
fn complete(store: &SqliteStore, budget: &CallBudget) -> Result<(), ApiError> {
    StorePort::complete_wake(
        store,
        WakeAttemptId::new("attempt"),
        WakeOutcome::Submitted,
        budget,
    )
}
fn blocked_completion(in_process: bool, cancel: bool) {
    // Break caught: an unconditional writer lock or fixed SQLite busy timeout.
    let fixture = Fixture::new();
    let guard = in_process.then(|| fixture.store.writer.lock().unwrap());
    let external = (!in_process).then(|| {
        let db = Connection::open(&fixture.path).unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        db
    });
    let budget = fixture.budget(if cancel { 5000 } else { 40 });
    let cancellation = budget.cancellation.clone();
    let store = fixture.store.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = complete(&store, &budget);
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    if cancel {
        std::thread::sleep(Duration::from_millis(10));
        cancellation.cancel();
    }
    let while_held = done_rx.recv_timeout(Duration::from_millis(500));
    fixture.assert_reserved();
    drop(guard);
    if let Some(db) = external {
        db.execute_batch("ROLLBACK").unwrap();
    }
    worker.join().unwrap();
    let result = while_held.expect("completion must return while the writer is still held");
    assert_eq!(
        result.unwrap_err().code,
        if cancel {
            ErrorCode::Cancelled
        } else {
            ErrorCode::DeadlineExceeded
        }
    );
    fixture.assert_reserved();
    // A new call on the persistent connection must not retain a cancelled callback.
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
#[test]
fn in_process_writer_deadline_returns_while_lock_stays_held() {
    blocked_completion(true, false);
}
#[test]
fn in_process_writer_cancellation_returns_while_lock_stays_held() {
    blocked_completion(true, true);
}
#[test]
fn external_sqlite_writer_deadline_preserves_exact_reservation() {
    blocked_completion(false, false);
}
#[test]
fn external_sqlite_writer_cancellation_preserves_exact_reservation() {
    blocked_completion(false, true);
}
#[test]
fn external_writer_released_before_deadline_settles_once() {
    let fixture = Fixture::new();
    let db = Connection::open(&fixture.path).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = fixture.store.clone();
    let budget = fixture.budget(5000);
    let worker = std::thread::spawn(move || complete(&store, &budget));
    std::thread::sleep(Duration::from_millis(20));
    fixture.assert_reserved();
    db.execute_batch("ROLLBACK").unwrap();
    worker.join().unwrap().unwrap();
    fixture.assert_settled();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}

#[test]
fn expiry_during_validation_rolls_back_before_publication() {
    // Break caught: validation SQL ignores the live predecision progress budget.
    let fixture = Fixture::new();
    let budget = fixture.budget(20);
    let mut writer = fixture.store.writer.lock().unwrap();
    let result = fixture.store.context.execute_budgeted_decision(&mut writer, &budget,
        |tx| {
            tx.execute("UPDATE wake_work SET reason_bits=2 WHERE seat_id='s'", []).map_err(store_error)?;
            tx.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0)).map_err(store_error)
        },
        |tx, _, _| { tx.execute("UPDATE wake_work SET reservation_id=NULL WHERE seat_id='s'", []).map_err(store_error)?; Ok(()) });
    assert_eq!(result.unwrap_err().code, ErrorCode::DeadlineExceeded);
    assert!(budget.is_exhausted(fixture.clock.as_ref()));
    assert!(writer.is_autocommit());
    assert_eq!(
        writer
            .query_row(
                "SELECT reason_bits FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(writer);
    fixture.assert_reserved();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
#[test]
fn cancellation_at_end_of_validation_rejects_before_decision() {
    // Break caught: no budget check between validated data and publication.
    let fixture = Fixture::new();
    let budget = fixture.budget(5000);
    let mut writer = fixture.store.writer.lock().unwrap();
    let result = fixture.store.context.execute_budgeted_decision(
        &mut writer,
        &budget,
        |_| {
            budget.cancellation.cancel();
            Ok(())
        },
        |tx, _, _| {
            tx.execute(
                "UPDATE wake_work SET reservation_id=NULL WHERE seat_id='s'",
                [],
            )
            .map_err(store_error)?;
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    assert!(writer.is_autocommit());
    drop(writer);
    fixture.assert_reserved();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
struct DecisionExpiryClock {
    expires_on_sample: std::sync::atomic::AtomicBool,
}
impl Clock for DecisionExpiryClock {
    fn utc_now(&self) -> UtcMillis {
        self.expires_on_sample
            .store(true, std::sync::atomic::Ordering::SeqCst);
        UtcMillis(123)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(
            if self
                .expires_on_sample
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                100
            } else {
                0
            },
        )
    }
}
#[test]
fn expiry_in_decision_clock_sample_rejects_before_apply() {
    // Break caught: checking only before sampling accepts an expired decision.
    let fixture = Fixture::new();
    let context = StoreContext::new(
        fixture.path.clone(),
        Arc::new(DecisionExpiryClock {
            expires_on_sample: std::sync::atomic::AtomicBool::new(false),
        }),
    );
    let budget = CallBudget {
        deadline: MonoInstant(100),
        cancellation: Cancellation::default(),
    };
    let mut writer = fixture.store.writer.lock().unwrap();
    let result = context.execute_budgeted_decision(
        &mut writer,
        &budget,
        |_| Ok(()),
        |tx, _, _| {
            tx.execute(
                "UPDATE wake_work SET reservation_id=NULL WHERE seat_id='s'",
                [],
            )
            .map_err(store_error)?;
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::DeadlineExceeded);
    assert!(budget.is_exhausted(context.clock()));
    assert!(writer.is_autocommit());
    drop(writer);
    fixture.assert_reserved();
}
#[test]
fn cancellation_after_accepted_decision_preserves_committed_result() {
    // Break caught: keeping a caller cancellation callback active after acceptance.
    let fixture = Fixture::new();
    let budget = fixture.budget(5000);
    let mut writer = fixture.store.writer.lock().unwrap();
    let result = fixture.store.context.execute_budgeted_decision(&mut writer, &budget,
        |_| Ok(()),
        |tx, _, _| {
            budget.cancellation.cancel();
            tx.execute("UPDATE wake_work SET reason_bits=(WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) SELECT count(*) FROM n) WHERE seat_id='s'", []).map_err(store_error)?;
            Ok(17)
        });
    assert_eq!(result.unwrap(), 17);
    assert!(writer.is_autocommit());
    assert_eq!(
        writer
            .query_row(
                "SELECT reason_bits FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        10000
    );
    drop(writer);
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}

struct ProgressClock {
    checks: std::sync::atomic::AtomicU64,
    cancellation: Option<Cancellation>,
}
impl Clock for ProgressClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        let check = self
            .checks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if check >= 50
            && let Some(cancellation) = &self.cancellation
        {
            cancellation.cancel();
        }
        MonoInstant(check)
    }
}
fn validation_progress(cancel: bool) {
    // Break caught: checking only after validation instead of during SQLite work.
    let fixture = Fixture::new();
    let cancellation = Cancellation::default();
    let clock = Arc::new(ProgressClock {
        checks: std::sync::atomic::AtomicU64::new(0),
        cancellation: cancel.then(|| cancellation.clone()),
    });
    let context = StoreContext::new(fixture.path.clone(), clock.clone());
    let budget = CallBudget {
        deadline: MonoInstant(if cancel { 5000 } else { 50 }),
        cancellation,
    };
    let mut writer = fixture.store.writer.lock().unwrap();
    let result = context.execute_budgeted_decision(&mut writer, &budget,
        |tx| {
            tx.execute("UPDATE wake_work SET reason_bits=2 WHERE seat_id='s'", []).map_err(store_error)?;
            tx.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0)).map_err(store_error)
        },
        |_, _, _| Ok(()));
    assert_eq!(
        result.unwrap_err().code,
        if cancel {
            ErrorCode::Cancelled
        } else {
            ErrorCode::DeadlineExceeded
        }
    );
    let checks = clock.checks.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        (50..100).contains(&checks),
        "validation must interrupt at the progress boundary: {checks}"
    );
    assert!(writer.is_autocommit());
    assert_eq!(
        writer
            .query_row(
                "SELECT reason_bits FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(writer);
    fixture.assert_reserved();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
#[test]
fn validation_progress_deadline_interrupts_sql_before_it_finishes() {
    validation_progress(false);
}
#[test]
fn validation_progress_cancellation_interrupts_sql_before_it_finishes() {
    validation_progress(true);
}
#[test]
fn validation_error_clears_cancelled_callbacks_and_rolls_back() {
    let fixture = Fixture::new();
    let budget = fixture.budget(5000);
    let mut writer = fixture.store.writer.lock().unwrap();
    let result: Result<(), _> = fixture.store.context.execute_budgeted_decision(
        &mut writer,
        &budget,
        |tx| {
            tx.execute("UPDATE wake_work SET reason_bits=2 WHERE seat_id='s'", [])
                .map_err(store_error)?;
            budget.cancellation.cancel();
            Err::<(), _>(api_error(ErrorCode::Conflict, "validation conflict"))
        },
        |_, _, _| Ok(()),
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    assert!(writer.is_autocommit());
    assert_eq!(
        writer
            .query_row(
                "SELECT reason_bits FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(writer);
    fixture.assert_reserved();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
#[test]
fn cancelled_apply_failure_preserves_storage_error_and_rolls_back() {
    let fixture = Fixture::new();
    let budget = fixture.budget(5000);
    let mut writer = fixture.store.writer.lock().unwrap();
    let result: Result<(), _> = fixture.store.context.execute_budgeted_decision(
        &mut writer,
        &budget,
        |_| Ok(()),
        |tx, _, _| {
            budget.cancellation.cancel();
            tx.execute("UPDATE wake_work SET reason_bits=2 WHERE seat_id='s'", [])
                .map_err(store_error)?;
            tx.execute("UPDATE wake_work SET reason_bits=-1 WHERE seat_id='s'", [])
                .map_err(store_error)?;
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    assert!(writer.is_autocommit());
    assert_eq!(
        writer
            .query_row(
                "SELECT reason_bits FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(writer);
    fixture.assert_reserved();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
#[test]
fn poisoned_writer_remains_a_store_corruption_error() {
    let fixture = Fixture::new();
    let store = fixture.store.clone();
    let _ = std::thread::spawn(move || {
        let _writer = store.writer.lock().unwrap();
        panic!("poison writer fixture");
    })
    .join();
    assert_eq!(
        complete(&fixture.store, &fixture.budget(5000))
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
    fixture.assert_reserved();
}

#[test]
fn in_process_writer_released_before_deadline_settles_once() {
    let fixture = Fixture::new();
    let held = fixture.store.writer.lock().unwrap();
    let store = fixture.store.clone();
    let budget = fixture.budget(5000);
    let (started_tx, started_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        complete(&store, &budget)
    });
    started_rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(20));
    fixture.assert_reserved();
    drop(held);
    worker.join().unwrap().unwrap();
    fixture.assert_settled();
    complete(&fixture.store, &fixture.budget(5000)).unwrap();
    fixture.assert_settled();
}
