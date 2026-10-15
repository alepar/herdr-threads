//! Deadline lane scope (ht-5fr.4.13, fact-kinds design section 13): only the
//! node that authored a message or invitation records it overdue. Rows are
//! seeded by direct inserts carrying a foreign `origin_node`; the clock stands
//! past every `deadline_at`. A self-origin twin in each test still goes
//! overdue, so a scan that selects nothing at all cannot pass. The two-node
//! versions (recipient against sender, retirement job, late ACK/answer) are
//! ht-5fr.4.18's.

use herdr_threads::{
    ports::TimeBasis,
    protocol::{
        authority::ObligationRef,
        ids::{InvitationId, MessageId, SeatId},
        time::{Clock, MonoInstant, UtcMillis},
    },
    store::{
        connection::StoreContext,
        invitation_due::scan_invitation_due_batch,
        receipts::{self, ReceiptDueCursor},
        schema::record_overdue_if_pending,
    },
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";
const FOREIGN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NOW: i64 = 10_000;
const DEADLINE: i64 = 5_000;

struct Fixed;
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(NOW)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

fn setup() -> (StoreContext, Connection) {
    let path = herdr_threads::test_support::scratch::temp_path(format!(
        "deadline-scope-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(Fixed)).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('rcpt',?1,'resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", [NODE]).unwrap();
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('thread',?1,'topic','goal',0,0,0)", [NODE]).unwrap();
    (context, conn)
}

/// An ordinary message `id` authored by `origin` (`None`: local node).
fn message(conn: &Connection, id: &str, sequence: i64, origin: Option<&str>) {
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,origin_node) VALUES (?1,?2,'thread',?3,'ordinary','author','x',0,?3,COALESCE(?4,(SELECT id FROM local_node)))", params![id, NODE, sequence, origin]).unwrap();
    conn.execute(
        "UPDATE local_node SET decision_seq=MAX(decision_seq,?1)",
        [sequence],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=MAX(next_sequence,?1) WHERE id='thread'",
        [sequence + 1],
    )
    .unwrap();
}

/// A pending, ACK-required physical receipt of `id` for `rcpt`.
fn physical_receipt(conn: &Connection, id: &str) {
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'thread','rcpt','pending',500,0,?2)", params![id, DEADLINE]).unwrap();
}

/// A pending, ACK-required manifest-era receipt row of `id` for `rcpt`.
fn state_receipt(conn: &Connection, id: &str) {
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'rcpt','pending',0,?2)", params![id, DEADLINE]).unwrap();
}

fn invitation(conn: &Connection, id: &str, episode: i64, origin: &str) {
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'thread','rcpt',?2,'pending',?2,0,100,?3,?4)", params![id, episode, DEADLINE, origin]).unwrap();
}

fn warned(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

fn warned_for(conn: &Connection, message: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id=?1",
        [message],
        |r| r.get(0),
    )
    .unwrap()
}

fn sparse(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT message_id FROM receipt_due_sparse ORDER BY message_id")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn record(conn: &mut Connection, obligation: &ObligationRef) -> bool {
    let tx = conn.transaction().unwrap();
    let outcome =
        record_overdue_if_pending(&tx, obligation, &TimeBasis::Decision, UtcMillis(NOW)).unwrap();
    tx.commit().unwrap();
    outcome.inserted
}

fn receipt_of(message: &str) -> ObligationRef {
    ObligationRef::Receipt {
        message: MessageId::new(message),
        seat: SeatId::new("rcpt"),
    }
}

#[test]
fn record_overdue_refuses_remote_receipt() {
    let (_context, mut conn) = setup();
    message(&conn, "far", 1, Some(FOREIGN));
    message(&conn, "near", 2, None);
    physical_receipt(&conn, "far");
    physical_receipt(&conn, "near");
    let before: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert!(!record(&mut conn, &receipt_of("far")));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before,
        "nothing written for another node's receipt"
    );
    assert!(
        conn.query_row(
            "SELECT warning_message_id IS NULL FROM receipts WHERE message_id='far'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert!(record(&mut conn, &receipt_of("near")), "self-origin twin");
    assert_eq!(warned_for(&conn, "near"), 1);
}

#[test]
fn record_overdue_refuses_remote_invitation() {
    let (_context, mut conn) = setup();
    invitation(&conn, "inv-far", 1, FOREIGN);
    invitation(&conn, "inv-near", 2, NODE);
    let obligation = |id: &str| ObligationRef::Invitation(InvitationId::new(id));
    assert!(!record(&mut conn, &obligation("inv-far")));
    assert_eq!(warned(&conn), 0, "nothing written for a foreign invitation");
    assert!(
        record(&mut conn, &obligation("inv-near")),
        "self-origin twin"
    );
    assert_eq!(warned(&conn), 1);
}

#[test]
fn sparse_due_triggers_skip_foreign_rows() {
    let (_context, conn) = setup();
    message(&conn, "far", 1, Some(FOREIGN));
    message(&conn, "near", 2, None);
    message(&conn, "far-upd", 3, Some(FOREIGN));
    message(&conn, "near-upd", 4, None);
    // Insert trigger.
    state_receipt(&conn, "far");
    state_receipt(&conn, "near");
    // Update trigger: the row starts acked, then turns pending again.
    for id in ["far-upd", "near-upd"] {
        conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) VALUES (?1,'rcpt','acked',0,?2)", params![id, DEADLINE]).unwrap();
        conn.execute(
            "UPDATE receipt_state SET state='pending' WHERE message_id=?1",
            [id],
        )
        .unwrap();
    }
    assert_eq!(sparse(&conn), ["near", "near-upd"]);
}

#[test]
fn scan_due_selects_no_foreign_row_self_row_overdue() {
    let (context, mut conn) = setup();
    message(&conn, "far-phys", 1, Some(FOREIGN));
    message(&conn, "near-phys", 2, None);
    physical_receipt(&conn, "far-phys");
    physical_receipt(&conn, "near-phys");
    let mut cursor = ReceiptDueCursor::default();
    let due = receipts::scan_due(&context, &mut conn, 20, &mut cursor).unwrap();
    assert_eq!(due.warnings, 1, "only the self-origin row");
    assert_eq!(due.inspected, 1, "the foreign row is never selected");
    assert_eq!(warned(&conn), 1);
    assert_eq!(warned_for(&conn, "near-phys"), 1);
}

#[test]
fn scan_invitation_due_batch_selects_no_foreign_row() {
    let (context, mut conn) = setup();
    invitation(&conn, "inv-far", 1, FOREIGN);
    invitation(&conn, "inv-near", 2, NODE);
    let batch = scan_invitation_due_batch(&context, &mut conn, None, 10).unwrap();
    assert_eq!(
        (batch.inspected, batch.warnings_added),
        (1, 1),
        "the foreign invitation is not even inspected"
    );
    assert_eq!(warned(&conn), 1);
}

#[test]
fn scan_extension_lapses_selects_no_foreign_row() {
    let (context, mut conn) = setup();
    message(&conn, "far-phys", 1, Some(FOREIGN));
    message(&conn, "near-phys", 2, None);
    physical_receipt(&conn, "far-phys");
    physical_receipt(&conn, "near-phys");
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('rcpt',1,'p','b',0,'codex','s','e','cooperative_top_level',0,'tm','inc')", []).unwrap();
    conn.execute("INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,extension_until,state) SELECT binding_id,'rcpt','thread',0,0,?1,'active' FROM occupant_bindings WHERE seat_id='rcpt'", [DEADLINE + 1]).unwrap();
    let mut cursor = ReceiptDueCursor::default();
    let lapses = receipts::scan_extension_lapses(&context, &mut conn, &mut cursor).unwrap();
    assert_eq!(lapses.warnings, 1, "only the self-origin row");
    assert_eq!(warned(&conn), 1);
    assert_eq!(warned_for(&conn, "near-phys"), 1);
}
