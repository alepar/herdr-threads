//! ht-5fr.2.12: dispatch by claim host. One elected daemon (this process, the
//! production `run_elected_probed` composition) over two fake Herdr servers
//! from a `FakeHostWorld`: every command that names a server attaches it
//! lazily on the request path, each server's seats are woken by that server's
//! wake worker alone, an outage of one server freezes only its own seats, and
//! a resumed session reattaches its seat onto another server (C1). The hook
//! case runs the real `hook claude` as a child (`test_support::spawn`) in a
//! pane of the second server while the installed argv bakes the first.
//!
//! The servers are fixtures: the same pane id (`pane-1`) exists on both, as it
//! does on two real Herdr servers, and each records the wake prompts it is
//! asked to submit.

use herdr_threads::{
    app::{LaneProbe, SystemClock, run_elected_guarded_probed, run_elected_probed},
    client::local::LocalSocketClient,
    daemon::{ownership::EndpointDescriptor, paths::NodePaths},
    ports::{
        AgentComposerState, ComposerObservation, EnumerationEvidence, EvidenceKind,
        ExecutionEvidence, HostCallContext, HostObservation, HostPort, HostSnapshot, HostUiState,
        IncarnationEvidence, LocalClient, NativeLaunchCapability, NativeLaunchOutcome,
        NativeLaunchRequest, ObservationProvenance, PromptOutcome, SafeWakeTarget,
        StructuralOccupancy, WakeTargetBasis,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint},
        commands::{
            Accept, AdvanceCursors, CheckIn, CheckInMode, Command, ContinuityCheckIn, CreateThread,
            Invite, ResolveSeat, SendMessage,
        },
        ids::{
            ExecutionId, HostBootId, HostCallId, HostLocator, HostTargetId, NativeSessionId,
            OperationId, SeatId, TerminalId, ThreadId,
        },
        results::{
            ApiError, CommandResult, ComponentState, ErrorCode, Health, HealthState,
            bound_endpoint_tail,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{config::ServiceConfig, kicks::Lane},
    test_support::{
        FakeHostWorld,
        isolation::TestIsolation,
        spawn::{OwnedChild, SpawnOwned},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// The node's state directory in its canonical spelling, the one the
/// daemon's descriptor and the hook's `--state-dir` resolve to.
fn state_dir(isolation: &TestIsolation) -> PathBuf {
    std::fs::canonicalize(isolation.state_root()).expect("the state root exists")
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A fake Herdr server: panes that exist, a boot (a restart is a new boot and
/// a new epoch), an up/down switch for the whole server, and an arming switch
/// for wakes (an unarmed server has no safe wake target, so the daemon refuses
/// and retries). Every submitted wake prompt is recorded with its pane.
struct ServerHost {
    /// The daemon's clock: observations are stamped with it, as `NativeCli` stamps them.
    clock: Arc<dyn Clock>,
    up: AtomicBool,
    armed: AtomicBool,
    boot: Mutex<(String, u64)>,
    panes: Mutex<BTreeSet<String>>,
    /// The agent process in the foreground of each pane.
    foreground: Mutex<BTreeMap<String, u32>>,
    sequence: AtomicU64,
    prompts: Mutex<Vec<(String, String)>>,
    /// Whether it answers `HostGuardedStart` (a launch-capable Herdr TUI).
    guarded: bool,
}

impl ServerHost {
    fn new(clock: &Arc<dyn Clock>, boot: &str, panes: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            clock: Arc::clone(clock),
            up: AtomicBool::new(true),
            armed: AtomicBool::new(true),
            boot: Mutex::new((boot.to_owned(), 1)),
            panes: Mutex::new(panes.iter().map(|pane| (*pane).to_owned()).collect()),
            foreground: Mutex::new(BTreeMap::new()),
            sequence: AtomicU64::new(0),
            prompts: Mutex::new(Vec::new()),
            guarded: false,
        })
    }

    /// A server that also answers `HostGuardedStart`.
    fn guarded(clock: &Arc<dyn Clock>, boot: &str, panes: &[&str]) -> Arc<Self> {
        let mut server = Arc::into_inner(Self::new(clock, boot, panes)).expect("a fresh server");
        server.guarded = true;
        Arc::new(server)
    }

    fn set_up(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }

    fn set_armed(&self, armed: bool) {
        self.armed.store(armed, Ordering::SeqCst);
    }

    /// `pid` is the agent in the foreground of `pane`.
    fn set_foreground(&self, pane: &str, pid: u32) {
        self.foreground.lock().unwrap().insert(pane.to_owned(), pid);
    }

    /// The server restarted: a new boot, a new epoch, new incarnation evidence.
    fn restart(&self, boot: &str) {
        let mut state = self.boot.lock().unwrap();
        *state = (boot.to_owned(), state.1 + 1);
    }

    /// The panes that were submitted a wake prompt, in order.
    fn woken(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .map(|(pane, _)| pane.clone())
            .collect()
    }

    fn down() -> ApiError {
        ApiError::new(ErrorCode::HostUnavailable, "fixture server is down")
    }

    fn observation(
        &self,
        target: &HostTargetId,
        provenance: ObservationProvenance,
        evidence_kind: EvidenceKind,
    ) -> Result<HostObservation, ApiError> {
        if !self.up.load(Ordering::SeqCst) {
            return Err(Self::down());
        }
        if !self.panes.lock().unwrap().contains(target.as_str()) {
            return Err(ApiError::new(ErrorCode::NotFound, "no such pane"));
        }
        let (boot, epoch) = self.boot.lock().unwrap().clone();
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let at = self.clock.monotonic_now();
        Ok(HostObservation {
            focused: false,
            target: target.clone(),
            host_boot: HostBootId::new(boot.clone()),
            epoch,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: at,
            provenance,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new(format!("term-{boot}-{}", target.as_str()))),
            occupancy: StructuralOccupancy::Occupied,
            incarnation: IncarnationEvidence::Verified {
                identity: format!("inc-{boot}"),
                evidence_kind,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: at,
            completed_at_mono: at,
        })
    }
}

impl HostPort for ServerHost {
    fn live_topology(
        &self,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<
        herdr_threads::host::observation::HostTopology,
        herdr_threads::protocol::results::ApiError,
    > {
        Err(herdr_threads::protocol::results::ApiError::unsupported(
            "fixture host has no topology",
        ))
    }
    /// The pane's foreground process is the checked-in agent: the composer
    /// wake types only to the binding with that pid.
    fn foreground_pids(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<Vec<u32>, ApiError> {
        if !self.up.load(Ordering::SeqCst) {
            return Err(Self::down());
        }
        Ok(self
            .foreground
            .lock()
            .unwrap()
            .get(target.as_str())
            .map(|pid| vec![*pid])
            .unwrap_or_default())
    }
    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        Err(ApiError::unsupported("fixture has no archival observation"))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        if self.guarded {
            NativeLaunchCapability::HostGuardedStart
        } else {
            NativeLaunchCapability::Unsupported
        }
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.observation(
            target,
            ObservationProvenance::FreshCurrentTarget,
            EvidenceKind::NativeCurrentTarget,
        )
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let panes: Vec<String> = self.panes.lock().unwrap().iter().cloned().collect();
        let mut targets = Vec::new();
        for pane in panes {
            targets.push(self.observation(
                &HostTargetId::new(pane),
                ObservationProvenance::CoherentEnumeration,
                EvidenceKind::CoherentEnumeration,
            )?);
        }
        if !self.up.load(Ordering::SeqCst) {
            return Err(Self::down());
        }
        let (boot, epoch) = self.boot.lock().unwrap().clone();
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(HostSnapshot {
            boot: HostBootId::new(boot.clone()),
            epoch,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: format!("inc-{boot}"),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets,
        })
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if !self.armed.load(Ordering::SeqCst) {
            return None;
        }
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.prompts
            .lock()
            .unwrap()
            .push((target.target.as_str().to_owned(), text.to_owned()));
        Ok(PromptOutcome::Submitted)
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        Ok(AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        Err(ApiError::unsupported("fixture launches nothing"))
    }
}

/// One in-process elected daemon over `world`, stopped (and joined, which
/// joins every session thread) on drop.
struct Daemon {
    probe: LaneProbe,
    descriptor: EndpointDescriptor,
    stop: Cancellation,
    thread: Option<JoinHandle<std::io::Result<bool>>>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Daemon {
    /// `guarded`: the production guarded composition, whose sessions carry a bootstrap runtime.
    fn start_composed(
        isolation: &TestIsolation,
        clock: &Arc<dyn Clock>,
        world: &Arc<FakeHostWorld>,
        hint: Option<HostLocator>,
        config: ServiceConfig,
        guarded: bool,
    ) -> Self {
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        let paths: NodePaths = herdr_threads::test_support::resolve_seeded(&state_dir(isolation));
        let clock = Arc::clone(clock);
        let probe = LaneProbe::default();
        probe.set_host_sweep_period(Duration::from_secs(600));
        probe.log_lane_errors_to(isolation.path("lanes.log"));
        let stop = Cancellation::default();
        let plan = world.plan(hint);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (daemon_stop, daemon_probe) = (stop.clone(), probe.clone());
        let guarded_dir = guarded.then(|| state_dir(isolation));
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let ready = move |descriptor: &EndpointDescriptor| {
                ready_tx
                    .send(descriptor.clone())
                    .map_err(std::io::Error::other)
            };
            match guarded_dir {
                Some(dir) => runtime.block_on(run_elected_guarded_probed(
                    &paths,
                    dir,
                    clock,
                    daemon_stop,
                    config,
                    plan,
                    daemon_probe,
                    ready,
                )),
                None => runtime.block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    config,
                    plan,
                    daemon_probe,
                    ready,
                )),
            }
        });
        let descriptor = ready_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the elected daemon did not publish its endpoint");
        // The elected daemon installs a quiet panic hook; restore visible failures.
        std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        Self {
            probe,
            descriptor,
            stop,
            thread: Some(thread),
            _guard: guard,
        }
    }

    /// Stops the daemon and waits for its drain (which joins every session
    /// thread); the error text when the daemon did not drain cleanly.
    fn shut_down(&mut self) -> Option<String> {
        self.stop.cancel();
        let thread = self.thread.take()?;
        match thread.join() {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(format!("daemon did not drain cleanly: {error}")),
            Err(_) => Some("daemon thread panicked".to_owned()),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let failure = self.shut_down();
        if !std::thread::panicking() {
            assert_eq!(failure, None);
        }
    }
}

/// A checked-in agent: its seat and the claim the daemon returned for it.
#[derive(Clone)]
struct Agent {
    seat: SeatId,
    claim: CallerClaim,
}

struct Rig {
    // Fields drop in order: the daemon stops before the fixtures under it go.
    daemon: Daemon,
    /// The agent processes the bindings' liveness fingerprints name.
    processes: Mutex<Vec<OwnedChild>>,
    a: Arc<ServerHost>,
    b: Arc<ServerHost>,
    loc_a: HostLocator,
    loc_b: HostLocator,
    clock: Arc<dyn Clock>,
    client: LocalSocketClient,
    instance: String,
    operations: AtomicU64,
    isolation: TestIsolation,
}

impl Drop for Rig {
    /// A failing test's messages went to the daemon's log (the elected daemon
    /// redirects stderr): stop it, then show the log.
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.daemon.shut_down();
            let paths = herdr_threads::test_support::resolve_seeded(&state_dir(&self.isolation));
            let log = herdr_threads::daemon::logs::daemon_log_path(&paths);
            eprintln!(
                "--- {} ---\n{}",
                log.display(),
                std::fs::read_to_string(&log).unwrap_or_default()
            );
        }
    }
}

/// What a guarded rig's startup is asked to attach.
#[derive(Clone, Copy)]
enum StartupHint {
    /// Server A.
    A,
    /// The built-in named host's reserved locator.
    Reserved,
}

impl Rig {
    /// A daemon over servers A (panes `pane-1`) and B (`pane-1`, `pane-2`),
    /// with `hint_a` naming the server attached at startup, if any.
    fn start(label: &str, hint_a: bool) -> Self {
        Self::build(label, hint_a.then_some(StartupHint::A), false)
    }

    /// The production guarded composition over a server A that is no Herdr TUI (it answers no
    /// guarded start, as a core does) and a Herdr TUI B, with `hint` naming what startup
    /// attaches, if anything.
    fn start_guarded(label: &str, hint: Option<StartupHint>) -> Self {
        Self::build(label, hint, true)
    }

    fn build(label: &str, hint: Option<StartupHint>, guarded: bool) -> Self {
        let isolation = TestIsolation::new(label);
        // Canonical spelling: the hook canonicalizes the pane's server path.
        let locator = |name: &str| {
            let socket = isolation.socket_path(name);
            let directory = std::fs::canonicalize(socket.parent().unwrap()).unwrap();
            HostLocator::parse(directory.join(name).to_string_lossy().into_owned()).unwrap()
        };
        let (loc_a, loc_b) = (locator("herdr-a.sock"), locator("herdr-b.sock"));
        // The daemon's clock, which the servers stamp their observations with.
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let a = ServerHost::new(&clock, "boot-a", &["pane-1"]);
        let b = if guarded {
            ServerHost::guarded(&clock, "boot-b", &["pane-1", "pane-2"])
        } else {
            ServerHost::new(&clock, "boot-b", &["pane-1", "pane-2"])
        };
        let world = FakeHostWorld::new();
        world.add(&loc_a, Arc::clone(&a) as Arc<dyn HostPort>);
        world.add(&loc_b, Arc::clone(&b) as Arc<dyn HostPort>);
        let hint = hint.map(|hint| match hint {
            StartupHint::A => loc_a.clone(),
            StartupHint::Reserved => {
                HostLocator::parse(herdr_threads::protocol::ids::NAMED_HOST_LOCATOR).unwrap()
            }
        });
        // Wakes go out at once, not after the ordinary batching window.
        let config = ServiceConfig::default().with_wake_batch_delay(0).unwrap();
        let daemon = Daemon::start_composed(&isolation, &clock, &world, hint, config, guarded);
        let client = LocalSocketClient::new(
            daemon.descriptor.endpoint.clone(),
            Arc::clone(&clock),
            daemon.descriptor.node_uuid,
            Some(daemon.descriptor.boot_id),
        );
        let instance = daemon.descriptor.node_uuid.to_string();
        Self {
            isolation,
            processes: Mutex::new(Vec::new()),
            a,
            b,
            loc_a,
            loc_b,
            clock,
            client,
            instance,
            operations: AtomicU64::new(0),
            daemon,
        }
    }

    fn sessions(&self) -> Arc<herdr_threads::service::host_sessions::HostSessions> {
        self.daemon.probe.host_sessions().expect("host sessions")
    }

    fn attached(&self, locator: &HostLocator) -> bool {
        self.sessions().by_locator(locator).is_some()
    }

    fn call(&self, command: Command) -> Result<CommandResult, ApiError> {
        let budget = CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 20_000),
            cancellation: Cancellation::default(),
        };
        self.client.call(command, &budget)
    }

    /// A fresh operation key, so no two calls share a journal entry.
    fn op(&self, what: &str) -> OperationId {
        OperationId::new(format!(
            "{what}-{}",
            self.operations.fetch_add(1, Ordering::SeqCst)
        ))
    }

    fn db(&self) -> rusqlite::Connection {
        let paths = herdr_threads::test_support::resolve_seeded(&state_dir(&self.isolation));
        let db = rusqlite::Connection::open(paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    /// The server at `locator`.
    fn server(&self, locator: &HostLocator) -> &Arc<ServerHost> {
        if *locator == self.loc_a {
            &self.a
        } else {
            &self.b
        }
    }

    /// A live agent process in the foreground of `pane`, and its fingerprint:
    /// a binding's process is what keeps it connected, and two bindings never
    /// share one.
    fn agent_process(&self, host: &HostLocator, pane: &str) -> LivenessFingerprint {
        use herdr_threads::ports::ProcessInfo;
        let child = herdr_threads::test_support::spawn::command("sleep")
            .arg("3600")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn_owned()
            .expect("spawn the agent process");
        let pid = child.id();
        self.processes.lock().unwrap().push(child);
        self.server(host).set_foreground(pane, pid);
        let record = herdr_threads::host::process_table::HostProcessTable
            .process(pid)
            .expect("the agent process is in the process table");
        LivenessFingerprint::of(&record)
    }

    fn claim(
        &self,
        host: &HostLocator,
        seat: &SeatId,
        pane: &str,
        session: &str,
        liveness: LivenessFingerprint,
    ) -> CallerClaim {
        CallerClaim {
            instance: self.instance.clone(),
            host: host.clone(),
            seat: seat.clone(),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(session),
            execution: ExecutionId::new(format!(
                "00000000-0000-4000-8000-{:012}",
                self.operations.fetch_add(1, Ordering::SeqCst)
            )),
            target: HostTargetId::new(pane),
            agent_id: None,
            liveness: Some(liveness),
        }
    }

    /// A claim of a seat nobody has, naming the server at `host`: the store
    /// refuses it, so what it shows is the attach that precedes the store.
    fn touch(&self, host: &HostLocator) {
        let refused = self
            .call(Command::AdvanceCursors(AdvanceCursors {
                claim: self.claim(
                    host,
                    &SeatId::new("no-such-seat"),
                    "pane-1",
                    "nobody",
                    LivenessFingerprint::TEST,
                ),
                operation: self.op("touch"),
                displayed: Vec::new(),
                in_turn: false,
                invitations: Vec::new(),
            }))
            .unwrap_err();
        assert_ne!(refused.code, ErrorCode::HostUnavailable, "{refused:?}");
    }

    /// Resolve `pane` of the server at `host` to a seat and check an agent in on it.
    fn enroll(&self, host: &HostLocator, pane: &str, session: &str) -> Agent {
        let resolved = self
            .call(Command::ResolveSeat(ResolveSeat {
                target: HostTargetId::new(pane),
                host: host.clone(),
                operation: self.op("resolve"),
                locator_hint: None,
                tmux_hint: None,
            }))
            .unwrap_or_else(|error| panic!("resolve {pane} on {}: {error:?}", host.as_str()));
        let CommandResult::SeatResolved(seat) = resolved else {
            panic!("unexpected resolve result {resolved:?}")
        };
        // A lifecycle check-in names the generation the seat is at.
        let generation: i64 = self
            .db()
            .query_row(
                "SELECT generation FROM seats WHERE id=?1",
                [seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        let generation = u64::try_from(generation).unwrap();
        let liveness = self.agent_process(host, pane);
        let mut claim = self.claim(host, &seat, pane, session, liveness);
        claim.binding_generation = generation;
        let checked = self
            .call(Command::CheckIn(CheckIn {
                readonly: false,
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: generation,
                },
                claim,
                operation: self.op("check-in"),
                lineage_parent: None,
            }))
            .unwrap_or_else(|error| panic!("check in {session}: {error:?}"));
        let CommandResult::CheckedIn(checked) = checked else {
            panic!("unexpected check-in result {checked:?}")
        };
        Agent {
            seat,
            claim: checked.context,
        }
    }

    /// `owner` opens a thread and invites `member`, who accepts.
    fn thread_with(&self, owner: &Agent, member: &Agent) -> ThreadId {
        let created = self
            .call(Command::CreateThread(CreateThread {
                name: None,
                topic: "multi-host".into(),
                goal: "dispatch by claim host".into(),
                operation: self.op("thread"),
                claim: owner.claim.clone(),
            }))
            .unwrap();
        let CommandResult::ThreadCreated(thread) = created else {
            panic!("unexpected thread result {created:?}")
        };
        self.call(Command::Invite(Invite {
            confirmation: None,
            thread: thread.clone(),
            seat: member.seat.clone(),
            deadline_millis: None,
            operation: self.op("invite"),
            claim: owner.claim.clone(),
        }))
        .unwrap();
        self.call(Command::Accept(Accept {
            thread: thread.clone(),
            operation: self.op("accept"),
            claim: member.claim.clone(),
        }))
        .unwrap();
        thread
    }

    /// An ordinary (nudge-class) message from `from` to the thread's members.
    fn send(&self, from: &Agent, thread: &ThreadId, body: &str) {
        self.call(Command::SendMessage(SendMessage {
            delivery_mode: Default::default(),
            priority: None,
            user_intent: None,
            thread: thread.clone(),
            body: body.into(),
            direct_recipient: None,
            invited_recipients: Vec::new(),
            deadline_millis: None,
            operation: self.op("send"),
            claim: from.claim.clone(),
            relays_user: false,
        }))
        .unwrap();
    }

    /// `wait_until` that reports the wake rows and prompts when it times out.
    fn wait(&self, what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(30);
        while !done() {
            if Instant::now() >= until {
                panic!("timed out waiting for {what}\n{}", self.wake_state());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The wake rows, the seats and the prompts each server recorded.
    fn wake_state(&self) -> String {
        let db = self.db();
        let mut out = String::new();
        let mut statement = db
            .prepare(
                "SELECT w.seat_id, s.host_id, s.state, w.reason_bits, w.attention_version, \
                 w.checkpoint_version, w.retry_step, w.reservation_id, w.last_outcome \
                 FROM wake_work w JOIN seats s ON s.id=w.seat_id",
            )
            .unwrap();
        let rows = statement
            .query_map([], |r| {
                Ok(format!(
                    "wake_work seat={} host={} state={} reason={} attention={} checkpoint={} retry={} reservation={:?} outcome={:?}",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            })
            .unwrap();
        for row in rows {
            out.push_str(&row.unwrap());
            out.push('\n');
        }
        let mut bindings = db
            .prepare(
                "SELECT seat_id, target_id, liveness, disconnect_reason, ended_at, liveness_pid \
                 FROM occupant_bindings",
            )
            .unwrap();
        for row in bindings
            .query_map([], |r| {
                Ok(format!(
                    "binding seat={} target={} liveness={} reason={:?} ended={:?} pid={:?}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .unwrap()
        {
            out.push_str(&row.unwrap());
            out.push('\n');
        }
        out.push_str(&format!(
            "a prompts: {:?}\nb prompts: {:?}\n",
            self.a.woken(),
            self.b.woken()
        ));
        for (name, locator) in [("a", &self.loc_a), ("b", &self.loc_b)] {
            out.push_str(&format!(
                "{name} down={} attached={}\n",
                self.is_down(locator),
                self.attached(locator)
            ));
        }
        out.push_str(
            &std::fs::read_to_string(self.isolation.path("lanes.log")).unwrap_or_default(),
        );
        out
    }

    /// Forces the next observation pass of `locator`'s server until `done`.
    fn observe_until(&self, locator: &HostLocator, what: &str, mut done: impl FnMut() -> bool) {
        wait_until(what, || {
            if let Some(pacer) = self.daemon.probe.host_pacer(locator, Lane::Observation) {
                pacer.kick();
            }
            done()
        });
    }

    fn is_down(&self, locator: &HostLocator) -> bool {
        self.sessions()
            .by_locator(locator)
            .is_some_and(|session| session.reachability().is_down())
    }

    /// `(reservation, retry step, last outcome)` of a seat's wake row.
    fn wake_row(&self, seat: &SeatId) -> Option<(Option<String>, i64, Option<String>)> {
        self.db()
            .query_row(
                "SELECT reservation_id, retry_step, last_outcome FROM wake_work WHERE seat_id=?1",
                [seat.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }

    fn health(&self) -> Health {
        match self.call(Command::Health).expect("health") {
            CommandResult::Health(health) => health,
            other => panic!("not a health result: {other:?}"),
        }
    }

    fn host_id_of(&self, locator: &HostLocator) -> i64 {
        self.db()
            .query_row(
                "SELECT host_id FROM herdr_hosts WHERE endpoint=?1",
                [locator.as_str()],
                |r| r.get(0),
            )
            .expect("the server has a host row")
    }
}

// Kills: a daemon that dials the named host (it would appear in `attached()`
// and Health) or never installs it in the built-in slot.
#[test]
fn named_host_is_built_in_and_never_attached() {
    let rig = Rig::start("mh-named-builtin", false);
    assert_eq!(rig.sessions().attached_count(), 0);
    assert!(rig.sessions().attached().is_empty());
    let all = rig.sessions().all_for_reads();
    assert_eq!(all.len(), 1, "only the built-in session");
    assert!(all[0].locator().is_named_host());
    assert!(rig.health().hosts.is_empty(), "Health lists dialled hosts");
    let kind: String = rig
        .db()
        .query_row(
            "SELECT kind FROM herdr_hosts WHERE endpoint=?1",
            [herdr_threads::protocol::ids::NAMED_HOST_LOCATOR],
            |r| r.get(0),
        )
        .expect("the built-in host has a row");
    assert_eq!(kind, "named");
}

// Kills: a built-in session left out of `health_entries`, so a failed tmux wake
// lane of the named host never reaches Health (`wake[<id>]` stayed invisible),
// and a built-in input that joins `Health.hosts` (it must not skew the aggregate).
#[test]
fn builtin_wake_lane_failure_reaches_health() {
    let rig = Rig::start("mh-builtin-wake-lane", false);
    let degraded_line = |health: &Health| {
        health
            .limitations
            .iter()
            .any(|line| line.starts_with("scheduler degraded: "))
    };
    let clean = rig.health();
    assert!(!degraded_line(&clean), "{clean:?}");
    let builtin_id: i64 = rig
        .db()
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE endpoint=?1",
            [herdr_threads::protocol::ids::NAMED_HOST_LOCATOR],
            |r| r.get(0),
        )
        .expect("the built-in host has a row");
    let probe = &rig.daemon.probe;
    probe.fail_lane(Lane::Wakes, ErrorCode::StoreBusy);
    rig.wait("the built-in wake lane to reach Health", || {
        probe.kick_registered(Lane::Wakes);
        let health = rig.health();
        health.state == HealthState::Degraded && degraded_line(&health)
    });
    let lanes: Vec<String> = rig
        .sessions()
        .health_entries()
        .into_iter()
        .flat_map(|entry| entry.lanes)
        .map(|lane| lane.lane)
        .collect();
    assert_eq!(lanes, vec![format!("wake[{builtin_id}]")]);
    let health = rig.health();
    assert!(health.hosts.is_empty(), "lanes only, no input: {health:?}");
    probe.heal_lane(Lane::Wakes);
    rig.wait("the built-in lane to recover", || {
        probe.kick_registered(Lane::Wakes);
        !degraded_line(&rig.health())
    });
}

// Kills: a built-in entry that is always present, which would turn a
// store-only daemon's capability fields into the named host's.
#[test]
fn store_only_daemon_stays_store_only_with_a_healthy_builtin() {
    let rig = Rig::start("mh-store-only-builtin", false);
    assert_eq!(rig.sessions().all_for_reads().len(), 1, "built-in attached");
    rig.wait("lanes to settle", || {
        std::thread::sleep(Duration::from_millis(300));
        true
    });
    let health = rig.health();
    assert!(health.hosts.is_empty(), "{health:?}");
    assert_eq!(rig.sessions().health_entries().len(), 0);
}

// Kills: a daemon that attaches only its startup hint (or nothing without
// one): with no hint and no remembered row nothing is attached, and the
// first claim from fake host A attaches A, and only A, with its two workers.
#[test]
fn claim_from_a_attaches_a_lazily() {
    let rig = Rig::start("mh-lazy-a", false);
    assert_eq!(rig.sessions().attached_count(), 0, "no hint, no row");
    assert!(
        rig.daemon
            .probe
            .host_pacer(&rig.loc_a, Lane::Wakes)
            .is_none()
    );
    // A claim that the store refuses still attaches the server it names.
    rig.touch(&rig.loc_a);
    assert!(rig.attached(&rig.loc_a), "A attached by its first claim");
    let agent = rig.enroll(&rig.loc_a, "pane-1", "session-a");
    assert!(!rig.attached(&rig.loc_b), "B was not named");
    assert_eq!(rig.sessions().attached_count(), 1);
    for lane in [Lane::Wakes, Lane::Observation] {
        assert!(
            rig.daemon.probe.host_pacer(&rig.loc_a, lane).is_some(),
            "A's {lane:?} worker runs"
        );
        assert!(rig.daemon.probe.host_pacer(&rig.loc_b, lane).is_none());
    }
    let host: i64 = rig
        .db()
        .query_row(
            "SELECT host_id FROM seats WHERE id=?1",
            [agent.seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host, rig.host_id_of(&rig.loc_a), "the seat sits on A");
}

// Kills: an archival lane gated on a startup attach (a store-only daemon never
// runs it) or started again per attach: it starts with the first lazy attach,
// once.
#[test]
fn archival_lane_starts_with_the_first_lazy_attach_once() {
    let rig = Rig::start("mh-archival-late", false);
    assert!(!rig.daemon.probe.registered(Lane::Archival));
    assert!(!rig.daemon.probe.status_has_pacer(Lane::Archival));
    rig.touch(&rig.loc_a);
    rig.wait("the archival lane to start", || {
        rig.daemon.probe.status_has_pacer(Lane::Archival)
    });
    assert_eq!(rig.daemon.probe.registered_count(Lane::Archival), 1);
    rig.touch(&rig.loc_b);
    assert!(rig.attached(&rig.loc_b));
    assert_eq!(
        rig.daemon.probe.registered_count(Lane::Archival),
        1,
        "a second host does not start a second lane"
    );
}

// A daemon with a host attached at startup starts the lane at startup, as
// before, and a later host does not start another. (A second rig in one test
// would wait on the first daemon's process-wide lock, hence two tests.)
#[test]
fn archival_lane_starts_at_startup_when_a_host_is_attached() {
    let rig = Rig::start("mh-archival-startup", true);
    assert_eq!(rig.daemon.probe.registered_count(Lane::Archival), 1);
    rig.touch(&rig.loc_b);
    assert!(rig.attached(&rig.loc_b));
    assert_eq!(rig.daemon.probe.registered_count(Lane::Archival), 1);
}

// Kills: a service that routes every claim to one server: the claim from B
// attaches B, each seat is woken by its own server's worker, and the same pane
// id on both servers stays two seats.
#[test]
fn claim_from_b_attaches_b_and_both_seats_nudged() {
    let rig = Rig::start("mh-both-nudged", true);
    assert!(rig.attached(&rig.loc_a), "the hint attached A at startup");
    assert!(!rig.attached(&rig.loc_b));
    // Wakes are refused until armed, so every prompt recorded afterwards
    // answers the messages sent below, not an earlier invitation.
    rig.a.set_armed(false);
    rig.b.set_armed(false);
    let on_a = rig.enroll(&rig.loc_a, "pane-1", "session-a");
    assert!(!rig.attached(&rig.loc_b));
    rig.touch(&rig.loc_b);
    assert!(rig.attached(&rig.loc_b), "B attached by its first claim");
    let on_b = rig.enroll(&rig.loc_b, "pane-1", "session-b");
    assert_ne!(on_a.seat, on_b.seat, "one pane id, two servers, two seats");
    let thread = rig.thread_with(&on_a, &on_b);
    rig.send(&on_a, &thread, "to b");
    rig.send(&on_b, &thread, "to a");
    assert!(rig.a.woken().is_empty() && rig.b.woken().is_empty());
    rig.a.set_armed(true);
    rig.b.set_armed(true);
    rig.wait("both servers to be nudged", || {
        !rig.a.woken().is_empty() && !rig.b.woken().is_empty()
    });
    // Each server was only ever asked about its own pane.
    assert!(rig.a.woken().iter().all(|pane| pane == "pane-1"));
    assert!(rig.b.woken().iter().all(|pane| pane == "pane-1"));
}

// Kills: guarded capabilities frozen at startup from whichever session led it: a daemon that
// started on the reserved named locator, on a core-shaped server, or on no server at all must
// advertise guarded bootstrap and automatic handoff once a Herdr host attaches, and Health must
// then list that host.
fn guarded_capabilities_follow_a_host_attached_later(label: &str, hint: Option<StartupHint>) {
    use herdr_threads::protocol::capabilities::{
        AUTOMATIC_HANDOFF_V1, BOOTSTRAP_GUARDED_RESOLUTION_V1,
    };
    let rig = Rig::start_guarded(label, hint);
    let listed = |rig: &Rig| match rig.call(Command::Capabilities).expect("capabilities") {
        CommandResult::Capabilities(list) => (
            list.capabilities
                .iter()
                .any(|name| name == BOOTSTRAP_GUARDED_RESOLUTION_V1),
            list.capabilities
                .iter()
                .any(|name| name == AUTOMATIC_HANDOFF_V1),
        ),
        other => panic!("not a capability list: {other:?}"),
    };
    assert_eq!(
        listed(&rig),
        (false, false),
        "no Herdr host is attached yet"
    );
    rig.touch(&rig.loc_b);
    assert!(rig.attached(&rig.loc_b));
    assert_eq!(
        listed(&rig),
        (true, true),
        "the host attached later is served"
    );
    let health = rig.health();
    assert!(
        health
            .hosts
            .iter()
            .any(|host| host.host_id == rig.host_id_of(&rig.loc_b)),
        "{health:?}"
    );
}

#[test]
fn reserved_hint_startup_serves_a_herdr_host_attached_later() {
    guarded_capabilities_follow_a_host_attached_later(
        "mh-guarded-reserved",
        Some(StartupHint::Reserved),
    );
}

#[test]
fn core_hint_startup_serves_a_herdr_host_attached_later() {
    guarded_capabilities_follow_a_host_attached_later("mh-guarded-core", Some(StartupHint::A));
}

#[test]
fn store_only_startup_serves_a_herdr_host_attached_later() {
    guarded_capabilities_follow_a_host_attached_later("mh-guarded-store-only", None);
}

// Kills: Health that reports a count or the startup server instead of one
// entry per attached server: none before any claim, one after the lazy attach
// of A, two (in host id order, each with its own endpoint) after B's.
#[test]
fn health_lists_hosts_as_they_attach() {
    let rig = Rig::start("mh-health-list", false);
    assert!(rig.health().hosts.is_empty(), "store-only until a claim");
    rig.touch(&rig.loc_a);
    let one = rig.health();
    assert_eq!(one.hosts.len(), 1, "{one:?}");
    assert_eq!(
        one.hosts[0].endpoint,
        bound_endpoint_tail(rig.loc_a.as_str())
    );
    assert_eq!(one.hosts[0].host_id, rig.host_id_of(&rig.loc_a));
    rig.touch(&rig.loc_b);
    let two = rig.health();
    assert_eq!(two.hosts.len(), 2, "{two:?}");
    let ids: Vec<i64> = two.hosts.iter().map(|host| host.host_id).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
    let endpoints: BTreeSet<&str> = two.hosts.iter().map(|h| h.endpoint.as_str()).collect();
    assert_eq!(
        endpoints,
        BTreeSet::from([
            bound_endpoint_tail(rig.loc_a.as_str()).as_str(),
            bound_endpoint_tail(rig.loc_b.as_str()).as_str()
        ])
    );
    two.validate().expect("a valid wire shape");
}

// Kills: an outage of one server that leaves Health `healthy` or unnamed, a
// line for the server that is up, and a recovery that stays reported down.
#[test]
fn health_degraded_while_host_down() {
    let Settled { rig, .. } = settled("mh-health-down");
    let line_a = format!(
        "Herdr server {} unreachable",
        bound_endpoint_tail(rig.loc_a.as_str())
    );
    let line_b = format!(
        "Herdr server {} unreachable",
        bound_endpoint_tail(rig.loc_b.as_str())
    );
    rig.a.set_up(false);
    rig.observe_until(&rig.loc_a, "A to be marked down", || {
        rig.is_down(&rig.loc_a)
    });
    let health = rig.health();
    assert_eq!(health.state, HealthState::Degraded, "{health:?}");
    assert!(health.limitations.contains(&line_a), "{health:?}");
    assert!(!health.limitations.contains(&line_b), "{health:?}");
    assert_eq!(health.host.reachability, ComponentState::Unavailable);
    let by_endpoint = |h: &Health, loc: &HostLocator| {
        h.hosts
            .iter()
            .find(|host| host.endpoint == bound_endpoint_tail(loc.as_str()))
            .map(|host| host.reachability)
    };
    assert_eq!(
        by_endpoint(&health, &rig.loc_a),
        Some(ComponentState::Unavailable)
    );
    assert_ne!(
        by_endpoint(&health, &rig.loc_b),
        Some(ComponentState::Unavailable)
    );
    rig.a.set_up(true);
    rig.observe_until(&rig.loc_a, "A to be marked up", || !rig.is_down(&rig.loc_a));
    rig.wait("A to stop being reported unreachable", || {
        !rig.health().limitations.contains(&line_a)
    });
}

/// Both seats checked in on their servers, with a thread between them. Nobody
/// has been woken yet: what a test sends next is the first attention of either.
struct Settled {
    rig: Rig,
    on_a: Agent,
    on_b: Agent,
    thread: ThreadId,
}

fn settled(label: &str) -> Settled {
    let rig = Rig::start(label, true);
    let on_a = rig.enroll(&rig.loc_a, "pane-1", "session-a");
    let on_b = rig.enroll(&rig.loc_b, "pane-1", "session-b");
    let thread = rig.thread_with(&on_a, &on_b);
    Settled {
        rig,
        on_a,
        on_b,
        thread,
    }
}

// Kills: an outage of one server freezing the wake lane of the other, or A's
// wake worker reserving, refusing or calling A while A is down.
#[test]
fn host_a_down_freezes_only_a_wake_worker() {
    let Settled {
        rig,
        on_a,
        on_b,
        thread,
    } = settled("mh-a-down");
    rig.a.set_up(false);
    rig.observe_until(&rig.loc_a, "A to be marked down", || {
        rig.is_down(&rig.loc_a)
    });
    assert!(!rig.is_down(&rig.loc_b), "B is unaffected");
    // Attention for A's seat (a message from B) and for B's seat (one from A).
    rig.send(&on_b, &thread, "to a while a is down");
    rig.send(&on_a, &thread, "to b while a is down");
    rig.wait("B's idle seat to be woken", || !rig.b.woken().is_empty());
    // A's wake worker had every chance to run, and stayed frozen: nothing
    // submitted, nothing reserved, no refusal or outcome recorded.
    std::thread::sleep(Duration::from_millis(500));
    assert!(rig.a.woken().is_empty(), "no prompt to a down server");
    assert_eq!(
        rig.wake_row(&on_a.seat),
        Some((None, 0, None)),
        "no reservation, refusal backoff or outcome for A's seat\n{}",
        rig.wake_state()
    );
    assert!(rig.b.woken().iter().all(|pane| pane == "pane-1"));
}

// Kills: a recovery that does not kick A's wake worker, leaving the seat
// that waited out the outage asleep until the safety tick.
#[test]
fn host_a_recovery_kicks_a_and_wakes_its_seat() {
    let Settled {
        rig, on_b, thread, ..
    } = settled("mh-a-recovery");
    rig.a.set_up(false);
    rig.observe_until(&rig.loc_a, "A to be marked down", || {
        rig.is_down(&rig.loc_a)
    });
    rig.send(&on_b, &thread, "to a while a is down");
    std::thread::sleep(Duration::from_millis(500));
    assert!(rig.a.woken().is_empty(), "frozen while down");
    let reachability = Arc::clone(
        rig.sessions()
            .by_locator(&rig.loc_a)
            .unwrap()
            .reachability(),
    );
    let recoveries = reachability.recoveries();
    rig.a.set_up(true);
    rig.observe_until(&rig.loc_a, "A to be marked up", || !rig.is_down(&rig.loc_a));
    assert!(reachability.recoveries() > recoveries);
    rig.wait("A's seat to be woken after recovery", || {
        !rig.a.woken().is_empty()
    });
    assert!(rig.a.woken().iter().all(|pane| pane == "pane-1"));
}

// Kills: a continuity check-in routed to the daemon's startup server instead
// of the server it names (C1 across servers): the seat that was last on A
// reattaches onto B's pane, and B's wake worker, not A's, serves it.
#[test]
fn c1_resume_naming_b_reattaches_seat_from_a() {
    let Settled {
        rig,
        on_a,
        on_b,
        thread,
    } = settled("mh-c1");
    // The resumed session runs in B's pane-2.
    let resumed = rig.agent_process(&rig.loc_b, "pane-2");
    // A restarts: a new Herdr incarnation unresolves A's saved seat.
    rig.a.restart("boot-a-2");
    rig.observe_until(&rig.loc_a, "A's seat to be unresolved", || {
        rig.db()
            .query_row(
                "SELECT state FROM seats WHERE id=?1",
                [on_a.seat.as_str()],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
            == "unresolved"
    });
    let reattached = rig
        .call(Command::ContinuityCheckIn(ContinuityCheckIn {
            target: HostTargetId::new("pane-2"),
            host: rig.loc_b.clone(),
            harness: Harness::Codex,
            native_session: NativeSessionId::new("session-a"),
            source: ContinuityCheckIn::RESUME_SOURCE.to_owned(),
            operation: rig.op("continuity"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000c1"),
            liveness: Some(resumed),
        }))
        .unwrap_or_else(|error| panic!("continuity: {error:?}"));
    let CommandResult::ContinuityReattached(reattached) = reattached else {
        panic!("unexpected continuity result {reattached:?}")
    };
    assert_eq!(reattached.seat, on_a.seat);
    let (host, target): (i64, String) = rig
        .db()
        .query_row(
            "SELECT host_id, target_id FROM seats WHERE id=?1",
            [on_a.seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(host, rig.host_id_of(&rig.loc_b), "the seat now sits on B");
    assert_eq!(target, "pane-2");
    // B's wake worker serves the moved seat; A is never asked.
    let a_prompts = rig.a.woken().len();
    rig.send(&on_b, &thread, "to the moved seat");
    wait_until("B to wake the moved seat on pane-2", || {
        rig.b.woken().iter().any(|pane| pane == "pane-2")
    });
    assert_eq!(rig.a.woken().len(), a_prompts);
}

// Kills: a claim that names the wrong server being accepted for a seat that
// sits on another one (N10): B's seat claimed with host A is refused.
#[test]
fn claim_naming_b_seat_with_host_a_refused() {
    let Settled {
        rig, on_a, on_b, ..
    } = settled("mh-wrong-host");
    let mut wrong = on_b.claim.clone();
    wrong.host = rig.loc_a.clone();
    let refused = rig
        .call(Command::CheckIn(CheckIn {
            readonly: false,
            mode: CheckInMode::Current,
            claim: wrong,
            operation: rig.op("wrong-host"),
            lineage_parent: None,
        }))
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::CallerUnverified, "{refused:?}");
    assert!(
        refused.detail.contains("sits on another Herdr server"),
        "{refused:?}"
    );
    // The right host for the same seat is accepted.
    rig.call(Command::CheckIn(CheckIn {
        readonly: false,
        mode: CheckInMode::Current,
        claim: on_b.claim.clone(),
        operation: rig.op("right-host"),
        lineage_parent: None,
    }))
    .unwrap();
    drop(on_a);
}

/// The Claude SessionStart payload of a fresh session.
fn session_start(session: &str, source: &str) -> Vec<u8> {
    format!(
        r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"{source}"}}"#
    )
    .into_bytes()
}

struct HookRun {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The installed hook command, `herdr-threads --state-dir S --host-endpoint
/// <baked> hook claude`, run under a synthetic `claude` parent in a pane of the
/// server at `pane_host`.
fn run_hook(
    rig: &Rig,
    baked: &HostLocator,
    pane_host: &HostLocator,
    pane: &str,
    source: &str,
) -> HookRun {
    use std::io::Write;
    let state = state_dir(&rig.isolation).to_string_lossy().into_owned();
    let mut command = herdr_threads::test_support::harness_shim::under_harness(
        "claude",
        Path::new(BIN),
        &[
            "--state-dir",
            &state,
            "--host-endpoint",
            baked.as_str(),
            "hook",
            "claude",
        ],
        &[],
    );
    command
        .env("HERDR_ENV", "1")
        .env("HERDR_PANE_ID", pane)
        .env("HERDR_SOCKET_PATH", pane_host.as_str())
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_BIN_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child: OwnedChild = command.spawn_owned().expect("spawn the hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&session_start("hook-session", source))
        .unwrap();
    let output = child.wait_with_output().expect("hook output");
    HookRun {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The mod's `watch` child under a synthetic `claude` parent in a pane of the
/// server at `pane_host` (no `--host-endpoint`: the pane's server is the claim
/// host), and the first JSON line it prints.
fn first_watch_line(
    rig: &Rig,
    pane_host: &HostLocator,
    pane: &str,
    session: &str,
) -> serde_json::Value {
    use std::io::BufRead;
    let state = state_dir(&rig.isolation).to_string_lossy().into_owned();
    let mut command = herdr_threads::test_support::harness_shim::under_harness(
        "claude",
        Path::new(BIN),
        &[
            "--state-dir",
            &state,
            "watch",
            "--harness",
            "claude",
            "--session",
            session,
        ],
        &[],
    );
    command
        .env("HERDR_ENV", "1")
        .env("HERDR_PANE_ID", pane)
        .env("HERDR_SOCKET_PATH", pane_host.as_str())
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_BIN_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Stopped with its process group when `child` drops, registered or not.
    let mut child: OwnedChild = command.spawn_owned().expect("spawn the watch child");
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(line);
    });
    let line = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("the watch child printed a first line");
    serde_json::from_str(&line).unwrap_or_else(|error| panic!("watch line {line:?}: {error}"))
}

// Kills: a hook that is silenced as foreign (or claims the baked server)
// in a pane of a second Herdr server: the installed argv bakes A while the
// daemon knows only A; the hook runs in a pane of B, its claim carries B, the
// daemon attaches B lazily, and the binding opens on B's seat.
#[test]
fn multi_host_hook_claim_attaches_b() {
    let rig = Rig::start("mh-hook", true);
    assert!(rig.attached(&rig.loc_a));
    assert!(!rig.attached(&rig.loc_b));
    let ran = run_hook(&rig, &rig.loc_a, &rig.loc_b, "pane-2", "startup");
    assert_eq!(ran.code, Some(0), "{}", ran.stderr);
    assert!(
        !ran.stdout.is_empty(),
        "the hook answered the SessionStart rather than staying silent: {}",
        ran.stderr
    );
    assert!(
        rig.attached(&rig.loc_b),
        "the claim attached B lazily\nstdout: {}\nstderr: {}\n{}",
        ran.stdout,
        ran.stderr,
        rig.wake_state()
    );
    let output: serde_json::Value = serde_json::from_str(&ran.stdout).expect("SessionStart JSON");
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("SessionStart context");
    assert!(
        !context.starts_with("herdr-threads: check-in unavailable ("),
        "cold startup must enroll in its first callback: {context}; {}",
        ran.stderr
    );
    let (host, binding_session): (i64, String) = rig
        .db()
        .query_row(
            "SELECT s.host_id, b.native_session FROM seats s \
             JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.target_id='pane-2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or_else(|error| {
            panic!(
                "a binding on the seat of B's pane: {error}; stdout: {}; stderr: {}; {}",
                ran.stdout,
                ran.stderr,
                rig.wake_state()
            )
        });
    assert_eq!(host, rig.host_id_of(&rig.loc_b), "the seat sits on B");
    assert_eq!(binding_session, "hook-session");
    let on_a: i64 = rig
        .db()
        .query_row(
            "SELECT count(*) FROM seats WHERE host_id=?1",
            [rig.host_id_of(&rig.loc_a)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(on_a, 0, "nothing was claimed on the baked server");
    // The mod's watch child in the same pane registers its channel on B's seat
    // as well: its claim names B, which is attached, and the daemon accepts it.
    let first = first_watch_line(&rig, &rig.loc_b, "pane-2", "hook-session");
    assert_eq!(first["kind"], "status", "{first}");
    assert_eq!(first["state"], "connected", "{first}");
}

// ---- H1b / D1: the kind probe, before any `herdr_hosts` row ----

mod kind_probe {
    use herdr_threads::{
        host::probe::probe_kind,
        ports::HostKind,
        test_support::{
            fake_hc::FakeHc, fake_herdr::FakeHerdr, isolated_herdr::IsolatedHerdr,
            scratch::temp_path,
        },
    };
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    /// A scripted peer: one canned reply line per method (`None`: read the
    /// request and never answer). Stops and unlinks its socket on drop.
    struct ScriptedPeer {
        socket: PathBuf,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl ScriptedPeer {
        fn start(hello: Option<&str>, ping: Option<&str>) -> Self {
            let socket = temp_path(format!("ht-scripted-{}", uuid::Uuid::new_v4().simple()));
            let listener = UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let replies = (hello.map(str::to_owned), ping.map(str::to_owned));
            let worker = {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut held: Vec<UnixStream> = Vec::new();
                    while !stop.load(Ordering::SeqCst) {
                        let Ok((stream, _)) = listener.accept() else {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        };
                        stream.set_nonblocking(false).unwrap();
                        let mut line = String::new();
                        let _ = BufReader::new(&stream).read_line(&mut line);
                        let reply = if line.contains("\"hello\"") {
                            &replies.0
                        } else {
                            &replies.1
                        };
                        if let Some(reply) = reply {
                            let mut stream = &stream;
                            let _ = writeln!(stream, "{reply}");
                        }
                        // A silent answer keeps the connection open.
                        held.push(stream);
                    }
                })
            };
            Self {
                socket,
                stop,
                worker: Some(worker),
            }
        }
    }

    impl Drop for ScriptedPeer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    const BUDGET: Duration = Duration::from_millis(800);
    const PONG: &str =
        r#"{"id":"hello-probe","result":{"type":"pong","version":"0.9.1","protocol":22}}"#;

    // Kills: a probe that sees `core.id` in a fake_hc reply and still calls it
    // Herdr, or sends a Herdr reply down the hc path. The ordering against the
    // row write is `service::host_sessions::tests::probe_runs_before_the_row_*`.
    #[test]
    fn probe_classifies_fake_hc_and_fake_herdr_before_row_written() {
        let hc = FakeHc::start();
        assert_eq!(probe_kind(hc.socket(), BUDGET).unwrap(), HostKind::Hc);
        assert_eq!(
            hc.calls()
                .iter()
                .map(|c| c.method.as_str())
                .collect::<Vec<_>>(),
            vec!["hello"],
            "an hc answer to hello needs no second call"
        );

        let socket = temp_path(format!(
            "hello-probe-herdr-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let herdr = FakeHerdr::start("probe", &socket, &["w1:p1"]);
        assert_eq!(probe_kind(herdr.socket(), BUDGET).unwrap(), HostKind::Herdr);
    }

    // Kills: a probe that waits for a reply that never comes, and one that
    // classifies a peer answering neither dialect.
    #[test]
    fn silent_peer_fails_within_probe_deadline() {
        let silent = ScriptedPeer::start(None, None);
        let started = Instant::now();
        let error = probe_kind(&silent.socket, Duration::from_millis(400)).unwrap_err();
        let elapsed = started.elapsed();
        assert_eq!(
            error.code,
            herdr_threads::protocol::results::ErrorCode::HostUnavailable
        );
        assert!(
            elapsed >= Duration::from_millis(200) && elapsed < Duration::from_millis(900),
            "the probe waits out its deadline and no longer: {elapsed:?}"
        );
        // Silent on hello only: the ping still classifies, inside the deadline.
        let late = ScriptedPeer::start(None, Some(PONG));
        assert_eq!(probe_kind(&late.socket, BUDGET).unwrap(), HostKind::Herdr);
        // An error reply to hello (an unknown method) is not an hc answer.
        let errors = ScriptedPeer::start(
            Some(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"no method"}}"#),
            Some(PONG),
        );
        assert_eq!(probe_kind(&errors.socket, BUDGET).unwrap(), HostKind::Herdr);
        // A peer that closes on hello and answers neither is unavailable.
        assert!(probe_kind(&temp_path("ht-no-such-socket"), BUDGET).is_err());
    }

    /// The reply a live Herdr TUI socket gave to a `hello` line (D1), captured
    /// once in an isolated session; `CLOSED` when it closed without a line.
    const CAPTURED: &str = include_str!("../fixtures/herdr-hello/reply.txt");

    // Kills: a Herdr that answers hello in a way the probe takes for hc, and a
    // fixture that no longer matches what the probe does with it.
    #[test]
    fn d1_captured_herdr_hello_reply_classifies_as_herdr() {
        let reply = CAPTURED.trim();
        let hello = (reply != "CLOSED").then_some(reply);
        let peer = ScriptedPeer::start(hello, Some(PONG));
        assert_eq!(probe_kind(&peer.socket, BUDGET).unwrap(), HostKind::Herdr);
    }

    // D1 live: an isolated Herdr test session answers hello (or closes) and
    // never hangs; the probe classifies it as Herdr. Skips when `herdr` is
    // absent and HT_SKIP_HERDR_TESTS=1. `HT_D1_CAPTURE=1` prints the raw
    // reply, which is how `herdr-hello/reply.txt` was recorded.
    #[test]
    fn d1_live_herdr_hello_capture() {
        let Some(herdr) = IsolatedHerdr::new("d1_live_herdr_hello_capture") else {
            return;
        };
        herdr.start();
        let socket = herdr.socket_path();
        assert_eq!(
            probe_kind(&socket, Duration::from_secs(3)).unwrap(),
            HostKind::Herdr
        );

        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\"}\n")
            .unwrap();
        let mut line = String::new();
        let read = BufReader::new(&stream).read_line(&mut line);
        let reply = match read {
            Ok(0) => "CLOSED".to_owned(),
            Ok(_) => line.trim().to_owned(),
            Err(error) => panic!("a live Herdr hung on hello: {error}"),
        };
        if std::env::var_os("HT_D1_CAPTURE").is_some() {
            eprintln!("D1 CAPTURE: {reply}");
        }
        let parsed: Option<serde_json::Value> = serde_json::from_str(&reply).ok();
        assert!(
            parsed.is_none_or(|v| v["result"]["core"]["id"].as_str().is_none()),
            "a Herdr TUI socket must not look like an hc core: {reply}"
        );
        herdr.stop();
    }
}

/// H1-twohost: the dialled-only consumers (07 §5.3) over a node with a Herdr TUI fixture and a
/// fake herdr-core, where the core holds the lowest host id. Every consumer here must filter on
/// a capability answer, so none of them may take "the first dialled host".
///
/// Crate-private consumers have their two-host tests in the library: `place()` in
/// `consumer_audit::place_two_host`, the seatless core with a wake-armed or unhinted named binding
/// in `tmux_idle_exit::combined_idle_exit_predicate_matrix` (the "seatless core" row), and S5's
/// two-kind restore in `named_restore::s5_unnamed_restored_row_held_unresolved_on_two_kind_node`.
mod two_kind {
    use super::*;
    use herdr_threads::{
        host::hc::{HcClient, HcHost},
        ports::{ArchivalEvidence, HostKind},
        service::{archival::ArchivalHosts, host_sessions::HostSession},
        test_support::fake_hc::FakeHc,
    };

    /// A daemon over the core `hc` (attached first: host id 1) and the TUI fixture `tui`.
    struct TwoKind {
        // Fields drop in order: the daemon stops before the fixtures under it go.
        daemon: Daemon,
        hc_session: Arc<HostSession>,
        tui_session: Arc<HostSession>,
        loc_hc: HostLocator,
        loc_tui: HostLocator,
        isolation: TestIsolation,
        _tui: Arc<ServerHost>,
        _hc: FakeHc,
    }

    fn budget(clock: &Arc<dyn Clock>) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 20_000),
            cancellation: Cancellation::default(),
        }
    }

    impl TwoKind {
        /// Both hosts attached, the core first; `hint` names the startup hint (the core is
        /// attached through it, the TUI lazily). `idle` is the idle-exit period, if any.
        fn start(label: &str, hint_hc: bool, idle: Option<Duration>, attach_tui: bool) -> Self {
            let isolation = TestIsolation::new(label);
            let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
            let hc = FakeHc::start();
            let loc_hc = HostLocator::parse(hc.socket().to_string_lossy().into_owned()).unwrap();
            let loc_tui = {
                let socket = isolation.socket_path("herdr-tui.sock");
                let directory = std::fs::canonicalize(socket.parent().unwrap()).unwrap();
                HostLocator::parse(
                    directory
                        .join("herdr-tui.sock")
                        .to_string_lossy()
                        .into_owned(),
                )
                .unwrap()
            };
            let tui = ServerHost::guarded(&clock, "boot-tui", &["pane-1"]);
            let world = FakeHostWorld::new();
            world.add(
                &loc_hc,
                Arc::new(HcHost::with_clock(
                    HcClient::new(hc.socket()),
                    Arc::clone(&clock),
                )) as Arc<dyn HostPort>,
            );
            world.add(&loc_tui, Arc::clone(&tui) as Arc<dyn HostPort>);
            let mut config = ServiceConfig::default().with_wake_batch_delay(0).unwrap();
            if let Some(idle) = idle {
                config = config.with_idle_exit_after(idle);
            }
            let daemon = Daemon::start_composed(
                &isolation,
                &clock,
                &world,
                hint_hc.then(|| loc_hc.clone()),
                config,
                true,
            );
            let sessions = daemon.probe.host_sessions().expect("host sessions");
            let hc_session = sessions
                .resolve(&loc_hc, &budget(&clock))
                .expect("the core attaches");
            let tui_session = if attach_tui {
                sessions
                    .resolve(&loc_tui, &budget(&clock))
                    .expect("the TUI attaches")
            } else {
                Arc::clone(&hc_session)
            };
            Self {
                daemon,
                hc_session,
                tui_session,
                loc_hc,
                loc_tui,
                isolation,
                _tui: tui,
                _hc: hc,
            }
        }

        fn sessions(&self) -> Arc<herdr_threads::service::host_sessions::HostSessions> {
            self.daemon.probe.host_sessions().expect("host sessions")
        }
    }

    // Kills: a rig that does not give the core the lowest host id, and a probe that takes the
    // core for a Herdr server (every other test here would then pass for the wrong reason).
    #[test]
    fn hc_holds_the_lowest_host_id_and_both_kinds_are_probed() {
        let rig = TwoKind::start("mh-two-kind-ids", false, None, true);
        assert!(rig.hc_session.host_id() < rig.tui_session.host_id());
        assert_eq!(rig.hc_session.port().kind(), HostKind::Hc);
        assert_eq!(rig.tui_session.port().kind(), HostKind::Herdr);
        let kinds: Vec<String> = rig
            .daemon_db()
            .prepare("SELECT kind FROM herdr_hosts WHERE kind<>'named' ORDER BY host_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(kinds, ["hc", "herdr"]);
        let _ = (&rig.loc_hc, &rig.loc_tui, &rig.isolation);
    }

    impl TwoKind {
        fn daemon_db(&self) -> rusqlite::Connection {
            let paths = herdr_threads::test_support::resolve_seeded(&state_dir(&self.isolation));
            let db = rusqlite::Connection::open(paths.database_path).unwrap();
            db.busy_timeout(Duration::from_secs(5)).unwrap();
            db
        }
    }

    // Kills: a startup primary that falls back to the lowest dialled host (the core: Health and
    // the advertised guarded bootstrap would follow a server that cannot run guarded bootstrap),
    // and one that ignores a hint naming the TUI.
    #[test]
    fn startup_primary_two_host() {
        let rig = TwoKind::start("mh-primary-two-host", false, None, true);
        let sessions = rig.sessions();
        let primary = sessions.startup_primary(None).expect("a guarded host");
        assert!(Arc::ptr_eq(&primary, &rig.tui_session), "never the core");
        let hinted = sessions.startup_primary(Some(&rig.loc_tui)).unwrap();
        assert!(Arc::ptr_eq(&hinted, &rig.tui_session));
        // With only the core attached there is no guarded-start host: store-only Health.
        drop(rig);
        let alone = TwoKind::start("mh-primary-core-only", false, None, false);
        assert!(alone.sessions().startup_primary(None).is_none());
    }

    // Kills: `first_host_pacer` following the lowest dialled session, whose core runs no wake
    // lane, so the wake Pacer of a Herdr TUI on the same node is never found.
    #[test]
    fn first_host_pacer_two_host() {
        let rig = TwoKind::start("mh-pacer-two-host", false, None, true);
        assert!(
            rig.hc_session.wake().is_none(),
            "the core runs no wake lane"
        );
        let sessions = rig.sessions();
        let wake = sessions
            .first_pacer(Lane::Wakes)
            .expect("the TUI's wake lane");
        assert!(Arc::ptr_eq(&wake, rig.tui_session.wake().unwrap().pacer()));
        // The lane the TUI shares with the core: the lowest id leads.
        let observation = sessions.first_pacer(Lane::Observation).unwrap();
        assert!(Arc::ptr_eq(
            &observation,
            rig.hc_session.observation().pacer()
        ));
        assert!(sessions.first_pacer(Lane::Archival).is_none());
    }

    // Kills: a peer-discovery gate opened by a reachable core (`machines` come from the Herdr
    // CLI), and a gate closed by one while a Herdr server is attached.
    #[test]
    fn discovery_gate_two_host() {
        let core_only = TwoKind::start("mh-gate-core-only", false, None, false);
        assert!(core_only.hc_session.port().creates_targets_itself());
        assert!(
            !core_only.sessions().any_reachable(),
            "a core alone never opens the Herdr discovery gate"
        );
        // One daemon at a time.
        drop(core_only);
        let both = TwoKind::start("mh-gate-two-host", false, None, true);
        assert!(both.sessions().any_reachable());
    }

    // Kills: archival sampling the core (no evidence: read as incoherent for ever) or skipping
    // the TUI because the core came first.
    #[test]
    fn archival_two_host() {
        let rig = TwoKind::start("mh-archival-two-host", false, None, true);
        assert_eq!(
            rig.hc_session.port().archival_evidence(),
            ArchivalEvidence::NoEvidence
        );
        let sessions = rig.sessions();
        let listed: Vec<_> = ArchivalHosts::attached(&*sessions)
            .iter()
            .map(|host| host.host_id)
            .collect();
        assert_eq!(listed, vec![rig.tui_session.host_id()]);
        assert!(ArchivalHosts::get(&*sessions, rig.hc_session.host_id()).is_none());
        assert!(ArchivalHosts::get(&*sessions, rig.tui_session.host_id()).is_some());
    }

    // Kills: the production guarded `BootstrapRuntime` gate (07 §4.6) keyed on the lowest id or
    // on the kind: only the guarded-start TUI has the V1/V2 runtime, only the core the V3 one,
    // and the daemon advertises guarded bootstrap on the TUI's locator.
    #[test]
    fn bootstrap_runtime_gate_two_host() {
        let rig = TwoKind::start("mh-bootstrap-two-host", false, None, true);
        assert!(rig.hc_session.bootstrap().is_none());
        assert!(rig.tui_session.bootstrap().is_some());
        let v3 = rig
            .hc_session
            .target_bootstrap()
            .expect("the core's V3 runtime");
        assert_eq!(
            v3.canonical.host_endpoint,
            PathBuf::from(rig.loc_hc.as_str())
        );
        assert!(rig.tui_session.target_bootstrap().is_none());
        // The daemon's advertisement: the startup primary, when it has a runtime.
        let guarded = rig
            .sessions()
            .startup_primary(None)
            .filter(|session| session.bootstrap().is_some())
            .map(|session| session.locator().clone());
        assert_eq!(guarded, Some(rig.loc_tui.clone()));
    }

    /// Waits for the daemon thread to end by itself (idle exit), up to `limit`.
    fn exits_by_itself(rig: &mut TwoKind, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            if rig
                .daemon
                .thread
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    // Kills: an idle exit that counts a dialled host holding no resolved seat, so a daemon with
    // only a seatless core attached never exits (the core's session is attached, its host holds
    // no seat); and, in the same run, one that exits while a client is connected.
    #[test]
    fn idle_exit_seatless_core_exits() {
        let mut rig = TwoKind::start(
            "mh-idle-seatless-core",
            true,
            Some(Duration::from_millis(400)),
            false,
        );
        assert_eq!(rig.sessions().attached_count(), 1, "the core is attached");
        assert!(
            exits_by_itself(&mut rig, Duration::from_secs(20)),
            "a daemon with only a seatless core attached idles out"
        );
    }
}
