//! Serializer/SQLite characterizations and binding-scoped presentation controls.
//! The migration probes exercise individual statements, not the complete
//! populated-store production migration runner.

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
             CREATE TABLE occupant_bindings(binding_id TEXT PRIMARY KEY) STRICT;
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
            db.execute("INSERT INTO occupant_bindings VALUES(?1)", [&seat])?;
            let values = columns
                .iter()
                .map(|(name, kind)| match name.as_str() {
                    "seat_id" | "binding_id" => Value::Text(seat.clone()),
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
                    _ => panic!("unsupported V44 consumer fixture column {name}: {kind}"),
                })
                .collect::<Vec<_>>();
            db.execute(&insert, params_from_iter(values))?;
        }
        Ok((db, names))
    }

    // Use the complete ACTUAL V44 consumer table and its two unique indexes,
    // with minimal referenced parents. No copied/reduced consumer DDL.
    let v44 = include_str!("../../migrations/0044_invitation_presentation.sql");
    let start = v44
        .find("CREATE TABLE invitation_presentation_consumers (")
        .expect("V44 consumer DDL");
    let end = v44[start..]
        .find("CREATE TABLE invitation_presentation_items (")
        .expect("V44 consumer DDL end")
        + start;
    let ddl = &v44[start..end];
    let v45 = include_str!("../../migrations/0045_presentation_relevance.sql");
    let alters = v45
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
fn composed_v43_invitation_epoch_alter_records_nested_work_and_preserves_state()
-> rusqlite::Result<()> {
    use rusqlite::{params, types::Value};

    fn snapshot(db: &Connection, sql: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
        let mut statement = db.prepare(sql)?;
        let width = statement.column_count();
        let mapped = statement.query_map([], |row| (0..width).map(|i| row.get(i)).collect())?;
        mapped.collect()
    }
    // Exact released Main 1..35 followed by composed 36..43. Empty-schema SQL setup does not
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
        include_str!("../../migrations/0032_session_bindings.sql"),
        include_str!("../../migrations/0033_archival_scan_scope.sql"),
        include_str!("../../migrations/0034_binding_delivery_state.sql"),
        include_str!("../../migrations/0035_binding_attention_frontiers.sql"),
        include_str!("../../migrations/0036_invitation_confirmation.sql"),
        include_str!("../../migrations/0037_receipt_send_time.sql"),
        include_str!("../../migrations/0038_handoff_body_bounds.sql"),
        include_str!("../../migrations/0039_receipt_deadline_backfill.sql"),
        include_str!("../../migrations/0040_invitation_lifecycle.sql"),
        include_str!("../../migrations/0041_invitation_warning_origin.sql"),
        include_str!("../../migrations/0042_receipt_warning_origins.sql"),
        include_str!("../../migrations/0043_automatic_handoff_body_bounds.sql"),
    ];
    let epoch_alter = include_str!("../../migrations/0044_invitation_presentation.sql")
        .lines()
        .find(|line| line.starts_with("ALTER TABLE invitations ADD COLUMN command_context_epoch "))
        .expect("exact V44 epoch ALTER");
    for rows in [1_i64, 32, 128] {
        let db = Connection::open_in_memory()?;
        // Some released migrations rebuild tables. Enable FK enforcement
        // after the empty historical schema is assembled, before any data.
        db.execute_batch("PRAGMA foreign_keys=OFF;")?;
        for migration in released {
            db.execute_batch(migration)?;
        }
        db.execute_batch("PRAGMA user_version=43; PRAGMA foreign_keys=ON;")?;
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
            "sqlite_version={version} composed43_invitations={rows} exact44_epoch_alter_outer_vm={outer} including_nested_progress={progress}"
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
            user_version, 43,
            "this probe applies one ALTER, not full migration44"
        );
    }
    Ok(())
}

// These store-boundary tests deliberately use two connected sessions with the
// same seat generation. A seat-keyed presentation slot, latest-binding notice
// frontier, or receipt-only recovery eligibility breaks their assertions.
mod binding_presentation {
    use herdr_threads::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::{PresentationInput, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            delivery::{
                DeliveryPageV2, DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationItemV2,
                PresentationRenderRequest,
            },
            ids::{BindingId, ExecutionId, HostTargetId, NativeSessionId, SeatId},
            time::{CallBudget, Clock, MonoInstant, UtcMillis},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use rusqlite::Connection;
    use std::{path::PathBuf, sync::Arc};
    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(100_000),
            cancellation: Default::default(),
        }
    }
    struct Fixture {
        store: SqliteStore,
        db: Connection,
        directory: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    impl Fixture {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let directory =
                std::env::temp_dir().join(format!("bound-presentation-{}", uuid::Uuid::new_v4()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            let context = StoreContext::new(directory.join("store.db"), Arc::new(FixedClock));
            let db = context.open_writer().unwrap();
            db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES('i',0,'boot',1,100);
                INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('s','i','resolved','native','p',1,0,0);
                INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES('t','i','topic','goal',0,0,1);
                INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES('t','s','joined','joined');
                INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s',1,1);").unwrap();
            for name in ["a", "b"] {
                db.execute("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES(?1,'s',1,0,'p','boot',1,'claude',?1,?1,'cooperative_env_session',0,0)", [name]).unwrap();
                db.execute(
                    "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES(?1,'t',0)",
                    [name],
                )
                .unwrap();
            }
            let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
            Self {
                store,
                db,
                directory,
            }
        }
        fn claim(&self, name: &str) -> CallerClaim {
            CallerClaim {
                instance: "i".into(),
                seat: SeatId::new("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Claude,
                native_session: NativeSessionId::new(name),
                execution: ExecutionId::new(name),
                target: HostTargetId::new("p"),
                agent_id: None,
                liveness: None,
            }
        }
        fn input(&self, name: &str) -> PresentationInput {
            let claim = self.claim(name);
            let frozen_render = self
                .store
                .presentation_render_selection(&claim, PresentationConsumer::Watch, &budget())
                .unwrap()
                .map(|identity| FrozenPresentationRender {
                    identity,
                    codec: None,
                });
            PresentationInput {
                binding_id: BindingId::new(name),
                owner_replay: None,
                ordinary_due_snapshot: None,
                frozen_render,
                render_profile: PresentationRenderProfile::WatchJsonLinesV1,
                pull: DeliveryPullV2 {
                    claim,
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    marker_prefix: vec![],
                },
                attention_version: 0,
                ordinary_version: 0,
                with_attention: false,
                retry_messages: true,
                blocked_messages: vec![],
                known_messages: vec![],
                new_message_slots: 256,
            }
        }
        fn pull(&self, name: &str) -> DeliveryPageV2 {
            match self
                .store
                .presentation_pull(self.input(name), &budget())
                .unwrap()
                .reply
            {
                DeliveryPullReplyV2::Page(page) => page,
                other => panic!("expected bound page: {other:?}"),
            }
        }
        fn notice(&self) {
            self.db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES('notice','i','t',50,'warn','{}',0,50,1);
                INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES('s','notice','t',50,1);").unwrap();
        }
        fn confirm(&self, name: &str, page: &DeliveryPageV2) -> DeliveryPresentationConfirmationV2 {
            self.store
                .presentation_confirm(
                    &DeliveryPresentationConfirmV2 {
                        claim: self.claim(name),
                        consumer: PresentationConsumer::Watch,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        token: page
                            .presentation_token
                            .clone()
                            .expect("notice outbox token"),
                        emitted_controls: page.emitted_controls.clone(),
                    },
                    &budget(),
                )
                .unwrap()
                .status
        }
        fn ordinary(&self) {
            self.db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('m','i','t',1,'ordinary','unread by b',0,3);
                INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('prep','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
                INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('prep','t','s',1,300000,1);
                INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','prep','m','t',3,0,1,0,1,0);
                INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES('m','s','acked',7,'a','s',1,'original');
                UPDATE binding_cursors SET seq=1 WHERE binding_id='a';").unwrap();
        }
    }
    #[test]
    fn same_generation_binding_outboxes_and_notice_frontiers_are_independent() {
        let f = Fixture::new();
        f.notice();
        let a = f.pull("a");
        assert!(
            a.items
                .iter()
                .any(|v| matches!(v, PresentationItemV2::Notice { .. }))
        );
        let b = f.pull("b");
        assert!(
            b.items
                .iter()
                .any(|v| matches!(v, PresentationItemV2::Notice { .. })),
            "A's frontier must not hide B's notice"
        );
        assert_ne!(a.presentation_token, b.presentation_token);
        assert_eq!(
            f.confirm("b", &a),
            DeliveryPresentationConfirmationV2::NotCurrentPage
        );
        assert_eq!(
            f.pull("a").presentation_token,
            a.presentation_token,
            "B cannot reset A's recovery outbox"
        );
        assert_eq!(
            f.confirm("a", &a),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert_eq!(
            f.pull("b").presentation_token,
            b.presentation_token,
            "A's confirmation cannot retire B's descriptors"
        );
        let cursors: i64 =
            f.db.query_row("SELECT sum(seq) FROM binding_cursors", [], |r| r.get(0))
                .unwrap();
        assert_eq!(
            cursors, 0,
            "physical presentation confirmation moves no message read cursor"
        );
    }
    #[test]
    fn binding_input_cannot_select_a_sibling_or_canonical_subagent() {
        let f = Fixture::new();
        f.notice();
        let mut wrong = f.input("a");
        wrong.binding_id = BindingId::new("b");
        assert!(matches!(
            f.store.presentation_pull(wrong, &budget()).unwrap().reply,
            DeliveryPullReplyV2::Refused { .. }
        ));
        f.db.execute(
            "UPDATE occupant_bindings SET role='subagent',parent_binding='a' WHERE binding_id='b'",
            [],
        )
        .unwrap();
        assert!(matches!(
            f.store
                .presentation_pull(f.input("b"), &budget())
                .unwrap()
                .reply,
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM invitation_presentation_consumers",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn sibling_settled_receipt_remains_in_bound_normal_and_exact_recovery() {
        use herdr_threads::{
            delivery::ledger::{Offer, OfferedLedger},
            ports::{WatchReplayScope, WatchReplaySnapshot},
            protocol::ids::MessageId,
        };
        let f = Fixture::new();
        f.ordinary();
        let before: (String, i64, String) =
            f.db.query_row(
                "SELECT state,acked_at,ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let page = f.pull("b");
        assert!(page.items.iter().any(|v| matches!(v, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && !content.ack_required)), "B's bound normal stream must retain A-settled unread mail");
        let mut ledger = OfferedLedger::new();
        let entry = ledger
            .binding_mut(&SeatId::new("s"), &BindingId::new("b"), 1)
            .unwrap();
        let stamp = Offer {
            first_at: UtcMillis(10),
            last_at: UtcMillis(20),
            count: 4,
            truncated: false,
        };
        entry.offers.insert(MessageId::new("m"), stamp);
        let snapshot = entry.due_snapshot(PresentationConsumer::Watch);
        let mut input = f.input("b");
        input.known_messages = vec![MessageId::new("m")];
        input.blocked_messages = input.known_messages.clone();
        input.new_message_slots = 255;
        input.owner_replay = Some(WatchReplaySnapshot {
            scope: WatchReplayScope {
                seat: SeatId::new("s"),
                binding_id: BindingId::new("b"),
                generation: 1,
                native_session: "b".into(),
                execution: ExecutionId::new("b"),
                lease: "0123456789abcdef0123456789abcdef".into(),
                incarnation: snapshot.incarnation,
            },
            remaining: snapshot.stamps.clone(),
            active: true,
        });
        input.ordinary_due_snapshot = Some(snapshot);
        let recovered = f.store.presentation_pull(input, &budget()).unwrap();
        assert!(
            recovered.reclaimed_messages.is_empty(),
            "another binding's ACK is not B's physical completion"
        );
        assert_eq!(
            recovered.owner_replay.emitted.get(&MessageId::new("m")),
            Some(&stamp)
        );
        assert_eq!(
            f.db.query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id='b'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            f.db.query_row(
                "SELECT state,acked_at,ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?
                ))
            )
            .unwrap(),
            before
        );
    }
    #[test]
    fn carried_message_rechecks_own_unread_and_sibling_ack_without_touching_receipt() {
        let f = Fixture::new();
        f.ordinary();
        f.db.execute("UPDATE receipt_state SET state='pending',acked_at=NULL,ack_binding_id=NULL,ack_actor_seat_id=NULL,ack_generation=NULL,ack_observation=NULL WHERE message_id='m'", []).unwrap();
        let first = f.pull("b");
        let message = first
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message { id, lazy, content } => {
                    Some(herdr_threads::protocol::delivery::PresentationMessage {
                        id: id.clone(),
                        lazy: *lazy,
                        content: content.clone(),
                    })
                }
                _ => None,
            })
            .expect("ordinary output to retain as a carry witness");
        assert!(message.content.ack_required);
        let carry =
            serde_json::json!({"prefix":[],"items":[{"content":message,"marker_lazy":false}]})
                .to_string();
        f.db.execute("UPDATE receipt_state SET state='acked',acked_at=7,ack_binding_id='a',ack_actor_seat_id='s',ack_generation=1,ack_observation='original' WHERE message_id='m'", []).unwrap();
        let seed_carry = || {
            f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1,ordinary_done=1,rotation=0 WHERE binding_id='b' AND consumer='watch'", [&carry]).unwrap()
        };
        seed_carry();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            panic!("B unread carry must remain eligible")
        };
        assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && !content.ack_required)));
        assert!(output.work.ordinary_carry_key_probes > 0);
        assert!(output.work.ordinary_carry_key_probes <= 579);
        assert!(output.work.ordinary_carry_hydrated <= 32);
        f.db.execute("UPDATE binding_cursors SET seq=1 WHERE binding_id='b'", [])
            .unwrap();
        seed_carry();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        if let DeliveryPullReplyV2::Page(page) = output.reply {
            assert!(
                !page
                    .items
                    .iter()
                    .any(|item| matches!(item, PresentationItemV2::Message { .. })),
                "B's own completed cursor retires carry"
            );
        }
        assert_eq!(
            f.db.query_row(
                "SELECT ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
    }
    #[test]
    fn carry_quantum_accounts_all_thirty_two_keys_and_refuses_a_thirty_third() {
        let f = Fixture::new();
        f.ordinary();
        let first = f.pull("b");
        let retained = first
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message { id, lazy, content } => {
                    Some(herdr_threads::protocol::delivery::PresentationMessage {
                        id: id.clone(),
                        lazy: *lazy,
                        content: content.clone(),
                    })
                }
                _ => None,
            })
            .unwrap();
        let mut items = vec![serde_json::json!({"content":retained,"marker_lazy":false})];
        for n in 2..=32 {
            let id = format!("m{n}");
            let prep = format!("prep{n}");
            f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES(?1,'i','t',?2,'ordinary','unread by b',0,?3)", rusqlite::params![id,n,n+3]).unwrap();
            f.db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT ?1,instance_id,operation_scope,?1,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status FROM send_preparations WHERE id='prep'", [&prep]).unwrap();
            f.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES(?1,'t','s',1,300000,1)", [&prep]).unwrap();
            f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i',?1,?2,'t',?3,0,?4,0,1,0)", rusqlite::params![prep,id,n+3,n]).unwrap();
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES(?1,'s','acked',7,'a','s',1,'original')", [&id]).unwrap();
            let mut item = items[0].clone();
            item["content"]["id"] = serde_json::json!(id);
            items.push(item);
        }
        f.db.execute("UPDATE binding_cursors SET seq=32 WHERE binding_id='a'", [])
            .unwrap();
        let carry = serde_json::json!({"prefix":[],"items":items}).to_string();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1,ordinary_done=1,rotation=0 WHERE binding_id='b' AND consumer='watch'", [&carry]).unwrap();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            panic!("32 canonical unread carry rows")
        };
        assert_eq!(
            page.items
                .iter()
                .filter(|item| matches!(item, PresentationItemV2::Message { .. }))
                .count(),
            32
        );
        assert_eq!(
            output.work.ordinary, 0,
            "carry must not add a fresh ordinary walk"
        );
        assert_eq!(
            output.work.ordinary_due_key_probes, 0,
            "due/replay accounting is separate"
        );
        assert_eq!(output.work.ordinary_carry_hydrated, 32);
        assert!(output.work.ordinary_carry_seeks <= 32);
        assert!(
            output.work.ordinary_carry_key_probes > 32
                && output.work.ordinary_carry_key_probes <= 579
        );
        items.push(items[0].clone());
        let invalid = serde_json::json!({"prefix":[],"items":items}).to_string();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1 WHERE binding_id='b' AND consumer='watch'", [&invalid]).unwrap();
        assert!(
            f.store.presentation_pull(f.input("b"), &budget()).is_err(),
            "33rd carry item must refuse before unbounded work"
        );
        assert_eq!(f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), invalid, "refused pull rolls back its canonical state changes");
    }
    #[test]
    fn unread_partial_presentation_resumes_after_same_binding_full_body_read() {
        use herdr_threads::{
            ports::ReadContext,
            protocol::{
                commands::{AdvanceCursors, BodyReadRequest, Command, MessageQuery},
                ids::{MessageId, OperationId},
                pagination::{InboxBatchV2CursorState, InboxBatchV2Source},
                results::{CommandResult, MessageContent},
            },
        };
        let f = Fixture::new();
        f.ordinary();
        // The public post-display mutation checks canonical A2 mapping and a
        // canonical execution UUID, unlike the smaller presentation-only fixture.
        let execution = "11111111-1111-4111-8111-111111111111";
        f.db.execute(
            "UPDATE occupant_bindings SET execution_id=?1 WHERE binding_id='b'",
            [execution],
        )
        .unwrap();
        f.db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','p','boot',1,0,0,'fresh','term','inc','coherent_enumeration',1)", []).unwrap();
        let mut claim = f.claim("b");
        claim.execution = ExecutionId::new(execution);
        let input = || {
            let mut input = f.input("b");
            input.pull.claim = claim.clone();
            input.frozen_render = f
                .store
                .presentation_render_selection(&claim, PresentationConsumer::Watch, &budget())
                .unwrap()
                .map(|identity| FrozenPresentationRender {
                    identity,
                    codec: None,
                });
            input
        };
        let body = "z".repeat(65_536);
        f.db.execute("UPDATE messages SET body=?1 WHERE id='m'", [&body])
            .unwrap();
        // A real second published recipient follows the long unread message.
        f.db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('later','i','t',2,'ordinary','later mail',0,4);
            INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT 'later-prep',instance_id,operation_scope,'later-op',digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status FROM send_preparations WHERE id='prep';
            INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('later-prep','t','s',1,300000,1);
            INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','later-prep','later','t',4,0,2,0,1,0);
            INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES('later','s','acked',7,'a','s',1,'original');").unwrap();
        let first = f.store.presentation_pull(input(), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(first) = first.reply else {
            panic!("initial unread presentation must produce its truncated body")
        };
        assert!(first.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && content.truncated && content.body_len==65_536)));
        assert!(!first.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="later")
        ));
        let (cursor, skip, done): (String, Option<String>, i64) = f.db.query_row("SELECT ordinary_cursor,ordinary_skip_body,ordinary_done FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let persisted =
            InboxBatchV2CursorState::decode_for(&cursor, "i", &SeatId::new("s")).unwrap();
        assert_eq!(persisted.source, InboxBatchV2Source::Unread);
        assert_eq!(persisted.body.as_ref().unwrap().message.as_str(), "m");
        assert_eq!(skip.as_deref(), Some("m"));
        assert_eq!(done, 0, "the real source persisted an unfinished body walk");

        // Reproduce `body m` followed by its actual post-display cursor command.
        // No test SQL changes any cursor after the fixture was established.
        let read = ReadContext {
            instance: "i".into(),
            output: Default::default(),
            operation_scope: None,
        };
        let mut body_cursor = None;
        let mut full_body = String::new();
        let mut complete = false;
        for _ in 0..4 {
            let result = f
                .store
                .query(
                    &Command::Message(MessageQuery {
                        message: MessageId::new("m"),
                        body: BodyReadRequest {
                            cursor: body_cursor,
                            offset: None,
                            max_bytes: 65_536,
                        },
                    }),
                    &read,
                    &budget(),
                )
                .unwrap();
            let CommandResult::Message(message) = result else {
                panic!("body read result")
            };
            let MessageContent::Ordinary {
                body_data,
                body_complete,
                body_next_cursor,
                ..
            } = message.content
            else {
                panic!("ordinary full body")
            };
            full_body.push_str(&body_data);
            if body_complete {
                complete = true;
                break;
            }
            body_cursor = Some(body_next_cursor.expect("unfinished body has continuation"));
        }
        assert!(complete);
        assert_eq!(full_body, body);
        let advanced = f
            .store
            .advance_cursors(
                &AdvanceCursors {
                    claim: claim.clone(),
                    operation: OperationId::new("body-read-b"),
                    displayed: vec![MessageId::new("m")],
                },
                &budget(),
            )
            .unwrap();
        assert!(
            matches!(advanced, CommandResult::CursorsAdvanced(ids) if ids==vec![MessageId::new("m")])
        );
        let positions = || {
            f.db.query_row("SELECT (SELECT seq FROM binding_cursors WHERE binding_id='a' AND thread_id='t'),(SELECT seq FROM binding_cursors WHERE binding_id='b' AND thread_id='t')", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap()
        };
        assert_eq!(
            positions(),
            (1, 1),
            "only B catches up to A's existing cursor"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
        assert_eq!(f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), cursor, "full-body read must not secretly rewrite presentation progress");

        let resumed = f.store.presentation_pull(input(), &budget()).expect(
            "a legitimate same-binding full read must not wedge private presentation continuation",
        );
        let DeliveryPullReplyV2::Page(resumed) = resumed.reply else {
            panic!("presentation must reach later unread mail")
        };
        assert!(resumed.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="later")
        ));
        assert!(!resumed.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="m")
        ));
        assert_eq!(
            positions(),
            (1, 1),
            "presentation does not advance either binding cursor"
        );
    }
}
