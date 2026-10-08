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
#[test]
fn lazy_publication_rejects_ack_and_deadline_before_preparation() {
    let iso = TestIsolation::new("lazy-invalid");
    let (context, mut conn) = setup(&iso);
    for deadline in [false, true] {
        let mut request = send_request();
        if deadline {
            request.deadline_millis = Some(1);
        } else {
            request.invited_recipients.push(SeatId::new("b"));
        }
        assert_eq!(
            prepare(&context, &mut conn, &request, 1).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM send_preparations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            messages::publish_send(
                &context,
                &mut conn,
                &request,
                &mut permit(&iso, &request),
                &budget(),
                || 65536
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
}
#[test]
fn lazy_publication_creates_no_receipt_warning_or_attention() {
    let iso = TestIsolation::new("lazy-no-attention");
    let (context, mut conn) = setup(&iso);
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('b','i','unresolved','native',1,0)",[]).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t','b','joined')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','b',1,1)",[]).unwrap();
    let request = send_request();
    ready(&context, &mut conn, &request);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM lazy_recipients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    use herdr_threads::store::lazy_delivery;
    let seat = SeatId::new("b");
    let pending = lazy_delivery::pending_page(
        &conn,
        "i",
        &seat,
        0,
        lazy_delivery::capture_high_water(&conn, "i", &seat).unwrap(),
        16,
    )
    .unwrap();
    assert!(pending.recipients.is_empty());
    let first = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&iso, &request),
        &budget(),
        || 65536,
    )
    .unwrap();
    let replay = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&iso, &request),
        &budget(),
        || 65536,
    )
    .unwrap();
    assert_eq!(first, replay);
    let pending = lazy_delivery::pending_page(
        &conn,
        "i",
        &seat,
        0,
        lazy_delivery::capture_high_water(&conn, "i", &seat).unwrap(),
        16,
    )
    .unwrap();
    assert_eq!(pending.recipients.len(), 1);
    let attention = herdr_threads::store::attention::wake_seat_attention(&conn, "b")
        .unwrap()
        .attention;
    assert!(!attention.has_pending_receipt);
    assert!(!attention.has_pending_invitation);
    assert!(attention.latest_warning_seq.is_none());
    assert!(!herdr_threads::store::attention::seat_has_pending_rows(&conn, "b").unwrap());
    assert!(
        herdr_threads::store::attention::warning_backlog(&conn, "b", &|| Ok(()))
            .unwrap()
            .items
            .is_empty()
    );
    for table in [
        "prepared_recipients",
        "prepared_unavailable_warnings",
        "receipts",
        "warning_conditions",
        "digest_pending_manifest_receipts",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT recipient_count+warning_count FROM send_manifests",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT delivery_mode FROM messages", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "lazy"
    );
}

fn member(conn: &Connection, seat: &str, state: &str, membership: &str) {
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) VALUES(?1,'i',?2,'native',1,0,CASE WHEN ?2='retired' THEN 0 END,CASE WHEN ?2='retired' THEN 1 END)", rusqlite::params![seat,state]).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t',?1,?2)",
        rusqlite::params![seat, membership],
    )
    .unwrap();
    if membership == "joined" {
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,1)",[seat]).unwrap();
    }
}
fn publish(
    iso: &TestIsolation,
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
) -> herdr_threads::protocol::results::CommandResult {
    messages::publish_send(
        context,
        conn,
        request,
        &mut permit(iso, request),
        &budget(),
        || 65536,
    )
    .unwrap()
}
fn recipients(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT seat_id FROM lazy_recipients ORDER BY seat_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
fn preparation_id(conn: &Connection) -> String {
    conn.query_row("SELECT id FROM send_preparations", [], |r| r.get(0))
        .unwrap()
}
fn cleanup(
    context: &StoreContext,
    conn: &mut Connection,
    id: &str,
) -> herdr_threads::ports::WorkProgress {
    herdr_threads::store::materialization::advance_work(
        conn,
        &format!("work:cleanup:{id}"),
        herdr_threads::ports::DurableWorkAdmission { max_units: 1 },
        &budget(),
        context.clock(),
    )
    .unwrap()
}
#[test]
fn lazy_publication_freezes_joined_human_unavailable_audience() {
    let iso = TestIsolation::new("lazy-audience");
    let (context, mut conn) = setup(&iso);
    member(&conn, "human", "resolved", "joined");
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES('human',1,'ph','b',1,1,'human','human-session','human-execution','operator_human',0,'term-ph','inc')",[]).unwrap();
    member(&conn, "unavailable", "unresolved", "joined");
    member(&conn, "invitee", "unresolved", "invited");
    member(&conn, "retired", "retired", "joined");
    let request = send_request();
    ready(&context, &mut conn, &request);
    assert_eq!(recipients(&conn), ["human", "unavailable"]);
    publish(&iso, &context, &mut conn, &request);
    member(&conn, "later", "unresolved", "joined");
    assert_eq!(recipients(&conn), ["human", "unavailable"]);
    let id = preparation_id(&conn);
    assert!(!messages::abandon_send_preparation(&mut conn, &id).unwrap());
    assert_eq!(recipients(&conn), ["human", "unavailable"]);
}
#[test]
fn lazy_publication_revision_fence_restart_and_replay() {
    let iso = TestIsolation::new("lazy-revision");
    let (context, mut conn) = setup(&iso);
    member(&conn, "b", "unresolved", "joined");
    let request = send_request();
    assert!(matches!(
        prepare(&context, &mut conn, &request, 1).unwrap(),
        herdr_threads::ports::SendPreparationProgress::More { visited: 1, .. }
    ));
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    ready(&context, &mut conn, &request);
    member(&conn, "postseal", "unresolved", "joined");
    conn.execute(
        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
        [],
    )
    .unwrap();
    assert_eq!(
        messages::publish_send(
            &context,
            &mut conn,
            &request,
            &mut permit(&iso, &request),
            &budget(),
            || 65536
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        prepare(&context, &mut conn, &request, 1).unwrap_err().code,
        ErrorCode::Conflict
    );
    let old = preparation_id(&conn);
    while cleanup(&context, &mut conn, &old).has_more {}
    ready(&context, &mut conn, &request);
    let first = publish(&iso, &context, &mut conn, &request);
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    assert_eq!(publish(&iso, &context, &mut conn, &request), first);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(recipients(&conn), ["b", "postseal"]);
    let mut changed = request.clone();
    changed.body = "changed".into();
    assert_eq!(
        prepare(&context, &mut conn, &changed, 1).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
}
#[test]
fn lazy_cleanup_deletes_one_bounded_unpublished_unit() {
    let iso = TestIsolation::new("lazy-cleanup");
    let (context, mut conn) = setup(&iso);
    member(&conn, "b", "unresolved", "joined");
    member(&conn, "c", "unresolved", "joined");
    ready(&context, &mut conn, &send_request());
    let id = preparation_id(&conn);
    assert!(messages::abandon_send_preparation(&mut conn, &id).unwrap());
    let progress = cleanup(&context, &mut conn, &id);
    assert_eq!(progress.processed_this_turn, 1);
    assert!(progress.has_more);
    assert_eq!(recipients(&conn).len(), 1);
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    assert!(cleanup(&context, &mut conn, &id).has_more);
    assert!(recipients(&conn).is_empty());
    assert!(!cleanup(&context, &mut conn, &id).has_more);
    assert_eq!(
        conn.query_row("SELECT status FROM send_preparations", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "discarded"
    );
    assert_eq!(
        conn.query_row("SELECT length(digest) FROM send_preparations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        32
    );
}
#[test]
fn ordinary_publication_positive_control() {
    let iso = TestIsolation::new("ordinary-control");
    let (context, mut conn) = setup(&iso);
    member(&conn, "b", "unresolved", "joined");
    let mut request = send_request();
    request.delivery_mode = herdr_threads::protocol::commands::DeliveryMode::Ordinary;
    assert!(
        messages::send_payload(&request)
            .get("delivery_mode")
            .is_none()
    );
    ready(&context, &mut conn, &request);
    let first = publish(&iso, &context, &mut conn, &request);
    assert_eq!(publish(&iso, &context, &mut conn, &request), first);
    assert_eq!(
        conn.query_row("SELECT delivery_mode FROM messages", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "ordinary"
    );
    assert_eq!(
        conn.query_row("SELECT recipient_count FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='send_attention'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(recipients(&conn).is_empty());
    assert!(
        herdr_threads::store::attention::wake_seat_attention(&conn, "b")
            .unwrap()
            .attention
            .has_pending_receipt
    );
}

#[test]
fn lazy_publication_fences_every_canonical_revision() {
    for (table, column) in [
        ("threads", "timeline_revision"),
        ("host_instances", "lifecycle_revision"),
        ("host_instances", "send_eligibility_revision"),
        ("host_instances", "duration_config_revision"),
    ] {
        let iso = TestIsolation::new("lazy-fence");
        let (context, mut conn) = setup(&iso);
        member(&conn, "b", "unresolved", "joined");
        let request = send_request();
        ready(&context, &mut conn, &request);
        conn.execute(&format!("UPDATE {table} SET {column}={column}+1"), [])
            .unwrap();
        assert_eq!(
            messages::publish_send(
                &context,
                &mut conn,
                &request,
                &mut permit(&iso, &request),
                &budget(),
                || 65536
            )
            .unwrap_err()
            .code,
            ErrorCode::Conflict,
            "{column}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM send_manifests", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn lazy_cleanup_cannot_delete_published_children() {
    let iso = TestIsolation::new("lazy-published-cleanup");
    let (context, mut conn) = setup(&iso);
    member(&conn, "b", "unresolved", "joined");
    let request = send_request();
    ready(&context, &mut conn, &request);
    publish(&iso, &context, &mut conn, &request);
    let id = preparation_id(&conn);
    conn.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES(?1,'preparation_cleanup',?2,0)",rusqlite::params![format!("work:cleanup:{id}"),id]).unwrap();
    let progress = cleanup(&context, &mut conn, &id);
    assert_eq!(progress.processed_this_turn, 0);
    assert!(
        progress
            .last_error
            .unwrap()
            .contains("unpublished discarded")
    );
    assert_eq!(recipients(&conn), ["b"]);
}
