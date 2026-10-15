use herdr_threads::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::SendMessage,
    ids::*,
    results::ErrorCode,
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

/// A store with joined cooperative senders `a` and `b` in thread `t`, rooted in a
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
    // A second local sender `b` in the same thread: its traffic is the unrelated timeline
    // traffic the narrowed fence must tolerate.
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES ('b','34750f98bd59fcfc946da45aaabe933b','resolved','native','pb',1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pb','b',1,1,0,'fresh','term-pb','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('b',1,'pb','b',1,1,'codex','nb','00000000-0000-4000-8000-0000000000bb','cooperative_top_level',0,'term-pb','inc')", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','b','joined')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','b',1,2)", []).unwrap();
    (context, conn)
}

fn send_request() -> SendMessage {
    request_for("a", "op", "n", "pa", "aa")
}

fn request_for(seat: &str, op: &str, session: &str, target: &str, exec: &str) -> SendMessage {
    SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Lazy,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new(op),
        claim: CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new(seat),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(session),
            execution: ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{exec}")),
            target: HostTargetId::new(target),
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

fn publish(
    iso: &TestIsolation,
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
) -> Result<
    herdr_threads::protocol::results::CommandResult,
    herdr_threads::protocol::results::ApiError,
> {
    messages::publish_send(
        context,
        conn,
        request,
        &mut permit(iso, request),
        &budget(),
        || 65536,
    )
}

fn b_request(n: usize) -> SendMessage {
    let mut r = request_for("b", &format!("b-op-{n}"), "nb", "pb", "bb");
    r.body = format!("from b {n}");
    r
}

fn a_request(n: usize) -> SendMessage {
    let mut r = send_request();
    r.operation = OperationId::new(format!("a-op-{n}"));
    r.body = format!("from a {n}");
    r
}

// Kills: a timeline fence that rejects a warning-free preparation because another
// sender appended to the thread between its preparation and its publish.
#[test]
fn interleaved_local_traffic_does_not_conflict_lazy_sends() {
    let iso = TestIsolation::new("send-fence-interleave");
    let (context, mut conn) = setup(&iso);
    let prepared: Vec<SendMessage> = (0..10).map(a_request).collect();
    for r in &prepared {
        ready(&context, &mut conn, r);
    }
    let before: i64 = conn
        .query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for n in 0..20 {
        let r = b_request(n);
        ready(&context, &mut conn, &r);
        publish(&iso, &context, &mut conn, &r).unwrap();
        if n % 2 == 1 {
            let a = &prepared[n / 2];
            let sent = publish(&iso, &context, &mut conn, a);
            assert!(sent.is_ok(), "send {} failed: {sent:?}", n / 2);
        }
    }
    let after: i64 = conn
        .query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after >= before + 30, "timeline moved {before} -> {after}");
    let sent: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE body LIKE 'from a %'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sent, 10);
}

fn seal_with_warning(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    warnings: i64,
) {
    ready(context, conn, request);
    conn.execute("UPDATE send_preparations SET warning_count=?1", [warnings])
        .unwrap();
}

// Kills: a narrowing that drops the timeline fence for preparations that reserved a
// warning span, or one that keeps it for warning-free preparations.
#[test]
fn prep_with_unavailable_warning_conflicts_on_timeline_bump() {
    let iso = TestIsolation::new("send-fence-warning");
    let (context, mut conn) = setup(&iso);
    let request = a_request(0);
    seal_with_warning(&context, &mut conn, &request, 1);
    conn.execute(
        "UPDATE threads SET timeline_revision=timeline_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    let error = publish(&iso, &context, &mut conn, &request).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict, "{}", error.detail);
    assert!(
        error
            .detail
            .starts_with("send preparation snapshot changed"),
        "{}",
        error.detail
    );

    // The same bump leaves a warning-free preparation publishable.
    let free = a_request(1);
    ready(&context, &mut conn, &free);
    conn.execute(
        "UPDATE threads SET timeline_revision=timeline_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    publish(&iso, &context, &mut conn, &free).unwrap();
}

// Kills: a restart check that stays on the full fence: a warning-free, partly built
// preparation restarted by timeline traffic, or a warning one that is not.
#[test]
fn restart_check_applies_the_timeline_fence_only_with_warnings() {
    let iso = TestIsolation::new("send-fence-restart");
    let (context, mut conn) = setup(&iso);
    let free = a_request(0);
    let warned = a_request(1);
    ready(&context, &mut conn, &free);
    ready(&context, &mut conn, &warned);
    conn.execute(
        "UPDATE send_preparations SET status='building' WHERE operation_key IN ('a-op-0','a-op-1')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE send_preparations SET warning_count=1 WHERE operation_key='a-op-1'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET timeline_revision=timeline_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    assert!(prepare(&context, &mut conn, &free, 1).is_ok());
    let error = prepare(&context, &mut conn, &warned, 1).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict, "{}", error.detail);
    assert!(
        error
            .detail
            .starts_with("send preparation snapshot changed"),
        "{}",
        error.detail
    );
}

// Kills: a narrowing that loosens the audience fences along with the timeline one.
#[test]
fn membership_lifecycle_eligibility_config_bumps_still_conflict() {
    for (i, bump) in [
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
        "UPDATE herdr_hosts SET lifecycle_revision=lifecycle_revision+1",
        "UPDATE local_node SET send_eligibility_revision=send_eligibility_revision+1",
        "UPDATE local_node SET duration_config_revision=duration_config_revision+1",
    ]
    .into_iter()
    .enumerate()
    {
        let iso = TestIsolation::new(&format!("send-fence-audience-{i}"));
        let (context, mut conn) = setup(&iso);
        let request = a_request(0);
        ready(&context, &mut conn, &request);
        conn.execute(bump, []).unwrap();
        let error = publish(&iso, &context, &mut conn, &request).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict, "{bump}: {}", error.detail);
        assert!(
            error
                .detail
                .starts_with("send preparation snapshot changed"),
            "{bump}: {}",
            error.detail
        );
    }
}
