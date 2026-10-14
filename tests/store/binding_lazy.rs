//! Lazy delivery state per binding (ht-54r.22). Included from `src/store/lazy_delivery/mod.rs` as
//! `binding_lazy_tests`. A lazy message is unread for a binding until its cursor passes it.
use super::*;
use crate::protocol::{ids::*, priority::NudgeThreshold, time::UtcMillis};
use crate::store::{
    bindings::{
        BindingRole, DisconnectReason, EnrollOutcome, EnrollRequest, EnrollTrigger, enroll,
        tests::{db, key, placement},
    },
    read_cursors,
};

fn enroll_binding(db: &Connection, session: &str, subagent_of: Option<&BindingId>) -> BindingId {
    let (k, p) = (key(session, subagent_of.map(|_| "agent-1")), placement());
    let req = EnrollRequest {
        key: &k,
        role: if subagent_of.is_some() {
            BindingRole::Subagent
        } else {
            BindingRole::TopLevel
        },
        parent: subagent_of,
        lineage: None,
        fingerprint: None,
        placement: &p,
        trigger: EnrollTrigger::Lifecycle,
    };
    match enroll(db, &req, UtcMillis(10)).unwrap() {
        EnrollOutcome::Enrolled { binding, .. } => binding.binding_id,
        other => panic!("{other:?}"),
    }
}

/// Seat `a` (the sender) and a lazy message `msg-<n>` at thread sequence `n` for seat `s`,
/// published. Sequences 1 and 2 are taken by the fixture's ordinary messages, so use n >= 3.
fn lazy(db: &Connection, n: i64) {
    lazy_with_priority(db, n, 1);
}

/// `lazy`, with the delivery row's priority set at insert (a later `UPDATE` is refused).
fn lazy_with_priority(db: &Connection, n: i64, priority: i64) {
    db.execute(
        "INSERT OR IGNORE INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('a','i','resolved','native',1,0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES(?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')",[format!("prep-{n}")]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,'t','s',?3)",params![format!("prep-{n}"),format!("msg-{n}"),priority]).unwrap();
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,actor_seat_id) VALUES(?1,'i','t',?2,'ordinary',?2,'passive',0,'lazy','a')",params![format!("msg-{n}"),n]).unwrap();
    db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES(?1,?2,'i','t',?3,0,?3,0,1,0)",params![format!("prep-{n}"),format!("msg-{n}"),n]).unwrap();
}

fn msg(n: i64) -> MessageId {
    MessageId::new(format!("msg-{n}"))
}

fn unread(db: &Connection, binding: &BindingId) -> Vec<MessageId> {
    binding_lazy_unread(db, binding, 100)
        .unwrap()
        .into_iter()
        .map(|u| u.message)
        .collect()
}

fn complete(db: &mut Connection, binding: Option<&BindingId>, ids: &[MessageId]) {
    let tx = db.transaction().unwrap();
    complete_for_binding(&tx, "i", &SeatId::new("s"), binding, ids).unwrap();
    tx.commit().unwrap();
}

fn disconnect(db: &Connection, binding: &BindingId, reason: DisconnectReason) {
    db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=5,disconnect_reason=?2 WHERE binding_id=?1",
        params![binding.as_str(), reason.as_str()],
    )
    .unwrap();
}

#[test]
fn lazy_message_is_delivered_once_per_binding() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    lazy(&db, 3);
    assert_eq!(unread(&db, &a), [msg(3)]);
    assert_eq!(unread(&db, &b), [msg(3)]);
    assert_eq!(seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0, 1);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert!(unread(&db, &a).is_empty(), "A's cursor passed it");
    assert_eq!(
        unread(&db, &b),
        [msg(3)],
        "completing for A leaves B unread"
    );
    assert_eq!(
        seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0,
        1,
        "the seat view still has B's unread message"
    );
    // Completing again is a no-op.
    complete(&mut db, Some(&a), &[msg(3)]);
    assert!(unread(&db, &a).is_empty());
    assert_eq!(unread(&db, &b), [msg(3)]);
    complete(&mut db, Some(&b), &[msg(3)]);
    assert!(unread(&db, &b).is_empty());
    assert_eq!(
        seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap(),
        (0, 0)
    );
    assert_eq!(
        read_cursors::cursor_for(&db, &a, &ThreadId::new("t")).unwrap(),
        Some(3)
    );
}

#[test]
fn a_later_binding_inherits_the_seat_maximum_cursor() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    lazy(&db, 3);
    // Cursors start at the seat's maximum: B joins while the message is still unread by A.
    let b = enroll_binding(&db, "sess-B", None);
    assert_eq!(unread(&db, &b), [msg(3)]);
    complete(&mut db, Some(&a), &[msg(3)]);
    // C joins after A displayed it: nothing is delivered twice.
    let c = enroll_binding(&db, "sess-C", None);
    assert!(unread(&db, &c).is_empty());
    assert_eq!(unread(&db, &b), [msg(3)]);
}

#[test]
fn legacy_pending_is_shown_to_top_level_bindings_until_displayed() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let sub = enroll_binding(&db, "sess-A", Some(&a));
    lazy(&db, 3);
    // The message was pending at the V34 upgrade, and the cursor is already past it.
    db.execute("UPDATE lazy_recipients SET legacy_pending=1", [])
        .unwrap();
    db.execute("UPDATE binding_cursors SET seq=3", []).unwrap();
    assert_eq!(
        unread(&db, &a),
        [msg(3)],
        "top-level bindings see legacy rows"
    );
    assert!(unread(&db, &sub).is_empty(), "a subagent never sees them");
    assert_eq!(seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0, 1);
    // A second top-level binding sees it too, until any top-level display clears it.
    let b = enroll_binding(&db, "sess-B", None);
    assert_eq!(unread(&db, &b), [msg(3)]);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert!(unread(&db, &a).is_empty());
    assert!(
        unread(&db, &b).is_empty(),
        "a top-level display clears the legacy row for the whole seat"
    );
    assert_eq!(
        db.query_row("SELECT legacy_pending FROM lazy_recipients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn legacy_pending_survives_a_subagent_display_attempt() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let sub = enroll_binding(&db, "sess-A", Some(&a));
    lazy(&db, 3);
    db.execute("UPDATE lazy_recipients SET legacy_pending=1", [])
        .unwrap();
    // The handler refuses subagent completions; the primitive itself must not clear the flag.
    complete(&mut db, Some(&sub), &[msg(3)]);
    assert_eq!(
        db.query_row("SELECT legacy_pending FROM lazy_recipients", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(unread(&db, &a), [msg(3)]);
}

#[test]
fn a_seat_without_a_connected_top_level_binding_counts_only_legacy_rows() {
    let db = db();
    let a = enroll_binding(&db, "sess-A", None);
    lazy(&db, 3);
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    assert_eq!(seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0, 1);
    disconnect(&db, &a, DisconnectReason::ProcessExit);
    assert_eq!(
        seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap(),
        (0, 0),
        "a disconnected binding is not a reader"
    );
    db.execute("UPDATE lazy_recipients SET legacy_pending=1", [])
        .unwrap();
    assert_eq!(
        seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0,
        1,
        "legacy rows count regardless"
    );
}

#[test]
fn a_disconnected_sibling_still_owes_its_display_and_sees_it_after_reconnecting() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    lazy(&db, 3);
    // The ordered seek reads the publication decision `note_published` copies onto the row.
    let tx = db.transaction().unwrap();
    note_published(&tx, "prep-3").unwrap();
    tx.commit().unwrap();
    let state = |db: &Connection| -> String {
        db.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id='msg-3'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let offered = |db: &Connection, b: &BindingId| -> Option<MessageId> {
        crate::store::lazy_delivery::seek_pending(
            db,
            "i",
            &SeatId::new("s"),
            RowFilter::Binding(b),
            None,
            i64::MAX as u64,
            100,
        )
        .unwrap()
        .row
        .map(|(_, recipient)| recipient.message)
    };
    disconnect(&db, &b, DisconnectReason::ProcessExit);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!(
        state(&db),
        "pending",
        "the open disconnected sibling still owes it"
    );
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(),
        None,
        "a disconnected sibling holds no wake frontier"
    );
    db.execute(
        "UPDATE occupant_bindings SET liveness='connected',disconnected_at=NULL,disconnect_reason=NULL WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    assert_eq!(unread(&db, &b), [msg(3)]);
    assert_eq!(
        offered(&db, &b),
        Some(msg(3)),
        "the ordered seek still offers it"
    );
    complete(&mut db, Some(&b), &[msg(3)]);
    assert_eq!(state(&db), "displayed");
}

#[test]
fn an_ended_sibling_owes_nothing_to_the_displayed_mark() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    lazy(&db, 3);
    db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=5,disconnect_reason='liveness_process_exit',ended_at=6 WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!(
        db.query_row(
            "SELECT state FROM lazy_recipients WHERE message_id='msg-3'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "displayed"
    );
}

#[test]
fn stale_superseded_and_end_event_siblings_owe_nothing_and_hold_no_wake_frontier() {
    let mut db = db();
    let live = enroll_binding(&db, "sess-A", None);
    let superseded = enroll_binding(&db, "sess-B", None);
    let ended_by_event = enroll_binding(&db, "sess-C", None);
    lazy(&db, 3);
    let tx = db.transaction().unwrap();
    note_published(&tx, "prep-3").unwrap();
    tx.commit().unwrap();
    disconnect(&db, &superseded, DisconnectReason::Superseded);
    disconnect(&db, &ended_by_event, DisconnectReason::EndEvent);
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(),
        Some(3),
        "the live binding has it unread"
    );
    complete(&mut db, Some(&live), &[msg(3)]);
    let state: String = db
        .query_row(
            "SELECT state FROM lazy_recipients WHERE message_id='msg-3'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "displayed",
        "stale open bindings never pin the row pending"
    );
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(),
        None,
        "served in-turn: no idle re-nudge"
    );
    assert_eq!(
        seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap(),
        (0, 0)
    );
    // The stale bindings still see their own unread view (cursor semantics unchanged).
    assert_eq!(unread(&db, &superseded), [msg(3)]);
}

#[test]
fn the_display_owing_reason_literal_is_the_process_exit_reason() {
    // `seat_owes_display_sql!` spells this reason in SQL.
    assert_eq!(
        DisconnectReason::ProcessExit.as_str(),
        "liveness_process_exit"
    );
}

/// An ordinary message `n` to seat `s` (a receipt), or from `s` itself, or to nobody.
fn ordinary(db: &Connection, n: i64, actor: Option<&str>, receipt_for_s: bool) {
    db.execute_batch("INSERT OR IGNORE INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('a','i','resolved','native',1,0)").unwrap();
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,actor_seat_id) VALUES(?1,'i','t',?2,'ordinary',?2,'b',0,?3)",params![format!("ord-{n}"),n,actor]).unwrap();
    if receipt_for_s {
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES(?1,'t','s','pending',100)",[format!("ord-{n}")]).unwrap();
        // "Receivable" is a prepared recipient joined to a send manifest (read_cursors).
        db.execute_batch(&format!(
            "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('prep-ord-{n}','i','actor','prep-ord-{n}',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
             INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('prep-ord-{n}','t','s',1,100,1);
             INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-ord-{n}','ord-{n}','i','t',{n},0,{n},0,1,0);"
        ))
        .unwrap();
    }
}

#[test]
fn advance_through_displayed_stops_at_the_first_gap() {
    let db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let t = ThreadId::new("t");
    // m1/m2 (fixture) have no recipient: skippable. 3 lazy, 4 ordinary to s (a gap), 5 lazy.
    lazy(&db, 3);
    ordinary(&db, 4, Some("a"), true);
    lazy(&db, 5);
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    assert_eq!(
        read_cursors::advance_through_displayed(&db, &a, &t, &[3, 5]).unwrap(),
        3,
        "stops at the undisplayed ordinary message 4"
    );
    assert_eq!(read_cursors::cursor_for(&db, &a, &t).unwrap(), Some(3));
    // Monotonic: showing an older message again never moves the cursor back.
    assert_eq!(
        read_cursors::advance_through_displayed(&db, &a, &t, &[3]).unwrap(),
        3
    );
    // Once 4 is displayed too, the prefix runs through 5.
    assert_eq!(
        read_cursors::advance_through_displayed(&db, &a, &t, &[4, 5]).unwrap(),
        5
    );
    // The seat's own sends and messages it never receives count as displayed.
    ordinary(&db, 6, Some("s"), false);
    ordinary(&db, 7, Some("a"), false);
    lazy(&db, 8);
    assert_eq!(
        read_cursors::advance_through_displayed(&db, &a, &t, &[8]).unwrap(),
        8
    );
    assert_eq!(read_cursors::cursor_for(&db, &a, &t).unwrap(), Some(8));
    // Nothing displayed: nothing moves.
    assert_eq!(
        read_cursors::advance_through_displayed(&db, &a, &t, &[]).unwrap(),
        8
    );
}

#[test]
fn has_unread_is_lazy_aware() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    assert!(!read_cursors::has_unread(&db, &a).unwrap());
    lazy(&db, 3);
    assert!(read_cursors::has_unread(&db, &a).unwrap());
    assert!(binding_has_unread_lazy(&db, &a).unwrap());
    complete(&mut db, Some(&a), &[msg(3)]);
    assert!(!read_cursors::has_unread(&db, &a).unwrap());
}

#[test]
fn a_binding_less_completion_counts_for_every_open_top_level_binding() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    lazy(&db, 3);
    complete(&mut db, None, &[msg(3)]);
    assert!(unread(&db, &a).is_empty());
    assert!(unread(&db, &b).is_empty());
}

#[test]
fn completion_state_follows_the_binding() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    lazy(&db, 3);
    fn state(db: &Connection, binding: Option<&BindingId>) -> LazyCompletion {
        lazy_completion_state(db, "i", &SeatId::new("s"), binding, &msg(3)).unwrap()
    }
    assert_eq!(state(&db, Some(&a)), LazyCompletion::Pending);
    assert_eq!(state(&db, None), LazyCompletion::Pending);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!(state(&db, Some(&a)), LazyCompletion::Completed);
    assert_eq!(state(&db, Some(&b)), LazyCompletion::Pending);
    assert_eq!(
        lazy_completion_state(&db, "i", &SeatId::new("other"), None, &msg(3)).unwrap(),
        LazyCompletion::NotAddressed
    );
}

fn leave_thread(db: &Connection) {
    assert_eq!(
        db.execute(
            "UPDATE memberships SET state='left' WHERE thread_id='t' AND seat_id='s'",
            []
        )
        .unwrap(),
        1,
        "the fixture seat was a member of the thread"
    );
}

/// A binding enrolled after the seat left the thread owns no cursor row for it: a missing row must
/// not read as "everything unread" for any lazy reader built from the predicate.
#[test]
fn left_threads_are_not_lazy_unread_for_a_binding_without_a_cursor() {
    let db = db();
    let early = enroll_binding(&db, "sess-A", None);
    lazy(&db, 3);
    leave_thread(&db);
    let a = enroll_binding(&db, "sess-B", None);
    assert!(
        read_cursors::cursor_for(&db, &a, &ThreadId::new("t"))
            .unwrap()
            .is_none(),
        "a binding enrolled after the leave seeds no cursor for the thread"
    );
    assert!(unread(&db, &a).is_empty(), "binding_lazy_unread");
    assert!(!binding_has_unread_lazy(&db, &a).unwrap());
    assert!(!read_cursors::has_unread(&db, &a).unwrap());
    assert!(
        binding_lazy_window(&db, "i", &a, i64::MAX, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        lazy_completion_state(&db, "i", &SeatId::new("s"), Some(&a), &msg(3)).unwrap(),
        LazyCompletion::Completed,
        "a message in a left thread is not pending for the new binding"
    );
    // The earlier binding addressed the message before the leave: leaving does not erase it.
    assert_eq!(unread(&db, &early), [msg(3)]);
    assert_eq!(seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0, 1);
    assert_eq!(
        lazy_completion_state(&db, "i", &SeatId::new("s"), Some(&early), &msg(3)).unwrap(),
        LazyCompletion::Pending
    );
}

#[test]
fn legacy_pending_survives_leaving_the_thread() {
    let db = db();
    lazy(&db, 3);
    db.execute("UPDATE lazy_recipients SET legacy_pending=1", [])
        .unwrap();
    leave_thread(&db);
    // Enrolled after the leave: no cursor row, yet the pre-upgrade pending row stays unread.
    let a = enroll_binding(&db, "sess-A", None);
    let sub = enroll_binding(&db, "sess-A", Some(&a));
    assert_eq!(unread(&db, &a), [msg(3)], "legacy rows are not gated");
    assert!(unread(&db, &sub).is_empty(), "never for a subagent");
    assert_eq!(seat_lazy_pending(&db, &SeatId::new("s"), 100).unwrap().0, 1);
    assert_eq!(
        lazy_completion_state(&db, "i", &SeatId::new("s"), Some(&a), &msg(3)).unwrap(),
        LazyCompletion::Pending
    );
}

// ht-yha.44: every binding keeps its own pending view (`binding_pending`).

/// `lazy(n)` published the way a send publishes it (`note_published` stamps `decision_seq`).
fn publish(db: &mut Connection, n: i64) {
    lazy(db, n);
    let tx = db.transaction().unwrap();
    note_published(&tx, &format!("prep-{n}")).unwrap();
    tx.commit().unwrap();
}

/// Rows in the binding's pending view.
fn view(db: &Connection, binding: &BindingId) -> i64 {
    db.query_row(
        "SELECT count(*) FROM binding_pending WHERE binding_id=?1",
        [binding.as_str()],
        |r| r.get(0),
    )
    .unwrap()
}

/// The first row a fresh page's seek returns for `filter`, and the candidates it visited.
fn first(db: &Connection, filter: RowFilter<'_>) -> (Option<MessageId>, usize) {
    let seek = seek_pending(
        db,
        "i",
        &SeatId::new("s"),
        filter,
        None,
        i64::MAX as u64,
        100,
    )
    .unwrap();
    (
        seek.row.map(|(_, recipient)| recipient.message),
        seek.visited,
    )
}

/// Every row the binding's ordered seek offers, in order.
fn drain_binding(db: &Connection, binding: &BindingId) -> Vec<MessageId> {
    let (mut after, mut seen) = (None, Vec::new());
    loop {
        let seek = seek_pending(
            db,
            "i",
            &SeatId::new("s"),
            RowFilter::Binding(binding),
            after,
            i64::MAX as u64,
            100,
        )
        .unwrap();
        seen.extend(seek.row.map(|(_, recipient)| recipient.message));
        after = seek.position;
        if seek.exhausted {
            return seen;
        }
    }
}

fn reconnect(db: &Connection, binding: &BindingId) {
    db.execute(
        "UPDATE occupant_bindings SET liveness='connected',disconnected_at=NULL,disconnect_reason=NULL WHERE binding_id=?1",
        [binding.as_str()],
    )
    .unwrap();
}

fn end_binding(db: &Connection, binding: &BindingId) {
    db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=5,disconnect_reason='liveness_process_exit',ended_at=6 WHERE binding_id=?1",
        [binding.as_str()],
    )
    .unwrap();
}

#[test]
fn two_sessions_keep_independent_views_through_every_display_path() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    for n in 3..=6 {
        publish(&mut db, n);
    }
    assert_eq!((view(&db, &a), view(&db, &b)), (4, 4));
    complete(&mut db, Some(&a), &[msg(3)]);
    let t = ThreadId::new("t");
    let tx = db.transaction().unwrap();
    read_cursors::advance_through_displayed(&tx, &a, &t, &[4]).unwrap();
    read_cursors::advance_through_displayed(&tx, &a, &t, &[6]).unwrap();
    tx.commit().unwrap();
    assert_eq!(drain_binding(&db, &a), [msg(5)]);
    assert_eq!(view(&db, &a), 1);
    assert_eq!(
        drain_binding(&db, &b),
        [msg(3), msg(4), msg(5), msg(6)],
        "a sibling's displays never change what b is offered"
    );
    assert_eq!(view(&db, &b), 4);
    let c = enroll_binding(&db, "sess-C", None);
    assert_eq!(unread(&db, &c), [msg(5)]);
    assert_eq!(drain_binding(&db, &c), unread(&db, &c));
    end_binding(&db, &b);
    assert_eq!(view(&db, &b), 0);
}

#[test]
fn a_process_exited_siblings_backlog_never_slows_the_live_session() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    disconnect(&db, &b, DisconnectReason::ProcessExit);
    for n in 3..=103 {
        publish(&mut db, n);
    }
    complete(&mut db, Some(&a), &(3..=103).map(msg).collect::<Vec<_>>());
    assert_eq!(view(&db, &b), 101, "b keeps its own view");
    assert_eq!(view(&db, &a), 0);
    publish(&mut db, 104);
    assert_eq!(first(&db, RowFilter::Binding(&a)), (Some(msg(104)), 1));
    assert_eq!(
        first(&db, RowFilter::Seat),
        (Some(msg(104)), 1),
        "seat views read connected bindings only"
    );
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(),
        Some(104)
    );
}

#[test]
fn a_live_session_that_displayed_more_than_a_page_gets_the_next_row_first() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    for n in 3..=103 {
        publish(&mut db, n);
    }
    complete(&mut db, Some(&a), &(3..=103).map(msg).collect::<Vec<_>>());
    publish(&mut db, 104);
    assert_eq!(first(&db, RowFilter::Binding(&a)), (Some(msg(104)), 1));
    assert_eq!(
        first(&db, RowFilter::Binding(&b)),
        (Some(msg(3)), 1),
        "b's own backlog is intact and costs a nothing"
    );
    assert_eq!(first(&db, RowFilter::Seat), (Some(msg(3)), 1));
}

#[test]
fn the_wake_frontier_ignores_process_exited_bindings() {
    let mut db = db();
    db.execute("UPDATE seats SET nudge_threshold=-1 WHERE id='s'", [])
        .unwrap();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    publish(&mut db, 3);
    disconnect(&db, &b, DisconnectReason::ProcessExit);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!(wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(), None);
    assert_eq!(
        oldest_pending_at(&db, "s", NudgeThreshold::ALL).unwrap(),
        None
    );
    assert!(!has_wake_rows(&db, "s").unwrap());
    assert_eq!(first(&db, RowFilter::Seat), (None, 0));
    reconnect(&db, &b);
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::ALL).unwrap(),
        Some(3)
    );
    assert!(has_wake_rows(&db, "s").unwrap());
    assert_eq!(first(&db, RowFilter::Binding(&b)), (Some(msg(3)), 1));
}

#[test]
fn legacy_rows_enter_top_level_views_and_leave_once_displayed() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    publish(&mut db, 3);
    complete(&mut db, Some(&a), &[msg(3)]);
    complete(&mut db, Some(&b), &[msg(3)]);
    assert_eq!((view(&db, &a), view(&db, &b)), (0, 0));
    db.execute("UPDATE lazy_recipients SET legacy_pending=1", [])
        .unwrap();
    assert_eq!((view(&db, &a), view(&db, &b)), (1, 1));
    assert_eq!(first(&db, RowFilter::Binding(&a)).0, Some(msg(3)));
    assert_eq!(first(&db, RowFilter::Seat).0, Some(msg(3)));
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!((view(&db, &a), view(&db, &b)), (0, 0));
    assert_eq!(first(&db, RowFilter::Seat), (None, 0));
}

#[test]
fn the_backfill_seeds_every_open_binding_from_the_predicate() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    let c = enroll_binding(&db, "sess-C", None);
    for n in 3..=5 {
        publish(&mut db, n);
    }
    complete(&mut db, Some(&a), &[msg(3)]);
    read_cursors::advance_through_displayed(&db, &b, &ThreadId::new("t"), &[5]).unwrap();
    end_binding(&db, &c);
    db.execute("DELETE FROM binding_pending", []).unwrap();
    seed_all_binding_pending(&db).unwrap();
    for (binding, expected) in [(&a, [msg(4), msg(5)]), (&b, [msg(3), msg(4)])] {
        let rows: Vec<MessageId> = db
            .prepare("SELECT r.message_id FROM binding_pending p JOIN lazy_recipients r ON r.ordinal=p.ordinal WHERE p.binding_id=?1 ORDER BY p.ordinal")
            .unwrap()
            .query_map([binding.as_str()], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|id| MessageId::new(id.unwrap()))
            .collect();
        assert_eq!(rows, expected);
        assert_eq!(rows, unread(&db, binding));
    }
    assert_eq!(view(&db, &c), 0);
}

// ht-yha.54: a view is held only by a binding that can still be offered rows.

/// The real disconnect transition (no side-effect hooks).
fn disconnect_now(db: &Connection, binding: &BindingId, reason: DisconnectReason) {
    assert!(
        crate::store::bindings::disconnect_with_hooks(db, binding, reason, UtcMillis(5), &[])
            .unwrap()
    );
}

/// The real connected transition (a lifecycle check-in such as a resume).
fn connect_now(db: &Connection, binding: &BindingId) {
    assert!(
        crate::store::bindings::connect(
            db,
            binding,
            None,
            crate::store::bindings::ConnectTrigger::Lifecycle,
            UtcMillis(7)
        )
        .unwrap()
    );
}

/// A p0 delivery row, published (the row's priority cannot change after insert).
fn publish_p0(db: &mut Connection, n: i64) {
    lazy_with_priority(db, n, 0);
    let tx = db.transaction().unwrap();
    note_published(&tx, &format!("prep-{n}")).unwrap();
    tx.commit().unwrap();
}

/// The message ids of a binding's view, in ordinal order.
fn view_messages(db: &Connection, binding: &BindingId) -> Vec<MessageId> {
    db.prepare("SELECT r.message_id FROM binding_pending p JOIN lazy_recipients r ON r.ordinal=p.ordinal WHERE p.binding_id=?1 ORDER BY p.ordinal")
        .unwrap()
        .query_map([binding.as_str()], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|id| MessageId::new(id.unwrap()))
        .collect()
}

#[test]
fn publication_writes_views_only_for_bindings_that_can_still_be_offered_rows() {
    let mut db = db();
    let live = enroll_binding(&db, "sess-live", None);
    let px = enroll_binding(&db, "sess-px", None);
    disconnect_now(&db, &px, DisconnectReason::ProcessExit);
    let mut away = Vec::new();
    for i in 0..10 {
        let sibling = enroll_binding(&db, &format!("sess-sib-{i}"), None);
        let reason = if i < 5 {
            DisconnectReason::EndEvent
        } else {
            DisconnectReason::Superseded
        };
        disconnect_now(&db, &sibling, reason);
        away.push(sibling);
    }
    for i in 0..8 {
        let sub = enroll_binding(&db, &format!("sess-sub-{i}"), Some(&live));
        disconnect_now(&db, &sub, DisconnectReason::EndEvent);
        away.push(sub);
    }
    let sub_live = enroll_binding(&db, "sess-sub-live", Some(&live));
    publish(&mut db, 3);
    publish(&mut db, 4);
    publish(&mut db, 5);
    for held in [&live, &px, &sub_live] {
        assert_eq!(view(&db, held), 3);
    }
    for binding in &away {
        assert_eq!(view(&db, binding), 0);
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM binding_pending", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        9
    );
}

#[test]
fn a_disconnect_other_than_process_exit_drops_the_view() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    let c = enroll_binding(&db, "sess-C", None);
    publish(&mut db, 3);
    publish(&mut db, 4);
    assert_eq!((view(&db, &a), view(&db, &b), view(&db, &c)), (2, 2, 2));
    disconnect_now(&db, &b, DisconnectReason::EndEvent);
    disconnect_now(&db, &c, DisconnectReason::ProcessExit);
    assert_eq!(view(&db, &b), 0);
    assert_eq!(view(&db, &c), 2, "a process-exited binding reconnects");
    assert_eq!(
        first(&db, RowFilter::Binding(&b)),
        (None, 0),
        "accepted limit: no rows until a lifecycle check-in reconnects it"
    );
}

#[test]
fn a_resumed_binding_is_offered_what_a_sibling_displayed_while_it_was_away() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    publish_p0(&mut db, 3);
    disconnect_now(&db, &b, DisconnectReason::EndEvent);
    complete(&mut db, Some(&a), &[msg(3)]);
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::DEFAULT).unwrap(),
        None
    );
    assert_eq!(view(&db, &b), 0);
    connect_now(&db, &b);
    assert_eq!(view(&db, &b), 1);
    assert_eq!(view_messages(&db, &b), unread(&db, &b));
    assert_eq!(drain_binding(&db, &b), [msg(3)]);
    assert_eq!(
        wake_frontier(&db, "s", NudgeThreshold::DEFAULT).unwrap(),
        Some(3)
    );
    assert!(has_wake_rows(&db, "s").unwrap());
}

#[test]
fn reconnect_reseeds_exactly_the_unread_rows() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    for n in 3..=6 {
        publish(&mut db, n);
    }
    disconnect_now(&db, &b, DisconnectReason::Superseded);
    let t = ThreadId::new("t");
    let tx = db.transaction().unwrap();
    read_cursors::advance_through_displayed(&tx, &b, &t, &[4]).unwrap();
    read_cursors::advance_through_displayed(&tx, &b, &t, &[6]).unwrap();
    tx.commit().unwrap();
    publish(&mut db, 7);
    assert_eq!(view(&db, &b), 0);
    assert_eq!(view(&db, &a), 5);
    connect_now(&db, &b);
    let expected = unread(&db, &b);
    assert!(!expected.is_empty());
    assert_eq!(view_messages(&db, &b), expected);
    assert!(expected.contains(&msg(7)) && !expected.contains(&msg(6)));
}

#[test]
fn the_view_triggers_spell_the_held_predicate() {
    let v47 = include_str!("../../migrations/0047_message_priority.sql");
    for name in ["binding_pending_publish", "binding_pending_legacy_set"] {
        let statement = v47
            .split("\n\n")
            .map(str::trim)
            .find(|s| s.starts_with(&format!("CREATE TRIGGER {name} ")))
            .unwrap();
        assert!(
            statement.contains(view_held_sql!("b")),
            "{name} must spell view_held_sql!(\"b\")"
        );
    }
}

/// The two view triggers as the first-amendment v47 (ht-yha.44) wrote them: every open binding.
const OLD_PUBLISH_TRIGGER: &str = "CREATE TRIGGER binding_pending_publish AFTER UPDATE OF decision_seq ON lazy_recipients
WHEN OLD.decision_seq IS NULL AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND b.ended_at IS NULL
      AND (EXISTS(SELECT 1 FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=NEW.thread_id)
        OR EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=NEW.thread_id AND mb.seat_id=NEW.seat_id AND mb.state IN ('joined','invited')));
END";
const OLD_LEGACY_SET_TRIGGER: &str = "CREATE TRIGGER binding_pending_legacy_set AFTER UPDATE OF legacy_pending ON lazy_recipients
WHEN OLD.legacy_pending=0 AND NEW.legacy_pending=1 AND NEW.decision_seq IS NOT NULL
BEGIN
    INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq)
    SELECT b.binding_id,NEW.ordinal,NEW.thread_id,NEW.priority,NEW.decision_seq FROM occupant_bindings b
    WHERE b.seat_id=NEW.seat_id AND b.ended_at IS NULL AND b.role='top_level';
END";

#[test]
fn a_first_amendment_v47_store_is_repaired_on_open() {
    let mut db = db();
    db.execute_batch(
        "DROP TRIGGER binding_pending_publish; DROP TRIGGER binding_pending_legacy_set;",
    )
    .unwrap();
    db.execute_batch(OLD_PUBLISH_TRIGGER).unwrap();
    db.execute_batch(OLD_LEGACY_SET_TRIGGER).unwrap();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    publish(&mut db, 3);
    assert_eq!((view(&db, &a), view(&db, &b)), (1, 1));
    disconnect(&db, &b, DisconnectReason::Superseded);
    assert_eq!(view(&db, &b), 1, "the raw helper leaves the old view");
    crate::store::schema::initialize(&db, || UtcMillis(0)).unwrap();
    let sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='binding_pending_publish'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains(view_held_sql!("b")));
    assert_eq!(view(&db, &b), 0);
    assert_eq!(view(&db, &a), 1);
    publish(&mut db, 4);
    assert_eq!((view(&db, &a), view(&db, &b)), (2, 0));
}

#[test]
fn the_backfill_skips_bindings_outside_the_held_set() {
    let mut db = db();
    let a = enroll_binding(&db, "sess-A", None);
    let b = enroll_binding(&db, "sess-B", None);
    let c = enroll_binding(&db, "sess-C", None);
    disconnect(&db, &b, DisconnectReason::EndEvent);
    disconnect(&db, &c, DisconnectReason::ProcessExit);
    publish(&mut db, 3);
    publish(&mut db, 4);
    db.execute("DELETE FROM binding_pending", []).unwrap();
    seed_all_binding_pending(&db).unwrap();
    assert_eq!((view(&db, &a), view(&db, &b), view(&db, &c)), (2, 0, 2));
}
