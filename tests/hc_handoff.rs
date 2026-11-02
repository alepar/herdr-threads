//! H3d2 (ht-kqrt.57): the hc handoff surface against `fake_hc`: target selection from the caller
//! (`--space`, `--new-tab`, `--new-terminal`, discovery), the refusals, and the launch gate from
//! the shim's request through the daemon's decision.
//!
//! The gate tests compose the real pieces without a daemon process: the shim's `authorize`
//! (`cli::launch_gate`) calls a client that hands the request to the daemon's decision
//! (`service::launch_gate::authorize`) over a scripted canonical view and the real `HcHost`
//! observer on the fake core.

use herdr_threads::{
    app::SystemClock,
    cli::{
        caller_location::CallerLocator,
        commands::{CliAction, parse_argv},
        hc_target,
        host_client::LaunchHandle,
        launch_gate,
        topology_handoff::{Request, TargetSelection},
    },
    host::hc::{HcBootstrapObserver, HcClient, HcHost},
    ports::{
        BootstrapObserver, CreateTargetOutcome, CreateTargetRequest, CreatedTarget,
        HostCallContext, HostPort, LaunchGateView, LocalClient, NativeLaunchCapability, Placement,
    },
    protocol::{
        commands::Command,
        handoff::{AuthorizeLaunchGate, BootstrapAttempt, LaunchGateRefusal, LaunchGateReply},
        ids::{HostCallId, HostTargetId, OperationId},
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::launch_gate::{GateWorld, authorize},
    test_support::{
        fake_hc::{FakeHc, FakeTerminal, RunFailure},
        scratch::temp_path,
    },
};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}

fn budget(clock: &dyn Clock, ms: u64) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + ms),
        cancellation: Cancellation::default(),
    }
}

fn context(clock: &dyn Clock) -> HostCallContext {
    HostCallContext {
        budget: budget(clock, 10_000),
        expected_boot: None,
        expected_epoch: None,
    }
}

fn request(
    selection: &TargetSelection,
    witness: herdr_threads::ports::HostWitness,
) -> CreateTargetRequest {
    CreateTargetRequest {
        correlation: HostCallId::new("corr-1"),
        placement: selection.placement.clone(),
        cwd: "/tmp".into(),
        label: "worker".into(),
        focus: false,
        env: Default::default(),
        directory_context: None,
        expected_witness: witness,
        gate_argv: [
            "herdr-threads",
            "internal",
            "launch-gate",
            "--bootstrap",
            "op-1:1",
            "--",
            "sh",
        ]
        .map(String::from)
        .to_vec(),
    }
}

fn handoff_request(args: &[&str]) -> Request {
    let mut argv = vec!["ht", "handoff"];
    argv.extend_from_slice(args);
    argv.extend(["--kind", "codex", "--", "literal work"]);
    match parse_argv(argv).expect("parses").action {
        CliAction::TopologyHandoff(request) => request,
        other => panic!("a topology handoff, got {other:?}"),
    }
}

fn terminal_site(fake: &FakeHc, id: &str) -> CallerLocator {
    CallerLocator::Terminal {
        socket: fake.socket().to_string_lossy().into_owned(),
        id: id.into(),
    }
}

fn no_env(_: &str) -> Option<OsString> {
    None
}

/// Create the target a selection names on the fake core and return it.
fn create(fake: &FakeHc, selection: &TargetSelection) -> CreatedTarget {
    let clock = clock();
    let handle = LaunchHandle::probed(
        fake.socket(),
        clock.clone(),
        &budget(clock.as_ref(), 10_000),
    )
    .expect("probe the fake core");
    let request = request(selection, handle.witness().expect("a witness").clone());
    match handle
        .create_port()
        .create_target(&request, &context(clock.as_ref()))
    {
        CreateTargetOutcome::Created(created) => *created,
        other => panic!("expected Created, got {other:?}"),
    }
}

fn run_params(fake: &FakeHc) -> serde_json::Value {
    let calls = fake.calls_to("terminal.run");
    assert_eq!(calls.len(), 1, "exactly one terminal.run: {calls:?}");
    calls[0].params.clone()
}

// ---- parsing and selection ----

#[test]
fn new_terminal_parses_and_conflicts_with_new_tab() {
    let request = handoff_request(&["--new-terminal"]);
    assert!(request.new_terminal && request.new_tab.is_none());
    let both = parse_argv([
        "ht",
        "handoff",
        "--new-terminal",
        "--new-tab",
        "x",
        "--kind",
        "codex",
        "--",
        "b",
    ]);
    assert!(both.is_err(), "--new-terminal and --new-tab are exclusive");
}

#[test]
fn space_label_set() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let request = handoff_request(&["--new-tab", "peer", "--space", "w1"]);
    let selection = hc_target::select(&request, Some(&terminal_site(&fake, "t1")), false, &no_env)
        .unwrap()
        .expect("a terminal-site caller creates on its own core");
    assert_eq!(selection.endpoint, fake.socket());
    assert_eq!(
        selection.placement,
        Placement::NewTab {
            space: HostTargetId::new("w1")
        }
    );
    create(&fake, &selection);
    let labels = run_params(&fake)["labels"].clone();
    let labels = labels.as_object().expect("labels");
    let keys: std::collections::BTreeSet<&str> = labels.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["herdr.space", "herdr.tab", "ht.bootstrap", "ht.label"].into()
    );
    assert_eq!(labels["herdr.space"], "w1");
    assert!(
        !labels["herdr.tab"].as_str().unwrap().is_empty(),
        "a fresh tab id"
    );
}

#[test]
fn new_terminal_no_app_labels_parent_caller() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let request = handoff_request(&["--new-terminal"]);
    let selection = hc_target::select(&request, Some(&terminal_site(&fake, "t1")), false, &no_env)
        .unwrap()
        .expect("selected");
    assert_eq!(
        selection.placement,
        Placement::NewTerminal {
            parent: HostTargetId::new("t1")
        }
    );
    create(&fake, &selection);
    let params = run_params(&fake);
    let labels = params["labels"].as_object().expect("labels");
    assert!(!labels.contains_key("herdr.space") && !labels.contains_key("herdr.tab"));
    assert_eq!(params["parent"], "t1", "parent is the caller");
}

#[test]
fn launch_selects_its_core_as_handoff_does() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let terminal = terminal_site(&fake, "t1");
    let pane = CallerLocator::pane("w1:p1");
    let wanted =
        |new_tab: bool, new_terminal: bool, space: Option<&'static str>| hc_target::Wanted {
            new_tab,
            new_terminal,
            space,
            narrowed: false,
            automatic: true,
        };
    // A Terminal site gives its own core: a terminal parented on the caller, or a tab in the space.
    let selected =
        hc_target::select_wanted(&wanted(false, true, None), Some(&terminal), false, &no_env)
            .unwrap()
            .expect("a core");
    assert_eq!(selected.endpoint, fake.socket());
    assert_eq!(
        selected.placement,
        Placement::NewTerminal {
            parent: HostTargetId::new("t1")
        }
    );
    let selected = hc_target::select_wanted(
        &wanted(true, false, Some("w1")),
        Some(&terminal),
        false,
        &no_env,
    )
    .unwrap()
    .expect("a core");
    assert_eq!(
        selected.placement,
        Placement::NewTab {
            space: HostTargetId::new("w1")
        }
    );
    // A Herdr pane keeps the Herdr path for a tab, and has no terminal to parent a new one.
    assert!(
        hc_target::select_wanted(
            &wanted(true, false, Some("w1")),
            Some(&pane),
            false,
            &no_env
        )
        .unwrap()
        .is_none()
    );
    assert!(
        hc_target::select_wanted(&wanted(false, true, None), Some(&pane), false, &no_env).is_err()
    );
    // A tab with no space on a core is the existing refusal.
    let error =
        hc_target::select_wanted(&wanted(true, false, None), Some(&terminal), false, &no_env)
            .expect_err("refused");
    assert!(error.to_string().contains("--space"), "{error}");
}

#[test]
fn herdr_callers_are_never_redirected_and_new_terminal_needs_a_terminal() {
    // A Herdr pane caller or a Herdr context keeps today's new-tab path.
    let tab = handoff_request(&["--new-tab", "peer", "--space", "w1"]);
    let pane = CallerLocator::pane("w1:p1");
    assert!(
        hc_target::select(&tab, Some(&pane), false, &no_env)
            .unwrap()
            .is_none()
    );
    assert!(
        hc_target::select(&tab, None, true, &no_env)
            .unwrap()
            .is_none()
    );
    // --new-terminal has no meaning there.
    let terminal = handoff_request(&["--new-terminal"]);
    assert!(hc_target::select(&terminal, Some(&pane), false, &no_env).is_err());
    assert!(hc_target::select(&terminal, None, true, &no_env).is_err());
}

#[test]
fn thread_flags_into_a_hc_target_are_refused_before_any_creation() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    for flags in [
        &["--new-tab", "peer", "--space", "w1", "--new-thread"][..],
        &["--new-tab", "peer", "--space", "w1", "--thread", "thread-1"][..],
    ] {
        let request = handoff_request(flags);
        assert!(!request.automatic, "{flags:?} takes the V1 path");
        let error = hc_target::select(&request, Some(&terminal_site(&fake, "t1")), false, &no_env)
            .expect_err("refused");
        assert!(error.to_string().contains("--thread"), "{error}");
    }
    assert!(
        fake.calls_to("terminal.run").is_empty(),
        "nothing was created"
    );
}

#[test]
fn a_hc_new_tab_needs_a_space() {
    let fake = FakeHc::start();
    let request = handoff_request(&["--new-tab", "peer"]);
    let error = hc_target::select(&request, Some(&terminal_site(&fake, "t1")), false, &no_env)
        .expect_err("refused");
    assert!(error.to_string().contains("--space"), "{error}");
}

#[test]
fn in_place_pane_tn_refused_by_port_answer() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let host = HcHost::new(HcClient::new(fake.socket()));
    // The port answer is the whole refusal: a launch into `tN` never reaches the host.
    assert_eq!(
        host.native_launch_capability(),
        NativeLaunchCapability::Unsupported
    );
    struct NoSeats;
    impl herdr_threads::harness::launch::LaunchSeatResolver for NoSeats {
        fn resolve_for_launch(
            &self,
            _: &HostTargetId,
            _: &herdr_threads::ports::HostObservation,
            _: &CallBudget,
        ) -> Result<herdr_threads::protocol::ids::SeatId, ApiError> {
            panic!("a refused launch resolves no seat");
        }
    }
    let clock = clock();
    let error = herdr_threads::harness::launch::launch_seat_guards(
        herdr_threads::harness::registry::builtins(),
        &host,
        &NoSeats,
        clock.as_ref(),
        &HostTargetId::new("t1"),
        &budget(clock.as_ref(), 5_000),
    )
    .expect_err("in-place launch is refused");
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert!(fake.calls_to("terminal.run").is_empty());
}

/// A scratch XDG layout holding cores for discovery, and a PATH holding an `hc` that records
/// being run.
struct Discovery {
    root: PathBuf,
    marker: PathBuf,
}
impl Discovery {
    fn new() -> Self {
        let root = temp_path(format!("ht-hch-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(root.join("run/herdr-core")).unwrap();
        std::fs::create_dir_all(root.join("state/herdr-core")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let marker = root.join("hc-was-run");
        let shim = root.join("bin/hc");
        std::fs::write(&shim, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { root, marker }
    }
    fn core(&self, session: &str, fake: &FakeHc) {
        let id = HcHost::new(HcClient::new(fake.socket()))
            .core_witness()
            .map(|w| match w {
                herdr_threads::ports::HostWitness::HcCore { core_id, .. } => core_id,
                _ => unreachable!(),
            })
            .unwrap();
        let dir = self.root.join("state/herdr-core").join(session);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("core.json"),
            format!(r#"{{"id":"{id}","name":"x","machine_id":"m"}}"#),
        )
        .unwrap();
        std::fs::hard_link(
            fake.socket(),
            self.root
                .join("run/herdr-core")
                .join(format!("{session}.sock")),
        )
        .unwrap();
    }
    fn env(&self) -> impl Fn(&str) -> Option<OsString> + use<> {
        let root = self.root.clone();
        move |name| match name {
            "XDG_STATE_HOME" => Some(root.join("state").into()),
            "XDG_RUNTIME_DIR" => Some(root.join("run").into()),
            "XDG_CONFIG_HOME" => Some(root.join("config").into()),
            "HOME" => Some(root.join("home").into()),
            "PATH" => Some(root.join("bin").into()),
            _ => None,
        }
    }
}

#[test]
fn plain_shell_handoff_uses_hc_discovery_never_spawns_hc() {
    let world = Discovery::new();
    let fake = FakeHc::start();
    world.core("alpha", &fake);
    let request = handoff_request(&["--new-tab", "peer", "--space", "w1"]);
    let selection = hc_target::select(&request, None, false, &world.env())
        .unwrap()
        .expect("a plain shell uses discovery");
    assert_eq!(
        selection.endpoint,
        world.root.join("run/herdr-core/alpha.sock")
    );
    assert!(!world.marker.exists(), "hc was never spawned");
    assert!(
        fake.calls().iter().all(|c| c.method == "hello"),
        "a scan only says hello"
    );

    // No core, and two cores, are refusals that name what was found.
    let none = Discovery::new();
    let error = hc_target::select(&request, None, false, &none.env()).expect_err("no core");
    assert!(error.to_string().contains("no herdr-core"), "{error}");
    let second = FakeHc::start();
    second.state().rotate_core_id("core-second");
    world.core("beta", &second);
    let error = hc_target::select(&request, None, false, &world.env()).expect_err("ambiguous");
    assert!(error.to_string().contains("alpha") && error.to_string().contains("beta"));
    assert!(!world.marker.exists() && !none.marker.exists());
}

// ---- the launch gate ----

/// One bootstrap on the fake core: its creation and the scripted canonical view.
struct Gate {
    fake: FakeHc,
    created: CreatedTarget,
    observers: Vec<Arc<dyn BootstrapObserver>>,
    log: Arc<Mutex<Vec<String>>>,
    /// The view reports the Send staged from this poll on (`usize::MAX`: never).
    staged_from: usize,
    polls: AtomicUsize,
    attempt_current: bool,
    /// The view reports the bootstrap cancelled.
    cancelled: bool,
    known: bool,
}

impl Gate {
    fn new(staged_from: usize) -> Self {
        let fake = FakeHc::start();
        fake.add_terminal(FakeTerminal::live("t1"));
        let request = handoff_request(&["--new-tab", "peer", "--space", "w1"]);
        let selection =
            hc_target::select(&request, Some(&terminal_site(&fake, "t1")), false, &no_env)
                .unwrap()
                .unwrap();
        let created = create(&fake, &selection);
        let observers: Vec<Arc<dyn BootstrapObserver>> = vec![Arc::new(HcBootstrapObserver::new(
            Arc::new(HcHost::new(HcClient::new(fake.socket()))),
        ))];
        Self {
            fake,
            created,
            observers,
            log: Arc::default(),
            staged_from,
            polls: AtomicUsize::new(0),
            attempt_current: true,
            cancelled: false,
            known: true,
        }
    }

    fn view(&self, _: &CallBudget) -> Result<Option<LaunchGateView>, ApiError> {
        if !self.known {
            return Ok(None);
        }
        let poll = self.polls.fetch_add(1, Ordering::SeqCst);
        let staged = poll >= self.staged_from;
        if poll == self.staged_from {
            self.log.lock().unwrap().push("staged".into());
        }
        Ok(Some(LaunchGateView {
            created: Some(self.created.clone()),
            current_attempt: self.attempt_current,
            cancelled: self.cancelled,
            send_staged: staged,
        }))
    }

    fn request(&self, terminal: &str, core: Option<&str>, timeout_ms: u32) -> AuthorizeLaunchGate {
        let herdr_threads::ports::HostWitness::HcCore { core_id, .. } = &self.created.witness
        else {
            panic!("an hc creation")
        };
        AuthorizeLaunchGate {
            compound: OperationId::new("op-1"),
            attempt: BootstrapAttempt::new(1).unwrap(),
            core_id: core.unwrap_or(core_id).to_owned(),
            terminal: terminal.to_owned(),
            timeout_ms,
            socket: None,
        }
    }

    fn decide(&self, request: &AuthorizeLaunchGate) -> LaunchGateReply {
        let clock = clock();
        let view = |b: &CallBudget| self.view(b);
        authorize(
            request,
            &GateWorld {
                view: &view,
                observers: &self.observers,
            },
            clock.as_ref(),
            &budget(clock.as_ref(), 20_000),
        )
        .expect("a decision")
    }
}

/// The shim's client: the daemon's decision, logging the moment of the reply.
struct InProcessDaemon<'a>(&'a Gate);
impl LocalClient for InProcessDaemon<'_> {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        let Command::AuthorizeLaunchGate(request) = command else {
            panic!("the shim sends only AuthorizeLaunchGate")
        };
        let reply = self.0.decide(&request);
        self.0.log.lock().unwrap().push("reply".into());
        Ok(CommandResult::LaunchGate(reply))
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}

#[test]
fn handoff_into_fake_hc_runs_gate_shim() {
    let gate = Gate::new(3);
    // The terminal fake_hc created runs the gate shim.
    let argv: Vec<String> = serde_json::from_value(run_params(&gate.fake)["argv"].clone()).unwrap();
    let CliAction::InternalLaunchGate {
        bootstrap,
        config_roots,
        argv: harness,
    } = parse_argv(argv.iter().map(String::as_str)).unwrap().action
    else {
        panic!("the recorded argv is the gate shim: {argv:?}")
    };
    assert_eq!(bootstrap, "op-1:1");
    assert_eq!(harness, vec!["sh".to_owned()]);

    // The shim's env: no HERDR_*, no seat name, no adapter session variable; the terminal's
    // own variables stay (the worker finds its seat through them).
    let leaked = [
        "HERDR_PANE_ID",
        "HERDR_SOCKET_PATH",
        "HERDR_THREADS_SEAT_NAME",
        "HC_TERMINAL_ID",
        "HC_TERMINAL_SOCKET",
    ];
    let command =
        launch_gate::prepare(&harness, &config_roots, leaked.map(OsString::from)).unwrap();
    let removed: std::collections::BTreeSet<String> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    for name in [
        "HERDR_PANE_ID",
        "HERDR_SOCKET_PATH",
        "HERDR_THREADS_SEAT_NAME",
    ] {
        assert!(removed.contains(name), "{name} is removed");
    }
    assert!(!removed.contains("HC_TERMINAL_ID") && !removed.contains("HC_TERMINAL_SOCKET"));
    for registration in herdr_threads::harness::registry::builtins().registrations() {
        if let Some(shape) = registration.caller_env() {
            assert!(
                removed.contains(shape.session_var),
                "{} is removed",
                shape.session_var
            );
        }
    }

    // Authorization precedes exec, and the Send is staged before the daemon replies.
    let shim = launch_gate::Gate {
        compound: OperationId::new("op-1"),
        attempt: BootstrapAttempt::new(1).unwrap(),
        socket: gate.fake.socket().to_string_lossy().into_owned(),
        terminal: gate.created.terminal.as_str().to_owned(),
    };
    let core = match &gate.created.witness {
        herdr_threads::ports::HostWitness::HcCore { core_id, .. } => core_id.clone(),
        _ => unreachable!(),
    };
    let clock = clock();
    launch_gate::authorize(
        &InProcessDaemon(&gate),
        &shim,
        &core,
        10_000,
        clock.as_ref(),
    )
    .expect("authorized");
    gate.log.lock().unwrap().push("exec".into());
    assert_eq!(*gate.log.lock().unwrap(), ["staged", "reply", "exec"]);
    assert!(
        gate.polls.load(Ordering::SeqCst) > 3,
        "the gate waited for the Send"
    );
    // The daemon confirmed the terminal with a fresh terminal.get.
    assert!(!gate.fake.calls_to("terminal.get").is_empty());
}

#[test]
fn launch_gate_handler_matches_witness_and_terminal_get() {
    let gate = Gate::new(2);
    let terminal = gate.created.terminal.as_str().to_owned();
    let gets_before = gate.fake.calls_to("terminal.get").len();
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 10_000)),
        LaunchGateReply::Authorized
    );
    assert!(
        gate.fake.calls_to("terminal.get").len() > gets_before,
        "terminal.get confirmed it"
    );
    // Not before the Send is staged: a view that never stages is not authorized.
    let waiting = Gate::new(usize::MAX);
    let terminal = waiting.created.terminal.as_str().to_owned();
    assert_ne!(
        waiting.decide(&waiting.request(&terminal, None, 100)),
        LaunchGateReply::Authorized
    );
}

#[test]
fn launch_gate_mismatch_refused() {
    let refused = |reply: LaunchGateReply, reason| {
        assert_eq!(reply, LaunchGateReply::Refused { reason });
    };
    let gate = Gate::new(0);
    // Another terminal on the right core; another core.
    refused(
        gate.decide(&gate.request("t9", None, 1_000)),
        LaunchGateRefusal::WitnessMismatch,
    );
    let terminal = gate.created.terminal.as_str().to_owned();
    refused(
        gate.decide(&gate.request(&terminal, Some("other-core"), 1_000)),
        LaunchGateRefusal::WitnessMismatch,
    );
    // The creation matches but the core no longer shows the terminal.
    gate.fake.prune(&terminal);
    refused(
        gate.decide(&gate.request(&terminal, None, 1_000)),
        LaunchGateRefusal::WitnessMismatch,
    );
    // A stale attempt and an unknown bootstrap.
    let mut stale = Gate::new(0);
    stale.attempt_current = false;
    let terminal = stale.created.terminal.as_str().to_owned();
    refused(
        stale.decide(&stale.request(&terminal, None, 1_000)),
        LaunchGateRefusal::StaleAttempt,
    );
    let mut unknown = Gate::new(0);
    unknown.known = false;
    refused(
        unknown.decide(&unknown.request("t2", None, 1_000)),
        LaunchGateRefusal::UnknownBootstrap,
    );
}

/// An observer that answers `fail` times with `error` (after `delay`), then delegates to the real
/// observer on the fake core; `calls` counts every call.
struct Scripted {
    fail: usize,
    error: ErrorCode,
    delay: Duration,
    calls: AtomicUsize,
    inner: Arc<dyn BootstrapObserver>,
}
impl BootstrapObserver for Scripted {
    fn observe_bootstrap_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<herdr_threads::ports::BootstrapPaneObservation, ApiError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.fail {
            std::thread::sleep(self.delay);
            return Err(ApiError::new(self.error.clone(), "scripted"));
        }
        self.inner.observe_bootstrap_target(target, context)
    }
}

/// An observer that must never be asked.
struct Never;
impl BootstrapObserver for Never {
    fn observe_bootstrap_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<herdr_threads::ports::BootstrapPaneObservation, ApiError> {
        panic!("a later observer is asked only when the earlier ones could not confirm")
    }
}

fn scripted(gate: &Gate, fail: usize, error: ErrorCode, delay: Duration) -> Arc<Scripted> {
    Arc::new(Scripted {
        fail,
        error,
        delay,
        calls: AtomicUsize::new(0),
        inner: Arc::clone(&gate.observers[0]),
    })
}

fn reason(reply: LaunchGateReply) -> LaunchGateRefusal {
    match reply {
        LaunchGateReply::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn transient_terminal_get_failure_retries_until_authorized() {
    let mut gate = Gate::new(0);
    let flaky = scripted(&gate, 2, ErrorCode::HostUnavailable, Duration::ZERO);
    gate.observers = vec![flaky.clone()];
    let terminal = gate.created.terminal.as_str().to_owned();
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 10_000)),
        LaunchGateReply::Authorized
    );
    assert_eq!(flaky.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn unavailable_until_the_deadline_is_timed_out_never_witness_mismatch() {
    let mut gate = Gate::new(0);
    gate.observers = vec![scripted(
        &gate,
        usize::MAX,
        ErrorCode::HostUnavailable,
        Duration::ZERO,
    )];
    let terminal = gate.created.terminal.as_str().to_owned();
    let started = Instant::now();
    assert_eq!(
        reason(gate.decide(&gate.request(&terminal, None, 200))),
        LaunchGateRefusal::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn not_found_still_refuses_witness_mismatch_without_retrying() {
    let mut gate = Gate::new(0);
    let gone = scripted(&gate, usize::MAX, ErrorCode::NotFound, Duration::ZERO);
    gate.observers = vec![gone.clone()];
    let terminal = gate.created.terminal.as_str().to_owned();
    assert_eq!(
        reason(gate.decide(&gate.request(&terminal, None, 10_000))),
        LaunchGateRefusal::WitnessMismatch
    );
    assert_eq!(gone.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn hung_second_observer_does_not_delay_the_creation_core() {
    let mut gate = Gate::new(0);
    let core = Arc::clone(&gate.observers[0]);
    gate.observers = vec![core, Arc::new(Never)];
    let terminal = gate.created.terminal.as_str().to_owned();
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 10_000)),
        LaunchGateReply::Authorized
    );
}

#[test]
fn deadline_is_rechecked_between_observers() {
    let mut gate = Gate::new(0);
    // The first observer burns the whole gate time and is unavailable; the second must not run.
    gate.observers = vec![
        scripted(
            &gate,
            usize::MAX,
            ErrorCode::HostUnavailable,
            Duration::from_millis(300),
        ),
        Arc::new(Never),
    ];
    let terminal = gate.created.terminal.as_str().to_owned();
    assert_eq!(
        reason(gate.decide(&gate.request(&terminal, None, 100))),
        LaunchGateRefusal::TimedOut
    );
}

// Kills (ht-kqrt.179): the `view.cancelled` refusal removed. The attempt is
// current, the creation matches the shim's terminal and core, and the Send
// is staged and the terminal confirmed, so only the cancelled flag differs
// from the authorized case.
#[test]
fn cancelled_bootstrap_refuses_with_a_matching_attempt_and_staged_send() {
    let mut gate = Gate::new(0);
    let terminal = gate.created.terminal.as_str().to_owned();
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 1_000)),
        LaunchGateReply::Authorized
    );
    gate.cancelled = true;
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 1_000)),
        LaunchGateReply::Refused {
            reason: LaunchGateRefusal::StaleAttempt
        }
    );
}

// Kills (ht-kqrt.179): `witness_matches` checking only the terminal, or only
// the core. Each mismatch alone, with the Send staged and not yet staged:
// the refusal is the mismatch at once, never a wait that times out or an
// authorization.
#[test]
fn witness_mismatch_alone_refuses_staged_or_not() {
    for staged_from in [0, usize::MAX] {
        let gate = Gate::new(staged_from);
        let terminal = gate.created.terminal.as_str().to_owned();
        let mismatch = LaunchGateReply::Refused {
            reason: LaunchGateRefusal::WitnessMismatch,
        };
        // The terminal alone differs.
        assert_eq!(
            gate.decide(&gate.request("t9", None, 1_000)),
            mismatch,
            "staged_from={staged_from}"
        );
        // The core alone differs.
        assert_eq!(
            gate.decide(&gate.request(&terminal, Some("other-core"), 1_000)),
            mismatch,
            "staged_from={staged_from}"
        );
    }
}

#[test]
fn launch_gate_timeout_refuses_shim_exits() {
    let gate = Gate::new(usize::MAX);
    let terminal = gate.created.terminal.as_str().to_owned();
    let started = Instant::now();
    assert_eq!(
        gate.decide(&gate.request(&terminal, None, 150)),
        LaunchGateReply::Refused {
            reason: LaunchGateRefusal::TimedOut
        }
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the wait is bounded"
    );

    // The shim turns the refusal into an error and never execs the harness.
    let marker = temp_path(format!("ht-gate-exec-{}", uuid::Uuid::new_v4().simple()));
    let clock = clock();
    let env = launch_gate::terminal_env(|name| match name {
        "HC_TERMINAL_ID" => Some(terminal.clone().into()),
        "HC_TERMINAL_SOCKET" => Some(gate.fake.socket().into()),
        _ => None,
    });
    let core = match &gate.created.witness {
        herdr_threads::ports::HostWitness::HcCore { core_id, .. } => core_id.clone(),
        _ => unreachable!(),
    };
    let error = launch_gate::run_with(
        &InProcessDaemon(&gate),
        clock.as_ref(),
        &env,
        "op-1:1",
        &[],
        &["touch".to_owned(), marker.to_string_lossy().into_owned()],
        Vec::new(),
        150,
        |_| Ok(core.clone()),
    );
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(!marker.exists(), "the harness never ran");
}

// ---- launched-worker finality (H3, D10) ----

mod launched_finality {
    use super::*;
    use herdr_threads::{
        identity::reconcile::plan_page,
        ports::{
            DurableWorkAdmission, ReconciliationAction, ReconciliationOutcome, SnapshotHeader,
        },
        protocol::time::{Cancellation as Cancel, UtcMillis},
        store::{connection::StoreContext, seats},
        test_support::{snapshot::stage_all, test_host_id},
    };

    const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    /// Publishes a capture of the fake core through the real `HcHost`, plans the saved seats
    /// and applies the planned transitions; returns the planned actions.
    fn settle(
        host: &HcHost,
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
    ) -> Vec<ReconciliationAction> {
        let budget = CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancel::default(),
        };
        let snapshot = host
            .enumerate_targets(&HostCallContext {
                budget: CallBudget {
                    deadline: MonoInstant(10_000),
                    cancellation: Cancel::default(),
                },
                expected_boot: None,
                expected_epoch: None,
            })
            .expect("capture");
        assert!(snapshot.authorizes_absence_closure(), "{snapshot:?}");
        let admission =
            seats::begin_host_observation(context, conn, INSTANCE, test_host_id(), &budget)
                .unwrap();
        let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, &budget).unwrap();
        if !snapshot.targets.is_empty() {
            stage_all(&snapshot.targets, |offset, targets| {
                seats::stage_snapshot_targets(
                    context,
                    conn,
                    &stage.id,
                    offset,
                    targets,
                    DurableWorkAdmission::new(16).unwrap(),
                    &budget,
                )
            })
            .unwrap();
        }
        seats::seal_snapshot_stage(context, conn, &stage.id, &budget).unwrap();
        let published = seats::publish_snapshot_stage(context, conn, &stage.id, &budget).unwrap();
        let page =
            seats::saved_seats_page(context, conn, &published.id, 0, None, 16, &budget).unwrap();
        let transitions = plan_page(&page).unwrap();
        for transition in &transitions {
            let outcome =
                seats::apply_reconciliation_transition(context, conn, transition.clone(), &budget)
                    .unwrap();
            assert!(
                matches!(
                    outcome,
                    ReconciliationOutcome::Applied | ReconciliationOutcome::RetirementStarted(_)
                ),
                "{transition:?}: {outcome:?}"
            );
        }
        transitions.into_iter().map(|t| t.action).collect()
    }

    fn seat_state(conn: &rusqlite::Connection) -> String {
        conn.query_row("SELECT state FROM seats WHERE id='s'", [], |r| r.get(0))
            .unwrap()
    }

    /// R48: a worker launched end to end (`terminal.run` with `keep: false`, the gate shim
    /// authorized, the stand-in exec'd as the terminal's root) that exits finishes its terminal
    /// for good; the core's own record retires the seat through the pane-close fence, resolved
    /// or not, and no launched seat is left unresolved.
    #[test]
    fn launched_worker_exit_retires_seat() {
        let gate = Gate::new(2);
        let terminal = gate.created.terminal.as_str().to_owned();
        let core = match &gate.created.witness {
            herdr_threads::ports::HostWitness::HcCore { core_id, .. } => core_id.clone(),
            _ => unreachable!(),
        };
        assert_eq!(
            run_params(&gate.fake)["keep"],
            serde_json::json!(false),
            "every launch is keep: false"
        );
        // The shim waits for the daemon's authorization, then execs the stand-in: the fake
        // terminal stays live with the harness as its root.
        let shim = launch_gate::Gate {
            compound: OperationId::new("op-1"),
            attempt: BootstrapAttempt::new(1).unwrap(),
            socket: gate.fake.socket().to_string_lossy().into_owned(),
            terminal: terminal.clone(),
        };
        let clock = clock();
        launch_gate::authorize(
            &InProcessDaemon(&gate),
            &shim,
            &core,
            10_000,
            clock.as_ref(),
        )
        .expect("authorized");

        // The launched seat, resolved on the new terminal in the core's boot.
        let boot = format!("hc-core:{core}");
        let path = temp_path(format!("ht-hc-final-{}.db", uuid::Uuid::new_v4().simple()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
        let mut conn = context.open_writer().unwrap();
        conn.execute("UPDATE herdr_hosts SET host_boot=?1,host_epoch=1", [&boot])
            .unwrap();
        let kind = herdr_threads::ports::EvidenceKind::CoherentEnumeration
            .structural_storage_spelling()
            .unwrap();
        conn.execute(
            "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,unavailability_open,home_node,is_local,host_id) VALUES ('s',?1,'resolved',NULL,'native',?4,1,1,?4,?2,?3,?2,1,1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
            rusqlite::params![INSTANCE, boot, kind, terminal],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,?2,?1,1,'claude','plugin_context:x','exec-x','cooperative_top_level',7,8,?2,?1)",
            rusqlite::params![boot, terminal],
        )
        .unwrap();

        let host = HcHost::with_clock(HcClient::new(gate.fake.socket()), Arc::new(FixedClock));
        assert!(
            settle(&host, &context, &mut conn).is_empty(),
            "a running worker changes nothing"
        );
        assert_eq!(seat_state(&conn), "resolved");

        // The worker exits: herdr-core finishes the keep: false terminal for good.
        gate.fake.state().finish(&terminal, "exited", Some(0), None);
        let actions = settle(&host, &context, &mut conn);
        assert!(
            matches!(
                actions.as_slice(),
                [ReconciliationAction::BeginRetirement { .. }]
            ),
            "{actions:?}"
        );
        assert_eq!(seat_state(&conn), "retired");
        let unresolved: i64 = conn
            .query_row(
                "SELECT count(*) FROM seats WHERE state='unresolved'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unresolved, 0, "no launched seat is left unresolved");
        let open: i64 = conn
            .query_row(
                "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

// ---- launch --new-terminal / --new-tab end to end (H3, ht-kqrt.186) ----

mod launch_new {
    use herdr_threads::test_support::{
        isolation::TestIsolation,
        resolve_seeded,
        spawn::{OwnedChild, SpawnOwned},
        spawn_node_daemon,
    };
    use std::{
        path::{Path, PathBuf},
        process::Stdio,
    };

    use super::*;

    const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

    /// What one CLI process printed and how it ended.
    struct Ran {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    }

    /// One real daemon over one fake core whose `terminal.run` really starts its argv, and a
    /// stand-in `claude` that records `<pid> <terminal>` in the marker file and sleeps.
    struct LaunchRig {
        // Drop order: the daemon stops before the core and the isolation under it go.
        daemon: Option<OwnedChild>,
        fake: FakeHc,
        isolation: TestIsolation,
        state: PathBuf,
        path: String,
        marker: PathBuf,
    }

    impl LaunchRig {
        fn start(label: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let isolation = TestIsolation::new(label);
            let fake = FakeHc::start();
            fake.add_terminal(FakeTerminal::live("t1"));
            let dir = temp_path(format!("ht-standin-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&dir).unwrap();
            let exe = dir.join("claude");
            std::fs::write(
                &exe,
                "#!/bin/sh\nprintf '%s\\n' \"$CLAUDE_CONFIG_DIR\" > \"$HT_STANDIN_MARKER.root\"\necho \"$$ $HC_TERMINAL_ID\" > \"$HT_STANDIN_MARKER\"\nexec sleep 300\n",
            )
            .unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            let marker = dir.join("marker");
            fake.exec_runs(&[("HT_STANDIN_MARKER", marker.to_str().unwrap())]);
            let path = format!(
                "{}:{}",
                dir.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            let state =
                std::fs::canonicalize(isolation.state_root()).expect("the state root exists");
            resolve_seeded(&state);
            let daemon = spawn_node_daemon(
                &isolation,
                BIN,
                &state,
                None,
                &isolation.path("daemon.stderr"),
            )
            .expect("spawn the daemon");
            let rig = Self {
                daemon: Some(daemon),
                fake,
                isolation,
                state,
                path,
                marker,
            };
            rig.wait_for_daemon();
            rig
        }

        fn wait_for_daemon(&self) {
            let until = Instant::now() + Duration::from_secs(30);
            while Instant::now() < until {
                if self.plain(&["daemon", "health"]).code == Some(0) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            panic!(
                "the daemon did not come up: {}",
                std::fs::read_to_string(self.isolation.path("daemon.stderr")).unwrap_or_default()
            );
        }

        fn finish(&self, mut command: std::process::Command) -> Ran {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let child: OwnedChild = command.spawn_owned().expect("spawn the process");
            let output = child.wait_with_output().expect("process output");
            Ran {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
        }

        fn command(&self, args: &[&str]) -> std::process::Command {
            let mut command = self.isolation.command(BIN);
            command
                .arg("--state-dir")
                .arg(&self.state)
                .args(args)
                .env("PATH", &self.path);
            command
        }

        fn plain(&self, args: &[&str]) -> Ran {
            self.finish(self.command(args))
        }

        /// `args` as a process in the fake core's terminal `t1`.
        fn run(&self, args: &[&str]) -> Ran {
            self.run_env(args, &[])
        }

        /// `run` with `env` set on the process (the caller's own profile variables).
        fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Ran {
            let mut command = self.command(args);
            command.envs(env.iter().copied());
            command
                .env("HC_TERMINAL_ID", "t1")
                .env("HC_TERMINAL_SOCKET", self.fake.socket());
            self.finish(command)
        }

        /// `args` as the Claude agent of `session` in the fake core's terminal `t1` (the core
        /// environment and nothing of Herdr), with `stdin` as a hook payload when given.
        fn agent(&self, session: &str, args: &[&str], stdin: Option<&[u8]>) -> Ran {
            self.agent_env(session, args, stdin, &[])
        }

        /// `agent` with `env` set on the process (the caller's own profile variables).
        fn agent_env(
            &self,
            session: &str,
            args: &[&str],
            stdin: Option<&[u8]>,
            env: &[(&str, &str)],
        ) -> Ran {
            use std::io::Write;
            let mut command = self.command(args);
            command.envs(env.iter().copied());
            herdr_threads::test_support::isolation::as_agent(&mut command, "claude", session);
            command
                .env("HC_TERMINAL_ID", "t1")
                .env("HC_TERMINAL_SOCKET", self.fake.socket())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command.stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
            let mut child: OwnedChild = command.spawn_owned().expect("spawn the process");
            if let Some(bytes) = stdin {
                child.stdin.take().unwrap().write_all(bytes).unwrap();
            }
            let output = child.wait_with_output().expect("process output");
            Ran {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
        }

        /// The SessionStart hook of `session` in `t1`: enrolls the terminal's seat.
        fn enroll(&self, session: &str) {
            let payload = format!(
                r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}}"#
            );
            let ran = self.agent(session, &["hook", "claude"], Some(payload.as_bytes()));
            assert_eq!(ran.code, Some(0), "{}{}", ran.stdout, ran.stderr);
        }

        /// `args` as an operator in a plain shell (no caller site) that names the fake core.
        fn operator(&self, args: &[&str]) -> Ran {
            let mut command = self.isolation.command(BIN);
            command
                .arg("human")
                .arg("--state-dir")
                .arg(&self.state)
                .arg("--host-endpoint")
                .arg(self.fake.socket())
                .args(args)
                .env("PATH", &self.path);
            self.finish(command)
        }

        /// `operator` with its stdout closed before it can print: it fails after the daemon
        /// answered and before it completes its journaled decision.
        fn operator_without_stdout(&self, args: &[&str]) -> Ran {
            let mut command = self.isolation.command(BIN);
            command
                .arg("human")
                .arg("--state-dir")
                .arg(&self.state)
                .arg("--host-endpoint")
                .arg(self.fake.socket())
                .args(args)
                .env("PATH", &self.path)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child: OwnedChild = command.spawn_owned().expect("spawn the process");
            drop(child.stdout.take());
            let output = child.wait_with_output().expect("process output");
            Ran {
                code: output.status.code(),
                stdout: String::new(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
        }

        /// A `--new-tab worker --space w1` handoff of `sess-a` that fails after its attempt is
        /// journaled; returns the recovery reference the `pending-ops` listing shows for it.
        fn failed_handoff(&self, extra: &[&str]) -> String {
            self.failed_handoff_env(extra, &[])
        }

        /// `failed_handoff` with `env` set on the handoff process.
        fn failed_handoff_env(&self, extra: &[&str], env: &[(&str, &str)]) -> String {
            self.fake
                .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
            self.enroll("sess-a");
            let mut args = vec!["--json", "handoff", "--new-tab", "worker", "--space", "w1"];
            args.extend(extra);
            args.extend(["--kind", "claude", "--", "do the work"]);
            let ran = self.agent_env("sess-a", &args, None, env);
            assert_ne!(ran.code, Some(0), "{}{}", ran.stdout, ran.stderr);
            self.pending_reference()
        }

        fn pending_reference(&self) -> String {
            let ran = self.agent("sess-a", &["--json", "pending-ops"], None);
            assert_eq!(ran.code, Some(0), "{}{}", ran.stdout, ran.stderr);
            let value: serde_json::Value = serde_json::from_str(ran.stdout.trim()).unwrap();
            value["result"]["data"]["items"][0]["recovery_ref"]
                .as_str()
                .unwrap_or_else(|| panic!("no pending reference in {}", ran.stdout))
                .to_owned()
        }

        /// The recovery reference of the newest journaled intent: the operator's decision once
        /// it is recorded (`pending-ops` lists the handoff's own reference, not this one).
        fn latest_intent_reference(&self) -> String {
            fn newest(dir: &std::path::Path, best: &mut u64) {
                for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        newest(&path, best);
                    } else if let Some(ordinal) = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .and_then(|name| name.strip_suffix(".intent"))
                        .and_then(|name| name.split('-').next())
                        .and_then(|ordinal| ordinal.parse::<u64>().ok())
                    {
                        *best = (*best).max(ordinal);
                    }
                }
            }
            let mut best = 0;
            newest(&self.state, &mut best);
            assert_ne!(best, 0, "no journaled intent under {:?}", self.state);
            format!("local:{best}")
        }

        /// The stand-in's `<pid> <terminal>` once the gate let it start.
        fn marker(&self) -> (u32, String) {
            let until = Instant::now() + Duration::from_secs(30);
            while Instant::now() < until {
                if let Ok(text) = std::fs::read_to_string(&self.marker)
                    && let Some((pid, terminal)) = text.trim().split_once(' ')
                {
                    return (pid.parse().unwrap(), terminal.to_owned());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            panic!(
                "the stand-in never started; daemon: {}",
                std::fs::read_to_string(self.isolation.path("daemon.stderr")).unwrap_or_default()
            );
        }

        /// The `CLAUDE_CONFIG_DIR` the stand-in ran under (empty: none), once it started.
        fn root(&self) -> String {
            self.marker();
            std::fs::read_to_string(self.marker.with_extension("root"))
                .expect("the stand-in recorded its root")
                .trim()
                .to_owned()
        }

        /// Models a core autostarted under the profile `root`: its terminals start with it.
        fn core_under(&self, root: &Path) {
            self.fake.exec_runs(&[
                ("HT_STANDIN_MARKER", self.marker.to_str().unwrap()),
                ("CLAUDE_CONFIG_DIR", root.to_str().unwrap()),
            ]);
        }

        fn db(&self) -> rusqlite::Connection {
            let paths = resolve_seeded(&self.state);
            let db = rusqlite::Connection::open(paths.database_path).unwrap();
            db.busy_timeout(Duration::from_secs(5)).unwrap();
            db
        }

        /// The one gate-only record: its id, creation and seat.
        fn gate_row(&self) -> (String, Option<String>, Option<String>) {
            self.db()
                .query_row(
                    "SELECT id, created, seat FROM local_launch_gates",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .expect("one gate-only record")
        }
    }

    impl Drop for LaunchRig {
        fn drop(&mut self) {
            let _ = self.plain(&["daemon", "stop"]);
            self.daemon.take();
        }
    }

    /// The `launch` object of a successful `--json` run.
    fn launched(ran: &Ran) -> serde_json::Value {
        assert_eq!(
            ran.code,
            Some(0),
            "stdout {} stderr {}",
            ran.stdout,
            ran.stderr
        );
        let value: serde_json::Value = serde_json::from_str(ran.stdout.trim()).unwrap();
        value["launch"].clone()
    }

    /// The seat's open binding provenance, and the number of messages, after a launch.
    fn assert_gate_recorded_and_idle(rig: &LaunchRig, report: &serde_json::Value) {
        let (_, created, seat) = rig.gate_row();
        assert!(created.is_some(), "the creation is recorded");
        assert_eq!(seat.as_deref(), report["seat"].as_str(), "the same seat");
        let provenance: String = rig
            .db()
            .query_row(
                "SELECT observation_provenance FROM occupant_bindings WHERE seat_id=?1",
                [seat.as_deref().unwrap()],
                |r| r.get(0),
            )
            .expect("the launched seat has a binding");
        assert_eq!(provenance, "managed_launch");
        let messages: i64 = rig
            .db()
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(messages, 0, "a plain launch stages no first message");
    }

    // A daemon whose only host is a herdr-core has no Herdr primary, so the node-wide automatic
    // handoff capability is off. A handoff that creates its target on the core is gated on the
    // core's own session, not on that flag.
    #[test]
    fn core_only_daemon_hands_off_onto_its_core() {
        let rig = LaunchRig::start("handoff-core-only");
        rig.fake
            .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
        rig.enroll("sess-a");
        let handoff = |placement: &[&str]| {
            let mut args = vec!["--json", "handoff"];
            args.extend(placement);
            args.extend(["--kind", "claude", "--", "do the work"]);
            rig.agent("sess-a", &args, None)
        };
        let ran = handoff(&["--new-tab", "worker", "--space", "w1"]);
        assert_eq!(
            ran.code,
            Some(0),
            "stdout {} stderr {}",
            ran.stdout,
            ran.stderr
        );
        let (_, terminal) = rig.marker();
        assert_ne!(terminal, "t1", "the stand-in runs in the created terminal");
        assert_eq!(run_params(&rig.fake)["labels"]["ht.label"], "worker");
        let ran = handoff(&["--new-terminal"]);
        assert!(
            !ran.stderr
                .contains("daemon lacks automatic handoff capability"),
            "{}",
            ran.stderr
        );
    }

    const NODE_WIDE_REFUSAL: &str = "daemon lacks automatic handoff capability";

    fn assert_routed(ran: &Ran) {
        assert_eq!(
            ran.code,
            Some(0),
            "stdout {} stderr {}",
            ran.stdout,
            ran.stderr
        );
        for refusal in [NODE_WIDE_REFUSAL, "invalid state/host context"] {
            assert!(!ran.stderr.contains(refusal), "{}", ran.stderr);
        }
    }

    // A crashed V3 attempt on a core-only daemon retries from the agent's own terminal: the retry
    // reads the plan's endpoint and gates on the plan's route, not on the node-wide flag.
    #[test]
    fn core_only_crash_retry_completes_a_v3_bootstrap() {
        let rig = LaunchRig::start("handoff-core-retry");
        rig.fake.fail_next_run(RunFailure::Error("busy"));
        let reference = rig.failed_handoff(&[]);
        let ran = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_routed(&ran);
        let (_, terminal) = rig.marker();
        assert_ne!(terminal, "t1", "the stand-in runs in the created terminal");
    }

    // A possibly-created attempt stays refused until an operator decides; the decision is served
    // by the core-only daemon, and the agent's retry then completes as the next attempt.
    #[test]
    fn core_only_operator_recovers_a_stuck_v3_attempt() {
        let rig = LaunchRig::start("handoff-core-recover");
        rig.fake.fail_next_run(RunFailure::Disconnect);
        let reference = rig.failed_handoff(&[]);
        let refused = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_ne!(refused.code, Some(0), "a possible creation stays refused");
        let ran = rig.operator(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--not-created",
        ]);
        assert_routed(&ran);
        let ran = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_routed(&ran);
        rig.marker();
    }

    #[test]
    fn core_only_operator_cancels_a_stuck_v3_attempt() {
        let rig = LaunchRig::start("handoff-core-cancel");
        rig.fake.fail_next_run(RunFailure::Disconnect);
        let reference = rig.failed_handoff(&[]);
        let ran = rig.operator(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--cancel",
            "--reason",
            "stuck",
        ]);
        assert_routed(&ran);
        let cancelled: i64 = rig
            .db()
            .query_row(
                "SELECT count(*) FROM automatic_bootstrap_parents WHERE state='cancelled'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cancelled, 1);
    }

    // A lost reply leaves a really created terminal. The operator keeps it: the recovery reads
    // it through the core's own observer, never as a Herdr pane, and creates no second one.
    #[test]
    fn core_only_operator_keeps_a_really_created_terminal() {
        let rig = LaunchRig::start("handoff-core-keep");
        rig.fake.fail_next_run(RunFailure::Disconnect);
        let reference = rig.failed_handoff(&[]);
        let ran = rig.operator(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--created-pane",
            "t2",
        ]);
        assert_routed(&ran);
        // The decision keeps the creation; the agent's retry then launches into it.
        let ran = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_routed(&ran);
        let (_, terminal) = rig.marker();
        assert_eq!(terminal, "t2", "the stand-in runs in the kept terminal");
        let herdr: Vec<String> = rig
            .fake
            .calls()
            .into_iter()
            .map(|call| call.method)
            .filter(|method| method == "ping")
            .collect();
        assert!(herdr.is_empty(), "the core was pinged as Herdr: {herdr:?}");
        assert_eq!(rig.fake.calls_to("terminal.run").len(), 1, "one creation");
        assert!(rig.fake.terminal("t2").is_some());
        assert!(rig.fake.terminal("t3").is_none(), "no second terminal");
    }

    // An asserted terminal the core does not hold is refused before any state moves, and the
    // operator's other decisions still work.
    #[test]
    fn core_only_created_pane_of_an_absent_terminal_refuses() {
        let rig = LaunchRig::start("handoff-core-keep-absent");
        rig.fake.fail_next_run(RunFailure::Disconnect);
        let reference = rig.failed_handoff(&[]);
        let refused = rig.operator(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--created-pane",
            "t9",
        ]);
        assert_ne!(
            refused.code,
            Some(0),
            "{}{}",
            refused.stdout,
            refused.stderr
        );
        assert!(
            rig.fake.calls().iter().all(|call| call.method != "ping"),
            "the core was pinged as Herdr"
        );
        let ran = rig.operator(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--not-created",
        ]);
        assert_routed(&ran);
    }

    // The operator's own decision is a journaled V3 record: its retry routes by the plan too.
    // The retry names the decision's own recovery reference, so it reaches the V3 route gate
    // instead of dying on the caller-site refusal of the handoff's reference.
    #[test]
    fn core_only_operator_retry_of_a_recovery_routes_too() {
        let rig = LaunchRig::start("handoff-core-recover-retry");
        rig.fake.fail_next_run(RunFailure::Disconnect);
        let reference = rig.failed_handoff(&[]);
        // The daemon applies the decision, but the operator's output has nowhere to go, so the
        // decision stays journaled as pending.
        let ran = rig.operator_without_stdout(&[
            "handoff",
            "recover",
            &reference,
            "--attempt",
            "1",
            "--not-created",
        ]);
        assert_ne!(ran.code, Some(0), "{}", ran.stderr);
        let decision = rig.latest_intent_reference();
        assert_ne!(decision, reference, "the decision has its own reference");
        let ran = rig.operator(&["retry", &decision]);
        assert_routed(&ran);
    }

    // A profile directory with a placement target needs no node-wide directory capability.
    #[test]
    fn core_only_profile_directory_with_a_target() {
        let rig = LaunchRig::start("handoff-core-profile");
        rig.fake
            .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
        rig.enroll("sess-a");
        let dir = temp_path(format!("ht-profile-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let ran = rig.agent(
            "sess-a",
            &[
                "--json",
                "handoff",
                "--new-tab",
                "worker",
                "--space",
                "w1",
                "--profile-dir",
                dir.to_str().unwrap(),
                "--kind",
                "claude",
                "--",
                "do the work",
            ],
            None,
        );
        assert!(
            !ran.stderr
                .contains("daemon lacks handoff launch directories capability"),
            "{}",
            ran.stderr
        );
        assert_routed(&ran);
    }

    #[test]
    fn core_only_crash_retry_with_a_profile_directory() {
        let rig = LaunchRig::start("handoff-core-profile-retry");
        let dir = temp_path(format!("ht-profile-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        rig.fake.fail_next_run(RunFailure::Error("busy"));
        let reference = rig.failed_handoff(&["--profile-dir", dir.to_str().unwrap()]);
        let ran = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_routed(&ran);
    }

    // Kills: a V3 handoff whose gate runs under the core's ambient profile instead of the
    // handing-off caller's.
    #[test]
    fn core_only_handoff_runs_under_the_callers_root() {
        let rig = LaunchRig::start("handoff-root-caller");
        let (a, b) = (profile("core-a"), profile("caller-b"));
        rig.core_under(&a);
        rig.fake
            .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
        rig.enroll("sess-a");
        let ran = rig.agent_env(
            "sess-a",
            &[
                "--json",
                "handoff",
                "--new-tab",
                "worker",
                "--space",
                "w1",
                "--kind",
                "claude",
                "--",
                "do the work",
            ],
            None,
            &[("CLAUDE_CONFIG_DIR", b.to_str().unwrap())],
        );
        assert_routed(&ran);
        assert_eq!(rig.root(), b.to_str().unwrap());
    }

    // Kills: a retry that re-reads the environment of whoever retries (the plan must freeze
    // the caller's root) or is refused by the plan check for differing from the first attempt.
    #[test]
    fn core_only_retry_keeps_the_frozen_root() {
        let rig = LaunchRig::start("handoff-root-retry");
        let (a, b) = (profile("core-a"), profile("caller-b"));
        rig.core_under(&a);
        rig.fake.fail_next_run(RunFailure::Error("busy"));
        let reference = rig.failed_handoff_env(&[], &[("CLAUDE_CONFIG_DIR", b.to_str().unwrap())]);
        let ran = rig.agent("sess-a", &["--json", "retry", &reference], None);
        assert_routed(&ran);
        assert_eq!(rig.root(), b.to_str().unwrap());
    }

    // Pins `runtime()`: a verified two-family terminal (its core an ancestor, a Herdr pane in
    // its environment, no Herdr socket) hands off onto its core; the context resolves as the
    // terminal site, not as a pane that needs a Herdr server.
    #[test]
    fn two_family_terminal_hands_off_onto_its_core() {
        let rig = LaunchRig::start("handoff-two-family");
        rig.fake.set_pid("t1", Some(std::process::id()));
        rig.fake
            .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
        rig.enroll("sess-a");
        let mut command = rig.command(&[
            "--json",
            "handoff",
            "--new-tab",
            "worker",
            "--space",
            "w1",
            "--kind",
            "claude",
            "--",
            "do the work",
        ]);
        herdr_threads::test_support::isolation::as_agent(&mut command, "claude", "sess-a");
        command
            .env("HC_TERMINAL_ID", "t1")
            .env("HC_TERMINAL_SOCKET", rig.fake.socket())
            .env("HERDR_PANE_ID", "w9:p9")
            .env("HERDR_ENV", "1")
            .env_remove("HERDR_SOCKET_PATH");
        let ran = rig.finish(command);
        assert_eq!(ran.code, Some(0), "{}{}", ran.stdout, ran.stderr);
    }

    #[test]
    fn launch_new_terminal_runs_the_harness_through_the_gate() {
        let rig = LaunchRig::start("launch-new-terminal");
        let ran = rig.run(&["--json", "launch", "--new-terminal", "--kind", "claude"]);
        let report = launched(&ran);
        assert!(report["seat"].as_str().is_some_and(|seat| !seat.is_empty()));
        assert_eq!(report["target"], "t2");
        let (pid, terminal) = rig.marker();
        assert_eq!(terminal, "t2", "the harness runs in the created terminal");
        assert_eq!(
            rig.fake.terminal("t2").and_then(|t| t.pid),
            Some(pid),
            "exec kept the terminal's process"
        );
        let params = run_params(&rig.fake);
        assert_eq!(params["parent"], "t1");
        let labels = params["labels"].as_object().expect("labels");
        assert!(!labels.contains_key("herdr.space"), "{labels:?}");
        let argv = params["argv"].to_string();
        assert!(
            argv.contains("--state-dir") && argv.contains("--bootstrap"),
            "{argv}"
        );
        assert_gate_recorded_and_idle(&rig, &report);
    }

    /// A real scratch profile directory.
    fn profile(tag: &str) -> PathBuf {
        let dir = temp_path(format!("ht-{tag}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    // Kills: a launch that runs under the autostarted core's profile (A) when the launching
    // caller's own profile (B) differs: the gate argv carried no root and nothing scrubbed A.
    #[test]
    fn launch_new_terminal_runs_under_the_callers_root_not_the_cores() {
        let rig = LaunchRig::start("launch-root-caller");
        let (a, b) = (profile("core-a"), profile("caller-b"));
        rig.core_under(&a);
        let ran = rig.run_env(
            &["--json", "launch", "--new-terminal", "--kind", "claude"],
            &[("CLAUDE_CONFIG_DIR", b.to_str().unwrap())],
        );
        launched(&ran);
        assert_eq!(rig.root(), b.to_str().unwrap());
    }

    // Kills: the core's ambient root surviving a launch from a caller that names none.
    #[test]
    fn launch_new_terminal_with_no_caller_root_leaves_the_core_root_behind() {
        let rig = LaunchRig::start("launch-root-none");
        rig.core_under(&profile("core-a"));
        let ran = rig.run(&["--json", "launch", "--new-terminal", "--kind", "claude"]);
        launched(&ran);
        assert_eq!(rig.root(), "");
    }

    // Kills: a daemon that answers a repeated `Record` with the seat although the record is not
    // open (the `local_launch_gate_record` refusal removed): the launch ran to completion, so
    // its row is closed, and the same creation sent again must change nothing.
    #[test]
    fn local_gate_record_refuses_a_record_that_is_not_open() {
        use herdr_threads::{
            client::local::LocalSocketClient,
            daemon::ownership::read_descriptor,
            node::json::read_hint,
            ports::LocalClient,
            protocol::{
                authority::Harness,
                handoff::{LocalLaunchGate, LocalLaunchGateAction},
                ids::HostLocator,
            },
        };
        let rig = LaunchRig::start("launch-record-closed");
        launched(&rig.run(&["--json", "launch", "--new-terminal", "--kind", "claude"]));
        let row = rig.gate_row();
        let (id, created, _) = row.clone();
        let created: CreatedTarget =
            serde_json::from_str(created.as_deref().expect("the creation is recorded")).unwrap();

        let paths = resolve_seeded(&rig.state);
        let node = read_hint(&paths.node_json_path).expect("node.json");
        let descriptor = read_descriptor(&paths, node.node_uuid).expect("the daemon's descriptor");
        let clock = clock();
        let client = LocalSocketClient::new(
            descriptor.endpoint.clone(),
            Arc::clone(&clock),
            descriptor.node_uuid,
            Some(descriptor.boot_id),
        );
        let refused = client
            .call(
                Command::LocalLaunchGate(Box::new(LocalLaunchGate {
                    id: OperationId::new(id),
                    host: HostLocator::parse(rig.fake.socket().to_string_lossy().into_owned())
                        .unwrap(),
                    action: LocalLaunchGateAction::Record {
                        harness: Harness::Claude,
                        created: Box::new(created),
                    },
                })),
                &budget(clock.as_ref(), 10_000),
            )
            .expect_err("a closed record cannot be recorded again");
        assert_eq!(refused.code, ErrorCode::Conflict);
        assert!(
            refused.detail.contains("launch gate record is not open"),
            "{}",
            refused.detail
        );
        assert_eq!(rig.gate_row(), row, "the row is unchanged");
    }

    #[test]
    fn launch_new_tab_runs_the_harness_through_the_gate() {
        let rig = LaunchRig::start("launch-new-tab");
        rig.fake
            .set_labels("t1", &[("herdr.space", "w1"), ("herdr.tab", "x")]);
        let ran = rig.run(&[
            "--json",
            "launch",
            "--new-tab",
            "worker",
            "--space",
            "w1",
            "--kind",
            "claude",
        ]);
        let report = launched(&ran);
        let (_, terminal) = rig.marker();
        assert_eq!(terminal, report["target"].as_str().unwrap());
        let params = run_params(&rig.fake);
        let labels = params["labels"].as_object().expect("labels");
        assert_eq!(labels["herdr.space"], "w1");
        assert!(!labels["herdr.tab"].as_str().unwrap().is_empty());
        assert_eq!(labels["ht.label"], "worker");
        assert_gate_recorded_and_idle(&rig, &report);
    }

    #[test]
    fn gate_without_a_record_refuses_and_starts_nothing() {
        let rig = LaunchRig::start("launch-no-record");
        let bootstrap = format!("{}:1", uuid::Uuid::new_v4());
        let ran = rig.run(&[
            "internal",
            "launch-gate",
            "--bootstrap",
            &bootstrap,
            "--",
            "claude",
        ]);
        assert_ne!(ran.code, Some(0));
        assert!(
            ran.stderr
                .contains("launch gate refused: unknown bootstrap"),
            "{}",
            ran.stderr
        );
        assert!(!rig.marker.exists(), "no harness started");
    }

    #[test]
    fn gate_of_a_foreign_terminal_refuses() {
        let rig = LaunchRig::start("launch-foreign");
        let report =
            launched(&rig.run(&["--json", "launch", "--new-terminal", "--kind", "claude"]));
        let (before, _) = rig.marker();
        let (id, _, _) = rig.gate_row();
        let bootstrap = format!("{id}:1");
        let mut command = rig.command(&[
            "internal",
            "launch-gate",
            "--bootstrap",
            &bootstrap,
            "--",
            "claude",
        ]);
        command
            .env("HC_TERMINAL_ID", "t99")
            .env("HC_TERMINAL_SOCKET", rig.fake.socket());
        let ran = rig.finish(command);
        assert_ne!(ran.code, Some(0), "{report}");
        assert!(
            ran.stderr
                .contains("terminal is not the bootstrap's recipient"),
            "{}",
            ran.stderr
        );
        assert_eq!(rig.marker().0, before, "the marker is unchanged");
    }

    #[test]
    fn launch_onto_a_core_refuses_binary_and_name() {
        let rig = LaunchRig::start("launch-refuse");
        for extra in [["--name", "n"], ["--harness-binary", "/bin/sh"]] {
            let mut args = vec!["launch", "--new-terminal", "--kind", "claude"];
            args.extend(extra);
            let ran = rig.run(&args);
            assert_ne!(ran.code, Some(0));
            assert!(
                ran.stderr.contains(
                    "--harness-binary and --name are not supported for a launch onto a herdr-core"
                ),
                "{}",
                ran.stderr
            );
        }
        assert!(rig.fake.calls_to("terminal.run").is_empty());
    }
}
