use super::{created, fixture, identity, namespace};
use herdr_threads::{
    ports::CreateTabOutcome,
    protocol::{
        handoff::*,
        results::{ApiError, ErrorCode},
        time::UtcMillis,
    },
    store::topology_handoff::{self, attempts::*},
};
fn reserve(id: &BootstrapIdentity, n: u32) -> ReserveBootstrapAttempt {
    let attempt = BootstrapAttempt::new(n).unwrap();
    ReserveBootstrapAttempt {
        identity: id.clone(),
        operation: attempt.operation(&id.compound, "reserve").unwrap(),
        expected_attempt: attempt,
    }
}
fn decision(
    id: &BootstrapIdentity,
    n: u32,
    disposition: BootstrapRecoveryDisposition,
) -> RecoverBootstrap {
    let mut d = RecoverBootstrap {
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::new(n).unwrap(),
        operation: id.compound.clone(),
        disposition,
    };
    d.operation = d.decision_operation().unwrap();
    d
}
fn not_created() -> BootstrapRecoveryDisposition {
    BootstrapRecoveryDisposition::NotCreated {
        quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
    }
}
fn cancel() -> BootstrapRecoveryDisposition {
    BootstrapRecoveryDisposition::Cancelled {
        reason: "inspected abandoned bootstrap".into(),
        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
        child_guard: BootstrapCancellationGuard {
            attached_child: None,
        },
    }
}
fn error() -> ApiError {
    ApiError::new(ErrorCode::Unsupported, "transport fixture")
}
#[test]
fn reserve_is_one_use_and_fresh_submission_checks_a2() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    let auth = reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    assert!(matches!(auth, ReserveBootstrapResult::Authorized { .. }));
    assert!(matches!(
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap(),
        ReserveBootstrapResult::Replay { .. }
    ));
    let check = CheckBootstrapSubmission {
        identity: id.clone(),
        operation: BootstrapAttempt::first()
            .operation(&id.compound, "check")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
        expected_administrative_revision: 0,
    };
    check_submission(&tx, &namespace(), &check).unwrap();
    tx.execute_batch("UPDATE occupant_bindings SET native_session='changed'")
        .unwrap();
    assert_eq!(
        check_submission(&tx, &namespace(), &check)
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
}
#[test]
fn unknown_creation_never_reauthorizes_and_proven_zero_submission_advances_once() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    let unknown = record_outcome(
        &tx,
        &namespace(),
        &id,
        BootstrapAttempt::first(),
        &CreateTabOutcome::OutcomeUnknown(error()),
    )
    .unwrap();
    assert_eq!(unknown.attempt_state, BootstrapAttemptState::OutcomeUnknown);
    assert!(matches!(
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap(),
        ReserveBootstrapResult::Replay { .. }
    ));
    // An already recorded unknown cannot be reclassified as transport zero.
    assert_eq!(
        record_outcome(
            &tx,
            &namespace(),
            &id,
            BootstrapAttempt::first(),
            &CreateTabOutcome::NotSubmitted(error())
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    recover(
        &tx,
        &namespace(),
        &decision(&id, 1, not_created()),
        501,
        UtcMillis(1),
        None,
    )
    .unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 2)).unwrap();
    let next = record_outcome(
        &tx,
        &namespace(),
        &id,
        BootstrapAttempt::new(2).unwrap(),
        &CreateTabOutcome::NotSubmitted(error()),
    )
    .unwrap();
    assert_eq!(next.attempt.get(), 3);
    assert_eq!(next.attempt_state, BootstrapAttemptState::Prepared);
    let replay = record_outcome(
        &tx,
        &namespace(),
        &id,
        BootstrapAttempt::new(2).unwrap(),
        &CreateTabOutcome::NotSubmitted(error()),
    )
    .unwrap();
    assert_eq!(replay.attempt.get(), 3);
}
#[test]
fn exact_created_replay_and_contradictory_or_late_evidence_refuse() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    let mut record = RecordBootstrapCreated {
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: BootstrapAttempt::first()
            .operation(&id.compound, "record")
            .unwrap(),
        evidence: created(),
    };
    let saved = record_created(&tx, &namespace(), &record).unwrap();
    assert_eq!(saved.state, BootstrapState::Created);
    assert_eq!(record_created(&tx, &namespace(), &record).unwrap(), saved);
    record.evidence.terminal = herdr_threads::protocol::ids::TerminalId::new("different");
    assert_eq!(
        record_created(&tx, &namespace(), &record).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
}
#[test]
fn recovery_old_decision_is_historical_and_stale_undecided_refuses() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    let d = decision(&id, 1, not_created());
    let saved = recover(&tx, &namespace(), &d, 501, UtcMillis(5), None).unwrap();
    assert_eq!(saved.state, BootstrapState::Prepared);
    assert_eq!(saved.operator_provenance, "operator:local-user:501");
    reserve_attempt(&tx, &namespace(), &reserve(&id, 2)).unwrap();
    assert_eq!(
        recover(&tx, &namespace(), &d, 501, UtcMillis(6), None).unwrap(),
        saved
    );
    assert_eq!(
        recover(
            &tx,
            &namespace(),
            &decision(&id, 1, cancel()),
            501,
            UtcMillis(7),
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    let record = RecordBootstrapCreated {
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: BootstrapAttempt::first()
            .operation(&id.compound, "record")
            .unwrap(),
        evidence: created(),
    };
    assert_eq!(
        record_created(&tx, &namespace(), &record).unwrap_err().code,
        ErrorCode::Conflict
    );
}
#[test]
fn cancellation_is_absorbing_and_requires_canonical_no_live_child() {
    for live in [false, true] {
        let mut db = fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        if live {
            tx.execute("INSERT INTO channel_handoff_fences(instance_id,actor_scope,compound,digest,claim_json,recipient,create_key,invite_key,send_key,original_thread,thread_id,origin,state,created_at) VALUES('i','seat:s','handoff',?1,'{}','child','create','invite','send','t','t','legacy_local_journal_hint','live',0)",["a".repeat(64)]).unwrap();
        }
        let result = recover(
            &tx,
            &namespace(),
            &decision(&id, 1, cancel()),
            501,
            UtcMillis(5),
            None,
        );
        if live {
            assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        } else {
            assert_eq!(result.unwrap().state, BootstrapState::Cancelled);
            assert_eq!(
                reserve_attempt(&tx, &namespace(), &reserve(&id, 1))
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                topology_handoff::current(&tx, &namespace(), &id)
                    .unwrap()
                    .unwrap()
                    .state,
                BootstrapState::Cancelled
            );
        }
    }
}

#[test]
fn historical_recovery_replay_refuses_corrupt_decision_snapshot() {
    let mut db = fixture();
    let id = identity();
    let old = decision(&id, 1, not_created());
    let mut saved = {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        let saved = recover(&tx, &namespace(), &old, 501, UtcMillis(1), None).unwrap();
        recover(
            &tx,
            &namespace(),
            &decision(&id, 2, not_created()),
            501,
            UtcMillis(2),
            None,
        )
        .unwrap();
        tx.commit().unwrap();
        saved
    };
    saved.state = BootstrapState::Cancelled;
    db.execute_batch("DROP TRIGGER bootstrap_recovery_immutable")
        .unwrap();
    db.execute(
        "UPDATE bootstrap_recovery_decisions SET result_json=?1 WHERE operation=?2",
        rusqlite::params![serde_json::to_vec(&saved).unwrap(), old.operation.as_str()],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        recover(&tx, &namespace(), &old, 501, UtcMillis(3), None)
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .attempt
            .get(),
        3
    );
}

#[test]
fn concurrent_connections_and_lost_reserve_reply_reopen_never_authorize_twice() {
    use std::sync::{Arc, Barrier};
    let (_dir, path, mut db) = super::file_fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    drop(db);
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = rusqlite::Connection::open(path).unwrap();
                db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                barrier.wait();
                let tx = db
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                let authorized = matches!(
                    reserve_attempt(&tx, &namespace(), &reserve(&identity(), 1)).unwrap(),
                    ReserveBootstrapResult::Authorized { .. }
                );
                tx.commit().unwrap();
                authorized
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| usize::from(w.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let mut db = rusqlite::Connection::open(&path).unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap(),
        ReserveBootstrapResult::Replay { .. }
    ));
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .attempt_state,
        BootstrapAttemptState::PossibleCreation
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn late_next_attempt_insert_failure_rolls_back_even_if_caller_commits_error() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    tx.execute_batch("CREATE TEMP TRIGGER late_attempt_refusal AFTER INSERT ON bootstrap_attempts WHEN NEW.attempt=2 BEGIN SELECT RAISE(ABORT,'late refusal'); END").unwrap();
    assert!(
        record_outcome(
            &tx,
            &namespace(),
            &id,
            BootstrapAttempt::first(),
            &CreateTabOutcome::NotSubmitted(error())
        )
        .is_err()
    );
    assert!(
        recover(
            &tx,
            &namespace(),
            &decision(&id, 1, not_created()),
            501,
            UtcMillis(1),
            None
        )
        .is_err()
    );
    tx.commit().unwrap();
    let result = topology_handoff::current(&db, &namespace(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(result.attempt.get(), 1);
    assert_eq!(
        result.attempt_state,
        BootstrapAttemptState::PossibleCreation
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_child_keys", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        14
    );
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

#[test]
fn stale_undecided_attempt_and_changed_phase_or_revision_refuse_without_effects() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    record_outcome(
        &tx,
        &namespace(),
        &id,
        BootstrapAttempt::first(),
        &CreateTabOutcome::NotSubmitted(error()),
    )
    .unwrap();
    assert_eq!(
        recover(
            &tx,
            &namespace(),
            &decision(&id, 1, not_created()),
            501,
            UtcMillis(1),
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    let mut wrong = reserve(&id, 2);
    wrong.operation = herdr_threads::protocol::ids::OperationId::new("wrong");
    assert_eq!(
        reserve_attempt(&tx, &namespace(), &wrong).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    reserve_attempt(&tx, &namespace(), &reserve(&id, 2)).unwrap();
    let check = CheckBootstrapSubmission {
        identity: id.clone(),
        operation: BootstrapAttempt::new(2)
            .unwrap()
            .operation(&id.compound, "check")
            .unwrap(),
        expected_attempt: BootstrapAttempt::new(2).unwrap(),
        expected_administrative_revision: 1,
    };
    assert_eq!(
        check_submission(&tx, &namespace(), &check)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert!(BootstrapAttempt::new(0).is_err());
}

#[test]
fn cancellation_retains_attached_identity_and_created_evidence_after_reopen() {
    let (_dir, path, mut db) = super::file_fixture();
    let id = identity();
    let attachment = BootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: created(),
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: herdr_threads::protocol::ids::SeatId::new("peer"),
        handoff: HandoffIdentity {
            compound: id.payload.handoff_key.clone(),
            digest: "b".repeat(64),
            claim: id.claim.clone(),
            thread: Some(herdr_threads::protocol::ids::ThreadId::new("t")),
            recipient: herdr_threads::protocol::ids::SeatId::new("peer"),
            create_key: id.payload.handoff.keys.create.clone(),
            invite_key: id.payload.handoff.keys.invite.clone(),
            send_key: id.payload.handoff.keys.send.clone(),
        },
    };
    let d = decision(
        &id,
        1,
        BootstrapRecoveryDisposition::Cancelled {
            reason: "confirmed lost pane; child never begun".into(),
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: Some(attachment.handoff.clone()),
            },
        },
    );
    let saved = {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
        record_created(
            &tx,
            &namespace(),
            &RecordBootstrapCreated {
                identity: id.clone(),
                operation: BootstrapAttempt::first()
                    .operation(&id.compound, "record")
                    .unwrap(),
                expected_attempt: BootstrapAttempt::first(),
                evidence: created(),
            },
        )
        .unwrap();
        tx.execute(
            "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
            [serde_json::to_vec(&attachment).unwrap()],
        )
        .unwrap();
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='attached' WHERE id=1",
            [],
        )
        .unwrap();
        assert_eq!(
            recover(
                &tx,
                &namespace(),
                &decision(&id, 1, cancel()),
                501,
                UtcMillis(1),
                None
            )
            .unwrap_err()
            .code,
            ErrorCode::OperationPayloadMismatch
        );
        let saved = recover(&tx, &namespace(), &d, 501, UtcMillis(1), None).unwrap();
        tx.commit().unwrap();
        saved
    };
    drop(db);
    let mut db = rusqlite::Connection::open(path).unwrap();
    let tx = db.transaction().unwrap();
    let status = topology_handoff::current(&tx, &namespace(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(status.state, BootstrapState::Cancelled);
    assert_eq!(status.attachment, Some(attachment));
    assert_eq!(status.creation, Some(created()));
    tx.execute_batch("UPDATE occupant_bindings SET native_session='changed'")
        .unwrap();
    assert_eq!(
        recover(&tx, &namespace(), &d, 501, UtcMillis(4), None).unwrap(),
        saved
    );
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(4))
            .unwrap()
            .state,
        BootstrapState::Cancelled
    );
}

#[test]
fn cancellation_invalid_reason_and_created_without_structural_guard_refuse() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    for reason in [" ".into(), "é".repeat(2049)] {
        let d = decision(
            &id,
            1,
            BootstrapRecoveryDisposition::Cancelled {
                reason,
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                child_guard: BootstrapCancellationGuard {
                    attached_child: None,
                },
            },
        );
        assert_eq!(
            recover(&tx, &namespace(), &d, 501, UtcMillis(1), None)
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let d = decision(
        &id,
        1,
        BootstrapRecoveryDisposition::CreatedPane {
            evidence: created(),
            structural_reference: herdr_threads::protocol::ids::HostCallId::new("fresh"),
        },
    );
    assert_eq!(
        recover(&tx, &namespace(), &d, 501, UtcMillis(1), None)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::PossibleCreation
    );
}

#[test]
fn recorded_creation_lost_reply_reopens_exact_without_new_authorization() {
    let (_dir, path, mut db) = super::file_fixture();
    let id = identity();
    let request = RecordBootstrapCreated {
        identity: id.clone(),
        operation: BootstrapAttempt::first()
            .operation(&id.compound, "record")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
        evidence: created(),
    };
    {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
        record_created(&tx, &namespace(), &request).unwrap();
        tx.commit().unwrap();
    }
    drop(db);
    let mut db = rusqlite::Connection::open(path).unwrap();
    let tx = db.transaction().unwrap();
    let status = topology_handoff::current(&tx, &namespace(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(status.state, BootstrapState::Created);
    assert_eq!(status.creation, Some(created()));
    assert_eq!(record_created(&tx, &namespace(), &request).unwrap(), status);
    assert!(matches!(
        reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap(),
        ReserveBootstrapResult::Replay { .. }
    ));
    assert_eq!(
        record_outcome(
            &tx,
            &namespace(),
            &id,
            BootstrapAttempt::first(),
            &CreateTabOutcome::NotSubmitted(error())
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
}

#[test]
fn cancellation_serializes_with_underlying_live_child_insertion() {
    // This exercises the actual persisted linked fence insertion, not the later
    // public Begin route owned by attachment/integration leaves.
    use std::sync::mpsc;
    let (_dir, path, mut db) = super::file_fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    drop(db);
    let (ready_rx, ready_tx) = (mpsc::channel::<()>(), mpsc::channel::<()>());
    let child_path = path.clone();
    let child = std::thread::spawn(move || {
        let mut db = rusqlite::Connection::open(child_path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("INSERT INTO channel_handoff_fences(instance_id,actor_scope,compound,digest,claim_json,recipient,create_key,invite_key,send_key,original_thread,thread_id,origin,state,created_at) VALUES('i','seat:s','handoff',?1,'{}','peer','create','invite','send','t','t','cooperative_pending_claim','live',0)",["b".repeat(64)]).unwrap();
        ready_rx.0.send(()).unwrap();
        ready_tx.1.recv().unwrap();
        tx.commit().unwrap();
    });
    ready_rx.1.recv().unwrap();
    let cancel_path = path.clone();
    let cancel = std::thread::spawn(move || {
        let mut db = rusqlite::Connection::open(cancel_path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let error = recover(
            &tx,
            &namespace(),
            &decision(&identity(), 1, self::cancel()),
            501,
            UtcMillis(1),
            None,
        )
        .unwrap_err();
        tx.commit().unwrap();
        error.code
    });
    ready_tx.0.send(()).unwrap();
    child.join().unwrap();
    assert_eq!(cancel.join().unwrap(), ErrorCode::Conflict);
    let db = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::Prepared
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM channel_handoff_fences WHERE state='live'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn recovery_registry_missing_or_cross_role_key_refuses_current_and_historical_replay() {
    for tamper in [
        "DROP TRIGGER bootstrap_keys_retained; DELETE FROM bootstrap_child_keys WHERE role='recovery'",
        "DROP TRIGGER bootstrap_keys_immutable; UPDATE bootstrap_child_keys SET operation_key='wrong' WHERE role='recovery'",
    ] {
        let mut db = fixture();
        let id = identity();
        let d = decision(&id, 1, not_created());
        {
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
            recover(&tx, &namespace(), &d, 501, UtcMillis(1), None).unwrap();
            tx.commit().unwrap();
        }
        db.execute_batch(tamper).unwrap();
        assert_eq!(
            topology_handoff::current(&db, &namespace(), &id)
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
        let tx = db.transaction().unwrap();
        assert_eq!(
            recover(&tx, &namespace(), &d, 501, UtcMillis(2), None)
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }
}

#[test]
fn decision_kind_namespace_attempt_and_excess_slots_are_retained_corruption() {
    for tamper in [
        "DROP TRIGGER bootstrap_recovery_immutable; UPDATE bootstrap_recovery_decisions SET decision_kind='cancellation'",
        "PRAGMA foreign_keys=OFF; DROP TRIGGER bootstrap_keys_immutable; UPDATE bootstrap_child_keys SET state_dir='/foreign' WHERE role='recovery'",
        "DROP TRIGGER bootstrap_keys_immutable; UPDATE bootstrap_child_keys SET attempt=2 WHERE role='recovery'",
        "PRAGMA ignore_check_constraints=ON; DROP TRIGGER bootstrap_recovery_key; INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,decision_kind,result_json) VALUES(1,1,'extra1','unknown1',X'7b7d'),(1,1,'extra2','unknown2',X'7b7d')",
    ] {
        let mut db = fixture();
        let id = identity();
        let d = decision(&id, 1, not_created());
        {
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
            recover(&tx, &namespace(), &d, 501, UtcMillis(1), None).unwrap();
            tx.commit().unwrap();
        }
        db.execute_batch(tamper).unwrap();
        assert_eq!(
            topology_handoff::current(&db, &namespace(), &id)
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }
}

#[test]
fn administrative_slots_and_registry_use_parent_attempt_indexes() {
    let db = fixture();
    for sql in [
        "EXPLAIN QUERY PLAN SELECT decision_kind,operation,result_json FROM bootstrap_recovery_decisions WHERE parent_id=1 AND attempt=1 LIMIT 3",
        "EXPLAIN QUERY PLAN SELECT role,operation_key FROM bootstrap_child_keys INDEXED BY bootstrap_keys_attempt WHERE parent_id=1 AND attempt=1 AND role IN ('recovery','cancel') LIMIT 3",
    ] {
        let plans = db
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plans
                .iter()
                .any(|p| p.contains("parent_id=? AND attempt=?")),
            "bounded exact-attempt query must use both leading keys: {plans:?}"
        );
        assert!(!plans.iter().any(|p| p.contains("SCAN ")));
    }
}

#[test]
fn begin_refuses_future_non_submission_key_colliding_with_frozen_invite() {
    let mut db = fixture();
    let mut id = identity();
    id.payload.handoff.keys.invite = herdr_threads::protocol::ids::OperationId::new(
        "bootstrap-727e596c7ec2f6663964eb23a37da9ed6d72a1fb7cca0be103cc9d5822cf9233",
    );
    id.digest = id.semantic_digest().unwrap();
    id.validate().unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn unknown_transport_outcome_cannot_be_later_reclassified_as_not_submitted() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    record_outcome(
        &tx,
        &namespace(),
        &id,
        BootstrapAttempt::first(),
        &CreateTabOutcome::OutcomeUnknown(error()),
    )
    .unwrap();
    assert_eq!(
        record_outcome(
            &tx,
            &namespace(),
            &id,
            BootstrapAttempt::first(),
            &CreateTabOutcome::NotSubmitted(error())
        )
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
}

fn zero_submission(id: &BootstrapIdentity, n: u32) -> RecordBootstrapNotSubmitted {
    let attempt = BootstrapAttempt::new(n).unwrap();
    RecordBootstrapNotSubmitted {
        identity: id.clone(),
        operation: attempt.operation(&id.compound, "not_submitted").unwrap(),
        expected_attempt: attempt,
    }
}

#[test]
fn typed_zero_submission_lost_reply_reopens_and_old_replay_never_advances_again() {
    let (_dir, path, mut db) = super::file_fixture();
    let id = identity();
    let request = zero_submission(&id, 1);
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
    let saved = record_not_submitted(&tx, &namespace(), &request).unwrap();
    assert_eq!(saved.attempt.get(), 2);
    assert_eq!(saved.attempt_state, BootstrapAttemptState::Prepared);
    tx.commit().unwrap();
    drop(db);
    let mut db = rusqlite::Connection::open(path).unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        record_not_submitted(&tx, &namespace(), &request).unwrap(),
        saved
    );
    reserve_attempt(&tx, &namespace(), &reserve(&id, 2)).unwrap();
    let current = record_not_submitted(&tx, &namespace(), &request).unwrap();
    assert_eq!(current.attempt.get(), 2);
    assert_eq!(
        current.attempt_state,
        BootstrapAttemptState::PossibleCreation
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    tx.execute_batch("UPDATE occupant_bindings SET native_session='changed'")
        .unwrap();
    assert_eq!(
        record_not_submitted(&tx, &namespace(), &request)
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
}

#[test]
fn typed_zero_submission_refuses_wrong_key_attempt_creation_unknown_and_terminal() {
    for outcome in ["prepared", "created", "unknown", "cancelled"] {
        let mut db = fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        let mut wrong = zero_submission(&id, 1);
        wrong.operation = id.compound.clone();
        assert_eq!(
            record_not_submitted(&tx, &namespace(), &wrong)
                .unwrap_err()
                .code,
            ErrorCode::OperationPayloadMismatch
        );
        assert_eq!(
            record_not_submitted(&tx, &namespace(), &zero_submission(&id, 2))
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        if outcome != "prepared" {
            reserve_attempt(&tx, &namespace(), &reserve(&id, 1)).unwrap();
        }
        match outcome {
            "created" => {
                let record = RecordBootstrapCreated {
                    identity: id.clone(),
                    expected_attempt: BootstrapAttempt::first(),
                    operation: BootstrapAttempt::first()
                        .operation(&id.compound, "record")
                        .unwrap(),
                    evidence: created(),
                };
                record_created(&tx, &namespace(), &record).unwrap();
            }
            "unknown" => {
                record_outcome(
                    &tx,
                    &namespace(),
                    &id,
                    BootstrapAttempt::first(),
                    &CreateTabOutcome::OutcomeUnknown(error()),
                )
                .unwrap();
            }
            "cancelled" => {
                recover(
                    &tx,
                    &namespace(),
                    &decision(&id, 1, cancel()),
                    501,
                    UtcMillis(1),
                    None,
                )
                .unwrap();
            }
            _ => {}
        }
        assert_eq!(
            record_not_submitted(&tx, &namespace(), &zero_submission(&id, 1))
                .unwrap_err()
                .code,
            ErrorCode::Conflict,
            "{outcome}"
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
