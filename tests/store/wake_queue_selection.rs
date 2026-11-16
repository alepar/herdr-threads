use super::wake_intents::{budget, fixture, native_report, prepare_raw};
use herdr_threads::{harness::adapter::TurnEdge, ports::StorePort};

#[test]
fn selection_requires_exact_matched_ended_unique_binding() {
    let (store, _path, reservation, preparation) = fixture();
    assert!(
        StorePort::queue_wake_selection(&store, &reservation, &budget())
            .unwrap()
            .binding
            .is_none()
    );
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        StorePort::record_native_turn(
            &store,
            &native_report(&store, &reservation, &preparation, "baseline", edge),
            &budget(),
        )
        .unwrap();
    }
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    let binding = selected.binding.unwrap();
    assert_eq!(binding.scope, preparation.target.scope);
    assert_eq!(binding.replay_scope, preparation.replay_scope);
    assert!(!selected.blockers.programmatic);
    assert!(!selected.blockers.native);
}

#[test]
fn origin_fence_survives_unqualified_and_disconnected_binding() {
    let (store, _path, reservation, preparation) = fixture();
    prepare_raw(&store, &reservation, &preparation);
    for disconnect in [false, true] {
        if disconnect {
            store.context().open_writer().unwrap().execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) VALUES ('s',1,1,'pane','host',0,'codex','invalid-current','replacement','cooperative_top_level',0,0,'term-pane','inc','replacement')", []).unwrap();
            store.context().open_writer().unwrap().execute("UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=100,disconnect_reason='liveness_process_exit' WHERE binding_id='binding'", []).unwrap();
        }
        let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
        assert!(selected.binding.is_none());
        assert!(selected.blockers.programmatic);
        assert!(selected.blockers.native);
    }
}

#[test]
fn cohort_fence_survives_different_origin_and_invalid_current_session() {
    let (store, _path, reservation, preparation) = fixture();
    prepare_raw(&store, &reservation, &preparation);
    let db = store.context().open_writer().unwrap();
    // Retained ledger origins need not have a live seat row.
    db.execute("UPDATE wake_intents SET origin_seat='old-origin'", [])
        .unwrap();
    drop(db);
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert!(selected.binding.is_none());
    assert!(selected.blockers.programmatic);
    assert!(selected.blockers.native);
    // The disconnected prior binding remains in the cohort census while a
    // current unsupported session is ineligible for programmatic selection.
    let db = store.context().open_writer().unwrap();
    db.execute("UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=100,disconnect_reason='liveness_process_exit' WHERE binding_id='binding'", []).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) VALUES ('s',1,1,'pane','host',0,'codex','invalid-session','other','cooperative_top_level',0,0,'term-pane','inc','other-binding')", []).unwrap();
    drop(db);
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert!(selected.binding.is_none());
    assert!(selected.blockers.programmatic);
    assert!(selected.blockers.native);
}

#[test]
fn multiple_connected_sessions_cannot_select_a_queue_binding() {
    let (store, _path, reservation, preparation) = fixture();
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        StorePort::record_native_turn(
            &store,
            &native_report(&store, &reservation, &preparation, "baseline", edge),
            &budget(),
        )
        .unwrap();
    }
    prepare_raw(&store, &reservation, &preparation);
    store.context().open_writer().unwrap().execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) VALUES ('s',1,1,'pane','host',0,'codex','00000000-0000-0000-0000-000000000002','sibling','cooperative_top_level',0,0,'term-pane','inc','sibling')", []).unwrap();
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert!(selected.binding.is_none());
    assert!(selected.blockers.programmatic);
    assert!(selected.blockers.native);
}

#[test]
fn end_first_and_stale_reservation_cannot_authorize_selection() {
    let (store, _path, reservation, preparation) = fixture();
    StorePort::record_native_turn(
        &store,
        &native_report(
            &store,
            &reservation,
            &preparation,
            "end-first",
            TurnEdge::Idle,
        ),
        &budget(),
    )
    .unwrap();
    assert!(
        StorePort::queue_wake_selection(&store, &reservation, &budget())
            .unwrap()
            .binding
            .is_none()
    );
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("DELETE FROM wake_work", [])
        .unwrap();
    let selection = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert!(selection.binding.is_none());
    assert!(!selection.blockers.native);
}

#[test]
fn large_binding_population_does_not_manufacture_effect_fences() {
    let (store, _path, reservation, _preparation) = fixture();
    store.context().open_writer().unwrap().execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<64) INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) SELECT 's',1,1,'pane','host',0,'codex','extra-'||x,'extra-'||x,'cooperative_top_level',0,0,'term-pane','inc','extra-'||x FROM n;").unwrap();
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert!(selected.binding.is_none());
    assert!(!selected.blockers.programmatic);
    assert!(!selected.blockers.native);
}

#[test]
fn ended_binding_history_does_not_saturate_current_cohort_census() {
    let (store, _path, reservation, preparation) = fixture();
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        StorePort::record_native_turn(
            &store,
            &native_report(&store, &reservation, &preparation, "baseline", edge),
            &budget(),
        )
        .unwrap();
    }
    store.context().open_writer().unwrap().execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100) INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id,ended_at) SELECT 's',1,1,'pane','host',0,'codex','historical-'||x,'historical-'||x,'cooperative_top_level',0,0,'term-pane','inc','historical-'||x,1 FROM n;").unwrap();
    let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
    assert_eq!(selected.binding.unwrap().scope, preparation.target.scope);
    assert!(!selected.blockers.programmatic);
    assert!(!selected.blockers.native);
}

#[test]
fn native_only_large_population_preserves_real_origin_fence() {
    for pending in [false, true] {
        let (store, _path, reservation, preparation) = fixture();
        if pending {
            prepare_raw(&store, &reservation, &preparation);
        }
        let db = store.context().open_writer().unwrap();
        db.execute("UPDATE occupant_bindings SET harness='claude'", [])
            .unwrap();
        db.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100) INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id) SELECT 's',1,1,'pane','host',0,'claude','historical-'||x,'historical-'||x,'cooperative_top_level',0,0,'term-pane','inc','historical-'||x FROM n;").unwrap();
        drop(db);
        let selected = StorePort::queue_wake_selection(&store, &reservation, &budget()).unwrap();
        assert!(selected.binding.is_none());
        assert_eq!(selected.blockers.native, pending);
        assert_eq!(selected.blockers.programmatic, pending);
    }
}
