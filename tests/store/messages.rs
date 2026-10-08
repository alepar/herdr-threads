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
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
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

#[test]
fn native_preparation_quantum_refreshes_inactivity_and_sealed_retry_does_not() {
    use std::sync::atomic::{AtomicI64, Ordering};
    struct ProgressClock(AtomicI64);
    impl Clock for ProgressClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0.load(Ordering::SeqCst))
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    let iso = TestIsolation::new("native-preparation-progress");
    let (_, mut db) = setup(&iso);
    for seat in ["b", "c"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','unresolved','native',0,0)", [seat]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let clock = Arc::new(ProgressClock(AtomicI64::new(1000)));
    let context = StoreContext::new(iso.path("store.db"), clock.clone());
    let request = send_request();
    let mut ready = false;
    let mut last = 0;
    for now in [2000, 3000, 4000] {
        clock.0.store(now, Ordering::SeqCst);
        let step = messages::prepare_send_step(
            &context,
            &mut db,
            &request,
            messages::MessageLimits::default(),
            &budget(),
            crate::ports::DurableWorkAdmission { max_units: 1 },
        )
        .unwrap();
        last = db
            .query_row("SELECT prepared_at FROM send_preparations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(last, now);
        if matches!(step, crate::ports::SendPreparationProgress::Ready { .. }) {
            ready = true;
            break;
        }
    }
    assert!(ready);
    clock.0.store(5000, Ordering::SeqCst);
    messages::prepare_send_step(
        &context,
        &mut db,
        &request,
        messages::MessageLimits::default(),
        &budget(),
        crate::ports::DurableWorkAdmission { max_units: 1 },
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT prepared_at FROM send_preparations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        last
    );
}

#[test]
fn user_intent_send_payload_preserves_legacy_digest() {
    use crate::protocol::summary::UserIntent;
    use crate::store::schema::canonical_digest;
    let mut request = send_request();
    let legacy = serde_json::json!({"kind":"send_message","thread":request.thread,
        "body":request.body,"invited_recipients":request.invited_recipients,
        "deadline_millis":request.deadline_millis,"claim":request.claim});
    assert_eq!(messages::send_payload(&request), legacy);
    assert_eq!(
        canonical_digest(&messages::send_payload(&request)).unwrap(),
        canonical_digest(&legacy).unwrap()
    );
    request.relays_user = true;
    let without = canonical_digest(&messages::send_payload(&request)).unwrap();
    let mut digests = vec![without];
    for intent in [UserIntent::Query, UserIntent::Request, UserIntent::Rule] {
        request.user_intent = Some(intent);
        let payload = messages::send_payload(&request);
        assert_eq!(payload["user_intent"], serde_json::json!(intent));
        let digest = canonical_digest(&payload).unwrap();
        assert!(!digests.contains(&digest));
        digests.push(digest);
    }
}

// Catches omitting lazy from frozen replay identity or changing ordinary digests.
#[test]
fn lazy_send_payload_changes_digest_only_for_lazy_mode() {
    use crate::protocol::{commands::DeliveryMode, summary::UserIntent};
    use crate::store::schema::canonical_digest;
    let mut request = send_request();
    request.relays_user = true;
    request.user_intent = Some(UserIntent::Rule);
    let legacy = serde_json::json!({"kind":"send_message","thread":"t","body":"hello",
        "invited_recipients":[],"deadline_millis":null,"claim":request.claim,
        "relays_user":true,"user_intent":"rule"});
    let ordinary_digest = canonical_digest(&legacy).unwrap();
    assert_eq!(
        canonical_digest(&messages::send_payload(&request)).unwrap(),
        ordinary_digest
    );
    request.delivery_mode = DeliveryMode::Lazy;
    let mut expected_lazy = legacy;
    expected_lazy["delivery_mode"] = serde_json::json!("lazy");
    let lazy_digest = canonical_digest(&messages::send_payload(&request)).unwrap();
    assert_eq!(lazy_digest, canonical_digest(&expected_lazy).unwrap());
    assert_ne!(lazy_digest, ordinary_digest);
}

// Catches publishing valid lazy mail through the ordinary attention path.
#[test]
fn lazy_send_publishes_without_attention() {
    use crate::{ports::DurableWorkAdmission, protocol::commands::DeliveryMode};
    let iso = TestIsolation::new("lazy-send");
    let (context, mut conn) = setup(&iso);
    let mut request = send_request();
    request.delivery_mode = DeliveryMode::Lazy;
    let preparation = messages::prepare_send_step(
        &context,
        &mut conn,
        &request,
        messages::MessageLimits::default(),
        &budget(),
        DurableWorkAdmission::new(16).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        preparation,
        crate::ports::SendPreparationProgress::Ready { .. }
    ));
    let publication = messages::publish_send(
        &context,
        &mut conn,
        &request,
        &mut permit(&request),
        &budget(),
        || messages::MAX_BODY_BYTES,
    )
    .unwrap();
    assert!(matches!(
        publication,
        crate::protocol::results::CommandResult::MessageSent(_)
    ));
    assert_eq!(
        conn.query_row("SELECT delivery_mode FROM messages", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "lazy"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM send_manifests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    for table in [
        "receipts",
        "work_jobs",
        "prepared_recipients",
        "prepared_unavailable_warnings",
        "warning_conditions",
    ] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} must stay empty");
    }
}

// Catches retry identity dropping intent before or after publication.
#[test]
fn user_intent_changed_preparation_retry_conflicts() {
    use crate::{
        ports::{DurableWorkAdmission, SendPreparationProgress},
        protocol::summary::UserIntent,
    };
    for published in [false, true] {
        let iso = TestIsolation::new("user-intent-retry");
        let (context, mut conn) = setup(&iso);
        let mut request = send_request();
        request.relays_user = true;
        request.user_intent = Some(UserIntent::Query);
        loop {
            match messages::prepare_send_step(
                &context,
                &mut conn,
                &request,
                messages::MessageLimits::default(),
                &budget(),
                DurableWorkAdmission { max_units: 16 },
            )
            .unwrap()
            {
                SendPreparationProgress::Ready { .. } => break,
                SendPreparationProgress::More { .. } => {}
                SendPreparationProgress::Committed(_) => panic!("unexpected committed"),
            }
        }
        let digest: Vec<u8> = conn
            .query_row("SELECT digest FROM send_preparations", [], |r| r.get(0))
            .unwrap();
        if published {
            let result = messages::publish_send(
                &context,
                &mut conn,
                &request,
                &mut permit(&request),
                &budget(),
                || messages::MessageLimits::default().body_bytes,
            )
            .unwrap();
            assert_eq!(
                messages::publish_send(
                    &context,
                    &mut conn,
                    &request,
                    &mut permit(&request),
                    &budget(),
                    || messages::MessageLimits::default().body_bytes
                )
                .unwrap(),
                result
            );
        }
        request.user_intent = Some(UserIntent::Request);
        let error = messages::prepare_send_step(
            &context,
            &mut conn,
            &request,
            messages::MessageLimits::default(),
            &budget(),
            DurableWorkAdmission { max_units: 16 },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationPayloadMismatch);
        assert_eq!(
            conn.query_row("SELECT digest FROM send_preparations", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            digest
        );
        if published {
            assert_eq!(
                messages::publish_send(
                    &context,
                    &mut conn,
                    &request,
                    &mut permit(&request),
                    &budget(),
                    || messages::MessageLimits::default().body_bytes
                )
                .unwrap_err()
                .code,
                ErrorCode::OperationPayloadMismatch
            );
        }
        assert_eq!(
            conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            i64::from(published)
        );
    }
}

fn prepare_intent_send(context: &StoreContext, conn: &mut Connection, request: &SendMessage) {
    loop {
        match messages::prepare_send_step(
            context,
            conn,
            request,
            messages::MessageLimits::default(),
            &budget(),
            crate::ports::DurableWorkAdmission { max_units: 16 },
        )
        .unwrap()
        {
            crate::ports::SendPreparationProgress::Ready { .. } => break,
            crate::ports::SendPreparationProgress::More { .. } => {}
            crate::ports::SendPreparationProgress::Committed(_) => panic!("already committed"),
        }
    }
}

fn assert_no_intent_publication(conn: &Connection) {
    for table in ["messages", "send_manifests", "receipts"] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "{table} must stay empty");
    }
}

// Catches SQLite errors escaping instead of a bounded native eligibility refusal.
#[test]
fn user_intent_send_canonical_eligibility_refuses_unrelayed_agent() {
    use crate::protocol::summary::UserIntent;
    for intent in [UserIntent::Query, UserIntent::Request, UserIntent::Rule] {
        let iso = TestIsolation::new("intent-refused-agent");
        let (context, mut conn) = setup(&iso);
        let mut request = send_request();
        request.user_intent = Some(intent);
        prepare_intent_send(&context, &mut conn, &request);
        let error = messages::publish_send(
            &context,
            &mut conn,
            &request,
            &mut permit(&request),
            &budget(),
            || messages::MessageLimits::default().body_bytes,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.detail);
        assert_no_intent_publication(&conn);
    }
}

// Catches intent bypassing live accountable checks or the preparation revision fence.
#[test]
fn user_intent_send_rechecks_live_role() {
    use crate::protocol::summary::UserIntent;
    for (change, expected) in [
        ("mismatched_human_hint", ErrorCode::CallerUnverified),
        ("closed_binding", ErrorCode::CallerUnverified),
        ("role_replaced", ErrorCode::CallerUnverified),
        ("revision_changed", ErrorCode::Conflict),
        ("stale_generation", ErrorCode::CallerUnverified),
    ] {
        let iso = TestIsolation::new("intent-live-role");
        let (context, mut conn) = setup(&iso);
        let mut request = send_request();
        request.relays_user = true;
        request.user_intent = Some(UserIntent::Query);
        if change == "mismatched_human_hint" {
            request.claim.harness = Harness::Human;
        }
        prepare_intent_send(&context, &mut conn, &request);
        match change {
            "mismatched_human_hint" => {}
            "closed_binding" => {
                conn.execute("UPDATE occupant_bindings SET ended_at=1000", [])
                    .unwrap();
            }
            "role_replaced" => {
                conn.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human'", []).unwrap();
            }
            "revision_changed" => {
                conn.execute(
                    "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1",
                    [],
                )
                .unwrap();
            }
            "stale_generation" => {
                conn.execute("UPDATE seats SET generation=generation+1", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let error = messages::publish_send(
            &context,
            &mut conn,
            &request,
            &mut permit(&request),
            &budget(),
            || messages::MessageLimits::default().body_bytes,
        )
        .unwrap_err();
        assert_eq!(error.code, expected, "{change}: {}", error.detail);
        assert_no_intent_publication(&conn);
    }
}

// Compares real canonical staged/effective receipts across legacy and classified
// human input. Classification must not turn receipt obligations into summary work.
#[test]
fn user_intent_send_priority_and_receipts_unchanged() {
    use crate::protocol::{
        results::CommandResult,
        summary::{AuthorRole, UserIntent, is_priority},
    };
    use crate::store::effective::{EffectiveReceiptState, effective_receipt};
    for (human, relay) in [(true, false), (true, true), (false, true)] {
        let mut baseline = None;
        for intent in [
            None,
            Some(UserIntent::Query),
            Some(UserIntent::Request),
            Some(UserIntent::Rule),
        ] {
            let iso = TestIsolation::new("intent-priority-receipts");
            let (context, mut conn) = setup(&iso);
            let mut request = send_request();
            if human {
                request.claim.harness = Harness::Human;
                conn.execute("UPDATE occupant_bindings SET harness='human',observation_provenance='operator_human'", []).unwrap();
            }
            // One agent and one human recipient, both joined and registered.
            for (seat, harness, provenance) in [
                ("b", "codex", "cooperative_top_level"),
                ("c", "human", "operator_human"),
            ] {
                conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [seat]).unwrap();
                conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh',?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
                conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,?1,'b',1,1,?2,'n',?1,?3,0,0,?1,'inc')", rusqlite::params![seat,harness,provenance]).unwrap();
                conn.execute(
                    "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
                    [seat],
                )
                .unwrap();
                conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
            }
            request.relays_user = relay;
            request.user_intent = intent;
            // Explicit ACK selection still cannot give a human an obligation.
            request.invited_recipients = vec![SeatId::new("b"), SeatId::new("c")];
            prepare_intent_send(&context, &mut conn, &request);
            let CommandResult::MessageSent(id) = messages::publish_send(
                &context,
                &mut conn,
                &request,
                &mut permit(&request),
                &budget(),
                || messages::MessageLimits::default().body_bytes,
            )
            .unwrap() else {
                panic!("not sent")
            };
            let (role, relay): (String, bool) = conn
                .query_row(
                    "SELECT author_role,relays_user FROM messages WHERE id=?1",
                    [id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert!(is_priority(AuthorRole::from_column(&role), relay));
            let agent = effective_receipt(&conn, id.as_str(), "b").unwrap().unwrap();
            assert_eq!(agent.state, EffectiveReceiptState::Pending);
            assert_eq!(agent.ack_observation, None);
            assert!(
                effective_receipt(&conn, id.as_str(), "c")
                    .unwrap()
                    .is_none()
            );
            let provenance: String = conn
                .query_row(
                    "SELECT availability_provenance FROM prepared_recipients WHERE seat_id='b'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(provenance, "cooperative_top_level");
            let comparison = (
                agent.state,
                agent.available_at,
                agent.deadline_at,
                agent.acked_at,
                agent.ack_observation,
                provenance,
            );
            if let Some(expected) = &baseline {
                assert_eq!(&comparison, expected);
            } else {
                baseline = Some(comparison);
            }
        }
    }
}
