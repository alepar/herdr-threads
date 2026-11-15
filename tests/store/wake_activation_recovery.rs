//! Activation recovery keeps logical purpose, helper resources and queue effects separate.
use super::wake_intents::{budget, fixture, native_report, prepare, prepare_raw, reserve_again};
use herdr_threads::{
    harness::{
        adapter::TurnEdge,
        wake::{WakeHelperPresence, WakeHelperPurpose, WakeHelperQuiescence},
    },
    ports::{
        PrepareWakeIntent, StorePort, WakeArmTicket, WakeHelperOperation, WakeIntentLogical,
        WakeIntentOutcome, WakeIntentPhase, WakeIntentPreparation, WakeIntentResult,
        WakeIntentSnapshot, WakeReservation,
    },
    store::SqliteStore,
};

fn row(store: &SqliteStore, r: &WakeReservation) -> WakeIntentSnapshot {
    StorePort::wake_intent(store, &r.attempt, &budget())
        .unwrap()
        .unwrap()
}
fn arm(store: &SqliteStore, r: &WakeReservation, p: &WakeIntentPreparation) -> WakeArmTicket {
    let prepared = prepare(store, r, p);
    StorePort::arm_wake_intent(store, r, prepared.revision, uuid::Uuid::new_v4(), &budget())
        .unwrap()
        .unwrap()
}
fn result(store: &SqliteStore, ticket: &WakeArmTicket, outcome: WakeIntentOutcome, clean: bool) {
    assert!(
        StorePort::record_wake_intent_result(
            store,
            ticket,
            &WakeIntentResult {
                outcome,
                cleanup_confirmed: clean,
                diagnostic: String::new(),
            },
            &budget()
        )
        .unwrap()
    );
}
fn op(
    r: &WakeReservation,
    p: &WakeIntentPreparation,
    purpose: WakeHelperPurpose,
    subject: Option<&str>,
) -> WakeHelperOperation {
    WakeHelperOperation {
        id: uuid::Uuid::new_v4(),
        purpose,
        subject: subject.map(str::to_owned),
        owner_boot: r.daemon_boot,
        kernel_boot: p.target.scope.kernel_boot.clone(),
        witness: None,
    }
}
fn begin(store: &SqliteStore, r: &WakeReservation, operation: &WakeHelperOperation) -> bool {
    StorePort::begin_wake_helper(
        store,
        &r.attempt,
        row(store, r).revision,
        operation,
        &budget(),
    )
    .unwrap()
}
fn complete(store: &SqliteStore, r: &WakeReservation, operation: &WakeHelperOperation) {
    assert!(
        StorePort::record_wake_helper_completion(store, &r.attempt, operation.id, true, &budget())
            .unwrap()
    );
}
fn fulfill(store: &SqliteStore, r: &WakeReservation, p: &WakeIntentPreparation) {
    StorePort::record_native_turn(
        store,
        &native_report(store, r, p, "activation-new-start", TurnEdge::Busy),
        &budget(),
    )
    .unwrap();
    assert!(matches!(
        row(store, r).logical,
        WakeIntentLogical::Fulfilled { .. }
    ));
}
fn assert_blocked(store: &SqliteStore, r: &WakeReservation, p: &WakeIntentPreparation) {
    let blockers =
        StorePort::wake_intent_blockers(store, &r.seat, p.adapter, &p.replay_scope, &budget())
            .unwrap();
    assert!(blockers.programmatic && blockers.native, "{blockers:?}");
}

#[test]
fn activation_recovery_active_unknown_survives_helper_quiescence_and_list_absence() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    result(&store, &ticket, WakeIntentOutcome::Unknown, false);
    let helper = row(&store, &r).helper.unwrap();
    assert!(
        StorePort::reconcile_wake_helper(
            &store,
            &r.attempt,
            helper.id,
            &WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent),
            &budget()
        )
        .unwrap()
    );
    let before_list = row(&store, &r);
    assert!(!before_list.cleanup_quarantined);
    assert!(
        StorePort::record_owned_wake_ids(
            &store,
            &r.attempt,
            before_list.revision,
            &[],
            true,
            &budget()
        )
        .unwrap()
    );
    let after = row(&store, &r);
    assert!(after.ids_complete && after.owned_ids.is_empty());
    assert_eq!(after.phase, WakeIntentPhase::Unknown);
    assert_eq!(after.logical, WakeIntentLogical::Active);
    assert_blocked(&store, &r, &p);
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, None)
    ));
    assert!(
        !StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            helper.id,
            true,
            None,
            &budget()
        )
        .unwrap()
    );
    assert!(
        !StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            helper.id,
            false,
            Some("absent-owned-id"),
            &budget()
        )
        .unwrap()
    );
    assert_eq!(row(&store, &r).logical, WakeIntentLogical::Active);
    assert_blocked(&store, &r, &p);
}

#[test]
fn activation_recovery_wrong_operation_or_nonce_cannot_release_fences() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    let helper = row(&store, &r).helper.unwrap();
    let wrong = uuid::Uuid::new_v4();
    assert!(
        !StorePort::record_wake_helper_completion(&store, &r.attempt, wrong, true, &budget())
            .unwrap()
    );
    assert!(
        !StorePort::reconcile_wake_helper(
            &store,
            &r.attempt,
            wrong,
            &WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent),
            &budget()
        )
        .unwrap()
    );
    let mut stale = ticket.clone();
    stale.nonce = wrong;
    assert!(
        !StorePort::record_wake_intent_result(
            &store,
            &stale,
            &WakeIntentResult {
                outcome: WakeIntentOutcome::NoEffect,
                cleanup_confirmed: true,
                diagnostic: String::new(),
            },
            &budget()
        )
        .unwrap()
    );
    let after = row(&store, &r);
    assert_eq!(after.phase, WakeIntentPhase::Armed);
    assert_eq!(after.helper.unwrap().id, helper.id);
    assert!(after.cleanup_quarantined);
    assert_blocked(&store, &r, &p);
}

#[test]
fn activation_recovery_disconnect_and_missing_host_observations_do_not_end_scope() {
    for change in [
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=101,disconnect_reason='liveness_end_event'",
        "DELETE FROM observed_targets; UPDATE herdr_hosts SET host_boot='unobserved-host'",
        "UPDATE occupant_bindings SET liveness_pid=NULL,liveness_start=NULL,liveness_boot=NULL",
    ] {
        let (store, _path, r, p) = fixture();
        let ticket = arm(&store, &r, &p);
        result(&store, &ticket, WakeIntentOutcome::Unknown, true);
        store
            .context()
            .open_writer()
            .unwrap()
            .execute_batch(change)
            .unwrap();
        assert!(
            !StorePort::retire_wake_intent_if_scope_ended(&store, &r.attempt, &budget()).unwrap(),
            "{change}"
        );
        assert_eq!(row(&store, &r).logical, WakeIntentLogical::Active);
        assert_blocked(&store, &r, &p);
    }
}

#[test]
fn activation_recovery_canonical_binding_or_mapping_finality_retires_scope() {
    for change in [
        "UPDATE occupant_bindings SET ended_at=101",
        "UPDATE occupant_bindings SET native_session='replacement-session'",
        "UPDATE occupant_bindings SET execution_id='replacement-execution'",
        "UPDATE seats SET target_id='replacement-pane'",
        "UPDATE seats SET target_generation=target_generation+1",
    ] {
        let (store, _path, r, p) = fixture();
        let ticket = arm(&store, &r, &p);
        result(&store, &ticket, WakeIntentOutcome::Unknown, true);
        store
            .context()
            .open_writer()
            .unwrap()
            .execute_batch(change)
            .unwrap();
        assert!(
            StorePort::retire_wake_intent_if_scope_ended(&store, &r.attempt, &budget()).unwrap(),
            "{change}"
        );
        let after = row(&store, &r);
        assert_eq!(after.logical, WakeIntentLogical::ScopeEnded);
        assert_eq!(after.phase, WakeIntentPhase::Unknown);
        let blockers =
            StorePort::wake_intent_blockers(&store, &r.seat, p.adapter, &p.replay_scope, &budget())
                .unwrap();
        assert!(!blockers.programmatic && !blockers.native);
    }
}

#[test]
fn activation_recovery_unwitnessed_spawn_pending_needs_actual_kernel_boot_change() {
    let (store, _path, r, p) = fixture();
    prepare_raw(&store, &r, &p);
    let helper = op(&r, &p, WakeHelperPurpose::Add, None);
    assert!(begin(&store, &r, &helper));
    for presence in [
        WakeHelperPresence::Unknown,
        WakeHelperPresence::Live,
        WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent),
        WakeHelperPresence::Quiescent(WakeHelperQuiescence::BootChanged),
    ] {
        assert!(
            !StorePort::reconcile_wake_helper(&store, &r.attempt, helper.id, &presence, &budget())
                .unwrap()
        );
        assert!(row(&store, &r).cleanup_quarantined);
    }
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE node_kernel_boot SET boot='new-kernel-boot'", [])
        .unwrap();
    assert!(
        StorePort::reconcile_wake_helper(
            &store,
            &r.attempt,
            helper.id,
            &WakeHelperPresence::Quiescent(WakeHelperQuiescence::BootChanged),
            &budget()
        )
        .unwrap()
    );
    assert!(!row(&store, &r).cleanup_quarantined);
    assert_blocked(&store, &r, &p);
}

#[test]
fn activation_recovery_cleanup_is_retired_clean_one_pass_and_exact_owned_subject() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    result(
        &store,
        &ticket,
        WakeIntentOutcome::Accepted {
            id: "owned-id".into(),
        },
        false,
    );
    let add = row(&store, &r).helper.unwrap();
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, None)
    ));
    fulfill(&store, &r, &p);
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, None)
    ));
    complete(&store, &r, &add);
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::Delete, Some("owned-id"))
    ));
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, Some("owned-id"))
    ));
    let listing = op(&r, &p, WakeHelperPurpose::List, None);
    assert!(begin(&store, &r, &listing));
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, None)
    ));
    complete(&store, &r, &listing);
    assert!(
        !StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            listing.id,
            false,
            Some("owned-id"),
            &budget()
        )
        .unwrap()
    );
    assert!(
        StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            listing.id,
            true,
            None,
            &budget()
        )
        .unwrap()
    );
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::List, None)
    ));
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::Delete, Some("unowned-id"))
    ));
    let deleting = op(&r, &p, WakeHelperPurpose::Delete, Some("owned-id"));
    assert!(begin(&store, &r, &deleting));
    complete(&store, &r, &deleting);
    assert!(
        !StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            deleting.id,
            false,
            Some("unowned-id"),
            &budget()
        )
        .unwrap()
    );
    assert!(
        !StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            deleting.id,
            true,
            None,
            &budget()
        )
        .unwrap()
    );
    assert!(
        StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            deleting.id,
            false,
            Some("owned-id"),
            &budget()
        )
        .unwrap()
    );
    assert!(!begin(
        &store,
        &r,
        &op(&r, &p, WakeHelperPurpose::Delete, Some("owned-id"))
    ));
    let after = row(&store, &r);
    assert!(after.cleanup_list_attempted && after.cleanup_pending_ids.is_empty());
    assert_eq!(after.owned_ids, ["owned-id"]);
    assert!(matches!(after.logical, WakeIntentLogical::Fulfilled { .. }));
    assert!(
        StorePort::wake_intent_recovery_page(&store, 0, 32, &budget())
            .unwrap()
            .items
            .iter()
            .any(|item| item.attempt == r.attempt),
        "retired clean rows still need node-wide recovery/GC visibility"
    );
}

#[test]
fn activation_recovery_gc_requires_finite_cleanup_pass_and_removed_wake_references() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    result(
        &store,
        &ticket,
        WakeIntentOutcome::Accepted {
            id: "owned-id".into(),
        },
        true,
    );
    fulfill(&store, &r, &p);
    let next = reserve_again(&store);
    prepare_raw(&store, &next, &p);
    assert!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .is_some(),
        "retirement and missing refs do not replace the finite cleanup pass"
    );
    assert!(
        !begin(&store, &r, &op(&r, &p, WakeHelperPurpose::List, None)),
        "cleanup defers while a different active intent occupies this cohort"
    );
    // Retire the new no-effect preparation so another legitimate prepare can run GC.
    let current = row(&store, &next);
    assert!(
        StorePort::refuse_prepared_wake_intent(
            &store,
            &next.attempt,
            current.revision,
            true,
            "no add",
            &budget()
        )
        .unwrap()
    );
    let listing = op(&r, &p, WakeHelperPurpose::List, None);
    assert!(begin(&store, &r, &listing));
    complete(&store, &r, &listing);
    assert!(
        StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            listing.id,
            true,
            None,
            &budget()
        )
        .unwrap()
    );
    let deleting = op(&r, &p, WakeHelperPurpose::Delete, Some("owned-id"));
    assert!(begin(&store, &r, &deleting));
    complete(&store, &r, &deleting);
    assert!(
        StorePort::record_wake_cleanup_progress(
            &store,
            &r.attempt,
            deleting.id,
            false,
            Some("owned-id"),
            &budget()
        )
        .unwrap()
    );
    let third = reserve_again(&store);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_work SET last_reservation_id=?1",
            [r.attempt.as_str()],
        )
        .unwrap();
    prepare_raw(&store, &third, &p);
    assert!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .is_some(),
        "finite cleanup does not permit GC while last wake-work reference remains"
    );
    let current = row(&store, &third);
    assert!(
        StorePort::refuse_prepared_wake_intent(
            &store,
            &third.attempt,
            current.revision,
            true,
            "no add",
            &budget()
        )
        .unwrap()
    );
    let fourth = reserve_again(&store);
    prepare_raw(&store, &fourth, &p);
    assert!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .is_none()
    );
}

#[test]
fn activation_recovery_armed_fulfillment_cannot_replace_helper_or_prune_late_result() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    let add = row(&store, &r).helper.unwrap();
    fulfill(&store, &r, &p);
    complete(&store, &r, &add);
    let next = reserve_again(&store);
    assert!(
        !begin(&store, &r, &op(&r, &p, WakeHelperPurpose::List, None)),
        "cleanup cannot overwrite the in-flight add's helper slot"
    );
    assert!(matches!(
        StorePort::prepare_wake_intent(&store, &next, &p, &budget()).unwrap(),
        PrepareWakeIntent::Prepared(_)
    ));
    let old = row(&store, &r);
    assert_eq!(old.phase, WakeIntentPhase::Armed);
    assert_eq!(old.helper.unwrap().id, add.id);
    assert!(!old.cleanup_list_attempted);
    result(
        &store,
        &ticket,
        WakeIntentOutcome::Accepted {
            id: "late-owned-id".into(),
        },
        true,
    );
    let committed = row(&store, &r);
    assert_eq!(committed.phase, WakeIntentPhase::Accepted);
    assert_eq!(committed.owned_ids, ["late-owned-id"]);
    assert_eq!(committed.cleanup_pending_ids, ["late-owned-id"]);
    assert!(!committed.cleanup_quarantined);
    assert!(matches!(
        committed.logical,
        WakeIntentLogical::Fulfilled { .. }
    ));
}

fn reconciled_add_with_late_cleanup_failure(report_completion: bool) {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    let add = row(&store, &r).helper.unwrap();
    assert!(
        StorePort::record_wake_helper_completion(&store, &r.attempt, add.id, false, &budget())
            .unwrap()
    );
    assert!(
        row(&store, &r).cleanup_quarantined,
        "a failure before positive evidence keeps the helper quarantined"
    );
    fulfill(&store, &r, &p);
    assert!(
        StorePort::reconcile_wake_helper(
            &store,
            &r.attempt,
            add.id,
            &WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent),
            &budget()
        )
        .unwrap()
    );
    assert!(!row(&store, &r).cleanup_quarantined);
    let next = reserve_again(&store);
    let prepared = prepare_raw(&store, &next, &p);
    assert_eq!(prepared.phase, WakeIntentPhase::Prepared);
    if report_completion {
        StorePort::record_wake_helper_completion(&store, &r.attempt, add.id, false, &budget())
            .expect("delayed failure cannot collide with a newly active cohort");
        assert!(
            !row(&store, &r).cleanup_quarantined,
            "positive evidence for this exact helper survives delayed failure"
        );
    }
    result(
        &store,
        &ticket,
        WakeIntentOutcome::Accepted {
            id: "late-accepted-after-positive-quiescence".into(),
        },
        false,
    );
    let committed = row(&store, &r);
    assert_eq!(committed.phase, WakeIntentPhase::Accepted);
    assert_eq!(committed.helper.unwrap().id, add.id);
    assert_eq!(
        committed.owned_ids,
        ["late-accepted-after-positive-quiescence"]
    );
    assert_eq!(
        committed.cleanup_pending_ids,
        ["late-accepted-after-positive-quiescence"]
    );
    assert!(!committed.cleanup_quarantined);
    assert!(matches!(
        committed.logical,
        WakeIntentLogical::Fulfilled { .. }
    ));
    let active = row(&store, &next);
    assert_eq!(active.ordinal, prepared.ordinal);
    assert_eq!(active.phase, WakeIntentPhase::Prepared);
    assert_eq!(active.logical, WakeIntentLogical::Active);
}

#[test]
fn activation_recovery_delayed_completion_failure_cannot_regress_positive_helper_evidence() {
    reconciled_add_with_late_cleanup_failure(true);
}

#[test]
fn activation_recovery_late_accepted_cleanup_failure_cannot_regress_positive_helper_evidence() {
    reconciled_add_with_late_cleanup_failure(false);
}

#[test]
fn activation_recovery_positive_helper_evidence_is_scoped_to_the_exact_operation() {
    let (store, _path, r, p) = fixture();
    let ticket = arm(&store, &r, &p);
    let add = row(&store, &r).helper.unwrap();
    fulfill(&store, &r, &p);
    result(
        &store,
        &ticket,
        WakeIntentOutcome::Accepted {
            id: "owned-id".into(),
        },
        true,
    );
    let listing = op(&r, &p, WakeHelperPurpose::List, None);
    assert!(begin(&store, &r, &listing));
    assert!(
        row(&store, &r).cleanup_quarantined,
        "beginning a new operation must re-arm quarantine"
    );
    assert!(
        !StorePort::record_wake_helper_completion(&store, &r.attempt, add.id, true, &budget())
            .unwrap()
    );
    assert!(
        row(&store, &r).cleanup_quarantined,
        "old operation's positive evidence cannot clear the new helper"
    );
    assert!(
        StorePort::record_wake_helper_completion(&store, &r.attempt, listing.id, false, &budget())
            .unwrap()
    );
    assert!(
        row(&store, &r).cleanup_quarantined,
        "failure before this operation's own positive evidence keeps quarantine"
    );
    complete(&store, &r, &listing);
    assert!(
        !StorePort::record_wake_helper_completion(&store, &r.attempt, add.id, false, &budget())
            .unwrap()
    );
    let after = row(&store, &r);
    assert_eq!(after.helper.unwrap().id, listing.id);
    assert!(
        !after.cleanup_quarantined,
        "stale failure from another operation cannot restore quarantine"
    );
}

#[test]
fn activation_recovery_prepared_refusal_cannot_regress_positive_helper_evidence() {
    for reconciled in [false, true] {
        let (store, _path, r, p) = fixture();
        let prepared = prepare(&store, &r, &p);
        let helper = prepared.helper.unwrap();
        assert_eq!(prepared.phase, WakeIntentPhase::Prepared);
        assert!(helper.witness.is_some());
        if reconciled {
            assert!(
                StorePort::reconcile_wake_helper(
                    &store,
                    &r.attempt,
                    helper.id,
                    &WakeHelperPresence::Quiescent(WakeHelperQuiescence::LeaderEndedAndGroupAbsent),
                    &budget()
                )
                .unwrap()
            );
        } else {
            complete(&store, &r, &helper);
        }
        let positive = row(&store, &r);
        assert!(!positive.cleanup_quarantined);
        assert!(
            StorePort::refuse_prepared_wake_intent(
                &store,
                &r.attempt,
                positive.revision,
                false,
                "late refusal reports uncertain helper cleanup",
                &budget()
            )
            .unwrap()
        );
        let refused = row(&store, &r);
        assert_eq!(refused.phase, WakeIntentPhase::NoEffect);
        assert_eq!(refused.helper.unwrap().id, helper.id);
        assert!(
            !refused.cleanup_quarantined,
            "late refusal cannot erase the current helper's positive quiescence evidence; reconciled={reconciled}"
        );
    }
}

#[test]
fn activation_recovery_new_helper_resets_prior_native_no_add_exception() {
    let (store, _path, r, p) = fixture();
    prepare_raw(&store, &r, &p);
    let old = op(&r, &p, WakeHelperPurpose::Add, None);
    assert!(begin(&store, &r, &old));
    complete(&store, &r, &old);
    let clean = row(&store, &r);
    assert!(
        StorePort::refuse_prepared_wake_intent(
            &store,
            &r.attempt,
            clean.revision,
            true,
            "old add never authorized",
            &budget()
        )
        .unwrap()
    );
    // Seed the prior revoked, unarmed operation's native exception independently
    // of daemon-owner recovery. Canonical binding finality retires its scope.
    store.context().open_writer().unwrap().execute_batch(
        "UPDATE wake_intents SET native_no_add_exception=1; UPDATE occupant_bindings SET ended_at=101"
    ).unwrap();
    assert!(StorePort::retire_wake_intent_if_scope_ended(&store, &r.attempt, &budget()).unwrap());
    let retired = row(&store, &r);
    assert_eq!(retired.logical, WakeIntentLogical::ScopeEnded);
    assert!(!retired.cleanup_quarantined);
    assert!(retired.native_no_add_exception);

    let listing = op(&r, &p, WakeHelperPurpose::List, None);
    assert!(begin(&store, &r, &listing));
    let begun = row(&store, &r);
    assert_eq!(begun.helper.unwrap().id, listing.id);
    assert!(
        !begun.native_no_add_exception,
        "a new helper cannot inherit its predecessor's native no-add exception"
    );
    assert!(
        StorePort::record_wake_helper_started(
            &store,
            &r.attempt,
            listing.id,
            &herdr_threads::harness::wake::WakeHelperWitness {
                process: p.target.scope.process,
                kernel_boot: p.target.scope.kernel_boot.clone(),
                pgid: p.target.scope.process.pid,
            },
            &budget()
        )
        .unwrap()
    );
    assert!(
        StorePort::record_wake_helper_completion(&store, &r.attempt, listing.id, false, &budget())
            .unwrap()
    );
    let uncertain = row(&store, &r);
    assert!(uncertain.cleanup_quarantined);
    assert!(!uncertain.native_no_add_exception);
    assert_blocked(&store, &r, &p);
}
