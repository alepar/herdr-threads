use super::*;
use crate::protocol::{ids::*, time::UtcMillis};
use crate::store::bindings::{
    BindingRole, EnrollOutcome, EnrollRequest, EnrollTrigger, enroll, tests::*,
};

fn open(db: &Connection, session: &str) -> BindingId {
    let (k, p) = (key(session, None), placement());
    let req = EnrollRequest {
        key: &k,
        role: BindingRole::TopLevel,
        parent: None,
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
fn t() -> ThreadId {
    ThreadId::new("t")
}

#[test]
fn init_for_new_binding_starts_at_the_seat_maximum_else_head() {
    let db = db();
    // A seat with no cursors: a new binding gets the head (2).
    let a = open(&db, "sess-A");
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
    // A's cursor is behind the head; B joins the seat and inherits the seat maximum (1).
    db.execute("UPDATE binding_cursors SET seq=1", []).unwrap();
    let b = open(&db, "sess-B");
    assert_eq!(cursor_for(&db, &b, &t()).unwrap(), Some(1));
    // A joined thread with no messages gets 0.
    db.execute_batch(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('e','i','empty','goal',0,0);
         INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('e','s','joined',2);",
    )
    .unwrap();
    let c = open(&db, "sess-C");
    assert_eq!(cursor_for(&db, &c, &ThreadId::new("e")).unwrap(), Some(0));
    // Only the missing cursor (the thread joined after A enrolled) is filled; A's own is kept.
    assert_eq!(init_for_new_binding(&db, &a).unwrap(), 1);
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(1));
    assert_eq!(init_for_new_binding(&db, &a).unwrap(), 0);
}

#[test]
fn advance_is_monotonic() {
    let db = db();
    let a = open(&db, "sess-A");
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    assert!(advance(&db, &a, &t(), 2).unwrap());
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
    assert!(!advance(&db, &a, &t(), 1).unwrap());
    assert!(!advance(&db, &a, &t(), 2).unwrap());
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
    // advance creates a missing cursor
    assert_eq!(cursor_for(&db, &a, &ThreadId::new("zz")).unwrap(), None);
}

#[test]
fn init_for_join_gives_every_open_binding_a_cursor_at_the_join_point() {
    let db = db();
    let a = open(&db, "sess-A");
    let b = open(&db, "sess-B");
    let ended = open(&db, "sess-C");
    db.execute(
        "UPDATE occupant_bindings SET ended_at=9 WHERE binding_id=?1",
        [ended.as_str()],
    )
    .unwrap();
    db.execute_batch(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('u','i','topic2','goal',0,0);
         INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('u','s','joined',2);
         INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES
           ('i','u1','u',1,'ordinary','b',3,3),('i','u2','u',2,'ordinary','b',4,4),('i','u3','u',3,'ordinary','b',5,5);",
    )
    .unwrap();
    assert_eq!(
        init_for_join(&db, &SeatId::new("s"), &ThreadId::new("u")).unwrap(),
        2
    );
    let u = ThreadId::new("u");
    assert_eq!(cursor_for(&db, &a, &u).unwrap(), Some(3));
    assert_eq!(cursor_for(&db, &b, &u).unwrap(), Some(3));
    assert_eq!(cursor_for(&db, &ended, &u).unwrap(), None);
    // Re-joining keeps a higher cursor.
    db.execute(
        "UPDATE binding_cursors SET seq=5 WHERE binding_id=?1 AND thread_id='u'",
        [a.as_str()],
    )
    .unwrap();
    init_for_join(&db, &SeatId::new("s"), &u).unwrap();
    assert_eq!(cursor_for(&db, &a, &u).unwrap(), Some(5));
}

#[test]
fn copy_lineage_copies_and_keeps_the_higher_cursor() {
    let db = db();
    let a = open(&db, "sess-A");
    let b = open(&db, "sess-B");
    db.execute(
        "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    db.execute(
        "UPDATE binding_cursors SET seq=2 WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    assert_eq!(copy_lineage(&db, &a, &b).unwrap(), 1);
    assert_eq!(cursor_for(&db, &b, &t()).unwrap(), Some(2)); // higher kept
    db.execute(
        "UPDATE binding_cursors SET seq=0 WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    copy_lineage(&db, &a, &b).unwrap();
    assert_eq!(cursor_for(&db, &b, &t()).unwrap(), Some(1)); // copied up
}

#[test]
fn seed_modes() {
    let db = db();
    let a = open(&db, "sess-A");
    db.execute("UPDATE binding_cursors SET seq=1", []).unwrap();
    // Build a bare row so seeding is the only thing that gives it cursors.
    let bare = |session: &str| -> BindingId {
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',4,'p','b',1,'claude',?1,'e','cooperative_top_level',1,'term','inc')",
            [session],
        )
        .unwrap();
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE native_session=?1",
            [session],
            |r| r.get::<_, String>(0),
        )
        .map(BindingId::new)
        .unwrap()
    };
    let fresh = bare("sess-F");
    let out = seed(&db, &fresh, SeedMode::Fresh).unwrap();
    assert!(out.first_contact);
    assert_eq!(cursor_for(&db, &fresh, &t()).unwrap(), Some(1));

    let lineage = bare("sess-L");
    let out = seed(&db, &lineage, SeedMode::Lineage(a.clone())).unwrap();
    assert!(!out.first_contact);
    assert_eq!(cursor_for(&db, &lineage, &t()).unwrap(), Some(1));

    // Readopt keeps existing cursors and fills missing joined threads.
    let readopt = bare("sess-R");
    db.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,'t',0)",
        [readopt.as_str()],
    )
    .unwrap();
    db.execute_batch(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('v','i','topic3','goal',0,0);
         INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('v','s','joined',2);",
    )
    .unwrap();
    let out = seed(&db, &readopt, SeedMode::Readopt).unwrap();
    assert!(!out.first_contact);
    assert_eq!(cursor_for(&db, &readopt, &t()).unwrap(), Some(0));
    assert_eq!(
        cursor_for(&db, &readopt, &ThreadId::new("v")).unwrap(),
        Some(0)
    );
}

#[test]
fn has_unread_is_a_false_stub() {
    let db = db();
    let a = open(&db, "sess-A");
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    assert!(!has_unread(&db, &a).unwrap());
}
