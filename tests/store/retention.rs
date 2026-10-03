//! Retention (nested spec D3): keep set, bounded batches, work-job pruning.
use super::*;
use crate::{
    ports::{
        DueScanRequest, DueScanState, DurableWorkAdmission, EnumerationEvidence, EvidenceKind,
        ExecutionEvidence, HostInvalidationReason, HostObservation, HostSnapshot, HostUiState,
        IncarnationEvidence, ObservationProvenance, SnapshotGenerationId, SnapshotHeader,
        StorePort, StructuralOccupancy,
    },
    protocol::{
        ids::{HostBootId, HostCallId, HostTargetId},
        time::{Cancellation, MonoInstant, UtcMillis},
    },
    service::fair_writer::FairWriter,
    store::{StoreSettings, WriterTurn, connection::StoreContext, cooperative_permit_request},
    test_support::isolation::{CostCounter, TestIsolation},
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

const HOUR_MS: i64 = 60 * 60 * 1000;
/// A wall-clock start late enough that "now minus 25 h" is positive.
const T0: i64 = 1_000 * HOUR_MS;

/// A settable UTC clock; the monotonic reading never moves, so no quantum
/// elapses and the row budget is the only limit under test.
struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    }
}

struct Fx {
    _iso: TestIsolation,
    path: std::path::PathBuf,
    store: Arc<SqliteStore>,
    clock: Arc<TestClock>,
}

impl Fx {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let clock = Arc::new(TestClock(AtomicI64::new(T0)));
        let path = iso.state_root().join("store.db");
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap();
        let fx = Self {
            _iso: iso,
            path,
            store: Arc::new(store),
            clock,
        };
        fx.db()
            .execute(
                "INSERT INTO host_instances(id,created_at) VALUES ('i',0) ON CONFLICT(id) DO NOTHING",
                [],
            )
            .unwrap();
        fx
    }

    /// The domain writer connection (foreign keys on, every trigger live).
    fn db(&self) -> WriterTurn<'_> {
        self.store.writer(&budget()).unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        self.db()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn ids(&self, sql: &str) -> BTreeSet<String> {
        let db = self.db();
        let mut stmt = db.prepare(sql).unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn generations(&self) -> BTreeSet<String> {
        self.ids("SELECT id FROM snapshot_generations")
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// Runs passes until one reports no more work; returns the totals.
    fn drain(&self) -> PruneProgress {
        let mut total = PruneProgress::default();
        for _ in 0..10_000 {
            let pass = prune_once(&self.store, &budget()).unwrap();
            total.generations += pass.generations;
            total.targets += pass.targets;
            total.jobs += pass.jobs;
            total.preparations += pass.preparations;
            if !pass.has_more {
                return total;
            }
        }
        panic!("retention did not drain");
    }
}

/// A raw generation row (`n` is unique per fixture) with `targets` targets.
fn add_generation(db: &Connection, n: i64, status: &str, admission: i64, targets: usize) -> String {
    let id = format!("g{n}");
    db.execute(
        "INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,admission_sequence,created_at) VALUES (?1,'i','b',1,?2,'inc',?3,?3,?4,0,0,?5,0)",
        params![id, n, targets as i64, status, admission],
    )
    .unwrap();
    let mut insert = db
        .prepare_cached("INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES (?1,?2,1,1,'unknown','unknown',0)")
        .unwrap();
    for t in 0..targets {
        insert.execute(params![id, format!("t{t}")]).unwrap();
    }
    id
}

fn set_host(db: &Connection, decided: i64, issued: i64) {
    db.execute(
        "UPDATE host_instances SET observation_admission_sequence=?2,observation_decided_sequence=?1 WHERE id='i'",
        params![decided, issued],
    )
    .unwrap();
}

fn set_pointers(db: &Connection, active: Option<&str>, baseline: Option<&str>) {
    db.execute(
        "UPDATE host_instances SET active_snapshot_id=?1,recovery_baseline_generation_id=?2 WHERE id='i'",
        params![active, baseline],
    )
    .unwrap();
}

fn pin_by_unresolved_seat(db: &Connection, seat: &str, generation: &str) {
    db.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,unresolved_from_generation_id,unresolved_prior_binding_generation,role,target_id,generation,created_at) VALUES (?1,'i','unresolved','host_invalidation',?2,1,'native','p',1,0)",
        params![seat, generation],
    )
    .unwrap();
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn observation(target: &str, sequence: u64) -> HostObservation {
    HostObservation {
        target: HostTargetId::new(target),
        focused: false,
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::CoherentEnumeration,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("capture-{target}-{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    }
}

fn snapshot(sequence: u64, panes: usize) -> HostSnapshot {
    HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: (0..panes)
            .map(|p| observation(&format!("pane-{p}"), sequence))
            .collect(),
    }
}

/// Stages one coherent capture of `panes` targets: admission, stage and
/// target slices. Returns the admission and the (building) stage.
fn stage(
    store: &SqliteStore,
    sequence: u64,
    panes: usize,
) -> (crate::ports::HostObservationAdmission, SnapshotGenerationId) {
    let admission = StorePort::begin_host_observation(store, "i", &budget()).unwrap();
    let snap = snapshot(sequence, panes);
    let header = SnapshotHeader::from_captured(admission.clone(), &snap).unwrap();
    let stage = StorePort::begin_snapshot_stage(store, header, &budget()).unwrap();
    for (chunk, targets) in snap.targets.chunks(16).enumerate() {
        StorePort::stage_snapshot_targets(
            store,
            &stage.id,
            (chunk * 16) as u64,
            targets,
            DurableWorkAdmission::new(16).unwrap(),
            &budget(),
        )
        .unwrap();
    }
    (admission, stage.id)
}

/// A full observation publish, as the observation lane does it.
fn publish(store: &SqliteStore, sequence: u64, panes: usize) -> SnapshotGenerationId {
    let (_, id) = stage(store, sequence, panes);
    StorePort::seal_snapshot_stage(store, &id, &budget()).unwrap();
    StorePort::publish_snapshot_stage(store, &id, &budget()).unwrap();
    id
}

fn active(fx: &Fx) -> String {
    fx.db()
        .query_row(
            "SELECT active_snapshot_id FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn bounded_after_1000_publishes() {
    let fx = Fx::new("bounded-1000");
    const PANES: usize = 4;
    let mut first = None;
    for sequence in 1..=1_000u64 {
        let id = publish(&fx.store, sequence, PANES);
        first.get_or_insert(id);
        // The lane runs once a tick; observation publishes in between.
        if sequence % 10 == 0 {
            fx.drain();
        }
    }
    fx.drain();
    // Keep set: active, previous published, recovery baseline (the first
    // publication of the run).
    let keep = 3usize;
    let generations = fx.count("snapshot_generations") as usize;
    let targets = fx.count("snapshot_targets") as usize;
    assert!(generations <= keep + 1, "{generations} generations remain");
    assert!(targets <= (keep + 1) * PANES, "{targets} targets remain");
    assert!(
        fx.generations().contains(first.unwrap().as_str()),
        "the recovery baseline survives"
    );
    assert!(fx.generations().contains(&active(&fx)));
}

#[test]
fn every_pin_survives() {
    let fx = Fx::new("every-pin");
    {
        let db = fx.db();
        add_generation(&db, 1, "published", 1, 2); // old superseded: pruned
        add_generation(&db, 2, "published", 2, 2); // recovery baseline
        add_generation(&db, 3, "published", 3, 2); // unresolved-seat reference
        add_generation(&db, 4, "published", 4, 2); // previous published
        add_generation(&db, 5, "published", 5, 2); // active
        add_generation(&db, 6, "building", 7, 1); // in flight (above decided)
        add_generation(&db, 7, "sealed", 6, 1); // in flight (above decided)
        add_generation(&db, 8, "building", 4, 1); // dead stage: pruned
        add_generation(&db, 9, "discarded", 2, 1); // discarded: pruned
        add_generation(&db, 10, "sealed", 5, 1); // dead (== decided): pruned
        set_host(&db, 5, 7);
        set_pointers(&db, Some("g5"), Some("g2"));
        pin_by_unresolved_seat(&db, "s", "g3");
        db.execute_batch(
            "INSERT INTO recovery_baseline_releases(instance_id,baseline_generation_id,target_id,decision_seq) VALUES ('i','g2','t0',1),('i','g1','t0',1)",
        )
        .unwrap();
    }
    fx.drain();
    assert_eq!(
        fx.generations(),
        set(&["g2", "g3", "g4", "g5", "g6", "g7"]),
        "exactly the keep set remains"
    );
    // Survivors keep every target; the pruned generations' targets are gone.
    let targets = fx.ids("SELECT DISTINCT generation_id FROM snapshot_targets");
    assert_eq!(targets, set(&["g2", "g3", "g4", "g5", "g6", "g7"]));
    assert_eq!(fx.count("snapshot_targets"), 2 * 4 + 2);
    // The current baseline's release stays; a non-current baseline's goes.
    assert_eq!(
        fx.ids("SELECT baseline_generation_id FROM recovery_baseline_releases"),
        set(&["g2"])
    );
}

#[test]
fn reader_holding_previous_pointer_resolves_targets() {
    let fx = Fx::new("previous-pointer");
    const PANES: usize = 3;
    let mut previous = None;
    let mut newest = None;
    for sequence in 1..=6u64 {
        let id = publish(&fx.store, sequence, PANES);
        previous = newest.replace(id);
    }
    let previous = previous.unwrap();
    fx.drain();
    let published = active(&fx);
    assert_ne!(published, previous.as_str());
    let targets: i64 = fx
        .db()
        .query_row(
            "SELECT count(*) FROM snapshot_targets WHERE generation_id=?1",
            [previous.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        targets, PANES as i64,
        "previous published keeps its targets"
    );
    // The previous generation is still a complete, published generation.
    let status: String = fx
        .db()
        .query_row(
            "SELECT status FROM snapshot_generations WHERE id=?1",
            [previous.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "published");
}

#[test]
fn dead_and_discarded_stages_pruned_live_stage_kept() {
    let fx = Fx::new("dead-stages");
    {
        let db = fx.db();
        add_generation(&db, 1, "published", 3, 1);
        add_generation(&db, 2, "building", 4, 3); // dead
        add_generation(&db, 3, "sealed", 5, 3); // dead (== decided)
        add_generation(&db, 4, "discarded", 4, 3); // discarded
        add_generation(&db, 5, "building", 6, 3); // live: above decided
        set_host(&db, 5, 6);
        set_pointers(&db, Some("g1"), Some("g1"));
    }
    let total = fx.drain();
    assert_eq!(fx.generations(), set(&["g1", "g5"]));
    assert_eq!(total.generations, 3);
    assert_eq!(total.targets, 9);
    assert_eq!(fx.count("snapshot_targets"), 1 + 3);
}

#[test]
fn every_transaction_touches_at_most_256_rows() {
    let fx = Fx::new("row-budget");
    {
        let db = fx.db();
        // 40 discarded generations of 20 targets, plus one of 1,000 targets
        // that cannot fit one batch: a target row each, plus one row to delete
        // each generation.
        for n in 1..=40 {
            add_generation(&db, n, "discarded", n, 20);
        }
        add_generation(&db, 41, "discarded", 41, 1_000);
    }
    let expected_rows = 40 * 21 + 1_001;
    let mut changed_rows = 0u64;
    let mut passes = 0;
    loop {
        let before = fx.db().total_changes();
        let pass = prune_once(&fx.store, &budget()).unwrap();
        let delta = fx.db().total_changes() - before;
        assert!(
            delta <= RETENTION_BATCH_ROWS as u64,
            "{delta} rows in a pass"
        );
        changed_rows += delta;
        passes += 1;
        if !pass.has_more {
            break;
        }
    }
    assert!(passes >= 8, "the backlog needs several batches: {passes}");
    assert_eq!(changed_rows, expected_rows);
    assert_eq!(fx.count("snapshot_generations"), 0);
    assert_eq!(fx.count("snapshot_targets"), 0);

    // The same bound holds for the work-job transaction.
    {
        let db = fx.db();
        for n in 0..600 {
            db.execute(
                "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) VALUES (?1,'send_attention',?1,0,'complete',?2)",
                params![format!("w{n}"), T0 - 25 * HOUR_MS],
            )
            .unwrap();
        }
    }
    let mut deleted = 0u64;
    loop {
        let before = fx.db().total_changes();
        let pass = prune_once(&fx.store, &budget()).unwrap();
        let delta = fx.db().total_changes() - before;
        assert!(
            delta <= RETENTION_BATCH_ROWS as u64,
            "{delta} job rows in a pass"
        );
        deleted += delta;
        if !pass.has_more {
            break;
        }
    }
    assert_eq!(deleted, 600);
    assert_eq!(fx.count("work_jobs"), 0);
}

#[test]
fn backlog_of_10k_drains_while_a_request_write_completes_within_one_batch() {
    let fx = Fx::new("backlog-10k");
    {
        let db = fx.db();
        db.execute_batch("BEGIN").unwrap();
        for n in 1..=10_000 {
            db.execute(
                "INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,admission_sequence,created_at) VALUES (?1,'i','b',1,?2,'inc',0,0,'discarded',0,0,?2,0)",
                params![format!("g{n}"), n],
            )
            .unwrap();
        }
        db.execute_batch("COMMIT").unwrap();
    }
    let writer = Arc::new(FairWriter::new(32));
    let batches = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let drainer = {
        let (store, writer, batches, done) = (
            Arc::clone(&fx.store),
            Arc::clone(&writer),
            Arc::clone(&batches),
            Arc::clone(&done),
        );
        std::thread::spawn(move || {
            let _origin = crate::service::kicks::enter_lane(crate::service::kicks::Lane::Retention);
            loop {
                let more = {
                    let _turn = writer.enter_background(&budget(), store.clock()).unwrap();
                    let more = prune_once(&store, &budget()).unwrap().has_more;
                    batches.fetch_add(1, Ordering::SeqCst);
                    more
                };
                if !more {
                    break;
                }
                // Between turns, as the lane waits on its pacer.
                std::thread::sleep(Duration::from_millis(2));
            }
            done.store(true, Ordering::SeqCst);
        })
    };
    while batches.load(Ordering::SeqCst) < 3 {
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = batches.load(Ordering::SeqCst);
    {
        // A request write: foreground admission, then a real write.
        let _turn = writer
            .enter_foreground(&budget(), fx.store.clock())
            .unwrap();
        fx.db()
            .execute(
                "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'",
                [],
            )
            .unwrap();
    }
    let during = batches.load(Ordering::SeqCst) - before;
    assert!(
        !done.load(Ordering::SeqCst),
        "the request finished before the backlog did"
    );
    assert!(during <= 1, "the request waited for {during} batches");
    drainer.join().unwrap();
    assert_eq!(fx.count("snapshot_generations"), 0);
}

fn add_job(db: &Connection, id: &str, kind: &str, status: &str, completed_at: Option<i64>) {
    db.execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) VALUES (?1,?2,?1,0,?3,?4)",
        params![id, kind, status, completed_at],
    )
    .unwrap();
}

fn job_ids(fx: &Fx) -> BTreeSet<String> {
    fx.ids("SELECT id FROM work_jobs")
}

#[test]
fn pruned_job_kinds_after_24h_or_null() {
    let fx = Fx::new("jobs-24h");
    {
        let db = fx.db();
        for (n, kind) in PRUNED_JOB_KINDS.iter().enumerate() {
            add_job(
                &db,
                &format!("old{n}"),
                kind,
                "complete",
                Some(T0 - 25 * HOUR_MS),
            );
            add_job(
                &db,
                &format!("edge{n}"),
                kind,
                "complete",
                Some(T0 - 24 * HOUR_MS),
            );
            add_job(
                &db,
                &format!("recent{n}"),
                kind,
                "complete",
                Some(T0 - 23 * HOUR_MS),
            );
            add_job(&db, &format!("null{n}"), kind, "complete", None);
        }
    }
    let first = fx.drain();
    assert_eq!(
        first.jobs, 9,
        "old, exactly-24h and NULL rows of three kinds"
    );
    assert_eq!(job_ids(&fx), set(&["recent0", "recent1", "recent2"]));
    // Two hours later the 23 h rows have aged past the window.
    fx.advance(2 * HOUR_MS);
    assert_eq!(fx.drain().jobs, 3);
    assert!(job_ids(&fx).is_empty());
}

#[test]
fn preparation_cleanup_pending_failed_kept() {
    let fx = Fx::new("jobs-kept");
    {
        let db = fx.db();
        let old = Some(T0 - 100 * HOUR_MS);
        // Its completion row is the marker messages.rs reads.
        add_job(&db, "cleanup-done", "preparation_cleanup", "complete", old);
        add_job(&db, "cleanup-null", "preparation_cleanup", "complete", None);
        for (n, kind) in PRUNED_JOB_KINDS.iter().enumerate() {
            add_job(&db, &format!("pending{n}"), kind, "pending", None);
            add_job(&db, &format!("failed{n}"), kind, "failed", None);
        }
    }
    let total = fx.drain();
    assert_eq!(total.jobs, 0);
    assert_eq!(fx.count("work_jobs"), 8);
}

/// Seeds the cooperative fixture the producers need (see facade tests).
fn cooperative_fx(label: &str, seats: &[&str], bound: bool) -> Fx {
    let fx = Fx::new(label);
    {
        let db = fx.db();
        db.execute(
            "UPDATE host_instances SET host_boot='b',host_epoch=1 WHERE id='i'",
            [],
        )
        .unwrap();
        for seat in seats {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)", [seat]).unwrap();
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
            if !bound {
                continue;
            }
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,1,?1,'b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||?1,'inc')", [seat]).unwrap();
        }
    }
    fx
}

fn complete_all(fx: &Fx, kind: &str) -> i64 {
    fx.db()
        .execute(
            "UPDATE work_jobs SET status='complete',completed_at=?2 WHERE kind=?1",
            params![kind, T0],
        )
        .unwrap() as i64
}

fn kind_count(fx: &Fx, kind: &str) -> i64 {
    fx.db()
        .query_row(
            "SELECT count(*) FROM work_jobs WHERE kind=?1",
            [kind],
            |r| r.get(0),
        )
        .unwrap()
}

fn fixture_claim(seat: &str) -> crate::protocol::authority::CallerClaim {
    use crate::protocol::{authority::*, ids::*};
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(seat),
    }
}

fn permitted(
    store: &SqliteStore,
    mutation: crate::protocol::commands::PermitMutation,
) -> Result<crate::protocol::results::CommandResult, ApiError> {
    let request = cooperative_permit_request(&mutation)?;
    let permit = StorePort::issue_cooperative_permit(store, request, &budget())?;
    StorePort::mutate(store, mutation, permit, &budget())
}

#[test]
fn rerunning_send_attention_producer_does_not_reenqueue() {
    use crate::protocol::{
        commands::{CreateThread, Invite, PermitMutation, SendMessage},
        ids::{OperationId, SeatId},
        results::CommandResult,
    };
    let fx = cooperative_fx("producer-send", &["s1", "s2"], true);
    let store = &*fx.store;
    let CommandResult::ThreadCreated(thread) = permitted(
        store,
        PermitMutation::CreateThread(CreateThread {
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("create"),
            claim: fixture_claim("s1"),
        }),
    )
    .unwrap() else {
        panic!()
    };
    permitted(
        store,
        PermitMutation::Invite(Invite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            deadline_millis: Some(1000),
            operation: OperationId::new("invite"),
            claim: fixture_claim("s1"),
        }),
    )
    .unwrap();
    let send = SendMessage {
        thread,
        body: "hello".into(),
        invited_recipients: vec![SeatId::new("s2")],
        deadline_millis: None,
        operation: OperationId::new("send"),
        claim: fixture_claim("s1"),
        relays_user: false,
    };
    loop {
        match StorePort::prepare_send_step(
            store,
            &send,
            DurableWorkAdmission::new(16).unwrap(),
            &budget(),
        )
        .unwrap()
        {
            crate::ports::SendPreparationProgress::Ready { .. } => break,
            crate::ports::SendPreparationProgress::More { .. } => {}
            crate::ports::SendPreparationProgress::Committed(_) => panic!("committed early"),
        }
    }
    let sent = permitted(store, PermitMutation::SendMessage(send.clone())).unwrap();
    assert_eq!(kind_count(&fx, "send_attention"), 1);
    complete_all(&fx, "send_attention");
    fx.advance(25 * HOUR_MS);
    assert_eq!(fx.drain().jobs, 1);
    assert_eq!(kind_count(&fx, "send_attention"), 0);
    let replay = permitted(store, PermitMutation::SendMessage(send)).unwrap();
    assert_eq!(replay, sent, "the producer replays its stored result");
    assert_eq!(
        kind_count(&fx, "send_attention"),
        0,
        "the pruned job is not enqueued again"
    );
}

#[test]
fn rerunning_warning_attribution_producer_does_not_reenqueue() {
    let fx = Fx::new("producer-warning");
    {
        let db = fx.db();
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s2','i','resolved','native',1,0)",[]).unwrap();
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s2','invited')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s2',1,'pending',0,1,100,100)",[]).unwrap();
    }
    let scan = DueScanRequest {
        state: DueScanState::default(),
        max_candidates: 10,
        run_invitations: true,
        run_receipts: false,
    };
    let first = StorePort::due_obligations(&*fx.store, scan.clone(), &budget()).unwrap();
    assert_eq!(first.warnings_added, 1);
    assert_eq!(kind_count(&fx, "warning_attribution"), 1);
    complete_all(&fx, "warning_attribution");
    fx.advance(25 * HOUR_MS);
    assert_eq!(fx.drain().jobs, 1);
    assert_eq!(kind_count(&fx, "warning_attribution"), 0);
    let second = StorePort::due_obligations(&*fx.store, scan, &budget()).unwrap();
    assert_eq!(second.warnings_added, 0);
    assert_eq!(
        kind_count(&fx, "warning_attribution"),
        0,
        "the pruned job is not enqueued again"
    );
}

#[test]
fn rerunning_receipt_timer_producer_does_not_reenqueue() {
    use crate::{
        ports::{ReadContext, RegisterAvailableRequest},
        protocol::{
            commands::{CheckIn, PermitMutation},
            ids::OperationId,
            output::OutputSpec,
        },
    };
    let fx = cooperative_fx("producer-receipt", &["s1"], false);
    let command = CheckIn {
        mode: crate::protocol::commands::CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
        claim: fixture_claim("s1"),
        operation: OperationId::new("check-in"),
    };
    let register = |fx: &Fx| {
        let permit = StorePort::issue_cooperative_permit(
            &*fx.store,
            cooperative_permit_request(&PermitMutation::CheckIn(command.clone())).unwrap(),
            &budget(),
        )
        .unwrap();
        StorePort::register_available(
            &*fx.store,
            RegisterAvailableRequest {
                command: command.clone(),
                read: ReadContext {
                    instance: "i".into(),
                    output: OutputSpec::default(),
                    operation_scope: None,
                },
                operator: None,
            },
            permit,
            &budget(),
        )
    };
    register(&fx).unwrap();
    assert_eq!(kind_count(&fx, "receipt_timer_materialization"), 1);
    complete_all(&fx, "receipt_timer_materialization");
    fx.advance(25 * HOUR_MS);
    assert_eq!(fx.drain().jobs, 1);
    assert_eq!(kind_count(&fx, "receipt_timer_materialization"), 0);
    // The same check-in again: availability did not change, so no new anchor.
    register(&fx).expect("the producer replays");
    assert_eq!(
        kind_count(&fx, "receipt_timer_materialization"),
        0,
        "the pruned job is not enqueued again"
    );
}

#[test]
fn pins_never_stall_pruning() {
    let fx = Fx::new("pins-stall");
    const PINNED: i64 = RETENTION_BATCH_ROWS as i64 + 44;
    const PRUNABLE: i64 = 50;
    {
        let db = fx.db();
        db.execute_batch("BEGIN").unwrap();
        // 300 old published generations, each pinned by an unresolved seat,
        // sort before the prunable ones in scan order.
        for n in 1..=PINNED {
            let id = add_generation(&db, n, "published", n, 0);
            pin_by_unresolved_seat(&db, &format!("s{n}"), &id);
        }
        for n in 1..=PRUNABLE {
            add_generation(&db, PINNED + n, "discarded", PINNED + n, 0);
        }
        let top = PINNED + PRUNABLE;
        add_generation(&db, top + 1, "published", top + 1, 0);
        add_generation(&db, top + 2, "published", top + 2, 0);
        db.execute_batch("COMMIT").unwrap();
        set_host(&db, top + 2, top + 2);
        set_pointers(&db, Some(&format!("g{}", top + 2)), Some("g1"));
    }
    let total = fx.drain();
    assert_eq!(total.generations as i64, PRUNABLE);
    assert_eq!(
        fx.count("snapshot_generations"),
        PINNED + 2,
        "every pinned generation and the active and previous remain"
    );
}

#[test]
fn retention_pass_is_flat_when_nothing_qualifies() {
    // Pinned generations (unresolved-seat references) and retained
    // preparation_cleanup rows grow 10x; the steady-state pass does not.
    fn measure(scale: i64) -> u64 {
        let fx = Fx::new("flat");
        {
            let db = fx.db();
            db.execute_batch("BEGIN").unwrap();
            for n in 1..=scale {
                let id = add_generation(&db, n, "published", n, 0);
                pin_by_unresolved_seat(&db, &format!("s{n}"), &id);
                add_job(
                    &db,
                    &format!("c{n}"),
                    "preparation_cleanup",
                    "complete",
                    Some(T0 - 100 * HOUR_MS),
                );
            }
            add_generation(&db, scale + 1, "published", scale + 1, 0);
            add_generation(&db, scale + 2, "published", scale + 2, 0);
            db.execute_batch("COMMIT").unwrap();
            set_host(&db, scale + 2, scale + 2);
            set_pointers(&db, Some(&format!("g{}", scale + 2)), Some("g1"));
        }
        // The first pass finds nothing and records that the tables are
        // unchanged; the measured pass is the idle steady state.
        assert_eq!(fx.drain(), PruneProgress::default());
        let counter = Arc::new(CostCounter::default());
        fx.store.set_retention_cost_counter(Some(counter.clone()));
        assert_eq!(
            prune_once(&fx.store, &budget()).unwrap(),
            PruneProgress::default()
        );
        counter.units()
    }
    let small = measure(100);
    let large = measure(1_000);
    eprintln!("retention idle pass vm units/10: scale 100 = {small}, scale 1000 = {large}");
    assert!(small > 0, "the work-job probes are measured");
    assert!(
        large as f64 <= small as f64 * 1.1 + 10.0,
        "idle pass grew with retained rows: {small} -> {large}"
    );
}

#[test]
fn candidate_and_job_queries_use_the_retention_indexes() {
    let fx = Fx::new("plans");
    let db = fx.db();
    for (name, sql) in [
        (
            "snapshot_generations_retention",
            scan_sql().replace("?1", "256"),
        ),
        (
            "work_jobs_retention",
            DUE_JOBS_SQL
                .replace("?1", "'send_attention'")
                .replace("?2", "5"),
        ),
        (
            "work_jobs_retention",
            DELETE_JOBS_SQL
                .replace("?1", "'send_attention'")
                .replace("?2", "5")
                .replace("?3", "256"),
        ),
    ] {
        let plan: Vec<String> = db
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let joined = plan.join("\n");
        assert!(joined.contains(name), "{name} unused:\n{joined}");
        assert!(
            !joined
                .lines()
                .any(|l| l.contains("SCAN work_jobs") || l.contains("SCAN seats")),
            "full scan:\n{joined}"
        );
    }
}

#[test]
fn triggers_allow_discard_then_delete() {
    let fx = Fx::new("triggers");
    // The audit: no trigger is defined on a table retention updates or
    // deletes from. The two host_instances pointer triggers guard updates
    // retention never makes; every trigger is still installed.
    let triggers: Vec<(String, String)> = {
        let db = fx.db();
        let mut stmt = db
            .prepare("SELECT name,tbl_name FROM sqlite_master WHERE type='trigger'")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(
        triggers.len() >= 30,
        "all triggers installed: {}",
        triggers.len()
    );
    let touched = [
        "snapshot_generations",
        "snapshot_targets",
        "recovery_baseline_releases",
        "work_jobs",
    ];
    let on_touched: Vec<_> = triggers
        .iter()
        .filter(|(_, table)| touched.contains(&table.as_str()))
        .collect();
    assert!(
        on_touched.is_empty(),
        "triggers on pruned tables: {on_touched:?}"
    );
    for pointer in [
        "host_active_snapshot_published",
        "host_baseline_snapshot_published",
    ] {
        assert!(
            triggers
                .iter()
                .any(|(name, table)| name == pointer && table == "host_instances")
        );
    }
    // And a real superseded generation is discarded then deleted with every
    // trigger live and foreign keys on.
    let foreign_keys: i64 = fx
        .db()
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    for sequence in 1..=4u64 {
        publish(&fx.store, sequence, 2);
    }
    let total = fx.drain();
    assert_eq!(total.generations, 1, "one superseded generation");
    assert_eq!(fx.count("snapshot_generations"), 3);
}

#[test]
fn work_job_fk_trigger_audit() {
    let fx = Fx::new("job-fk");
    let digests = [
        "digest_notice_offer",
        "digest_open_warning_recipients",
        "digest_open_warnings",
        "digest_pending_invitations",
        "digest_pending_manifest_receipts",
        "digest_programmatic_warnings",
    ];
    let foreign_keys: i64 = fx
        .db()
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    {
        let db = fx.db();
        for (n, kind) in PRUNED_JOB_KINDS.iter().enumerate() {
            add_job(
                &db,
                &format!("w{n}"),
                kind,
                "complete",
                Some(T0 - 30 * HOUR_MS),
            );
        }
    }
    let dump = |fx: &Fx| -> Vec<(String, i64)> {
        digests
            .iter()
            .map(|t| (t.to_string(), fx.count(t)))
            .collect()
    };
    let before_digests = dump(&fx);
    let before_changes = fx.db().total_changes();
    assert_eq!(fx.drain().jobs, 3);
    // total_changes counts trigger-fired changes too: exactly the three
    // deleted jobs changed, so no digest projection row did.
    assert_eq!(fx.db().total_changes() - before_changes, 3);
    assert_eq!(dump(&fx), before_digests);
    assert_eq!(fx.count("work_jobs"), 0);
}

#[test]
fn herdr_down_discarded_stages_are_pruned() {
    let fx = Fx::new("herdr-down");
    const PANES: usize = 20;
    // Herdr is down: every observation cycle stages a capture that cannot be
    // published, is invalidated, and (on alternate cycles) cleaned up by the
    // lane's 16-row discard step, which leaves the generation row behind.
    for cycle in 1..=300u64 {
        let (admission, stage_id) = stage(&fx.store, cycle, PANES);
        StorePort::invalidate_host_observation(
            &*fx.store,
            &admission,
            HostInvalidationReason::PublicationFailed,
            &budget(),
        )
        .unwrap();
        if cycle % 2 == 0 {
            StorePort::discard_snapshot_stage(
                &*fx.store,
                &stage_id,
                DurableWorkAdmission::new(16).unwrap(),
                &budget(),
            )
            .unwrap();
        }
        if cycle % 25 == 0 {
            fx.drain();
            assert_eq!(fx.count("snapshot_generations"), 0, "cycle {cycle}");
            assert_eq!(fx.count("snapshot_targets"), 0, "cycle {cycle}");
        }
    }
    assert_eq!(fx.count("snapshot_generations"), 0);
}

#[test]
fn nothing_qualifying_opens_no_write_transaction() {
    let fx = Fx::new("no-write");
    publish(&fx.store, 1, 2);
    publish(&fx.store, 2, 2);
    let before = fx.store.commit_counts();
    for _ in 0..3 {
        assert_eq!(
            prune_once(&fx.store, &budget()).unwrap(),
            PruneProgress::default()
        );
    }
    assert_eq!(
        fx.store.commit_counts(),
        before,
        "an empty pass commits nothing"
    );
    // A row another connection commits lifts the skip too.
    {
        let other = Connection::open(&fx.path).unwrap();
        other.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        add_generation(&other, 900, "discarded", 900, 2);
    }
    let pruned = prune_once(&fx.store, &budget()).unwrap();
    assert_eq!((pruned.generations, pruned.targets), (1, 2));
    // A new publication makes the superseded one prunable: the skip is lifted.
    publish(&fx.store, 3, 2);
    publish(&fx.store, 4, 2);
    assert_eq!(prune_once(&fx.store, &budget()).unwrap().generations, 1);
}

#[test]
fn abandoned_preparations_expire_but_keep_exact_key_headers() {
    let fx = Fx::new("preparation-retention");
    let db = fx.db();
    assert!(db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('send_preparations') WHERE name='prepared_at')", [], |r| r.get::<_, bool>(0)).unwrap(), "preparation progress must be persisted");
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    for (id, status) in [("building", "building"), ("sealed", "sealed")] {
        db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,prepared_at) VALUES (?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,?2,?3)", params![id,status,T0]).unwrap();
    }
    drop(db);
    fx.advance(24 * HOUR_MS - 1);
    fx.drain();
    assert_eq!(
        fx.count("work_jobs"),
        0,
        "within-window retry must remain resumable"
    );
    fx.advance(1);
    fx.drain();
    assert_eq!(
        fx.count("send_preparations"),
        2,
        "digest headers are tombstones"
    );
    assert_eq!(fx.count("work_jobs"), 2);
    assert_eq!(
        fx.ids("SELECT id FROM send_preparations WHERE status='discarded'"),
        BTreeSet::from(["building".into(), "sealed".into()])
    );
    fx.drain();
    assert_eq!(fx.count("work_jobs"), 2, "cleanup markers are retained");
}

fn seed_preparation(db: &Connection, id: &str, at: i64) {
    db.execute("INSERT OR IGNORE INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,prepared_at) VALUES (?1,'i','scope',?1,zeroblob(32),'t',0,0,0,0,0,0,0,'sealed',?2)",params![id,at]).unwrap();
}

fn publish_preparation(db: &Connection, id: &str) {
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,event_offset) VALUES (?1,'i','t',1,'ordinary','body',0,1,0)", [id]).unwrap();
    db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES (?1,?1,'i','t',1,0,1,0,0,0)", [id]).unwrap();
}

#[test]
fn expiry_rechecks_progress_publication_generation_status_and_age() {
    for race in ["progress", "publication", "generation", "status", "clock"] {
        let fx = Fx::new("expiry-canonical-recheck");
        let mut db = fx.db();
        let at = T0 - PREPARATION_RETENTION_MS;
        seed_preparation(&db, "p", at);
        let found = preparation_candidates(&db, at).unwrap();
        assert_eq!(found.len(), 1);
        match race {
            // Keep the replacement time old enough to qualify: equality itself
            // must fence a quantum that raced the scan.
            "progress" => {
                db.execute("UPDATE send_preparations SET prepared_at=prepared_at-1", [])
                    .unwrap();
            }
            "publication" => publish_preparation(&db, "p"),
            "generation" => {
                db.execute("UPDATE send_preparations SET id='successor'", [])
                    .unwrap();
            }
            "status" => {
                db.execute("UPDATE send_preparations SET status='building'", [])
                    .unwrap();
            }
            "clock" => fx.advance(-1),
            _ => unreachable!(),
        }
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            discard_expired_preparations(&tx, &found, fx.clock.as_ref(), &budget())
                .unwrap()
                .0,
            0,
            "{race}"
        );
        tx.rollback().unwrap();
        drop(db);
        assert_eq!(fx.count("work_jobs"), 0, "{race} must not enqueue cleanup");
    }
}

#[test]
fn expiry_batch_bounds_include_cleanup_enqueue_and_scan_uses_index() {
    let fx = Fx::new("expiry-bounded-indexed");
    let db = fx.db();
    for n in 0..300 {
        seed_preparation(&db, &format!("p{n}"), T0 - PREPARATION_RETENTION_MS);
    }
    let plan: String = db
        .prepare(&format!("EXPLAIN QUERY PLAN {PREPARATION_CANDIDATES_SQL}"))
        .unwrap()
        .query_map(params![T0, 128], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    assert!(
        plan.contains("USING INDEX send_preparations_retention"),
        "{plan}"
    );
    let before = db.total_changes();
    drop(db);
    let pass = prune_once(&fx.store, &budget()).unwrap();
    assert_eq!(pass.preparations, 128);
    assert!(pass.has_more);
    assert_eq!(fx.db().total_changes() - before, 256);
    assert_eq!(fx.drain().preparations, 172);
}

#[test]
fn preparation_expiry_stops_on_quantum_and_cancelled_decision() {
    struct Tick(AtomicU64);
    impl Clock for Tick {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(T0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.fetch_add(1, Ordering::SeqCst))
        }
    }
    let fx = Fx::new("expiry-quantum");
    let mut db = fx.db();
    for n in 0..10 {
        seed_preparation(&db, &format!("p{n}"), T0 - PREPARATION_RETENTION_MS);
    }
    let found = preparation_candidates(&db, T0 - PREPARATION_RETENTION_MS).unwrap();
    let tx = db.transaction().unwrap();
    let (discarded, more) =
        discard_expired_preparations(&tx, &found, &Tick(AtomicU64::new(0)), &budget()).unwrap();
    assert!(discarded > 0 && discarded < 10);
    assert!(more);
    tx.rollback().unwrap();
    let cancelled = budget();
    cancelled.cancellation.cancel();
    let tx = db.transaction().unwrap();
    assert_eq!(
        discard_expired_preparations(&tx, &found, fx.clock.as_ref(), &cancelled)
            .unwrap_err()
            .code,
        crate::protocol::results::ErrorCode::Cancelled
    );
}

#[test]
fn discard_of_published_or_missing_generation_does_not_enqueue_cleanup() {
    let fx = Fx::new("discard-published-fence");
    let mut db = fx.db();
    seed_preparation(&db, "p", T0 - PREPARATION_RETENTION_MS);
    publish_preparation(&db, "p");
    let tx = db.transaction().unwrap();
    super::super::messages::discard_preparation(&tx, "p").unwrap();
    super::super::messages::discard_preparation(&tx, "missing").unwrap();
    assert_eq!(
        tx.query_row("SELECT count(*) FROM work_jobs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn idle_preparation_scan_cost_is_flat_with_retained_tombstones() {
    fn measure(scale: usize) -> u64 {
        let fx = Fx::new("preparation-index-cost");
        let db = fx.db();
        db.execute_batch("BEGIN").unwrap();
        for n in 0..scale {
            seed_preparation(&db, &format!("p{n}"), T0);
        }
        // Published preparations have NULL progress; discarded ones retain
        // their digest and completion marker but leave the partial index.
        db.execute("UPDATE send_preparations SET prepared_at=CASE WHEN rowid%2=0 THEN NULL ELSE prepared_at END,status=CASE WHEN rowid%2=0 THEN 'sealed' ELSE 'discarded' END",[]).unwrap();
        seed_preparation(&db, "live", T0);
        db.execute_batch("COMMIT").unwrap();
        drop(db);
        assert_eq!(fx.drain(), PruneProgress::default());
        let counter = Arc::new(CostCounter::default());
        fx.store.set_retention_cost_counter(Some(counter.clone()));
        let before = fx.db().total_changes();
        assert_eq!(
            prune_once(&fx.store, &budget()).unwrap(),
            PruneProgress::default()
        );
        assert_eq!(fx.db().total_changes(), before);
        counter.units()
    }
    let small = measure(100);
    let large = measure(1000);
    assert!(small > 0);
    assert!(
        large <= small + 10,
        "expiry scan grew with history: {small} -> {large}"
    );
}
