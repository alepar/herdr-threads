use herdr_threads::{protocol::time::UtcMillis, store::schema};
use rusqlite::Connection;

pub(super) fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0);")
        .unwrap();
    db
}

#[test]
fn archival_schema_new_channel_gets_dirty_grace_without_historical_backdating() {
    let db = fixture();
    db.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',-999999,-999999)").unwrap();
    let row = db.query_row("SELECT activity_revision,quiet_mono,due_mono FROM channel_archival WHERE thread_id='t'", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,i64>(2)?))).expect("new channels need explicit fresh archival grace");
    assert_eq!(row, (1, None, 0));
}

#[test]
fn archival_schema_channel_activity_invalidates_certificate() {
    let db = fixture();
    db.execute_batch("INSERT INTO archival_instances(instance_id,runtime_boot) VALUES('i','boot'); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0); UPDATE channel_archival SET quiet_mono=10,scan_revision=3 WHERE thread_id='t'; UPDATE threads SET topic='changed' WHERE id='t';").unwrap();
    let row = db
        .query_row(
            "SELECT quiet_mono,scan_revision FROM channel_archival WHERE thread_id='t'",
            [],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .unwrap();
    assert_eq!(row, (None, None));
    assert!(
        db.query_row(
            "SELECT mutation_revision FROM archival_instances WHERE instance_id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > 0
    );
}

use herdr_threads::store::archival::{self, Runtime};
fn runtime(at: i64) -> Runtime {
    Runtime {
        boot: "boot".into(),
        mono: at,
        utc: UtcMillis(at),
        after_ms: 3_600_000,
        host_generation: 0,
        coherent: true,
        valid_until_mono: None,
        legacy_source: Some("covered-source".into()),
    }
}
pub(super) fn thread(db: &Connection) {
    db.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',-999999999,-999999999)").unwrap();
}
fn archived(db: &Connection) -> bool {
    db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn tick(db: &mut Connection, rt: &Runtime) {
    // Continuations are independently bounded transactions. Cap protects tests
    // against an accidental worker busy loop.
    for _ in 0..20 {
        if !archival::advance(db, "i", rt).unwrap().has_more {
            return;
        }
    }
    panic!("archival continuation did not yield");
}
#[test]
fn archival_all_left_waits_full_monotonic_grace_and_records_seatless_event() {
    let mut db = fixture();
    thread(&db);
    for at in (0..3_600_000).step_by(60_000) {
        tick(&mut db, &runtime(at));
        assert!(!archived(&db));
    }
    tick(&mut db, &runtime(3_600_000));
    assert!(
        archived(&db),
        "all-left quiet channel should archive after fresh grace"
    );
    let (actor, payload): (Option<String>, String) = db
        .query_row(
            "SELECT actor_seat_id,event_json FROM messages WHERE thread_id='t'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(actor.is_none());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&payload).unwrap()["provenance"],
        "daemon_lifecycle"
    );
}
#[test]
fn archival_forward_clock_jump_restart_outage_and_partial_import_never_age_into_permission() {
    for interruption in ["jump", "restart", "outage", "coverage"] {
        let mut db = fixture();
        thread(&db);
        let mut initial = runtime(0);
        initial.after_ms = 60_000;
        tick(&mut db, &initial);
        let mut rt = runtime(60_000);
        rt.after_ms = 60_000;
        match interruption {
            "jump" => rt.utc = UtcMillis(10_000_000),
            "restart" => rt.boot = "next".into(),
            "outage" => rt.coherent = false,
            _ => rt.legacy_source = None,
        }
        tick(&mut db, &rt);
        assert!(!archived(&db));
    }
}
#[test]
fn archival_reopen_and_new_activity_each_require_fresh_grace() {
    let mut db = fixture();
    thread(&db);
    let mut rt = runtime(0);
    rt.after_ms = 60_000;
    tick(&mut db, &rt);
    rt.mono = 60_000;
    rt.utc = UtcMillis(rt.mono);
    tick(&mut db, &rt);
    assert!(archived(&db));
    db.execute("UPDATE threads SET archived=0 WHERE id='t'", [])
        .unwrap();
    tick(&mut db, &rt);
    assert!(!archived(&db), "reopen must not reuse old quiet grace");
    rt.mono = 90_000;
    rt.utc = UtcMillis(rt.mono);
    db.execute("UPDATE threads SET topic='new work' WHERE id='t'", [])
        .unwrap();
    tick(&mut db, &rt);
    rt.mono = 120_000;
    rt.utc = UtcMillis(rt.mono);
    tick(&mut db, &rt);
    assert!(!archived(&db), "new activity must restart reopened grace");
    rt.mono = 150_000;
    rt.utc = UtcMillis(rt.mono);
    tick(&mut db, &rt);
    assert!(archived(&db));
}
#[test]
fn archival_disabled_policy_never_archives() {
    let mut db = fixture();
    thread(&db);
    let mut rt = runtime(0);
    rt.after_ms = 0;
    tick(&mut db, &rt);
    rt.mono = 4_000_000;
    rt.utc = UtcMillis(rt.mono);
    tick(&mut db, &rt);
    assert!(!archived(&db));
}

pub(super) fn joined_agent(db: &Connection) {
    db.execute_batch("UPDATE host_instances SET host_boot='b',host_epoch=1 WHERE id='i';
INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('s','i','resolved','native','p',1,0,0);
INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s',1,'p','b',1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0);
INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch,ui_state) VALUES('i','p','b',1,0,1,'fresh',0,'term','inc','native_current_target',1,'idle');
INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('t','s','joined',0);").unwrap();
}
pub(super) fn sample(
    at: i64,
    ui: herdr_threads::ports::HostUiState,
) -> archival::ComposerObservation {
    use herdr_threads::{
        ports::*,
        protocol::{authority::Harness, ids::*, time::MonoInstant},
    };
    archival::ComposerObservation(HostObservation {
        target: HostTargetId::new("p"),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 0,
        observed_at_utc: UtcMillis(at),
        observed_at_mono: MonoInstant(at as u64),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: Some(NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("not-execution-proof"),
            execution: ExecutionId::new("not-execution-proof"),
            is_top_level: false,
        }),
        ui,
        focused: false,
        terminal: Some(TerminalId::new("term")),
        occupancy: StructuralOccupancy::Unknown,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("read-{at}")),
        connection_epoch: 1,
        observation_sequence: (at / 60_000 + 2) as u64,
        started_at_mono: MonoInstant(at as u64),
        completed_at_mono: MonoInstant(at as u64),
    })
}
fn authority_snapshot(db: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "seats",
        "occupant_bindings",
        "memberships",
        "membership_intervals",
        "receipts",
        "receipt_state",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns).map(|column| row.get(column)).collect()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}
#[test]
fn archival_joined_agent_needs_repeated_composer_reads_then_preserves_membership_and_binding() {
    use herdr_threads::ports::HostUiState;
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    ordinary(&db);
    db.execute_batch("UPDATE threads SET next_sequence=2 WHERE id='t'; INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES('m','t','s','acked',300000,0,300000,'s',1,'cooperative_top_level',1)").unwrap();
    let before = authority_snapshot(&db);
    for at in (0..=3_600_000).step_by(60_000) {
        // Runtime admission precedes capturing evidence; final scan follows it.
        archival::advance(&db, "i", &runtime(at)).unwrap();
        let ticket = archival::observation_ticket(&db, "i", "s", &runtime(at))
            .unwrap()
            .unwrap();
        archival::record_sample(
            &mut db,
            &ticket,
            &runtime(at),
            &sample(at, HostUiState::Idle),
        )
        .unwrap();
        tick(&mut db, &runtime(at));
        if at < 3_600_000 {
            assert!(!archived(&db));
        }
    }
    assert!(
        archived(&db),
        "continuous composer-aware idle evidence must permit archival"
    );
    assert_eq!(
        authority_snapshot(&db),
        before,
        "successful archive must preserve exact identity, membership and settled receipt rows"
    );
}

#[test]
fn archival_current_structure_change_after_member_scan_invalidates_final_decision() {
    use herdr_threads::ports::HostUiState;
    for race in ["current", "missing_snapshot", "matching_snapshot"] {
        let mut db = fixture();
        thread(&db);
        joined_agent(&db);
        let mut rt = runtime(0);
        rt.after_ms = 60_000;
        archival::advance(&db, "i", &rt).unwrap();
        let ticket = archival::observation_ticket(&db, "i", "s", &rt)
            .unwrap()
            .unwrap();
        archival::record_sample(&mut db, &ticket, &rt, &sample(0, HostUiState::Idle)).unwrap();
        rt.mono = 60_000;
        rt.utc = UtcMillis(60_000);
        let ticket = archival::observation_ticket(&db, "i", "s", &rt)
            .unwrap()
            .unwrap();
        archival::record_sample(&mut db, &ticket, &rt, &sample(60_000, HostUiState::Idle)).unwrap();
        archival::advance(&db, "i", &rt).unwrap(); // capture certificate
        archival::advance(&db, "i", &rt).unwrap(); // scan member
        assert_eq!(
            db.query_row(
                "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        if race == "current" {
            db.execute_batch("UPDATE observed_targets SET terminal_id='different',observation_sequence=99 WHERE target_id='p'").unwrap();
        } else {
            db.execute_batch("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES('snap','i','b',1,100,'inc',0,'published',0,0,60000)").unwrap();
            if race == "matching_snapshot" {
                db.execute_batch("UPDATE snapshot_generations SET expected_targets=1,staged_targets=1 WHERE id='snap'; INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,connection_epoch,incarnation_source_kind,occupancy,ui_state,observed_at) VALUES('snap','p','term',0,100,1,'coherent_enumeration','unknown','unknown',60000)").unwrap();
            }
            db.execute_batch("UPDATE host_instances SET active_snapshot_id='snap',observation_sequence=100 WHERE id='i'").unwrap();
        }
        tick(&mut db, &rt);
        assert_eq!(
            archived(&db),
            race == "matching_snapshot",
            "canonical identity race: {race}"
        );
    }
}
#[test]
fn channel_archival_production_store_admission_cancels_without_mutation() {
    use herdr_threads::{
        ports::StorePort,
        protocol::time::{CallBudget, Cancellation, MonoInstant},
    };
    let fixture = super::handoff_fences::store_fixture();
    let rt = herdr_threads::store::archival::Runtime {
        boot: "worker".into(),
        mono: 0,
        utc: UtcMillis(0),
        after_ms: 3_600_000,
        host_generation: 0,
        coherent: true,
        valid_until_mono: None,
        legacy_source: Some("source".into()),
    };
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation,
    };
    assert_eq!(
        fixture
            .store
            .archival_pass(&rt, &[], &budget)
            .unwrap_err()
            .code,
        herdr_threads::protocol::results::ErrorCode::Cancelled
    );
    assert_eq!(
        fixture
            .db
            .query_row("SELECT count(*) FROM archival_instances", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    fixture.store.archival_pass(&rt, &[], &budget).unwrap();
    assert_eq!(
        fixture
            .db
            .query_row("SELECT last_mono FROM archival_instances", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        100,
        "production time is sampled after admission"
    );
    let work = fixture.store.archival_next(&rt, &budget).unwrap();
    let ticket = work.ticket.expect("registered joined seat selected");
    assert!(
        !fixture
            .store
            .archival_sample(&rt, &ticket, None, &budget)
            .unwrap()
    );
    assert!(
        fixture
            .db
            .query_row(
                "SELECT idle_mono IS NULL FROM seat_archival WHERE seat_id='s'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
}
fn preparation(db: &Connection, id: &str, status: &str) {
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES(?1,'i','seat:s',?1,zeroblob(32),'t',0,0,0,0,0,0,0,?2)",rusqlite::params![id,status]).unwrap();
}
fn ordinary(db: &Connection) {
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,decision_seq,body,decision_at,author_kind) VALUES('m','i','t',1,'ordinary','s',1,'work',0,'native')").unwrap();
}
#[test]
fn archival_protected_occupants_and_work_remain_unchanged() {
    use herdr_threads::ports::HostUiState;
    for case in [
        "human",
        "unregistered",
        "unbound",
        "unresolved",
        "held",
        "unknown",
        "draft",
        "working",
        "approval",
        "invitation",
        "physical_receipt",
        "logical_receipt",
        "preparation",
        "service_preparation",
        "catchup",
        "summary_lease",
        "managed",
        "requirement",
        "live_handoff",
    ] {
        let mut db = fixture();
        thread(&db);
        joined_agent(&db);
        let mut ui = HostUiState::Idle;
        match case {
            "human" => {
                db.execute_batch("UPDATE occupant_bindings SET harness='human'")
                    .unwrap();
            }
            "unregistered" => {
                db.execute_batch("UPDATE occupant_bindings SET registered_at=NULL")
                    .unwrap();
            }
            "unbound" => {
                db.execute_batch("UPDATE occupant_bindings SET ended_at=1")
                    .unwrap();
            }
            "unresolved" => {
                db.execute_batch("UPDATE seats SET state='unresolved'")
                    .unwrap();
            }
            "held" => {
                db.execute_batch("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'repair')").unwrap();
            }
            "unknown" => ui = HostUiState::Unknown,
            "draft" => ui = HostUiState::HumanInput,
            "working" => ui = HostUiState::ActiveTurn,
            "approval" => ui = HostUiState::ApprovalOrQuestion,
            "invitation" | "requirement" => {
                db.execute_batch("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('q','i','resolved','native','q',1,0,0); INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES('inv','t','q',1,'pending',1,0,300000,300000)").unwrap();
                if case == "requirement" {
                    db.execute_batch("INSERT INTO service_authors(id,instance_id,created_at) VALUES('service','i',0); UPDATE threads SET managed_owner_author_id='service'; INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES('req','t','q','service','inv','pending',1,0)").unwrap();
                }
            }
            "physical_receipt" => {
                ordinary(&db);
                db.execute_batch("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES('m','t','s','pending',300000,0,300000)").unwrap();
            }
            "logical_receipt" => {
                ordinary(&db);
                preparation(&db, "prep", "sealed");
                db.execute_batch("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('prep','t','s',1,300000,1); INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep','m','i','t',1,0,1,0,1,0)").unwrap();
            }
            "preparation" => preparation(&db, "prep", "building"),
            "service_preparation" => {
                db.execute_batch("INSERT INTO service_authors(id,instance_id,created_at) VALUES('service','i',0); INSERT INTO service_notification_preparations(id,instance_id,author_id,operation_key,digest,thread_id,membership_revision,lifecycle_revision,interval_high_water,requirement_high_water,status) VALUES('prep','i','service','op',zeroblob(32),'t',0,0,0,0,'building')").unwrap();
            }
            "catchup" => {
                db.execute_batch("INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,state) VALUES('s','t',0,1,'execution',0,'active')").unwrap();
            }
            "summary_lease" => {
                db.execute_batch("INSERT INTO summary_jobs(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,lease_seat_id,lease_token,lease_until,created_at) VALUES('job','i','t','v1',0,0,1,2,'s','token',300000,0)").unwrap();
            }
            "managed" => {
                db.execute_batch("INSERT INTO service_authors(id,instance_id,created_at) VALUES('service','i',0); UPDATE threads SET managed_owner_author_id='service'").unwrap();
            }
            "live_handoff" => {
                let tx = db.transaction().unwrap();
                herdr_threads::store::handoff::begin_pending(
                    &tx,
                    &super::handoff_fences::identity(),
                    UtcMillis(0),
                )
                .unwrap();
                tx.commit().unwrap();
            }
            _ => unreachable!(),
        }
        let before = authority_snapshot(&db);
        for at in [0, 60_000, 120_000] {
            let mut rt = runtime(at);
            rt.after_ms = 60_000;
            archival::advance(&db, "i", &rt).unwrap();
            if let Some(ticket) = archival::observation_ticket(&db, "i", "s", &rt).unwrap() {
                archival::record_sample(&mut db, &ticket, &rt, &sample(at, ui)).unwrap();
            }
            tick(&mut db, &rt);
            assert!(!archived(&db), "{case} must prevent archival");
        }
        let after = authority_snapshot(&db);
        assert_eq!(before, after, "{case} mutated caller state");
        if matches!(case, "physical_receipt" | "logical_receipt") {
            assert_eq!(
                herdr_threads::store::effective::effective_receipt(&db, "m", "s")
                    .unwrap()
                    .unwrap()
                    .state,
                herdr_threads::store::effective::EffectiveReceiptState::Pending
            );
        }
    }
}
#[test]
fn archival_deciding_clock_expiry_rolls_back_provisional_work() {
    use herdr_threads::{
        ports::StorePort,
        protocol::time::{CallBudget, MonoInstant},
    };
    let fixture = super::handoff_fences::store_fixture();
    let mut rt = runtime(0);
    rt.valid_until_mono = Some(99);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    assert_eq!(
        fixture
            .store
            .archival_pass(&rt, &[], &budget)
            .unwrap_err()
            .code,
        herdr_threads::protocol::results::ErrorCode::StaleHostObservation
    );
    assert_eq!(
        fixture
            .db
            .query_row("SELECT count(*) FROM archival_instances", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn archival_preparation_history_pages_have_flat_vm_cost_and_never_imply_exhaustion() {
    use herdr_threads::test_support::isolation::{CostCounter, count_vm_units};
    fn measure(size: usize) -> u64 {
        let db = fixture();
        thread(&db);
        db.execute_batch("BEGIN").unwrap();
        for n in 0..size {
            preparation(&db, &format!("prep-{n:08}"), "discarded");
        }
        db.execute_batch("COMMIT").unwrap();
        let mut rt = runtime(0);
        rt.after_ms = 60_000;
        archival::advance(&db, "i", &rt).unwrap();
        rt.mono = 60_000;
        rt.utc = UtcMillis(60_000);
        for _ in 0..8 {
            let phase = db
                .query_row(
                    "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
            if phase == 3 {
                break;
            }
            archival::advance(&db, "i", &rt).unwrap();
        }
        assert_eq!(
            db.query_row(
                "SELECT scan_phase FROM channel_archival WHERE thread_id='t'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        let counter = CostCounter::default();
        let progress = count_vm_units(&db, &counter, || archival::advance(&db, "i", &rt).unwrap());
        assert_eq!(progress.visited, archival::PAGE);
        assert!(progress.has_more);
        assert!(!archived(&db));
        let plan=db.prepare("EXPLAIN QUERY PLAN SELECT id,status FROM send_preparations WHERE thread_id='t' AND id>'' ORDER BY id LIMIT 32").unwrap().query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join("; ");
        assert!(plan.contains("archival_preparations_page"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        counter.units()
    }
    let small = measure(64);
    let large = measure(10_000);
    eprintln!("archival preparation page VM units (10 instructions): {small} -> {large}");
    assert!(small > 0);
    assert!(
        large <= small + 30,
        "history grew one bounded turn: {small} -> {large}"
    );
}
#[test]
fn archival_delayed_sample_cannot_erase_accountable_activity_or_count_duplicates() {
    use herdr_threads::ports::HostUiState;
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let rt = runtime(0);
    archival::advance(&db, "i", &rt).unwrap();
    let ticket = archival::observation_ticket(&db, "i", "s", &rt)
        .unwrap()
        .unwrap();
    db.execute_batch("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES('seat:s','accepted-action',zeroblob(32),'{}',0)").unwrap();
    assert!(
        !archival::record_sample(&mut db, &ticket, &rt, &sample(0, HostUiState::Idle)).unwrap()
    );
    let fresh = archival::observation_ticket(&db, "i", "s", &rt)
        .unwrap()
        .unwrap();
    assert!(archival::record_sample(&mut db, &fresh, &rt, &sample(0, HostUiState::Idle)).unwrap());
    assert!(!archival::record_sample(&mut db, &fresh, &rt, &sample(0, HostUiState::Idle)).unwrap());
    assert_eq!(
        db.query_row(
            "SELECT samples FROM seat_archival WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn archival_restart_accepts_new_producer_order_then_requires_fresh_grace() {
    use herdr_threads::ports::HostUiState;
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let mut rt = runtime(0);
    rt.after_ms = 60_000;
    archival::advance(&db, "i", &rt).unwrap();
    let old = archival::observation_ticket(&db, "i", "s", &rt)
        .unwrap()
        .unwrap();
    let mut observation = sample(0, HostUiState::Idle);
    observation.0.observation_sequence = 1_000_000;
    assert!(archival::record_sample(&mut db, &old, &rt, &observation).unwrap());
    rt.boot = "restarted".into();
    archival::advance(&db, "i", &rt).unwrap();
    let new = archival::observation_ticket(&db, "i", "s", &rt)
        .unwrap()
        .unwrap();
    observation.0.observation_sequence = 2;
    assert!(
        archival::record_sample(&mut db, &new, &rt, &observation).unwrap(),
        "new producer must not inherit old sequence or equal timestamp"
    );
    assert!(
        !archival::record_sample(&mut db, &old, &rt, &observation).unwrap(),
        "old ticket must still refuse"
    );
    // The refused old ticket is conservative negative evidence; restart the fresh interval.
    rt.mono = 1;
    rt.utc = UtcMillis(1);
    observation = sample(1, HostUiState::Idle);
    observation.0.observation_sequence = 3;
    assert!(archival::record_sample(&mut db, &new, &rt, &observation).unwrap());
    tick(&mut db, &rt);
    assert!(!archived(&db));
    rt.mono = 60_001;
    rt.utc = UtcMillis(rt.mono);
    observation = sample(rt.mono, HostUiState::Idle);
    observation.0.observation_sequence = 4;
    assert!(archival::record_sample(&mut db, &new, &rt, &observation).unwrap());
    tick(&mut db, &rt);
    assert!(archived(&db), "fresh grace must eventually archive");
}
#[test]
fn archival_recurring_observation_work_excludes_retained_inactive_history() {
    use herdr_threads::test_support::isolation::{CostCounter, count_vm_units};
    fn measure(size: usize) -> u64 {
        let db = fixture();
        thread(&db);
        joined_agent(&db);
        for n in 0..size {
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at) VALUES(?1,'i',?2,'native',1,0,CASE WHEN ?2='retired' THEN 0 END)",rusqlite::params![format!("a{n:08}"),if n%2==0 {"retired"}else{"unresolved"}]).unwrap();
        }
        let rt = runtime(0);
        for _ in 0..size / archival::PAGE + 3 {
            archival::advance(&db, "i", &rt).unwrap();
        }
        // Initial discovery may visit each retained row once. Drain bounded pages.
        for _ in 0..size / archival::PAGE + 3 {
            let _ = archival::next_observation(&db, "i", &rt).unwrap();
        }
        // Retired seats may retain the last sampled target. New target evidence
        // must not requeue that history on every observer publication.
        db.execute(
            "UPDATE seat_archival SET target_id='p' WHERE seat_id!='s'",
            [],
        )
        .unwrap();
        let before = db.total_changes();
        let rt = runtime(60_000);
        let counter = CostCounter::default();
        let ticket = count_vm_units(&db, &counter, || {
            db.execute_batch("UPDATE observed_targets SET ui_state='human_input'; UPDATE observed_targets SET ui_state='idle'").unwrap();
            archival::next_observation(&db, "i", &rt).unwrap()
        });
        assert_eq!(ticket.as_ref().map(|t| t.seat()), Some("s"));
        assert!(
            db.total_changes() - before <= 8,
            "recurring scan must not reschedule inactive history"
        );
        let due = db
            .query_row(
                "SELECT count(*) FROM seat_archival WHERE instance_id='i' AND next_mono<=60000",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(due, 0, "only the eligible seat was due");
        let plan=db.prepare("EXPLAIN QUERY PLAN SELECT seat_id FROM seat_archival WHERE instance_id='i' AND next_mono<=60000 ORDER BY next_mono,seat_id LIMIT 32").unwrap().query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join("; ");
        assert!(plan.contains("seat_archival_due"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        counter.units()
    }
    let small = measure(64);
    let large = measure(10_000);
    eprintln!("recurring observation VM units (10 instructions): {small} -> {large}");
    assert!(
        large <= small + 2,
        "retained history changed recurring VM cost: {small} -> {large}"
    );
}

#[test]
fn archival_parked_seat_requeues_after_canonical_membership_binding_and_state_changes() {
    let db = fixture();
    thread(&db);
    joined_agent(&db);
    let rt = runtime(0);
    archival::advance(&db, "i", &rt).unwrap();
    for (deactivate, reactivate) in [
        (
            "UPDATE memberships SET state='left'",
            "UPDATE memberships SET state='joined'",
        ),
        (
            "UPDATE occupant_bindings SET registered_at=NULL",
            "UPDATE occupant_bindings SET registered_at=1",
        ),
        (
            "UPDATE seats SET state='unresolved'",
            "UPDATE seats SET state='resolved'",
        ),
    ] {
        db.execute_batch(deactivate).unwrap();
        assert!(archival::next_observation(&db, "i", &rt).unwrap().is_none());
        assert!(
            db.query_row(
                "SELECT next_mono IS NULL FROM seat_archival WHERE seat_id='s'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        db.execute_batch(reactivate).unwrap();
        assert_eq!(
            archival::next_observation(&db, "i", &rt)
                .unwrap()
                .unwrap()
                .seat(),
            "s"
        );
    }
}

#[test]
fn archival_connection_epoch_change_accepts_new_order_but_rejects_delayed_old_epoch() {
    use herdr_threads::ports::HostUiState;
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let mut rt = runtime(0);
    archival::advance(&db, "i", &rt).unwrap();
    let ticket = archival::observation_ticket(&db, "i", "s", &rt)
        .unwrap()
        .unwrap();
    let mut observation = sample(0, HostUiState::Idle);
    observation.0.observation_sequence = 1_000_000;
    assert!(archival::record_sample(&mut db, &ticket, &rt, &observation).unwrap());
    rt.mono = 1;
    rt.utc = UtcMillis(1);
    observation = sample(1, HostUiState::Idle);
    observation.0.connection_epoch = 2;
    assert!(archival::record_sample(&mut db, &ticket, &rt, &observation).unwrap());
    rt.mono = 2;
    rt.utc = UtcMillis(2);
    observation = sample(2, HostUiState::Idle);
    observation.0.observation_sequence = 1_000_001;
    assert!(
        !archival::record_sample(&mut db, &ticket, &rt, &observation).unwrap(),
        "delayed earlier connection cannot regain positive evidence"
    );
}

#[test]
fn archival_startup_waits_for_canonical_host_instance() {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    let mut rt = runtime(0);
    rt.coherent = false;
    assert_eq!(
        archival::advance(&db, "i", &rt).unwrap(),
        archival::Progress::default()
    );
    let count: i64 = db
        .query_row("SELECT count(*) FROM archival_instances", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "archival cannot manufacture canonical host state");
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES('i',0)",
        [],
    )
    .unwrap();
    thread(&db);
    assert!(
        archival::advance(&db, "i", &rt)
            .unwrap()
            .archived
            .is_empty()
    );
    let count: i64 = db
        .query_row("SELECT count(*) FROM archival_instances", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "next pass resumes after canonical initialization");
    assert!(!archived(&db));
}

#[test]
fn archival_store_startup_pass_and_next_wait_for_canonical_host_instance() {
    use herdr_threads::{
        ports::StorePort,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    let dir = super::handoff_fences::Directory(
        std::env::temp_dir().join(format!("ht-archival-startup-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&dir.0).unwrap();
    let path = dir.0.join("store.db");
    let store = SqliteStore::new(
        StoreContext::new(
            path.clone(),
            std::sync::Arc::new(herdr_threads::app::SystemClock::default()),
        ),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let db = Connection::open(&path).unwrap();
    let budget = herdr_threads::protocol::time::CallBudget {
        deadline: herdr_threads::protocol::time::MonoInstant(
            store.clock().monotonic_now().0 + 5000,
        ),
        cancellation: Default::default(),
    };
    let mut rt = runtime(0);
    rt.coherent = false;
    let mut identity = super::handoff_fences::identity();
    identity.thread = None;
    let hints = [herdr_threads::archival_legacy::Hint {
        identity,
        progress_thread: None,
    }];
    assert_eq!(
        store.archival_pass(&rt, &hints, &budget).unwrap(),
        archival::Progress::default()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let next = store.archival_next(&rt, &budget).unwrap();
    assert!(next.ticket.is_none());
    assert!(!next.has_more);
    assert_eq!(
        db.query_row("SELECT count(*) FROM host_instances", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM archival_instances", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES('i',0)",
        [],
    )
    .unwrap();
    thread(&db);
    assert!(
        store
            .archival_pass(&rt, &hints, &budget)
            .unwrap()
            .archived
            .is_empty()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1,
        "fresh retry imports the veto after canonical initialization"
    );
    assert!(store.archival_next(&rt, &budget).unwrap().ticket.is_none());
    assert_eq!(
        db.query_row("SELECT count(*) FROM archival_instances", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!archived(&db));
}
