//! Send publication by priority (ht-yha.3, spec D3/D9): delivery rows for every audience member that
//! is not an explicit ACK recipient, receipts only for explicit ACK recipients.

use crate::ports::{DurableWorkAdmission, SendPreparationProgress};
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{DeliveryMode, SendMessage},
    ids::*,
    priority::DeliveryPriority,
    results::{CommandResult, ErrorCode},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use crate::store::{
    connection::StoreContext,
    effective::{EffectiveReceipt, EffectiveReceiptState, effective_receipt},
    messages,
};
use crate::test_support::isolation::TestIsolation;
use rusqlite::Connection;
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
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

fn p(value: u8) -> Option<DeliveryPriority> {
    Some(DeliveryPriority::new(value).unwrap())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Agent,
    Human,
    /// An agent seat with no resolved target: the unavailable-recipient warning applies.
    Unavailable,
}

/// Adds a joined member `id` to thread `t`.
fn add_member(conn: &Connection, id: &str, kind: Kind) {
    let target = format!("p{id}");
    let execution = format!("00000000-0000-4000-8000-0000000000{:02x}", id.as_bytes()[0]);
    if kind == Kind::Unavailable {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','unresolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",[id]).unwrap();
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,'codex',?1,?3,'cooperative_top_level',0,?4,'inc')",rusqlite::params![id,target,execution,format!("term-{target}")]).unwrap();
    } else {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?2,1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",rusqlite::params![id,target]).unwrap();
        conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,1,0,'fresh',?2,'inc','coherent_enumeration',1)",rusqlite::params![target,format!("term-{target}")]).unwrap();
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'codex',?1,?3,'cooperative_top_level',0,0,?4,'inc')",rusqlite::params![id,target,execution,format!("term-{target}")]).unwrap();
        if kind == Kind::Human {
            conn.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id=?1",[id]).unwrap();
        }
    }
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
        [id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",
        [id],
    )
    .unwrap();
}

/// Inserts the fixture rows: sender `a` plus `members`, all joined to thread `t`.
fn fixture(conn: &Connection, members: &[(&str, Kind)]) {
    conn.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0)", []).unwrap();
    add_member(conn, "a", Kind::Agent);
    // The sender's own binding is the one the claim names.
    conn.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-0000000000aa',native_session='n' WHERE seat_id='a'",[]).unwrap();
    for (id, kind) in members {
        add_member(conn, id, *kind);
    }
}

/// Sender `a` plus agents `b` and `c` and human `h`, all joined to thread `t`.
fn setup(iso: &TestIsolation, members: &[(&str, Kind)]) -> (StoreContext, Connection) {
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
    let conn = context.open_writer().unwrap();
    fixture(&conn, members);
    (context, conn)
}

fn request() -> SendMessage {
    SendMessage {
        priority: None,
        delivery_mode: DeliveryMode::Ordinary,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op"),
        claim: CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new("a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            target: HostTargetId::new("pa"),
            agent_id: None,
            liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
        },
        relays_user: false,
        user_intent: None,
    }
}

fn permit(request: &SendMessage) -> MutationPermit {
    let digest = crate::store::schema::canonical_digest(&messages::send_payload(request)).unwrap();
    MutationPermit::cooperative(
        request.claim.clone(),
        request.operation.clone(),
        ObligationRef::Control(request.thread.clone()),
        digest,
        MonoInstant(50),
        (1, 0),
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
    )
}

fn prepare(context: &StoreContext, conn: &mut Connection, request: &SendMessage) {
    loop {
        let step = messages::prepare_send_step(
            context,
            conn,
            request,
            messages::MessageLimits::default(),
            &budget(),
            DurableWorkAdmission::new(16).unwrap(),
        )
        .unwrap();
        if matches!(step, SendPreparationProgress::Ready { .. }) {
            break;
        }
    }
}

fn publish(context: &StoreContext, conn: &mut Connection, request: &SendMessage) -> MessageId {
    match messages::publish_send(
        context,
        conn,
        request,
        &mut permit(request),
        &budget(),
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        CommandResult::MessageSent(id) => id,
        other => panic!("unexpected result {other:?}"),
    }
}

fn send(context: &StoreContext, conn: &mut Connection, request: &SendMessage) -> MessageId {
    prepare(context, conn, request);
    publish(context, conn, request)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `(seat, priority)` of every delivery row, ordered by seat.
fn rows(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare("SELECT seat_id,priority FROM lazy_recipients ORDER BY seat_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn receipt(conn: &Connection, id: &MessageId, seat: &str) -> Option<EffectiveReceipt> {
    effective_receipt(conn, id.as_str(), seat).unwrap()
}

fn row(seat: &str, priority: i64) -> (String, i64) {
    (seat.to_owned(), priority)
}

fn manifest(conn: &Connection) -> (i64, i64) {
    conn.query_row(
        "SELECT recipient_count,warning_count FROM send_manifests",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn message_mode(conn: &Connection) -> (String, i64) {
    conn.query_row("SELECT delivery_mode,priority FROM messages", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap()
}

const GROUP: &[(&str, Kind)] = &[("b", Kind::Agent), ("c", Kind::Agent), ("h", Kind::Human)];

// Kills: a priority >= 1 send that still creates receipts or warnings for agents, or skips the
// human seat.
#[test]
fn p1_group_send_rows_for_every_member_including_human() {
    let iso = TestIsolation::new("prio-p1-group");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.priority = p(1);
    let id = send(&context, &mut conn, &req);
    assert_eq!(
        rows(&conn),
        vec![row("a", 1), row("b", 1), row("c", 1), row("h", 1)],
        "rows for the audience plus local author siblings"
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM prepared_recipients"), 0);
    assert_eq!(manifest(&conn), (0, 0));
    assert_eq!(message_mode(&conn), ("lazy".to_owned(), 1));
    assert_eq!(count(&conn, "SELECT count(*) FROM receipts"), 0);
    for seat in ["b", "c", "h"] {
        assert!(receipt(&conn, &id, seat).is_none(), "{seat}");
    }
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'"
        ),
        0
    );
}

// Kills: a priority-0 send without ACK recipients that still owes receipts (every joined agent owed
// an ACK before D9), freezes a deadline, or stages a send_attention job that could go overdue.
#[test]
fn p0_without_ack_rows_only_no_receipt_or_deadline() {
    let iso = TestIsolation::new("prio-p0-rows");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.priority = p(0);
    let id = send(&context, &mut conn, &req);
    assert_eq!(
        rows(&conn),
        vec![row("a", 0), row("b", 0), row("c", 0), row("h", 0)]
    );
    assert_eq!(manifest(&conn), (0, 0));
    for seat in ["b", "c", "h"] {
        assert!(receipt(&conn, &id, seat).is_none(), "{seat}");
    }
    assert_eq!(message_mode(&conn), ("lazy".to_owned(), 0));
    for table in [
        "prepared_recipients",
        "receipts",
        "work_jobs",
        "warning_conditions",
        "prepared_unavailable_warnings",
    ] {
        assert_eq!(
            count(&conn, &format!("SELECT count(*) FROM {table}")),
            0,
            "{table}"
        );
    }
}

// Kills: dropping the unavailable-recipient warning for a priority-0 row-only send (r2), or
// producing it for priority >= 1 (the v47 manifest trigger would abort that publication).
#[test]
fn p0_send_to_unavailable_joined_member_publishes_the_warning() {
    let iso = TestIsolation::new("prio-p0-unavailable");
    let (context, mut conn) = setup(&iso, &[("b", Kind::Unavailable)]);
    let mut req = request();
    req.priority = p(0);
    send(&context, &mut conn, &req);
    assert_eq!(rows(&conn), vec![row("a", 0), row("b", 0)]);
    assert_eq!(manifest(&conn), (0, 1));
    assert_eq!(
        count(&conn, "SELECT count(*) FROM prepared_unavailable_warnings"),
        1
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM prepared_recipients"), 0);

    let iso = TestIsolation::new("prio-p1-unavailable");
    let (context, mut conn) = setup(&iso, &[("b", Kind::Unavailable)]);
    let mut req = request();
    req.priority = p(1);
    send(&context, &mut conn, &req);
    assert_eq!(rows(&conn), vec![row("a", 1), row("b", 1)]);
    assert_eq!(manifest(&conn), (0, 0));
}

// Kills: receipts for non-explicit members of an ACK send, or no receipt for the explicit one.
#[test]
fn ack_send_receipt_for_explicit_rows_for_the_rest() {
    let iso = TestIsolation::new("prio-ack-mixed");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.invited_recipients = vec![SeatId::new("b")];
    let id = send(&context, &mut conn, &req);
    assert_eq!(
        conn.query_row(
            "SELECT seat_id,frozen_duration_ms FROM prepared_recipients",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("b".to_owned(), messages::DEFAULT_RECEIPT_MILLIS)
    );
    let owed = receipt(&conn, &id, "b").expect("the explicit recipient has a receipt");
    assert_eq!(owed.state, EffectiveReceiptState::Pending);
    assert_eq!(
        owed.deadline_at,
        Some(1000 + messages::DEFAULT_RECEIPT_MILLIS)
    );
    for seat in ["c", "h"] {
        assert!(receipt(&conn, &id, seat).is_none(), "{seat}");
    }
    assert_eq!(rows(&conn), vec![row("a", 0), row("c", 0), row("h", 0)]);
    assert_eq!(message_mode(&conn), ("ordinary".to_owned(), 0));
    assert_eq!(manifest(&conn), (1, 0));
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'"
        ),
        1
    );
}

// Kills: a human explicit recipient getting a receipt, or no row at all (r1/r2).
#[test]
fn only_human_explicit_recipient_gets_a_p0_row() {
    let iso = TestIsolation::new("prio-human-explicit");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.invited_recipients = vec![SeatId::new("h")];
    let id = send(&context, &mut conn, &req);
    assert_eq!(
        rows(&conn),
        vec![row("a", 0), row("b", 0), row("c", 0), row("h", 0)]
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM prepared_recipients"), 0);
    for seat in ["b", "c", "h"] {
        assert!(receipt(&conn, &id, seat).is_none(), "{seat}");
    }
    assert_eq!(message_mode(&conn), ("ordinary".to_owned(), 0));
}

// Kills: any of the four refusals missing, or one that fires only after a preparation row exists.
#[test]
fn refusals() {
    let iso = TestIsolation::new("prio-refusals");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut ack_p2 = request();
    ack_p2.invited_recipients = vec![SeatId::new("b")];
    ack_p2.priority = p(2);
    let mut deadline_only = request();
    deadline_only.deadline_millis = Some(5000);
    let mut nudge_deadline = request();
    nudge_deadline.delivery_mode = DeliveryMode::Ordinary;
    nudge_deadline.deadline_millis = Some(5000);
    let mut lazy_p0 = request();
    lazy_p0.delivery_mode = DeliveryMode::Lazy;
    lazy_p0.priority = p(0);
    let mut lazy_ack = request();
    lazy_ack.delivery_mode = DeliveryMode::Lazy;
    lazy_ack.invited_recipients = vec![SeatId::new("b")];
    for (case, request, detail) in [
        ("ack+p2", &ack_p2, "--require-ack implies priority 0"),
        (
            "deadline",
            &deadline_only,
            "a deadline requires --require-ack recipients",
        ),
        (
            "nudge deadline",
            &nudge_deadline,
            "a deadline requires --require-ack recipients",
        ),
        (
            "lazy+p0",
            &lazy_p0,
            "lazy delivery mode conflicts with priority 0",
        ),
        (
            "lazy ack",
            &lazy_ack,
            "lazy messages cannot request receipts or deadlines",
        ),
    ] {
        let error = messages::prepare_send_step(
            &context,
            &mut conn,
            request,
            messages::MessageLimits::default(),
            &budget(),
            DurableWorkAdmission::new(16).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{case}");
        assert_eq!(error.detail, detail, "{case}");
        let error = messages::publish_send(
            &context,
            &mut conn,
            request,
            &mut permit(request),
            &budget(),
            || messages::MAX_BODY_BYTES,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{case} publish");
        assert_eq!(
            count(&conn, "SELECT count(*) FROM send_preparations"),
            0,
            "{case}"
        );
        assert!(
            crate::protocol::commands::Command::SendMessage(request.clone())
                .validate()
                .is_err(),
            "{case}: the client-side validator agrees"
        );
    }
    // Out of range is refused by the type before the daemon sees it.
    let mut wire = serde_json::to_value(request()).unwrap();
    wire["priority"] = serde_json::json!(10);
    assert!(serde_json::from_value::<SendMessage>(wire).is_err());
}

// Kills: a digest that changes for a request that carries no priority (an old client's journaled
// intent would no longer match its prepared row), and a wrong legacy mapping.
#[test]
fn old_client_shapes_map_and_keep_their_digest() {
    let literal = |mode: Option<&str>| {
        let mut payload = serde_json::json!({"kind":"send_message","thread":"t","body":"hello",
            "invited_recipients":[],"deadline_millis":null,"claim":request().claim});
        if let Some(mode) = mode {
            payload["delivery_mode"] = serde_json::json!(mode);
        }
        payload
    };
    let ordinary = request();
    let mut lazy = request();
    lazy.delivery_mode = DeliveryMode::Lazy;
    assert_eq!(messages::send_payload(&ordinary), literal(None));
    assert_eq!(messages::send_payload(&lazy), literal(Some("lazy")));
    assert_eq!(
        crate::store::schema::canonical_digest(&messages::send_payload(&ordinary)).unwrap(),
        crate::store::schema::canonical_digest(&literal(None)).unwrap()
    );
    let mut with_priority = request();
    with_priority.priority = p(0);
    assert_eq!(messages::send_payload(&with_priority)["priority"], 0);

    let iso = TestIsolation::new("prio-old-client");
    let (context, mut conn) = setup(&iso, GROUP);
    send(&context, &mut conn, &ordinary);
    assert_eq!(
        rows(&conn),
        vec![row("a", 0), row("b", 0), row("c", 0), row("h", 0)]
    );
    assert_eq!(message_mode(&conn), ("lazy".to_owned(), 0));
    let iso = TestIsolation::new("prio-old-client-lazy");
    let (context, mut conn) = setup(&iso, GROUP);
    send(&context, &mut conn, &lazy);
    assert_eq!(
        rows(&conn),
        vec![row("a", 1), row("b", 1), row("c", 1), row("h", 1)]
    );
    assert_eq!(message_mode(&conn), ("lazy".to_owned(), 1));
}

// Kills: a retry that re-derives its identity from the new shape: the journaled old-shape intent
// retries in its original shape; the same key with an explicit priority is a different payload.
#[test]
fn retry_keeps_its_recorded_shape() {
    for (mode, expected) in [(DeliveryMode::Ordinary, 0), (DeliveryMode::Lazy, 1)] {
        let iso = TestIsolation::new("prio-retry");
        let (context, mut conn) = setup(&iso, GROUP);
        let mut old = request();
        old.delivery_mode = mode;
        prepare(&context, &mut conn, &old);
        let stored: (i64, Vec<u8>) = conn
            .query_row("SELECT priority,digest FROM send_preparations", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(stored.0, expected);
        let mut new_shape = old.clone();
        new_shape.priority = p(expected as u8);
        let error = messages::prepare_send_step(
            &context,
            &mut conn,
            &new_shape,
            messages::MessageLimits::default(),
            &budget(),
            DurableWorkAdmission::new(16).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationPayloadMismatch);
        publish(&context, &mut conn, &old);
        assert_eq!(message_mode(&conn).1, expected);
        assert_eq!(rows(&conn).len(), 4);
        assert!(rows(&conn).iter().all(|(_, p)| *p == expected));
    }
}

// Kills: priority living in memory only: after a reopen the message, preparation and row
// priorities are the frozen ones.
#[test]
fn restart_keeps_priority() {
    let iso = TestIsolation::new("prio-restart");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.priority = p(3);
    send(&context, &mut conn, &req);
    drop(conn);
    let reopened = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock))
        .with_test_host()
        .open_writer()
        .unwrap();
    assert_eq!(message_mode(&reopened), ("lazy".to_owned(), 3));
    assert_eq!(
        count(&reopened, "SELECT priority FROM send_preparations"),
        3
    );
    assert_eq!(
        rows(&reopened),
        vec![row("a", 3), row("b", 3), row("c", 3), row("h", 3)]
    );
}

// Kills: the mixed ACK send breaking either settlement path: the receipt recipient settles its
// receipt, the row recipient completes its row, and only the receipt recipient has attention owed.
#[test]
fn mixed_ack_display_and_completion() {
    use crate::delivery::settle::SettlePlan;
    use crate::store::lazy_delivery::{self, LazyCompletion};
    let iso = TestIsolation::new("prio-mixed");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.invited_recipients = vec![SeatId::new("b")];
    let id = send(&context, &mut conn, &req);
    let plan = |conn: &mut Connection, seat: &str| {
        let tx = conn.transaction().unwrap();
        let plan = crate::store::receipts::plan_delivery_ack(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new(seat),
            None,
            &id,
        )
        .unwrap();
        tx.rollback().unwrap();
        plan
    };
    assert_eq!(plan(&mut conn, "b"), SettlePlan::Settle);
    assert_eq!(plan(&mut conn, "c"), SettlePlan::Complete);
    assert_eq!(plan(&mut conn, "h"), SettlePlan::Complete);
    let attention = |seat: &str| {
        crate::store::attention::wake_seat_attention(&conn, seat)
            .unwrap()
            .attention
            .has_pending_receipt
    };
    assert!(attention("b"));
    assert!(!attention("c"));
    // The row recipient's page lists the message and completing it settles the row.
    let seat = SeatId::new("c");
    assert_eq!(seat_pending(&conn, &seat).len(), 1);
    let tx = conn.transaction().unwrap();
    lazy_delivery::complete_for_binding(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &seat,
        None,
        std::slice::from_ref(&id),
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &conn,
            "34750f98bd59fcfc946da45aaabe933b",
            &seat,
            None,
            &id
        )
        .unwrap(),
        LazyCompletion::Completed
    );
    // The receipt recipient is not addressed by a row: completing is refused for it.
    let tx = conn.transaction().unwrap();
    lazy_delivery::complete_for_binding(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("b"),
        None,
        std::slice::from_ref(&id),
    )
    .unwrap();
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new("b"),
            None,
            &id
        )
        .unwrap(),
        LazyCompletion::NotAddressed
    );
    tx.rollback().unwrap();
}

/// Every message the seat-level delivery-row seek returns within the current publication bound.
fn seat_pending(conn: &Connection, seat: &SeatId) -> Vec<MessageId> {
    use crate::store::lazy_delivery::{RowFilter, seek_pending};
    let dh: i64 = conn
        .query_row(
            "SELECT decision_seq FROM local_node WHERE id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (mut after, mut out) = (None, Vec::new());
    loop {
        let seek = seek_pending(
            conn,
            "34750f98bd59fcfc946da45aaabe933b",
            seat,
            RowFilter::Seat,
            after,
            dh as u64,
            100,
        )
        .unwrap();
        out.extend(seek.row.map(|(_, recipient)| recipient.message));
        after = seek.position;
        if seek.exhausted {
            return out;
        }
    }
}

// Kills: the merged page changing the mixed ACK send: the explicit recipient's page lists the
// receipt, the row recipients' pages list the delivery row, and completing the row empties only
// that recipient's page.
#[test]
fn mixed_ack_merged_pages_and_completion() {
    use crate::protocol::{
        commands::{Command, InboxQuery},
        output::OutputSpec,
        pagination::PageRequest,
        results::InboxBatchV2Item,
    };
    use crate::store::lazy_delivery;
    let iso = TestIsolation::new("prio-mixed-page");
    let (context, mut conn) = setup(&iso, GROUP);
    let mut req = request();
    req.invited_recipients = vec![SeatId::new("b")];
    let id = send(&context, &mut conn, &req);
    let page = |seat: &str| -> Vec<String> {
        let CommandResult::InboxBatchV2(page) = crate::store::queries::query_with_output(
            &context,
            "34750f98bd59fcfc946da45aaabe933b",
            &Command::InboxBatchV2(InboxQuery {
                pushed: false,
                thread: None,
                claim: None,
                seat: Some(SeatId::new(seat)),
                page: PageRequest::default(),
            }),
            &OutputSpec::default(),
            &budget(),
        )
        .unwrap() else {
            panic!("v2 page")
        };
        assert!(!page.has_more);
        page.items
            .iter()
            .map(|item| match item {
                InboxBatchV2Item::Message { message, .. } => {
                    format!("receipt {}", message.as_str())
                }
                InboxBatchV2Item::LazyMessage { message, .. } => {
                    format!("row {}", message.as_str())
                }
                other => panic!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(page("b"), [format!("receipt {}", id.as_str())]);
    assert_eq!(page("c"), [format!("row {}", id.as_str())]);
    assert_eq!(page("h"), [format!("row {}", id.as_str())]);
    let tx = conn.transaction().unwrap();
    lazy_delivery::complete_for_binding(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("c"),
        None,
        std::slice::from_ref(&id),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(
        page("c").is_empty(),
        "a completed row leaves the merged page"
    );
    assert_eq!(page("b"), [format!("receipt {}", id.as_str())]);
    assert_eq!(page("h"), [format!("row {}", id.as_str())]);
}
