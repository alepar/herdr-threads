//! Service `History` and `Receipts` reads (ht-5nb.3): reader path, native
//! pages and cursors, author scoping.
use super::*;
use crate::{
    ports::{
        ClosureEvidence, DurableWorkAdmission, ServiceAuthorityGate, ServiceConnectionAuthority,
        StorePort, WorkAdmission,
    },
    protocol::{
        commands::{Command, DeliveryInspectQuery, HistoryQuery, HistoryRange},
        ids::*,
        pagination::{Page, PageRequest},
        results::{CommandResult, DeliveryInspection, MessageKind, MessageSummary, ReceiptStatus},
        service::*,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::fair_writer::FairWriter,
    store::{connection::StoreContext, queries, schema, service_substrate},
    test_support::history,
};
use rusqlite::Connection;
use sha2::Digest;
use std::{path::PathBuf, sync::Arc};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

struct ReadGate(ServiceAuthorId);
struct ReadGuard(ServiceAuthorId);
impl crate::ports::ServiceDecisionGuard for ReadGuard {
    fn author(&self) -> &ServiceAuthorId {
        &self.0
    }
}
impl ServiceAuthorityGate for ReadGate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<ServiceConnectionAuthority, crate::protocol::results::ApiError> {
        Ok(ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _: &crate::ports::ServiceWriteTransactionProof,
        connection: &ServiceConnectionAuthority,
    ) -> Result<Box<dyn crate::ports::ServiceDecisionGuard + 'a>, crate::protocol::results::ApiError>
    {
        if connection.author() != &self.0 {
            return Err(api_error(ErrorCode::Unauthorized, "wrong service author"));
        }
        Ok(Box::new(ReadGuard(self.0.clone())))
    }
    fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
        true
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(2_000),
        cancellation: Cancellation::default(),
    }
}

struct Fixture {
    store: SqliteStore,
    conn: Connection,
    path: PathBuf,
    author: ServiceAuthorId,
    gate: ReadGate,
    connection: ServiceConnectionAuthority,
    thread: ThreadId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Instance `i` with resolved seats `s1`, `s2`, `s3`; `s1` and `s2` are joined
/// to the service-managed thread `t`, `s3` is not.
fn fixture() -> Fixture {
    let path =
        std::env::temp_dir().join(format!("herdr-service-reads-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    for seat in ["s1", "s2", "s3"] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [seat]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
    }
    let author = {
        let tx = conn.transaction().unwrap();
        let author = service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
        tx.commit().unwrap();
        author
    };
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let gate = ReadGate(author.clone());
    let connection = gate.register("i", "b", author.clone()).unwrap();
    let thread = ThreadId::new("t");
    store
        .service_operation(
            ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: thread.clone(),
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("ensure"),
            }),
            &connection,
            &gate,
            &budget(),
            None,
        )
        .unwrap();
    {
        let tx = conn.transaction().unwrap();
        for seat in ["s1", "s2"] {
            let joined_seq = schema::next_decision_seq(&tx, "i").unwrap();
            tx.execute(
                "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t',?1,'joined',100)",
                [seat],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,?2)",
                rusqlite::params![seat, joined_seq as i64],
            )
            .unwrap();
        }
        schema::bump_membership_revision(&tx, &thread).unwrap();
        tx.commit().unwrap();
    }
    Fixture {
        store,
        conn,
        path,
        author,
        gate,
        connection,
        thread,
    }
}

impl Fixture {
    fn context(&self) -> &StoreContext {
        &self.store.context
    }

    /// A second handle on the same database for the test-support writers.
    fn ctx(&self) -> StoreContext {
        StoreContext::new(self.path.clone(), Arc::new(FixedClock))
    }

    /// One service-authored, receipt-bearing ordinary message, written as
    /// `messages::publish_send` writes it, then projected by the send worker.
    fn seed_service_send(
        &mut self,
        author: &ServiceAuthorId,
        instance: &str,
        key: &str,
        recipients: &[&str],
        duration_ms: i64,
    ) -> MessageId {
        let thread = self.thread.as_str().to_owned();
        let tx = self.conn.transaction().unwrap();
        let prep = format!("prep-{key}");
        let message = format!("msg-{key}");
        let (high_water, membership, timeline): (i64, i64, i64) = tx
            .query_row(
                "SELECT COALESCE((SELECT MAX(ordinal) FROM membership_intervals WHERE thread_id=t.id),0),t.membership_revision,t.timeline_revision FROM threads t WHERE t.id=?1",
                [&thread],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let (lifecycle, eligibility, config): (i64, i64, i64) = tx
            .query_row(
                "SELECT lifecycle_revision,send_eligibility_revision,duration_config_revision FROM host_instances WHERE id='i'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let count = recipients.len() as i64;
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_cursor,recipient_count,warning_count,status) VALUES (?1,'i',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?11,?12,0,'sealed')",
            rusqlite::params![prep, format!("service:i:{}", author.as_str()), key, sha2::Sha256::digest(key.as_bytes()).as_slice(), thread, membership, lifecycle, eligibility, timeline, config, high_water, count]).unwrap();
        for (ordinal, seat) in recipients.iter().enumerate() {
            tx.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES (?1,?2,?3,?4,?5,1,NULL)",
                rusqlite::params![prep, thread, seat, ordinal as i64 + 1, duration_ms]).unwrap();
        }
        let decision_seq = schema::next_decision_seq(&tx, "i").unwrap();
        let base: i64 = tx
            .query_row(
                "SELECT next_sequence FROM threads WHERE id=?1",
                [&thread],
                |r| r.get(0),
            )
            .unwrap();
        tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,actor_label,body,decision_at,decision_seq,author_kind,author_service_id) VALUES (?1,?2,?3,?4,'ordinary',NULL,'herdr-graph',?5,100,?6,'programmatic',?7)",
            rusqlite::params![message, instance, thread, base, format!("service message {key}"), decision_seq as i64, author.as_str()]).unwrap();
        tx.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES (?1,?2,?3,?4,?5,100,?6,?7,?8,0)",
            rusqlite::params![prep, message, instance, thread, decision_seq as i64, base, high_water, count]).unwrap();
        tx.execute(
            "UPDATE threads SET next_sequence=?1,updated_at=100 WHERE id=?2",
            rusqlite::params![base + 1, thread],
        )
        .unwrap();
        schema::bump_timeline_revision(&tx, &self.thread).unwrap();
        tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'send_attention',?2,?3)", rusqlite::params![format!("work:send:{message}"), prep, count + 1]).unwrap();
        tx.commit().unwrap();
        let job = format!("work:send:{message}");
        for _ in 0..50 {
            let outstanding: bool = self
                .conn
                .query_row(
                    "SELECT status IN ('pending','failed') FROM work_jobs WHERE id=?1",
                    [&job],
                    |r| r.get(0),
                )
                .unwrap();
            if !outstanding {
                break;
            }
            crate::store::materialization::advance_work(
                &mut self.conn,
                &job,
                DurableWorkAdmission { max_units: 16 },
                &CallBudget {
                    deadline: MonoInstant(u64::MAX),
                    cancellation: Cancellation::default(),
                },
                self.store.context.clock(),
            )
            .unwrap();
        }
        MessageId::new(message)
    }

    fn own_send(&mut self, key: &str, recipients: &[&str]) -> MessageId {
        let author = self.author.clone();
        self.seed_service_send(&author, "i", key, recipients, 60_000)
    }

    fn service_history(
        &self,
        page: PageRequest,
        initial: Option<HistoryRange>,
    ) -> Result<Page<MessageSummary>, crate::protocol::results::ApiError> {
        self.service_history_on(&self.connection, page, initial)
    }

    fn service_history_on(
        &self,
        connection: &ServiceConnectionAuthority,
        page: PageRequest,
        initial: Option<HistoryRange>,
    ) -> Result<Page<MessageSummary>, crate::protocol::results::ApiError> {
        match self.store.service_operation(
            ServiceOperation::History(ServiceHistoryQuery {
                thread: self.thread.clone(),
                page,
                initial,
            }),
            connection,
            &self.gate,
            &budget(),
            None,
        )? {
            ServiceResult::History(page) => Ok(page),
            other => panic!("unexpected result {other:?}"),
        }
    }

    fn native_history(
        &self,
        page: PageRequest,
        initial: Option<HistoryRange>,
    ) -> Result<Page<MessageSummary>, crate::protocol::results::ApiError> {
        self.native_history_of(&self.thread, page, initial)
    }

    fn native_history_of(
        &self,
        thread: &ThreadId,
        page: PageRequest,
        initial: Option<HistoryRange>,
    ) -> Result<Page<MessageSummary>, crate::protocol::results::ApiError> {
        let command = Command::History(HistoryQuery {
            thread: thread.clone(),
            page,
            initial,
            full_bodies: false,
        });
        match queries::query(self.context(), "i", &command, &budget())? {
            CommandResult::History(page) => Ok(page),
            other => panic!("unexpected result {other:?}"),
        }
    }

    fn service_receipts(
        &self,
        message: &MessageId,
        page: PageRequest,
    ) -> Result<DeliveryInspection, crate::protocol::results::ApiError> {
        self.service_receipts_on(&self.connection, message, page)
    }

    fn service_receipts_on(
        &self,
        connection: &ServiceConnectionAuthority,
        message: &MessageId,
        page: PageRequest,
    ) -> Result<DeliveryInspection, crate::protocol::results::ApiError> {
        match self.store.service_operation(
            ServiceOperation::Receipts(ServiceReceiptsQuery {
                message: message.clone(),
                page,
            }),
            connection,
            &self.gate,
            &budget(),
            None,
        )? {
            ServiceResult::Receipts(inspection) => Ok(inspection),
            other => panic!("unexpected result {other:?}"),
        }
    }

    fn native_receipts(&self, message: &MessageId, page: PageRequest) -> DeliveryInspection {
        let command = Command::DeliveryInspect(DeliveryInspectQuery {
            message: message.clone(),
            page,
        });
        match queries::query(self.context(), "i", &command, &budget()).unwrap() {
            CommandResult::DeliveryInspect(inspection) => inspection,
            other => panic!("unexpected result {other:?}"),
        }
    }
}

fn bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn page_of(limit: u16, cursor: Option<String>) -> PageRequest {
    PageRequest {
        cursor,
        limit,
        ..PageRequest::default()
    }
}

fn status_of(inspection: &DeliveryInspection, seat: &str) -> ReceiptStatus {
    inspection
        .recipients
        .items
        .iter()
        .find(|recipient| recipient.seat.as_str() == seat)
        .unwrap_or_else(|| panic!("no recipient {seat}"))
        .status
        .clone()
}

#[test]
fn service_history_after_is_byte_identical_to_native_history() {
    let mut fx = fixture();
    let context = fx.ctx();
    for round in 0..5 {
        history::write_pending_sends(&context, &mut fx.conn, "t", "s1", 2, &format!("nat{round}"))
            .unwrap();
        history::write_programmatic_warnings(
            &context,
            &mut fx.conn,
            "t",
            2,
            &format!("note{round}"),
        )
        .unwrap();
        for n in 0..2 {
            fx.own_send(&format!("svc{round}-{n}"), &["s1", "s2"]);
        }
    }
    let after = Some(HistoryRange::After { sequence: 3 });
    let native_first = fx.native_history(page_of(10, None), after.clone()).unwrap();
    let service_first = fx
        .service_history(page_of(10, None), after.clone())
        .unwrap();
    assert_eq!(
        native_first.items.len(),
        10,
        "the thread must hold more than one page after sequence 3"
    );
    assert!(native_first.has_more);
    assert_eq!(bytes(&service_first), bytes(&native_first));
    let everything = fx
        .native_history(
            page_of(100, None),
            Some(HistoryRange::After { sequence: 0 }),
        )
        .unwrap();
    let native_count = everything
        .items
        .iter()
        .filter(|m| m.author.is_some())
        .count();
    let service_count = everything
        .items
        .iter()
        .filter(|m| m.author.is_none() && m.kind == MessageKind::Ordinary)
        .count();
    let notify_count = everything
        .items
        .iter()
        .filter(|m| m.kind != MessageKind::Ordinary && m.event_author.is_some())
        .count();
    assert_eq!(
        (native_count, service_count, notify_count),
        (10, 10, 10),
        "the thread mixes native, service and notify messages"
    );
    // A native cursor continues on the service read exactly as it does natively.
    let native_next = native_first
        .next_cursor
        .clone()
        .expect("a second page exists");
    let native_second = fx
        .native_history(page_of(10, Some(native_next.clone())), None)
        .unwrap();
    let service_second_from_native = fx
        .service_history(page_of(10, Some(native_next)), None)
        .unwrap();
    assert_eq!(bytes(&service_second_from_native), bytes(&native_second));
    assert_ne!(bytes(&native_second), bytes(&native_first));
    // A service cursor continues on the native read exactly as it does on the service.
    let service_next = service_first
        .next_cursor
        .clone()
        .expect("a second page exists");
    let service_second = fx
        .service_history(page_of(10, Some(service_next.clone())), None)
        .unwrap();
    let native_second_from_service = fx
        .native_history(page_of(10, Some(service_next)), None)
        .unwrap();
    assert_eq!(bytes(&native_second_from_service), bytes(&service_second));
    assert_eq!(bytes(&service_second), bytes(&native_second));
}

#[test]
fn service_history_unknown_thread_and_bad_cursor() {
    let mut fx = fixture();
    let message = fx.own_send("m1", &["s1", "s2"]);
    fx.own_send("m2", &["s1", "s2"]);
    let unknown = ServiceOperation::History(ServiceHistoryQuery {
        thread: ThreadId::new("no-such-thread"),
        page: PageRequest::default(),
        initial: None,
    });
    let error = fx
        .store
        .service_operation(unknown, &fx.connection, &fx.gate, &budget(), None)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    // A cursor that is not even shaped like one fails request validation, with
    // whatever code native gives it.
    let native_garbage = fx
        .native_history(page_of(10, Some("garbage".into())), None)
        .unwrap_err();
    let garbage = fx
        .service_history(page_of(10, Some("garbage".into())), None)
        .unwrap_err();
    assert_eq!(garbage.code, native_garbage.code);
    assert_eq!(garbage.detail, native_garbage.detail);
    // A well-formed History cursor of another thread is an InvalidCursor.
    fx.store
        .service_operation(
            ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: ThreadId::new("t2"),
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("ensure-2"),
            }),
            &fx.connection,
            &fx.gate,
            &budget(),
            None,
        )
        .unwrap();
    let first = fx.service_history(page_of(1, None), None).unwrap();
    let cursor = first
        .next_cursor
        .expect("the thread holds more than one message");
    let other_thread = ServiceOperation::History(ServiceHistoryQuery {
        thread: ThreadId::new("t2"),
        page: page_of(1, Some(cursor.clone())),
        initial: None,
    });
    let mismatched = fx
        .store
        .service_operation(other_thread, &fx.connection, &fx.gate, &budget(), None)
        .unwrap_err();
    assert_eq!(mismatched.code, ErrorCode::InvalidCursor);
    let native_mismatch = fx
        .native_history_of(&ThreadId::new("t2"), page_of(1, Some(cursor)), None)
        .unwrap_err();
    assert_eq!(mismatched.code, native_mismatch.code);
    // A DeliveryInspect cursor on a History read gets whatever native returns.
    let first = fx.native_receipts(&message, page_of(1, None));
    let foreign = first
        .recipients
        .next_cursor
        .expect("two recipients, one per page");
    let native = fx
        .native_history(page_of(10, Some(foreign.clone())), None)
        .unwrap_err();
    let service = fx
        .service_history(page_of(10, Some(foreign)), None)
        .unwrap_err();
    assert_eq!(service.code, native.code);
    assert_eq!(service.detail, native.detail);
}

#[test]
fn service_receipts_equals_native_delivery_inspect_and_shows_states() {
    let mut fx = fixture();
    let context = fx.ctx();
    let message = fx.own_send("m1", &["s1", "s2"]);
    let page = page_of(10, None);

    let pending = fx.service_receipts(&message, page.clone()).unwrap();
    assert_eq!(
        bytes(&pending),
        bytes(&fx.native_receipts(&message, page.clone()))
    );
    assert_eq!(status_of(&pending, "s1"), ReceiptStatus::Pending);
    assert_eq!(status_of(&pending, "s2"), ReceiptStatus::Pending);
    assert_eq!(pending.delivery.committed, 2);
    assert_eq!(pending.delivery.acknowledged, 0);

    history::ack_as(
        &context,
        &mut fx.conn,
        "s1",
        std::slice::from_ref(&message),
        "ack",
    )
    .unwrap();
    let acked = fx.service_receipts(&message, page.clone()).unwrap();
    assert_eq!(
        bytes(&acked),
        bytes(&fx.native_receipts(&message, page.clone()))
    );
    assert_eq!(status_of(&acked, "s1"), ReceiptStatus::Acknowledged);
    assert_eq!(status_of(&acked, "s2"), ReceiptStatus::Pending);
    let provenance = acked
        .recipients
        .items
        .iter()
        .find(|r| r.seat.as_str() == "s1")
        .unwrap()
        .ack_provenance
        .clone()
        .unwrap();
    assert_eq!(provenance.decided_at, UtcMillis(100));
    assert_eq!(acked.delivery.acknowledged, 1);

    let job = StorePort::begin_retirement(
        &fx.store,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
        &budget(),
    )
    .unwrap();
    for _ in 0..20 {
        if StorePort::advance_retirement(
            &fx.store,
            job.id.clone(),
            WorkAdmission::Background,
            &budget(),
        )
        .unwrap()
        .complete
        {
            break;
        }
    }
    let retired = fx.service_receipts(&message, page.clone()).unwrap();
    assert_eq!(bytes(&retired), bytes(&fx.native_receipts(&message, page)));
    assert_eq!(status_of(&retired, "s1"), ReceiptStatus::Acknowledged);
    assert_eq!(status_of(&retired, "s2"), ReceiptStatus::Retired);
    let cutover = retired
        .recipients
        .items
        .iter()
        .find(|r| r.seat.as_str() == "s2")
        .unwrap()
        .retirement_cutover;
    assert_eq!(cutover, Some(UtcMillis(100)));
}

#[test]
fn service_receipts_scoping() {
    let mut fx = fixture();
    let context = fx.ctx();
    let native = history::write_pending_sends(&context, &mut fx.conn, "t", "s1", 1, "nat").unwrap();
    let notify =
        history::write_programmatic_warnings(&context, &mut fx.conn, "t", 1, "note").unwrap();
    let own = fx.own_send("mine", &["s1", "s2"]);
    for id in native.iter().chain(notify.iter()) {
        let error = fx.service_receipts(id, PageRequest::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{id:?}");
        assert_eq!(error.detail, "message is not authored by this service");
    }
    let unknown = fx
        .service_receipts(&MessageId::new("no-such-message"), PageRequest::default())
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::NotFound);
    assert_eq!(unknown.detail, "message not found");
    // The schema allows one reserved author per instance (the message trigger
    // rejects any other), so "a different author" is a connection whose author
    // is not the one that wrote the message.
    let other = ServiceConnectionAuthority::new(
        "i".into(),
        "b".into(),
        1,
        ServiceAuthorId::new("graph:other"),
    );
    let error = fx
        .service_receipts_on(&other, &own, PageRequest::default())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.detail, "message is not authored by this service");
    // The scoped read still serves the service's own message.
    assert_eq!(
        fx.service_receipts(&own, PageRequest::default())
            .unwrap()
            .message
            .message,
        own
    );
}

#[test]
fn service_reads_progress_while_writer_is_held() {
    let mut fx = fixture();
    let message = fx.own_send("m1", &["s1", "s2"]);
    let held_budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let _writer = fx.store.writer(&held_budget).unwrap();
    assert!(
        fx.store.writer.try_lock().is_err(),
        "the fixture must really hold the writer"
    );
    let blocker = fx.context().open_writer().unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let lane = FairWriter::new(4);
    let _turn = lane
        .enter_foreground(&held_budget, fx.context().clock())
        .unwrap();
    let started = std::time::Instant::now();
    for operation in [
        ServiceOperation::History(ServiceHistoryQuery {
            thread: fx.thread.clone(),
            page: PageRequest::default(),
            initial: None,
        }),
        ServiceOperation::Receipts(ServiceReceiptsQuery {
            message: message.clone(),
            page: PageRequest::default(),
        }),
    ] {
        let result =
            fx.store
                .service_operation(operation, &fx.connection, &fx.gate, &budget(), Some(&lane));
        assert!(result.is_ok(), "{result:?}");
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(lane.waiting(), (0, 0));
    blocker.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn service_reads_reject_foreign_instance_connection() {
    let mut fx = fixture();
    let message = fx.own_send("m1", &["s1", "s2"]);
    let foreign = ServiceConnectionAuthority::new("other".into(), "b".into(), 1, fx.author.clone());
    let history = fx
        .service_history_on(&foreign, PageRequest::default(), None)
        .unwrap_err();
    assert_eq!(history.code, ErrorCode::Unauthorized);
    let receipts = fx
        .service_receipts_on(&foreign, &message, PageRequest::default())
        .unwrap_err();
    assert_eq!(receipts.code, ErrorCode::Unauthorized);
}

/// A body search hit on a service-authored message carries the service's
/// actor label, as history does (ht-e3l).
#[test]
fn search_hit_on_a_service_message_keeps_its_actor_label() {
    let mut fx = fixture();
    let sent = fx.own_send("labelled", &["s1"]);
    let command = Command::Search(crate::protocol::commands::SearchQuery {
        literal: "service message labelled".into(),
        thread: Some(fx.thread.clone()),
        page: page_of(10, None),
        max_candidates: 100,
    });
    let CommandResult::Search(result) =
        queries::query(fx.context(), "i", &command, &budget()).unwrap()
    else {
        panic!("wrong result")
    };
    let hits: Vec<_> = result
        .matches
        .items
        .iter()
        .filter_map(|hit| match hit {
            crate::protocol::results::SearchHit::Body(summary) => Some(summary),
            _ => None,
        })
        .collect();
    assert_eq!(hits.len(), 1, "{result:?}");
    assert_eq!(hits[0].message, sent);
    assert_eq!(hits[0].actor_label.as_deref(), Some("herdr-graph"));
}
