use herdr_threads::{
    harness::wake::{WakeHelperPurpose, WakeHelperWitness},
    host::process_table::HostProcessTable,
    notification::queue_recovery::QueueRecovery,
    ports::{PrepareWakeIntent, StorePort, WakeIntentSnapshot},
    protocol::{
        authority::LivenessFingerprint,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::fair_writer::FairWriter,
    store::SqliteStore,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Default::default(),
    }
}
fn rig() -> (
    Arc<SqliteStore>,
    Arc<TestClock>,
    QueueRecovery,
    WakeIntentSnapshot,
) {
    let (store, path, reservation, preparation) = super::wake_intents::fixture();
    let PrepareWakeIntent::Prepared(row) = store
        .prepare_wake_intent(&reservation, &preparation, &budget())
        .unwrap()
    else {
        panic!("fresh intent");
    };
    let boot = reservation.daemon_boot;
    let store = Arc::new(store);
    let clock = Arc::new(TestClock(AtomicU64::new(1000)));
    let recovery = QueueRecovery::new(
        store.clone(),
        Arc::new(FairWriter::new(32)),
        clock.clone(),
        Arc::new(HostProcessTable),
        path.parent().unwrap().to_owned(),
        boot,
    );
    (store, clock, recovery, row)
}
fn retire(store: &SqliteStore, row: &WakeIntentSnapshot, phase: &str) {
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET logical_status='scope_ended',phase=?2,arm_nonce='00000000-0000-0000-0000-000000000011' WHERE attempt=?1",
            rusqlite::params![row.attempt.as_str(), phase],
        )
        .unwrap();
}
fn get(store: &SqliteStore, row: &WakeIntentSnapshot) -> WakeIntentSnapshot {
    store.wake_intent(&row.attempt, &budget()).unwrap().unwrap()
}
#[test]
fn retired_cleanup_is_attempted_once_and_never_creates_a_real_helper() {
    let (store, _clock, recovery, row) = rig();
    retire(&store, &row, "unknown");
    recovery.tick(&budget()).unwrap();
    let next = get(&store, &row);
    assert!(next.cleanup_list_attempted);
    assert!(
        !next.cleanup_quarantined,
        "unsupported descriptor fails before spawn"
    );
    assert!(
        !recovery.has_work(),
        "finished finite cleanup permits daemon idle exit"
    );
}
#[test]
fn active_effects_and_retired_armed_attempts_never_start_cleanup() {
    for (logical, phase) in [("active", "unknown"), ("scope_ended", "armed")] {
        let (store, _clock, recovery, row) = rig();
        store
            .context()
            .open_writer()
            .unwrap()
            .execute(
                "UPDATE wake_intents SET logical_status=?2,phase=?3,arm_nonce='00000000-0000-0000-0000-000000000011' WHERE attempt=?1",
                rusqlite::params![row.attempt.as_str(), logical, phase],
            )
            .unwrap();
        recovery.tick(&budget()).unwrap();
        let next = get(&store, &row);
        assert!(!next.cleanup_list_attempted);
        assert!(next.helper.is_none());
        assert!(!recovery.has_work());
    }
}
#[test]
fn cleanup_quantum_has_one_control_and_at_least_five_second_spacing() {
    let (store, clock, recovery, row) = rig();
    retire(&store, &row, "unknown");
    let db = store.context().open_writer().unwrap();
    // Duplicate only this frozen, retired cleanup record under a distinct historical attempt.
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(wake_intents)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let selected = columns
        .iter()
        .map(|c| match c.as_str() {
            "ordinal" => "NULL".to_owned(),
            "attempt" => "'second-attempt'".to_owned(),
            _ => c.clone(),
        })
        .collect::<Vec<_>>()
        .join(",");
    db.execute(
        &format!(
            "INSERT INTO wake_intents({}) SELECT {selected} FROM wake_intents WHERE attempt=?1",
            columns.join(",")
        ),
        [row.attempt.as_str()],
    )
    .unwrap();
    drop(db);
    recovery.tick(&budget()).unwrap();
    let count = || {
        store
            .context()
            .open_writer()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM wake_intents WHERE cleanup_list_attempted=1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(count(), 1);
    assert!(recovery.has_work());
    clock.0.store(5999, Ordering::SeqCst);
    recovery.tick(&budget()).unwrap();
    assert_eq!(count(), 1);
    clock.0.store(6000, Ordering::SeqCst);
    recovery.tick(&budget()).unwrap();
    assert_eq!(count(), 2);
    assert!(!recovery.has_work());
}
#[test]
fn unwitnessed_same_boot_quarantine_remains_unknown_without_busy_keepalive() {
    let (store, _clock, recovery, row) = rig();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
        .unwrap()
        .unwrap();
    recovery.tick(&budget()).unwrap();
    let next = get(&store, &row);
    assert!(next.cleanup_quarantined);
    assert_eq!(next.helper.unwrap().id, journal.op_id());
    assert!(!recovery.has_work());
}
#[test]
fn helper_journal_commits_witness_and_idempotent_completion() {
    use herdr_threads::{harness::wake::WakeHelperObserver, ports::ProcessInfo};
    let (store, _clock, recovery, row) = rig();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
        .unwrap()
        .unwrap();
    let witness = WakeHelperWitness {
        process: LivenessFingerprint { pid: 10, start: 1 },
        pgid: 10,
        kernel_boot: HostProcessTable.kernel_boot().unwrap(),
    };
    journal.started(&witness).unwrap();
    assert_eq!(get(&store, &row).helper.unwrap().witness, Some(witness));
    journal.complete(true).unwrap();
    let revision = get(&store, &row).revision;
    journal.complete(true).unwrap();
    assert_eq!(
        get(&store, &row).revision,
        revision,
        "outer completion must not repeat adapter completion"
    );
    assert!(!get(&store, &row).cleanup_quarantined);
}

#[test]
fn lost_cleanup_progress_closes_saved_exact_operation_without_replay() {
    for purpose in [WakeHelperPurpose::List, WakeHelperPurpose::Delete] {
        let (store, _clock, recovery, row) = rig();
        retire(&store, &row, "unknown");
        let subject = if purpose == WakeHelperPurpose::Delete {
            let db = store.context().open_writer().unwrap();
            db.execute(
                "INSERT INTO wake_intent_ids(intent_ordinal,queue_id) VALUES(?1,'owned')",
                [row.ordinal as i64],
            )
            .unwrap();
            db.execute(
                "UPDATE wake_intents SET cleanup_list_attempted=1 WHERE attempt=?1",
                [row.attempt.as_str()],
            )
            .unwrap();
            Some("owned".to_owned())
        } else {
            None
        };
        let row = get(&store, &row);
        let journal = recovery
            .begin_helper(&row, purpose, subject, &budget())
            .unwrap()
            .unwrap();
        journal.complete(true).unwrap();
        let mut helper = get(&store, &row).helper.unwrap();
        helper.owner_boot = uuid::Uuid::new_v4();
        store
            .context()
            .open_writer()
            .unwrap()
            .execute(
                "UPDATE wake_intents SET helper_json=?2 WHERE attempt=?1",
                rusqlite::params![row.attempt.as_str(), serde_json::to_vec(&helper).unwrap()],
            )
            .unwrap();
        recovery.tick(&budget()).unwrap();
        let next = get(&store, &row);
        assert_eq!(
            next.helper.unwrap().id,
            journal.op_id(),
            "saved attempt must not be replaced by another control"
        );
        assert!(next.cleanup_list_attempted);
        assert!(next.cleanup_pending_ids.is_empty());
        assert!(!recovery.has_work());
    }
}
#[test]
fn kernel_change_clears_unwitnessed_quarantine_without_queue_control() {
    let (store, _clock, recovery, row) = rig();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
        .unwrap()
        .unwrap();
    let mut helper = get(&store, &row).helper.unwrap();
    helper.kernel_boot = "prior-kernel".to_owned();
    store.context().open_writer().unwrap().execute("UPDATE wake_intents SET helper_json=?2,phase='armed',arm_nonce='00000000-0000-0000-0000-000000000011' WHERE attempt=?1",rusqlite::params![row.attempt.as_str(),serde_json::to_vec(&helper).unwrap()]).unwrap();
    recovery.tick(&budget()).unwrap();
    let next = get(&store, &row);
    assert!(!next.cleanup_quarantined);
    assert!(!next.cleanup_list_attempted);
    assert_eq!(next.helper.unwrap().id, journal.op_id());
    assert!(
        !recovery.has_work(),
        "current-owner Armed awaits result rather than cleanup work"
    );
}
#[test]
fn recovery_scans_at_most_thirty_two_summaries_per_quantum() {
    let (store, clock, recovery, row) = rig();
    let db = store.context().open_writer().unwrap();
    let old = uuid::Uuid::new_v4().to_string();
    db.execute(
        "UPDATE wake_intents SET owner_boot=?2 WHERE attempt=?1",
        rusqlite::params![row.attempt.as_str(), old],
    )
    .unwrap();
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(wake_intents)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let binding_columns: Vec<String> = db
        .prepare("PRAGMA table_info(occupant_bindings)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for n in 0..32 {
        let mut scope = row.preparation.target.scope.clone();
        scope.binding =
            herdr_threads::protocol::ids::BindingId::new(format!("recovery-binding-{n}"));
        scope.native_session = herdr_threads::protocol::ids::NativeSessionId::new(
            uuid::Uuid::from_u128(n + 2).to_string(),
        );
        let selected = binding_columns
            .iter()
            .map(|c| match c.as_str() {
                "ordinal" => "NULL".to_owned(),
                "binding_id" => "?2".to_owned(),
                "native_session" => "?3".to_owned(),
                _ => c.clone(),
            })
            .collect::<Vec<_>>()
            .join(",");
        db.execute(&format!("INSERT INTO occupant_bindings({}) SELECT {selected} FROM occupant_bindings WHERE binding_id=?1", binding_columns.join(",")), rusqlite::params![row.preparation.target.scope.binding.as_str(), scope.binding.as_str(), scope.native_session.as_str()]).unwrap();
        let selected = columns
            .iter()
            .map(|c| match c.as_str() {
                "ordinal" => "NULL".to_owned(),
                "attempt" => "?2".to_owned(),
                "target_json" => "?3".to_owned(),
                "replay_scope" => "?4".to_owned(),
                _ => c.clone(),
            })
            .collect::<Vec<_>>()
            .join(",");
        db.execute(
            &format!(
                "INSERT INTO wake_intents({}) SELECT {selected} FROM wake_intents WHERE attempt=?1",
                columns.join(",")
            ),
            rusqlite::params![
                row.attempt.as_str(),
                format!("recovery-{n}"),
                serde_json::to_vec(&scope).unwrap(),
                format!("session:{}", scope.native_session.as_str())
            ],
        )
        .unwrap();
    }
    drop(db);
    let count = || {
        store
            .context()
            .open_writer()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM wake_intents WHERE phase='no_effect'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    recovery.tick(&budget()).unwrap();
    assert_eq!(count(), 32);
    clock.0.store(6000, Ordering::SeqCst);
    recovery.tick(&budget()).unwrap();
    assert_eq!(count(), 33);
    assert!(!recovery.has_work());
}

#[test]
fn nonquiescent_witness_keeps_quarantine_and_finite_recovery_work() {
    use herdr_threads::{harness::wake::WakeHelperObserver, ports::ProcessInfo};
    let (store, _clock, recovery, row) = rig();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
        .unwrap()
        .unwrap();
    journal
        .started(&WakeHelperWitness {
            process: LivenessFingerprint::current_process(),
            pgid: std::process::id(),
            kernel_boot: HostProcessTable.kernel_boot().unwrap(),
        })
        .unwrap();
    recovery.tick(&budget()).unwrap();
    assert!(get(&store, &row).cleanup_quarantined);
    assert!(recovery.has_work());
}
#[test]
fn cancelled_send_budget_refuses_start_but_preserves_cleanup_evidence() {
    use herdr_threads::{
        harness::wake::WakeHelperObserver, ports::ProcessInfo, protocol::results::ErrorCode,
    };
    let (store, _clock, recovery, row) = rig();
    let budget = budget();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget)
        .unwrap()
        .unwrap();
    budget.cancellation.cancel();
    let witness = WakeHelperWitness {
        process: LivenessFingerprint { pid: 10, start: 1 },
        pgid: 10,
        kernel_boot: HostProcessTable.kernel_boot().unwrap(),
    };
    assert!(journal.started(&witness).is_err());
    journal.complete(true).unwrap();
    assert_eq!(
        recovery.tick(&budget).unwrap_err().code,
        ErrorCode::Cancelled
    );
    let next = get(&store, &row);
    assert!(!next.cleanup_quarantined);
    assert!(next.helper.unwrap().witness.is_none());
}
#[test]
fn prior_owner_unarmed_helper_becomes_no_effect_without_replaying_add() {
    use herdr_threads::ports::WakeIntentPhase;
    let (store, _clock, recovery, row) = rig();
    recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
        .unwrap()
        .unwrap();
    let mut helper = get(&store, &row).helper.unwrap();
    let old = uuid::Uuid::new_v4();
    helper.owner_boot = old;
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET owner_boot=?2,helper_json=?3 WHERE attempt=?1",
            rusqlite::params![
                row.attempt.as_str(),
                old.to_string(),
                serde_json::to_vec(&helper).unwrap()
            ],
        )
        .unwrap();
    recovery.tick(&budget()).unwrap();
    let next = get(&store, &row);
    assert_eq!(next.phase, WakeIntentPhase::NoEffect);
    assert!(next.cleanup_quarantined);
    assert!(next.native_no_add_exception);
    assert_eq!(next.helper.unwrap().id, helper.id);
    assert!(!recovery.has_work());
}

#[test]
fn deferred_cohort_does_not_block_later_independent_cleanup() {
    use herdr_threads::protocol::ids::WakeAttemptId;
    let (store, _clock, recovery, row) = rig();
    retire(&store, &row, "unknown");
    let db = store.context().open_writer().unwrap();
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(wake_intents)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (attempt, phase, logical, different) in [
        ("new-active", "prepared", "active", false),
        ("other-cleanup", "unknown", "scope_ended", true),
    ] {
        let mut scope = row.preparation.target.scope.clone();
        if different {
            scope.native_session = herdr_threads::protocol::ids::NativeSessionId::new(
                "00000000-0000-0000-0000-000000000002",
            );
        }
        let selected = columns
            .iter()
            .map(|c| match c.as_str() {
                "ordinal" => "NULL".to_owned(),
                "attempt" => "?2".to_owned(),
                "phase" => "?3".to_owned(),
                "logical_status" => "?4".to_owned(),
                "target_json" => "?5".to_owned(),
                "replay_scope" => "?6".to_owned(),
                "arm_nonce" => "CASE WHEN ?3='prepared' THEN NULL ELSE arm_nonce END".to_owned(),
                _ => c.clone(),
            })
            .collect::<Vec<_>>()
            .join(",");
        db.execute(
            &format!(
                "INSERT INTO wake_intents({}) SELECT {selected} FROM wake_intents WHERE attempt=?1",
                columns.join(",")
            ),
            rusqlite::params![
                row.attempt.as_str(),
                attempt,
                phase,
                logical,
                serde_json::to_vec(&scope).unwrap(),
                format!("session:{}", scope.native_session.as_str())
            ],
        )
        .unwrap();
    }
    drop(db);
    recovery.tick(&budget()).unwrap();
    let old = get(&store, &row);
    assert!(!old.cleanup_list_attempted);
    assert!(
        old.helper.is_none(),
        "deferral must not allocate a helper slot"
    );
    let new = store
        .wake_intent(&WakeAttemptId::new("new-active"), &budget())
        .unwrap()
        .unwrap();
    assert_eq!(new.phase, herdr_threads::ports::WakeIntentPhase::Prepared);
    assert!(new.helper.is_none());
    let other = store
        .wake_intent(&WakeAttemptId::new("other-cleanup"), &budget())
        .unwrap()
        .unwrap();
    assert!(
        other.cleanup_list_attempted,
        "independent cleanup behind deferred history must progress"
    );
    assert!(
        !recovery.has_work(),
        "only the indefinitely deferred history remains"
    );
}

#[test]
fn completed_helper_cleanup_survives_send_cancellation_or_deadline() {
    use herdr_threads::{harness::wake::WakeHelperObserver, ports::ProcessInfo};
    for cancel in [true, false] {
        let (store, clock, recovery, row) = rig();
        let mut send = budget();
        send.deadline = MonoInstant(1500);
        let journal = recovery
            .begin_helper(&row, WakeHelperPurpose::Add, None, &send)
            .unwrap()
            .unwrap();
        let witness = WakeHelperWitness {
            process: LivenessFingerprint { pid: 10, start: 1 },
            pgid: 10,
            kernel_boot: HostProcessTable.kernel_boot().unwrap(),
        };
        journal.started(&witness).unwrap();
        if cancel {
            send.cancellation.cancel();
        } else {
            clock.0.store(1600, Ordering::SeqCst);
        }
        assert!(
            journal.started(&witness).is_err(),
            "send expiry still forbids startup callbacks"
        );
        journal.completed(true).unwrap();
        let next = get(&store, &row);
        assert!(
            !next.cleanup_quarantined,
            "physical completion is independent of the send budget"
        );
        assert_eq!(next.helper.unwrap().witness, Some(witness));
    }
}
#[test]
fn cleanup_metadata_budget_cannot_override_lost_helper_ownership() {
    let (store, _clock, recovery, row) = rig();
    let send = budget();
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &send)
        .unwrap()
        .unwrap();
    let mut helper = get(&store, &row).helper.unwrap();
    helper.owner_boot = uuid::Uuid::new_v4();
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET helper_json=?2 WHERE attempt=?1",
            rusqlite::params![row.attempt.as_str(), serde_json::to_vec(&helper).unwrap()],
        )
        .unwrap();
    send.cancellation.cancel();
    assert!(journal.complete(true).is_err());
    assert!(!journal.cleanup_confirmed());
    assert!(journal.complete(false).is_err());
    assert!(!journal.cleanup_confirmed());
    assert!(get(&store, &row).cleanup_quarantined);
}

fn seed_cleanup_gap(
    store: &SqliteStore,
    row: &WakeIntentSnapshot,
    purpose: WakeHelperPurpose,
    active: bool,
    witnessed: bool,
) -> herdr_threads::ports::WakeHelperOperation {
    use herdr_threads::ports::{ProcessInfo, WakeHelperOperation};
    let helper = WakeHelperOperation {
        id: uuid::Uuid::new_v4(),
        purpose,
        subject: (purpose == WakeHelperPurpose::Delete).then(|| "retained-id".to_owned()),
        owner_boot: uuid::Uuid::new_v4(),
        kernel_boot: HostProcessTable.kernel_boot().unwrap(),
        witness: witnessed.then(|| WakeHelperWitness {
            process: LivenessFingerprint::current_process(),
            pgid: std::process::id(),
            kernel_boot: HostProcessTable.kernel_boot().unwrap(),
        }),
    };
    let db = store.context().open_writer().unwrap();
    db.execute("UPDATE wake_intents SET logical_status=?2,phase='accepted',arm_nonce='00000000-0000-0000-0000-000000000011',helper_json=?3,cleanup_quarantined=1,cleanup_list_attempted=?4 WHERE attempt=?1",rusqlite::params![row.attempt.as_str(),if active {"active"} else {"scope_ended"},serde_json::to_vec(&helper).unwrap(),purpose==WakeHelperPurpose::Delete]).unwrap();
    db.execute(
        "INSERT INTO wake_intent_ids(intent_ordinal,queue_id) VALUES(?1,'retained-id')",
        [row.ordinal as i64],
    )
    .unwrap();
    helper
}
#[test]
fn old_unwitnessed_retired_cleanup_gap_allows_only_native_without_changing_effects() {
    use herdr_threads::ports::{WakeIntentLogical, WakeIntentPhase};
    for purpose in [WakeHelperPurpose::List, WakeHelperPurpose::Delete] {
        let (store, _clock, recovery, row) = rig();
        let helper = seed_cleanup_gap(&store, &row, purpose, false, false);
        recovery.tick(&budget()).unwrap();
        let next = get(&store, &row);
        assert!(next.native_no_add_exception);
        assert_eq!(next.phase, WakeIntentPhase::Accepted);
        assert_eq!(next.logical, WakeIntentLogical::ScopeEnded);
        assert!(next.cleanup_quarantined);
        assert_eq!(next.owned_ids, ["retained-id"]);
        assert_eq!(next.cleanup_pending_ids, ["retained-id"]);
        assert_eq!(
            next.cleanup_list_attempted,
            purpose == WakeHelperPurpose::Delete
        );
        assert_eq!(next.helper, Some(helper));
        assert!(
            !recovery.has_work(),
            "same-boot unwitnessed resource uncertainty has no runnable probe"
        );
    }
}
#[test]
fn active_effect_or_durable_witness_refuses_cleanup_gap_native_exception() {
    for purpose in [WakeHelperPurpose::List, WakeHelperPurpose::Delete] {
        for (active, witnessed) in [(true, false), (false, true)] {
            let (store, _clock, recovery, row) = rig();
            // A durable witness remains authoritative even when its callback reply was lost.
            let helper = seed_cleanup_gap(&store, &row, purpose, active, witnessed);
            recovery.tick(&budget()).unwrap();
            let next = get(&store, &row);
            assert!(!next.native_no_add_exception);
            assert!(next.cleanup_quarantined);
            assert_eq!(next.helper, Some(helper));
            assert_eq!(next.owned_ids, ["retained-id"]);
            assert_eq!(next.cleanup_pending_ids, ["retained-id"]);
            assert_eq!(
                next.cleanup_list_attempted,
                purpose == WakeHelperPurpose::Delete
            );
        }
    }
}

#[test]
fn cleanup_commit_retry_preserves_first_physical_observation() {
    for observed in [true, false] {
        let (store, _clock, recovery, row) = rig();
        let journal = recovery
            .begin_helper(&row, WakeHelperPurpose::Add, None, &budget())
            .unwrap()
            .unwrap();
        let original = get(&store, &row).helper.unwrap();
        let mut refused = original.clone();
        refused.owner_boot = uuid::Uuid::new_v4();
        let set_helper = |helper: &herdr_threads::ports::WakeHelperOperation| {
            store
                .context()
                .open_writer()
                .unwrap()
                .execute(
                    "UPDATE wake_intents SET helper_json=?2 WHERE attempt=?1",
                    rusqlite::params![row.attempt.as_str(), serde_json::to_vec(helper).unwrap()],
                )
                .unwrap();
        };
        // A deterministic exact-owner CAS refusal leaves the metadata write uncommitted.
        set_helper(&refused);
        assert!(journal.complete(observed).is_err());
        assert!(
            !journal.cleanup_confirmed(),
            "physical observation alone is not committed evidence"
        );
        set_helper(&original);
        journal.complete(!observed).unwrap();
        assert_eq!(
            get(&store, &row).cleanup_quarantined,
            !observed,
            "retry must persist the first physical observation"
        );
        assert_eq!(journal.cleanup_confirmed(), observed);
        let revision = get(&store, &row).revision;
        journal.complete(!observed).unwrap();
        assert_eq!(
            get(&store, &row).revision,
            revision,
            "committed completion stays idempotent"
        );
    }
}

struct CleanupWorker {
    stop: herdr_threads::protocol::time::Cancellation,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for CleanupWorker {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[test]
fn queue_cleanup_worker_uses_pacer_clock_and_cancels_during_wait() {
    use herdr_threads::{protocol::time::Cancellation, service::pacer::Pacer};
    use std::{sync::mpsc, time::Duration};
    let (_store, clock, recovery, _row) = rig();
    let stop = Cancellation::default();
    let pacer = Arc::new(Pacer::new("queue-cleanup", clock.clone(), stop.clone()));
    let (idle, received) = mpsc::channel();
    pacer.set_idle_hook(Box::new(move |count| {
        let _ = idle.send(count);
    }));
    let worker_pacer = pacer.clone();
    let worker_stop = stop.clone();
    let worker = CleanupWorker {
        stop,
        thread: Some(std::thread::spawn(move || {
            recovery.run(&worker_pacer, &worker_stop, |_| {});
        })),
    };
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        1,
        "the immediate first pass must enter the pacer"
    );
    clock.0.store(5999, Ordering::SeqCst);
    pacer.clock_advanced();
    assert!(
        received.recv_timeout(Duration::from_millis(30)).is_err(),
        "no early pass"
    );
    clock.0.store(6000, Ordering::SeqCst);
    pacer.clock_advanced();
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
    worker.stop.cancel();
    drop(worker);
    assert_eq!(
        pacer.wakes(),
        2,
        "one safety tick and one cancellation, without spinning"
    );
}

#[test]
fn queue_cleanup_worker_cancels_during_writer_admission() {
    use herdr_threads::{protocol::time::Cancellation, service::pacer::Pacer};
    use std::time::{Duration, Instant};
    let (store, clock, _recovery, row) = rig();
    let writer = Arc::new(FairWriter::new(32));
    let permit = writer.enter_foreground(&budget(), clock.as_ref()).unwrap();
    let recovery = QueueRecovery::new(
        store,
        writer.clone(),
        clock.clone(),
        Arc::new(HostProcessTable),
        std::path::PathBuf::from("."),
        row.owner_boot,
    );
    let stop = Cancellation::default();
    let worker_stop = stop.clone();
    let worker_clock = clock.clone();
    let worker = CleanupWorker {
        stop,
        thread: Some(std::thread::spawn(move || {
            let pacer = Pacer::new("queue-cleanup", worker_clock, worker_stop.clone());
            recovery.run(&pacer, &worker_stop, |_| {});
        })),
    };
    let until = Instant::now() + Duration::from_secs(2);
    while writer.waiting().1 == 0 && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(writer.waiting().1, 1, "cleanup reached the held writer");
    worker.stop.cancel();
    drop(worker);
    assert_eq!(
        writer.waiting(),
        (0, 0),
        "cancellation removes the admitted cleanup waiter"
    );
    drop(permit);
}

#[test]
fn queue_cleanup_worker_exhausted_quantum_waits_for_next_safety_tick() {
    use herdr_threads::{
        protocol::{results::ErrorCode, time::Cancellation},
        service::pacer::Pacer,
    };
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let (store, clock, _recovery, row) = rig();
    let writer = Arc::new(FairWriter::new(32));
    let permit = writer.enter_foreground(&budget(), clock.as_ref()).unwrap();
    let recovery = QueueRecovery::new(
        store,
        writer.clone(),
        clock.clone(),
        Arc::new(HostProcessTable),
        std::path::PathBuf::from("."),
        row.owner_boot,
    );
    let stop = Cancellation::default();
    let pacer = Arc::new(Pacer::new("queue-cleanup", clock.clone(), stop.clone()));
    let (idle, received) = mpsc::channel();
    pacer.set_idle_hook(Box::new(move |count| {
        let _ = idle.send(count);
    }));
    let (errors, failure) = mpsc::channel();
    let worker_pacer = pacer.clone();
    let worker_stop = stop.clone();
    let worker = CleanupWorker {
        stop,
        thread: Some(std::thread::spawn(move || {
            recovery.run(&worker_pacer, &worker_stop, |error| {
                let _ = errors.send(error.code.clone());
            });
        })),
    };
    let until = Instant::now() + Duration::from_secs(2);
    while writer.waiting().1 == 0 && Instant::now() < until {
        std::thread::yield_now();
    }
    assert_eq!(writer.waiting().1, 1);
    clock.0.store(4000, Ordering::SeqCst);
    drop(permit);
    assert_eq!(
        failure.recv_timeout(Duration::from_secs(2)).unwrap(),
        ErrorCode::DeadlineExceeded
    );
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
    assert!(
        received.recv_timeout(Duration::from_millis(30)).is_err(),
        "exhaustion must not busy-spin"
    );
    clock.0.store(8999, Ordering::SeqCst);
    pacer.clock_advanced();
    assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
    clock.0.store(9000, Ordering::SeqCst);
    pacer.clock_advanced();
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
    drop(worker);
}
