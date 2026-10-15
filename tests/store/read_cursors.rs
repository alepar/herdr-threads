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
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('e','34750f98bd59fcfc946da45aaabe933b','empty','goal',0,0,0);
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
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('u','34750f98bd59fcfc946da45aaabe933b','topic2','goal',0,0,0);
         INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('u','s','joined',2);
         INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES
           ('34750f98bd59fcfc946da45aaabe933b','u1','u',1,'ordinary','b',3,3),('34750f98bd59fcfc946da45aaabe933b','u2','u',2,'ordinary','b',4,4),('34750f98bd59fcfc946da45aaabe933b','u3','u',3,'ordinary','b',5,5);",
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
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('v','34750f98bd59fcfc946da45aaabe933b','topic3','goal',0,0,0);
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

/// An ordinary message `id` (sequence `seq` of thread `t`) published to seat `s` as a recipient.
fn deliver(db: &Connection, id: &str, seq: u32) {
    db.execute_batch(&format!(
        "INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','{id}','t',{seq},'ordinary','b',{seq},{seq}) ON CONFLICT DO NOTHING;
         INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p-{id}','34750f98bd59fcfc946da45aaabe933b','actor','{id}',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
         INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p-{id}','t','s',1,300,0);
         INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) SELECT 'p-{id}','{id}','34750f98bd59fcfc946da45aaabe933b','t',decision_seq,{seq},{seq},0,1,0 FROM messages WHERE id='{id}';"
    ))
    .unwrap();
}

#[test]
fn seed_precedence_readopt_over_lineage_over_seat_max() {
    let db = db();
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
    // The seat maximum in `t` is 2 (binding hi); the predecessor sits at 1.
    let hi = open(&db, "sess-hi");
    let pred = open(&db, "sess-pred");
    db.execute(
        "UPDATE binding_cursors SET seq=2 WHERE binding_id=?1",
        [hi.as_str()],
    )
    .unwrap();
    db.execute(
        "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1",
        [pred.as_str()],
    )
    .unwrap();
    // Fresh: the seat maximum.
    let fresh = bare("sess-F");
    seed(&db, &fresh, SeedMode::Fresh).unwrap();
    assert_eq!(cursor_for(&db, &fresh, &t()).unwrap(), Some(2));
    // Lineage: the predecessor's cursor beats the seat maximum.
    let successor = bare("sess-L");
    seed(&db, &successor, SeedMode::Lineage(pred.clone())).unwrap();
    assert_eq!(cursor_for(&db, &successor, &t()).unwrap(), Some(1));
    // Re-adoption: its own stored cursor beats both, and nothing is first contact.
    let readopt = bare("sess-R");
    db.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,'t',0)",
        [readopt.as_str()],
    )
    .unwrap();
    let out = seed(&db, &readopt, SeedMode::Readopt).unwrap();
    assert!(!out.first_contact);
    assert_eq!(cursor_for(&db, &readopt, &t()).unwrap(), Some(0));
}

#[test]
fn readopt_never_lowers_a_cursor() {
    let db = db();
    let a = open(&db, "sess-A");
    // A cursor already at the head stays there, and one the seat maximum would put lower too.
    db.execute(
        "UPDATE binding_cursors SET seq=2 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    let b = open(&db, "sess-B");
    db.execute(
        "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    seed(&db, &a, SeedMode::Readopt).unwrap();
    seed(&db, &b, SeedMode::Readopt).unwrap();
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
    assert_eq!(cursor_for(&db, &b, &t()).unwrap(), Some(1));
}

#[test]
fn retired_seat_keeps_its_cursor_rows() {
    let db = db();
    let a = open(&db, "sess-A");
    crate::store::bindings::end_open_bindings(
        &db,
        &SeatId::new("s"),
        crate::store::bindings::DisconnectReason::SeatRetired,
        UtcMillis(20),
    )
    .unwrap();
    db.execute(
        "UPDATE seats SET state='retired', retired_at=21 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE memberships SET state='retired', retired_at=21 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
}

#[test]
fn has_unread_tracks_ordinary_mail_past_the_cursor() {
    let db = db();
    let a = open(&db, "sess-A");
    let b = open(&db, "sess-B");
    // Nothing is addressed to the seat yet.
    assert!(!has_unread(&db, &a).unwrap());
    deliver(&db, "m1", 1);
    deliver(&db, "m2", 2);
    // Both bindings start at the head (2): everything is behind them.
    assert!(!has_unread(&db, &a).unwrap());
    db.execute(
        "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1",
        [a.as_str()],
    )
    .unwrap();
    assert!(has_unread(&db, &a).unwrap(), "m2 is past A's cursor");
    assert!(
        !has_unread(&db, &b).unwrap(),
        "B's cursor is unmoved by A's"
    );
    advance(&db, &a, &t(), 2).unwrap();
    assert!(!has_unread(&db, &a).unwrap());
    // A message in a thread the seat has left is not unread.
    db.execute(
        "UPDATE binding_cursors SET seq=0 WHERE binding_id=?1",
        [b.as_str()],
    )
    .unwrap();
    assert!(has_unread(&db, &b).unwrap());
    db.execute(
        "UPDATE memberships SET state='left', left_at=9 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    assert!(!has_unread(&db, &b).unwrap());
}

#[test]
fn advance_through_displayed_stops_at_the_first_gap() {
    let db = db();
    let a = open(&db, "sess-A");
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    // Messages 1..=4 reach the seat; 5 is an info event; 6 was sent by the seat itself (no
    // recipient row for it), 7 reaches the seat again.
    for (id, seq) in [("m1", 1), ("m2", 2), ("m3", 3), ("m4", 4)] {
        deliver(&db, id, seq);
    }
    db.execute_batch(
        "INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','info5','t',5,'info','{}',5,5);
         INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','own6','t',6,'ordinary','b',6,6);",
    )
    .unwrap();
    deliver(&db, "m7", 7);
    // 1 and 2 shown, 3 not: the cursor stops after 2.
    assert_eq!(
        advance_through_displayed(&db, &a, &t(), &[1, 2, 4]).unwrap(),
        2
    );
    assert_eq!(cursor_for(&db, &a, &t()).unwrap(), Some(2));
    // 3 and 4 shown: past them, the info event (5) and the seat's own message (6) count as
    // displayed, and the walk stops at 7 which was not shown.
    assert_eq!(
        advance_through_displayed(&db, &a, &t(), &[3, 4]).unwrap(),
        6
    );
    // Nothing shown moves nothing, and never backwards.
    assert_eq!(advance_through_displayed(&db, &a, &t(), &[]).unwrap(), 6);
    assert_eq!(advance_through_displayed(&db, &a, &t(), &[1]).unwrap(), 6);
    assert_eq!(advance_through_displayed(&db, &a, &t(), &[7]).unwrap(), 7);
}

/// ht-54r F1: a receipt this binding display-ACKed (its observation carries `action_provenance`:
/// a text-inbox display, a mod or hook-pull display settlement) is evidence the binding showed
/// the message whole, so the walk passes it. Another binding's display ACK, or this binding's
/// explicit (non-display) `ack`, is not.
#[test]
fn advance_through_displayed_counts_only_this_bindings_display_acks() {
    let db = db();
    let a = open(&db, "sess-A");
    let b = open(&db, "sess-B");
    db.execute("UPDATE binding_cursors SET seq=0", []).unwrap();
    for (id, seq) in [("m1", 1), ("m2", 2), ("m3", 3), ("m4", 4)] {
        deliver(&db, id, seq);
    }
    let acked = |id: &str, by: &BindingId, observation: &str| {
        db.execute(
            "INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at,ack_binding_id) VALUES (?1,'s','acked','s',4,?2,9,?3)",
            [id, observation, by.as_str()],
        )
        .unwrap();
    };
    acked(
        "m2",
        &a,
        r#"{"provenance":"cooperative_top_level","action_provenance":"cooperative_mod_delivery"}"#,
    );
    acked(
        "m3",
        &b,
        r#"{"provenance":"cooperative_top_level","action_provenance":"cooperative_inbox_display"}"#,
    );
    acked("m4", &a, r#"{"provenance":"cooperative_top_level"}"#);
    // 1 shown now, 2 displayed earlier by a's own mod settlement; b's display of 3 is not a's.
    assert_eq!(advance_through_displayed(&db, &a, &t(), &[1]).unwrap(), 2);
    // 3 shown now; a's explicit ACK of 4 shows nothing.
    assert_eq!(advance_through_displayed(&db, &a, &t(), &[3]).unwrap(), 3);
    // b: its own display ACK of 3 counts, a's ACKs of 2 and 4 do not.
    assert_eq!(advance_through_displayed(&db, &b, &t(), &[1]).unwrap(), 1);
    assert_eq!(advance_through_displayed(&db, &b, &t(), &[2]).unwrap(), 3);
}
