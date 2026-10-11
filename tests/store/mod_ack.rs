//! `AckModDelivered` deciding transaction (spec D6, ht-j16.3): per-id results,
//! `cooperative_mod_delivery` provenance, lazy completion and replay.
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::AckModDelivered,
    ids::*,
    results::CommandResult,
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
    watch::{ModAckOutcome, ModAckReason, ModAckReport, ModDeliveryVia, WATCH_BODY_LIMIT_BYTES},
};
use crate::store::{
    connection::StoreContext,
    lazy_delivery::{self, LazyCompletion},
    read_cursors, receipts,
};
use rusqlite::{Connection, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

/// Seats `a` (sender), `b` (the Claude recipient) and `c` (not addressed).
fn setup() -> (StoreContext, Connection, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1000)));
    let path = std::env::temp_dir().join(format!("mod-ack-test-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path, clock.clone());
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    for (seat, target) in [("a", "pa"), ("b", "pb"), ("c", "pc")] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![seat,target]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    for (seat, target, session, execution) in [
        ("a", "pa", "n", "00000000-0000-4000-8000-0000000000aa"),
        ("b", "pb", "nb", "00000000-0000-4000-8000-0000000000bb"),
        ("c", "pc", "nc", "00000000-0000-4000-8000-0000000000cc"),
    ] {
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'claude',?3,?4,'cooperative_top_level',0,'term-'||?2,'inc')", params![seat,target,session,execution]).unwrap();
    }
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    for seat in ["a", "b"] {
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",[seat]).unwrap();
    }
    (context, conn, clock)
}

fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("b"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("nb"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000bb"),
        target: HostTargetId::new("pb"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
fn request(ids: &[&str]) -> AckModDelivered {
    AckModDelivered {
        via: ModDeliveryVia::Context,
        messages: ids.iter().map(|id| MessageId::new(*id)).collect(),
        operation: OperationId::new("mod-ack-op"),
        claim: claim(),
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn run(
    context: &StoreContext,
    conn: &mut Connection,
    request: &AckModDelivered,
) -> Result<ModAckReport, crate::protocol::results::ApiError> {
    let digest =
        crate::store::schema::canonical_digest(&receipts::ack_mod_delivered_payload(request))
            .unwrap();
    let mut permit = MutationPermit::cooperative(
        request.claim.clone(),
        request.operation.clone(),
        ObligationRef::CheckIn(request.claim.seat.clone()),
        digest,
        MonoInstant(50),
        (1, 0),
        budget(),
    );
    match receipts::ack_mod_delivered(context, conn, &budget(), request, &mut permit)? {
        CommandResult::ModDeliveryAcked(report) => Ok(report),
        other => panic!("unexpected result {other:?}"),
    }
}
fn outcomes(report: &ModAckReport) -> Vec<(&str, ModAckOutcome, Option<ModAckReason>)> {
    report
        .results
        .iter()
        .map(|item| (item.id.as_str(), item.result, item.reason))
        .collect()
}
/// Adds an ordinary message from `a` with a pending receipt for `b`.
fn pending(conn: &Connection, id: &str, sequence: i64, body: &str) {
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','a',?3,0,?4)", params![id,sequence,body,sequence+1]).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','b','pending',300000)", [id]).unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 1],
    )
    .unwrap();
}
/// Like [`pending`], but receivable: a sealed preparation, a prepared recipient row for `b` and a
/// manifest (the join `read_cursors::RECEIVABLE_FROM` walks), so a cursor cannot pass it unshown.
fn receivable_pending(conn: &Connection, id: &str, sequence: i64, body: &str) {
    pending(conn, id, sequence, body);
    conn.execute_batch(&format!(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('prep-{id}','i','actor','prep-{id}',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('prep-{id}','t','b',1,300000,1);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-{id}','{id}','i','t',{},0,{sequence},0,1,0);",
        sequence + 1
    ))
    .unwrap();
}
/// The binding id of seat `b`'s first (claim `nb`) binding.
fn binding_b(conn: &Connection) -> BindingId {
    BindingId::new(
        conn.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='b' AND native_session='nb'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
}
fn cursor_b(conn: &Connection, binding: &BindingId) -> u64 {
    read_cursors::cursor_for(conn, binding, &ThreadId::new("t"))
        .unwrap()
        .unwrap_or(0)
}
fn receipt_state(conn: &Connection, id: &str) -> String {
    conn.query_row(
        "SELECT state FROM receipts WHERE message_id=?1 AND seat_id='b'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
/// A published lazy message `m<id>` addressed to `b` (state `pending`).
fn lazy(conn: &Connection, prep: &str, sequence: i64, body: &str) -> String {
    let message = format!("m{}", &prep[1..]);
    conn.execute_batch(&format!("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES ('{prep}','i','actor','o-{prep}',zeroblob(32),'t',0,0,0,0,0,0,1,'building','lazy');")).unwrap();
    conn.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,'t','b')", params![prep,message]).unwrap();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,delivery_mode) VALUES (?1,'i','t',?2,'ordinary','a',?3,0,?4,'lazy')", params![message,sequence,body,sequence+1]).unwrap();
    conn.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,0,?4,0,1,0)", params![prep,message,sequence+1,sequence]).unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 1],
    )
    .unwrap();
    message
}
fn lazy_state(conn: &Connection, message: &str) -> String {
    conn.query_row(
        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='b'",
        [message],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn settles_pending_ordinary_with_mod_delivery_provenance() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    let report = run(&context, &mut conn, &request(&["m1"])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![("m1", ModAckOutcome::Settled, None)]
    );
    assert_eq!(receipt_state(&conn, "m1"), "acked");
    let observation: String = conn
        .query_row(
            "SELECT ack_observation FROM receipts WHERE message_id='m1' AND seat_id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["action_provenance"], "cooperative_mod_delivery");
    assert_eq!(observation["via"], "context");
    assert_eq!(observation["provenance"], "cooperative_top_level");
    let event: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(event, 1, "one compact ack event per thread");
}

#[test]
fn already_acked_is_already_settled_and_not_rewritten() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    run(&context, &mut conn, &request(&["m1"])).unwrap();
    let first: (String, i64) = conn
        .query_row(
            "SELECT ack_observation,acked_at FROM receipts WHERE message_id='m1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut again = request(&["m1"]);
    again.operation = OperationId::new("second-op");
    again.via = ModDeliveryVia::Submit;
    let report = run(&context, &mut conn, &again).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![("m1", ModAckOutcome::AlreadySettled, None)]
    );
    let second: (String, i64) = conn
        .query_row(
            "SELECT ack_observation,acked_at FROM receipts WHERE message_id='m1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(first, second);
}

#[test]
fn unknown_id_refused_terminal_unknown_others_still_settle() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    pending(&conn, "m2", 2, "world");
    let report = run(&context, &mut conn, &request(&["m2", "ghost", "m1"])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            ("m2", ModAckOutcome::Settled, None),
            (
                "ghost",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::Unknown)
            ),
            ("m1", ModAckOutcome::Settled, None),
        ]
    );
    assert_eq!(receipt_state(&conn, "m1"), "acked");
    assert_eq!(receipt_state(&conn, "m2"), "acked");
}

#[test]
fn unaddressed_id_refused_terminal_not_addressed() {
    let (context, mut conn, _) = setup();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('foreign','i','t',1,'ordinary','a','other',0,2)", []).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('foreign','t','c','pending',300000)", []).unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=2 WHERE id='i'", [])
        .unwrap();
    let report = run(&context, &mut conn, &request(&["foreign"])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![(
            "foreign",
            ModAckOutcome::RefusedTerminal,
            Some(ModAckReason::NotAddressed)
        )]
    );
    let state: String = conn
        .query_row(
            "SELECT state FROM receipts WHERE message_id='foreign' AND seat_id='c'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "pending");
}

#[test]
fn body_over_8_kib_refused_terminal_truncated_and_stays_pending() {
    let (context, mut conn, _) = setup();
    pending(&conn, "exact", 1, &"x".repeat(WATCH_BODY_LIMIT_BYTES));
    pending(&conn, "over", 2, &"x".repeat(WATCH_BODY_LIMIT_BYTES + 1));
    // Multi-byte body: 4097 two-byte chars is 8194 bytes but only 4097 chars.
    pending(&conn, "wide", 3, &"é".repeat(4097));
    let report = run(&context, &mut conn, &request(&["exact", "over", "wide"])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            ("exact", ModAckOutcome::Settled, None),
            (
                "over",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::Truncated)
            ),
            (
                "wide",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::Truncated)
            ),
        ]
    );
    assert_eq!(receipt_state(&conn, "exact"), "acked");
    assert_eq!(receipt_state(&conn, "over"), "pending");
    assert_eq!(receipt_state(&conn, "wide"), "pending");
}

#[test]
fn lazy_row_completes_as_displayed_and_replay_is_already_settled() {
    let (context, mut conn, _) = setup();
    let message = lazy(&conn, "p0000aaaa", 1, "lazy body");
    assert_eq!(lazy_state(&conn, &message), "pending");
    let mut first = request(&[&message]);
    first.via = ModDeliveryVia::Append;
    let report = run(&context, &mut conn, &first).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![(message.as_str(), ModAckOutcome::Settled, None)]
    );
    assert_eq!(lazy_state(&conn, &message), "displayed");
    let mut second = request(&[&message]);
    second.operation = OperationId::new("later-op");
    let report = run(&context, &mut conn, &second).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![(message.as_str(), ModAckOutcome::AlreadySettled, None)]
    );
}

#[test]
fn lazy_over_limit_refused_truncated() {
    let (context, mut conn, _) = setup();
    let message = lazy(
        &conn,
        "p0000bbbb",
        1,
        &"y".repeat(WATCH_BODY_LIMIT_BYTES + 1),
    );
    let report = run(&context, &mut conn, &request(&[&message])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![(
            message.as_str(),
            ModAckOutcome::RefusedTerminal,
            Some(ModAckReason::Truncated)
        )]
    );
    assert_eq!(lazy_state(&conn, &message), "pending");
}

#[test]
fn replay_same_operation_returns_identical_report() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    let req = request(&["m1", "ghost"]);
    let first = run(&context, &mut conn, &req).unwrap();
    let again = run(&context, &mut conn, &req).unwrap();
    assert_eq!(first, again);
    assert_eq!(first.results[0].result, ModAckOutcome::Settled);
    let events: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1, "replay writes nothing");
    let mut changed = req.clone();
    changed.via = ModDeliveryVia::Submit;
    assert_eq!(
        run(&context, &mut conn, &changed).unwrap_err().code,
        crate::protocol::results::ErrorCode::OperationPayloadMismatch
    );
}

#[test]
fn warning_or_notice_id_refused_not_addressed() {
    let (context, mut conn, _) = setup();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES ('notice','i','t',1,'info','k-notice','{}',0,2),('warning','i','t',2,'warn','k-warn','{}',0,3)", []).unwrap();
    conn.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
        .unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=3 WHERE id='i'", [])
        .unwrap();
    let report = run(&context, &mut conn, &request(&["notice", "warning"])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            (
                "notice",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::NotAddressed)
            ),
            (
                "warning",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::NotAddressed)
            ),
        ]
    );
}

#[test]
fn overdue_pending_receipt_records_late_warning_before_settlement() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    conn.execute(
        "UPDATE receipts SET available_at=0,deadline_at=500 WHERE message_id='m1'",
        [],
    )
    .unwrap();
    run(&context, &mut conn, &request(&["m1"])).unwrap();
    let warning: i64 = conn
        .query_row(
            "SELECT sequence FROM messages WHERE kind='warn' AND source_message_id='m1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let ack_event: i64 = conn
        .query_row(
            "SELECT sequence FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(warning < ack_event, "late warning precedes the ack event");
    assert_eq!(receipt_state(&conn, "m1"), "acked");
}

#[test]
fn claim_rejected_without_writes_for_wrong_harness_and_subagent() {
    let (context, mut conn, _) = setup();
    pending(&conn, "m1", 1, "hello");
    let mut codex = request(&["m1"]);
    codex.claim.harness = Harness::Codex;
    assert!(run(&context, &mut conn, &codex).is_err());
    let mut child = request(&["m1"]);
    child.claim.role = CallerRole::Subagent;
    assert!(run(&context, &mut conn, &child).is_err());
    assert_eq!(receipt_state(&conn, "m1"), "pending");
}

#[test]
fn mod_ack_of_ordinary_and_lazy_advances_the_cursor_past_both() {
    let (context, mut conn, _) = setup();
    receivable_pending(&conn, "o6", 1, "ordinary");
    let lazy_id = lazy(&conn, "p0000aaaa", 2, "lazy body");
    let report = run(&context, &mut conn, &request(&["o6", &lazy_id])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            ("o6", ModAckOutcome::Settled, None),
            (lazy_id.as_str(), ModAckOutcome::Settled, None)
        ]
    );
    let binding = binding_b(&conn);
    // The cursor also skips the non-receivable ack event the settlement appended (seq 3).
    assert!(
        cursor_b(&conn, &binding) >= 2,
        "passes the ordinary and the lazy row"
    );
    assert!(!read_cursors::has_unread(&conn, &binding).unwrap());
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &conn,
            "i",
            &SeatId::new("b"),
            Some(&binding),
            &MessageId::new(lazy_id.as_str())
        )
        .unwrap(),
        LazyCompletion::Completed
    );
}

#[test]
fn mod_ack_of_only_the_lazy_row_stops_before_an_unshown_ordinary_message() {
    let (context, mut conn, _) = setup();
    receivable_pending(&conn, "o6", 1, "ordinary");
    let lazy_id = lazy(&conn, "p0000aaaa", 2, "lazy body");
    run(&context, &mut conn, &request(&[&lazy_id])).unwrap();
    let binding = binding_b(&conn);
    assert_eq!(cursor_b(&conn, &binding), 0, "o6 was never shown");
    assert!(read_cursors::has_unread(&conn, &binding).unwrap());
    assert_eq!(receipt_state(&conn, "o6"), "pending");
}

#[test]
fn already_settled_ordinary_still_advances_a_second_bindings_cursor() {
    let (context, mut conn, _) = setup();
    receivable_pending(&conn, "o6", 1, "ordinary");
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('b',1,'pb','b',1,1,'claude','nb2','00000000-0000-4000-8000-0000000000b2','cooperative_top_level',0,'term-pb','inc')", []).unwrap();
    let second: BindingId = BindingId::new(
        conn.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session='nb2'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    run(&context, &mut conn, &request(&["o6"])).unwrap();
    assert!(cursor_b(&conn, &binding_b(&conn)) >= 1);
    assert_eq!(cursor_b(&conn, &second), 0);
    let mut again = request(&["o6"]);
    again.operation = OperationId::new("second-binding-op");
    again.claim.native_session = NativeSessionId::new("nb2");
    again.claim.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000b2");
    let report = run(&context, &mut conn, &again).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![("o6", ModAckOutcome::AlreadySettled, None)]
    );
    assert!(cursor_b(&conn, &second) >= 1);
    assert!(!read_cursors::has_unread(&conn, &second).unwrap());
}

#[test]
fn truncated_ordinary_next_to_a_lazy_row_does_not_let_the_cursor_pass() {
    let (context, mut conn, _) = setup();
    receivable_pending(&conn, "big", 1, &"x".repeat(WATCH_BODY_LIMIT_BYTES + 1));
    let lazy_id = lazy(&conn, "p0000aaaa", 2, "lazy body");
    let report = run(&context, &mut conn, &request(&["big", &lazy_id])).unwrap();
    assert_eq!(
        outcomes(&report),
        vec![
            (
                "big",
                ModAckOutcome::RefusedTerminal,
                Some(ModAckReason::Truncated)
            ),
            (lazy_id.as_str(), ModAckOutcome::Settled, None)
        ]
    );
    assert_eq!(cursor_b(&conn, &binding_b(&conn)), 0);
}
