use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::SendMessage,
    ids::*,
    results::ErrorCode,
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use crate::store::{connection::StoreContext, messages};
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

// Kills: a send to an unknown thread answered as transient StoreBusy (the
// unclassified QueryReturnedNoRows mapping). The cooperative runner keeps
// such an intent pending forever. A missing thread is NotFound, decided
// before any preparation write.
#[test]
fn send_to_unknown_thread_is_not_found_and_commits_nothing() {
    let iso = TestIsolation::new("send-to-unknown-thread");
    let (context, mut conn) = setup(&iso);
    let mut request = send_request();
    request.thread = ThreadId::new("missing");
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    let error = messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget(),
        admission,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound, "{}", error.detail);
    for table in [
        "operations",
        "send_preparations",
        "prepared_recipients",
        "messages",
        "send_manifests",
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} must stay empty");
    }
}

// Kills: a publish that trusts the preparation sealed before the thread was
// archived (the publish-time recheck of `threads.archived`): the send would
// commit a message into an archived thread. Archiving here changes no revision
// the snapshot compares, so only the archived check can refuse it, and it must
// refuse with the Archived code (not the generic snapshot Conflict).
#[test]
fn publish_rechecks_archive_state() {
    let iso = TestIsolation::new("publish-rechecks-archive-state");
    let (context, mut conn) = setup(&iso);
    let request = send_request();
    let admission = crate::ports::DurableWorkAdmission { max_units: 16 };
    let limits = messages::MessageLimits::default();
    assert!(matches!(
        messages::prepare_send_step(&context, &mut conn, &request, limits, &budget(), admission)
            .unwrap(),
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    // The thread is archived between prepare and publish.
    conn.execute("UPDATE threads SET archived=1 WHERE id='t'", [])
        .unwrap();
    let error = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget(),
        || limits.body_bytes,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Archived);
    for table in ["messages", "send_manifests", "receipts"] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} must stay empty");
    }
}
