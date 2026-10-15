use super::*;
use crate::{
    ports::StorePort,
    protocol::commands::{DeliveryMode, EnsureDirectThread, PermitMutation, SendMessage},
    store::{SqliteStore, StoreSettings},
};

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn store_fixture() -> (SqliteStore, Connection, PathBuf) {
    let (context, conn, path, _) = bound_fixture(100);
    (
        SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
        conn,
        path,
    )
}
fn ensure(store: &SqliteStore, from: &str, to: &str, key: &str) -> Result<CommandResult, ApiError> {
    let v = EnsureDirectThread {
        creation_policy: None,
        recipient: SeatId::new(to),
        operation: OperationId::new(key),
        claim: claim(from),
    };
    let hash = cooperative_payload_hash("ensure_direct_thread", &v).unwrap();
    store.mutate(
        PermitMutation::EnsureDirectThread(v),
        permit(
            from,
            key,
            ObligationRef::CheckIn(SeatId::new(from)),
            hash,
            100,
        ),
        &budget(),
    )
}
fn thread(result: CommandResult) -> ThreadId {
    let CommandResult::ThreadCreated(t) = result else {
        panic!("unexpected result")
    };
    t
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn raw_pair(db: &Connection) {
    db.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('pair','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0); INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id) VALUES('34750f98bd59fcfc946da45aaabe933b','s1','s2','pair'); INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES('pair','s1','joined','joined',0),('pair','s2','invited','invited',NULL); INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('pair','s1',1,1); INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('s3','34750f98bd59fcfc946da45aaabe933b','resolved','native','s3',1,1,0); INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('34750f98bd59fcfc946da45aaabe933b','s3','b',1,1,0,'fresh','term-s3','inc','coherent_enumeration',1); INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES('s3',1,'s3','b',1,1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-s3','inc');").unwrap();
}
#[test]
fn atomic_create_reuse_invitation_and_historical_replay() {
    let (store, db, path) = store_fixture();
    let selected = thread(ensure(&store, "s1", "s2", "first").unwrap());
    assert_eq!(
        thread(ensure(&store, "s1", "s2", "second").unwrap()),
        selected
    );
    assert_eq!(count(&db, "direct_threads"), 1);
    assert_eq!(count(&db, "threads"), 1);
    assert_eq!(count(&db, "invitations"), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM invitations WHERE state='accepted'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(count(&db, "membership_intervals"), 1);
    assert_eq!(
        db.query_row("SELECT membership_revision FROM threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(db.query_row("SELECT json_extract(event_json,'$.actor_seat') FROM messages WHERE json_extract(event_json,'$.action')='create_thread'",[],|r|r.get::<_,String>(0)).unwrap(),"s1");
    assert_eq!(db.query_row("SELECT json_extract(event_json,'$.actor_seat'),json_extract(event_json,'$.seat') FROM messages WHERE json_extract(event_json,'$.action')='invite'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).unwrap(),("s1".into(),"s2".into()));
    assert_eq!(count(&db, "occupant_bindings"), 2);
    assert_eq!(
        db.query_row(
            "SELECT reason_bits FROM wake_work WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            & 1,
        1
    );

    let text: (String, String) = db
        .query_row("SELECT topic,goal FROM threads", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(text, ("1:1 s1 s2".into(), "1:1 s1 s2".into()));
    assert!(
        ensure(&store, "s2", "s1", "reverse")
            .unwrap_err()
            .detail
            .contains("accept")
    );
    db.execute("UPDATE threads SET archived=1", []).unwrap();
    assert_eq!(
        thread(ensure(&store, "s1", "s2", "first").unwrap()),
        selected
    );
    assert!(
        ensure(&store, "s1", "s2", "fresh")
            .unwrap_err()
            .detail
            .contains("reopen")
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn pair_refuses_third_ordinary_invite_without_effects() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    let v = Invite {
        confirmation: None,
        thread: ThreadId::new("pair"),
        seat: SeatId::new("s3"),
        deadline_millis: None,
        operation: OperationId::new("third"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &v).unwrap();
    let result = store.mutate(
        PermitMutation::Invite(v),
        permit(
            "s1",
            "third",
            ObligationRef::Control(ThreadId::new("pair")),
            hash,
            100,
        ),
        &budget(),
    );
    assert!(result.is_err());
    assert_eq!(count(&db, "memberships"), 2);
    assert_eq!(count(&db, "invitations"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn direct_lazy_invited_target_is_frozen_without_receipt() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    let v = SendMessage {
        delivery_mode: DeliveryMode::Lazy,
        thread: ThreadId::new("pair"),
        body: "quiet".into(),
        direct_recipient: Some(SeatId::new("s2")),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("lazy"),
        claim: claim("s1"),
        relays_user: false,
        user_intent: None,
    };
    loop {
        match store
            .prepare_send_step(
                &v,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget(),
            )
            .unwrap()
        {
            crate::ports::SendPreparationProgress::Ready { .. } => break,
            crate::ports::SendPreparationProgress::More { .. } => (),
            _ => panic!("unexpected replay"),
        }
    }
    assert_eq!(count(&db, "lazy_recipients"), 1);
    let hash = schema::canonical_digest(&crate::store::messages::send_payload(&v)).unwrap();
    store
        .mutate(
            PermitMutation::SendMessage(v),
            permit(
                "s1",
                "lazy",
                ObligationRef::Control(ThreadId::new("pair")),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    assert_eq!(count(&db, "lazy_recipients"), 1);
    assert_eq!(count(&db, "receipts"), 0);
    assert_eq!(count(&db, "wake_work"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

fn send_direct(store: &SqliteStore, thread: &ThreadId, mode: DeliveryMode, key: &str) {
    let v = SendMessage {
        delivery_mode: mode,
        thread: thread.clone(),
        body: key.into(),
        direct_recipient: Some(SeatId::new("s2")),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new(key),
        claim: claim("s1"),
        relays_user: false,
        user_intent: None,
    };
    while let crate::ports::SendPreparationProgress::More { .. } = store
        .prepare_send_step(
            &v,
            crate::ports::DurableWorkAdmission::new(16).unwrap(),
            &budget(),
        )
        .unwrap()
    {}
    let hash = schema::canonical_digest(&crate::store::messages::send_payload(&v)).unwrap();
    store
        .mutate(
            PermitMutation::SendMessage(v),
            permit("s1", key, ObligationRef::Control(thread.clone()), hash, 100),
            &budget(),
        )
        .unwrap();
}

/// Q4: active pair acceptance seeds a missing reader at the invitation event floor.
/// Both ordinary and Lazy opening bodies remain unread, with no seat-wide legacy marker.
#[test]
fn accepting_a_pair_invitation_keeps_the_opening_messages_unread() {
    for mode in [DeliveryMode::Lazy, DeliveryMode::Ordinary] {
        let (store, db, path) = store_fixture();
        let pair = thread(ensure(&store, "s1", "s2", "ensure").unwrap());
        send_direct(&store, &pair, mode, "open-1");
        send_direct(&store, &pair, mode, "open-2");
        let invitation: String = db
            .query_row("SELECT id FROM invitations WHERE seat_id='s2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let v = crate::protocol::commands::Accept {
            thread: pair.clone(),
            operation: OperationId::new("accept"),
            claim: claim("s2"),
        };
        let hash = cooperative_payload_hash("accept", &v).unwrap();
        store
            .mutate(
                PermitMutation::Accept(v),
                permit(
                    "s2",
                    "accept",
                    ObligationRef::Invitation(InvitationId::new(invitation.clone())),
                    hash,
                    100,
                ),
                &budget(),
            )
            .unwrap();
        let binding: String = db
            .query_row(
                "SELECT binding_id FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let binding = BindingId::parse(binding).unwrap();
        let event_floor: i64 = db
            .query_row(
                "SELECT sequence FROM messages WHERE event_key=?1",
                [format!("invite:{invitation}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            crate::store::read_cursors::cursor_for(&db, &binding, &pair).unwrap(),
            Some(u64::try_from(event_floor).unwrap()),
            "{mode:?}"
        );
        assert!(
            crate::store::read_cursors::has_unread(&db, &binding).unwrap(),
            "{mode:?}"
        );
        assert_eq!(
            count(&db, "lazy_recipients WHERE legacy_pending=1"),
            0,
            "{mode:?}"
        );
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn pair_refuses_third_self_join() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    let v = crate::protocol::commands::Join {
        thread: ThreadId::new("pair"),
        operation: OperationId::new("join-third"),
        claim: claim("s3"),
    };
    let hash = cooperative_payload_hash("join", &v).unwrap();
    assert!(
        store
            .mutate(
                PermitMutation::Join(v),
                permit(
                    "s3",
                    "join-third",
                    ObligationRef::Control(ThreadId::new("pair")),
                    hash,
                    100
                ),
                &budget()
            )
            .is_err()
    );
    assert_eq!(count(&db, "memberships"), 2);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn pair_refuses_service_third_invite_and_ownership() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut db, path, _) = bound_fixture(100);
    raw_pair(&db);
    let author = ServiceAuthorId::new(format!(
        "graph:{:x}",
        sha2::Sha256::digest(b"34750f98bd59fcfc946da45aaabe933b")
    ));
    let gate = GraphGate(author.clone());
    let connection = gate
        .register("34750f98bd59fcfc946da45aaabe933b", "boot", author)
        .unwrap();
    let op = ServiceOperation::Invite(ServiceInvite {
        thread: ThreadId::new("pair"),
        seat: SeatId::new("s3"),
        constraint: InvitationConstraint::Ordinary,
        deadline_millis: None,
        operation: OperationId::new("service-third"),
    });
    assert!(
        crate::store::service_controls::operate(
            &context,
            &mut db,
            "34750f98bd59fcfc946da45aaabe933b",
            op,
            &connection,
            &gate,
            &budget(),
            None
        )
        .is_err()
    );
    assert_eq!(count(&db, "memberships"), 2);
    let op = ServiceOperation::EnsureThread(EnsureManagedThread {
        thread: ThreadId::new("pair"),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("own"),
    });
    assert!(
        crate::store::service_controls::operate(
            &context,
            &mut db,
            "34750f98bd59fcfc946da45aaabe933b",
            op,
            &connection,
            &gate,
            &budget(),
            None
        )
        .is_err()
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn pair_corrupt_membership_refuses_raw_send_preparation() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES('pair','s3','joined','joined')",[]).unwrap();
    let v = SendMessage {
        delivery_mode: DeliveryMode::Lazy,
        thread: ThreadId::new("pair"),
        body: "quiet".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("corrupt"),
        claim: claim("s1"),
        relays_user: false,
        user_intent: None,
    };
    assert!(
        store
            .prepare_send_step(
                &v,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget()
            )
            .is_err()
    );
    assert_eq!(count(&db, "send_preparations"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

fn send_request_to(
    t: &ThreadId,
    from: &str,
    to: &str,
    key: &str,
    mode: DeliveryMode,
) -> SendMessage {
    SendMessage {
        delivery_mode: mode,
        thread: t.clone(),
        body: "body".into(),
        direct_recipient: Some(SeatId::new(to)),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new(key),
        claim: claim(from),
        relays_user: false,
        user_intent: None,
    }
}
fn prepare(store: &SqliteStore, v: &SendMessage) -> Result<(), ApiError> {
    loop {
        match store.prepare_send_step(
            v,
            crate::ports::DurableWorkAdmission::new(16).unwrap(),
            &budget(),
        )? {
            crate::ports::SendPreparationProgress::Ready { .. } => return Ok(()),
            crate::ports::SendPreparationProgress::More { .. } => (),
            _ => panic!("unexpected committed prep"),
        }
    }
}
fn publish(store: &SqliteStore, v: SendMessage) -> Result<CommandResult, ApiError> {
    let hash = schema::canonical_digest(&crate::store::messages::send_payload(&v)).unwrap();
    let p = permit(
        v.claim.seat.as_str(),
        v.operation.as_str(),
        ObligationRef::Control(v.thread.clone()),
        hash,
        100,
    );
    store.mutate(PermitMutation::SendMessage(v), p, &budget())
}
#[test]
fn direct_prepare_revalidates_original_caller_binding() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    db.execute("UPDATE seats SET generation=generation+1 WHERE id='s1'", [])
        .unwrap();
    assert!(
        prepare(
            &store,
            &send_request_to(
                &ThreadId::new("pair"),
                "s1",
                "s2",
                "stale",
                DeliveryMode::Lazy
            )
        )
        .is_err()
    );
    assert_eq!(count(&db, "send_preparations"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn direct_prepare_lifecycle_guidance_names_existing_pair() {
    for (sql, from, to, action) in [
        (
            "UPDATE memberships SET state='invited',voluntary_state='invited' WHERE seat_id='s1'",
            "s1",
            "s2",
            "accept",
        ),
        (
            "UPDATE memberships SET state='left',voluntary_state='left' WHERE seat_id='s1'",
            "s1",
            "s2",
            "join",
        ),
        ("UPDATE threads SET archived=1", "s1", "s2", "reopen"),
    ] {
        let (store, db, path) = store_fixture();
        raw_pair(&db);
        db.execute(sql, []).unwrap();
        let err = prepare(
            &store,
            &send_request_to(
                &ThreadId::new("pair"),
                from,
                to,
                "state",
                DeliveryMode::Lazy,
            ),
        )
        .unwrap_err();
        assert!(err.detail.contains(action), "{err:?}");
        assert!(err.detail.contains("pair"), "{err:?}");
        assert_eq!(count(&db, "send_preparations"), 0);
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
#[test]
fn corrupt_lazy_prepared_audience_refuses_publication() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    let v = send_request_to(
        &ThreadId::new("pair"),
        "s1",
        "s2",
        "corrupt-audience",
        DeliveryMode::Lazy,
    );
    prepare(&store, &v).unwrap();
    db.execute_batch("DROP TRIGGER lazy_recipient_identity_immutable;")
        .unwrap();
    db.execute("UPDATE lazy_recipients SET seat_id='s3'", [])
        .unwrap();
    assert!(publish(&store, v).is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ensure_invalid_targets_and_claims_have_no_effects() {
    for (target, sql) in [
        ("s1", ""),
        ("missing", ""),
        (
            "s2",
            "UPDATE seats SET state='retired',retired_at=1 WHERE id='s2'",
        ),
        ("s2", "UPDATE seats SET state='unresolved' WHERE id='s2'"),
        (
            "s2",
            "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('34750f98bd59fcfc946da45aaabe933b','s2','b',1,'held')",
        ),
        (
            "s2",
            "PRAGMA foreign_keys=OFF; UPDATE seats SET instance_id='foreign' WHERE id='s2'",
        ),
        (
            "s2",
            "UPDATE observed_targets SET generation=2 WHERE target_id='s2'",
        ),
        (
            "s2",
            "UPDATE seats SET generation=generation+1 WHERE id='s1'",
        ),
    ] {
        let (store, db, path) = store_fixture();
        db.execute_batch(sql).unwrap();
        let expected = if target == "s1" {
            ErrorCode::InvalidRequest
        } else if sql.contains("generation=generation+1") {
            // Session-keyed bindings: the execution is no part of the binding key; the caller's
            // binding changes here with the seat's binding generation (a rebind).
            ErrorCode::CallerUnverified
        } else {
            ErrorCode::TargetUnresolved
        };
        assert_eq!(
            ensure(&store, "s1", target, "bad").unwrap_err().code,
            expected,
            "{target} {sql}"
        );
        for table in [
            "threads",
            "direct_threads",
            "invitations",
            "messages",
            "operations",
            "wake_work",
        ] {
            assert_eq!(count(&db, table), 0, "{table} for {sql}");
        }
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
#[test]
fn resolved_unbound_and_human_ordinary_direct_smoke() {
    for human in [false, true] {
        let (context, db, path, _) = bound_fixture_recipient_harness(
            100,
            if human {
                Harness::Human
            } else {
                Harness::Codex
            },
        );
        if !human {
            db.execute("DELETE FROM occupant_bindings WHERE seat_id='s2'", [])
                .unwrap();
        }
        let binding_count = count(&db, "occupant_bindings");
        let store = SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap();
        let t = thread(ensure(&store, "s1", "s2", "smoke").unwrap());
        let v = send_request_to(&t, "s1", "s2", "ordinary", DeliveryMode::Ordinary);
        prepare(&store, &v).unwrap();
        assert_eq!(count(&db, "prepared_recipients"), i64::from(!human));
        if !human {
            assert_eq!(
                db.query_row(
                    "SELECT seat_id,ack_required FROM prepared_recipients",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                )
                .unwrap(),
                ("s2".into(), 1)
            );
        }
        publish(&store, v).unwrap();
        assert_eq!(count(&db, "occupant_bindings"), binding_count);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM invitations WHERE state='accepted'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(thread(ensure(&store, "s1", "s2", "again").unwrap()), t);
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
fn accept_pair(store: &SqliteStore, db: &Connection, t: &ThreadId, seat: &str, key: &str) {
    let invitation:String=db.query_row("SELECT id FROM invitations WHERE thread_id=?1 AND seat_id=?2 ORDER BY episode DESC LIMIT 1",rusqlite::params![t.as_str(),seat],|r|r.get(0)).unwrap();
    let v = Accept {
        thread: t.clone(),
        operation: OperationId::new(key),
        claim: claim(seat),
    };
    let hash = cooperative_payload_hash("accept", &v).unwrap();
    store
        .mutate(
            PermitMutation::Accept(v),
            permit(
                seat,
                key,
                ObligationRef::Invitation(InvitationId::new(invitation)),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
}
#[test]
fn reversed_concurrent_ensures_converge_then_explicit_accept_reuses() {
    let (store, db, path) = store_fixture();
    let second = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(FixedClock(AtomicI64::new(100)))).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let first = std::thread::spawn(move || {
        other.wait();
        let r = ensure(&store, "s1", "s2", "race-a");
        (store, r)
    });
    barrier.wait();
    let reverse = ensure(&second, "s2", "s1", "race-b");
    let (store, forward) = first.join().unwrap();
    assert_eq!(
        usize::from(forward.is_ok()) + usize::from(reverse.is_ok()),
        1
    );
    let (t, invitee) = match (forward, reverse) {
        (Ok(r), Err(e)) => {
            assert!(e.detail.contains("accept"));
            (thread(r), "s2")
        }
        (Err(e), Ok(r)) => {
            assert!(e.detail.contains("accept"));
            (thread(r), "s1")
        }
        _ => panic!("not exactly one pair"),
    };
    assert_eq!(count(&db, "direct_threads"), 1);
    assert_eq!(count(&db, "invitations"), 1);
    accept_pair(&store, &db, &t, invitee, "accept-race");
    assert_eq!(thread(ensure(&store, "s1", "s2", "fresh-a").unwrap()), t);
    assert_eq!(thread(ensure(&second, "s2", "s1", "fresh-b").unwrap()), t);
    let v = send_request_to(&t, "s1", "s2", "dedup", DeliveryMode::Ordinary);
    prepare(&store, &v).unwrap();
    assert_eq!(count(&db, "prepared_recipients"), 1);
    let query:String=db.query_row("EXPLAIN QUERY PLAN SELECT thread_id FROM direct_threads WHERE instance_id='34750f98bd59fcfc946da45aaabe933b' AND seat_a='s1' AND seat_b='s2'",[],|r|r.get(3)).unwrap();
    assert!(query.contains("SEARCH"), "{query}");
    let plan:Vec<String>=db.prepare("EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM membership_intervals WHERE thread_id=?1 AND seat_id<?2) OR EXISTS(SELECT 1 FROM membership_intervals WHERE thread_id=?1 AND seat_id>?2 AND seat_id<?3) OR EXISTS(SELECT 1 FROM membership_intervals WHERE thread_id=?1 AND seat_id>?3)").unwrap().query_map(rusqlite::params![t.as_str(),"s1","s2"],|r|r.get(3)).unwrap().collect::<Result<_,_>>().unwrap();
    // Each range is a covering index search bounded by (thread_id, seat_id),
    // never a scan of the thread's intervals. Both membership_intervals_snapshot
    // (thread_id,seat_id,joined_seq) and migration 46's
    // presentation_membership_history (thread_id,seat_id,ordinal) serve it.
    assert_eq!(
        plan.iter()
            .filter(|s| {
                s.starts_with(
                    "SEARCH membership_intervals USING COVERING INDEX membership_intervals_snapshot (thread_id=? AND seat_id",
                ) || s.starts_with(
                    "SEARCH membership_intervals USING COVERING INDEX presentation_membership_history (thread_id=? AND seat_id",
                )
            })
            .count(),
        3,
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|s| s.starts_with("SCAN membership_intervals")),
        "{plan:?}"
    );

    crate::store::retention::prune_once(&store, &budget()).unwrap();
    assert_eq!(count(&db, "direct_threads"), 1);
    assert_eq!(
        thread(ensure(&store, "s1", "s2", "after-prune").unwrap()),
        t
    );
    drop(store);
    drop(second);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn direct_lifecycle_rejection_and_departure_require_explicit_repair() {
    let (store, db, path) = store_fixture();
    let t = thread(ensure(&store, "s1", "s2", "initial").unwrap());
    let invitation: String = db
        .query_row("SELECT id FROM invitations", [], |r| r.get(0))
        .unwrap();
    let v = crate::protocol::commands::Reject {
        thread: t.clone(),
        invitation: InvitationId::new(invitation),
        reason: "declined".into(),
        operation: OperationId::new("reject"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("reject", &v).unwrap();
    let obligation = ObligationRef::Invitation(v.invitation.clone());
    store
        .mutate(
            PermitMutation::Reject(v),
            permit("s2", "reject", obligation, hash, 100),
            &budget(),
        )
        .unwrap();
    assert!(
        ensure(&store, "s1", "s2", "after-reject")
            .unwrap_err()
            .detail
            .contains("reinvite")
    );
    assert!(
        prepare(
            &store,
            &send_request_to(&t, "s1", "s2", "after-reject-send", DeliveryMode::Lazy)
        )
        .unwrap_err()
        .detail
        .contains("reinvite")
    );
    assert_eq!(count(&db, "invitations"), 1);
    let invite = Invite {
        confirmation: None,
        thread: t.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: None,
        operation: OperationId::new("reinvite"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &invite).unwrap();
    store
        .mutate(
            PermitMutation::Invite(invite),
            permit(
                "s1",
                "reinvite",
                ObligationRef::Control(t.clone()),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    assert_eq!(thread(ensure(&store, "s1", "s2", "repaired").unwrap()), t);
    accept_pair(&store, &db, &t, "s2", "accept");
    let v = send_request_to(&t, "s1", "s2", "staged", DeliveryMode::Lazy);
    prepare(&store, &v).unwrap();
    let leave = Leave {
        thread: t.clone(),
        operation: OperationId::new("leave"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("leave", &leave).unwrap();
    store
        .mutate(
            PermitMutation::Leave(leave),
            permit("s2", "leave", ObligationRef::Control(t.clone()), hash, 100),
            &budget(),
        )
        .unwrap();
    assert!(publish(&store, v).unwrap_err().detail.contains("reinvite"));
    assert_eq!(count(&db, "direct_threads"), 1);
    assert_eq!(count(&db, "invitations"), 2);
    assert_eq!(thread(ensure(&store, "s1", "s2", "initial").unwrap()), t);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn bootstrap_adapter_admits_unlinked_direct_and_revalidates_namespace() {
    use crate::ports::BootstrapStorePort;
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    let v = send_request_to(
        &ThreadId::new("pair"),
        "s1",
        "s2",
        "bootstrap-ordinary",
        DeliveryMode::Lazy,
    );
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: "/tmp/direct".into(),
        host_endpoint: "/tmp/endpoint".into(),
    };
    let mut bad = namespace.clone();
    bad.instance = "foreign".into();
    assert_eq!(
        store
            .bootstrap_prepare_send_step(
                &bad,
                &v,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::InstanceMismatch
    );
    loop {
        match store
            .bootstrap_prepare_send_step(
                &namespace,
                &v,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget(),
            )
            .unwrap()
        {
            crate::ports::SendPreparationProgress::Ready { .. } => break,
            crate::ports::SendPreparationProgress::More { .. } => (),
            _ => panic!("unexpected replay"),
        }
    }
    assert_eq!(count(&db, "lazy_recipients"), 1);
    publish(&store, v).unwrap();
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn pair_operator_orphan_recovery_cannot_enroll_third_seat() {
    let (context, mut db, path, _) = bound_fixture(100);
    raw_pair(&db);
    db.execute(
        "UPDATE memberships SET state='left',voluntary_state='left'",
        [],
    )
    .unwrap();
    let v = OperatorOrphanInvite {
        thread: ThreadId::new("pair"),
        seat: SeatId::new("s3"),
        deadline_millis: None,
        operation: OperationId::new("operator-third"),
    };
    let actor = OperatorActor::from_peer(
        crate::protocol::authority::PeerIdentity::from_kernel(501),
        501,
    )
    .unwrap();
    assert!(
        operator_orphan_invite(
            &context,
            &mut db,
            "34750f98bd59fcfc946da45aaabe933b",
            &v,
            actor,
            None
        )
        .is_err()
    );
    assert_eq!(count(&db, "invitations"), 0);
    assert_eq!(count(&db, "memberships"), 2);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn raw_pair_audience_and_marker_cannot_address_third_or_sender() {
    let (store, db, path) = store_fixture();
    raw_pair(&db);
    for (marker, acks) in [
        (Some("s3"), vec![]),
        (Some("s1"), vec![]),
        (Some("s2"), vec!["s1"]),
        (Some("s2"), vec!["s3"]),
        (None, vec!["s3"]),
    ] {
        let mut v = send_request_to(
            &ThreadId::new("pair"),
            "s1",
            "s2",
            "bad-audience",
            DeliveryMode::Ordinary,
        );
        v.direct_recipient = marker.map(SeatId::new);
        v.invited_recipients = acks.into_iter().map(SeatId::new).collect();
        assert!(prepare(&store, &v).is_err());
        assert_eq!(count(&db, "send_preparations"), 0);
    }
    let mut lazy = send_request_to(
        &ThreadId::new("pair"),
        "s1",
        "s2",
        "lazy-ack",
        DeliveryMode::Lazy,
    );
    lazy.invited_recipients = vec![SeatId::new("s2")];
    assert!(prepare(&store, &lazy).is_err());
    assert_eq!(count(&db, "send_preparations"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn raw_service_send_cannot_own_or_address_automatic_pair() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut db, path, _) = bound_fixture(100);
    raw_pair(&db);
    let author = ServiceAuthorId::new(format!(
        "graph:{:x}",
        sha2::Sha256::digest(b"34750f98bd59fcfc946da45aaabe933b")
    ));
    let gate = GraphGate(author.clone());
    let connection = gate
        .register("34750f98bd59fcfc946da45aaabe933b", "boot", author)
        .unwrap();
    let v = ServiceSend {
        thread: ThreadId::new("pair"),
        body: "service".into(),
        recipients: vec![SeatId::new("s3")],
        deadline_millis: None,
        operation: OperationId::new("service-send"),
    };
    assert!(
        crate::store::service_send::prepare_service_send_step(
            &context,
            &mut db,
            &connection,
            &gate,
            &v,
            crate::store::messages::MessageLimits {
                body_bytes: 4096,
                receipt_duration_ms: 300000
            },
            &budget(),
            16
        )
        .is_err()
    );
    assert_eq!(count(&db, "send_preparations"), 0);
    drop(db);
    let _ = std::fs::remove_file(path);
}
#[test]
fn exact_ensure_replay_survives_binding_change_but_fresh_send_refuses() {
    let (store, db, path) = store_fixture();
    let t = thread(ensure(&store, "s1", "s2", "historical").unwrap());
    let rows = count(&db, "messages");
    db.execute("UPDATE seats SET generation=generation+1 WHERE id='s1'", [])
        .unwrap();
    assert_eq!(thread(ensure(&store, "s1", "s2", "historical").unwrap()), t);
    assert_eq!(count(&db, "messages"), rows);
    assert!(ensure(&store, "s1", "s2", "fresh-after-binding").is_err());
    assert!(
        prepare(
            &store,
            &send_request_to(&t, "s1", "s2", "fresh-send", DeliveryMode::Lazy)
        )
        .is_err()
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn direct_publication_rechecks_archive_target_retirement_unresolve_and_hold() {
    for (sql, expected) in [
        ("UPDATE threads SET archived=1", ErrorCode::Archived),
        (
            "UPDATE seats SET state='retired',retired_at=1 WHERE id='s2'",
            ErrorCode::TargetUnresolved,
        ),
        (
            "UPDATE seats SET state='unresolved' WHERE id='s2'",
            ErrorCode::TargetUnresolved,
        ),
        (
            "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('34750f98bd59fcfc946da45aaabe933b','s2','b',1,'held')",
            ErrorCode::TargetUnresolved,
        ),
    ] {
        let (store, db, path) = store_fixture();
        let t = thread(ensure(&store, "s1", "s2", "guard").unwrap());
        let v = send_request_to(&t, "s1", "s2", "guard-send", DeliveryMode::Lazy);
        prepare(&store, &v).unwrap();
        db.execute_batch(sql).unwrap();
        assert_eq!(publish(&store, v).unwrap_err().code, expected, "{sql}");
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM messages WHERE kind='ordinary'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(count(&db, "direct_threads"), 1);
        assert_eq!(count(&db, "invitations"), 1);
        assert_eq!(thread(ensure(&store, "s1", "s2", "guard").unwrap()), t);
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn manual_two_member_channel_is_not_adopted_and_normal_send_is_unchanged() {
    let (store, db, path) = store_fixture();
    db.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('manual','34750f98bd59fcfc946da45aaabe933b','manual','manual',0,0); INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES('manual','s1','joined','joined',0),('manual','s2','joined','joined',0); INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('manual','s1',1,1),('manual','s2',1,1);").unwrap();
    let t = thread(ensure(&store, "s1", "s2", "distinct").unwrap());
    assert_ne!(t, ThreadId::new("manual"));
    assert_eq!(count(&db, "threads"), 2);
    assert_eq!(count(&db, "direct_threads"), 1);
    let wake_before: i64 = db
        .query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut v = send_request_to(
        &ThreadId::new("manual"),
        "s1",
        "s2",
        "manual-send",
        DeliveryMode::Lazy,
    );
    v.direct_recipient = None;
    prepare(&store, &v).unwrap();
    publish(&store, v).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE thread_id='manual' AND seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        wake_before
    );
    assert_eq!(count(&db, "invitations"), 1);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[path = "direct_lazy_readers.rs"]
mod reader_controls;

#[path = "direct_lazy_writer_floors.rs"]
mod writer_floor_controls;
/// ht-8y9: a join never marks seat-pending a lazy row some completion already displayed. The
/// binding displays `shown` while its cursor is held behind `held` (an earlier row no completion
/// covered, as a truncated delivery leaves it), so `shown` is past the seat's furthest cursor at
/// the rejoin; only `held` becomes seat-pending.
#[test]
// This retained upstream key exercises plain generic Join, which owns the head policy.
fn rejoin_never_reshows_a_lazy_row_displayed_out_of_order() {
    let (store, db, path) = store_fixture();
    let pair = thread(ensure(&store, "s1", "s2", "ensure").unwrap());
    accept_pair(&store, &db, &pair, "s2", "accept");
    send_direct(&store, &pair, DeliveryMode::Lazy, "held");
    send_direct(&store, &pair, DeliveryMode::Lazy, "shown");
    let message = |body: &str| -> (MessageId, i64) {
        db.query_row(
            "SELECT id,sequence FROM messages WHERE body=?1",
            [body],
            |r| Ok((MessageId::new(r.get::<_, String>(0)?), r.get(1)?)),
        )
        .unwrap()
    };
    let ((held, held_seq), (shown, _)) = (message("held"), message("shown"));
    let binding: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let binding = BindingId::parse(binding).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::store::lazy_delivery::complete_for_binding(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("s2"),
        Some(&binding),
        std::slice::from_ref(&shown),
    )
    .unwrap();
    tx.commit().unwrap();
    let cursor = crate::store::read_cursors::cursor_for(&db, &binding, &pair)
        .unwrap()
        .unwrap();
    assert!(
        cursor < u64::try_from(held_seq).unwrap(),
        "cursor held behind `held`"
    );
    let leave = Leave {
        thread: pair.clone(),
        operation: OperationId::new("leave"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("leave", &leave).unwrap();
    store
        .mutate(
            PermitMutation::Leave(leave),
            permit(
                "s2",
                "leave",
                ObligationRef::Control(pair.clone()),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    // A plain Join deliberately retains main's generic head/seat-pending policy.
    // No active invitation exists, so this is not Q4 direct acceptance.
    let join = crate::protocol::commands::Join {
        thread: pair.clone(),
        operation: OperationId::new("generic-rejoin"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("join", &join).unwrap();
    store
        .mutate(
            PermitMutation::Join(join),
            permit(
                "s2",
                "generic-rejoin",
                ObligationRef::Control(pair.clone()),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    let pending = |id: &MessageId| -> i64 {
        db.query_row(
            "SELECT legacy_pending FROM lazy_recipients WHERE message_id=?1 AND seat_id='s2'",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        pending(&held),
        1,
        "the undisplayed row stays pending for the seat"
    );
    assert_eq!(pending(&shown), 0, "the displayed row is not shown again");
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
