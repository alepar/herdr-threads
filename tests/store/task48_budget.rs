//! Task48 real SQLite producer admission tests. Trace gates establish ordering.
use crate::{
    ports::*,
    protocol::{
        ids::*,
        results::{ApiError, ErrorCode},
        time::*,
    },
    store::{connection::StoreContext, seats},
};
use rusqlite::{Connection, ffi};
use std::{
    ffi::{CStr, c_void},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct OwnedConnection {
    connection: Connection,
    _directory: Arc<Directory>,
}
struct Fixture {
    connection: Connection,
    context: Arc<StoreContext>,
    clock: Arc<TestClock>,
    directory: Arc<Directory>,
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(2000),
        cancellation: Default::default(),
    }
}
fn fixture() -> Fixture {
    let directory = Arc::new(Directory(
        std::env::temp_dir().join(format!("task48-{}", uuid::Uuid::new_v4())),
    ));
    std::fs::create_dir(&directory.0).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(100)));
    let context = Arc::new(StoreContext::new(
        directory.0.join("store.db"),
        clock.clone(),
    ));
    let connection = context.open_writer().unwrap();
    Fixture {
        connection,
        context,
        clock,
        directory,
    }
}
fn owned_blocker(f: &Fixture) -> OwnedConnection {
    OwnedConnection {
        connection: f.context.open_writer().unwrap(),
        _directory: f.directory.clone(),
    }
}
fn image(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let tables:Vec<String>=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    tables
        .into_iter()
        .map(|name| {
            let sql = format!("SELECT * FROM \"{}\"", name.replace('"', "\"\""));
            let mut stmt = conn.prepare(&sql).unwrap();
            let count = stmt.column_count();
            let mut values: Vec<String> = stmt
                .query_map([], |r| {
                    Ok((0..count)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            values.sort();
            (name, values)
        })
        .collect()
}
struct TraceState {
    marker: &'static str,
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    cancel: Option<Cancellation>,
}
struct TraceGuard {
    handle: *mut ffi::sqlite3,
    _state: Box<TraceState>,
}
impl TraceGuard {
    fn install(
        conn: &Connection,
        marker: &'static str,
        gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
        cancel: Option<Cancellation>,
    ) -> Self {
        let state = Box::new(TraceState {
            marker,
            gate: Mutex::new(gate),
            cancel,
        });
        let handle = unsafe { conn.handle() };
        assert_eq!(
            unsafe {
                ffi::sqlite3_trace_v2(
                    handle,
                    ffi::SQLITE_TRACE_STMT,
                    Some(trace),
                    (&*state as *const TraceState).cast_mut().cast(),
                )
            },
            ffi::SQLITE_OK
        );
        Self {
            handle,
            _state: state,
        }
    }
}
impl Drop for TraceGuard {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_trace_v2(self.handle, 0, None, std::ptr::null_mut());
        }
    }
}
unsafe extern "C" fn trace(
    _: u32,
    data: *mut c_void,
    statement: *mut c_void,
    _: *mut c_void,
) -> i32 {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(data as *const TraceState) };
        let raw = unsafe { ffi::sqlite3_sql(statement.cast()) };
        if raw.is_null() {
            return;
        }
        let sql = unsafe { CStr::from_ptr(raw) }.to_string_lossy();
        if !sql.contains(state.marker) {
            return;
        }
        if let Some(cancel) = &state.cancel {
            cancel.cancel();
        }
        if let Some((started, release)) = state.gate.lock().unwrap().take() {
            let _ = started.send(());
            let _ = release.recv();
        }
    }));
    0
}
#[derive(Clone)]
enum Input {
    Begin,
    Target(HostObservationAdmission, HostObservation),
    Invalidate(HostObservationAdmission),
    BeginStage(SnapshotHeader),
    Publish(SnapshotGenerationId),
    Mark(GuardedInvalidationTransition),
    Reconcile(GuardedSeatTransition),
    Retire(RetirementJobId),
}
impl Input {
    fn run(&self, f: &mut Fixture, b: &CallBudget) -> Result<String, ApiError> {
        let context = &f.context;
        let conn = &mut f.connection;
        match self {
            Self::Begin => {
                seats::begin_host_observation(context, conn, "i", b).map(|r| format!("{r:?}"))
            }
            Self::Target(a, o) => seats::publish_current_target_observation(context, conn, a, o, b)
                .map(|r| format!("{r:?}")),
            Self::Invalidate(a) => seats::invalidate_host_observation(
                context,
                conn,
                a,
                HostInvalidationReason::HostUnavailable,
                b,
            )
            .map(|r| format!("{r:?}")),
            Self::BeginStage(h) => {
                seats::begin_snapshot_stage(context, conn, h.clone(), b).map(|r| format!("{r:?}"))
            }
            Self::Publish(id) => {
                seats::publish_snapshot_stage(context, conn, id, b).map(|r| format!("{r:?}"))
            }
            Self::Mark(t) => seats::mark_unresolved_from_invalidation(context, conn, t.clone(), b)
                .map(|r| format!("{r:?}")),
            Self::Reconcile(t) => {
                seats::apply_reconciliation_transition(context, conn, t.clone(), b)
                    .map(|r| format!("{r:?}"))
            }
            Self::Retire(id) => {
                super::advance_retirement(context, conn, id.clone(), WorkAdmission::Background, b)
                    .map(|r| {
                        assert!(r.processed_this_turn <= 16);
                        format!("{r:?}")
                    })
            }
        }
    }
}
fn header(a: HostObservationAdmission, sequence: u64) -> SnapshotHeader {
    SnapshotHeader {
        instance: "i".into(),
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: sequence,
        incarnation: "incarnation".into(),
        expected_targets: 0,
        admission: a,
    }
}
fn stage(f: &mut Fixture, sequence: u64) -> SnapshotGenerationId {
    let a = seats::begin_host_observation(&f.context, &mut f.connection, "i", &budget()).unwrap();
    let staged = seats::begin_snapshot_stage(
        &f.context,
        &mut f.connection,
        header(a, sequence),
        &budget(),
    )
    .unwrap();
    f.connection
        .execute(
            "UPDATE snapshot_generations SET status='sealed' WHERE id=?1",
            [staged.id.as_str()],
        )
        .unwrap();
    staged.id
}
fn prepare(kind: &str) -> (Fixture, Input) {
    let mut f = fixture();
    if kind == "begin" {
        return (f, Input::Begin);
    }
    let initial = stage(&mut f, 1);
    let publication =
        seats::publish_snapshot_stage(&f.context, &mut f.connection, &initial, &budget()).unwrap();
    f.connection.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,1,0)",[]).unwrap();
    let input = match kind {
        "target" => {
            let a = seats::begin_host_observation(&f.context, &mut f.connection, "i", &budget())
                .unwrap();
            Input::Target(
                a,
                HostObservation {
                    focused: false,
                    target: HostTargetId::new("p"),
                    host_boot: HostBootId::new("b"),
                    epoch: 1,
                    generation: 1,
                    observed_at_utc: UtcMillis(100),
                    observed_at_mono: MonoInstant(100),
                    provenance: ObservationProvenance::FreshCurrentTarget,
                    occupant: None,
                    ui: HostUiState::Idle,
                    terminal: None,
                    occupancy: StructuralOccupancy::Unknown,
                    incarnation: IncarnationEvidence::Unknown,
                    execution: ExecutionEvidence::Unknown,
                    call_id: HostCallId::new("call"),
                    connection_epoch: 1,
                    observation_sequence: 2,
                    started_at_mono: MonoInstant(100),
                    completed_at_mono: MonoInstant(100),
                },
            )
        }
        "invalidate" => Input::Invalidate(
            seats::begin_host_observation(&f.context, &mut f.connection, "i", &budget()).unwrap(),
        ),
        "stage" => {
            let a = seats::begin_host_observation(&f.context, &mut f.connection, "i", &budget())
                .unwrap();
            Input::BeginStage(header(a, 2))
        }
        "publish" => Input::Publish(stage(&mut f, 2)),
        "mark" => {
            let a = seats::begin_host_observation(&f.context, &mut f.connection, "i", &budget())
                .unwrap();
            let fence = seats::invalidate_host_observation(
                &f.context,
                &mut f.connection,
                &a,
                HostInvalidationReason::HostUnavailable,
                &budget(),
            )
            .unwrap()
            .unwrap();
            Input::Mark(GuardedInvalidationTransition {
                fence,
                seat: SeatId::new("s"),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("p")),
                expected_terminal: None,
            })
        }
        "reconcile" => Input::Reconcile(GuardedSeatTransition {
            publication,
            seat: SeatId::new("s"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("p")),
            expected_terminal: None,
            action: ReconciliationAction::MarkUnresolved,
        }),
        "retire" => {
            let tx = f.connection.transaction().unwrap();
            let job = super::begin_retirement_fence(
                &tx,
                UtcMillis(50),
                SeatId::new("s"),
                "i",
                "b",
                1,
                "p",
                1,
            )
            .unwrap();
            tx.commit().unwrap();
            Input::Retire(job.id)
        }
        _ => panic!("unknown producer"),
    };
    (f, input)
}
fn rejects_while_blocked(kind: &str, expires: bool) {
    let (f, input) = prepare(kind);
    let before = image(&f.connection);
    let blocker = owned_blocker(&f);
    blocker.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let b = budget();
    let worker_budget = b.clone();
    let clock = f.clock.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut f = f;
        let trace = TraceGuard::install(
            &f.connection,
            "BEGIN IMMEDIATE",
            Some((started_tx, release_rx)),
            None,
        );
        let result = input.run(&mut f, &worker_budget);
        drop(trace);
        let _ = done_tx.send(());
        (f, input, result)
    });
    let reached_begin = started_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    if expires {
        clock.0.store(2000, Ordering::SeqCst);
    } else {
        b.cancellation.cancel();
    }
    let _ = release_tx.send(());
    let bounded = done_rx.recv_timeout(Duration::from_millis(500)).is_ok();
    blocker.connection.execute_batch("ROLLBACK").unwrap();
    let (mut f, input, result) = worker.join().unwrap();
    assert!(
        reached_begin,
        "{kind} never reached BEGIN after entry admission"
    );
    assert!(
        bounded,
        "{kind} ignored {} while SQLite BEGIN blocked",
        if expires { "deadline" } else { "cancellation" }
    );
    assert_eq!(
        result.unwrap_err().code,
        if expires {
            ErrorCode::DeadlineExceeded
        } else {
            ErrorCode::Cancelled
        }
    );
    assert_eq!(
        image(&f.connection),
        before,
        "{kind} changed durable state before rejection"
    );
    let fresh = CallBudget {
        deadline: MonoInstant(3000),
        cancellation: Default::default(),
    };
    assert!(
        input.run(&mut f, &fresh).is_ok(),
        "{kind} same-connection retry"
    );
    assert_ne!(
        image(&f.connection),
        before,
        "{kind} retry did not produce a durable effect"
    );
}
macro_rules! producer_cases {
    ($cancel:ident,$deadline:ident,$kind:literal) => {
        #[test]
        fn $cancel() {
            rejects_while_blocked($kind, false)
        }
        #[test]
        fn $deadline() {
            rejects_while_blocked($kind, true)
        }
    };
}
producer_cases!(begin_cancel, begin_deadline, "begin");
producer_cases!(target_cancel, target_deadline, "target");
producer_cases!(invalidate_cancel, invalidate_deadline, "invalidate");
producer_cases!(stage_cancel, stage_deadline, "stage");
producer_cases!(publish_cancel, publish_deadline, "publish");
producer_cases!(mark_cancel, mark_deadline, "mark");
producer_cases!(reconcile_cancel, reconcile_deadline, "reconcile");
producer_cases!(retire_cancel, retire_deadline, "retire");

#[test]
fn accepted_publication_cancellation_keeps_commit_and_real_storage_error() {
    for fails in [false, true] {
        let (mut f, input) = prepare("publish");
        let before = image(&f.connection);
        let b = budget();
        if fails {
            f.connection.execute_batch("CREATE TRIGGER reject_publication BEFORE UPDATE OF status ON snapshot_generations WHEN NEW.status='published' BEGIN SELECT RAISE(ABORT,'controlled storage failure'); END").unwrap();
        }
        let trace = TraceGuard::install(
            &f.connection,
            "UPDATE snapshot_generations SET status='published'",
            None,
            Some(b.cancellation.clone()),
        );
        let result = input.run(&mut f, &b);
        drop(trace);
        assert!(b.cancellation.is_cancelled());
        if fails {
            let error = result.unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.detail.contains("controlled storage failure"));
            assert_eq!(image(&f.connection), before);
            assert_eq!(
                f.connection
                    .query_row(
                        "SELECT count(*) FROM snapshot_generations WHERE status='published'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
        } else {
            assert!(result.is_ok());
            assert_ne!(image(&f.connection), before);
            assert_eq!(
                f.connection
                    .query_row(
                        "SELECT count(*) FROM snapshot_generations WHERE status='published'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                2
            );
        }
    }
}
#[test]
fn accepted_retirement_cancellation_keeps_committed_cursor() {
    let (mut f, input) = prepare("retire");
    let b = budget();
    let trace = TraceGuard::install(
        &f.connection,
        "UPDATE retirements SET thread_ordinal",
        None,
        Some(b.cancellation.clone()),
    );
    let result = input.run(&mut f, &b);
    drop(trace);
    assert!(b.cancellation.is_cancelled());
    assert!(result.is_ok());
    assert_eq!(
        f.connection
            .query_row("SELECT status FROM retirements", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "complete"
    );
    assert_eq!(
        f.connection
            .query_row("SELECT cutover_at FROM retirements", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        50
    );
}
#[test]
fn owned_fixture_removes_db_wal_shm_on_normal_and_unwind() {
    let path = {
        let f = fixture();
        let path = f.directory.0.clone();
        f.connection
            .execute_batch(
                "CREATE TABLE cleanup_probe(value); INSERT INTO cleanup_probe VALUES (1)",
            )
            .unwrap();
        assert!(path.join("store.db-wal").exists());
        assert!(path.join("store.db-shm").exists());
        path
    };
    assert!(!path.exists());
    let mut panic_path = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let f = fixture();
        panic_path = Some(f.directory.0.clone());
        panic!("controlled fixture unwind");
    }));
    assert!(result.is_err());
    assert!(!panic_path.unwrap().exists());
}

#[test]
fn accepted_retirement_storage_failure_keeps_real_error_and_rolls_back_cursor() {
    let (mut f, input) = prepare("retire");
    let before = image(&f.connection);
    let b = budget();
    f.connection.execute_batch("CREATE TRIGGER reject_retirement BEFORE UPDATE ON retirements BEGIN SELECT RAISE(ABORT,'controlled retirement storage failure'); END").unwrap();
    let trace = TraceGuard::install(
        &f.connection,
        "UPDATE retirements SET thread_ordinal",
        None,
        Some(b.cancellation.clone()),
    );
    let error = input.run(&mut f, &b).unwrap_err();
    drop(trace);
    assert!(b.cancellation.is_cancelled());
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(
        error
            .detail
            .contains("controlled retirement storage failure")
    );
    assert_eq!(image(&f.connection), before);
}
