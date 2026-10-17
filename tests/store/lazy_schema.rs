use herdr_threads::{protocol::time::UtcMillis, store::schema};
use rusqlite::Connection;

use herdr_threads::{
    protocol::{
        commands::DeliveryMode,
        ids::{MessageId, SeatId},
    },
    store::lazy_delivery::{self, RowFilter, seek_pending},
};
use rusqlite::params;

fn fixture() -> Connection {
    let db = super::channel_archival::fixture();
    super::channel_archival::thread(&db);
    super::channel_archival::joined_agent(&db);
    db
}
fn stage(db: &Connection, n: i64) {
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',1)",[format!("prep-{n}")]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,'t','s')",params![format!("prep-{n}"),format!("msg-{n}")]).unwrap();
}
fn publish(db: &Connection, n: i64) {
    publish_at(db, n, n);
}
fn publish_at(db: &Connection, n: i64, decision: i64) {
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary',?3,'passive',0,'lazy',1)",params![format!("msg-{n}"),n,decision]).unwrap();
    db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'34750f98bd59fcfc946da45aaabe933b','t',?4,0,?3,0,1,0)",params![format!("prep-{n}"),format!("msg-{n}"),n,decision]).unwrap();
    // note_published: the decision is copied onto the delivery row exactly once.
    db.execute(
        "UPDATE lazy_recipients SET decision_seq=?2 WHERE preparation_id=?1 AND decision_seq IS NULL",
        params![format!("prep-{n}"), decision],
    )
    .unwrap();
    db.execute(
        "UPDATE local_node SET decision_seq=max(decision_seq,?1) WHERE id='34750f98bd59fcfc946da45aaabe933b'",
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

/// Every message the seat-level seek returns up to publication bound `dh`, and the index rows it
/// visited.
fn drain(db: &Connection, seat: &SeatId, dh: u64) -> (Vec<MessageId>, usize) {
    let (mut after, mut seen, mut work) = (None, Vec::new(), 0);
    loop {
        let seek = seek_pending(
            db,
            "34750f98bd59fcfc946da45aaabe933b",
            seat,
            RowFilter::Seat,
            after,
            dh,
            37,
        )
        .unwrap();
        work += seek.visited;
        seen.extend(seek.row.map(|(_, recipient)| recipient.message));
        after = seek.position;
        if seek.exhausted {
            return (seen, work);
        }
    }
}

#[test]
fn lazy_pending_scan_bounds_unpublished_work() {
    let db = fixture();
    for n in 1..=1200 {
        stage(&db, n);
    }
    let seat = SeatId::new("s");
    // Staged rows carry no decision_seq: the seek never visits one.
    assert_eq!(drain(&db, &seat, 1_000_000), (vec![], 0));
    publish(&db, 1200);
    let dh = lazy_delivery::capture_high_water(&db, "34750f98bd59fcfc946da45aaabe933b", &seat, None)
        .unwrap()
        .publication_decision as u64;
    stage(&db, 1201);
    publish(&db, 1201);
    // A row published after capture stays outside the publication fence.
    publish_at(&db, 1100, 1202);
    let (seen, work) = drain(&db, &seat, dh);
    assert_eq!(seen, vec![MessageId::new("msg-1200")]);
    assert_eq!(
        work, 1,
        "only the one published row inside the fence is visited"
    );
    // A lower publication fence excludes the same row.
    assert_eq!(drain(&db, &seat, 1000), (vec![], 0));
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
    // The binding's cursor has passed all but the last message and completion marked the rows
    // displayed (V34: the cursor, not the row state, is the binding's display state); the partial
    // index then no longer holds the displayed rows.
    db.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) SELECT binding_id,'t',1199 FROM occupant_bindings WHERE seat_id='s'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE lazy_recipients SET state='displayed' WHERE ordinal<1200",
        [],
    )
    .unwrap();
    // Previously addressed content survives loss of current membership and
    // retirement/archive; scanning must not consult those lifecycle filters.
    db.execute_batch("UPDATE memberships SET state='left' WHERE seat_id='s'; UPDATE seats SET state='retired',retired_at=0 WHERE id='s'; UPDATE threads SET archived=1 WHERE id='t'").unwrap();
    let seat = SeatId::new("s");
    let (seen, work) = drain(&db, &seat, 1200);
    assert_eq!(seen, vec![MessageId::new("msg-1200")]);
    assert_eq!(work, 1);
    let tx = db.transaction().unwrap();
    for _ in 0..2 {
        lazy_delivery::complete_for_binding(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &seat,
            None,
            &[MessageId::new("msg-1200")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(drain(&db, &seat, 1200), (vec![], 0));
}

#[test]
fn lazy_mode_lookup_is_exact_and_instance_scoped() {
    let db = fixture();
    stage(&db, 1);
    publish(&db, 1);
    assert_eq!(
        lazy_delivery::recorded_mode(
            &db,
            "34750f98bd59fcfc946da45aaabe933b",
            &MessageId::new("msg-1")
        )
        .unwrap(),
        Some(DeliveryMode::Lazy)
    );
    assert_eq!(
        lazy_delivery::recorded_mode(&db, "other", &MessageId::new("msg-1")).unwrap(),
        None
    );
    assert_eq!(
        lazy_delivery::recorded_mode(
            &db,
            "34750f98bd59fcfc946da45aaabe933b",
            &MessageId::new("msg-")
        )
        .unwrap(),
        None
    );
    assert!(lazy_delivery::capture_high_water(&db, "other", &SeatId::new("s"), None).is_err());
    // The seek is instance-scoped: another instance sees no row of this one.
    let other = seek_pending(
        &db,
        "other",
        &SeatId::new("s"),
        RowFilter::Seat,
        None,
        1,
        10,
    )
    .unwrap();
    assert!(other.row.is_none());
    assert_eq!(
        other.visited, 1,
        "the row is examined and rejected, not returned"
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
                schema::initialize(
                    &db,
                    || UtcMillis(0),
                    &herdr_threads::test_support::test_node_seed()
                )
                .is_err(),
                "{guard} altered={altered}"
            );
        }
    }
    let db = fixture();
    db.execute_batch("DROP INDEX lazy_recipients_pending_seat_ordinal")
        .unwrap();
    assert!(
        schema::initialize(
            &db,
            || UtcMillis(0),
            &herdr_threads::test_support::test_node_seed()
        )
        .is_err()
    );
}

#[test]
fn priority_schema_audit_rejects_missing_or_altered_objects() {
    for guard in [
        "preparation_priority_immutable",
        "lazy_recipient_priority_immutable",
        "lazy_recipient_decision_seq_once",
        "binding_pending_publish",
        "binding_pending_legacy_set",
        "binding_pending_legacy_clear",
        "binding_pending_cursor_insert",
        "binding_pending_cursor_advance",
        "binding_pending_row_display",
        "binding_pending_binding_end",
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
                schema::initialize(
                    &db,
                    || UtcMillis(0),
                    &herdr_threads::test_support::test_node_seed()
                )
                .is_err(),
                "{guard} altered={altered}"
            );
        }
    }
    for index in [
        "binding_pending_order",
        "binding_pending_thread_order",
        "lazy_recipients_legacy_order",
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
                schema::initialize(
                    &db,
                    || UtcMillis(0),
                    &herdr_threads::test_support::test_node_seed()
                )
                .is_err(),
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
            schema::initialize(
                &db,
                || UtcMillis(0),
                &herdr_threads::test_support::test_node_seed()
            )
            .is_err(),
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
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new("s"),
            None,
            &MessageId::new("msg-1")
        )
        .unwrap(),
        lazy_delivery::LazyCompletion::NotAddressed
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
        .query_map(
            params!["s", "msg-1", "34750f98bd59fcfc946da45aaabe933b"],
            |r| r.get::<_, String>(3),
        )
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
    let (seat, message) = (SeatId::new("s"), MessageId::new("msg-1"));
    for _ in 0..2 {
        lazy_delivery::complete_for_binding(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &seat,
            None,
            std::slice::from_ref(&message),
        )
        .unwrap();
    }
    assert_eq!(
        lazy_delivery::lazy_completion_state(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &seat,
            None,
            &message
        )
        .unwrap(),
        lazy_delivery::LazyCompletion::Completed
    );
    assert_eq!(
        lazy_delivery::lazy_completion_state(&tx, "other", &seat, None, &message).unwrap(),
        lazy_delivery::LazyCompletion::NotAddressed
    );
}

#[test]
fn lazy_source_guards_pin_existing_preparation_message_derivation() {
    let db = fixture();
    stage(&db, 1);
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','wrong','t','s')",[]).is_err());
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('q','34750f98bd59fcfc946da45aaabe933b','resolved','native','q',1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",[]).unwrap();
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','wrong','t','q')",[]).is_err());
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES('pTstABCDEFGH','34750f98bd59fcfc946da45aaabe933b','scope','compact',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',1)",[]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('pTstABCDEFGH','mTstABCDEFGH','t','s')",[]).unwrap();
    assert!(db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('pTstABCDEFGH','mTstABCDEFG!', 't','q')",[]).is_err());
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('wrong','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary',1,'passive',0,'lazy',1)",[]).unwrap();
    assert!(db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-1','wrong','34750f98bd59fcfc946da45aaabe933b','t',1,0,1,0,1,0)",[]).is_err());
}

#[test]
fn lazy_cleanup_large_preparation_advances_bounded_units() {
    let mut db = fixture();
    stage(&db, 1);
    for n in 1..=300 {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?1,1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",[format!("seat{n}")]).unwrap();
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
fn priority_schema_fresh_node_baseline_has_the_audited_v47_shape() {
    let db = fixture();
    assert_eq!(
        scalar(&db, "PRAGMA user_version"),
        schema::NODE_SCHEMA_VERSION
    );
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
        ("index", "binding_pending_order"),
        ("index", "binding_pending_thread_order"),
        ("index", "lazy_recipients_legacy_order"),
        ("table", "binding_row_displays"),
        ("table", "binding_pending"),
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
fn priority_schema_ordered_pending_seek_uses_the_new_indexes() {
    let db = fixture();
    let seek = "SELECT p.ordinal FROM binding_pending p WHERE p.binding_id=?1 AND p.decision_seq<=?2 AND (p.priority,p.decision_seq,p.ordinal)>(?3,?4,?5) ORDER BY p.priority,p.decision_seq,p.ordinal LIMIT 1";
    let plans = plan(&db, seek, &[&"b", &100, &1, &0, &0]);
    assert!(
        plans
            .iter()
            .any(|p| p.contains("binding_pending_order") && p.contains("SEARCH")),
        "{plans:?}"
    );
    assert!(
        plans.iter().all(|p| !p.contains("TEMP B-TREE")),
        "{plans:?}"
    );
    let thread_seek = seek.replace("ORDER BY", "AND p.thread_id=?6 ORDER BY");
    let plans = plan(&db, &thread_seek, &[&"b", &100, &1, &0, &0, &"t"]);
    assert!(
        plans
            .iter()
            .any(|p| p.contains("binding_pending_thread_order") && p.contains("SEARCH")),
        "{plans:?}"
    );
    assert!(
        plans.iter().all(|p| !p.contains("TEMP B-TREE")),
        "{plans:?}"
    );
    let legacy = "SELECT r.ordinal FROM lazy_recipients r WHERE r.seat_id=?1 AND r.legacy_pending=1 AND r.decision_seq IS NOT NULL AND r.decision_seq<=?2 AND (r.priority,r.decision_seq,r.ordinal)>(?3,?4,?5) ORDER BY r.priority,r.decision_seq,r.ordinal LIMIT 1";
    let plans = plan(&db, legacy, &[&"s", &100, &1, &0, &0]);
    assert!(
        plans
            .iter()
            .any(|p| p.contains("lazy_recipients_legacy_order") && p.contains("SEARCH")),
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
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','ordinary',0)")).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-1','msg-1','t','s',0)", []).unwrap();
    db.execute_batch(&format!("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-1','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary',1,'mixed',0,'ordinary',0); INSERT INTO send_manifests({MANIFEST_COLUMNS}) VALUES('prep-1','msg-1','34750f98bd59fcfc946da45aaabe933b','t',1,0,1,0,1,1)")).unwrap();
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
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',1)")).unwrap();
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
    assert_eq!(scalar(&db, "SELECT priority FROM messages"), 1);
    // Ranges.
    assert!(err(&db, "UPDATE seats SET nudge_threshold=-2").contains("CHECK constraint failed"));
    assert!(err(&db, "UPDATE seats SET nudge_threshold=10").contains("CHECK constraint failed"));
    db.execute("UPDATE seats SET nudge_threshold=-1", [])
        .unwrap();
    db.execute("UPDATE seats SET nudge_threshold=9", [])
        .unwrap();
    assert!(
        err(&db, "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,priority) VALUES('big','34750f98bd59fcfc946da45aaabe933b','t',9,'ordinary',9,'x',0,10)")
            .contains("CHECK constraint failed")
    );
    // Manifest guards.
    let db = fixture();
    let prepare = |db: &Connection, n: i64, preparation: i64, message: i64| {
        db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-{n}','34750f98bd59fcfc946da45aaabe933b','scope','prep-{n}',zeroblob(32),'t',0,0,0,0,0,0,0,'building','ordinary',{preparation}); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-{n}','34750f98bd59fcfc946da45aaabe933b','t',{n},'ordinary',{n},'x',0,'ordinary',{message})")).unwrap();
    };
    let manifest = |n: i64, warnings: i64| {
        format!(
            "INSERT INTO send_manifests({MANIFEST_COLUMNS}) VALUES('prep-{n}','msg-{n}','34750f98bd59fcfc946da45aaabe933b','t',{n},0,{n},0,1,{warnings})"
        )
    };
    prepare(&db, 1, 1, 0);
    assert!(err(&db, &manifest(1, 0)).contains("lazy manifest source mismatch"));
    prepare(&db, 2, 1, 1);
    assert!(err(&db, &manifest(2, 1)).contains("lazy manifest source mismatch"));
    db.execute_batch("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('prep-2','t','s',1,300000,1)").unwrap();
    db.execute(&manifest(2, 1), []).unwrap();
}

// Kills (ht-yha.66 M8): a delivery row staged at a priority other than its preparation's, and a
// schema audit that accepts the pre-check copy of `lazy_recipient_source`.
#[test]
fn priority_schema_row_priority_matches_its_preparation() {
    let db = fixture();
    db.execute_batch(&format!("INSERT INTO send_preparations({PREPARATION_COLUMNS},priority) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',3)")).unwrap();
    for wrong in [0, 1, 4] {
        let error = db
            .execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-1','msg-1','t','s',?1)", [wrong])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("lazy recipient source mismatch"),
            "p{wrong}: {error}"
        );
    }
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-1','msg-1','t','s',3)", [])
        .unwrap();

    let db = fixture();
    let live: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='lazy_recipient_source'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let unchecked = live.replace(" AND p.priority=NEW.priority", "");
    assert_ne!(unchecked, live, "the live trigger checks the priority");
    db.execute_batch(&format!("DROP TRIGGER lazy_recipient_source; {unchecked};"))
        .unwrap();
    assert!(
        schema::initialize(
            &db,
            || UtcMillis(0),
            &herdr_threads::test_support::test_node_seed()
        )
        .is_err()
    );
}
