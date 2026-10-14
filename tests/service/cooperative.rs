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
        native_session: NativeSessionId::new("session-n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
fn lifecycle(claim: CallerClaim, operation: &str) -> Command {
    Command::CheckIn(CheckIn {
        lineage_parent: None,
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
    // ht-54r.3: the seat starts at generation 0 here and a check-in never bumps it.
    assert_eq!(result.context.binding_generation, 0);
    assert_eq!(checked(&service, original), result);
    assert_eq!(
        checked(
            &service,
            Command::CheckIn(CheckIn {
                lineage_parent: None,
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
        ErrorCode::SubagentReadOnly
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
                confirmation: None,
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
                priority: None,
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                thread: created.clone(),
                body: "pending mail".into(),
                direct_recipient: None,
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
    recipient.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
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
                        lineage_parent: None,
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
    // The same session's binding is still open, and execution is not binding identity
    // (spec section 1; ht-54r.3), so replaying the predecessor check-in is `Current`.
    assert_eq!(
        replay.context_disposition,
        CheckInContextDisposition::Current
    );
    // The replay cannot restore the predecessor: the open binding keeps the successor's
    // execution, and no second binding was registered or forked.
    assert_eq!(
        db.query_row(
            "SELECT execution_id FROM occupant_bindings WHERE seat_id='s' AND native_session='session-n' AND ended_at IS NULL",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "00000000-0000-4000-8000-000000000002"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // ht-54r.3: the same session re-registers in place; the mapping generation stays 0.
    assert_eq!(successor.context.binding_generation, 0);
    assert_eq!(
        db.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
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
    // Once the session's binding has ended, the same replay presents the stored result as history.
    db.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    let ended_replay = checked(&service, lifecycle(claim(), "initial"));
    assert_eq!(ended_replay.context, initial.context);
    assert_eq!(
        ended_replay.context_disposition,
        CheckInContextDisposition::Historical
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
        priority: None,
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread,
        body: "bounded preparation".into(),
        direct_recipient: None,
        invited_recipients: (0..40)
            .map(|index| SeatId::new(format!("recipient-{index}")))
            .collect(),
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
        priority: None,
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread,
        body: "stale sender".into(),
        direct_recipient: None,
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
    // Retain the prepared snapshot while simulating a rebind of the seat (a new mapping
    // generation), which fences every context prepared before it.
    db.execute("UPDATE seats SET generation=generation+1", [])
        .unwrap();
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
                0
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
                    0
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
    recipient.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
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
                confirmation: None,
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
                lineage_parent: None,
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
            priority: None,
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(),
            body: format!("mail {operation}"),
            direct_recipient: None,
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
    let plain = send(&sender, vec![SeatId::new("recipient")], "a-plain");
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
    recipient.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
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
                confirmation: None,
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
            priority: None,
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(),
            body: "before the first pass".into(),
            direct_recipient: None,
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

/// A process table: only the listed pids are alive (start stamp 1).
struct AlivePids(&'static [u32]);
impl crate::ports::ProcessInfo for AlivePids {
    fn process(&self, pid: u32) -> Option<crate::ports::ProcessRecord> {
        self.0.contains(&pid).then(|| crate::ports::ProcessRecord {
            pid,
            parent: None,
            start: 1,
            argv: Vec::new(),
        })
    }
}

/// The sender (`LivenessFingerprint::TEST`, pid 1) and the recipient's main
/// session (pid 2) are alive.
const SENDER_AND_MAIN_ALIVE: AlivePids = AlivePids(&[1, 2]);
/// Only the sender is alive.
const SENDER_ALIVE: AlivePids = AlivePids(&[1]);

/// A sender `s` on `p` and a recipient seat `recipient` on `q`, both joined to
/// one thread, built through the production check-in and thread commands
/// after a fully reconciled epoch-1 publication (no baseline hold left). The
/// recipient's main session is a lifecycle check-in with pid 2.
struct SendScenario {
    service: OwnedFixture<DomainService>,
    db: OwnedFixture<Connection>,
    context: StoreContext,
    store: Arc<dyn StorePort>,
    sender: CallerClaim,
    main: CallerClaim,
    thread: ThreadId,
}

impl SendScenario {
    fn new() -> Self {
        use crate::protocol::commands::{Accept, Invite};
        let (service, db) = fixture();
        let path = db.directory.0.join("store.db");
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let store: Arc<dyn StorePort> = Arc::new(
            SqliteStore::new(
                StoreContext::new(path, Arc::new(FixedClock)),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient','i','resolved','native','q',0,0,0)", []).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','q','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'q','inc','coherent_enumeration',1)", []).unwrap();
        // Allocation records the seats' structural proof (terminal and
        // incarnation); the raw fixture rows carry none, so record it here.
        db.execute("UPDATE seats SET structural_terminal_id='term-'||target_id,structural_incarnation='inc',structural_incarnation_kind='coherent_enumeration',structural_host_boot='b',structural_host_epoch=1,structural_connection_epoch=1,structural_observation_sequence=1,target_generation=1 WHERE id IN ('s','recipient')", []).unwrap();
        db.execute("UPDATE observed_targets SET generation=1", [])
            .unwrap();
        let mut scenario = Self {
            service,
            db,
            context,
            store,
            sender: claim(),
            main: claim(),
            thread: ThreadId::new("unset"),
        };
        // Epoch 1 is published and fully reconciled first, so no baseline
        // hold is left for a later marker to lift (a lift bumps both fences).
        let first = publish_snapshot_at(&scenario.context, 1, 2, &["p", "q"], true);
        scenario.reconcile_pages(&first);
        assert!(
            scenario
                .store
                .record_reconciliation_pass(&first, &budget())
                .unwrap()
        );
        assert_eq!(
            scenario.count("SELECT baseline_hold_unclaimed FROM host_instances WHERE id='i'"),
            0
        );
        let service = &scenario.service;
        let peer = PeerIdentity::from_kernel(501);
        let sender = checked(service, lifecycle(claim(), "sender")).context;
        let mut main = claim();
        main.seat = SeatId::new("recipient");
        main.target = HostTargetId::new("q");
        main.native_session = NativeSessionId::new("recipient-main");
        main.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
        main.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
        let main = checked(service, lifecycle(main, "recipient-main")).context;
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
                    confirmation: None,
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
                    claim: main.clone(),
                }),
                peer,
                &budget(),
            )
            .unwrap();
        scenario.sender = sender;
        scenario.main = main;
        scenario.thread = thread;
        scenario
    }

    fn count(&self, sql: &str) -> i64 {
        self.db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn fences(&self) -> (i64, i64) {
        self.db
            .query_row(
                "SELECT lifecycle_revision,send_eligibility_revision FROM host_instances WHERE id='i'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn available(&self) -> bool {
        crate::store::schema::effective_registered_availability(&self.db, "recipient", Some("i"))
            .unwrap()
            .is_some()
    }

    fn carry_pending(&self) -> bool {
        crate::store::schema::carry_pending(&self.db, "recipient", "i").unwrap()
    }

    /// The production pass: plan each saved-seat page, apply its
    /// transitions. Returns the planned `seat:action` list.
    fn reconcile_pages(&self, publication: &crate::ports::PublishedSnapshot) -> Vec<String> {
        let mut planned = Vec::new();
        let mut after = 0;
        let mut high = None;
        loop {
            let page = self
                .store
                .saved_seats_page(&publication.id, after, high, 16, &budget())
                .unwrap();
            for transition in crate::identity::reconcile::plan_page(&page).unwrap() {
                planned.push(format!(
                    "{}:{:?}",
                    transition.seat.as_str(),
                    transition.action
                ));
                self.store
                    .apply_reconciliation_transition(transition, &budget())
                    .unwrap();
            }
            if !page.has_more {
                return planned;
            }
            after = page.after_ordinal;
            high = Some(page.high_water_ordinal);
        }
    }

    /// The liveness worker's pass over `processes`; returns how many bindings
    /// it disconnected.
    fn liveness_pass(&self, processes: AlivePids) -> usize {
        crate::service::liveness::LivenessWorker::new(
            self.store.clone(),
            Arc::new(crate::service::fair_writer::FairWriter::new(32)),
            Arc::new(processes),
        )
        .pass(&budget())
        .unwrap()
        .disconnected
    }

    fn request(&self, operation: &str) -> crate::protocol::commands::SendMessage {
        crate::protocol::commands::SendMessage {
            priority: None,
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread: self.thread.clone(),
            body: "staged before, decided after".into(),
            direct_recipient: None,
            invited_recipients: vec![SeatId::new("recipient")],
            deadline_millis: None,
            operation: OperationId::new(operation),
            claim: self.sender.clone(),
            relays_user: false,
            user_intent: None,
        }
    }

    /// Bounded preparation to Ready through the store port.
    fn prepare(&self, send: &crate::protocol::commands::SendMessage) {
        use crate::ports::SendPreparationProgress;
        loop {
            match self
                .store
                .prepare_send_step(
                    send,
                    crate::ports::DurableWorkAdmission::new(16).unwrap(),
                    &budget(),
                )
                .unwrap()
            {
                SendPreparationProgress::Ready { .. } => break,
                SendPreparationProgress::More { .. } => {}
                other => panic!("unexpected preparation progress {other:?}"),
            }
        }
    }

    /// Recipient warnings staged by a live (not discarded) preparation.
    fn staged_warnings(&self) -> i64 {
        self.count("SELECT count(*) FROM prepared_unavailable_warnings w JOIN send_preparations p ON p.id=w.preparation_id WHERE p.status!='discarded' AND w.affected_seat_id='recipient'")
    }

    /// Decides the prepared `send` with a fresh permit, as the service does;
    /// a refused stale preparation is retried once under a fresh operation,
    /// as a client would. Returns the message and how many decisions ran.
    fn decide_with_retry(&self, send: crate::protocol::commands::SendMessage) -> (MessageId, u32) {
        use crate::protocol::commands::PermitMutation;
        let mut send = send;
        let mut attempts = 0;
        loop {
            attempts += 1;
            assert!(attempts <= 2, "the send does not converge");
            let mutation = PermitMutation::SendMessage(send.clone());
            let permit = self
                .store
                .issue_cooperative_permit(
                    crate::store::cooperative_permit_request(&mutation).unwrap(),
                    &budget(),
                )
                .unwrap();
            match self.store.mutate(mutation, permit, &budget()) {
                Ok(CommandResult::MessageSent(message)) => return (message, attempts),
                Err(error)
                    if error.code == ErrorCode::Conflict
                        && error
                            .detail
                            .starts_with("send preparation snapshot changed") =>
                {
                    send = self.request(&format!("{}-retry", send.operation.as_str()));
                    self.prepare(&send);
                }
                other => panic!("unexpected decision {other:?}"),
            }
        }
    }

    /// What the published send recorded for the recipient: its staged
    /// eligibility and the published unavailable warnings.
    fn published(&self, message: &MessageId) -> (bool, Vec<String>) {
        let eligible: bool = self
            .db
            .query_row(
                "SELECT r.eligible_at_snapshot FROM prepared_recipients r JOIN send_manifests m ON m.preparation_id=r.preparation_id WHERE m.message_id=?1 AND r.seat_id='recipient'",
                [message.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        let warnings = self
            .db
            .prepare("SELECT w.affected_seat_id FROM prepared_unavailable_warnings w JOIN send_manifests m ON m.preparation_id=w.preparation_id WHERE m.message_id=?1 ORDER BY w.warning_offset")
            .unwrap()
            .query_map([message.as_str()], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        (eligible, warnings)
    }

    /// The recipient's canonical warning requirement: unavailable and no
    /// pending carry (it is joined, with no pending invitation).
    fn warned(&self) -> bool {
        !self.available() && !self.carry_pending()
    }

    /// Everything a send stages for the recipient from its bindings:
    /// availability with its provenance, the pending carry deciding the
    /// warning, and whether an ACK is expected (a person's binding).
    fn staged_state(&self) -> (Option<String>, bool, bool) {
        (
            crate::store::schema::effective_registered_availability(
                &self.db,
                "recipient",
                Some("i"),
            )
            .unwrap(),
            self.carry_pending(),
            crate::store::schema::open_binding_role(&self.db, &SeatId::new("recipient")).unwrap()
                == Some("human"),
        )
    }

    /// Stage a send with the recipient in its current state, run `change`
    /// between staging and the decision, and decide. The published send must
    /// match the canonical state after `change` (eligible when available,
    /// warned when unavailable with no pending carry); a change of the staged
    /// state refuses the stale preparation once, and one that leaves it
    /// unchanged refuses nothing (one attempt). Returns the attempts.
    fn stage_change_decide(&self, label: &str, change: impl FnOnce(&Self)) -> u32 {
        let staged = self.staged_state();
        let send = self.request(label);
        self.prepare(&send);
        assert_eq!(
            self.staged_warnings(),
            i64::from(self.warned()),
            "{label}: staged with the pre-change state"
        );
        let fences = self.fences();
        change(self);
        let (after, warned) = (self.available(), self.warned());
        let (message, attempts) = self.decide_with_retry(send);
        assert_eq!(
            self.published(&message),
            (
                after,
                if warned {
                    vec!["recipient".to_owned()]
                } else {
                    vec![]
                }
            ),
            "{label}: the published send matches the canonical post-change state (fences at staging {fences:?}, after the change {:?})",
            self.fences()
        );
        let now = self.staged_state();
        assert_eq!(
            attempts,
            if staged != now { 2 } else { 1 },
            "{label}: refused once exactly when the staged state changed ({staged:?} -> {now:?})"
        );
        attempts
    }
}

// Kills: a liveness-worker disconnect (process exit) that ends a recipient's
// availability without bumping the send-eligibility fence, so a send staged
// while it was available publishes it eligible and unwarned (shared contract
// amendment §8: "process exit ... or transition to unavailable" bumps).
#[test]
fn liveness_disconnect_between_stage_and_decide_publishes_the_warning() {
    let scenario = SendScenario::new();
    assert!(scenario.available());
    scenario.stage_change_decide("liveness-exit", |s| {
        assert_eq!(s.liveness_pass(SENDER_ALIVE), 1);
        assert!(!s.available());
    });
}

// Kills: a SessionEnd binding event that disconnects the recipient's only
// connected session without a fence bump.
#[test]
fn session_end_between_stage_and_decide_publishes_the_warning() {
    use crate::protocol::commands::BindingEventKind::SessionEnd;
    let scenario = SendScenario::new();
    assert!(scenario.available());
    scenario.stage_change_decide("session-end", |s| {
        assert_eq!(
            recorded(
                &s.service,
                binding_event(s.main.clone(), SessionEnd, None, "main-end")
            ),
            1
        );
        assert!(!s.available());
    });
}

// Kills: a SubagentStop that disconnects a registered binding without a fence
// bump. The top-level session ends first (disconnecting its children), then a
// fresh subagent of it starts: it is the seat's only connected registered
// binding, so its stop is a real transition to unavailable.
#[test]
fn subagent_stop_between_stage_and_decide_publishes_the_warning() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStart, SubagentStop};
    let scenario = SendScenario::new();
    let sub = subagent_of(&scenario.main);
    assert_eq!(
        recorded(
            &scenario.service,
            binding_event(sub.clone(), SubagentStart, None, "sub-start")
        ),
        1
    );
    let connected = |s: &SendScenario| {
        s.count("SELECT count(*) FROM occupant_bindings WHERE seat_id='recipient' AND ended_at IS NULL AND liveness='connected' AND registered_at IS NOT NULL")
    };
    assert_eq!(connected(&scenario), 2);
    // A stop of a subagent the seat still has a connected parent for is a
    // conservative refusal at most; the published send stays canonical.
    scenario.stage_change_decide("sub-stop-with-parent", |s| {
        assert_eq!(
            recorded(
                &s.service,
                binding_event(sub.clone(), SubagentStop, None, "sub-stop-1")
            ),
            1
        );
        assert!(s.available());
    });
    assert_eq!(
        recorded(
            &scenario.service,
            binding_event(scenario.main.clone(), SessionEnd, None, "main-end")
        ),
        1
    );
    let sub = CallerClaim {
        agent_id: Some(AgentId::new("agent-2")),
        ..sub
    };
    // The subagent start re-checks the parent's mapping, not its liveness.
    let restarted = scenario.service.handle(
        binding_event(sub.clone(), SubagentStart, None, "sub-start-2"),
        PeerIdentity::from_kernel(501),
        &budget(),
    );
    match restarted {
        Ok(CommandResult::BindingEventRecorded { changed: 1 }) if connected(&scenario) == 1 => {
            assert!(
                scenario.available(),
                "the subagent alone keeps the seat available"
            );
            scenario.stage_change_decide("sub-stop-alone", |s| {
                assert_eq!(
                    recorded(
                        &s.service,
                        binding_event(sub.clone(), SubagentStop, None, "sub-stop-2")
                    ),
                    1
                );
                assert!(!s.available());
            });
        }
        other => panic!(
            "a subagent of an ended session must be able to start for this shape: {other:?}, connected {}",
            connected(&scenario)
        ),
    }
}

// Proves connect() is fenced on every path, not only the check-in: a plain
// call that reconnects a process-exited binding (no check-in) makes the
// recipient available again; a send staged while it was unavailable (warned)
// must not publish that stale warning.
#[test]
fn plain_call_reconnect_between_stage_and_decide_drops_the_stale_warning() {
    use crate::protocol::commands::AdvanceCursors;
    let scenario = SendScenario::new();
    assert_eq!(scenario.liveness_pass(SENDER_ALIVE), 1);
    assert!(!scenario.available());
    scenario.stage_change_decide("plain-reconnect", |s| {
        let resumed = CallerClaim {
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 4, start: 1 }),
            ..s.main.clone()
        };
        s.service
            .handle(
                Command::AdvanceCursors(AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: resumed,
                    operation: OperationId::new("main-plain-call"),
                    displayed: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap();
        assert!(s.available(), "the plain call reconnected the binding");
    });
}

// Enrollment inserts a registered row already connected (not through
// `connect`): a plain call from a new session of the recipient's pane makes
// the seat available again; the stale warning staged before must not publish.
#[test]
fn plain_call_enrollment_between_stage_and_decide_drops_the_stale_warning() {
    use crate::protocol::commands::AdvanceCursors;
    let scenario = SendScenario::new();
    assert_eq!(scenario.liveness_pass(SENDER_ALIVE), 1);
    assert!(!scenario.available());
    scenario.stage_change_decide("plain-enroll", |s| {
        let fresh = CallerClaim {
            native_session: NativeSessionId::new("recipient-fresh"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000005"),
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 5, start: 1 }),
            ..s.main.clone()
        };
        s.service
            .handle(
                Command::AdvanceCursors(AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: fresh,
                    operation: OperationId::new("fresh-plain-call"),
                    displayed: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap();
        assert!(s.available(), "the plain call enrolled a connected binding");
    });
}

// Kills: a liveness fence that bumps on every binding row change rather than
// on a change of the seat's staged (availability, provenance) pair. A Claude
// subagent starting and stopping under a connected parent changes neither, so
// an in-flight send is decided in one attempt (each refusal makes the CLI drop
// the intent).
#[test]
fn subagent_start_and_stop_under_a_connected_parent_refuse_no_send() {
    use crate::protocol::commands::BindingEventKind::{SubagentStart, SubagentStop};
    let scenario = SendScenario::new();
    let sub = subagent_of(&scenario.main);
    let attempts = scenario.stage_change_decide("quiet-sub-start", |s| {
        assert_eq!(
            recorded(
                &s.service,
                binding_event(sub.clone(), SubagentStart, None, "quiet-sub-start")
            ),
            1
        );
    });
    assert_eq!(attempts, 1, "quiet-sub-start refuses no in-flight send");
    let attempts = scenario.stage_change_decide("quiet-sub-stop", |s| {
        assert_eq!(
            recorded(
                &s.service,
                binding_event(sub.clone(), SubagentStop, None, "quiet-sub-stop")
            ),
            1
        );
    });
    assert_eq!(attempts, 1, "quiet-sub-stop refuses no in-flight send");
    assert!(scenario.available());
}

// Kills the same over-broad fence for one-shot sessions: a one-shot in the
// recipient's pane enrolls by plain call and exits while the main session
// stays connected; neither refuses an in-flight send. An idle liveness pass
// (everything alive) writes nothing.
#[test]
fn a_one_shot_under_a_live_main_session_refuses_no_send() {
    use crate::protocol::commands::AdvanceCursors;
    let scenario = SendScenario::new();
    let attempts = scenario.stage_change_decide("one-shot-enroll", |s| {
        let one_shot = CallerClaim {
            native_session: NativeSessionId::new("recipient-one-shot"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000003"),
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 3, start: 1 }),
            ..s.main.clone()
        };
        s.service
            .handle(
                Command::AdvanceCursors(AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: one_shot,
                    operation: OperationId::new("one-shot-plain-call"),
                    displayed: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap();
        assert_eq!(
            s.count("SELECT count(*) FROM occupant_bindings WHERE seat_id='recipient' AND native_session='recipient-one-shot' AND liveness='connected'"),
            1,
            "the one-shot enrolled"
        );
    });
    assert_eq!(attempts, 1, "one-shot-enroll refuses no in-flight send");
    let attempts = scenario.stage_change_decide("one-shot-exit", |s| {
        assert_eq!(s.liveness_pass(SENDER_AND_MAIN_ALIVE), 1);
    });
    assert_eq!(attempts, 1, "one-shot-exit refuses no in-flight send");
    let idle = scenario.fences();
    assert_eq!(scenario.liveness_pass(SENDER_AND_MAIN_ALIVE), 0);
    assert_eq!(scenario.fences(), idle, "an idle liveness pass stays quiet");
    assert!(scenario.available());
}

// Kills a fence that compares availability only: in the C4 window (epoch 2
// published, pass not run) the recipient's lagging main session is connected,
// so a send stages it unwarned (carry assumed). Its SessionEnd ends the pending
// carry while availability stays None; the decision must refuse the stale
// staging and publish the warning.
#[test]
fn a_pending_carry_ending_between_stage_and_decide_publishes_the_warning() {
    use crate::protocol::commands::BindingEventKind::SessionEnd;
    let scenario = SendScenario::new();
    publish_snapshot_at(&scenario.context, 2, 7, &["p", "q"], true);
    assert!(!scenario.available());
    assert!(
        scenario.carry_pending(),
        "lagging connected main: carry pending"
    );
    let attempts = scenario.stage_change_decide("carry-pending-end", |s| {
        assert_eq!(
            recorded(
                &s.service,
                binding_event(s.main.clone(), SessionEnd, None, "main-end-window")
            ),
            1
        );
        assert!(!s.carry_pending(), "the ended session is no longer carried");
    });
    assert_eq!(attempts, 2);
}

// Kills a fence around the reconnect alone: a V32-migrated binding (migrated,
// disconnected `migrated`, stale placement; the shape the migration leaves) is
// re-adopted by a plain call, which reconnects it and then refreshes its
// placement. Only the refresh makes the seat available; both must sit inside
// one fence.
#[test]
fn a_readoption_refreshing_a_stale_placement_between_stage_and_decide_is_fenced() {
    let scenario = SendScenario::new();
    scenario.db.execute("UPDATE occupant_bindings SET migrated=1, migrated_session=native_session, liveness='disconnected', disconnected_at=1, disconnect_reason='migrated', liveness_pid=NULL, liveness_start=NULL, target_generation=0 WHERE seat_id='recipient'", []).unwrap();
    assert!(!scenario.available());
    let attempts = scenario.stage_change_decide("readopt", |s| {
        let resumed = CallerClaim {
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 4, start: 1 }),
            ..s.main.clone()
        };
        s.service
            .handle(
                Command::AdvanceCursors(crate::protocol::commands::AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: resumed,
                    operation: OperationId::new("main-readopt"),
                    displayed: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap();
        assert!(s.available(), "the plain call re-adopted the binding");
    });
    assert_eq!(attempts, 2);
}

// Kills the same gap on reopening: after an unresolve (bindings ended
// `seat_unresolved`) and a rebind (seat generation + 1), the session's plain
// call reopens its own row, reconnects it and refreshes its placement.
#[test]
fn a_plain_reopen_after_a_rebind_between_stage_and_decide_is_fenced() {
    let scenario = SendScenario::new();
    scenario.db.execute("UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=1, disconnect_reason='seat_unresolved', ended_at=1 WHERE seat_id='recipient'", []).unwrap();
    scenario
        .db
        .execute(
            "UPDATE seats SET generation=generation+1 WHERE id='recipient'",
            [],
        )
        .unwrap();
    assert!(!scenario.available());
    let attempts = scenario.stage_change_decide("reopen", |s| {
        let resumed = CallerClaim {
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 4, start: 1 }),
            binding_generation: s.main.binding_generation + 1,
            ..s.main.clone()
        };
        s.service
            .handle(
                Command::AdvanceCursors(crate::protocol::commands::AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: resumed,
                    operation: OperationId::new("main-reopen"),
                    displayed: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap();
        assert!(s.available(), "the plain call reopened the binding");
    });
    assert_eq!(attempts, 2);
}

// Pins the Current-mode check-in placement write inside the fence: in the C4
// window the lagging main session's Current check-in rewrites its placement
// to the host epoch, making the seat available (and ending the pending
// carry); a send staged before must be refused once and publish eligible.
#[test]
fn a_current_check_in_refreshing_the_placement_between_stage_and_decide_is_fenced() {
    let scenario = SendScenario::new();
    publish_snapshot_at(&scenario.context, 2, 7, &["p", "q"], true);
    assert!(!scenario.available());
    let attempts = scenario.stage_change_decide("current-check-in", |s| {
        checked(
            &s.service,
            Command::CheckIn(CheckIn {
                lineage_parent: None,
                mode: CheckInMode::Current,
                claim: s.main.clone(),
                operation: OperationId::new("main-current"),
            }),
        );
        assert!(
            s.available(),
            "the Current check-in refreshed the placement"
        );
    });
    assert_eq!(attempts, 2);
}

// Kills a publication recheck that compares the live send fence after the
// author's own claim resolution: an author whose only session exited sends
// from a new session by plain call; its enrollment makes the author's seat
// available (a real fence bump), yet it is never a staged recipient, so the
// send is decided in one attempt.
#[test]
fn an_authors_first_plain_call_enrollment_sends_in_one_attempt() {
    let scenario = SendScenario::new();
    // Only the recipient's main session is alive: the sender's session exits.
    assert_eq!(scenario.liveness_pass(AlivePids(&[2])), 1);
    let author_seat_available = |s: &SendScenario| {
        crate::store::schema::effective_registered_availability(&s.db, "s", Some("i"))
            .unwrap()
            .is_some()
    };
    assert!(!author_seat_available(&scenario));
    let mut send = scenario.request("first-plain-send");
    send.claim = CallerClaim {
        native_session: NativeSessionId::new("sender-fresh"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000006"),
        liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 6, start: 1 }),
        ..scenario.sender.clone()
    };
    scenario.prepare(&send);
    let staged = scenario.fences();
    let (message, attempts) = scenario.decide_with_retry(send);
    assert!(
        author_seat_available(&scenario),
        "the send enrolled the author's new session"
    );
    assert!(
        scenario.fences().1 > staged.1,
        "the author's enrollment bumped the fence"
    );
    assert_eq!(attempts, 1, "the author's own bump refuses nothing");
    assert_eq!(scenario.published(&message), (true, vec![]));
}

/// When the send's recipients are staged relative to the reconciliation pass
/// of the new host epoch.
#[derive(Clone, Copy, Debug)]
enum PassRace {
    /// Staged before the saved-seat page is planned and applied.
    StagedBeforePage,
    /// Staged after the saved-seat page, before the pass records its marker.
    StagedAfterPage,
}

/// Production shape (Main review I1 / C4): the recipient's main cooperative
/// session (pid 2) is registered at host epoch 1; Herdr restarts (epoch 2,
/// same boot and incarnation; the pass has not run, so the marker lags); a
/// one-shot agent in the same pane checks in as a second session at epoch 2
/// and exits (liveness `process_exit`). The seat's newest open registered
/// binding is epoch 2 and disconnected; the main session, still alive, lags at
/// epoch 1. The sender checks in again first, so the pass has nothing of the
/// sender's to carry. Returns the epoch-2 publication.
fn one_shot_after_restart(scenario: &SendScenario) -> crate::ports::PublishedSnapshot {
    let second = publish_snapshot_at(&scenario.context, 2, 7, &["p", "q"], true);
    checked(
        &scenario.service,
        lifecycle(scenario.sender.clone(), "sender-again"),
    );
    let mut one_shot = scenario.main.clone();
    one_shot.native_session = NativeSessionId::new("recipient-one-shot");
    one_shot.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 3, start: 1 });
    one_shot.execution = ExecutionId::new("00000000-0000-4000-8000-000000000003");
    checked(&scenario.service, lifecycle(one_shot, "recipient-one-shot"));
    assert_eq!(scenario.liveness_pass(SENDER_AND_MAIN_ALIVE), 1);
    assert_eq!(
        recipient_bindings(scenario),
        vec![
            ("recipient-main".to_owned(), 1, "connected".to_owned()),
            (
                "recipient-one-shot".to_owned(),
                2,
                "disconnected".to_owned()
            ),
        ],
        "older connected carried binding, newest registered binding disconnected"
    );
    assert!(
        scenario.carry_pending(),
        "the lagging marker leaves a carry pending"
    );
    assert!(!scenario.available());
    second
}

/// `(native_session, host_epoch, liveness)` of the recipient's open registered
/// bindings, in ordinal order.
fn recipient_bindings(scenario: &SendScenario) -> Vec<(String, i64, String)> {
    scenario
        .db
        .prepare("SELECT native_session,host_epoch,liveness FROM occupant_bindings WHERE seat_id='recipient' AND ended_at IS NULL AND registered_at IS NOT NULL ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The pass plans exactly one carry, for the recipient.
fn assert_recipient_carried(planned: &[String]) {
    let carries: Vec<&String> = planned
        .iter()
        .filter(|p| p.contains("CarryForward"))
        .collect();
    assert_eq!(
        carries.len(),
        1,
        "one carry is planned, for the recipient: {planned:?}"
    );
    assert!(carries[0].starts_with("recipient:"), "{planned:?}");
}

// Kills (C4): a planner that decides the carry from the newest open
// registered binding only. The newer one-shot binding (epoch 2) must not hide
// the older, still connected main binding (epoch 1): the pass carries it, so
// the recipient is available after the pass, the wake lane's seat binding
// (newest open top-level, connected first) is current in the host epoch, and
// the episode the one-shot's exit opened is closed.
#[test]
fn an_older_connected_binding_is_carried_past_a_newer_session_of_the_new_epoch() {
    let scenario = SendScenario::new();
    let second = one_shot_after_restart(&scenario);
    assert_recipient_carried(&scenario.reconcile_pages(&second));
    assert!(
        scenario
            .store
            .record_reconciliation_pass(&second, &budget())
            .unwrap()
    );
    assert_eq!(
        recipient_bindings(&scenario),
        vec![
            ("recipient-main".to_owned(), 2, "connected".to_owned()),
            (
                "recipient-one-shot".to_owned(),
                2,
                "disconnected".to_owned()
            ),
        ],
        "the carry moves the lagging binding; liveness is untouched"
    );
    assert!(scenario.available(), "the live main session is available");
    assert!(!scenario.carry_pending());
    let wake_binding: (i64, String) = scenario
        .db
        .query_row(
            &format!(
                "SELECT host_epoch,native_session FROM occupant_bindings WHERE binding_id=({})",
                crate::store::attention::SEAT_NOTICE_BINDING_SQL
            ),
            ["recipient"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        wake_binding,
        (2, "recipient-main".to_owned()),
        "wake follows the connected main binding, current in the host epoch"
    );
    assert_eq!(
        scenario.count("SELECT unavailability_open FROM seats WHERE id='recipient'"),
        0
    );
    // A send after the pass: eligible, no warning.
    let send = scenario.request("after-pass");
    scenario.prepare(&send);
    let (message, attempts) = scenario.decide_with_retry(send);
    assert_eq!(attempts, 1);
    assert_eq!(scenario.published(&message), (true, vec![]));
}

// Kills (C4, availability follows liveness): a carry that anchors availability
// (starting receipt timers) and closes the unavailability episode although no
// carried row is connected, and a `carry_pending` that suppresses the warning
// for a seat whose lagging bindings are all disconnected. Shape: the main
// session exits before Herdr restarts; after the restart a one-shot registers
// at epoch 2 and exits. The pass still carries the lagging main row (it
// resumes on its next reconnect), but the seat stays unavailable.
#[test]
fn a_disconnected_carry_neither_anchors_availability_nor_suppresses_the_warning() {
    let scenario = SendScenario::new();
    assert_eq!(
        scenario.liveness_pass(SENDER_ALIVE),
        1,
        "the main session exits"
    );
    let second = publish_snapshot_at(&scenario.context, 2, 7, &["p", "q"], true);
    checked(
        &scenario.service,
        lifecycle(scenario.sender.clone(), "sender-again"),
    );
    let mut one_shot = scenario.main.clone();
    one_shot.native_session = NativeSessionId::new("recipient-one-shot");
    one_shot.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 3, start: 1 });
    one_shot.execution = ExecutionId::new("00000000-0000-4000-8000-000000000003");
    checked(&scenario.service, lifecycle(one_shot, "recipient-one-shot"));
    assert_eq!(
        scenario.liveness_pass(SENDER_ALIVE),
        1,
        "the one-shot exits"
    );
    assert_eq!(
        recipient_bindings(&scenario),
        vec![
            ("recipient-main".to_owned(), 1, "disconnected".to_owned()),
            (
                "recipient-one-shot".to_owned(),
                2,
                "disconnected".to_owned()
            ),
        ]
    );
    assert!(!scenario.available());
    assert!(
        !scenario.carry_pending(),
        "no connected lagging binding: no carry restores availability"
    );
    // A send in the restart window warns about the recipient.
    let send = scenario.request("in-window");
    scenario.prepare(&send);
    assert_eq!(scenario.staged_warnings(), 1, "the window send warns");
    let anchors =
        || scenario.count("SELECT count(*) FROM seat_availability WHERE seat_id='recipient'");
    let anchored = anchors();
    assert_recipient_carried(&scenario.reconcile_pages(&second));
    assert!(
        scenario
            .store
            .record_reconciliation_pass(&second, &budget())
            .unwrap()
    );
    assert_eq!(
        recipient_bindings(&scenario),
        vec![
            ("recipient-main".to_owned(), 2, "disconnected".to_owned()),
            (
                "recipient-one-shot".to_owned(),
                2,
                "disconnected".to_owned()
            ),
        ],
        "the lagging row is carried, still disconnected"
    );
    assert!(!scenario.available());
    assert_eq!(
        anchors(),
        anchored,
        "no availability anchor without a connected row"
    );
    assert_eq!(
        scenario.count("SELECT unavailability_open FROM seats WHERE id='recipient'"),
        1,
        "the unavailability episode stays open"
    );
    let (message, _) = scenario.decide_with_retry(send);
    assert_eq!(
        scenario.published(&message),
        (false, vec!["recipient".to_owned()]),
        "the window send publishes the warning"
    );
}

/// A send staged across the pass of the new epoch (before or after its
/// saved-seat page, before its marker) and decided after the marker must
/// publish what the post-pass canonical state requires: the carried main
/// session makes the recipient available, eligible and unwarned. Every
/// transition in between (the carry, the moving marker) refuses the stale
/// preparation once. Kills: a carry or a moving marker that leaves the send
/// fences unchanged, so the decision keeps the pre-pass staging.
fn send_staged_across_the_reconciliation_pass(race: PassRace) {
    let scenario = SendScenario::new();
    let second = one_shot_after_restart(&scenario);
    let send = scenario.request("across-pass");
    if matches!(race, PassRace::StagedAfterPage) {
        assert_recipient_carried(&scenario.reconcile_pages(&second));
        assert!(scenario.available());
    }
    scenario.prepare(&send);
    assert_eq!(
        scenario.staged_warnings(),
        0,
        "{race:?}: no warning staged (a pending carry, or carried)"
    );
    if matches!(race, PassRace::StagedBeforePage) {
        assert_recipient_carried(&scenario.reconcile_pages(&second));
    }
    let before_marker = scenario.fences();
    assert!(
        scenario
            .store
            .record_reconciliation_pass(&second, &budget())
            .unwrap()
    );
    let after_marker = scenario.fences();
    assert_eq!(
        after_marker.1,
        before_marker.1 + 1,
        "{race:?}: the moving marker bumps the send fence once"
    );
    assert!(scenario.available());
    assert!(!scenario.carry_pending());
    let (message, attempts) = scenario.decide_with_retry(send);
    assert_eq!(
        scenario.published(&message),
        (true, vec![]),
        "{race:?}: the published send is the post-pass canonical one"
    );
    assert_eq!(
        attempts, 2,
        "{race:?}: the stale preparation is refused once"
    );
}

#[test]
fn send_staged_before_the_saved_seat_page_publishes_the_post_pass_state() {
    send_staged_across_the_reconciliation_pass(PassRace::StagedBeforePage);
}

#[test]
fn send_staged_after_the_saved_seat_page_publishes_the_post_pass_state() {
    send_staged_across_the_reconciliation_pass(PassRace::StagedAfterPage);
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
        lineage_parent: None,
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
    recipient.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
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
                    priority: None,
                    delivery_mode: DeliveryMode::Lazy,
                    thread: thread.clone(),
                    body: "quiet".into(),
                    direct_recipient: None,
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
    assert_eq!(invoke(sub).unwrap_err().code, ErrorCode::SubagentReadOnly);
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

#[test]
fn two_sessions_on_one_seat_both_send() {
    use crate::protocol::commands::SendMessage;
    let (service, db) = fixture();
    let session = |name: &str, execution: &str| CallerClaim {
        native_session: NativeSessionId::new(name),
        execution: ExecutionId::new(execution),
        ..claim()
    };
    let a = checked(
        &service,
        lifecycle(
            session("sess-A", "00000000-0000-4000-8000-0000000000a1"),
            "a",
        ),
    )
    .context;
    let b = checked(
        &service,
        lifecycle(
            session("sess-B", "00000000-0000-4000-8000-0000000000b1"),
            "b",
        ),
    )
    .context;
    let peer = PeerIdentity::from_kernel(501);
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: a.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    for (who, claim) in [("a", a), ("b", b)] {
        let sent = service
            .handle(
                Command::SendMessage(SendMessage {
                    priority: None,
                    delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                    thread: thread.clone(),
                    body: format!("from {who}"),
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("send-{who}")),
                    claim,
                    relays_user: false,
                    user_intent: None,
                    direct_recipient: None,
                }),
                peer,
                &budget(),
            )
            .unwrap();
        assert!(
            matches!(sent, CommandResult::MessageSent(_)),
            "{who}: {sent:?}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

/// A subagent claim of the same session: the declared role and an agent id.
fn subagent_of(top: &CallerClaim) -> CallerClaim {
    CallerClaim {
        role: CallerRole::Subagent,
        agent_id: Some(AgentId::new("agent-1")),
        ..top.clone()
    }
}

#[test]
fn subagent_ack_send_accept_leave_invite_and_check_in_are_refused() {
    use crate::protocol::commands::{Accept, Ack, Invite, Leave, Reject, SendMessage};
    let (service, db) = fixture();
    let top = checked(&service, lifecycle(claim(), "top")).context;
    let peer = PeerIdentity::from_kernel(501);
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("thread"),
                claim: top.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    let before = |db: &Connection| {
        db.query_row("SELECT count(*) FROM operations", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let operations = before(&db);
    // The declared-role shape and the bare agent-id shape are both recognized.
    let bare_agent = CallerClaim {
        agent_id: Some(AgentId::new("agent-2")),
        ..top.clone()
    };
    for sub in [subagent_of(&top), bare_agent] {
        let op = |n: &str| OperationId::new(n);
        let commands = vec![
            Command::Ack(Ack {
                messages: vec![MessageId::new("m")],
                operation: op("ack"),
                claim: sub.clone(),
            }),
            Command::AckDisplayed(Ack {
                messages: vec![MessageId::new("m")],
                operation: op("ack-displayed"),
                claim: sub.clone(),
            }),
            Command::SendMessage(SendMessage {
                priority: None,
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                thread: thread.clone(),
                body: "from a subagent".into(),
                invited_recipients: vec![],
                deadline_millis: None,
                operation: op("send"),
                claim: sub.clone(),
                relays_user: false,
                user_intent: None,
                direct_recipient: None,
            }),
            Command::Accept(Accept {
                thread: thread.clone(),
                operation: op("accept"),
                claim: sub.clone(),
            }),
            Command::Reject(Reject {
                thread: thread.clone(),
                invitation: InvitationId::new("inv"),
                reason: "no".into(),
                operation: op("reject"),
                claim: sub.clone(),
            }),
            Command::Leave(Leave {
                thread: thread.clone(),
                operation: op("leave"),
                claim: sub.clone(),
            }),
            Command::Invite(Invite {
                confirmation: None,
                thread: thread.clone(),
                seat: SeatId::new("other"),
                deadline_millis: None,
                operation: op("invite"),
                claim: sub.clone(),
            }),
            lifecycle(sub.clone(), "check-in"),
        ];
        for command in commands {
            let label = format!("{command:?}");
            let error = service.handle(command, peer, &budget()).unwrap_err();
            assert_eq!(error.code, ErrorCode::SubagentReadOnly, "{label}");
            assert_eq!(
                error.detail,
                crate::protocol::authority::SUBAGENT_READ_ONLY_DETAIL
            );
        }
    }
    assert_eq!(before(&db), operations, "a refusal records no decision");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE agent_id<>''",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "a refusal enrolls no subagent binding"
    );
}

#[test]
fn subagent_reads_and_advance_cursors_are_allowed() {
    use crate::protocol::commands::AdvanceCursors;
    let (service, _db) = fixture();
    let top = checked(&service, lifecycle(claim(), "top")).context;
    let peer = PeerIdentity::from_kernel(501);
    let sub = subagent_of(&top);
    let advanced = service
        .handle(
            Command::AdvanceCursors(AdvanceCursors {
                in_turn: false,
                invitations: Vec::new(),
                claim: sub.clone(),
                operation: OperationId::new("advance"),
                displayed: vec![],
            }),
            peer,
            &budget(),
        )
        .unwrap();
    assert!(
        matches!(advanced, CommandResult::CursorsAdvanced(_)),
        "{advanced:?}"
    );
    let read = service
        .handle(
            Command::PendingReceipts(crate::protocol::commands::PendingReceiptsQuery {
                seat: Some(sub.seat.clone()),
                thread: None,
                page: Default::default(),
            }),
            peer,
            &budget(),
        )
        .unwrap();
    assert!(
        matches!(read, CommandResult::PendingReceipts(_)),
        "{read:?}"
    );
}

// ---- Binding events (ht-54r.11): harness start/stop/end events enroll and disconnect ----

fn binding_event(
    claim: CallerClaim,
    kind: crate::protocol::commands::BindingEventKind,
    child_session: Option<&str>,
    operation: &str,
) -> Command {
    Command::BindingEvent(crate::protocol::commands::BindingEvent {
        claim,
        kind,
        child_session: child_session.map(NativeSessionId::new),
        operation: OperationId::new(operation),
    })
}
fn recorded(service: &DomainService, command: Command) -> u32 {
    match service
        .handle(command, PeerIdentity::from_kernel(501), &budget())
        .unwrap()
    {
        CommandResult::BindingEventRecorded { changed } => changed,
        other => panic!("unexpected {other:?}"),
    }
}
/// `(agent_id, role, liveness, disconnect_reason, ended)` of every binding, in ordinal order.
type BindingState = (String, String, String, Option<String>, bool);
fn binding_states(db: &Connection) -> Vec<BindingState> {
    db.prepare("SELECT agent_id,role,liveness,disconnect_reason,ended_at IS NOT NULL FROM occupant_bindings ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn with_fingerprint(claim: CallerClaim) -> CallerClaim {
    CallerClaim {
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
        ..claim
    }
}

#[test]
fn subagent_start_enrolls_and_stop_disconnects() {
    use crate::protocol::commands::BindingEventKind::{SubagentStart, SubagentStop};
    let (service, db) = fixture();
    let top = with_fingerprint(checked(&service, lifecycle(claim(), "top")).context);
    let sub = subagent_of(&top);
    assert_eq!(
        recorded(
            &service,
            binding_event(sub.clone(), SubagentStart, None, "start")
        ),
        1
    );
    let states = binding_states(&db);
    assert_eq!(states.len(), 2, "{states:?}");
    assert_eq!(
        (
            states[1].0.as_str(),
            states[1].1.as_str(),
            states[1].2.as_str()
        ),
        ("agent-1", "subagent", "connected")
    );
    let parent: String = db
        .query_row(
            "SELECT p.binding_id FROM occupant_bindings p WHERE p.role='top_level'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let recorded_parent: String = db
        .query_row(
            "SELECT parent_binding FROM occupant_bindings WHERE role='subagent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(recorded_parent, parent);
    // A replayed start is one decision; a second start of the same agent enrolls nothing.
    assert_eq!(
        recorded(
            &service,
            binding_event(sub.clone(), SubagentStart, None, "start")
        ),
        1
    );
    assert_eq!(
        recorded(
            &service,
            binding_event(sub.clone(), SubagentStart, None, "start-2")
        ),
        0
    );
    assert_eq!(binding_states(&db).len(), 2);
    assert_eq!(
        recorded(
            &service,
            binding_event(sub.clone(), SubagentStop, None, "stop")
        ),
        1
    );
    let states = binding_states(&db);
    assert_eq!(
        (states[1].2.as_str(), states[1].3.as_deref()),
        ("disconnected", Some("liveness_end_event"))
    );
    assert_eq!(states[0].2, "connected", "the parent stays connected");
    // Stopping again changes nothing.
    assert_eq!(
        recorded(&service, binding_event(sub, SubagentStop, None, "stop-2")),
        0
    );
}

#[test]
fn session_end_disconnects_the_root_and_its_subagents() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStart};
    let (service, db) = fixture();
    let top = with_fingerprint(checked(&service, lifecycle(claim(), "top")).context);
    for (n, agent) in ["agent-1", "agent-2"].into_iter().enumerate() {
        let sub = CallerClaim {
            agent_id: Some(AgentId::new(agent)),
            ..subagent_of(&top)
        };
        recorded(
            &service,
            binding_event(sub, SubagentStart, None, &format!("start-{n}")),
        );
    }
    assert_eq!(binding_states(&db).len(), 3);
    assert_eq!(
        recorded(&service, binding_event(top, SessionEnd, None, "end")),
        3
    );
    for state in binding_states(&db) {
        assert_eq!(
            (state.2.as_str(), state.3.as_deref()),
            ("disconnected", Some("liveness_end_event")),
            "{state:?}"
        );
    }
}

#[test]
fn stop_for_an_unknown_binding_is_a_noop() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStop};
    let (service, db) = fixture();
    let top = with_fingerprint(claim());
    assert_eq!(
        recorded(
            &service,
            binding_event(subagent_of(&top), SubagentStop, None, "stop")
        ),
        0
    );
    assert_eq!(
        recorded(&service, binding_event(top, SessionEnd, None, "end")),
        0
    );
    assert_eq!(binding_states(&db), Vec::<BindingState>::new());
}

#[test]
fn end_events_never_end_a_binding() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStart, SubagentStop};
    let (service, db) = fixture();
    let top = with_fingerprint(checked(&service, lifecycle(claim(), "top")).context);
    let sub = subagent_of(&top);
    recorded(
        &service,
        binding_event(sub.clone(), SubagentStart, None, "start"),
    );
    recorded(&service, binding_event(sub, SubagentStop, None, "stop"));
    recorded(&service, binding_event(top, SessionEnd, None, "end"));
    let states = binding_states(&db);
    assert_eq!(states.len(), 2);
    assert!(
        states.iter().all(|state| !state.4),
        "ended_at must stay NULL: {states:?}"
    );
}

#[test]
fn subagent_claims_may_report_binding_events() {
    // A binding event is no accountable seat write, so a subagent claim is not refused
    // (SubagentReadOnly) the way an ACK or send is.
    use crate::protocol::commands::BindingEventKind::SubagentStart;
    let (service, _db) = fixture();
    let top = with_fingerprint(checked(&service, lifecycle(claim(), "top")).context);
    recorded(
        &service,
        binding_event(subagent_of(&top), SubagentStart, None, "start"),
    );
}

fn hermes() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("hermes")
            .expect("hermes is a builtin adapter"),
    )
}

#[test]
fn hermes_child_start_registers_the_child_session_and_its_cli_claim_is_read_only() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStart, SubagentStop};
    let (service, db) = fixture();
    let peer = PeerIdentity::from_kernel(501);
    let parent = with_fingerprint(CallerClaim {
        harness: hermes(),
        native_session: NativeSessionId::new("hermes-parent"),
        ..claim()
    });
    // The child's start enrolls the parent first, then registers the child as its subagent.
    assert_eq!(
        recorded(
            &service,
            binding_event(parent.clone(), SubagentStart, Some("hermes-child"), "start")
        ),
        1
    );
    let rows: Vec<(String, String, String, String)> = db
        .prepare(
            "SELECT native_session,agent_id,role,harness FROM occupant_bindings ORDER BY ordinal",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "hermes-parent".into(),
                "".into(),
                "top_level".into(),
                "hermes".into()
            ),
            (
                "hermes-child".into(),
                "hermes-child".into(),
                "subagent".into(),
                "hermes".into()
            ),
        ]
    );
    // The child's own CLI claim (its session, no agent id) resolves to the subagent binding, so
    // a write is refused as a subagent write and no top-level binding is enrolled for it.
    let child_cli = CallerClaim {
        native_session: NativeSessionId::new("hermes-child"),
        ..parent.clone()
    };
    let error = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "t".into(),
                goal: "g".into(),
                operation: OperationId::new("child-thread"),
                claim: child_cli,
            }),
            peer,
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SubagentReadOnly, "{error:?}");
    assert_eq!(
        binding_states(&db).len(),
        2,
        "the child enrolled no top-level binding"
    );
    // The child's stop disconnects only the child.
    assert_eq!(
        recorded(
            &service,
            binding_event(parent.clone(), SubagentStop, Some("hermes-child"), "stop")
        ),
        1
    );
    let states = binding_states(&db);
    assert_eq!(
        (states[0].2.as_str(), states[1].2.as_str()),
        ("connected", "disconnected")
    );
    assert_eq!(states[1].3.as_deref(), Some("liveness_end_event"));
    // The parent's session end disconnects the parent.
    assert_eq!(
        recorded(&service, binding_event(parent, SessionEnd, None, "end")),
        1
    );
}

/// Kills: a Hermes child's CLI call after `subagent_stop` that reconnects the child (a plain
/// call never reconnects an end-event binding, ht-54r.87), enrolls a new top-level binding for it
/// or disturbs the parent (ht-54r.81, spec 3).
#[test]
fn hermes_registered_child_call_after_subagent_stop_stays_disconnected() {
    use crate::protocol::commands::{
        AdvanceCursors,
        BindingEventKind::{SubagentStart, SubagentStop},
    };
    let (service, db) = fixture();
    let peer = PeerIdentity::from_kernel(501);
    let parent = with_fingerprint(CallerClaim {
        harness: hermes(),
        native_session: NativeSessionId::new("hermes-parent"),
        ..claim()
    });
    recorded(
        &service,
        binding_event(parent.clone(), SubagentStart, Some("hermes-child"), "start"),
    );
    recorded(
        &service,
        binding_event(parent.clone(), SubagentStop, Some("hermes-child"), "stop"),
    );
    let states = binding_states(&db);
    assert_eq!(
        (states[0].2.as_str(), states[1].2.as_str()),
        ("connected", "disconnected")
    );
    // The child's next CLI claim (its session, no agent id, a fingerprint).
    let child_cli = CallerClaim {
        native_session: NativeSessionId::new("hermes-child"),
        ..parent.clone()
    };
    service
        .handle(
            Command::AdvanceCursors(AdvanceCursors {
                in_turn: false,
                invitations: Vec::new(),
                claim: child_cli,
                operation: OperationId::new("child-after-stop"),
                displayed: vec![],
            }),
            peer,
            &budget(),
        )
        .unwrap();
    let states = binding_states(&db);
    assert_eq!(states.len(), 2, "no new binding: {states:?}");
    assert_eq!(
        states[1].1, "subagent",
        "still the read-only subagent binding"
    );
    assert_eq!(states[1].2, "disconnected", "the harness declared it over");
    assert_eq!(states[1].3.as_deref(), Some("liveness_end_event"));
    assert_eq!(states[0].2, "connected", "the parent is untouched");
}

struct AliveTable;
impl crate::ports::ProcessInfo for AliveTable {
    fn process(&self, pid: u32) -> Option<crate::ports::ProcessRecord> {
        let fingerprint = crate::protocol::authority::LivenessFingerprint::TEST;
        (pid == fingerprint.pid).then(|| crate::ports::ProcessRecord {
            pid,
            parent: None,
            start: fingerprint.start,
            argv: Vec::new(),
        })
    }
}

/// One run of the early-child scenario: the parent is enrolled by a lifecycle check-in
/// (`parent_lifecycle`) or by a plain `current` claim, then the child's plain call enrolls a
/// stray top-level row, then `subagent_start` registers the child.
fn hermes_child_early_call_then_subagent_start(parent_lifecycle: bool) {
    use crate::protocol::commands::{AdvanceCursors, BindingEventKind::SubagentStart};
    let (service, db) = fixture();
    let peer = PeerIdentity::from_kernel(501);
    let parent = with_fingerprint(CallerClaim {
        harness: hermes(),
        native_session: NativeSessionId::new("hermes-parent"),
        ..claim()
    });
    if parent_lifecycle {
        checked(&service, lifecycle(parent.clone(), "parent-check-in"));
    } else {
        checked(
            &service,
            Command::CheckIn(CheckIn {
                lineage_parent: None,
                mode: CheckInMode::Current,
                claim: parent.clone(),
                operation: OperationId::new("parent-current"),
            }),
        );
    }
    // The child's plain call, under its own session and the parent's fingerprint, before any
    // `subagent_start` registered that session.
    let child_cli = CallerClaim {
        native_session: NativeSessionId::new("hermes-child"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000002"),
        ..parent.clone()
    };
    let advance = |operation: &str| {
        service
            .handle(
                Command::AdvanceCursors(AdvanceCursors {
                    in_turn: false,
                    invitations: Vec::new(),
                    claim: child_cli.clone(),
                    operation: OperationId::new(operation),
                    displayed: vec![],
                }),
                peer,
                &budget(),
            )
            .unwrap();
    };
    advance("child-early");
    let rows: Vec<(String, String, String, String)> = db
        .prepare(
            "SELECT native_session,role,liveness,observation_provenance FROM occupant_bindings ORDER BY ordinal",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].0, "hermes-parent");
    assert_eq!(
        rows[0].2, "connected",
        "the parent stays connected: {rows:?}"
    );
    assert_eq!(
        (rows[1].0.as_str(), rows[1].1.as_str(), rows[1].3.as_str()),
        ("hermes-child", "top_level", "cooperative_env_session"),
        "{rows:?}"
    );
    // The worker ranks the plain-call child below a lifecycle-claimed parent (it can lose, never
    // win); with a plain-call parent too, nothing claims the process and nothing changes.
    let candidates = crate::store::bindings::liveness_candidates(&db, None).unwrap();
    assert_eq!(candidates.len(), 2);
    let verdicts = crate::service::liveness::classify(&candidates, &AliveTable);
    let child_row = candidates
        .iter()
        .find(|c| c.binding.native_session == "hermes-child")
        .unwrap();
    if parent_lifecycle {
        assert_eq!(
            verdicts.len(),
            1,
            "the child loses to the parent: {verdicts:?}"
        );
        assert_eq!(verdicts[0].binding, child_row.binding.binding_id);
        assert_eq!(
            verdicts[0].reason,
            crate::store::bindings::DisconnectReason::Superseded
        );
    } else {
        assert_eq!(verdicts, vec![], "two plain-only bindings stay as they are");
    }
    // The child's `subagent_start` registers it and converts the early row.
    assert_eq!(
        recorded(
            &service,
            binding_event(parent.clone(), SubagentStart, Some("hermes-child"), "start")
        ),
        1
    );
    let tops: Vec<(String, String, Option<String>)> = db
        .prepare(
            "SELECT native_session,liveness,disconnect_reason FROM occupant_bindings WHERE role='top_level' ORDER BY ordinal",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        tops,
        vec![
            ("hermes-parent".to_owned(), "connected".to_owned(), None),
            (
                "hermes-child".to_owned(),
                "disconnected".to_owned(),
                Some("superseded".to_owned())
            ),
        ]
    );
    // The parent is the unique wake target and the composer wake picks it.
    let seat = parent.seat.clone();
    let targets = crate::store::wake::wake_targets(&db, &seat, true).unwrap();
    assert_eq!(targets.len(), 1, "{targets:?}");
    let parent_id: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session='hermes-parent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let authority = crate::ports::ReservedWakeAuthority::Cooperative {
        terminal: crate::protocol::ids::TerminalId::new("t"),
        incarnation: "i".into(),
        binding_generation: None,
        harness: None,
        targets,
    };
    match authority.composer_choice(&[crate::protocol::authority::LivenessFingerprint::TEST.pid]) {
        crate::ports::ComposerChoice::Binding(target) => {
            assert_eq!(target.binding.as_str(), parent_id);
        }
        other => panic!("expected the parent, got {other:?}"),
    }
    // A later child plain call resolves to the subagent row and reconnects nothing.
    advance("child-late");
    let early: (String, Option<String>) = db
        .query_row(
            "SELECT liveness,disconnect_reason FROM occupant_bindings WHERE native_session='hermes-child' AND role='top_level'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        early,
        ("disconnected".to_owned(), Some("superseded".to_owned()))
    );
    // And the child stays read-only.
    let error = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "t".into(),
                goal: "g".into(),
                operation: OperationId::new("child-thread"),
                claim: child_cli.clone(),
            }),
            peer,
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SubagentReadOnly, "{error:?}");
}

/// Kills: a Hermes child's CLI call that arrives before `subagent_start` enrolls a top-level
/// binding that survives the registration as a second connected wake target with a frozen
/// cursor (ht-54r.79, ht-54r.87), or that supersedes the parent.
#[test]
fn hermes_child_early_call_then_subagent_start_leaves_only_the_parent_connected() {
    hermes_child_early_call_then_subagent_start(true);
    hermes_child_early_call_then_subagent_start(false);
}

/// Kills: read-only decided from the claim's shape alone. A Hermes child's CLI claim is
/// top-level-shaped but resolves to its registered subagent row; a check-in under it must be
/// refused (no availability row, child row untouched) while its own cursor advance still works.
#[test]
fn hermes_child_check_in_is_refused_read_only() {
    use crate::protocol::commands::{AdvanceCursors, BindingEventKind::SubagentStart};
    let (service, db) = fixture();
    let peer = PeerIdentity::from_kernel(501);
    let parent = with_fingerprint(CallerClaim {
        harness: hermes(),
        native_session: NativeSessionId::new("hermes-parent"),
        ..claim()
    });
    assert_eq!(
        recorded(
            &service,
            binding_event(parent.clone(), SubagentStart, Some("hermes-child"), "start")
        ),
        1
    );
    let child_cli = CallerClaim {
        native_session: NativeSessionId::new("hermes-child"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000002"),
        ..parent.clone()
    };
    type Snapshot = (i64, Vec<(String, Option<String>, String)>, i64);
    let snapshot = |db: &Connection| -> Snapshot {
        let availability = db
            .query_row("SELECT COUNT(*) FROM seat_availability", [], |r| r.get(0))
            .unwrap();
        let rows = db
            .prepare(
                "SELECT native_session,CAST(registered_at AS TEXT),observation_provenance \
                 FROM occupant_bindings ORDER BY ordinal",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let cursors = db
            .query_row("SELECT COUNT(*) FROM binding_cursors", [], |r| r.get(0))
            .unwrap();
        (availability, rows, cursors)
    };
    let before = snapshot(&db);
    let error = service
        .handle(
            lifecycle(child_cli.clone(), "child-check-in"),
            peer,
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SubagentReadOnly, "{error:?}");
    assert_eq!(
        snapshot(&db),
        before,
        "the refused check-in changed nothing"
    );
    // Its own stream is still readable: the advance succeeds and moves no other binding.
    let advanced = service
        .handle(
            Command::AdvanceCursors(AdvanceCursors {
                in_turn: false,
                invitations: Vec::new(),
                claim: child_cli,
                operation: OperationId::new("child-advance"),
                displayed: vec![],
            }),
            peer,
            &budget(),
        )
        .unwrap();
    assert!(
        matches!(advanced, CommandResult::CursorsAdvanced(_)),
        "{advanced:?}"
    );
    assert_eq!(snapshot(&db).1, before.1);
}

/// Kills: a binding event whose claim shape contradicts its kind being accepted (a session end
/// carrying an agent id would name a subagent binding; a subagent event naming neither an agent id
/// nor a child session would name nothing; a Hermes child event under a subagent claim has two
/// owners), and an unusable liveness fingerprint.
#[test]
fn binding_event_claim_shapes_are_validated() {
    use crate::protocol::commands::BindingEventKind::{SessionEnd, SubagentStart, SubagentStop};
    let top = with_fingerprint(claim());
    let sub = subagent_of(&top);
    let valid = [
        binding_event(sub.clone(), SubagentStart, None, "a"),
        binding_event(sub.clone(), SubagentStop, None, "b"),
        binding_event(top.clone(), SessionEnd, None, "c"),
        binding_event(top.clone(), SubagentStart, Some("child"), "d"),
        binding_event(top.clone(), SubagentStop, Some("child"), "e"),
    ];
    for command in &valid {
        assert_eq!(command.validate(), Ok(()), "{command:?}");
    }
    let invalid = [
        binding_event(top.clone(), SubagentStart, None, "f"),
        binding_event(top.clone(), SubagentStop, None, "g"),
        binding_event(sub.clone(), SessionEnd, None, "h"),
        binding_event(sub.clone(), SubagentStart, Some("child"), "i"),
        binding_event(top.clone(), SessionEnd, Some("child"), "j"),
        binding_event(
            CallerClaim {
                liveness: Some(crate::protocol::authority::LivenessFingerprint {
                    pid: 0,
                    start: 1,
                }),
                ..sub
            },
            SubagentStart,
            None,
            "k",
        ),
    ];
    for command in &invalid {
        assert!(command.validate().is_err(), "{command:?}");
    }
}

#[test]
fn direct_thread_guarded_runtime_refuses_missing_target_without_effects() {
    use crate::protocol::commands::{EnsureDirectThread, SendMessage};
    let (service, db) = fixture();
    let caller = checked(&service, lifecycle(claim(), "invalid-direct-checkin")).context;
    let tables = [
        "threads",
        "invitations",
        "messages",
        "send_preparations",
        "operations",
    ];
    let before = tables.map(|table| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    });
    let ensure = Command::EnsureDirectThread(EnsureDirectThread {
        creation_policy: None,
        recipient: SeatId::new("peer"),
        operation: OperationId::new("ensure"),
        claim: caller.clone(),
    });
    let send = Command::SendMessage(SendMessage {
        priority: None,
        delivery_mode: crate::protocol::commands::DeliveryMode::Lazy,
        thread: ThreadId::new("t"),
        body: "body".into(),
        direct_recipient: Some(SeatId::new("peer")),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("send"),
        claim: caller.clone(),
        relays_user: false,
        user_intent: None,
    });
    for (command, expected) in [
        (ensure, ErrorCode::TargetUnresolved),
        (send, ErrorCode::NotFound),
    ] {
        assert_eq!(
            service
                .handle(command, PeerIdentity::from_kernel(501), &budget())
                .unwrap_err()
                .code,
            expected
        );
    }
    for (index, table) in tables.into_iter().enumerate() {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, before[index], "{table}");
    }
}

#[test]
fn direct_thread_real_service_ensure_and_lazy_send() {
    use crate::protocol::commands::{DeliveryMode, EnsureDirectThread, SendMessage};
    let (service, db) = fixture();
    let caller = checked(&service, lifecycle(claim(), "direct-checkin")).context;
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('peer','i','resolved','native','peer-pane',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','peer-pane','b',1,1,0,'fresh','term-peer','inc','coherent_enumeration',1)",[]).unwrap();
    let ensure = Command::EnsureDirectThread(EnsureDirectThread {
        creation_policy: None,
        recipient: SeatId::new("peer"),
        operation: OperationId::new("service-direct"),
        claim: caller.clone(),
    });
    let CommandResult::ThreadCreated(thread) = service
        .handle(ensure, PeerIdentity::from_kernel(501), &budget())
        .unwrap()
    else {
        panic!("no direct thread")
    };
    let send = Command::SendMessage(SendMessage {
        priority: None,
        delivery_mode: DeliveryMode::Lazy,
        thread,
        body: "quiet peer".into(),
        direct_recipient: Some(SeatId::new("peer")),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("service-direct-send"),
        claim: caller,
        relays_user: false,
        user_intent: None,
    });
    assert!(matches!(
        service
            .handle(send, PeerIdentity::from_kernel(501), &budget())
            .unwrap(),
        CommandResult::MessageSent(_)
    ));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='peer'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM invitations WHERE state='accepted'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

/// Re-entering foreground admission from the real post-store callback proves
/// the outer cooperative turn has ended. A real clock makes failure bounded
/// even though the store fixture intentionally uses a fixed clock.
struct CompletionHintChannels {
    writer: Arc<crate::service::fair_writer::FairWriter>,
    calls: std::sync::Mutex<Vec<(SeatId, Result<(), ErrorCode>)>>,
    effects: std::sync::atomic::AtomicUsize,
}
impl crate::ports::ModChannels for CompletionHintChannels {
    fn presentation_delivery_may_have_changed(&self, seat: &SeatId) {
        let clock = crate::app::SystemClock::new();
        let callback_budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 200),
            cancellation: Default::default(),
        };
        let admitted = self
            .writer
            .enter_foreground(&callback_budget, &clock)
            .map(drop)
            .map_err(|error| error.code);
        self.calls.lock().unwrap().push((seat.clone(), admitted));
    }
    fn notify(&self, _: &SeatId, _: u64) {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn record_ack(&self, _: &SeatId, _: &BindingId, _: UtcMillis) {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn record_attention_push(&self, _: &SeatId, _: &BindingId, _: UtcMillis) {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn record_presentation_attention_push(
        &self,
        _: &SeatId,
        _: &BindingId,
        _: u64,
        _: &str,
        _: &str,
        _: UtcMillis,
    ) {
        self.effects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
fn completion_hint_channels(service: &mut DomainService) -> Arc<CompletionHintChannels> {
    let channels = Arc::new(CompletionHintChannels {
        writer: service.cooperative_runtime.as_ref().unwrap().1.clone(),
        calls: Default::default(),
        effects: Default::default(),
    });
    service.mod_channels = channels.clone();
    channels
}
fn completion_hint_mail(db: &Connection) {
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('hint-thread','i','topic','goal',0,0,3)", []).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('hint-thread','s','joined')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('hint-thread','s',1,1)", []).unwrap();
    for (index, id) in ["hint-m1", "hint-m2"].into_iter().enumerate() {
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','hint-thread',?2,'ordinary','hello',0,?3)", rusqlite::params![id, index as i64 + 1, index as i64 + 2]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'hint-thread','s','pending',300000)", [id]).unwrap();
    }
    db.execute(
        "UPDATE host_instances SET decision_seq=MAX(decision_seq,3) WHERE id='i'",
        [],
    )
    .unwrap();
}
#[test]
fn q3_completion_hint_ack_and_displayed_release_foreground_writer() {
    use crate::protocol::commands::Ack;
    for displayed in [false, true] {
        let (mut service, db) = fixture();
        let channels = completion_hint_channels(&mut service.value);
        let context = checked(&service, lifecycle(claim(), "hint-ci")).context;
        completion_hint_mail(&db);
        assert!(
            channels.calls.lock().unwrap().is_empty(),
            "check-in is unrelated"
        );
        let invoke = |operation| {
            let ack = Ack {
                messages: vec![MessageId::new("hint-m1")],
                operation: OperationId::new(operation),
                claim: context.clone(),
            };
            service
                .handle(
                    if displayed {
                        Command::AckDisplayed(ack)
                    } else {
                        Command::Ack(ack)
                    },
                    PeerIdentity::from_kernel(501),
                    &budget(),
                )
                .unwrap()
        };
        let CommandResult::Acknowledged(first) = invoke("hint-first") else {
            panic!("ACK result")
        };
        assert_eq!(first.acknowledged, vec![MessageId::new("hint-m1")]);
        assert!(first.already_acknowledged.is_empty());
        let receipt_snapshot = || {
            db.query_row(
            "SELECT json_group_array(json_array(message_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,acked_at,ack_observation,ack_binding_id)) FROM receipts WHERE thread_id='hint-thread'",
            [], |r| r.get::<_, String>(0),
        ).unwrap()
        };
        let committed = receipt_snapshot();
        let operation_snapshot = || {
            db.query_row(
            "SELECT json_array(actor_scope,operation_key,hex(digest),result_json,decided_at) FROM operations WHERE actor_scope='seat:s' AND operation_key='hint-first'",
            [], |r| r.get::<_, String>(0),
        ).unwrap()
        };
        let retained_operation = operation_snapshot();
        assert_eq!(
            invoke("hint-first"),
            CommandResult::Acknowledged(first.clone()),
            "the exact successful operation key replays its original result"
        );
        assert_eq!(
            receipt_snapshot(),
            committed,
            "idempotent replay makes no second receipt transition"
        );
        assert_eq!(
            operation_snapshot(),
            retained_operation,
            "the exact retained operation record is unchanged"
        );
        assert_eq!(
            *channels.calls.lock().unwrap(),
            vec![(context.seat.clone(), Ok(())); 2],
            "a retained successful replay may make a harmless post-writer hint"
        );
        let CommandResult::Acknowledged(repeat) = invoke("hint-already") else {
            panic!("ACK result")
        };
        assert!(repeat.acknowledged.is_empty());
        assert_eq!(repeat.already_acknowledged, vec![MessageId::new("hint-m1")]);
        assert_eq!(
            *channels.calls.lock().unwrap(),
            vec![(context.seat.clone(), Ok(())); 3],
            "fresh and retained canonical results hint the exact seat after releasing the writer"
        );
        assert_eq!(
            channels.effects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a hint provides no wake, ACK health or presentation health evidence"
        );
        assert_eq!(channels.writer.waiting(), (0, 0));
    }
}
#[test]
fn q3_completion_hint_ack_errors_and_rollback_are_inert() {
    use crate::protocol::commands::Ack;
    let (mut service, db) = fixture();
    let channels = completion_hint_channels(&mut service.value);
    let context = checked(&service, lifecycle(claim(), "hint-ci")).context;
    completion_hint_mail(&db);
    let invoke = |messages: Vec<MessageId>, operation: &str, claim| {
        service.handle(
            Command::Ack(Ack {
                messages,
                operation: OperationId::new(operation),
                claim,
            }),
            PeerIdentity::from_kernel(501),
            &budget(),
        )
    };
    assert!(invoke(vec![], "hint-empty", context.clone()).is_err());
    assert!(
        invoke(
            vec![MessageId::new("ghost")],
            "hint-unknown",
            context.clone()
        )
        .is_err()
    );
    let mut stale = context.clone();
    stale.binding_generation += 1;
    assert!(invoke(vec![MessageId::new("hint-m1")], "hint-stale", stale).is_err());
    db.execute_batch("CREATE TRIGGER hint_abort_second_receipt BEFORE UPDATE ON receipts WHEN OLD.message_id='hint-m2' AND NEW.state='acked' BEGIN SELECT RAISE(ABORT,'hint rollback'); END;").unwrap();
    let rollback = invoke(
        vec![MessageId::new("hint-m1"), MessageId::new("hint-m2")],
        "hint-rollback",
        context,
    )
    .unwrap_err();
    assert_eq!(rollback.code, ErrorCode::Conflict);
    assert!(
        rollback.detail.contains("hint rollback"),
        "the canonical receipt trigger caused rollback: {rollback:?}"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM receipts WHERE thread_id='hint-thread' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "the first receipt write rolls back with the failed second one"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE operation_key='hint-rollback'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        channels.calls.lock().unwrap().is_empty(),
        "no committed completion means no callback"
    );
    assert_eq!(
        channels.effects.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(channels.writer.waiting(), (0, 0));
}
