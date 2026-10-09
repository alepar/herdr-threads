use herdr_threads::{protocol::time::UtcMillis, store::schema};
use rusqlite::Connection;

#[test]
fn lazy_schema_upgrade25_defaults_existing_messages_to_ordinary() {
    let db = Connection::open_in_memory().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths.into_iter().take(25) {
        db.execute_batch(&std::fs::read_to_string(path).unwrap())
            .unwrap();
    }
    db.execute_batch("PRAGMA user_version=25; INSERT INTO host_instances(id,created_at) VALUES('i',0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at) VALUES('m','i','t',1,'ordinary',1,'old',0)").unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(
        db.query_row("SELECT delivery_mode FROM messages WHERE id='m'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "ordinary"
    );
    assert_eq!(
        db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        28
    );
}

use herdr_threads::{
    protocol::{
        commands::DeliveryMode,
        ids::{MessageId, SeatId},
    },
    store::lazy_delivery::{self, HighWater},
};
use rusqlite::params;

fn fixture() -> Connection {
    let db = super::channel_archival::fixture();
    super::channel_archival::thread(&db);
    super::channel_archival::joined_agent(&db);
    db
}
fn stage(db: &Connection, n: i64) {
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES(?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')",[format!("prep-{n}")]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,'t','s')",params![format!("prep-{n}"),format!("msg-{n}")]).unwrap();
}
fn publish(db: &Connection, n: i64) {
    publish_at(db, n, n);
}
fn publish_at(db: &Connection, n: i64, decision: i64) {
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES(?1,'i','t',?2,'ordinary',?3,'passive',0,'lazy')",params![format!("msg-{n}"),n,decision]).unwrap();
    db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'i','t',?4,0,?3,0,1,0)",params![format!("prep-{n}"),format!("msg-{n}"),n,decision]).unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=max(decision_seq,?1) WHERE id='i'",
        [decision],
    )
    .unwrap();
}

#[test]
fn lazy_schema_rejects_identity_and_mode_rewrites() {
    let db = fixture();
    stage(&db, 1);
    for assignment in [
        "ordinal=9",
        "preparation_id='different'",
        "message_id='different'",
        "thread_id='different'",
        "seat_id='different'",
        "state='displayed'",
    ] {
        assert!(
            db.execute(&format!("UPDATE lazy_recipients SET {assignment}"), [])
                .is_err(),
            "{assignment}"
        );
    }
    assert!(
        db.execute("UPDATE send_preparations SET delivery_mode='ordinary'", [])
            .is_err()
    );
    publish(&db, 1);
    assert!(
        db.execute("UPDATE messages SET delivery_mode='ordinary'", [])
            .is_err()
    );
    db.execute("UPDATE lazy_recipients SET state='displayed'", [])
        .unwrap();
    assert!(
        db.execute("UPDATE lazy_recipients SET state='pending'", [])
            .is_err()
    );
    assert!(db.execute("DELETE FROM lazy_recipients", []).is_err());
}

#[test]
fn lazy_pending_scan_bounds_unpublished_work() {
    let db = fixture();
    for n in 1..=1200 {
        stage(&db, n);
    }
    publish(&db, 1200);
    let seat = SeatId::new("s");
    let high = lazy_delivery::capture_high_water(&db, "i", &seat).unwrap();
    stage(&db, 1201);
    publish(&db, 1201);
    // A row published after capture stays outside the publication fence even
    // though its ordinal was captured.
    publish_at(&db, 1100, 1202);
    let mut after = 0;
    let mut work = 0;
    let mut seen = Vec::new();
    loop {
        let page = lazy_delivery::pending_page(&db, "i", &seat, after, high, 37).unwrap();
        assert!(page.inspected <= 37);
        if page.inspected > 0 {
            assert!(page.last_inspected > after);
        }
        if after == 0 {
            assert!(page.recipients.is_empty());
            assert!(page.has_more);
        }
        work += page.inspected;
        seen.extend(page.recipients.into_iter().map(|r| r.message));
        after = page.last_inspected;
        if !page.has_more {
            break;
        }
    }
    // A lower publication fence also excludes the same candidate.
    let old = HighWater {
        publication_decision: 1000,
        ..high
    };
    assert!(
        lazy_delivery::pending_page(&db, "i", &seat, 1099, old, 1)
            .unwrap()
            .recipients
            .is_empty()
    );
    assert_eq!(work, 1200);
    assert_eq!(after, high.recipient_ordinal);
    assert_eq!(seen, vec![MessageId::new("msg-1200")]);
    assert!(lazy_delivery::pending_page(&db, "i", &seat, 0, high, 0).is_err());
    assert_index(
        &db,
        lazy_delivery::PENDING_CANDIDATES_SQL,
        "lazy_recipients_pending_seat_ordinal",
    );
}
fn assert_index(db: &Connection, sql: &str, index: &str) {
    let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let plans = stmt
        .query_map(params!["s", 0, 999999, 37], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plans
            .iter()
            .any(|p| p.contains(index) && p.contains("SEARCH")),
        "{plans:?}"
    );
    assert!(
        plans.iter().all(|p| !p.contains("TEMP B-TREE")),
        "{plans:?}"
    );
}

#[test]
fn lazy_pending_scan_skips_large_displayed_history() {
    let mut db = fixture();
    for n in 1..=1200 {
        stage(&db, n);
        publish(&db, n);
    }
    db.execute(
        "UPDATE lazy_recipients SET state='displayed' WHERE ordinal<=1199",
        [],
    )
    .unwrap();
    // Previously addressed content survives loss of current membership and
    // retirement/archive; scanning must not consult those lifecycle filters.
    db.execute_batch("UPDATE memberships SET state='left' WHERE seat_id='s'; UPDATE seats SET state='retired',retired_at=0 WHERE id='s'; UPDATE threads SET archived=1 WHERE id='t'").unwrap();
    let seat = SeatId::new("s");
    let high = lazy_delivery::capture_high_water(&db, "i", &seat).unwrap();
    let page = lazy_delivery::pending_page(&db, "i", &seat, 0, high, 2).unwrap();
    assert_eq!(page.inspected, 1);
    assert_eq!(page.recipients[0].message, MessageId::new("msg-1200"));
    assert!(!page.has_more);
    let tx = db.transaction().unwrap();
    assert!(
        lazy_delivery::complete_addressed(&tx, "i", &seat, &MessageId::new("msg-1200")).unwrap()
    );
    assert!(
        lazy_delivery::complete_addressed(&tx, "i", &seat, &MessageId::new("msg-1200")).unwrap()
    );
    tx.commit().unwrap();
    assert_eq!(
        lazy_delivery::pending_page(&db, "i", &seat, 0, high, 2)
            .unwrap()
            .inspected,
        0
    );
    assert_index(
        &db,
        lazy_delivery::PENDING_CANDIDATES_SQL,
        "lazy_recipients_pending_seat_ordinal",
    );
}

#[test]
fn lazy_mode_lookup_is_exact_and_instance_scoped() {
    let db = fixture();
    stage(&db, 1);
    publish(&db, 1);
    assert_eq!(
        lazy_delivery::recorded_mode(&db, "i", &MessageId::new("msg-1")).unwrap(),
        Some(DeliveryMode::Lazy)
    );
    assert_eq!(
        lazy_delivery::recorded_mode(&db, "other", &MessageId::new("msg-1")).unwrap(),
        None
    );
    assert_eq!(
        lazy_delivery::recorded_mode(&db, "i", &MessageId::new("msg-")).unwrap(),
        None
    );
    assert!(lazy_delivery::capture_high_water(&db, "other", &SeatId::new("s")).is_err());
    assert!(
        lazy_delivery::pending_page(
            &db,
            "other",
            &SeatId::new("s"),
            0,
            HighWater {
                recipient_ordinal: 1,
                publication_decision: 1
            },
            1
        )
        .is_err()
    );
}

#[test]
fn lazy_schema_audit_rejects_missing_or_altered_guards() {
    for guard in [
        "lazy_recipient_identity_immutable",
        "lazy_recipient_progress_forward",
        "lazy_preparation_mode_immutable",
        "lazy_recipient_source",
        "lazy_message_shape",
        "lazy_message_mode_immutable",
        "lazy_manifest_source",
        "lazy_recipient_display_published",
        "lazy_recipient_published_retained",
    ] {
        for altered in [false, true] {
            let db = fixture();
            db.execute_batch(&format!("DROP TRIGGER {guard}")).unwrap();
            if altered {
                db.execute_batch(&format!(
                    "CREATE TRIGGER {guard} BEFORE UPDATE ON lazy_recipients BEGIN SELECT 1; END"
                ))
                .unwrap();
            }
            assert!(
                schema::initialize(&db, || UtcMillis(0)).is_err(),
                "{guard} altered={altered}"
            );
        }
    }
    let db = fixture();
    db.execute_batch("DROP INDEX lazy_recipients_pending_seat_ordinal")
        .unwrap();
    assert!(schema::initialize(&db, || UtcMillis(0)).is_err());
}

#[test]
fn lazy_cleanup_is_bounded_and_preserves_published_rows() {
    let mut db = fixture();
    stage(&db, 1);
    stage(&db, 2);
    publish(&db, 2);
    let tx = db.transaction().unwrap();
    let high = lazy_delivery::preparation_high_water(&tx, "prep-1").unwrap();
    let page = lazy_delivery::cleanup_unpublished(&tx, "prep-1", 0, high, 1).unwrap();
    assert_eq!(
        (page.inspected, page.deleted, page.last_inspected),
        (1, 1, high)
    );
    let high = lazy_delivery::preparation_high_water(&tx, "prep-2").unwrap();
    let page = lazy_delivery::cleanup_unpublished(&tx, "prep-2", 0, high, 1).unwrap();
    assert_eq!((page.inspected, page.deleted), (1, 0));
    assert!(
        !lazy_delivery::complete_addressed(&tx, "i", &SeatId::new("s"), &MessageId::new("msg-1"))
            .unwrap()
    );
    tx.commit().unwrap();
    assert_index(
        &db,
        lazy_delivery::CLEANUP_CANDIDATES_SQL,
        "lazy_recipients_preparation_ordinal",
    );
}

#[test]
fn lazy_addressed_completion_uses_exact_index_even_after_display() {
    let mut db = fixture();
    stage(&db, 1);
    publish(&db, 1);
    let mut stmt = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            lazy_delivery::ADDRESSED_SQL
        ))
        .unwrap();
    let plan = stmt
        .query_map(params!["s", "msg-1", "i"], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().any(
            |p| p.contains("SEARCH r USING INDEX sqlite_autoindex_lazy_recipients_2")
                && p.contains("message_id=? AND seat_id=?")
        ),
        "{plan:?}"
    );
    drop(stmt);
    let tx = db.transaction().unwrap();
    assert!(
        lazy_delivery::complete_addressed(&tx, "i", &SeatId::new("s"), &MessageId::new("msg-1"))
            .unwrap()
    );
    assert!(
        lazy_delivery::complete_addressed(&tx, "i", &SeatId::new("s"), &MessageId::new("msg-1"))
            .unwrap()
    );
    assert!(
        !lazy_delivery::complete_addressed(
            &tx,
            "other",
            &SeatId::new("s"),
            &MessageId::new("msg-1")
        )
        .unwrap()
    );
}

#[test]
fn lazy_source_guards_pin_existing_preparation_message_derivation() {
    let db = fixture();
    stage(&db, 1);
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','wrong','t','s')",[]).is_err());
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('q','i','resolved','native','q',1,0,0)",[]).unwrap();
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','wrong','t','q')",[]).is_err());
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('pABCDEFGH','i','scope','compact',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')",[]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('pABCDEFGH','mABCDEFGH','t','s')",[]).unwrap();
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('pABCDEFGH','mABCDEFG!', 't','q')",[]).is_err());
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('wrong','i','t',1,'ordinary',1,'passive',0,'lazy')",[]).unwrap();
    assert!(db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-1','wrong','i','t',1,0,1,0,1,0)",[]).is_err());
}

#[test]
fn lazy_cleanup_large_preparation_advances_bounded_units() {
    let mut db = fixture();
    stage(&db, 1);
    for n in 1..=300 {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES(?1,'i','resolved','native',?1,1,0,0)",[format!("seat{n}")]).unwrap();
        db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t',?1)",[format!("seat{n}")]).unwrap();
    }
    let tx = db.transaction().unwrap();
    let high = lazy_delivery::preparation_high_water(&tx, "prep-1").unwrap();
    let mut after = 0;
    let mut total = 0;
    loop {
        let page = lazy_delivery::cleanup_unpublished(&tx, "prep-1", after, high, 17).unwrap();
        assert!(page.inspected <= 17);
        assert_eq!(page.inspected, page.deleted);
        assert!(page.last_inspected > after);
        after = page.last_inspected;
        total += page.deleted;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(total, 301);
    assert_eq!(after, high);
}
