use super::{identity, namespace};
use herdr_threads::{
    protocol::{results::ErrorCode, time::UtcMillis},
    store::schema,
};
use rusqlite::{Connection, OptionalExtension};

// Build genuine historical schemas from their immutable migration chain, never
// relabel a latest database by rewinding user_version. Replaying the chain
// costs far more than each audit case, so each test process replays it once,
// keeps the image after every prefix, and each call deserializes a private,
// independent copy of the requested prefix (with its user_version).
fn historical(version: usize) -> Connection {
    use rusqlite::ffi;
    static IMAGES: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    let images = IMAGES.get_or_init(|| {
        let db = Connection::open_in_memory().unwrap();
        let mut paths =
            std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"))
                .unwrap()
                .map(|p| p.unwrap().path())
                .collect::<Vec<_>>();
        paths.sort();
        let mut images = Vec::new();
        for (applied, path) in std::iter::once(None)
            .chain(paths.into_iter().take(47).map(Some))
            .enumerate()
        {
            if let Some(path) = path {
                db.execute_batch(&std::fs::read_to_string(path).unwrap())
                    .unwrap();
            }
            db.pragma_update(None, "user_version", applied as i64)
                .unwrap();
            let mut size: ffi::sqlite3_int64 = 0;
            // SAFETY: serialize the live main schema of our own open connection;
            // the returned sqlite3_malloc buffer is copied and freed here.
            images.push(unsafe {
                let data = ffi::sqlite3_serialize(db.handle(), c"main".as_ptr(), &mut size, 0);
                assert!(!data.is_null(), "serialize historical {applied}");
                let image = std::slice::from_raw_parts(data, size as usize).to_vec();
                ffi::sqlite3_free(data.cast());
                image
            });
        }
        images
    });
    let image = &images[version];
    let db = Connection::open_in_memory().unwrap();
    // SAFETY: SQLite takes ownership of the sqlite3_malloc64 copy (FREEONCLOSE)
    // and may grow it (RESIZEABLE); the connection handle is ours and open.
    unsafe {
        let buffer = ffi::sqlite3_malloc64(image.len() as u64).cast::<u8>();
        assert!(!buffer.is_null(), "allocate historical {version}");
        std::ptr::copy_nonoverlapping(image.as_ptr(), buffer, image.len());
        let size = image.len() as ffi::sqlite3_int64;
        let rc = ffi::sqlite3_deserialize(
            db.handle(),
            c"main".as_ptr(),
            buffer,
            size,
            size,
            (ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE) as _,
        );
        assert_eq!(rc, ffi::SQLITE_OK, "deserialize historical {version}");
    }
    assert_eq!(self::version(&db), version as i64);
    db
}
// Today's begin_pending authority check reads the session-keyed binding columns
// of published schema 32, so it cannot run against a genuine older prefix such
// as 28. For such a prefix, run the current producer on a genuine 38 prefix
// (whose bootstrap_* DDL is unchanged from 28) seeded by the same `setup`, and
// replay the exact rows it wrote into the genuine old database; the old
// schema's own triggers derive the attempt keys, which must match.
fn begin_pending_at(
    db: &Connection,
    previous: usize,
    setup: impl Fn(&Connection),
    id: &herdr_threads::protocol::handoff::BootstrapIdentity,
    now: UtcMillis,
) {
    let begin = |db: &Connection| {
        let tx = db.unchecked_transaction().unwrap();
        herdr_threads::store::topology_handoff::begin_pending(&tx, &namespace(), id, now).unwrap();
        tx.commit().unwrap();
    };
    setup(db);
    if previous >= 32 {
        begin(db);
        return;
    }
    let producer = historical(38);
    setup(&producer);
    begin(&producer);
    let rows = |db: &Connection, table: &str, filter: &str| {
        let mut select = db
            .prepare(&format!("SELECT * FROM {table} {filter} ORDER BY rowid"))
            .unwrap();
        let columns: Vec<String> = select
            .column_names()
            .into_iter()
            .map(String::from)
            .collect();
        let rows = select
            .query_map([], |row| {
                (0..columns.len())
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (columns, rows)
    };
    // One transaction: the parent's current-attempt reference is a deferred FK.
    let tx = db.unchecked_transaction().unwrap();
    for (table, filter) in [
        ("bootstrap_handoffs", ""),
        ("bootstrap_child_keys", "WHERE attempt=0"),
        ("bootstrap_attempts", ""),
    ] {
        let (columns, written) = rows(&producer, table, filter);
        let insert = format!(
            "INSERT INTO {table}({}) VALUES({})",
            columns.join(","),
            vec!["?"; columns.len()].join(",")
        );
        for row in written {
            tx.execute(&insert, rusqlite::params_from_iter(row))
                .unwrap();
        }
    }
    tx.commit().unwrap();
    for table in [
        "bootstrap_handoffs",
        "bootstrap_child_keys",
        "bootstrap_attempts",
        "bootstrap_recovery_decisions",
        "bootstrap_attachments",
        "bootstrap_reports",
    ] {
        assert_eq!(rows(db, table, ""), rows(&producer, table, ""), "{table}");
    }
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
fn registered28_fresh_and_genuine_historical_chains_reopen() {
    for previous in [
        0, 1, 15, 22, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43,
        44, 45, 46,
    ] {
        let db = historical(previous);
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        assert_eq!(version(&db), 47, "actual {previous}->latest");
        assert_eq!(objects(&db), objects(&historical(47)));
        schema::initialize(&db, || UtcMillis(1)).unwrap();
        assert_eq!(version(&db), 47);
    }
}
#[test]
fn registered28_audits_every_table_index_and_complete_trigger_body() {
    let pristine = historical(28);
    schema::initialize(&pristine, || UtcMillis(0)).unwrap();
    for (kind, name, sql) in objects(&pristine) {
        // Missing objects must fail, including every guard and every lookup index.
        let db = historical(28);
        db.execute_batch(&format!("DROP {kind} {name}")).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0)).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "missing {name}"
        );
        // Full SQL must be audited, beyond an object's name and column inventory.
        let db = historical(28);
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
    let db = historical(28);
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
    let db = historical(28);
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
fn registered28_preserves25_and26_history_and_rolls_back_late_ddl_failure() {
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
            assert_eq!(
                version(&db),
                27,
                "adapter27 committed before topology28 rollback"
            );
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
        assert_eq!(version(&db), 47);
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
fn registered28_identity_lookup_is_absent_without_a_begin() {
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

#[test]
fn handoff_body_bounds_upgrade_preserves_existing_identity_bytes_and_keys() {
    for previous in [28, 36, 37, 38] {
        let db = historical(previous);
        let mut id = identity();
        id.payload.handoff.channel = herdr_threads::protocol::handoff::HandoffChannel::New {
            name: None,
            topic: "topic".into(),
            goal: "goal".into(),
        };
        id.digest = id.semantic_digest().unwrap();
        begin_pending_at(
            &db,
            previous,
            |db| {
                db.execute_batch(
                    "PRAGMA foreign_keys=ON; INSERT INTO host_instances(id,created_at) VALUES('i',0)",
                )
                .unwrap();
                super::thread(db);
                super::joined_agent(db);
            },
            &id,
            UtcMillis(0),
        );
        assert_eq!(
            version(&db),
            previous as i64,
            "seeding must preserve the genuine old schema"
        );
        let bytes: Vec<u8> = db
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get(0)
            })
            .unwrap();
        schema::initialize(&db, || UtcMillis(1)).unwrap();
        assert_eq!(version(&db), 47);
        let retained: Vec<u8> = db
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(retained, bytes);
        assert_eq!(
            db.pragma_query_value::<i64, _>(None, "foreign_keys", |r| r.get(0))
                .unwrap(),
            1
        );
        assert!(
            herdr_threads::store::topology_handoff::current(&db, &namespace(), &id)
                .unwrap()
                .is_some()
        );
        let mut max = id;
        max.compound = herdr_threads::protocol::ids::OperationId::new("maximum");
        max.payload.handoff.keys.compound = max.compound.clone();
        max.payload.handoff.keys.begin =
            herdr_threads::protocol::ids::OperationId::new("max-begin");
        max.payload.handoff.keys.create =
            herdr_threads::protocol::ids::OperationId::new("max-create");
        max.payload.handoff.keys.invite =
            herdr_threads::protocol::ids::OperationId::new("max-invite");
        max.payload.handoff.keys.send = herdr_threads::protocol::ids::OperationId::new("max-send");
        max.payload.handoff.keys.complete =
            herdr_threads::protocol::ids::OperationId::new("max-complete");
        max.payload.handoff_key = herdr_threads::protocol::ids::OperationId::new("max-handoff");
        max.payload.resolve_key = herdr_threads::protocol::ids::OperationId::new("max-resolve");
        max.payload.attach_key = herdr_threads::protocol::ids::OperationId::new("max-attach");
        max.payload.linked_complete_key =
            herdr_threads::protocol::ids::OperationId::new("max-linked-complete");
        max.payload.handoff.body = "\u{1}".repeat(65_536);
        max.digest = max.semantic_digest().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        herdr_threads::store::topology_handoff::begin_pending(
            &tx,
            &namespace(),
            &max,
            UtcMillis(2),
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(
            herdr_threads::store::topology_handoff::current(&db, &namespace(), &max)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn handoff_bounds_upgrade_preserves_populated_completed_and_recovery_history() {
    use herdr_threads::protocol::{handoff::*, ids::*};
    use sha2::{Digest, Sha256};
    for previous in [28, 36, 37, 38] {
        let db = historical(previous);
        let id = identity();
        begin_pending_at(
            &db,
            previous,
            |db| {
                db.execute_batch("PRAGMA foreign_keys=ON; INSERT INTO host_instances(id,created_at) VALUES('i',0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0)").unwrap();
                super::joined_agent(db);
            },
            &id,
            UtcMillis(0),
        );
        assert_eq!(
            version(&db),
            previous as i64,
            "seeding must preserve the genuine old schema"
        );
        let tx = db.unchecked_transaction().unwrap();
        super::record_created(&tx);
        let recovery = super::saved_recovery(
            &tx,
            &id,
            BootstrapRecoveryDisposition::CreatedPane {
                evidence: super::created(),
                structural_reference: HostCallId::new("inspection"),
            },
            BootstrapState::Created,
            Some(super::created()),
        );
        let attachment = BootstrapAttachment {
            attempt: BootstrapAttempt::first(),
            created: super::created(),
            resolve_operation: id.payload.resolve_key.clone(),
            resolved_seat: SeatId::new("peer"),
            handoff: HandoffIdentity {
                compound: id.payload.handoff_key.clone(),
                digest: "b".repeat(64),
                claim: id.claim.clone(),
                thread: Some(ThreadId::new("t")),
                recipient: SeatId::new("peer"),
                create_key: id.payload.handoff.keys.create.clone(),
                invite_key: id.payload.handoff.keys.invite.clone(),
                send_key: id.payload.handoff.keys.send.clone(),
            },
        };
        attachment.validate(&id).unwrap();
        let report = serde_json::json!({"outcome":"started","pane":"w:p2","seat":"peer","harness":"codex","argv":id.payload.launch.argv});
        let completed = CompletedBootstrapResult {
            identity: id.clone(),
            attachment: attachment.clone(),
            retained: LinkedBootstrapReport {
                thread: ThreadId::new("t"),
                recipient: SeatId::new("peer"),
                pane: HostTargetId::new("w:p2"),
                kind: "launch".into(),
                launch: id.payload.launch.clone(),
                report_digest: format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&report).unwrap())
                ),
                report,
                terminal: super::created().terminal,
                host_incarnation: super::created().host_incarnation,
            },
            legacy_result: HandoffResult {
                compound: id.payload.handoff_key.clone(),
                thread: Some(ThreadId::new("t")),
                state: HandoffState::Completed,
            },
        };
        completed.retained.validate().unwrap();
        tx.execute(
            "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
            [serde_json::to_vec(&attachment).unwrap()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO bootstrap_reports(parent_id,completed_json) VALUES(1,?1)",
            [serde_json::to_vec(&completed).unwrap()],
        )
        .unwrap();
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='completed',terminal_at=1 WHERE id=1",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
        let before = herdr_threads::store::topology_handoff::current(&db, &namespace(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(before.completed, Some(Box::new(completed)));
        assert_eq!(before.recovery, Some(Box::new(recovery)));
        let dump = |db: &Connection| {
            [
                "bootstrap_handoffs",
                "bootstrap_attempts",
                "bootstrap_child_keys",
                "bootstrap_recovery_decisions",
                "bootstrap_attachments",
                "bootstrap_reports",
            ]
            .into_iter()
            .map(|table| {
                let mut statement = db
                    .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                    .unwrap();
                let columns = statement.column_count();
                statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|column| row.get::<_, rusqlite::types::Value>(column))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            })
            .collect::<Vec<_>>()
        };
        let roots = |db: &Connection| {
            db.prepare("SELECT name,rootpage FROM sqlite_schema WHERE rootpage>0 AND name NOT LIKE 'bootstrap_%' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('receipts','receipt_state') AND name NOT LIKE 'receipts_%' AND name NOT LIKE 'receipt_state_%' AND name NOT IN ('automatic_handoff_companions','automatic_bootstrap_parents','automatic_bootstrap_attachments','automatic_bootstrap_recoveries') AND (?1>=32 OR tbl_name<>'occupant_bindings') AND (?1>=34 OR tbl_name<>'catch_up') AND (?1>=35 OR tbl_name NOT IN ('warning_offer','digest_notice_offer')) ORDER BY name")
            .unwrap().query_map([previous as i64], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>()
        };
        let original = dump(&db);
        struct PrivateDb(std::path::PathBuf);
        impl Drop for PrivateDb {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let private = PrivateDb(std::env::temp_dir().join(format!(
            "ht-handoff-bounds-upgrade-{}.db",
            uuid::Uuid::new_v4()
        )));
        db.execute("VACUUM INTO ?1", [private.0.to_str().unwrap()])
            .unwrap();
        drop(db);
        let db = Connection::open(&private.0).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        assert_eq!(version(&db), previous as i64);
        let old_roots = roots(&db);
        let old_cutover: Option<i64> = if previous == 38 {
            db.query_row("SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'", [], |r| r.get(0)).optional().unwrap()
        } else {
            None
        };
        schema::initialize(&db, || UtcMillis(2)).unwrap();
        assert_eq!(version(&db), 47);
        let new_roots = roots(&db);
        for root in old_roots {
            assert!(new_roots.contains(&root), "retained root {root:?}");
        }
        if previous == 38 {
            assert_eq!(db.query_row("SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='i'", [], |r| r.get::<_,i64>(0)).optional().unwrap(), old_cutover);
        }
        assert_eq!(dump(&db), original);
        drop(db);
        let db = Connection::open(&private.0).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        schema::initialize(&db, || UtcMillis(3)).unwrap();
        assert_eq!(
            herdr_threads::store::topology_handoff::current(&db, &namespace(), &id)
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(
            db.pragma_query_value::<i64, _>(None, "foreign_keys", |r| r.get(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn handoff_bounds_upgrade_failure_restores_fk_and_retries_without_recapturing_receipt_cutover() {
    for previous in [28, 36, 37, 38] {
        for foreign_keys in [false, true] {
            let db = historical(previous);
            db.execute_batch(
                "INSERT INTO host_instances(id,created_at,decision_seq) VALUES('i',0,7)",
            )
            .unwrap();
            db.pragma_update(None, "foreign_keys", foreign_keys)
                .unwrap();
            // Fail inside handoff bounds (39) after receipt send time (38) has
            // committed, then repair only the local obstruction and retry the
            // same actual old database.
            db.execute_batch("CREATE TEMP TABLE ht29_handoffs(obstruction TEXT)")
                .unwrap();
            assert!(schema::initialize(&db, || UtcMillis(0)).is_err());
            assert_eq!(version(&db), 38);
            assert_eq!(
                db.pragma_query_value::<bool, _>(None, "foreign_keys", |r| r.get(0))
                    .unwrap(),
                foreign_keys
            );
            assert_eq!(
                objects(&db),
                objects(&historical(38)),
                "failed39 must retain old bootstrap DDL"
            );
            let cutover: Vec<(String, i64)> = db.prepare("SELECT instance_id,historical_decision_seq FROM receipt_send_time_cutovers ORDER BY instance_id").unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
            db.execute_batch("DROP TABLE temp.ht29_handoffs; UPDATE host_instances SET decision_seq=99 WHERE id='i'").unwrap();
            schema::initialize(&db, || UtcMillis(1)).unwrap();
            assert_eq!(version(&db), 47);
            schema::initialize(&db, || UtcMillis(2)).unwrap();
            let retained: Vec<(String, i64)> = db.prepare("SELECT instance_id,historical_decision_seq FROM receipt_send_time_cutovers ORDER BY instance_id").unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?))).unwrap().map(Result::unwrap).collect();
            assert_eq!(
                retained, cutover,
                "retry39 and reopen must never recapture cutover38"
            );
            assert_eq!(
                db.pragma_query_value::<bool, _>(None, "foreign_keys", |r| r.get(0))
                    .unwrap(),
                foreign_keys
            );
            assert!(
                db.prepare("PRAGMA foreign_key_check")
                    .unwrap()
                    .query([])
                    .unwrap()
                    .next()
                    .unwrap()
                    .is_none()
            );
        }
    }
}
