//! The observation lane loop on its Pacer (pacer spec D4, ht-p03.9.5): the
//! real `spawn_observation_loop` over a real `SqliteStore`, a fake host and a
//! fake clock that the tests advance (and announce with
//! `Pacer::clock_advanced`).
use super::*;
use crate::identity::repair::OrdinaryIdentity;
use crate::ports::*;
use crate::protocol::{
    commands::ResolveSeat,
    ids::*,
    results::{ApiError, ErrorCode},
    time::{Cancellation, Clock, MonoInstant, UtcMillis},
};
use crate::service::host_evidence::HostEvidenceStatus;
use crate::service::host_reachability::HostReachability;
use crate::service::kicks::{Lane, LaneSet};
use crate::store::{SqliteStore, StoreSettings, connection::StoreContext};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

const START: u64 = 10_000;

struct LaneClock(AtomicU64);
impl Clock for LaneClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = crate::test_support::scratch::temp_path(format!(
            "herdr-obs-lane-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One empty pane. While `down` is set every host call fails.
struct LaneHost {
    clock: Arc<LaneClock>,
    sequence: AtomicU64,
    down: AtomicBool,
    snapshots: AtomicU64,
    shell_reads: AtomicU64,
}
impl LaneHost {
    fn observation(&self, sequence: u64) -> HostObservation {
        let at = self.clock.monotonic_now();
        HostObservation {
            target: HostTargetId::new("pane"),
            focused: false,
            host_boot: HostBootId::new("host"),
            epoch: 1,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: at,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new("terminal")),
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: at,
            completed_at_mono: at,
        }
    }
    fn down_error() -> ApiError {
        ApiError::new(ErrorCode::HostUnavailable, "herdr is stopped")
    }
}
impl HostPort for LaneHost {
    fn live_topology(
        &self,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::host::observation::HostTopology, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "fixture host has no topology",
        ))
    }
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        if self.down.load(Ordering::SeqCst) {
            return Err(Self::down_error());
        }
        Ok(self.observation(self.sequence.fetch_add(1, Ordering::SeqCst)))
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        if self.down.load(Ordering::SeqCst) {
            return Err(Self::down_error());
        }
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        Ok(HostSnapshot {
            boot: HostBootId::new("host"),
            epoch: 1,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: vec![self.observation(sequence)],
        })
    }
    fn pane_shell_pid(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        self.shell_reads.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        unreachable!()
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        unreachable!()
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
}

type Kicked = Arc<Mutex<Vec<(LaneSet, Option<Lane>)>>>;

struct Rig {
    _dir: Scratch,
    db: std::path::PathBuf,
    clock: Arc<LaneClock>,
    store: Arc<SqliteStore>,
    pacer: Arc<Pacer>,
    host: Arc<LaneHost>,
    status: Arc<WorkerStatus>,
    evidence: Arc<HostEvidenceStatus>,
    reachability: Arc<HostReachability>,
    identity: Arc<OrdinaryIdentity>,
    instance: String,
    cancel: Cancellation,
    kicked: Kicked,
    worker: Option<thread::JoinHandle<()>>,
}
impl Rig {
    fn new() -> Self {
        let dir = Scratch::new();
        let clock = Arc::new(LaneClock(AtomicU64::new(START)));
        let instance = crate::node::NodeUuid::new_v4().to_string();
        let db = dir.0.join("db");
        let store = SqliteStore::new(
            StoreContext::new(db.clone(), clock.clone())
                .with_test_host()
                .with_node_seed(crate::test_support::node_seed_for(&instance)),
            instance.clone(),
            StoreSettings::default(),
        )
        .unwrap();
        let kicked: Kicked = Arc::default();
        let sink = Arc::clone(&kicked);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            sink.lock().unwrap().push((lanes, origin));
        }));
        let store = Arc::new(store);
        let cancel = Cancellation::default();
        let pacer = Arc::new(Pacer::new("observation", clock.clone(), cancel.clone()));
        let host = Arc::new(LaneHost {
            clock: clock.clone(),
            sequence: AtomicU64::new(1),
            down: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
            shell_reads: AtomicU64::new(0),
        });
        let writer = Arc::new(FairWriter::new(32));
        let identity = Arc::new(
            OrdinaryIdentity::new(
                instance.clone(),
                crate::test_support::test_host_id(),
                store.clone(),
                host.clone(),
                clock.clone(),
                writer,
            )
            .with_observation_pacer(pacer.clone()),
        );
        Self {
            _dir: dir,
            db,
            clock,
            store,
            pacer,
            host,
            status: Arc::new(WorkerStatus::default()),
            evidence: Arc::new(HostEvidenceStatus::default()),
            reachability: Arc::new(HostReachability::default()),
            identity,
            instance,
            cancel,
            kicked,
            worker: None,
        }
    }

    fn start(&mut self) {
        let port = ScheduledStore::new(self.store.clone(), Arc::new(FairWriter::new(32)));
        self.start_on(port);
    }

    fn start_on<S>(&mut self, port: S)
    where
        S: crate::identity::reconcile::observation_store::ObservationStore + 'static,
    {
        self.worker = Some(
            spawn_observation_loop(
                self.identity.clone(),
                port,
                self.cancel.clone(),
                self.status.clone(),
                self.evidence.clone(),
                self.pacer.clone(),
                self.reachability.clone(),
            )
            .unwrap(),
        );
    }

    fn advance(&self, ms: u64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
        self.pacer.clock_advanced();
    }

    fn snapshots(&self) -> u64 {
        self.host.snapshots.load(Ordering::SeqCst)
    }

    fn observation_commits(&self) -> u64 {
        self.store.commit_counts()["observation"]
    }

    fn wait(&self, what: &str, done: &dyn Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until the lane has finished a pass and blocked again.
    fn wait_idle(&self, events: u64) {
        self.wait("the lane to block in its Pacer", &|| {
            self.pacer.idle_events() >= events
        });
    }

    fn stop(&mut self) -> Duration {
        let at = Instant::now();
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        at.elapsed()
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[test]
fn cancellation_stops_lane_in_backoff_within_20ms() {
    let mut rig = Rig::new();
    // Drive the Pacer to its 30 s cap, then fail the lane's first capture.
    for _ in 0..12 {
        rig.pacer.on_failure();
    }
    rig.host.down.store(true, Ordering::SeqCst);
    rig.start();
    rig.wait("the first failed capture", &|| rig.snapshots() == 1);
    rig.wait_idle(1);
    let (attempts, next) = rig.status.retry().expect("retrying");
    assert!(attempts >= 13, "attempts {attempts}");
    let remaining = next.0 - rig.clock.monotonic_now().0;
    assert!(
        (24_000..=30_000).contains(&remaining),
        "backoff wait is the capped ~30 s, got {remaining} ms"
    );
    // Still blocked in the wait: no further capture without time passing.
    thread::sleep(Duration::from_millis(50));
    assert_eq!(rig.snapshots(), 1);
    let stopped_in = rig.stop();
    assert!(
        stopped_in < Duration::from_millis(20),
        "cancel took {stopped_in:?}"
    );
}

/// An explicit target read (the operator's first contact after a Herdr
/// restart) forces one lane capture even while the lane backs off; kill: a
/// lane that ignores explicit reads while backing off waits out up to 30 s.
#[test]
fn explicit_target_read_during_backoff_runs_one_capture() {
    let mut rig = Rig::new();
    for _ in 0..12 {
        rig.pacer.on_failure();
    }
    rig.host.down.store(true, Ordering::SeqCst);
    rig.start();
    rig.wait("the first failed capture", &|| rig.snapshots() == 1);
    rig.wait_idle(1);
    let read = |rig: &Rig, op: &str| {
        let _ = rig.identity.resolve(
            ResolveSeat {
                host: crate::test_support::test_host_locator(),
                target: HostTargetId::new("pane"),
                operation: OperationId::new(op),
            },
            &CallBudget {
                deadline: MonoInstant(START + 60_000),
                cancellation: Cancellation::default(),
            },
        );
    };
    // (a) Host still down: the read forces exactly one more capture, which
    // fails and counts as the next backoff step; then the lane is back in
    // its backoff wait without any clock advance.
    rig.wait("the backoff to settle", &|| rig.status.retry().is_some());
    let attempts = rig.pacer.attempts();
    read(&rig, "op-1");
    rig.wait("the forced capture", &|| rig.snapshots() == 2);
    rig.wait_idle(2);
    assert_eq!(rig.pacer.attempts(), attempts + 1);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(
        rig.snapshots(),
        2,
        "still backing off after the forced pass"
    );
    // (b) Host back: the next explicit read forces a capture that publishes.
    rig.host.down.store(false, Ordering::SeqCst);
    read(&rig, "op-2");
    rig.wait("the publishing capture", &|| {
        rig.snapshots() >= 3 && rig.pacer.attempts() == 0
    });
    assert!(rig.status.retry().is_none(), "success clears the retry");
}

/// The lane is the host-reachability writer (ht-72q): a capture frozen for
/// unavailability marks the host down without kicking the wake lane, and the
/// first capture Herdr answers marks it up and kicks the wake lane once.
/// Kills: a lane that never reports the outage (the wake lane keeps
/// attempting the dead host), and a recovery that leaves the wake lane to its
/// safety tick.
#[test]
fn frozen_capture_marks_host_down_and_publication_kicks_wake_lane() {
    let mut rig = Rig::new();
    let wake = Arc::new(Pacer::new("wake", rig.clock.clone(), rig.cancel.clone()));
    rig.reachability.attach_wake_pacer(wake.clone());
    rig.host.down.store(true, Ordering::SeqCst);
    rig.start();
    rig.wait("the frozen capture", &|| rig.snapshots() == 1);
    rig.wait_idle(1);
    assert!(
        rig.reachability.is_down(),
        "a frozen capture marks the host down"
    );
    assert_eq!(rig.reachability.recoveries(), 0);
    // Herdr is back: the next capture (after the 100 ms-ish backoff) publishes.
    rig.host.down.store(false, Ordering::SeqCst);
    rig.advance(1_000);
    rig.wait("the publishing capture", &|| !rig.reachability.is_down());
    assert_eq!(rig.reachability.recoveries(), 1, "one recovery");
    // The wake lane's Pacer holds the latched recovery kick.
    assert_eq!(
        wake.wait_blocking(Duration::from_secs(5)),
        crate::service::pacer::Wake::Kicked,
        "recovery kicked the wake lane"
    );
    // Steady publications are no transition: no further kick. The lane
    // must be blocked again before the clock moves, or the move is missed.
    rig.wait_idle(2);
    rig.advance(5_000);
    rig.wait("another publication", &|| rig.snapshots() >= 3);
    rig.wait_idle(3);
    assert_eq!(rig.reachability.recoveries(), 1, "no recovery while up");
}

#[test]
fn idle_herdr_up_makes_at_most_one_cycle_per_5s() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    assert_eq!(rig.snapshots(), 1);
    let after_first = rig.observation_commits();
    assert!(after_first > 0, "the first cycle publishes durably");

    // Under 5 s of clock: no capture, no commit, and the lane stays blocked.
    rig.advance(4_999);
    thread::sleep(Duration::from_millis(60));
    assert_eq!(rig.snapshots(), 1);
    assert_eq!(rig.observation_commits(), after_first);
    assert_eq!(rig.pacer.idle_events(), 1, "no spurious wake before 5 s");

    // Each 5 s step runs exactly one cycle with a fixed commit cost.
    rig.advance(1);
    rig.wait_idle(2);
    assert_eq!(rig.snapshots(), 2);
    let second = rig.observation_commits() - after_first;
    rig.advance(5_000);
    rig.wait_idle(3);
    assert_eq!(rig.snapshots(), 3);
    let third = rig.observation_commits() - after_first - second;
    assert!(
        (1..=6).contains(&second),
        "one cycle is admission, begin, stage, seal, publish (+ pages): {second}"
    );
    assert_eq!(third, second, "steady cost per 5 s step");
    assert_eq!(rig.pacer.attempts(), 0, "a healthy lane never backs off");
}

#[test]
fn unchanged_host_cycle_commits_touch_no_wakes_mapped_table() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    // A saved seat bound to the pane makes the reconciliation page non-empty.
    let seat = rig
        .identity
        .resolve(
            ResolveSeat {
                host: crate::test_support::test_host_locator(),
                target: HostTargetId::new("pane"),
                operation: OperationId::new("op-1"),
            },
            &CallBudget {
                deadline: MonoInstant(START + 60_000),
                cancellation: Cancellation::default(),
            },
        )
        .expect("seat resolved");
    let db = rusqlite::Connection::open(&rig.db).unwrap();
    let seats: i64 = db
        .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seats, 1, "seat {seat:?} exists for the reconciliation page");

    // The explicit target read kicks the lane: one dirty-refresh cycle sees
    // the new seat (a real change, so it may kick the wake lane). Settle it,
    // then one more 5 s cycle, before measuring a pure unchanged cycle.
    rig.wait("the kick-driven refresh cycle", &|| {
        rig.snapshots() >= 2 && rig.pacer.idle_events() >= 2
    });
    let cycle = |target: u64| {
        let idle = rig.pacer.idle_events();
        rig.advance(5_000);
        rig.wait("a 5 s cycle", &|| {
            rig.snapshots() >= target && rig.pacer.idle_events() > idle
        });
    };
    cycle(3);
    rig.kicked.lock().unwrap().clear();
    let before = rig.observation_commits();
    cycle(4);
    assert_eq!(rig.snapshots(), 4);
    assert!(
        rig.observation_commits() > before,
        "the unchanged cycle still commits (admission, stage, publish)"
    );
    let kicked = rig.kicked.lock().unwrap().clone();
    assert!(
        kicked.iter().all(|(lanes, _)| !lanes.contains(Lane::Wakes)),
        "an unchanged-host observation cycle kicked the wake lane: {kicked:?}"
    );
}

/// The real scheduled store, except that it refuses every transition of `seat`.
struct RefusingPort {
    inner: ScheduledStore,
    seat: SeatId,
    refused: Arc<AtomicU64>,
}
impl crate::identity::reconcile::observation_store::ObservationStore for RefusingPort {
    fn clock(&self) -> &dyn Clock {
        crate::identity::reconcile::observation_store::ObservationStore::clock(&self.inner)
    }
    fn begin_host_observation(
        &self,
        instance: &str,
        host: HostId,
        budget: &CallBudget,
    ) -> Result<HostObservationAdmission, ApiError> {
        self.inner.begin_host_observation(instance, host, budget)
    }
    fn invalidate_host_observation(
        &self,
        admission: &HostObservationAdmission,
        reason: HostInvalidationReason,
        budget: &CallBudget,
    ) -> Result<Option<HostInvalidationFence>, ApiError> {
        self.inner
            .invalidate_host_observation(admission, reason, budget)
    }
    fn mark_unresolved_from_invalidation(
        &self,
        transition: GuardedInvalidationTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        self.inner
            .mark_unresolved_from_invalidation(transition, budget)
    }
    fn saved_seats_page_for_invalidation(
        &self,
        fence: &HostInvalidationFence,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<InvalidationSeatPage, ApiError> {
        self.inner.saved_seats_page_for_invalidation(
            fence,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )
    }
    fn begin_snapshot_stage(
        &self,
        header: SnapshotHeader,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        self.inner.begin_snapshot_stage(header, budget)
    }
    fn stage_snapshot_targets(
        &self,
        stage: &SnapshotGenerationId,
        offset: u64,
        targets: &[HostObservation],
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotStageProgress, ApiError> {
        self.inner
            .stage_snapshot_targets(stage, offset, targets, admission, budget)
    }
    fn seal_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        self.inner.seal_snapshot_stage(stage, budget)
    }
    fn publish_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<PublishedSnapshot, ApiError> {
        self.inner.publish_snapshot_stage(stage, budget)
    }
    fn discard_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotCleanupProgress, ApiError> {
        self.inner.discard_snapshot_stage(stage, admission, budget)
    }
    fn saved_seats_page(
        &self,
        published: &SnapshotGenerationId,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<SnapshotSeatPage, ApiError> {
        self.inner
            .saved_seats_page(published, after_ordinal, high_water_ordinal, limit, budget)
    }
    fn apply_reconciliation_transition(
        &self,
        transition: GuardedSeatTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        if transition.seat == self.seat {
            self.refused.fetch_add(1, Ordering::SeqCst);
            return Ok(ReconciliationOutcome::Stale(StaleReason::new(
                "forced_refusal",
            )));
        }
        self.inner
            .apply_reconciliation_transition(transition, budget)
    }
    fn record_reconciliation_pass(
        &self,
        published: &PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.inner.record_reconciliation_pass(published, budget)
    }
}

/// ht-wp7n: one seat refused on every pass, on a real store and the real lane.
struct RefusedPass {
    rig: Rig,
    db: rusqlite::Connection,
    refused: Arc<AtomicU64>,
}
impl RefusedPass {
    /// Seeds seat `culprit`, resolved under `incarnation`, at an absent target.
    /// Under the published incarnation the pass plans a retirement; under any
    /// other it plans `MarkUnresolved`. `seed` adds to the seeded store.
    fn start(incarnation: &str, seed: impl FnOnce(&rusqlite::Connection, &str)) -> Self {
        let mut rig = Rig::new();
        let db = rusqlite::Connection::open(&rig.db).unwrap();
        db.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence) \
             VALUES ('culprit',?1,'resolved','native','gone',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'term-gone',?2,'coherent_enumeration','host',1,1,1)",
            rusqlite::params![rig.instance, incarnation],
        )
        .unwrap();
        seed(&db, &rig.instance);
        let refused = Arc::new(AtomicU64::new(0));
        rig.start_on(RefusingPort {
            inner: ScheduledStore::new(rig.store.clone(), Arc::new(FairWriter::new(32))),
            seat: SeatId::new("culprit"),
            refused: refused.clone(),
        });
        rig.wait_idle(1);
        Self { rig, db, refused }
    }

    /// Runs passes until `count` have refused the seat, each finished.
    fn run_to(&self, count: u64) {
        assert_eq!(self.refused.load(Ordering::SeqCst) + 1, count);
        let idle = self.rig.pacer.idle_events();
        self.rig.advance(5_000);
        self.rig.wait_idle(idle + 1);
        assert_eq!(self.refused.load(Ordering::SeqCst), count, "one pass");
    }

    fn scalar(&self, sql: &str) -> i64 {
        self.db.query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn marked(&self) -> bool {
        self.scalar("SELECT reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot AND reconciled_epoch=recovery_epoch FROM herdr_hosts WHERE host_id=(SELECT min(host_id) FROM herdr_hosts)") == 1
    }
    fn hold_flag(&self) -> i64 {
        self.scalar("SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=(SELECT min(host_id) FROM herdr_hosts)")
    }
    fn raise_hold(&self) {
        self.db
            .execute("UPDATE herdr_hosts SET baseline_hold_unclaimed=1", [])
            .unwrap();
    }
    fn kept_claims(&self) -> i64 {
        self.scalar("SELECT count(*) FROM kept_continuity_claims")
    }
    fn shell_reads(&self) -> u64 {
        self.rig.host.shell_reads.load(Ordering::SeqCst)
    }
    fn seat_state(&self, seat: &str) -> String {
        self.db
            .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
            .unwrap()
    }
}

/// `other`, a seat a past incarnation left unresolved at `pane`: its binding's
/// process is this test process, so it is a live C1 process candidate. And a
/// kept claim whose process is gone, which any decision round drops.
fn seed_other_seat_and_claim(db: &rusqlite::Connection, instance: &str) {
    use crate::ports::ProcessInfo;
    let pid = std::process::id();
    let start = crate::host::process_table::HostProcessTable
        .process(pid)
        .expect("this process")
        .start;
    db.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
         VALUES ('other',?1,'unresolved','other','native','pane',2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
        [instance],
    )
    .unwrap();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason,liveness_pid,liveness_start) \
         VALUES ('other',1,'pane','term-old','inc-old','boot-old',1,1,'claude','SA','execution-1','cooperative_top_level',50,60,'disconnected',60,'seat_unresolved',?1,?2)",
        rusqlite::params![pid, start as i64],
    )
    .unwrap();
    let claim = crate::protocol::commands::ContinuityCheckIn {
        host: crate::test_support::test_host_locator(),
        target: HostTargetId::new("pane"),
        harness: crate::protocol::authority::Harness::Claude,
        native_session: NativeSessionId::new("kept"),
        source: crate::protocol::commands::ContinuityCheckIn::RESUME_SOURCE.into(),
        operation: OperationId::new("op-kept"),
        execution: ExecutionId::new("execution-kept"),
        liveness: Some(crate::protocol::authority::LivenessFingerprint {
            pid: u32::MAX,
            start: 1,
        }),
    };
    db.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) \
         VALUES ((SELECT min(host_id) FROM herdr_hosts),'pane','claude','kept',?1,100)",
        [serde_json::to_string(&claim).unwrap()],
    )
    .unwrap();
}

/// Kills: a marker that waits for a refusal-free pass (the stall ht-wp7n fixes)
/// or moves before the third pass; kept claims and process continuity that stay
/// gated on it; and an excluded unresolved seat treated as nothing left to hold.
#[test]
fn a_persistently_refused_seat_lets_kept_claims_and_process_continuity_proceed() {
    let pass = RefusedPass::start("incarnation", seed_other_seat_and_claim);
    pass.raise_hold();
    for count in 2..=3 {
        assert!(!pass.marked(), "before pass {count}");
        assert_eq!(pass.kept_claims(), 1, "claims wait for the marker");
        assert_eq!(pass.shell_reads(), 0, "process continuity waits for it");
        pass.run_to(count);
    }
    assert!(pass.marked(), "the third pass records the marker");
    assert_eq!(pass.kept_claims(), 0, "the kept claim was decided");
    assert!(pass.shell_reads() > 0, "process continuity read the pane");
    assert_eq!(pass.seat_state("culprit"), "resolved", "nothing moved it");
    assert_eq!(pass.seat_state("other"), "unresolved");
    assert_eq!(pass.hold_flag(), 1, "an unresolved seat keeps the hold");
    pass.run_to(4);
    assert!(pass.marked(), "the refused seat is still replanned");
}

/// Kills: a hold lift still gated on a refusal-free pass.
#[test]
fn a_persistently_refused_resolved_seat_lets_the_hold_lift() {
    let pass = RefusedPass::start("incarnation", |_, _| {});
    pass.raise_hold();
    pass.run_to(2);
    assert_eq!(pass.hold_flag(), 1, "no marker, no lift");
    pass.run_to(3);
    assert!(pass.marked());
    assert_eq!(pass.hold_flag(), 0, "nothing is left to protect");
}

/// A resolved seat a pass would unresolve looks resolved on a dead incarnation:
/// the accepted limit in C2. Kills: exclusion that ignores the action, which
/// would lift holds and finalize no-matches past it.
#[test]
fn a_refused_mark_unresolved_holds_the_marker_however_long_it_lasts() {
    let pass = RefusedPass::start("dead-incarnation", seed_other_seat_and_claim);
    pass.raise_hold();
    for count in 2..=crate::identity::reconcile::PERSISTENT_REFUSAL_PASSES as u64 + 2 {
        pass.run_to(count);
    }
    assert!(!pass.marked());
    assert_eq!(pass.hold_flag(), 1);
    assert_eq!(pass.kept_claims(), 1);
    assert_eq!(pass.shell_reads(), 0);
    assert_eq!(pass.seat_state("culprit"), "resolved", "nothing moved it");
}

/// Kills: a pass that keeps only its last page's refusals, which would record
/// the marker past a refused `MarkUnresolved` on an earlier page.
#[test]
fn a_refusal_on_an_earlier_page_survives_a_clean_last_page() {
    let saved = crate::identity::reconcile::MAX_SAVED_SEATS_PER_PLAN;
    let pass = RefusedPass::start("dead-incarnation", |db, instance| {
        for n in 0..saved {
            db.execute(
                "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
                 VALUES (?1,?2,'unresolved','other','native',?3,2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
                rusqlite::params![format!("clean-{n}"), instance, format!("gone-{n}")],
            )
            .unwrap();
        }
    });
    for count in 2..=crate::identity::reconcile::PERSISTENT_REFUSAL_PASSES as u64 + 1 {
        pass.run_to(count);
    }
    assert!(!pass.marked());
}
