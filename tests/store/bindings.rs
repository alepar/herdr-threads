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
        rule: FingerprintRule,
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
            rule,
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

    #[test]
    fn plain_call_of_an_unknown_session_enrolls_with_env_provenance() {
        let db = db();
        let r = resolve(
            &db,
            &claim("sess-A", None),
            false,
            FingerprintRule::Required,
        )
        .unwrap();
        let Resolution::Enrolled { ref binding, .. } = r else {
            panic!("expected Enrolled, got {r:?}");
        };
        assert_eq!(binding.provenance, COOPERATIVE_ENV_SESSION_PROVENANCE);
        assert_eq!(binding.liveness, Liveness::Connected);
        assert_eq!(binding.fingerprint, Some(FP));
        assert_eq!(binding.generation, 4);
        // A second plain call of the same session is that binding, not a new one.
        let again = resolve(
            &db,
            &claim("sess-A", None),
            false,
            FingerprintRule::Required,
        )
        .unwrap();
        assert!(
            matches!(again, Resolution::Existing { reconnected: false, ref binding } if binding.binding_id == r.binding().binding_id)
        );
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
    }

    #[test]
    fn lifecycle_enrollment_uses_the_harness_cooperative_provenance() {
        let db = db();
        let r = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
        assert_eq!(r.binding().provenance, COOPERATIVE_TOP_LEVEL_PROVENANCE);
    }

    #[test]
    fn plain_touch_of_a_disconnected_binding_serves_it_without_reconnecting() {
        let db = db();
        let first = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
        let id = first.binding().binding_id.clone();
        disconnect(&db, &id, DisconnectReason::EndEvent, UtcMillis(20)).unwrap();
        let mut c = claim("sess-A", None);
        c.liveness = Some(LivenessFingerprint { pid: 8, start: 99 });
        // Required, yet a touch of an open binding needs and gets no reconnect.
        let r = resolve(&db, &c, false, FingerprintRule::Required).unwrap();
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
        let first = resolve_claim_with_hooks(
            &db,
            &request,
            FingerprintRule::Required,
            UtcMillis(10),
            hooks,
        )
        .unwrap();
        let id = first.binding().binding_id.clone();
        disconnect(&db, &id, DisconnectReason::ProcessExit, UtcMillis(20)).unwrap();
        let again = resolve_claim_with_hooks(
            &db,
            &request,
            FingerprintRule::Required,
            UtcMillis(30),
            hooks,
        )
        .unwrap();
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
            FingerprintRule::Required,
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
        resolve(&db, &c, true, FingerprintRule::Required).unwrap();
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
            FingerprintRule::Required,
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
        let a = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
        let b = resolve(
            &db,
            &other_process("sess-B"),
            true,
            FingerprintRule::Required,
        )
        .unwrap();
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
        let r = resolve(&db, &sub, false, FingerprintRule::Required).unwrap();
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
        let other = resolve(
            &db,
            &claim("sess-A", Some("a2")),
            false,
            FingerprintRule::Required,
        )
        .unwrap();
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
        let r = resolve(&db, &sub, false, FingerprintRule::Transitional).unwrap();
        assert_eq!(r.binding().provenance, COOPERATIVE_ENV_SESSION_PROVENANCE);
    }

    #[test]
    fn attach_subagent_attaches_to_an_existing_parent_without_touching_it() {
        let db = db();
        let parent = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
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
        let err = resolve(&db, &c, false, FingerprintRule::Transitional).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        // And an agent id on a top-level claim.
        let mut c = claim("sess-A", Some("a1"));
        c.role = CallerRole::TopLevel;
        let err = resolve(&db, &c, false, FingerprintRule::Transitional).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
    }

    #[test]
    fn reopen_after_unresolve_keeps_cursors() {
        let db = db();
        let c = claim("sess-A", None);
        let first = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
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
        let r = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
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
        let first = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
        end_open_bindings(
            &db,
            &SeatId::new("s"),
            DisconnectReason::Replaced,
            UtcMillis(20),
        )
        .unwrap();
        let r = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
        assert!(matches!(r, Resolution::Enrolled { .. }), "{r:?}");
        assert_ne!(r.binding().binding_id, first.binding().binding_id);
    }

    #[test]
    fn unresolved_seat_does_not_reopen_before_it_is_resolved_again() {
        let db = db();
        let c = claim("sess-A", None);
        let first = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
        end_open_bindings(
            &db,
            &SeatId::new("s"),
            DisconnectReason::SeatUnresolved,
            UtcMillis(20),
        )
        .unwrap();
        db.execute("UPDATE seats SET state='unresolved'", [])
            .unwrap();
        let r = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
        assert!(matches!(r, Resolution::Enrolled { .. }), "{r:?}");
        assert_ne!(r.binding().binding_id, first.binding().binding_id);
    }

    #[test]
    fn launch_placeholder_session_is_refused() {
        let db = db();
        for rule in [FingerprintRule::Transitional, FingerprintRule::Required] {
            let err = resolve(&db, &claim("launch:x", None), true, rule).unwrap_err();
            assert_eq!(err.code, ErrorCode::SessionClaimRequired, "{rule:?}");
        }
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
    }

    #[test]
    fn required_rule_refuses_fingerprintless_enrollment_but_serves_touches() {
        let db = db();
        let mut bare = claim("sess-A", None);
        bare.liveness = None;
        let err = resolve(&db, &bare, false, FingerprintRule::Required).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
        // A lifecycle claim (re)connects, so it needs one too.
        let err = resolve(&db, &bare, true, FingerprintRule::Required).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
        // Transitional admits the enrollment.
        resolve(&db, &bare, false, FingerprintRule::Transitional).unwrap();
        // An existing open binding is served without a fingerprint under Required.
        let r = resolve(&db, &bare, false, FingerprintRule::Required).unwrap();
        assert!(matches!(r, Resolution::Existing { .. }));
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 1);
        // A lifecycle claim without one is refused again, even for an open binding.
        let err = resolve(&db, &bare, true, FingerprintRule::Required).unwrap_err();
        assert_eq!(err.code, ErrorCode::LivenessFingerprintRequired);
    }

    #[test]
    fn required_rule_refuses_plugin_context_sessions() {
        let db = db();
        let c = claim("plugin_context:x", None);
        let err = resolve(&db, &c, false, FingerprintRule::Required).unwrap_err();
        assert_eq!(err.code, ErrorCode::SessionClaimRequired);
        // The transitional rule still admits what today's clients send.
        resolve(&db, &c, false, FingerprintRule::Transitional).unwrap();
    }

    #[test]
    fn human_claims_are_exempt_from_the_agent_refusals() {
        let db = db();
        let mut c = claim("launch:x", None);
        c.harness = Harness::Human;
        c.liveness = None;
        let r = resolve(&db, &c, true, FingerprintRule::Required).unwrap();
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
        let err = resolve(&db, &c, false, FingerprintRule::Transitional).unwrap_err();
        assert_eq!(err.code, ErrorCode::CallerUnverified);
        assert_eq!(count(&db, "SELECT count(*) FROM occupant_bindings"), 0);
        // The person's lifecycle check-in enrolls; afterwards plain calls are served.
        resolve(&db, &c, true, FingerprintRule::Transitional).unwrap();
        let r = resolve(&db, &c, false, FingerprintRule::Transitional).unwrap();
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
                        FingerprintRule::Required,
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
        let a = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
        let b = resolve(
            &db,
            &other_process("sess-B"),
            true,
            FingerprintRule::Required,
        )
        .unwrap();
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
        let on_a = resolve(&db, &claim("sess-A", None), true, FingerprintRule::Required).unwrap();
        let mut moved = claim("sess-A", None);
        moved.seat = SeatId::new("s2");
        let on_b = resolve(&db, &moved, true, FingerprintRule::Required).unwrap();
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
            FingerprintRule::Required,
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
