use super::*;
use crate::{
    protocol::ids::{MessageId, SeatId},
    store::connection::StoreContext,
    test_support::history,
};
use std::{path::PathBuf, sync::Arc};

const WINDOW_MS: u64 = 100_000;

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

/// Production-shaped receipts: sends and the send worker's `receipt_state`
/// projection go through the real writers. `snd` sends, `rcv` receives.
fn fixture() -> (StoreContext, Connection, RemoveOnDrop) {
    let path = std::env::temp_dir().join(format!("ht-poke-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(crate::app::SystemClock::new()));
    let guard = RemoveOnDrop(path);
    let conn = context.open_writer().unwrap();
    conn.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,unavailability_open) VALUES ('snd','i','resolved','native','p-snd',1,1,0,1,0),('rcv','i','resolved','native','p-rcv',1,1,0,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at) VALUES ('i','p-rcv','b',1,1,1,'fresh','unknown','unknown',0,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('rcv',1,1,'p-rcv','b',1,'codex','hist-rcv','hist-rcv','fresh',0,0,'term-rcv','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('ta','i','a','g',0,0),('tb','i','b','g',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('ta','snd','joined'),('ta','rcv','joined'),('tb','snd','joined'),('tb','rcv','joined');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('ta','snd',1,1),('ta','rcv',1,1),('tb','snd',1,1),('tb','rcv',1,1);\
    ").unwrap();
    (context, conn, guard)
}

fn send(context: &StoreContext, conn: &mut Connection, thread: &str, count: u64, tag: &str) {
    history::write_pending_sends_with_deadline(context, conn, thread, "snd", count, WINDOW_MS, tag)
        .unwrap();
}

/// `(deadline_at, available_at)` of the earliest receipt_state row.
fn first_window(conn: &Connection) -> (i64, i64) {
    conn.query_row(
        "SELECT deadline_at, available_at FROM receipt_state WHERE seat_id='rcv' ORDER BY deadline_at, message_id LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn settings() -> SummarySettings {
    SummarySettings::default()
}

fn due(conn: &Connection, now: i64) -> Vec<PokeDue> {
    due_pokes(conn, now, &settings(), 16).unwrap()
}

#[test]
fn candidate_after_soft_point_only() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, available) = first_window(&conn);
    assert_eq!(deadline - available, WINDOW_MS as i64);
    // soft_fraction 0.6: the soft point is 40s before the deadline.
    let soft = deadline - 40_000;
    assert!(due(&conn, soft - 1).is_empty());
    let found = due(&conn, soft);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].seat, SeatId::new("rcv"));
    assert_eq!(found[0].receipts.len(), 1);
    let receipt = &found[0].receipts[0];
    assert_eq!(receipt.thread.as_str(), "ta");
    assert_eq!(receipt.source, PokeSource::ReceiptState);
    assert_eq!(receipt.effective_deadline, deadline);
    // The sender holds no receipt for its own message.
    assert!(found.iter().all(|d| d.seat.as_str() != "snd"));
}

#[test]
fn past_effective_deadline_is_not_a_poke() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, _) = first_window(&conn);
    assert_eq!(due(&conn, deadline - 1).len(), 1);
    assert!(due(&conn, deadline).is_empty());
    assert!(due(&conn, deadline + 10_000).is_empty());
}

#[test]
fn effective_deadline_moves_the_soft_point() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, _) = first_window(&conn);
    let moved = deadline + 100_000;
    let with =
        |now: i64| due_pokes_with(&conn, now, &settings(), 16, &|_| Ok(Some(moved))).unwrap();
    // Frozen, it would be due at deadline-40s; the extension delays it.
    assert!(with(deadline - 40_000).is_empty());
    assert!(with(moved - 40_001).is_empty());
    let found = with(moved - 40_000);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].receipts[0].effective_deadline, moved);
    // Past the moved deadline it is the hard warning's again.
    assert!(with(moved).is_empty());
    // A receipt without an effective deadline is never a candidate.
    assert!(
        due_pokes_with(&conn, deadline - 40_000, &settings(), 16, &|_| Ok(None))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn active_catch_up_excludes_the_thread() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "a");
    send(&context, &mut conn, "tb", 1, "b");
    let latest: i64 = conn
        .query_row("SELECT max(deadline_at) FROM receipt_state", [], |r| {
            r.get(0)
        })
        .unwrap();
    // Both soft points have passed and neither receipt has expired.
    let now = latest - 40_000;
    assert_eq!(due(&conn, now)[0].receipts.len(), 2);
    conn.execute(
        "INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,state) VALUES ('rcv','ta',0,1,'e',0,'active')",
        [],
    )
    .unwrap();
    let found = due(&conn, now);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].receipts.len(), 1);
    assert_eq!(found[0].receipts[0].thread.as_str(), "tb");
    // The row ends: the thread's receipt is a candidate again.
    conn.execute(
        "UPDATE catch_up SET state='ended', end_reason='ready', ended_at=1 WHERE seat_id='rcv' AND thread_id='ta'",
        [],
    )
    .unwrap();
    assert_eq!(due(&conn, now)[0].receipts.len(), 2);
}

#[test]
fn poked_is_not_rearmed() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, _) = first_window(&conn);
    let now = deadline - 40_000;
    let found = due(&conn, now);
    let tx = conn.transaction().unwrap();
    assert_eq!(
        mark_soft_poked(&tx, &found[0].receipts, UtcMillis(now)).unwrap(),
        1
    );
    // A second mark keeps the first.
    assert_eq!(
        mark_soft_poked(&tx, &found[0].receipts, UtcMillis(now + 7)).unwrap(),
        0
    );
    tx.commit().unwrap();
    let marked: i64 = conn
        .query_row("SELECT soft_poked_at FROM receipt_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marked, now);
    assert!(due(&conn, now).is_empty());
    // An extension that would put the soft point in the future, and one that
    // leaves it passed, both leave a poked receipt unarmed.
    for moved in [deadline + 100_000, deadline + 1] {
        assert!(
            due_pokes_with(&conn, moved - 40_000, &settings(), 16, &|_| Ok(Some(moved)))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn coalesces_per_seat() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 2, "a");
    send(&context, &mut conn, "tb", 1, "b");
    let latest: i64 = conn
        .query_row("SELECT max(deadline_at) FROM receipt_state", [], |r| {
            r.get(0)
        })
        .unwrap();
    let found = due(&conn, latest - 40_000);
    assert_eq!(found.len(), 1, "one PokeDue per seat");
    let receipts = &found[0].receipts;
    assert_eq!(receipts.len(), 3);
    let keys: Vec<(i64, &str)> = receipts
        .iter()
        .map(|r| (r.effective_deadline, r.message.as_str()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "ordered by effective deadline then message");
    let threads: std::collections::BTreeSet<_> =
        receipts.iter().map(|r| r.thread.as_str()).collect();
    assert_eq!(threads.into_iter().collect::<Vec<_>>(), ["ta", "tb"]);
    // `limit` bounds seats, not receipts.
    assert_eq!(
        due_pokes(&conn, latest - 40_000, &settings(), 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn warned_receipts_are_not_candidates() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 2, "a");
    let latest: i64 = conn
        .query_row("SELECT max(deadline_at) FROM receipt_state", [], |r| {
            r.get(0)
        })
        .unwrap();
    let now = latest - 40_000;
    assert_eq!(due(&conn, now)[0].receipts.len(), 2);
    conn.execute(
        "UPDATE receipt_state SET warning_message_id='w' WHERE message_id=(SELECT message_id FROM receipt_state ORDER BY deadline_at, message_id LIMIT 1)",
        [],
    )
    .unwrap();
    assert_eq!(due(&conn, now)[0].receipts.len(), 1);
}

#[test]
fn legacy_physical_receipt_is_marked_on_its_own_row() {
    let (_context, conn, _guard) = fixture();
    conn.execute_batch("\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('legacy','i','ta',1,'ordinary','snd','x',0,5);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('legacy','ta','rcv','pending',1000,0,1000);\
    ").unwrap();
    assert!(due(&conn, 599).is_empty());
    let found = due(&conn, 600);
    assert_eq!(found[0].receipts[0].source, PokeSource::Receipts);
    assert_eq!(found[0].receipts[0].message, MessageId::new("legacy"));
    let mut conn = conn;
    let tx = conn.transaction().unwrap();
    assert_eq!(
        mark_soft_poked(&tx, &found[0].receipts, UtcMillis(600)).unwrap(),
        1
    );
    tx.commit().unwrap();
    assert!(due(&conn, 600).is_empty());
    let marked: i64 = conn
        .query_row("SELECT soft_poked_at FROM receipts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(marked, 600);
}

struct FixedClock;
impl crate::protocol::time::Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(700)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(1_000)
    }
}

fn budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(10_000),
        cancellation: crate::protocol::time::Cancellation::default(),
    }
}

/// A cooperatively bound seat `s` on a verified target with one pending legacy
/// receipt whose soft point (600) has passed and deadline (1000) has not.
fn bound_store() -> (crate::store::SqliteStore, PathBuf) {
    use crate::store::{SqliteStore, StoreSettings};
    let path = std::env::temp_dir().join(format!("ht-poke-store-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',0,'codex','session','self-reported','cooperative_top_level',0,0,'term-pane','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined');\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m','t','s','pending',1000,0,1000);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(
                uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            ),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (store, path)
}

#[test]
fn poke_reservation_shares_the_wake_slot_and_marks_only_on_submit() {
    use crate::ports::{ReservedWakeAuthority, StorePort, WakeOutcome};
    let (store, path) = bound_store();
    let reader = || {
        StoreContext::new(path.clone(), Arc::new(FixedClock))
            .open_writer()
            .unwrap()
    };
    let marked = || -> Option<i64> {
        reader()
            .query_row("SELECT soft_poked_at FROM receipts", [], |r| r.get(0))
            .unwrap()
    };
    let due = StorePort::poke_candidates(&store, 16, &budget()).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].seat, SeatId::new("s"));

    let reserved = StorePort::reserve_poke(&store, &due[0], &budget())
        .unwrap()
        .expect("a bound cooperative seat reserves a poke");
    assert_eq!(reserved.reservation.reasons, ["soft_deadline"]);
    assert!(matches!(
        &reserved.reservation.authority,
        ReservedWakeAuthority::Cooperative { harness: Some(h), .. } if h == "codex"
    ));
    assert_eq!(reserved.receipts, due[0].receipts);
    assert!(
        StorePort::validate_wake_reservation(&store, &reserved.reservation, &budget()).unwrap(),
        "the final fence accepts a fresh poke reservation"
    );
    // One reservation slot per seat: a second poke is refused while it is held.
    assert!(
        StorePort::reserve_poke(&store, &due[0], &budget())
            .unwrap()
            .is_none()
    );

    // An abandoned attempt marks nothing: the receipt may be poked again.
    StorePort::complete_poke(
        &store,
        reserved.reservation.attempt.clone(),
        WakeOutcome::OutcomeUnknown,
        &reserved.receipts,
        &budget(),
    )
    .unwrap();
    assert_eq!(marked(), None);
    let again = StorePort::poke_candidates(&store, 16, &budget()).unwrap();
    assert_eq!(again.len(), 1);

    // Host acceptance marks exactly the reserved receipts, atomically.
    let reserved = StorePort::reserve_poke(&store, &again[0], &budget())
        .unwrap()
        .unwrap();
    StorePort::complete_poke(
        &store,
        reserved.reservation.attempt.clone(),
        WakeOutcome::Submitted,
        &reserved.receipts,
        &budget(),
    )
    .unwrap();
    assert_eq!(marked(), Some(700));
    assert!(
        StorePort::poke_candidates(&store, 16, &budget())
            .unwrap()
            .is_empty()
    );
    // A poke never writes the wake retry history.
    let (step, last): (i64, Option<String>) = reader()
        .query_row(
            "SELECT retry_step, last_reservation_id FROM wake_work WHERE seat_id='s'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((step, last), (0, None));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn human_bound_seat_is_not_a_candidate() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, _) = first_window(&conn);
    let soft = deadline - 40_000;
    assert_eq!(due(&conn, soft).len(), 1);
    conn.execute(
        "UPDATE occupant_bindings SET harness='human' WHERE seat_id='rcv'",
        [],
    )
    .unwrap();
    assert!(due(&conn, soft).is_empty());
    assert!(
        due_pokes_for_seat(&conn, &SeatId::new("rcv"), soft, &settings())
            .unwrap()
            .is_none()
    );
}

#[test]
fn unresolved_targetless_and_held_seats_are_not_candidates() {
    let breakages = [
        "UPDATE seats SET state='unresolved', unresolved_reason='other' WHERE id='rcv'",
        "UPDATE seats SET target_id=NULL WHERE id='rcv'",
        "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','p-rcv','b',1,'test')",
    ];
    for breakage in breakages {
        let (context, mut conn, _guard) = fixture();
        send(&context, &mut conn, "ta", 1, "one");
        let (deadline, _) = first_window(&conn);
        let soft = deadline - 40_000;
        assert_eq!(due(&conn, soft).len(), 1, "before: {breakage}");
        conn.execute(breakage, []).unwrap();
        assert!(due(&conn, soft).is_empty(), "after: {breakage}");
    }
    // A released hold does not exclude the seat.
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, _) = first_window(&conn);
    conn.execute(
        "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason,released_at) VALUES ('i','p-rcv','b',1,'test',1)",
        [],
    )
    .unwrap();
    assert_eq!(due(&conn, deadline - 40_000).len(), 1);
}

#[test]
fn never_reservable_seats_do_not_displace_a_pokeable_seat() {
    let (context, mut conn, _guard) = fixture();
    send(&context, &mut conn, "ta", 1, "one");
    let (deadline, available) = first_window(&conn);
    for n in 0..16 {
        let seat = format!("h{n:02}");
        let target = format!("p-{seat}");
        let message = format!("m-{seat}");
        conn.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,unavailability_open) VALUES (?1,'i','resolved','native',?2,1,1,0,1,0)",
            rusqlite::params![seat, target],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'b',1,'human','s','e','fresh',0,0,'t','inc')",
            rusqlite::params![seat, target],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('ta',?1,'joined')",
            [&seat],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,'i','ta',?2,'ordinary','body',?2,0)",
            rusqlite::params![message, 1000 + n],
        )
        .unwrap();
        // Due at rcv's soft point, with a deadline earlier than rcv's.
        conn.execute(
            "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'ta',?2,'pending',?3,?4,?5)",
            rusqlite::params![message, seat, WINDOW_MS as i64, available, deadline - 1_000 + n],
        )
        .unwrap();
    }
    let found = due(&conn, deadline - 40_000);
    let seats: Vec<&str> = found.iter().map(|d| d.seat.as_str()).collect();
    assert_eq!(seats, ["rcv"], "only the pokeable seat is a candidate");
}
