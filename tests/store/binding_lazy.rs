//! Lazy delivery state per binding (ht-54r.22). Included from `src/store/lazy_delivery.rs` as
//! `binding_lazy_tests`. A lazy message is unread for a binding until its cursor passes it.
use super::*;
use crate::protocol::{ids::*, time::UtcMillis};
use crate::store::{
    bindings::{
        BindingRole, EnrollOutcome, EnrollRequest, EnrollTrigger, enroll,
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
    db.execute(
        "INSERT OR IGNORE INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('a','i','resolved','native',1,0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES(?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy')",[format!("prep-{n}")]).unwrap();
    db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,'t','s')",params![format!("prep-{n}"),format!("msg-{n}")]).unwrap();
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
    // The message was pending at the V31 upgrade, and the cursor is already past it.
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
    db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=5,disconnect_reason='process_exited' WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
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
