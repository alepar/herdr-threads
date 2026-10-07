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
        protocol::{ids::*, time::MonoInstant},
    };
    archival::ComposerObservation(
        HostObservation {
            target: HostTargetId::new("p"),
            host_boot: HostBootId::new("b"),
            epoch: 1,
            generation: 0,
            observed_at_utc: UtcMillis(at),
            observed_at_mono: MonoInstant(at as u64),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
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
        },
        Some(RegisteredComposerEvidence {
            parser: herdr_threads::harness::registry::builtins()
                .agent("codex")
                .unwrap(),
            reported_host_kind: "codex".into(),
            classification: if ui == HostUiState::Idle {
                ComposerClassification::Empty
            } else {
                ComposerClassification::Text
            },
            basis: ComposerEvidenceBasis::RegisteredHostKindComposerRead,
        }),
    )
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

// SOURCE_UNCOMPILED_UNEXECUTED. Additive draft for tests/store/channel_archival.rs.
// Not a new test target or product module. Rebind to the actual B4 successor.
// Uses only existing BASE APIs, including the one-field ComposerObservation.
#[cfg(target_os = "macos")]
mod archival_actual_producer_base_fixture {
    use herdr_threads::{
        host::native::NativeCli,
        ports::{
            ExecutionEvidence, HostCallContext, HostObservation, HostPort, HostUiState,
            IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
        },
        protocol::{
            ids::HostTargetId,
            time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
        },
        store::{archival, effective},
    };
    use rusqlite::{Connection, params};
    use serde_json::{Value, json};
    use std::{
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1_000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1_000)
        }
    }

    // No child process, native executable, daemon or real Herdr. The actual
    // NativeCli witnesses this test process's socket peer PID/start/UID.
    struct OwnedPeer {
        socket: PathBuf,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        errors: Arc<Mutex<Vec<String>>>,
        methods: Arc<Mutex<Vec<String>>>,
    }
    impl OwnedPeer {
        fn new(kind: &'static str, capture: &'static str) -> Self {
            Self::configured(kind, capture, "idle")
        }
        fn configured(kind: &'static str, capture: &'static str, status: &'static str) -> Self {
            let socket =
                PathBuf::from(format!("/private/tmp/ha-{}", uuid::Uuid::new_v4().simple()));
            assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
            let listener = UnixListener::bind(&socket).expect("owned fixture socket bind");
            // Guard owns the pathname before every later fallible operation.
            let mut owned = Self {
                socket,
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
                errors: Arc::new(Mutex::new(Vec::new())),
                methods: Arc::new(Mutex::new(Vec::new())),
            };
            listener
                .set_nonblocking(true)
                .expect("owned nonblocking listener");
            let stop = owned.stop.clone();
            let errors = owned.errors.clone();
            let methods = owned.methods.clone();
            owned.worker = Some(thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(5);
                while !stop.load(Ordering::Acquire) && Instant::now() < until {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(e) => {
                            errors.lock().unwrap().push(format!("accept: {e}"));
                            break;
                        }
                    };
                    if let Err(e) =
                        Self::exchange(&mut stream, kind, capture, status, &stop, until, &methods)
                    {
                        if !stop.load(Ordering::Acquire) {
                            errors.lock().unwrap().push(e);
                        }
                        break;
                    }
                }
                if !stop.load(Ordering::Acquire) {
                    errors
                        .lock()
                        .unwrap()
                        .push("owned server reached five-second bound".into());
                }
            }));
            owned
        }
        fn exchange(
            stream: &mut UnixStream,
            kind: &str,
            capture: &str,
            status: &str,
            stop: &AtomicBool,
            until: Instant,
            methods: &Mutex<Vec<String>>,
        ) -> Result<(), String> {
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .map_err(|e| e.to_string())?;
            stream
                .set_write_timeout(Some(Duration::from_millis(100)))
                .map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            loop {
                if stop.load(Ordering::Acquire) || Instant::now() >= until {
                    return Err("owned fixture stopped while reading".into());
                }
                let mut byte = [0];
                match stream.read(&mut byte) {
                    Ok(0) => return Err("request EOF before newline".into()),
                    Ok(_) if byte[0] == b'\n' => break,
                    Ok(_) => {
                        bytes.push(byte[0]);
                        if bytes.len() > 4_096 {
                            return Err("fixture request exceeds bound".into());
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            let request: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let method = request["method"].as_str().ok_or("missing method")?;
            methods.lock().unwrap().push(method.to_owned());
            let result = match method {
                "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                "pane.get" if request["params"]["pane_id"] == "w4:p1" => json!({
                    "type":"pane_info","pane":{"pane_id":"w4:p1","terminal_id":"term_1",
                    "workspace_id":"w4","tab_id":"w4:t1","focused":false,
                    "agent_status":status,"agent":kind,"revision":2}}),
                "agent.read"
                    if request["params"] == json!({"target":"w4:p1","source":"detection"}) =>
                {
                    json!({
                    "type":"pane_read","read":{"pane_id":if capture=="FIXTURE WRONG PANE" {"w4:p2"} else {"w4:p1"},"source":if capture=="FIXTURE WRONG SOURCE" {"recent"} else {"detection"},
                    "format":"text","text":capture,"revision":0,"truncated":false}})
                }
                _ => return Err(format!("unexpected bounded fixture request: {request}")),
            };
            writeln!(stream, "{}", json!({"id":request["id"],"result":result}))
                .map_err(|e| e.to_string())
        }
        fn settle(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take()
                && worker.join().is_err()
            {
                self.errors
                    .lock()
                    .unwrap()
                    .push("owned worker panicked".into());
            }
        }
        fn assert_protocol(&mut self) {
            self.settle();
            std::fs::remove_file(&self.socket)
                .expect("owned fixture socket removal after joined worker");
            assert!(
                !self.socket.exists(),
                "owned socket removed before deciding consumer"
            );
            let errors = self.errors.lock().unwrap().clone();
            assert!(errors.is_empty(), "fixture errors: {errors:?}");
            assert_eq!(
                *self.methods.lock().unwrap(),
                ["ping", "pane.get", "ping", "pane.get", "ping", "agent.read"],
                "one ordinary observation plus exactly one actual archival composer read"
            );
        }
    }
    impl Drop for OwnedPeer {
        fn drop(&mut self) {
            self.settle();
            // A unique successfully bound owned path; no foreign path scanning.
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    // Derive canonical state from actual ordinary NativeCli observation;
    // response JSON cannot set the peer-derived boot/incarnation/generation.
    fn seed_canonical(db: &Connection, harness: &str, observation: &HostObservation) {
        let proof = observation
            .verified_structural_proof()
            .expect("prerequisite: macOS actual peer structural proof");
        assert!(observation.occupant.is_none());
        assert!(
            matches!(
                observation.ui,
                HostUiState::Unknown | HostUiState::ApprovalOrQuestion
            ),
            "ordinary read cannot prove idle"
        );
        db.execute(
            "UPDATE host_instances SET host_boot=?1,host_epoch=?2 WHERE id='i'",
            params![proof.host_boot().as_str(), proof.host_epoch() as i64],
        )
        .unwrap();
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('s','i','resolved','native',?1,1,?2,0)",
            params![proof.target().as_str(), proof.target_generation() as i64]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s',1,?1,?2,?3,?4,'session','00000000-0000-4000-8000-000000000001','cooperative_top_level',1000,1000)",
            params![proof.target().as_str(), proof.host_boot().as_str(), proof.host_epoch() as i64, harness]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch,ui_state) VALUES('i',?1,?2,?3,?4,?5,'fresh',1000,?6,?7,'native_current_target',?8,'unknown')",
            params![proof.target().as_str(), proof.host_boot().as_str(), proof.host_epoch() as i64,
                proof.target_generation() as i64, observation.observation_sequence as i64,
                proof.terminal().as_str(), proof.incarnation(), proof.connection_epoch() as i64]).unwrap();
        db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('t','s','joined',1000)").unwrap();
    }

    fn one_case(harness: &'static str, capture: &'static str) -> bool {
        let mut peer = OwnedPeer::new(harness, capture);
        let cli = NativeCli::new(peer.socket.clone(), Arc::new(FixedClock));
        let target = HostTargetId::new("w4:p1");
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(11_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let ordinary = cli
            .observe_current_target(&target, &context)
            .expect("prerequisite: ordinary witnessed native read");
        assert_eq!(
            ordinary.ui,
            HostUiState::Unknown,
            "first causal ordinary C/C read remains Unknown"
        );
        let mut db = super::fixture();
        super::thread(&db);
        seed_canonical(&db, harness, &ordinary);
        let rt = archival::Runtime {
            boot: "actual-producer-fixture-runtime".into(),
            mono: 1_000,
            utc: UtcMillis(1_000),
            after_ms: 60_000,
            host_generation: 0,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered-source".into()),
        };
        super::tick(&mut db, &rt); // Actual advance initializes runtime and seat certificate rows.
        assert!(!super::archived(&db), "fresh joined channel cannot archive");
        let captured: (String, i64, i64) = db.query_row("SELECT ai.runtime_boot,ai.evidence_epoch,a.activity_revision FROM archival_instances ai JOIN seat_archival a ON a.instance_id=ai.instance_id WHERE ai.instance_id='i' AND a.seat_id='s'", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(captured.0, rt.boot);
        let ticket = archival::observation_ticket(&db, "i", "s", &rt)
            .unwrap()
            .expect("prerequisite: canonical registered cooperative C/C ticket exists");
        assert_eq!(ticket.target(), &target);
        assert_eq!(ticket.seat(), "s");
        let sample = cli
            .observe_current_target_for_archival(
                &target,
                &ticket.host_context(context.budget.clone()),
            )
            .expect("prerequisite: real archival parser/wrapper completes");
        peer.assert_protocol(); // Join before causal assertion; Drop also covers any prior panic.
        let observed = &sample.0; // Never mutate/copy substitute this envelope.
        assert_eq!(
            observed.ui,
            HostUiState::Idle,
            "prerequisite: captured real composer grammar Empty + Idle"
        );
        assert!(
            observed.occupant.is_none(),
            "actual native producer must stay honest"
        );
        assert_eq!(observed.occupancy, StructuralOccupancy::Unknown);
        assert_eq!(observed.execution, ExecutionEvidence::Unknown);
        assert_eq!(
            observed.provenance,
            ObservationProvenance::FreshCurrentTarget
        );
        assert!(matches!(
            observed.incarnation,
            IncarnationEvidence::Verified { .. }
        ));
        let proof = observed
            .verified_structural_proof()
            .expect("prerequisite: actual archival structural proof");
        let current = effective::effective_observation(&db, "i", target.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(proof.target().as_str(), current.target_id);
        assert_eq!(proof.host_boot().as_str(), current.host_boot);
        assert_eq!(proof.host_epoch() as i64, current.epoch);
        assert_eq!(
            proof.target_generation() as i64,
            current.structural_generation
        );
        assert_eq!(
            Some(proof.terminal().as_str()),
            current.terminal_id.as_deref()
        );
        assert_eq!(Some(proof.incarnation()), current.incarnation.as_deref());
        assert!(current.observation_sequence <= observed.observation_sequence as i64);
        assert_eq!(proof.connection_epoch(), cli.epoch());
        assert!(observed.completed_at_mono.0 <= rt.mono as u64);
        assert!(observed.started_at_mono.0 >= rt.mono.saturating_sub(5_000).max(0) as u64);
        let deciding: (String, i64, i64) = db.query_row("SELECT ai.runtime_boot,ai.evidence_epoch,a.activity_revision FROM archival_instances ai JOIN seat_archival a ON a.instance_id=ai.instance_id WHERE ai.instance_id='i' AND a.seat_id='s'", [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(
            captured, deciding,
            "ticket runtime/evidence/activity unchanged across real host I/O"
        );
        let registered: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id='s' AND generation=1 AND harness=?1 AND native_session='session' AND execution_id='00000000-0000-4000-8000-000000000001' AND ended_at IS NULL AND registered_at IS NOT NULL AND observation_provenance='cooperative_top_level')", [harness], |r| r.get(0)).unwrap();
        assert!(
            registered,
            "prerequisite: exact open registered cooperative identity unchanged"
        );
        let accepted = archival::record_sample(&mut db, &ticket, &rt, &sample).unwrap();
        eprintln!(
            "actual-producer case={harness} prerequisite_checks=passed occupant=None execution=Unknown sample_accepted={accepted}"
        );
        accepted
    }

    struct MovingClock(std::sync::atomic::AtomicU64);
    impl Clock for MovingClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0.load(Ordering::Acquire) as i64)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::Acquire))
        }
    }
    fn complete_authority(db: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        ["seats","occupant_bindings","memberships","membership_intervals","receipts","receipt_state",
         "invitations","invitation_rejections","invitation_cancellations","requirement_episodes",
         "send_preparations","prepared_recipients","prepared_unavailable_warnings","send_manifests",
         "service_notification_preparations","catch_up","summary_jobs","channel_handoff_fences","host_instances"]
        .into_iter().map(|table| {
            let projection=if table=="host_instances" {
                let mut fields=db.prepare("SELECT name FROM pragma_table_info('host_instances') WHERE name<>'decision_seq' ORDER BY cid").unwrap();
                fields.query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join(",")
            } else { "*".into() };
            let mut st=db.prepare(&format!("SELECT {projection} FROM {table} ORDER BY rowid")).unwrap();let n=st.column_count();
            st.query_map([],|r|(0..n).map(|i|r.get(i)).collect()).unwrap().collect::<Result<_,_>>().unwrap()
        }).collect()
    }
    pub(super) fn registered_alias_family_actual_producer_and_poke() {
        use herdr_threads::{
            harness::registry, test_support::archival_composer_fixture as injected,
        };
        let r = injected::registry();
        assert_eq!(injected::ALIAS_64.len(), 64);
        assert_eq!(injected::ALIAS_65.len(), 65);
        assert_eq!(
            r.agent("synthetic_fourth").unwrap(),
            registry::builtins().agent("synthetic_fourth").unwrap()
        );
        let mut cases = Vec::new();
        for kind in [
            injected::ALIAS_64,
            injected::ALIAS_65,
            injected::ALIAS_CONTROL,
        ] {
            assert_eq!(
                r.by_host_kind(kind).unwrap().metadata().id,
                "synthetic_fourth"
            );
            cases.push((
                kind,
                r,
                "idle",
                HostUiState::Unknown,
                HostUiState::Idle,
                true,
            ));
            cases.push((
                kind,
                r,
                "blocked",
                HostUiState::ApprovalOrQuestion,
                HostUiState::ApprovalOrQuestion,
                false,
            ));
        }
        cases.extend([
            (
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                r,
                "idle",
                HostUiState::Unknown,
                HostUiState::Unknown,
                false,
            ),
            (
                "synthetic_fourth\tundeclared",
                r,
                "blocked",
                HostUiState::Unknown,
                HostUiState::Unknown,
                false,
            ),
            (
                "synthetic_fourth_alias",
                registry::builtins(),
                "idle",
                HostUiState::Unknown,
                HostUiState::Unknown,
                false,
            ),
        ]);
        let mut failures = Vec::new();
        for (kind, r, status, want_ordinary, want_composer, want_accept) in cases {
            let mut peer = OwnedPeer::configured(kind, "SYNTHETIC COMPOSER EMPTY", status);
            let cli = NativeCli::with_registry(peer.socket.clone(), Arc::new(FixedClock), r);
            let target = HostTargetId::new("w4:p1");
            let context = HostCallContext {
                budget: CallBudget {
                    deadline: MonoInstant(10000),
                    cancellation: Cancellation::default(),
                },
                expected_boot: None,
                expected_epoch: None,
            };
            let ordinary = cli.observe_current_target(&target, &context).unwrap();
            let db = super::fixture();
            super::thread(&db);
            seed_canonical(&db, "synthetic_fourth", &ordinary);
            let authority = complete_authority(&db);
            let rt = archival::Runtime {
                boot: "alias-family-runtime".into(),
                mono: 1000,
                utc: UtcMillis(1000),
                after_ms: 60000,
                host_generation: 0,
                coherent: true,
                valid_until_mono: None,
                legacy_source: Some("covered-source".into()),
            };
            injected::advance(&db, "i", &rt, r).unwrap();
            let ticket = injected::ticket(&db, "i", "s", &rt, r).unwrap();
            let sample = cli
                .observe_current_target_for_archival(&target, &context)
                .unwrap();
            let poke = cli
                .observe_current_target_for_poke(&target, &context)
                .unwrap();
            let accepted = ticket
                .as_ref()
                .is_some_and(|t| injected::sample(&db, t, &rt, &sample, r).unwrap());
            // Cleanup precedes the behavioral assertion, including at frozen RED.
            peer.settle();
            std::fs::remove_file(&peer.socket).unwrap();
            assert!(!peer.socket.exists());
            let errors = peer.errors.lock().unwrap().clone();
            assert!(errors.is_empty(), "fixture protocol errors: {errors:?}");
            let methods = peer.methods.lock().unwrap().clone();
            let reads = methods
                .iter()
                .filter(|m| m.as_str() == "agent.read")
                .count();
            assert_eq!(
                methods.iter().filter(|m| m.as_str() == "pane.get").count(),
                3
            );
            assert!(sample.0.verified_structural_proof().is_some());
            assert!(sample.0.occupant.is_none());
            assert_eq!(sample.0.occupancy, StructuralOccupancy::Unknown);
            assert_eq!(sample.0.execution, ExecutionEvidence::Unknown);
            assert_eq!(complete_authority(&db), authority);
            let evidence_matches = sample.1.as_ref().is_some_and(|e| {
                e.parser == r.agent("synthetic_fourth").unwrap()
                    && e.reported_host_kind == kind
                    && e.classification == herdr_threads::ports::ComposerClassification::Empty
            });
            let ok = ordinary.ui == want_ordinary
                && sample.0.ui == want_composer
                && poke.ui == want_composer
                && accepted == want_accept
                && evidence_matches == want_accept
                && reads == if want_accept { 2 } else { 0 };
            eprintln!(
                "alias-family actual kind={kind:?} bytes={} status={status} ordinary={:?} archival={:?} poke={:?} ticket={} evidence={} accepted={accepted} detection_reads={reads} peer_joined=true socket_absent=true protocol_errors=0",
                kind.len(),
                ordinary.ui,
                sample.0.ui,
                poke.ui,
                ticket.is_some(),
                evidence_matches
            );
            if !ok {
                failures.push(format!("{kind:?}/{status}: ordinary={:?} archival={:?} poke={:?} accepted={accepted} evidence={evidence_matches} reads={reads}", ordinary.ui, sample.0.ui, poke.ui));
            }
        }
        assert!(
            failures.is_empty(),
            "exact declared aliases must preserve real producer/canonical/poke behavior: {failures:?}"
        );
    }

    pub(super) fn full_grace_actual_producers() {
        use herdr_threads::{
            harness::registry, test_support::archival_composer_fixture as injected,
        };
        for (binding, kind, capture, r) in [
            (
                "claude",
                "claude",
                include_str!(
                    "../../docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt"
                ),
                registry::builtins(),
            ),
            (
                "codex",
                "codex",
                include_str!(
                    "../../docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt"
                ),
                registry::builtins(),
            ),
            (
                "synthetic_fourth",
                "synthetic_fourth_alias",
                "SYNTHETIC COMPOSER EMPTY",
                injected::registry(),
            ),
            (
                "synthetic_fourth",
                injected::ALIAS_64,
                "SYNTHETIC COMPOSER EMPTY",
                injected::registry(),
            ),
            (
                "synthetic_fourth",
                injected::ALIAS_65,
                "SYNTHETIC COMPOSER EMPTY",
                injected::registry(),
            ),
            (
                "synthetic_fourth",
                injected::ALIAS_CONTROL,
                "SYNTHETIC COMPOSER EMPTY",
                injected::registry(),
            ),
        ] {
            let mut peer = OwnedPeer::new(kind, capture);
            let clock = Arc::new(MovingClock(std::sync::atomic::AtomicU64::new(1000)));
            let cli = NativeCli::with_registry(peer.socket.clone(), clock.clone(), r);
            let target = HostTargetId::new("w4:p1");
            let context = HostCallContext {
                budget: CallBudget {
                    deadline: MonoInstant(10000),
                    cancellation: Cancellation::default(),
                },
                expected_boot: None,
                expected_epoch: None,
            };
            let ordinary = cli.observe_current_target(&target, &context).unwrap();
            let db = super::fixture();
            super::thread(&db);
            seed_canonical(&db, binding, &ordinary);
            let authority = complete_authority(&db);
            let before_decision: i64 = db
                .query_row(
                    "SELECT decision_seq FROM host_instances WHERE id='i'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            for at in [1000, 61000] {
                clock.0.store(at, Ordering::Release);
                let rt = archival::Runtime {
                    boot: "full-grace-runtime".into(),
                    mono: at as i64,
                    utc: UtcMillis(at as i64),
                    after_ms: 60000,
                    host_generation: 0,
                    coherent: true,
                    valid_until_mono: None,
                    legacy_source: Some("covered-source".into()),
                };
                injected::advance(&db, "i", &rt, r).unwrap();
                let ticket = injected::ticket(&db, "i", "s", &rt, r).unwrap().unwrap();
                let sample = cli
                    .observe_current_target_for_archival(
                        &target,
                        &ticket.host_context(CallBudget {
                            deadline: MonoInstant(at + 5000),
                            cancellation: Cancellation::default(),
                        }),
                    )
                    .unwrap();
                assert!(sample.0.occupant.is_none());
                assert_eq!(sample.0.execution, ExecutionEvidence::Unknown);
                assert_eq!(sample.1.as_ref().unwrap().parser, r.agent(binding).unwrap());
                assert_eq!(sample.1.as_ref().unwrap().reported_host_kind, kind);
                assert!(
                    injected::sample(&db, &ticket, &rt, &sample, r).unwrap(),
                    "real unchanged {binding} envelope at {at}"
                );
                for _ in 0..20 {
                    if !injected::advance(&db, "i", &rt, r).unwrap().has_more {
                        break;
                    }
                }
                assert_eq!(super::archived(&db), at == 61000, "whole grace {binding}");
            }
            peer.settle();
            std::fs::remove_file(&peer.socket).unwrap();
            assert!(peer.errors.lock().unwrap().is_empty());
            assert!(!peer.socket.exists());
            assert_eq!(
                *peer.methods.lock().unwrap(),
                [
                    "ping",
                    "pane.get",
                    "ping",
                    "pane.get",
                    "ping",
                    "agent.read",
                    "ping",
                    "pane.get",
                    "ping",
                    "agent.read"
                ]
            );
            assert_eq!(
                complete_authority(&db),
                authority,
                "archive changes no accountable authority for {binding}"
            );
            assert_eq!(
                db.query_row(
                    "SELECT decision_seq FROM host_instances WHERE id='i'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                before_decision + 1,
                "one lifecycle bookkeeping decision"
            );
            let identity:(String,String,String,i64)=db.query_row("SELECT harness,native_session,execution,binding_generation FROM seat_archival WHERE seat_id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(
                identity,
                (
                    binding.into(),
                    "session".into(),
                    "00000000-0000-4000-8000-000000000001".into(),
                    1
                )
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
                serde_json::from_str::<Value>(&payload).unwrap()["provenance"],
                "daemon_lifecycle"
            );
        }
    }
    pub(super) fn actual_parser_negatives() {
        let empty = include_str!(
            "../../docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt"
        );
        for (binding, kind, capture, status, want_ui, want_read) in [
            ("codex", "claude", empty, "idle", HostUiState::Idle, true),
            (
                "codex",
                "codex",
                "unrecognizable capture",
                "idle",
                HostUiState::Unknown,
                true,
            ),
            (
                "codex",
                "hermes",
                "unused",
                "idle",
                HostUiState::Unknown,
                false,
            ),
            (
                "codex",
                "unregistered",
                "unused",
                "idle",
                HostUiState::Unknown,
                false,
            ),
            (
                "codex",
                "codex",
                include_str!(
                    "../../docs/evidence/poke-spike/captures/codex-q-cycle-1-draft.read-detection.txt"
                ),
                "idle",
                HostUiState::HumanInput,
                true,
            ),
            (
                "claude",
                "claude",
                include_str!(
                    "../../docs/evidence/poke-spike/captures/claude-q-cycle-1-draft.read-detection.txt"
                ),
                "idle",
                HostUiState::HumanInput,
                true,
            ),
            (
                "codex",
                "codex",
                "FIXTURE WRONG SOURCE",
                "idle",
                HostUiState::Unknown,
                true,
            ),
            (
                "codex",
                "codex",
                "FIXTURE WRONG PANE",
                "idle",
                HostUiState::Unknown,
                true,
            ),
            (
                "codex",
                "codex",
                "unused",
                "blocked",
                HostUiState::ApprovalOrQuestion,
                false,
            ),
            (
                "codex",
                "codex",
                "unused",
                "unrecognized_status",
                HostUiState::Unknown,
                false,
            ),
            (
                "codex",
                "codex",
                include_str!(
                    "../../docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt"
                ),
                "working",
                HostUiState::ActiveTurn,
                true,
            ),
        ] {
            let mut peer = OwnedPeer::configured(kind, capture, status);
            let cli = NativeCli::new(peer.socket.clone(), Arc::new(FixedClock));
            let target = HostTargetId::new("w4:p1");
            let context = HostCallContext {
                budget: CallBudget {
                    deadline: MonoInstant(10000),
                    cancellation: Cancellation::default(),
                },
                expected_boot: None,
                expected_epoch: None,
            };
            let observed = cli.observe_current_target(&target, &context);
            if status == "unrecognized_status" {
                assert_eq!(
                    observed.unwrap_err().code,
                    herdr_threads::protocol::results::ErrorCode::StaleHostObservation
                );
                peer.settle();
                std::fs::remove_file(&peer.socket).unwrap();
                assert!(peer.errors.lock().unwrap().is_empty());
                assert!(!peer.socket.exists());
                assert_eq!(*peer.methods.lock().unwrap(), ["ping", "pane.get"]);
                continue;
            }
            let ordinary = observed.unwrap();
            let mut db = super::fixture();
            super::thread(&db);
            seed_canonical(&db, binding, &ordinary);
            let rt = archival::Runtime {
                boot: "negative-runtime".into(),
                mono: 1000,
                utc: UtcMillis(1000),
                after_ms: 60000,
                host_generation: 0,
                coherent: true,
                valid_until_mono: None,
                legacy_source: Some("covered-source".into()),
            };
            super::tick(&mut db, &rt);
            let ticket = archival::observation_ticket(&db, "i", "s", &rt)
                .unwrap()
                .unwrap();
            let sample = cli
                .observe_current_target_for_archival(&target, &ticket.host_context(context.budget))
                .unwrap();
            peer.settle();
            std::fs::remove_file(&peer.socket).unwrap();
            assert!(peer.errors.lock().unwrap().is_empty());
            assert!(!peer.socket.exists());
            assert_eq!(sample.0.ui, want_ui, "{kind}");
            assert!(sample.0.occupant.is_none());
            assert_eq!(
                peer.methods
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|m| m.as_str() == "agent.read")
                    .count(),
                usize::from(want_read)
            );
            assert!(
                !archival::record_sample(&mut db, &ticket, &rt, &sample).unwrap(),
                "parser/refusal {kind}"
            );
            assert!(!super::archived(&db));
        }
    }

    pub(super) fn run_both_captured_grammars() {
        let observations = [
            (
                "claude",
                one_case(
                    "claude",
                    include_str!(
                        "../../docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt"
                    ),
                ),
            ),
            (
                "codex",
                one_case(
                    "codex",
                    include_str!(
                        "../../docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt"
                    ),
                ),
            ),
        ];
        // Delayed assertion ensures BOTH captured grammars reach the deciding
        // consumer on BASE, and both owned servers are joined before RED.
        assert!(
            observations.iter().all(|(_, accepted)| *accepted),
            "actual unchanged None-occupant native envelopes must qualify first canonical samples: {observations:?}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_archival_none_occupant_reaches_canonical_sample() {
    archival_actual_producer_base_fixture::run_both_captured_grammars();
}

#[test]
fn archival_registry_scaffold_builtin_semantics_are_equal() {
    use herdr_threads::{
        harness::registry, ports::HostUiState, test_support::archival_composer_fixture as injected,
    };
    let mut a = fixture();
    let b = fixture();
    thread(&a);
    thread(&b);
    joined_agent(&a);
    joined_agent(&b);
    ordinary(&a);
    ordinary(&b);
    let rt = runtime(0);
    let r = registry::builtins();
    assert_eq!(
        archival::advance(&a, "i", &rt).unwrap(),
        injected::advance(&b, "i", &rt, r).unwrap()
    );
    let ta = archival::observation_ticket(&a, "i", "s", &rt)
        .unwrap()
        .unwrap();
    let tb = injected::ticket(&b, "i", "s", &rt, r).unwrap().unwrap();
    assert_eq!((ta.target(), ta.seat()), (tb.target(), tb.seat()));
    assert_eq!(
        archival::record_sample(&mut a, &ta, &rt, &sample(0, HostUiState::Idle)).unwrap(),
        injected::sample(&b, &tb, &rt, &sample(0, HostUiState::Idle), r).unwrap()
    );
    let na = archival::next_observation(&a, "i", &rt).unwrap();
    let nb = injected::next(&b, "i", &rt, r).unwrap();
    assert_eq!(
        na.as_ref().map(|t| (t.target(), t.seat())),
        nb.as_ref().map(|t| (t.target(), t.seat()))
    );
    for table in ["seat_archival", "channel_archival", "archival_instances"] {
        fn rows(db: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
            let mut st = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let n = st.column_count();
            st.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        }
        assert_eq!(
            rows(&a, table),
            rows(&b, table),
            "same canonical state for {table}"
        );
    }
}
#[test]
fn registered_composer_archival_ticket_and_scheduler_preserve_identity() {
    use herdr_threads::{harness::registry, test_support::archival_composer_fixture as injected};
    let db = fixture();
    thread(&db);
    joined_agent(&db);
    ordinary(&db);
    db.execute(
        "UPDATE occupant_bindings SET harness='synthetic_fourth' WHERE seat_id='s'",
        [],
    )
    .unwrap();
    let rt = runtime(0);
    let r = injected::registry();
    assert_eq!(
        r.agent("synthetic_fourth").unwrap(),
        registry::builtins().agent("synthetic_fourth").unwrap()
    );
    assert!(
        registry::builtins()
            .by_id(registry::builtins().agent("synthetic_fourth").unwrap())
            .unwrap()
            .composer_policy()
            .is_none()
    );
    assert!(
        r.by_host_kind("synthetic_fourth_alias")
            .unwrap()
            .composer_policy()
            .is_some()
    );
    injected::advance(&db, "i", &rt, r).unwrap();
    let ticket = injected::ticket(&db, "i", "s", &rt, r).unwrap();
    let scheduled = injected::next(&db, "i", &rt, r).unwrap();
    let next: Option<i64> = db
        .query_row(
            "SELECT next_mono FROM seat_archival WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!(
        "fourth gate witness ticket={} scheduled={} next={next:?}",
        ticket.is_some(),
        scheduled.is_some()
    );
    assert!(
        ticket.is_some() && scheduled.is_some() && next.is_some(),
        "lawful composer registry identity must retain ticket and due scheduling"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn archival_actual_producer_full_grace_and_authority_are_exact() {
    archival_actual_producer_base_fixture::full_grace_actual_producers();
}
#[cfg(target_os = "macos")]
#[test]
fn archival_actual_producer_parser_mismatch_and_absence_refuse() {
    archival_actual_producer_base_fixture::actual_parser_negatives();
}
#[test]
fn archival_registered_binding_and_composer_evidence_neighbors_refuse() {
    use herdr_threads::{
        harness::registry, ports::*, test_support::archival_composer_fixture as injected,
    };
    for bad in [
        "no evidence",
        "wrong parser",
        "wrong kind",
        "unreadable",
        "text",
        "unsafe",
        "unknown ui",
        "conflicting occupant",
        "unregistered",
        "managed",
        "held",
        "unresolved",
        "retired",
        "session",
        "execution",
        "generation",
        "target",
        "boot",
        "epoch",
        "terminal",
        "incarnation",
    ] {
        let mut db = fixture();
        thread(&db);
        joined_agent(&db);
        ordinary(&db);
        let rt = runtime(0);
        archival::advance(&db, "i", &rt).unwrap();
        let ticket = archival::observation_ticket(&db, "i", "s", &rt)
            .unwrap()
            .unwrap();
        let mut value = sample(0, HostUiState::Idle);
        match bad {
            "no evidence" => value.1 = None,
            "wrong parser" => {
                value.1.as_mut().unwrap().parser = registry::builtins().agent("claude").unwrap()
            }
            "wrong kind" => value.1.as_mut().unwrap().reported_host_kind = "claude".into(),
            "unreadable" => {
                value.1.as_mut().unwrap().classification = ComposerClassification::Unreadable
            }
            "text" => value.1.as_mut().unwrap().classification = ComposerClassification::Text,
            "unsafe" => value.1.as_mut().unwrap().classification = ComposerClassification::Unsafe,
            "unknown ui" => value.0.ui = HostUiState::Unknown,
            "conflicting occupant" => {
                value.0.occupant = Some(NativeOccupant {
                    harness: herdr_threads::protocol::authority::Harness::Claude,
                    session: herdr_threads::protocol::ids::NativeSessionId::new("claim"),
                    execution: herdr_threads::protocol::ids::ExecutionId::new("claim"),
                    is_top_level: true,
                })
            }
            "unregistered" => {
                db.execute("UPDATE occupant_bindings SET registered_at=NULL", [])
                    .unwrap();
            }
            "managed" => {
                db.execute(
                    "UPDATE occupant_bindings SET observation_provenance='managed_launch'",
                    [],
                )
                .unwrap();
            }
            "held" => {
                db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'fixture hold')",[]).unwrap();
            }
            "unresolved" => {
                db.execute("UPDATE seats SET state='unresolved'", [])
                    .unwrap();
            }
            "retired" => {
                db.execute("UPDATE seats SET state='retired',retired_at=1000", [])
                    .unwrap();
            }
            "session" => {
                db.execute("UPDATE occupant_bindings SET native_session='changed'", [])
                    .unwrap();
            }
            "execution" => {
                db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-000000000002'",[]).unwrap();
            }
            "generation" => {
                db.execute("UPDATE seats SET generation=2", []).unwrap();
            }
            "target" => {
                value.0.target = herdr_threads::protocol::ids::HostTargetId::new("different");
            }
            "boot" => {
                value.0.host_boot = herdr_threads::protocol::ids::HostBootId::new("different");
            }
            "epoch" => value.0.epoch = 2,
            "terminal" => {
                value.0.terminal = Some(herdr_threads::protocol::ids::TerminalId::new("different"))
            }
            "incarnation" => {
                value.0.incarnation = IncarnationEvidence::Verified {
                    identity: "different".into(),
                    evidence_kind: EvidenceKind::NativeCurrentTarget,
                }
            }
            _ => unreachable!(),
        }
        assert!(
            !archival::record_sample(&mut db, &ticket, &rt, &value).unwrap(),
            "canonical/evidence refusal {bad}"
        );
    }
    for absent in ["human", "unknown_adapter", "hermes", "synthetic_fourth"] {
        let db = fixture();
        thread(&db);
        joined_agent(&db);
        ordinary(&db);
        db.execute("UPDATE occupant_bindings SET harness=?1", [absent])
            .unwrap();
        let rt = runtime(0);
        archival::advance(&db, "i", &rt).unwrap();
        assert!(
            archival::observation_ticket(&db, "i", "s", &rt)
                .unwrap()
                .is_none()
        );
        assert!(archival::next_observation(&db, "i", &rt).unwrap().is_none());
    }
    // The same overlay identity is eligible only with its optional provider.
    assert!(
        injected::registry()
            .by_id(injected::registry().agent("synthetic_fourth").unwrap())
            .unwrap()
            .composer_policy()
            .is_some()
    );
}
#[test]
fn archival_stale_sample_harness_cannot_pass_final_member_scan() {
    use herdr_threads::ports::HostUiState;
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    ordinary(&db);
    let mut rt = runtime(0);
    rt.after_ms = 60000;
    for at in [0, 60000] {
        rt.mono = at;
        rt.utc = UtcMillis(at);
        archival::advance(&db, "i", &rt).unwrap();
        let t = archival::observation_ticket(&db, "i", "s", &rt)
            .unwrap()
            .unwrap();
        assert!(archival::record_sample(&mut db, &t, &rt, &sample(at, HostUiState::Idle)).unwrap());
    }
    db.execute(
        "UPDATE seat_archival SET harness='claude' WHERE seat_id='s'",
        [],
    )
    .unwrap();
    tick(&mut db, &rt);
    assert!(
        !archived(&db),
        "stale sample harness must not qualify exact current binding"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn archival_registered_alias_family_actual_producer_and_poke() {
    archival_actual_producer_base_fixture::registered_alias_family_actual_producer_and_poke();
}

// Isolates the deciding consumer guard with explicitly synthetic envelopes.
// Actual NativeCli closure is exercised separately without rewriting its envelope.
#[test]
fn archival_registered_alias_family_canonical_sample() {
    use herdr_threads::{
        harness::registry, ports::HostUiState, test_support::archival_composer_fixture as injected,
    };
    let r = injected::registry();
    let mut failures = Vec::new();
    for (kind, wrong_parser, want_accept) in [
        (injected::ALIAS_64, false, true),
        (injected::ALIAS_65, false, true),
        (injected::ALIAS_CONTROL, false, true),
        (
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            false,
            false,
        ),
        ("synthetic_fourth\tundeclared", false, false),
        (injected::ALIAS_65, true, false),
    ] {
        let db = fixture();
        thread(&db);
        joined_agent(&db);
        ordinary(&db);
        db.execute(
            "UPDATE occupant_bindings SET harness='synthetic_fourth'",
            [],
        )
        .unwrap();
        let rt = runtime(0);
        injected::advance(&db, "i", &rt, r).unwrap();
        let ticket = injected::ticket(&db, "i", "s", &rt, r).unwrap().unwrap();
        assert!(injected::next(&db, "i", &rt, r).unwrap().is_some());
        let mut value = sample(0, HostUiState::Idle);
        let e = value.1.as_mut().unwrap();
        e.parser = if wrong_parser {
            registry::builtins().agent("codex").unwrap()
        } else {
            r.agent("synthetic_fourth").unwrap()
        };
        e.reported_host_kind = kind.into();
        assert!(value.0.occupant.is_none());
        let accepted = injected::sample(&db, &ticket, &rt, &value, r).unwrap();
        eprintln!(
            "alias-family synthetic-consumer kind={kind:?} bytes={} wrong_parser={wrong_parser} accepted={accepted}",
            kind.len()
        );
        if accepted != want_accept {
            failures.push(format!(
                "{kind:?}: accepted={accepted} expected={want_accept}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "exact registered aliases must qualify canonical synthetic samples: {failures:?}"
    );
}
