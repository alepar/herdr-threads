use herdr_threads::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{
        Command, CompleteInboxDelivery, DeliveryMode, InboxQuery, PermitMutation, SendMessage,
    },
    ids::*,
    output::{OutputFormat, OutputSpec, encode_selected},
    pagination::PageRequest,
    results::{CommandResult, ErrorCode, InboxBatchV2Item},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use herdr_threads::store::{connection::StoreContext, messages};
use herdr_threads::test_support::isolation::TestIsolation;
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

/// A store with one joined cooperative sender `a` in thread `t`, rooted in a
/// private isolation directory.
fn setup(iso: &TestIsolation) -> (StoreContext, Connection) {
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('a','i','resolved','native','pa',1,1,0,1)", []).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pa','b',1,1,0,'fresh','term-pa','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('a',1,'pa','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,'term-pa','inc')", []).unwrap();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','a','joined')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','a',1,1)",
        [],
    )
    .unwrap();
    (context, conn)
}

fn send_request() -> SendMessage {
    SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Lazy,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            target: HostTargetId::new("pa"),
        },
        relays_user: false,
        user_intent: None,
    }
}

fn permit(iso: &TestIsolation, request: &SendMessage) -> MutationPermit {
    use herdr_threads::ports::StorePort;
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)),
        "i",
        Default::default(),
    )
    .unwrap();
    store
        .issue_cooperative_permit(
            herdr_threads::ports::CooperativePermitRequest {
                claim: request.claim.clone(),
                operation: request.operation.clone(),
                obligation: ObligationRef::Control(request.thread.clone()),
                payload_hash: herdr_threads::store::schema::canonical_digest(
                    &messages::send_payload(request),
                )
                .unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap()
}

fn prepare(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    units: u8,
) -> Result<herdr_threads::ports::SendPreparationProgress, herdr_threads::protocol::results::ApiError>
{
    messages::prepare_send_step(
        context,
        conn,
        request,
        messages::MessageLimits::default(),
        &budget(),
        herdr_threads::ports::DurableWorkAdmission { max_units: units },
    )
}
fn ready(context: &StoreContext, conn: &mut Connection, request: &SendMessage) {
    loop {
        if matches!(
            prepare(context, conn, request, 1).unwrap(),
            herdr_threads::ports::SendPreparationProgress::Ready { .. }
        ) {
            break;
        }
    }
}

fn fixture(name: &str) -> (TestIsolation, StoreContext, Connection) {
    let iso = TestIsolation::new(name);
    let (context, conn) = setup(&iso);
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('b','i','resolved','native','pb',1,1,0)", []).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','pb','b',1,1,0,'fresh','term-pb','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('b',1,'pb','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000bb','cooperative_top_level',0,0,'term-pb','inc')", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t','b','joined')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','b',1,1)", []).unwrap();
    (iso, context, conn)
}
fn send(
    iso: &TestIsolation,
    context: &StoreContext,
    conn: &mut Connection,
    op: &str,
    body: &str,
    mode: DeliveryMode,
) -> MessageId {
    let mut request = send_request();
    request.operation = OperationId::new(op);
    request.body = body.into();
    request.delivery_mode = mode;
    ready(context, conn, &request);
    let CommandResult::MessageSent(result) = messages::publish_send(
        context,
        conn,
        &request,
        &mut permit(iso, &request),
        &budget(),
        || 65536,
    )
    .unwrap() else {
        panic!("send");
    };
    result
}
fn query(
    context: &StoreContext,
    request: PageRequest,
    spec: &OutputSpec,
) -> herdr_threads::protocol::pagination::Page<InboxBatchV2Item> {
    let result = herdr_threads::store::queries::query_with_output(
        context,
        "i",
        &Command::InboxBatchV2(InboxQuery {
            seat: Some(SeatId::new("b")),
            page: request,
        }),
        spec,
        &budget(),
    )
    .unwrap();
    let CommandResult::InboxBatchV2(page) = result else {
        panic!("v2");
    };
    page
}
fn claim() -> CallerClaim {
    let mut claim = send_request().claim;
    claim.seat = SeatId::new("b");
    claim.target = HostTargetId::new("pb");
    claim.execution = ExecutionId::new("00000000-0000-4000-8000-0000000000bb");
    claim
}
fn complete(
    iso: &TestIsolation,
    request: CompleteInboxDelivery,
) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
    use herdr_threads::ports::StorePort;
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)),
        "i",
        Default::default(),
    )
    .unwrap();
    let mutation = PermitMutation::CompleteInboxDelivery(request);
    let permit = store.issue_cooperative_permit(
        herdr_threads::store::cooperative_permit_request(&mutation)?,
        &budget(),
    )?;
    store.mutate(mutation, permit, &budget())
}
#[test]
fn lazy_inbox_v2_bounded_round_robin_opportunity() {
    let (iso, context, mut conn) = fixture("lazy-v2-fair");
    send(
        &iso,
        &context,
        &mut conn,
        "ordinary",
        &"retained".repeat(1000),
        DeliveryMode::Ordinary,
    );
    let lazy = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    let page = query(
        &context,
        PageRequest {
            limit: 1,
            max_bytes: 4096,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(
        matches!(&page.items[0], InboxBatchV2Item::LazyMessage { message, .. } if message == &lazy)
    );
    assert!(page.has_more);
    let later = query(
        &context,
        PageRequest {
            cursor: page.next_cursor,
            limit: 1,
            max_bytes: 4096,
        },
        &OutputSpec::default(),
    );
    assert!(matches!(&later.items[0], InboxBatchV2Item::Message { .. }));
}
#[test]
fn lazy_inbox_v2_unpublished_empty_continuation() {
    let (_iso, context, mut conn) = fixture("lazy-v2-unpublished");
    ready(&context, &mut conn, &send_request());
    for n in 0..150 {
        let mut request = send_request();
        request.operation = OperationId::new(format!("unpublished-{n}"));
        ready(&context, &mut conn, &request);
    }
    let page = query(&context, PageRequest::default(), &OutputSpec::default());
    assert!(page.items.is_empty());
    assert!(page.has_more);
    let captured = herdr_threads::protocol::pagination::InboxBatchV2CursorState::decode(
        page.next_cursor.as_ref().unwrap(),
    )
    .unwrap();
    assert!((1..=100).contains(&captured.lazy_after_ordinal));
    assert_eq!(captured.lazy_high_water_ordinal, 151);
    let text = String::from_utf8(
        encode_selected(
            &CommandResult::InboxBatchV2(page.clone()),
            &OutputSpec {
                format: OutputFormat::Text,
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(text.contains("next:"));
    assert!(!text.contains("empty"));
    let last = query(
        &context,
        PageRequest {
            cursor: page.next_cursor,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(last.items.is_empty());
    assert!(!last.has_more);
}
#[test]
fn lazy_inbox_v2_utf8_body_output_budget() {
    for format in [OutputFormat::Text, OutputFormat::Json] {
        let (iso, context, mut conn) = fixture("lazy-v2-utf8");
        let body = "é🦀\n\t\u{1b}danger".repeat(1200);
        send(&iso, &context, &mut conn, "long", &body, DeliveryMode::Lazy);
        let spec = OutputSpec {
            format,
            ..Default::default()
        };
        let mut cursor = None;
        let mut shown = String::new();
        for _ in 0..200 {
            let page = query(
                &context,
                PageRequest {
                    cursor: cursor.clone(),
                    limit: 1,
                    max_bytes: 4096,
                },
                &spec,
            );
            assert!(
                encode_selected(&CommandResult::InboxBatchV2(page.clone()), &spec)
                    .unwrap()
                    .len()
                    <= 4096
            );
            for item in &page.items {
                let InboxBatchV2Item::LazyMessage {
                    body,
                    body_start,
                    body_end,
                    body_len,
                    ..
                } = item
                else {
                    panic!("lazy");
                };
                assert_eq!(*body_start, shown.len() as u64);
                assert_eq!(*body_end - *body_start, body.len() as u64);
                assert_eq!(*body_len, "é🦀\n\t\u{1b}danger".len() as u64 * 1200);
                shown.push_str(body);
            }
            if !page.has_more {
                break;
            }
            assert_ne!(page.next_cursor, cursor);
            cursor = page.next_cursor;
        }
        assert_eq!(shown, body);
    }
}
#[test]
fn lazy_inbox_v2_snapshot_restart_concurrent_traversals() {
    let (iso, context, mut conn) = fixture("lazy-v2-snapshot");
    let first = send(
        &iso,
        &context,
        &mut conn,
        "first",
        "one",
        DeliveryMode::Lazy,
    );
    let second = send(
        &iso,
        &context,
        &mut conn,
        "second",
        "two",
        DeliveryMode::Lazy,
    );
    let request = PageRequest {
        limit: 1,
        ..Default::default()
    };
    let a = query(&context, request.clone(), &OutputSpec::default());
    let b = query(&context, request, &OutputSpec::default());
    assert_eq!(a, b);
    let third = send(
        &iso,
        &context,
        &mut conn,
        "third",
        "three",
        DeliveryMode::Lazy,
    );
    drop(conn);
    let restarted = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
    let next = query(
        &restarted,
        PageRequest {
            cursor: a.next_cursor,
            limit: 100,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    let ids: Vec<_> = next
        .items
        .iter()
        .filter_map(|i| {
            if let InboxBatchV2Item::LazyMessage { message, .. } = i {
                Some(message)
            } else {
                None
            }
        })
        .collect();
    assert!(ids.contains(&&second));
    assert!(!ids.contains(&&third));
    assert!(!ids.contains(&&first));
    assert_eq!(
        query(&restarted, PageRequest::default(), &OutputSpec::default())
            .items
            .len(),
        3
    );
}
#[test]
fn lazy_inbox_completion_exact_id_idempotent_authority() {
    let (iso, context, mut conn) = fixture("lazy-v2-complete");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    let ordinary = send(
        &iso,
        &context,
        &mut conn,
        "ordinary",
        "loud",
        DeliveryMode::Ordinary,
    );
    let request = CompleteInboxDelivery {
        via: None,
        messages: vec![id.clone()],
        operation: OperationId::new("done"),
        claim: claim(),
    };
    let before: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    let result = complete(&iso, request.clone()).unwrap();
    assert_eq!(
        result,
        CommandResult::InboxDeliveryCompleted(vec![id.clone()])
    );
    assert_eq!(complete(&iso, request.clone()).unwrap(), result);
    let mut again = request.clone();
    again.operation = OperationId::new("done-again");
    assert_eq!(complete(&iso, again).unwrap(), result);
    for (op, ids) in [
        ("ordinary", vec![ordinary]),
        ("unknown", vec![MessageId::new("missing")]),
        ("mixed", vec![id.clone(), MessageId::new("missing")]),
        ("duplicate", vec![id.clone(), id]),
    ] {
        let mut invalid = request.clone();
        invalid.operation = OperationId::new(format!("invalid-{op}"));
        invalid.messages = ids;
        assert_eq!(
            complete(&iso, invalid).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let mut sub = request.clone();
    sub.claim.role = CallerRole::Subagent;
    sub.operation = OperationId::new("sub");
    assert!(complete(&iso, sub).is_err());
    let mut stale = request;
    stale.claim.binding_generation = 2;
    stale.operation = OperationId::new("stale");
    assert!(complete(&iso, stale).is_err());
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='acknowledged'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT state FROM lazy_recipients", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "displayed"
    );
}
#[test]
fn lazy_inbox_addressed_leave_retire_archive() {
    let (iso, context, mut conn) = fixture("lazy-v2-lifecycle");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    conn.execute(
        "UPDATE memberships SET state='left',voluntary_state='left' WHERE seat_id='b'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE threads SET archived=1", []).unwrap();
    assert!(
        query(&context, PageRequest::default(), &OutputSpec::default())
            .items
            .iter()
            .any(|i| matches!(i,InboxBatchV2Item::LazyMessage{message,..} if message==&id))
    );
    let done = CompleteInboxDelivery {
        via: None,
        messages: vec![id],
        operation: OperationId::new("human-done"),
        claim: CallerClaim {
            harness: Harness::Human,
            ..claim()
        },
    };
    conn.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human' WHERE seat_id='b'",[]).unwrap();
    assert!(complete(&iso, done).is_ok());
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1000,retired_seq=10 WHERE id='b'",
        [],
    )
    .unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM lazy_recipients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let err = herdr_threads::store::queries::query(
        &context,
        "i",
        &Command::InboxBatchV2(InboxQuery {
            seat: Some(SeatId::new("b")),
            page: PageRequest::default(),
        }),
        &budget(),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

#[test]
fn lazy_inbox_v2_rejects_changed_binding_foreign_and_bad_body_cursor() {
    use herdr_threads::protocol::pagination::InboxBatchV2CursorState as State;
    let (iso, context, mut conn) = fixture("lazy-v2-invalid-cursor");
    send(
        &iso,
        &context,
        &mut conn,
        "long",
        &"é".repeat(4000),
        DeliveryMode::Lazy,
    );
    let first = query(
        &context,
        PageRequest {
            limit: 1,
            max_bytes: 2048,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    let raw = first.next_cursor.unwrap();
    let captured = State::decode(&raw).unwrap();
    assert!(captured.body.is_some());
    let issue = |raw: String| {
        herdr_threads::store::queries::query(
            &context,
            "i",
            &Command::InboxBatchV2(InboxQuery {
                seat: Some(SeatId::new("b")),
                page: PageRequest {
                    cursor: Some(raw),
                    limit: 1,
                    max_bytes: 2048,
                },
            }),
            &budget(),
        )
    };
    for bad in 0..4 {
        let mut state = captured.clone();
        match bad {
            0 => state.body.as_mut().unwrap().offset = 1,
            1 => state.body.as_mut().unwrap().body_len += 1,
            2 => state.body.as_mut().unwrap().message = MessageId::new("absent"),
            _ => state.binding_generation = Some(2),
        }
        assert_eq!(
            issue(state.encode("i").unwrap()).unwrap_err().code,
            ErrorCode::InvalidCursor
        );
    }
    assert_eq!(
        issue(captured.encode("other-instance").unwrap())
            .unwrap_err()
            .code,
        ErrorCode::InvalidCursor
    );
    let mut foreign = captured.clone();
    foreign.seat = SeatId::new("a");
    assert_eq!(
        issue(foreign.encode("i").unwrap()).unwrap_err().code,
        ErrorCode::InvalidCursor
    );
    conn.execute(
        "UPDATE occupant_bindings SET execution_id='replacement' WHERE seat_id='b'",
        [],
    )
    .unwrap();
    assert_eq!(issue(raw).unwrap_err().code, ErrorCode::InvalidCursor);
    assert!(
        !query(&context, PageRequest::default(), &OutputSpec::default())
            .items
            .is_empty()
    );
}
#[test]
fn lazy_inbox_v2_late_publication_and_completion_do_not_hide_other_mail() {
    let (iso, context, mut conn) = fixture("lazy-v2-late-publish");
    let first = send(
        &iso,
        &context,
        &mut conn,
        "first",
        "one",
        DeliveryMode::Lazy,
    );
    let second = send(
        &iso,
        &context,
        &mut conn,
        "second",
        "two",
        DeliveryMode::Lazy,
    );
    let mut prepared = send_request();
    prepared.operation = OperationId::new("late");
    ready(&context, &mut conn, &prepared);
    let a = query(
        &context,
        PageRequest {
            limit: 1,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    let mut p = permit(&iso, &prepared);
    let CommandResult::MessageSent(late) =
        messages::publish_send(&context, &mut conn, &prepared, &mut p, &budget(), || 65536)
            .unwrap()
    else {
        panic!("send")
    };
    complete(
        &iso,
        CompleteInboxDelivery {
            via: None,
            messages: vec![first],
            operation: OperationId::new("settle-first"),
            claim: claim(),
        },
    )
    .unwrap();
    let page = query(
        &context,
        PageRequest {
            cursor: a.next_cursor,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(
        page.items
            .iter()
            .any(|i| matches!(i,InboxBatchV2Item::LazyMessage{message,..}if message==&second))
    );
    assert!(
        !page
            .items
            .iter()
            .any(|i| matches!(i,InboxBatchV2Item::LazyMessage{message,..}if message==&late))
    );
    assert_eq!(
        query(&context, PageRequest::default(), &OutputSpec::default())
            .items
            .len(),
        2
    );
}
#[test]
fn lazy_inbox_v2_small_budget_and_read_only_queries() {
    let (iso, context, mut conn) = fixture("lazy-v2-readonly");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "one",
        "quiet",
        DeliveryMode::Lazy,
    );
    conn.execute(
        "UPDATE threads SET topic=?1 WHERE id='t'",
        ["topic".repeat(100)],
    )
    .unwrap();
    let before = conn
        .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
        .unwrap();
    for format in [OutputFormat::Text, OutputFormat::Json] {
        let spec = OutputSpec {
            format,
            ..Default::default()
        };
        let page = query(&context, PageRequest::default(), &spec);
        let encoded = encode_selected(&CommandResult::InboxBatchV2(page), &spec).unwrap();
        if format == OutputFormat::Text {
            assert!(String::from_utf8(encoded).unwrap().contains("[lazy]"));
        }
        assert_eq!(
            conn.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        let result = herdr_threads::store::queries::query_with_output(
            &context,
            "i",
            &Command::InboxBatchV2(InboxQuery {
                seat: Some(SeatId::new("b")),
                page: PageRequest {
                    max_bytes: 256,
                    ..Default::default()
                },
            }),
            &spec,
            &budget(),
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::InvalidBudget);
    }
    assert_eq!(
        conn.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id=?1",
            [id.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
}
#[test]
fn lazy_inbox_v2_preserves_invitations_warnings_and_ordinary_candidates() {
    let (iso, context, mut conn) = fixture("lazy-v2-mixed");
    let ordinary = send(
        &iso,
        &context,
        &mut conn,
        "ordinary",
        "loud",
        DeliveryMode::Ordinary,
    );
    send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    let decision = conn
        .query_row(
            "SELECT decision_seq+1 FROM host_instances WHERE id='i'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES('inv','t','b',2,'pending',0,?1,300,300)",[decision]).unwrap();
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES('i','warning','t',100,'warn','{}',?1,0)",[decision+1]).unwrap();
    conn.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('warning',?1,'t',100,'b','invitation','inv')",[decision+1]).unwrap();
    conn.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [decision + 1],
    )
    .unwrap();
    let page = query(&context, PageRequest::default(), &OutputSpec::default());
    assert!(matches!(
        &page.items[0],
        InboxBatchV2Item::LazyMessage { .. }
    ));
    assert!(page.items.iter().any(
        |i| matches!(i,InboxBatchV2Item::Invitation{invitation,..}if invitation.as_str()=="inv")
    ));
    assert!(
        page.items.iter().any(
            |i| matches!(i,InboxBatchV2Item::Message{ack_candidate:Some(id),..}if id==&ordinary)
        )
    );
    assert!(
        page.items.iter().any(
            |i| matches!(i,InboxBatchV2Item::Warning{warning,..}if warning.as_str()=="warning")
        )
    );
    assert!(!page.has_more);
}
// Lazy hook delivery (main merge review P2): receipt eligibility is the
// registered-agent registry, not a fixed Claude/Codex list. A real Hermes
// binding's ordinary ACK-required message is an ACK candidate on the v2 page,
// and the hook page renderer then names its exact `ack` command instead of
// claiming nothing else is needed. Kills: the hard-coded `IN ('claude','codex')`
// predicate (Hermes bodies shown without ACK instructions) and an always-true
// eligibility (an unregistered or human harness still gets candidates).
#[test]
fn lazy_inbox_v2_ack_candidates_follow_registered_agent_harnesses() {
    use herdr_threads::cli::hook_inbox::{InboxOffer, append};
    let (iso, context, mut conn) = fixture("lazy-v2-hermes-ack");
    let ordinary = send(
        &iso,
        &context,
        &mut conn,
        "ordinary",
        "loud",
        DeliveryMode::Ordinary,
    );
    let candidate = |conn: &Connection, harness: &str| {
        conn.execute(
            "UPDATE occupant_bindings SET harness=?1 WHERE seat_id='b'",
            [harness],
        )
        .unwrap();
        let page = query(&context, PageRequest::default(), &OutputSpec::default());
        let found = page.items.iter().find_map(|item| match item {
            InboxBatchV2Item::Message {
                message,
                ack_candidate,
                ..
            } if message == &ordinary => Some(ack_candidate.clone()),
            _ => None,
        });
        (page, found.expect("ordinary row listed"))
    };
    let (page, hermes) = candidate(&conn, "hermes");
    assert_eq!(hermes.as_ref(), Some(&ordinary));
    // The renderer reads only the page; the claim matters for completion.
    let offer = InboxOffer {
        page,
        claim: claim(),
    };
    let (context_text, lazy) = append(String::new(), &offer, &["herdr-threads".to_owned()], 4096);
    assert!(lazy.is_empty(), "ordinary rows are never lazy-completed");
    assert!(
        context_text.contains(&format!("herdr-threads ack {}", ordinary.as_str())),
        "{context_text}"
    );
    for (harness, wanted) in [("claude", true), ("codex", true), ("not-a-harness", false)] {
        assert_eq!(candidate(&conn, harness).1.is_some(), wanted, "{harness}");
    }
}
#[test]
fn lazy_inbox_completion_prevalidates_all_ids_and_authority_before_progress() {
    let (iso, context, mut conn) = fixture("lazy-v2-atomic");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    let request = CompleteInboxDelivery {
        via: None,
        messages: vec![id.clone(), MessageId::new("missing")],
        operation: OperationId::new("mixed-bad"),
        claim: claim(),
    };
    assert_eq!(
        complete(&iso, request).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    for bad in 0..5 {
        let mut caller = claim();
        match bad {
            0 => caller.harness = Harness::Claude,
            1 => caller.execution = ExecutionId::new("other"),
            2 => caller.native_session = NativeSessionId::new("other"),
            3 => caller.target = HostTargetId::new("pa"),
            _ => caller = send_request().claim,
        }
        let request = CompleteInboxDelivery {
            via: None,
            messages: vec![id.clone()],
            operation: OperationId::new(format!("bad-claim-{bad}")),
            claim: caller,
        };
        assert!(complete(&iso, request).is_err());
        assert_eq!(
            conn.query_row("SELECT state FROM lazy_recipients", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "pending"
        );
    }
    assert_eq!(conn.query_row("SELECT count(*) FROM operations WHERE operation_key LIKE 'bad-claim-%' OR operation_key='mixed-bad'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn lazy_inbox_retirement_retains_pending_rows_and_excludes_new_sends() {
    let (iso, context, mut conn) = fixture("lazy-v2-retired-pending");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "before",
        "quiet",
        DeliveryMode::Lazy,
    );
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1000,retired_seq=10 WHERE id='b'",
        [],
    )
    .unwrap();
    send(
        &iso,
        &context,
        &mut conn,
        "after",
        "new",
        DeliveryMode::Lazy,
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM lazy_recipients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id=?1",
            [id.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert!(
        complete(
            &iso,
            CompleteInboxDelivery {
                via: None,
                messages: vec![id],
                operation: OperationId::new("retired-done"),
                claim: claim()
            }
        )
        .is_err()
    );
    assert_eq!(
        conn.query_row("SELECT state FROM lazy_recipients", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}
#[test]
fn lazy_inbox_completion_mismatched_permit_is_write_free() {
    use herdr_threads::ports::StorePort;
    let (iso, context, mut conn) = fixture("lazy-v2-mismatched-permit");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "one",
        "quiet",
        DeliveryMode::Lazy,
    );
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)),
        "i",
        Default::default(),
    )
    .unwrap();
    let request = CompleteInboxDelivery {
        via: None,
        messages: vec![id],
        operation: OperationId::new("original-done"),
        claim: claim(),
    };
    let mutation = PermitMutation::CompleteInboxDelivery(request.clone());
    let permit = store
        .issue_cooperative_permit(
            herdr_threads::store::cooperative_permit_request(&mutation).unwrap(),
            &budget(),
        )
        .unwrap();
    let mut changed = request;
    changed.operation = OperationId::new("changed-done");
    assert!(
        store
            .mutate(
                PermitMutation::CompleteInboxDelivery(changed),
                permit,
                &budget()
            )
            .is_err()
    );
    assert_eq!(
        conn.query_row("SELECT state FROM lazy_recipients", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM operations WHERE operation_key IN ('original-done','changed-done')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn lazy_inbox_fix_partial_concurrent_completion() {
    let (iso, context, mut conn) = fixture("lazy-fix-partial");
    let first = send(
        &iso,
        &context,
        &mut conn,
        "first",
        &"界".repeat(3000),
        DeliveryMode::Lazy,
    );
    let second = send(
        &iso,
        &context,
        &mut conn,
        "second",
        "second intact",
        DeliveryMode::Lazy,
    );
    let page = query(
        &context,
        PageRequest {
            limit: 1,
            max_bytes: 1200,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(
        matches!(&page.items[0], InboxBatchV2Item::LazyMessage { message, body_end, body_len, .. } if message == &first && body_end < body_len)
    );
    complete(
        &iso,
        CompleteInboxDelivery {
            via: None,
            messages: vec![first],
            operation: OperationId::new("concurrent"),
            claim: claim(),
        },
    )
    .unwrap();
    let mut cursor = page.next_cursor;
    let mut found = false;
    for _ in 0..100 {
        let page = query(
            &context,
            PageRequest {
                cursor,
                limit: 1,
                max_bytes: 1200,
            },
            &OutputSpec::default(),
        );
        for item in &page.items {
            if let InboxBatchV2Item::LazyMessage {
                message,
                body,
                body_start,
                ..
            } = item
                && message == &second
            {
                assert_eq!(*body_start, 0);
                assert_eq!(body, "second intact");
                found = true;
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(found);
    assert!(cursor.is_none());
}

#[test]
fn lazy_inbox_fix_partial_concurrent_ack() {
    let (iso, context, mut conn) = fixture("ordinary-fix-partial");
    let first = send(
        &iso,
        &context,
        &mut conn,
        "first",
        &"界".repeat(3000),
        DeliveryMode::Ordinary,
    );
    let second = send(
        &iso,
        &context,
        &mut conn,
        "second",
        "second intact",
        DeliveryMode::Ordinary,
    );
    let page = query(
        &context,
        PageRequest {
            limit: 1,
            max_bytes: 1200,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(
        matches!(&page.items[0], InboxBatchV2Item::Message { message, body_end, body_len, .. } if message == &first && body_end < body_len)
    );
    {
        use herdr_threads::ports::StorePort;
        let store = herdr_threads::store::SqliteStore::new(
            StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)),
            "i",
            Default::default(),
        )
        .unwrap();
        let mutation = PermitMutation::Ack(herdr_threads::protocol::commands::Ack {
            messages: vec![first],
            operation: OperationId::new("concurrent-ack"),
            claim: claim(),
        });
        let permit = store
            .issue_cooperative_permit(
                herdr_threads::store::cooperative_permit_request(&mutation).unwrap(),
                &budget(),
            )
            .unwrap();
        store.mutate(mutation, permit, &budget()).unwrap();
    }
    let mut cursor = page.next_cursor;
    let mut found = false;
    for _ in 0..100 {
        let page = query(
            &context,
            PageRequest {
                cursor,
                limit: 1,
                max_bytes: 1200,
            },
            &OutputSpec::default(),
        );
        for item in &page.items {
            if let InboxBatchV2Item::Message {
                message,
                body,
                body_start,
                ..
            } = item
                && message == &second
            {
                assert_eq!(*body_start, 0);
                assert_eq!(body, "second intact");
                found = true;
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!(found);
    assert!(cursor.is_none());
}
