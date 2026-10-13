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
        47
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
    let high = lazy_delivery::capture_high_water(&db, "i", &seat, None).unwrap();
    stage(&db, 1201);
    publish(&db, 1201);
    // A row published after capture stays outside the publication fence even
    // though its ordinal was captured.
    publish_at(&db, 1100, 1202);
    let mut after = 0;
    let mut work = 0;
    let mut seen = Vec::new();
    loop {
        let page = lazy_delivery::pending_page(&db, "i", &seat, None, after, high, 37).unwrap();
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
        lazy_delivery::pending_page(&db, "i", &seat, None, 1099, old, 1)
            .unwrap()
            .recipients
            .is_empty()
    );
    assert_eq!(work, 1200);
    assert_eq!(after, high.recipient_ordinal);
    assert_eq!(seen, vec![MessageId::new("msg-1200")]);
    assert!(lazy_delivery::pending_page(&db, "i", &seat, None, 0, high, 0).is_err());
    assert_index(
        &db,
        lazy_delivery::PENDING_CANDIDATES_SQL,
        "lazy_recipients_seat_ordinal",
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
    // The binding's cursor has passed all but the last message (V34: display state is the
    // cursor, not the recipient row's state).
    db.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',1199 FROM occupant_bindings WHERE seat_id='s'",
        [],
    )
    .unwrap();
    // Previously addressed content survives loss of current membership and
    // retirement/archive; scanning must not consult those lifecycle filters.
    db.execute_batch("UPDATE memberships SET state='left' WHERE seat_id='s'; UPDATE seats SET state='retired',retired_at=0 WHERE id='s'; UPDATE threads SET archived=1 WHERE id='t'").unwrap();
    let seat = SeatId::new("s");
    let high = lazy_delivery::capture_high_water(&db, "i", &seat, None).unwrap();
    let page = lazy_delivery::pending_page(&db, "i", &seat, None, 0, high, 2).unwrap();
    assert_eq!(page.inspected, 1);
    assert_eq!(page.recipients[0].message, MessageId::new("msg-1200"));
    assert!(!page.has_more);
    let tx = db.transaction().unwrap();
    for _ in 0..2 {
        lazy_delivery::complete_for_binding(&tx, "i", &seat, None, &[MessageId::new("msg-1200")])
            .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(
        lazy_delivery::pending_page(&db, "i", &seat, None, 0, high, 2)
            .unwrap()
            .inspected,
        0
    );
    assert_index(
        &db,
        lazy_delivery::PENDING_CANDIDATES_SQL,
        "lazy_recipients_seat_ordinal",
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
    assert!(lazy_delivery::capture_high_water(&db, "other", &SeatId::new("s"), None).is_err());
    assert!(
        lazy_delivery::pending_page(
            &db,
            "other",
            &SeatId::new("s"),
            None,
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
fn priority_schema_audit_rejects_missing_or_altered_objects() {
    for guard in [
        "message_priority_immutable",
        "preparation_priority_immutable",
        "lazy_recipient_priority_immutable",
        "lazy_recipient_decision_seq_once",
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
    for index in [
        "lazy_recipients_pending_order",
        "lazy_recipients_pending_thread_order",
    ] {
        for altered in [false, true] {
            let db = fixture();
            db.execute_batch(&format!("DROP INDEX {index}")).unwrap();
            if altered {
                db.execute_batch(&format!(
                    "CREATE INDEX {index} ON lazy_recipients(seat_id,ordinal)"
                ))
                .unwrap();
            }
            assert!(
                schema::initialize(&db, || UtcMillis(0)).is_err(),
                "{index} altered={altered}"
            );
        }
    }
    for altered in [false, true] {
        let db = fixture();
        db.execute_batch("DROP TABLE binding_row_displays").unwrap();
        if altered {
            db.execute_batch("CREATE TABLE binding_row_displays(binding_id TEXT, message_id TEXT)")
                .unwrap();
        }
        assert!(
            schema::initialize(&db, || UtcMillis(0)).is_err(),
            "binding_row_displays altered={altered}"
        );
    }
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

fn v46_database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths.into_iter().take(46) {
        db.execute_batch(&std::fs::read_to_string(path).unwrap())
            .unwrap();
    }
    db.execute_batch("PRAGMA user_version=46").unwrap();
    db
}
const SEAT_ROW: &str = "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES";
const PREPARATION_COLUMNS: &str = "id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode";
const MANIFEST_COLUMNS: &str = "preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count";

fn scalar(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn plan(db: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Vec<String> {
    let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map(args, |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn priority_schema_fresh_database_is_v47_with_audited_shape() {
    let db = fixture();
    assert_eq!(scalar(&db, "PRAGMA user_version"), 47);
    for (table, column, not_null, default) in [
        ("messages", "priority", 1, Some("0")),
        ("send_preparations", "priority", 1, Some("0")),
        ("lazy_recipients", "priority", 1, Some("1")),
        ("lazy_recipients", "decision_seq", 0, None),
        ("seats", "nudge_threshold", 1, Some("0")),
    ] {
        let found: (String, i64, Option<String>) = db
            .query_row(
                &format!("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('{table}') WHERE name=?1"),
                [column],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            found,
            ("INTEGER".to_owned(), not_null, default.map(str::to_owned)),
            "{table}.{column}"
        );
    }
    for (kind, name) in [
        ("index", "lazy_recipients_pending_order"),
        ("index", "lazy_recipients_pending_thread_order"),
        ("table", "binding_row_displays"),
    ] {
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type=?1 AND name=?2",
                [kind, name],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "{name}"
        );
    }
}

#[test]
fn priority_schema_upgrade46_backfills_priority_threshold_and_publication() {
    let db = v46_database();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0);").unwrap();
    db.execute_batch(&format!("{SEAT_ROW} ('a','i','resolved','native','a',1,0,0),('n','i','resolved','native','n',1,0,0),('w','i','resolved','native','w',1,0,0); UPDATE seats SET wake_on='all' WHERE id='a'; UPDATE seats SET wake_on='nudge' WHERE id='n'")).unwrap();
    // Ordinary message with its preparation.
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS}) VALUES('prep-1','i','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','ordinary'); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('msg-1','i','t',1,'ordinary',1,'plain',0,'ordinary')")).unwrap();
    // Published lazy message at decision 7, and a staged lazy preparation.
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS}) VALUES('prep-2','i','scope','prep-2',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy'),('prep-3','i','scope','prep-3',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy'); INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-2','msg-2','t','a'),('prep-3','msg-3','t','a'); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('msg-2','i','t',2,'ordinary',7,'passive',0,'lazy'); INSERT INTO send_manifests({MANIFEST_COLUMNS}) VALUES('prep-2','msg-2','i','t',7,0,2,0,1,0)")).unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(scalar(&db, "PRAGMA user_version"), 47);
    let pair = |sql: &str| -> Vec<(String, i64)> {
        db.prepare(sql)
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        pair("SELECT id,priority FROM messages ORDER BY id"),
        [("msg-1".into(), 0), ("msg-2".into(), 1)]
    );
    assert_eq!(
        pair("SELECT id,priority FROM send_preparations ORDER BY id"),
        [
            ("prep-1".into(), 0),
            ("prep-2".into(), 1),
            ("prep-3".into(), 1)
        ]
    );
    assert_eq!(
        pair("SELECT message_id,priority FROM lazy_recipients ORDER BY message_id"),
        [("msg-2".into(), 1), ("msg-3".into(), 1)]
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT decision_seq FROM lazy_recipients WHERE message_id='msg-2'"
        ),
        7
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT decision_seq IS NULL FROM lazy_recipients WHERE message_id='msg-3'"
        ),
        1
    );
    assert_eq!(
        pair("SELECT id,nudge_threshold FROM seats ORDER BY id"),
        [("a".into(), -1), ("n".into(), 0), ("w".into(), 0)]
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM lazy_recipients r JOIN send_preparations p ON p.id=r.preparation_id WHERE r.priority<>p.priority"
        ),
        0
    );
    let catalog = || -> Vec<(String, String, Option<String>)> {
        db.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = catalog();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(catalog(), before);
    assert_eq!(scalar(&db, "PRAGMA user_version"), 47);
    assert_eq!(
        pair("SELECT id,priority FROM messages ORDER BY id"),
        [("msg-1".into(), 0), ("msg-2".into(), 1)]
    );
}

fn in_flight_rows(db: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    [
        "receipts",
        "receipt_state",
        "invitations",
        "requirement_episodes",
        "digest_pending_invitations",
        "presentation_confirming_invitations",
        "warning_conditions",
        "wake_work",
        "direct_threads",
        "memberships",
    ]
    .into_iter()
    .map(|table| {
        let stmt = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = stmt.column_count();
        let order = (1..=columns)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        drop(stmt);
        let mut stmt = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .unwrap();
        let rows = stmt
            .query_map([], |r| (0..columns).map(|n| r.get(n)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (table.to_owned(), rows)
    })
    .collect()
}

#[test]
fn priority_schema_upgrade46_leaves_in_flight_state_as_recorded() {
    let db = v46_database();
    db.execute_batch("INSERT INTO host_instances(id,created_at,decision_seq) VALUES('i',0,1000); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0),('pair','i','topic','goal',0,0);").unwrap();
    db.execute_batch(&format!("{SEAT_ROW} ('s1','i','resolved','native','s1',1,0,0),('s2','i','resolved','native','s2',1,0,0)")).unwrap();
    // Pending receipt owed with a deadline, and its overdue warning condition.
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at) VALUES('m1','i','t',1,'ordinary',1,'ack me',0); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES('m1','t','s2','pending',300000,0,300000); INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,episode,open_warning_id,opened_seq) VALUES('receipt','t','m1:s2','s2',NULL,'warn-1',2)").unwrap();
    // Pending lazy invitation and pending pair invitation.
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,frozen_duration_ms,deadline_at,created_decision_seq,confirmation,inviter_seat_id) VALUES('inv-lazy','t','s2',1,'pending',7,300,307,3,'lazy','s1'),('inv-pair','pair','s2',1,'pending',7,300,307,4,'require_ack','s1'); INSERT INTO direct_threads(instance_id,seat_a,seat_b,thread_id) VALUES('i','s1','s2','pair'); INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES('pair','s1','joined','joined',0),('pair','s2','invited','invited',NULL)").unwrap();
    let before = in_flight_rows(&db);
    for (table, rows) in &before {
        if [
            "receipts",
            "warning_conditions",
            "invitations",
            "direct_threads",
        ]
        .contains(&table.as_str())
        {
            assert!(!rows.is_empty(), "fixture populates {table}");
        }
    }
    schema::initialize(&db, || UtcMillis(10_000)).unwrap();
    assert_eq!(in_flight_rows(&db), before);
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM receipts WHERE state='pending' AND deadline_at=300000"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM invitations WHERE state='pending' AND confirmation='lazy'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM memberships WHERE thread_id='pair' AND state='invited'"
        ),
        1
    );
}

#[test]
fn priority_schema_ordered_pending_seek_uses_the_new_indexes() {
    let db = fixture();
    let seek = "SELECT ordinal FROM lazy_recipients WHERE seat_id=?1 AND state='pending' AND decision_seq IS NOT NULL AND decision_seq<=?2 AND (priority,decision_seq,ordinal)>(?3,?4,?5) ORDER BY priority,decision_seq,ordinal LIMIT 1";
    let plans = plan(&db, seek, &[&"s", &100, &1, &0, &0]);
    assert!(
        plans
            .iter()
            .any(|p| p.contains("lazy_recipients_pending_order") && p.contains("SEARCH")),
        "{plans:?}"
    );
    assert!(
        plans.iter().all(|p| !p.contains("TEMP B-TREE")),
        "{plans:?}"
    );
    let thread_seek = seek.replace("ORDER BY", "AND thread_id=?6 ORDER BY");
    let plans = plan(&db, &thread_seek, &[&"s", &100, &1, &0, &0, &"t"]);
    assert!(
        plans
            .iter()
            .any(|p| p.contains("lazy_recipients_pending_thread_order") && p.contains("SEARCH")),
        "{plans:?}"
    );
    assert!(
        plans.iter().all(|p| !p.contains("TEMP B-TREE")),
        "{plans:?}"
    );
}

#[test]
fn priority_schema_mixed_ack_preparation_stages_and_completes() {
    let db = fixture();
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-1','i','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','ordinary',0)")).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-1','msg-1','t','s',0)", []).unwrap();
    db.execute_batch(&format!("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-1','i','t',1,'ordinary',1,'mixed',0,'ordinary',0); INSERT INTO send_manifests({MANIFEST_COLUMNS}) VALUES('prep-1','msg-1','i','t',1,0,1,0,1,1)")).unwrap();
    db.execute("UPDATE lazy_recipients SET decision_seq=1", [])
        .unwrap();
    db.execute("UPDATE lazy_recipients SET state='displayed'", [])
        .unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM lazy_recipients WHERE state='displayed' AND decision_seq=1"
        ),
        1
    );
}

#[test]
fn priority_schema_guards() {
    let err = |db: &Connection, sql: &str| db.execute(sql, []).unwrap_err().to_string();
    // Staging must carry a NULL decision_seq.
    let db = fixture();
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS}) VALUES('prep-1','i','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')")).unwrap();
    assert!(
        err(&db, "INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,decision_seq) VALUES('prep-1','msg-1','t','s',3)")
            .contains("lazy recipient source mismatch")
    );
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t','s')", []).unwrap();
    // A NULL decision_seq may be set once; priority never changes.
    assert!(
        err(&db, "UPDATE lazy_recipients SET priority=2")
            .contains("delivery row priority is immutable")
    );
    assert!(
        err(&db, "UPDATE send_preparations SET priority=2")
            .contains("preparation priority is immutable")
    );
    publish(&db, 1);
    db.execute("UPDATE lazy_recipients SET decision_seq=1", [])
        .unwrap();
    assert!(
        err(&db, "UPDATE lazy_recipients SET decision_seq=2")
            .contains("delivery row publication is immutable")
    );
    assert!(
        err(&db, "UPDATE lazy_recipients SET decision_seq=NULL")
            .contains("delivery row publication is immutable")
    );
    assert!(db.execute("UPDATE messages SET priority=2", []).is_err());
    assert_eq!(scalar(&db, "SELECT priority FROM messages"), 0);
    // Ranges.
    assert!(err(&db, "UPDATE seats SET nudge_threshold=-2").contains("CHECK constraint failed"));
    assert!(err(&db, "UPDATE seats SET nudge_threshold=10").contains("CHECK constraint failed"));
    db.execute("UPDATE seats SET nudge_threshold=-1", [])
        .unwrap();
    db.execute("UPDATE seats SET nudge_threshold=9", [])
        .unwrap();
    assert!(
        err(&db, "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,priority) VALUES('big','i','t',9,'ordinary',9,'x',0,10)")
            .contains("CHECK constraint failed")
    );
    // Manifest guards.
    let db = fixture();
    let prepare = |db: &Connection, n: i64, preparation: i64, message: i64| {
        db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-{n}','i','scope','prep-{n}',zeroblob(32),'t',0,0,0,0,0,0,0,'building','ordinary',{preparation}); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-{n}','i','t',{n},'ordinary',{n},'x',0,'ordinary',{message})")).unwrap();
    };
    let manifest = |n: i64, warnings: i64| {
        format!(
            "INSERT INTO send_manifests({MANIFEST_COLUMNS}) VALUES('prep-{n}','msg-{n}','i','t',{n},0,{n},0,1,{warnings})"
        )
    };
    prepare(&db, 1, 1, 0);
    assert!(err(&db, &manifest(1, 0)).contains("lazy manifest source mismatch"));
    prepare(&db, 2, 1, 1);
    assert!(err(&db, &manifest(2, 1)).contains("lazy manifest source mismatch"));
    db.execute_batch("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('prep-2','t','s',1,300000,1)").unwrap();
    db.execute(&manifest(2, 1), []).unwrap();
}
