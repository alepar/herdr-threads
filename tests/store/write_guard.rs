//! The write guard: armed, a replicated-table write outside an apply or a
//! `local_only` scope fails; inside either it passes; disarmed nothing is
//! checked.

use super::*;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::protocol::ids::ThreadId;
use crate::store::journal;
use crate::store::materialize::local_only;
use crate::store::thread_registers::{self, RegisterField};
use crate::sync::EntityKey;
use crate::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
use crate::test_support::journal_oracle::{CONVERTED_TABLES, OracleStore};
use crate::test_support::write_guard::REPLICATED_TABLES;

#[derive(Serialize, Deserialize)]
struct Reg {
    thread: String,
}

/// A test kind whose apply writes a replicated table (`thread_registers`).
struct RegKind;
impl FactKind for RegKind {
    const NAME: &'static str = "guard.reg";
    const MAJOR: u32 = 1;
    type Payload = Reg;
    fn creates(_: &Reg) -> Option<EntityKey> {
        None
    }
    fn needs(_: &Reg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Reg) -> Result<(), FactError> {
        thread_registers::upsert(
            tx,
            &ThreadId::parse(p.thread).expect("a thread id"),
            RegisterField::Topic,
            Some("t"),
            cx.hlc,
            cx.origin.node,
            cx.ingest_seq,
        )
        .map(|_| ())
        .map_err(|e| FactError::from_api(crate::store::connection::store_error(e)))
    }
}

fn store() -> OracleStore {
    OracleStore::open(
        [31; 32],
        FactRegistry::builtins_plus(|b| b.with::<RegKind>()),
    )
}

const RAW_INSERT: &str = "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
                          VALUES('t-x','i','topic','',1,1,1)";
/// A prepare-time check needs no matching rows.
const REGISTER_WRITE: &str = "DELETE FROM thread_registers WHERE 0";

fn denied(result: rusqlite::Result<usize>) -> bool {
    result.is_err_and(|e| e.to_string().contains("not authorized"))
}

// Kills: an authorizer that lets a raw write to a replicated table through, or
// one that denies tables outside the list.
#[test]
fn armed_raw_insert_into_threads_fails() {
    let s = store();
    let conn = s.context().open_writer().unwrap();
    let _guard = arm(&conn);
    assert!(denied(conn.execute(RAW_INSERT, [])));
    assert!(denied(conn.execute(REGISTER_WRITE, [])));
    conn.execute("UPDATE local_node SET decision_seq=decision_seq", [])
        .expect("an unlisted table is not checked");
}

// Kills: a scope marker `apply_entry` does not raise (the kind's own write
// would be denied), or a guard that denies the write wherever it happens.
#[test]
fn write_inside_test_kind_apply_passes() {
    let s = store();
    let ctx = s.context();
    let mut conn = ctx.open_writer().unwrap();
    let guard = arm(&conn);
    ctx.execute_decision(
        &mut conn,
        |_| Ok(()),
        |tx, d, ()| {
            journal::author::<RegKind>(
                tx,
                ctx,
                d,
                &Reg {
                    thread: "t1".into(),
                },
            )
        },
    )
    .expect("the apply writes inside its savepoint");
    drop(guard);
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM thread_registers WHERE thread_id='t1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "the kind's write landed");
    let _again = arm(&conn);
    assert!(
        denied(conn.execute(REGISTER_WRITE, [])),
        "and is checked outside"
    );
}

// Kills: a `local_only` input that does not raise the scope, or one whose scope
// outlives the input.
#[test]
fn write_inside_local_only_passes() {
    let s = store();
    let ctx = s.context();
    let mut conn = ctx.open_writer().unwrap();
    let _guard = arm(&conn);
    let late = ctx
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| {
                let m = local_only(tx, d)?;
                tx.execute(REGISTER_WRITE, [])
                    .map_err(crate::store::connection::store_error)?;
                drop(m);
                Ok(tx.execute(REGISTER_WRITE, []))
            },
        )
        .expect("a local_only scope writes");
    assert!(denied(late), "the write after the input dropped is denied");
}

// Kills: a guard that keeps checking after it is dropped, or a Guard that
// changes a connection nobody armed.
#[test]
fn disarmed_checks_nothing() {
    let s = store();
    let conn = s.context().open_writer().unwrap();
    conn.execute(REGISTER_WRITE, []).expect("never armed");
    drop(arm(&conn));
    conn.execute(REGISTER_WRITE, []).expect("disarmed on drop");
}

// Kills: a statement cached before arming (or inside a scope) that is reused
// past the prepare-time check.
#[test]
fn cached_statement_does_not_bypass() {
    let s = store();
    let conn = s.context().open_writer().unwrap();
    conn.prepare_cached(REGISTER_WRITE)
        .unwrap()
        .execute([])
        .unwrap();
    let _guard = arm(&conn);
    let again = conn.prepare_cached(REGISTER_WRITE);
    assert!(again.is_err_and(|e| e.to_string().contains("not authorized")));
}

// Kills: seat rules that deny every seat write, or none.
#[test]
fn seat_writes_are_denied_by_column() {
    let s = store();
    let conn = s.context().open_writer().unwrap();
    let mut guard = arm(&conn);
    conn.execute("UPDATE seats SET wake_on=wake_on WHERE 0", [])
        .expect("a non-state column is free");
    assert!(denied(
        conn.execute("UPDATE seats SET state=state WHERE 0", [])
    ));
    assert!(denied(
        conn.execute("UPDATE seats SET retired_at=NULL WHERE 0", [])
    ));
    conn.execute("DELETE FROM seats WHERE 0", [])
        .expect("deletes are not seat-gated");
    let insert = "INSERT INTO seats(id) SELECT NULL WHERE 0";
    // Without strict mode a seat INSERT is not told apart from a local seat's.
    assert!(!denied(conn.execute(insert, [])));
    guard.strict_seat_inserts();
    assert!(denied(conn.execute(insert, [])));
}

// Kills: `allow` that exempts nothing.
#[test]
fn allow_exempts_a_table() {
    let s = store();
    let conn = s.context().open_writer().unwrap();
    let mut guard = arm(&conn);
    guard.allow(&["thread_registers"]);
    conn.execute(REGISTER_WRITE, []).expect("exempted");
    assert!(denied(conn.execute("DELETE FROM threads WHERE 0", [])));
}

// Kills: a projection table the guard does not list, or the reverse drift the
// module doc warns about.
#[test]
fn projection_tables_are_guarded_tables() {
    for spec in CONVERTED_TABLES {
        assert!(
            REPLICATED_TABLES.contains(&spec.name),
            "{} is projected but not guarded",
            spec.name
        );
    }
}
