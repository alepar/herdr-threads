use herdr_threads::{
    ports::StorePort,
    protocol::{
        pagination::PageRequest,
        time::{CallBudget, Clock, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::sync::Arc;

#[test]
fn wake_intent_schema_is_independent_of_live_origin_rows() {
    let path = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-wake-intents-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context =
        StoreContext::new(path, Arc::new(herdr_threads::app::SystemClock::new())).with_test_host();
    context.require_commit_durability();
    let db = context.open_writer().unwrap();
    let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('wake_intents','wake_intent_ids')", [], |r| r.get(0)).unwrap();
    assert_eq!(
        tables, 2,
        "durable queue effects need their own retained ledger"
    );
    let mut stmt = db.prepare("PRAGMA foreign_key_list(wake_intents)").unwrap();
    assert_eq!(
        stmt.query_map([], |r| r.get::<_, String>(2))
            .unwrap()
            .count(),
        0,
        "origin retirement must not cascade uncertain effects"
    );
}

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const SESSION: &str = "00000000-0000-0000-0000-000000000001";
struct IntentClock;
impl Clock for IntentClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> herdr_threads::protocol::time::MonoInstant {
        herdr_threads::protocol::time::MonoInstant(1000)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: herdr_threads::protocol::time::MonoInstant(10000),
        cancellation: Default::default(),
    }
}
fn open(path: std::path::PathBuf, boot: uuid::Uuid) -> SqliteStore {
    let context = StoreContext::new(path, Arc::new(IntentClock)).with_test_host();
    context.require_commit_durability();
    SqliteStore::new(
        context,
        INSTANCE,
        StoreSettings {
            daemon_boot: Some(boot),
            ..Default::default()
        },
    )
    .unwrap()
}
fn fixture() -> (
    SqliteStore,
    std::path::PathBuf,
    herdr_threads::ports::WakeReservation,
    herdr_threads::ports::WakeIntentPreparation,
) {
    use herdr_threads::{
        harness::{
            registry::builtins,
            wake::{WakeControlTarget, WakeReplayScope, WakeScope},
        },
        protocol::{
            authority::LivenessFingerprint,
            ids::{BindingId, ExecutionId, NativeSessionId},
        },
    };
    let path = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-wake-intents-{}.db",
        uuid::Uuid::new_v4()
    ));
    let store = open(path.clone(), uuid::Uuid::new_v4());
    let db = store.context().open_writer().unwrap();
    assert_eq!(
        db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.execute_batch("UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=1;
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
        INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0);
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES ('inv','t','s',1,'pending',0,1,100,100,(SELECT id FROM local_node));
    ").unwrap();
    let kernel: String = db
        .query_row("SELECT boot FROM node_kernel_boot WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id,liveness_pid,liveness_start,liveness_boot) VALUES ('s',1,1,'pane','host',0,'codex',?1,'execution','cooperative_top_level',0,0,'term-pane','inc','binding',10,1,?2)", rusqlite::params![SESSION,kernel]).unwrap();
    drop(db);
    let candidate = StorePort::wake_candidates(
        &store,
        herdr_threads::test_support::test_host_id(),
        PageRequest::default(),
        &budget(),
    )
    .unwrap()
    .items
    .remove(0);
    let reservation = StorePort::reserve_wake(
        &store,
        herdr_threads::test_support::test_host_id(),
        &candidate,
        &budget(),
    )
    .unwrap()
    .unwrap();
    let preparation = herdr_threads::ports::WakeIntentPreparation {
        adapter: builtins().agent("codex").unwrap(),
        replay_scope: WakeReplayScope::parse(format!("session:{SESSION}")).unwrap(),
        target: WakeControlTarget {
            scope: WakeScope {
                binding: BindingId::new("binding"),
                native_session: NativeSessionId::new(SESSION),
                execution: ExecutionId::new("execution"),
                generation: 1,
                process: LivenessFingerprint { pid: 10, start: 1 },
                kernel_boot: kernel,
            },
            descriptor: b"frozen-descriptor".to_vec(),
        },
        correlation: "frozen-correlation".into(),
    };
    (store, path, reservation, preparation)
}
fn prepare(
    store: &SqliteStore,
    r: &herdr_threads::ports::WakeReservation,
    p: &herdr_threads::ports::WakeIntentPreparation,
) -> herdr_threads::ports::WakeIntentSnapshot {
    match StorePort::prepare_wake_intent(store, r, p, &budget()).unwrap() {
        herdr_threads::ports::PrepareWakeIntent::Prepared(row) => row,
        other => panic!("expected prepared intent, got {other:?}"),
    }
}
#[test]
fn committed_arm_is_never_authorized_twice_even_after_reopen() {
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    assert_eq!(first.phase, herdr_threads::ports::WakeIntentPhase::Prepared);
    assert_eq!(first.frontier, r.attention_witness.frontier());
    let nonce = uuid::Uuid::new_v4();
    let ticket = StorePort::arm_wake_intent(&store, &r, first.revision, nonce, &budget())
        .unwrap()
        .unwrap();
    assert!(
        StorePort::arm_wake_intent(&store, &r, first.revision, nonce, &budget())
            .unwrap()
            .is_none()
    );
    drop(store);
    let store = open(path, r.daemon_boot);
    assert!(
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .is_none()
    );
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.phase, herdr_threads::ports::WakeIntentPhase::Armed);
    assert_eq!(row.arm_nonce, Some(ticket.nonce));
    assert_eq!(row.preparation, p);
}

#[test]
fn stale_result_cannot_erase_acceptance_or_cleanup_quarantine() {
    use herdr_threads::ports::{WakeIntentOutcome, WakeIntentPhase, WakeIntentResult};
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let ticket =
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .unwrap();
    let result = WakeIntentResult {
        outcome: WakeIntentOutcome::Accepted {
            id: "owned-id".into(),
        },
        cleanup_confirmed: false,
        diagnostic: "cleanup not confirmed".into(),
    };
    let mut stale = ticket.clone();
    stale.nonce = uuid::Uuid::new_v4();
    assert!(!StorePort::record_wake_intent_result(&store, &stale, &result, &budget()).unwrap());
    stale = ticket.clone();
    stale.owner_boot = uuid::Uuid::new_v4();
    assert!(!StorePort::record_wake_intent_result(&store, &stale, &result, &budget()).unwrap());
    assert!(StorePort::record_wake_intent_result(&store, &ticket, &result, &budget()).unwrap());
    assert!(
        !StorePort::record_wake_intent_result(
            &store,
            &ticket,
            &WakeIntentResult {
                outcome: WakeIntentOutcome::NoEffect,
                cleanup_confirmed: true,
                diagnostic: String::new()
            },
            &budget()
        )
        .unwrap()
    );
    drop(store);
    let store = open(path, uuid::Uuid::new_v4());
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.phase, WakeIntentPhase::Accepted);
    assert!(row.cleanup_quarantined);
    assert_eq!(row.owned_ids, ["owned-id"]);
    assert_eq!(row.preparation, p);
    assert!(
        StorePort::record_owned_wake_ids(&store, &r.attempt, row.revision, &[], true, &budget())
            .unwrap()
    );
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.owned_ids, ["owned-id"]);
    assert!(row.cleanup_quarantined);
    assert_eq!(row.phase, WakeIntentPhase::Accepted);
}
#[test]
fn recovery_keeps_old_possible_effect_after_contact_frontier_and_retirement() {
    use herdr_threads::ports::WakeIntentPhase;
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let ticket =
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .unwrap();
    drop(store);
    let store = open(path, uuid::Uuid::new_v4());
    assert!(
        StorePort::recover_wake_intent(
            &store,
            &r.attempt,
            ticket.revision,
            r.daemon_boot,
            &budget()
        )
        .unwrap()
    );
    let before = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(before.phase, WakeIntentPhase::Unknown);
    assert!(before.cleanup_quarantined);
    let db = store.context().open_writer().unwrap();
    db.execute_batch("UPDATE local_node SET decision_seq=99; UPDATE occupant_bindings SET observed_at=999,liveness='disconnected',disconnected_at=999,disconnect_reason='seat_retired'; DELETE FROM wake_work; UPDATE seats SET state='retired',retired_at=999,generation=generation+1;").unwrap();
    drop(db);
    let after = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(after, before);
    let page =
        StorePort::unresolved_wake_intents_for_seat(&store, &r.seat, 0, 32, &budget()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].attempt, r.attempt);
    assert!(
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .is_none()
    );
}
#[test]
fn prepare_is_immutable_and_arm_rechecks_exact_canonical_scope() {
    let (store, _path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let mut changed = p.clone();
    changed.correlation = "replacement".into();
    assert!(StorePort::prepare_wake_intent(&store, &r, &changed, &budget()).is_err());
    let mut changed = p.clone();
    changed.target.descriptor = b"new-storage".to_vec();
    assert!(StorePort::prepare_wake_intent(&store, &r, &changed, &budget()).is_err());
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET execution_id='changed'", [])
        .unwrap();
    assert!(
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap(),
        first
    );
}
#[test]
fn only_one_connection_wins_arm() {
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let second = open(path, r.daemon_boot);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let won = std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
                .unwrap()
                .is_some()
        });
        let two = scope.spawn(|| {
            barrier.wait();
            StorePort::arm_wake_intent(&second, &r, first.revision, uuid::Uuid::new_v4(), &budget())
                .unwrap()
                .is_some()
        });
        usize::from(one.join().unwrap()) + usize::from(two.join().unwrap())
    });
    assert_eq!(won, 1);
}

fn reserve_again(store: &SqliteStore) -> herdr_threads::ports::WakeReservation {
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("DELETE FROM wake_work", [])
        .unwrap();
    let candidate = StorePort::wake_candidates(
        store,
        herdr_threads::test_support::test_host_id(),
        PageRequest::default(),
        &budget(),
    )
    .unwrap()
    .items
    .remove(0);
    StorePort::reserve_wake(
        store,
        herdr_threads::test_support::test_host_id(),
        &candidate,
        &budget(),
    )
    .unwrap()
    .unwrap()
}
#[test]
fn unresolved_cohort_blocks_reconnect_even_when_descriptor_changes() {
    let (store, _path, r, mut p) = fixture();
    let first = prepare(&store, &r, &p);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE occupant_bindings SET execution_id='new-execution',liveness_pid=20",
            [],
        )
        .unwrap();
    p.target.scope.execution = herdr_threads::protocol::ids::ExecutionId::new("new-execution");
    p.target.scope.process.pid = 20;
    p.target.descriptor = b"copied-or-relocated-profile".to_vec();
    let second = reserve_again(&store);
    assert!(matches!(
        StorePort::prepare_wake_intent(&store, &second, &p, &budget()).unwrap(),
        herdr_threads::ports::PrepareWakeIntent::Refused
    ));
    assert_eq!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap(),
        first
    );
    let other = "00000000-0000-0000-0000-000000000002";
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET native_session=?1", [other])
        .unwrap();
    p.target.scope.native_session = herdr_threads::protocol::ids::NativeSessionId::new(other);
    p.replay_scope =
        herdr_threads::harness::wake::WakeReplayScope::parse(format!("session:{other}")).unwrap();
    let third = reserve_again(&store);
    prepare(&store, &third, &p);
    assert_eq!(
        StorePort::unresolved_wake_intents_for_seat(&store, &r.seat, 0, 32, &budget())
            .unwrap()
            .items
            .len(),
        2
    );
}
#[test]
fn recovered_prepared_means_no_add_authority_but_unknown_helper_cleanup() {
    use herdr_threads::ports::WakeIntentPhase;
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    drop(store);
    let store = open(path.clone(), uuid::Uuid::new_v4());
    assert!(
        StorePort::recover_wake_intent(
            &store,
            &r.attempt,
            first.revision,
            r.daemon_boot,
            &budget()
        )
        .unwrap()
    );
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.phase, WakeIntentPhase::Prepared);
    assert!(row.cleanup_quarantined);
    assert!(
        !StorePort::recover_wake_intent(&store, &r.attempt, row.revision, r.daemon_boot, &budget())
            .unwrap()
    );
    assert!(
        !StorePort::refuse_prepared_wake_intent(
            &store,
            &r.attempt,
            row.revision,
            true,
            "",
            &budget()
        )
        .unwrap()
    );
    drop(store);
    let old_owner = open(path, r.daemon_boot);
    assert!(
        StorePort::arm_wake_intent(
            &old_owner,
            &r,
            row.revision,
            uuid::Uuid::new_v4(),
            &budget()
        )
        .unwrap()
        .is_none()
    );
    assert!(
        !StorePort::refuse_prepared_wake_intent(
            &old_owner,
            &r.attempt,
            row.revision,
            true,
            "",
            &budget()
        )
        .unwrap()
    );
}
#[test]
fn only_clean_no_effect_can_release_cohort_and_be_pruned_after_last_reference() {
    for clean in [false, true] {
        use herdr_threads::ports::{
            PrepareWakeIntent, WakeIntentOutcome, WakeIntentPhase, WakeIntentResult,
        };
        let (store, _path, r, p) = fixture();
        let first = prepare(&store, &r, &p);
        let ticket =
            StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
                .unwrap()
                .unwrap();
        assert!(
            StorePort::record_wake_intent_result(
                &store,
                &ticket,
                &WakeIntentResult {
                    outcome: WakeIntentOutcome::NoEffect,
                    cleanup_confirmed: clean,
                    diagnostic: String::new()
                },
                &budget()
            )
            .unwrap()
        );
        let row = StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, WakeIntentPhase::NoEffect);
        assert_eq!(row.cleanup_quarantined, !clean);
        assert!(matches!(
            StorePort::prepare_wake_intent(&store, &r, &p, &budget()).unwrap(),
            PrepareWakeIntent::Existing(_)
        ));
        let second = reserve_again(&store);
        let next = StorePort::prepare_wake_intent(&store, &second, &p, &budget()).unwrap();
        if clean {
            assert!(matches!(next, PrepareWakeIntent::Prepared(_)));
            assert!(
                StorePort::wake_intent(&store, &r.attempt, &budget())
                    .unwrap()
                    .is_none()
            );
            assert!(
                matches!(
                    StorePort::prepare_wake_intent(&store, &r, &p, &budget()).unwrap(),
                    PrepareWakeIntent::Refused
                ),
                "old cached reservation cannot resurrect a pruned attempt"
            );
        } else {
            assert!(matches!(next, PrepareWakeIntent::Refused));
            assert_eq!(
                StorePort::wake_intent(&store, &r.attempt, &budget())
                    .unwrap()
                    .unwrap(),
                row
            );
        }
    }
}
#[test]
fn owned_id_union_is_bounded_and_absence_never_erases_known_ids() {
    use herdr_threads::ports::{WakeIntentOutcome, WakeIntentResult};
    let (store, _path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let ticket =
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .unwrap();
    StorePort::record_wake_intent_result(
        &store,
        &ticket,
        &WakeIntentResult {
            outcome: WakeIntentOutcome::Accepted {
                id: "accepted".into(),
            },
            cleanup_confirmed: true,
            diagnostic: String::new(),
        },
        &budget(),
    )
    .unwrap();
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    let ids: Vec<String> = (0..127).map(|n| format!("id-{n}")).collect();
    assert!(
        StorePort::record_owned_wake_ids(&store, &r.attempt, row.revision, &ids, false, &budget())
            .unwrap()
    );
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.owned_ids.len(), 128);
    assert!(row.owned_ids.iter().any(|id| id == "accepted"));
    assert!(
        StorePort::record_owned_wake_ids(
            &store,
            &r.attempt,
            row.revision,
            &["129th".into()],
            true,
            &budget()
        )
        .is_err()
    );
    assert_eq!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap(),
        row
    );
    assert!(
        StorePort::record_owned_wake_ids(&store, &r.attempt, row.revision, &[], true, &budget())
            .unwrap()
    );
    let last = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(last.owned_ids, row.owned_ids);
    assert_eq!(last.phase, row.phase);
    assert!(last.ids_complete);
}
#[test]
fn every_frontier_component_round_trips_independently_and_is_frozen() {
    use herdr_threads::ports::{LogicalAttentionFrontier, LogicalPublicationKey};
    let (store, _path, r, p) = fixture();
    prepare(&store, &r, &p);
    store.context().open_writer().unwrap().execute("UPDATE wake_intents SET invitation_seq=11,invitation_offset=2,receipt_seq=7,receipt_offset=3,warning_seq=5,warning_offset=4,lazy_ordinal=9",[]).unwrap();
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(
        row.frontier,
        LogicalAttentionFrontier {
            invitation: Some(LogicalPublicationKey {
                decision_seq: 11,
                event_offset: 2
            }),
            addressed_receipt: Some(LogicalPublicationKey {
                decision_seq: 7,
                event_offset: 3
            }),
            actionable_warning: Some(LogicalPublicationKey {
                decision_seq: 5,
                event_offset: 4
            }),
            lazy: Some(9)
        }
    );
    assert!(StorePort::prepare_wake_intent(&store, &r, &p, &budget()).is_err());
}
#[test]
fn accepted_id_and_cleanup_result_roll_back_together_on_sql_failure() {
    use herdr_threads::ports::{WakeIntentOutcome, WakeIntentResult};
    let (store, path, r, p) = fixture();
    let first = prepare(&store, &r, &p);
    let ticket =
        StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
            .unwrap()
            .unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("CREATE TRIGGER injected_result_failure BEFORE UPDATE ON wake_intents WHEN NEW.phase='accepted' BEGIN SELECT RAISE(ABORT,'injected result failure'); END;").unwrap();
    assert!(
        StorePort::record_wake_intent_result(
            &store,
            &ticket,
            &WakeIntentResult {
                outcome: WakeIntentOutcome::Accepted { id: "owned".into() },
                cleanup_confirmed: false,
                diagnostic: "cleanup uncertain".into()
            },
            &budget()
        )
        .is_err()
    );
    db.execute_batch("DROP TRIGGER injected_result_failure")
        .unwrap();
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(row.phase, herdr_threads::ports::WakeIntentPhase::Armed);
    assert!(row.owned_ids.is_empty());
    assert!(!row.cleanup_quarantined);
    assert_eq!(row.revision, ticket.revision);
}
#[test]
fn arm_refuses_changed_scope_and_already_served_attention() {
    let changes = [
        "UPDATE occupant_bindings SET native_session='different'",
        "UPDATE occupant_bindings SET liveness_pid=11",
        "UPDATE occupant_bindings SET liveness_start=2",
        "UPDATE occupant_bindings SET liveness_boot='different'",
        "UPDATE node_kernel_boot SET boot='different'",
        "UPDATE occupant_bindings SET ended_at=101",
        "UPDATE occupant_bindings SET registered_at=NULL",
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=101,disconnect_reason='liveness_end_event'",
        "INSERT INTO in_turn_offer(binding_id,seat_id,item_id) VALUES('binding','s','inv')",
    ];
    for change in changes {
        let (store, _path, r, p) = fixture();
        let first = prepare(&store, &r, &p);
        store
            .context()
            .open_writer()
            .unwrap()
            .execute_batch(change)
            .unwrap();
        assert!(
            StorePort::arm_wake_intent(&store, &r, first.revision, uuid::Uuid::new_v4(), &budget())
                .unwrap()
                .is_none(),
            "{change}"
        );
    }
}
#[test]
fn prepared_refusal_preserves_explicit_cleanup_uncertainty() {
    for clean in [false, true] {
        let (store, path, r, p) = fixture();
        let first = prepare(&store, &r, &p);
        assert!(
            StorePort::refuse_prepared_wake_intent(
                &store,
                &r.attempt,
                first.revision,
                clean,
                "pre-add refusal",
                &budget()
            )
            .unwrap()
        );
        drop(store);
        let store = open(path, uuid::Uuid::new_v4());
        let row = StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap();
        assert_eq!(row.phase, herdr_threads::ports::WakeIntentPhase::NoEffect);
        assert_eq!(row.cleanup_quarantined, !clean);
        assert_eq!(row.diagnostic, "pre-add refusal");
    }
}
#[test]
fn binary_descriptor_bound_and_input_limits_are_enforced_before_preparation() {
    let (store, _path, r, mut p) = fixture();
    p.target.descriptor = vec![255; 16385];
    assert!(StorePort::prepare_wake_intent(&store, &r, &p, &budget()).is_err());
    p.target.descriptor = vec![255; 16384];
    p.correlation = "c".repeat(129);
    assert!(StorePort::prepare_wake_intent(&store, &r, &p, &budget()).is_err());
    p.correlation = "c".repeat(128);
    let first = prepare(&store, &r, &p);
    assert_eq!(first.preparation.target.descriptor.len(), 16384);
    assert_eq!(first.preparation.target.descriptor[0], 255);
    assert!(StorePort::wake_intent_recovery_page(&store, 0, 0, &budget()).is_err());
    assert!(StorePort::wake_intent_recovery_page(&store, 0, 33, &budget()).is_err());
}

#[test]
fn retained_cap_refuses_new_effect_and_recovery_pages_are_bounded() {
    use herdr_threads::ports::PrepareWakeIntent;
    let (store, _path, r, mut p) = fixture();
    let first = prepare(&store, &r, &p);
    let db = store.context().open_writer().unwrap();
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(wake_intents)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let native = "printf('00000000-0000-0000-0000-%012d',n+10)";
    let select: Vec<String> = columns
        .iter()
        .map(|c| match c.as_str() {
            "ordinal" => "NULL".into(),
            "attempt" => "'bulk-'||n".into(),
            "replay_scope" => format!("'session:'||{native}"),
            "target_json" => {
                format!("CAST(json_set(base.target_json,'$.native_session',{native}) AS BLOB)")
            }
            "phase" => "'unknown'".into(),
            "arm_nonce" => "'00000000-0000-0000-0000-000000000003'".into(),
            _ => format!("base.{c}"),
        })
        .collect();
    db.execute(&format!("WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<4095) INSERT INTO wake_intents({}) SELECT {} FROM wake_intents base CROSS JOIN seq WHERE base.ordinal=?1",columns.join(","),select.join(",")),[i64::try_from(first.ordinal).unwrap()]).unwrap();
    drop(db);
    let a = StorePort::wake_intent_recovery_page(&store, 0, 32, &budget()).unwrap();
    assert_eq!(a.items.len(), 32);
    let b =
        StorePort::wake_intent_recovery_page(&store, a.next_after.unwrap(), 32, &budget()).unwrap();
    assert_eq!(b.items.len(), 32);
    assert!(a.items.last().unwrap().ordinal < b.items[0].ordinal);
    assert_eq!(
        StorePort::unresolved_wake_intents_for_seat(&store, &r.seat, 0, 32, &budget()).unwrap(),
        a
    );
    let native = "00000000-0000-0000-0000-000000000002";
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET native_session=?1", [native])
        .unwrap();
    p.target.scope.native_session = herdr_threads::protocol::ids::NativeSessionId::new(native);
    p.replay_scope =
        herdr_threads::harness::wake::WakeReplayScope::parse(format!("session:{native}")).unwrap();
    let second = reserve_again(&store);
    assert!(matches!(
        StorePort::prepare_wake_intent(&store, &second, &p, &budget()).unwrap(),
        PrepareWakeIntent::Refused
    ));
    assert!(
        StorePort::wake_intent(&store, &second.attempt, &budget())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap(),
        first
    );
    // Remove one fixture row only: space is now available, with the same canonical request.
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("DELETE FROM wake_intents WHERE attempt='bulk-4095'", [])
        .unwrap();
    assert!(matches!(
        StorePort::prepare_wake_intent(&store, &second, &p, &budget()).unwrap(),
        PrepareWakeIntent::Prepared(_)
    ));
}

#[test]
fn schema_audit_rejects_weakened_ledger_index() {
    let (store, _path, r, p) = fixture();
    prepare(&store, &r, &p);
    let db = store.context().open_writer().unwrap();
    db.execute_batch("DROP INDEX wake_intents_unresolved_cohort; CREATE INDEX wake_intents_unresolved_cohort ON wake_intents(instance_id);").unwrap();
    assert!(herdr_threads::store::schema::verify_baseline(&db).is_err());
}

#[test]
fn missing_canonical_scope_refuses_first_preparation_without_a_row() {
    for change in [
        "UPDATE occupant_bindings SET execution_id='changed'",
        "UPDATE occupant_bindings SET liveness_pid=11",
        "UPDATE occupant_bindings SET registered_at=NULL",
        "INSERT INTO in_turn_offer(binding_id,seat_id,item_id) VALUES('binding','s','inv')",
    ] {
        let (store, _path, r, p) = fixture();
        store
            .context()
            .open_writer()
            .unwrap()
            .execute_batch(change)
            .unwrap();
        assert!(
            matches!(
                StorePort::prepare_wake_intent(&store, &r, &p, &budget()).unwrap(),
                herdr_threads::ports::PrepareWakeIntent::Refused
            ),
            "{change}"
        );
        assert!(
            StorePort::wake_intent(&store, &r.attempt, &budget())
                .unwrap()
                .is_none()
        );
    }
}

fn native_report(
    store: &SqliteStore,
    r: &herdr_threads::ports::WakeReservation,
    p: &herdr_threads::ports::WakeIntentPreparation,
    id: &str,
    edge: herdr_threads::harness::adapter::TurnEdge,
) -> herdr_threads::protocol::commands::TurnStateReport {
    use herdr_threads::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        ids::{HostLocator, NativeTurnId},
    };
    let host:String=store.context().open_writer().unwrap().query_row("SELECT endpoint FROM herdr_hosts WHERE host_id=(SELECT host_id FROM seats WHERE id='s')",[],|r|r.get(0)).unwrap();
    herdr_threads::protocol::commands::TurnStateReport {
        claim: CallerClaim {
            instance: INSTANCE.into(),
            host: HostLocator::parse(host).unwrap(),
            seat: r.seat.clone(),
            binding_generation: p.target.scope.generation,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: p.target.scope.native_session.clone(),
            execution: p.target.scope.execution.clone(),
            target: r.target.clone(),
            agent_id: None,
            liveness: Some(p.target.scope.process),
        },
        edge,
        tmux_hint: None,
        native_turn_id: Some(NativeTurnId::new(id)),
    }
}
#[test]
fn native_turn_current_retries_end_first_and_mismatched_end_are_idempotent() {
    use herdr_threads::{harness::adapter::TurnEdge, ports::NativeTurnPhase};
    let (store, _path, r, p) = fixture();
    let observe = |id, edge| {
        StorePort::record_native_turn(&store, &native_report(&store, &r, &p, id, edge), &budget())
            .unwrap()
    };
    assert!(observe("a", TurnEdge::Busy).changed);
    assert!(!observe("a", TurnEdge::Busy).changed);
    assert!(observe("a", TurnEdge::Idle).changed);
    assert!(!observe("a", TurnEdge::Busy).changed);
    let current = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(current.ordinal, 2);
    assert_eq!(current.phase, NativeTurnPhase::Ended);
    assert!(observe("b", TurnEdge::Busy).changed);
    assert!(observe("a", TurnEdge::Idle).edge.is_none());
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap()
            .ordinal,
        3
    );
    assert!(observe("b", TurnEdge::Idle).changed);
    assert!(observe("c", TurnEdge::Idle).changed);
    assert!(!observe("c", TurnEdge::Busy).changed);
    let current = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(current.phase, NativeTurnPhase::EndFirst);
    assert_eq!(current.ordinal, 5);
}
#[test]
fn native_turn_restart_preserves_current_id_and_scope_change_resets_qualification() {
    use herdr_threads::{harness::adapter::TurnEdge, ports::NativeTurnPhase};
    let (store, path, r, p) = fixture();
    let start = native_report(&store, &r, &p, "turn", TurnEdge::Busy);
    StorePort::record_native_turn(&store, &start, &budget()).unwrap();
    drop(store);
    let store = open(path, r.daemon_boot);
    assert!(
        !StorePort::record_native_turn(&store, &start, &budget())
            .unwrap()
            .changed
    );
    let mut end = start.clone();
    end.edge = TurnEdge::Idle;
    StorePort::record_native_turn(&store, &end, &budget()).unwrap();
    let before = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(before.phase, NativeTurnPhase::Ended);
    let db = store.context().open_writer().unwrap();
    db.execute("UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=101,disconnect_reason='liveness_end_event'", []).unwrap();
    assert!(
        StorePort::record_native_turn(&store, &start, &budget())
            .unwrap()
            .edge
            .is_none()
    );
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap(),
        before,
        "disconnect is not canonical binding finality"
    );
    db.execute("UPDATE occupant_bindings SET liveness='connected',disconnected_at=NULL,disconnect_reason=NULL", []).unwrap();
    drop(db);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET liveness_pid=20", [])
        .unwrap();
    assert!(
        StorePort::record_native_turn(&store, &start, &budget())
            .unwrap()
            .edge
            .is_none()
    );
    let mut replacement = start;
    replacement.claim.liveness.as_mut().unwrap().pid = 20;
    replacement.native_turn_id = Some(herdr_threads::protocol::ids::NativeTurnId::new("new-scope"));
    assert!(
        StorePort::record_native_turn(&store, &replacement, &budget())
            .unwrap()
            .changed
    );
    let after = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(after.phase, NativeTurnPhase::Started);
    assert!(after.ordinal > before.ordinal);
    assert_eq!(after.scope.process.pid, 20);
}
#[test]
fn native_turn_never_changes_intents_or_qualifies_invalid_claims() {
    use herdr_threads::harness::adapter::TurnEdge;
    let (store, _path, r, p) = fixture();
    let before = prepare(&store, &r, &p);
    let good = native_report(&store, &r, &p, "turn", TurnEdge::Busy);
    for index in 0..4 {
        let mut bad = good.clone();
        match index {
            0 => bad.claim.execution = herdr_threads::protocol::ids::ExecutionId::new("foreign"),
            1 => bad.claim.binding_generation += 1,
            2 => bad.claim.liveness.as_mut().unwrap().start += 1,
            _ => {
                bad.claim.native_session =
                    herdr_threads::protocol::ids::NativeSessionId::new("sibling")
            }
        }
        assert!(
            StorePort::record_native_turn(&store, &bad, &budget())
                .unwrap()
                .edge
                .is_none()
        );
    }
    StorePort::record_native_turn(&store, &good, &budget()).unwrap();
    assert_eq!(
        StorePort::wake_intent(&store, &r.attempt, &budget())
            .unwrap()
            .unwrap(),
        before
    );
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET ended_at=101", [])
        .unwrap();
    assert!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .is_none()
    );
    assert!(
        StorePort::record_native_turn(&store, &good, &budget())
            .unwrap()
            .edge
            .is_none()
    );
}

#[test]
fn native_turn_missing_metadata_invalidates_qualification_without_forgetting_id() {
    use herdr_threads::{harness::adapter::TurnEdge, ports::NativeTurnPhase};
    let (store, _path, r, p) = fixture();
    let mut report = native_report(&store, &r, &p, "a", TurnEdge::Busy);
    StorePort::record_native_turn(&store, &report, &budget()).unwrap();
    report.edge = TurnEdge::Idle;
    StorePort::record_native_turn(&store, &report, &budget()).unwrap();
    report.native_turn_id = None;
    report.edge = TurnEdge::Busy;
    assert!(
        StorePort::record_native_turn(&store, &report, &budget())
            .unwrap()
            .changed
    );
    let state = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(state.phase, NativeTurnPhase::Unknown);
    assert_eq!(state.id.as_str(), "a");
    assert_eq!(state.ordinal, 3);
    report.native_turn_id = Some(herdr_threads::protocol::ids::NativeTurnId::new("a"));
    assert!(
        StorePort::record_native_turn(&store, &report, &budget())
            .unwrap()
            .edge
            .is_none()
    );
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap(),
        state
    );
}
#[test]
fn native_turn_first_end_has_no_matched_cycle_and_counter_cannot_wrap() {
    use herdr_threads::{harness::adapter::TurnEdge, ports::NativeTurnPhase};
    let (store, _path, r, p) = fixture();
    let mut report = native_report(&store, &r, &p, "end-first", TurnEdge::Idle);
    StorePort::record_native_turn(&store, &report, &budget()).unwrap();
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap()
            .phase,
        NativeTurnPhase::EndFirst
    );
    report.edge = TurnEdge::Busy;
    assert!(
        !StorePort::record_native_turn(&store, &report, &budget())
            .unwrap()
            .changed
    );
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE native_turn_state SET observation_ordinal=9223372036854775807",
            [],
        )
        .unwrap();
    report.native_turn_id = Some(herdr_threads::protocol::ids::NativeTurnId::new("later"));
    assert!(StorePort::record_native_turn(&store, &report, &budget()).is_err());
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap()
            .ordinal,
        i64::MAX as u64
    );
}
#[test]
fn native_turn_refuses_host_hold_child_and_missing_live_scope() {
    use herdr_threads::{harness::adapter::TurnEdge, protocol::authority::CallerRole};
    let (store, _path, r, p) = fixture();
    let good = native_report(&store, &r, &p, "a", TurnEdge::Busy);
    for index in 0..4 {
        let mut bad = good.clone();
        match index {
            0 => {
                bad.claim.host =
                    herdr_threads::protocol::ids::HostLocator::parse("/different.sock").unwrap()
            }
            1 => bad.claim.role = CallerRole::Subagent,
            2 => bad.claim.liveness = None,
            _ => bad.claim.agent_id = Some(herdr_threads::protocol::ids::AgentId::new("child")),
        }
        assert!(
            StorePort::record_native_turn(&store, &bad, &budget())
                .unwrap()
                .edge
                .is_none()
        );
    }
    let db = store.context().open_writer().unwrap();
    db.execute("INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES((SELECT min(host_id) FROM herdr_hosts),'pane','host',1,'test')",[]).unwrap();
    assert!(
        StorePort::record_native_turn(&store, &good, &budget())
            .unwrap()
            .edge
            .is_none()
    );
    db.execute("DELETE FROM recovery_holds", []).unwrap();
    db.execute("UPDATE node_kernel_boot SET boot='different'", [])
        .unwrap();
    assert!(
        StorePort::record_native_turn(&store, &good, &budget())
            .unwrap()
            .edge
            .is_none()
    );
}
#[test]
fn native_turn_capacity_never_evicts_a_current_binding() {
    use herdr_threads::harness::adapter::TurnEdge;
    let (store, _path, r, p) = fixture();
    let start = native_report(&store, &r, &p, "a", TurnEdge::Busy);
    StorePort::record_native_turn(&store, &start, &budget()).unwrap();
    let db = store.context().open_writer().unwrap();
    db.execute("WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<4095) INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,binding_id,terminal_id,incarnation) SELECT 's',1,1,'pane','host',0,'codex','bulk-'||n,'execution','cooperative_top_level',0,'bulk-'||n,'term-pane','inc' FROM seq",[]).unwrap();
    db.execute("WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<4095) INSERT INTO native_turn_state(binding_id,instance_id,scope_json,turn_id,phase,observation_ordinal) SELECT 'bulk-'||n,base.instance_id,CAST(json_set(base.scope_json,'$.binding','bulk-'||n,'$.native_session','bulk-'||n) AS BLOB),'bulk-turn','started',1 FROM native_turn_state base CROSS JOIN seq WHERE base.binding_id='binding'",[]).unwrap();
    let before = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    let mut end = start.clone();
    end.edge = TurnEdge::Idle;
    assert!(
        StorePort::record_native_turn(&store, &end, &budget())
            .unwrap()
            .changed,
        "existing state remains writable at capacity"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_turn_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4096
    );
    db.execute(
        "DELETE FROM native_turn_state WHERE binding_id='binding'",
        [],
    )
    .unwrap();
    // Replace that slot with another live canonical binding, leaving the subject untracked.
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,binding_id,terminal_id,incarnation) VALUES('s',1,1,'pane','host',0,'codex','last','execution','cooperative_top_level',0,'last','term-pane','inc')",[]).unwrap();
    db.execute("INSERT INTO native_turn_state SELECT 'last',instance_id,CAST(json_set(scope_json,'$.binding','last','$.native_session','last') AS BLOB),turn_id,phase,observation_ordinal FROM native_turn_state WHERE binding_id='bulk-1'",[]).unwrap();
    assert!(
        !StorePort::record_native_turn(&store, &start, &budget())
            .unwrap()
            .changed
    );
    assert!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .is_none()
    );
    assert_eq!(before.id.as_str(), "a");
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_turn_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4096
    );
}

#[test]
fn native_turn_domain_updates_memory_only_after_canonical_acceptance() {
    use herdr_threads::{
        harness::adapter::TurnEdge,
        ports::LocalService,
        protocol::{commands::Command, results::CommandResult},
        service::{
            dispatch::DomainService,
            fair_writer::FairWriter,
            turn_state::{TurnKey, TurnState},
        },
    };
    let (store, _path, r, p) = fixture();
    let valid = native_report(&store, &r, &p, "a", TurnEdge::Busy);
    let service = DomainService::new(INSTANCE.into(), Arc::new(store), Arc::new(IntentClock))
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    let key = TurnKey {
        seat: r.seat.clone(),
        generation: 1,
        harness: "codex".into(),
        native_session: p.target.scope.native_session.clone(),
    };
    let mut invalid = valid.clone();
    invalid.claim.execution = herdr_threads::protocol::ids::ExecutionId::new("foreign");
    let peer = || herdr_threads::test_support::peer_identity(501);
    assert!(matches!(
        service
            .handle(Command::TurnState(invalid), peer(), &budget())
            .unwrap(),
        CommandResult::TurnState { recorded: false }
    ));
    assert_eq!(service.turn_state().state(&key), TurnState::Unknown);
    assert!(matches!(
        service
            .handle(Command::TurnState(valid.clone()), peer(), &budget())
            .unwrap(),
        CommandResult::TurnState { recorded: true }
    ));
    assert_eq!(service.turn_state().state(&key), TurnState::Busy);
    assert!(matches!(
        service
            .handle(Command::TurnState(valid.clone()), peer(), &budget())
            .unwrap(),
        CommandResult::TurnState { recorded: false }
    ));
    let mut wrong_end = valid.clone();
    wrong_end.edge = TurnEdge::Idle;
    wrong_end.native_turn_id = Some(herdr_threads::protocol::ids::NativeTurnId::new(
        "sibling-turn",
    ));
    assert!(matches!(
        service
            .handle(Command::TurnState(wrong_end), peer(), &budget())
            .unwrap(),
        CommandResult::TurnState { recorded: false }
    ));
    assert_eq!(service.turn_state().state(&key), TurnState::Busy);
    let mut end = valid;
    end.edge = TurnEdge::Idle;
    service
        .handle(Command::TurnState(end), peer(), &budget())
        .unwrap();
    assert_eq!(service.turn_state().state(&key), TurnState::Idle);
}

#[test]
fn native_turn_concurrent_current_id_has_one_accepted_observation() {
    use herdr_threads::harness::adapter::TurnEdge;
    let (store, path, r, p) = fixture();
    let second = open(path, r.daemon_boot);
    let report = native_report(&store, &r, &p, "same", TurnEdge::Busy);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let changes = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            StorePort::record_native_turn(&store, &report, &budget())
                .unwrap()
                .changed
        });
        let b = scope.spawn(|| {
            barrier.wait();
            StorePort::record_native_turn(&second, &report, &budget())
                .unwrap()
                .changed
        });
        usize::from(a.join().unwrap()) + usize::from(b.join().unwrap())
    });
    assert_eq!(changes, 1);
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap()
            .ordinal,
        1
    );
}
#[test]
fn native_turn_retains_only_current_identity_and_accepts_observation_order_limit() {
    use herdr_threads::harness::adapter::TurnEdge;
    let (store, _path, r, p) = fixture();
    for id in ["a", "b"] {
        for edge in [TurnEdge::Busy, TurnEdge::Idle] {
            StorePort::record_native_turn(
                &store,
                &native_report(&store, &r, &p, id, edge),
                &budget(),
            )
            .unwrap();
        }
    }
    assert!(
        StorePort::record_native_turn(
            &store,
            &native_report(&store, &r, &p, "a", TurnEdge::Busy),
            &budget()
        )
        .unwrap()
        .changed
    );
    let current = StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(current.ordinal, 5);
    assert_eq!(current.id.as_str(), "a");
    assert_eq!(
        store
            .context()
            .open_writer()
            .unwrap()
            .query_row("SELECT count(*) FROM native_turn_state", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn native_turn_cancelled_recording_changes_no_observation() {
    use herdr_threads::harness::adapter::TurnEdge;
    let (store, _path, r, p) = fixture();
    let call = budget();
    call.cancellation.cancel();
    assert!(
        StorePort::record_native_turn(
            &store,
            &native_report(&store, &r, &p, "a", TurnEdge::Busy),
            &call
        )
        .is_err()
    );
    assert!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .is_none()
    );
}
