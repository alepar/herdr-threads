use crate::protocol::{
    authority::{CallerClaim, Harness, MutationPermit, ObligationRef},
    commands::{Ack, SendMessage},
    ids::*,
    time::{Clock, MonoInstant, UtcMillis},
};
use crate::store::{attention, connection::StoreContext, messages, receipts};
use rusqlite::{Connection, params};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
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
fn setup() -> (StoreContext, Connection, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1000)));
    let path: PathBuf =
        std::env::temp_dir().join(format!("receipt-test-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path, clock.clone());
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    for (seat, target) in [("a", "pa"), ("b", "pb"), ("c", "pc"), ("d", "pd")] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![seat,target]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    // The acting seats hold a live, not-yet-registered cooperative binding.
    for (seat, target, session, execution) in [
        ("a", "pa", "n", "00000000-0000-4000-8000-0000000000aa"),
        ("b", "pb", "nb", "00000000-0000-4000-8000-0000000000bb"),
    ] {
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'codex',?3,?4,'cooperative_top_level',0,'term-'||?2,'inc')", params![seat,target,session,execution]).unwrap();
    }
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    for (seat, state) in [("a", "joined"), ("b", "joined"), ("c", "invited")] {
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,?2)",
            params![seat, state],
        )
        .unwrap();
        if state == "joined" {
            conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",[seat]).unwrap();
        }
    }
    (context, conn, clock)
}
fn send_request(explicit: Vec<&str>) -> SendMessage {
    SendMessage {
        thread: ThreadId::new("t"),
        body: "hello".into(),
        invited_recipients: explicit.into_iter().map(SeatId::new).collect(),
        deadline_millis: None,
        operation: OperationId::new("op"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("a"),
            binding_generation: 1,
            role: crate::protocol::authority::CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            target: HostTargetId::new("pa"),
        },
        relays_user: false,
        user_intent: None,
    }
}

#[test]
fn human_recipient_gets_message_without_an_ack_expectation() {
    let (context, mut conn, _) = setup();
    conn.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id='b'", []).unwrap();
    let request = send_request(vec![]);
    let mut permit = permit(&request);
    let result = send_prepared(
        &context,
        &mut conn,
        &request,
        &mut permit,
        messages::MessageLimits::default(),
    )
    .unwrap();
    let message = match result {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("unexpected send result: {other:?}"),
    };
    let ordinary: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE id=?1 AND kind='ordinary'",
            [message.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let receipts: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_recipients WHERE seat_id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let unavailable: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_unavailable_warnings WHERE affected_seat_id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ordinary, 1);
    assert_eq!(receipts, 0);
    assert_eq!(unavailable, 1);
}

#[test]
fn overdue_scan_does_not_warn_for_a_waived_legacy_human_receipt() {
    let (context, mut conn, _) = setup();
    conn.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('legacy','i','t',1,'ordinary','old message',100,2); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('legacy','t','b','pending',800,100,900); UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id='b'; INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('b',2,1,100);").unwrap();
    let mut cursor = receipts::ReceiptDueCursor::default();
    let scan = receipts::scan_due(&context, &mut conn, 10, &mut cursor).unwrap();
    assert_eq!(scan.warnings, 0);
    let marker: Option<String> = conn
        .query_row(
            "SELECT warning_message_id FROM receipts WHERE message_id='legacy' AND seat_id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(marker.is_none());
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "legacy", "b")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::NotRequired
    );
}

#[test]
fn human_may_explicitly_ack_waived_older_receipt_without_new_overdue_warning() {
    let (context, mut conn, _) = setup();
    conn.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('legacy','i','t',1,'ordinary','old message',100,2); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_required) VALUES ('legacy','t','b','pending',800,100,900,0); UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id='b'; INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('b',2,1,100);").unwrap();
    conn.execute_batch("UPDATE threads SET next_sequence=2 WHERE id='t'; UPDATE host_instances SET decision_seq=2 WHERE id='i';").unwrap();
    assert_eq!(conn.query_row(
        "SELECT state,acked_at,ack_actor_seat_id,ack_observation FROM receipts WHERE message_id='legacy' AND seat_id='b'",
        [], |r| Ok((r.get::<_, String>(0)?,r.get::<_, Option<i64>>(1)?,r.get::<_, Option<String>>(2)?,r.get::<_, Option<String>>(3)?)),
    ).unwrap(), ("pending".into(),None,None,None), "a waiver does not fabricate an ACK");
    let agent_request = ack_request(vec![MessageId::new("legacy")]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    assert_eq!(
        receipts::ack(
            &context,
            &mut conn,
            &budget,
            &agent_request,
            &mut ack_permit(&agent_request)
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::InvalidRequest,
        "agents cannot revive waived obligations"
    );
    let mut request = ack_request(vec![MessageId::new("legacy")]);
    request.claim.harness = Harness::Human;
    assert_eq!(
        receipts::ack_displayed(
            &context,
            &mut conn,
            &budget,
            &request,
            &mut ack_permit(&request)
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::InvalidRequest,
        "human ACK is explicit, never display settlement"
    );
    let result = receipts::ack(
        &context,
        &mut conn,
        &budget,
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::Acknowledged(result) = result else {
        panic!("unexpected ACK result: {result:?}");
    };
    assert_eq!(result.acknowledged, vec![MessageId::new("legacy")]);
    let receipt = crate::store::effective::effective_receipt(&conn, "legacy", "b")
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.state,
        crate::store::effective::EffectiveReceiptState::Acknowledged
    );
    assert_eq!(receipt.ack_actor_seat_id.as_deref(), Some("b"));
    let observation: serde_json::Value =
        serde_json::from_str(receipt.ack_observation.as_deref().unwrap()).unwrap();
    assert_eq!(observation["provenance"], "operator_human");
    assert_eq!(
        conn.query_row(
            "SELECT ack_required FROM receipts WHERE message_id='legacy' AND seat_id='b'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let repeat = Ack {
        operation: OperationId::new("human-ack-again"),
        ..request.clone()
    };
    let repeated = receipts::ack(
        &context,
        &mut conn,
        &budget,
        &repeat,
        &mut ack_permit(&repeat),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::Acknowledged(repeated) = repeated else {
        panic!();
    };
    assert!(repeated.acknowledged.is_empty());
    assert_eq!(
        repeated.already_acknowledged,
        vec![MessageId::new("legacy")]
    );
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "legacy", "b")
            .unwrap()
            .unwrap()
            .ack_observation,
        receipt.ack_observation,
        "repeat keeps the original human provenance"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "an optional human ACK creates no overdue warning"
    );
}

#[test]
fn persisted_waivers_leave_pending_and_due_windows_for_agent_mail() {
    let (context, mut conn, _) = setup();
    conn.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1001) INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) SELECT 'human-'||x,'i','t',x,'ordinary','old human mail',100,x+1 FROM n; INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_required) SELECT id,'t','b','pending',10,100,110,0 FROM messages WHERE id LIKE 'human-%'; INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('agent-mail','i','t',1002,'ordinary','agent mail',1000,1003); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('agent-mail','t','b','pending',1000,1000,2000);").unwrap();
    let pending = attention::pending_receipts(&conn, "b", None).unwrap();
    assert_eq!(pending.count(), (1, false));
    assert_eq!(pending.items[0].id, "agent-mail");
    assert!(
        pending.work_steps < 20,
        "waived rows filled the bounded walk: {pending:?}"
    );
    let mut cursor = receipts::ReceiptDueCursor::default();
    let scan = receipts::scan_due(&context, &mut conn, 10, &mut cursor).unwrap();
    assert_eq!((scan.warnings, scan.inspected), (0, 1));
    assert!(!scan.more);
}

fn send_prepared(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    permit: &mut MutationPermit,
    limits: messages::MessageLimits,
) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError> {
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    loop {
        match messages::prepare_send_step(context, conn, request, limits, &budget, admission)? {
            crate::ports::SendPreparationProgress::Committed(result) => return Ok(result),
            crate::ports::SendPreparationProgress::Ready { .. } => break,
            crate::ports::SendPreparationProgress::More { visited, .. } => assert!(visited > 0),
        }
    }
    messages::publish_send(context, conn, request, permit, &budget, || {
        limits.body_bytes
    })
}
fn cooperative_permit(
    claim: &CallerClaim,
    operation: &OperationId,
    obligation: ObligationRef,
    digest: [u8; 32],
    revisions: (u64, u64),
) -> MutationPermit {
    MutationPermit::cooperative(
        claim.clone(),
        operation.clone(),
        obligation,
        digest,
        MonoInstant(50),
        revisions,
        crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
    )
}
fn permit(request: &SendMessage) -> MutationPermit {
    let digest = crate::store::schema::canonical_digest(&messages::send_payload(request)).unwrap();
    cooperative_permit(
        &request.claim,
        &request.operation,
        ObligationRef::Control(request.thread.clone()),
        digest,
        (1, 0),
    )
}
fn ack_request(ids: Vec<MessageId>) -> Ack {
    Ack {
        messages: ids,
        operation: OperationId::new("ack-op"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("b"),
            binding_generation: 1,
            role: crate::protocol::authority::CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("nb"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000bb"),
            target: HostTargetId::new("pb"),
        },
    }
}
fn ack_permit(request: &Ack) -> MutationPermit {
    let digest = crate::store::schema::canonical_digest(&receipts::ack_payload(request)).unwrap();
    cooperative_permit(
        &request.claim,
        &request.operation,
        ObligationRef::CheckIn(request.claim.seat.clone()),
        digest,
        (1, 0),
    )
}

fn filter_revision(conn: &Connection, kind: &str, key: &str) -> i64 {
    conn.query_row(
        "SELECT COALESCE((SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind=?1 AND scope_key=?2),0)",
        params![kind, key],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn send_snapshots_joined_plus_explicit_invited_once() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec!["c", "b"]);
    let result = send_prepared(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        messages::MessageLimits::default(),
    )
    .unwrap();
    let id = match result {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!("wrong result"),
    };
    assert!(
        crate::protocol::ids::is_short_public_id(
            crate::protocol::ids::prefix::MESSAGE,
            id.as_str()
        ),
        "new message IDs are short: {}",
        id.as_str()
    );
    let preparation: String = conn
        .query_row(
            "SELECT preparation_id FROM send_manifests WHERE message_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        crate::protocol::ids::send_message_id_for_preparation(&preparation).as_deref(),
        Some(id.as_str())
    );
    let mut stmt = conn
        .prepare("SELECT pr.seat_id FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 ORDER BY pr.seat_id")
        .unwrap();
    let seats: Vec<String> = stmt
        .query_map([id.as_str()], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(seats, ["b", "c"]);
    let frozen: Vec<i64> = conn
        .prepare("SELECT pr.frozen_duration_ms FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1")
        .unwrap()
        .query_map([id.as_str()], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(frozen, [300_000, 300_000]);
}

#[test]
fn persisted_legacy_preparation_publishes_beside_a_new_compact_message() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let preparation = match messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        crate::ports::DurableWorkAdmission { max_units: 16 },
    )
    .unwrap()
    {
        crate::ports::SendPreparationProgress::Ready { preparation_id, .. } => preparation_id,
        other => panic!("{other:?}"),
    };
    let staged_message = send_message_id_for_preparation(&preparation).unwrap();
    let old_preparation = "prep-bc121d12-9d30-4510-b93c-e7cd26e890f1";
    let old_message = "msg-bc121d12-9d30-4510-b93c-e7cd26e890f1";
    // Simulate an in-flight preparation written by the prior release. Keep
    // the same durable operation and recipient snapshot; only its stored ID
    // and the event's copied source ID use the persisted format.
    let tx = conn.transaction().unwrap();
    tx.execute_batch("PRAGMA defer_foreign_keys=ON").unwrap();
    tx.execute(
        "UPDATE prepared_unavailable_warnings SET event_json=replace(event_json, ?1, ?2) WHERE preparation_id=?3",
        params![staged_message, old_message, preparation],
    )
    .unwrap();
    tx.execute(
        "UPDATE send_preparations SET id=?1 WHERE id=?2",
        params![old_preparation, preparation],
    )
    .unwrap();
    tx.execute(
        "UPDATE prepared_recipients SET preparation_id=?1 WHERE preparation_id=?2",
        params![old_preparation, preparation],
    )
    .unwrap();
    tx.execute(
        "UPDATE prepared_unavailable_warnings SET preparation_id=?1 WHERE preparation_id=?2",
        params![old_preparation, preparation],
    )
    .unwrap();
    tx.commit().unwrap();

    let old = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap();
    assert_eq!(
        old,
        crate::protocol::results::CommandResult::MessageSent(MessageId::new(old_message))
    );
    let mut next = send_request(vec![]);
    next.operation = OperationId::new("op-next");
    let new = send_prepared(
        &context,
        &mut conn,
        &next,
        &mut permit(&next),
        messages::MessageLimits::default(),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::MessageSent(new_id) = new else {
        panic!("{new:?}")
    };
    assert!(is_short_public_id(prefix::MESSAGE, new_id.as_str()));
    let ids: Vec<String> = conn
        .prepare(
            "SELECT id FROM messages WHERE thread_id='t' AND kind='ordinary' ORDER BY sequence",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, [old_message.to_owned(), new_id.as_str().to_owned()]);
}

#[test]
fn publication_uses_preparation_instance_for_message_and_manifest() {
    let (context, mut conn, _) = setup();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('other',0,'other-boot',1,1)",
        [],
    )
    .unwrap();
    let request = send_request(vec![]);
    let first = send_prepared(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        messages::MessageLimits::default(),
    )
    .unwrap();
    let id = match &first {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    let published: (String, String, i64, i64) = conn
        .query_row(
            "SELECT m.instance_id, sm.instance_id, m.decision_seq, sm.decision_seq FROM messages m JOIN send_manifests sm ON sm.message_id=m.id WHERE m.id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(published.0, "i");
    assert_eq!(published.1, "i");
    assert_eq!(published.2, published.3);
    assert!(conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('wrong-instance','other','t',999,'ordinary','a','x',0,99)", []).is_err());
    assert!(
        conn.execute(
            "UPDATE send_manifests SET instance_id='other' WHERE message_id=?1",
            [id.as_str()]
        )
        .is_err()
    );
    let replay = send_prepared(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        messages::MessageLimits::default(),
    )
    .unwrap();
    assert_eq!(replay, first);
    let counts: (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM messages WHERE kind='ordinary'),(SELECT count(*) FROM send_manifests)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (1, 1));
}

#[test]
fn invalid_ack_batch_rolls_back_all_settlement_and_warnings() {
    let (context, mut conn, clock) = setup();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    clock.0.store(2000, Ordering::SeqCst);
    let request = ack_request(vec![id.clone(), MessageId::new("missing")]);
    assert!(
        receipts::ack(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &request,
            &mut ack_permit(&request),
        )
        .is_err()
    );
    let state = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap()
        .state;
    assert_eq!(
        state,
        crate::store::effective::EffectiveReceiptState::Pending
    );
    let warnings: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 0);
}

#[test]
fn availability_uses_frozen_duration_and_scan_warns_once_at_equality() {
    let (context, mut conn, clock) = setup();
    let send = send_request(vec![]);
    let limits = messages::MessageLimits {
        receipt_duration_ms: 50,
        body_bytes: 10,
    };
    let id = match send_prepared(&context, &mut conn, &send, &mut permit(&send), limits).unwrap() {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1100,1,'verified')",[]).unwrap();
    let effective = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap();
    let duration:i64=conn.query_row("SELECT pr.frozen_duration_ms FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='b'",[id.as_str()],|r|r.get(0)).unwrap();
    let deadline = effective.deadline_at.unwrap();
    assert_eq!((duration, deadline), (50, 1150));
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1100,1150)",[id.as_str()]).unwrap();
    let before_revision: i64 = conn.query_row("SELECT COALESCE((SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='b'),0)", [], |r| r.get(0)).unwrap();
    clock.0.store(1150, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    assert_eq!(
        receipts::scan_due(&context, &mut conn, 10, &mut cursor)
            .unwrap()
            .warnings,
        1
    );
    let after_revision: i64 = conn.query_row("SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='b'", [], |r| r.get(0)).unwrap();
    assert_eq!(after_revision, before_revision + 1);
    assert_eq!(
        receipts::scan_due(&context, &mut conn, 10, &mut cursor)
            .unwrap()
            .warnings,
        0
    );
    let warnings: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 1);
}

#[test]
fn retired_recipient_is_fenced_from_new_send_and_due_scan() {
    let (context, mut conn, clock) = setup();
    let mut send = send_request(vec![]);
    send.deadline_millis = Some(500);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1000,1,'verified')",[]).unwrap();
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1000,1500)",[id.as_str()]).unwrap();
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1500,retired_seq=4 WHERE id='b'",
        [],
    )
    .unwrap();
    clock.0.store(3000, Ordering::SeqCst);
    assert_eq!(
        receipts::scan_due(
            &context,
            &mut conn,
            10,
            &mut receipts::ReceiptDueCursor::default()
        )
        .unwrap()
        .warnings,
        0
    );
    let mut other_send = send_request(vec![]);
    other_send.operation = OperationId::new("op2");
    let new_id = match send_prepared(
        &context,
        &mut conn,
        &other_send,
        &mut permit(&other_send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1",
            [new_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn ack_at_deadline_warns_before_info_and_replay_keeps_original_attribution() {
    let (context, mut conn, clock) = setup();
    let mut send = send_request(vec![]);
    send.deadline_millis = Some(500);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1500,1,'verified')",[]).unwrap();
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1500,2000)",[id.as_str()]).unwrap();
    clock.0.store(2000, Ordering::SeqCst);
    let request = ack_request(vec![id.clone()]);
    let directory_before = filter_revision(&conn, "directory", "t");
    let inbox_before = filter_revision(&conn, "inbox", "b");
    let first = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    assert_eq!(
        filter_revision(&conn, "directory", "t"),
        directory_before + 1
    );
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_before + 1);
    let rows: Vec<(String,i64)> = conn.prepare("SELECT kind,sequence FROM messages WHERE source_message_id=?1 OR kind='info' ORDER BY sequence").unwrap()
        .query_map([id.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        ["warn", "info"]
    );
    clock.0.store(9000, Ordering::SeqCst);
    assert_eq!(
        receipts::ack(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &request,
            &mut ack_permit(&request),
        )
        .unwrap(),
        first
    );
    let (at, observation): (i64, String) = conn
        .query_row(
            "SELECT acked_at,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(at, 2000);
    assert_eq!(
        filter_revision(&conn, "directory", "t"),
        directory_before + 1
    );
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_before + 1);
    assert!(observation.contains("\"session\":\"nb\""));
    assert_eq!(
        receipts::scan_due(
            &context,
            &mut conn,
            10,
            &mut receipts::ReceiptDueCursor::default()
        )
        .unwrap()
        .warnings,
        0
    );
}

#[test]
fn configured_body_limit_rejects_one_byte_over_and_later_config_does_not_block_replay() {
    let (context, mut conn, _) = setup();
    let mut send = send_request(vec![]);
    send.body = "éé".into(); // four UTF-8 bytes
    let limits = messages::MessageLimits {
        receipt_duration_ms: 25,
        body_bytes: 4,
    };
    let original = send_prepared(&context, &mut conn, &send, &mut permit(&send), limits).unwrap();
    let replay = send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits {
            receipt_duration_ms: 50,
            body_bytes: 3,
        },
    )
    .unwrap();
    assert_eq!(replay, original);
    let mut over = send_request(vec![]);
    over.operation = OperationId::new("other");
    over.body = "ééx".into();
    let error = send_prepared(&context, &mut conn, &over, &mut permit(&over), limits).unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn one_ack_batch_can_settle_messages_from_two_threads() {
    let (context, mut conn, _) = setup();
    let send = send_request(vec![]);
    let first = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('u','i','second','goal',0,0)", []).unwrap();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('m2','i','u',1,'ordinary','a','other',0,100000)", []).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=100000 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id='u'", [])
        .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m2','u','b','pending',300000)", []).unwrap();
    let request = ack_request(vec![first, MessageId::new("m2")]);
    let t_before = filter_revision(&conn, "directory", "t");
    let u_before = filter_revision(&conn, "directory", "u");
    let result = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    assert!(matches!(
        result,
        crate::protocol::results::CommandResult::Acknowledged(_)
    ));
    let info: i64 = conn
        .query_row("SELECT count(*) FROM messages WHERE kind='info'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(info, 2);
    assert_eq!(filter_revision(&conn, "directory", "t"), t_before + 1);
    assert_eq!(filter_revision(&conn, "directory", "u"), u_before + 1);
}

#[test]
fn committed_ack_replay_survives_retirement_without_new_attribution() {
    let (context, mut conn, clock) = setup();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    let request = ack_request(vec![id.clone()]);
    let original = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1500 WHERE id='b'",
        [],
    )
    .unwrap();
    clock.0.store(9000, Ordering::SeqCst);
    assert_eq!(
        receipts::ack(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &request,
            &mut ack_permit(&request),
        )
        .unwrap(),
        original
    );
    let at: i64 = conn
        .query_row(
            "SELECT acked_at FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, 1000);
}

#[test]
fn unavailable_joined_recipient_gets_one_warning_for_its_episode() {
    let (context, mut conn, clock) = setup();
    for key in ["op", "op2"] {
        let mut send = send_request(vec!["c"]);
        send.operation = OperationId::new(key);
        send_prepared(
            &context,
            &mut conn,
            &send,
            &mut permit(&send),
            messages::MessageLimits::default(),
        )
        .unwrap();
    }
    let mut stmt = conn
        .prepare("SELECT w.event_json FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id")
        .unwrap();
    let warnings: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(stmt);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("\"seat\":\"b\""));
    let open: i64 = conn.query_row("SELECT count(*) FROM warning_conditions WHERE condition_kind='unavailable' AND clear_warning_id IS NULL", [], |r| r.get(0)).unwrap();
    assert_eq!(open, 1);
    let tx = conn.transaction().unwrap();
    assert_eq!(
        crate::store::schema::clear_open_unavailability_for_seat(&tx, "b", UtcMillis(2_000))
            .unwrap(),
        1
    );
    tx.execute("UPDATE seats SET unavailability_open=0 WHERE id='b'", [])
        .unwrap();
    assert_eq!(
        crate::store::schema::clear_open_unavailability_for_seat(&tx, "b", UtcMillis(2_000))
            .unwrap(),
        0
    );
    tx.commit().unwrap();
    let transitions_before: i64 = conn.query_row("SELECT count(*) FROM warning_jobs WHERE warning_id IN (SELECT open_warning_id FROM warning_conditions UNION SELECT clear_warning_id FROM warning_conditions)", [], |r| r.get(0)).unwrap();
    assert_eq!(transitions_before, 1);
    let mut next = send_request(vec!["c"]);
    next.operation = OperationId::new("op3");
    let blocked = send_prepared(
        &context,
        &mut conn,
        &next,
        &mut permit(&next),
        messages::MessageLimits::default(),
    )
    .unwrap_err();
    assert_eq!(blocked.code, crate::protocol::results::ErrorCode::StoreBusy);
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let close_job: String = conn
        .query_row(
            "SELECT id FROM work_jobs WHERE kind='warning_condition_close'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: crate::protocol::time::Cancellation::default(),
    };
    while crate::store::materialization::advance_work(
        &mut conn,
        &close_job,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let transitions: i64 = conn.query_row("SELECT count(*) FROM warning_jobs WHERE warning_id IN (SELECT open_warning_id FROM warning_conditions UNION SELECT clear_warning_id FROM warning_conditions)", [], |r| r.get(0)).unwrap();
    assert_eq!(transitions, 2);
    let mut retry = next.clone();
    retry.operation = OperationId::new("op4");
    send_prepared(
        &context,
        &mut conn,
        &retry,
        &mut permit(&retry),
        messages::MessageLimits::default(),
    )
    .unwrap();
    let transitions: Vec<(i64, Option<i64>)> = conn.prepare("SELECT opened_seq,cleared_seq FROM warning_conditions WHERE condition_kind='unavailable' ORDER BY ordinal").unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(transitions.len(), 2);
    assert!(transitions[0].0 < transitions[0].1.unwrap());
    assert!(transitions[0].1.unwrap() < transitions[1].0);
}

#[test]
fn send_does_not_start_timer_from_stale_registered_binding() {
    let (context, mut conn, _) = setup();
    conn.execute("UPDATE occupant_bindings SET host_boot='old',host_epoch=0,target_generation=0,registered_at=0 WHERE seat_id='b'", []).unwrap();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    let start = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap()
        .available_at;
    assert_eq!(start, None);
}

#[test]
fn current_binding_with_bumped_snapshot_generation_is_unavailable_and_warns_once() {
    let (context, mut conn, _) = setup();
    // setup() already gives seat b a live cooperative binding on pb and a
    // fresh observation of pb; register that binding (merge note: main's
    // fixture inserted both rows itself).
    conn.execute(
        "UPDATE occupant_bindings SET registered_at=0 WHERE seat_id='b' AND ended_at IS NULL",
        [],
    )
    .unwrap();
    // Control: the same binding is available while the observed structural
    // generation matches, so the bump below is the only thing that changes.
    assert_eq!(
        crate::store::schema::effective_registered_availability(&conn, "b", None).unwrap(),
        Some("cooperative_top_level".into())
    );
    conn.execute(
        "UPDATE observed_targets SET generation=2 WHERE instance_id='i' AND target_id='pb'",
        [],
    )
    .unwrap();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        _ => panic!(),
    };
    let receipt = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.available_at, None);
    let warnings: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.event_json LIKE '%\"seat\":\"b\"%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 1);
}

#[test]
fn failed_operation_commit_rolls_back_message_receipt_and_wake() {
    let (context, mut conn, _) = setup();
    conn.execute_batch("CREATE TRIGGER fail_op BEFORE INSERT ON operations BEGIN SELECT RAISE(ABORT,'injected commit failure'); END;").unwrap();
    let send = send_request(vec![]);
    assert!(
        send_prepared(
            &context,
            &mut conn,
            &send,
            &mut permit(&send),
            messages::MessageLimits::default(),
        )
        .is_err()
    );
    for table in ["messages", "receipts", "wake_work"] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn due_cursor_progresses_in_bounded_physical_slices_past_warned_rows() {
    let (context, mut conn, _) = setup();
    for n in 0..205 {
        let id = format!("m{n}");
        conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','a','x',0,?3)", params![id,n+1,100000+n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','b','pending',1,0,500)", [id.as_str()]).unwrap();
        let key = format!("overdue:receipt:{}:{}:b", id.len(), id);
        conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,event_json,decision_at,source_message_id,decision_seq) VALUES (?1,'i','t',?2,'warn',?3,'{}',500,?4,?5)", params![format!("w{n}"),n+1001,key,id,200000+n]).unwrap();
    }
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('last','i','t',500,'ordinary','a','x',0,300000)", []).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=300000 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('last','t','b','pending',1,0,500)", []).unwrap();
    conn.execute("UPDATE threads SET next_sequence=2000 WHERE id='t'", [])
        .unwrap();
    let mut cursor = receipts::ReceiptDueCursor::default();
    let mut warnings = 0;
    for _ in 0..14 {
        let page = receipts::scan_due(&context, &mut conn, 16, &mut cursor).unwrap();
        assert!(page.inspected <= 16);
        warnings += page.warnings;
    }
    assert_eq!(warnings, 1);
}

#[test]
fn target_change_at_decision_rejects_prepared_send() {
    let (context, mut conn, _) = setup();
    let send = send_request(vec![]);
    conn.execute("UPDATE seats SET target_id='rebound' WHERE id='a'", [])
        .unwrap();
    let result = send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    );
    assert!(result.is_err());
    let count: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn maximum_ack_batch_with_long_ids_fits_compact_info_event() {
    let (context, mut conn, _) = setup();
    let mut ids = Vec::new();
    for n in 0..100 {
        let id = MessageId::new(format!("m{n:03}{}", "x".repeat(115)));
        conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','a','x',0,?3)", params![id.as_str(),n+1,100000+n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','b','pending',300000)", [id.as_str()]).unwrap();
        ids.push(id);
    }
    conn.execute(
        "UPDATE host_instances SET decision_seq=100099 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE threads SET next_sequence=101 WHERE id='t'", [])
        .unwrap();
    let request = ack_request(ids);
    let result = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::Acknowledged(result) = result else {
        panic!()
    };
    assert_eq!(result.acknowledged.len(), 100);
    let info: i64 = conn
        .query_row("SELECT count(*) FROM messages WHERE kind='info'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(info, 1);
}

#[test]
fn wide_send_publishes_one_manifest_with_all_logical_recipients() {
    let (context, mut conn, _) = setup();
    for n in 0..205 {
        let seat = format!("wide-{n:03}");
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![seat, format!("pane-{n:03}")]).unwrap();
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat.as_str()],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat.as_str()]).unwrap();
    }
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let mut steps = 0;
    loop {
        steps += 1;
        match messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            crate::ports::DurableWorkAdmission { max_units: 16 },
        )
        .unwrap()
        {
            crate::ports::SendPreparationProgress::More { visited, .. } => assert!(visited <= 16),
            crate::ports::SendPreparationProgress::Ready { visited, .. } => {
                assert!(visited <= 16);
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(steps >= 13);
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    let logical: i64 = conn.query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1", [id.as_str()], |r| r.get(0)).unwrap();
    assert_eq!(logical, 206);
    let physical: i64 = conn
        .query_row(
            "SELECT count(*) FROM receipts WHERE message_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(physical, 0);
    assert!(
        crate::store::effective::effective_receipt(&conn, id.as_str(), "wide-204")
            .unwrap()
            .is_some()
    );
}

#[test]
fn manifest_receipt_can_be_acked_before_physical_projection() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            crate::ports::DurableWorkAdmission { max_units: 16 }
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    let ack = ack_request(vec![id.clone()]);
    let result = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &ack,
        &mut ack_permit(&ack),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::Acknowledged(result) = result else {
        panic!()
    };
    assert_eq!(result.acknowledged, vec![id.clone()]);
    let state: String = conn
        .query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "acked");
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::Acknowledged
    );
}

#[test]
fn due_scan_ignores_large_already_warned_history() {
    let (context, mut conn, _) = setup();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES ('old-warning','i','t',207,'warn','old-warning','{}',0,200000)",[]).unwrap();
    for n in 0..205 {
        let id = format!("old-{n}");
        conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','a','x',0,?3)",params![id,n+1,100000+n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,warning_message_id) VALUES (?1,'t','b','pending',1,0,500,'old-warning')",[id.as_str()]).unwrap();
    }
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('new','i','t',208,'ordinary','a','x',0,300000)",[]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=300000 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('new','t','b','pending',1,0,500)",[]).unwrap();
    conn.execute("UPDATE threads SET next_sequence=209 WHERE id='t'", [])
        .unwrap();
    let mut cursor = receipts::ReceiptDueCursor::default();
    let page = receipts::scan_due(&context, &mut conn, 16, &mut cursor).unwrap();
    assert_eq!(page.inspected, 1);
    assert_eq!(page.warnings, 1);
}

#[test]
fn stale_preparation_cleans_and_rebuilds_with_new_generation() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 1 };
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::More { .. }
    ));
    let first: String = conn
        .query_row("SELECT id FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    conn.execute(
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    assert!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .is_err()
    );
    let status: String = conn
        .query_row("SELECT status FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "discarded");
    let cleanup: String = conn
        .query_row(
            "SELECT status FROM work_jobs WHERE kind='preparation_cleanup' AND subject_id=?1",
            [first.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cleanup, "pending");
    let mut changed = request.clone();
    changed.body.push('!');
    assert_eq!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &changed,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    conn.execute(
        "DELETE FROM prepared_recipients WHERE preparation_id=?1",
        [first.as_str()],
    )
    .unwrap();
    conn.execute(
        "UPDATE work_jobs SET status='complete' WHERE kind='preparation_cleanup' AND subject_id=?1",
        [first.as_str()],
    )
    .unwrap();
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    let second: String = conn
        .query_row("SELECT id FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    assert_ne!(first, second);
    conn.execute(
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    assert!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .is_err()
    );
    let second_job: i64 = conn
        .query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='preparation_cleanup' AND subject_id=?1",
            [second.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(second_job, 1);
}

#[test]
fn first_anchor_makes_sparse_receipt_due_for_late_ack() {
    let (context, mut conn, clock) = setup();
    let mut request = send_request(vec![]);
    request.deadline_millis = Some(500);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1500,1,'verified')",[]).unwrap();
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',4,9000,1,'verified')",[]).unwrap();
    let effective = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap();
    assert_eq!(effective.available_at, Some(1500));
    assert_eq!(effective.deadline_at, Some(2000));
    clock.0.store(2000, Ordering::SeqCst);
    let ack = ack_request(vec![id.clone()]);
    receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &ack,
        &mut ack_permit(&ack),
    )
    .unwrap();
    let marker: String = conn
        .query_row(
            "SELECT warning_message_id FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let kind: String = conn
        .query_row(
            "SELECT kind FROM messages WHERE id=?1",
            [marker.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "warn");
    let replay = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &ack,
        &mut ack_permit(&ack),
    )
    .unwrap();
    assert!(matches!(
        replay,
        crate::protocol::results::CommandResult::Acknowledged(_)
    ));
    let warnings: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 1);
}

#[test]
fn manifest_warning_is_visible_before_worker_projection() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    let key = crate::store::effective::UnavailableWarningKey {
        instance: "i".into(),
        thread_id: "t".into(),
        affected_seat_id: "b".into(),
        unavailability_episode: 1,
    };
    let warning = crate::store::effective::effective_warning_by_key(&conn, &key)
        .unwrap()
        .unwrap();
    assert_eq!(warning.sequence, 2);
    assert_eq!(warning.source_message_id.as_deref(), Some(id.as_str()));
    let physical: i64 = conn
        .query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(physical, 0);
    let next: i64 = conn
        .query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(next, 3);
}

#[test]
fn sparse_pending_unwarned_due_scan_marks_once() {
    let (context, mut conn, clock) = setup();
    let mut request = send_request(vec![]);
    request.deadline_millis = Some(500);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1500,1,'verified')",[]).unwrap();
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1500,2000)",[id.as_str()]).unwrap();
    clock.0.store(2000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let first = receipts::scan_due(&context, &mut conn, 16, &mut cursor).unwrap();
    assert_eq!(first.warnings, 1);
    let marker: String = conn
        .query_row(
            "SELECT warning_message_id FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!marker.is_empty());
    let second = receipts::scan_due(&context, &mut conn, 16, &mut cursor).unwrap();
    assert_eq!(second.warnings, 0);
}

#[test]
fn publish_rechecks_scalar_eligibility_after_decision_callback() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    conn.execute("UPDATE host_instances SET send_eligibility_revision=send_eligibility_revision+1 WHERE id='i'",[]).unwrap();
    let result = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    );
    // The captured eligibility revision no longer matches the live one: the
    // publish-time snapshot comparison refuses with Conflict, not a generic error.
    assert_eq!(
        result.unwrap_err().code,
        crate::protocol::results::ErrorCode::Conflict
    );
    let messages: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(messages, 0);
}

#[test]
fn publish_rejects_changed_structural_target_generation() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    conn.execute("UPDATE seats SET target_generation=2 WHERE id='a'", [])
        .unwrap();
    let result = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    );
    assert!(result.is_err());
    let published: i64 = conn
        .query_row("SELECT count(*) FROM send_manifests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(published, 0);
}

#[test]
fn ack_records_binding_generation_separately_from_target_generation() {
    let (context, mut conn, _) = setup();
    conn.execute(
        "UPDATE seats SET generation=3,target_generation=2 WHERE id='b'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('m','i','t',1,'ordinary','a','x',0,100000)",[]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=100000 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','b','pending',300000)",[]).unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    conn.execute(
        "UPDATE observed_targets SET generation=2 WHERE instance_id='i' AND target_id='pb'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE occupant_bindings SET generation=3,target_generation=2 WHERE seat_id='b'",
        [],
    )
    .unwrap();
    let mut request = ack_request(vec![MessageId::new("m")]);
    request.claim.binding_generation = 3;
    let digest = crate::store::schema::canonical_digest(&receipts::ack_payload(&request)).unwrap();
    let mut grant = cooperative_permit(
        &request.claim,
        &request.operation,
        ObligationRef::CheckIn(SeatId::new("b")),
        digest,
        (2, 0),
    );
    receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut grant,
    )
    .unwrap();
    let generation: i64 = conn
        .query_row(
            "SELECT ack_generation FROM receipts WHERE message_id='m' AND seat_id='b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(generation, 3);
}

#[test]
fn inbox_display_ack_records_claim_and_only_settles_addressed_pending_agent_receipt() {
    let (context, mut conn, _) = setup();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('displayed','i','t',1,'ordinary','a','read body',0,2),('foreign','i','t',2,'ordinary','a','other body',0,3)", []).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('displayed','t','b','pending',300000),('foreign','t','c','pending',300000)", []).unwrap();
    conn.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
        .unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=3 WHERE id='i'", [])
        .unwrap();
    let request = ack_request(vec![MessageId::new("displayed")]);
    let digest =
        crate::store::schema::canonical_digest(&receipts::ack_displayed_payload(&request)).unwrap();
    let mut grant = cooperative_permit(
        &request.claim,
        &request.operation,
        ObligationRef::CheckIn(SeatId::new("b")),
        digest,
        (1, 0),
    );
    receipts::ack_displayed(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut grant,
    )
    .unwrap();
    let observation: String = conn
        .query_row(
            "SELECT ack_observation FROM receipts WHERE message_id='displayed' AND seat_id='b'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["provenance"], "cooperative_top_level");
    assert_eq!(
        observation["action_provenance"],
        "cooperative_inbox_display"
    );
    let foreign_state: String = conn
        .query_row(
            "SELECT state FROM receipts WHERE message_id='foreign' AND seat_id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foreign_state, "pending");

    let mut foreign = ack_request(vec![MessageId::new("foreign")]);
    foreign.operation = OperationId::new("foreign-display-op");
    let digest =
        crate::store::schema::canonical_digest(&receipts::ack_displayed_payload(&foreign)).unwrap();
    let mut grant = cooperative_permit(
        &foreign.claim,
        &foreign.operation,
        ObligationRef::CheckIn(SeatId::new("b")),
        digest,
        (1, 0),
    );
    assert!(
        receipts::ack_displayed(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &foreign,
            &mut grant
        )
        .is_err()
    );
}

#[test]
fn published_send_freezes_available_recipient_duration() {
    let (context, mut conn, _) = setup();
    conn.execute(
        "UPDATE occupant_bindings SET registered_at=900 WHERE seat_id='b'",
        [],
    )
    .unwrap();
    let mut request = send_request(vec![]);
    request.deadline_millis = Some(750);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits {
            receipt_duration_ms: 9000,
            body_bytes: 65536,
        },
        &budget,
        admission,
    )
    .unwrap();
    let id = match messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    conn.execute("UPDATE host_instances SET duration_config_revision=duration_config_revision+1 WHERE id='i'",[]).unwrap();
    let receipt = crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.available_at, Some(1000));
    assert_eq!(receipt.deadline_at, Some(1750));
    let frozen:i64=conn.query_row("SELECT pr.frozen_duration_ms FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='b'",[id.as_str()],|r|r.get(0)).unwrap();
    assert_eq!(frozen, 750);
    let provenance:String=conn.query_row("SELECT pr.availability_provenance FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='b'",[id.as_str()],|r|r.get(0)).unwrap();
    assert_eq!(provenance, "cooperative_top_level");
    let observation: String = conn
        .query_row(
            "SELECT native_observation FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["binding_generation"], 1);
    assert_eq!(observation["target_generation"], 1);
}

#[test]
fn later_send_reuses_first_published_unavailability_warning() {
    let (context, mut conn, _) = setup();
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    for operation in ["op", "second"] {
        let mut request = send_request(vec![]);
        request.operation = OperationId::new(operation);
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission,
        )
        .unwrap();
        messages::publish_send(
            &context,
            &mut conn,
            &request,
            &mut permit(&request),
            &budget,
            || messages::MAX_BODY_BYTES,
        )
        .unwrap();
    }
    let key = crate::store::effective::UnavailableWarningKey {
        instance: "i".into(),
        thread_id: "t".into(),
        affected_seat_id: "b".into(),
        unavailability_episode: 1,
    };
    let warning = crate::store::effective::effective_warning_by_key(&conn, &key)
        .unwrap()
        .unwrap();
    assert_eq!(warning.sequence, 2);
    let warning_count: i64 = conn
        .query_row("SELECT sum(warning_count) FROM send_manifests", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(warning_count, 1);
    let next: i64 = conn
        .query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(next, 4);
}

#[test]
fn invalid_explicit_recipient_discards_hidden_preparation() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec!["d"]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            crate::ports::DurableWorkAdmission { max_units: 1 }
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::More { .. }
    ));
    let error = messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        crate::ports::DurableWorkAdmission { max_units: 16 },
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    let (prep, status): (String, String) = conn
        .query_row(
            "SELECT id,status FROM send_preparations WHERE operation_key='op'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "discarded");
    let job: String = conn
        .query_row(
            "SELECT status FROM work_jobs WHERE kind='preparation_cleanup' AND subject_id=?1",
            [prep.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(job, "pending");
    let published: i64 = conn
        .query_row("SELECT count(*) FROM send_manifests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(published, 0);
}

#[test]
fn abandon_only_discards_expected_unpublished_generation() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 1 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    let original: String = conn
        .query_row("SELECT id FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    assert!(messages::abandon_send_preparation(&mut conn, &original).unwrap());
    assert!(!messages::abandon_send_preparation(&mut conn, &original).unwrap());
    conn.execute(
        "DELETE FROM prepared_recipients WHERE preparation_id=?1",
        [original.as_str()],
    )
    .unwrap();
    conn.execute(
        "DELETE FROM prepared_unavailable_warnings WHERE preparation_id=?1",
        [original.as_str()],
    )
    .unwrap();
    conn.execute(
        "UPDATE work_jobs SET status='complete' WHERE kind='preparation_cleanup' AND subject_id=?1",
        [original.as_str()],
    )
    .unwrap();
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    let successor: String = conn
        .query_row("SELECT id FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    assert_ne!(original, successor);
    assert!(!messages::abandon_send_preparation(&mut conn, &original).unwrap());
    let status: String = conn
        .query_row(
            "SELECT status FROM send_preparations WHERE id=?1",
            [successor.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "building");
}

#[test]
fn sequence_exhaustion_rolls_back_manifest_publication() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        crate::ports::DurableWorkAdmission { max_units: 16 },
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [i64::MAX],
    )
    .unwrap();
    let result = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    );
    assert_eq!(
        result.unwrap_err().code,
        crate::protocol::results::ErrorCode::SequenceExhausted
    );
    for table in ["messages", "send_manifests", "operations"] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let next: i64 = conn
        .query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(next, i64::MAX);
}

#[test]
fn send_rejects_receipt_deadline_overflow_at_publication() {
    let (context, mut conn, clock) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        crate::ports::DurableWorkAdmission { max_units: 16 },
    )
    .unwrap();
    clock.0.store(i64::MAX - 1, Ordering::SeqCst);
    let error = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || messages::MAX_BODY_BYTES,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    let manifests: i64 = conn
        .query_row("SELECT count(*) FROM send_manifests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(manifests, 0);
}

#[test]
fn changed_duration_between_steps_discards_preparation() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 2 };
    messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    let changed = messages::MessageLimits {
        receipt_duration_ms: 30_000,
        body_bytes: messages::MAX_BODY_BYTES,
    };
    let error =
        messages::prepare_send_step(&context, &mut conn, &request, changed, &budget, admission)
            .unwrap_err();
    assert_eq!(error.code, crate::protocol::results::ErrorCode::Conflict);
    let status: String = conn
        .query_row("SELECT status FROM send_preparations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "discarded");
}

#[test]
fn publication_rechecks_reduced_body_limit_and_replay_keeps_committed_result() {
    let (context, mut conn, _) = setup();
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    let five = messages::MessageLimits {
        receipt_duration_ms: 750,
        body_bytes: 5,
    };
    let prep_id =
        match messages::prepare_send_step(&context, &mut conn, &request, five, &budget, admission)
            .unwrap()
        {
            crate::ports::SendPreparationProgress::Ready { preparation_id, .. } => preparation_id,
            other => panic!("{other:?}"),
        };
    let staged_message_id =
        crate::protocol::ids::send_message_id_for_preparation(&prep_id).unwrap();
    let effective_body_limit = AtomicUsize::new(5);
    effective_body_limit.store(4, Ordering::SeqCst);
    let error = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || effective_body_limit.load(Ordering::SeqCst),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    for table in [
        "messages",
        "send_manifests",
        "operations",
        "receipt_state",
        "warning_jobs",
        "work_jobs",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert!(
        crate::store::effective::effective_receipt(&conn, &staged_message_id, "b")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        conn.query_row(
            "SELECT decision_seq FROM host_instances WHERE id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    effective_body_limit.store(5, Ordering::SeqCst);
    let original = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || effective_body_limit.load(Ordering::SeqCst),
    )
    .unwrap();
    let replay = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget,
        || panic!("committed replay read current limit"),
    )
    .unwrap();
    assert_eq!(replay, original);
}

#[test]
fn failed_ack_event_rolls_back_settlement_warning_and_scoped_revisions() {
    let (context, mut conn, clock) = setup();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1000,1500)",[id.as_str()]).unwrap();
    clock.0.store(2000, Ordering::SeqCst);
    let directory_before = filter_revision(&conn, "directory", "t");
    let inbox_before = filter_revision(&conn, "inbox", "b");
    let timeline_before: i64 = conn
        .query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_ack_info BEFORE INSERT ON messages WHEN NEW.kind='info' BEGIN SELECT RAISE(FAIL, 'injected ACK event failure'); END;").unwrap();
    let request = ack_request(vec![id.clone()]);
    assert!(
        receipts::ack(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &request,
            &mut ack_permit(&request),
        )
        .is_err()
    );
    assert_eq!(filter_revision(&conn, "directory", "t"), directory_before);
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_before);
    assert_eq!(
        conn.query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        timeline_before
    );
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, id.as_str(), "b")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::Pending
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn ack_last_pending_receipt_for_left_seat_invalidates_thread_directory() {
    let (context, mut conn, _) = setup();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('left-message','i','t',1,'ordinary','a','x',0,100000)",[]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=100000 WHERE id='i'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('left-message','t','b','pending',300000)",[]).unwrap();
    conn.execute(
        "UPDATE memberships SET state='left' WHERE thread_id='t' AND seat_id='b'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE membership_intervals SET left_seq=2 WHERE thread_id='t' AND seat_id='b'",
        [],
    )
    .unwrap();
    let before = filter_revision(&conn, "directory", "t");
    let inbox_before = filter_revision(&conn, "inbox", "b");
    let request = ack_request(vec![MessageId::new("left-message")]);
    conn.execute_batch("CREATE TRIGGER fail_physical_ack_info BEFORE INSERT ON messages WHEN NEW.kind='info' BEGIN SELECT RAISE(ABORT,'injected physical ACK event failure'); END;").unwrap();
    assert!(
        receipts::ack(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &request,
            &mut ack_permit(&request),
        )
        .is_err()
    );
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_before);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM receipts WHERE message_id='left-message' AND seat_id='b'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    conn.execute_batch("DROP TRIGGER fail_physical_ack_info;")
        .unwrap();
    receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    assert_eq!(filter_revision(&conn, "directory", "t"), before + 1);
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_before + 1);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE thread_id='t' AND seat_id='b' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn already_acknowledged_new_operation_does_not_invalidate_directory() {
    let (context, mut conn, _) = setup();
    let send = send_request(vec![]);
    let id = match send_prepared(
        &context,
        &mut conn,
        &send,
        &mut permit(&send),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    let first = ack_request(vec![id.clone()]);
    receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &first,
        &mut ack_permit(&first),
    )
    .unwrap();
    let revision = filter_revision(&conn, "directory", "t");
    let inbox_revision = filter_revision(&conn, "inbox", "b");
    let event_count: i64 = conn
        .query_row("SELECT count(*) FROM messages WHERE kind='info'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let mut second = ack_request(vec![id.clone()]);
    second.operation = OperationId::new("ack-again");
    let result = receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &second,
        &mut ack_permit(&second),
    )
    .unwrap();
    let crate::protocol::results::CommandResult::Acknowledged(result) = result else {
        panic!()
    };
    assert!(result.acknowledged.is_empty());
    assert_eq!(result.already_acknowledged, vec![id]);
    assert_eq!(filter_revision(&conn, "directory", "t"), revision);
    assert_eq!(filter_revision(&conn, "inbox", "b"), inbox_revision);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='info'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        event_count
    );
}

#[test]
fn foreground_write_between_preparation_quanta_and_failed_publication_stays_hidden_after_reopen() {
    let (context, mut conn, _) = setup();
    for n in 0..34 {
        let seat = format!("interleave-{n:02}");
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", params![seat, format!("pane-{n:02}")]).unwrap();
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat.as_str()],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",[seat.as_str()]).unwrap();
    }
    let request = send_request(vec![]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    let first = messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget,
        admission,
    )
    .unwrap();
    assert!(matches!(
        first,
        crate::ports::SendPreparationProgress::More {
            visited: 1..=16,
            ..
        }
    ));
    let foreground = context.open_writer().unwrap();
    foreground.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('foreground','i','fg','goal',0,0)",[]).unwrap();
    let prep_id = loop {
        match messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget,
            admission,
        )
        .unwrap()
        {
            crate::ports::SendPreparationProgress::More { visited, .. } => assert!(visited <= 16),
            crate::ports::SendPreparationProgress::Ready { preparation_id, .. } => {
                break preparation_id;
            }
            other => panic!("{other:?}"),
        }
    };
    let staged: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_recipients WHERE preparation_id=?1",
            [prep_id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(staged, 35);
    let message_id = crate::protocol::ids::send_message_id_for_preparation(&prep_id).unwrap();
    assert!(
        crate::store::effective::effective_receipt(&conn, &message_id, "b")
            .unwrap()
            .is_none()
    );
    conn.execute("UPDATE host_instances SET send_eligibility_revision=send_eligibility_revision+1 WHERE id='i'",[]).unwrap();
    assert_eq!(
        messages::publish_send(
            &context,
            &mut conn,
            &request,
            &mut permit(&request),
            &budget,
            || messages::MAX_BODY_BYTES
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::Conflict
    );
    let reopened = context.open_writer().unwrap();
    assert!(
        crate::store::effective::effective_receipt(&reopened, &message_id, "b")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reopened
            .query_row(
                "SELECT count(*) FROM messages WHERE id=?1",
                [message_id.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .query_row(
                "SELECT count(*) FROM send_manifests WHERE preparation_id=?1",
                [prep_id.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        reopened
            .query_row(
                "SELECT count(*) FROM threads WHERE id='foreground'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

/// Publish one send from `a` through the real prepare/publish writers.
fn publish(
    context: &StoreContext,
    conn: &mut Connection,
    operation: &str,
    explicit: Vec<&str>,
) -> MessageId {
    let mut request = send_request(explicit);
    request.operation = OperationId::new(operation);
    match send_prepared(
        context,
        conn,
        &request,
        &mut permit(&request),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    }
}

/// Every (message, seat) in thread `t` that the canonical rule rates Pending,
/// found by evaluating `effective_receipt` over every staged and physical
/// receipt row the thread has ever held (no index restriction at all).
fn canonical_pending(
    conn: &Connection,
) -> std::collections::BTreeMap<(String, String), crate::store::effective::EffectiveReceipt> {
    let mut pairs = conn
        .prepare(
            "SELECT sm.message_id,pr.seat_id FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE pr.thread_id='t' \
             UNION SELECT message_id,seat_id FROM receipts WHERE thread_id='t'",
        )
        .unwrap();
    let pairs: Vec<(String, String)> = pairs
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    pairs
        .into_iter()
        .filter_map(|(message, seat)| {
            let receipt = crate::store::effective::effective_receipt(conn, &message, &seat)
                .unwrap()
                .unwrap();
            (receipt.state == crate::store::effective::EffectiveReceiptState::Pending)
                .then_some(((message, seat), receipt))
        })
        .collect()
}

/// The thread-scoped receipt scan run to completion, Pending items only, and
/// the number of indexed candidates it examined.
fn scanned_pending(
    conn: &Connection,
) -> (
    std::collections::BTreeMap<(String, String), crate::store::effective::EffectiveReceipt>,
    u64,
) {
    use crate::store::effective::{
        EffectiveReceiptState, ReceiptScanScope, scan_effective_receipts,
    };
    let mut found = std::collections::BTreeMap::new();
    let mut visited = 0u64;
    let mut position = None;
    loop {
        let slice =
            scan_effective_receipts(conn, &ReceiptScanScope::Thread("t".into()), position, 100)
                .unwrap();
        visited += u64::from(slice.visited);
        for receipt in slice.items {
            if receipt.state == EffectiveReceiptState::Pending {
                let key = (receipt.message_id.clone(), receipt.seat_id.clone());
                assert!(found.insert(key, receipt).is_none(), "duplicate candidate");
            }
        }
        if !slice.has_more {
            return (found, visited);
        }
        position = Some(slice.position);
    }
}

// Digest fix3 step 2, superset proof. The thread-scoped manifest candidate
// that check-in's inbox walks is restricted to the v8 pending projection; this
// proves that projection loses nothing the canonical `effective_receipt`
// evaluation needs. Written through the real send/ACK writers: an ACKed
// receipt, an unavailable-at-send receipt whose first ordered availability
// anchor (not the later one) fixes `available_at`, the same receipt after the
// due path stores a pending sparse `receipt_state` marker, an invited seat
// that never became available (`available_at` None), receipts the send
// worker has projected (pending `receipt_state` rows), a seat retired after
// the send, and an unpublished staged recipient. Every canonical Pending receipt is in the projection and the
// scan returns it with identical fields (anchor, deadline, marker included).
// Kills: a settlement trigger without its `state!='pending'` guard (the
// pending sparse marker would drop the receipt from the projection), a
// restriction that treats any `receipt_state` row as settled, and a
// restriction that requires availability (`eligible_at_snapshot=1` or an
// anchor) before a receipt is pending.
#[test]
fn pending_only_thread_candidates_cover_every_canonical_pending_receipt() {
    let (context, mut conn, _) = setup();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','d','invited')",
        [],
    )
    .unwrap();
    let acked = publish(&context, &mut conn, "send-acked", vec![]);
    let anchored = publish(&context, &mut conn, "send-anchored", vec![]);
    let invited = publish(&context, &mut conn, "send-invited", vec!["c"]);
    let retired = publish(&context, &mut conn, "send-retired", vec!["d"]);
    // Unpublished: staged recipients exist, no manifest.
    let mut staged = send_request(vec![]);
    staged.operation = OperationId::new("send-staged");
    assert!(matches!(
        messages::prepare_send_step(
            &context,
            &mut conn,
            &staged,
            messages::MessageLimits::default(),
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(1000),
                cancellation: Default::default(),
            },
            crate::ports::DurableWorkAdmission { max_units: 16 },
        )
        .unwrap(),
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    let eligible: i64 = conn
        .query_row(
            "SELECT count(*) FROM prepared_recipients WHERE eligible_at_snapshot=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(eligible, 0, "every recipient is unavailable at send");
    // b's first ordered availability anchor after the sends, then a later one.
    let seq: i64 = conn
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',?1,1500,1,'verified')",[seq+1]).unwrap();
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',?1,9000,1,'verified')",[seq+2]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [seq + 2],
    )
    .unwrap();
    let ack = ack_request(vec![acked.clone()]);
    receipts::ack(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &ack,
        &mut ack_permit(&ack),
    )
    .unwrap();
    // The due path stores a pending sparse marker for the anchored receipt.
    let deadline = crate::store::effective::effective_receipt(&conn, anchored.as_str(), "b")
        .unwrap()
        .unwrap()
        .deadline_at
        .unwrap();
    let tx = conn.transaction().unwrap();
    let overdue = crate::store::schema::record_overdue_if_pending(
        &tx,
        &ObligationRef::Receipt {
            message: anchored.clone(),
            seat: SeatId::new("b"),
        },
        &crate::ports::TimeBasis::Decision,
        UtcMillis(deadline + 1),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(overdue.inserted);
    let marker: (String, Option<String>) = conn
        .query_row(
            "SELECT state,warning_message_id FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [anchored.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(marker.0, "pending");
    assert!(marker.1.is_some());
    // The send worker projects the invited send's receipts: production
    // projection writes pending `receipt_state` rows (no `receipts` rows).
    let job = format!("work:send:{}", invited.as_str());
    while crate::store::materialization::advance_work(
        &mut conn,
        &job,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &TestClock(AtomicI64::new(1000)),
    )
    .unwrap()
    .has_more
    {}
    let physical: i64 = conn
        .query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1 AND state='pending'",
            [invited.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(physical, 2, "b and c projected");
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=5000,retired_seq=?1 WHERE id='d'",
        [seq + 3],
    )
    .unwrap();

    let canonical = canonical_pending(&conn);
    let keys: Vec<(&str, &str)> = canonical
        .keys()
        .map(|(m, s)| (m.as_str(), s.as_str()))
        .collect();
    let mut expected = vec![
        (anchored.as_str(), "b"),
        (invited.as_str(), "b"),
        (invited.as_str(), "c"),
        (retired.as_str(), "b"),
    ];
    expected.sort();
    assert_eq!(keys, expected);
    let first_anchor = &canonical[&(anchored.as_str().to_owned(), "b".to_owned())];
    assert_eq!(first_anchor.available_at, Some(1500));
    assert_eq!(first_anchor.warning_message_id, marker.1);
    assert_eq!(
        canonical[&(invited.as_str().to_owned(), "c".to_owned())].available_at,
        None
    );
    // Superset: every canonical pending manifest receipt is in the projection
    // at its staged thread and ordinal.
    for (message, seat) in canonical.keys() {
        let projected: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts d JOIN send_manifests sm ON sm.preparation_id=d.preparation_id JOIN prepared_recipients pr ON pr.preparation_id=d.preparation_id AND pr.seat_id=d.seat_id WHERE sm.message_id=?1 AND d.seat_id=?2 AND d.thread_id=pr.thread_id AND d.ordinal=pr.ordinal)",
                params![message, seat],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            projected,
            "{message}/{seat} missing from the pending projection"
        );
    }
    let (scanned, _) = scanned_pending(&conn);
    assert_eq!(scanned, canonical);
}

// ---------------------------------------------------------------------------
// Deadline extension (spec §8, ht-1ip.7).
// ---------------------------------------------------------------------------

fn catch_up_row(
    conn: &Connection,
    seat: &str,
    thread: &str,
    active: bool,
    extension_until: Option<i64>,
) {
    let (state, reason, ended): (&str, Option<&str>, Option<i64>) = if active {
        ("active", None, None)
    } else {
        ("ended", Some("ready"), Some(10))
    };
    conn.execute(
        "INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,extension_until,state,end_reason,ended_at) VALUES (?1,?2,0,1,'e',0,?3,?4,?5,?6)",
        params![seat, thread, extension_until, state, reason, ended],
    )
    .unwrap();
}

fn extension_of(conn: &Connection, seat: &str, thread: &str) -> Option<i64> {
    conn.query_row(
        "SELECT extension_until FROM catch_up WHERE seat_id=?1 AND thread_id=?2",
        params![seat, thread],
        |r| r.get(0),
    )
    .unwrap()
}

fn receipt_view(
    seat: &str,
    thread: &str,
    deadline_at: Option<i64>,
) -> crate::store::effective::EffectiveReceipt {
    use crate::store::effective::{EffectiveReceipt, EffectiveReceiptState, ReceiptSource};
    EffectiveReceipt {
        source: ReceiptSource::Physical,
        message_id: "m".into(),
        thread_id: thread.into(),
        seat_id: seat.into(),
        sequence: 1,
        source_ordinal: 1,
        decision_seq: Some(1),
        decision_at: 0,
        frozen_duration_ms: 1_000,
        state: EffectiveReceiptState::Pending,
        available_at: Some(0),
        deadline_at,
        warning_message_id: None,
        ack_actor_seat_id: None,
        ack_generation: None,
        ack_observation: None,
        acked_at: None,
        retired_at: None,
    }
}

#[test]
fn no_row_means_frozen() {
    let (_context, conn, _clock) = setup();
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", Some(1000))).unwrap(),
        Some(1000)
    );
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", None)).unwrap(),
        None
    );
    // A row without an extension is also the frozen deadline.
    catch_up_row(&conn, "b", "t", true, None);
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", Some(1000))).unwrap(),
        Some(1000)
    );
}

#[test]
fn active_row_extends() {
    let (_context, conn, _clock) = setup();
    catch_up_row(&conn, "b", "t", true, Some(5000));
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", Some(1000))).unwrap(),
        Some(5000)
    );
    // Another seat or another thread is untouched.
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("a", "t", Some(1000))).unwrap(),
        Some(1000)
    );
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t2", Some(1000))).unwrap(),
        Some(1000)
    );
}

#[test]
fn frozen_later_wins() {
    let (_context, conn, _clock) = setup();
    catch_up_row(&conn, "b", "t", true, Some(800));
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", Some(1000))).unwrap(),
        Some(1000)
    );
}

#[test]
fn entry_sets_entry_plus_p99_never_lowering() {
    let (_context, mut conn, _clock) = setup();
    catch_up_row(&conn, "b", "t", true, None);
    let tx = conn.transaction().unwrap();
    receipts::extension_on_entry(
        &tx,
        &SeatId::new("b"),
        &ThreadId::new("t"),
        UtcMillis(100),
        90,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(extension_of(&conn, "b", "t"), Some(190));
    // A later extension is never lowered by an earlier entry.
    conn.execute("UPDATE catch_up SET extension_until=900", [])
        .unwrap();
    let tx = conn.transaction().unwrap();
    receipts::extension_on_entry(
        &tx,
        &SeatId::new("b"),
        &ThreadId::new("t"),
        UtcMillis(100),
        90,
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(extension_of(&conn, "b", "t"), Some(900));
}

#[test]
fn progress_extends_every_active_seat_on_the_thread() {
    let (_context, mut conn, _clock) = setup();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','i','topic','goal',0,0)", []).unwrap();
    catch_up_row(&conn, "a", "t", true, Some(150));
    catch_up_row(&conn, "b", "t", true, None);
    catch_up_row(&conn, "c", "t", false, Some(40));
    catch_up_row(&conn, "a", "t2", true, Some(150));
    let tx = conn.transaction().unwrap();
    receipts::extension_on_progress(&tx, &ThreadId::new("t"), UtcMillis(200), 90).unwrap();
    tx.commit().unwrap();
    assert_eq!(extension_of(&conn, "a", "t"), Some(290));
    assert_eq!(extension_of(&conn, "b", "t"), Some(290));
    assert_eq!(extension_of(&conn, "c", "t"), Some(40));
    assert_eq!(extension_of(&conn, "a", "t2"), Some(150));
}

#[test]
fn exit_sets_now_plus_grace() {
    let (_context, mut conn, _clock) = setup();
    catch_up_row(&conn, "b", "t", false, Some(9_999));
    catch_up_row(&conn, "a", "t", true, Some(9_999));
    let tx = conn.transaction().unwrap();
    receipts::extension_on_exit(
        &tx,
        &SeatId::new("b"),
        &ThreadId::new("t"),
        UtcMillis(300),
        60,
    )
    .unwrap();
    tx.commit().unwrap();
    // Set, not raised: the exit grace replaces the longer progress window.
    assert_eq!(extension_of(&conn, "b", "t"), Some(360));
    assert_eq!(extension_of(&conn, "a", "t"), Some(9_999));
}

/// A pending physical receipt for seat `b` on thread `t`: message `id`,
/// frozen deadline `deadline`.
fn legacy_receipt(conn: &Connection, id: &str, thread: &str, sequence: i64, deadline: i64) {
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i',?2,?3,'ordinary','a','x',0,(SELECT COALESCE(MAX(decision_seq),1)+1 FROM messages))", params![id, thread, sequence]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=MAX(decision_seq,(SELECT MAX(decision_seq) FROM messages))",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=MAX(next_sequence,?2) WHERE id=?1",
        params![thread, sequence + 1],
    )
    .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,?2,'b','pending',500,0,?3)", params![id, thread, deadline]).unwrap();
}

/// A manifest-era receipt for seat `b` on thread `t` (a `receipt_state` row
/// only), frozen deadline 2000.
fn manifest_receipt(context: &StoreContext, conn: &mut Connection) -> MessageId {
    let mut request = send_request(vec![]);
    request.deadline_millis = Some(500);
    let id = match send_prepared(
        context,
        conn,
        &request,
        &mut permit(&request),
        messages::MessageLimits::default(),
    )
    .unwrap()
    {
        crate::protocol::results::CommandResult::MessageSent(id) => id,
        other => panic!("{other:?}"),
    };
    conn.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('b',3,1500,1,'verified')",[]).unwrap();
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'b','pending',1500,2000)",[id.as_str()]).unwrap();
    id
}

fn warning_count(conn: &Connection, message: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id=?1",
        [message],
        |r| r.get(0),
    )
    .unwrap()
}

fn due_phase(context: &StoreContext, conn: &mut Connection) -> u16 {
    let mut cursor = receipts::ReceiptDueCursor::default();
    let due = receipts::scan_due(context, conn, 10, &mut cursor).unwrap();
    let lapses = receipts::scan_extension_lapses(context, conn, &mut cursor).unwrap();
    due.warnings + lapses.warnings
}

#[test]
fn extended_legacy_receipt_is_skipped_until_the_extension_lapses() {
    let (context, mut conn, clock) = setup();
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", true, Some(5000));
    clock.0.store(2000, Ordering::SeqCst);
    // The frozen deadline passed during the extension: nothing fires.
    assert_eq!(due_phase(&context, &mut conn), 0);
    assert_eq!(warning_count(&conn, "ml"), 0);
    // The lapse is found by the extension recheck even when the due scan had
    // already moved past the receipt: run only the recheck.
    clock.0.store(5000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let lapses = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert_eq!(lapses.warnings, 1);
    assert_eq!(warning_count(&conn, "ml"), 1);
    assert_eq!(due_phase(&context, &mut conn), 0);
    assert_eq!(warning_count(&conn, "ml"), 1);
}

#[test]
fn extended_manifest_receipt_is_skipped_until_the_extension_lapses() {
    let (context, mut conn, clock) = setup();
    let id = manifest_receipt(&context, &mut conn);
    catch_up_row(&conn, "b", "t", true, Some(5000));
    clock.0.store(2000, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 0);
    assert_eq!(warning_count(&conn, id.as_str()), 0);
    clock.0.store(4999, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 0);
    clock.0.store(5000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let lapses = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert_eq!(lapses.warnings, 1);
    assert_eq!(warning_count(&conn, id.as_str()), 1);
    assert_eq!(due_phase(&context, &mut conn), 0);
    assert_eq!(warning_count(&conn, id.as_str()), 1);
}

#[test]
fn due_phase_warns_exactly_once_when_the_extension_lapses() {
    let (context, mut conn, clock) = setup();
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", true, Some(5000));
    clock.0.store(4999, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 0);
    clock.0.store(5000, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 1);
    assert_eq!(due_phase(&context, &mut conn), 0);
    assert_eq!(warning_count(&conn, "ml"), 1);
}

/// ht-hqg: a stall and re-entry cycle grants no fresh entry extension, so the
/// overdue warning fires on the frozen deadline.
#[test]
fn warning_fires_on_the_frozen_deadline_after_stall_and_reentry() {
    use crate::protocol::summary::SummarySettings;
    use crate::store::catch_up;
    let (context, mut conn, clock) = setup();
    legacy_receipt(&conn, "ml", "t", 1, 200_000);
    let (seat, thread) = (SeatId::new("b"), ThreadId::new("t"));
    let execution = ExecutionId::new("00000000-0000-4000-8000-0000000000bb");
    let enter = |conn: &mut Connection, now: i64| {
        let tx = conn.transaction().unwrap();
        catch_up::enter_or_keep(
            &tx,
            &catch_up::CatchUpEntry {
                seat: &seat,
                thread: &thread,
                frontier_seq: 1,
                binding_generation: 1,
                execution: &execution,
                now: UtcMillis(now),
            },
            &SummarySettings::default(),
        )
        .unwrap();
        tx.commit().unwrap();
    };
    // First entry: cold p99 (90 s) extends to 91_000, before the frozen 200_000.
    enter(&mut conn, 1_000);
    assert_eq!(extension_of(&conn, "b", "t"), Some(91_000));
    let tx = conn.transaction().unwrap();
    assert_eq!(catch_up::stall_scan(&tx, UtcMillis(95_000), 10).unwrap(), 1);
    tx.commit().unwrap();
    // Re-entry at 150_000 would have extended to 240_000; it grants nothing.
    enter(&mut conn, 150_000);
    assert_eq!(extension_of(&conn, "b", "t"), Some(91_000));
    assert_eq!(
        receipts::effective_deadline(&conn, &receipt_view("b", "t", Some(200_000))).unwrap(),
        Some(200_000)
    );
    clock.0.store(199_999, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 0);
    clock.0.store(200_000, Ordering::SeqCst);
    assert_eq!(due_phase(&context, &mut conn), 1);
    assert_eq!(warning_count(&conn, "ml"), 1);
}

#[test]
fn recheck_is_idempotent_after_restart() {
    let (context, mut conn, clock) = setup();
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", false, Some(1800));
    clock.0.store(2000, Ordering::SeqCst);
    for _ in 0..2 {
        // Watermark None each time, as after a restart.
        let mut cursor = receipts::ReceiptDueCursor::default();
        assert_eq!(cursor.extension, receipts::ExtensionLapseCursor::default());
        receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    }
    assert_eq!(warning_count(&conn, "ml"), 1);
}

#[test]
fn recheck_walks_the_extension_index_in_bounded_pages_with_a_watermark() {
    let (context, mut conn, clock) = setup();
    // 100 lapsed rows on other threads fill the first page; the row that
    // matters is the 101st by extension_until.
    for n in 0..100 {
        let thread = format!("x{n}");
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [&thread]).unwrap();
        catch_up_row(&conn, "a", &thread, false, Some(100 + n));
    }
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", false, Some(1800));
    clock.0.store(2000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let first = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert!(first.more);
    assert_eq!(first.warnings, 0);
    assert_eq!(
        cursor.extension.after,
        Some((199, "a".into(), "x99".into()))
    );
    assert_eq!(cursor.extension.through, None);
    assert_eq!(warning_count(&conn, "ml"), 0);
    let second = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert!(!second.more);
    assert_eq!(second.warnings, 1);
    assert_eq!(cursor.extension.after, None);
    assert_eq!(cursor.extension.through, Some(1999));
    assert_eq!(warning_count(&conn, "ml"), 1);
}

fn pass(
    context: &StoreContext,
    conn: &mut Connection,
    cursor: &mut receipts::ReceiptDueCursor,
) -> (u16, bool) {
    let due = receipts::scan_due(context, conn, 10, cursor).unwrap();
    let lapses = receipts::scan_extension_lapses(context, conn, cursor).unwrap();
    (due.warnings + lapses.warnings, due.more || lapses.more)
}

#[test]
fn lapse_watermark_survives_a_completed_due_scan() {
    let (context, mut conn, _clock) = setup();
    let mut cursor = receipts::ReceiptDueCursor::default();
    cursor.extension.through = Some(1500);
    cursor.extension.after = Some((10, "a".into(), "x".into()));
    let expected = cursor.extension.clone();
    let due = receipts::scan_due(&context, &mut conn, 10, &mut cursor).unwrap();
    assert!(!due.more);
    assert_eq!(cursor.extension, expected);
}

#[test]
fn many_ended_rows_stop_reporting_more() {
    let (context, mut conn, clock) = setup();
    for n in 0..250 {
        let thread = format!("x{n:03}");
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [&thread]).unwrap();
        catch_up_row(&conn, "a", &thread, false, Some(100 + n));
    }
    clock.0.store(2000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let mut more_passes = 0;
    while pass(&context, &mut conn, &mut cursor).1 {
        more_passes += 1;
        assert!(more_passes <= 3, "continuation never clears");
    }
    for _ in 0..5 {
        assert!(!pass(&context, &mut conn, &mut cursor).1);
        assert_eq!(
            cursor.extension,
            receipts::ExtensionLapseCursor {
                through: Some(1999),
                after: None
            }
        );
    }
}

#[test]
fn lapse_after_a_completed_walk_is_found_once() {
    let (context, mut conn, clock) = setup();
    clock.0.store(2000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    assert_eq!(pass(&context, &mut conn, &mut cursor), (0, false));
    assert_eq!(cursor.extension.through, Some(1999));
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", false, Some(2500));
    clock.0.store(2499, Ordering::SeqCst);
    assert_eq!(pass(&context, &mut conn, &mut cursor).0, 0);
    clock.0.store(2500, Ordering::SeqCst);
    assert_eq!(pass(&context, &mut conn, &mut cursor).0, 1);
    clock.0.store(2600, Ordering::SeqCst);
    assert_eq!(pass(&context, &mut conn, &mut cursor).0, 0);
    assert_eq!(warning_count(&conn, "ml"), 1);
}

#[test]
fn keyset_pages_through_rows_sharing_one_extension_until() {
    let (context, mut conn, clock) = setup();
    for n in 0..150 {
        let thread = format!("x{n:03}");
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [&thread]).unwrap();
        catch_up_row(&conn, "a", &thread, false, Some(1800));
    }
    legacy_receipt(&conn, "ml", "t", 1, 1500);
    catch_up_row(&conn, "b", "t", false, Some(1800));
    clock.0.store(2000, Ordering::SeqCst);
    let mut cursor = receipts::ReceiptDueCursor::default();
    let first = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert!(first.more);
    let second = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert!(!second.more);
    assert_eq!(first.warnings + second.warnings, 1);
    assert_eq!(warning_count(&conn, "ml"), 1);
}

#[test]
fn record_overdue_uses_effective_for_both_bases() {
    use crate::ports::TimeBasis;
    let (context, mut conn, _clock) = setup();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','i','topic','goal',0,0)", []).unwrap();
    legacy_receipt(&conn, "ext", "t", 1, 1500);
    legacy_receipt(&conn, "plain", "t2", 1, 1500);
    catch_up_row(&conn, "b", "t", true, Some(5000));
    let obligation = |message: &str| ObligationRef::Receipt {
        message: MessageId::new(message),
        seat: SeatId::new("b"),
    };
    // Decision basis: between frozen and effective is not overdue; the same
    // instant without an extension is.
    let (extended, plain) = context
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, _, _| {
                let extended = crate::store::schema::record_overdue_if_pending(
                    tx,
                    &obligation("ext"),
                    &TimeBasis::Decision,
                    UtcMillis(3000),
                )?;
                let plain = crate::store::schema::record_overdue_if_pending(
                    tx,
                    &obligation("plain"),
                    &TimeBasis::Decision,
                    UtcMillis(3000),
                )?;
                Ok((extended.inserted, plain.inserted))
            },
        )
        .unwrap();
    assert!(!extended);
    assert!(plain);
    // At the effective deadline it is overdue, and the payload keeps the
    // frozen deadline.
    let at_effective = context
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, _, _| {
                crate::store::schema::record_overdue_if_pending(
                    tx,
                    &obligation("ext"),
                    &TimeBasis::Decision,
                    UtcMillis(5000),
                )
            },
        )
        .unwrap();
    assert!(at_effective.inserted);
    let payload: String = conn
        .query_row(
            "SELECT event_json FROM messages WHERE kind='warn' AND source_message_id='ext'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(payload.contains("\"deadline_at\":1500"), "{payload}");
}

#[test]
fn retirement_cutover_is_classified_against_the_effective_deadline() {
    use crate::ports::TimeBasis;
    let (context, mut conn, _clock) = setup();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','i','topic','goal',0,0)", []).unwrap();
    legacy_receipt(&conn, "ext", "t", 1, 1500);
    legacy_receipt(&conn, "plain", "t2", 1, 1500);
    catch_up_row(&conn, "b", "t", true, Some(5000));
    // Cutover 3000 is after the frozen deadline but before the effective one.
    conn.execute(
        "UPDATE seats SET state='retired', retired_at=3000 WHERE id='b'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('j', 'b', 3000, 'b', 1, 'p', 1)", []).unwrap();
    let basis = TimeBasis::Retirement(RetirementJobId::new("j"));
    let (extended, plain) = context
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, at, _| {
                let run = |message: &str| {
                    crate::store::schema::record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new(message),
                            seat: SeatId::new("b"),
                        },
                        &basis,
                        at.utc,
                    )
                    .map(|outcome| outcome.inserted)
                };
                Ok((run("ext")?, run("plain")?))
            },
        )
        .unwrap();
    assert!(!extended, "cutover before the effective deadline");
    assert!(plain, "control: no extension means overdue at the cutover");
}

#[test]
fn ack_after_frozen_deadline_during_extension_records_no_late_warning() {
    let (context, mut conn, clock) = setup();
    let id = manifest_receipt(&context, &mut conn);
    catch_up_row(&conn, "b", "t", true, Some(9000));
    clock.0.store(3000, Ordering::SeqCst);
    let request = ack_request(vec![id.clone()]);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    receipts::ack(
        &context,
        &mut conn,
        &budget,
        &request,
        &mut ack_permit(&request),
    )
    .unwrap();
    assert_eq!(warning_count(&conn, id.as_str()), 0);
}

// Catches display settlement trusting a recognized claimed brand despite a
// noncooperative or unknown canonical binding. Refusal leaves history open.
#[test]
fn adapter_display_ack_rechecks_exact_binding_and_cooperative_provenance() {
    for change in [
        "observation_provenance='verified'",
        "harness='future_agent'",
        "native_session='different'",
        "execution_id='different'",
        "generation=2",
    ] {
        let (context, mut db, _) = setup();
        db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('displayed','i','t',1,'ordinary','body',0,2); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('displayed','t','b','pending',300000); UPDATE threads SET next_sequence=2 WHERE id='t'; UPDATE host_instances SET decision_seq=2 WHERE id='i';").unwrap();
        db.execute(
            &format!("UPDATE occupant_bindings SET {change} WHERE seat_id='b'"),
            [],
        )
        .unwrap();
        let request = ack_request(vec![MessageId::new("displayed")]);
        let digest =
            crate::store::schema::canonical_digest(&receipts::ack_displayed_payload(&request))
                .unwrap();
        let mut grant = cooperative_permit(
            &request.claim,
            &request.operation,
            ObligationRef::CheckIn(SeatId::new("b")),
            digest,
            (1, 0),
        );
        let result = receipts::ack_displayed(
            &context,
            &mut db,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            &request,
            &mut grant,
        );
        assert!(result.is_err(), "must refuse {change}: {result:?}");
        assert_eq!(
            db.query_row(
                "SELECT state FROM receipts WHERE message_id='displayed'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
        assert_eq!(
            db.query_row(
                "SELECT ended_at FROM occupant_bindings WHERE seat_id='b'",
                [],
                |r| r.get::<_, Option<i64>>(0)
            )
            .unwrap(),
            None
        );
    }
}
