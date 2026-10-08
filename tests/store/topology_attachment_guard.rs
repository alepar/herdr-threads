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
    for target in &mut snapshot.targets {
        target.host_boot = created.host_incarnation.clone();
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
        identity: "test-incarnation".into(),
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
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('sender',1,'w1:p1',?1,1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0,'sender-terminal','test-incarnation')",[created.host_incarnation.as_str()]).unwrap();
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
    {
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
    wrong_boot.host_boot = HostBootId::new("changed");
    let pane = BootstrapPaneObservation::try_new(
        wrong_boot,
        HostTargetId::new("w1"),
        HostTargetId::new("w1:t2"),
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
    let (context, mut db, path, _) = bound_fixture(100);
    let mut id = crate::protocol::handoff::topology_contract_tests::identity();
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
