//! Catch-up lifecycle, hold and release (ht-1ip.6). Included from
//! `src/store/catch_up.rs` as `lifecycle_tests`. Time is an injected value
//! passed as `now`; the production-path tests drive a settable clock.
use super::*;
use crate::ports::LogicalAttentionFrontier;
use crate::protocol::ids::{BindingId, ExecutionId};
use crate::{
    ports::{
        CooperativePermitRequest, DuePhaseProgress, DueScanRequest, DueScanState, ReadContext,
        RegisterAvailableRequest, StorePort,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, ObligationRef},
        commands::{CheckIn, CheckInMode},
        ids::{HostTargetId, NativeSessionId, OperationId},
        results::CommandResult,
        time::{CallBudget, Clock, MonoInstant},
    },
    store::{
        SqliteStore, StoreSettings,
        attention::{self, DigestRun},
        connection::StoreContext,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

const NOW: UtcMillis = UtcMillis(500);

// ---------------------------------------------------------------------------
// In-memory fixture for the lifecycle and the hold: seats s, p, q each with an
// open top-level binding, one thread `t` and one thread `u`.
// ---------------------------------------------------------------------------

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::store::schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    crate::test_support::attach_test_host_to_connection(&db);
    db.execute_batch(
        "UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=40;\
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-s',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts)),('p','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-p',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts)),('q','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-q',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
         INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'pane-s','host',1,'codex','n','exec-s','cooperative_top_level',0,'tm','inc'),('p',1,'pane-p','host',1,'codex','n','exec-p','cooperative_top_level',0,'tm','inc'),('q',1,'pane-q','host',1,'codex','n','exec-q','cooperative_top_level',0,'tm','inc');\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0),('u','34750f98bd59fcfc946da45aaabe933b','other','goal',0,0);",
    )
    .unwrap();
    db
}

fn seat(id: &str) -> SeatId {
    SeatId::new(id)
}
fn thread() -> ThreadId {
    ThreadId::new("t")
}
/// The seat's newest open binding.
fn binding_of(db: &Connection, seat: &str) -> BindingId {
    BindingId::parse(
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1",
            [seat],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap()
}

/// A second open top-level binding of `seat` (another session), returned.
fn add_binding(db: &Connection, seat: &str, session: &str) -> BindingId {
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,'pane-'||?1,'host',1,'codex',?2,'exec-'||?2,'cooperative_top_level',0,'tm','inc')",
        [seat, session],
    )
    .unwrap();
    binding_of(db, seat)
}

fn enter(db: &mut Connection, seat_id: &str, frontier: u64, now: UtcMillis) -> u64 {
    let binding = binding_of(db, seat_id);
    enter_as(db, &binding, seat_id, frontier, now)
}

fn enter_as(
    db: &mut Connection,
    binding: &BindingId,
    seat_id: &str,
    frontier: u64,
    now: UtcMillis,
) -> u64 {
    let tx = db.transaction().unwrap();
    let seat = seat(seat_id);
    let thread = thread();
    let frontier = enter_or_keep(
        &tx,
        &CatchUpEntry {
            binding,
            seat: &seat,
            thread: &thread,
            frontier_seq: frontier,
            now,
        },
        &SummarySettings::default(),
    )
    .unwrap();
    tx.commit().unwrap();
    frontier
}

#[derive(Debug, PartialEq, Eq)]
struct Row {
    state: String,
    frontier: i64,
    end_reason: Option<String>,
    ended_at: Option<i64>,
    release_seq: Option<i64>,
    last_progress_at: Option<i64>,
    extension_until: Option<i64>,
}

fn row(db: &Connection, seat: &str) -> Row {
    row_where(db, "seat_id", seat)
}

fn row_of(db: &Connection, binding: &BindingId) -> Row {
    row_where(db, "binding_id", binding.as_str())
}

fn row_where(db: &Connection, column: &str, key: &str) -> Row {
    db.query_row(
        &format!("SELECT state,frontier_seq,end_reason,ended_at,release_seq,last_progress_at,extension_until FROM catch_up WHERE {column}=?1 AND thread_id='t'"),
        [key],
        |r| {
            Ok(Row {
                state: r.get(0)?,
                frontier: r.get(1)?,
                end_reason: r.get(2)?,
                ended_at: r.get(3)?,
                release_seq: r.get(4)?,
                last_progress_at: r.get(5)?,
                extension_until: r.get(6)?,
            })
        },
    )
    .unwrap()
}

fn decision_seq(db: &Connection) -> i64 {
    db.query_row("SELECT decision_seq FROM local_node", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn enter_opens_row_at_frontier_and_keep_never_moves_it() {
    let mut db = db();
    assert_eq!(enter(&mut db, "s", 10, UtcMillis(100)), 10);
    let first = row(&db, "s");
    assert_eq!((first.state.as_str(), first.frontier), ("active", 10));
    assert_eq!(first.release_seq, None);
    // Kept: the second call reports and stores F = 10, not 20.
    assert_eq!(enter(&mut db, "s", 20, UtcMillis(200)), 10);
    assert_eq!(row(&db, "s"), first);
    assert_eq!(held_above(&db, &seat("s"), &thread()).unwrap(), Some(10));
}

#[test]
fn entry_requires_an_open_binding_of_the_seat() {
    let mut db = db();
    let before = decision_seq(&db);
    let ended = add_binding(&db, "s", "gone");
    db.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE binding_id=?1",
        [ended.as_str()],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let thread = thread();
    let s = seat("s");
    let others = binding_of(&tx, "p");
    let unknown = BindingId::parse("no-such-binding").unwrap();
    for binding in [&ended, &others, &unknown] {
        let error = enter_or_keep(
            &tx,
            &CatchUpEntry {
                binding,
                seat: &s,
                thread: &thread,
                frontier_seq: 4,
                now: NOW,
            },
            &SummarySettings::default(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::CallerUnverified);
        assert_eq!(
            error.detail,
            "catch-up entry requires an open binding of the seat"
        );
    }
    tx.commit().unwrap();
    assert_eq!(catch_up_rows(&db), 0);
    assert_eq!(decision_seq(&db), before);
}

fn catch_up_rows(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM catch_up", [], |r| r.get(0))
        .unwrap()
}

fn ready(db: &mut Connection, seat_id: &str, now: UtcMillis) -> bool {
    let binding = binding_of(db, seat_id);
    ready_as(db, &binding, now)
}

fn ready_as(db: &mut Connection, binding: &BindingId, now: UtcMillis) -> bool {
    let tx = db.transaction().unwrap();
    let ended = on_ready(&tx, binding, &thread(), now, &SummarySettings::default()).unwrap();
    tx.commit().unwrap();
    ended
}

#[test]
fn on_ready_ends_and_releases() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    let main = binding_of(&db, "s");
    let before = decision_seq(&db);
    // A Ready answer to another binding leaves the row active.
    let other = add_binding(&db, "s", "other");
    assert!(!ready_as(&mut db, &other, NOW));
    assert_eq!(row(&db, "s").state, "active");
    assert_eq!(decision_seq(&db), before);
    assert!(ready_as(&mut db, &main, NOW));
    let ended = row(&db, "s");
    assert_eq!(ended.state, "ended");
    assert_eq!(ended.end_reason.as_deref(), Some("ready"));
    assert_eq!(ended.ended_at, Some(NOW.0));
    assert_eq!(ended.release_seq, Some(before + 1));
    assert_eq!(decision_seq(&db), before + 1);
    // Idempotent: a second Ready ends nothing and allocates nothing.
    assert!(!ready_as(&mut db, &main, NOW));
    assert_eq!(row(&db, "s"), ended);
    assert_eq!(decision_seq(&db), before + 1);
    assert_eq!(held_above(&db, &seat("s"), &thread()).unwrap(), None);
}

#[test]
fn reentry_after_end_reopens() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    assert!(ready(&mut db, "s", UtcMillis(200)));
    let release = row(&db, "s").release_seq.unwrap();
    assert_eq!(enter(&mut db, "s", 30, UtcMillis(300)), 30);
    let reopened = row(&db, "s");
    assert_eq!(reopened.state, "active");
    assert_eq!(reopened.frontier, 30);
    assert_eq!(reopened.end_reason, None);
    assert_eq!(reopened.ended_at, None);
    assert_eq!(reopened.last_progress_at, None);
    // The old release key stays until the next end.
    assert_eq!(reopened.release_seq, Some(release));
    assert!(ready(&mut db, "s", UtcMillis(400)));
    assert!(row(&db, "s").release_seq.unwrap() > release);
}

#[test]
fn replacement_binding_opens_its_own_row_and_the_old_one_is_superseded() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    let old = binding_of(&db, "s");
    // The seat's binding is replaced; the old row is stale.
    db.execute_batch(
        "UPDATE seats SET generation=2 WHERE id='s';\
         UPDATE occupant_bindings SET ended_at=150 WHERE seat_id='s';",
    )
    .unwrap();
    let new = add_binding(&db, "s", "n2");
    let before = decision_seq(&db);
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_stale(&tx, &old, NOW).unwrap(), 1);
    tx.commit().unwrap();
    let superseded = row_of(&db, &old);
    assert_eq!(superseded.end_reason.as_deref(), Some("superseded"));
    // The superseded row was released (a decision key was allocated).
    assert_eq!(superseded.release_seq, Some(before + 1));
    assert_eq!(enter_as(&mut db, &new, "s", 25, NOW), 25);
    let opened = row_of(&db, &new);
    assert_eq!((opened.state.as_str(), opened.frontier), ("active", 25));
    assert_eq!(row_of(&db, &old), superseded, "the old row is untouched");
}

#[test]
fn on_progress_marks_every_active_row_on_the_thread() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    enter(&mut db, "p", 10, UtcMillis(100));
    enter(&mut db, "q", 10, UtcMillis(100));
    assert!(ready(&mut db, "q", UtcMillis(150)));
    // A row on another thread is never touched.
    db.execute(
        "INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state) SELECT binding_id,'s','u',1,0,'active' FROM occupant_bindings WHERE seat_id='s'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    on_progress(&tx, &thread(), UtcMillis(333), &SummarySettings::default()).unwrap();
    tx.commit().unwrap();
    assert_eq!(row(&db, "s").last_progress_at, Some(333));
    assert_eq!(row(&db, "p").last_progress_at, Some(333));
    assert_eq!(row(&db, "q").last_progress_at, None);
    let other: Option<i64> = db
        .query_row(
            "SELECT last_progress_at FROM catch_up WHERE thread_id='u'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other, None);
}

#[test]
fn lifecycle_writes_extension_through_the_hooks() {
    // Cold p99 (no duration samples) is 90_000 ms and the exit grace 60_000 ms.
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    assert_eq!(row(&db, "s").extension_until, Some(100 + 90_000));
    let tx = db.transaction().unwrap();
    on_progress(&tx, &thread(), UtcMillis(200), &SummarySettings::default()).unwrap();
    tx.commit().unwrap();
    assert_eq!(row(&db, "s").extension_until, Some(200 + 90_000));
    assert!(ready(&mut db, "s", UtcMillis(300)));
    assert_eq!(row(&db, "s").extension_until, Some(300 + 60_000));
}

fn progress(db: &mut Connection, now: UtcMillis) {
    let tx = db.transaction().unwrap();
    on_progress(&tx, &thread(), now, &SummarySettings::default()).unwrap();
    tx.commit().unwrap();
}

fn stall(db: &mut Connection, now: UtcMillis) -> u16 {
    let tx = db.transaction().unwrap();
    let ended = stall_scan(&tx, now, 100).unwrap();
    tx.commit().unwrap();
    ended
}

#[test]
fn first_entry_extends_by_p99() {
    // Cold p99 is 90_000 ms; the first entry for (s, t) grants it.
    let mut db = db();
    assert_eq!(catch_up_rows(&db), 0);
    enter(&mut db, "s", 10, UtcMillis(1_000));
    assert_eq!(row(&db, "s").extension_until, Some(1_000 + 90_000));
    // A keep-call on the active row grants nothing more.
    enter(&mut db, "s", 10, UtcMillis(50_000));
    assert_eq!(row(&db, "s").extension_until, Some(1_000 + 90_000));
}

#[test]
fn reentry_after_stall_extends_nothing_until_progress_is_stored() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    assert_eq!(stall(&mut db, UtcMillis(100_000)), 1);
    assert_eq!(row(&db, "s").end_reason.as_deref(), Some("stalled"));
    // Re-entry reopens the row but keeps the lapsed extension.
    enter(&mut db, "s", 20, UtcMillis(200_000));
    let reopened = row(&db, "s");
    assert_eq!(reopened.state, "active");
    assert_eq!(reopened.extension_until, Some(100 + 90_000));
    // A newly stored block extends again.
    progress(&mut db, UtcMillis(210_000));
    assert_eq!(row(&db, "s").extension_until, Some(210_000 + 90_000));
}

#[test]
fn reentry_after_ready_or_supersession_extends_again() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    assert!(ready(&mut db, "s", UtcMillis(200)));
    assert_eq!(row(&db, "s").extension_until, Some(200 + 60_000));
    // A later catch-up (say after the next compaction) earns a fresh p99.
    enter(&mut db, "s", 30, UtcMillis(300_000));
    assert_eq!(row(&db, "s").extension_until, Some(300_000 + 90_000));
    // A keep-call on the active row grants nothing more.
    enter(&mut db, "s", 30, UtcMillis(350_000));
    assert_eq!(row(&db, "s").extension_until, Some(300_000 + 90_000));
    // A successor binding (after /clear) supersedes the active row and its
    // entry is eligible too.
    let old = binding_of(&db, "s");
    db.execute_batch(
        "UPDATE seats SET generation=2 WHERE id='s';\
         UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='s';",
    )
    .unwrap();
    let new = add_binding(&db, "s", "n2");
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_stale(&tx, &old, UtcMillis(390_000)).unwrap(), 1);
    tx.commit().unwrap();
    enter_as(&mut db, &new, "s", 40, UtcMillis(400_000));
    let reopened = row_of(&db, &new);
    assert_eq!((reopened.state.as_str(), reopened.frontier), ("active", 40));
    assert_eq!(reopened.extension_until, Some(400_000 + 90_000));
}

/// A stored summary block on `t` at `created_at`.
fn store_block(db: &Connection, id: &str, created_at: i64) {
    db.execute(
        "INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','t','v1',0,?2,1,2,'h','n','s','m','p',?3)",
        rusqlite::params![id, created_at, created_at],
    )
    .unwrap();
}

#[test]
fn reentry_after_a_stall_and_a_stored_block_extends_again() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    // A block stored before the stall does not count as progress since it.
    store_block(&db, "b0", 50);
    assert_eq!(stall(&mut db, UtcMillis(100_000)), 1);
    enter(&mut db, "s", 20, UtcMillis(150_000));
    assert_eq!(row(&db, "s").extension_until, Some(100 + 90_000));
    assert_eq!(stall(&mut db, UtcMillis(160_000)), 1);
    // A block stored after the stall (by any seat) makes the next entry eligible.
    store_block(&db, "b1", 170_000);
    enter(&mut db, "s", 30, UtcMillis(200_000));
    assert_eq!(row(&db, "s").extension_until, Some(200_000 + 90_000));
}

#[test]
fn stall_scan_ends_lapsed_rows() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    enter(&mut db, "p", 10, UtcMillis(100));
    db.execute(
        "UPDATE catch_up SET extension_until=?1 WHERE seat_id='s'",
        [NOW.0 - 1],
    )
    .unwrap();
    db.execute(
        "UPDATE catch_up SET extension_until=?1 WHERE seat_id='p'",
        [NOW.0 + 1000],
    )
    .unwrap();
    let before = decision_seq(&db);
    let tx = db.transaction().unwrap();
    assert_eq!(stall_scan(&tx, NOW, 100).unwrap(), 1);
    tx.commit().unwrap();
    let stalled = row(&db, "s");
    assert_eq!(stalled.state, "ended");
    assert_eq!(stalled.end_reason.as_deref(), Some("stalled"));
    assert_eq!(stalled.ended_at, Some(NOW.0));
    assert_eq!(stalled.release_seq, Some(before + 1));
    assert_eq!(row(&db, "p").state, "active");
    // The ended row is not ended again by the next pass.
    let tx = db.transaction().unwrap();
    assert_eq!(stall_scan(&tx, NOW, 100).unwrap(), 0);
    tx.commit().unwrap();
    assert_eq!(row(&db, "s"), stalled);
}

#[test]
fn stall_scan_is_bounded_per_pass() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    enter(&mut db, "p", 10, UtcMillis(100));
    db.execute("UPDATE catch_up SET extension_until=1", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(stall_scan(&tx, NOW, 1).unwrap(), 1);
    assert_eq!(stall_scan(&tx, NOW, 1).unwrap(), 1);
    assert_eq!(stall_scan(&tx, NOW, 1).unwrap(), 0);
    tx.commit().unwrap();
}

// ---------------------------------------------------------------------------
// The hold in pushed attention.
// ---------------------------------------------------------------------------

/// Receipts for seat `s` on thread `t` at message sequences 5 (below the
/// frontier) and 12 (above it), plus the same shape on thread `u`.
fn with_receipts(db: &Connection) {
    db.execute_batch(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,author_role) VALUES ('m5','34750f98bd59fcfc946da45aaabe933b','t',5,'ordinary','b',0,5,'agent'),('m12','34750f98bd59fcfc946da45aaabe933b','t',12,'ordinary','b',0,12,'agent'),('u1','34750f98bd59fcfc946da45aaabe933b','u',1,'ordinary','b',0,14,'agent');\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m5','t','s','pending',100),('m12','t','s','pending',100),('u1','u','s','pending',100);",
    )
    .unwrap();
}

fn digest(db: &Connection, seat: &str) -> DigestRun {
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let run = attention::seat_digest(
        db,
        "34750f98bd59fcfc946da45aaabe933b",
        &SeatId::new(seat),
        &|| Ok(()),
    )
    .unwrap();
    db.execute_batch("COMMIT").unwrap();
    run
}

fn receipt_ids(run: &DigestRun) -> Vec<&str> {
    run.digest
        .receipts
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect()
}

/// The production wake source's frontier, checked against the pre-D2 oracle
/// scan (both apply the catch-up hold through the same helper).
fn wake_frontier(db: &Connection, seat: &str) -> LogicalAttentionFrontier {
    let wake = attention::wake_seat_attention(db, seat).unwrap();
    let mut position = None;
    let oracle = loop {
        let slice = crate::test_support::attention_oracle::scan_effective_seat_attention(
            db,
            seat,
            position.take(),
            100,
        )
        .unwrap();
        if let Some(attention) = slice.attention {
            break attention.frontier;
        }
        position = Some(slice.position);
    };
    assert_eq!(
        wake.attention.frontier.addressed_receipt,
        oracle.addressed_receipt
    );
    wake.attention.frontier
}

fn thread_receipts(db: &Connection, thread: &str) -> u64 {
    attention::pending_receipts(db, "s", Some(thread))
        .unwrap()
        .count()
        .0
}

fn receipt_key(frontier: &LogicalAttentionFrontier) -> Option<(i64, i64)> {
    frontier
        .addressed_receipt
        .map(|k| (k.decision_seq as i64, k.event_offset as i64))
}

#[test]
fn held_receipt_is_excluded_from_digest_inbox_and_wake() {
    let mut db = db();
    with_receipts(&db);
    // Before the row: everything pushes. The key of the newest is u1's (14).
    let open = digest(&db, "s");
    assert_eq!(receipt_ids(&open), ["u1", "m12", "m5"]);
    assert_eq!(thread_receipts(&db, "t"), 2);
    enter(&mut db, "s", 10, UtcMillis(100));
    let held = digest(&db, "s");
    // m12 (sequence 12 > F = 10) is held; m5 (<= F) and the other thread are not.
    assert_eq!(receipt_ids(&held), ["u1", "m5"]);
    assert_eq!(held.digest.receipts.count, 2);
    assert_eq!(thread_receipts(&db, "t"), 1, "inbox count for the thread");
    assert_eq!(thread_receipts(&db, "u"), 1);
    // The wake source applies the same rule: m12's decision key 12 does not
    // count; with the other thread's u1 the frontier is 14, so isolate t.
    db.execute("DELETE FROM receipts WHERE message_id='u1'", [])
        .unwrap();
    let wake = wake_frontier(&db, "s");
    assert_eq!(receipt_key(&wake), Some((5, 0)));
    assert_eq!(receipt_key(&digest(&db, "s").frontier), Some((5, 0)));
    // History and explicit reads are unaffected: the held message is a normal
    // message row and the receipt is still pending in the explicit query's
    // source table.
    let held_row: String = db
        .query_row(
            "SELECT state FROM receipts WHERE message_id='m12'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(held_row, "pending");
    let history: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE thread_id='t' AND sequence>10",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(history, 1);
}

#[test]
fn bypasses() {
    let mut db = db();
    with_receipts(&db);
    db.execute_batch(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,author_role) VALUES ('hum','34750f98bd59fcfc946da45aaabe933b','t',13,'ordinary','b',0,15,'human');\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,author_role,relays_user) VALUES ('rel','34750f98bd59fcfc946da45aaabe933b','t',14,'ordinary','b',0,16,'agent',1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,author_role) VALUES ('ord','34750f98bd59fcfc946da45aaabe933b','t',15,'ordinary','b',0,17,'agent');\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('hum','t','s','pending',100),('rel','t','s','pending',100),('ord','t','s','pending',100);\
         INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,3,100,100);\
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1);\
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('warn1','34750f98bd59fcfc946da45aaabe933b','t',16,'warn','{}',0,18,1);\
         INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('warn1',18,'t',100,'s','invitation','inv');",
    )
    .unwrap();
    enter(&mut db, "s", 10, UtcMillis(100));
    let run = digest(&db, "s");
    // Held: m12 and ord (ordinary, above F). Bypassing: priority by a human
    // author, priority by relays_user, below-frontier m5, and the other thread.
    let mut ids = receipt_ids(&run);
    ids.sort();
    assert_eq!(ids, ["hum", "m5", "rel", "u1"]);
    assert_eq!(run.digest.receipts.count, 4, "m12 and ord are held");
    assert_eq!(
        thread_receipts(&db, "t"),
        3,
        "hum, rel and m5 on the thread"
    );
    // An invitation and a warning on the same thread are counted.
    assert_eq!(run.digest.invitations.count, 1);
    assert_eq!(
        run.digest
            .warnings
            .items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["warn1"]
    );
    // The wake scan agrees with the digest on every bypass: the newest
    // pushed receipt is the relayed message (decision 16), not held `ord` (17).
    assert_eq!(receipt_key(&wake_frontier(&db, "s")), Some((16, 0)));
    assert_eq!(receipt_key(&run.frontier), Some((16, 0)));
    // The priority messages keep their own keys after a release as well.
    assert!(ready(&mut db, "s", NOW));
    let after = digest(&db, "s");
    assert_eq!(after.digest.receipts.count, 6);
    assert_eq!(thread_receipts(&db, "t"), 5);
    // The released ones are the newest by key; hum (15) and rel (16) keep
    // their own keys, so they rank below the released key.
    let mut ids = receipt_ids(&after);
    ids.sort();
    assert_eq!(ids.len(), 4);
    assert!(ids.contains(&"m12") && ids.contains(&"ord"));
}

/// Shared release assertions: ending the row by `end` in one transaction
/// re-publishes the held message above the seat's mark and re-derives the
/// wake reasons in that same transaction.
fn assert_release_pushes(end: impl FnOnce(&Transaction<'_>), expected_reason: &str) {
    let mut db = db();
    with_receipts(&db);
    db.execute("DELETE FROM receipts WHERE message_id='u1'", [])
        .unwrap();
    enter(&mut db, "s", 10, UtcMillis(100));
    let old = digest(&db, "s");
    assert_eq!(receipt_ids(&old), ["m5"]);
    let wake_before: Option<(i64, i64)> = db
        .query_row(
            "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id='s'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let inbox_revision = |db: &Connection| -> i64 {
        db.query_row(
            "SELECT COALESCE((SELECT revision FROM filter_revisions WHERE scope_kind='inbox' AND scope_key='s'),0)",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let revision_before = inbox_revision(&db);
    let tx = db.transaction().unwrap();
    end(&tx);
    // Same transaction, before commit: the wake reasons were re-derived.
    let (bits, version): (i64, i64) = tx
        .query_row(
            "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id='s'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(bits & 1, 1, "attention reason bit set at the row end");
    assert!(version > wake_before.map_or(0, |(_, v)| v));
    let release: i64 = tx
        .query_row(
            "SELECT release_seq FROM catch_up WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    tx.commit().unwrap();
    assert!(inbox_revision(&db) > revision_before);
    assert_eq!(row(&db, "s").end_reason.as_deref(), Some(expected_reason));
    // The held item is offered again, with a fresh key above the old mark.
    let new = digest(&db, "s");
    assert_eq!(receipt_ids(&new), ["m12", "m5"]);
    assert_eq!(new.digest.token.receipt, Some((release as u64, 0)));
    assert!(release as u64 > old.digest.token.receipt.unwrap().0);
    assert!(new.frontier.advanced_beyond(&old.frontier));
    // The dispatcher's frontier sees the same key.
    assert_eq!(receipt_key(&wake_frontier(&db, "s")), Some((release, 0)));
    // A receipt published after the release keeps its own, later key.
    db.execute_batch(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,author_role) VALUES ('late','34750f98bd59fcfc946da45aaabe933b','t',20,'ordinary','b',0,900,'agent');\
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('late','t','s','pending',100);",
    )
    .unwrap();
    assert_eq!(
        digest(&db, "s").digest.token.receipt,
        Some((900, 0)),
        "a post-release message is not re-keyed"
    );
}

#[test]
fn release_pushes_held_items_above_the_mark_on_ready() {
    assert_release_pushes(
        |tx| {
            assert!(
                on_ready(
                    tx,
                    &binding_of(tx, "s"),
                    &thread(),
                    NOW,
                    &SummarySettings::default()
                )
                .unwrap()
            );
        },
        "ready",
    );
}

#[test]
fn release_pushes_held_items_above_the_mark_on_stall() {
    assert_release_pushes(
        |tx| {
            tx.execute("UPDATE catch_up SET extension_until=?1", [NOW.0 - 1])
                .unwrap();
            assert_eq!(stall_scan(tx, NOW, 100).unwrap(), 1);
        },
        "stalled",
    );
}

#[test]
fn release_pushes_held_items_above_the_mark_on_supersession() {
    assert_release_pushes(
        |tx| {
            let old = binding_of(tx, "s");
            tx.execute(
                "UPDATE occupant_bindings SET ended_at=150 WHERE seat_id='s'",
                [],
            )
            .unwrap();
            add_binding(tx, "s", "n2");
            assert_eq!(supersede_stale(tx, &old, NOW).unwrap(), 1);
        },
        "superseded",
    );
}

#[test]
fn supersede_stale_leaves_current_rows_and_other_seats_alone() {
    let mut db = db();
    enter(&mut db, "s", 10, UtcMillis(100));
    enter(&mut db, "p", 10, UtcMillis(100));
    let (bs, bp) = (binding_of(&db, "s"), binding_of(&db, "p"));
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_stale(&tx, &bs, NOW).unwrap(), 0);
    assert_eq!(supersede_ended_bindings(&tx, &seat("s"), NOW).unwrap(), 0);
    tx.commit().unwrap();
    assert_eq!(row(&db, "s").state, "active");
    // Ending only p's binding ends only p's row.
    db.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='p'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_stale(&tx, &bs, NOW).unwrap(), 0);
    assert_eq!(supersede_stale(&tx, &bp, NOW).unwrap(), 1);
    tx.commit().unwrap();
    assert_eq!(row(&db, "p").end_reason.as_deref(), Some("superseded"));
    assert_eq!(row(&db, "s").state, "active");
}

#[test]
fn supersede_stale_only_touches_its_binding() {
    let mut db = db();
    let a = binding_of(&db, "s");
    let b = add_binding(&db, "s", "second");
    enter_as(&mut db, &a, "s", 10, UtcMillis(100));
    enter_as(&mut db, &b, "s", 12, UtcMillis(100));
    // Both bindings end; superseding A's rows leaves B's alone.
    db.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_stale(&tx, &a, NOW).unwrap(), 1);
    tx.commit().unwrap();
    assert_eq!(row_of(&db, &a).end_reason.as_deref(), Some("superseded"));
    assert_eq!(row_of(&db, &b).state, "active");
    // The seat-level call picks up what is left.
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_ended_bindings(&tx, &seat("s"), NOW).unwrap(), 1);
    tx.commit().unwrap();
    assert_eq!(row_of(&db, &b).end_reason.as_deref(), Some("superseded"));
}

#[test]
fn catch_up_survives_another_bindings_check_in() {
    let mut db = db();
    let a = binding_of(&db, "s");
    enter_as(&mut db, &a, "s", 10, UtcMillis(100));
    // Binding B checks in on the same seat; the transaction that ends replaced bindings runs the
    // seat-level supersede, and A (still open) keeps its row.
    let b = add_binding(&db, "s", "second");
    let tx = db.transaction().unwrap();
    assert_eq!(supersede_ended_bindings(&tx, &seat("s"), NOW).unwrap(), 0);
    assert_eq!(supersede_stale(&tx, &b, NOW).unwrap(), 0);
    tx.commit().unwrap();
    assert_eq!(row_of(&db, &a).state, "active");
    assert_eq!(held_above(&db, &seat("s"), &thread()).unwrap(), Some(10));
    // B enters its own row at its own frontier; ending B's row leaves A held.
    enter_as(&mut db, &b, "s", 20, UtcMillis(200));
    assert_eq!(
        held_above(&db, &seat("s"), &thread()).unwrap(),
        Some(10),
        "the lowest active frontier holds"
    );
    assert!(ready_as(&mut db, &b, NOW));
    assert_eq!(row_of(&db, &a).state, "active");
    assert_eq!(row_of(&db, &b).state, "ended");
    assert_eq!(held_above(&db, &seat("s"), &thread()).unwrap(), Some(10));
}

// ---------------------------------------------------------------------------
// Production paths: binding replacement and retirement through the real
// writers, and the stall scan inside the receipts due phase.
// ---------------------------------------------------------------------------

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Store {
    store: SqliteStore,
    conn: Connection,
    _directory: PrivateDirectory,
}

fn store() -> Store {
    let directory = PrivateDirectory(crate::test_support::scratch::temp_path(format!(
        "catch-up-store-{}",
        uuid::Uuid::new_v4()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(100)));
    let context = StoreContext::new(directory.0.join("store.db"), clock).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','p',0,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'p','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)", []).unwrap();
    Store {
        store: SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
        conn,
        _directory: directory,
    }
}

fn claim(execution: &str, generation: u64) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        seat: seat("s"),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("session-n"),
        execution: ExecutionId::new(execution),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

/// A lifecycle check-in by `execution`: the production binding writer.
fn check_in(store: &SqliteStore, execution: &str, expected_generation: u64) -> u64 {
    let command = CheckIn {
        lineage_parent: None,
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: expected_generation,
        },
        claim: claim(execution, expected_generation),
        operation: OperationId::new(format!("op-{execution}")),
    };
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: command.claim.clone(),
                operation: command.operation.clone(),
                obligation: ObligationRef::CheckIn(command.claim.seat.clone()),
                payload_hash: crate::store::schema::canonical_digest(&(
                    "check_in",
                    &command.mode,
                    &command.claim,
                ))
                .unwrap(),
                check_in_mode: Some(command.mode),
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::CheckedIn(result) = store
        .register_available(
            RegisterAvailableRequest {
                command,
                read: ReadContext {
                    instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                    output: Default::default(),
                    operation_scope: None,
                },
                operator: None,
            },
            permit,
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    result.context.binding_generation
}

fn enter_in_store(conn: &mut Connection) {
    let binding = binding_of(conn, "s");
    let tx = conn.transaction().unwrap();
    let (s, t) = (seat("s"), thread());
    enter_or_keep(
        &tx,
        &CatchUpEntry {
            binding: &binding,
            seat: &s,
            thread: &t,
            frontier_seq: 7,
            now: UtcMillis(150),
        },
        &SummarySettings::default(),
    )
    .unwrap();
    tx.commit().unwrap();
}

const EXEC_1: &str = "00000000-0000-4000-8000-000000000001";
const EXEC_2: &str = "00000000-0000-4000-8000-000000000002";

#[test]
fn rechecking_in_on_a_new_execution_keeps_the_bindings_catch_up() {
    let mut f = store();
    let generation = check_in(&f.store, EXEC_1, 0);
    // ht-54r.3: a check-in keeps the seat's mapping generation; it never bumps it.
    assert_eq!(generation, 0);
    enter_in_store(&mut f.conn);
    assert_eq!(row(&f.conn, "s").state, "active");
    let binding = binding_of(&f.conn, "s");
    // A fresh lifecycle check-in with a new execution refreshes the same session binding: the
    // row follows the binding_id, so nothing is superseded.
    assert_eq!(check_in(&f.store, EXEC_2, 0), 0);
    assert_eq!(binding_of(&f.conn, "s"), binding);
    let kept = row(&f.conn, "s");
    assert_eq!(kept.state, "active");
    assert_eq!(kept.release_seq, None, "nothing was released");
}

#[test]
fn retirement_supersedes() {
    let mut f = store();
    check_in(&f.store, EXEC_1, 0);
    enter_in_store(&mut f.conn);
    let tx = f.conn.transaction().unwrap();
    crate::store::control::begin_retirement_fence(
        &tx,
        UtcMillis(200),
        seat("s"),
        "34750f98bd59fcfc946da45aaabe933b",
        "b",
        1,
        "p",
        1,
    )
    .unwrap();
    tx.commit().unwrap();
    let ended = row(&f.conn, "s");
    assert_eq!(ended.state, "ended");
    assert_eq!(ended.end_reason.as_deref(), Some("superseded"));
    assert_eq!(ended.ended_at, Some(200));
    assert!(ended.release_seq.is_some());
}

#[test]
fn due_phase_runs_the_stall_scan() {
    let mut f = store();
    check_in(&f.store, EXEC_1, 0);
    enter_in_store(&mut f.conn);
    f.conn
        .execute("UPDATE catch_up SET extension_until=50", [])
        .unwrap();
    let progress = StorePort::due_obligations(
        &f.store,
        DueScanRequest {
            state: DueScanState::default(),
            max_candidates: 5,
            run_invitations: false,
            run_receipts: true,
        },
        &budget(),
    )
    .unwrap();
    assert!(matches!(progress.receipts, DuePhaseProgress::Complete));
    assert_eq!(
        progress.examined_candidates, 0,
        "session maintenance is not a receipt candidate"
    );
    assert_eq!(
        progress.maintenance_sessions, 1,
        "the bounded ended session is counted separately"
    );
    let stalled = row(&f.conn, "s");
    assert_eq!(stalled.state, "ended");
    assert_eq!(stalled.end_reason.as_deref(), Some("stalled"));
    assert!(stalled.release_seq.is_some());
    // A pass with nothing lapsed examines nothing.
    let again = StorePort::due_obligations(
        &f.store,
        DueScanRequest {
            state: DueScanState::default(),
            max_candidates: 5,
            run_invitations: false,
            run_receipts: true,
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(again.examined_candidates, 0);
}

#[test]
fn due_phase_continuation_clears_with_many_ended_lapsed_rows() {
    let f = store();
    check_in(&f.store, EXEC_1, 0);
    let binding = binding_of(&f.conn, "s");
    for n in 0..250 {
        let thread = format!("x{n:03}");
        f.conn
            .execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)", [&thread])
            .unwrap();
        f.conn
            .execute("INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,extension_until,state,end_reason,ended_at) VALUES (?3,'s',?1,0,0,?2,'ended','ready',10)", rusqlite::params![thread, 1 + n, binding.as_str()])
            .unwrap();
    }
    let mut state = DueScanState::default();
    let call = |state: DueScanState| {
        StorePort::due_obligations(
            &f.store,
            DueScanRequest {
                state,
                max_candidates: 100,
                run_receipts: true,
                run_invitations: false,
            },
            &budget(),
        )
        .unwrap()
    };
    let mut settled = false;
    for _ in 0..4 {
        let progress = call(state);
        state = progress.state.clone();
        if matches!(progress.receipts, DuePhaseProgress::Complete) && !progress.has_more {
            settled = true;
            break;
        }
    }
    assert!(settled, "continuation never cleared in 4 calls");
    for _ in 0..3 {
        let progress = call(state);
        state = progress.state.clone();
        assert!(matches!(progress.receipts, DuePhaseProgress::Complete));
        assert!(!progress.has_more);
        assert!(
            progress.state.receipts.is_none(),
            "completed ordinary cycles need no redundant lapse watermark"
        );
        assert_eq!(
            progress.maintenance_sessions, 0,
            "already ended sessions are not revisited as active maintenance"
        );
    }
}
