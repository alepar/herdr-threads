use super::*;
use crate::ports::DurableWorkAdmission as WorkAdmission;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::{materialization::advance_work, schema};
use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(999_999)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// Local seats `s1`, `s2`, one remote seat `r1`, thread `t`.
fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    crate::test_support::attach_test_host_to_connection(&db);
    db.execute_batch(&format!("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES('t','{INSTANCE}','topic','goal',0,0,0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES('s1','{INSTANCE}','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts)),('s2','{INSTANCE}','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) VALUES('r1',(SELECT id FROM local_node),'remote','native',1,0,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,'agent',1);\
    ")).unwrap();
    db
}

fn version(db: &Connection, seat: &str) -> Option<i64> {
    db.query_row(
        "SELECT attention_version FROM wake_work WHERE seat_id=?1",
        [seat],
        |r| r.get(0),
    )
    .ok()
}

/// A published send `m` (preparation `p`) to `s1` and `s2`, as a `send_attention` job.
fn publish_send(db: &Connection, suppressed: Option<&str>) {
    let suppressed = suppressed.map_or("NULL".to_owned(), |list| format!("'{list}'"));
    db.execute_batch(&format!("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES('p','{INSTANCE}','actor','op',zeroblob(32),'t',0,0,0,0,0,0,2,2,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0),('p','s2','t',2,300,0);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('{INSTANCE}','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count,suppressed_seats) VALUES('{INSTANCE}','p','m','t',10,1000,1,0,2,0,{suppressed});\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('send','send_attention','p',3);\
    ")).unwrap();
}

fn run_send_job(db: &mut Connection) {
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        db,
        "send",
        WorkAdmission::new(1).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
}

#[test]
fn attention_on_remote_seat_writes_nothing() {
    let db = fixture();
    DeliveryTail::attention(&db, "r1", 1).unwrap();
    DeliveryTail::attention_unless_retired(&db, "r1", 4).unwrap();
    DeliveryTail::invitation(&db, &SeatId::new("r1"), true).unwrap();
    assert_eq!(version(&db, "r1"), None);
    // The local seat takes the same call.
    DeliveryTail::attention(&db, "s1", 1).unwrap();
    DeliveryTail::attention(&db, "s1", 4).unwrap();
    let (bits, v): (i64, i64) = db
        .query_row(
            "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((bits, v), (5, 2));
}

#[test]
fn untimed_invitation_writes_no_attention() {
    let db = fixture();
    DeliveryTail::invitation(&db, &SeatId::new("s1"), false).unwrap();
    assert_eq!(version(&db, "s1"), None);
    DeliveryTail::invitation(&db, &SeatId::new("s1"), true).unwrap();
    assert_eq!(version(&db, "s1"), Some(1));
}

#[test]
fn send_unit_skips_suppressed_seat_but_projects_receipt() {
    let mut db = fixture();
    publish_send(&db, Some("s1"));
    run_send_job(&mut db);
    for seat in ["s1", "s2"] {
        let n: i64 = db
            .query_row(
                "SELECT count(*) FROM receipt_state WHERE message_id='m' AND seat_id=?1",
                [seat],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "receipt for {seat}");
    }
    assert_eq!(version(&db, "s1"), None, "suppressed seat got attention");
    assert_eq!(version(&db, "s2"), Some(1));
}

#[test]
fn suppressed_seat_existing_wake_work_not_bumped() {
    let mut db = fixture();
    db.execute(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES('s1',1,7)",
        [],
    )
    .unwrap();
    publish_send(&db, Some("s1\ns2"));
    run_send_job(&mut db);
    assert_eq!(version(&db, "s1"), Some(7));
    assert_eq!(version(&db, "s2"), None);
}

#[test]
fn unsuppressed_send_bumps_every_local_recipient() {
    let mut db = fixture();
    publish_send(&db, None);
    run_send_job(&mut db);
    assert_eq!(version(&db, "s1"), Some(1));
    assert_eq!(version(&db, "s2"), Some(1));
}

#[test]
fn assert_no_local_delivery_for_remote_green_on_fixtures() {
    let mut db = fixture();
    publish_send(&db, None);
    run_send_job(&mut db);
    DeliveryTail::attention(&db, "r1", 1).unwrap();
    crate::test_support::assert_no_local_delivery_for_remote(&db);
}

#[test]
#[should_panic(expected = "wake_work names remote seats")]
fn assert_no_local_delivery_for_remote_fails_on_a_remote_row() {
    let db = fixture();
    db.execute(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES('r1',1,1)",
        [],
    )
    .unwrap();
    crate::test_support::assert_no_local_delivery_for_remote(&db);
}

#[test]
fn lazy_note_published_skips_remote_wake_on_all_recipient() {
    let db = fixture();
    db.execute_batch(&format!("\
        UPDATE seats SET wake_on='all' WHERE id IN ('s1','r1');\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-1','{INSTANCE}','actor','op',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');\
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-1','msg-1','t','s1'),('prep-1','msg-1','t','r1');\
    ")).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::store::lazy_delivery::note_published(&tx, "prep-1").unwrap();
    tx.commit().unwrap();
    assert_eq!(version(&db, "s1"), Some(1));
    assert_eq!(version(&db, "r1"), None);
}
