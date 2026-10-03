use herdr_threads::{
    app::{SystemClock, run_elected},
    client::local::LocalSocketClient,
    daemon::{
        ownership::{EndpointDescriptor, OwnerLock},
        paths::{InstancePaths, RuntimeContext},
    },
    ports::*,
    protocol::{
        commands::{Command, ResolveSeat},
        ids::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::config::ServiceConfig,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::OptionalExtension;
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use uuid::Uuid;

struct ResolutionHost {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    sequence: AtomicU64,
    entered: mpsc::SyncSender<()>,
    held: AtomicBool,
    /// Holds a target capture until the test clears it, ignoring the daemon's
    /// host-call budget, so the daemon cannot answer before the CLI's own
    /// client deadline (no daemon/client deadline race).
    held_past_budget: AtomicBool,
    fail: AtomicBool,
    /// Error code a failing capture reports: 0 HostUnavailable, 1 NotFound,
    /// 2 StaleHostObservation.
    fail_code: AtomicU64,
    /// A target read answers, but without verified structural proof (no
    /// terminal): the evidence-based `CoherenceLost` invalidation, unlike a
    /// failed read (Herdr unavailable), which freezes and writes nothing.
    incoherent: AtomicBool,
    calls: AtomicU64,
    snapshots: AtomicU64,
    snapshot_mode: AtomicU64,
    snapshot_held: AtomicBool,
    /// When false, snapshot captures skip the "no SQLite writer held" probe
    /// (a test holds the writer from outside the daemon on purpose).
    snapshot_writer_probe: AtomicBool,
    active_reads: AtomicU64,
    /// A test that drives a second `OrdinaryIdentity` beside the daemon's own
    /// observation lane has two independent lanes, so overlap of their
    /// captures is expected there and is not a serialization failure.
    overlap_allowed: AtomicBool,
    /// This many target reads each commit an unrelated seat decision's fence
    /// (the instance lifecycle revision, as another agent's check-in does)
    /// while in flight, superseding their own publication.
    supersede_reads: AtomicU64,
}
struct ActiveCapture<'a>(&'a AtomicU64);
impl Drop for ActiveCapture<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ResolutionHost {
    fn enter_capture(&self) -> ActiveCapture<'_> {
        let active = self.active_reads.fetch_add(1, Ordering::SeqCst);
        if !self.overlap_allowed.load(Ordering::SeqCst) {
            assert_eq!(active, 0, "target and snapshot captures overlapped");
        }
        ActiveCapture(&self.active_reads)
    }
    fn observation(&self, sequence: u64) -> HostObservation {
        let at = self.clock.monotonic_now();
        HostObservation {
            focused: false,
            target: HostTargetId::new("pane"),
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
}
impl HostPort for ResolutionHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        let _capture = self.enter_capture();
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(target.as_str(), "pane");
        // Before the first publication (first contact) nothing is expected.
        if let Some(boot) = context.expected_boot.as_ref() {
            assert_eq!(boot.as_str(), "host");
            assert_eq!(context.expected_epoch, Some(1));
        }
        let db = rusqlite::Connection::open(&self.path).unwrap();
        db.busy_timeout(super::HOST_IO_WRITER_PROBE_WAIT).unwrap();
        db.execute_batch("BEGIN IMMEDIATE")
            .expect("host I/O was called under SQLite writer");
        let issued: i64 = db
            .query_row(
                "SELECT observation_admission_sequence FROM host_instances",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            issued >= 1,
            "host read preceded store observation admission"
        );
        db.execute_batch("ROLLBACK").unwrap();
        if self
            .supersede_reads
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            db.execute_batch("UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1")
                .unwrap();
        }
        let _ = self.entered.try_send(());
        while self.held_past_budget.load(Ordering::SeqCst)
            || (self.held.load(Ordering::SeqCst)
                && !context.budget.is_exhausted(self.clock.as_ref()))
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.fail.load(Ordering::SeqCst) || context.budget.is_exhausted(self.clock.as_ref()) {
            return Err(ApiError::new(
                match self.fail_code.load(Ordering::SeqCst) {
                    1 => ErrorCode::NotFound,
                    2 => ErrorCode::StaleHostObservation,
                    _ => ErrorCode::HostUnavailable,
                },
                "controlled capture failed",
            ));
        }
        let mut observation = self.observation(self.sequence.fetch_add(1, Ordering::SeqCst));
        if self.incoherent.load(Ordering::SeqCst) {
            observation.terminal = None;
        }
        Ok(observation)
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let _capture = self.enter_capture();
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        while self.snapshot_held.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let mode = self.snapshot_mode.load(Ordering::SeqCst);
        let db = rusqlite::Connection::open(&self.path).unwrap();
        db.busy_timeout(super::HOST_IO_WRITER_PROBE_WAIT).unwrap();
        if self.snapshot_writer_probe.load(Ordering::SeqCst) {
            db.execute_batch("BEGIN IMMEDIATE; ROLLBACK")
                .expect("snapshot capture held SQLite writer");
        }
        if mode == 1 {
            return Err(ApiError::host_unavailable("capture unavailable"));
        }
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        Ok(HostSnapshot {
            boot: HostBootId::new("host"),
            epoch: 1,
            observation_sequence: sequence,
            complete: mode != 2,
            enumeration: if mode == 2 {
                EnumerationEvidence::Partial
            } else {
                EnumerationEvidence::CoherentVerified
            },
            incarnation: if mode == 3 {
                IncarnationEvidence::Unknown
            } else {
                IncarnationEvidence::Verified {
                    identity: "incarnation".into(),
                    evidence_kind: EvidenceKind::CoherentEnumeration,
                }
            },
            targets: vec![self.observation(sequence)],
        })
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
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<herdr_threads::ports::AgentComposerState, ApiError> {
        Ok(herdr_threads::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(
        &self,
        _: &herdr_threads::ports::SafeWakeTarget,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<(), herdr_threads::protocol::results::ApiError> {
        Ok(())
    }
}

struct HeldRequest<T> {
    host: Arc<ResolutionHost>,
    worker: Option<std::thread::JoinHandle<T>>,
}
impl<T> HeldRequest<T> {
    fn join(mut self) -> std::thread::Result<T> {
        self.worker.take().unwrap().join()
    }
}
impl<T> Drop for HeldRequest<T> {
    fn drop(&mut self) {
        self.host.held.store(false, Ordering::SeqCst);
        self.host.snapshot_held.store(false, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(feature = "test-support")]
struct JoinedCall<T> {
    cancellation: Cancellation,
    worker: Option<std::thread::JoinHandle<T>>,
}
#[cfg(feature = "test-support")]
impl<T> JoinedCall<T> {
    fn join(mut self) -> std::thread::Result<T> {
        self.worker.take().unwrap().join()
    }
}
#[cfg(feature = "test-support")]
impl<T> Drop for JoinedCall<T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(feature = "test-support")]
struct HeldSqliteWriter<'a>(&'a rusqlite::Connection);
#[cfg(feature = "test-support")]
impl Drop for HeldSqliteWriter<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("ROLLBACK");
    }
}

struct Fixture {
    root: PathBuf,
    paths: InstancePaths,
    clock: Arc<dyn Clock>,
    store: Arc<SqliteStore>,
    host: Arc<ResolutionHost>,
    entered: mpsc::Receiver<()>,
    _stdio_guard: std::sync::MutexGuard<'static, ()>,
    descriptor: EndpointDescriptor,
    stop: Cancellation,
    daemon: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
}
impl Fixture {
    fn new(hold_baseline: bool) -> Self {
        Self::start(hold_baseline, true, 0)
    }
    fn start(hold_baseline: bool, seed: bool, snapshot_mode: u64) -> Self {
        let stdio_guard = super::IN_PROCESS_DAEMON
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = std::env::temp_dir().join(format!("herdr-resolve-service-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let runtime_context =
            RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
        let paths = InstancePaths::resolve(&runtime_context).unwrap();
        let owner = OwnerLock::acquire(&paths).unwrap();
        let instance = owner.instance_uuid();
        drop(owner);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(paths.database_path.clone(), Arc::clone(&clock)),
                instance.to_string(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let (tx, entered) = mpsc::sync_channel(1);
        let host = Arc::new(ResolutionHost {
            path: paths.database_path.clone(),
            clock: Arc::clone(&clock),
            sequence: AtomicU64::new(2),
            entered: tx,
            held: AtomicBool::new(false),
            held_past_budget: AtomicBool::new(false),
            fail: AtomicBool::new(false),
            fail_code: AtomicU64::new(0),
            incoherent: AtomicBool::new(false),
            calls: AtomicU64::new(0),
            snapshots: AtomicU64::new(0),
            snapshot_mode: AtomicU64::new(snapshot_mode),
            snapshot_held: AtomicBool::new(snapshot_mode == 4),
            snapshot_writer_probe: AtomicBool::new(true),
            active_reads: AtomicU64::new(0),
            overlap_allowed: AtomicBool::new(false),
            supersede_reads: AtomicU64::new(0),
        });
        if hold_baseline {
            let db = rusqlite::Connection::open(&paths.database_path).unwrap();
            db.execute(
                "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
                [instance.to_string()],
            )
            .unwrap();
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('saved',?1,'unresolved','native',1,0)", [instance.to_string()]).unwrap();
        }
        if seed {
            let budget = CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
                cancellation: Cancellation::default(),
            };
            let captured = HostSnapshot {
                boot: HostBootId::new("host"),
                epoch: 1,
                observation_sequence: 1,
                complete: true,
                enumeration: EnumerationEvidence::CoherentVerified,
                incarnation: IncarnationEvidence::Verified {
                    identity: "incarnation".into(),
                    evidence_kind: EvidenceKind::CoherentEnumeration,
                },
                targets: vec![host.observation(1)],
            };
            let admission = store
                .begin_host_observation(&instance.to_string(), &budget)
                .unwrap();
            let stage = store
                .begin_snapshot_stage(
                    SnapshotHeader::from_captured(admission, &captured).unwrap(),
                    &budget,
                )
                .unwrap();
            store
                .stage_snapshot_targets(
                    &stage.id,
                    0,
                    &captured.targets,
                    DurableWorkAdmission::new(16).unwrap(),
                    &budget,
                )
                .unwrap();
            store.seal_snapshot_stage(&stage.id, &budget).unwrap();
            store.publish_snapshot_stage(&stage.id, &budget).unwrap();
        }
        let stop = Cancellation::default();
        let thread_stop = stop.clone();
        let thread_paths = paths.clone();
        let thread_clock = Arc::clone(&clock);
        let thread_host = host.clone();
        let (ready, received) = mpsc::sync_channel(1);
        let daemon = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected(
                    &thread_paths,
                    thread_clock,
                    thread_stop,
                    ServiceConfig::default(),
                    thread_host,
                    move |descriptor| {
                        ready.send(descriptor.clone()).unwrap();
                        Ok(())
                    },
                ))
        });
        let descriptor = match received.recv_timeout(Duration::from_secs(30)) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                stop.cancel();
                host.held.store(false, Ordering::SeqCst);
                host.held_past_budget.store(false, Ordering::SeqCst);
                let outcome = daemon.join();
                panic!("private daemon did not become ready: {error}; {outcome:?}");
            }
        };
        Self {
            _stdio_guard: stdio_guard,
            root,
            paths,
            clock,
            store,
            host,
            entered,
            descriptor,
            stop,
            daemon: Some(daemon),
        }
    }
    fn restart(&mut self) {
        self.stop.cancel();
        assert!(self.daemon.take().unwrap().join().unwrap().unwrap());
        self.stop = Cancellation::default();
        let paths = self.paths.clone();
        let clock = self.clock.clone();
        let host = self.host.clone();
        let stop = self.stop.clone();
        let (ready, received) = mpsc::sync_channel(1);
        self.daemon = Some(std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected(
                    &paths,
                    clock,
                    stop,
                    ServiceConfig::default(),
                    host,
                    move |descriptor| {
                        ready.send(descriptor.clone()).unwrap();
                        Ok(())
                    },
                ))
        }));
        self.descriptor = received.recv_timeout(Duration::from_secs(30)).unwrap();
    }
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        }
    }
    fn client(&self) -> LocalSocketClient {
        LocalSocketClient::new(
            self.descriptor.endpoint.clone(),
            Arc::clone(&self.clock),
            self.descriptor.instance_uuid,
            Some(self.descriptor.boot_id),
        )
    }
    fn resolve(&self, operation: &str) -> Result<CommandResult, ApiError> {
        LocalClient::call(
            &self.client(),
            Command::ResolveSeat(ResolveSeat {
                target: HostTargetId::new("pane"),
                operation: OperationId::new(operation),
            }),
            &self.budget(),
        )
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.paths.database_path).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        self.host.held.store(false, Ordering::SeqCst);
        self.host.held_past_budget.store(false, Ordering::SeqCst);
        self.host.snapshot_held.store(false, Ordering::SeqCst);
        if let Some(worker) = self.daemon.take() {
            let _ = worker.join();
        }
        if std::thread::panicking()
            && let Ok(log) = fs::read_to_string(self.paths.instance_dir.join("daemon.log"))
        {
            eprintln!("{log}");
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn real_ipc_resolve_admits_host_read_then_commits_structural_seat_without_native_authority() {
    let fixture = Fixture::new(false);
    let result = fixture.resolve("allocate-first").unwrap();
    let CommandResult::SeatResolved(seat) = result else {
        panic!("missing resolved seat");
    };
    let db = fixture.db();
    let proof: (String, String, i64) = db.query_row("SELECT structural_terminal_id,structural_incarnation,structural_observation_sequence FROM seats WHERE id=?1", [seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (&proof.0, &proof.1),
        (&"terminal".into(), &"incarnation".into())
    );
    assert!(proof.2 >= 2);
    let bindings: i64 = db
        .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bindings, 0);
    let published: (i64,i64) = db.query_row("SELECT observation_sequence,(SELECT count(*) FROM allocation_decisions) FROM observed_targets WHERE target_id='pane'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(published.0 >= proof.2);
    assert_eq!(published.1, 1);
}

/// A concurrent seat decision (here: the instance lifecycle revision another
/// agent's check-in bumps) that lands while a resolve's target read is in
/// flight supersedes that read's publication. The daemon admits and reads
/// again inside the request budget instead of refusing the caller with a
/// transient `StaleHostObservation`. Kills: refusing on the first supersession
/// (the parallel-suite flake ht-zo4.3: one agent's check-in failed another's
/// resolve or send).
#[test]
fn superseded_target_read_is_admitted_again_within_the_request() {
    let fixture = Fixture::new(false);
    let calls = fixture.host.calls.load(Ordering::SeqCst);
    fixture.host.supersede_reads.store(1, Ordering::SeqCst);
    let result = fixture.resolve("superseded-once").unwrap();
    assert!(
        matches!(result, CommandResult::SeatResolved(_)),
        "{result:?}"
    );
    assert_eq!(fixture.host.supersede_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.host.calls.load(Ordering::SeqCst) - calls,
        2,
        "one superseded read, one fresh admission and read"
    );
}

/// A view that keeps moving is refused after the bounded attempts, as the
/// transient `StaleHostObservation` the caller may retry. Kills: an unbounded
/// re-admission loop.
#[test]
fn persistently_superseded_target_read_is_refused_after_bounded_attempts() {
    let fixture = Fixture::new(false);
    let calls = fixture.host.calls.load(Ordering::SeqCst);
    fixture
        .host
        .supersede_reads
        .store(u64::MAX, Ordering::SeqCst);
    let error = fixture.resolve("superseded-always").unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleHostObservation, "{error:?}");
    assert_eq!(
        fixture.host.calls.load(Ordering::SeqCst) - calls,
        u64::from(herdr_threads::identity::repair::SUPERSEDED_READ_ATTEMPTS)
    );
}

/// First contact: a resolve whose read finds no published snapshot yet (the
/// daemon just started, or Herdr restarted) waits for the observation lane's
/// capture and reads again inside its budget, instead of refusing the caller
/// with a transient `StaleHostObservation`. Deterministic: lane captures fail
/// until the resolve's own read is in flight, then succeed. Kills: refusing a
/// first-contact read (the ht-zo4.3 flake: `seat resolve` right after
/// `daemon ensure`), and waiting without re-reading.
#[test]
fn first_contact_read_waits_for_the_lane_capture_it_needs() {
    let fixture = Fixture::start(false, false, 1);
    fixture.host.held.store(true, Ordering::SeqCst);
    let calls = fixture.host.calls.load(Ordering::SeqCst);
    let client = fixture.client();
    let budget = fixture.budget();
    let request = HeldRequest {
        host: fixture.host.clone(),
        worker: Some(std::thread::spawn(move || {
            LocalClient::call(
                &client,
                Command::ResolveSeat(ResolveSeat {
                    target: HostTargetId::new("pane"),
                    operation: OperationId::new("first-contact"),
                }),
                &budget,
            )
        })),
    };
    fixture
        .entered
        .recv_timeout(Duration::from_secs(30))
        .expect("service did not admit target read");
    let published: Option<String> = fixture
        .db()
        .query_row("SELECT active_snapshot_id FROM host_instances", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(published, None, "no snapshot is published before the read");
    // The read holds the lane; the capture it asks for afterwards succeeds.
    fixture.host.snapshot_mode.store(0, Ordering::SeqCst);
    fixture.host.held.store(false, Ordering::SeqCst);
    let result = request.join().unwrap().unwrap();
    assert!(
        matches!(result, CommandResult::SeatResolved(_)),
        "{result:?}"
    );
    assert_eq!(
        fixture.host.calls.load(Ordering::SeqCst) - calls,
        2,
        "the refused first-contact read, then one read after the capture"
    );
}

#[test]
fn direct_socket_hook_keeps_selected_check_in_and_directory_continuations() {
    use herdr_threads::{
        cli::journal::Journal,
        harness::{
            Capability, LifecycleEvent,
            bridge::{OverviewReason, run_hook_event_with_reason},
            context::{
                ContextJournal, EventKind, Harness, OccupantContext, Role, SessionReference,
            },
        },
        protocol::output::{ContinuationContext, OutputFormat, OutputSpec},
    };
    let fixture = Fixture::new(false);
    let CommandResult::SeatResolved(seat) = fixture.resolve("selected-hook-seat").unwrap() else {
        panic!("missing resolved seat")
    };
    let db = fixture.db();
    // The bridge's first lifecycle event starts from the unbound generation.
    db.execute("UPDATE seats SET generation=0 WHERE id=?1", [seat.as_str()])
        .unwrap();
    for n in 0..25 {
        let thread = format!("selected-thread-{n:02}");
        let invitation = format!("selected-invite-{n:02}");
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,?2,'selected topic','goal',0,0)", rusqlite::params![thread, fixture.descriptor.instance_uuid.to_string()]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'invited')",
            rusqlite::params![thread, seat.as_str()],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES (?1,?2,?3,1,'pending',0,?4,9223372036854775807,300000)", rusqlite::params![invitation, thread, seat.as_str(), n+1]).unwrap();
    }
    db.execute(
        "UPDATE host_instances SET decision_seq=25 WHERE id=?1",
        [fixture.descriptor.instance_uuid.to_string()],
    )
    .unwrap();
    drop(db);
    let intents = Journal::open(fixture.root.join("hook-intents")).unwrap();
    let contexts = ContextJournal::open(
        &fixture.root.canonicalize().unwrap(),
        fixture.descriptor.instance_uuid,
        seat.as_str(),
        Duration::from_millis(500),
    )
    .unwrap();
    let execution = Uuid::new_v4();
    let seed = OccupantContext {
        format_version: 1,
        instance: fixture.descriptor.instance_uuid,
        seat: seat.as_str().into(),
        target: "pane".into(),
        harness: Harness::Codex,
        binding_generation: 0,
        execution,
        session: SessionReference::PluginContext(execution),
        role: Role::TopLevel,
    };
    let event = LifecycleEvent {
        harness: Harness::Codex,
        source: "explicit".into(),
        kind: EventKind::Startup,
        native_session: None,
        role: Role::TopLevel,
        event_id: "selected-socket-start".into(),
        capability: Capability::SourceSupported,
    };
    let first = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some(fixture.root.join("state").to_str().unwrap().into()),
            host: Some(fixture.root.join("host.sock").to_str().unwrap().into()),
        },
    };
    let client = fixture.client();
    let mut first_hook = Vec::new();
    run_hook_event_with_reason(
        &intents,
        &contexts,
        &event,
        Some(&seed),
        1,
        &client,
        fixture.clock.as_ref(),
        &first,
        OverviewReason::Lifecycle,
        &mut first_hook,
    )
    .unwrap();
    let saved = contexts
        .dispatch(
            &event.event_id,
            &mut |_pending: &herdr_threads::harness::context::PendingCheckIn| {
                panic!("completed CheckIn must not redispatch")
            },
        )
        .unwrap();
    let CommandResult::CheckedIn(check) = serde_json::from_slice(&saved.output).unwrap() else {
        panic!()
    };
    let original_next = check.inbox.next_argv.expect("real inbox continuation");
    assert!(
        original_next
            .windows(2)
            .any(|v| v == ["--state-dir", first.context.state_dir.as_ref().unwrap()])
    );
    assert!(original_next.windows(2).any(|v| v
        == [
            "--host-endpoint",
            first.context.host.as_ref().unwrap().as_str()
        ]));
    assert!(!original_next.iter().any(|part| part == "--json"));
    let first_text = String::from_utf8(first_hook).unwrap();
    assert!(first_text.contains("Current directory overview at presentation time"));
    assert!(first_text.contains(first.context.state_dir.as_ref().unwrap()));
    let mut changed = first.clone();
    changed.format = OutputFormat::Json;
    changed.context.state_dir = Some("/tmp/changed-state".into());
    changed.context.host = Some("/tmp/changed-host".into());
    let mut duplicate_hook = Vec::new();
    run_hook_event_with_reason(
        &intents,
        &contexts,
        &event,
        Some(&seed),
        2,
        &client,
        fixture.clock.as_ref(),
        &changed,
        OverviewReason::Lifecycle,
        &mut duplicate_hook,
    )
    .unwrap();
    assert_eq!(
        contexts
            .dispatch(
                &event.event_id,
                &mut |_pending: &herdr_threads::harness::context::PendingCheckIn| panic!(
                    "completed CheckIn must not redispatch"
                )
            )
            .unwrap()
            .output,
        saved.output
    );
    let duplicate_text = String::from_utf8(duplicate_hook).unwrap();
    assert!(duplicate_text.contains(first.context.state_dir.as_ref().unwrap()));
    assert!(duplicate_text.contains("/tmp/changed-state"));

    struct LoseFirst<'a> {
        client: &'a LocalSocketClient,
        lost: AtomicBool,
        committed: std::sync::Mutex<Option<Vec<u8>>>,
    }
    impl LocalClient for LoseFirst<'_> {
        fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
            LocalClient::call(self.client, command, budget)
        }
        fn call_with_output(
            &self,
            command: Command,
            output: &herdr_threads::protocol::output::OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            let check_in = matches!(command, Command::CheckIn(_));
            let result = LocalClient::call_with_output(self.client, command, output, budget)?;
            if check_in && !self.lost.swap(true, Ordering::SeqCst) {
                *self.committed.lock().unwrap() = Some(serde_json::to_vec(&result).unwrap());
                return Err(ApiError::unknown_outcome(
                    "fixture lost committed CheckIn response",
                ));
            }
            Ok(result)
        }
    }
    let lost = LoseFirst {
        client: &client,
        lost: AtomicBool::new(false),
        committed: Default::default(),
    };
    let mut resume = event.clone();
    resume.kind = EventKind::Resume;
    resume.event_id = "selected-socket-resume".into();
    let mut lost_output = Vec::new();
    assert!(
        run_hook_event_with_reason(
            &intents,
            &contexts,
            &resume,
            None,
            3,
            &lost,
            fixture.clock.as_ref(),
            &first,
            OverviewReason::Lifecycle,
            &mut lost_output
        )
        .is_err()
    );
    assert!(lost_output.is_empty());
    let pending = contexts.pending().unwrap().unwrap();
    drop(contexts);
    let contexts = ContextJournal::open(
        &fixture.root.canonicalize().unwrap(),
        fixture.descriptor.instance_uuid,
        seat.as_str(),
        Duration::from_millis(500),
    )
    .unwrap();
    assert_eq!(contexts.pending().unwrap().unwrap(), pending);
    let mut replay_hook = Vec::new();
    run_hook_event_with_reason(
        &intents,
        &contexts,
        &resume,
        None,
        4,
        &lost,
        fixture.clock.as_ref(),
        &changed,
        OverviewReason::Lifecycle,
        &mut replay_hook,
    )
    .unwrap();
    let replay_saved = contexts
        .dispatch(
            &resume.event_id,
            &mut |_pending: &herdr_threads::harness::context::PendingCheckIn| {
                panic!("cached replay")
            },
        )
        .unwrap();
    assert_eq!(
        replay_saved.output,
        lost.committed.lock().unwrap().clone().unwrap()
    );
    assert!(contexts.pending().unwrap().is_none());
    let replay_text = String::from_utf8(replay_hook).unwrap();
    assert!(replay_text.contains(first.context.state_dir.as_ref().unwrap()));
    assert!(replay_text.contains("/tmp/changed-state"));

    let bound = herdr_threads::cli::SelectedSocketClient {
        client: &client,
        output: &first,
    };
    let mut clear = event.clone();
    clear.kind = EventKind::Clear;
    clear.event_id = "selected-wrapper-clear".into();
    let mut wrapper_hook = Vec::new();
    run_hook_event_with_reason(
        &intents,
        &contexts,
        &clear,
        None,
        5,
        &bound,
        fixture.clock.as_ref(),
        &first,
        OverviewReason::Lifecycle,
        &mut wrapper_hook,
    )
    .unwrap();
    assert!(
        String::from_utf8(wrapper_hook)
            .unwrap()
            .contains(first.context.state_dir.as_ref().unwrap())
    );
    let mut mismatched = event.clone();
    mismatched.kind = EventKind::Restart;
    mismatched.event_id = "selected-wrapper-mismatch".into();
    let mut rejected_output = Vec::new();
    let error = run_hook_event_with_reason(
        &intents,
        &contexts,
        &mismatched,
        None,
        6,
        &bound,
        fixture.clock.as_ref(),
        &changed,
        OverviewReason::Lifecycle,
        &mut rejected_output,
    )
    .unwrap_err();
    assert!(
        matches!(error, herdr_threads::harness::bridge::BridgeError::Api(ref api)
        if api.code == ErrorCode::InvalidRequest)
    );
    assert!(rejected_output.is_empty());
}

#[test]
fn real_ipc_resolve_preserves_retained_recovery_hold() {
    let fixture = Fixture::new(true);
    assert_eq!(
        fixture.resolve("held-target").unwrap_err().code,
        ErrorCode::TargetUnresolved
    );
    let db = fixture.db();
    let count: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    let hold: i64 = db
        .query_row(
            "SELECT baseline_hold_unclaimed FROM host_instances",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hold, 1);
}

#[test]
fn real_ipc_late_target_read_cannot_publish_after_newer_invalidation_and_health_progresses() {
    let fixture = Fixture::new(false);
    fixture.host.held.store(true, Ordering::SeqCst);
    let client = fixture.client();
    let budget = fixture.budget();
    let request = std::thread::spawn(move || {
        LocalClient::call(
            &client,
            Command::ResolveSeat(ResolveSeat {
                target: HostTargetId::new("pane"),
                operation: OperationId::new("late-read"),
            }),
            &budget,
        )
    });
    let request = HeldRequest {
        host: fixture.host.clone(),
        worker: Some(request),
    };
    fixture
        .entered
        .recv_timeout(Duration::from_secs(30))
        .expect("service did not admit target read");
    assert!(matches!(
        LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap(),
        CommandResult::Health(_)
    ));
    let admission = fixture
        .store
        .begin_host_observation(
            &fixture.descriptor.instance_uuid.to_string(),
            &fixture.budget(),
        )
        .unwrap();
    fixture
        .store
        .invalidate_host_observation(
            &admission,
            HostInvalidationReason::HostUnavailable,
            &fixture.budget(),
        )
        .unwrap()
        .unwrap();
    fixture.host.held.store(false, Ordering::SeqCst);
    assert_eq!(
        request.join().unwrap().unwrap_err().code,
        ErrorCode::StaleHostObservation
    );
    let db = fixture.db();
    let counts: (i64,i64) = db.query_row("SELECT (SELECT count(*) FROM observed_targets),(SELECT count(*) FROM allocation_decisions)", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
}

/// A failed target read (Herdr unavailable) is missing evidence, not evidence
/// of change (TRUST-POLICY C4, ht-yms): the request is refused as transient,
/// nothing is invalidated, the published snapshot stays effective and nothing
/// is allocated.
/// Kills: `OrdinaryIdentity::invalidate` writing an invalidation for an
/// unavailability reason (the revision moves to 1).
#[test]
fn real_ipc_failed_target_read_freezes_effective_snapshot_without_allocating() {
    let fixture = Fixture::new(false);
    fixture.host.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.resolve("failed-read").unwrap_err().code,
        ErrorCode::HostUnavailable
    );
    let db = fixture.db();
    let frozen: (i64,i64) = db.query_row("SELECT invalidation_revision,(SELECT count(*) FROM allocation_decisions) FROM host_instances", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(frozen, (0, 0));
    // The frozen view is still effective: once Herdr answers, the same target
    // resolves against it.
    fixture.host.fail.store(false, Ordering::SeqCst);
    assert!(matches!(
        fixture.resolve("after-freeze").unwrap(),
        CommandResult::SeatResolved(_)
    ));
}

/// An incoherent target read (an answer without verified structural proof)
/// is evidence: it durably invalidates the effective snapshot
/// (`CoherenceLost`) and allocates nothing.
/// Kills: dropping the `CoherenceLost` invalidation from the target read, or
/// treating it as unavailability (the revision stays 0).
#[test]
fn real_ipc_incoherent_target_read_invalidates_effective_snapshot_without_allocating() {
    let fixture = Fixture::new(false);
    fixture.host.incoherent.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.resolve("incoherent-read").unwrap_err().code,
        ErrorCode::StaleHostObservation
    );
    let db = fixture.db();
    let invalidated: (i64,i64) = db.query_row("SELECT invalidation_revision,(SELECT count(*) FROM allocation_decisions) FROM host_instances", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(invalidated, (1, 0));
}

#[test]
fn real_ipc_new_operation_reuses_current_structural_owner_without_allocation_churn() {
    let fixture = Fixture::new(false);
    let CommandResult::SeatResolved(first) = fixture.resolve("first").unwrap() else {
        panic!("missing first seat");
    };
    let CommandResult::SeatResolved(second) = fixture.resolve("second").unwrap() else {
        panic!("missing second seat");
    };
    assert_eq!(first, second);
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 2);
    let count: i64 = fixture
        .db()
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn real_ipc_exact_resolution_replay_after_retirement_and_hold_is_historical_without_host_or_revival()
 {
    let fixture = Fixture::new(false);
    let CommandResult::SeatResolved(first) = fixture.resolve("original").unwrap() else {
        panic!("missing seat");
    };
    let db = fixture.db();
    db.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id=?1",
        [first.as_str()],
    )
    .unwrap();
    db.execute("UPDATE host_instances SET baseline_hold_unclaimed=1", [])
        .unwrap();
    // Keep the hold legitimately in force (TRUST-POLICY C1-C3, main's B5):
    // the baseline hold lifts as soon as a reconciliation pass finds no
    // unresolved nonretired seat, and this branch's commit kicks run that
    // pass promptly, so an unresolved saved seat stands for the seats the
    // hold protects.
    db.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,generation,created_at) SELECT 'still-unresolved',instance_id,'unresolved','other','native',1,0 FROM seats WHERE id=?1",
        [first.as_str()],
    )
    .unwrap();
    fixture.host.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.resolve("original").unwrap(),
        CommandResult::SeatResolved(first.clone())
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 1);
    let state: (String, i64) = db
        .query_row(
            "SELECT state,(SELECT count(*) FROM allocation_decisions) FROM seats WHERE id=?1",
            [first.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), 1));
    fixture.host.fail.store(false, Ordering::SeqCst);
    assert_eq!(
        fixture.resolve("new-key").unwrap_err().code,
        ErrorCode::TargetUnresolved
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 2);
    let count: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn real_ipc_reused_resolution_key_for_different_target_rejects_before_host_read() {
    let fixture = Fixture::new(false);
    fixture.resolve("same-key").unwrap();
    let result = LocalClient::call(
        &fixture.client(),
        Command::ResolveSeat(ResolveSeat {
            target: HostTargetId::new("other"),
            operation: OperationId::new("same-key"),
        }),
        &fixture.budget(),
    );
    assert_eq!(
        result.unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn real_cli_resolve_journals_before_submission_and_exact_retry_survives_output_failure() {
    use herdr_threads::cli::{
        self,
        journal::{IntentScope, Journal},
    };
    use std::io::{self, Write};
    struct FailedFlush(Vec<u8>);
    impl Write for FailedFlush {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("controlled output loss"))
        }
    }
    let fixture = Fixture::new(false);
    let argv = vec![
        "herdr-threads".to_owned(),
        "--state-dir".into(),
        fixture.root.join("state").to_string_lossy().into_owned(),
        "--host-endpoint".into(),
        fixture
            .root
            .join("host.sock")
            .to_string_lossy()
            .into_owned(),
        "--json".into(),
        "seat".into(),
        "resolve".into(),
        "--pane".into(),
        "pane".into(),
    ];
    let mut failed = FailedFlush(Vec::new());
    assert!(cli::run(argv, &mut failed).is_err());
    let journal = Journal::open(fixture.paths.instance_dir.join("intents")).unwrap();
    let reference = journal
        .resolve_recovery_ref("local:1")
        .expect("resolve did not retain its private intent before submission");
    let pending = journal.load(&reference).unwrap();
    assert_eq!(
        pending.header.scope,
        IntentScope::ServiceAllocation {
            instance: fixture.descriptor.instance_uuid.to_string(),
            target: HostTargetId::new("pane")
        }
    );
    let db = fixture.db();
    let committed: String = db
        .query_row(
            "SELECT result_json FROM operations WHERE operation_key=?1",
            [reference.operation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let CommandResult::SeatResolved(seat) = serde_json::from_str(&committed).unwrap() else {
        panic!("CLI did not commit resolution");
    };
    fixture.host.fail.store(true, Ordering::SeqCst);
    let retry_argv = vec![
        "herdr-threads".to_owned(),
        "--state-dir".into(),
        fixture.root.join("state").to_string_lossy().into_owned(),
        "--host-endpoint".into(),
        fixture
            .root
            .join("host.sock")
            .to_string_lossy()
            .into_owned(),
        "--json".into(),
        "retry".into(),
        "local:1".into(),
    ];
    let mut output = Vec::new();
    cli::run(retry_argv, &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().contains(seat.as_str()));
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 1);
    assert!(journal.resolve_recovery_ref("local:1").is_err());
    let count: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

/// Kills Mutation W (review S2): reverting the `seat resolve` call site in
/// `cli::run` to `retry::run_new_api_to_writer(..)` + `client.call_with_output(..)`
/// (the pre-fix wiring, composition probe B7 / P2-9). Under W a correlated
/// NotFound/StaleHostObservation leaves a stuck `resolve_seat` pending intent.
/// The unknown-outcome leg kills the opposite mutation (discarding on every
/// failure): an unknown outcome must stay pending for exact `retry`.
#[test]
fn real_cli_resolve_discards_intent_only_after_correlated_definitive_rejection() {
    use herdr_threads::cli::{self, journal::Journal};
    let fixture = Fixture::new(false);
    let state = fixture.root.join("state").to_string_lossy().into_owned();
    let host = fixture
        .root
        .join("host.sock")
        .to_string_lossy()
        .into_owned();
    let argv = |tail: &[&str]| {
        let mut argv = vec![
            "herdr-threads".to_owned(),
            "--state-dir".into(),
            state.clone(),
            "--host-endpoint".into(),
            host.clone(),
            "--json".into(),
        ];
        argv.extend(tail.iter().map(|part| (*part).to_owned()));
        argv
    };
    let pending_ops = || {
        let mut out = Vec::new();
        cli::run(argv(&["pending-ops"]), &mut out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        value
    };
    let journal = Journal::open(fixture.paths.instance_dir.join("intents")).unwrap();
    fixture.host.fail.store(true, Ordering::SeqCst);
    for (mode, code, recovery) in [
        (1, ErrorCode::NotFound, "local:1"),
        (2, ErrorCode::StaleHostObservation, "local:2"),
    ] {
        fixture.host.fail_code.store(mode, Ordering::SeqCst);
        let mut out = Vec::new();
        let failure = cli::run(argv(&["seat", "resolve", "--pane", "pane"]), &mut out)
            .expect_err("rejected resolution reported success");
        let cli::RunError::Api(error) = failure else {
            panic!("expected a daemon rejection, got {failure:?}");
        };
        assert_eq!(error.code, code);
        assert!(
            journal.resolve_recovery_ref(recovery).is_err(),
            "definitively rejected resolve kept intent {recovery}"
        );
        let pending = pending_ops();
        assert_eq!(
            pending["result"]["data"]["items"].as_array().map(Vec::len),
            Some(0),
            "pending-ops after {code:?}: {pending}"
        );
    }
    // Unknown outcome: the daemon holds the capture until the test releases
    // it, past both the CLI's client deadline and the daemon's own host-call
    // budget, so the CLI sees no correlated answer and must keep the intent.
    // Deterministic: the stale `fail_code` of the previous leg is cleared, and
    // the double never answers at daemon-budget exhaustion while held, so the
    // daemon cannot win a race against the client deadline (review S1).
    fixture.host.fail.store(false, Ordering::SeqCst);
    fixture.host.fail_code.store(0, Ordering::SeqCst);
    while fixture.entered.try_recv().is_ok() {}
    fixture.host.held_past_budget.store(true, Ordering::SeqCst);
    let failure = cli::run(
        argv(&["seat", "resolve", "--pane", "pane"]),
        &mut Vec::new(),
    )
    .expect_err("held resolution reported success");
    // Explicit synchronization: the capture was entered and was still held
    // (unanswered) when the CLI gave up, so the outcome is the client's.
    fixture
        .entered
        .try_recv()
        .expect("held resolution never reached the host capture");
    assert_eq!(
        fixture.host.active_reads.load(Ordering::SeqCst),
        1,
        "capture was not still held when the CLI returned"
    );
    fixture.host.held_past_budget.store(false, Ordering::SeqCst);
    let released = std::time::Instant::now() + Duration::from_secs(30);
    while fixture.host.active_reads.load(Ordering::SeqCst) != 0 {
        assert!(
            std::time::Instant::now() < released,
            "released capture did not return"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let cli::RunError::Api(error) = failure else {
        panic!("expected an unknown outcome, got {failure:?}");
    };
    assert_eq!(error.code, ErrorCode::UnknownOutcome, "{error:?}");
    let reference = journal
        .resolve_recovery_ref("local:3")
        .expect("unknown-outcome resolve discarded its intent");
    assert!(journal.load(&reference).is_ok());
    let pending = pending_ops();
    assert_eq!(
        pending["result"]["data"]["items"].as_array().map(Vec::len),
        Some(1),
        "pending-ops after UnknownOutcome: {pending}"
    );
}

/// Kills a Health mutation that reports the host as verified (or leaves the
/// stale hard-coded constants) regardless of evidence: an elected daemon whose
/// adapter captures only Unknown-incarnation snapshots (the non-macOS
/// NativeCli shape) must report coherent enumeration Unsupported, a
/// non-ready host and no reconciliation time. After the adapter starts
/// producing verified coherent snapshots, Health reports them.
#[test]
fn elected_health_reports_unverified_then_verified_host_evidence() {
    use herdr_threads::protocol::results::{CapabilityState, ComponentState};
    // Mode 4 holds the first capture at entry; release it as an
    // Unknown-incarnation capture. No settling sleep: the double's writer
    // probe waits a bounded time for unrelated elected startup writers
    // (`HOST_IO_WRITER_PROBE_WAIT`) instead of racing them at zero timeout.
    let fixture = Fixture::start(false, false, 4);
    fixture.host.snapshot_mode.store(3, Ordering::SeqCst);
    fixture.host.snapshot_held.store(false, Ordering::SeqCst);
    let health = |fixture: &Fixture| {
        let CommandResult::Health(health) =
            LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap()
        else {
            panic!("missing health");
        };
        health
    };
    let until = std::time::Instant::now() + Duration::from_secs(30);
    let unverified = loop {
        let health = health(&fixture);
        if health.host.coherent_enumeration != CapabilityState::Unknown
            || std::time::Instant::now() >= until
        {
            break health;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(fixture.host.snapshots.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        unverified.host.coherent_enumeration,
        CapabilityState::Unsupported,
        "{unverified:?}"
    );
    assert_ne!(unverified.host.reachability, ComponentState::Ready);
    assert_eq!(unverified.last_reconciliation_at, None);
    assert!(
        unverified
            .limitations
            .iter()
            .any(|line| line.contains("no verified server incarnation")),
        "{:?}",
        unverified.limitations
    );
    fixture.host.snapshot_mode.store(0, Ordering::SeqCst);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    let verified = loop {
        let health = health(&fixture);
        if health.last_reconciliation_at.is_some() || std::time::Instant::now() >= until {
            break health;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        verified.host.coherent_enumeration,
        CapabilityState::Supported,
        "{verified:?}"
    );
    assert_eq!(verified.host.reachability, ComponentState::Ready);
    assert!(verified.last_reconciliation_at.is_some());
}

/// Worker-level: after a verified publication, an observation attempt that
/// ends in `Err` (here `begin_host_observation` cannot take the SQLite writer
/// within its budget, so the host is never enumerated) must not leave Health
/// reporting a `Ready` host with `Supported` coherent enumeration.
///
/// Kills: the observation worker's `capture_if_due` `Err` arm not recording
/// an explicit non-verified host evidence state (the fix1 N1 shape, where
/// the prior `Published` stays sticky and Health over-claims).
#[test]
fn elected_health_does_not_keep_verified_host_after_errored_capture() {
    use herdr_threads::protocol::results::{CapabilityState, ComponentState};
    let fixture = Fixture::start(false, false, 0);
    let health = |fixture: &Fixture| {
        let CommandResult::Health(health) =
            LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap()
        else {
            panic!("missing health");
        };
        health
    };
    let wait_verified = |fixture: &Fixture, what: &str| {
        let until = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let health = health(fixture);
            if health.host.coherent_enumeration == CapabilityState::Supported
                && health.host.reachability == ComponentState::Ready
            {
                break health;
            }
            assert!(
                std::time::Instant::now() < until,
                "{what}: no verified publication: {health:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    wait_verified(&fixture, "initial");

    // Hold the SQLite writer from outside the daemon. Every later
    // observation attempt now ends in `Err`: admission cannot take the
    // writer, and a capture already admitted cannot publish nor durably
    // invalidate. No `Invalidated` outcome can be recorded while the writer
    // is held, so any change in Health comes from the errored attempt.
    fixture
        .host
        .snapshot_writer_probe
        .store(false, Ordering::SeqCst);
    let db = fixture.db();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let held = HeldSqliteWriterGuard(&db);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    let errored = loop {
        let health = health(&fixture);
        if health.host.reachability != ComponentState::Ready {
            break health;
        }
        assert!(
            std::time::Instant::now() < until,
            "errored capture left a verified host in Health: {health:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        errored.host.reachability,
        ComponentState::Degraded,
        "{errored:?}"
    );
    assert_eq!(
        errored.host.coherent_enumeration,
        CapabilityState::Unknown,
        "{errored:?}"
    );
    assert!(
        errored
            .limitations
            .iter()
            .any(|line| line.contains("latest host capture attempt errored")),
        "{:?}",
        errored.limitations
    );

    // Recovery: once the writer is free a later verified publication wins.
    drop(held);
    fixture
        .host
        .snapshot_writer_probe
        .store(true, Ordering::SeqCst);
    wait_verified(&fixture, "after writer release");
}

/// Worker-level monotone evidence rule: after a verified fail-closed
/// `Invalidated(HostUnavailable)` outcome, observation attempts that end in
/// `Err` (the SQLite writer is held from outside, so admission cannot begin)
/// must keep Health's host `unavailable`, never raise it to `degraded`.
///
/// The test only passes once the errored attempt is witnessed in the host
/// limitation (appended to the preserved unavailable detail), so it cannot
/// pass vacuously because no attempt ran while the writer was held.
///
/// Kills: M1 "Errored overwrites any prior evidence" (the fix2 shape: the
/// reviewed repro, unavailable -> degraded while the writer is held).
#[test]
fn elected_health_keeps_unavailable_host_after_errored_capture() {
    use herdr_threads::protocol::results::{CapabilityState, ComponentState};
    let fixture = Fixture::start(false, false, 1);
    let health = |fixture: &Fixture| {
        let CommandResult::Health(health) =
            LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap()
        else {
            panic!("missing health");
        };
        health
    };
    let until = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let health = health(&fixture);
        if health.host.reachability == ComponentState::Unavailable {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "no HostUnavailable invalidation: {health:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    fixture
        .host
        .snapshot_writer_probe
        .store(false, Ordering::SeqCst);
    let db = fixture.db();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let held = HeldSqliteWriterGuard(&db);
    let errored_witness = |line: &String| {
        line.starts_with("host unavailable: latest host capture failed: ")
            && line.contains("later host capture attempt errored without a verified outcome")
    };
    let until = std::time::Instant::now() + Duration::from_secs(30);
    let mut witnessed_polls = 0;
    // Keep polling for a few more rounds after the first witness so a
    // Health read racing the errored write cannot pass by luck.
    while witnessed_polls < 5 {
        let health = health(&fixture);
        assert_eq!(
            health.host.reachability,
            ComponentState::Unavailable,
            "errored capture raised an unavailable host: {health:?}"
        );
        assert_eq!(
            health.host.coherent_enumeration,
            CapabilityState::Unknown,
            "{health:?}"
        );
        if health.limitations.iter().any(errored_witness) {
            witnessed_polls += 1;
        }
        assert!(
            std::time::Instant::now() < until,
            "no errored capture attempt witnessed while the writer was held: {health:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(held);
    fixture
        .host
        .snapshot_writer_probe
        .store(true, Ordering::SeqCst);
}

struct HeldSqliteWriterGuard<'a>(&'a rusqlite::Connection);
impl Drop for HeldSqliteWriterGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("ROLLBACK");
    }
}

#[test]
fn committed_resolution_with_lost_wire_response_retains_exact_intent_for_explicit_retry() {
    use herdr_threads::cli::journal::{IntentScope, Journal, SemanticMutation};
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    let fixture = Fixture::new(false);
    let journal = Journal::open(fixture.paths.instance_dir.join("intents")).unwrap();
    let semantic = SemanticMutation::ResolveSeat {
        target: HostTargetId::new("pane"),
    };
    let reference = journal
        .record(
            IntentScope::ServiceAllocation {
                instance: fixture.descriptor.instance_uuid.to_string(),
                target: HostTargetId::new("pane"),
            },
            semantic.clone(),
            fixture.clock.utc_now().0,
        )
        .unwrap();
    let command = semantic
        .to_command(reference.operation.clone(), None)
        .unwrap();
    let proxy_path = std::env::temp_dir().join(format!("h-{}.sock", Uuid::new_v4().simple()));
    let listener = UnixListener::bind(&proxy_path).unwrap();
    let endpoint = fixture.descriptor.endpoint.clone();
    let (committed_tx, committed_rx) = mpsc::sync_channel(1);
    let proxy = std::thread::spawn(move || {
        let (mut caller, _) = listener.accept().unwrap();
        let mut owner = UnixStream::connect(endpoint).unwrap();
        let mut prefix = [0; 4];
        caller.read_exact(&mut prefix).unwrap();
        let mut body = vec![0; u32::from_be_bytes(prefix) as usize];
        caller.read_exact(&mut body).unwrap();
        owner.write_all(&prefix).unwrap();
        owner.write_all(&body).unwrap();
        owner.read_exact(&mut prefix).unwrap();
        let mut response = vec![0; u32::from_be_bytes(prefix) as usize];
        owner.read_exact(&mut response).unwrap();
        committed_tx.send(response).unwrap();
        // The owner replied after its durable decision, but the caller sees EOF.
    });
    let proxy_client = LocalSocketClient::new(
        proxy_path.clone(),
        fixture.clock.clone(),
        fixture.descriptor.instance_uuid,
        Some(fixture.descriptor.boot_id),
    );
    let lost = LocalClient::call(&proxy_client, command.clone(), &fixture.budget()).unwrap_err();
    assert_eq!(lost.code, ErrorCode::UnknownOutcome);
    let wire: herdr_threads::protocol::wire::WireResponse =
        serde_json::from_slice(&committed_rx.recv_timeout(Duration::from_secs(30)).unwrap())
            .unwrap();
    proxy.join().unwrap();
    fs::remove_file(proxy_path).unwrap();
    let committed = wire.result.unwrap();
    let saved = journal.load(&reference).unwrap();
    assert_eq!(saved.operation, reference.operation);
    assert_eq!(saved.semantic, semantic);
    let db = fixture.db();
    let durable: String = db
        .query_row(
            "SELECT result_json FROM operations WHERE operation_key=?1",
            [reference.operation.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<CommandResult>(&durable).unwrap(),
        committed
    );
    let captures = fixture.host.calls.load(Ordering::SeqCst);
    assert_eq!(
        LocalClient::call(&fixture.client(), command, &fixture.budget()).unwrap(),
        committed
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), captures);
    let decisions: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(decisions, 1);
}

#[test]
fn disconnected_short_budget_cannot_commit_queued_resolution_after_writer_unlocks() {
    #[cfg(feature = "test-support")]
    use herdr_threads::test_support::server_completion::ServerCompletion;
    let fixture = Fixture::new(false);
    let db = fixture.db();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    #[cfg(feature = "test-support")]
    let held_writer = HeldSqliteWriter(&db);
    let command = Command::ResolveSeat(ResolveSeat {
        target: HostTargetId::new("pane"),
        operation: OperationId::new("expired-queued-resolution"),
    });
    #[cfg(feature = "test-support")]
    let completion = ServerCompletion::resolution("expired-queued-resolution");
    let budget = CallBudget {
        deadline: MonoInstant(fixture.clock.monotonic_now().0 + 150),
        cancellation: Cancellation::default(),
    };
    #[cfg(feature = "test-support")]
    let caller = JoinedCall {
        cancellation: budget.cancellation.clone(),
        worker: Some(std::thread::spawn({
            let client = fixture.client();
            let command = command.clone();
            move || LocalClient::call(&client, command, &budget)
        })),
    };
    #[cfg(feature = "test-support")]
    assert!(
        completion.wait_worker_entered(Duration::from_secs(30)),
        "original server handler was not admitted while SQLite writer was held"
    );
    #[cfg(feature = "test-support")]
    let outcome = caller.join().unwrap().unwrap_err();
    #[cfg(not(feature = "test-support"))]
    let outcome = LocalClient::call(&fixture.client(), command.clone(), &budget).unwrap_err();
    assert_eq!(outcome.code, ErrorCode::UnknownOutcome);
    #[cfg(feature = "test-support")]
    drop(held_writer);
    #[cfg(not(feature = "test-support"))]
    db.execute_batch("ROLLBACK").unwrap();
    #[cfg(feature = "test-support")]
    assert!(
        completion.wait_worker_exited(Duration::from_secs(30)),
        "original server handler did not exit after caller expiry and writer release"
    );
    #[cfg(not(feature = "test-support"))]
    std::thread::sleep(Duration::from_millis(150));
    let decisions: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(decisions, 0);
    let operations: i64 = db
        .query_row(
            "SELECT count(*) FROM operations WHERE operation_key='expired-queued-resolution'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(operations, 0);
    // The observation lane may republish meanwhile; the daemon re-admits a
    // superseded read itself (SUPERSEDED_READ_ATTEMPTS), so one call resolves.
    assert!(matches!(
        LocalClient::call(&fixture.client(), command, &fixture.budget()).unwrap(),
        CommandResult::SeatResolved(_)
    ));
    let decisions: i64 = db
        .query_row("SELECT count(*) FROM allocation_decisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(decisions, 1);
}

#[cfg(feature = "test-support")]
#[test]
fn held_real_sqlite_search_allows_health_and_writer_then_cancels_and_reuses_worker() {
    use herdr_threads::{
        protocol::pagination::PageRequest,
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::{Ack, CheckIn, CheckInMode, SearchQuery},
        },
        test_support::{search_barrier::SearchBarrier, server_completion::ServerCompletion},
    };
    // The daemon's background snapshot capture stays held (mode 4) so its
    // reconciliation cannot move the hand-inserted check-in seat's mapping
    // between the insert and the check-in below; this test is about the held
    // search, not the observation lane.
    let fixture = Fixture::start(false, true, 4);
    let literal = format!("held-search-{}", Uuid::new_v4().simple());
    let successor_literal = format!("successor-search-{}", Uuid::new_v4().simple());
    let db = fixture.db();
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('search-thread',?1,?2,'goal',0,0)",
        rusqlite::params![fixture.descriptor.instance_uuid.to_string(), literal],
    ).unwrap();
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('successor-thread',?1,?2,'goal',0,0)",
        rusqlite::params![fixture.descriptor.instance_uuid.to_string(), successor_literal],
    ).unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('checkin-seat',?1,'resolved','native','pane',0,1,0)",
        [fixture.descriptor.instance_uuid.to_string()],
    ).unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('sender',?1,'resolved','native',1,0)",
        [fixture.descriptor.instance_uuid.to_string()],
    ).unwrap();
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq)
        SELECT 'search-message',id,'search-thread',1,'ordinary','sender','hello',0,1000 FROM host_instances;
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at)
        VALUES ('search-message','search-thread','checkin-seat','pending',300000,0,9223372036854775807);
        UPDATE threads SET next_sequence=2 WHERE id='search-thread';").unwrap();
    let query = Command::Search(SearchQuery {
        literal: literal.clone(),
        thread: None,
        page: PageRequest::default(),
        max_candidates: 100,
    });
    let completion = ServerCompletion::search(literal.clone());
    let barrier = SearchBarrier::install_until_cancelled(literal);
    let cancellation = Cancellation::default();
    let client = fixture.client();
    let budget = CallBudget {
        deadline: MonoInstant(fixture.clock.monotonic_now().0 + 5_000),
        cancellation: cancellation.clone(),
    };
    let held = JoinedCall {
        cancellation: cancellation.clone(),
        worker: Some(std::thread::spawn({
            let query = query.clone();
            move || LocalClient::call(&client, query, &budget)
        })),
    };
    assert!(
        barrier.wait_entered(1, Duration::from_secs(30)),
        "search did not reach SQLite transaction"
    );
    assert!(matches!(
        LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap(),
        CommandResult::Health(_)
    ));
    let claim = CallerClaim {
        instance: fixture.descriptor.instance_uuid.to_string(),
        seat: SeatId::new("checkin-seat"),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("plugin_context:search-checkin"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("pane"),
    };
    let check_in = Command::CheckIn(CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: 0,
        },
        claim: claim.clone(),
        operation: OperationId::new("writer-during-held-search"),
    });
    assert!(matches!(
        LocalClient::call(&fixture.client(), check_in, &fixture.budget()).unwrap(),
        CommandResult::CheckedIn(_)
    ));
    let generation: i64 = db
        .query_row(
            "SELECT generation FROM seats WHERE id='checkin-seat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(generation, 1);
    let ack = Command::Ack(Ack {
        messages: vec![MessageId::new("search-message")],
        operation: OperationId::new("ack-during-held-search"),
        claim: CallerClaim {
            binding_generation: 1,
            ..claim
        },
    });
    assert!(matches!(
        LocalClient::call(&fixture.client(), ack, &fixture.budget()).unwrap(),
        CommandResult::Acknowledged(_)
    ));
    let receipt: String = db.query_row("SELECT state FROM receipts WHERE message_id='search-message' AND seat_id='checkin-seat'", [], |row| row.get(0)).unwrap();
    assert_eq!(receipt, "acked");
    cancellation.cancel();
    assert_eq!(
        held.join().unwrap().unwrap_err().code,
        ErrorCode::UnknownOutcome
    );
    assert!(
        completion.wait_worker_exited(Duration::from_secs(30)),
        "cancelled search worker remained inside its SQLite transaction"
    );
    assert!(
        completion.wait_search_permits_returned(Duration::from_secs(30)),
        "cancelled search retained active or queue permit"
    );
    assert!(
        !barrier.is_released(),
        "test released the original barrier early"
    );
    let successor = Command::Search(SearchQuery {
        literal: successor_literal,
        thread: None,
        page: PageRequest::default(),
        max_candidates: 100,
    });
    let CommandResult::Search(successor_page) =
        LocalClient::call(&fixture.client(), successor, &fixture.budget()).unwrap()
    else {
        panic!("successor did not reach search worker");
    };
    assert!(matches!(
        successor_page.matches.items.as_slice(),
        [herdr_threads::protocol::results::SearchHit::Topic(topic)]
            if topic.thread.as_str() == "successor-thread"
    ));
    assert!(
        !barrier.is_released(),
        "successor waited for original barrier release"
    );
    barrier.release();
}

#[cfg(feature = "test-support")]
#[test]
fn composed_search_admits_one_active_four_waiters_and_releases_all_slots() {
    use herdr_threads::{
        protocol::{commands::SearchQuery, pagination::PageRequest},
        test_support::search_barrier::SearchBarrier,
    };
    let fixture = Fixture::new(false);
    let literal = format!("queue-search-{}", Uuid::new_v4().simple());
    fixture.db().execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('queue-thread',?1,?2,'goal',0,0)",
        rusqlite::params![fixture.descriptor.instance_uuid.to_string(), literal],
    ).unwrap();
    let query = Command::Search(SearchQuery {
        literal: literal.clone(),
        thread: None,
        page: PageRequest::default(),
        max_candidates: 100,
    });
    let barrier = SearchBarrier::install_stalled(literal);
    let mut callers = Vec::new();
    for admitted in 1..=5 {
        let client = fixture.client();
        let command = query.clone();
        let budget = fixture.budget();
        callers.push(JoinedCall {
            cancellation: budget.cancellation.clone(),
            worker: Some(std::thread::spawn(move || {
                LocalClient::call(&client, command, &budget)
            })),
        });
        assert!(
            barrier.wait_admitted(admitted, Duration::from_secs(30)),
            "search slot {admitted} was not admitted"
        );
    }
    assert!(
        barrier.wait_entered(1, Duration::from_secs(30)),
        "active search did not enter the SQLite worker"
    );
    assert_eq!(
        barrier.entered(),
        1,
        "queued searches entered the SQLite worker"
    );
    let rejected =
        LocalClient::call(&fixture.client(), query.clone(), &fixture.budget()).unwrap_err();
    assert_eq!(rejected.code, ErrorCode::StoreBusy);
    assert!(matches!(
        LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap(),
        CommandResult::Health(_)
    ));
    barrier.release();
    for caller in callers {
        let outcome = caller.join().unwrap();
        assert!(
            matches!(
                outcome,
                Ok(CommandResult::Search(_))
                    | Err(ApiError {
                        code: ErrorCode::ReadBudgetExhausted,
                        ..
                    })
            ),
            "{outcome:?}"
        );
    }
    assert!(matches!(
        LocalClient::call(&fixture.client(), query, &fixture.budget()).unwrap(),
        CommandResult::Search(_)
    ));
}

#[test]
fn identity_final_currentness_check_uses_new_admitted_read_and_rejects_existing_owner_hold() {
    use herdr_threads::{identity::OrdinaryIdentity, service::fair_writer::FairWriter};
    let fixture = Fixture::new(false);
    // This test's own `OrdinaryIdentity` is a second observation lane beside
    // the daemon's. A daemon snapshot admitted after the test's own read would
    // supersede it (`StaleHostObservation`) and its capture could overlap the
    // test's reads, so the daemon's background refresh is parked first: wait
    // out the boot capture, hold every later capture inside the host double
    // (its admission is already issued by then, so it is older than anything
    // the test admits), and let the resolve's own kick-driven refresh enter
    // the hold before the test reads.
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while fixture.host.snapshots.load(Ordering::SeqCst) == 0
        || fixture.host.active_reads.load(Ordering::SeqCst) != 0
    {
        assert!(
            std::time::Instant::now() < until,
            "boot snapshot capture never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let boot_snapshots = fixture.host.snapshots.load(Ordering::SeqCst);
    fixture.host.overlap_allowed.store(true, Ordering::SeqCst);
    fixture.host.snapshot_held.store(true, Ordering::SeqCst);
    let request = ResolveSeat {
        target: HostTargetId::new("pane"),
        operation: OperationId::new("historical-resolve"),
    };
    let CommandResult::SeatResolved(seat) = fixture.resolve(request.operation.as_str()).unwrap()
    else {
        panic!("missing seat");
    };
    while fixture.host.snapshots.load(Ordering::SeqCst) == boot_snapshots {
        assert!(
            std::time::Instant::now() < until,
            "the resolve's kick-driven refresh never reached capture"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let identity = OrdinaryIdentity::new(
        fixture.descriptor.instance_uuid.to_string(),
        fixture.store.clone(),
        fixture.host.clone(),
        fixture.clock.clone(),
        Arc::new(FairWriter::new(32)),
    );
    identity
        .check_current_target(seat.clone(), request.clone(), &fixture.budget())
        .unwrap();
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 2);
    let db = fixture.db();
    db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,'pane','host',1,'repair')", [fixture.descriptor.instance_uuid.to_string()]).unwrap();
    // Exact resolution replay still returns historical data; the independent
    // check must reject the active hold without treating replay as authority.
    assert_eq!(
        fixture.resolve(request.operation.as_str()).unwrap(),
        CommandResult::SeatResolved(seat.clone())
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        identity
            .check_current_target(seat, request, &fixture.budget())
            .unwrap_err()
            .code,
        ErrorCode::TargetUnresolved
    );
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 3);
    let outcomes: (i64, i64) = db
        .query_row(
            "SELECT (SELECT count(*) FROM allocation_decisions),(SELECT count(*) FROM operations)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(outcomes, (1, 1));
}

#[test]
fn elected_snapshot_driver_establishes_baseline_before_real_ipc_resolution() {
    let fixture = Fixture::start(false, false, 0);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let baseline: Option<String> = fixture
            .db()
            .query_row(
                "SELECT recovery_baseline_generation_id FROM host_instances",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
            .flatten();
        if baseline.is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "elected runtime never established a baseline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        fixture.resolve("runtime-baseline").unwrap(),
        CommandResult::SeatResolved(_)
    ));
    assert!(fixture.host.snapshots.load(Ordering::SeqCst) > 0);
}

/// No allocation baseline comes from a failed, partial or unknown capture.
/// Mode 1 (capture unavailable) is Herdr unavailability: it freezes
/// (TRUST-POLICY C4, ht-yms), so no invalidation is written and every saved
/// seat stays resolved. Modes 2 (partial enumeration) and 3 (unknown
/// incarnation) are evidence: they invalidate and the bounded continuation
/// marks every saved seat unresolved. In every mode no baseline is
/// established, resolution is refused and nothing is allocated or retired.
#[test]
fn elected_failed_partial_unknown_captures_cannot_create_allocation_baseline() {
    for mode in 1..=3 {
        let fixture = Fixture::start(false, false, 4);
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while fixture.host.snapshots.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let db = fixture.db();
        for n in 0..17 {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?3,1,1,0)", rusqlite::params![format!("denied-{n}"),fixture.descriptor.instance_uuid.to_string(),format!("missing-{n}")]).unwrap();
        }
        fixture.host.snapshot_mode.store(mode, Ordering::SeqCst);
        fixture.host.snapshot_held.store(false, Ordering::SeqCst);
        let unresolved = || -> i64 {
            db.query_row(
                "SELECT count(*) FROM seats WHERE state='unresolved'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        if mode == 1 {
            // The frozen outcome reaches Health once the lane processed it.
            loop {
                let CommandResult::Health(health) =
                    LocalClient::call(&fixture.client(), Command::Health, &fixture.budget())
                        .unwrap()
                else {
                    panic!("missing health");
                };
                if health.limitations.iter().any(|line| {
                    line.contains("host unavailable (HostUnavailable): seats and bindings frozen")
                }) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "unavailable capture never froze the lane: {:?}",
                    health.limitations
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(unresolved(), 0, "an unavailable capture unresolved seats");
            assert_eq!(invalidation_revision(&fixture), 0);
        } else {
            while unresolved() != 17 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "denied capture did not project its bounded continuation"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(invalidation_revision(&fixture) >= 1);
        }
        assert!(fixture.resolve("denied-baseline").is_err());
        let baseline: Option<String> = db
            .query_row(
                "SELECT recovery_baseline_generation_id FROM host_instances",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
            .flatten();
        assert_eq!(baseline, None, "mode {mode} created an allocation baseline");
        let counts: (i64,i64) = db.query_row("SELECT (SELECT count(*) FROM allocation_decisions),(SELECT count(*) FROM seats WHERE state IN ('retiring','retired'))", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(counts, (0, 0));
    }
}

#[test]
fn elected_hung_snapshot_keeps_health_deadlines_and_owner_until_physical_join() {
    let mut fixture = Fixture::start(false, false, 4);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while fixture.host.snapshots.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    let instance = fixture.descriptor.instance_uuid.to_string();
    let db = fixture.db();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('due',?1,'unresolved','native',1,0)", [&instance]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('due-thread',?1,'topic','goal',0,0)", [&instance]).unwrap();
    db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('due-thread','due','invited'); INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('due-inv','due-thread','due',1,'pending',0,1,100,100);").unwrap();
    assert!(matches!(
        LocalClient::call(&fixture.client(), Command::Health, &fixture.budget()).unwrap(),
        CommandResult::Health(_)
    ));
    // The row above is inserted on a separate connection, so it fires no
    // commit hook and no kick: only the deadline lane's 5 s safety tick finds
    // it (ht-p03.9.4; it used to be a 1 s gate).
    let until = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let warnings: i64 = db
            .query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
                r.get(0)
            })
            .unwrap();
        if warnings > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "snapshot hang blocked deadlines"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        LocalClient::call(
            &fixture.client(),
            Command::Stop(herdr_threads::protocol::commands::StopRequest {
                expected_boot: fixture.descriptor.boot_id.to_string()
            }),
            &fixture.budget()
        )
        .unwrap(),
        CommandResult::StopAccepted(_)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert!(!fixture.daemon.as_ref().unwrap().is_finished());
    assert!(OwnerLock::acquire(&fixture.paths).is_err());
    fixture.host.snapshot_held.store(false, Ordering::SeqCst);
    assert!(fixture.daemon.take().unwrap().join().unwrap().unwrap());
    let active: Option<String> = db
        .query_row("SELECT active_snapshot_id FROM host_instances", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(active.is_none(), "cancelled capture published a baseline");
    assert!(OwnerLock::acquire(&fixture.paths).is_ok());
}

#[test]
fn elected_snapshot_and_target_captures_share_one_lane_and_dirty_refresh() {
    let fixture = Fixture::start(false, false, 4);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while fixture.host.snapshots.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    let client = fixture.client();
    let budget = fixture.budget();
    let request = HeldRequest {
        host: fixture.host.clone(),
        worker: Some(std::thread::spawn(move || {
            LocalClient::call(
                &client,
                Command::ResolveSeat(ResolveSeat {
                    target: HostTargetId::new("pane"),
                    operation: OperationId::new("ordered"),
                }),
                &budget,
            )
        })),
    };
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        fixture.host.calls.load(Ordering::SeqCst),
        0,
        "target capture bypassed snapshot lane"
    );
    fixture.host.snapshot_held.store(false, Ordering::SeqCst);
    assert!(matches!(
        request.join().unwrap().unwrap(),
        CommandResult::SeatResolved(_)
    ));
    while fixture.host.snapshots.load(Ordering::SeqCst) < 2 {
        assert!(
            std::time::Instant::now() < until,
            "target read did not trigger dirty refresh"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn elected_driver_reconciles_multiple_saved_pages_and_reopens_with_fresh_capture() {
    let mut fixture = Fixture::start(false, false, 4);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    while fixture.host.snapshots.load(Ordering::SeqCst) == 0 {
        assert!(std::time::Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    let db = fixture.db();
    for n in 0..33 {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?3,1,1,0)", rusqlite::params![format!("saved-{n}"),fixture.descriptor.instance_uuid.to_string(),format!("missing-{n}")]).unwrap();
    }
    fixture.host.snapshot_held.store(false, Ordering::SeqCst);
    loop {
        let unresolved: i64 = db
            .query_row(
                "SELECT count(*) FROM seats WHERE state='unresolved'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if unresolved == 33 {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "reconciliation stopped at first saved-seat page: {unresolved}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let retired: i64 = db
        .query_row(
            "SELECT count(*) FROM seats WHERE state IN ('retiring','retired')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        retired, 0,
        "missing structural proof supplied absence authority"
    );
    let previous: String = db
        .query_row("SELECT active_snapshot_id FROM host_instances", [], |r| {
            r.get(0)
        })
        .unwrap();
    let old_boot = fixture.descriptor.boot_id;
    fixture.restart();
    assert_ne!(fixture.descriptor.boot_id, old_boot);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let current: Option<String> = db
            .query_row("SELECT active_snapshot_id FROM host_instances", [], |r| {
                r.get(0)
            })
            .unwrap();
        if current.as_deref().is_some_and(|id| id != previous) {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "reopened runtime inherited old in-memory observation lane"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fixture.resolve("reopened-held-target").unwrap_err().code,
        ErrorCode::TargetUnresolved
    );
}

#[test]
fn elected_driver_periodically_refreshes_without_target_requests() {
    let fixture = Fixture::start(false, false, 0);
    let until = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let published: Option<i64> = fixture.db().query_row("SELECT observation_sequence FROM host_instances WHERE active_snapshot_id IS NOT NULL", [], |r| r.get(0)).optional().unwrap();
        if published.is_some_and(|sequence| sequence >= 3) {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "periodic snapshot refresh never published"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(fixture.host.snapshots.load(Ordering::SeqCst) >= 2);
    assert_eq!(fixture.host.calls.load(Ordering::SeqCst), 0);
}

/// Detached invalidation compensation (ht-4is.4.5). The daemon of the fixture
/// is stopped so only the directly constructed identity touches the host.
fn quiesced_identity(
    fixture: &mut Fixture,
    shutdown: Cancellation,
) -> (
    Arc<herdr_threads::identity::OrdinaryIdentity>,
    Arc<herdr_threads::service::fair_writer::FairWriter>,
) {
    use herdr_threads::{identity::OrdinaryIdentity, service::fair_writer::FairWriter};
    fixture.stop.cancel();
    assert!(fixture.daemon.take().unwrap().join().unwrap().unwrap());
    let writer = Arc::new(FairWriter::new(32));
    let identity = OrdinaryIdentity::new(
        fixture.descriptor.instance_uuid.to_string(),
        fixture.store.clone(),
        fixture.host.clone(),
        fixture.clock.clone(),
        Arc::clone(&writer),
    )
    .with_service_cancellation(shutdown);
    (Arc::new(identity), writer)
}

/// Starts a resolve whose target read answers incoherently (no verified
/// structural proof: the evidence-based `CoherenceLost`, which still writes an
/// invalidation; a failed read is unavailability and freezes instead,
/// TRUST-POLICY C4). The read is held until the test holds `writer`, then
/// released; the call returns once the read's compensation is queued behind
/// that held writer turn, observed through `FairWriter::waiting`. The request
/// budget is still live then, so the read passed its own budget check.
fn queued_incoherent_compensation<'w>(
    fixture: &Fixture,
    identity: &Arc<herdr_threads::identity::OrdinaryIdentity>,
    writer: &'w herdr_threads::service::fair_writer::FairWriter,
    operation: &str,
) -> (
    Cancellation,
    mpsc::Receiver<Result<SeatId, ApiError>>,
    std::thread::JoinHandle<()>,
    herdr_threads::service::fair_writer::WriterGuard<'w>,
) {
    fixture.host.incoherent.store(true, Ordering::SeqCst);
    fixture.host.held.store(true, Ordering::SeqCst);
    let request = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(fixture.clock.monotonic_now().0 + 30_000),
        cancellation: request.clone(),
    };
    let (done, result) = mpsc::channel();
    let identity = Arc::clone(identity);
    let operation = OperationId::new(operation);
    let worker = std::thread::spawn(move || {
        let _ = done.send(identity.resolve(
            ResolveSeat {
                target: HostTargetId::new("pane"),
                operation,
            },
            &budget,
        ));
    });
    fixture
        .entered
        .recv_timeout(Duration::from_secs(30))
        .expect("identity did not admit target read");
    // The admission's writer turn ended before the host read began.
    let held = writer
        .enter_foreground(&fixture.budget(), fixture.clock.as_ref())
        .unwrap();
    fixture.host.held.store(false, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while writer.waiting().0 == 0 {
        assert!(
            result.try_recv().is_err(),
            "the incoherent read returned before queuing its compensation"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "compensation never queued behind the held writer"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    (request, result, worker, held)
}

fn invalidation_revision(fixture: &Fixture) -> i64 {
    fixture
        .db()
        .query_row(
            "SELECT invalidation_revision FROM host_instances",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// Kills mutation "derive invalidation compensation from the request budget"
/// (`let budget = request_budget.clone()` in `OrdinaryIdentity::invalidate`):
/// an incoherent read queues its `CoherenceLost` compensation, the caller then
/// cancels, and the known invalidation must still be recorded against the
/// published target.
#[test]
fn identity_invalidation_compensation_completes_after_request_cancellation() {
    let mut fixture = Fixture::new(false);
    let (identity, writer) = quiesced_identity(&mut fixture, Cancellation::default());
    // Stopping the fixture daemon may itself invalidate an in-flight capture.
    let baseline = invalidation_revision(&fixture);
    let (request, result, worker, held) =
        queued_incoherent_compensation(&fixture, &identity, &writer, "cancelled-read");
    request.cancel();
    drop(held);
    let outcome = result
        .recv_timeout(Duration::from_secs(30))
        .expect("resolve did not return after request cancellation");
    worker.join().unwrap();
    assert_eq!(outcome.unwrap_err().code, ErrorCode::StaleHostObservation);
    assert_eq!(
        invalidation_revision(&fixture),
        baseline + 1,
        "request cancellation suppressed the invalidation compensation"
    );
    let allocations: i64 = fixture
        .db()
        .query_row("SELECT count(*) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(allocations, 0);
}

/// Kills mutation "compensation ignores the service shutdown token"
/// (`cancellation: Cancellation::default()` in `compensation_budget`): a
/// compensation queued behind a held writer must end promptly at shutdown
/// instead of holding it for the rest of its 2 s bound.
#[test]
fn identity_invalidation_compensation_does_not_hold_shutdown() {
    let mut fixture = Fixture::new(false);
    let shutdown = Cancellation::default();
    let (identity, writer) = quiesced_identity(&mut fixture, shutdown.clone());
    let baseline = invalidation_revision(&fixture);
    let (request, result, worker, held) =
        queued_incoherent_compensation(&fixture, &identity, &writer, "shutdown-read");
    request.cancel();
    // The incoherent read is queued for its compensation behind `held`.
    let stopped = std::time::Instant::now();
    shutdown.cancel();
    let outcome = result.recv_timeout(Duration::from_millis(600));
    let waited = stopped.elapsed();
    drop(held);
    worker.join().unwrap();
    let outcome = outcome.expect("shutdown waited on detached compensation");
    assert_eq!(outcome.unwrap_err().code, ErrorCode::Cancelled);
    assert!(waited < Duration::from_millis(600), "{waited:?}");
    // Abandoned at shutdown: restart reconciles before host-dependent work.
    assert_eq!(invalidation_revision(&fixture), baseline);
}

/// Kills mutation "unbounded compensation deadline"
/// (`deadline: MonoInstant(u64::MAX)` in `compensation_budget`): without
/// shutdown, a compensation blocked behind the writer still gives up at its
/// own finite bound, long after the request was cancelled.
#[test]
fn identity_invalidation_compensation_is_bounded_without_shutdown() {
    use herdr_threads::identity::repair::INVALIDATION_COMPENSATION_MS;
    let mut fixture = Fixture::new(false);
    let (identity, writer) = quiesced_identity(&mut fixture, Cancellation::default());
    let baseline = invalidation_revision(&fixture);
    let (request, result, worker, held) =
        queued_incoherent_compensation(&fixture, &identity, &writer, "bounded-read");
    let cancelled = std::time::Instant::now();
    request.cancel();
    let outcome = result.recv_timeout(Duration::from_millis(INVALIDATION_COMPENSATION_MS + 2_000));
    let waited = cancelled.elapsed();
    drop(held);
    worker.join().unwrap();
    let outcome = outcome.expect("detached compensation exceeded its bound");
    assert_eq!(outcome.unwrap_err().code, ErrorCode::DeadlineExceeded);
    // It outlived the cancelled request by (about) its own bound.
    assert!(
        waited >= Duration::from_millis(INVALIDATION_COMPENSATION_MS - 250),
        "{waited:?}"
    );
    assert_eq!(invalidation_revision(&fixture), baseline);
}

/// Kills mutation "elected owner does not bind compensation to shutdown"
/// (drop `.with_service_cancellation(cancellation.clone())` in `run_elected`):
/// a real IPC read answers incoherently (no verified structural proof, the
/// evidence-based `CoherenceLost`; a failed read would freeze and write
/// nothing) while an external SQLite writer blocks the daemon's compensation;
/// stopping the owner must end the in-flight handler promptly instead of
/// letting it wait out the 2 s compensation bound.
///
/// Only the request handler is timed. The external SQLite writer also stalls
/// the elected deadline worker, whose own join at teardown is a separate,
/// pre-existing latency outside this compensation (reported, not asserted).
#[test]
fn elected_shutdown_is_not_held_by_invalidation_compensation() {
    let mut fixture = Fixture::new(false);
    fixture
        .host
        .snapshot_writer_probe
        .store(false, Ordering::SeqCst);
    fixture.host.held_past_budget.store(true, Ordering::SeqCst);
    let client = fixture.client();
    let budget = fixture.budget();
    let (answered, answer) = mpsc::channel();
    let request = std::thread::spawn(move || {
        let _ = answered.send(LocalClient::call(
            &client,
            Command::ResolveSeat(ResolveSeat {
                target: HostTargetId::new("pane"),
                operation: OperationId::new("elected-shutdown-read"),
            }),
            &budget,
        ));
    });
    fixture
        .entered
        .recv_timeout(Duration::from_secs(30))
        .expect("daemon did not admit target read");
    let external = fixture.db();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let baseline = invalidation_revision(&fixture);
    fixture.host.incoherent.store(true, Ordering::SeqCst);
    fixture.host.held_past_budget.store(false, Ordering::SeqCst);
    // The incoherent read is now in its compensation, blocked behind SQLite.
    assert!(
        answer.recv_timeout(Duration::from_millis(300)).is_err(),
        "compensation did not wait for the external SQLite writer"
    );
    let stopped = std::time::Instant::now();
    fixture.stop.cancel();
    let outcome = answer.recv_timeout(Duration::from_millis(1_000));
    let waited = stopped.elapsed();
    external.execute_batch("ROLLBACK").unwrap();
    request.join().unwrap();
    assert!(fixture.daemon.take().unwrap().join().unwrap().unwrap());
    let code = outcome
        .expect("owner shutdown waited on invalidation compensation")
        .unwrap_err()
        .code;
    // Shutdown may win the race to the response frame.
    assert!(
        matches!(code, ErrorCode::Cancelled | ErrorCode::UnknownOutcome),
        "{code:?}"
    );
    assert!(waited < Duration::from_millis(1_000), "{waited:?}");
    // Abandoned at shutdown: the compensation never committed.
    assert_eq!(invalidation_revision(&fixture), baseline);
}

/// The launch-side Herdr adapter: an available shell on `pane` that records
/// every guarded start and correlates it (it never prompts).
struct LaunchHost {
    sequence: AtomicU64,
    submitted: std::sync::Mutex<Vec<NativeLaunchRequest>>,
    unknown: bool,
}
impl LaunchHost {
    fn observation(&self) -> HostObservation {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        HostObservation {
            focused: false,
            target: HostTargetId::new("pane"),
            host_boot: HostBootId::new("host"),
            epoch: 1,
            generation: 1,
            observed_at_utc: herdr_threads::protocol::time::UtcMillis(0),
            observed_at_mono: MonoInstant(sequence),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            terminal: Some(TerminalId::new("terminal")),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("launch-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: MonoInstant(sequence),
            completed_at_mono: MonoInstant(sequence),
        }
    }
}
impl HostPort for LaunchHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::HostGuardedStart
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
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
        panic!("launch never prompts")
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<herdr_threads::ports::AgentComposerState, ApiError> {
        Ok(herdr_threads::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.submitted.lock().unwrap().push(request.clone());
        if self.unknown {
            return Ok(NativeLaunchOutcome::OutcomeUnknown);
        }
        let mut diagnostic = self.observation();
        diagnostic.occupancy = StructuralOccupancy::Occupied;
        Ok(NativeLaunchOutcome::ObservedStartup {
            correlation: CorrelatedStartup {
                seat: request.seat.clone(),
                agent_name: request.agent_name(),
                harness: request.harness,
                target: request.target.clone(),
                terminal: request.expected_terminal.clone(),
                expected_generation: request.expected_generation,
                expected_incarnation: request.expected_incarnation.clone(),
                argv: request.argv.clone(),
                host_boot: context.expected_boot.clone().unwrap(),
                epoch: context.expected_epoch.unwrap(),
                submitted_at_mono: diagnostic.started_at_mono,
                completed_at_mono: diagnostic.completed_at_mono,
            },
            diagnostic,
        })
    }
    fn send_submit_key(
        &self,
        _: &herdr_threads::ports::SafeWakeTarget,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<(), herdr_threads::protocol::results::ApiError> {
        Ok(())
    }
}

/// Managed launch through the production seams against a real elected
/// daemon: the seat comes from the daemon's ordinary resolution (the same
/// seat an earlier `seat resolve` returned, so an invitation sent before
/// launch is addressed to it), the owned Claude installation from real
/// setup, and the handoff from a real inbox read. Both an observed startup
/// and a lost start (outcome unknown, the lost-initial-prompt case) leave
/// the prelaunch invitation pending and discoverable, with no binding,
/// acceptance or ACK. Kills: a launch path that registers, accepts, ACKs or
/// allocates a second seat, and a report without the durable handoff.
#[test]
fn managed_launch_uses_daemon_seat_and_keeps_prelaunch_handoff_pending() {
    use herdr_threads::cli::{
        journal::Journal,
        launch::{CodexShellProbe, DaemonSeatResolver, LaunchParts, LaunchRequest, execute},
        setup::{self, SetupEnv, SetupRequest, SetupVerb},
    };
    use herdr_threads::harness::context::Harness as ContextHarness;
    use herdr_threads::protocol::commands::OperatorOrphanInvite;
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new(false);
    let CommandResult::SeatResolved(seat) = fixture.resolve("prelaunch").unwrap() else {
        panic!("missing seat");
    };
    let db = fixture.db();
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('handoff',?1,'topic','goal',0,0)",
        [fixture.descriptor.instance_uuid.to_string()],
    )
    .unwrap();
    let invited = LocalClient::call(
        &fixture.client(),
        Command::OperatorOrphanInvite(OperatorOrphanInvite {
            thread: ThreadId::new("handoff"),
            seat: seat.clone(),
            deadline_millis: Some(600_000),
            operation: OperationId::new("prelaunch-invite"),
        }),
        &fixture.budget(),
    );
    assert!(
        matches!(invited, Ok(CommandResult::OperatorInvited(_))),
        "{invited:?}"
    );

    // Real owned user-level Claude setup into a scratch CLAUDE_CONFIG_DIR
    // (synthetic `claude` on PATH).
    let root = fixture.root.canonicalize().unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    let claude = root.join("bin/claude");
    fs::write(&claude, "#!/bin/sh\nprintf '2.1.285 (Claude Code)\\n'\n").unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    let env = SetupEnv {
        executable: root.join("herdr-threads"),
        state_dir: Some(root.join("state")),
        cwd: root.clone(),
        path: Some(root.join("bin").into_os_string()),
        codex_home: None,
        claude_config_dir: Some(root.join("claude-config")),
        host_endpoint: Some(root.join("host.sock")),
        instance_source: serde_json::Value::Null,
    };
    setup::execute(
        &SetupRequest {
            verb: SetupVerb::Install,
            harness: ContextHarness::Claude,
            harness_binary: None,
        },
        &env,
    )
    .expect("setup claude");

    let client = fixture.client();
    let journal = Journal::open(fixture.paths.instance_dir.join("intents")).unwrap();
    let resolver = DaemonSeatResolver::new(
        &client,
        &journal,
        fixture.descriptor.instance_uuid,
        fixture.clock.as_ref(),
    );
    // Claude launch never asks the pane shell how it resolves `codex`.
    struct NoShell;
    impl CodexShellProbe for NoShell {
        fn resolve_codex(&self) -> Result<String, String> {
            panic!("Claude launch probed the shell for codex")
        }
    }
    for unknown in [false, true] {
        let host = LaunchHost {
            sequence: AtomicU64::new(1),
            submitted: std::sync::Mutex::new(Vec::new()),
            unknown,
        };
        let out = execute(
            &LaunchRequest {
                target: HostTargetId::new("pane"),
                harness: ContextHarness::Claude,
                harness_binary: None,
                argv: vec!["--model".into(), "haiku".into()],
                name: None,
                pane_label: None,
            },
            &LaunchParts {
                env: &env,
                host: &host,
                seats: &resolver,
                handoff: &client,
                clock: fixture.clock.as_ref(),
                record_dir: Some(&fixture.paths.instance_dir),
                shell_probe: &NoShell,
            },
        )
        .unwrap();
        assert_eq!(out.exit, if unknown { 5 } else { 0 });
        let submitted = host.submitted.lock().unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0].seat, seat, "launch resolved another seat");
        assert_eq!(submitted[0].argv, ["--model", "haiku"]);
        assert_eq!(out.report["seat"], seat.as_str());
        assert_eq!(
            out.report["handoff"]["pending_invitations"], 1,
            "{}",
            out.report
        );
        assert_eq!(
            out.report["handoff"]["threads"],
            serde_json::json!(["handoff"])
        );
    }
    let (membership, bindings, acks, seats): (String, i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT state FROM memberships WHERE thread_id='handoff' AND seat_id=?1),
                    (SELECT count(*) FROM occupant_bindings),
                    (SELECT count(*) FROM receipts WHERE acked_at IS NOT NULL),
                    (SELECT count(*) FROM seats)",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (membership.as_str(), bindings, acks, seats),
        ("invited", 0, 0, 1)
    );
    let records = fs::read_to_string(fixture.paths.instance_dir.join("launches.jsonl")).unwrap();
    assert_eq!(records.lines().count(), 2);
}
