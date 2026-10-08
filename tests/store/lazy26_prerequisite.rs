use herdr_threads::{
    app::SystemClock,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{path::PathBuf, sync::Arc};

struct Database {
    directory: PathBuf,
}

impl Database {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("ht-lazy26-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        Self { directory }
    }

    fn connection(&self) -> Connection {
        Connection::open(self.directory.join("store.db")).unwrap()
    }

    fn store(&self) -> Result<SqliteStore, herdr_threads::protocol::results::ApiError> {
        SqliteStore::new(
            StoreContext::new(
                self.directory.join("store.db"),
                Arc::new(SystemClock::default()),
            ),
            "i",
            StoreSettings::default(),
        )
    }

    fn historical25(&self) {
        let db = self.connection();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut migrations = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        migrations.sort();
        for path in migrations.into_iter().take(25) {
            db.execute_batch(&std::fs::read_to_string(path).unwrap())
                .unwrap();
        }
        db.pragma_update(None, "user_version", 25).unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at,decision_seq) VALUES('i',10,3);
            INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s','i','resolved','native',1,11);
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',12,13);
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,actor_seat_id,author_role,relays_user,user_intent) VALUES('msg-old','i','t',1,'ordinary',1,'original evidence',14,'s','agent',1,'request');
            INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('prep-old','i','scope','key',zeroblob(32),'t',0,0,0,0,0,0,0,'building');
            INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES('msg-old','t','s','acked',100,14,114,'s',1,'cooperative_top_level',15);").unwrap();
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn version(db: &Connection) -> i64 {
    db.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn evidence(db: &Connection) -> Vec<String> {
    [
        "SELECT json_array(id,body,decision_seq,decision_at,actor_seat_id,author_role,relays_user,user_intent,author_role_backfilled) FROM messages ORDER BY ordinal",
        "SELECT json_array(id,operation_scope,operation_key,hex(digest),status) FROM send_preparations ORDER BY id",
        "SELECT json_array(message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,ack_required) FROM receipts ORDER BY ordinal",
    ].into_iter().flat_map(|sql| db.prepare(sql).unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().map(Result::unwrap).collect::<Vec<_>>()).collect()
}

// Missing startup registration leaves a new Store unable to record ordinary mode.
#[test]
fn fresh_store_installs26_and_reopens() {
    let database = Database::new();
    drop(database.store().unwrap());
    let db = database.connection();
    assert_eq!(version(&db), 26);
    for table in ["messages", "send_preparations"] {
        let default: String = db
            .query_row(
                "SELECT dflt_value FROM pragma_table_info(?1) WHERE name='delivery_mode'",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default, "'ordinary'");
    }
    drop(db);
    drop(database.store().unwrap());
}

// Missing/rewriting upgrade loses the recorded message and ACK attribution.
#[test]
fn historical25_store_upgrade_preserves_evidence_and_defaults_modes() {
    let database = Database::new();
    database.historical25();
    let db = database.connection();
    assert_eq!(version(&db), 25);
    let before = evidence(&db);
    drop(db);
    drop(database.store().unwrap());
    let db = database.connection();
    assert_eq!(version(&db), 26);
    assert_eq!(evidence(&db), before);
    for table in ["messages", "send_preparations"] {
        let mode: String = db
            .query_row(&format!("SELECT delivery_mode FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(mode, "ordinary");
    }
    let recipients: i64 = db
        .query_row("SELECT count(*) FROM lazy_recipients", [], |row| row.get(0))
        .unwrap();
    assert_eq!(recipients, 0);
    drop(db);
    drop(database.store().unwrap());
    assert_eq!(evidence(&database.connection()), before);
}

// Removing the upgrade transaction leaves partial columns after a DDL failure.
#[test]
fn failed26_upgrade_rolls_back_columns_objects_and_version() {
    let database = Database::new();
    database.historical25();
    let db = database.connection();
    db.execute_batch("CREATE INDEX lazy_recipients_preparation_ordinal ON messages(ordinal)")
        .unwrap();
    let before = evidence(&db);
    drop(db);
    assert!(
        database.store().is_err(),
        "conflicting index must fail upgrade"
    );
    let db = database.connection();
    assert_eq!(version(&db), 25);
    assert_eq!(evidence(&db), before);
    for table in ["messages", "send_preparations"] {
        let columns: i64 = db
            .query_row(
                "SELECT count(*) FROM pragma_table_info(?1) WHERE name='delivery_mode'",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(columns, 0, "partial migration on {table}");
    }
    let objects: Vec<String> = db
        .prepare("SELECT name FROM sqlite_schema WHERE name LIKE 'lazy_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(objects, ["lazy_recipients_preparation_ordinal"]);
}

fn rejected_on_reopen(database: &Database, description: &str) {
    let error = match database.store() {
        Ok(_) => panic!("Store accepted {description}"),
        Err(error) => error,
    };
    assert_eq!(
        error.code,
        herdr_threads::protocol::results::ErrorCode::IncompatibleSchema,
        "{description}: {error:?}"
    );
    assert_eq!(version(&database.connection()), 26);
}

// Omitting the v26 startup audit accepts a removed or weakened immutable guard.
#[test]
fn reopen_rejects_missing_and_altered_lazy_triggers() {
    for trigger in [
        "lazy_preparation_mode_immutable",
        "lazy_message_mode_immutable",
        "lazy_message_shape",
        "lazy_recipient_identity_immutable",
        "lazy_recipient_progress_forward",
        "lazy_recipient_source",
        "lazy_recipient_display_published",
        "lazy_recipient_published_retained",
        "lazy_manifest_source",
    ] {
        for altered in [false, true] {
            let database = Database::new();
            drop(database.store().unwrap());
            let db = database.connection();
            db.execute_batch(&format!("DROP TRIGGER {trigger}"))
                .unwrap();
            if altered {
                db.execute_batch(&format!(
                    "CREATE TRIGGER {trigger} BEFORE INSERT ON messages BEGIN SELECT 1; END;"
                ))
                .unwrap();
            }
            drop(db);
            rejected_on_reopen(&database, &format!("{trigger}, altered={altered}"));
        }
    }
}

// Auditing only trigger names misses a removed or altered lazy table/index.
#[test]
fn reopen_rejects_missing_and_altered_lazy_table_and_indexes() {
    for mutation in [
        "DROP TABLE lazy_recipients",
        "DROP INDEX lazy_recipients_pending_seat_ordinal",
        "DROP INDEX lazy_recipients_preparation_ordinal",
        "DROP INDEX lazy_recipients_pending_seat_ordinal; CREATE INDEX lazy_recipients_pending_seat_ordinal ON lazy_recipients(ordinal)",
        "DROP INDEX lazy_recipients_preparation_ordinal; CREATE INDEX lazy_recipients_preparation_ordinal ON lazy_recipients(ordinal)",
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,\"CHECK(state IN ('pending','displayed'))\",\"CHECK(state IN ('pending','displayed','other'))\") WHERE type='table' AND name='lazy_recipients'; PRAGMA writable_schema=OFF;",
    ] {
        let database = Database::new();
        drop(database.store().unwrap());
        database.connection().execute_batch(mutation).unwrap();
        rejected_on_reopen(&database, mutation);
    }
}

// Auditing only lazy objects misses loss of ordinary defaults or mode checks.
#[test]
fn reopen_rejects_missing_or_weakened_mode_columns() {
    const COLUMN: &str = "delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'))";
    for table in ["messages", "send_preparations"] {
        for replacement in [
            "",
            "delivery_mode TEXT NOT NULL DEFAULT 'lazy' CHECK(delivery_mode IN ('ordinary','lazy'))",
            "delivery_mode TEXT DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'))",
            "delivery_mode TEXT NOT NULL DEFAULT 'ordinary'",
            "delivery_mode BLOB NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'))",
        ] {
            let database = Database::new();
            drop(database.store().unwrap());
            let db = database.connection();
            let sql: String = db
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(sql.contains(COLUMN));
            let altered = if replacement.is_empty() {
                sql.replace(&format!(", {COLUMN}"), "")
            } else {
                sql.replace(COLUMN, replacement)
            };
            assert_ne!(altered, sql);
            db.execute_batch("PRAGMA writable_schema=ON").unwrap();
            db.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name=?2",
                rusqlite::params![altered, table],
            )
            .unwrap();
            db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
            drop(db);
            rejected_on_reopen(&database, &format!("{table}: {replacement}"));
        }
    }
}
