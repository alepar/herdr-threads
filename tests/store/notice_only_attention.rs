//! Wake survey D1: when a warning alone explains an attention, so that the
//! attention may name its notice instead of `run herdr-threads inbox`.
use super::notice_only_attention;
use crate::{
    protocol::time::{Clock, MonoInstant, UtcMillis},
    store::connection::StoreContext,
};
use rusqlite::{Connection, params};
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

struct Fixture {
    db: Connection,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

/// Seats `s` (the reader, bindings `a` and `b`) and `c`, one thread.
fn fixture() -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory =
        crate::test_support::scratch::temp_path(format!("notice-only-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let db = StoreContext::new(directory.join("store.db"), clock)
        .with_test_host()
        .open_writer()
        .unwrap();
    db.execute_batch("UPDATE herdr_hosts SET host_boot='boot',host_epoch=1; UPDATE local_node SET decision_seq=100;")
        .unwrap();
    for (seat, target) in [("s", "p"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES(?1,?2,'resolved','native',?3,1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", params![seat, INSTANCE, target]).unwrap();
    }
    db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','boot',1,0,0,'fresh','term','inc','coherent_enumeration',1)", []).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled) VALUES('t',?1,'topic','goal',0,0,1,0)", [INSTANCE]).unwrap();
    for name in ["a", "b"] {
        db.execute("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES(?1,'s',1,0,'p','term','inc','boot',1,'claude',?1,?1,'cooperative_env_session',0,0)", [name]).unwrap();
    }
    Fixture { db, directory }
}

impl Fixture {
    /// An open condition of `kind` about `affected`, opened by warning `id`
    /// (`origin` names the inviter of an invitation condition), projected to
    /// `s` as notice ordinal `n`.
    fn condition(&self, id: &str, kind: &str, affected: &str, origin: Option<&str>, n: i64) {
        self.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES(?1,?2,'t',?3,'warn','{}',0,?3,1)", params![id, INSTANCE, 10 + n]).unwrap();
        self.db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq,invitation_originator_instance_id,invitation_originator_seat_id) VALUES(?1,'t',?2,?3,?2,?4,?5,?6)", params![kind, id, affected, 10 + n, origin.map(|_| INSTANCE), origin]).unwrap();
        self.db.execute("INSERT INTO digest_programmatic_warnings(ordinal,seat_id,warning_id,thread_id,event_seq,event_offset) VALUES(?1,'s',?2,'t',?3,1)", params![n, id, 10 + n]).unwrap();
    }
    /// `binding` was offered notices through ordinal `n`.
    fn offered(&self, binding: &str, n: i64) {
        self.db.execute("INSERT INTO digest_notice_offer(binding_id,seat_id,offered_ordinal) VALUES(?1,'s',?2) ON CONFLICT(binding_id) DO UPDATE SET offered_ordinal=excluded.offered_ordinal", params![binding, n]).unwrap();
    }
    fn only(&self, binding: &str, warning: &str) -> bool {
        notice_only_attention(&self.db, "s", binding, warning).unwrap()
    }
}

#[test]
fn an_offered_receipt_origin_notice_alone_explains_the_attention() {
    let f = fixture();
    f.condition("w1", "receipt", "c", None, 1);
    f.offered("a", 1);
    assert!(f.only("a", "w1"));
}

#[test]
fn a_notice_not_yet_offered_to_this_binding_keeps_the_inbox_pointer() {
    let f = fixture();
    f.condition("w1", "receipt", "c", None, 1);
    assert!(!f.only("a", "w1"), "never offered");
    f.offered("b", 1);
    assert!(!f.only("a", "w1"), "offered only to a sibling binding");
}

#[test]
fn an_offered_invitation_origin_notice_alone_explains_the_attention() {
    let f = fixture();
    f.condition("w1", "invitation", "c", Some("s"), 1);
    f.offered("a", 1);
    assert!(f.only("a", "w1"));
}

#[test]
fn the_affected_seat_or_a_closed_condition_keeps_the_inbox_pointer() {
    let f = fixture();
    f.condition("w1", "receipt", "s", None, 1);
    f.offered("a", 1);
    assert!(!f.only("a", "w1"), "the seat is the affected seat");
    f.condition("w2", "receipt", "c", None, 2);
    f.offered("a", 2);
    f.db.execute("UPDATE warning_conditions SET clear_warning_id='w2-clear',cleared_seq=50 WHERE open_warning_id='w2'", []).unwrap();
    assert!(!f.only("a", "w2"), "the condition was cleared");
}

// F1 (main review): an origin witness hides nothing the seat owes.
#[test]
fn a_seat_overdue_itself_or_owing_an_ack_keeps_the_inbox_pointer() {
    let f = fixture();
    f.condition("w1", "receipt", "c", None, 1);
    f.offered("a", 1);
    assert!(f.only("a", "w1"));
    f.condition("w2", "receipt", "s", None, 2);
    assert!(!f.only("a", "w1"), "the seat is overdue on w2");

    let f = fixture();
    f.condition("w1", "receipt", "c", None, 1);
    f.offered("a", 1);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('m',?1,'t',40,'ordinary','owed',0,40)", [INSTANCE]).unwrap();
    f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES('m','t','s','pending',1,0,999999)", []).unwrap();
    assert!(!f.only("a", "w1"), "the seat owes an ACK");
}

#[test]
fn the_notice_marker_is_one_line_of_twenty_six_bytes_plus_the_id() {
    let text = crate::protocol::watch::attention_notice_marker("enSGVM8o6irv");
    assert_eq!(text, "herdr-threads: see notice enSGVM8o6irv");
    assert_eq!(text.len(), 26 + "enSGVM8o6irv".len());
    assert!(!text.contains('\n'));
}
