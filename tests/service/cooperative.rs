use crate::{
    ports::{LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command, CreateThread},
        ids::*,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::dispatch::DomainService,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
// Drop the SQLite-bearing value before releasing its directory ownership.
struct OwnedFixture<T> {
    value: T,
    directory: Arc<PrivateDirectory>,
}
impl<T> std::ops::Deref for OwnedFixture<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
struct CancelBeforeJoin(crate::protocol::time::Cancellation);
impl Drop for CancelBeforeJoin {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn fixture() -> (OwnedFixture<DomainService>, OwnedFixture<Connection>) {
    fixture_with_clock(Arc::new(FixedClock))
}
fn fixture_with_clock(
    clock: Arc<dyn Clock>,
) -> (OwnedFixture<DomainService>, OwnedFixture<Connection>) {
    let path = std::env::temp_dir().join(format!("service-cooperative-{}", uuid::Uuid::new_v4()));
    eprintln!("OWNED_COOPERATIVE_DIRECTORY {}", path.display());
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = Arc::new(PrivateDirectory(path));
    let context = StoreContext::new(directory.0.join("store.db"), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',0,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'p','inc','coherent_enumeration',1)", []).unwrap();
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    (
        OwnedFixture {
            value: DomainService::new("i".into(), store, clock)
                .with_operator_owner(501)
                .with_cooperative_owner(
                    501,
                    Arc::new(crate::service::fair_writer::FairWriter::new(32)),
                ),
            directory: directory.clone(),
        },
        OwnedFixture {
            value: db,
            directory,
        },
    )
}
fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("plugin_context:n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
    }
}
fn lifecycle(claim: CallerClaim, operation: &str) -> Command {
    Command::CheckIn(CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
    })
}
fn checked(service: &DomainService, command: Command) -> crate::protocol::results::CheckInResult {
    match service
        .handle(command, PeerIdentity::from_kernel(501), &budget())
        .unwrap()
    {
        CommandResult::CheckedIn(result) => result,
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn elected_owner_checks_in_and_uses_returned_context_without_native_evidence() {
    let (service, db) = fixture();
    let original = lifecycle(claim(), "first");
    let result = checked(&service, original.clone());
    assert_eq!(result.context.binding_generation, 1);
    assert_eq!(checked(&service, original), result);
    assert_eq!(
        checked(
            &service,
            Command::CheckIn(CheckIn {
                mode: CheckInMode::Current,
                claim: result.context.clone(),
                operation: OperationId::new("current")
            })
        )
        .context,
        result.context
    );
    assert!(
        service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: "topic".into(),
                    goal: "goal".into(),
                    operation: OperationId::new("thread"),
                    claim: result.context
                }),
                PeerIdentity::from_kernel(501),
                &budget()
            )
            .is_ok()
    );
    let verified: Option<String> = db
        .query_row("SELECT verified_execution FROM observed_targets", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(verified, None);
    assert_eq!(
        db.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn cooperative_routes_reject_wrong_owner_and_declared_child_without_decisions() {
    let (service, db) = fixture();
    assert_eq!(
        service
            .handle(
                lifecycle(claim(), "peer"),
                PeerIdentity::from_kernel(502),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::Unauthorized
    );
    let mut child = claim();
    child.role = CallerRole::Subagent;
    assert_eq!(
        service
            .handle(
                lifecycle(child, "child"),
                PeerIdentity::from_kernel(501),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn explicit_send_accept_and_ack_are_separate_sqlite_decisions() {
    use crate::protocol::commands::{Accept, Ack, Invite, SendMessage};
    let (service, db) = fixture();
    let sender = checked(&service, lifecycle(claim(), "sender")).context;
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient','i','resolved','native','q',0,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','q','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'q','inc','coherent_enumeration',1)", []).unwrap();
    let peer = PeerIdentity::from_kernel(501);
    let created = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap();
    let CommandResult::ThreadCreated(created) = created else {
        panic!("wrong result")
    };
    service
        .handle(
            Command::Invite(Invite {
                thread: created.clone(),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: OperationId::new("invite"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap();
    let sent = service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                thread: created.clone(),
                body: "pending mail".into(),
                invited_recipients: vec![SeatId::new("recipient")],
                deadline_millis: None,
                operation: OperationId::new("send"),
                claim: sender,
                relays_user: false,
                user_intent: None,
            }),
            peer,
            &budget(),
        )
        .unwrap();
    let CommandResult::MessageSent(sent) = sent else {
        panic!("wrong result")
    };
    let mut recipient = claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = HostTargetId::new("q");
    recipient.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let recipient = checked(&service, lifecycle(recipient, "recipient")).context;
    let pending = || {
        let result = service
            .handle(
                Command::PendingReceipts(crate::protocol::commands::PendingReceiptsQuery {
                    seat: Some(SeatId::new("recipient")),
                    thread: None,
                    page: Default::default(),
                }),
                peer,
                &budget(),
            )
            .unwrap();
        let CommandResult::PendingReceipts(page) = result else {
            panic!("wrong query result")
        };
        page.items.len()
    };
    assert_eq!(pending(), 1);
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='recipient'",
            [created.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invited"
    );
    service
        .handle(
            Command::Accept(Accept {
                thread: created,
                operation: OperationId::new("accept"),
                claim: recipient.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap();
    assert_eq!(pending(), 1);
    service
        .handle(
            Command::Ack(Ack {
                messages: vec![sent.clone()],
                operation: OperationId::new("ack"),
                claim: recipient,
            }),
            peer,
            &budget(),
        )
        .unwrap();
    assert_eq!(pending(), 0);
    assert_eq!(
        db.query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='recipient'",
            [sent.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "acked"
    );
}

#[test]
fn changed_context_is_fenced_and_historical_replay_cannot_restore_predecessor() {
    use crate::protocol::results::CheckInContextDisposition;
    let (service, db) = fixture();
    let original = lifecycle(claim(), "initial");
    let initial = checked(&service, original.clone());
    for (index, wrong) in [
        {
            let mut c = initial.context.clone();
            c.instance = "other".into();
            c
        },
        {
            let mut c = initial.context.clone();
            c.seat = SeatId::new("other");
            c
        },
        {
            let mut c = initial.context.clone();
            c.target = HostTargetId::new("other");
            c
        },
        {
            let mut c = initial.context.clone();
            c.binding_generation += 1;
            c
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            service
                .handle(
                    Command::CheckIn(CheckIn {
                        mode: CheckInMode::Current,
                        claim: wrong,
                        operation: OperationId::new(format!("wrong-{index}"))
                    }),
                    PeerIdentity::from_kernel(501),
                    &budget()
                )
                .unwrap_err()
                .code,
            ErrorCode::CallerUnverified
        );
    }
    let mut successor = initial.context.clone();
    successor.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let successor = checked(&service, lifecycle(successor, "successor"));
    let replay = checked(&service, original);
    assert_eq!(replay.context, initial.context);
    assert_eq!(
        replay.context_disposition,
        CheckInContextDisposition::Historical
    );
    assert_eq!(successor.context.binding_generation, 2);
    assert_eq!(
        db.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let mut mismatch = claim();
    mismatch.native_session = NativeSessionId::new("plugin_context:changed");
    assert_eq!(
        service
            .handle(
                lifecycle(mismatch, "initial"),
                PeerIdentity::from_kernel(501),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
}

#[test]
fn foreground_wait_cancellation_and_expiry_leave_no_checkin_decision() {
    let (service, db) = fixture();
    let writer = service.cooperative_runtime.as_ref().unwrap().1.clone();
    let held = writer
        .enter_background(&budget(), service.clock.as_ref())
        .unwrap();
    let service = Arc::new(service);
    let cancellation = crate::protocol::time::Cancellation::default();
    let waiting = CallBudget {
        cancellation: cancellation.clone(),
        ..budget()
    };
    let worker_service = service.clone();
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            worker_service.handle(
                lifecycle(claim(), "cancel"),
                PeerIdentity::from_kernel(501),
                &waiting,
            )
        });
        let _cancel_before_join = CancelBeforeJoin(cancellation.clone());
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while writer.waiting().0 == 0 && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        assert_eq!(writer.waiting().0, 1);
        cancellation.cancel();
        assert_eq!(
            worker.join().unwrap().unwrap_err().code,
            ErrorCode::Cancelled
        );
    });
    assert_eq!(writer.waiting().0, 0);
    drop(held);
    assert_eq!(
        service
            .handle(
                lifecycle(claim(), "expired"),
                PeerIdentity::from_kernel(501),
                &CallBudget {
                    deadline: MonoInstant(100),
                    cancellation: Default::default()
                }
            )
            .unwrap_err()
            .code,
        ErrorCode::DeadlineExceeded
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

struct PreparationExpiryClock {
    path: std::sync::OnceLock<OwnedFixture<std::path::PathBuf>>,
    expire: std::sync::atomic::AtomicBool,
}
impl Clock for PreparationExpiryClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        if self.expire.load(std::sync::atomic::Ordering::SeqCst)
            && let Some(path) = self.path.get()
        {
            let db = Connection::open(&**path).unwrap();
            let committed: i64 = db
                .query_row("SELECT count(*) FROM prepared_recipients", [], |r| r.get(0))
                .unwrap();
            if committed > 0 {
                return MonoInstant(1000);
            }
        }
        MonoInstant(100)
    }
}
#[test]
fn partial_send_preparation_survives_budget_expiry_and_resumes_exact_intent() {
    use crate::protocol::commands::SendMessage;
    let clock = Arc::new(PreparationExpiryClock {
        path: Default::default(),
        expire: Default::default(),
    });
    let (service, db) = fixture_with_clock(clock.clone());
    clock
        .path
        .set(OwnedFixture {
            value: std::path::PathBuf::from(db.path().unwrap()),
            directory: db.directory.clone(),
        })
        .unwrap_or_else(|_| panic!("preparation clock path already set"));
    let sender = checked(&service, lifecycle(claim(), "initial")).context;
    let peer = PeerIdentity::from_kernel(501);
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    for index in 0..40 {
        let seat = format!("recipient-{index}");
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','resolved','native',0,0)",[seat.as_str()]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'joined')",
            rusqlite::params![thread.as_str(), seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)",rusqlite::params![thread.as_str(),seat]).unwrap();
    }
    let send = Command::SendMessage(SendMessage {
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread,
        body: "bounded preparation".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("send"),
        claim: sender,
        relays_user: false,
        user_intent: None,
    });
    clock
        .expire
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        service
            .handle(send.clone(), peer, &budget())
            .unwrap_err()
            .code,
        ErrorCode::DeadlineExceeded
    );
    let staged: i64 = db
        .query_row("SELECT count(*) FROM prepared_recipients", [], |r| r.get(0))
        .unwrap();
    assert!(staged > 0 && staged < 40);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    clock
        .expire
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let result = service.handle(send.clone(), peer, &budget()).unwrap();
    assert!(matches!(result, CommandResult::MessageSent(_)));
    assert_eq!(service.handle(send, peer, &budget()).unwrap(), result);
    assert_eq!(
        db.query_row("SELECT count(*) FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn stale_sender_cannot_publish_a_prepared_message() {
    use crate::{
        ports::{DurableWorkAdmission, SendPreparationProgress},
        protocol::commands::SendMessage,
    };
    let (service, db) = fixture();
    let sender = checked(&service, lifecycle(claim(), "first")).context;
    let peer = PeerIdentity::from_kernel(501);
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    let send = SendMessage {
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread,
        body: "stale sender".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("send"),
        claim: sender,
        relays_user: false,
        user_intent: None,
    };
    loop {
        match service
            .store
            .prepare_send_step(&send, DurableWorkAdmission::new(16).unwrap(), &budget())
            .unwrap()
        {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {}
            SendPreparationProgress::Committed(_) => panic!("unexpected committed send"),
        }
    }
    // Retain the prepared snapshot while simulating a changed durable occupant.
    db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-000000000002' WHERE ended_at IS NULL",[]).unwrap();
    assert_eq!(
        service
            .handle(Command::SendMessage(send), peer, &budget())
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn fixture_removes_database_after_both_sqlite_owners_drop() {
    let (service, db) = fixture();
    let path = std::path::PathBuf::from(db.path().unwrap());
    assert!(path.is_file());
    assert_eq!(
        db.query_row("SELECT count(*) FROM seats", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(service);
    drop(db);
    assert!(
        !path.exists(),
        "fixture database remained at {}",
        path.display()
    );
}

#[test]
fn fixture_retains_wal_and_durable_rows_across_both_drop_orders_and_reopen() {
    use std::os::unix::fs::PermissionsExt;
    for service_first in [true, false] {
        let (service, db) = fixture();
        let path = std::path::PathBuf::from(db.path().unwrap());
        let directory = path.parent().unwrap().to_path_buf();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let sidecars = [
            path.clone(),
            path.with_file_name("store.db-wal"),
            path.with_file_name("store.db-shm"),
        ];
        checked(&service, lifecycle(claim(), "durable"));
        let reopened = OwnedFixture {
            value: Connection::open(&path).unwrap(),
            directory: db.directory.clone(),
        };
        reopened.execute_batch("BEGIN").unwrap();
        assert_eq!(
            reopened
                .query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(sidecars.iter().all(|p| p.is_file()));
        if service_first {
            drop(service);
            assert_eq!(
                db.query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            drop(db);
        } else {
            drop(db);
            assert_eq!(
                checked(&service, lifecycle(claim(), "durable"))
                    .context
                    .binding_generation,
                1
            );
            drop(service);
        }
        assert!(sidecars.iter().all(|p| p.is_file()));
        assert_eq!(
            reopened
                .query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        reopened.execute_batch("ROLLBACK").unwrap();
        drop(reopened);
        assert!(!directory.exists());
    }
}

#[test]
fn fixture_keeps_preparation_clock_database_alive_until_callback_owner_drops() {
    let clock = Arc::new(PreparationExpiryClock {
        path: Default::default(),
        expire: Default::default(),
    });
    let (service, db) = fixture_with_clock(clock.clone());
    let path = std::path::PathBuf::from(db.path().unwrap());
    let directory = path.parent().unwrap().to_path_buf();
    clock
        .path
        .set(OwnedFixture {
            value: path.clone(),
            directory: db.directory.clone(),
        })
        .unwrap_or_else(|_| panic!("preparation clock path already set"));
    drop(service);
    drop(db);
    assert!(path.is_file());
    clock
        .expire
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(clock.monotonic_now(), MonoInstant(100));
    drop(clock);
    assert!(!directory.exists());
}

#[test]
fn fixture_unwind_cancels_and_physically_joins_queued_worker_before_cleanup() {
    let (service, db) = fixture();
    let directory = std::path::PathBuf::from(db.path().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let writer = service.cooperative_runtime.as_ref().unwrap().1.clone();
    let held = writer
        .enter_background(&budget(), service.clock.as_ref())
        .unwrap();
    let service = Arc::new(service);
    let cancellation = crate::protocol::time::Cancellation::default();
    let waiting = CallBudget {
        cancellation: cancellation.clone(),
        ..budget()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            let worker_service = service.clone();
            scope.spawn(move || {
                let result = worker_service.handle(
                    lifecycle(claim(), "unwind"),
                    PeerIdentity::from_kernel(501),
                    &waiting,
                );
                tx.send(result.unwrap_err().code).unwrap();
            });
            let _cancel_before_join = CancelBeforeJoin(cancellation.clone());
            let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while writer.waiting().0 == 0 && std::time::Instant::now() < until {
                std::thread::yield_now();
            }
            assert_eq!(writer.waiting().0, 1);
            panic!("intended queued fixture unwind");
        });
    }));
    assert_eq!(
        *result.unwrap_err().downcast::<&str>().unwrap(),
        "intended queued fixture unwind"
    );
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap(),
        ErrorCode::Cancelled
    );
    assert_eq!(writer.waiting().0, 0);
    assert!(directory.exists());
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(held);
    drop(service);
    drop(db);
    assert!(!directory.exists());
}

#[test]
fn fixture_worker_keeps_sqlite_alive_after_callers_drop_and_cleans_on_panic() {
    struct ReleaseBeforeJoin(Option<std::sync::mpsc::SyncSender<()>>);
    impl Drop for ReleaseBeforeJoin {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
    let (service, db) = fixture();
    let directory = std::path::PathBuf::from(db.path().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    let path = directory.join("store.db");
    let (release, receive) = std::sync::mpsc::sync_channel(1);
    let (done, completed) = std::sync::mpsc::channel();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        std::thread::scope(|scope| {
            let worker_path = path.clone();
            scope.spawn(move || {
                receive
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
                assert_eq!(
                    checked(&service, lifecycle(claim(), "worker"))
                        .context
                        .binding_generation,
                    1
                );
                let reopened = OwnedFixture {
                    value: Connection::open(worker_path).unwrap(),
                    directory: service.directory.clone(),
                };
                drop(service);
                assert_eq!(
                    reopened
                        .query_row("SELECT count(*) FROM operations", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
                drop(reopened);
                done.send(()).unwrap();
            });
            let _release_before_join = ReleaseBeforeJoin(Some(release));
            drop(db);
            assert!(path.is_file());
            assert!(path.with_file_name("store.db-wal").is_file());
            assert!(path.with_file_name("store.db-shm").is_file());
            panic!("intended held worker fixture unwind");
        });
    }));
    assert_eq!(
        *result.unwrap_err().downcast::<&str>().unwrap(),
        "intended held worker fixture unwind"
    );
    completed
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(!directory.exists());
}

/// Publish one complete coherent host snapshot for `targets` at `epoch`
/// through the real store publication path (the reconfirmation a host
/// recovery performs after an outage).
fn publish_snapshot(context: &StoreContext, epoch: u64, sequence: u64, targets: &[&str]) {
    publish_snapshot_at(context, epoch, sequence, targets, false);
}

/// As `publish_snapshot`, returning the published snapshot. `production`
/// reports what the production adapter does (structural generation 1, which a
/// carry requires, and unknown occupancy).
fn publish_snapshot_at(
    context: &StoreContext,
    epoch: u64,
    sequence: u64,
    targets: &[&str],
    production: bool,
) -> crate::ports::PublishedSnapshot {
    use crate::ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
        HostObservation, HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance,
        SnapshotHeader, StructuralOccupancy,
    };
    use crate::store::seats;
    let mut conn = context.open_writer().unwrap();
    let admission = seats::begin_host_observation(context, &mut conn, "i", &budget()).unwrap();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: targets
            .iter()
            .map(|target| HostObservation {
                focused: false,
                target: HostTargetId::new(*target),
                host_boot: HostBootId::new("b"),
                epoch,
                generation: u64::from(production),
                observed_at_utc: UtcMillis(100),
                observed_at_mono: MonoInstant(100),
                provenance: ObservationProvenance::CoherentEnumeration,
                occupant: None,
                ui: HostUiState::Idle,
                terminal: Some(TerminalId::new(format!("term-{target}"))),
                occupancy: if production {
                    StructuralOccupancy::Unknown
                } else {
                    StructuralOccupancy::Occupied
                },
                incarnation: IncarnationEvidence::Verified {
                    identity: "inc".into(),
                    evidence_kind: EvidenceKind::CoherentEnumeration,
                },
                execution: ExecutionEvidence::Unknown,
                call_id: HostCallId::new(format!("capture-{target}-{sequence}")),
                connection_epoch: 1,
                observation_sequence: sequence,
                started_at_mono: MonoInstant(100),
                completed_at_mono: MonoInstant(100),
            })
            .collect(),
    };
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = seats::begin_snapshot_stage(context, &mut conn, header, &budget()).unwrap();
    seats::stage_snapshot_targets(
        context,
        &mut conn,
        &stage.id,
        0,
        &snapshot.targets,
        DurableWorkAdmission::new(16).unwrap(),
        &budget(),
    )
    .unwrap();
    seats::seal_snapshot_stage(context, &mut conn, &stage.id, &budget()).unwrap();
    seats::publish_snapshot_stage(context, &mut conn, &stage.id, &budget()).unwrap()
}

/// ht-4is.11.10 (host-recovery D1, R01+R04 shape). Two registered seats share
/// a joined thread; a host outage invalidates the publication and the
/// reconfirming snapshot advances the host epoch (1 -> 3) while the per-target
/// `observed_targets` rows keep epoch 1. After fresh check-ins every send, plain
/// or with required ACK, must succeed, and the recipient receipt must start its
/// timer. Kills: recipient staging judging availability from the raw
/// old-epoch `observed_targets` row while `ensure_unavailability_episode` uses
/// the effective projection ("seat is not effectively unavailable" conflict),
/// and an unstarted receipt timer for a reconfirmed available recipient.
#[test]
fn sends_succeed_after_host_outage_epoch_advance_and_reconfirmation() {
    use crate::protocol::commands::{Accept, Invite, SendMessage};
    let (service, db) = fixture();
    let context = StoreContext::new(db.directory.0.join("store.db"), Arc::new(FixedClock));
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient','i','resolved','native','q',0,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','q','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'q','inc','coherent_enumeration',1)", []).unwrap();
    // R01: both seats register; the recipient joins the sender's thread.
    let peer = PeerIdentity::from_kernel(501);
    let sender = checked(&service, lifecycle(claim(), "sender")).context;
    let mut recipient = claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = HostTargetId::new("q");
    recipient.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let recipient = checked(&service, lifecycle(recipient, "recipient")).context;
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    service
        .handle(
            Command::Invite(Invite {
                thread: thread.clone(),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: OperationId::new("invite"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap();
    service
        .handle(
            Command::Accept(Accept {
                thread: thread.clone(),
                operation: OperationId::new("accept"),
                claim: recipient.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap();

    // R04: the host becomes unavailable, then a coherent reconfirmation
    // publishes both panes at a later host epoch.
    {
        use crate::ports::HostInvalidationReason;
        use crate::store::seats;
        let mut conn = context.open_writer().unwrap();
        let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget()).unwrap();
        seats::invalidate_host_observation(
            &context,
            &mut conn,
            &admission,
            HostInvalidationReason::HostUnavailable,
            &budget(),
        )
        .unwrap()
        .unwrap();
    }
    publish_snapshot(&context, 3, 7, &["p", "q"]);
    let epochs = || -> (i64, Vec<i64>) {
        let host: i64 = db
            .query_row(
                "SELECT host_epoch FROM host_instances WHERE id='i'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut stmt = db
            .prepare("SELECT epoch FROM observed_targets ORDER BY target_id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (host, rows)
    };
    // The exact durable shape from the host-recovery diagnostic.
    assert_eq!(epochs(), (3, vec![1, 1]));

    // Fresh check-ins after reconfirmation register both seats at epoch 3.
    let current = |claim: &CallerClaim, operation: &str| {
        checked(
            &service,
            Command::CheckIn(CheckIn {
                mode: CheckInMode::Current,
                claim: claim.clone(),
                operation: OperationId::new(operation),
            }),
        )
        .context
    };
    let sender = current(&sender, "sender-after-outage");
    let recipient = current(&recipient, "recipient-after-outage");
    let open_epochs: Vec<i64> = {
        let mut stmt = db
            .prepare("SELECT host_epoch FROM occupant_bindings WHERE ended_at IS NULL AND registered_at IS NOT NULL ORDER BY seat_id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(open_epochs, vec![3, 3]);

    let send = |claim: &CallerClaim, required: Vec<SeatId>, operation: &str| match service.handle(
        Command::SendMessage(SendMessage {
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(),
            body: format!("mail {operation}"),
            invited_recipients: required,
            deadline_millis: None,
            operation: OperationId::new(operation),
            claim: claim.clone(),
            relays_user: false,
            user_intent: None,
        }),
        peer,
        &budget(),
    ) {
        Ok(CommandResult::MessageSent(message)) => message,
        other => panic!("send {operation} after outage failed: {other:?}"),
    };
    let required = send(
        &sender,
        vec![SeatId::new("recipient")],
        "a-to-b-require-ack",
    );
    let plain = send(&sender, vec![], "a-plain");
    let reverse = send(&recipient, vec![SeatId::new("s")], "b-to-a-require-ack");

    // Every receipt to a reconfirmed, available seat starts its timer and no
    // unavailability warning or episode was opened for it.
    for (message, seat) in [
        (&required, "recipient"),
        (&plain, "recipient"),
        (&reverse, "s"),
    ] {
        let eligible: bool = db
            .query_row(
                "SELECT pr.eligible_at_snapshot FROM prepared_recipients pr JOIN send_manifests m ON m.preparation_id=pr.preparation_id WHERE m.message_id=?1 AND pr.seat_id=?2",
                rusqlite::params![message.as_str(), seat],
                |r| r.get(0),
            )
            .unwrap();
        assert!(eligible, "{message:?} -> {seat} staged unavailable");
    }
    let page = |seat: &str| {
        let CommandResult::PendingReceipts(page) = service
            .handle(
                Command::PendingReceipts(crate::protocol::commands::PendingReceiptsQuery {
                    seat: Some(SeatId::new(seat)),
                    thread: None,
                    page: Default::default(),
                }),
                peer,
                &budget(),
            )
            .unwrap()
        else {
            panic!("wrong query result")
        };
        page.items
    };
    let pending = page("recipient");
    assert!(pending.iter().any(|item| item.message == required));
    for item in pending.iter().chain(page("s").iter()) {
        assert!(
            item.available_at.is_some(),
            "receipt timer not started: {item:?}"
        );
    }
    let open: i64 = db
        .query_row(
            "SELECT count(*) FROM seats WHERE unavailability_open=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(open, 0);
}

/// C4 pre-reconciliation window (ht-rzi.19). A recipient registered at host
/// epoch 1 whose Herdr then restarts (epoch 2, same boot and incarnation) is
/// structurally continuous: a send before the first reconciliation pass is
/// staged as not yet available but writes no `recipient_unavailable` warning,
/// and once the carry-forward lands the receipt's timer starts. Kills: the
/// send warning about a seat the pending pass will carry; a carry that writes
/// no availability anchor, leaving the already-staged receipt without a timer.
#[test]
fn send_before_first_pass_to_structurally_continuous_seat_has_no_warning() {
    use crate::protocol::commands::{Accept, Invite, SendMessage};
    let (service, db) = fixture();
    let context = StoreContext::new(db.directory.0.join("store.db"), Arc::new(FixedClock));
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient','i','resolved','native','q',0,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','q','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'q','inc','coherent_enumeration',1)", []).unwrap();
    // Allocation records the seats' structural proof (terminal and
    // incarnation); the raw fixture rows carry none, so record it here.
    db.execute("UPDATE seats SET structural_terminal_id='term-'||target_id,structural_incarnation='inc',structural_incarnation_kind='coherent_enumeration',structural_host_boot='b',structural_host_epoch=1,structural_connection_epoch=1,structural_observation_sequence=1,target_generation=1 WHERE id IN ('s','recipient')", []).unwrap();
    db.execute("UPDATE observed_targets SET generation=1", [])
        .unwrap();
    let peer = PeerIdentity::from_kernel(501);
    let sender = checked(&service, lifecycle(claim(), "sender")).context;
    let mut recipient = claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = HostTargetId::new("q");
    recipient.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let recipient = checked(&service, lifecycle(recipient, "recipient")).context;
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    for (op, command) in [
        (
            "invite",
            Command::Invite(Invite {
                thread: thread.clone(),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: OperationId::new("invite"),
                claim: sender.clone(),
            }),
        ),
        (
            "accept",
            Command::Accept(Accept {
                thread: thread.clone(),
                operation: OperationId::new("accept"),
                claim: recipient.clone(),
            }),
        ),
    ] {
        service
            .handle(command, peer, &budget())
            .unwrap_or_else(|e| panic!("{op}: {e:?}"));
    }
    // Daemon restart: a newer host epoch of the same boot and incarnation is
    // published; the reconciliation pass has not run, so the marker lags.
    let publication = publish_snapshot_at(&context, 2, 7, &["p", "q"], true);
    let lagging: bool = db
        .query_row(
            "SELECT reconciled_boot IS NOT recovery_boot OR reconciled_epoch IS NOT recovery_epoch FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        lagging,
        "the published epoch advances recovery past the marker"
    );
    let binding_epoch: i64 = db
        .query_row(
            "SELECT host_epoch FROM occupant_bindings WHERE seat_id='recipient' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(binding_epoch, 1, "the recipient binding lags by one epoch");

    let sent = match service.handle(
        Command::SendMessage(SendMessage {
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(),
            body: "before the first pass".into(),
            invited_recipients: vec![SeatId::new("recipient")],
            deadline_millis: None,
            operation: OperationId::new("pre-pass-send"),
            claim: sender.clone(),
            relays_user: false,
            user_intent: None,
        }),
        peer,
        &budget(),
    ) {
        Ok(CommandResult::MessageSent(message)) => message,
        other => panic!("send before the first pass failed: {other:?}"),
    };
    let count = |sql: &str| -> i64 { db.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM prepared_unavailable_warnings"),
        0,
        "no recipient_unavailable warning for a seat the pending pass will carry"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM prepared_recipients WHERE seat_id='recipient' AND eligible_at_snapshot=0"
        ),
        1,
        "the recipient is staged not yet available"
    );
    assert_eq!(
        count("SELECT unavailability_open FROM seats WHERE id='recipient'"),
        0
    );

    // The first pass carries the binding: availability anchor plus the timer
    // for the receipt staged before it.
    let page = crate::store::seats::saved_seats_page(
        &context,
        &db,
        &publication.id,
        0,
        None,
        16,
        &budget(),
    )
    .unwrap();
    let mut conn = context.open_writer().unwrap();
    let mut carried = 0;
    for transition in crate::identity::reconcile::plan_page(&page).unwrap() {
        if transition.seat.as_str() == "recipient" {
            assert!(
                matches!(
                    transition.action,
                    crate::ports::ReconciliationAction::CarryForward { .. }
                ),
                "{:?} {:?}",
                transition.action,
                page.seats
            );
            assert_eq!(
                crate::store::seats::apply_reconciliation_transition(
                    &context,
                    &mut conn,
                    transition,
                    &budget()
                )
                .unwrap(),
                crate::ports::ReconciliationOutcome::Applied
            );
            carried += 1;
        }
    }
    assert_eq!(carried, 1, "the pass plans one carry for the recipient");
    let CommandResult::PendingReceipts(pending) = service
        .handle(
            Command::PendingReceipts(crate::protocol::commands::PendingReceiptsQuery {
                seat: Some(SeatId::new("recipient")),
                thread: None,
                page: Default::default(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong query result")
    };
    let item = pending
        .items
        .iter()
        .find(|item| item.message == sent)
        .expect("the staged receipt is pending");
    assert!(
        item.available_at.is_some(),
        "the carry-forward starts the receipt timer: {item:?}"
    );
    assert_eq!(
        count("SELECT count(*) FROM prepared_unavailable_warnings"),
        0
    );
}

#[test]
fn operator_check_in_requires_human_lifecycle() {
    let (service, db) = fixture();
    let operator = |command: Command| match command {
        Command::CheckIn(check) => Command::OperatorCheckIn(check),
        other => other,
    };
    let call = |command: Command, uid: u32| {
        service.handle(command, PeerIdentity::from_kernel(uid), &budget())
    };
    let human = |claim: CallerClaim| CallerClaim {
        harness: Harness::Human,
        native_session: NativeSessionId::new("plugin_context:person"),
        ..claim
    };
    // An agent harness is never an operator override.
    let refused = call(operator(lifecycle(claim(), "agent")), 501).unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidRequest);
    // Nor is a Current-mode (non-lifecycle) check-in.
    let current = Command::OperatorCheckIn(CheckIn {
        mode: CheckInMode::Current,
        claim: human(claim()),
        operation: OperationId::new("current"),
    });
    assert_eq!(
        call(current, 501).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    // Only the elected owner may override.
    let foreign = operator(lifecycle(human(claim()), "foreign"));
    assert_eq!(
        call(foreign, 502).unwrap_err().code,
        ErrorCode::Unauthorized
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // A plain human check-in over an agent's binding is refused with guidance.
    let agent = checked(&service, lifecycle(claim(), "agent-ok"));
    let plain = lifecycle(
        human(CallerClaim {
            binding_generation: agent.context.binding_generation,
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            ..claim()
        }),
        "human",
    );
    let refused = call(plain, 501).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Unauthorized);
    assert!(refused.detail.contains("me init --operator"));
}

#[test]
fn lazy_inbox_fix_real_service_completion() {
    use crate::protocol::commands::{CompleteInboxDelivery, DeliveryMode, SendMessage};
    let (service, db) = fixture();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient','i','resolved','native','q',0,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','q','b',1,0,1,'fresh','unknown','unknown',0,0,'term-q','inc','coherent_enumeration',1)", []).unwrap();
    let peer = PeerIdentity::from_kernel(501);
    let sender = checked(&service, lifecycle(claim(), "sender")).context;
    let mut recipient = claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = HostTargetId::new("q");
    recipient.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let recipient = checked(&service, lifecycle(recipient, "recipient")).context;
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: sender.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,'recipient','joined')",
        [thread.as_str()],
    )
    .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,'recipient',1,1)", [thread.as_str()]).unwrap();
    let send = |op| {
        let CommandResult::MessageSent(id) = service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    thread: thread.clone(),
                    body: "quiet".into(),
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(op),
                    claim: sender.clone(),
                    relays_user: false,
                    user_intent: None,
                }),
                peer,
                &budget(),
            )
            .unwrap()
        else {
            panic!("send")
        };
        id
    };
    let id = send("lazy");
    let request = CompleteInboxDelivery {
        via: None,
        messages: vec![id.clone()],
        operation: OperationId::new("done"),
        claim: recipient.clone(),
    };
    let invoke = |r| service.handle(Command::CompleteInboxDelivery(r), peer, &budget());
    assert_eq!(
        service
            .handle(
                Command::CompleteInboxDelivery(request.clone()),
                PeerIdentity::from_kernel(502),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::Unauthorized,
    );
    let mut sub = request.clone();
    sub.claim.role = CallerRole::Subagent;
    sub.operation = OperationId::new("sub");
    assert_eq!(invoke(sub).unwrap_err().code, ErrorCode::CallerUnverified);
    let mut stale = request.clone();
    stale.claim.binding_generation += 1;
    stale.operation = OperationId::new("stale");
    assert!(invoke(stale).is_err());
    assert_eq!(
        db.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id=?1",
            [id.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    let result = CommandResult::InboxDeliveryCompleted(vec![id]);
    assert_eq!(invoke(request.clone()).unwrap(), result);
    assert_eq!(invoke(request).unwrap(), result);
    let human = send("human-lazy");
    db.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id='recipient' AND ended_at IS NULL", []).unwrap();
    assert_eq!(
        invoke(CompleteInboxDelivery {
            via: None,
            messages: vec![human.clone()],
            operation: OperationId::new("human-done"),
            claim: CallerClaim {
                harness: Harness::Human,
                ..recipient
            }
        })
        .unwrap(),
        CommandResult::InboxDeliveryCompleted(vec![human])
    );
}
