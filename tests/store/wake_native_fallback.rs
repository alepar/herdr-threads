use super::wake_intents::{budget, fixture, native_report, prepare, reserve_again};
use herdr_threads::{
    harness::adapter::TurnEdge,
    ports::{
        NativeFallbackOutcome, NativeTurnPhase, StorePort, WakeIntentLogical, WakeIntentOutcome,
        WakeIntentPreparation, WakeIntentResult, WakeReservation,
    },
    store::SqliteStore,
};
use std::path::PathBuf;
pub(super) fn accepted() -> (SqliteStore, PathBuf, WakeReservation, WakeIntentPreparation) {
    let (store, path, r, p) = fixture();
    let row = prepare(&store, &r, &p);
    let arm = StorePort::arm_wake_intent(&store, &r, row.revision, uuid::Uuid::new_v4(), &budget())
        .unwrap()
        .unwrap();
    assert!(
        StorePort::record_wake_intent_result(
            &store,
            &arm,
            &WakeIntentResult {
                outcome: WakeIntentOutcome::Accepted {
                    id: "retained-owned".into()
                },
                cleanup_confirmed: true,
                diagnostic: String::new()
            },
            &budget()
        )
        .unwrap()
    );
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET armed_at_utc=armed_at_utc-30000",
            [],
        )
        .unwrap();
    (store, path, r, p)
}
fn attention(store: &SqliteStore, seq: i64) {
    let db = store.context().open_writer().unwrap();
    // Preserve immutable publication keys. Hide other pending rows, then offer
    // either an older existing invitation or a newly published p0 episode.
    db.execute(
        "UPDATE local_node SET decision_seq=max(decision_seq,?1)",
        [seq],
    )
    .unwrap();
    db.execute("UPDATE invitations SET state='already_joined'", [])
        .unwrap();
    let id = if seq == 1 {
        "inv".to_owned()
    } else {
        format!("inv-{seq}")
    };
    db.execute("INSERT OR IGNORE INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'t','s',?2,'pending',?2,0,100,100,(SELECT id FROM local_node))",rusqlite::params![id,seq]).unwrap();
    db.execute("UPDATE invitations SET state='pending' WHERE id=?1", [id])
        .unwrap();
}
fn claim(store: &SqliteStore) -> (WakeReservation, uuid::Uuid, bool) {
    let r = reserve_again(store);
    let ticket = uuid::Uuid::new_v4();
    let granted = StorePort::claim_native_wake_fallback(store, &r, ticket, &budget()).unwrap();
    (r, ticket, granted)
}
#[test]
fn newer_frontier_rearms_once_and_old_item_reappearance_does_not() {
    let (store, _path, _r, _p) = accepted();
    let (first, ticket, granted) = claim(&store);
    assert!(granted);
    assert!(
        StorePort::finish_native_wake_fallback(
            &store,
            &first,
            ticket,
            NativeFallbackOutcome::Unknown,
            &budget()
        )
        .unwrap()
    );
    assert!(
        !claim(&store).2,
        "new reservation alone is not new attention"
    );
    attention(&store, 200);
    let (newer, new_ticket, granted) = claim(&store);
    assert!(granted);
    assert!(!StorePort::claim_native_wake_fallback(&store, &newer, new_ticket, &budget()).unwrap());
    assert!(
        StorePort::finish_native_wake_fallback(
            &store,
            &newer,
            new_ticket,
            NativeFallbackOutcome::Submitted,
            &budget()
        )
        .unwrap()
    );
    for old in [1, 100, 200] {
        attention(&store, old);
        let candidates = StorePort::wake_candidates(
            &store,
            herdr_threads::test_support::test_host_id(),
            herdr_threads::protocol::pagination::PageRequest::default(),
            &budget(),
        )
        .unwrap();
        // Old invitation episodes can be suppressed by canonical invitation
        // semantics before fallback admission; any still offered projection
        // must also remain covered by the durable fallback high-water.
        if !candidates.items.is_empty() {
            assert!(!claim(&store).2, "old projection {old} rearmed");
        }
    }
    attention(&store, 300);
    let (unsent, unsent_ticket, granted) = claim(&store);
    assert!(granted);
    assert!(
        StorePort::finish_native_wake_fallback(
            &store,
            &unsent,
            unsent_ticket,
            NativeFallbackOutcome::NoEffect,
            &budget()
        )
        .unwrap()
    );
    assert!(
        claim(&store).2,
        "proven unsent new attention must remain retryable"
    );
    assert!(
        !StorePort::finish_native_wake_fallback(
            &store,
            &first,
            ticket,
            NativeFallbackOutcome::NoEffect,
            &budget()
        )
        .unwrap()
    );
}
#[test]
fn missing_completion_spends_coverage_and_late_completion_cannot_release_newer_claim() {
    let (store, _path, _r, _p) = accepted();
    let (first, ticket, granted) = claim(&store);
    assert!(granted);
    assert!(
        !claim(&store).2,
        "missing callback is not proof of no effect"
    );
    attention(&store, 200);
    assert!(claim(&store).2);
    assert!(
        !StorePort::finish_native_wake_fallback(
            &store,
            &first,
            ticket,
            NativeFallbackOutcome::NoEffect,
            &budget()
        )
        .unwrap()
    );
    assert!(
        !claim(&store).2,
        "late old completion released new coverage"
    );
}
#[test]
fn missing_arm_timestamp_and_wrong_owner_never_grant_or_release() {
    let (store, _path, _r, _p) = accepted();
    let db = store.context().open_writer().unwrap();
    db.execute("UPDATE wake_intents SET armed_at_utc=NULL", [])
        .unwrap();
    drop(db);
    assert!(!claim(&store).2);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE wake_intents SET armed_at_utc=-29900", [])
        .unwrap();
    let (r, ticket, granted) = claim(&store);
    assert!(granted);
    let mut wrong = r.clone();
    wrong.daemon_boot = uuid::Uuid::new_v4();
    assert!(
        !StorePort::finish_native_wake_fallback(
            &store,
            &wrong,
            ticket,
            NativeFallbackOutcome::NoEffect,
            &budget()
        )
        .unwrap()
    );
    assert!(!claim(&store).2);
}
fn second_history(store: &SqliteStore) {
    let db = store.context().open_writer().unwrap();
    let mut q = db.prepare("PRAGMA table_info(wake_intents)").unwrap();
    let names = q
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(q);
    let names = names
        .into_iter()
        .filter(|n| n != "ordinal")
        .collect::<Vec<_>>();
    let columns = names
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",");
    let values=names.iter().map(|n|match n.as_str(){
        "attempt"|"correlation"=>"'other-history'".into(),
        "replay_scope"=>"'session:00000000-0000-0000-0000-000000000002'".into(),
        "target_json"=>"CAST(json_set(target_json,'$.native_session','00000000-0000-0000-0000-000000000002','$.binding','old-other-binding') AS BLOB)".into(),
        _=>format!("\"{n}\""),
    }).collect::<Vec<_>>().join(",");
    db.execute_batch(&format!(
        "INSERT INTO wake_intents ({columns}) SELECT {values} FROM wake_intents;"
    ))
    .unwrap();
}
#[test]
fn all_applicable_blockers_claim_atomically_and_partial_stale_group_cannot_release() {
    let (store, _path, _r, _p) = accepted();
    second_history(&store);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET cleanup_quarantined=1 WHERE attempt='other-history'",
            [],
        )
        .unwrap();
    assert!(!claim(&store).2);
    assert_eq!(
        store
            .context()
            .open_query(budget())
            .unwrap()
            .query_row(
                "SELECT count(*) FROM wake_intents WHERE native_fallback_json IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE wake_intents SET cleanup_quarantined=0", [])
        .unwrap();
    let (first, ticket, granted) = claim(&store);
    assert!(granted);
    assert_eq!(
        store
            .context()
            .open_query(budget())
            .unwrap()
            .query_row(
                "SELECT count(*) FROM wake_intents WHERE native_fallback_json IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    // Only one old cohort remains relevant to the current seat. A newer claim
    // replaces that member; the late old group completion must change neither.
    store
        .context()
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE wake_intents SET origin_seat='elsewhere' WHERE attempt='other-history'",
            [],
        )
        .unwrap();
    attention(&store, 200);
    assert!(claim(&store).2);
    assert!(
        !StorePort::finish_native_wake_fallback(
            &store,
            &first,
            ticket,
            NativeFallbackOutcome::NoEffect,
            &budget()
        )
        .unwrap()
    );
    assert!(!claim(&store).2);
}
#[test]
fn qualified_new_end_first_fulfills_without_qualifying_a_queue_baseline() {
    let (store, _path, r, p) = accepted();
    StorePort::record_native_turn(
        &store,
        &native_report(&store, &r, &p, "missed-start-new-end", TurnEdge::Idle),
        &budget(),
    )
    .unwrap();
    let row = StorePort::wake_intent(&store, &r.attempt, &budget())
        .unwrap()
        .unwrap();
    assert!(matches!(row.logical, WakeIntentLogical::Fulfilled { .. }));
    assert_eq!(
        StorePort::native_turn_state(&store, &p.target.scope.binding, &budget())
            .unwrap()
            .unwrap()
            .phase,
        NativeTurnPhase::EndFirst
    );
}
