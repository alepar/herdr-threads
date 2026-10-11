use super::*;
use crate::ports::{BootstrapAttachmentGuard, BootstrapPaneObservation};
use crate::{
    ports::{
        OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome, StorePort,
    },
    protocol::{
        commands::ResolveSeat,
        handoff::*,
        time::{CallBudget, Cancellation},
    },
    store::{handoff, topology_handoff},
};

type AttachmentFixture = (
    StoreContext,
    Connection,
    PathBuf,
    Arc<crate::store::SqliteStore>,
    crate::ports::HostObservation,
    CallBudget,
    BootstrapIdentity,
    BootstrapAttachment,
);
fn attachment_fixture() -> AttachmentFixture {
    attachment_fixture_with_recording(false)
}
fn attachment_fixture_with_recording(real_writer: bool) -> AttachmentFixture {
    attachment_fixture_inner(real_writer, true)
}
fn attachment_fixture_inner(real_writer: bool, retain_legacy: bool) -> AttachmentFixture {
    let (context, mut db, path, clock) = fixture(100);
    db.execute_batch("DELETE FROM seat_archival; DELETE FROM seats;")
        .unwrap();
    let created = crate::protocol::handoff::topology_contract_tests::created();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut snapshot = snapshot_for_test(1, &["w1:p1", "w1:p2"]);
    snapshot.boot = created.host_incarnation.clone();
    snapshot.incarnation = crate::ports::IncarnationEvidence::Verified {
        identity: created.host_incarnation.as_str().into(),
        evidence_kind: crate::ports::EvidenceKind::CoherentEnumeration,
    };
    for target in &mut snapshot.targets {
        target.host_boot = created.host_incarnation.clone();
        target.incarnation = crate::ports::IncarnationEvidence::Verified {
            identity: created.host_incarnation.as_str().into(),
            evidence_kind: crate::ports::EvidenceKind::CoherentEnumeration,
        };
        target.terminal = Some(if target.target.as_str() == "w1:p2" {
            created.terminal.clone()
        } else {
            TerminalId::new("sender-terminal")
        });
    }
    let admission =
        crate::store::seats::begin_host_observation(&context, &mut db, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut db, admission, &snapshot, &budget);
    crate::store::seats::publish_snapshot_stage(&context, &mut db, &stage, &budget).unwrap();
    let store = Arc::new(
        crate::store::SqliteStore::new(
            StoreContext::new(path.clone(), clock),
            "i",
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let mut observation = snapshot.targets.remove(1);
    observation.provenance = crate::ports::ObservationProvenance::FreshCurrentTarget;
    observation.incarnation = crate::ports::IncarnationEvidence::Verified {
        identity: "herdr-server:pid=42:start=1.000002:uid=501".into(),
        evidence_kind: crate::ports::EvidenceKind::NativeCurrentTarget,
    };
    let mut id = crate::protocol::handoff::topology_contract_tests::identity();
    id.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
    id.digest = id.semantic_digest().unwrap();
    let request = ResolveSeat {
        target: created.root_pane.clone(),
        operation: id.payload.resolve_key.clone(),
    };
    let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
    let guard = OrdinaryResolutionGuard::try_new(&request, current, &admission).unwrap();
    let OrdinaryResolutionOutcome::Resolved(recipient) = store
        .resolve_seat(request, OrdinaryResolutionAttempt::Observed(guard), &budget)
        .unwrap()
    else {
        panic!("missing recipient")
    };
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('sender','i','resolved','native','w1:p1',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('sender',1,'w1:p1',?1,1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0,'sender-terminal','herdr-server:pid=42:start=1.000002:uid=501')",[created.host_incarnation.as_str()]).unwrap();
    db.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('thread','i','topic','goal',0,0); INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('thread','sender','joined',0)").unwrap();
    let a = BootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created,
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: recipient.clone(),
        handoff: HandoffIdentity {
            compound: id.payload.handoff_key.clone(),
            digest: "b".repeat(64),
            claim: id.claim.clone(),
            thread: Some(ThreadId::new("thread")),
            recipient,
            create_key: id.payload.handoff.keys.create.clone(),
            invite_key: id.payload.handoff.keys.invite.clone(),
            send_key: id.payload.handoff.keys.send.clone(),
        },
    };
    if retain_legacy {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &id.payload.handoff.namespace, &id, UtcMillis(100))
            .unwrap();
        if real_writer {
            topology_handoff::attempts::reserve_attempt(
                &tx,
                &id.payload.handoff.namespace,
                &ReserveBootstrapAttempt {
                    identity: id.clone(),
                    expected_attempt: BootstrapAttempt::first(),
                    operation: BootstrapAttempt::first()
                        .operation(&id.compound, "reserve")
                        .unwrap(),
                },
            )
            .unwrap();
            topology_handoff::attempts::record_created(
                &tx,
                &id.payload.handoff.namespace,
                &RecordBootstrapCreated {
                    identity: id.clone(),
                    expected_attempt: BootstrapAttempt::first(),
                    operation: BootstrapAttempt::first()
                        .operation(&id.compound, "record")
                        .unwrap(),
                    evidence: a.created.clone(),
                },
            )
            .unwrap();
        } else {
            tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1 AND attempt=1",[serde_json::to_vec(&a.created).unwrap()]).unwrap();
            tx.execute(
                "UPDATE bootstrap_handoffs SET state='created' WHERE id=1",
                [],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    (context, db, path, store, observation, budget, id, a)
}
fn attachment_guard(
    store: &crate::store::SqliteStore,
    observation: &crate::ports::HostObservation,
    budget: &CallBudget,
    operation: &OperationId,
    sequence: u64,
) -> BootstrapAttachmentGuard {
    let (current, admission) = ordinary_resolution_read(store, observation, sequence, budget);
    let pane = BootstrapPaneObservation::try_new(
        current.clone(),
        HostTargetId::new("w1"),
        HostTargetId::new("w1:t2"),
        crate::protocol::handoff::topology_contract_tests::created().witness,
    )
    .unwrap();
    BootstrapAttachmentGuard::try_new(
        &ResolveSeat {
            target: current.target.clone(),
            operation: operation.clone(),
        },
        pane,
        &admission,
    )
    .unwrap()
}
#[test]
fn topology_attachment_uses_real_resolution_and_fresh_guard_without_allocating() {
    let (_context, mut db, path, store, observation, budget, id, a) = attachment_fixture();
    let (mut wrong_boot, admission) = ordinary_resolution_read(&store, &observation, 3, &budget);
    let mut changed_witness = crate::protocol::handoff::topology_contract_tests::created().witness;
    changed_witness.peer_pid = 43;
    wrong_boot.host_boot = HostBootId::new("herdr-server:pid=43:start=1.000002:uid=501");
    wrong_boot.incarnation = crate::ports::IncarnationEvidence::Verified {
        identity: wrong_boot.host_boot.as_str().into(),
        evidence_kind: crate::ports::EvidenceKind::NativeCurrentTarget,
    };
    let pane = BootstrapPaneObservation::try_new(
        wrong_boot,
        HostTargetId::new("w1"),
        HostTargetId::new("w1:t2"),
        changed_witness,
    )
    .unwrap();
    let wrong_boot_guard = BootstrapAttachmentGuard::try_new(
        &ResolveSeat {
            target: observation.target.clone(),
            operation: a.resolve_operation.clone(),
        },
        pane,
        &admission,
    )
    .unwrap();
    let guard = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 4);
    let command = AttachBootstrapHandoff {
        identity: id.clone(),
        operation: id.payload.attach_key.clone(),
        attachment: a.clone(),
    };
    let before: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(
        topology_handoff::attach_pending(
            &tx,
            &id.payload.handoff.namespace,
            &command,
            &wrong_boot_guard
        )
        .is_err(),
        "another server boot cannot attach"
    );
    let result =
        topology_handoff::attach_pending(&tx, &id.payload.handoff.namespace, &command, &guard)
            .unwrap();
    assert_eq!(result.state, BootstrapState::Attached);
    assert_eq!(result.attachment, Some(a.clone()));
    assert_eq!(
        tx.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert!(
        handoff::begin_pending(&tx, &a.handoff, UtcMillis(101)).is_err(),
        "bare Begin cannot infer namespace from parent"
    );
    assert_eq!(
        handoff::begin_linked_pending(
            &tx,
            &id.payload.handoff.namespace,
            &a.handoff,
            UtcMillis(101)
        )
        .unwrap()
        .state,
        HandoffState::Live
    );
    assert_eq!(
        topology_handoff::attach_pending(&tx, &id.payload.handoff.namespace, &command, &guard)
            .unwrap(),
        result
    );
    let mut changed = command.clone();
    changed.attachment.resolved_seat = SeatId::new("wrong");
    changed.attachment.handoff.recipient = SeatId::new("wrong");
    assert!(
        topology_handoff::attach_pending(&tx, &id.payload.handoff.namespace, &changed, &guard)
            .is_err()
    );
    tx.commit().unwrap();
    drop(store);
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn topology_attachment_refuses_stale_or_held_evidence_and_missing_or_changed_resolution() {
    for corruption in [
        "hold",
        "stale",
        "missing",
        "digest",
        "result",
        "terminal",
        "current_owner",
        "tab",
        "namespace",
    ] {
        let (_context, mut db, path, store, observation, budget, id, a) = attachment_fixture();
        let mut guard = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 3);
        match corruption {
            "hold" => {
                db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','w1:p2',?1,1,'held')",[a.created.host_incarnation.as_str()]).unwrap();
            }
            "stale" => {
                let _ = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 4);
            }
            "missing" => {
                db.execute(
                    "DELETE FROM operations WHERE actor_scope='service-allocation:i'",
                    [],
                )
                .unwrap();
            }
            "digest" => {
                db.execute("UPDATE operations SET digest=zeroblob(32) WHERE actor_scope='service-allocation:i'",[]).unwrap();
            }
            "result" => {
                db.execute(
                    "UPDATE operations SET result_json=?1 WHERE actor_scope='service-allocation:i'",
                    [
                        serde_json::to_string(&CommandResult::SeatResolved(SeatId::new("wrong")))
                            .unwrap(),
                    ],
                )
                .unwrap();
            }
            "terminal" => {
                let mut changed = observation.clone();
                changed.terminal = Some(TerminalId::new("changed"));
                guard = attachment_guard(&store, &changed, &budget, &a.resolve_operation, 4);
            }
            "current_owner" => {
                db.execute(
                    "UPDATE seats SET target_id='w1:p9' WHERE id=?1",
                    [a.resolved_seat.as_str()],
                )
                .unwrap();
            }
            "tab" => {
                let (current, admission) =
                    ordinary_resolution_read(&store, &observation, 4, &budget);
                let pane = BootstrapPaneObservation::try_new(
                    current.clone(),
                    HostTargetId::new("w1"),
                    HostTargetId::new("w1:other-tab"),
                    crate::protocol::handoff::topology_contract_tests::created().witness,
                )
                .unwrap();
                guard = BootstrapAttachmentGuard::try_new(
                    &ResolveSeat {
                        target: current.target.clone(),
                        operation: a.resolve_operation.clone(),
                    },
                    pane,
                    &admission,
                )
                .unwrap();
            }
            _ => {}
        }
        let mut ns = id.payload.handoff.namespace.clone();
        if corruption == "namespace" {
            ns.state_dir = "/copied-state".into();
        }
        let command = AttachBootstrapHandoff {
            identity: id.clone(),
            operation: id.payload.attach_key.clone(),
            attachment: a,
        };
        let tx = db.transaction().unwrap();
        assert!(
            topology_handoff::attach_pending(&tx, &ns, &command, &guard).is_err(),
            "{corruption}"
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM bootstrap_attachments", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            tx.query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "created"
        );
        tx.commit().unwrap();
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

fn new_thread_fixture() -> (
    StoreContext,
    Connection,
    PathBuf,
    BootstrapIdentity,
    HandoffIdentity,
) {
    new_thread_fixture_with_policy(None)
}
fn new_thread_fixture_with_policy(
    policy: Option<HandoffPolicy>,
) -> (
    StoreContext,
    Connection,
    PathBuf,
    BootstrapIdentity,
    HandoffIdentity,
) {
    let (context, mut db, path, _) = bound_fixture(100);
    let mut id = crate::protocol::handoff::topology_contract_tests::identity();
    id.payload.handoff.policy = policy;
    id.claim = claim("s1");
    id.scope = crate::cli::journal::IntentScope::Cooperative {
        instance: "i".into(),
        seat: SeatId::new("s1"),
    };
    id.payload.handoff.channel = HandoffChannel::New {
        name: Some("new-thread".into()),
        topic: "new topic".into(),
        goal: "new goal".into(),
    };
    id.digest = id.semantic_digest().unwrap();
    let child = HandoffIdentity {
        compound: id.payload.handoff_key.clone(),
        digest: "b".repeat(64),
        claim: id.claim.clone(),
        thread: None,
        recipient: SeatId::new("s2"),
        create_key: id.payload.handoff.keys.create.clone(),
        invite_key: id.payload.handoff.keys.invite.clone(),
        send_key: id.payload.handoff.keys.send.clone(),
    };
    let a = BootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: crate::protocol::handoff::topology_contract_tests::created(),
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: SeatId::new("s2"),
        handoff: child.clone(),
    };
    {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &id.payload.handoff.namespace, &id, UtcMillis(0))
            .unwrap();
        tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1 AND attempt=1",[serde_json::to_vec(&a.created).unwrap()]).unwrap();
        tx.execute(
            "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
            [serde_json::to_vec(&a).unwrap()],
        )
        .unwrap();
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='attached' WHERE id=1",
            [],
        )
        .unwrap();
        handoff::begin_linked_pending(&tx, &id.payload.handoff.namespace, &child, UtcMillis(1))
            .unwrap();
        tx.commit().unwrap();
    }
    (context, db, path, id, child)
}
fn create_for(id: &BootstrapIdentity) -> CreateThread {
    CreateThread {
        name: Some("new-thread".into()),
        topic: "new topic".into(),
        goal: "new goal".into(),
        operation: id.payload.handoff.keys.create.clone(),
        claim: id.claim.clone(),
    }
}
fn create_permit(command: &CreateThread) -> MutationPermit {
    permit(
        "s1",
        command.operation.as_str(),
        ObligationRef::CheckIn(SeatId::new("s1")),
        cooperative_payload_hash("create_thread", command).unwrap(),
        100,
    )
}
#[test]
fn topology_attachment_real_create_links_both_fences_in_the_deciding_transaction() {
    let (context, mut db, path, id, child) = new_thread_fixture();
    let command = create_for(&id);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let CommandResult::ThreadCreated(thread) = create_thread_in_namespace(
        &context,
        &mut db,
        &budget,
        &command,
        create_permit(&command),
        &id.payload.handoff.namespace,
    )
    .unwrap() else {
        panic!("missing created thread")
    };
    assert_eq!(
        handoff::current(&db, &child).unwrap().unwrap().thread,
        Some(thread.clone())
    );
    assert_eq!(
        db.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        thread.as_str()
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s1'",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    assert_eq!(
        create_thread_in_namespace(
            &context,
            &mut db,
            &budget,
            &command,
            create_permit(&command),
            &id.payload.handoff.namespace
        )
        .unwrap(),
        CommandResult::ThreadCreated(thread)
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn topology_attachment_real_create_refuses_unscoped_foreign_and_changed_channel_without_writes() {
    for change in ["unscoped", "namespace", "topic", "goal", "name"] {
        let (context, mut db, path, id, child) = new_thread_fixture();
        let mut command = create_for(&id);
        let mut ns = id.payload.handoff.namespace.clone();
        match change {
            "namespace" => ns.host_endpoint = "/foreign.sock".into(),
            "topic" => command.topic = "wrong".into(),
            "goal" => command.goal = "wrong".into(),
            "name" => command.name = Some("wrong".into()),
            _ => {}
        }
        let budget = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        };
        let result = if change == "unscoped" {
            create_thread(
                &context,
                &mut db,
                &budget,
                &command,
                create_permit(&command),
            )
        } else {
            create_thread_in_namespace(
                &context,
                &mut db,
                &budget,
                &command,
                create_permit(&command),
                &ns,
            )
        };
        assert!(result.is_err(), "{change}");
        assert_eq!(
            db.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0,
            "{change}"
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{change}"
        );
        assert_eq!(handoff::current(&db, &child).unwrap().unwrap().thread, None);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
#[test]
fn topology_attachment_real_create_rolls_back_new_thread_and_both_links_on_late_failure() {
    let (context, mut db, path, id, child) = new_thread_fixture();
    let command = create_for(&id);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    db.execute_batch("CREATE TEMP TRIGGER reject_join BEFORE INSERT ON memberships BEGIN SELECT RAISE(ABORT,'late failure'); END;").unwrap();
    assert!(
        create_thread_in_namespace(
            &context,
            &mut db,
            &budget,
            &command,
            create_permit(&command),
            &id.payload.handoff.namespace
        )
        .is_err()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    assert_eq!(handoff::current(&db, &child).unwrap().unwrap().thread, None);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[path = "topology_composition.rs"]
mod composition;

#[test]
fn actual_main_linked_phase_preserves_ordinary_and_rejects_lazy_mode() {
    use crate::protocol::commands::{DeliveryMode, PermitMutation, SendMessage};
    let (context, mut db, path, id, child) = new_thread_fixture();
    let create = create_for(&id);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let CommandResult::ThreadCreated(thread) = create_thread_in_namespace(
        &context,
        &mut db,
        &budget,
        &create,
        create_permit(&create),
        &id.payload.handoff.namespace,
    )
    .unwrap() else {
        panic!("missing exact canonical thread")
    };
    let before = serde_json::to_vec(
        &topology_handoff::current(&db, &id.payload.handoff.namespace, &id).unwrap(),
    )
    .unwrap();
    let before_child = serde_json::to_vec(&handoff::current(&db, &child).unwrap()).unwrap();
    let mut send = SendMessage {
        delivery_mode: DeliveryMode::Ordinary,
        thread,
        body: id.payload.handoff.body.clone(),
        direct_recipient: None,
        invited_recipients: vec![child.recipient.clone()],
        deadline_millis: None,
        operation: id.payload.handoff.keys.send.clone(),
        claim: id.claim.clone(),
        relays_user: false,
        user_intent: None,
    };
    topology_handoff::validate_selected_child_phase(
        &db,
        &id.payload.handoff.namespace,
        &PermitMutation::SendMessage(send.clone()),
        false,
    )
    .unwrap();
    send.direct_recipient = Some(child.recipient.clone());
    assert_eq!(
        topology_handoff::validate_selected_child_phase(
            &db,
            &id.payload.handoff.namespace,
            &PermitMutation::SendMessage(send.clone()),
            false,
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    send.direct_recipient = None;
    send.delivery_mode = DeliveryMode::Lazy;
    assert!(
        crate::protocol::commands::Command::SendMessage(send.clone())
            .validate()
            .is_err()
    );
    let direct_refusal = topology_handoff::validate_selected_child_phase(
        &db,
        &id.payload.handoff.namespace,
        &PermitMutation::SendMessage(send),
        false,
    )
    .is_err();
    assert_eq!(
        serde_json::to_vec(
            &topology_handoff::current(&db, &id.payload.handoff.namespace, &id).unwrap()
        )
        .unwrap(),
        before
    );
    assert_eq!(
        serde_json::to_vec(&handoff::current(&db, &child).unwrap()).unwrap(),
        before_child
    );
    drop(db);
    drop(context);
    std::fs::remove_file(path).unwrap();
    assert!(
        direct_refusal,
        "linked frozen Ordinary phase must reject a changed Lazy mode"
    );
}

#[test]
fn actual_main_linked_phase_rejects_unfrozen_invitation_confirmation() {
    use crate::protocol::commands::{InvitationConfirmation, Invite, PermitMutation};
    let (context, mut db, path, id, child) = new_thread_fixture();
    let create = create_for(&id);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let CommandResult::ThreadCreated(thread) = create_thread_in_namespace(
        &context,
        &mut db,
        &budget,
        &create,
        create_permit(&create),
        &id.payload.handoff.namespace,
    )
    .unwrap() else {
        panic!()
    };
    let before = db.total_changes();
    let mut invite = Invite {
        confirmation: None,
        thread,
        seat: child.recipient.clone(),
        deadline_millis: None,
        operation: id.payload.handoff.keys.invite.clone(),
        claim: id.claim.clone(),
    };
    topology_handoff::validate_selected_child_phase(
        &db,
        &id.payload.handoff.namespace,
        &PermitMutation::Invite(invite.clone()),
        false,
    )
    .unwrap();
    for mode in [
        InvitationConfirmation::Lazy,
        InvitationConfirmation::RequireAck,
    ] {
        invite.confirmation = Some(mode);
        let error = topology_handoff::validate_selected_child_phase(
            &db,
            &id.payload.handoff.namespace,
            &PermitMutation::Invite(invite.clone()),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::OperationPayloadMismatch);
    }
    assert_eq!(
        db.total_changes(),
        before,
        "policy refusal must precede effects"
    );
    drop(db);
    drop(context);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn handoff_policy_linked_frozen_children_match_both_modes_and_reject_tampering() {
    use crate::protocol::commands::{
        DeliveryMode, InvitationConfirmation, Invite, PermitMutation, SendMessage,
    };
    for confirmation in [
        InvitationConfirmation::Lazy,
        InvitationConfirmation::RequireAck,
    ] {
        let invite_duration = (confirmation == InvitationConfirmation::RequireAck).then_some(11000);
        let (context, mut db, path, id, child) =
            new_thread_fixture_with_policy(Some(HandoffPolicy {
                deadline_millis: Some(7000),
                invitation_confirmation: confirmation,
                invite_deadline_millis: invite_duration,
            }));
        let create = create_for(&id);
        let budget = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        };
        let CommandResult::ThreadCreated(thread) = create_thread_in_namespace(
            &context,
            &mut db,
            &budget,
            &create,
            create_permit(&create),
            &id.payload.handoff.namespace,
        )
        .unwrap() else {
            panic!()
        };
        let invite = Invite {
            confirmation: Some(confirmation),
            thread: thread.clone(),
            seat: child.recipient.clone(),
            deadline_millis: invite_duration,
            operation: id.payload.handoff.keys.invite.clone(),
            claim: id.claim.clone(),
        };
        let send = SendMessage {
            direct_recipient: None,
            delivery_mode: DeliveryMode::Ordinary,
            thread,
            body: id.payload.handoff.body.clone(),
            invited_recipients: vec![child.recipient.clone()],
            deadline_millis: Some(7000),
            relays_user: false,
            user_intent: None,
            operation: id.payload.handoff.keys.send.clone(),
            claim: id.claim.clone(),
        };
        let validate = |mutation: &PermitMutation| {
            topology_handoff::validate_selected_child_phase(
                &db,
                &id.payload.handoff.namespace,
                mutation,
                false,
            )
        };
        let before = db.total_changes();
        validate(&PermitMutation::Invite(invite.clone())).unwrap();
        validate(&PermitMutation::SendMessage(send.clone())).unwrap();
        for change in ["confirmation", "duration", "key"] {
            let mut altered = invite.clone();
            match change {
                "confirmation" => {
                    altered.confirmation = Some(if confirmation == InvitationConfirmation::Lazy {
                        InvitationConfirmation::RequireAck
                    } else {
                        InvitationConfirmation::Lazy
                    })
                }
                "duration" => altered.deadline_millis = Some(19000),
                _ => altered.operation = id.payload.handoff.keys.send.clone(),
            }
            assert!(
                validate(&PermitMutation::Invite(altered)).is_err(),
                "{confirmation:?} {change}"
            );
        }
        for change in ["duration", "key", "mode"] {
            let mut altered = send.clone();
            match change {
                "duration" => altered.deadline_millis = Some(13000),
                "key" => altered.operation = id.payload.handoff.keys.invite.clone(),
                _ => altered.delivery_mode = DeliveryMode::Lazy,
            }
            assert!(
                validate(&PermitMutation::SendMessage(altered)).is_err(),
                "{change}"
            );
        }
        assert_eq!(
            db.total_changes(),
            before,
            "phase validation mutates no canonical or native state"
        );
        drop(db);
        drop(context);
        std::fs::remove_file(path).unwrap();
    }
}

fn automatic_parent(id: &BootstrapIdentity) -> AutomaticBootstrapIdentity {
    AutomaticBootstrapIdentity::new(
        id.scope.clone(),
        id.claim.clone(),
        AutomaticBootstrapPlan {
            version: 2,
            payload: AutomaticBootstrapPayload {
                handoff: AutomaticHandoffPayload::new(
                    id.payload.handoff.namespace.clone(),
                    OperationId::new("auto-child"),
                    "automatic work".into(),
                )
                .unwrap(),
                keys: AutomaticBootstrapKeys::new(OperationId::new("auto-parent")),
                workspace: id.payload.workspace.clone(),
                cwd: id.payload.cwd.clone(),
                label: id.payload.label.clone(),
                focus: false,
                env: Default::default(),
                launch: id.payload.launch.clone(),
                startup_input: None,
            },
        },
    )
    .unwrap()
}

fn automatic_parent_mutate(
    store: &crate::store::SqliteStore,
    id: &AutomaticBootstrapIdentity,
    action: AutomaticBootstrapAction,
    guard: Option<BootstrapAttachmentGuard>,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    let request = AutomaticBootstrapMutation {
        identity: id.clone(),
        action,
    };
    let permit = store.issue_cooperative_permit(
        crate::store::cooperative_permit_request(
            &crate::protocol::commands::PermitMutation::AutomaticBootstrap(Box::new(
                request.clone(),
            )),
        )?,
        budget,
    )?;
    crate::ports::BootstrapStorePort::automatic_bootstrap_mutate(
        store,
        &id.plan.payload.handoff.namespace,
        &request,
        permit,
        guard,
        budget,
    )
}

fn automatic_parent_created(
    store: &crate::store::SqliteStore,
    id: &AutomaticBootstrapIdentity,
    budget: &CallBudget,
) {
    automatic_parent_mutate(store, id, AutomaticBootstrapAction::Begin, None, budget).unwrap();
    automatic_parent_mutate(
        store,
        id,
        AutomaticBootstrapAction::Reserve {
            attempt: BootstrapAttempt::first(),
        },
        None,
        budget,
    )
    .unwrap();
    automatic_parent_mutate(
        store,
        id,
        AutomaticBootstrapAction::RecordCreated {
            attempt: BootstrapAttempt::first(),
            evidence: crate::protocol::handoff::topology_contract_tests::created(),
        },
        None,
        budget,
    )
    .unwrap();
}

#[test]
fn automatic_linked_selected_resolve_attach_and_exact_pair_share_canonical_parent() {
    let (_context, db, path, store, observation, budget, old, a) =
        attachment_fixture_inner(false, false);
    let id = automatic_parent(&old);
    automatic_parent_created(&store, &id, &budget);
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        5,
    );
    let resolved = automatic_parent_mutate(
        &store,
        &id,
        AutomaticBootstrapAction::Resolve {
            attempt: BootstrapAttempt::first(),
        },
        Some(guard),
        &budget,
    )
    .unwrap();
    assert_eq!(
        resolved,
        CommandResult::SeatResolved(a.resolved_seat.clone())
    );
    let child = id.child(&a.created, a.resolved_seat.clone()).unwrap();
    let attachment = AutomaticBootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: a.created,
        resolve_operation: id.plan.payload.keys.resolve.clone(),
        resolved_seat: a.resolved_seat,
        child: child.clone(),
    };
    let mut wrong_original = child.original.clone();
    let AutomaticOriginal::Handoff(ref mut wrong_plan) = wrong_original else {
        panic!("derived child must retain the full handoff original")
    };
    wrong_plan.delivery.payload.body.push_str(" substituted");
    let mut wrong_attachment = attachment.clone();
    wrong_attachment.child =
        AutomaticDeliveryMutation::new(child.scope.clone(), child.claim.clone(), wrong_original)
            .unwrap();
    wrong_attachment.child.validate().unwrap();
    let wrong_guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        6,
    );
    assert!(
        automatic_parent_mutate(
            &store,
            &id,
            AutomaticBootstrapAction::Attach(Box::new(wrong_attachment)),
            Some(wrong_guard),
            &budget,
        )
        .is_err(),
        "a valid child with a recomputed digest is still not the derived child"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automatic_bootstrap_attachments",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        7,
    );
    let attached = automatic_parent_mutate(
        &store,
        &id,
        AutomaticBootstrapAction::Attach(Box::new(attachment)),
        Some(guard),
        &budget,
    )
    .unwrap();
    assert!(
        matches!(attached,CommandResult::AutomaticBootstrap(ref v) if v.state==BootstrapState::Attached)
    );
    let begin = child.begin();
    let permit = store
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&handoff::automatic::inner(&begin).unwrap())
                .unwrap(),
            &budget,
        )
        .unwrap();
    crate::ports::BootstrapStorePort::automatic_delivery_mutate(
        &*store,
        &id.plan.payload.handoff.namespace,
        &begin,
        permit,
        &budget,
    )
    .unwrap();
    let ensure = child.ensure();
    let permit = store
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&handoff::automatic::inner(&ensure).unwrap())
                .unwrap(),
            &budget,
        )
        .unwrap();
    let selected = crate::ports::BootstrapStorePort::automatic_delivery_mutate(
        &*store,
        &id.plan.payload.handoff.namespace,
        &ensure,
        permit,
        &budget,
    )
    .unwrap();
    let raw = handoff::automatic::inner(&ensure).unwrap();
    let raw_permit = store
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&raw).unwrap(),
            &budget,
        )
        .unwrap();
    assert!(
        store.mutate(raw, raw_permit, &budget).is_err(),
        "a reachable unscoped Ensure cannot replay a registered child operation"
    );
    let thread = automatic_selected_thread(&selected).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT thread_id FROM automatic_bootstrap_parents WHERE compound='auto-parent'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        thread.as_str()
    );
    assert_eq!(
        db.query_row(
            "SELECT thread_id FROM direct_threads WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        thread.as_str()
    );
    let CommandResult::AutomaticBootstrap(status) =
        crate::ports::BootstrapStorePort::automatic_bootstrap_query(
            &*store,
            &id.plan.payload.handoff.namespace,
            &AutomaticBootstrapMutation {
                identity: id.clone(),
                action: AutomaticBootstrapAction::Status,
            },
            &budget,
        )
        .unwrap()
    else {
        panic!("parent status")
    };
    let cancel = AutomaticBootstrapMutation {
        identity: id.clone(),
        action: AutomaticBootstrapAction::Recover {
            attempt: status.attempt,
            inspection: status.recovery_inspection().unwrap(),
            disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                reason: "inspected quiescence".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                attached_child: Some(Box::new(child.clone())),
            },
        },
    };
    let actor = OperatorActor::from_peer(
        crate::protocol::authority::PeerIdentity::from_kernel(501),
        501,
    )
    .unwrap();
    assert!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_recover(
            &*store,
            &id.plan.payload.handoff.namespace,
            &cancel,
            &actor,
            None,
            &budget,
        )
        .is_err(),
        "operator cancellation cannot release the live exact child"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automatic_bootstrap_recoveries",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn automatic_linked_registered_child_cannot_fall_back_before_attachment() {
    let (_context, db, path, store, _observation, budget, old, a) =
        attachment_fixture_inner(false, false);
    let id = automatic_parent(&old);
    automatic_parent_created(&store, &id, &budget);
    let child = id.child(&a.created, a.resolved_seat).unwrap().begin();
    let permit = store
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&handoff::automatic::inner(&child).unwrap())
                .unwrap(),
            &budget,
        )
        .unwrap();
    assert!(
        crate::ports::BootstrapStorePort::automatic_delivery_mutate(
            &*store,
            &id.plan.payload.handoff.namespace,
            &child,
            permit,
            &budget
        )
        .is_err()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automatic_handoff_companions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let raw = handoff::automatic::inner(&child.ensure()).unwrap();
    let raw_permit = store
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&raw).unwrap(),
            &budget,
        )
        .unwrap();
    assert!(
        store.mutate(raw, raw_permit, &budget).is_err(),
        "a reachable unscoped Ensure cannot create a registered child before attachment"
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

fn automatic_child_mutate(
    store: &crate::store::SqliteStore,
    request: &AutomaticDeliveryMutation,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    let permit = store.issue_cooperative_permit(
        crate::store::cooperative_permit_request(&handoff::automatic::inner(request)?)?,
        budget,
    )?;
    crate::ports::BootstrapStorePort::automatic_delivery_mutate(
        store,
        &request.plan().payload.namespace,
        request,
        permit,
        budget,
    )
}

#[test]
fn automatic_linked_terminal_reconciles_completed_child_without_reopening_or_relaunch() {
    use sha2::Digest;
    let (context, mut db, path, store, observation, budget, old, a) =
        attachment_fixture_inner(false, false);
    let id = automatic_parent(&old);
    automatic_parent_created(&store, &id, &budget);
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        5,
    );
    automatic_parent_mutate(
        &store,
        &id,
        AutomaticBootstrapAction::Resolve {
            attempt: BootstrapAttempt::first(),
        },
        Some(guard),
        &budget,
    )
    .unwrap();
    let child = id.child(&a.created, a.resolved_seat.clone()).unwrap();
    let attachment = AutomaticBootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: a.created,
        resolve_operation: id.plan.payload.keys.resolve.clone(),
        resolved_seat: a.resolved_seat,
        child: child.clone(),
    };
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        6,
    );
    automatic_parent_mutate(
        &store,
        &id,
        AutomaticBootstrapAction::Attach(Box::new(attachment.clone())),
        Some(guard),
        &budget,
    )
    .unwrap();
    automatic_child_mutate(&store, &child.begin(), &budget).unwrap();
    let thread = automatic_selected_thread(
        &automatic_child_mutate(&store, &child.ensure(), &budget).unwrap(),
    )
    .unwrap()
    .clone();
    let start = AutomaticBootstrapMutation {
        identity: id.clone(),
        action: AutomaticBootstrapAction::CheckStart,
    };
    assert!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_query(
            &*store,
            &id.plan.payload.handoff.namespace,
            &start,
            &budget
        )
        .is_err(),
        "Ensure alone cannot authorize possible start"
    );
    let send = child.send(thread.clone());
    loop {
        if let crate::ports::SendPreparationProgress::Ready { .. } =
            crate::ports::BootstrapStorePort::automatic_delivery_prepare_send_step(
                &*store,
                &id.plan.payload.handoff.namespace,
                &send,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
            .unwrap()
        {
            break;
        }
    }
    automatic_child_mutate(&store, &send, &budget).unwrap();
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        7,
    );
    crate::ports::BootstrapStorePort::automatic_bootstrap_check_start(
        &*store,
        &id.plan.payload.handoff.namespace,
        &start,
        guard,
        &budget,
    )
    .unwrap();
    let previous: String = db
        .query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
            rusqlite::params![thread.as_str(), attachment.resolved_seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE memberships SET state='left',voluntary_state='left' WHERE thread_id=?1 AND seat_id=?2",
        rusqlite::params![thread.as_str(), attachment.resolved_seat.as_str()],
    )
    .unwrap();
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        8,
    );
    assert!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_check_start(
            &*store,
            &id.plan.payload.handoff.namespace,
            &start,
            guard,
            &budget
        )
        .is_err(),
        "published Send cannot grant possible start after recipient leave"
    );
    db.execute(
        "UPDATE memberships SET state=?3,voluntary_state=?3 WHERE thread_id=?1 AND seat_id=?2",
        rusqlite::params![thread.as_str(), attachment.resolved_seat.as_str(), previous],
    )
    .unwrap();
    automatic_child_mutate(&store, &child.complete(), &budget).unwrap();
    let AutomaticOriginal::Handoff(plan) = &child.original else {
        panic!("outer")
    };
    let prompt = crate::cli::handoff::bootstrap_v1::retained_prompts(
        thread.as_str(),
        &plan.context,
        &id.claim.instance,
    )
    .into_iter()
    .next()
    .unwrap();
    let mut argv = id.plan.payload.launch.argv.clone();
    argv.push(prompt);
    let report = serde_json::json!({"outcome":"started","pane":attachment.created.root_pane,"seat":attachment.resolved_seat,"harness":id.plan.payload.launch.harness.as_str(),"argv":argv});
    let retained = LinkedBootstrapReport {
        thread: thread.clone(),
        recipient: attachment.resolved_seat.clone(),
        pane: attachment.created.root_pane.clone(),
        kind: "launch".into(),
        launch: id.plan.payload.launch.clone(),
        report_digest: format!(
            "{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&report).unwrap())
        ),
        report,
        terminal: attachment.created.terminal.clone(),
        host_incarnation: attachment.created.host_incarnation.clone(),
    };
    let action = AutomaticBootstrapAction::Complete {
        attachment: Box::new(attachment.clone()),
        retained: retained.clone(),
    };
    db.execute(
        "UPDATE memberships SET state='left',voluntary_state='left' WHERE thread_id=?1",
        [thread.as_str()],
    )
    .unwrap();
    db.execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
    for seat in [&id.claim.seat, &attachment.resolved_seat] {
        crate::store::seats::mutate_operator(
            &context,
            &mut db,
            "i",
            crate::ports::OperatorRequest::Retire(crate::protocol::commands::OperatorRetire {
                seat: seat.clone(),
                operation: OperationId::new(format!("terminal-retire-{}", seat.as_str())),
            }),
            OperatorActor::from_peer(
                crate::protocol::authority::PeerIdentity::from_kernel(501),
                501,
            )
            .unwrap(),
            None,
        )
        .unwrap();
    }
    let request = AutomaticBootstrapMutation {
        identity: id.clone(),
        action: action.clone(),
    };
    let done = crate::ports::BootstrapStorePort::automatic_bootstrap_complete(
        &*store,
        &id.plan.payload.handoff.namespace,
        &request,
        &budget,
    )
    .unwrap();
    assert!(
        matches!(done,CommandResult::AutomaticBootstrap(ref status) if status.state==BootstrapState::Completed && status.completed.as_ref()==Some(&retained))
    );
    db.execute(
        "UPDATE memberships SET state='left',voluntary_state='left' WHERE thread_id=?1",
        [thread.as_str()],
    )
    .unwrap();
    db.execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
    assert_eq!(
        automatic_parent_mutate(&store, &id, action, None, &budget).unwrap(),
        done
    );
    assert!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_query(
            &*store,
            &id.plan.payload.handoff.namespace,
            &start,
            &budget
        )
        .is_err()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND kind='ordinary'",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for at in [0, 60000, 120000, 180000] {
        let rt = crate::store::archival::Runtime {
            boot: "terminal-release".into(),
            mono: at,
            utc: UtcMillis(at),
            after_ms: 60000,
            host_generation: 0,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered".into()),
        };
        for _ in 0..30 {
            if !crate::store::archival::advance(&db, "i", &rt)
                .unwrap()
                .has_more
            {
                break;
            }
        }
    }
    assert!(
        db.query_row(
            "SELECT archived FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        "completed parent and child release archival after retired fixtures leave no protected work"
    );
    assert_eq!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_complete(
            &*store,
            &id.plan.payload.handoff.namespace,
            &request,
            &budget
        )
        .unwrap(),
        done
    );
    assert_eq!(
        db.query_row("SELECT state FROM automatic_bootstrap_parents", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "completed"
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn automatic_linked_wire_refuses_unknown_fields_inside_derived_launch() {
    let (_context, db, path, _store, _observation, _budget, old, a) =
        attachment_fixture_inner(false, false);
    let id = automatic_parent(&old);
    let child = id.child(&a.created, a.resolved_seat.clone()).unwrap();
    let attachment = AutomaticBootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: a.created,
        resolve_operation: id.plan.payload.keys.resolve.clone(),
        resolved_seat: a.resolved_seat,
        child,
    };
    let command = crate::protocol::commands::Command::AutomaticBootstrap(Box::new(
        AutomaticBootstrapMutation {
            identity: id,
            action: AutomaticBootstrapAction::Attach(Box::new(attachment)),
        },
    ));
    let mut wire = serde_json::to_value(&command).unwrap();
    wire["args"]["action"]["request"]["child"]["original"]["plan"]["launch"]["unexpected_authority"] =
        serde_json::json!(true);
    assert!(
        serde_json::from_value::<crate::protocol::commands::Command>(wire).is_err(),
        "V2 derived launch wire must reject lost fields"
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn automatic_linked_created_recovery_uses_fresh_guard_and_replays_before_current_lifecycle() {
    let (_context, db, path, store, observation, budget, old, a) =
        attachment_fixture_inner(false, false);
    let id = automatic_parent(&old);
    automatic_parent_mutate(&store, &id, AutomaticBootstrapAction::Begin, None, &budget).unwrap();
    let CommandResult::AutomaticBootstrapReserved(reserved) = automatic_parent_mutate(
        &store,
        &id,
        AutomaticBootstrapAction::Reserve {
            attempt: BootstrapAttempt::first(),
        },
        None,
        &budget,
    )
    .unwrap() else {
        panic!("reserve")
    };
    let request = AutomaticBootstrapMutation {
        identity: id.clone(),
        action: AutomaticBootstrapAction::Recover {
            attempt: BootstrapAttempt::first(),
            inspection: reserved.status.recovery_inspection().unwrap(),
            disposition: AutomaticBootstrapRecoveryDisposition::CreatedPane {
                evidence: Box::new(a.created.clone()),
                structural_reference: HostCallId::new("operator-correlation"),
            },
        },
    };
    let actor = crate::protocol::authority::OperatorActor::from_peer(
        crate::protocol::authority::PeerIdentity::from_kernel(501),
        501,
    )
    .unwrap();
    let before: i64 = db
        .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    let guard = attachment_guard(
        &store,
        &observation,
        &budget,
        &id.plan.payload.keys.resolve,
        5,
    );
    let saved = crate::ports::BootstrapStorePort::automatic_bootstrap_recover(
        &*store,
        &id.plan.payload.handoff.namespace,
        &request,
        &actor,
        Some(guard),
        &budget,
    )
    .unwrap();
    assert!(
        matches!(saved,CommandResult::AutomaticBootstrap(ref status) if status.state==BootstrapState::Created && status.creation.as_ref()==Some(&a.created))
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    let CommandResult::AutomaticBootstrap(status) = &saved else {
        panic!("status")
    };
    let cancel = AutomaticBootstrapMutation {
        identity: id.clone(),
        action: AutomaticBootstrapAction::Recover {
            attempt: BootstrapAttempt::first(),
            inspection: status.recovery_inspection().unwrap(),
            disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                reason: "inspected quiescence".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                attached_child: None,
            },
        },
    };
    let cancelled = crate::ports::BootstrapStorePort::automatic_bootstrap_recover(
        &*store,
        &id.plan.payload.handoff.namespace,
        &cancel,
        &actor,
        None,
        &budget,
    )
    .unwrap();
    assert!(
        matches!(cancelled,CommandResult::AutomaticBootstrap(ref status) if status.state==BootstrapState::Cancelled && status.creation.as_ref()==Some(&a.created))
    );
    db.execute_batch("UPDATE occupant_bindings SET ended_at=1;UPDATE host_instances SET lifecycle_revision=lifecycle_revision+5;").unwrap();
    assert_eq!(
        crate::ports::BootstrapStorePort::automatic_bootstrap_recovery_replay(
            &*store,
            &id.plan.payload.handoff.namespace,
            &request,
            &actor,
            &budget
        )
        .unwrap(),
        Some(saved)
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automatic_bootstrap_recoveries",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(
        store
            .resolve_seat(
                ResolveSeat {
                    target: a.created.root_pane,
                    operation: id.plan.payload.keys.resolve.clone()
                },
                OrdinaryResolutionAttempt::ReplayOnly,
                &budget
            )
            .is_err()
    );
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn automatic_linked_created_recovery_refuses_changed_witness_and_lifecycle_without_effects() {
    for changed in ["witness", "lifecycle"] {
        let (_context, db, path, store, observation, budget, old, a) =
            attachment_fixture_inner(false, false);
        let id = automatic_parent(&old);
        automatic_parent_mutate(&store, &id, AutomaticBootstrapAction::Begin, None, &budget)
            .unwrap();
        let CommandResult::AutomaticBootstrapReserved(reserved) = automatic_parent_mutate(
            &store,
            &id,
            AutomaticBootstrapAction::Reserve {
                attempt: BootstrapAttempt::first(),
            },
            None,
            &budget,
        )
        .unwrap() else {
            panic!("reserve")
        };
        let mut evidence = a.created;
        let guard = attachment_guard(
            &store,
            &observation,
            &budget,
            &id.plan.payload.keys.resolve,
            5,
        );
        if changed == "witness" {
            evidence.witness.socket.inode += 1;
        } else {
            db.execute(
                "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1",
                [],
            )
            .unwrap();
        }
        let request = AutomaticBootstrapMutation {
            identity: id.clone(),
            action: AutomaticBootstrapAction::Recover {
                attempt: BootstrapAttempt::first(),
                inspection: reserved.status.recovery_inspection().unwrap(),
                disposition: AutomaticBootstrapRecoveryDisposition::CreatedPane {
                    evidence: Box::new(evidence),
                    structural_reference: HostCallId::new("operator-correlation"),
                },
            },
        };
        let actor = crate::protocol::authority::OperatorActor::from_peer(
            crate::protocol::authority::PeerIdentity::from_kernel(501),
            501,
        )
        .unwrap();
        assert!(
            crate::ports::BootstrapStorePort::automatic_bootstrap_recover(
                &*store,
                &id.plan.payload.handoff.namespace,
                &request,
                &actor,
                Some(guard),
                &budget
            )
            .is_err(),
            "{changed}"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM automatic_bootstrap_recoveries",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT state FROM automatic_bootstrap_parents", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "possible_creation"
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

// Catches released128/256KiB checks rejecting body-bearing automatic records.
// Public semantic acceptance and canonical loaders are tested by the successor task.
#[test]
fn automatic_capacity_actual_serializer_maximum_body_records() {
    use sha2::Digest;
    for body in ["x".repeat(65_536), "\u{1}".repeat(65_536)] {
        let (_context, db, path, store, _observation, _budget, old, a) =
            attachment_fixture_inner(false, false);
        let mut id = automatic_parent(&old);
        let mut attachment = AutomaticBootstrapAttachment::derive(
            &id,
            BootstrapAttempt::first(),
            a.created.clone(),
            a.resolved_seat.clone(),
        )
        .unwrap();
        id.plan.payload.handoff.body = body.clone();
        id.digest = id.semantic_digest().unwrap();
        let mut child = serde_json::to_value(&attachment.child).unwrap();
        fn replace_body(value: &mut serde_json::Value, body: &str) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, value) in fields {
                        if key == "body" {
                            *value = serde_json::Value::String(body.into());
                        } else {
                            replace_body(value, body);
                        }
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        replace_body(value, body);
                    }
                }
                _ => {}
            }
        }
        replace_body(&mut child, &body);
        attachment.child = serde_json::from_value(child).unwrap();
        let recovery = AutomaticBootstrapRecoveryResult {
            operation: OperationId::new("capacity-recovery"),
            attempt: BootstrapAttempt::first(),
            inspection: BootstrapRecoveryInspection {
                version: 1,
                digest: "b".repeat(64),
            },
            disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                reason: "inspected".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                attached_child: Some(Box::new(attachment.child.clone())),
            },
            operator_uid: 501,
            operator_provenance: "operator:local-user:501".into(),
            state: BootstrapState::Cancelled,
            administrative_revision: 1,
        };
        let result = CommandResult::AutomaticBootstrap(Box::new(AutomaticBootstrapResult {
            identity: id.clone(),
            state: BootstrapState::Cancelled,
            attempt: BootstrapAttempt::first(),
            attempt_state: BootstrapAttemptState::Created,
            administrative_revision: 1,
            creation: Some(a.created.clone()),
            attachment: Some(attachment.clone()),
            thread: Some(ThreadId::new("thread")),
            completed: None,
            recovery: Some(recovery),
        }));
        let original = serde_json::to_vec(&id).unwrap();
        let attached = serde_json::to_vec(&attachment).unwrap();
        let recovered = serde_json::to_vec(&result).unwrap();
        let retained = LinkedBootstrapReport {
            thread: ThreadId::new("thread"),
            recipient: attachment.resolved_seat.clone(),
            pane: attachment.created.root_pane.clone(),
            kind: "launch".into(),
            launch: id.plan.payload.launch.clone(),
            report: serde_json::json!({"outcome":"started","pane":attachment.created.root_pane,"seat":attachment.resolved_seat,"harness":id.plan.payload.launch.harness.as_str(),"argv":["launch"]}),
            report_digest: "c".repeat(64),
            terminal: attachment.created.terminal.clone(),
            host_incarnation: attachment.created.host_incarnation.clone(),
        };
        let request_bytes = |identity: AutomaticBootstrapIdentity, action| {
            serde_json::to_vec(&crate::protocol::wire::WireRequest {
                version: crate::protocol::wire::PROTOCOL_VERSION,
                request_id: "capacity".into(),
                expected_instance: "00000000-0000-4000-8000-000000000001".into(),
                expected_boot: Some("00000000-0000-4000-8000-000000000002".into()),
                output: None,
                command: crate::protocol::commands::Command::AutomaticBootstrap(Box::new(
                    AutomaticBootstrapMutation { identity, action },
                )),
            })
            .unwrap()
        };
        let complete = request_bytes(
            id.clone(),
            AutomaticBootstrapAction::Complete {
                attachment: Box::new(attachment.clone()),
                retained: retained.clone(),
            },
        );
        let recover = request_bytes(
            id.clone(),
            AutomaticBootstrapAction::Recover {
                attempt: BootstrapAttempt::first(),
                inspection: BootstrapRecoveryInspection {
                    version: 1,
                    digest: "b".repeat(64),
                },
                disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                    reason: "inspected".into(),
                    quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                    attached_child: Some(Box::new(attachment.child.clone())),
                },
            },
        );
        assert!(complete.len() <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES);
        assert!(recover.len() <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES);
        let mut maximum_argv = id.clone();
        maximum_argv.plan.payload.launch.argv = vec!["\u{1}".repeat(4096); 16];
        maximum_argv.plan.payload.launch.validate().unwrap();
        let maximum_original = serde_json::to_vec(&maximum_argv).unwrap();
        let maximum_complete = request_bytes(
            maximum_argv,
            AutomaticBootstrapAction::Complete {
                attachment: Box::new(attachment.clone()),
                retained,
            },
        );
        eprintln!(
            "automatic requests escaped={} Complete={} Recover={} maxargv_original={} maxargv_Complete={}",
            body.starts_with('\u{1}'),
            complete.len(),
            recover.len(),
            maximum_original.len(),
            maximum_complete.len()
        );
        if body.starts_with('\u{1}') {
            assert!(maximum_original.len() > 524288);
            assert!(db.execute("INSERT INTO automatic_bootstrap_parents(id,instance_id,actor_scope,compound,state_dir,host_endpoint,original_json,state,current_attempt) VALUES(98,'i','seat:sender','too-large','/state','/host',?1,'created',1)",[&maximum_original]).is_err());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM automatic_bootstrap_parents WHERE id=98",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        if body.starts_with('\u{1}') {
            let mut near = id.clone();
            // Largest six-byte escaped argv addition below the aggregate SQL ceiling.
            let mut lo = 0_usize;
            let mut hi = 65536_usize;
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                near.plan.payload.launch.argv = "\u{1}"
                    .repeat(mid)
                    .as_bytes()
                    .chunks(4096)
                    .map(|v| String::from_utf8(v.to_vec()).unwrap())
                    .collect();
                if serde_json::to_vec(&near).unwrap().len() <= 524288 {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            near.plan.payload.launch.argv = "\u{1}"
                .repeat(lo)
                .as_bytes()
                .chunks(4096)
                .map(|v| String::from_utf8(v.to_vec()).unwrap())
                .collect();
            near.plan.payload.launch.validate().unwrap();
            let near_original = serde_json::to_vec(&near).unwrap();
            let report = serde_json::json!({"outcome":"started","pane":attachment.created.root_pane,"seat":attachment.resolved_seat,"harness":near.plan.payload.launch.harness.as_str(),"argv":near.plan.payload.launch.argv});
            let near_retained = LinkedBootstrapReport {
                thread: ThreadId::new("thread"),
                recipient: attachment.resolved_seat.clone(),
                pane: attachment.created.root_pane.clone(),
                kind: "launch".into(),
                launch: near.plan.payload.launch.clone(),
                report_digest: format!(
                    "{:x}",
                    sha2::Sha256::digest(serde_json::to_vec(&report).unwrap())
                ),
                report,
                terminal: attachment.created.terminal.clone(),
                host_incarnation: attachment.created.host_incarnation.clone(),
            };
            near_retained.validate().unwrap();
            let near_complete = request_bytes(
                near,
                AutomaticBootstrapAction::Complete {
                    attachment: Box::new(attachment.clone()),
                    retained: near_retained,
                },
            );
            eprintln!(
                "automatic nearaggregate original={} Complete={} rawargv={}",
                near_original.len(),
                near_complete.len(),
                lo
            );
            assert!(near_original.len() <= 524288);
            assert!(
                near_complete.len() > crate::protocol::wire::MAX_WIRE_FRAME_BYTES,
                "SQL capacity alone is not public preflight"
            );
        }
        let wire = serde_json::to_vec(&crate::protocol::wire::WireResponse {
            version: crate::protocol::wire::PROTOCOL_VERSION,
            request_id: "capacity".into(),
            instance: "00000000-0000-4000-8000-000000000001".into(),
            daemon_boot: "00000000-0000-4000-8000-000000000002".into(),
            result: Ok(result),
        })
        .unwrap();
        eprintln!(
            "automatic capacity escaped={} original={} attachment={} recovery={} WireResponse={} framed={}",
            body.starts_with('\u{1}'),
            original.len(),
            attached.len(),
            recovered.len(),
            wire.len(),
            wire.len() + 4
        );
        assert!(original.len() <= 524288);
        assert!(attached.len() <= 524288);
        assert!(recovered.len() <= 2097152);
        db.execute("INSERT INTO automatic_bootstrap_parents(id,instance_id,actor_scope,compound,state_dir,host_endpoint,original_json,state,current_attempt) VALUES(99,'i','seat:sender','capacity','/state','/host',?1,'created',1)",[&original]).unwrap();
        db.execute(
            "INSERT INTO automatic_bootstrap_attempts VALUES(99,1,'created',0,?1)",
            [serde_json::to_vec(&a.created).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO automatic_bootstrap_attachments VALUES(99,1,?1)",
            [&attached],
        )
        .unwrap();
        db.execute("INSERT INTO automatic_bootstrap_recoveries VALUES(99,1,'cancellation','capacity-recovery',1,?1)",[&recovered]).unwrap();
        assert!(
            db.execute(
                "INSERT INTO automatic_bootstrap_reports VALUES(99,?1)",
                [vec![b'x'; 2097153]]
            )
            .is_err()
        );
        assert!(
            db.execute(
                "INSERT INTO automatic_bootstrap_attempts VALUES(99,2,'created',0,?1)",
                [vec![b'x'; 131073]]
            )
            .is_err()
        );
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
