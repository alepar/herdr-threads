use super::*;
use crate::{
    ports::{
        ClosureEvidence, CooperativePermitRequest, OperationReadScope, ReadContext,
        RegisterAvailableRequest, ServiceDecisionGuard, ServiceWriteTransactionProof, StorePort,
        WorkAdmission,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, ObligationRef},
        commands::{
            BodyReadRequest, CheckIn, CheckInMode, Command, DeliveryInspectQuery, HistoryQuery,
            MessageQuery, PendingReceiptsQuery,
        },
        ids::{ExecutionId, HostBootId, HostTargetId, NativeSessionId, OperationId, ThreadId},
        output::{OutputFormat, OutputSpec},
        pagination::PageRequest,
        results::{CommandResult, ReceiptStatus},
        service::{
            EnsureManagedThread, EventAuthor, InvitationConstraint, NotificationSeverity,
            ServiceInvite, ServiceNotify, ServiceOperation, ServiceThreadMutation,
        },
        time::{Cancellation, Clock, MonoInstant},
    },
    store::{
        SqliteStore, StoreSettings, effective::EffectiveReceiptState, receipts, service_substrate,
    },
    test_support::history,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

/// A service gate whose authority can be revoked between steps.
struct Gate {
    author: ServiceAuthorId,
    revoked: Arc<AtomicBool>,
}
struct Guard(ServiceAuthorId);
impl ServiceDecisionGuard for Guard {
    fn author(&self) -> &ServiceAuthorId {
        &self.0
    }
}
impl ServiceAuthorityGate for Gate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<ServiceConnectionAuthority, ApiError> {
        Ok(ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _: &ServiceWriteTransactionProof,
        connection: &ServiceConnectionAuthority,
    ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
        if self.revoked.load(Ordering::SeqCst) || connection.author() != &self.author {
            return Err(api_error(
                ErrorCode::StaleServiceGeneration,
                "service generation revoked",
            ));
        }
        Ok(Box::new(Guard(self.author.clone())))
    }
    fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
        true
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

struct Fixture {
    store: SqliteStore,
    connection: ServiceConnectionAuthority,
    gate: Gate,
    author: ServiceAuthorId,
    clock: Arc<TestClock>,
    path: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.path.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A resolved seat with a registered cooperative binding and a confirmed
/// target observation: available, so receipt timers start at the send.
fn seed_seat(db: &Connection, seat: &str) {
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [seat]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) VALUES (?1,1,1,?1,'b',1,'codex','n',?2,'cooperative_top_level',0,'term-'||?1,'inc',0)", params![seat, history::fixture_execution(seat)]).unwrap();
}

fn seed_joined_in(db: &Connection, thread: &str, seat: &str) {
    db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES (?1,?2,'joined','joined',1)", params![thread, seat]).unwrap();
    db.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)",
        params![thread, seat],
    )
    .unwrap();
    db.execute(
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id=?1",
        [thread],
    )
    .unwrap();
}

fn count(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

impl Fixture {
    fn new() -> Self {
        Self::with_settings(StoreSettings::default())
    }

    fn with_settings(settings: StoreSettings) -> Self {
        let path = std::env::temp_dir().join(format!("service-send-{}.db", uuid::Uuid::new_v4()));
        let clock = Arc::new(TestClock(AtomicI64::new(100)));
        let context = StoreContext::new(path.clone(), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
            [],
        )
        .unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let author = service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
        tx.commit().unwrap();
        drop(db);
        let store = SqliteStore::new(context, "i", settings).unwrap();
        let connection =
            ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author.clone());
        let fixture = Self {
            store,
            connection,
            gate: Gate {
                author: author.clone(),
                revoked: Arc::new(AtomicBool::new(false)),
            },
            author,
            clock,
            path,
        };
        fixture
            .op(ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: ThreadId::new("t"),
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("ensure-t"),
            }))
            .unwrap();
        fixture
    }

    /// A fixture whose managed thread `t` has the given joined seats.
    fn with_joined(seats: &[&str]) -> Self {
        let fixture = Self::new();
        fixture.seed_joined(seats);
        fixture
    }

    fn db(&self) -> Connection {
        self.store.context.open_writer().unwrap()
    }

    fn seed_joined(&self, seats: &[&str]) {
        let db = self.db();
        for seat in seats {
            seed_seat(&db, seat);
            seed_joined_in(&db, "t", seat);
        }
    }

    fn op(&self, operation: ServiceOperation) -> Result<ServiceResult, ApiError> {
        StorePort::service_operation(
            &self.store,
            operation,
            &self.connection,
            &self.gate,
            &budget(),
            None,
        )
    }

    fn request(
        &self,
        key: &str,
        body: &str,
        recipients: &[&str],
        deadline: Option<u64>,
    ) -> ServiceSend {
        ServiceSend {
            thread: ThreadId::new("t"),
            body: body.into(),
            recipients: recipients.iter().map(|s| SeatId::new(*s)).collect(),
            deadline_millis: deadline,
            operation: OperationId::new(key),
        }
    }

    fn send(
        &self,
        key: &str,
        body: &str,
        recipients: &[&str],
        deadline: Option<u64>,
    ) -> Result<ServiceResult, ApiError> {
        self.op(ServiceOperation::Send(
            self.request(key, body, recipients, deadline),
        ))
    }

    fn sent(&self, key: &str, body: &str, recipients: &[&str]) -> ServiceMessageSent {
        let ServiceResult::MessageSent(sent) = self.send(key, body, recipients, None).unwrap()
        else {
            panic!("not a message-sent result")
        };
        sent
    }

    fn read(&self, command: Command) -> CommandResult {
        let read = ReadContext {
            instance: "i".into(),
            // Text selects no `--json` continuation flag, like a service summary.
            output: OutputSpec {
                format: OutputFormat::Text,
                ..OutputSpec::default()
            },
            operation_scope: None::<OperationReadScope>,
        };
        StorePort::query(&self.store, &command, &read, &budget()).unwrap()
    }

    /// Runs the message's `send_attention` job to completion.
    fn drain(&self, message: &MessageId) {
        let job = format!("work:send:{}", message.as_str());
        let mut db = self.db();
        loop {
            let status: String = db
                .query_row("SELECT status FROM work_jobs WHERE id=?1", [&job], |r| {
                    r.get(0)
                })
                .unwrap();
            if status == "complete" {
                return;
            }
            materialization::advance_work(
                &mut db,
                &job,
                DurableWorkAdmission::new(16).unwrap(),
                &budget(),
                self.store.context.clock(),
            )
            .unwrap();
        }
    }
}

#[test]
fn service_send_writes_programmatic_ordinary_row() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let sent = f.sent("k1", "hello there", &[]);
    let id = sent.summary.message.clone();
    let db = f.db();
    let row: (String, String, String, Option<String>, String, Option<String>) = db
        .query_row(
            "SELECT kind,author_kind,author_service_id,actor_seat_id,actor_label,native_observation FROM messages WHERE id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "ordinary".into(),
            "programmatic".into(),
            f.author.as_str().into(),
            None,
            "herdr-graph".into(),
            None
        )
    );
    assert_eq!(sent.author, f.author);
    assert_eq!(sent.recipient_count, 2);
    assert_eq!(
        sent.receipt_duration_millis,
        u64::try_from(messages::DEFAULT_RECEIPT_MILLIS).unwrap()
    );
    assert_eq!(
        sent.summary.event_author,
        Some(EventAuthor::Programmatic(f.author.clone()))
    );
    let CommandResult::History(page) = f.read(Command::History(HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
        full_bodies: false,
    })) else {
        panic!("not a history page")
    };
    let native = page
        .items
        .iter()
        .find(|item| item.message == id)
        .expect("native history lists the message");
    assert_eq!(&sent.summary, native);
    let explicit = f.send("k2", "second", &["s1"], Some(777)).unwrap();
    let ServiceResult::MessageSent(explicit) = explicit else {
        panic!()
    };
    assert_eq!(explicit.receipt_duration_millis, 777);
    // An explicit recipient that is also joined is one obligation.
    assert_eq!(explicit.recipient_count, 2);
}

#[test]
fn obligations_are_joined_snapshot_plus_explicit_invitees() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    {
        let db = f.db();
        for seat in ["s3", "s4", "s5"] {
            seed_seat(&db, seat);
        }
        seed_joined_in(&db, "t", "s5");
        db.execute(
            "UPDATE memberships SET state='left',left_at=2 WHERE seat_id='s5'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE membership_intervals SET left_seq=2 WHERE seat_id='s5'",
            [],
        )
        .unwrap();
    }
    for (seat, constraint, key) in [
        ("s3", InvitationConstraint::Required, "inv-s3"),
        ("s4", InvitationConstraint::Ordinary, "inv-s4"),
    ] {
        f.op(ServiceOperation::Invite(ServiceInvite {
            thread: ThreadId::new("t"),
            seat: SeatId::new(seat),
            constraint,
            deadline_millis: Some(300_000),
            operation: OperationId::new(key),
        }))
        .unwrap();
    }
    let sent = f.sent("k", "to everyone", &["s3", "s4"]);
    assert_eq!(sent.recipient_count, 4);
    let id = sent.summary.message;
    let db = f.db();
    let mut prepared: Vec<String> = db
        .prepare("SELECT pr.seat_id FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 ORDER BY pr.seat_id")
        .unwrap()
        .query_map([id.as_str()], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    prepared.sort();
    assert_eq!(prepared, ["s1", "s2", "s3", "s4"]);
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT recipient_count FROM send_manifests WHERE message_id='{}'",
                id.as_str()
            )
        ),
        4
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM prepared_recipients WHERE seat_id='s5'"
        ),
        0
    );
    drop(db);
    f.drain(&id);
    let db = f.db();
    let pending: Vec<String> = db
        .prepare("SELECT seat_id FROM receipt_state WHERE message_id=?1 AND state='pending' ORDER BY seat_id")
        .unwrap()
        .query_map([id.as_str()], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(pending, ["s1", "s2", "s3", "s4"]);
}

#[test]
fn native_ack_by_exact_id_settles_and_repeat_is_idempotent() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let sent = f.sent("k", "please ack", &[]);
    let id = sent.summary.message;
    f.drain(&id);
    let mut db = f.db();
    let first = history::ack_as(
        &f.store.context,
        &mut db,
        "s1",
        std::slice::from_ref(&id),
        "ack-1",
    )
    .unwrap();
    let CommandResult::Acknowledged(first_ack) = &first else {
        panic!("not an ack result")
    };
    assert_eq!(first_ack.acknowledged, std::slice::from_ref(&id));
    assert!(first_ack.already_acknowledged.is_empty());
    let acked: (String, String, String) = db
        .query_row(
            "SELECT state,ack_actor_seat_id,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='s1'",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((acked.0.as_str(), acked.1.as_str()), ("acked", "s1"));
    let observation: serde_json::Value = serde_json::from_str(&acked.2).unwrap();
    assert_eq!(observation["provenance"], "cooperative_top_level");
    // Exact-key replay returns the stored result and writes nothing more.
    let replay = history::ack_as(
        &f.store.context,
        &mut db,
        "s1",
        std::slice::from_ref(&id),
        "ack-1",
    )
    .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM receipt_state WHERE state='acked'"
        ),
        1
    );
    // A new key finds nothing newly acknowledged.
    let again = history::ack_as(
        &f.store.context,
        &mut db,
        "s1",
        std::slice::from_ref(&id),
        "ack-2",
    )
    .unwrap();
    let CommandResult::Acknowledged(again) = again else {
        panic!("not an ack result")
    };
    assert!(again.acknowledged.is_empty());
    assert_eq!(again.already_acknowledged, std::slice::from_ref(&id));
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM receipt_state WHERE state='acked'"
        ),
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM receipt_state WHERE seat_id='s2' AND state='pending'"
        ),
        1
    );
}

#[test]
fn reads_and_notify_create_no_acks() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let sent = f.sent("k", "read me", &[]);
    let id = sent.summary.message;
    f.drain(&id);
    f.read(Command::History(HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
        full_bodies: false,
    }));
    f.read(Command::DeliveryInspect(DeliveryInspectQuery {
        message: id.clone(),
        page: PageRequest::default(),
    }));
    f.read(Command::Message(MessageQuery {
        message: id.clone(),
        body: BodyReadRequest {
            cursor: None,
            offset: None,
            max_bytes: 4096,
        },
    }));
    f.read(Command::PendingReceipts(PendingReceiptsQuery {
        seat: Some(SeatId::new("s1")),
        thread: None,
        page: PageRequest::default(),
    }));
    f.op(ServiceOperation::Notify(ServiceNotify {
        thread: ThreadId::new("t"),
        severity: NotificationSeverity::Warn,
        event_json: serde_json::json!({"text":"notice"}),
        operation: OperationId::new("notify"),
    }))
    .unwrap();
    // A check-in by a recipient presents the pending item and acknowledges nothing.
    let claim = CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s1"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new(history::fixture_execution("s1")),
        target: HostTargetId::new("s1"),
    };
    let check_in = CheckIn {
        mode: CheckInMode::Current,
        claim: claim.clone(),
        operation: OperationId::new("check-in"),
    };
    let permit = f
        .store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim,
                operation: check_in.operation.clone(),
                obligation: ObligationRef::CheckIn(SeatId::new("s1")),
                payload_hash: schema::canonical_digest(&(
                    "check_in",
                    &check_in.mode,
                    &check_in.claim,
                ))
                .unwrap(),
                check_in_mode: Some(check_in.mode),
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::CheckedIn(checked) = f
        .store
        .register_available(
            RegisterAvailableRequest {
                command: check_in,
                read: ReadContext {
                    instance: "i".into(),
                    output: OutputSpec::default(),
                    operation_scope: None,
                },
                operator: None,
            },
            permit,
            &budget(),
        )
        .unwrap()
    else {
        panic!("not a check-in result")
    };
    assert!(
        checked
            .inbox
            .items
            .iter()
            .any(|item| item.pending_receipts > 0),
        "the check-in presents the pending service message: {checked:?}"
    );
    let db = f.db();
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM receipt_state WHERE state='acked'"
        ),
        0
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM receipt_state WHERE state='pending'"
        ),
        2
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM receipts WHERE state='acked'"),
        0
    );
}

#[test]
fn retirement_settles_service_obligation_to_recipient_retired() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let id = f.sent("k", "retire me", &[]).summary.message;
    f.drain(&id);
    let job = StorePort::begin_retirement(
        &f.store,
        SeatId::new("s1"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s1"),
            generation: 1,
        },
        &budget(),
    )
    .unwrap();
    for _ in 0..20 {
        let progress = StorePort::advance_retirement(
            &f.store,
            job.id.clone(),
            WorkAdmission::Background,
            &budget(),
        )
        .unwrap();
        if progress.complete {
            break;
        }
    }
    let db = f.db();
    let effective = crate::store::effective::effective_receipt(&db, id.as_str(), "s1")
        .unwrap()
        .unwrap();
    assert_eq!(effective.state, EffectiveReceiptState::RecipientRetired);
    let other = crate::store::effective::effective_receipt(&db, id.as_str(), "s2")
        .unwrap()
        .unwrap();
    assert_eq!(other.state, EffectiveReceiptState::Pending);
    drop(db);
    let CommandResult::DeliveryInspect(inspection) =
        f.read(Command::DeliveryInspect(DeliveryInspectQuery {
            message: id,
            page: PageRequest::default(),
        }))
    else {
        panic!("not a delivery inspection")
    };
    let retired = inspection
        .recipients
        .items
        .iter()
        .find(|r| r.seat == SeatId::new("s1"))
        .unwrap();
    assert_eq!(retired.effective_status, ReceiptStatus::Retired);
    assert!(retired.retirement_cutover.is_some());
}

#[test]
fn deadline_and_overdue_warning_use_frozen_duration() {
    let f = Fixture::with_joined(&["s1"]);
    let ServiceResult::MessageSent(sent) = f.send("k", "be quick", &[], Some(50)).unwrap() else {
        panic!()
    };
    assert_eq!(sent.receipt_duration_millis, 50);
    let id = sent.summary.message;
    f.drain(&id);
    let mut db = f.db();
    let (decision_at, deadline): (i64, i64) = db
        .query_row(
            "SELECT m.decision_at,r.deadline_at FROM messages m JOIN receipt_state r ON r.message_id=m.id WHERE m.id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((decision_at, deadline), (100, 150));
    let mut cursor = receipts::ReceiptDueCursor::default();
    // Not yet due: the frozen 50 ms has not elapsed.
    f.clock.0.store(149, Ordering::SeqCst);
    assert_eq!(
        receipts::scan_due(&f.store.context, &mut db, 10, &mut cursor)
            .unwrap()
            .warnings,
        0
    );
    f.clock.0.store(150, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    assert_eq!(
        receipts::scan_due(&f.store.context, &mut db, 10, &mut cursor)
            .unwrap()
            .warnings,
        1
    );
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id='{}'",
                id.as_str()
            )
        ),
        1
    );
}

#[test]
fn send_attention_job_and_pending_receipts_attribution() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let id = f.sent("k", "attention", &[]).summary.message;
    {
        let db = f.db();
        let jobs: Vec<String> = db
            .prepare("SELECT id FROM work_jobs WHERE kind='send_attention'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(jobs, [format!("work:send:{}", id.as_str())]);
    }
    f.drain(&id);
    let CommandResult::PendingReceipts(pending) =
        f.read(Command::PendingReceipts(PendingReceiptsQuery {
            seat: Some(SeatId::new("s1")),
            thread: None,
            page: PageRequest::default(),
        }))
    else {
        panic!("not pending receipts")
    };
    let item = pending
        .items
        .iter()
        .find(|item| item.message == id)
        .expect("pending receipt lists the service message");
    assert_eq!(item.sender, None);
    assert_eq!(
        item.sender_author,
        Some(EventAuthor::Programmatic(f.author.clone()))
    );
}

fn assert_refused(
    f: &Fixture,
    result: Result<ServiceResult, ApiError>,
    code: ErrorCode,
    detail: &str,
    body: &str,
) {
    let error = result.expect_err("send must be refused");
    assert_eq!(error.code, code, "{error:?}");
    assert!(
        error.detail.contains(detail),
        "detail {:?} lacks {detail:?}",
        error.detail
    );
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT count(*) FROM messages WHERE body=?1", [body], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "a refused send must publish nothing"
    );
}

#[test]
fn error_cases_map_to_section_2_codes() {
    let f = Fixture::with_joined(&["s1"]);
    {
        let db = f.db();
        db.execute_batch(
            "INSERT INTO host_instances(id,created_at) VALUES ('j',0);
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('other-instance','j','x','x',0,0);
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('plain','i','x','x',0,0);
",
        )
        .unwrap();
        seed_seat(&db, "retired");
        db.execute(
            "UPDATE seats SET state='retired',retired_at=1,retired_seq=1 WHERE id='retired'",
            [],
        )
        .unwrap();
        seed_seat(&db, "outsider");
    }
    for thread in ["archived", "empty"] {
        f.op(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new(thread),
            topic: "x".into(),
            goal: "x".into(),
            operation: OperationId::new(format!("ensure-{thread}")),
        }))
        .unwrap();
    }
    {
        let db = f.db();
        seed_joined_in(&db, "archived", "s1");
    }
    f.op(ServiceOperation::Archive(ServiceThreadMutation {
        thread: ThreadId::new("archived"),
        operation: OperationId::new("archive-it"),
    }))
    .unwrap();
    let to = |thread: &str, key: &str, body: &str, recipients: &[&str]| {
        let mut request = f.request(key, body, recipients, None);
        request.thread = ThreadId::new(thread);
        f.op(ServiceOperation::Send(request))
    };
    assert_refused(
        &f,
        to("missing", "e1", "b-missing", &[]),
        ErrorCode::NotFound,
        "",
        "b-missing",
    );
    assert_refused(
        &f,
        to("other-instance", "e2", "b-other", &[]),
        ErrorCode::Unauthorized,
        "another instance",
        "b-other",
    );
    assert_refused(
        &f,
        to("plain", "e3", "b-plain", &[]),
        ErrorCode::IncompatibleOwnership,
        "",
        "b-plain",
    );
    // The reserved author is unique per instance, so a thread owned by another
    // author cannot exist; a connection under a different author meets the
    // managed thread as a foreign owner.
    let foreign_author = ServiceAuthorId::new("graph:other");
    let foreign_gate = Gate {
        author: foreign_author.clone(),
        revoked: Arc::new(AtomicBool::new(false)),
    };
    let foreign_connection =
        ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, foreign_author);
    assert_refused(
        &f,
        StorePort::service_operation(
            &f.store,
            ServiceOperation::Send(f.request("e4", "b-foreign", &[], None)),
            &foreign_connection,
            &foreign_gate,
            &budget(),
            None,
        ),
        ErrorCode::IncompatibleOwnership,
        "",
        "b-foreign",
    );
    assert_refused(
        &f,
        to("archived", "e5", "b-archived", &[]),
        ErrorCode::Archived,
        "",
        "b-archived",
    );
    assert_refused(
        &f,
        to("t", "e6", "b-ghost", &["ghost"]),
        ErrorCode::NotFound,
        "recipient seat unknown: ghost",
        "b-ghost",
    );
    assert_refused(
        &f,
        to("t", "e7", "b-retired", &["retired"]),
        ErrorCode::InvalidRequest,
        "recipient retired: retired",
        "b-retired",
    );
    assert_refused(
        &f,
        to("t", "e8", "b-outsider", &["outsider"]),
        ErrorCode::InvalidRequest,
        "recipient not a member or invitee: outsider",
        "b-outsider",
    );
    assert_refused(
        &f,
        to("empty", "e9", "b-empty", &[]),
        ErrorCode::InvalidRequest,
        "no receipt recipients",
        "b-empty",
    );
    // A refused preparation leaves nothing live: the same key retried with a
    // valid audience still sends.
    f.sent("e9-ok", "after refusals", &[]);
    // Key reuse with another payload, and across operation kinds.
    f.sent("reused", "first body", &[]);
    assert_refused(
        &f,
        f.send("reused", "second body", &[], None),
        ErrorCode::OperationPayloadMismatch,
        "",
        "second body",
    );
    f.op(ServiceOperation::Notify(ServiceNotify {
        thread: ThreadId::new("t"),
        severity: NotificationSeverity::Info,
        event_json: serde_json::json!({"text":"n"}),
        operation: OperationId::new("notify-key"),
    }))
    .unwrap();
    assert_refused(
        &f,
        f.send("notify-key", "cross-kind", &[], None),
        ErrorCode::OperationPayloadMismatch,
        "",
        "cross-kind",
    );
    // The recipient order is not part of the replay identity.
    {
        let db = f.db();
        seed_seat(&db, "s2");
        seed_joined_in(&db, "t", "s2");
    }
    let first = f.send("order", "ordered", &["s2", "s1"], None).unwrap();
    assert_eq!(
        f.send("order", "ordered", &["s1", "s2"], None).unwrap(),
        first
    );
    // Body over the instance limit.
    let small = Fixture::with_settings(StoreSettings {
        message_limits: MessageLimits {
            body_bytes: 8,
            ..MessageLimits::default()
        },
        ..StoreSettings::default()
    });
    small.seed_joined(&["s1"]);
    assert_refused(
        &small,
        small.send("big", "123456789", &[], None),
        ErrorCode::InvalidRequest,
        "message body byte limit",
        "123456789",
    );
    small.sent("fits", "12345678", &[]);
}

#[test]
fn archived_between_prepare_and_publish_is_refused() {
    let f = Fixture::with_joined(&["s1"]);
    let request = f.request("k", "late archive", &[], None);
    let limits = MessageLimits::default();
    let mut db = f.db();
    loop {
        match prepare_service_send_step(
            &f.store.context,
            &mut db,
            &f.connection,
            &f.gate,
            &request,
            limits,
            &budget(),
            16,
        )
        .unwrap()
        {
            SendPrepare::Step(SendStep::More { .. }) => {}
            SendPrepare::Step(SendStep::Ready { .. }) => break,
            _ => panic!("unexpected preparation outcome"),
        }
    }
    f.op(ServiceOperation::Archive(ServiceThreadMutation {
        thread: ThreadId::new("t"),
        operation: OperationId::new("archive"),
    }))
    .unwrap();
    let error = publish_service_send(
        &f.store.context,
        &mut db,
        &f.connection,
        &f.gate,
        &request,
        limits,
        &budget(),
    )
    .err()
    .expect("publish must be refused");
    assert_eq!(error.code, ErrorCode::Archived);
    assert_eq!(
        count(&db, "SELECT count(*) FROM messages WHERE kind='ordinary'"),
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM send_manifests"), 0);
}

#[test]
fn replay_after_response_loss_returns_stored_result_without_second_message() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let lost = f.send("k", "once only", &[], None).unwrap();
    // The response was lost; the caller retries with the same key and payload.
    let replayed = f.send("k", "once only", &[], None).unwrap();
    assert_eq!(replayed, lost);
    let db = f.db();
    assert_eq!(
        count(&db, "SELECT count(*) FROM messages WHERE body='once only'"),
        1
    );
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM operations WHERE actor_scope='service:i:{}'",
                f.author.as_str()
            )
        ),
        // The managed-thread ensure and the send.
        2
    );
}

#[test]
fn audience_drift_restarts_then_conflicts_when_exhausted() {
    // One drift between steps: the send succeeds and includes the new seat.
    let f = Fixture::with_joined(&["s1"]);
    {
        let db = f.db();
        seed_seat(&db, "late");
    }
    let request = f.request("drift", "drifting", &[], None);
    let mut changed = false;
    let result = f
        .store
        .service_send_with_observer(
            &request,
            &f.connection,
            &f.gate,
            &budget(),
            |step, context| {
                if !changed && matches!(step, SendStep::Ready { .. }) {
                    let db = context.open_writer().unwrap();
                    seed_joined_in(&db, "t", "late");
                    changed = true;
                }
            },
        )
        .unwrap();
    assert!(changed);
    let ServiceResult::MessageSent(sent) = result else {
        panic!()
    };
    assert_eq!(sent.recipient_count, 2);
    let db = f.db();
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM prepared_recipients WHERE seat_id='late' AND preparation_id IN (SELECT preparation_id FROM send_manifests WHERE message_id='{}')",
                sent.summary.message.as_str()
            )
        ),
        1
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM messages WHERE body='drifting'"),
        1
    );
    drop(db);

    // Drift on every step: the allowance is finite and nothing is published.
    let g = Fixture::with_joined(&["s1"]);
    let request = g.request("always", "never lands", &[], None);
    let mut changes = 0;
    let error = g
        .store
        .service_send_with_observer(
            &request,
            &g.connection,
            &g.gate,
            &budget(),
            |step, context| {
                if matches!(step, SendStep::Ready { .. }) {
                    context
                        .open_writer()
                        .unwrap()
                        .execute(
                            "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
                            [],
                        )
                        .unwrap();
                    changes += 1;
                }
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.detail.contains("restart allowance"), "{error:?}");
    assert_eq!(changes, usize::from(MAX_AUDIENCE_RESTARTS) + 1);
    let db = g.db();
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM messages WHERE body='never lands'"
        ),
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM send_manifests"), 0);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM operations WHERE operation_key='always'"
        ),
        0
    );
}

#[test]
fn revoked_authority_publishes_nothing() {
    let f = Fixture::with_joined(&["s1"]);
    f.gate.revoked.store(true, Ordering::SeqCst);
    let error = f.send("k", "revoked", &[], None).unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleServiceGeneration);
    let db = f.db();
    for table in ["send_preparations", "send_manifests"] {
        assert_eq!(
            count(&db, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        count(&db, "SELECT count(*) FROM messages WHERE body='revoked'"),
        0
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM operations WHERE operation_key='k'"
        ),
        0
    );
}

#[test]
fn revocation_between_prepare_and_publish_publishes_nothing() {
    let f = Fixture::with_joined(&["s1"]);
    let request = f.request("k", "revoked late", &[], None);
    let error = f
        .store
        .service_send_with_observer(&request, &f.connection, &f.gate, &budget(), |step, _| {
            if matches!(step, SendStep::Ready { .. }) {
                f.gate.revoked.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleServiceGeneration);
    let db = f.db();
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM messages WHERE body='revoked late'"
        ),
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM send_manifests"), 0);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM operations WHERE operation_key='k'"
        ),
        0
    );
}

#[test]
fn audience_larger_than_one_quantum() {
    let f = Fixture::new();
    let names: Vec<String> = (0..40).map(|n| format!("m{n:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    f.seed_joined(&refs);
    let request = f.request("big", "to forty", &[], None);
    let mut more = 0;
    let result = f
        .store
        .service_send_with_observer(&request, &f.connection, &f.gate, &budget(), |step, _| {
            if matches!(step, SendStep::More { .. }) {
                more += 1;
            }
        })
        .unwrap();
    let ServiceResult::MessageSent(sent) = result else {
        panic!()
    };
    assert_eq!(sent.recipient_count, 40);
    assert!(more >= 1, "40 seats exceed one 16-unit quantum");
    let id = sent.summary.message;
    f.drain(&id);
    let db = f.db();
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT count(*) FROM receipt_state WHERE message_id='{}' AND state='pending'",
                id.as_str()
            )
        ),
        40
    );
}

#[test]
fn service_author_never_in_receipt_tables() {
    let f = Fixture::with_joined(&["s1", "s2"]);
    let id = f.sent("k", "never a recipient", &[]).summary.message;
    f.drain(&id);
    let mut db = f.db();
    history::ack_as(
        &f.store.context,
        &mut db,
        "s1",
        std::slice::from_ref(&id),
        "ack",
    )
    .unwrap();
    for table in ["receipts", "receipt_state", "prepared_recipients"] {
        assert_eq!(
            db.query_row(
                &format!("SELECT count(*) FROM {table} WHERE seat_id=?1"),
                [f.author.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM receipt_state WHERE ack_actor_seat_id=?1",
            [f.author.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM seats WHERE id=?1",
            [f.author.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn service_preparation_retention_preserves_resume_and_rebuild_fences() {
    let f = Fixture::with_joined(&["s1", "s2", "s3"]);
    let request = f.request("retention", "body", &[], None);
    let step = |db: &mut Connection| {
        prepare_service_send_step(
            &f.store.context,
            db,
            &f.connection,
            &f.gate,
            &request,
            MessageLimits::default(),
            &budget(),
            1,
        )
        .unwrap()
    };
    let mut db = f.db();
    let SendPrepare::Step(SendStep::More {
        preparation_id: original,
        ..
    }) = step(&mut db)
    else {
        panic!("must be partial");
    };
    assert_eq!(
        db.query_row(
            "SELECT prepared_at FROM send_preparations WHERE id=?1",
            [&original],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100
    );
    f.clock.0.store(200, Ordering::SeqCst);
    let SendPrepare::Step(SendStep::More { preparation_id, .. }) = step(&mut db) else {
        panic!("must resume");
    };
    assert_eq!(preparation_id, original);
    assert_eq!(
        db.query_row(
            "SELECT prepared_at FROM send_preparations WHERE id=?1",
            [&original],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        200
    );
    drop(db);
    f.store.prune_retention(&budget()).unwrap();
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT status FROM send_preparations", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "building"
    );
    drop(db);
    f.clock.0.store(
        200 + crate::store::retention::PREPARATION_RETENTION_MS,
        Ordering::SeqCst,
    );
    assert_eq!(f.store.prune_retention(&budget()).unwrap().preparations, 1);
    let mut db = f.db();
    assert!(matches!(step(&mut db), SendPrepare::CleanupPending { .. }));
    let changed = f.request("retention", "different", &[], None);
    assert_eq!(
        prepare_service_send_step(
            &f.store.context,
            &mut db,
            &f.connection,
            &f.gate,
            &changed,
            MessageLimits::default(),
            &budget(),
            1
        )
        .err()
        .expect("changed payload must fail")
        .code,
        ErrorCode::OperationPayloadMismatch
    );
    let job = format!("work:cleanup:{original}");
    loop {
        let progress = materialization::advance_work(
            &mut db,
            &job,
            DurableWorkAdmission::new(16).unwrap(),
            &budget(),
            f.store.context.clock(),
        )
        .unwrap();
        if !progress.has_more {
            break;
        }
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM prepared_recipients WHERE preparation_id=?1",
            [&original],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let SendPrepare::Step(SendStep::More {
        preparation_id: rebuilt,
        ..
    }) = step(&mut db)
    else {
        panic!("must rebuild");
    };
    assert_ne!(rebuilt, original);
    assert_eq!(
        db.query_row("SELECT status FROM work_jobs WHERE id=?1", [&job], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "complete"
    );
    drop(db);
    let published = f.sent("retention", "body", &[]);
    assert!(!published.summary.message.as_str().is_empty());
    f.clock.0.fetch_add(
        2 * crate::store::retention::PREPARATION_RETENTION_MS,
        Ordering::SeqCst,
    );
    assert_eq!(f.store.prune_retention(&budget()).unwrap().preparations, 0);
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT prepared_at FROM send_preparations", [], |r| r
            .get::<_, Option<i64>>(0))
            .unwrap(),
        None
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
