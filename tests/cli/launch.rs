//! Managed launch composition through fake host, seat and handoff ports with
//! the real setup inspection and synthetic harness binaries. Each test names
//! the mutation it kills.

use super::*;
use crate::ports::{
    CorrelatedStartup, EvidenceKind, ExecutionEvidence, HostCallContext, HostLifecycleSubscription,
    HostSnapshot, HostUiState, IncarnationEvidence, NativeLaunchCapability, NativeLaunchRequest,
    ObservationProvenance, PromptOutcome, SafeWakeTarget, StructuralOccupancy,
};
use crate::protocol::{
    ids::{HostBootId, HostCallId, TerminalId},
    time::UtcMillis,
};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

struct Clock0(AtomicU64);
impl Clock for Clock0 {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

struct FakeHost {
    sequence: AtomicU64,
    occupancy: StructuralOccupancy,
    submitted: Mutex<Vec<NativeLaunchRequest>>,
    unknown: bool,
}
impl FakeHost {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(1),
            occupancy: StructuralOccupancy::Unknown,
            submitted: Mutex::new(Vec::new()),
            unknown: false,
        }
    }
    fn observation(&self) -> HostObservation {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        HostObservation {
            target: HostTargetId::new("w9:p1"),
            host_boot: HostBootId::new("boot"),
            epoch: 1,
            generation: 1,
            observed_at_utc: UtcMillis(0),
            observed_at_mono: MonoInstant(sequence),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            terminal: Some(TerminalId::new("term_9")),
            occupancy: self.occupancy,
            incarnation: IncarnationEvidence::Verified {
                identity: "herdr-server:pid=1".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: MonoInstant(sequence),
            completed_at_mono: MonoInstant(sequence),
        }
    }
    fn submitted(&self) -> Vec<NativeLaunchRequest> {
        self.submitted.lock().unwrap().clone()
    }
}
impl HostPort for FakeHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::HostGuardedStart
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        assert_eq!(target.as_str(), "w9:p1");
        Ok(self.observation())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn subscribe_lifecycle(
        &self,
        _: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
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
        panic!("launch must never prompt")
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
}

struct FakeSeats {
    calls: AtomicUsize,
    held: bool,
}
impl LaunchSeatResolver for FakeSeats {
    fn resolve_for_launch(
        &self,
        _: &HostTargetId,
        _: &HostObservation,
        _: &CallBudget,
    ) -> Result<SeatId, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.held {
            return Err(api(ErrorCode::TargetUnresolved, "recovery hold"));
        }
        Ok(SeatId::new("seat_launch"))
    }
}

/// Answers with a fixed pending handoff and counts reads (never mutations).
struct FakeHandoff(AtomicUsize);
impl HandoffReader for FakeHandoff {
    fn pending(&self, seat: &SeatId, _: &CallBudget) -> Result<Value, ApiError> {
        assert_eq!(seat.as_str(), "seat_launch");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"threads":["thread_1"],"pending_invitations":1,"pending_receipts":1}))
    }
}

/// A canned pane-shell answer for `codex`; never runs a shell.
struct FakeProbe(Result<String, String>);
impl CodexShellProbe for FakeProbe {
    fn resolve_codex(&self) -> Result<String, String> {
        self.0.clone()
    }
}

struct Scratch {
    root: std::path::PathBuf,
    env: SetupEnv,
}
impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ht-launch-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = root.canonicalize().unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(root.join("state"))
            .unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        let env = SetupEnv {
            executable: root.join("h t/herdr-threads"),
            state_dir: Some(root.join("state")),
            cwd: root.clone(),
            path: Some(root.join("bin").into_os_string()),
            codex_home: Some(root.join("codex home")),
            claude_config_dir: Some(root.join("claude config")),
            host_endpoint: Some(root.join("herdr.sock")),
            instance_source: Value::Null,
        };
        Self { root, env }
    }
    fn harness(&self, name: &str, version_line: &str, tail: &[u8]) {
        let mut bytes = format!("#!/bin/sh\nprintf '{version_line}\\n'\nexit 0\n").into_bytes();
        bytes.extend_from_slice(tail);
        let path = self.root.join("bin").join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn setup(&self, harness: ContextHarness) -> Value {
        setup::execute(
            &SetupRequest {
                verb: SetupVerb::Install,
                harness,
                harness_binary: None,
            },
            &self.env,
        )
        .expect("setup")
    }
    fn setup_claude(&self) {
        self.setup(ContextHarness::Claude);
    }
    fn launch(
        &self,
        host: &FakeHost,
        seats: &FakeSeats,
        handoff: &FakeHandoff,
        request: LaunchRequest,
    ) -> Result<LaunchReport, RunError> {
        let probe = FakeProbe(Ok("codex is /usr/local/bin/codex\n".into()));
        self.launch_with_probe(host, seats, handoff, request, &probe)
    }
    fn launch_with_probe(
        &self,
        host: &FakeHost,
        seats: &FakeSeats,
        handoff: &FakeHandoff,
        request: LaunchRequest,
        probe: &dyn CodexShellProbe,
    ) -> Result<LaunchReport, RunError> {
        let clock = Clock0(AtomicU64::new(1));
        execute(
            &request,
            &LaunchParts {
                env: &self.env,
                host,
                seats,
                handoff,
                clock: &clock,
                record_dir: Some(&self.root),
                shell_probe: probe,
            },
        )
    }
    fn records(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("launches.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn seats() -> FakeSeats {
    FakeSeats {
        calls: AtomicUsize::new(0),
        held: false,
    }
}
fn handoff() -> FakeHandoff {
    FakeHandoff(AtomicUsize::new(0))
}
fn request(harness: ContextHarness, argv: &[&str]) -> LaunchRequest {
    LaunchRequest {
        target: HostTargetId::new("w9:p1"),
        harness,
        harness_binary: None,
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        name: None,
        pane_label: None,
    }
}
fn code(result: Result<LaunchReport, RunError>) -> ErrorCode {
    match result {
        Err(RunError::Api(error)) => error.code,
        other => panic!("expected an API refusal, got {other:?}"),
    }
}

/// The committed Codex 0.158.0 hook schema extraction, concatenated (what a
/// schema-matched unlisted binary embeds).
fn committed_codex_schemas() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs/evidence/codex-158-hook-capture/schemas-0.158.0");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".schema.json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        out.extend_from_slice(&fs::read(file).unwrap());
        out.push(b'\n');
    }
    out
}

/// Claude end to end through the composition: gate, resolved seat,
/// installed owned user-level hook, one guarded start with the caller's
/// arguments unchanged (Claude hooks load from its user settings, so nothing
/// is added), a record, and a report that is explicitly not receipt and points
/// at the durable handoff. Kills: dropping or reordering caller arguments,
/// adding an auto-approve flag, skipping the record, and reporting launch
/// as acceptance.
#[test]
fn claude_launch_starts_with_caller_argv_and_reports_durable_handoff() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let caller = [
        "--model",
        "haiku",
        "--permission-mode",
        "default",
        "é \"q\"",
    ];
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &caller),
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    let submitted = host.submitted();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].argv, caller);
    assert_eq!(submitted[0].harness, Harness::Claude);
    assert_eq!(submitted[0].configured_hook.scope, "user");
    assert_eq!(submitted[0].seat.as_str(), "seat_launch");
    assert_eq!(seats.calls.load(Ordering::SeqCst), 1);
    assert_eq!(out.report["outcome"], "started");
    assert_eq!(out.report["seat"], "seat_launch");
    assert_eq!(out.report["handoff"]["pending_invitations"], 1);
    assert!(
        out.report["receipt"]
            .as_str()
            .unwrap()
            .contains("not receipt")
    );
    assert!(
        out.report["next"]
            .as_str()
            .unwrap()
            .contains("inbox --seat seat_launch")
    );
    let records = s.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["outcome"], "started");
    assert_eq!(records[0]["argv"], json!(caller));
    assert_eq!(records[0]["seat"], "seat_launch");
}

/// Kills: launching without the owned installation (a missing hook must
/// refuse with the setup command, before any start).
#[test]
fn claude_without_owned_hooks_refuses_with_setup_hint() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let result = s.launch(
        &host,
        &seats,
        &handoff,
        request(ContextHarness::Claude, &[]),
    );
    match result {
        Err(RunError::Api(error)) => {
            assert_eq!(error.code, ErrorCode::MissingHook);
            assert!(error.detail.contains("herdr-threads setup claude"));
            assert!(error.detail.contains("settings.json"));
        }
        other => panic!("expected missing hook, got {other:?}"),
    }
    assert!(host.submitted().is_empty());
    assert!(s.records().is_empty());
}

/// Claude loads user hooks from the settings of its own CLAUDE_CONFIG_DIR.
/// Kills: accepting an installation recorded for another config directory.
#[test]
fn claude_launch_inspects_the_resolved_config_dir_only() {
    let mut s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    s.env.claude_config_dir = Some(s.root.join("other config"));
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    assert_eq!(
        code(s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[])
        )),
        ErrorCode::MissingHook
    );
    assert!(host.submitted().is_empty());
}

/// Kills: skipping the installed-version recipe gate (nothing is observed,
/// resolved or started for an uncovered version).
#[test]
fn uncovered_harness_version_refuses_before_any_host_or_seat_call() {
    let s = Scratch::new();
    s.harness("claude", "9.9.9 (Claude Code)", b"");
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    assert_eq!(
        code(s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[]),
        )),
        ErrorCode::UnsupportedHarness
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.sequence.load(Ordering::SeqCst), 1);
}

/// A recovery-held target refuses through the ordinary resolver. Kills:
/// allocating around a hold or starting before resolution.
#[test]
fn held_target_refuses_without_start() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let (host, handoff) = (FakeHost::new(), handoff());
    let held = FakeSeats {
        calls: AtomicUsize::new(0),
        held: true,
    };
    assert_eq!(
        code(s.launch(&host, &held, &handoff, request(ContextHarness::Claude, &[]),)),
        ErrorCode::TargetUnresolved
    );
    assert!(host.submitted().is_empty());
}

/// An occupied pane refuses before seat resolution. Kills: typing into an
/// occupant.
#[test]
fn occupied_pane_refuses_before_seat_resolution() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let mut host = FakeHost::new();
    host.occupancy = StructuralOccupancy::Occupied;
    let (seats, handoff) = (seats(), handoff());
    assert_eq!(
        code(s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[]),
        )),
        ErrorCode::TargetUnsafe
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
}

/// A possible start that was not confirmed exits 5, is recorded as unknown
/// and still reports the pending handoff (the lost-initial-prompt case: the
/// invitation and message stay discoverable). Kills: treating an unknown
/// outcome as success or as a safe retry, and hiding the handoff.
#[test]
fn unknown_outcome_exits_five_and_keeps_handoff_discoverable() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let mut host = FakeHost::new();
    host.unknown = true;
    let (seats, handoff) = (seats(), handoff());
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[]),
        )
        .unwrap();
    assert_eq!(out.exit, 5);
    assert_eq!(out.report["outcome"], "outcome_unknown");
    assert_eq!(out.report["handoff"]["pending_invitations"], 1);
    assert!(
        out.report["note"]
            .as_str()
            .unwrap()
            .contains("before launching again")
    );
    assert_eq!(handoff.0.load(Ordering::SeqCst), 1);
    assert_eq!(s.records()[0]["outcome"], "outcome_unknown");
    assert_eq!(host.submitted().len(), 1);
}

/// Codex on a version whose sandbox default-deny was measured, set up at
/// user level: the hooks and the socket allowance are on disk, so launch
/// adds no hook or sandbox arguments, keeps the caller's unchanged and adds
/// `--no-daemon` exactly once. Kills: launching without the installation or
/// the allowance, reordering caller arguments, and duplicating `--no-daemon`.
#[test]
fn codex_launch_needs_the_user_installation_and_adds_only_no_daemon() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let caller = ["--model", "gpt-test", "-a", "on-request"];
    assert_eq!(
        code(s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &caller)
        )),
        ErrorCode::MissingHook
    );
    let report = s.setup(ContextHarness::Codex);
    assert_eq!(report["sandbox"]["present"], true, "{report}");
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &caller),
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    let argv = &host.submitted()[0].argv;
    assert_eq!(argv[0], "--no-daemon");
    assert_eq!(&argv[1..], caller);
    assert!(!argv.iter().any(|a| a.contains("bypass") || a == "--yolo"));
    assert_eq!(host.submitted()[0].configured_hook.scope, "user");

    // A caller that already passes --no-daemon keeps exactly one.
    let host = FakeHost::new();
    s.launch(
        &host,
        &seats,
        &handoff,
        request(ContextHarness::Codex, &["--no-daemon"]),
    )
    .unwrap();
    assert_eq!(host.submitted()[0].argv, ["--no-daemon"]);

    // The allowance removed by hand: the default sandbox could not reach
    // the daemon, so launch refuses.
    let config = s.env.codex_home.clone().unwrap().join("config.toml");
    let text = fs::read_to_string(&config).unwrap();
    fs::write(&config, text.replace("\"allow\"", "\"deny\"")).unwrap();
    let host = FakeHost::new();
    assert_eq!(
        code(s.launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))),
        ErrorCode::Unsupported
    );
    assert!(host.submitted().is_empty());
}

/// `codex exec` (native-codex-matrix-1 P4/P5): `--no-daemon` precedes
/// `exec` and the caller's arguments follow unchanged. Kills: `--no-daemon`
/// after the subcommand or the prompt, and starting an unconfigurable
/// subcommand.
#[test]
fn codex_exec_launch_keeps_no_daemon_before_exec() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let caller = ["exec", "--json", "-s", "read-only", "PROMPT"];
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &caller),
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    let argv = &host.submitted()[0].argv;
    assert_eq!(argv[0], "--no-daemon");
    assert_eq!(&argv[1..], caller);

    // An unconfigurable subcommand is refused and nothing is started.
    let host = FakeHost::new();
    assert_eq!(
        code(s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &["fork", "ID"])
        )),
        ErrorCode::InvalidRequest
    );
    assert!(host.submitted().is_empty());
}

/// A cold Codex fingerprint scan (0.159.2 is admitted only by its embedded
/// hook schemas) is persisted by managed launch, which is not bound by the
/// hook's time budget, into the hook's private cache `<state>/harness`: the
/// hook's next observation of the same binary is a cache hit, not a rescan.
/// Kills: launch observing through the in-memory cache only, so a slow
/// machine's hook rescans (and refuses) forever.
#[test]
fn codex_launch_warms_the_hook_fingerprint_cache() {
    use crate::harness::{codex, codex_evidence, codex_schema};
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    let binary = s.root.join("bin/codex");
    let state = s.env.state_dir.clone().unwrap();
    s.setup(ContextHarness::Codex);
    codex_schema::forget_memory_entry_for_test(&binary);
    let cache = codex_evidence::cache_path(&codex_evidence::dir(&state));
    assert!(!cache.exists(), "cache starts cold");
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let out = s
        .launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))
        .unwrap();
    assert_eq!(out.exit, 0);
    assert_eq!(
        fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Mark the persisted entry so only a cache hit can report it, then take
    // the hook's view: a fresh process (no in-memory entry) reading the file.
    let mut stored: Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    let entries = stored["entries"].as_array_mut().unwrap();
    assert_eq!(entries.len(), 1, "{stored}");
    let sentinel = "a".repeat(64);
    entries[0]["binary_sha256"] = Value::String(sentinel.clone());
    fs::write(&cache, serde_json::to_vec(&stored).unwrap()).unwrap();
    codex_schema::forget_memory_entry_for_test(&binary);
    let hook = codex::InstalledAdmission::observe_binary(
        binary,
        codex::VERSION_TIMEOUT,
        codex_schema::FingerprintCache::ReadWrite(&cache),
    );
    match hook.result.unwrap().admission() {
        codex::Admission::SchemaMatched { binary_sha256, .. } => {
            assert_eq!(*binary_sha256, sentinel, "hook rescanned the binary")
        }
        other => panic!("expected a schema-matched admission, got {other:?}"),
    }
}

/// On a version without a measured sandbox default-deny setup withholds the
/// allowance, so the default sandbox could not reach the daemon: launch
/// refuses unless the caller explicitly chose full access. Kills: silently
/// launching an agent that cannot run herdr-threads commands.
#[test]
fn codex_without_measured_allowance_needs_explicit_full_access() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0", b"");
    let report = s.setup(ContextHarness::Codex);
    assert!(report["sandbox"]["socket_path"].is_null(), "{report}");
    assert!(
        !s.env
            .codex_home
            .clone()
            .unwrap()
            .join("config.toml")
            .exists()
    );
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    assert_eq!(
        code(s.launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))),
        ErrorCode::Unsupported
    );
    assert!(host.submitted().is_empty());
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &["-s", "danger-full-access"]),
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    assert_eq!(
        host.submitted()[0].argv,
        ["--no-daemon", "-s", "danger-full-access"]
    );
}

/// Kills: letting a caller `-c hooks.*` override silently replace the owned
/// hook, and rewriting an explicit daemon mode.
#[test]
fn codex_conflicting_caller_arguments_refuse_before_seat() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    for argv in [
        &["-c", "hooks.SessionStart=[]"][..],
        &["--config=hooks.PreToolUse=[]"][..],
        &["-chooks.SubagentStart=[]"][..],
        &["--daemon"][..],
    ] {
        assert_eq!(
            code(s.launch(
                &host,
                &seats,
                &handoff,
                request(ContextHarness::Codex, argv),
            )),
            ErrorCode::InvalidRequest,
            "{argv:?}"
        );
    }
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
}

/// Kills: treating the default or a restrictive sandbox as not needing the
/// allowance, and reading the prompt after `--` as an option.
#[test]
fn sandbox_allowance_is_unneeded_only_for_explicit_full_access() {
    let args = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert!(codex_sandbox_needs_allowance(&args(&[])));
    assert!(codex_sandbox_needs_allowance(&args(&[
        "-s",
        "workspace-write"
    ])));
    assert!(codex_sandbox_needs_allowance(&args(&[
        "-s",
        "danger-full-access",
        "-s",
        "read-only"
    ])));
    assert!(!codex_sandbox_needs_allowance(&args(&[
        "-s",
        "danger-full-access"
    ])));
    assert!(!codex_sandbox_needs_allowance(&args(&[
        "--sandbox=danger-full-access"
    ])));
    assert!(!codex_sandbox_needs_allowance(&args(&[
        "-c",
        "sandbox_mode=\"danger-full-access\""
    ])));
    assert!(!codex_sandbox_needs_allowance(&args(&[
        "--dangerously-bypass-approvals-and-sandbox"
    ])));
    assert!(codex_sandbox_needs_allowance(&args(&[
        "--",
        "-s",
        "danger-full-access"
    ])));
}

/// The public form is `launch --pane PANE --kind claude|codex
/// [-- AGENT_ARG...]`. Kills: losing or reordering arguments after `--`
/// (including ones that look like herdr-threads flags), accepting the
/// removed `--project`, and defaulting the pane.
#[test]
fn launch_syntax_keeps_agent_arguments_verbatim() {
    use crate::cli::commands::{CliAction, parse_argv};
    let parsed = parse_argv([
        "herdr-threads",
        "--state-dir",
        "/s",
        "launch",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "--json",
        "-c",
        "model=\"x y\"",
        "--",
        "prompt",
    ])
    .unwrap();
    let CliAction::Launch(launch) = parsed.action else {
        panic!("not a launch");
    };
    assert_eq!(launch.target.as_str(), "w1:p2");
    assert_eq!(launch.harness, ContextHarness::Codex);
    assert_eq!(
        launch.argv,
        ["--json", "-c", "model=\"x y\"", "--", "prompt"]
    );
    assert_eq!(
        parsed.output.format,
        crate::protocol::output::OutputFormat::Text
    );
    let claude = parse_argv([
        "herdr-threads",
        "launch",
        "--pane",
        "w1:p2",
        "--kind",
        "claude",
    ])
    .unwrap();
    let CliAction::Launch(claude) = claude.action else {
        panic!("not a launch");
    };
    assert!(claude.argv.is_empty());
    for bad in [
        &["herdr-threads", "launch", "--kind", "codex"][..],
        &["herdr-threads", "launch", "--pane", "p", "--kind", "gemini"][..],
        &[
            "herdr-threads",
            "launch",
            "--pane",
            "p",
            "--kind",
            "claude",
            "--project",
            "d",
        ][..],
    ] {
        assert_eq!(
            parse_argv(bad.iter().copied()).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "{bad:?}"
        );
    }
}

/// The user's zsh wrapper (`whence -f codex`) already passes `--no-daemon`:
/// launch adds none (Codex refuses the flag twice) and says so in the report
/// and the record; a wrapper without it, or a failed probe, keeps launch's
/// single `--no-daemon`. Kills: `error: the argument '--no-daemon' cannot be
/// used multiple times` under such a wrapper, and dropping the flag when the
/// shell could not be asked.
#[test]
fn codex_shell_wrapper_with_no_daemon_suppresses_launch_flag() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let caller = ["--model", "gpt-test"];
    let wrapper = "codex () {\n\tcommand aisw workspace check --tool codex || return $?\n\t\
                   HERDR_AGENT=codex command codex --no-daemon --approve-for-me \"$@\"\n}\n";
    let launch = |probe: FakeProbe| {
        let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
        let out = s
            .launch_with_probe(
                &host,
                &seats,
                &handoff,
                request(ContextHarness::Codex, &caller),
                &probe,
            )
            .unwrap();
        assert_eq!(out.exit, 0);
        (host.submitted()[0].argv.clone(), out.report)
    };

    let (argv, report) = launch(FakeProbe(Ok(wrapper.into())));
    assert_eq!(argv, caller);
    assert!(!argv.iter().any(|a| a == "--approve-for-me"));
    assert_eq!(report["codex_wrapper"], CODEX_WRAPPER_NO_DAEMON);
    assert_eq!(
        s.records().last().unwrap()["codex_wrapper"],
        CODEX_WRAPPER_NO_DAEMON
    );

    let plain = "codex () {\n\tcommand codex --approve-for-me \"$@\"\n}\n";
    let (argv, report) = launch(FakeProbe(Ok(plain.into())));
    assert_eq!(argv, ["--no-daemon", "--model", "gpt-test"]);
    assert_eq!(report["codex_wrapper"], Value::Null);

    let (argv, report) = launch(FakeProbe(Err("the shell probe timed out".into())));
    assert_eq!(argv, ["--no-daemon", "--model", "gpt-test"]);
    assert_eq!(report["codex_wrapper"], Value::Null);
}

/// The wrapper scan matches `--no-daemon` as its own word only. Kills:
/// matching a comment, a longer flag, or `--no-daemon=...`.
#[test]
fn wrapper_scan_matches_the_flag_word_only() {
    assert!(wrapper_passes_no_daemon(
        "codex () {\n\tcommand codex --no-daemon \"$@\"\n}"
    ));
    assert!(wrapper_passes_no_daemon("codex='codex --no-daemon'"));
    assert!(wrapper_passes_no_daemon(
        "codex is aliased to `codex --no-daemon'"
    ));
    assert!(!wrapper_passes_no_daemon("codex is /usr/local/bin/codex"));
    assert!(!wrapper_passes_no_daemon("\t# add --no-daemon later\n"));
    assert!(!wrapper_passes_no_daemon("command codex --no-daemon-x"));
    assert!(!wrapper_passes_no_daemon("command codex --no-daemon=false"));
    assert!(!wrapper_passes_no_daemon(""));
}

/// The real probe is bounded: a shell that never answers times out and is
/// reported as a failure (launch then keeps its own `--no-daemon`).
#[test]
fn system_shell_probe_is_bounded_and_reads_stdout() {
    let dir = std::env::temp_dir().join(format!(
        "ht-probe-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::create_dir(&dir).unwrap();
    let write = |name: &str, body: &str| {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    };
    let slow = write("slow", "#!/bin/sh\nexec sleep 30\n");
    let probe = SystemShellProbe {
        shell: slow,
        timeout: std::time::Duration::from_millis(200),
    };
    let started = std::time::Instant::now();
    assert!(probe.resolve_codex().is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let fake_zsh = write(
        "zsh",
        "#!/bin/sh\n[ \"$1\" = -ic ] || exit 9\necho \"$2\"\necho noise >&2\n",
    );
    let probe = SystemShellProbe {
        shell: fake_zsh,
        timeout: std::time::Duration::from_secs(3),
    };
    assert_eq!(
        probe.resolve_codex().unwrap(),
        "whence -f codex 2>/dev/null || type codex\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The agent gets a readable Herdr name: `--name` first, else the pane's
/// Herdr label, else the short seat id; the name and its source land in the
/// report and the launch record. Kills: `ht-<hash>` names in Herdr's agent
/// list, and the label overriding an explicit `--name`.
#[test]
fn launch_names_agent_from_name_flag_then_pane_label_then_seat() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let cases = [
        (
            None,
            Some("Mad Tea Hatter"),
            "mad-tea-hatter",
            "pane_label",
            false,
        ),
        (
            Some("Reviewer 2"),
            Some("Mad Tea Hatter"),
            "reviewer-2",
            "name",
            true,
        ),
        (Some("reviewer"), None, "reviewer", "name", false),
        (None, Some("!!!"), "seat-launch", "seat", false),
        (None, None, "seat-launch", "seat", false),
    ];
    for (index, (name, label, expected, source, fitted)) in cases.into_iter().enumerate() {
        let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
        let mut launch = request(ContextHarness::Claude, &[]);
        launch.name = name.map(str::to_owned);
        launch.pane_label = label.map(str::to_owned);
        let out = s.launch(&host, &seats, &handoff, launch).unwrap();
        assert_eq!(out.exit, 0);
        assert_eq!(host.submitted()[0].agent_name(), expected);
        assert_eq!(out.report["agent_name"], expected, "{name:?} {label:?}");
        assert_eq!(out.report["agent_name_source"], source);
        let warned = out.report["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("--name"));
        assert_eq!(warned, fitted, "{name:?}");
        let record = &s.records()[index];
        assert_eq!(record["agent_name"], expected);
        assert_eq!(record["agent_name_source"], source);
    }
}

/// An unconfirmed start records both names the adapter may have used, so
/// the operator can `herdr agent get` either.
#[test]
fn unknown_launch_records_both_candidate_agent_names() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let mut host = FakeHost::new();
    host.unknown = true;
    let (seats, handoff) = (seats(), handoff());
    let mut launch = request(ContextHarness::Claude, &[]);
    launch.pane_label = Some("hatter".into());
    let out = s.launch(&host, &seats, &handoff, launch).unwrap();
    assert_eq!(out.exit, 5);
    assert_eq!(out.report["agent_name"], Value::Null);
    let candidates = json!(["hatter", "hatter-launch"]);
    assert_eq!(out.report["agent_name_candidates"], candidates);
    assert_eq!(s.records()[0]["agent_name_candidates"], candidates);
}

#[test]
fn launch_pane_label_uses_pane_label_then_single_pane_tab_label() {
    let pane = |id: &str, label: Option<&str>, tab: Option<&str>, count: usize| PaneName {
        target: HostTargetId::new(id),
        label: label.map(str::to_owned),
        tab_label: tab.map(str::to_owned),
        tab_pane_count: count,
    };
    let panes = [
        pane("w1:p1", Some("mad-tea-hatter-codex"), Some("party"), 2),
        pane("w1:p2", None, Some("party"), 2),
        pane("w1:p3", None, Some("solo"), 1),
        pane("w1:p4", None, None, 1),
    ];
    let label = |id: &str| pane_label(&HostTargetId::new(id), &panes);
    assert_eq!(label("w1:p1").as_deref(), Some("mad-tea-hatter-codex"));
    assert_eq!(label("w1:p2"), None);
    assert_eq!(label("w1:p3").as_deref(), Some("solo"));
    assert_eq!(label("w1:p4"), None);
    assert_eq!(label("w9:p9"), None);
}

#[test]
fn launch_name_flag_parses_and_refuses_unusable_names() {
    use crate::cli::commands::{CliAction, parse_argv};
    let argv = |name: &'static str| {
        [
            "herdr-threads",
            "launch",
            "--pane",
            "w1:p2",
            "--kind",
            "claude",
            "--name",
            name,
        ]
    };
    let parsed = parse_argv(argv("Mad Hatter")).unwrap();
    let CliAction::Launch(launch) = parsed.action else {
        panic!("not a launch");
    };
    assert_eq!(launch.name.as_deref(), Some("Mad Hatter"));
    assert_eq!(launch.pane_label, None);
    for bad in ["123", "", "--"] {
        assert_eq!(
            parse_argv(argv(bad)).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "{bad:?}"
        );
    }
}

/// Scripted `SeatInspect` pages for the launch guard's open-binding read.
struct HistoryClient {
    pages: std::sync::Mutex<Vec<crate::protocol::results::SeatInspection>>,
    cursors: std::sync::Mutex<Vec<Option<String>>>,
}
impl LocalClient for HistoryClient {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        let Command::SeatInspect(query) = command else {
            panic!("only seat inspection is expected");
        };
        assert_eq!(query.page.limit, 50);
        self.cursors.lock().unwrap().push(query.page.cursor);
        Ok(CommandResult::SeatInspect(
            self.pages.lock().unwrap().remove(0),
        ))
    }
}

fn history_binding(
    ordinal: u64,
    target: &str,
    provenance: &str,
    ended: bool,
) -> crate::protocol::results::SeatHistoryItem {
    use crate::protocol::{
        ids::{ExecutionId, NativeSessionId},
        results::{BindingHistory, SeatHistoryItem},
    };
    SeatHistoryItem::Binding(BindingHistory {
        ordinal,
        generation: ordinal,
        target: HostTargetId::new(target),
        terminal: None,
        incarnation: None,
        host_boot: HostBootId::new("boot"),
        host_epoch: 1,
        native_session: NativeSessionId::new("s"),
        execution: ExecutionId::new("e"),
        observed_at: UtcMillis(1),
        registered_at: None,
        ended_at: ended.then_some(UtcMillis(2)),
        provenance: provenance.into(),
    })
}

fn history_page(
    items: Vec<crate::protocol::results::SeatHistoryItem>,
    next: Option<&str>,
) -> crate::protocol::results::SeatInspection {
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::{ContinuityStatus, MappingStatus, SeatInspection, SeatSummary},
    };
    SeatInspection {
        summary: SeatSummary {
            seat: SeatId::new("seat_1"),
            continuity: ContinuityStatus::Resolved,
            target: None,
            generation: 2,
            created_at: UtcMillis(0),
            retired_at: None,
        },
        mapping: MappingStatus {
            state: ContinuityStatus::Resolved,
            target: None,
            detail_argv: None,
        },
        hold: None,
        retirement: None,
        history: Page {
            items,
            next_cursor: next.map(Into::into),
            next_argv: None,
            high_water_ordinal: 9,
            scope_revision: None,
            has_more: next.is_some(),
            stop_reason: if next.is_some() {
                StopReason::Rows
            } else {
                StopReason::Complete
            },
            consistency: Consistency::BoundedLive,
        },
    }
}

/// Kills: reading only the first history page (a long-lived seat's open
/// binding is on a later page), and treating an ended binding as open.
#[test]
fn daemon_resolver_open_binding_returns_last_open_binding() {
    let client = HistoryClient {
        pages: std::sync::Mutex::new(vec![
            history_page(
                vec![
                    history_binding(1, "w1:p1", "cooperative_top_level", true),
                    history_binding(2, "w2:p2", "cooperative_top_level", false),
                ],
                Some("c1"),
            ),
            history_page(
                vec![history_binding(3, "w4:p9", "cooperative_top_level", false)],
                None,
            ),
        ]),
        cursors: std::sync::Mutex::new(vec![]),
    };
    let budget = CallBudget {
        deadline: MonoInstant(60_000),
        cancellation: Cancellation::default(),
    };
    let bound = open_binding_from_history(&client, &SeatId::new("seat_1"), &budget)
        .unwrap()
        .unwrap();
    assert_eq!(bound.target.as_str(), "w4:p9");
    assert_eq!(bound.provenance, "cooperative_top_level");
    assert_eq!(
        *client.cursors.lock().unwrap(),
        vec![None, Some("c1".to_string())]
    );

    let ended = HistoryClient {
        pages: std::sync::Mutex::new(vec![history_page(
            vec![history_binding(1, "w1:p1", "cooperative_top_level", true)],
            None,
        )]),
        cursors: std::sync::Mutex::new(vec![]),
    };
    assert_eq!(
        open_binding_from_history(&ended, &SeatId::new("seat_1"), &budget).unwrap(),
        None
    );
}
