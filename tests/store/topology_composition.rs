//! Actual approved attempts writer versus namespace-aware attachment/Begin.
//! Two connections establish deciding transaction order, with no timing oracle.
use super::*;
use crate::store::topology_handoff::attempts::recover;
use std::sync::mpsc;

fn cancellation_request(id: &BootstrapIdentity, a: &BootstrapAttachment) -> RecoverBootstrap {
    let mut request = RecoverBootstrap {
        identity: id.clone(),
        expected_attempt: a.attempt,
        operation: OperationId::new("placeholder"),
        disposition: BootstrapRecoveryDisposition::Cancelled {
            reason: "inspected quiescent lost pane".into(),
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: Some(a.handoff.clone()),
            },
        },
    };
    request.operation = request.decision_operation().unwrap();
    request
}

fn real_attached() -> AttachmentFixture {
    let (context, mut db, path, store, observation, budget, id, a) =
        attachment_fixture_with_recording(true);
    let guard = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 3);
    let tx = db.transaction().unwrap();
    let result = topology_handoff::attach_pending(
        &tx,
        &id.payload.handoff.namespace,
        &AttachBootstrapHandoff {
            identity: id.clone(),
            operation: id.payload.attach_key.clone(),
            attachment: a.clone(),
        },
        &guard,
    )
    .unwrap();
    assert_eq!(result.state, BootstrapState::Attached);
    tx.commit().unwrap();
    (context, db, path, store, observation, budget, id, a)
}

#[test]
fn topology_composition_real_cancel_and_namespace_begin_serialize_in_both_orders() {
    for cancel_first in [true, false] {
        let (_context, mut db, path, store, _observation, _budget, id, a) = real_attached();
        let request = cancellation_request(&id, &a);
        let ns = id.payload.handoff.namespace.clone();
        let first = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        if cancel_first {
            let saved = recover(&first, &ns, &request, 501, UtcMillis(102), None).unwrap();
            assert_eq!(saved.state, BootstrapState::Cancelled);
            assert_eq!(saved.creation, Some(a.created.clone()));
        } else {
            assert_eq!(
                handoff::begin_linked_pending(&first, &ns, &a.handoff, UtcMillis(102))
                    .unwrap()
                    .state,
                HandoffState::Live
            );
        }
        let second_path = path.clone();
        let second_ns = ns.clone();
        let second_request = request.clone();
        let second_child = a.handoff.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            let mut other = Connection::open(second_path).unwrap();
            other
                .busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            ready_tx.send(()).unwrap();
            let tx = other
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let error = if cancel_first {
                handoff::begin_linked_pending(&tx, &second_ns, &second_child, UtcMillis(103))
                    .unwrap_err()
            } else {
                recover(&tx, &second_ns, &second_request, 501, UtcMillis(103), None).unwrap_err()
            };
            // Catch/commit must not install a partial second decision.
            tx.commit().unwrap();
            error.code
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        first.commit().unwrap();
        assert_eq!(second.join().unwrap(), ErrorCode::Conflict);
        let status = topology_handoff::current(&db, &ns, &id).unwrap().unwrap();
        assert_eq!(status.attachment, Some(a.clone()));
        assert_eq!(status.creation, Some(a.created.clone()));
        if cancel_first {
            assert_eq!(status.state, BootstrapState::Cancelled);
            assert!(handoff::current(&db, &a.handoff).unwrap().is_none());
            let tx = db.transaction().unwrap();
            assert_eq!(
                recover(&tx, &ns, &request, 501, UtcMillis(104), None).unwrap(),
                *status.recovery.unwrap()
            );
            assert!(
                handoff::import_hint(
                    &tx,
                    &a.handoff,
                    a.handoff.thread.as_ref(),
                    "delayed",
                    UtcMillis(104)
                )
                .is_err()
            );
            tx.commit().unwrap();
        } else {
            assert_eq!(status.state, BootstrapState::Attached);
            assert!(status.recovery.is_none());
            let live = handoff::current(&db, &a.handoff).unwrap().unwrap();
            assert_eq!(live.state, HandoffState::Live);
            let tx = db.transaction().unwrap();
            assert_eq!(
                handoff::import_hint(
                    &tx,
                    &a.handoff,
                    a.handoff.thread.as_ref(),
                    "delayed",
                    UtcMillis(104)
                )
                .unwrap(),
                live
            );
            tx.commit().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn topology_composition_late_cancel_and_begin_failures_preserve_opposite_decision() {
    for fail_cancel in [true, false] {
        let (_context, mut db, path, store, _observation, _budget, id, a) = real_attached();
        let ns = &id.payload.handoff.namespace;
        let request = cancellation_request(&id, &a);
        if fail_cancel {
            db.execute_batch("CREATE TEMP TRIGGER fail_cancel BEFORE UPDATE OF state ON bootstrap_handoffs WHEN NEW.state='cancelled' BEGIN SELECT RAISE(ABORT,'cancel late fixture failure'); END").unwrap();
        } else {
            db.execute_batch("CREATE TEMP TRIGGER fail_begin AFTER INSERT ON channel_handoff_fences BEGIN SELECT RAISE(ABORT,'begin late fixture failure'); END").unwrap();
        }
        let tx = db.transaction().unwrap();
        if fail_cancel {
            assert!(recover(&tx, ns, &request, 501, UtcMillis(102), None).is_err());
        } else {
            assert!(handoff::begin_linked_pending(&tx, ns, &a.handoff, UtcMillis(102)).is_err());
        }
        tx.commit().unwrap();
        let status = topology_handoff::current(&db, ns, &id).unwrap().unwrap();
        assert_eq!(status.state, BootstrapState::Attached);
        assert_eq!(status.attachment, Some(a.clone()));
        assert!(status.recovery.is_none());
        assert!(handoff::current(&db, &a.handoff).unwrap().is_none());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM bootstrap_recovery_decisions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let tx = db.transaction().unwrap();
        if fail_cancel {
            assert_eq!(
                handoff::begin_linked_pending(&tx, ns, &a.handoff, UtcMillis(103))
                    .unwrap()
                    .state,
                HandoffState::Live
            );
        } else {
            assert_eq!(
                recover(&tx, ns, &request, 501, UtcMillis(103), None)
                    .unwrap()
                    .state,
                BootstrapState::Cancelled
            );
        }
        tx.commit().unwrap();
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn topology_composition_linked_begin_requires_current_original_authority_and_namespace() {
    for changed in ["binding", "membership", "archive", "namespace"] {
        let (_context, mut db, path, store, _observation, _budget, id, a) = real_attached();
        match changed {
            "binding" => {
                db.execute("UPDATE occupant_bindings SET native_session='replacement' WHERE seat_id='sender'", []).unwrap();
            }
            "membership" => {
                db.execute(
                    "UPDATE memberships SET state='left' WHERE seat_id='sender'",
                    [],
                )
                .unwrap();
            }
            "archive" => {
                db.execute("UPDATE threads SET archived=1 WHERE id='thread'", [])
                    .unwrap();
            }
            _ => {}
        }
        let mut ns = id.payload.handoff.namespace.clone();
        if changed == "namespace" {
            ns.state_dir = "/foreign-state".into();
        }
        let tx = db.transaction().unwrap();
        assert!(
            handoff::begin_linked_pending(&tx, &ns, &a.handoff, UtcMillis(102)).is_err(),
            "{changed}"
        );
        tx.commit().unwrap();
        assert!(handoff::current(&db, &a.handoff).unwrap().is_none());
        let status = topology_handoff::current(&db, &id.payload.handoff.namespace, &id)
            .unwrap()
            .unwrap();
        assert_eq!(status.state, BootstrapState::Attached);
        assert_eq!(status.attachment, Some(a));
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn topology_composition_confirmed_creation_never_bypasses_fresh_restore_or_owner_guards() {
    for changed in ["hold", "stale", "owner"] {
        let (_context, mut db, path, store, observation, budget, id, a) =
            attachment_fixture_with_recording(true);
        let guard = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 3);
        match changed {
            "hold" => {
                db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','w1:p2',?1,1,'held')", [a.created.host_incarnation.as_str()]).unwrap();
            }
            "stale" => {
                let _ = attachment_guard(&store, &observation, &budget, &a.resolve_operation, 4);
            }
            "owner" => {
                db.execute(
                    "UPDATE seats SET target_id='w1:p9' WHERE id=?1",
                    [a.resolved_seat.as_str()],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let tx = db.transaction().unwrap();
        assert!(
            topology_handoff::attach_pending(
                &tx,
                &id.payload.handoff.namespace,
                &AttachBootstrapHandoff {
                    identity: id.clone(),
                    operation: id.payload.attach_key.clone(),
                    attachment: a.clone()
                },
                &guard
            )
            .is_err(),
            "{changed}"
        );
        tx.commit().unwrap();
        let status = topology_handoff::current(&db, &id.payload.handoff.namespace, &id)
            .unwrap()
            .unwrap();
        assert_eq!(status.state, BootstrapState::Created);
        assert_eq!(status.creation, Some(a.created));
        assert!(status.attachment.is_none());
        assert_eq!(
            db.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(handoff::current(&db, &a.handoff).unwrap().is_none());
        drop(store);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
