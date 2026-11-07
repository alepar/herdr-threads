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

/// One process: (start, session, tty).
type ProcessRow = (u64, u32, Option<u64>);

/// The kernel boot id every fixture fingerprint is recorded under: the host's, so the C1 process
/// path (which reads the host's own process table) and the fake table agree.
fn lane_boot() -> String {
    crate::host::process_table::HostProcessTable
        .kernel_boot()
        .expect("kernel boot id readable")
}

/// A process table the test writes.
#[derive(Default)]
struct LaneProcesses(Mutex<std::collections::HashMap<u32, ProcessRow>>);
impl ProcessInfo for LaneProcesses {
    fn kernel_boot(&self) -> Option<String> {
        Some(lane_boot())
    }
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0.lock().unwrap().get(&pid).map(|row| ProcessRecord {
            pid,
            parent: None,
            start: row.0,
            argv: Vec::new(),
        })
    }
    fn terminal_session(&self, pid: u32) -> Option<TerminalSession> {
        self.0.lock().unwrap().get(&pid).map(|row| TerminalSession {
            session: row.1,
            tty: row.2,
        })
    }
}

/// A hook run inside a shell read, after the fence admitted it.
type OnShellRead = Box<dyn FnMut() + Send>;

/// One empty pane. While `down` is set every host call fails. `shells`: what Herdr reports as a
/// pane's process (none by default). `boot`/`epoch`: the host's current context, which a fenced
/// shell read must match as the native host's does: the epoch before the read (`check_context`),
/// then the epoch and the responding boot after it (`check_epoch`, `check_boot`); the snapshot it
/// publishes is (`host`, 1), so moving them is a restart the store has not reconciled. `shell_contexts`: each
/// shell read's (expected boot, expected epoch). `exhaust_budget_at_read`: on that read (counted
/// from 1; 0 never) the clock jumps past the 3 s maintenance budget.
struct LaneHost {
    clock: Arc<LaneClock>,
    sequence: AtomicU64,
    down: AtomicBool,
    snapshots: AtomicU64,
    shell_reads: AtomicU64,
    shell_reads_by_target: Mutex<Vec<String>>,
    shells: Mutex<std::collections::HashMap<String, u32>>,
    boot: Mutex<String>,
    epoch: AtomicU64,
    shell_contexts: Mutex<Vec<(Option<String>, Option<u64>)>>,
    exhaust_budget_at_read: AtomicU64,
    on_shell_read: Mutex<Option<OnShellRead>>,
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
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        self.shell_contexts.lock().unwrap().push((
            context
                .expected_boot
                .as_ref()
                .map(|b| b.as_str().to_owned()),
            context.expected_epoch,
        ));
        // Native `check_context`: only the epoch is compared before I/O.
        if context.expected_boot.is_some()
            && context.expected_epoch != Some(self.epoch.load(Ordering::SeqCst))
        {
            return Err(ApiError::new(
                ErrorCode::StaleHostObservation,
                "host context changed",
            ));
        }
        let epoch = self.epoch.load(Ordering::SeqCst);
        let reads = self.shell_reads.fetch_add(1, Ordering::SeqCst) + 1;
        if reads == self.exhaust_budget_at_read.load(Ordering::SeqCst) {
            self.clock.0.fetch_add(60_000, Ordering::SeqCst);
        }
        self.shell_reads_by_target
            .lock()
            .unwrap()
            .push(target.as_str().to_owned());
        if let Some(hook) = self.on_shell_read.lock().unwrap().as_mut() {
            hook();
        }
        // Native `check_epoch` and `check_boot` after the (witnessed) response.
        if let Some(boot) = context.expected_boot.as_ref() {
            if self.epoch.load(Ordering::SeqCst) != epoch {
                return Err(ApiError::new(
                    ErrorCode::StaleHostObservation,
                    "host connection epoch changed",
                ));
            }
            if boot.as_str() != self.boot.lock().unwrap().as_str() {
                return Err(ApiError::new(
                    ErrorCode::StaleHostObservation,
                    "host server incarnation changed",
                ));
            }
        }
        Ok(self.shells.lock().unwrap().get(target.as_str()).copied())
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
    processes: Arc<LaneProcesses>,
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
            shell_reads_by_target: Mutex::default(),
            shells: Mutex::default(),
            boot: Mutex::new("host".into()),
            epoch: AtomicU64::new(1),
            shell_contexts: Mutex::default(),
            exhaust_budget_at_read: AtomicU64::new(0),
            on_shell_read: Mutex::default(),
        });
        let processes = Arc::new(LaneProcesses::default());
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
            .with_observation_pacer(pacer.clone())
            .with_process_info(processes.clone()),
        );
        Self {
            _dir: dir,
            db,
            clock,
            store,
            pacer,
            host,
            processes,
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
                locator_hint: None,
                tmux_hint: None,
                continuity_key: None,
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
                locator_hint: None,
                tmux_hint: None,
                continuity_key: None,
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
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason,liveness_pid,liveness_start,liveness_boot) \
         VALUES ('other',1,'pane','term-old','inc-old','boot-old',1,1,'claude','SA','execution-1','cooperative_top_level',50,60,'disconnected',60,'seat_unresolved',?1,?2,?3)",
        rusqlite::params![pid, start as i64, lane_boot()],
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
        continuity_key: None,
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

// --- TRUST-POLICY C1 shell continuity (ht-ma6i.1): the lane around the store's decision.

const SHELL_PID: u32 = 4242;
const SHELL_START: u64 = 7;
const SHELL_TTY: u64 = 34817;

impl Rig {
    /// The lane is up and has published; `pane`'s shell is a session leader on its terminal.
    fn started_with_shell() -> Self {
        let mut rig = Self::new();
        rig.shell_at("pane", SHELL_PID);
        rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
        rig.start();
        rig.wait_idle(1);
        rig
    }

    fn shell_at(&self, target: &str, pid: u32) {
        self.host
            .shells
            .lock()
            .unwrap()
            .insert(target.to_owned(), pid);
    }

    fn process(&self, pid: u32, start: u64, session: u32, tty: Option<u64>) {
        self.processes
            .0
            .lock()
            .unwrap()
            .insert(pid, (start, session, tty));
    }

    fn resolve_pane(&self) -> SeatId {
        self.identity
            .resolve(
                ResolveSeat {
                    host: crate::test_support::test_host_locator(),
                    target: HostTargetId::new("pane"),
                    operation: OperationId::new(format!("op-{}", uuid::Uuid::new_v4())),
                    locator_hint: None,
                    tmux_hint: None,
                    continuity_key: None,
                },
                &CallBudget {
                    deadline: MonoInstant(START + 60_000),
                    cancellation: Cancellation::default(),
                },
            )
            .expect("seat resolved")
    }

    fn db_conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.db).unwrap()
    }

    fn scalar(&self, sql: &str) -> i64 {
        self.db_conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn fingerprint_rows(&self) -> i64 {
        self.scalar("SELECT count(*) FROM seat_shell_fingerprints")
    }

    fn shell_reads(&self) -> u64 {
        self.host.shell_reads.load(Ordering::SeqCst)
    }

    fn pass(&self) {
        let idle = self.pacer.idle_events();
        self.advance(5_000);
        self.wait_idle(idle + 1);
    }

    fn shell_targets(&self) -> Vec<String> {
        self.host.shell_reads_by_target.lock().unwrap().clone()
    }

    /// A resolved seat of a past incarnation the published one left at `pane`, with a row.
    fn left_by_handoff(&self, seat: &str, target: &str, pid: u32, start: u64) {
        let db = self.db_conn();
        db.execute(
            "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
             VALUES (?1,?2,'unresolved','other','native',?3,2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
            rusqlite::params![seat, self.instance, target],
        )
        .unwrap();
        db.execute(
            "INSERT INTO seat_shell_fingerprints(seat_id,host_id,target_id,host_boot,incarnation,terminal_id,pid,start,session,tty,kernel_boot,recorded_at) \
             VALUES (?1,(SELECT min(host_id) FROM herdr_hosts),?2,'old-boot','old-incarnation','old-term',?3,?4,?3,?5,?6,1)",
            rusqlite::params![seat, target, pid, start as i64, SHELL_TTY as i64, lane_boot()],
        )
        .unwrap();
    }

    fn seat_state(&self, seat: &str) -> String {
        self.db_conn()
            .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
            .unwrap()
    }

    fn shell_decisions(&self, seat: &str) -> i64 {
        self.db_conn()
            .query_row(
                "SELECT count(*) FROM allocation_decisions WHERE seat_id=?1 AND kind='structural_shell_continuity'",
                [seat],
                |r| r.get(0),
            )
            .unwrap()
    }
}

/// Allocation asks the lane for its own capture (`with_read_inner`), so no clock tick is needed.
/// Kills: an allocation that leaves the row to the next cadence, or to no pass at all.
#[test]
fn new_bare_seat_gets_its_row_with_no_other_trigger() {
    let rig = Rig::started_with_shell();
    let before = rig.clock.monotonic_now();
    let seat = rig.resolve_pane();
    rig.wait("the fingerprint row", &|| rig.fingerprint_rows() == 1);
    assert_eq!(rig.clock.monotonic_now(), before, "the clock never moved");
    assert_eq!(
        rig.host.shell_contexts.lock().unwrap()[0],
        (Some("host".to_owned()), Some(1)),
        "the read is fenced to the listing's publication"
    );
    let db = rig.db_conn();
    let row: (String, u32, i64, u32, i64) = db
        .query_row(
            "SELECT seat_id,pid,start,session,tty FROM seat_shell_fingerprints",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            seat.as_str().to_owned(),
            SHELL_PID,
            SHELL_START as i64,
            SHELL_PID,
            SHELL_TTY as i64
        )
    );
}

/// Kills: re-reading a seat that has its row, the ht-kaw9 shape: a stalled host read multiplied
/// by seat count on every pass.
#[test]
fn current_row_makes_no_host_call() {
    let rig = Rig::started_with_shell();
    rig.resolve_pane();
    rig.wait("the fingerprint row", &|| rig.fingerprint_rows() == 1);
    rig.wait("the lane to settle", &|| rig.pacer.idle_events() >= 2);
    let reads = rig.shell_reads();
    assert!(reads >= 1, "the row came from a read");
    for _ in 0..3 {
        rig.pass();
    }
    assert_eq!(rig.shell_reads(), reads, "steady passes read nothing");
    assert_eq!(rig.fingerprint_rows(), 1);
}

/// Seed `n` resolved seats at their own panes, on the published incarnation.
fn seed_resolved(rig: &Rig, n: usize) {
    let db = rig.db_conn();
    for i in 0..n {
        db.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,home_node,is_local,host_id) \
             VALUES (?1,?2,'resolved','native',?3,1,1,'term','incarnation','coherent_enumeration','host',1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
            rusqlite::params![format!("seat-{i:02}"), rig.instance, format!("pane-{i:02}")],
        )
        .unwrap();
    }
}

/// Kills: more than 16 reads a pass; a cursor that stays on an unreadable seat, which would starve
/// the ones behind it; a cursor that never wraps, which would never retry one.
#[test]
fn an_unreadable_shell_cannot_starve_the_others_and_reads_are_capped() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    seed_resolved(&rig, 20);
    // Every seat's shell unreadable but the last.
    rig.shell_at("pane-19", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.shell_reads(), 16, "one page");
    assert_eq!(rig.fingerprint_rows(), 0);
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.shell_reads(), 20, "the rest of the listing");
    assert_eq!(rig.fingerprint_rows(), 1, "the seat behind the unreadable");
    // The cursor wrapped: the listing starts over, without the seat that has its row.
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.shell_reads(), 36);
    let reads = rig.host.shell_reads_by_target.lock().unwrap().clone();
    assert_eq!(reads[20], "pane-00");
}

/// A shell that leads no session or has no terminal records nothing and is retried.
#[test]
fn a_shell_with_no_session_or_terminal_records_nothing() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    seed_resolved(&rig, 1);
    rig.shell_at("pane-00", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, 1, Some(SHELL_TTY));
    rig.identity.record_shell_fingerprints().unwrap();
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, None);
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 0);
    assert_eq!(rig.shell_reads(), 2, "retried on the next pass");
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 1);
}

/// The whole path: a seat a handoff left at `pane`, its shell alive there. Kills: the lane never
/// deciding, or deciding outside its completed pass.
#[test]
fn a_surviving_shell_resolves_its_seat_on_the_lane() {
    let rig = Rig::started_with_shell();
    rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    rig.pass();
    assert_eq!(rig.seat_state("bare"), "resolved");
    assert_eq!(rig.shell_decisions("bare"), 1);
    assert_eq!(
        rig.scalar("SELECT count(*) FROM occupant_bindings"),
        0,
        "no binding opened"
    );
}

/// Each mismatch of the facts decides nothing. Kills: any one fact dropped from the judgment.
#[test]
fn a_mismatch_decides_nothing() {
    type Scene = fn(&Rig);
    let scenes: [(&str, Scene); 5] = [
        ("new shell after a cold restart", |rig| {
            rig.shell_at("pane", 9999);
            rig.process(9999, 1, 9999, Some(SHELL_TTY));
        }),
        ("other tty", |rig| {
            rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY + 1));
        }),
        ("not session leader", |rig| {
            rig.process(SHELL_PID, SHELL_START, 1, Some(SHELL_TTY));
        }),
        ("pid reused", |rig| {
            rig.process(SHELL_PID, SHELL_START + 1, SHELL_PID, Some(SHELL_TTY));
        }),
        ("unread pane", |rig| {
            rig.host.shells.lock().unwrap().clear();
        }),
    ];
    for (what, scene) in scenes {
        let rig = Rig::started_with_shell();
        rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
        scene(&rig);
        rig.pass();
        assert_eq!(rig.seat_state("bare"), "unresolved", "{what}");
        assert_eq!(rig.shell_decisions("bare"), 0, "{what}");
    }
}

/// Kills: a decline that is forgotten (a host read per pass), one that never expires, or a gone
/// process remembered as ruled out.
#[test]
fn a_decline_is_remembered_ten_minutes_and_gone_is_retried() {
    let mut rig = Rig::started_with_shell();
    // The lane is stopped, so only the direct calls below read the pane.
    rig.stop();
    rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    // Another shell in the pane: ruled out.
    rig.shell_at("pane", 9999);
    rig.process(9999, 1, 9999, Some(SHELL_TTY));
    rig.identity.decide_shell_continuity().unwrap();
    let reads = rig.shell_reads();
    assert_eq!(reads, 1);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_reads(), reads, "the decline stands");
    // Just short of the TTL.
    rig.clock.0.fetch_add(10 * 60 * 1000 - 1, Ordering::SeqCst);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_reads(), reads, "still within ten minutes");
    rig.clock.0.fetch_add(1, Ordering::SeqCst);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_reads(), reads + 1, "read again after ten minutes");

    // The pane reports the saved pid and the process is dead: gone is no decline.
    let mut gone = Rig::started_with_shell();
    gone.stop();
    gone.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    gone.processes.0.lock().unwrap().clear();
    for reads in 1..=2 {
        gone.identity.decide_shell_continuity().unwrap();
        assert_eq!(gone.shell_reads(), reads, "gone is retried");
    }
    assert_eq!(gone.seat_state("bare"), "unresolved");
}

/// A decline is keyed by the fingerprint: another generation is judged afresh.
#[test]
fn a_decline_does_not_outlive_the_seat_generation() {
    let rig = Rig::started_with_shell();
    rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    rig.shell_at("pane", 9999);
    rig.process(9999, 1, 9999, Some(SHELL_TTY));
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_reads(), 1);
    rig.db_conn()
        .execute(
            "UPDATE seats SET generation=generation+1 WHERE id='bare'",
            [],
        )
        .unwrap();
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_reads(), 2);
}

/// A live binding process of `seat`: a process continuity candidate at `pane`, pid `pid`.
fn seed_live_binding(rig: &Rig, seat: &str, pid: u32, start: u64) {
    rig.db_conn().execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason,liveness_pid,liveness_start,liveness_boot) \
         VALUES (?1,1,'pane','term-old','old-incarnation','old-boot',1,1,'claude','session-'||?1,'execution-'||?1,'cooperative_top_level',50,60,'disconnected',60,'seat_unresolved',?2,?3,?4)",
        rusqlite::params![seat, pid, start as i64, lane_boot()],
    )
    .unwrap();
}

/// A live undeclined ht-hsyg candidate is left to that path, which also reopens a binding. Kills:
/// the shell path stealing the seat and leaving the agent unbound. Once the process exited or was
/// ruled out, the shell path decides.
#[test]
fn a_live_process_candidate_is_left_to_its_own_path() {
    let rig = Rig::started_with_shell();
    rig.left_by_handoff("agent", "pane", SHELL_PID, SHELL_START);
    // The binding's process is in another session, and alive, so its path has not declined it yet.
    rig.process(777, 3, SHELL_PID, Some(SHELL_TTY));
    seed_live_binding(&rig, "agent", 777, 3);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.seat_state("agent"), "unresolved");
    assert_eq!(rig.shell_reads(), 0, "left to its path, at no host read");
    rig.processes.0.lock().unwrap().remove(&777);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.seat_state("agent"), "resolved", "an exited agent");
    assert_eq!(rig.shell_decisions("agent"), 1);
}

/// Kills: deciding past another seat's fingerprint, or another seat's live or unreadable process.
#[test]
fn a_rival_decides_nothing() {
    // Another seat's fingerprint names the same shell.
    let rig = Rig::started_with_shell();
    rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    rig.left_by_handoff("rival", "other-pane", SHELL_PID, SHELL_START);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.seat_state("bare"), "unresolved");
    assert_eq!(rig.seat_state("rival"), "unresolved");
    assert_eq!(rig.shell_reads(), 0, "a rival fingerprint costs no read");

    // Another seat saved at the pane has its process there.
    let rig = Rig::started_with_shell();
    rig.left_by_handoff("bare", "pane", SHELL_PID, SHELL_START);
    let db = rig.db_conn();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
         VALUES ('other',?1,'unresolved','other','native','pane',2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
        [&rig.instance],
    )
    .unwrap();
    rig.process(777, 3, SHELL_PID, Some(SHELL_TTY));
    seed_live_binding(&rig, "other", 777, 3);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.seat_state("bare"), "unresolved");
    assert_eq!(rig.shell_decisions("bare"), 0);
}

/// Kills: an unfenced shell read recording the new incarnation's shell under the old (boot,
/// incarnation, terminal) row, which a later cold restart could match as continuous.
#[test]
fn a_restart_between_listing_and_read_records_nothing() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_resolved(&rig, 1);
    rig.shell_at("pane-00", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    // Herdr restarted; no pass has reconciled it, so the store still lists under (host, 1).
    *rig.host.boot.lock().unwrap() = "host-2".into();
    rig.host.epoch.store(2, Ordering::SeqCst);
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 0);
    assert_eq!(rig.shell_reads(), 0, "refused before I/O");
    assert_eq!(
        rig.host.shell_contexts.lock().unwrap().clone(),
        vec![(Some("host".to_owned()), Some(1))],
        "fenced to the listing's publication"
    );
    // The same seat records once the host is back on that context: the fence refused it.
    *rig.host.boot.lock().unwrap() = "host".into();
    rig.host.epoch.store(1, Ordering::SeqCst);
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 1);
}

/// Kills: a shell read fenced only on the connection epoch recording a restarted server's shell
/// under the old row (a Herdr restart on the same socket leaves the native epoch unchanged).
#[test]
fn a_restart_that_keeps_the_epoch_records_nothing() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_resolved(&rig, 1);
    rig.shell_at("pane-00", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    // Herdr restarted under the same connection epoch; the store still lists under (host, 1).
    *rig.host.boot.lock().unwrap() = "host-2".into();
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 0);
    assert_eq!(
        rig.shell_reads(),
        1,
        "admitted before I/O; refused on the response"
    );
    assert_eq!(
        rig.host.shell_contexts.lock().unwrap().clone(),
        vec![(Some("host".to_owned()), Some(1))]
    );
    *rig.host.boot.lock().unwrap() = "host".into();
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.fingerprint_rows(), 1);
}

/// Kills: a shell read with no post-read epoch check.
#[test]
fn an_epoch_moved_during_the_read_records_nothing() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_resolved(&rig, 1);
    rig.shell_at("pane-00", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    let host = Arc::downgrade(&rig.host);
    *rig.host.on_shell_read.lock().unwrap() = Some(Box::new(move || {
        if let Some(host) = host.upgrade() {
            host.epoch.store(2, Ordering::SeqCst);
        }
    }));
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.shell_reads(), 1, "the read itself was admitted");
    assert_eq!(rig.fingerprint_rows(), 0);
}

/// Kills: a write that trusts the seat columns alone after reconciliation published another
/// epoch between the read and the write.
#[test]
fn a_publication_moved_during_the_read_records_nothing() {
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_resolved(&rig, 1);
    rig.shell_at("pane-00", SHELL_PID);
    rig.process(SHELL_PID, SHELL_START, SHELL_PID, Some(SHELL_TTY));
    let db = rig.db.clone();
    let publish = move |epoch: i64| {
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE snapshot_generations SET epoch=?1 WHERE id=(SELECT active_snapshot_id FROM herdr_hosts)",
                [epoch],
            )
            .unwrap();
    };
    *rig.host.on_shell_read.lock().unwrap() = Some(Box::new(move || publish(2)));
    rig.identity.record_shell_fingerprints().unwrap();
    assert_eq!(rig.shell_reads(), 1, "the read itself was admitted");
    assert_eq!(rig.fingerprint_rows(), 0);
}

/// `n` unresolved seats `bare-NN` a handoff left at `pane-NN`, each with its own shell (no rival)
/// and no pane reporting one, so each judged candidate costs exactly one read and is retried.
fn seed_left_by_handoff(rig: &Rig, n: usize) {
    for i in 0..n {
        let pid = 5_000 + u32::try_from(i).unwrap();
        rig.left_by_handoff(&format!("bare-{i:02}"), &format!("pane-{i:02}"), pid, 1);
    }
}

fn panes(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|i| format!("pane-{i:02}")).collect()
}

/// Kills: a cap other than 32, a cursor that does not advance, or one that never wraps.
#[test]
fn shell_decisions_page_at_32_and_the_cursor_wraps() {
    let page = crate::ports::SHELL_CONTINUITY_CANDIDATES_PER_PASS;
    assert_eq!(page, 32);
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_left_by_handoff(&rig, page + 1);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_targets(), panes(0..page), "one page");
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(
        rig.shell_targets()[page..],
        panes(page..page + 1),
        "the cursor advanced"
    );
    // That page was short: the cursor reset and the listing starts over.
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_targets()[page + 1..], panes(0..page));
}

/// Kills: a budget stop that loses the cursor, skips a seat or keeps going.
#[test]
fn an_exhausted_budget_stops_the_page_and_resumes_after_the_last_judged() {
    const K: usize = 5;
    let mut rig = Rig::new();
    rig.start();
    rig.wait_idle(1);
    rig.stop();
    seed_left_by_handoff(&rig, 2 * K);
    rig.host
        .exhaust_budget_at_read
        .store(u64::try_from(K).unwrap(), Ordering::SeqCst);
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_targets(), panes(0..K), "stopped at the budget");
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(
        rig.shell_targets()[K..],
        panes(K..2 * K),
        "resumed after the last judged"
    );
    // The page completed: the listing wraps.
    rig.identity.decide_shell_continuity().unwrap();
    assert_eq!(rig.shell_targets()[2 * K..], panes(0..2 * K));
}
