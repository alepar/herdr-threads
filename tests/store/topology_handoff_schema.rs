use super::{identity, namespace};
use herdr_threads::{
    protocol::{results::ErrorCode, time::UtcMillis},
    store::schema,
};
use rusqlite::Connection;

// Build genuine historical schemas from their immutable migration chain, never
// relabel a latest database by rewinding user_version.
fn historical(version: usize) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    let mut paths =
        std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
    paths.sort();
    for path in paths.into_iter().take(version) {
        db.execute_batch(&std::fs::read_to_string(path).unwrap())
            .unwrap();
    }
    db.pragma_update(None, "user_version", version as i64)
        .unwrap();
    db
}
fn version(db: &Connection) -> i64 {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}
fn objects(db: &Connection) -> Vec<(String, String, String)> {
    db.prepare(
        "SELECT type,name,sql FROM sqlite_schema WHERE name LIKE 'bootstrap_%' ORDER BY name",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}
#[test]
fn registered27_fresh_and_genuine_historical_chains_reopen() {
    for previous in [0, 1, 15, 22, 25, 26] {
        let db = historical(previous);
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        assert_eq!(version(&db), 27, "actual {previous}->27");
        assert_eq!(objects(&db), objects(&historical(27)));
        schema::initialize(&db, || UtcMillis(1)).unwrap();
        assert_eq!(version(&db), 27);
    }
}
#[test]
fn registered27_audits_every_table_index_and_complete_trigger_body() {
    let pristine = historical(27);
    schema::initialize(&pristine, || UtcMillis(0)).unwrap();
    for (kind, name, sql) in objects(&pristine) {
        // Missing objects must fail, including every guard and every lookup index.
        let db = historical(27);
        db.execute_batch(&format!("DROP {kind} {name}")).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "missing {name}"
        );
        // Full SQL must be audited, beyond an object's name and column inventory.
        let db = historical(27);
        let replacement = if kind == "trigger" {
            format!("CREATE TRIGGER {name} BEFORE UPDATE ON bootstrap_handoffs BEGIN SELECT 1; END")
        } else if kind == "index" {
            format!("CREATE INDEX {name} ON bootstrap_handoffs(compound)")
        } else {
            // Valid SQL with a removed bound, FK, or state constraint.
            sql.replacen("CHECK(", "CHECK(1 OR ", 1)
        };
        db.execute_batch("PRAGMA writable_schema=ON").unwrap();
        db.execute(
            "UPDATE sqlite_schema SET sql=?1 WHERE name=?2",
            rusqlite::params![replacement, name],
        )
        .unwrap();
        db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "altered {name}"
        );
    }
    let db = historical(27);
    db.execute_batch("PRAGMA writable_schema=ON").unwrap();
    db.execute("UPDATE sqlite_schema SET sql=replace(sql,'REFERENCES host_instances(id)','REFERENCES threads(id)') WHERE name='bootstrap_handoffs'", []).unwrap();
    db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
    assert_eq!(
        schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
        ErrorCode::IncompatibleSchema,
        "foreign-key target must be audited"
    );
    // Alter a middle statement of a multi-statement trigger. Splitting on ';'
    // or auditing only the first INSERT would miss this registry corruption.
    let db = historical(27);
    db.execute_batch("DROP TRIGGER bootstrap_attempt_keys")
        .unwrap();
    let original: String = pristine
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='bootstrap_attempt_keys'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch(&original.replace("NEW.record_key,'record'", "NEW.record_key,'reserve'"))
        .unwrap();
    assert_eq!(
        schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}
fn seed(db: &Connection) {
    db.execute_batch("INSERT INTO host_instances(id,created_at,decision_seq) VALUES('i',10,3);
      INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s','i','resolved','native',1,11);
      INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',12,13);
      INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,actor_seat_id,author_role,relays_user,user_intent) VALUES('old','i','t',1,'ordinary',1,'original evidence',14,'s','agent',1,'request');
      INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('prep','i','scope','key',zeroblob(32),'t',0,0,0,0,0,0,0,'building');
      INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES('old','t','s','acked',100,14,114,'s',1,'cooperative_top_level',15);").unwrap();
}
fn evidence(db: &Connection) -> Vec<String> {
    ["SELECT json_array(id,body,decision_seq,decision_at,actor_seat_id,author_role,relays_user,user_intent) FROM messages ORDER BY ordinal",
     "SELECT json_array(id,operation_scope,operation_key,hex(digest),status) FROM send_preparations ORDER BY id",
     "SELECT json_array(message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) FROM receipts ORDER BY ordinal"]
        .into_iter().flat_map(|sql| db.prepare(sql).unwrap().query_map([], |r| r.get::<_,String>(0)).unwrap().map(Result::unwrap).collect::<Vec<_>>()).collect()
}
#[test]
fn registered27_preserves25_and26_history_and_rolls_back_late_ddl_failure() {
    for previous in [25, 26] {
        let db = historical(previous);
        seed(&db);
        if previous == 26 {
            db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-lazy','i','scope','lazy',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');
            INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-lazy','msg-lazy','t','s');
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,actor_seat_id,author_role,relays_user,user_intent,delivery_mode) VALUES('msg-lazy','i','t',2,'ordinary',2,'retained lazy evidence',16,'s','agent',0,NULL,'lazy');").unwrap();
        }
        let before = evidence(&db);
        if previous == 26 {
            db.execute_batch("CREATE TRIGGER bootstrap_report_retained BEFORE UPDATE ON messages BEGIN SELECT 1; END").unwrap();
            let lazy_before: String = db.query_row("SELECT json_array(preparation_id,message_id,thread_id,seat_id,state) FROM lazy_recipients", [], |r| r.get(0)).unwrap();
            let before_objects = objects(&db);
            assert!(schema::initialize(&db, || UtcMillis(0)).is_err());
            assert_eq!(version(&db), 26);
            assert_eq!(
                objects(&db),
                before_objects,
                "no partial tables/indexes/guards"
            );
            assert_eq!(evidence(&db), before);
            let lazy_after: String = db.query_row("SELECT json_array(preparation_id,message_id,thread_id,seat_id,state) FROM lazy_recipients", [], |r| r.get(0)).unwrap();
            assert_eq!(lazy_before, lazy_after);
            db.execute_batch("DROP TRIGGER bootstrap_report_retained")
                .unwrap();
        }
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        assert_eq!(version(&db), 27);
        assert_eq!(evidence(&db), before);
        let ordinary: String = db
            .query_row(
                "SELECT delivery_mode FROM messages WHERE id='old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ordinary, "ordinary");
        if previous == 26 {
            let lazy: String = db
                .query_row(
                    "SELECT delivery_mode FROM send_preparations WHERE id='prep-lazy'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(lazy, "lazy");
            let lazy_message: String = db
                .query_row(
                    "SELECT delivery_mode FROM messages WHERE id='msg-lazy'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(lazy_message, "lazy");
            let pending: i64 = db.query_row("SELECT count(*) FROM lazy_recipients WHERE preparation_id='prep-lazy' AND message_id='msg-lazy' AND thread_id='t' AND seat_id='s' AND state='pending'", [], |r| r.get(0)).unwrap();
            assert_eq!(pending, 1);
        }
    }
}
#[test]
fn historical25_and26_audits_run_before_installing27() {
    for (previous, trigger) in [
        (25, "digest_transition_warning_projected"),
        (26, "lazy_recipient_progress_forward"),
    ] {
        let db = historical(previous);
        db.execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
            ErrorCode::IncompatibleSchema
        );
        assert_eq!(version(&db), previous as i64);
        assert!(objects(&db).is_empty());
    }
}
// Keep this module's identity helpers tied to the same reviewed consumer API.
#[test]
fn registered27_identity_lookup_is_absent_without_a_begin() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    assert_eq!(
        herdr_threads::store::topology_handoff::current(&db, &namespace(), &identity()).unwrap(),
        None
    );
}

#[test]
fn legacy_key_index_retains_foreign_namespaces_with_bounded_lookup_after_1000_attempts() {
    use herdr_threads::{protocol::handoff::BootstrapAttempt, store::topology_handoff};
    let mut db = super::fixture();
    let id = identity();
    let mut other = id.clone();
    other.payload.handoff.namespace.state_dir = "/other-state".into();
    other.digest = other.semantic_digest().unwrap();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    topology_handoff::begin_pending(&tx, &other.payload.handoff.namespace, &other, UtcMillis(0))
        .unwrap();
    tx.execute(
        "UPDATE bootstrap_attempts SET state='not_submitted' WHERE parent_id=1 AND attempt=1",
        [],
    )
    .unwrap();
    for n in 2..=1001 {
        let attempt = BootstrapAttempt::new(n).unwrap();
        tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,?1,?2,?3,?4,?5,?6)",rusqlite::params![n,if n==1001{"prepared"}else{"not_submitted"},attempt.operation(&id.compound,"reserve").unwrap().as_str(),attempt.operation(&id.compound,"record").unwrap().as_str(),attempt.operation(&id.compound,"check").unwrap().as_str(),attempt.operation(&id.compound,"not_submitted").unwrap().as_str()]).unwrap();
    }
    tx.execute(
        "UPDATE bootstrap_handoffs SET current_attempt=1001 WHERE id=1",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let sql = "SELECT state_dir,host_endpoint FROM bootstrap_child_keys INDEXED BY bootstrap_keys_legacy_lookup WHERE instance_id='i' AND actor_scope='seat:s' AND operation_key='invite' LIMIT 2";
    let plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("instance_id=? AND actor_scope=? AND operation_key=?")),
        "{plan:?}"
    );
    let mut statement = db.prepare(sql).unwrap();
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "unscoped lookup must retain both namespaces for conservative refusal, not impose uniqueness or choose authority"
    );
    assert!(rows.contains(&("/state".into(), "/host.sock".into())));
    assert!(rows.contains(&("/other-state".into(), "/host.sock".into())));
    assert!(
        statement.get_status(rusqlite::StatementStatus::VmStep) < 100,
        "lookup must not scan all historical attempt keys"
    );
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap()
            .unwrap()
            .attempt
            .get(),
        1001
    );
}
