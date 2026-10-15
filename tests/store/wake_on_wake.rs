//! DP8 core wake decision: a `wake_on: all` seat is woken by a pending lazy
//! row, once per lazy frontier; a `nudge` seat never is, and no poke exists.
use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use std::sync::Arc;

struct LazyWakeClock;
const DAEMON_BOOT: &str = "00000000-0000-0000-0000-000000000001";
impl Clock for LazyWakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1_000)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// An idle, registered seat `s` with a lazy-capable thread `t`, and no
/// ordinary attention at all, so a lazy row is the only possible wake reason.
fn lazy_store(wake_on: &str, batch_delay_ms: u64) -> (SqliteStore, std::path::PathBuf) {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-wake-on-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path.clone(), Arc::new(LazyWakeClock)).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute_batch(
        "UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=1;\
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
         INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pane','host',1,1,0,'fresh','term-pane','inc','coherent_enumeration',1);\
         UPDATE observed_targets SET occupancy='occupied',ui_state='idle',verified_execution='exec',top_level_occupant=1 WHERE host_id=(SELECT min(host_id) FROM herdr_hosts) AND target_id='pane';\
         INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',1,'codex','session','exec','fresh',0,0,'term-pane','inc');\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0);\
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined');\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1);",
    )
    .unwrap();
    db.execute("UPDATE seats SET wake_on=?1 WHERE id='s'", [wake_on])
        .unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            wake_batch_delay_ms: batch_delay_ms,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (store, path)
}

/// Publish lazy row `n` (message `msg-n`, recipient ordinal in insertion
/// order) to seat `s`, published at `decision_at`.
fn publish_lazy(store: &SqliteStore, n: i64, decision_at: i64) {
    stage_lazy(store, n);
    publish_staged(store, n, n, decision_at);
}

/// Stage lazy row `n` for seat `s` (preparation and recipient row; the
/// recipient ordinal follows staging order). Not yet published.
fn stage_lazy(store: &SqliteStore, n: i64) {
    let db = store.context.open_writer().unwrap();
    db.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')",
        [format!("prep-{n}")],
    )
    .unwrap();
    db.execute(
        "INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,'t','s')",
        [format!("prep-{n}"), format!("msg-{n}")],
    )
    .unwrap();
}

/// Publish the staged row `n` in the decision `decision_seq` at `decision_at`
/// (message and manifest).
fn publish_staged(store: &SqliteStore, n: i64, decision_seq: i64, decision_at: i64) {
    let db = store.context.open_writer().unwrap();
    db.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary',?3,'passive',?4,'lazy')",
        params![format!("msg-{n}"), n, decision_seq, decision_at],
    )
    .unwrap();
    db.execute(
        "INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'34750f98bd59fcfc946da45aaabe933b','t',?4,?3,?4,0,1,0)",
        params![format!("prep-{n}"), format!("msg-{n}"), decision_at, decision_seq],
    )
    .unwrap();
}

fn candidates(store: &SqliteStore) -> Vec<WakeCandidate> {
    StorePort::wake_candidates(
        store,
        crate::test_support::test_host_id(),
        PageRequest::default(),
        &budget(),
    )
    .unwrap()
    .items
}

/// The candidate of seat `s` read directly, whether or not it is actionable.
fn direct_candidate(store: &SqliteStore) -> WakeCandidate {
    let db = store.context.open_writer().unwrap();
    let scan = attention::wake_seat_attention(&db, "s").unwrap();
    wake::load_candidate(
        &db,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new("s"),
        &scan.attention,
        scan.decision_seq,
    )
    .unwrap()
}

fn reserve(store: &SqliteStore, candidate: &WakeCandidate) -> Option<WakeReservation> {
    StorePort::reserve_wake(
        store,
        crate::test_support::test_host_id(),
        candidate,
        &budget(),
    )
    .unwrap()
}

fn last_lazy_ordinal(store: &SqliteStore) -> Option<i64> {
    store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT last_lazy_ordinal FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .flatten()
}

fn settle(store: &SqliteStore, reservation: &WakeReservation) {
    StorePort::complete_wake(
        store,
        crate::test_support::test_host_id(),
        reservation.attempt.clone(),
        WakeOutcome::Submitted,
        None,
        &budget(),
    )
    .unwrap();
}

#[test]
fn idle_all_seat_with_only_a_lazy_row_is_reserved_with_the_lazy_reason() {
    let (store, path) = lazy_store("all", 0);
    publish_lazy(&store, 1, 50);
    let found = candidates(&store);
    assert_eq!(found.len(), 1, "a lazy-only all seat is discovered");
    assert!(!found[0].has_pending_receipt && !found[0].has_pending_invitation);
    assert!(found[0].has_actionable_work());
    let reservation = reserve(&store, &found[0]).expect("reserved");
    assert_eq!(reservation.reasons, vec!["lazy".to_string()]);
    assert_eq!(last_lazy_ordinal(&store), Some(1));
    assert!(StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn same_lazy_frontier_is_not_reserved_again_but_a_new_row_is() {
    let (store, path) = lazy_store("all", 0);
    publish_lazy(&store, 1, 50);
    let first = candidates(&store).remove(0);
    let reservation = reserve(&store, &first).expect("first reservation");
    settle(&store, &reservation);
    // Same frontier: the row is still pending, but the seat was woken for it.
    let same = direct_candidate(&store);
    assert_eq!(same.pending_lazy(), Some(1), "the row is still pending");
    assert!(!same.has_actionable_work());
    assert!(candidates(&store).is_empty());
    assert!(reserve(&store, &same).is_none());
    // A higher ordinal advances the frontier and is reserved again.
    publish_lazy(&store, 2, 60);
    let next = candidates(&store).remove(0);
    assert_eq!(next.pending_lazy(), Some(2));
    assert!(
        next.attention_witness
            .as_ref()
            .unwrap()
            .frontier()
            .advanced_beyond(&next.last_reserved_frontier)
    );
    let again = reserve(&store, &next).expect("new lazy row reserves again");
    assert_eq!(again.reasons, vec!["lazy".to_string()]);
    assert_eq!(last_lazy_ordinal(&store), Some(2));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_row_staged_earlier_but_published_later_wakes_the_seat_again() {
    let (store, path) = lazy_store("all", 0);
    // A stages row 5 (ordinal 1), B stages row 6 (ordinal 2) and publishes first.
    stage_lazy(&store, 5);
    stage_lazy(&store, 6);
    publish_staged(&store, 6, 10, 50);
    let first = candidates(&store).remove(0);
    let reservation = reserve(&store, &first).expect("first reservation");
    assert_eq!(reservation.reasons, vec!["lazy".to_string()]);
    settle(&store, &reservation);
    assert!(!direct_candidate(&store).has_actionable_work());
    // A publishes later: a larger publication sequence, a smaller ordinal.
    publish_staged(&store, 5, 11, 60);
    let found = candidates(&store);
    assert_eq!(found.len(), 1, "the late-published row is discovered");
    assert!(found[0].has_unreserved_lazy());
    let again = reserve(&store, &found[0]).expect("late row reserves again");
    assert_eq!(again.reasons, vec!["lazy".to_string()]);
    assert_eq!(last_lazy_ordinal(&store), Some(11));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn nudge_seat_is_never_woken_by_a_lazy_row_and_no_poke_exists() {
    let (store, path) = lazy_store("nudge", 0);
    publish_lazy(&store, 1, 50);
    assert!(candidates(&store).is_empty());
    let direct = direct_candidate(&store);
    assert_eq!(direct.pending_lazy(), None);
    assert!(!direct.has_actionable_work());
    assert!(reserve(&store, &direct).is_none());
    assert_eq!(last_lazy_ordinal(&store), None);
    drop(store);
    let _ = std::fs::remove_file(path);

    // The all seat's wake creates no receipt and no poke either.
    let (store, path) = lazy_store("all", 0);
    publish_lazy(&store, 1, 50);
    reserve(&store, &candidates(&store).remove(0)).expect("reserved");
    let db = store.context.open_writer().unwrap();
    for table in ["receipts", "receipt_state"] {
        let rows: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} gains no row from a lazy wake");
    }
    drop(db);
    assert!(
        StorePort::poke_candidates(&store, crate::test_support::test_host_id(), 16, &budget())
            .unwrap()
            .is_empty(),
        "lazy rows have no deadline, so no poke is due"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn refused_wake_restores_the_lazy_frontier_so_the_row_wakes_again() {
    let (store, path) = lazy_store("all", 0);
    publish_lazy(&store, 1, 50);
    let candidate = candidates(&store).remove(0);
    let prior = crate::ports::PriorLadder::from_candidate(&candidate);
    let reservation = reserve(&store, &candidate).expect("reserved");
    assert_eq!(last_lazy_ordinal(&store), Some(1));
    StorePort::complete_wake(
        &store,
        crate::test_support::test_host_id(),
        reservation.attempt.clone(),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unsafe),
        Some(&prior),
        &budget(),
    )
    .unwrap();
    assert_eq!(last_lazy_ordinal(&store), None, "nothing was sent");
    assert_eq!(candidates(&store).len(), 1);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn witness_recheck_rejects_a_reservation_whose_lazy_frontier_moved() {
    let (store, path) = lazy_store("all", 0);
    publish_lazy(&store, 1, 50);
    let stale = candidates(&store).remove(0);
    // A second row publishes without any decision-sequence bump, so only the
    // lazy re-check can see that the witness no longer matches.
    publish_lazy(&store, 2, 60);
    assert!(reserve(&store, &stale).is_none());
    assert_eq!(last_lazy_ordinal(&store), None);
    // Likewise a `wake_on` change after discovery.
    let fresh = candidates(&store).remove(0);
    store
        .context
        .open_writer()
        .unwrap()
        .execute("UPDATE seats SET wake_on='nudge' WHERE id='s'", [])
        .unwrap();
    assert!(reserve(&store, &fresh).is_none());
    store
        .context
        .open_writer()
        .unwrap()
        .execute("UPDATE seats SET wake_on='all' WHERE id='s'", [])
        .unwrap();
    let current = candidates(&store).remove(0);
    assert!(reserve(&store, &current).is_some());
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn batch_window_anchors_on_the_oldest_pending_lazy_row() {
    let (store, path) = lazy_store("all", 30_000);
    publish_lazy(&store, 1, 50);
    publish_lazy(&store, 2, 90);
    let candidate = candidates(&store).remove(0);
    assert_eq!(
        StorePort::wake_batch_window(
            &store,
            crate::test_support::test_host_id(),
            &candidate,
            &budget()
        )
        .unwrap(),
        Some((UtcMillis(30_050), 30_000)),
        "the oldest lazy row opens the window like an ordinary receipt"
    );
    // A later pass keeps the retained deadline instead of sliding it.
    publish_lazy(&store, 3, 95);
    assert_eq!(
        StorePort::wake_batch_window(
            &store,
            crate::test_support::test_host_id(),
            &candidate,
            &budget()
        )
        .unwrap(),
        Some((UtcMillis(30_050), 30_000))
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn lazy_publication_sets_the_reason_bit_only_for_all_recipients() {
    let (store, path) = lazy_store("all", 0);
    let mut db = store.context.open_writer().unwrap();
    db.execute_batch(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES('n','34750f98bd59fcfc946da45aaabe933b','unresolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');\
         INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t','s');\
         INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t','n');",
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    lazy_delivery::note_published(&tx, "prep-1").unwrap();
    tx.commit().unwrap();
    let bits = |seat: &str| -> Option<(i64, i64)> {
        db.query_row(
            "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id=?1",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
    };
    assert_eq!(bits("s"), Some((1, 1)), "the all recipient is re-judged");
    assert_eq!(bits("n"), None, "a nudge recipient is untouched");
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}
