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
    // Two processes (the same process would supersede, ht-54r.10).
    let mut req_b = request(&kb, &p);
    req_b.fingerprint = Some(crate::protocol::authority::LivenessFingerprint { pid: 8, start: 9 });
    let (b, _) = enrolled(enroll(&db, &req_b, UtcMillis(10)).unwrap());
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

// ---- ht-54r.3: claim resolution ----

mod resolution {
    use super::*;
    use crate::protocol::{
        authority::{
            COOPERATIVE_ENV_SESSION_PROVENANCE, COOPERATIVE_HOOK_AGENT_PROVENANCE,
            COOPERATIVE_TOP_LEVEL_PROVENANCE, CallerClaim, CallerRole, Harness,
            LivenessFingerprint,
        },
        results::ErrorCode,
    };

    const EXEC: &str = "11111111-1111-4111-8111-111111111111";
    const FP: LivenessFingerprint = LivenessFingerprint { pid: 7, start: 9 };

    fn claim(session: &str, agent: Option<&str>) -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 4,
            role: if agent.is_some() {
                CallerRole::Subagent
            } else {
                CallerRole::TopLevel
            },
            harness: Harness::Claude,
            native_session: NativeSessionId::new(session),
            execution: ExecutionId::new(EXEC),
            target: HostTargetId::new("p"),
            agent_id: agent.map(AgentId::new),
            liveness: Some(FP),
        }
    }
    fn resolve(
        db: &Connection,
        claim: &CallerClaim,
        lifecycle: bool,
    ) -> Result<Resolution, crate::protocol::results::ApiError> {
        let place = placement();
        resolve_claim(
            db,
            &ResolveRequest {
                claim,
                lifecycle,
                placement: &place,
                lineage: None,
            },
            UtcMillis(50),
        )
    }
    /// A top-level claim from a different process than `claim` (the same process would supersede).
    fn other_process(session: &str) -> CallerClaim {
        let mut c = claim(session, None);
        c.liveness = Some(LivenessFingerprint { pid: 8, start: 9 });
        c
    }
    fn count(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// The state the V29 migration leaves an agent binding in: open, `migrated`, disconnected for
    /// reason `migrated`, no fingerprint, keyed by its pre-upgrade session.
    fn migrate(db: &Connection, id: &BindingId, session: &str) {
        db.execute(
            "UPDATE occupant_bindings SET migrated=1, migrated_session=?2, liveness='disconnected', disconnected_at=1, disconnect_reason='migrated', liveness_pid=NULL, liveness_start=NULL WHERE binding_id=?1",
            rusqlite::params![id.as_str(), session],
        )
        .unwrap();
    }

    #[test]
    fn migrated_binding_is_readopted_by_a_plain_call_with_its_session() {
        let db = db();
        let id = resolve(&db, &claim("sess-A", None), true)
            .unwrap()
            .binding()
            .binding_id
            .clone();
        migrate(&db, &id, "sess-A");
        db.execute(
            "UPDATE binding_cursors SET seq=1 WHERE binding_id=?1",
            [id.as_str()],
        )
        .unwrap();
        // Mail that arrived while the binding was disconnected: a re-seed at head would skip it.
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m3','t',3,'ordinary','b',3,3)", []).unwrap();
        let r = resolve(&db, &claim("sess-A", None), false).unwrap();
        let Resolution::Existing {
            binding,
            reconnected,
        } = r
        else {
            panic!("expected Existing, got {r:?}");
        };
        assert!(reconnected);
        assert_eq!(binding.binding_id, id, "the same binding, not a new one");
        assert_eq!(binding.liveness, Liveness::Connected);
        assert_eq!(binding.disconnect_reason, None);
        assert_eq!(
            read_cursors::cursor_for(&db, &id, &thread()).unwrap(),
            Some(1),
            "cursors are kept"
        );
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
    }

    #[test]
    fn migrated_binding_with_another_session_is_not_readopted() {
        let db = db();
        let id = resolve(&db, &claim("sess-A", None), true)
            .unwrap()
            .binding()
            .binding_id
            .clone();
        migrate(&db, &id, "sess-A");
        let r = resolve(&db, &other_process("sess-B"), false).unwrap();
        assert!(matches!(r, Resolution::Enrolled { .. }), "{r:?}");
        let old = get(&db, &id).unwrap().unwrap();
        assert_eq!(old.liveness, Liveness::Disconnected);
        assert_eq!(old.disconnect_reason.as_deref(), Some("migrated"));
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 2);
    }

    #[test]
    fn readoption_records_the_fingerprint() {
        let db = db();
        let id = resolve(&db, &claim("sess-A", None), true)
            .unwrap()
            .binding()
            .binding_id
            .clone();
        migrate(&db, &id, "sess-A");
        assert_eq!(get(&db, &id).unwrap().unwrap().fingerprint, None);
        let r = resolve(&db, &claim("sess-A", None), false).unwrap();
        assert_eq!(r.binding().fingerprint, Some(FP));
        assert_eq!(get(&db, &id).unwrap().unwrap().fingerprint, Some(FP));
        // Re-adoption without a fingerprint would leave a connected binding nothing can verify.
        let id2 = resolve(&db, &other_process("sess-C"), true)
            .unwrap()
            .binding()
            .binding_id
            .clone();
        migrate(&db, &id2, "sess-C");
        let mut bare = claim("sess-C", None);
        bare.liveness = None;
        let err = resolve(&db, &bare, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
        assert_eq!(
            get(&db, &id2).unwrap().unwrap().liveness,
            Liveness::Disconnected
        );
    }

    #[test]
    fn placeholder_session_refusal_names_the_fix() {
        let db = db();
        let err = resolve(&db, &claim("plugin_context:x", None), false).unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionClaimRequired);
        assert!(err.detail.contains("restart the agent"), "{}", err.detail);
        assert!(err.detail.contains("herdr-threads setup"), "{}", err.detail);
    }

    #[test]
    fn plain_call_of_an_unknown_session_enrolls_with_env_provenance() {
        let db = db();
        let r = resolve(&db, &claim("sess-A", None), false).unwrap();
        let Resolution::Enrolled { ref binding, .. } = r else {
            panic!("expected Enrolled, got {r:?}");
        };
        assert_eq!(binding.provenance, COOPERATIVE_ENV_SESSION_PROVENANCE);
        assert_eq!(binding.liveness, Liveness::Connected);
        assert_eq!(binding.fingerprint, Some(FP));
        assert_eq!(binding.generation, 4);
        // A second plain call of the same session is that binding, not a new one.
        let again = resolve(&db, &claim("sess-A", None), false).unwrap();
        assert!(
            matches!(again, Resolution::Existing { reconnected: false, ref binding } if binding.binding_id == r.binding().binding_id)
        );
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
    }

    fn hermes_claim(session: &str) -> CallerClaim {
        CallerClaim {
            harness: Harness::Agent(
                crate::harness::registry::builtins()
                    .agent("hermes")
                    .expect("hermes is a builtin adapter"),
            ),
            ..claim(session, None)
        }
    }

    #[test]
    fn hermes_cli_claim_of_a_registered_child_resolves_to_the_subagent_binding() {
        let db = db();
        let parent_claim = hermes_claim("hermes-parent");
        let parent = resolve(&db, &parent_claim, true).unwrap().binding().clone();
        let child_claim =
            registered_child_claim(&parent_claim, &NativeSessionId::new("hermes-child")).unwrap();
        let place = placement();
        attach_subagent(&db, &child_claim, &parent, &place, UtcMillis(50)).unwrap();
        // The child's CLI claim: its own session id, no agent id.
        let cli = resolve(&db, &hermes_claim("hermes-child"), false).unwrap();
        let Resolution::Existing {
            binding,
            reconnected: false,
        } = cli
        else {
            panic!("expected the registered child, got {cli:?}");
        };
        assert_eq!(binding.role, BindingRole::Subagent);
        assert_eq!(binding.agent_id, "hermes-child");
        assert_eq!(binding.parent_binding, Some(parent.binding_id.clone()));
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 2);
        // A lifecycle claim of the child session resolves the same way (it never enrolls a
        // top-level binding for a delegate).
        let lifecycle = resolve(&db, &hermes_claim("hermes-child"), true).unwrap();
        assert_eq!(lifecycle.binding().binding_id, binding.binding_id);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 2);
        // An unrelated Hermes session still enrolls as top-level, and so does the same session id
        // under another harness (the lookup is Hermes-only).
        let other = resolve(&db, &other_process_of(hermes_claim("hermes-other")), false).unwrap();
        assert_eq!(other.binding().role, BindingRole::TopLevel);
        let claude = resolve(&db, &other_process("hermes-child"), false).unwrap();
        assert_eq!(claude.binding().role, BindingRole::TopLevel);
    }

    fn other_process_of(mut claim: CallerClaim) -> CallerClaim {
        claim.liveness = Some(LivenessFingerprint { pid: 9, start: 9 });
        claim
    }

    #[test]
    fn end_event_helpers_disconnect_without_ending_and_ignore_unknown_bindings() {
        let db = db();
        let top_claim = claim("sess-A", None);
        let top = resolve(&db, &top_claim, true).unwrap().binding().clone();
        let sub = resolve(&db, &claim("sess-A", Some("a1")), false)
            .unwrap()
            .binding()
            .clone();
        let top_key = key("sess-A", None);
        let sub_key = key("sess-A", Some("a1"));
        // A top-level key is no subagent target and vice versa.
        assert_eq!(end_event_subagent(&db, &top_key, UtcMillis(60)).unwrap(), 0);
        assert_eq!(end_event_session(&db, &sub_key, UtcMillis(60)).unwrap(), 0);
        assert_eq!(
            end_event_session(&db, &key("sess-Z", None), UtcMillis(60)).unwrap(),
            0
        );
        assert_eq!(end_event_subagent(&db, &sub_key, UtcMillis(60)).unwrap(), 1);
        assert_eq!(end_event_subagent(&db, &sub_key, UtcMillis(61)).unwrap(), 0);
        assert_eq!(end_event_session(&db, &top_key, UtcMillis(62)).unwrap(), 1);
        for binding in [top, sub] {
            let row = get(&db, &binding.binding_id).unwrap().unwrap();
            assert_eq!(row.liveness, Liveness::Disconnected);
            assert_eq!(row.disconnect_reason.as_deref(), Some("liveness_end_event"));
            assert_eq!(row.ended_at, None);
        }
    }

    #[test]
    fn lifecycle_enrollment_uses_the_harness_cooperative_provenance() {
        let db = db();
        let r = resolve(&db, &claim("sess-A", None), true).unwrap();
        assert_eq!(r.binding().provenance, COOPERATIVE_TOP_LEVEL_PROVENANCE);
    }

    #[test]
    fn plain_touch_of_a_disconnected_binding_serves_it_without_reconnecting() {
        let db = db();
        let first = resolve(&db, &claim("sess-A", None), true).unwrap();
        let id = first.binding().binding_id.clone();
        disconnect(&db, &id, DisconnectReason::EndEvent, UtcMillis(20)).unwrap();
        let mut c = claim("sess-A", None);
        c.liveness = Some(LivenessFingerprint { pid: 8, start: 99 });
        // Required, yet a touch of an open binding needs and gets no reconnect.
        let r = resolve(&db, &c, false).unwrap();
        let Resolution::Existing {
            binding,
            reconnected,
        } = r
        else {
            panic!("expected Existing");
        };
        assert!(!reconnected);
        assert_eq!(binding.binding_id, id);
        assert_eq!(binding.liveness, Liveness::Disconnected);
        assert_eq!(
            binding.disconnect_reason.as_deref(),
            Some("liveness_end_event")
        );
        assert_eq!(binding.fingerprint, c.liveness);
    }

    #[test]
    fn lifecycle_reconnects_a_disconnected_binding_and_runs_hooks() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<(bool, EnrollTrigger)>> = Mutex::new(Vec::new());
        fn record(
            _: &Connection,
            e: &EnrollEvent<'_>,
            _: UtcMillis,
        ) -> Result<(), crate::protocol::results::ApiError> {
            SEEN.lock().unwrap().push((e.created, e.trigger));
            Ok(())
        }
        let hooks: &[(&str, EnrollHook)] = &[("record", record)];
        let db = db();
        let c = claim("sess-A", None);
        let place = placement();
        let request = ResolveRequest {
            claim: &c,
            lifecycle: true,
            placement: &place,
            lineage: None,
        };
        let first = resolve_claim_with_hooks(&db, &request, UtcMillis(10), hooks).unwrap();
        let id = first.binding().binding_id.clone();
        disconnect(&db, &id, DisconnectReason::ProcessExit, UtcMillis(20)).unwrap();
        let again = resolve_claim_with_hooks(&db, &request, UtcMillis(30), hooks).unwrap();
        let Resolution::Existing {
            binding,
            reconnected,
        } = again
        else {
            panic!("expected Existing");
        };
        assert!(reconnected);
        assert_eq!(binding.binding_id, id);
        assert_eq!(binding.liveness, Liveness::Connected);
        assert_eq!(binding.disconnect_reason, None);
        assert_eq!(binding.disconnected_at, None);
        assert_eq!(
            *SEEN.lock().unwrap(),
            vec![
                (true, EnrollTrigger::Lifecycle),
                (false, EnrollTrigger::Lifecycle)
            ]
        );
        // A plain call runs no hook.
        resolve_claim_with_hooks(
            &db,
            &ResolveRequest {
                lifecycle: false,
                ..request
            },
            UtcMillis(31),
            hooks,
        )
        .unwrap();
        assert_eq!(SEEN.lock().unwrap().len(), 2);
    }

    #[test]
    fn lifecycle_refreshes_the_placement_of_an_open_binding() {
        let db = db();
        let c = claim("sess-A", None);
        resolve(&db, &c, true).unwrap();
        let mut place = placement();
        place.execution = "e2".into();
        place.host_epoch = 9;
        place.generation = 6;
        let r = resolve_claim(
            &db,
            &ResolveRequest {
                claim: &c,
                lifecycle: true,
                placement: &place,
                lineage: None,
            },
            UtcMillis(60),
        )
        .unwrap();
        assert_eq!(r.binding().generation, 6);
        let (exec, epoch): (String, i64) = db
            .query_row(
                "SELECT execution_id,host_epoch FROM occupant_bindings",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((exec.as_str(), epoch), ("e2", 9));
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
    }

    #[test]
    fn two_sessions_resolve_to_two_open_bindings() {
        let db = db();
        let a = resolve(&db, &claim("sess-A", None), true).unwrap();
        let b = resolve(&db, &other_process("sess-B"), true).unwrap();
        assert_ne!(a.binding().binding_id, b.binding().binding_id);
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL"
            ),
            2
        );
        // B's check-in ended nothing of A's.
        assert_eq!(
            get(&db, &a.binding().binding_id).unwrap().unwrap(),
            *a.binding()
        );
    }

    #[test]
    fn subagent_claim_attaches_to_its_parent_and_enrolls_a_missing_parent() {
        let db = db();
        let sub = claim("sess-A", Some("a1"));
        // No parent yet: it is enrolled first, from the plain call.
        let r = resolve(&db, &sub, false).unwrap();
        let child = r.binding();
        assert_eq!(child.role, BindingRole::Subagent);
        assert_eq!(child.provenance, COOPERATIVE_HOOK_AGENT_PROVENANCE);
        let parent = resolve_parent(&db, &sub).unwrap().unwrap();
        assert_eq!(parent.role, BindingRole::TopLevel);
        assert_eq!(parent.provenance, COOPERATIVE_ENV_SESSION_PROVENANCE);
        assert_eq!(parent.fingerprint, Some(FP));
        assert_eq!(
            parent_of(&db, &child.binding_id).unwrap(),
            Some(parent.binding_id.clone())
        );
        // A second subagent of the same session attaches to the same parent.
        let other = resolve(&db, &claim("sess-A", Some("a2")), false).unwrap();
        assert_eq!(
            parent_of(&db, &other.binding().binding_id).unwrap(),
            Some(parent.binding_id)
        );
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 3);
    }

    #[test]
    fn codex_subagent_uses_env_session_provenance() {
        let db = db();
        let mut sub = claim("sess-A", Some("a1"));
        sub.harness = Harness::Codex;
        let r = resolve(&db, &sub, false).unwrap();
        assert_eq!(r.binding().provenance, COOPERATIVE_ENV_SESSION_PROVENANCE);
    }

    #[test]
    fn attach_subagent_attaches_to_an_existing_parent_without_touching_it() {
        let db = db();
        let parent = resolve(&db, &claim("sess-A", None), true).unwrap();
        let sub = claim("sess-A", Some("a1"));
        let mut place = placement();
        place.provenance = COOPERATIVE_HOOK_AGENT_PROVENANCE.into();
        let out = attach_subagent(&db, &sub, parent.binding(), &place, UtcMillis(40)).unwrap();
        let EnrollOutcome::Enrolled { binding, .. } = out else {
            panic!("expected Enrolled");
        };
        assert_eq!(
            binding.parent_binding,
            Some(parent.binding().binding_id.clone())
        );
        assert_eq!(
            get(&db, &parent.binding().binding_id).unwrap().unwrap(),
            *parent.binding()
        );
        let err = attach_subagent(
            &db,
            &claim("sess-A", None),
            parent.binding(),
            &place,
            UtcMillis(41),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn subagent_role_without_agent_id_is_invalid() {
        let db = db();
        let mut c = claim("sess-A", None);
        c.role = CallerRole::Subagent;
        let err = resolve(&db, &c, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        // And an agent id on a top-level claim.
        let mut c = claim("sess-A", Some("a1"));
        c.role = CallerRole::TopLevel;
        let err = resolve(&db, &c, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
    }

    #[test]
    fn reopen_after_unresolve_keeps_cursors() {
        let db = db();
        let c = claim("sess-A", None);
        let first = resolve(&db, &c, true).unwrap();
        let id = first.binding().binding_id.clone();
        read_cursors::advance(&db, &id, &thread(), 7).unwrap();
        assert_eq!(
            end_open_bindings(
                &db,
                &SeatId::new("s"),
                DisconnectReason::SeatUnresolved,
                UtcMillis(20)
            )
            .unwrap(),
            1
        );
        let r = resolve(&db, &c, true).unwrap();
        let Resolution::Reopened(binding) = r else {
            panic!("expected Reopened, got {r:?}");
        };
        assert_eq!(binding.binding_id, id);
        assert_eq!(binding.ended_at, None);
        assert_eq!(binding.liveness, Liveness::Connected);
        assert_eq!(binding.disconnect_reason, None);
        assert_eq!(
            read_cursors::cursor_for(&db, &id, &thread()).unwrap(),
            Some(7)
        );
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
    }

    #[test]
    fn rows_ended_as_replaced_are_never_reopened() {
        let db = db();
        let c = claim("sess-A", None);
        let first = resolve(&db, &c, true).unwrap();
        end_open_bindings(
            &db,
            &SeatId::new("s"),
            DisconnectReason::Replaced,
            UtcMillis(20),
        )
        .unwrap();
        let r = resolve(&db, &c, true).unwrap();
        assert!(matches!(r, Resolution::Enrolled { .. }), "{r:?}");
        assert_ne!(r.binding().binding_id, first.binding().binding_id);
    }

    #[test]
    fn unresolved_seat_does_not_reopen_before_it_is_resolved_again() {
        let db = db();
        let c = claim("sess-A", None);
        let first = resolve(&db, &c, true).unwrap();
        end_open_bindings(
            &db,
            &SeatId::new("s"),
            DisconnectReason::SeatUnresolved,
            UtcMillis(20),
        )
        .unwrap();
        db.execute("UPDATE seats SET state='unresolved'", [])
            .unwrap();
        let r = resolve(&db, &c, true).unwrap();
        assert!(matches!(r, Resolution::Enrolled { .. }), "{r:?}");
        assert_ne!(r.binding().binding_id, first.binding().binding_id);
    }

    #[test]
    fn launch_placeholder_session_is_refused() {
        let db = db();
        let err = resolve(&db, &claim("launch:x", None), true).unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionClaimRequired);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
    }

    #[test]
    fn fingerprintless_enrollment_is_refused_but_touches_are_served() {
        let db = db();
        let mut bare = claim("sess-A", None);
        bare.liveness = None;
        let err = resolve(&db, &bare, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
        // A lifecycle claim (re)connects, so it needs one too.
        let err = resolve(&db, &bare, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
        // A fingerprinted claim enrolls; an existing open binding is then served without one.
        resolve(&db, &claim("sess-A", None), false).unwrap();
        let r = resolve(&db, &bare, false).unwrap();
        assert!(matches!(r, Resolution::Existing { .. }));
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
        // A lifecycle claim without one is refused again, even for an open binding.
        let err = resolve(&db, &bare, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
    }

    #[test]
    fn plugin_context_sessions_are_refused() {
        let db = db();
        let c = claim("plugin_context:x", None);
        let err = resolve(&db, &c, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionClaimRequired);
    }

    #[test]
    fn human_claims_are_exempt_from_the_agent_refusals() {
        let db = db();
        let mut c = claim("launch:x", None);
        c.harness = Harness::Human;
        c.liveness = None;
        let r = resolve(&db, &c, true).unwrap();
        assert_eq!(
            r.binding().provenance,
            crate::protocol::authority::OPERATOR_HUMAN_PROVENANCE
        );
    }

    #[test]
    fn plain_human_claim_without_a_binding_is_refused_not_enrolled() {
        let db = db();
        let mut c = claim("person", None);
        c.harness = Harness::Human;
        let err = resolve(&db, &c, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::CallerUnverified);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
        // The person's lifecycle check-in enrolls; afterwards plain calls are served.
        resolve(&db, &c, true).unwrap();
        let r = resolve(&db, &c, false).unwrap();
        assert!(matches!(r, Resolution::Existing { .. }));
    }

    #[test]
    fn concurrent_enrollment_creates_exactly_one_binding() {
        let dir = std::env::temp_dir().join(format!("ht-54r3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        {
            let seed = Connection::open(&path).unwrap();
            crate::store::schema::initialize(&seed, || UtcMillis(0)).unwrap();
            seed.execute_batch(
                "INSERT INTO host_instances(id,created_at,decision_seq) VALUES ('i',0,30);
                 INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',4,0);",
            )
            .unwrap();
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (path, barrier) = (path.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let db = Connection::open(&path).unwrap();
                    db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
                    let c = claim("sess-A", None);
                    barrier.wait();
                    loop {
                        match db.execute_batch("BEGIN IMMEDIATE") {
                            Ok(()) => break,
                            Err(rusqlite::Error::SqliteFailure(e, _))
                                if e.code == rusqlite::ErrorCode::DatabaseBusy => {}
                            Err(e) => panic!("{e}"),
                        }
                    }
                    let place = placement();
                    let r = resolve_claim(
                        &db,
                        &ResolveRequest {
                            claim: &c,
                            lifecycle: true,
                            placement: &place,
                            lineage: None,
                        },
                        UtcMillis(50),
                    )
                    .unwrap();
                    db.execute_batch("COMMIT").unwrap();
                    r
                })
            })
            .collect();
        let outcomes: Vec<Resolution> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            outcomes[0].binding().binding_id,
            outcomes[1].binding().binding_id
        );
        let enrolled = outcomes
            .iter()
            .filter(|o| matches!(o, Resolution::Enrolled { .. }))
            .count();
        assert_eq!(enrolled, 1, "{outcomes:?}");
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            count(
                &db,
                "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL"
            ),
            1
        );
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn end_open_bindings_disconnects_then_ends() {
        let db = db();
        let a = resolve(&db, &claim("sess-A", None), true).unwrap();
        let b = resolve(&db, &other_process("sess-B"), true).unwrap();
        disconnect(
            &db,
            &b.binding().binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(15),
        )
        .unwrap();
        assert_eq!(
            end_open_bindings(
                &db,
                &SeatId::new("s"),
                DisconnectReason::SeatRetired,
                UtcMillis(20)
            )
            .unwrap(),
            2
        );
        let a = get(&db, &a.binding().binding_id).unwrap().unwrap();
        assert_eq!(a.ended_at, Some(UtcMillis(20)));
        assert_eq!(a.liveness, Liveness::Disconnected);
        assert_eq!(a.disconnect_reason.as_deref(), Some("seat_retired"));
        // An already disconnected binding keeps its own reason.
        let b = get(&db, &b.binding().binding_id).unwrap().unwrap();
        assert_eq!(b.ended_at, Some(UtcMillis(20)));
        assert_eq!(b.disconnect_reason.as_deref(), Some("liveness_end_event"));
        assert_eq!(
            end_open_bindings(
                &db,
                &SeatId::new("s"),
                DisconnectReason::SeatRetired,
                UtcMillis(21)
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn cross_pane_session_enrolls_on_the_new_seat_and_supersedes_the_old_row() {
        let db = db();
        db.execute_batch(
            "INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s2','i','resolved','native',4,0);",
        )
        .unwrap();
        let on_a = resolve(&db, &claim("sess-A", None), true).unwrap();
        let mut moved = claim("sess-A", None);
        moved.seat = SeatId::new("s2");
        let on_b = resolve(&db, &moved, true).unwrap();
        assert!(matches!(on_b, Resolution::Enrolled { .. }));
        assert_ne!(on_a.binding().binding_id, on_b.binding().binding_id);
        assert_eq!(on_b.binding().seat, SeatId::new("s2"));
        // The same process claiming from another seat supersedes (disconnects) the old row; it
        // stays open, on its own seat.
        let old = get(&db, &on_a.binding().binding_id).unwrap().unwrap();
        assert_eq!(old.seat, SeatId::new("s"));
        assert_eq!(old.ended_at, None);
        assert_eq!(old.liveness, Liveness::Disconnected);
        assert_eq!(old.disconnect_reason.as_deref(), Some("superseded"));
    }
}

// ---- ht-54r.4: human exclusivity and the managed-launch placeholder ----

mod exclusivity {
    use super::*;
    use crate::protocol::authority::{
        CallerClaim, CallerRole, Harness, LivenessFingerprint, MANAGED_LAUNCH_PROVENANCE,
    };

    const EXEC: &str = "11111111-1111-4111-8111-111111111111";

    fn claim(harness: Harness, session: &str, pid: u32) -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 4,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new(session),
            execution: ExecutionId::new(EXEC),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: (harness != Harness::Human).then_some(LivenessFingerprint { pid, start: 9 }),
        }
    }
    fn resolve(db: &Connection, claim: &CallerClaim, lifecycle: bool) -> Resolution {
        let place = placement();
        resolve_claim(
            db,
            &ResolveRequest {
                claim,
                lifecycle,
                placement: &place,
                lineage: None,
            },
            UtcMillis(50),
        )
        .unwrap()
    }
    /// A `managed_launch` placeholder of `harness`, enrolled the way `record_managed_launch` does.
    fn placeholder(db: &Connection, harness: &str, pid: u32) -> BindingRow {
        let place = BindingPlacement {
            provenance: MANAGED_LAUNCH_PROVENANCE.into(),
            registered_at: None,
            ..placement()
        };
        let key = BindingKey {
            seat: SeatId::new("s"),
            harness: harness.into(),
            native_session: NativeSessionId::new(format!("launch:{harness}-{pid}")),
            agent_id: None,
        };
        let mut req = request(&key, &place);
        req.fingerprint = Some(LivenessFingerprint { pid, start: 9 });
        enrolled(enroll(db, &req, UtcMillis(10)).unwrap()).0
    }
    fn row(db: &Connection, id: &BindingRow) -> BindingRow {
        get(db, &id.binding_id).unwrap().unwrap()
    }

    #[test]
    fn agent_claims_end_the_human_binding_through_the_disconnect_transition() {
        for (lifecycle, label) in [(true, "lifecycle enroll"), (false, "plain call")] {
            let db = db();
            let human = resolve(&db, &claim(Harness::Human, "plugin_context:p", 0), true);
            assert_eq!(human.binding().harness, "human");
            let agent = resolve(&db, &claim(Harness::Claude, "sess-A", 7), lifecycle);
            let ended = row(&db, human.binding());
            assert!(ended.ended_at.is_some(), "{label}");
            assert_eq!(ended.liveness, Liveness::Disconnected, "{label}");
            assert_eq!(
                ended.disconnect_reason.as_deref(),
                Some("replaced"),
                "{label}"
            );
            assert_eq!(row(&db, agent.binding()).ended_at, None, "{label}");
        }
    }

    #[test]
    fn a_human_claim_never_ends_agent_bindings() {
        let db = db();
        let agent = resolve(&db, &claim(Harness::Claude, "sess-A", 7), true);
        resolve(&db, &claim(Harness::Human, "plugin_context:p", 0), true);
        let after = row(&db, agent.binding());
        assert_eq!(
            (after.ended_at, after.liveness),
            (None, Liveness::Connected)
        );
    }

    #[test]
    fn end_agent_bindings_ends_connected_and_disconnected_but_not_the_person() {
        let db = db();
        let a = resolve(&db, &claim(Harness::Claude, "sess-A", 7), true);
        let b = resolve(&db, &claim(Harness::Claude, "sess-B", 8), true);
        disconnect(
            &db,
            &b.binding().binding_id,
            DisconnectReason::ProcessExit,
            UtcMillis(20),
        )
        .unwrap();
        let human = resolve(&db, &claim(Harness::Human, "plugin_context:p", 0), true);
        assert_eq!(
            end_agent_bindings(&db, &SeatId::new("s"), UtcMillis(60)).unwrap(),
            2
        );
        let a = row(&db, a.binding());
        assert_eq!(a.ended_at, Some(UtcMillis(60)));
        assert_eq!(a.disconnect_reason.as_deref(), Some("replaced"));
        let b = row(&db, b.binding());
        assert_eq!(b.ended_at, Some(UtcMillis(60)));
        assert_eq!(
            b.disconnect_reason.as_deref(),
            Some("liveness_process_exit")
        );
        assert_eq!(row(&db, human.binding()).ended_at, None);
    }

    #[test]
    fn the_first_agent_check_in_replaces_only_its_harness_placeholders() {
        let db = db();
        let codex_placeholder = placeholder(&db, "codex", 100);
        let claude_placeholder = placeholder(&db, "claude", 101);
        let other_claude = placeholder(&db, "claude", 102);
        let real = resolve(&db, &claim(Harness::Codex, "sess-real", 7), true);
        let agent = resolve(&db, &claim(Harness::Claude, "sess-C", 8), true);
        assert_eq!(real.binding().harness, "codex");
        // The codex check-in ended the codex placeholder only; the claude check-in both claude ones.
        let c = row(&db, &codex_placeholder);
        assert_eq!(c.ended_at, Some(UtcMillis(50)));
        assert_eq!(c.liveness, Liveness::Disconnected);
        assert_eq!(c.disconnect_reason.as_deref(), Some("replaced"));
        for p in [&claude_placeholder, &other_claude] {
            let p = row(&db, p);
            assert_eq!(p.ended_at, Some(UtcMillis(50)));
            assert_eq!(p.disconnect_reason.as_deref(), Some("replaced"));
        }
        assert_eq!(row(&db, real.binding()).ended_at, None);
        assert_eq!(row(&db, agent.binding()).ended_at, None);
    }

    #[test]
    fn a_launch_placeholder_is_untouched_by_other_harnesses_and_by_agent_siblings() {
        let db = db();
        let codex_placeholder = placeholder(&db, "codex", 100);
        let sibling = resolve(&db, &claim(Harness::Claude, "sess-A", 7), true);
        let after = row(&db, &codex_placeholder);
        assert_eq!(
            (after.ended_at, after.liveness),
            (None, Liveness::Connected)
        );
        // A claude subagent or a plain touch is not a top-level lifecycle event of codex either.
        resolve(&db, &claim(Harness::Claude, "sess-A", 7), false);
        assert_eq!(row(&db, &codex_placeholder).ended_at, None);
        assert_eq!(row(&db, sibling.binding()).ended_at, None);
    }

    #[test]
    fn a_plain_call_enrollment_never_adopts_the_placeholder() {
        let db = db();
        let p = placeholder(&db, "claude", 100);
        resolve(&db, &claim(Harness::Claude, "sess-A", 7), false);
        let after = row(&db, &p);
        assert_eq!(
            (after.ended_at, after.liveness),
            (None, Liveness::Connected)
        );
    }

    #[test]
    fn a_placeholder_never_supersedes_a_session_with_the_same_process() {
        let db = db();
        let agent = resolve(&db, &claim(Harness::Claude, "sess-A", 7), true);
        // The placeholder carries the same process fingerprint as the session (the started process
        // is the agent): it is no session, so it must not disconnect the real binding.
        let p = placeholder(&db, "claude", 7);
        assert_eq!(
            p.fingerprint,
            Some(LivenessFingerprint { pid: 7, start: 9 })
        );
        assert_eq!(row(&db, agent.binding()).liveness, Liveness::Connected);
        // The real session's lifecycle check-in replaces the placeholder instead.
        resolve(&db, &claim(Harness::Claude, "sess-A", 7), true);
        assert_eq!(row(&db, &p).ended_at, Some(UtcMillis(50)));
    }
}

// ---- ht-54r.10: binding liveness ----

mod liveness {
    use super::*;
    use crate::protocol::authority::LivenessFingerprint;
    use std::sync::Mutex;

    const FP: LivenessFingerprint = LivenessFingerprint { pid: 7, start: 9 };

    fn enroll_top(db: &Connection, session: &str, fp: LivenessFingerprint, now: i64) -> BindingRow {
        let (k, p) = (key(session, None), placement());
        let mut req = request(&k, &p);
        req.fingerprint = Some(fp);
        enrolled(enroll(db, &req, UtcMillis(now)).unwrap()).0
    }
    fn row(db: &Connection, b: &BindingRow) -> BindingRow {
        get(db, &b.binding_id).unwrap().unwrap()
    }
    fn reason(db: &Connection, b: &BindingRow) -> Option<String> {
        row(db, b).disconnect_reason
    }
    /// A lifecycle check-in of an existing binding, as the resolver does it.
    fn lifecycle_checkin(db: &Connection, b: &BindingRow, fp: LivenessFingerprint, now: i64) {
        connect(
            db,
            &b.binding_id,
            Some(fp),
            ConnectTrigger::Lifecycle,
            UtcMillis(now),
        )
        .unwrap();
        let current = row(db, b);
        run_enroll_hooks(
            db,
            &EnrollEvent {
                binding: &current,
                trigger: EnrollTrigger::Lifecycle,
                created: false,
            },
            UtcMillis(now),
            ENROLL_HOOKS,
        )
        .unwrap();
    }

    #[test]
    fn connect_reconnects_and_refreshes_fingerprint() {
        let db = db();
        let b = enroll_top(&db, "sess-A", FP, 10);
        // Enrolled rows are connected; connect on a connected row changes nothing.
        let fresh = LivenessFingerprint { pid: 8, start: 99 };
        assert!(
            !connect(
                &db,
                &b.binding_id,
                Some(fresh),
                ConnectTrigger::Lifecycle,
                UtcMillis(11)
            )
            .unwrap()
        );
        assert_eq!(row(&db, &b).fingerprint, Some(FP));
        disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(20),
        )
        .unwrap();
        assert!(
            connect(
                &db,
                &b.binding_id,
                Some(fresh),
                ConnectTrigger::Lifecycle,
                UtcMillis(30)
            )
            .unwrap()
        );
        let r = row(&db, &b);
        assert_eq!(r.liveness, Liveness::Connected);
        assert_eq!(r.disconnected_at, None);
        assert_eq!(r.disconnect_reason, None);
        assert_eq!(r.fingerprint, Some(fresh));
        assert!(
            !connect(
                &db,
                &b.binding_id,
                Some(fresh),
                ConnectTrigger::Lifecycle,
                UtcMillis(31)
            )
            .unwrap()
        );
    }

    #[test]
    fn disconnect_runs_hooks_only_on_change() {
        static SEEN: Mutex<Vec<(String, &'static str, i64)>> = Mutex::new(Vec::new());
        fn record(_: &Connection, e: &DisconnectEvent<'_>) -> Result<(), ApiError> {
            SEEN.lock().unwrap().push((
                e.binding.binding_id.as_str().to_owned(),
                e.reason.as_str(),
                e.at.0,
            ));
            // The hook sees the row already changed.
            assert_eq!(e.binding.liveness, Liveness::Disconnected);
            Ok(())
        }
        let db = db();
        let b = enroll_top(&db, "sess-A", FP, 10);
        let hooks: &[(&str, DisconnectHook)] = &[("record", record)];
        assert!(
            disconnect_with_hooks(
                &db,
                &b.binding_id,
                DisconnectReason::SeatRetired,
                UtcMillis(20),
                hooks
            )
            .unwrap()
        );
        assert!(
            !disconnect_with_hooks(
                &db,
                &b.binding_id,
                DisconnectReason::ProcessExit,
                UtcMillis(21),
                hooks
            )
            .unwrap()
        );
        assert_eq!(
            *SEEN.lock().unwrap(),
            vec![(b.binding_id.as_str().to_owned(), "seat_retired", 20)]
        );
    }

    #[test]
    fn disconnect_reasons_round_trip() {
        let db = db();
        let all = [
            (DisconnectReason::Migrated, "migrated"),
            (DisconnectReason::ProcessExit, "liveness_process_exit"),
            (DisconnectReason::EndEvent, "liveness_end_event"),
            (DisconnectReason::Superseded, "superseded"),
            (DisconnectReason::SeatRetired, "seat_retired"),
            (DisconnectReason::SeatUnresolved, "seat_unresolved"),
            (DisconnectReason::Replaced, "replaced"),
        ];
        for (i, (why, text)) in all.into_iter().enumerate() {
            let b = enroll_top(
                &db,
                &format!("sess-{i}"),
                LivenessFingerprint {
                    pid: 100 + i as u32,
                    start: 1,
                },
                10,
            );
            assert!(disconnect(&db, &b.binding_id, why, UtcMillis(20)).unwrap());
            assert_eq!(reason(&db, &b).as_deref(), Some(text));
            assert_eq!(row(&db, &b).disconnected_at, Some(UtcMillis(20)));
        }
    }

    #[test]
    fn seat_active_over_mixed_bindings() {
        let db = db();
        let seat = SeatId::new("s");
        assert!(!seat_active(&db, &seat).unwrap());
        let a = enroll_top(&db, "sess-A", FP, 10);
        let b = enroll_top(&db, "sess-B", LivenessFingerprint { pid: 8, start: 9 }, 11);
        assert!(seat_active(&db, &seat).unwrap());
        disconnect(
            &db,
            &a.binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(20),
        )
        .unwrap();
        assert!(seat_active(&db, &seat).unwrap(), "one connected is enough");
        disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::EndEvent,
            UtcMillis(21),
        )
        .unwrap();
        assert!(!seat_active(&db, &seat).unwrap());
        // An ended connected binding does not count.
        connect(
            &db,
            &a.binding_id,
            Some(FP),
            ConnectTrigger::Lifecycle,
            UtcMillis(22),
        )
        .unwrap();
        assert!(seat_active(&db, &seat).unwrap());
        db.execute("UPDATE occupant_bindings SET ended_at=30", [])
            .unwrap();
        assert!(!seat_active(&db, &seat).unwrap());
    }

    #[test]
    fn seat_active_for_human_binding_follows_the_seat_state() {
        let db = db();
        let seat = SeatId::new("s");
        let mut k = key("person", None);
        k.harness = "human".into();
        let p = placement();
        let mut req = request(&k, &p);
        req.fingerprint = None;
        enrolled(enroll(&db, &req, UtcMillis(10)).unwrap());
        assert!(
            seat_active(&db, &seat).unwrap(),
            "open human binding on a resolved seat"
        );
        db.execute("UPDATE seats SET state='retired', retired_at=40", [])
            .unwrap();
        assert!(!seat_active(&db, &seat).unwrap(), "retired seat");
    }

    #[test]
    fn same_pid_supersede_disconnects_the_prior_top_level_binding_at_enrollment() {
        let db = db();
        let a = enroll_top(&db, "sess-A", FP, 10);
        let b = enroll_top(&db, "sess-B", FP, 11);
        assert_eq!(row(&db, &a).liveness, Liveness::Disconnected);
        assert_eq!(reason(&db, &a).as_deref(), Some("superseded"));
        assert_eq!(row(&db, &a).disconnected_at, Some(UtcMillis(11)));
        assert_eq!(row(&db, &b).liveness, Liveness::Connected);
        // Superseding disconnects; it never ends the binding.
        assert_eq!(row(&db, &a).ended_at, None);
    }

    #[test]
    fn switching_back_to_an_older_session_disconnects_the_one_just_left() {
        let db = db();
        let a = enroll_top(&db, "sess-A", FP, 10);
        let b = enroll_top(&db, "sess-B", FP, 11);
        lifecycle_checkin(&db, &a, FP, 12);
        assert_eq!(row(&db, &a).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).liveness, Liveness::Disconnected);
        assert_eq!(reason(&db, &b).as_deref(), Some("superseded"));
    }

    #[test]
    fn plain_touch_never_supersedes() {
        let db = db();
        let a = enroll_top(&db, "sess-A", FP, 10);
        let b = enroll_top(&db, "sess-B", LivenessFingerprint { pid: 8, start: 9 }, 11);
        // B's process is now reported as A's process by a plain touch.
        assert!(touch(&db, &b.binding_id, Some(FP), UtcMillis(12)).unwrap());
        assert_eq!(row(&db, &a).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).fingerprint, Some(FP));
    }

    #[test]
    fn subagents_never_supersede_nor_are_superseded() {
        let db = db();
        let p = placement();
        let parent = enroll_top(&db, "sess-A", FP, 10);
        let sub_key = key("sess-A", Some("a1"));
        let mut req = request(&sub_key, &p);
        req.role = BindingRole::Subagent;
        req.parent = Some(&parent.binding_id);
        req.fingerprint = Some(FP);
        let (sub, _) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
        assert_eq!(
            row(&db, &parent).liveness,
            Liveness::Connected,
            "subagent did not supersede"
        );
        assert_eq!(row(&db, &sub).liveness, Liveness::Connected);
        // A lifecycle check-in of the subagent does not supersede either.
        let current = row(&db, &sub);
        run_enroll_hooks(
            &db,
            &EnrollEvent {
                binding: &current,
                trigger: EnrollTrigger::Lifecycle,
                created: false,
            },
            UtcMillis(12),
            ENROLL_HOOKS,
        )
        .unwrap();
        assert_eq!(row(&db, &parent).liveness, Liveness::Connected);
        // A new top-level session in the same process supersedes the old top-level binding only.
        let b = enroll_top(&db, "sess-B", FP, 13);
        assert_eq!(row(&db, &parent).liveness, Liveness::Disconnected);
        assert_eq!(row(&db, &sub).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).liveness, Liveness::Connected);
    }

    #[test]
    fn pid_reuse_does_not_supersede() {
        let db = db();
        let a = enroll_top(&db, "sess-A", FP, 10);
        let b = enroll_top(&db, "sess-B", LivenessFingerprint { pid: 7, start: 10 }, 11);
        assert_eq!(row(&db, &a).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).liveness, Liveness::Connected);
    }

    #[test]
    fn different_harness_same_pid_does_not_supersede() {
        let db = db();
        let a = enroll_top(&db, "sess-A", FP, 10);
        let mut k = key("sess-B", None);
        k.harness = "codex".into();
        let p = placement();
        let mut req = request(&k, &p);
        req.fingerprint = Some(FP);
        let (b, _) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
        assert_eq!(row(&db, &a).liveness, Liveness::Connected);
        assert_eq!(row(&db, &b).liveness, Liveness::Connected);
    }

    #[test]
    fn lineage_successor_copies_cursors_before_superseding() {
        let db = db();
        let pred = enroll_top(&db, "sess-A", FP, 10);
        db.execute("UPDATE binding_cursors SET seq=1", []).unwrap();
        let (k, p) = (key("sess-B", None), placement());
        let mut req = request(&k, &p);
        req.lineage = Some(&pred.binding_id);
        req.fingerprint = Some(FP);
        let (succ, first_contact) = enrolled(enroll(&db, &req, UtcMillis(11)).unwrap());
        assert!(!first_contact);
        assert_eq!(
            read_cursors::cursor_for(&db, &succ.binding_id, &thread()).unwrap(),
            Some(1)
        );
        assert_eq!(row(&db, &pred).liveness, Liveness::Disconnected);
        assert_eq!(reason(&db, &pred).as_deref(), Some("superseded"));
    }
}

// ---- ht-54r.23: availability from binding liveness ----

mod availability {
    use super::*;
    use crate::protocol::authority::LivenessFingerprint;

    fn enroll_pid(db: &Connection, session: &str, pid: u32) -> BindingRow {
        let (k, p) = (key(session, None), placement());
        let mut req = request(&k, &p);
        req.fingerprint = Some(LivenessFingerprint { pid, start: 9 });
        enrolled(enroll(db, &req, UtcMillis(10)).unwrap()).0
    }
    /// The seat is placed on an observed target so its connected, registered bindings count.
    fn observable_db() -> Connection {
        let db = db();
        db.execute_batch(
            "UPDATE seats SET target_id='p', target_generation=1, unavailability_open=0 WHERE id='s'; \
             UPDATE host_instances SET host_boot='b', host_epoch=1 WHERE id='i'; \
             INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','p','b',1,1,1,'fresh');",
        )
        .unwrap();
        db
    }
    fn episode(db: &Connection) -> (i64, i64) {
        db.query_row(
            "SELECT unavailability_open, unavailability_episode FROM seats WHERE id='s'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }
    fn availability(db: &Connection) -> SeatAvailability {
        seat_availability(db, &SeatId::new("s")).unwrap()
    }

    #[test]
    fn availability_episode_opens_when_the_last_binding_disconnects() {
        let db = observable_db();
        let a = enroll_pid(&db, "sess-A", 7);
        assert_eq!(availability(&db), SeatAvailability::Available);
        assert_eq!(episode(&db), (0, 1));
        assert!(
            disconnect(
                &db,
                &a.binding_id,
                DisconnectReason::ProcessExit,
                UtcMillis(20)
            )
            .unwrap()
        );
        assert_eq!(episode(&db), (1, 2));
        // Idempotent: a second disconnect changes nothing and opens no second episode.
        assert!(
            !disconnect(
                &db,
                &a.binding_id,
                DisconnectReason::ProcessExit,
                UtcMillis(21)
            )
            .unwrap()
        );
        assert_eq!(episode(&db), (1, 2));
    }

    #[test]
    fn episode_stays_closed_while_another_binding_is_connected() {
        let db = observable_db();
        let a = enroll_pid(&db, "sess-A", 7);
        let b = enroll_pid(&db, "sess-B", 8);
        assert!(
            disconnect(
                &db,
                &a.binding_id,
                DisconnectReason::ProcessExit,
                UtcMillis(20)
            )
            .unwrap()
        );
        assert_eq!(episode(&db), (0, 1));
        assert_eq!(availability(&db), SeatAvailability::Available);
        assert!(
            disconnect(
                &db,
                &b.binding_id,
                DisconnectReason::ProcessExit,
                UtcMillis(21)
            )
            .unwrap()
        );
        assert_eq!(episode(&db), (1, 2));
    }

    #[test]
    fn retired_seat_opens_no_episode_on_disconnect() {
        let db = observable_db();
        let a = enroll_pid(&db, "sess-A", 7);
        db.execute(
            "UPDATE seats SET state='retired', retired_at=15 WHERE id='s'",
            [],
        )
        .unwrap();
        assert!(
            disconnect(
                &db,
                &a.binding_id,
                DisconnectReason::SeatRetired,
                UtcMillis(20)
            )
            .unwrap()
        );
        assert_eq!(episode(&db), (0, 1));
    }

    #[test]
    fn seat_availability_reports_since() {
        let db = observable_db();
        let a = enroll_pid(&db, "sess-A", 7);
        let b = enroll_pid(&db, "sess-B", 8);
        disconnect(
            &db,
            &a.binding_id,
            DisconnectReason::ProcessExit,
            UtcMillis(20),
        )
        .unwrap();
        assert_eq!(availability(&db), SeatAvailability::Available);
        disconnect(
            &db,
            &b.binding_id,
            DisconnectReason::Superseded,
            UtcMillis(31),
        )
        .unwrap();
        assert_eq!(
            availability(&db),
            SeatAvailability::Unavailable {
                since: UtcMillis(31)
            }
        );
    }

    #[test]
    fn lifecycle_reconnect_closes_the_episode() {
        use crate::store::cooperative_checkin_tests::{check_in, claim, fixture, lifecycle};
        let (store, conn, _clock) = fixture();
        check_in(&store, lifecycle(claim(), "op-1")).unwrap();
        let binding: String = conn
            .query_row("SELECT binding_id FROM occupant_bindings", [], |r| r.get(0))
            .unwrap();
        let binding = BindingId::parse(binding).unwrap();
        assert_eq!(
            seat_availability(&conn, &SeatId::new("s")).unwrap(),
            SeatAvailability::Available
        );
        disconnect(
            &conn,
            &binding,
            DisconnectReason::ProcessExit,
            UtcMillis(150),
        )
        .unwrap();
        assert_eq!(episode(&conn).0, 1);
        assert!(matches!(
            seat_availability(&conn, &SeatId::new("s")).unwrap(),
            SeatAvailability::Unavailable { .. }
        ));
        // The same session checks in again: the binding reconnects and the episode closes.
        let mut again = claim();
        again.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
        check_in(&store, lifecycle(again, "op-2")).unwrap();
        let liveness: String = conn
            .query_row(
                "SELECT liveness FROM occupant_bindings WHERE binding_id=?1",
                [binding.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(liveness, "connected");
        assert_eq!(episode(&conn).0, 0);
        assert_eq!(
            seat_availability(&conn, &SeatId::new("s")).unwrap(),
            SeatAvailability::Available
        );
    }
}

// ---- ht-54r.20: lineage validation and linking ----

mod lineage {
    use super::*;
    use crate::protocol::authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint};

    const EXEC: &str = "11111111-1111-4111-8111-111111111111";

    fn claim(session: &str, pid: u32) -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 4,
            role: CallerRole::TopLevel,
            harness: Harness::Claude,
            native_session: NativeSessionId::new(session),
            execution: ExecutionId::new(EXEC),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: Some(LivenessFingerprint { pid, start: 9 }),
        }
    }
    fn resolve(db: &Connection, claim: &CallerClaim, lineage: Option<&BindingId>) -> BindingRow {
        let place = placement();
        resolve_claim(
            db,
            &ResolveRequest {
                claim,
                lifecycle: true,
                placement: &place,
                lineage,
            },
            UtcMillis(50),
        )
        .unwrap()
        .binding()
        .clone()
    }
    fn cursor(db: &Connection, binding: &BindingRow) -> Option<u64> {
        read_cursors::cursor_for(db, &binding.binding_id, &thread()).unwrap()
    }
    fn set_cursor(db: &Connection, binding: &BindingRow, seq: i64) {
        db.execute(
            "UPDATE binding_cursors SET seq=?2 WHERE binding_id=?1",
            rusqlite::params![binding.binding_id.as_str(), seq],
        )
        .unwrap();
    }

    // Kills: a lineage successor seeded from the seat maximum (2, the other process's cursor)
    // instead of its predecessor's cursor (1).
    #[test]
    fn lineage_successor_starts_at_predecessor_cursors() {
        let db = db();
        let predecessor = resolve(&db, &claim("sess-A", 7), None);
        let other = resolve(&db, &claim("sess-X", 8), None);
        set_cursor(&db, &predecessor, 1);
        set_cursor(&db, &other, 2);
        let successor = resolve(&db, &claim("sess-B", 7), Some(&predecessor.binding_id));
        assert_eq!(successor.lineage_from, Some(predecessor.binding_id.clone()));
        assert_eq!(cursor(&db, &successor), Some(1));
        // The same-pid supersede ran after the copy.
        assert_eq!(
            get(&db, &predecessor.binding_id).unwrap().unwrap().liveness,
            Liveness::Disconnected
        );
    }

    // Kills: a lineage hint trusted blindly (successor 1 and lineage_from set) when the
    // successor is another process.
    #[test]
    fn lineage_from_another_process_is_ignored() {
        let db = db();
        let predecessor = resolve(&db, &claim("sess-A", 7), None);
        let other = resolve(&db, &claim("sess-X", 8), None);
        set_cursor(&db, &predecessor, 1);
        set_cursor(&db, &other, 2);
        let stranger = claim("sess-B", 9);
        assert!(!validate_lineage(&db, &stranger, &predecessor.binding_id).unwrap());
        let successor = resolve(&db, &stranger, Some(&predecessor.binding_id));
        assert_eq!(successor.lineage_from, None);
        assert_eq!(cursor(&db, &successor), Some(2), "fresh seed: seat maximum");
        assert_eq!(
            get(&db, &predecessor.binding_id).unwrap().unwrap().liveness,
            Liveness::Connected
        );
    }

    #[test]
    fn validate_lineage_requires_a_top_level_predecessor_of_the_same_seat_and_harness() {
        let db = db();
        let predecessor = resolve(&db, &claim("sess-A", 7), None);
        let same_process = claim("sess-B", 7);
        assert!(validate_lineage(&db, &same_process, &predecessor.binding_id).unwrap());
        // Unknown predecessor.
        assert!(!validate_lineage(&db, &same_process, &BindingId::new("nope")).unwrap());
        // No fingerprint: nothing ties the successor to the process.
        let mut bare = same_process.clone();
        bare.liveness = None;
        assert!(!validate_lineage(&db, &bare, &predecessor.binding_id).unwrap());
        // Another harness.
        let mut other_harness = same_process.clone();
        other_harness.harness = Harness::Codex;
        assert!(!validate_lineage(&db, &other_harness, &predecessor.binding_id).unwrap());
        // Another seat.
        db.execute_batch("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s2','i','resolved','native',4,0);").unwrap();
        let mut other_seat = same_process.clone();
        other_seat.seat = SeatId::new("s2");
        assert!(!validate_lineage(&db, &other_seat, &predecessor.binding_id).unwrap());
        // A subagent predecessor.
        let subagent = {
            let mut sub = claim("sess-A", 7);
            sub.role = CallerRole::Subagent;
            sub.agent_id = Some(AgentId::new("agent-1"));
            resolve(&db, &sub, None)
        };
        assert_eq!(subagent.role, BindingRole::Subagent);
        assert!(!validate_lineage(&db, &same_process, &subagent.binding_id).unwrap());
    }

    // Kills: a link that overwrites a higher successor cursor with the predecessor's lower one.
    #[test]
    fn link_lineage_never_moves_cursors_backwards() {
        let db = db();
        let predecessor = resolve(&db, &claim("sess-A", 7), None);
        let successor = resolve(&db, &claim("sess-B", 8), None);
        set_cursor(&db, &predecessor, 1);
        set_cursor(&db, &successor, 2);
        link_lineage(&db, &successor.binding_id, &predecessor.binding_id).unwrap();
        assert_eq!(cursor(&db, &successor), Some(2), "ahead stays ahead");
        assert_eq!(
            get(&db, &successor.binding_id)
                .unwrap()
                .unwrap()
                .lineage_from,
            Some(predecessor.binding_id.clone())
        );
        // And a successor behind the predecessor catches up.
        set_cursor(&db, &predecessor, 2);
        set_cursor(&db, &successor, 1);
        link_lineage(&db, &successor.binding_id, &predecessor.binding_id).unwrap();
        assert_eq!(cursor(&db, &successor), Some(2));
    }
}

/// ht-54r.25: the side-effect registries are wired. A hook that is written but not registered
/// compiles and passes its own unit test, so the names and order are pinned here.
mod registries {
    use super::*;

    // Kills: an enroll hook dropped from the registry, or the two reordered (the placeholder must
    // go before the same-process supersede decides against the seat's remaining bindings).
    #[test]
    fn enroll_hooks_are_the_placeholder_replace_then_the_same_pid_supersede() {
        let names: Vec<&str> = ENROLL_HOOKS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["launch_placeholder_replace", "same_pid_supersede"]);
    }

    // Kills: a disconnect side effect (mod channel release, availability recompute) that exists but
    // is not registered, so a disconnect leaves a stale channel or availability.
    #[test]
    fn disconnect_hooks_release_the_mod_channel_and_recompute_availability() {
        let names: Vec<&str> = DISCONNECT_HOOKS.iter().map(|(name, _)| *name).collect();
        assert!(names.contains(&"mod_channel_release"), "{names:?}");
        assert!(names.contains(&"availability_recompute"), "{names:?}");
        assert_eq!(names.len(), 2, "an unlisted hook needs a test: {names:?}");
    }
}
