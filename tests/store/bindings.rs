use super::*;
use crate::protocol::{ids::*, time::UtcMillis};
use rusqlite::Connection;

pub(crate) fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::store::schema::initialize(&db, || UtcMillis(0)).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON;
        INSERT INTO host_instances(id,created_at,decision_seq) VALUES ('i',0,30);
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',4,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
        INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s','joined',1);
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m1','t',1,'ordinary','b',1,1),('i','m2','t',2,'ordinary','b',2,2);").unwrap();
    db
}
pub(crate) fn placement() -> BindingPlacement {
    BindingPlacement {
        generation: 4,
        target_generation: 1,
        target: "p".into(),
        terminal: "term".into(),
        incarnation: "inc".into(),
        host_boot: "b".into(),
        host_epoch: 1,
        execution: "e".into(),
        provenance: "cooperative_top_level".into(),
        observed_at: UtcMillis(5),
        registered_at: Some(UtcMillis(5)),
    }
}
pub(crate) fn key(session: &str, agent: Option<&str>) -> BindingKey {
    BindingKey {
        seat: SeatId::new("s"),
        harness: "claude".into(),
        native_session: NativeSessionId::new(session),
        agent_id: agent.map(AgentId::new),
    }
}
fn request<'a>(key: &'a BindingKey, place: &'a BindingPlacement) -> EnrollRequest<'a> {
    EnrollRequest {
        key,
        role: BindingRole::TopLevel,
        parent: None,
        lineage: None,
        fingerprint: Some(crate::protocol::authority::LivenessFingerprint { pid: 7, start: 9 }),
        placement: place,
        trigger: EnrollTrigger::Lifecycle,
    }
}
fn enrolled(outcome: EnrollOutcome) -> (BindingRow, bool) {
    match outcome {
        EnrollOutcome::Enrolled {
            binding,
            first_contact,
        } => (binding, first_contact),
        other => panic!("expected Enrolled, got {other:?}"),
    }
}
fn thread() -> ThreadId {
    ThreadId::new("t")
}

#[test]
fn enroll_is_idempotent_per_key_and_seeds_cursors_at_head() {
    let db = db();
    let (k, p) = (key("sess-A", None), placement());
    let (first, first_contact) = enrolled(enroll(&db, &request(&k, &p), UtcMillis(10)).unwrap());
    assert!(first_contact);
    assert_eq!(first.role, BindingRole::TopLevel);
    assert_eq!(first.agent_id, "");
    assert_eq!(first.liveness, Liveness::Connected);
    assert_eq!(
        first.fingerprint,
        Some(crate::protocol::authority::LivenessFingerprint { pid: 7, start: 9 })
    );
    assert_eq!(first.generation, 4);
    assert_eq!(
        read_cursors::cursor_for(&db, &first.binding_id, &thread()).unwrap(),
        Some(2)
    );
    match enroll(&db, &request(&k, &p), UtcMillis(11)).unwrap() {
        EnrollOutcome::Existing(again) => assert_eq!(again.binding_id, first.binding_id),
        other => panic!("expected Existing, got {other:?}"),
    }
    assert_eq!(
        db.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn two_sessions_hold_two_open_bindings_on_one_seat() {
    let db = db();
    let p = placement();
    let (ka, kb) = (key("sess-A", None), key("sess-B", None));
    let (a, _) = enrolled(enroll(&db, &request(&ka, &p), UtcMillis(10)).unwrap());
    let (b, _) = enrolled(enroll(&db, &request(&kb, &p), UtcMillis(10)).unwrap());
    assert_ne!(a.binding_id, b.binding_id);
    let listed = list_connected(&db, &SeatId::new("s")).unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|r| r.binding_id.clone())
            .collect::<Vec<_>>(),
        vec![a.binding_id, b.binding_id]
    );
    assert!(listed[0].ordinal < listed[1].ordinal);
}

#[test]
fn match_claim_ignores_ended_rows() {
    let db = db();
    let (k, p) = (key("sess-A", None), placement());
    let (first, _) = enrolled(enroll(&db, &request(&k, &p), UtcMillis(10)).unwrap());
    assert_eq!(
        match_claim(&db, &k).unwrap().map(|r| r.binding_id),
        Some(first.binding_id.clone())
    );
    db.execute("UPDATE occupant_bindings SET ended_at=9", [])
        .unwrap();
    assert_eq!(match_claim(&db, &k).unwrap(), None);
    let (second, _) = enrolled(enroll(&db, &request(&k, &p), UtcMillis(11)).unwrap());
    assert_ne!(second.binding_id, first.binding_id);
}

#[test]
fn match_claim_distinguishes_agent_ids() {
    let db = db();
    let p = placement();
    let top = key("sess-A", None);
    let sub = key("sess-A", Some("a1"));
    let (t, _) = enrolled(enroll(&db, &request(&top, &p), UtcMillis(10)).unwrap());
    assert_eq!(match_claim(&db, &sub).unwrap(), None);
    let mut req = request(&sub, &p);
    req.role = BindingRole::Subagent;
    req.parent = Some(&t.binding_id);
    let (s, _) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
    assert_eq!(s.agent_id, "a1");
    assert_eq!(
        match_claim(&db, &top).unwrap().map(|r| r.binding_id),
        Some(t.binding_id)
    );
    assert_eq!(
        match_claim(&db, &sub).unwrap().map(|r| r.binding_id),
        Some(s.binding_id)
    );
}

#[test]
fn touch_refreshes_fingerprint_and_never_changes_liveness() {
    let db = db();
    let (k, p) = (key("sess-A", None), placement());
    let (b, _) = enrolled(enroll(&db, &request(&k, &p), UtcMillis(10)).unwrap());
    assert!(
        disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(20)
        )
        .unwrap()
    );
    let fresh = crate::protocol::authority::LivenessFingerprint { pid: 8, start: 99 };
    assert!(touch(&db, &b.binding_id, Some(fresh), UtcMillis(30)).unwrap());
    let row = get(&db, &b.binding_id).unwrap().unwrap();
    assert_eq!(row.liveness, Liveness::Disconnected);
    assert_eq!(row.fingerprint, Some(fresh));
    assert!(touch(&db, &b.binding_id, None, UtcMillis(31)).unwrap());
    assert_eq!(get(&db, &b.binding_id).unwrap().unwrap(), row);
    // An ended binding is not found.
    db.execute("UPDATE occupant_bindings SET ended_at=40", [])
        .unwrap();
    assert!(!touch(&db, &b.binding_id, Some(fresh), UtcMillis(41)).unwrap());
}

#[test]
fn disconnect_records_reason_once() {
    let db = db();
    let (k, p) = (key("sess-A", None), placement());
    let (b, _) = enrolled(enroll(&db, &request(&k, &p), UtcMillis(10)).unwrap());
    assert!(
        disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(20)
        )
        .unwrap()
    );
    let row = get(&db, &b.binding_id).unwrap().unwrap();
    assert_eq!(row.liveness, Liveness::Disconnected);
    assert_eq!(row.disconnect_reason.as_deref(), Some("liveness_end_event"));
    assert_eq!(row.disconnected_at, Some(UtcMillis(20)));
    assert!(
        !disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::ProcessExit,
            UtcMillis(30)
        )
        .unwrap()
    );
    let again = get(&db, &b.binding_id).unwrap().unwrap();
    assert_eq!(
        again.disconnect_reason.as_deref(),
        Some("liveness_end_event")
    );
    assert_eq!(again.disconnected_at, Some(UtcMillis(20)));
    assert!(list_connected(&db, &SeatId::new("s")).unwrap().is_empty());
}

#[test]
fn subagent_records_parent_and_parent_of_reads_it() {
    let db = db();
    let p = placement();
    let top = key("sess-A", None);
    let sub = key("sess-A", Some("a1"));
    let (parent, _) = enrolled(enroll(&db, &request(&top, &p), UtcMillis(10)).unwrap());
    let mut req = request(&sub, &p);
    req.role = BindingRole::Subagent;
    req.parent = Some(&parent.binding_id);
    let (child, _) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
    assert_eq!(child.role, BindingRole::Subagent);
    assert_eq!(
        parent_of(&db, &child.binding_id).unwrap(),
        Some(parent.binding_id.clone())
    );
    assert_eq!(parent_of(&db, &parent.binding_id).unwrap(), None);
    assert_eq!(
        parent_of(&db, &BindingId::new("no-such-binding")).unwrap(),
        None
    );
}

#[test]
fn lineage_successor_copies_predecessor_cursors() {
    let db = db();
    let p = placement();
    let (ka, kb) = (key("sess-A", None), key("sess-B", None));
    let (pred, _) = enrolled(enroll(&db, &request(&ka, &p), UtcMillis(10)).unwrap());
    db.execute("UPDATE binding_cursors SET seq=1", []).unwrap();
    let mut req = request(&kb, &p);
    req.lineage = Some(&pred.binding_id);
    let (succ, first_contact) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
    assert!(!first_contact);
    assert_eq!(succ.lineage_from, Some(pred.binding_id.clone()));
    assert_eq!(
        read_cursors::cursor_for(&db, &succ.binding_id, &thread()).unwrap(),
        Some(1)
    );
}

#[test]
fn enroll_hooks_run_in_order_after_seeding() {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    fn first(
        db: &Connection,
        e: &EnrollEvent<'_>,
        _: UtcMillis,
    ) -> Result<(), crate::protocol::results::ApiError> {
        // cursors already seeded when hooks run
        assert!(
            crate::store::read_cursors::cursor_for(db, &e.binding.binding_id, &ThreadId::new("t"))
                .unwrap()
                .is_some()
        );
        assert!(e.created);
        SEEN.lock().unwrap().push("first");
        Ok(())
    }
    fn second(
        _: &Connection,
        _: &EnrollEvent<'_>,
        _: UtcMillis,
    ) -> Result<(), crate::protocol::results::ApiError> {
        SEEN.lock().unwrap().push("second");
        Ok(())
    }
    let db = db();
    let (k, p) = (key("sess-H", None), placement());
    enroll_with_hooks(
        &db,
        &request(&k, &p),
        UtcMillis(10),
        &[("first", first), ("second", second)],
    )
    .unwrap();
    assert_eq!(*SEEN.lock().unwrap(), vec!["first", "second"]);
    // An existing binding runs no enroll hooks.
    enroll_with_hooks(
        &db,
        &request(&k, &p),
        UtcMillis(11),
        &[("first", first), ("second", second)],
    )
    .unwrap();
    assert_eq!(SEEN.lock().unwrap().len(), 2);
}

#[test]
fn disconnect_reason_strings_match_provenance_vocabulary() {
    assert_eq!(
        DisconnectReason::ProcessExit.as_str(),
        crate::protocol::authority::LIVENESS_PROCESS_EXIT_PROVENANCE
    );
    assert_eq!(
        DisconnectReason::EndEvent.as_str(),
        crate::protocol::authority::LIVENESS_END_EVENT_PROVENANCE
    );
    assert_eq!(DisconnectReason::Migrated.as_str(), "migrated");
    for (reason, text) in [
        (DisconnectReason::Superseded, "superseded"),
        (DisconnectReason::SeatRetired, "seat_retired"),
        (DisconnectReason::SeatUnresolved, "seat_unresolved"),
        (DisconnectReason::Replaced, "replaced"),
    ] {
        assert_eq!(reason.as_str(), text);
    }
}

#[test]
fn fingerprint_start_beyond_i64_is_refused() {
    let db = db();
    let (k, p) = (key("sess-A", None), placement());
    let mut req = request(&k, &p);
    req.fingerprint = Some(crate::protocol::authority::LivenessFingerprint {
        pid: 7,
        start: u64::MAX,
    });
    assert_eq!(
        enroll(&db, &req, UtcMillis(10)).unwrap_err().code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    assert_eq!(match_claim(&db, &k).unwrap(), None);
}

#[test]
fn binding_key_from_claim_reads_seat_harness_session_and_agent() {
    use crate::protocol::authority::{CallerClaim, CallerRole, Harness};
    let claim = CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 4,
        role: CallerRole::Subagent,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("sess-A"),
        execution: ExecutionId::new("e"),
        target: HostTargetId::new("p"),
        agent_id: Some(AgentId::new("a1")),
        liveness: None,
    };
    assert_eq!(BindingKey::from_claim(&claim), key("sess-A", Some("a1")));
}

#[test]
fn binding_live_defaults_to_the_seat_registry() {
    use crate::ports::{ModChannels, NoModChannels};
    struct Live;
    impl ModChannels for Live {
        fn seat_live(&self, _: &SeatId) -> bool {
            true
        }
    }
    let (seat, binding) = (SeatId::new("s"), BindingId::new("b"));
    assert!(Live.binding_live(&seat, &binding));
    assert!(!NoModChannels.binding_live(&seat, &binding));
}
