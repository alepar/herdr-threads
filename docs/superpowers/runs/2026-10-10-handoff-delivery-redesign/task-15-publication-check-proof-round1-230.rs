#[test]
fn presentation_publication_scalar_checks_validate_retained_consumer_rows() -> rusqlite::Result<()>
{
    use std::cell::Cell;
    use std::ffi::{c_int, c_void};

    use rusqlite::{params_from_iter, types::Value};

    // This callback is connection-local and never calls back into SQLite.
    // Saturation avoids any possible arithmetic panic crossing the C ABI.
    unsafe extern "C" fn count_instruction(context: *mut c_void) -> c_int {
        // SAFETY: measured() passes a stable live Box<Cell<u64>> and removes
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
    fn measured(db: &Connection, sql: &str) -> rusqlite::Result<(i32, u64)> {
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
                let (outer, progress) = measured(&db, sql)?;
                // Matched uncomplicated ADD COLUMN: only CHECK is removed.
                // The control isolates fixed DDL from validation of retained rows.
                let (prefix, _) = sql.split_once(" CHECK(").expect("scalar CHECK");
                let (control_outer, control_progress) = measured(&control, &format!("{prefix};"))?;
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
            assert_eq!(control_work[0][column], control_work[1][column]);
            assert_eq!(control_work[1][column], control_work[2][column]);
            let first_delta = checked_work[1][column]
                .checked_sub(checked_work[0][column])
                .expect("positive validation slope");
            let second_delta = checked_work[2][column]
                .checked_sub(checked_work[1][column])
                .expect("positive validation slope");
            assert!(first_delta > 0 && second_delta > 0);
            // Each of four ALTERs validates S retained consumer rows, plus fixed
            // DDL/scan overhead. VM ticks are NOT asserted equal to 4*S: their
            // per-row slope includes all existing V40 constraints and new ones.
            assert_eq!(first_delta * 96, second_delta * 31);
        }
    }
    Ok(())
}
