//! Characterizations of the serializer and linked SQLite assumptions used by
//! invitation presentation state. These do not exercise private state guards or
//! claim to test the complete populated-store migration.

use std::collections::HashMap;

use herdr_threads::protocol::ids::MessageId;
use rusqlite::{Connection, StatementStatus};

#[test]
fn ordinary_partial_worst_escaped_witness_preserves_exact_bytes() {
    // Every permitted ASCII-graphic ID byte needs at most two JSON bytes;
    // every raw UTF-8 byte needs at most six. Quotes/backslashes and NUL attain
    // those maxima. A one-entry JSON object adds seven structural bytes.
    let id = "\"\\".repeat(64);
    let parsed = MessageId::parse(id.clone()).expect("128-byte graphic ID");
    assert_eq!(parsed.as_str(), id);
    let body = "\0".repeat(8191);
    let partial = HashMap::from([(id.clone(), body.clone())]);
    let json = serde_json::to_string(&partial).expect("serialize witness");
    assert_eq!(json.len(), 49_409);
    assert!(json.len() <= 49_415);
    let restored: HashMap<String, String> = serde_json::from_str(&json).expect("round trip");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored.get(&id), Some(&body));
    assert_eq!(restored[&id].as_bytes(), vec![0; 8191]);

    // The serialized cap alone is insufficient to enforce raw length <8192:
    // the first forbidden raw length still fits exactly. D's canonical guard
    // tests must independently cover that semantic rejection.
    let at_raw_limit = HashMap::from([(id, "\0".repeat(8192))]);
    assert_eq!(
        serde_json::to_string(&at_raw_limit)
            .expect("serialize raw-limit witness")
            .len(),
        49_415
    );
}

fn populated(rows: i64) -> rusqlite::Result<Connection> {
    let db = Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE invitations(id INTEGER PRIMARY KEY, topic TEXT NOT NULL)")?;
    db.execute(
        "WITH RECURSIVE n(id) AS (SELECT 1 WHERE ?1>0 UNION ALL SELECT id+1 FROM n WHERE id<?1)
         INSERT INTO invitations(id,topic) SELECT id,'retained topic' FROM n",
        [rows],
    )?;
    Ok(db)
}

fn alter_steps(db: &Connection, sql: &str) -> rusqlite::Result<i32> {
    let mut statement = db.prepare(sql)?;
    statement.execute([])?;
    Ok(statement.get_status(StatementStatus::VmStep))
}

fn quick_check_steps(db: &Connection) -> rusqlite::Result<i32> {
    let mut statement = db.prepare("PRAGMA quick_check(invitations)")?;
    let results = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(results, ["ok"]);
    Ok(statement.get_status(StatementStatus::VmStep))
}

#[test]
fn linked_sqlite_default_column_alter_has_row_independent_vm_work() -> rusqlite::Result<()> {
    let mut default_steps = Vec::new();
    let mut checked_outer_steps = Vec::new();
    let mut direct_check_steps = Vec::new();
    for count in [0, 64, 4096] {
        let db = populated(count)?;
        let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        let steps = alter_steps(
            &db,
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0",
        )?;
        let (retained, defaults): (i64, i64) = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(epoch=0 AND topic='retained topic'),0) FROM invitations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!((retained, defaults), (count, count));
        assert!(steps > 0, "VM counter must be active");
        default_steps.push(steps);

        let checked = populated(count)?;
        let outer_steps = alter_steps(
            &checked,
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0)",
        )?;
        // ALTER's CHECK validation runs a nested pragma_quick_check VDBE.
        // Its outer statement counter is NOT a measurement of total nested
        // work. Direct quick_check below characterizes scan work separately.
        let scan_steps = quick_check_steps(&checked)?;
        checked_outer_steps.push(outer_steps);
        direct_check_steps.push(scan_steps);
        eprintln!(
            "sqlite_version={version} rows={count} default_alter_vm_steps={steps} checked_alter_outer_vm_steps={outer_steps} direct_quick_check_vm_steps={scan_steps} (outer CHECK counter excludes nested-work guarantee)"
        );
    }
    assert_eq!(default_steps[0], default_steps[1]);
    assert_eq!(default_steps[1], default_steps[2]);
    assert!(checked_outer_steps.iter().all(|steps| *steps > 0));
    assert!(direct_check_steps[1] > direct_check_steps[0]);
    assert!(direct_check_steps[2] > direct_check_steps[1]);
    Ok(())
}

#[test]
fn linked_sqlite_checked_column_validates_existing_rows_and_rolls_back() -> rusqlite::Result<()> {
    let db = populated(64)?;
    // Default zero is valid for the new column; only the last retained row
    // violates the cross-column CHECK. Success would silently skip old rows.
    let error = db
        .execute_batch(
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0 AND id<64)",
        )
        .expect_err("the existing last row must fail validation");
    assert!(
        error.to_string().contains("CHECK constraint failed"),
        "expected existing-row CHECK refusal, got {error}"
    );
    let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    eprintln!("sqlite_version={version} checked_alter_existing_row_refusal={error}");
    let epoch_columns: i64 = db.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('invitations') WHERE name='epoch'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(epoch_columns, 0, "failed ALTER must not leave the column");
    let retained: i64 = db.query_row(
        "SELECT COUNT(*) FROM invitations WHERE topic='retained topic'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(retained, 64);
    Ok(())
}

fn presentation_sqlite_measured(db: &Connection, sql: &str) -> rusqlite::Result<(i32, u64)> {
    use std::cell::Cell;
    use std::ffi::{c_int, c_void};

    // This callback is connection-local and never calls back into SQLite.
    // Saturation avoids any possible arithmetic panic crossing the C ABI.
    unsafe extern "C" fn count_instruction(context: *mut c_void) -> c_int {
        // SAFETY: presentation_sqlite_measured() passes a stable live Box<Cell<u64>> and removes
        // this callback before freeing it. SQLite invokes it synchronously on
        // the same thread; the private in-memory connection is not shared.
        let count = unsafe { &*context.cast::<Cell<u64>>() };
        count.set(count.get().saturating_add(1));
        0
    }
    struct ProgressGuard<'a>(&'a Connection);
    impl Drop for ProgressGuard<'_> {
        fn drop(&mut self) {
            // SAFETY: this live, exclusively used test connection owns the
            // handler. Unregister before the counter's storage is dropped,
            // including on a returned error or Rust panic.
            unsafe {
                rusqlite::ffi::sqlite3_progress_handler(
                    self.0.handle(),
                    0,
                    None,
                    std::ptr::null_mut(),
                );
            }
        }
    }
    let mut statement = db.prepare(sql)?;
    let count = Box::new(Cell::new(0_u64));
    // SAFETY: the Box's pointee cannot move. The guard unregisters before
    // count drops. No connection operation overlaps this synchronous
    // execute, and the callback returns zero without unwinding.
    unsafe {
        rusqlite::ffi::sqlite3_progress_handler(
            db.handle(),
            1,
            Some(count_instruction),
            (&*count as *const Cell<u64>).cast_mut().cast::<c_void>(),
        );
    }
    let guard = ProgressGuard(db);
    statement.execute([])?;
    let result = (statement.get_status(StatementStatus::VmStep), count.get());
    drop(guard);
    Ok(result)
}

#[test]
fn presentation_publication_scalar_checks_validate_retained_consumer_rows() -> rusqlite::Result<()>
{
    use rusqlite::{params_from_iter, types::Value};

    fn retained(db: &Connection, columns: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
        let mut statement = db.prepare(&format!(
            "SELECT {columns} FROM invitation_presentation_consumers ORDER BY seat_id,consumer"
        ))?;
        let width = statement.column_count();
        let mapped = statement.query_map([], |row| (0..width).map(|i| row.get(i)).collect())?;
        mapped.collect()
    }
    fn fixture(rows: usize, ddl: &str, consumer: &str) -> rusqlite::Result<(Connection, String)> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE seats(id TEXT PRIMARY KEY) STRICT;
             CREATE TABLE host_instances(id TEXT PRIMARY KEY) STRICT;
             INSERT INTO host_instances VALUES('instance');",
        )?;
        db.execute_batch(ddl)?;
        let columns = {
            let mut statement =
                db.prepare("PRAGMA table_info(invitation_presentation_consumers)")?;
            let mapped = statement.query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let names = columns
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let placeholders = vec!["?"; columns.len()].join(",");
        let insert = format!(
            "INSERT INTO invitation_presentation_consumers({names}) VALUES({placeholders})"
        );
        for n in 0..rows {
            let seat = format!("seat-{n:04}");
            db.execute("INSERT INTO seats VALUES(?1)", [&seat])?;
            let values = columns
                .iter()
                .map(|(name, kind)| match name.as_str() {
                    "seat_id" => Value::Text(seat.clone()),
                    "consumer" => Value::Text(consumer.into()),
                    "instance_id" => Value::Text("instance".into()),
                    "render_key" => Value::Text("a".repeat(64)),
                    "ordinary_cursor" | "ordinary_skip_body" | "outbox_token" | "last_token" => {
                        Value::Null
                    }
                    "ordinary_pending" => Value::Text(r#"{"items":[],"prefix":[]}"#.into()),
                    "ordinary_partial" | "control_state" => Value::Text("{}".into()),
                    "last_descriptors" | "outbox_prefix" | "outbox_descriptors" => {
                        Value::Text("[]".into())
                    }
                    "ordinary_done" | "outbox_more" => Value::Integer(1),
                    _ if kind == "INTEGER" => Value::Integer(2),
                    _ if kind == "TEXT" => Value::Text("retained".into()),
                    _ => panic!("unsupported V40 consumer fixture column {name}: {kind}"),
                })
                .collect::<Vec<_>>();
            db.execute(&insert, params_from_iter(values))?;
        }
        Ok((db, names))
    }

    // Use the complete ACTUAL V40 consumer table and its two unique indexes,
    // with minimal referenced parents. No copied/reduced consumer DDL.
    let v40 = include_str!("../../migrations/0040_invitation_presentation.sql");
    let start = v40
        .find("CREATE TABLE invitation_presentation_consumers (")
        .expect("V40 consumer DDL");
    let end = v40[start..]
        .find("CREATE TABLE invitation_presentation_items (")
        .expect("V40 consumer DDL end")
        + start;
    let ddl = &v40[start..end];
    let v41 = include_str!("../../migrations/0041_presentation_relevance.sql");
    let alters = v41
        .lines()
        .filter(|line| {
            line.starts_with("ALTER TABLE invitation_presentation_consumers ADD COLUMN ")
        })
        .take(4)
        .collect::<Vec<_>>();
    assert_eq!(alters.len(), 4, "the four scalar ALTERs must be present");
    let scalar_names = [
        "ordinary_publication_after",
        "ordinary_publication_high",
        "ordinary_passive_generation",
        "ordinary_scan_passive_generation",
    ];
    // CHECK(consumer IN ('hook','watch')) can execute different comparison
    // paths. Keep each size series homogeneous and measure BOTH supported
    // kinds independently, so row-count deltas never mix those paths.
    for consumer in ["hook", "watch"] {
        let mut checked_work = Vec::new();
        let mut control_work = Vec::new();
        for rows in [1, 32, 128] {
            let (db, old_columns) = fixture(rows, ddl, consumer)?;
            let (control, control_columns) = fixture(rows, ddl, consumer)?;
            let before = retained(&db, &old_columns)?;
            assert_eq!(before.len(), rows);
            let mut checked = Vec::new();
            let mut controls = Vec::new();
            let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
            for (sql, column) in alters.iter().zip(scalar_names) {
                let (outer, progress) = presentation_sqlite_measured(&db, sql)?;
                // Matched uncomplicated ADD COLUMN: only CHECK is removed.
                // The matched control isolates the added CHECK work; both may visit retained rows.
                let (prefix, _) = sql.split_once(" CHECK(").expect("scalar CHECK");
                let (control_outer, control_progress) =
                    presentation_sqlite_measured(&control, &format!("{prefix};"))?;
                assert!(outer > 0 && control_outer > 0 && progress > control_progress);
                assert_eq!(retained(&db, &old_columns)?, before);
                let zeroes: i64 = db.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM invitation_presentation_consumers WHERE {column}=0"
                    ),
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(zeroes, rows as i64);
                // Every added scalar independently enforces its actual CHECK;
                // rejected writes cannot corrupt defaults or pre-upgrade values.
                let error = db.execute(&format!("UPDATE invitation_presentation_consumers SET {column}=-1 WHERE seat_id='seat-0000'"), []).expect_err("negative scalar must fail");
                assert!(
                    error.to_string().contains("CHECK constraint failed"),
                    "{error}"
                );
                assert_eq!(retained(&db, &old_columns)?, before);
                let negative: i64 = db.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM invitation_presentation_consumers WHERE {column}<>0"
                    ),
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(negative, 0);
                eprintln!(
                    "sqlite_version={version} consumer={consumer} retained_consumers={rows} scalar={column} checked_outer_vm={outer} checked_including_nested_progress={progress} control_outer_vm={control_outer} control_progress={control_progress}"
                );
                checked.push(progress);
                controls.push(control_progress);
            }
            assert_eq!(retained(&control, &control_columns)?, before);
            checked_work.push(checked);
            control_work.push(controls);
        }
        for column in 0..4 {
            for work in [&checked_work, &control_work] {
                let small = work[1][column]
                    .checked_sub(work[0][column])
                    .expect("positive retained-row work");
                let total = work[2][column]
                    .checked_sub(work[0][column])
                    .expect("positive retained-row work");
                assert!(small > 0);
                // 127 added rows versus 31: bounded linear growth permits
                // fixed/page overhead; it does not guess an exact VM slope.
                assert!(total >= small * 4 && total <= small * 5);
            }
            let added = (0..3)
                .map(|size| {
                    checked_work[size][column]
                        .checked_sub(control_work[size][column])
                        .expect("CHECK adds work")
                })
                .collect::<Vec<_>>();
            assert!(added[0] > 0);
            // Isolate this linked fixture's added scalar CHECKs through this ALTER.
            // Infer its coefficient from the one-row sample, not a literal
            // VM-cost assumption or a claim that total work equals 4*S.
            assert_eq!(added[1], added[0] * 32);
            assert_eq!(added[2], added[0] * 128);
        }
    }
    Ok(())
}

#[test]
fn released_v39_invitation_epoch_alter_records_nested_work_and_preserves_state()
-> rusqlite::Result<()> {
    use rusqlite::{params, types::Value};

    fn snapshot(db: &Connection, sql: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
        let mut statement = db.prepare(sql)?;
        let width = statement.column_count();
        let mapped = statement.query_map([], |row| (0..width).map(|i| row.get(i)).collect())?;
        mapped.collect()
    }
    // Exact released migrations in order. Empty-schema SQL setup does not
    // substitute for the production migration runner or public invite path.
    let released = [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
        include_str!("../../migrations/0005_service_notifications.sql"),
        include_str!("../../migrations/0006_retirement_health.sql"),
        include_str!("../../migrations/0007_attention_digest.sql"),
        include_str!("../../migrations/0008_digest_pending_paths.sql"),
        include_str!("../../migrations/0009_human_occupant.sql"),
        include_str!("../../migrations/0010_b5_trust_guards.sql"),
        include_str!("../../migrations/0011_cooperative_only.sql"),
        include_str!("../../migrations/0012_harness_version_evidence.sql"),
        include_str!("../../migrations/0013_thread_summaries.sql"),
        include_str!("../../migrations/0014_catch_up_release.sql"),
        include_str!("../../migrations/0015_preparation_retention.sql"),
        include_str!("../../migrations/0016_human_receipt_waivers.sql"),
        include_str!("../../migrations/0017_wake_batches.sql"),
        include_str!("../../migrations/0018_warning_conditions.sql"),
        include_str!("../../migrations/0019_thread_names.sql"),
        include_str!("../../migrations/0020_recent_activity.sql"),
        include_str!("../../migrations/0021_invitation_rejections.sql"),
        include_str!("../../migrations/0022_user_message_intent.sql"),
        include_str!("../../migrations/0023_channel_archival.sql"),
        include_str!("../../migrations/0024_harness_contract_diagnostics.sql"),
        include_str!("../../migrations/0025_warning_notice_delivery.sql"),
        include_str!("../../migrations/0026_lazy_message_delivery.sql"),
        include_str!("../../migrations/0027_harness_adapters.sql"),
        include_str!("../../migrations/0028_handoff_topology.sql"),
        include_str!("../../migrations/0029_direct_threads.sql"),
        include_str!("../../migrations/0030_automatic_handoff.sql"),
        include_str!("../../migrations/0031_seat_wake_on.sql"),
        include_str!("../../migrations/0032_invitation_confirmation.sql"),
        include_str!("../../migrations/0033_receipt_send_time.sql"),
        include_str!("../../migrations/0034_handoff_body_bounds.sql"),
        include_str!("../../migrations/0035_receipt_deadline_backfill.sql"),
        include_str!("../../migrations/0036_invitation_lifecycle.sql"),
        include_str!("../../migrations/0037_invitation_warning_origin.sql"),
        include_str!("../../migrations/0038_receipt_warning_origins.sql"),
        include_str!("../../migrations/0039_automatic_handoff_body_bounds.sql"),
    ];
    let epoch_alter = include_str!("../../migrations/0040_invitation_presentation.sql")
        .lines()
        .find(|line| line.starts_with("ALTER TABLE invitations ADD COLUMN command_context_epoch "))
        .expect("exact V40 epoch ALTER");
    for rows in [1_i64, 32, 128] {
        let db = Connection::open_in_memory()?;
        // Some released migrations rebuild tables. Enable FK enforcement
        // after the empty historical schema is assembled, before any data.
        db.execute_batch("PRAGMA foreign_keys=OFF;")?;
        for migration in released {
            db.execute_batch(migration)?;
        }
        db.execute_batch("PRAGMA user_version=39; PRAGMA foreign_keys=ON;")?;
        let foreign_keys: i64 = db.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        assert_eq!(foreign_keys, 1);
        db.execute_batch(
            "INSERT INTO host_instances(id,created_at,decision_seq) VALUES('instance',0,1);
             INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('origin','instance','resolved','native',1,0);
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('thread','instance','retained topic','retained goal',0,0);
             INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at) VALUES('thread','origin',1,'joined','joined',0);
             INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('thread','origin',1,1);",
        )?;
        for n in 0..rows {
            let seat = format!("recipient-{n:04}");
            let invitation = format!("invitation-{n:04}");
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES(?1,'instance','resolved','native',1,0)", [&seat])?;
            db.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES('thread',?1,1,'invited','invited')", [&seat])?;
            db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES(?1,'thread',?2,1,'pending',0,?3,1000,1000,'require_ack','origin')", params![invitation,seat,n+2])?;
        }
        db.execute(
            "UPDATE host_instances SET decision_seq=?1 WHERE id='instance'",
            [rows + 1],
        )?;
        assert!(snapshot(&db, "PRAGMA foreign_key_check")?.is_empty());
        let names = {
            let mut statement = db.prepare("PRAGMA table_info(invitations)")?;
            let mapped = statement.query_map([], |row| row.get::<_, String>(1))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?.join(",")
        };
        let old_query = format!("SELECT {names} FROM invitations ORDER BY ordinal");
        let invitations_before = snapshot(&db, &old_query)?;
        assert_eq!(invitations_before.len(), rows as usize);
        let digest_query = "SELECT * FROM digest_pending_invitations ORDER BY ordinal";
        let digest_before = snapshot(&db, digest_query)?;
        assert_eq!(digest_before.len(), rows as usize);
        let catalog_query = "SELECT type,name,tbl_name,rootpage,sql FROM sqlite_schema WHERE name<>'invitations' ORDER BY type,name";
        let catalog_before = snapshot(&db, catalog_query)?;
        let fk_query = "PRAGMA foreign_key_list(invitations)";
        let fk_before = snapshot(&db, fk_query)?;
        let rootpage: i64 = db.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='invitations'",
            [],
            |row| row.get(0),
        )?;
        let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        let (outer, progress) = presentation_sqlite_measured(&db, epoch_alter)?;
        assert!(
            outer > 0 && progress > 0,
            "both linked counters must be active"
        );
        // Record actual nested work; neither constant nor linear complexity
        // is assumed from the absence of a CHECK on the new column.
        eprintln!(
            "sqlite_version={version} released39_invitations={rows} exact40_epoch_alter_outer_vm={outer} including_nested_progress={progress}"
        );
        assert_eq!(snapshot(&db, &old_query)?, invitations_before);
        assert_eq!(snapshot(&db, digest_query)?, digest_before);
        assert_eq!(snapshot(&db, catalog_query)?, catalog_before);
        assert_eq!(snapshot(&db, fk_query)?, fk_before);
        assert!(snapshot(&db, "PRAGMA foreign_key_check")?.is_empty());
        let after_rootpage: i64 = db.query_row(
            "SELECT rootpage FROM sqlite_schema WHERE name='invitations'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(after_rootpage, rootpage);
        let defaults: i64 = db.query_row(
            "SELECT COUNT(*) FROM invitations WHERE command_context_epoch=0",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(defaults, rows);
        let new_column: (String, i64, String) = db.query_row("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('invitations') WHERE name='command_context_epoch'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        assert_eq!(new_column, ("INTEGER".into(), 1, "0".into()));
        let user_version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        assert_eq!(
            user_version, 39,
            "this probe applies one ALTER, not full migration40"
        );
    }
    Ok(())
}
