//! D5 (ht-iygh): mail reaches a seat that still exists. Direct send and pair ensure accept a
//! local, nonretired recipient in any mapping state (resolved, unresolved, waiting with no target,
//! or on a held target); a retired, missing or foreign seat is still refused.
use super::*;

/// The recipient `s2` in each mapping state a restart can leave it in.
const EXISTING: [(&str, &str); 4] = [
    (
        "unresolved",
        "UPDATE seats SET state='unresolved' WHERE id='s2'",
    ),
    (
        "waiting",
        "UPDATE seats SET state='unresolved',target_id=NULL WHERE id='s2'",
    ),
    (
        "held",
        "INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ((SELECT min(host_id) FROM herdr_hosts),'s2','b',1,'unresolved saved seat continuity')",
    ),
    (
        "invalidated",
        "UPDATE observed_targets SET generation=2 WHERE target_id='s2'",
    ),
];

#[test]
fn direct_ensure_and_send_queue_to_an_unresolved_waiting_or_held_seat() {
    for (state, sql) in EXISTING {
        let (store, db, path) = store_fixture();
        db.execute_batch(sql).unwrap();
        let t = thread(
            ensure(&store, "s1", "s2", "ensure").unwrap_or_else(|e| panic!("{state}: {e:?}")),
        );
        let v = send_request_to(&t, "s1", "s2", "send", DeliveryMode::Lazy);
        prepare(&store, &v).unwrap_or_else(|e| panic!("{state}: {e:?}"));
        publish(&store, v).unwrap_or_else(|e| panic!("{state}: {e:?}"));
        assert_eq!(count(&db, "direct_threads"), 1, "{state}");
        assert_eq!(
            count(&db, "messages WHERE kind='ordinary'"),
            1,
            "{state}: the message is published"
        );
        assert_eq!(
            count(&db, "lazy_recipients WHERE seat_id='s2'"),
            1,
            "{state}: the row is queued to the seat"
        );
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn direct_publication_reaches_a_seat_unresolved_or_held_after_preparation() {
    for (state, sql) in EXISTING {
        let (store, db, path) = store_fixture();
        let t = thread(ensure(&store, "s1", "s2", "ensure").unwrap());
        let v = send_request_to(&t, "s1", "s2", "send", DeliveryMode::Lazy);
        prepare(&store, &v).unwrap();
        db.execute_batch(sql).unwrap();
        publish(&store, v).unwrap_or_else(|e| panic!("{state}: {e:?}"));
        assert_eq!(count(&db, "messages WHERE kind='ordinary'"), 1, "{state}");
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn direct_ensure_still_refuses_retired_missing_and_foreign_seats() {
    for (target, sql) in [
        ("missing", ""),
        (
            "s2",
            "UPDATE seats SET state='retired',retired_at=1 WHERE id='s2'",
        ),
        (
            "s2",
            "UPDATE seats SET state='unresolved',retired_seq=1 WHERE id='s2'",
        ),
        (
            "s2",
            "PRAGMA foreign_keys=OFF; UPDATE seats SET instance_id='foreign' WHERE id='s2'",
        ),
    ] {
        let (store, db, path) = store_fixture();
        db.execute_batch(sql).unwrap();
        assert_eq!(
            ensure(&store, "s1", target, "bad").unwrap_err().code,
            ErrorCode::TargetUnresolved,
            "{target} {sql}"
        );
        assert_eq!(count(&db, "direct_threads"), 0, "{sql}");
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

/// TRUST-POLICY A6: the deadline of an ACK request to an unavailable recipient freezes at send
/// time plus its duration, and the sender is told the recipient is unavailable.
#[test]
fn direct_ack_request_to_an_unresolved_seat_runs_from_send_time_and_warns() {
    let (store, db, path) = store_fixture();
    let t = thread(ensure(&store, "s1", "s2", "ensure").unwrap());
    db.execute_batch(EXISTING[0].1).unwrap();
    let mut v = send_request_to(&t, "s1", "s2", "ack", DeliveryMode::Ordinary);
    v.invited_recipients = vec![SeatId::new("s2")];
    v.deadline_millis = Some(60_000);
    prepare(&store, &v).unwrap();
    assert_eq!(
        count(
            &db,
            "prepared_unavailable_warnings WHERE affected_seat_id='s2' AND event_json LIKE '%recipient_unavailable%'"
        ),
        1,
        "the sender sees recipient_unavailable"
    );
    publish(&store, v).unwrap();
    let message: String = db
        .query_row("SELECT id FROM messages WHERE kind='ordinary'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let receipt = crate::store::effective::effective_receipt(&db, &message, "s2")
        .unwrap()
        .expect("the ACK request owes s2 a receipt");
    let (sent, deadline) = (receipt.decision_at, receipt.deadline_at);
    assert_eq!(
        deadline,
        Some(sent + 60_000),
        "send time plus duration (A6)"
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn remote_check_answers_eligible_for_an_existing_local_seat_in_any_mapping_state() {
    for (state, sql) in EXISTING {
        let (store, db, path) = store_fixture();
        db.execute_batch(sql).unwrap();
        db.execute("UPDATE seats SET published=1 WHERE id='s2'", [])
            .unwrap();
        assert_eq!(
            crate::service::remote_resolver::check_seat(&db, "s2").unwrap(),
            Ok(()),
            "{state}"
        );
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    let (store, db, path) = store_fixture();
    db.execute_batch(
        "UPDATE seats SET published=1 WHERE id='s2'; UPDATE seats SET state='retired',retired_at=1 WHERE id='s2'",
    )
    .unwrap();
    assert!(
        crate::service::remote_resolver::check_seat(&db, "s2")
            .unwrap()
            .is_err()
    );
    drop(store);
    drop(db);
    let _ = std::fs::remove_file(path);
}

/// Launch and attachment paths keep the live-pane predicate.
#[test]
fn live_pane_predicate_still_refuses_unresolved_and_held_seats() {
    for (state, sql) in EXISTING {
        let (store, db, path) = store_fixture();
        db.execute_batch(sql).unwrap();
        assert!(
            crate::store::seats::eligible_delivery_recipient(
                &db,
                "34750f98bd59fcfc946da45aaabe933b",
                &SeatId::new("s2")
            )
            .is_err(),
            "{state}"
        );
        assert!(
            crate::store::seats::existing_seat_recipient(
                &db,
                "34750f98bd59fcfc946da45aaabe933b",
                &SeatId::new("s2")
            )
            .is_ok(),
            "{state}"
        );
        drop(store);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
