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
        let b = resolve(&db, &claim("sess-B", None), true, FingerprintRule::Required).unwrap();
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
        let b = resolve(&db, &claim("sess-B", None), true, FingerprintRule::Required).unwrap();
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
    fn cross_pane_session_enrolls_on_the_new_seat_and_leaves_the_old_row() {
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
        assert_eq!(
            get(&db, &on_a.binding().binding_id).unwrap().unwrap(),
            *on_a.binding()
        );
    }
}
