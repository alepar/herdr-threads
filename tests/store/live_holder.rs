//! The live-holder guard (SA16, D19): a named seat refuses a second, unrelated, live top-level
//! holder with M7. Included from `src/store/seats/cooperative.rs` as `live_holder_tests`.
use super::*;
use crate::{
    ports::{ProcessInfo, ProcessRecord},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint},
        ids::{ExecutionId, NativeSessionId},
        time::{Clock, MonoInstant},
    },
    store::{
        SqliteStore, StoreSettings,
        cooperative_checkin_tests::{check_in, claim, lifecycle},
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// A mutable process table: tests start, exit and reuse pids.
#[derive(Default)]
struct Table(Mutex<HashMap<u32, ProcessRecord>>);
impl Table {
    fn set(&self, pid: u32, parent: u32, start: u64) {
        self.0.lock().unwrap().insert(
            pid,
            ProcessRecord {
                pid,
                parent: Some(parent),
                start,
                argv: Vec::new(),
            },
        );
    }
    fn exit(&self, pid: u32) {
        self.0.lock().unwrap().remove(&pid);
    }
}
impl ProcessInfo for Table {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0.lock().unwrap().get(&pid).cloned()
    }
}

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Fixture {
    store: Arc<SqliteStore>,
    conn: Connection,
    table: Arc<Table>,
    _dir: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

/// The cooperative fixture's seat `s`, renamed `reviewer` (a named seat), over a fake process table.
fn fixture() -> Fixture {
    let dir =
        crate::test_support::scratch::temp_path(format!("live-holder-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let table = Arc::new(Table::default());
    let clock = Arc::new(FixedClock);
    let context = StoreContext::new(dir.join("store.db"), clock)
        .with_test_host()
        .with_processes(table.clone());
    let conn = context.open_writer().unwrap();
    conn.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,name) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','p',0,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'reviewer')", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'p','inc','coherent_enumeration',1)", []).unwrap();
    let store = Arc::new(
        SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    Fixture {
        store,
        conn,
        table,
        _dir: dir,
    }
}

/// A Codex top-level claim for session `n`, whose harness process is `pid` (start stamp 100).
fn process_claim(n: u32, pid: u32) -> CallerClaim {
    let mut claim = claim();
    claim.native_session = NativeSessionId::new(format!("session-{n}"));
    claim.execution = ExecutionId::new(format!("00000000-0000-4000-8000-{n:012}"));
    claim.liveness = Some(LivenessFingerprint { pid, start: 100 });
    claim
}

fn enroll(f: &Fixture, n: u32, pid: u32) -> Result<(), crate::protocol::results::ApiError> {
    enroll_on(&f.store, n, pid)
}

fn enroll_on(
    store: &SqliteStore,
    n: u32,
    pid: u32,
) -> Result<(), crate::protocol::results::ApiError> {
    check_in(store, lifecycle(process_claim(n, pid), &format!("op-{n}"))).map(|_| ())
}

fn open_bindings(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

// Kills: no guard (the second enroll succeeds), a guard that ignores the name, and an M7 detail
// that drifts from the golden bytes.
#[test]
fn unrelated_live_holder_refused_m7() {
    let f = fixture();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    enroll(&f, 1, 20).unwrap();
    let error = enroll(&f, 2, 40).unwrap_err();
    assert_eq!(error.code, ErrorCode::SeatNameInUse);
    assert_eq!(
        format!("herdr-threads: {}\n", error.detail),
        include_str!("../cli/golden/m7.txt")
    );
    assert_eq!(
        "herdr-threads: seat  is held by live pid ; use another name".len(),
        59,
        "the fixed bytes of M7"
    );
    assert_eq!(
        open_bindings(&f.conn),
        1,
        "the refused enroll left no binding"
    );
}

// Kills: a guard on unnamed seats (the same two processes bind one pane-less seat freely).
#[test]
fn an_unnamed_seat_has_no_guard() {
    let f = fixture();
    f.conn
        .execute("UPDATE seats SET name=NULL WHERE id='s'", [])
        .unwrap();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    enroll(&f, 1, 20).unwrap();
    enroll(&f, 2, 40).unwrap();
    assert_eq!(open_bindings(&f.conn), 2);
}

// Kills: a holder judged by the stored row alone: an exited or pid-reused holder must free the
// name at once.
#[test]
fn holder_exit_or_pid_reuse_frees_name() {
    let f = fixture();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    enroll(&f, 1, 20).unwrap();
    assert!(enroll(&f, 2, 40).is_err());
    f.table.exit(20);
    enroll(&f, 2, 40).unwrap();
    assert_eq!(
        open_bindings(&f.conn),
        2,
        "the name binds beside the dead row"
    );

    let g = fixture();
    g.table.set(20, 1, 100);
    g.table.set(40, 1, 100);
    enroll(&g, 1, 20).unwrap();
    g.table.set(20, 1, 999); // pid 20 now a different process
    enroll(&g, 2, 40).unwrap();
}

// Kills: refusing the holder's own process (a new session id in the same harness process, as
// /clear), its descendants and its ancestors.
#[test]
fn same_process_ancestors_and_descendants_are_admitted() {
    let f = fixture();
    f.table.set(10, 1, 100);
    f.table.set(20, 10, 100);
    f.table.set(30, 20, 100);
    enroll(&f, 1, 20).unwrap();
    enroll(&f, 2, 20).unwrap(); // same process, new session
    enroll(&f, 3, 30).unwrap(); // descendant of the holder
    enroll(&f, 4, 10).unwrap(); // ancestor of the holder
    assert_eq!(open_bindings(&f.conn), 4);
}

// Kills: refusing a subagent or a readonly holder's neighbour, and counting a readonly row as the
// holder.
#[test]
fn descendant_subagent_readonly_admitted() {
    let f = fixture();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    f.table.set(50, 1, 100);
    enroll(&f, 1, 20).unwrap();

    let tx_check = |claim: &CallerClaim| {
        let tx = f.conn.unchecked_transaction().unwrap();
        guard_live_holder(&tx, f.table.as_ref(), claim, false)
    };
    // The probe discriminates: an unrelated top-level claim is refused.
    assert_eq!(
        tx_check(&process_claim(2, 40)).unwrap_err().code,
        ErrorCode::SeatNameInUse
    );
    // A subagent of an unrelated process is admitted.
    let mut subagent = process_claim(3, 40);
    subagent.role = CallerRole::Subagent;
    subagent.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
    tx_check(&subagent).unwrap();
    // A Codex fork (independent session) is admitted.
    let tx = f.conn.unchecked_transaction().unwrap();
    guard_live_holder(&tx, f.table.as_ref(), &process_claim(2, 40), true).unwrap();
    drop(tx);
    // A readonly holder does not hold the name.
    f.conn
        .execute("UPDATE occupant_bindings SET readonly=1", [])
        .unwrap();
    enroll(&f, 2, 40).unwrap();
    // A person's claim is never guarded.
    let mut human = process_claim(5, 50);
    human.harness = Harness::Human;
    human.liveness = None;
    tx_check(&human).unwrap();
}

// Kills: a touch of the holder's own connected binding judged as a new holder (the sole holder
// refusing itself when another unrelated binding appeared later).
#[test]
fn a_connected_binding_keeps_serving_its_own_calls() {
    let f = fixture();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    enroll(&f, 1, 20).unwrap();
    f.conn
        .execute("UPDATE seats SET name=NULL WHERE id='s'", [])
        .unwrap();
    enroll(&f, 2, 40).unwrap(); // two bindings while the seat was unnamed
    f.conn
        .execute("UPDATE seats SET name='reviewer' WHERE id='s'", [])
        .unwrap();
    // Each already-connected holder still resolves its own calls.
    let tx = f.conn.unchecked_transaction().unwrap();
    guard_live_holder(&tx, f.table.as_ref(), &process_claim(1, 20), false).unwrap();
    guard_live_holder(&tx, f.table.as_ref(), &process_claim(2, 40), false).unwrap();
}

// SA7: two concurrent unrelated enrolls of one name yield exactly one binding.
#[test]
fn sa7_concurrent_unrelated_enrolls_one_binding() {
    let f = fixture();
    f.table.set(20, 1, 100);
    f.table.set(40, 1, 100);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [(1, 20), (2, 40)]
        .into_iter()
        .map(|(n, pid)| {
            let store = f.store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                enroll_on(&store, n, pid)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    let refused = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert_eq!(refused.code, ErrorCode::SeatNameInUse);
    assert_eq!(open_bindings(&f.conn), 1);
}
