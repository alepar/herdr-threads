use herdr_threads::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{
        Command, CompleteInboxDelivery, DeliveryMode, InboxQuery, Invite, Leave, PermitMutation,
        SendMessage,
    },
    ids::*,
    output::{OutputFormat, OutputSpec, encode_selected},
    pagination::PageRequest,
    results::{CommandResult, ErrorCode, InboxBatchV2Item},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use herdr_threads::store::{connection::StoreContext, messages};
use herdr_threads::test_support::isolation::TestIsolation;
use rusqlite::{Connection, OptionalExtension};
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
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES ('a','34750f98bd59fcfc946da45aaabe933b','resolved','native','pa',1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pa','b',1,1,0,'fresh','term-pa','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('a',1,'pa','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,'term-pa','inc')", []).unwrap();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0)", []).unwrap();
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
        priority: None,
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Lazy,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op"),
        claim: CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new("a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            target: HostTargetId::new("pa"),
            agent_id: None,
            liveness: Some(herdr_threads::protocol::authority::LivenessFingerprint::TEST),
        },
        relays_user: false,
        user_intent: None,
    }
}

fn permit(iso: &TestIsolation, request: &SendMessage) -> MutationPermit {
    use herdr_threads::ports::StorePort;
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
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
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('b','34750f98bd59fcfc946da45aaabe933b','resolved','native','pb',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pb','b',1,1,0,'fresh','term-pb','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('b',1,'pb','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000bb','cooperative_top_level',0,0,'term-pb','inc')", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t','b','joined')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','b',1,1)", []).unwrap();
    // A binding open while its seat joins holds a cursor for the thread (join seeds it).
    conn.execute("INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',0 FROM occupant_bindings WHERE seat_id='b'", []).unwrap();
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
    // An ordinary send is the explicit-ACK shape: only the named recipient owes a receipt (D9).
    if mode == DeliveryMode::Ordinary {
        request.invited_recipients = vec![SeatId::new("b")];
    }
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
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            pushed: false,
            thread: None,
            claim: None,
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
    mutate(iso, PermitMutation::CompleteInboxDelivery(request))
}

fn mutate(
    iso: &TestIsolation,
    mutation: PermitMutation,
) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
    use herdr_threads::ports::StorePort;
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap();
    let permit = store.issue_cooperative_permit(
        herdr_threads::store::cooperative_permit_request(&mutation)?,
        &budget(),
    )?;
    store.mutate(mutation, permit, &budget())
}

#[test]
fn hook_receipt_completion_retains_left_and_invited_obligations_without_joining() {
    for reinvite in [false, true] {
        let (iso, context, mut conn) = fixture("hook-receipt-left");
        let retained = send(
            &iso,
            &context,
            &mut conn,
            "before-leave",
            "retained receipt",
            DeliveryMode::Ordinary,
        );
        mutate(
            &iso,
            PermitMutation::Leave(Leave {
                thread: ThreadId::new("t"),
                operation: OperationId::new("leave"),
                claim: claim(),
            }),
        )
        .unwrap();
        let mut candidates = vec![retained];
        if reinvite {
            mutate(
                &iso,
                PermitMutation::Invite(Invite {
                    confirmation: None,
                    thread: ThreadId::new("t"),
                    seat: SeatId::new("b"),
                    deadline_millis: None,
                    operation: OperationId::new("reinvite"),
                    claim: send_request().claim,
                }),
            )
            .unwrap();
            candidates.push(send(
                &iso,
                &context,
                &mut conn,
                "while-invited",
                "explicit invitee receipt",
                DeliveryMode::Ordinary,
            ));
        }
        let page = query(&context, PageRequest::default(), &OutputSpec::default());
        for id in &candidates {
            assert!(page.items.iter().any(
                |item| matches!(item, InboxBatchV2Item::Message { message, .. } if message == id)
            ));
        }
        let request: CompleteInboxDelivery = serde_json::from_value(serde_json::json!({
            "messages":[], "receipt_candidates":candidates, "operation":"hook-left",
            "claim":claim(), "via":"hook_context"
        }))
        .unwrap();
        complete(&iso, request.clone()).unwrap();
        complete(&iso, request).unwrap();
        for id in &candidates {
            let (state, observation): (String, String) = conn.query_row(
                "SELECT state,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
                [id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap();
            assert_eq!(state, "acked");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&observation).unwrap()["action_provenance"],
                "cooperative_hook_context"
            );
        }
        let state: String = conn
            .query_row(
                "SELECT state FROM memberships WHERE thread_id='t' AND seat_id='b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if reinvite { "invited" } else { "left" });
        assert_eq!(conn.query_row(
            "SELECT count(*) FROM invitations WHERE thread_id='t' AND seat_id='b' AND state='pending'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap(), i64::from(reinvite));
    }
}

#[test]
fn hook_receipt_completion_is_canonical_and_idempotent() {
    let (iso, context, mut conn) = fixture("hook-receipt-completion");
    let id = send(
        &iso,
        &context,
        &mut conn,
        "receipt-send",
        "complete",
        DeliveryMode::Ordinary,
    );
    let value = serde_json::json!({"messages":[],"receipt_candidates":[id],"operation":"hook-receipt","claim":claim(),"via":"hook_context"});
    let request: CompleteInboxDelivery = serde_json::from_value(value).unwrap();
    complete(&iso, request.clone()).unwrap();
    complete(&iso, request.clone()).unwrap();
    let mut repeated = request.clone();
    repeated.operation = OperationId::new("hook-receipt-again");
    complete(&iso, repeated).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let (state, observation): (String, String) = conn
        .query_row(
            "SELECT state,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "acked");
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(observation["action_provenance"], "cooperative_hook_context");
    assert_eq!(observation["provenance"], "cooperative_top_level");
    let mut changed = request;
    changed.via = None;
    assert!(complete(&iso, changed).is_err());
}

#[test]
fn hook_receipt_completion_refuses_ineligible_claims_atomically() {
    for case in [
        "disconnected",
        "stale",
        "foreign",
        "subagent",
        "unknown",
        "wrong-source",
        "hook-pull",
        "duplicate",
    ] {
        let (iso, context, mut conn) = fixture("hook-receipt-refusal");
        let id = send(
            &iso,
            &context,
            &mut conn,
            "receipt-send",
            "complete",
            DeliveryMode::Ordinary,
        );
        let mut value = serde_json::json!({"messages":[],"receipt_candidates":[id],"operation":"hook-receipt","claim":claim(),"via":"hook_context"});
        match case {
            "disconnected" => {
                conn.execute("UPDATE occupant_bindings SET liveness='disconnected',disconnect_reason='superseded' WHERE seat_id='b'", []).unwrap();
            }
            "stale" => {
                value["claim"]["execution"] =
                    serde_json::json!("00000000-0000-4000-8000-000000000099")
            }
            "foreign" => value["claim"]["instance"] = serde_json::json!("other"),
            "subagent" => value["claim"]["role"] = serde_json::json!("subagent"),
            "unknown" => value["receipt_candidates"] = serde_json::json!([id, "unknown"]),
            "wrong-source" => {
                value.as_object_mut().unwrap().remove("via");
            }
            "hook-pull" => {
                conn.execute(
                    "UPDATE occupant_bindings SET harness='agy' WHERE seat_id='b'",
                    [],
                )
                .unwrap();
                value["claim"]["harness"] = serde_json::json!("agy");
            }
            "duplicate" => value["receipt_candidates"] = serde_json::json!([id, id]),
            _ => unreachable!(),
        }
        let request = serde_json::from_value(value).unwrap();
        assert!(complete(&iso, request).is_err(), "{case}");
        assert_eq!(
            conn.query_row(
                "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
                [id.as_str()],
                |r| r.get::<_, String>(0)
            )
            .optional()
            .unwrap()
            .as_deref()
            .unwrap_or("pending"),
            "pending",
            "{case}"
        );
    }
}

#[test]
fn hook_receipt_completion_mixed_batch_keeps_obligations_separate() {
    let (iso, context, mut conn) = fixture("hook-receipt-mixed");
    let receipt = send(
        &iso,
        &context,
        &mut conn,
        "receipt-send",
        "complete",
        DeliveryMode::Ordinary,
    );
    let lazy = send(
        &iso,
        &context,
        &mut conn,
        "lazy-send",
        "quiet",
        DeliveryMode::Lazy,
    );
    let omitted: CompleteInboxDelivery = serde_json::from_value(serde_json::json!({"messages":[lazy],"operation":"only-lazy","claim":claim(),"via":"hook_context"})).unwrap();
    complete(&iso, omitted).unwrap();
    assert!(
        conn.query_row(
            "SELECT acked_at FROM receipt_state WHERE message_id=?1 AND seat_id='b'",
            [receipt.as_str()],
            |r| r.get::<_, Option<i64>>(0)
        )
        .optional()
        .unwrap()
        .flatten()
        .is_none()
    );
    let mixed: CompleteInboxDelivery = serde_json::from_value(serde_json::json!({"messages":[lazy],"receipt_candidates":[receipt],"operation":"mixed","claim":claim(),"via":"hook_context"})).unwrap();
    complete(&iso, mixed).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='b'",
            [lazy.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "displayed"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1 AND state='acked'",
            [receipt.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1",
            [lazy.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn hook_receipt_completion_cannot_create_an_obligation() {
    let (iso, context, mut conn) = fixture("hook-receipt-not-required");
    let mut request = send_request();
    request.delivery_mode = DeliveryMode::Ordinary;
    request.operation = OperationId::new("default-send");
    ready(&context, &mut conn, &request);
    let CommandResult::MessageSent(id) = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&iso, &request),
        &budget(),
        || 65536,
    )
    .unwrap() else {
        panic!("send");
    };
    let completion: CompleteInboxDelivery = serde_json::from_value(serde_json::json!({"messages":[],"receipt_candidates":[id],"operation":"unsupported-receipt","claim":claim(),"via":"hook_context"})).unwrap();
    assert!(complete(&iso, completion).is_err());
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1",
            [id.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn lazy_inbox_v2_bounded_priority_opportunity() {
    // Rotation is gone: the explicit-ACK receipt (p0, with a deadline) precedes the p1 delivery
    // row whichever was sent first, and a one-row page resumes exactly after it.
    let (iso, context, mut conn) = fixture("lazy-v2-fair");
    let lazy = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    send(
        &iso,
        &context,
        &mut conn,
        "ordinary",
        &"retained".repeat(1000),
        DeliveryMode::Ordinary,
    );
    let page = query(
        &context,
        PageRequest {
            limit: 1,
            max_bytes: 16384,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(matches!(&page.items[0], InboxBatchV2Item::Message { .. }));
    assert!(page.has_more);
    let later = query(
        &context,
        PageRequest {
            cursor: page.next_cursor,
            limit: 1,
            max_bytes: 16384,
        },
        &OutputSpec::default(),
    );
    assert!(
        matches!(&later.items[0], InboxBatchV2Item::LazyMessage { message, .. } if message == &lazy)
    );
}
#[test]
fn lazy_inbox_v2_unpublished_rows_are_never_visited() {
    let (_iso, context, mut conn) = fixture("lazy-v2-unpublished");
    ready(&context, &mut conn, &send_request());
    for n in 0..150 {
        let mut request = send_request();
        request.operation = OperationId::new(format!("unpublished-{n}"));
        ready(&context, &mut conn, &request);
    }
    // Staged rows carry no publication decision: the ordered seek never visits one, so the
    // traversal is exact and complete at once (no false continuation to walk 150 rows).
    let page = query(&context, PageRequest::default(), &OutputSpec::default());
    assert!(page.items.is_empty());
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());
    let text = String::from_utf8(
        encode_selected(
            &CommandResult::InboxBatchV2(page),
            &OutputSpec {
                format: OutputFormat::Text,
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!text.contains("next:"));
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
    let restarted = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
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
        receipt_candidates: Vec::new(),
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
        receipt_candidates: Vec::new(),
        via: None,
        messages: vec![id],
        operation: OperationId::new("human-done"),
        claim: CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
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
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            pushed: false,
            thread: None,
            claim: None,
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
            "34750f98bd59fcfc946da45aaabe933b",
            &Command::InboxBatchV2(InboxQuery {
                pushed: false,
                thread: None,
                claim: None,
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
            0 => state.body.as_mut().unwrap().1.offset = 1,
            1 => state.body.as_mut().unwrap().1.body_len += 1,
            2 => state.body.as_mut().unwrap().1.message = MessageId::new("absent"),
            _ => state.binding_generation = Some(2),
        }
        assert_eq!(
            issue(state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap())
                .unwrap_err()
                .code,
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
        issue(foreign.encode("34750f98bd59fcfc946da45aaabe933b").unwrap())
            .unwrap_err()
            .code,
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
            receipt_candidates: Vec::new(),
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
            let text = String::from_utf8(encoded).unwrap();
            // A default-priority (p1) delivery row is silent; the old `[lazy]` marker is gone.
            assert!(
                text.contains("message ") && !text.contains("[lazy]"),
                "{text}"
            );
        } else {
            let json = String::from_utf8(encoded).unwrap();
            assert!(json.contains("\"priority\":1"), "{json}");
        }
        assert_eq!(
            conn.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        let result = herdr_threads::store::queries::query_with_output(
            &context,
            "34750f98bd59fcfc946da45aaabe933b",
            &Command::InboxBatchV2(InboxQuery {
                pushed: false,
                thread: None,
                claim: None,
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
            "SELECT decision_seq+1 FROM local_node WHERE id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,origin_node) VALUES('inv','t','b',2,'pending',0,?1,300,300,(SELECT id FROM local_node))",[decision]).unwrap();
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES('34750f98bd59fcfc946da45aaabe933b','warning','t',100,'warn','{}',?1,0)",[decision+1]).unwrap();
    conn.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('warning',?1,'t',100,'b','invitation','inv')",[decision+1]).unwrap();
    conn.execute(
        "UPDATE local_node SET decision_seq=?1 WHERE id='34750f98bd59fcfc946da45aaabe933b'",
        [decision + 1],
    )
    .unwrap();
    let page = query(&context, PageRequest::default(), &OutputSpec::default());
    // Merged order: the priority-0 receipt (deadline), the deadline-less invitation and warning
    // by publication, then the priority-1 row.
    assert!(matches!(
        page.items.as_slice(),
        [
            InboxBatchV2Item::Message { .. },
            InboxBatchV2Item::Invitation { .. },
            InboxBatchV2Item::Warning { .. },
            InboxBatchV2Item::LazyMessage { .. }
        ]
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
        receipt_completion: true,
        in_turn_offer: false,
        page,
        claim: claim(),
        cursors: false,
    };
    let (context_text, lazy) = append(String::new(), &offer, &["herdr-threads".to_owned()], 4096);
    assert!(lazy.is_empty(), "ordinary rows are never lazy-completed");
    assert!(
        !context_text.contains(&format!("herdr-threads ack {}", ordinary.as_str())),
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
        receipt_candidates: Vec::new(),
        via: None,
        messages: vec![id.clone(), MessageId::new("missing")],
        operation: OperationId::new("mixed-bad"),
        claim: claim(),
    };
    assert_eq!(
        complete(&iso, request).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    // ht-54r.3: a different harness or native session is a different binding key, so a plain
    // call from it enrolls its own binding on the seat instead of being refused as "occupant
    // context changed"; those two cases are no longer bad claims. The rest still are.
    for bad in [1, 3, 4] {
        let mut caller = claim();
        match bad {
            1 => caller.execution = ExecutionId::new("other"),
            3 => caller.target = HostTargetId::new("pa"),
            _ => caller = send_request().claim,
        }
        let request = CompleteInboxDelivery {
            receipt_candidates: Vec::new(),
            via: None,
            messages: vec![id.clone()],
            operation: OperationId::new(format!("bad-claim-{bad}")),
            claim: caller,
        };
        assert!(complete(&iso, request).is_err(), "bad case {bad}");
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
                receipt_candidates: Vec::new(),
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
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap();
    let request = CompleteInboxDelivery {
        receipt_candidates: Vec::new(),
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
            receipt_candidates: Vec::new(),
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
            StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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

#[test]
fn invitation_confirmation_inbox_metadata_distinguishes_lazy_timed_and_legacy() {
    let (_iso, context, conn) = fixture("invitation-metadata");
    for (index, mode) in [Some("lazy"), Some("require_ack"), None]
        .into_iter()
        .enumerate()
    {
        conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id,origin_node) VALUES (?1,'t','b',?2,'pending',0,1,300,300,?3,CASE WHEN ?3 IS NOT NULL THEN 'a' END,(SELECT id FROM local_node))",rusqlite::params![format!("policy-{index}"),index as i64+1,mode]).unwrap();
    }
    for command in [
        Command::InboxBatch(InboxQuery {
            pushed: false,
            thread: None,
            seat: Some(SeatId::new("b")),
            page: PageRequest::default(),
            claim: Some(claim()),
        }),
        Command::InboxBatchV2(InboxQuery {
            pushed: false,
            thread: None,
            seat: Some(SeatId::new("b")),
            page: PageRequest::default(),
            claim: Some(claim()),
        }),
    ] {
        let result = herdr_threads::store::queries::query_with_output(
            &context,
            "34750f98bd59fcfc946da45aaabe933b",
            &command,
            &OutputSpec::default(),
            &budget(),
        )
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        let items = value["data"]["items"].as_array().unwrap();
        let find = |id: &str| items.iter().find(|v| v["invitation"] == id).unwrap();
        assert_eq!(find("policy-0")["confirmation"], "lazy");
        assert!(find("policy-0").get("deadline_at").is_none());
        assert_eq!(find("policy-1")["confirmation"], "require_ack");
        assert_eq!(find("policy-1")["deadline_at"], 300);
        assert!(find("policy-2").get("confirmation").is_none());
        assert!(
            find("policy-2").get("deadline_at").is_none(),
            "legacy item bytes stay unchanged"
        );
    }
}

/// ht-yha.65 (main review M1): a stored pre-upgrade lazy invitation keeps its no-wake shape. It is
/// listed at p0 (ahead of an older p1 row) but is never in the confirming set the wake readers use.
#[test]
fn a_pre_upgrade_lazy_invitation_is_listed_at_p0_and_never_wakes() {
    let (iso, context, mut conn) = fixture("pre-upgrade-lazy-invitation");
    let p1 = send_priority(&iso, &context, &mut conn, "p1", Some(1), None);
    let after: i64 = conn
        .query_row("SELECT decision_seq+1 FROM local_node", [], |r| r.get(0))
        .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id,origin_node) VALUES('inv-lazy','t','b',2,'pending',0,?1,300,300,'lazy','a',(SELECT id FROM local_node))", [after]).unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id,origin_node) VALUES('inv-ack','t','b',3,'pending',0,?1,300,300,'require_ack','a',(SELECT id FROM local_node))", [after]).unwrap();
    conn.execute("UPDATE local_node SET decision_seq=?1", [after])
        .unwrap();
    let page = query(
        &context,
        PageRequest {
            limit: 50,
            max_bytes: 65536,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    let ids: Vec<_> = page.items.iter().map(item_id).collect();
    let position = |id: &str| ids.iter().position(|x| x == id).unwrap();
    assert!(
        position("inv-lazy") < position(p1.as_str()),
        "the lazy invitation sorts as p0, ahead of the older p1 row: {ids:?}"
    );
    let lazy = page
        .items
        .iter()
        .find(|item| item_id(item) == "inv-lazy")
        .unwrap();
    let InboxBatchV2Item::Invitation { confirmation, .. } = lazy else {
        panic!("expected an invitation, got {lazy:?}")
    };
    assert_eq!(
        *confirmation,
        Some(herdr_threads::protocol::commands::InvitationConfirmation::Lazy)
    );
    let confirming: Vec<String> =
        herdr_threads::store::attention::pending_confirming_invitations(&conn, "b", None, after)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect();
    assert_eq!(confirming, ["inv-ack"], "the lazy invitation never wakes");
}

/// Delivery-policy DP8: the InboxBatchV2 query marks a lazy row `wakes` only
/// when the reading seat's `wake_on` is `all`, and a push-channel ACK completes
/// the promoted row as a lazy row.
#[test]
fn lazy_inbox_wakes_follows_wake_on_and_mod_ack_completes_it() {
    let (iso, context, mut conn) = fixture("lazy-wakes");
    let lazy = send(
        &iso,
        &context,
        &mut conn,
        "lazy",
        "quiet",
        DeliveryMode::Lazy,
    );
    let first_page = |context: &StoreContext| {
        query(
            context,
            PageRequest {
                limit: 5,
                max_bytes: 65536,
                ..Default::default()
            },
            &OutputSpec::default(),
        )
    };
    let wakes_of =
        |page: &herdr_threads::protocol::pagination::Page<InboxBatchV2Item>| match &page.items[0] {
            InboxBatchV2Item::LazyMessage { message, wakes, .. } => {
                assert_eq!(message, &lazy);
                *wakes
            }
            other => panic!("expected lazy row, got {other:?}"),
        };
    assert!(
        !wakes_of(&first_page(&context)),
        "default seat does not wake"
    );
    conn.execute("UPDATE seats SET nudge_threshold=-1 WHERE id='b'", [])
        .unwrap();
    let page = first_page(&context);
    assert!(wakes_of(&page), "wake_on=all seat wakes");
    // Wire: the waking row carries the key, nothing else changed.
    let wire = serde_json::to_value(&page.items[0]).unwrap();
    assert_eq!(wire["wakes"], serde_json::json!(true));
    conn.execute("UPDATE seats SET nudge_threshold=0 WHERE id='b'", [])
        .unwrap();
    let wire = serde_json::to_value(&first_page(&context).items[0]).unwrap();
    assert!(wire.as_object().unwrap().get("wakes").is_none());

    // A push-channel ACK (`watch ack --via submit`) completes it as a lazy row.
    conn.execute("UPDATE seats SET nudge_threshold=-1 WHERE id='b'", [])
        .unwrap();
    conn.execute(
        "UPDATE occupant_bindings SET harness='claude' WHERE seat_id='b'",
        [],
    )
    .unwrap();
    let mut claim = claim();
    claim.harness = Harness::Claude;
    use herdr_threads::ports::StorePort;
    let store = herdr_threads::store::SqliteStore::new(
        StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap();
    let mutation =
        PermitMutation::AckModDelivered(herdr_threads::protocol::commands::AckModDelivered {
            via: herdr_threads::protocol::watch::ModDeliveryVia::Submit,
            messages: vec![lazy.clone()],
            operation: OperationId::new("promoted-ack"),
            claim,
        });
    let permit = store
        .issue_cooperative_permit(
            herdr_threads::store::cooperative_permit_request(&mutation).unwrap(),
            &budget(),
        )
        .unwrap();
    let CommandResult::ModDeliveryAcked(report) =
        store.mutate(mutation, permit, &budget()).unwrap()
    else {
        panic!("mod ack");
    };
    assert_eq!(
        report.results[0].result,
        herdr_threads::protocol::watch::ModAckOutcome::Settled
    );
    assert!(
        first_page(&context).items.is_empty(),
        "the completed lazy row leaves the inbox"
    );
}

// ---- inbox v2 min-head merge (ht-yha.5.4) ----

/// `send` with an explicit priority (an ordinary-mode send naming no ACK recipient: delivery rows
/// for every member) and an optional deadline (which names the explicit ACK recipient `b`).
fn send_priority(
    iso: &TestIsolation,
    context: &StoreContext,
    conn: &mut Connection,
    op: &str,
    priority: Option<u8>,
    deadline_millis: Option<u64>,
) -> MessageId {
    use herdr_threads::protocol::priority::DeliveryPriority;
    let mut request = send_request();
    request.operation = OperationId::new(op);
    request.body = op.into();
    request.delivery_mode = DeliveryMode::Ordinary;
    request.priority = priority.map(|p| DeliveryPriority::new(p).unwrap());
    if deadline_millis.is_some() {
        request.invited_recipients = vec![SeatId::new("b")];
        request.deadline_millis = deadline_millis;
    }
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

/// A published delivery row for seat `b` (message `raw-<n>` at sequence `n`, priority `p`,
/// publication decision `d`), written with the send path's statements so large backlogs and exact
/// decisions need no per-row send.
fn raw_row(conn: &Connection, n: i64, p: u8, d: i64, body: &str) -> MessageId {
    use rusqlite::params;
    let id = format!("msg-raw-{n}");
    let prep = format!("prep-raw-{n}");
    conn.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',?2)", params![prep, p]).unwrap();
    conn.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,'t','b',?3)", params![prep, id, p]).unwrap();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,actor_seat_id,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary',?3,?4,0,'lazy','a',?5)", params![id, n, d, body, p]).unwrap();
    conn.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'34750f98bd59fcfc946da45aaabe933b','t',?3,0,?4,0,1,0)", params![prep, id, d, n]).unwrap();
    conn.execute("UPDATE lazy_recipients SET decision_seq=?2 WHERE preparation_id=?1 AND decision_seq IS NULL", params![prep, d]).unwrap();
    conn.execute(
        "UPDATE local_node SET decision_seq=max(decision_seq,?1)",
        [d],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=max(next_sequence,?1) WHERE id='t'",
        [n + 1],
    )
    .unwrap();
    MessageId::new(id)
}

fn item_id(item: &InboxBatchV2Item) -> String {
    match item {
        InboxBatchV2Item::Message { message, .. }
        | InboxBatchV2Item::LazyMessage { message, .. } => message.as_str().to_owned(),
        InboxBatchV2Item::Invitation { invitation, .. } => invitation.as_str().to_owned(),
        InboxBatchV2Item::Warning { warning, .. } => warning.as_str().to_owned(),
        other => panic!("unexpected item {other:?}"),
    }
}

/// The ids of every page of a traversal, in order, and the page count.
fn traverse(
    context: &StoreContext,
    limit: u16,
    max_bytes: u32,
    spec: &OutputSpec,
) -> (Vec<String>, usize) {
    let (mut cursor, mut ids, mut pages) = (None, Vec::new(), 0);
    loop {
        let page = query(
            context,
            PageRequest {
                cursor: cursor.clone(),
                limit,
                max_bytes,
            },
            spec,
        );
        pages += 1;
        ids.extend(page.items.iter().map(item_id));
        if !page.has_more {
            return (ids, pages);
        }
        assert_ne!(page.next_cursor, cursor, "every page advances");
        cursor = page.next_cursor;
        assert!(pages < 500, "traversal must terminate");
    }
}

#[test]
fn p0_row_behind_500_p1_rows_is_first_in_one_page() {
    let (_iso, context, conn) = fixture("merge-p0-first");
    for n in 0..500 {
        raw_row(&conn, 1000 + n, 1, 100 + n, "p1");
    }
    let p0 = raw_row(&conn, 2000, 0, 700, "p0");
    let page = query(
        &context,
        PageRequest {
            limit: 5,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    let ids: Vec<_> = page.items.iter().map(item_id).collect();
    assert_eq!(
        ids,
        [
            p0.as_str(),
            "msg-raw-1000",
            "msg-raw-1001",
            "msg-raw-1002",
            "msg-raw-1003"
        ],
        "the late p0 row precedes the 500 older p1 rows, which follow in publication order"
    );
    assert!(page.has_more);
}

#[test]
fn partial_body_resumes_first_and_becomes_last_key() {
    use herdr_threads::protocol::{
        pagination::{InboxBatchV2CursorState as State, InboxBatchV2Source},
        priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
    };
    let (_iso, context, conn) = fixture("merge-partial-body");
    let big = raw_row(&conn, 1000, 0, 10, &"x".repeat(3000));
    let small = raw_row(&conn, 1001, 0, 11, "after");
    let page = |cursor: Option<String>, limit, max_bytes| {
        query(
            &context,
            PageRequest {
                cursor,
                limit,
                max_bytes,
            },
            &OutputSpec::default(),
        )
    };
    let first = page(None, 10, 2048);
    let [
        InboxBatchV2Item::LazyMessage {
            message,
            body_end,
            body_len,
            ..
        },
    ] = first.items.as_slice()
    else {
        panic!("one chunk of the big row: {:?}", first.items);
    };
    assert_eq!((message, *body_len), (&big, 3000));
    assert!(*body_end > 0 && *body_end < 3000);
    let cursor = State::decode(first.next_cursor.as_ref().unwrap()).unwrap();
    assert_eq!(cursor.body.as_ref().unwrap().0, InboxBatchV2Source::Lazy);
    assert_eq!(cursor.last, None, "a partly shown body is not yet emitted");
    // The next page resumes that body before anything else, even with room for one row only.
    let second = page(first.next_cursor, 1, 16384);
    let [
        InboxBatchV2Item::LazyMessage {
            message,
            body_start,
            body_end: end,
            ..
        },
    ] = second.items.as_slice()
    else {
        panic!("the resumed remainder: {:?}", second.items);
    };
    assert_eq!((message, *body_start, *end), (&big, *body_end, 3000));
    let ordinal: i64 = conn
        .query_row(
            "SELECT ordinal FROM lazy_recipients WHERE message_id=?1",
            [big.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let key = DeliveryOrderKey {
        priority: DeliveryPriority::new(0).unwrap(),
        deadline: None,
        decision_seq: 10,
        rank: SourceRank::DeliveryRow,
        ordinal: ordinal as u64,
    };
    let cursor = State::decode(second.next_cursor.as_ref().unwrap()).unwrap();
    assert_eq!(cursor.last, Some(key), "its key is the last emitted key");
    assert!(cursor.body.is_none());
    let third = page(second.next_cursor, 10, 16384);
    assert_eq!(
        third.items.iter().map(item_id).collect::<Vec<_>>(),
        [small.as_str()]
    );
    assert!(!third.has_more);
}

#[test]
fn seventy_receipts_paged_end_to_end_lose_and_repeat_nothing() {
    let (iso, context, mut conn) = fixture("merge-70-receipts");
    let sent: Vec<String> = (0..70)
        .map(|n| {
            send(
                &iso,
                &context,
                &mut conn,
                &format!("r{n}"),
                "receipt",
                DeliveryMode::Ordinary,
            )
            .as_str()
            .to_owned()
        })
        .collect();
    let (seen, pages) = traverse(&context, 5, 65536, &OutputSpec::default());
    assert_eq!(pages, 14);
    // Equal deadlines order by publication: the first window of 64 and the next are exact.
    assert_eq!(seen, sent);
}

#[test]
fn merged_order_across_classes() {
    use rusqlite::params;
    let (iso, context, mut conn) = fixture("merge-order");
    let r1 = send_priority(&iso, &context, &mut conn, "r1", None, Some(1_000));
    let r3 = send_priority(&iso, &context, &mut conn, "r3", None, Some(1_000));
    let r2 = send_priority(&iso, &context, &mut conn, "r2", None, Some(5_000));
    let w0 = send_priority(&iso, &context, &mut conn, "w0", Some(0), None);
    let p1 = send_priority(&iso, &context, &mut conn, "p1", Some(1), None);
    let p5 = send_priority(&iso, &context, &mut conn, "p5", Some(5), None);
    let w0_decision: i64 = conn
        .query_row(
            "SELECT decision_seq FROM messages WHERE id=?1",
            [w0.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    // A confirming invitation due after every receipt, and a legacy (deadline-less) invitation
    // published in the same decision as the p0 row: the invitation ranks before the row.
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id,origin_node) VALUES('inv-ack','t','b',3,'pending',0,2,100,9000,'require_ack','a',(SELECT id FROM local_node))", []).unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,origin_node) VALUES('inv-legacy','t','b',2,'pending',0,?1,300,300,(SELECT id FROM local_node))", [w0_decision]).unwrap();
    // An actionable warning about the legacy invitation, published after everything else.
    let warned: i64 = conn
        .query_row("SELECT decision_seq+1 FROM local_node", [], |r| r.get(0))
        .unwrap();
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES('34750f98bd59fcfc946da45aaabe933b','warning','t',100,'warn','{}',?1,0)", [warned]).unwrap();
    conn.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('warning',?1,'t',100,'b','invitation','inv-legacy')", params![warned]).unwrap();
    conn.execute("UPDATE local_node SET decision_seq=?1", [warned])
        .unwrap();
    let page = query(
        &context,
        PageRequest {
            limit: 50,
            max_bytes: 65536,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert!(!page.has_more);
    let ids: Vec<_> = page.items.iter().map(item_id).collect();
    assert_eq!(
        ids,
        [
            // p0 with deadlines: ascending deadline, equal deadlines by publication.
            r1.as_str(),
            r3.as_str(),
            r2.as_str(),
            "inv-ack",
            // p0 without a deadline: by publication, equal publication by source rank
            // (invitation before delivery row), then the warning published last.
            "inv-legacy",
            w0.as_str(),
            "warning",
            // Lower priorities follow, ascending.
            p1.as_str(),
            p5.as_str(),
        ]
    );
}

#[test]
fn read_only_json_paging_split_inside_equal_priority_run_is_stable_and_complete() {
    let (_iso, context, conn) = fixture("merge-json-paging");
    let rows: Vec<String> = (0..10)
        .map(|n| {
            raw_row(&conn, 1000 + n, 1, 100 + n, "run")
                .as_str()
                .to_owned()
        })
        .collect();
    let json = OutputSpec {
        format: OutputFormat::Json,
        ..Default::default()
    };
    let (seen, pages) = traverse(&context, 3, 65536, &json);
    assert_eq!(pages, 4);
    assert_eq!(
        seen, rows,
        "no completion between pages: nothing lost, repeated or reordered"
    );
    // The same traversal is repeatable: the order does not depend on who reads when.
    assert_eq!(traverse(&context, 3, 65536, &json).0, rows);
}

#[test]
fn work_limited_pages_resume_without_skips() {
    let (_iso, context, conn) = fixture("merge-work-limited");
    // 150 published rows the reader's binding has already passed: each is examined and rejected.
    for n in 1..=150 {
        raw_row(&conn, n, 1, 10 + n, "passed");
    }
    let visible: Vec<String> = (151..=153)
        .map(|n| raw_row(&conn, n, 1, 200 + n, "unread").as_str().to_owned())
        .collect();
    conn.execute(
        "UPDATE binding_cursors SET seq=150 WHERE thread_id='t' AND binding_id IN (SELECT binding_id FROM occupant_bindings WHERE seat_id='b')",
        [],
    )
    .unwrap();
    // The cursor advance clears the binding's pending view of the passed rows; restore them as stale
    // view entries so the post-seek filter still examines and rejects each (the work bound's input).
    conn.execute(
        "INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq) SELECT ob.binding_id,r.ordinal,r.thread_id,r.priority,r.decision_seq FROM lazy_recipients r JOIN messages m ON m.id=r.message_id JOIN occupant_bindings ob ON ob.seat_id=r.seat_id WHERE m.sequence<=150",
        [],
    )
    .unwrap();
    let bound = |cursor: Option<String>| {
        let result = herdr_threads::store::queries::query_with_output(
            &context,
            "34750f98bd59fcfc946da45aaabe933b",
            &Command::InboxBatchV2(InboxQuery {
                pushed: false,
                thread: None,
                claim: Some(claim()),
                seat: Some(SeatId::new("b")),
                page: PageRequest {
                    cursor,
                    limit: 50,
                    max_bytes: 65536,
                },
            }),
            &OutputSpec::default(),
            &budget(),
        )
        .unwrap();
        let CommandResult::InboxBatchV2(page) = result else {
            panic!("v2");
        };
        page
    };
    let first = bound(None);
    assert!(first.items.is_empty());
    assert!(
        first.has_more,
        "a work-limited page is a continuation, never an empty end"
    );
    assert_eq!(
        first.stop_reason,
        herdr_threads::protocol::pagination::StopReason::Work
    );
    let text = String::from_utf8(
        encode_selected(
            &CommandResult::InboxBatchV2(first.clone()),
            &OutputSpec {
                format: OutputFormat::Text,
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(text.contains("next:") && !text.contains("empty"));
    let (mut seen, mut cursor) = (Vec::new(), first.next_cursor);
    for _ in 0..10 {
        let page = bound(cursor.take());
        seen.extend(page.items.iter().map(item_id));
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }
    assert_eq!(seen, visible, "every pending row exactly once, in order");
}

#[test]
fn arrivals_after_capture_are_deferred() {
    let (_iso, context, conn) = fixture("merge-arrivals");
    let first = raw_row(&conn, 1000, 1, 10, "one");
    let second = raw_row(&conn, 1001, 1, 11, "two");
    let page = query(
        &context,
        PageRequest {
            limit: 1,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert_eq!(
        page.items.iter().map(item_id).collect::<Vec<_>>(),
        [first.as_str()]
    );
    // A row published after the traversal's capture, even one that sorts earlier (p0).
    let late = raw_row(&conn, 1002, 0, 12, "late");
    let rest = query(
        &context,
        PageRequest {
            cursor: page.next_cursor,
            ..Default::default()
        },
        &OutputSpec::default(),
    );
    assert_eq!(
        rest.items.iter().map(item_id).collect::<Vec<_>>(),
        [second.as_str()]
    );
    assert!(!rest.has_more);
    let fresh = query(&context, PageRequest::default(), &OutputSpec::default());
    assert_eq!(
        fresh.items.iter().map(item_id).collect::<Vec<_>>(),
        [late.as_str(), first.as_str(), second.as_str()]
    );
}

#[test]
fn v1_cursor_is_invalid_cursor_with_the_older_daemon_detail() {
    // A format-1 payload starts with version byte 1; the codec rejects it before reading further.
    let v1 = "ib2:AQAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned();
    let (_iso, context, _conn) = fixture("merge-v1-cursor");
    let error = herdr_threads::store::queries::query(
        &context,
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            pushed: false,
            thread: None,
            claim: None,
            seat: Some(SeatId::new("b")),
            page: PageRequest {
                cursor: Some(v1),
                ..Default::default()
            },
        }),
        &budget(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCursor);
    assert_eq!(error.detail, "cursor from an older daemon; run inbox again");
}
