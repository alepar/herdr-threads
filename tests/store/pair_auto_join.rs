//! 1:1 pairs auto-join both seats, and every new invitation is confirming (ht-yha.6, D6/D7).
//! A child module of `direct_threads`: it reuses that module's fixtures.
use super::*;
use crate::protocol::{
    commands::{Accept, InvitationConfirmation},
    priority::{DeliveryPriority, NudgeThreshold},
    results::{DirectThreadEnsureOutcome, DirectThreadEnsured},
};

fn ensure_with(
    store: &SqliteStore,
    from: &str,
    to: &str,
    key: &str,
    policy: Option<crate::protocol::commands::EnsureCreationPolicy>,
) -> DirectThreadEnsured {
    // A modern request (any creation policy) gets the bounded `DirectThreadEnsured` result.
    let policy = policy.or(Some(crate::protocol::commands::EnsureCreationPolicy {
        confirmation: InvitationConfirmation::RequireAck,
        deadline_millis: None,
    }));
    let v = EnsureDirectThread {
        creation_policy: policy,
        recipient: SeatId::new(to),
        operation: OperationId::new(key),
        claim: claim(from),
    };
    let hash = cooperative_payload_hash("ensure_direct_thread", &v).unwrap();
    match store
        .mutate(
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
        .unwrap()
    {
        CommandResult::DirectThreadEnsured(v) => v,
        other => panic!("unexpected ensure result {other:?}"),
    }
}

fn send_at(store: &SqliteStore, thread: &ThreadId, priority: u8, key: &str) {
    let v = SendMessage {
        priority: Some(DeliveryPriority::new(priority).unwrap()),
        delivery_mode: DeliveryMode::Ordinary,
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

fn open_binding(db: &Connection, seat: &str) -> BindingId {
    BindingId::parse(
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            [seat],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn direct_send_creates_pair_with_both_seats_joined_and_no_invitation() {
    let (store, db, path) = auto_join_fixture();
    let first = ensure_with(&store, "s1", "s2", "first", None);
    assert_eq!(first.outcome, DirectThreadEnsureOutcome::AlreadyJoined);
    assert_eq!(first.invitation, None);
    let pair = first.thread.clone();
    assert_eq!(count(&db, "invitations"), 0);
    assert_eq!(count(&db, "membership_intervals"), 2);
    for seat in ["s1", "s2"] {
        let (state, voluntary): (String, String) = db
            .query_row(
                "SELECT state,voluntary_state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                params![pair.as_str(), seat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((state.as_str(), voluntary.as_str()), ("joined", "joined"));
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE json_extract(event_json,'$.action')='join' AND json_extract(event_json,'$.seat')='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "the peer has its own join event"
    );
    let reserved: (String, String) = db
        .query_row("SELECT seat_a,seat_b FROM direct_threads", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(reserved, ("s1".into(), "s2".into()));
    assert_eq!(
        count(&db, "wake_work"),
        0,
        "joining a pair is not a wake source"
    );
    // The peer is a member at once: it may reply without accepting anything.
    let again = ensure_with(&store, "s2", "s1", "reverse", None);
    assert_eq!(again.outcome, DirectThreadEnsureOutcome::AlreadyJoined);
    assert_eq!(again.invitation, None);
    assert_eq!(again.thread, pair);
    assert_eq!(count(&db, "invitations"), 0);
    assert_eq!(count(&db, "membership_intervals"), 2);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn creation_policy_is_ignored() {
    for confirmation in [
        InvitationConfirmation::Lazy,
        InvitationConfirmation::RequireAck,
    ] {
        let (store, db, path) = auto_join_fixture();
        let policy = crate::protocol::commands::EnsureCreationPolicy {
            confirmation,
            deadline_millis: (confirmation == InvitationConfirmation::RequireAck).then_some(5_000),
        };
        let ensured = ensure_with(&store, "s1", "s2", "policy", Some(policy));
        assert_eq!(
            ensured.outcome,
            DirectThreadEnsureOutcome::AlreadyJoined,
            "{confirmation:?}"
        );
        assert_eq!(ensured.invitation, None, "{confirmation:?}");
        assert_eq!(count(&db, "invitations"), 0, "{confirmation:?}");
        assert_eq!(count(&db, "membership_intervals"), 2, "{confirmation:?}");
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn first_direct_message_is_a_row_at_its_priority() {
    // p1 does not wake a T=0 peer; p0 does. Each opener is a delivery row at its own priority.
    for (priority, wakes) in [(1u8, false), (0u8, true)] {
        let (store, db, path) = auto_join_fixture();
        let pair = ensure_with(&store, "s1", "s2", "ensure", None).thread;
        send_at(&store, &pair, priority, "opener");
        let stored: i64 = db
            .query_row(
                "SELECT priority FROM lazy_recipients WHERE seat_id='s2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, i64::from(priority));
        assert_eq!(count(&db, "invitations"), 0);
        assert_eq!(
            crate::store::lazy_delivery::wake_frontier(&db, "s2", NudgeThreshold::default())
                .unwrap()
                .is_some(),
            wakes,
            "p{priority} at T=0"
        );
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn peer_offline_then_session_starts_is_offered_opener_and_later_sends() {
    let (store, db, path) = auto_join_fixture();
    // The peer has no open binding when the pair is created and written to.
    db.execute(
        "UPDATE occupant_bindings SET ended_at=50 WHERE seat_id='s2'",
        [],
    )
    .unwrap();
    let pair = ensure_with(&store, "s1", "s2", "ensure", None).thread;
    send_at(&store, &pair, 1, "opener");
    send_at(&store, &pair, 1, "later");
    // The peer's first session starts now: a fresh binding, no cursor anywhere.
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s2',1,'s2','b',1,1,'codex','n2','00000000-0000-4000-8000-000000000002','cooperative_top_level',0,'term-s2','inc')", [])
        .unwrap();
    let binding = open_binding(&db, "s2");
    crate::store::read_cursors::seed(&db, &binding, crate::store::read_cursors::SeedMode::Fresh)
        .unwrap();
    let first_ordinary: i64 = db
        .query_row(
            "SELECT MIN(sequence) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let head = crate::store::read_cursors::thread_head(&db, &pair).unwrap();
    let cursor = crate::store::read_cursors::cursor_for(&db, &binding, &pair)
        .unwrap()
        .expect("the first session seeds a cursor");
    assert!(
        cursor < first_ordinary as u64 && cursor < head,
        "an auto-joined pair seeds at the head as of the join ({cursor}), not the current head ({head})"
    );
    let unread = crate::store::lazy_delivery::binding_lazy_unread(&db, &binding, 10).unwrap();
    assert_eq!(
        unread.len(),
        2,
        "the opener and the later send are both offered"
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

fn raw_thread(db: &Connection, id: &str) -> ThreadId {
    db.execute_batch(&format!("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('{id}','i','t','g',0,0); INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES('{id}','s1','joined','joined',0); INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('{id}','s1',1,1);")).unwrap();
    ThreadId::new(id)
}

fn invite_store(
    store: &SqliteStore,
    thread: &ThreadId,
    confirmation: Option<InvitationConfirmation>,
    deadline_millis: Option<u64>,
    key: &str,
) -> InvitationId {
    let v = Invite {
        confirmation,
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis,
        operation: OperationId::new(key),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &v).unwrap();
    match store
        .mutate(
            PermitMutation::Invite(v),
            permit("s1", key, ObligationRef::Control(thread.clone()), hash, 100),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::Invitation(id) => id,
        other => panic!("unexpected invite result {other:?}"),
    }
}

fn recorded(db: &Connection, id: &InvitationId) -> (Option<String>, i64, i64, i64) {
    db.query_row(
        "SELECT confirmation,frozen_duration_ms,deadline_at,created_at FROM invitations WHERE id=?1",
        [id.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

#[test]
fn every_new_invitation_is_confirming_with_deadline() {
    let (store, db, path) = auto_join_fixture();
    for (i, confirmation) in [
        None,
        Some(InvitationConfirmation::Lazy),
        Some(InvitationConfirmation::RequireAck),
    ]
    .into_iter()
    .enumerate()
    {
        let thread = raw_thread(&db, &format!("t{i}"));
        // A lazy request with a deadline used to be refused; it is now just a confirming invitation.
        let id = invite_store(
            &store,
            &thread,
            confirmation,
            (i == 1).then_some(7_000),
            &format!("inv{i}"),
        );
        let (stored, duration, deadline, created) = recorded(&db, &id);
        assert_eq!(stored.as_deref(), Some("require_ack"), "{confirmation:?}");
        assert!(duration > 0);
        assert_eq!(deadline, created + duration, "{confirmation:?}");
        if i == 1 {
            assert_eq!(duration, 7_000);
        } else {
            assert_eq!(duration, 300_000);
        }
        let payload: String = db
            .query_row(
                "SELECT event_json FROM messages WHERE event_key=?1",
                [format!("invite:{}", id.as_str())],
                |r| r.get(0),
            )
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload["confirmation"], "require_ack");
        assert_eq!(payload["deadline_at"], deadline);
    }
    assert_eq!(
        db.query_row(
            "SELECT reason_bits&1 FROM wake_work WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "a confirming invitation wakes its recipient"
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn overdue_invitation_warns_the_inviter() {
    let (context, mut db, path, clock) = bound_fixture(100);
    let scan_context = crate::store::connection::StoreContext::new(path.clone(), clock.clone());
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let thread = raw_thread(&db, "t");
    let id = invite_store(
        &store,
        &thread,
        Some(InvitationConfirmation::Lazy),
        None,
        "inv",
    );
    let (_, _, deadline, _) = recorded(&db, &id);
    clock
        .0
        .store(deadline + 1, std::sync::atomic::Ordering::SeqCst);
    let batch =
        crate::store::invitation_due::scan_invitation_due_batch(&scan_context, &mut db, None, 100)
            .unwrap();
    assert_eq!(batch.warnings_added, 1);
    let warning: Option<String> = db
        .query_row(
            "SELECT warning_message_id FROM invitations WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let warning = warning.expect("overdue warning recorded");
    assert!(crate::store::effective::is_warning_recipient(&db, &warning, "s1").unwrap());
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn old_client_lazy_invite_records_require_ack() {
    // The serialized shape an old client sends: `confirmation: "lazy"` on the wire.
    let command: crate::protocol::commands::Command = serde_json::from_value(serde_json::json!({
        "kind":"invite",
        "args":{"confirmation":"lazy","thread":"t","seat":"s2","deadline_millis":null,"operation":"old",
            "claim":{"instance":"i","seat":"s1","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-000000000001","target":"s1"}}
    }))
    .unwrap();
    let crate::protocol::commands::Command::Invite(old) = command else {
        panic!("not an invite")
    };
    assert_eq!(old.confirmation, Some(InvitationConfirmation::Lazy));
    let (store, db, path) = auto_join_fixture();
    let thread = raw_thread(&db, "t");
    let id = invite_store(
        &store,
        &thread,
        old.confirmation,
        old.deadline_millis,
        "old",
    );
    assert_eq!(recorded(&db, &id).0.as_deref(), Some("require_ack"));
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_orphan_invitation_stays_timed_with_effective_require_ack() {
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::OperatorOrphanInvite,
    };
    let (context, mut db, path, _) = fixture(100);
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,archived) VALUES ('orphan','i','topic','goal',0,0,1)", [])
        .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let command = OperatorOrphanInvite {
        thread: ThreadId::new("orphan"),
        seat: SeatId::new("s2"),
        deadline_millis: None,
        operation: OperationId::new("operator"),
    };
    let CommandResult::OperatorInvited(id) =
        operator_orphan_invite(&context, &mut db, "i", &command, actor, None).unwrap()
    else {
        panic!("not an operator invitation")
    };
    let (stored, duration, deadline, created) = recorded(&db, &id);
    // No inviter seat, so no stored policy: the absent policy is the effective RequireAck.
    assert_eq!(stored, None);
    assert_eq!(deadline, created + duration);
    assert_eq!(
        db.query_row(
            "SELECT reason_bits&1 FROM wake_work WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "an operator invitation wakes its recipient"
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn pre_upgrade_pending_pair_invitation_still_accepts() {
    // A pair as the pre-upgrade code created it (the fixture marker): caller joined, peer invited
    // with a pending lazy invitation, which keeps its stored behaviour: no wake, and `accept` joins.
    let (store, db, path) = store_fixture();
    let policy = crate::protocol::commands::EnsureCreationPolicy {
        confirmation: InvitationConfirmation::Lazy,
        deadline_millis: None,
    };
    let created = ensure_with(&store, "s1", "s2", "ensure", Some(policy));
    assert_eq!(created.outcome, DirectThreadEnsureOutcome::Created);
    let invitation = created
        .invitation
        .expect("pre-upgrade pair has an invitation");
    assert_eq!(invitation.confirmation, InvitationConfirmation::Lazy);
    assert_eq!(count(&db, "wake_work"), 0);
    let again = ensure_with(&store, "s1", "s2", "again", None);
    assert_eq!(again.outcome, DirectThreadEnsureOutcome::ExistingPending);
    assert_eq!(again.invitation.unwrap().id, invitation.id);
    let v = Accept {
        thread: created.thread.clone(),
        operation: OperationId::new("accept-old"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("accept", &v).unwrap();
    store
        .mutate(
            PermitMutation::Accept(v),
            permit(
                "s2",
                "accept-old",
                ObligationRef::Invitation(invitation.id.clone()),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s2'",
            [created.thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ack_send_on_an_auto_joined_pair_has_a_receipt_and_no_invitation() {
    // The automatic handoff route ensures the pair (no invitation) and sends its body with the
    // recipient as an explicit ACK recipient (priority 0, deadline).
    let (store, db, path) = auto_join_fixture();
    let pair = ensure_with(&store, "s1", "s2", "ensure", None).thread;
    let v = SendMessage {
        priority: None,
        delivery_mode: DeliveryMode::Ordinary,
        thread: pair.clone(),
        body: "task".into(),
        direct_recipient: Some(SeatId::new("s2")),
        invited_recipients: vec![SeatId::new("s2")],
        deadline_millis: Some(60_000),
        operation: OperationId::new("handoff-body"),
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
            permit(
                "s1",
                "handoff-body",
                ObligationRef::Control(pair.clone()),
                hash,
                100,
            ),
            &budget(),
        )
        .unwrap();
    assert_eq!(count(&db, "invitations"), 0);
    assert_eq!(count(&db, "prepared_recipients WHERE seat_id='s2'"), 1);
    assert_eq!(count(&db, "lazy_recipients"), 0);
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}
