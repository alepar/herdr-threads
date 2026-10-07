//! Managed launch composition through fake host, seat and handoff ports with
//! the real setup inspection and synthetic harness binaries. Each test names
//! the mutation it kills.

use super::*;
use crate::ports::{
    CorrelatedStartup, EvidenceKind, ExecutionEvidence, HostCallContext, HostSnapshot, HostUiState,
    IncarnationEvidence, NativeLaunchCapability, NativeLaunchRequest, ObservationProvenance,
    PromptOutcome, SafeWakeTarget, StructuralOccupancy,
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
    confirmed_refusal: bool,
}
impl FakeHost {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(1),
            occupancy: StructuralOccupancy::Unknown,
            submitted: Mutex::new(Vec::new()),
            unknown: false,
            confirmed_refusal: false,
        }
    }
    fn observation(&self) -> HostObservation {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        HostObservation {
            focused: false,
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
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn launch_native_with_evidence(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, crate::ports::NativeLaunchFailure> {
        if self.confirmed_refusal {
            return Err(crate::ports::NativeLaunchFailure::not_submitted(api(
                ErrorCode::TargetUnsafe,
                "confirmed busy",
            )));
        }
        self.launch_native(request, context).map_err(Into::into)
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
                process_hint: false,
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
        _: &crate::ports::SafeWakeTarget,
        _: &crate::ports::HostCallContext,
    ) -> Result<(), crate::protocol::results::ApiError> {
        Ok(())
    }
}

/// How the fake daemon answers a `RecordManagedLaunch`.
#[derive(Clone)]
enum RecordAnswer {
    Recorded,
    AlreadyBound,
    /// An older daemon without the `seat.managed_launch` capability.
    Unsupported,
    Refused,
}
struct FakeSeats {
    calls: AtomicUsize,
    held: bool,
    answer: RecordAnswer,
    recorded: Mutex<Vec<crate::protocol::commands::RecordManagedLaunch>>,
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
    fn record_managed_launch(
        &self,
        startup: &CorrelatedStartup,
        _: &CallBudget,
    ) -> Result<Option<ManagedLaunchRecord>, ApiError> {
        self.recorded
            .lock()
            .unwrap()
            .push(managed_launch_command(startup));
        let record = |recorded: bool, provenance: &str| {
            Ok(Some(ManagedLaunchRecord {
                seat: startup.seat.clone(),
                recorded,
                binding_generation: 3,
                provenance: provenance.into(),
            }))
        };
        match self.answer {
            RecordAnswer::Recorded => record(true, "managed_launch"),
            RecordAnswer::AlreadyBound => record(false, "cooperative_top_level"),
            RecordAnswer::Unsupported => Ok(None),
            RecordAnswer::Refused => Err(api(
                ErrorCode::StaleHostObservation,
                "launch evidence differs from the current pane observation",
            )),
        }
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

/// A pane shell with no configuration exports; never runs a real shell.
struct FakeProbe;
impl CodexShellProbe for FakeProbe {}

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
            home: None,
            declared_environment: Default::default(),
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
                scope: Default::default(),
                verb: SetupVerb::Install,
                harness,
                harness_binary: None,
                prompt_suggestions: Default::default(),
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
        let probe = FakeProbe;
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
    seats_answering(RecordAnswer::Recorded)
}
fn seats_answering(answer: RecordAnswer) -> FakeSeats {
    FakeSeats {
        calls: AtomicUsize::new(0),
        held: false,
        answer,
        recorded: Mutex::new(Vec::new()),
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

/// Kills a residual version gate before hook inspection.
#[test]
fn uncovered_metadata_still_requires_owned_hooks() {
    let s = Scratch::new();
    s.harness("claude", "2.1.200 (Claude Code)", b"");
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

/// A recovery-held target refuses through the ordinary resolver. Kills:
/// allocating around a hold or starting before resolution.
#[test]
fn held_target_refuses_without_start() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let (host, handoff) = (FakeHost::new(), handoff());
    let held = FakeSeats {
        held: true,
        ..seats()
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

/// Codex set up at user level: owned hooks are on disk, so launch
/// adds no hook, sandbox or daemon arguments and keeps the caller's unchanged.
/// Command approvals replace the allowance check.
/// Kills launching without owned hooks or changing caller arguments.
#[test]
fn codex_launch_needs_the_user_installation_without_injected_daemon_flags() {
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
    assert_eq!(report["sandbox"]["present"], false, "{report}");
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
    assert_eq!(argv, &caller);
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

    // A socket policy edit does not prevent approved command execution.
    let config = s.env.codex_home.clone().unwrap().join("config.toml");
    let denied = "sandbox_workspace_write.network_access = false\n";
    fs::write(&config, denied).unwrap();
    let host = FakeHost::new();
    let out = s
        .launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))
        .unwrap();
    assert_eq!(out.exit, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), denied);
}

/// Kills implicit defaults, splitting quoted values, shell expansion and moving
/// configured top-level options after a native subcommand.
#[test]
fn configured_launch_options_preserve_quotes_literals_and_caller_order() {
    let caller = ["exec", "--json", "prompt  with spaces", ""];
    for harness in [ContextHarness::Codex, ContextHarness::Claude] {
        for options in [None, Some(""), Some("   ")] {
            let launch = request(harness, &caller)
                .with_configured_options(options.map(Into::into))
                .unwrap();
            assert_eq!(launch.argv, caller);
        }
        let launch = request(harness, &caller)
            .with_configured_options(Some(
                r#"--no-daemon --model 'model with spaces' "literal $HOME $(touch nope) `id`" '' escaped\ value"#.into(),
            ))
            .unwrap();
        assert_eq!(
            launch.argv,
            [
                "--no-daemon",
                "--model",
                "model with spaces",
                "literal $HOME $(touch nope) `id`",
                "",
                "escaped value",
                "exec",
                "--json",
                "prompt  with spaces",
                "",
            ]
        );
    }
}

/// Kills silently dropping malformed or non-native argument values.
#[test]
fn configured_launch_options_reject_invalid_values() {
    use std::os::unix::ffi::OsStringExt;
    for options in [
        std::ffi::OsString::from("'unterminated"),
        std::ffi::OsString::from("trailing\\"),
        std::ffi::OsString::from("embedded\0nul"),
        std::ffi::OsString::from_vec(vec![0xff]),
    ] {
        let error = request(ContextHarness::Codex, &[])
            .with_configured_options(Some(options))
            .unwrap_err();
        assert!(matches!(error, RunError::Api(ref api) if api.code == ErrorCode::InvalidRequest));
    }
}

/// Kills bypassing managed validation for configured arguments.
#[test]
fn configured_launch_options_pass_through_managed_validation() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let launch = request(ContextHarness::Codex, &["exec", "PROMPT"])
        .with_configured_options(Some("--no-daemon".into()))
        .unwrap();
    s.launch(&host, &seats, &handoff, launch).unwrap();
    assert_eq!(host.submitted()[0].argv, ["--no-daemon", "exec", "PROMPT"]);
    let host = FakeHost::new();
    let launch = request(ContextHarness::Codex, &[])
        .with_configured_options(Some("--daemon".into()))
        .unwrap();
    assert_eq!(
        code(s.launch(&host, &seats, &handoff, launch)),
        ErrorCode::InvalidRequest
    );
    assert!(host.submitted().is_empty());
}

/// `codex exec` retains caller arguments without injected daemon flags.
/// Unsupported subcommands are refused.
#[test]
fn codex_exec_launch_preserves_caller_arguments() {
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
    assert_eq!(argv, &caller);

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

/// Kills cold schema-cache warming in the ordinary launch path.
#[test]
fn codex_launch_leaves_the_historical_fingerprint_cache_cold() {
    use crate::harness::{codex_evidence, codex_schema};
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    let state = s.env.state_dir.clone().unwrap();
    s.setup(ContextHarness::Codex);
    codex_schema::forget_memory_entry_for_test(&s.root.join("bin/codex"));
    let cache = codex_evidence::cache_path(&codex_evidence::dir(&state));
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    assert_eq!(
        s.launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))
            .unwrap()
            .exit,
        0
    );
    assert!(!cache.exists());
}

/// Even a listed older version can launch without an in-sandbox allowance.
#[test]
fn codex_without_measured_allowance_uses_command_approvals() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.158.0", b"");
    s.setup(ContextHarness::Codex);
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let result = s
        .launch(&host, &seats, &handoff, request(ContextHarness::Codex, &[]))
        .unwrap();
    assert_eq!(result.exit, 0);
    assert!(host.submitted()[0].argv.is_empty());
}

/// Newer admitted Codex builds use approved outside-sandbox CLI calls. A
/// socket-policy version allowlist must never prevent their managed start.
#[test]
fn future_codex_launch_uses_command_approvals_without_network_allowance() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.160.0", &committed_codex_schemas());
    let setup = s.setup(ContextHarness::Codex);
    assert_eq!(setup["sandbox"]["validation"], "unvalidated", "{setup}");
    let config = s.env.codex_home.as_ref().unwrap().join("config.toml");
    assert!(!config.exists());
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let caller = ["-a", "on-request", "You are Bob."];
    let result = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &caller),
        )
        .unwrap();
    assert_eq!(result.exit, 0);
    assert_eq!(host.submitted()[0].argv, caller);
    assert!(
        !config.exists(),
        "launch must not install network permissions"
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

/// A wrapper selected as `codex` rejects --no-daemon. Managed launch must
/// preserve its supported arguments without inspecting aliases or adding flags.
#[test]
fn codex_wrapper_rejecting_no_daemon_accepts_managed_launch_arguments() {
    let s = Scratch::new();
    let wrapper = s.root.join("bin/codex");
    fs::write(&wrapper, b"#!/bin/sh\nfor arg do\n  if [ \"$arg\" = --no-daemon ]; then exit 64; fi\ndone\nprintf '%s\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut rejected = crate::test_support::spawn::command(&wrapper);
    assert_eq!(
        rejected.arg("--no-daemon").output().unwrap().status.code(),
        Some(64)
    );
    s.setup(ContextHarness::Codex);
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let caller = ["--model", "gpt-test"];
    let out = s
        .launch_with_probe(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &caller),
            &FakeProbe,
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    let argv = host.submitted()[0].argv.clone();
    assert_eq!(argv, caller);
    let output = crate::test_support::spawn::command(&wrapper)
        .args(&argv)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "--model\ngpt-test\n"
    );
    assert!(out.report.get("codex_wrapper").is_none());
}

/// The real probe is bounded: a shell that never answers times out and is
/// returns no export.
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
    assert_eq!(probe.pane_shell_env("CODEX_HOME"), None);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    // The pane shell's own export is read back between markers (startup-file
    // noise is ignored); a shell that exports nothing yields none, whatever
    // the launcher's environment holds (the variable is removed first).
    let exporting = write(
        "exporting",
        "#!/bin/sh\n[ \"$1\" = -ic ] || exit 9\necho banner\nexport CODEX_HOME=/pane/codex\neval \"$2\"\necho tail\n",
    );
    let probe = SystemShellProbe {
        shell: exporting,
        // A liveness bound only: these probes answer at once, but `sh` start-up
        // under a loaded parallel suite can take seconds (the bound itself is
        // the 200 ms `slow` case above).
        timeout: std::time::Duration::from_secs(30),
    };
    assert_eq!(
        probe.pane_shell_env("CODEX_HOME").as_deref(),
        Some("/pane/codex")
    );
    assert_eq!(probe.pane_shell_env("CLAUDE_CONFIG_DIR"), None);
    assert_eq!(probe.pane_shell_env("not a name; rm"), None);
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

/// A pane shell that exports the given config directories itself.
struct PaneExports {
    codex_home: Option<String>,
    claude_config_dir: Option<String>,
}
impl CodexShellProbe for PaneExports {
    fn pane_shell_env(&self, var: &str) -> Option<String> {
        match var {
            "CODEX_HOME" => self.codex_home.clone(),
            "CLAUDE_CONFIG_DIR" => self.claude_config_dir.clone(),
            other => panic!("unexpected variable {other}"),
        }
    }
}

/// Wave 17: Herdr's `agent.start` hands the agent the pane shell's
/// environment, so launch inspects the config directory that shell exports,
/// not only the launcher's. Kills: inspecting the launcher's directory while
/// the agent will read another one (a launch that "passes" yet runs without
/// hooks), and not naming the inspected directory in the refusal.
#[test]
fn launch_checks_the_config_dir_it_hands_the_pane() {
    // Codex: hooks only in the launcher's CODEX_HOME (A); the pane shell
    // exports a different one (B).
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let a = s.env.codex_home.clone().unwrap();
    let b = s.root.join("pane codex home");
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let pane_b = PaneExports {
        codex_home: Some(b.display().to_string()),
        claude_config_dir: None,
    };
    match s.launch_with_probe(
        &host,
        &seats,
        &handoff,
        request(ContextHarness::Codex, &[]),
        &pane_b,
    ) {
        Err(RunError::Api(error)) => {
            assert_eq!(error.code, ErrorCode::MissingHook);
            assert!(
                error.detail.contains(&b.display().to_string()),
                "{}",
                error.detail
            );
            assert!(
                !error.detail.contains(&a.display().to_string()),
                "{}",
                error.detail
            );
        }
        other => panic!("expected a refusal naming the pane's CODEX_HOME, got {other:?}"),
    }
    assert!(host.submitted().is_empty());

    // The pane shell exports A itself: launch inspects A and reports it.
    let pane_a = PaneExports {
        codex_home: Some(a.display().to_string()),
        claude_config_dir: None,
    };
    let out = s
        .launch_with_probe(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, &[]),
            &pane_a,
        )
        .unwrap();
    assert_eq!(out.report["config_dir"]["path"], a.display().to_string());
    assert_eq!(out.report["config_dir"]["source"], "pane_shell");

    // A relative export is not a usable directory: the launcher's wins.
    let relative = PaneExports {
        codex_home: Some("relative/dir".into()),
        claude_config_dir: None,
    };
    let out = s
        .launch_with_probe(
            &FakeHost::new(),
            &seats,
            &handoff,
            request(ContextHarness::Codex, &[]),
            &relative,
        )
        .unwrap();
    assert_eq!(out.report["config_dir"]["source"], "launcher");

    // Claude, same rule with CLAUDE_CONFIG_DIR.
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let other = s.root.join("pane claude config");
    let host = FakeHost::new();
    let pane_other = PaneExports {
        codex_home: None,
        claude_config_dir: Some(other.display().to_string()),
    };
    match s.launch_with_probe(
        &host,
        &seats,
        &handoff,
        request(ContextHarness::Claude, &[]),
        &pane_other,
    ) {
        Err(RunError::Api(error)) => {
            assert_eq!(error.code, ErrorCode::MissingHook);
            assert!(error.detail.contains(&other.display().to_string()));
        }
        other => panic!("expected a refusal naming the pane's config dir, got {other:?}"),
    }
    assert!(host.submitted().is_empty());
}

/// Wave 28: the report answers "was my Codex profile applied?" with the
/// effective CODEX_HOME, its config.toml and the profile Codex selects
/// (`-p/--profile` beats `profile` in config.toml). Kills: reporting no
/// profile, preferring config.toml over the command line, and reading
/// arguments after `--` (a prompt) as the profile option.
#[test]
fn launch_reports_the_effective_codex_profile_and_config() {
    let s = Scratch::new();
    s.harness("codex", "codex-cli 0.159.2", &committed_codex_schemas());
    s.setup(ContextHarness::Codex);
    let home = s.env.codex_home.clone().unwrap();
    let config = home.join("config.toml");
    let original = fs::read_to_string(&config).unwrap_or_default();
    fs::write(&config, format!("profile = \"work\"\n{original}")).unwrap();
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let run = |argv: &[&str]| {
        s.launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Codex, argv),
        )
        .unwrap()
        .report["codex"]
            .clone()
    };
    let codex = run(&[]);
    assert_eq!(codex["codex_home"], home.display().to_string());
    assert_eq!(codex["config_path"], config.display().to_string());
    assert_eq!(codex["config_present"], true);
    assert_eq!(
        (&codex["profile"], &codex["profile_source"]),
        (&json!("work"), &json!("config.toml"))
    );
    for argv in [
        &["-p", "ci"][..],
        &["--profile", "ci"],
        &["--profile=ci"],
        &["-pci"],
        &["--profile", "other", "--profile=ci"],
    ] {
        let codex = run(argv);
        assert_eq!(
            (&codex["profile"], &codex["profile_source"]),
            (&json!("ci"), &json!("argv")),
            "{argv:?}"
        );
    }
    let codex = run(&["--", "-p", "ci"]);
    assert_eq!(codex["profile"], "work", "{codex}");
    // No profile anywhere: reported as the default.
    fs::write(&config, original).unwrap();
    let codex = run(&[]);
    assert_eq!(
        (&codex["profile"], &codex["profile_source"]),
        (&json!("default"), &json!("none"))
    );
    // The launch record carries it too.
    assert_eq!(s.records().last().unwrap()["codex"]["profile"], "default");
    // A Claude launch reports no Codex block.
    let c = Scratch::new();
    c.harness("claude", "2.1.285 (Claude Code)", b"");
    c.setup_claude();
    let out = c
        .launch(
            &FakeHost::new(),
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[]),
        )
        .unwrap();
    assert_eq!(out.report["codex"], Value::Null);
}

/// Scripted `SeatInspect` answers for the launch guard's open-binding read.
struct HistoryClient {
    answer: crate::protocol::results::SeatInspection,
    limits: std::sync::Mutex<Vec<u16>>,
}
impl LocalClient for HistoryClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        let Command::SeatInspect(query) = command else {
            panic!("only seat inspection is expected");
        };
        self.limits.lock().unwrap().push(query.page.limit);
        Ok(CommandResult::SeatInspect(self.answer.clone()))
    }
}

fn inspection_with(
    open: Option<crate::protocol::results::OpenBindingSummary>,
    has_more: bool,
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
        open_binding: open,
        history: Page {
            items: vec![],
            next_cursor: has_more.then(|| "c1".to_string()),
            next_argv: None,
            high_water_ordinal: 5_000,
            scope_revision: None,
            has_more,
            stop_reason: if has_more {
                StopReason::Rows
            } else {
                StopReason::Complete
            },
            consistency: Consistency::BoundedLive,
        },
    }
}

fn guard_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(60_000),
        cancellation: Cancellation::default(),
    }
}

/// Kills: a launch guard that pages the history (a long-lived seat would
/// exceed any page bound), or one that ignores the daemon's open binding.
#[test]
fn launch_guard_reads_open_binding_regardless_of_history_length() {
    let client = HistoryClient {
        answer: inspection_with(
            Some(crate::protocol::results::OpenBindingSummary {
                provenance: "cooperative_top_level".into(),
                harness: "claude".into(),
                target: HostTargetId::new("w4:p9"),
            }),
            true,
        ),
        limits: std::sync::Mutex::new(vec![]),
    };
    let bound = open_binding_of(&client, &SeatId::new("seat_1"), &guard_budget())
        .unwrap()
        .unwrap();
    assert_eq!(bound.target.as_str(), "w4:p9");
    assert_eq!(bound.provenance, "cooperative_top_level");
    assert_eq!(*client.limits.lock().unwrap(), vec![1]);
}

/// Kills: reporting an open binding when the daemon names none (the guard
/// would then run its live-agent check against a stale pane).
#[test]
fn launch_with_no_open_binding_skips_the_live_agent_check() {
    let client = HistoryClient {
        answer: inspection_with(None, true),
        limits: std::sync::Mutex::new(vec![]),
    };
    assert_eq!(
        open_binding_of(&client, &SeatId::new("seat_1"), &guard_budget()).unwrap(),
        None
    );
    assert_eq!(*client.limits.lock().unwrap(), vec![1]);
}

// ---- TRUST-POLICY A3 `managed_launch` (ht-5n6) ----

/// An accepted, correlated startup is reported to the daemon exactly once,
/// with the correlation's seat, pane, harness and structural evidence, and
/// the report and launch record show the binding as launched, not checked
/// in. Kills: a launch that leaves the seat unbound (no lost-prompt wake),
/// evidence taken from somewhere other than the correlation, or a binding
/// presented as a check-in.
#[test]
fn accepted_startup_records_a_managed_launch_binding() {
    let s = Scratch::new();
    s.harness("claude", "2.1.285 (Claude Code)", b"");
    s.setup_claude();
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let out = s
        .launch(
            &host,
            &seats,
            &handoff,
            request(ContextHarness::Claude, &[]),
        )
        .unwrap();
    assert_eq!(out.exit, 0);
    let recorded = seats.recorded.lock().unwrap().clone();
    assert_eq!(
        recorded,
        [crate::protocol::commands::RecordManagedLaunch {
            seat: SeatId::new("seat_launch"),
            target: HostTargetId::new("w9:p1"),
            harness: Harness::Claude,
            terminal: TerminalId::new("term_9"),
            incarnation: "herdr-server:pid=1".into(),
            host_boot: HostBootId::new("boot"),
            target_generation: 1,
        }]
    );
    assert_eq!(out.report["binding"]["recorded"], true);
    assert_eq!(out.report["binding"]["state"], "launched, not checked in");
    assert_eq!(out.report["binding"]["provenance"], "managed_launch");
    assert!(
        out.report["receipt"]
            .as_str()
            .unwrap()
            .contains("not receipt")
    );
    assert_eq!(s.records()[0]["binding"]["recorded"], true);
}

/// Kills: reporting an unconfirmed start (exit 5) to the daemon, which would
/// bind the seat to an agent nobody saw start.
#[test]
fn unconfirmed_start_records_no_binding() {
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
    assert!(seats.recorded.lock().unwrap().is_empty());
    assert!(out.report["binding"].is_null());
}

/// The agent started either way: an older daemon, a refusal, or a seat that
/// already has a binding never fails the launch; the report says what
/// happened. Kills: a launch exit that depends on the binding record.
#[test]
fn binding_record_outcomes_never_fail_the_launch() {
    for (answer, recorded, needle) in [
        (RecordAnswer::AlreadyBound, false, "left unchanged"),
        (RecordAnswer::Unsupported, false, "seat.managed_launch"),
        (RecordAnswer::Refused, false, "StaleHostObservation"),
    ] {
        let s = Scratch::new();
        s.harness("claude", "2.1.285 (Claude Code)", b"");
        s.setup_claude();
        let (host, seats, handoff) = (FakeHost::new(), seats_answering(answer), handoff());
        let out = s
            .launch(
                &host,
                &seats,
                &handoff,
                request(ContextHarness::Claude, &[]),
            )
            .unwrap();
        assert_eq!(out.exit, 0, "{needle}");
        assert_eq!(out.report["outcome"], "started");
        assert_eq!(out.report["binding"]["recorded"], recorded, "{needle}");
        assert!(
            out.report["binding"]["note"]
                .as_str()
                .unwrap()
                .contains(needle),
            "{needle}: {}",
            out.report["binding"]
        );
    }
}

#[test]
fn handoff_preflight_and_failed_durable_gate_never_submit_or_record_start() {
    let scratch = Scratch::new();
    scratch.harness("claude", "2.1.285 (Claude Code)", b"");
    scratch.setup_claude();
    let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let parts = LaunchParts {
        env: &scratch.env,
        host: &host,
        seats: &seats,
        handoff: &handoff,
        clock: &clock,
        record_dir: Some(&scratch.root),
        shell_probe: &probe,
    };
    let req = request(ContextHarness::Claude, &["bootstrap"]);
    let ready =
        execute_guarded(&req, &parts, true, &mut |_| panic!("preflight submitted")).unwrap();
    assert_eq!(ready.report["seat"], "seat_launch");
    let result = execute_guarded(&req, &parts, false, &mut |boundary| {
        let LaunchBoundary::BeforeSubmit(native) = boundary else {
            panic!("unexpected refusal boundary");
        };
        assert_eq!(native.seat.as_str(), "seat_launch");
        Err(api(ErrorCode::StoreCorrupt, "durable fence failed"))
    });
    assert_eq!(code(result), ErrorCode::StoreCorrupt);
    assert!(host.submitted().is_empty());
    assert!(seats.recorded.lock().unwrap().is_empty());
    assert!(scratch.records().is_empty());
}

#[test]
fn handoff_native_launcher_checks_frozen_seat_and_propagates_confirmed_refusal() {
    use crate::cli::handoff::{HandoffLauncher, NativeLauncher};
    let scratch = Scratch::new();
    scratch.harness("claude", "2.1.285 (Claude Code)", b"");
    scratch.setup_claude();
    let (mut host, seats, handoff) = (FakeHost::new(), seats(), handoff());
    host.confirmed_refusal = true;
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let parts = LaunchParts {
        env: &scratch.env,
        host: &host,
        seats: &seats,
        handoff: &handoff,
        clock: &clock,
        record_dir: Some(&scratch.root),
        shell_probe: &probe,
    };
    let mut launcher = NativeLauncher { parts };
    let req = request(ContextHarness::Claude, &["bootstrap"]);
    let mut events = vec![];
    let result = launcher.launch(&req, &SeatId::new("changed-seat"), &mut |possible| {
        events.push(possible);
        Ok(())
    });
    assert_eq!(code(result), ErrorCode::TargetUnsafe);
    assert!(events.is_empty());
    let result = launcher.launch(&req, &SeatId::new("seat_launch"), &mut |possible| {
        events.push(possible);
        Ok(())
    });
    assert_eq!(code(result), ErrorCode::TargetUnsafe);
    assert_eq!(events, vec![true, false]);
    assert!(host.submitted().is_empty());
    assert!(scratch.records().is_empty());
}

/// Missing policy must refuse before observing configuration or allocating a seat.
#[test]
fn adapter_launch_missing_provider_refuses_before_seat_or_start() {
    let s = Scratch::new();
    let host = FakeHost::new();
    let seats = seats();
    let handoff = FakeHandoff(AtomicUsize::new(0));
    let error = s
        .launch(&host, &seats, &handoff, request(ContextHarness::Human, &[]))
        .unwrap_err();
    let RunError::Api(error) = error else {
        panic!("API refusal required")
    };
    assert_eq!(error.code, ErrorCode::UnsupportedHarness);
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
    assert!(s.records().is_empty());
}

/// Mutation after preparation (including a durable fence) cannot start stale hooks.
#[test]
fn adapter_launch_configuration_mutation_before_submit_refuses() {
    let s = Scratch::new();
    s.harness("claude", "2.1.283 (Claude Code)", &[]);
    s.setup_claude();
    let host = FakeHost::new();
    let seats = seats();
    let handoff = FakeHandoff(AtomicUsize::new(0));
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let result = execute_guarded(
        &request(ContextHarness::Claude, &[]),
        &LaunchParts {
            env: &s.env,
            host: &host,
            seats: &seats,
            handoff: &handoff,
            clock: &clock,
            record_dir: Some(&s.root),
            shell_probe: &probe,
        },
        false,
        &mut |boundary| {
            if matches!(boundary, LaunchBoundary::BeforeSubmit(_)) {
                fs::write(
                    s.env
                        .claude_config_dir
                        .as_ref()
                        .unwrap()
                        .join("settings.json"),
                    b"{}",
                )
                .unwrap();
            }
            Ok(())
        },
    );
    let RunError::Api(error) = result.unwrap_err() else {
        panic!("API refusal required")
    };
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(host.submitted().is_empty());
    assert!(seats.recorded.lock().unwrap().is_empty());
}

pub(crate) mod fourth_adapter {
    use super::*;
    use crate::harness::adapter::*;
    use crate::harness::registry::{AdmittedHandle, Registration, Registry};

    const METADATA: AdapterMetadata = AdapterMetadata {
        id: "fourth",
        display_label: "Fourth fixture",
        context_spelling: "Fourth",
        context_aliases: &[],
        executable: ExecutableLookup::Path("fourth"),
        host_kinds: &["fourth"],
        setup_scopes: &[SetupScopeKind::ConfigRoot],
        budget: EventBudgetPolicy {
            lifecycle_ms: 1000,
            observer_ms: 1000,
        },
        runtime_sources: &["installed_probe"],
    };
    pub struct Fourth {
        pub provider: bool,
        pub options_key: Option<&'static str>,
        pub options_fixture: bool,
        pub disabled: bool,
        pub wrong_scope: bool,
        pub mutate_binary: std::sync::atomic::AtomicBool,
        pub status_calls: AtomicUsize,
    }
    impl HarnessAdapter for Fourth {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            if self.options_fixture {
                static OPTIONS_METADATA: AdapterMetadata = AdapterMetadata {
                    id: "synthetic_fourth",
                    context_spelling: "SyntheticFourth",
                    ..METADATA
                };
                return &OPTIONS_METADATA;
            }
            &METADATA
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            &[]
        }
        fn observe_install(&self, env: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            InstallObservation::Available {
                binary: crate::cli::hook::resolve_on_path("fourth", env.path.as_deref()).unwrap(),
                identity: RuntimeIdentity::stable_release("1.0.0", "installed_probe").unwrap(),
            }
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            AdmissionDecision::Listed {
                state: (),
                recipe: "fourth-fixture",
            }
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            Ladder::Admitted
        }
        fn classify(&self, _: &HookInput) -> ContractObservation {
            panic!("launch cannot classify hooks")
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            panic!("launch cannot check in")
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            panic!("launch cannot provide receipt output")
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            panic!("launch is not runtime evidence")
        }
        fn setup(
            &self,
            _: &crate::harness::adapter::SetupRequest,
            _: &CallBudget,
        ) -> Result<SetupOutcome, SetupFailure> {
            panic!("launch must never install")
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            panic!("launch must never remove")
        }
        fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
            self.status_calls.fetch_add(1, Ordering::SeqCst);
            if self.mutate_binary.load(Ordering::SeqCst) {
                fs::write(
                    request.native_binary.as_ref().unwrap(),
                    b"changed executable during preparation",
                )
                .unwrap();
            }
            let root = match &request.scope {
                ResolvedSetupScope::ConfigRoot(root) => root,
                _ => panic!("wrong scope"),
            };
            SetupStatus::Detailed(Box::new(LocalSetupStatus {
                scope: if self.wrong_scope {
                    ResolvedSetupScope::ConfigRoot(root.join("other"))
                } else {
                    request.scope.clone()
                },
                installed: true,
                enabled: Some(!self.disabled),
                admitted: Some(true),
                observed: None,
                configured_hook: Some(ConfiguredHook {
                    scope: "fixture".into(),
                    path: root.join("fixture-hooks").display().to_string(),
                    fingerprint: "owned-fourth".into(),
                }),
                fingerprint: Some("owned-fourth".into()),
                diagnostics: vec![],
                repairs: vec![],
                projection: Value::Null,
            }))
        }
        fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
            self.provider.then_some(self)
        }
    }
    impl LaunchPolicy for Fourth {
        fn native_options_env(&self) -> Option<&'static str> {
            self.options_key
        }
        fn resolve_scope(
            &self,
            request: &crate::harness::adapter::LaunchRequest,
            _: &dyn CodexShellProbe,
            _: &CallBudget,
        ) -> Result<LaunchScope, ApiError> {
            Ok(LaunchScope {
                setup: ResolvedSetupScope::ConfigRoot(request.environment.cwd.clone()),
                working_directory: request.environment.cwd.clone(),
                config_source: "fixture",
            })
        }
        fn validate_native_argv(&self, argv: &[String]) -> Result<(), ApiError> {
            if argv.iter().any(|arg| arg == "refuse") {
                return Err(api(
                    ErrorCode::InvalidRequest,
                    "fourth native grammar refusal",
                ));
            }
            Ok(())
        }
        fn compose_argv(
            &self,
            caller: Vec<String>,
            owned: Vec<String>,
        ) -> Result<Vec<String>, ApiError> {
            Ok([vec!["--fourth-owned".into()], owned, caller].concat())
        }
        fn prepare_launch(
            &self,
            request: &crate::harness::adapter::LaunchRequest,
            scope: &LaunchScope,
            _: &AdmittedHandle,
            status: &LocalSetupStatus,
            _: &dyn CodexShellProbe,
            _: &CallBudget,
        ) -> Result<LaunchPreparation, ApiError> {
            Ok(LaunchPreparation {
                argv: self.compose_argv(request.argv.clone(), vec![])?,
                hook: crate::harness::launch::owned_launch_hook(status)?,
                working_directory: scope.working_directory.clone(),
                environment_overrides: Default::default(),
                report: json!({"fourth": {"mode": "fixture"}}),
            })
        }
        fn configuration_fingerprint(
            &self,
            _: &crate::harness::adapter::LaunchRequest,
            _: &LaunchScope,
        ) -> Result<String, ApiError> {
            Ok("fourth-unchanged".into())
        }
        fn expected_host_kinds(&self) -> &'static [&'static str] {
            &["fourth"]
        }
    }
    pub fn registry(provider: bool, disabled: bool) -> (Registry, &'static Fourth) {
        registry_with_scope(provider, disabled, false)
    }
    pub fn registry_with_scope(
        provider: bool,
        disabled: bool,
        wrong_scope: bool,
    ) -> (Registry, &'static Fourth) {
        registry_internal(provider, disabled, wrong_scope, None, false)
    }
    pub fn registry_with_options(
        provider: bool,
        disabled: bool,
        wrong_scope: bool,
        options_key: Option<&'static str>,
    ) -> (Registry, &'static Fourth) {
        registry_internal(provider, disabled, wrong_scope, options_key, true)
    }
    fn registry_internal(
        provider: bool,
        disabled: bool,
        wrong_scope: bool,
        options_key: Option<&'static str>,
        options_fixture: bool,
    ) -> (Registry, &'static Fourth) {
        let fourth = Box::leak(Box::new(Fourth {
            provider,
            options_key,
            options_fixture,
            disabled,
            wrong_scope,
            mutate_binary: std::sync::atomic::AtomicBool::new(false),
            status_calls: AtomicUsize::new(0),
        }));
        // Includes a third metadata-only entry, so the launch target really is fourth.
        let third = Box::leak(Box::new(Third));
        let registrations = Box::leak(
            vec![
                Registration::new(&crate::harness::claude::ClaudeAdapter),
                Registration::new(&crate::harness::codex::CodexAdapter),
                Registration::new(third),
                Registration::new(fourth),
            ]
            .into_boxed_slice(),
        );
        (Registry::new(registrations).unwrap(), fourth)
    }
    struct Third;
    impl HarnessAdapter for Third {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            static THIRD: AdapterMetadata = AdapterMetadata {
                id: "third",
                display_label: "Third fixture",
                context_spelling: "Third",
                context_aliases: &[],
                executable: ExecutableLookup::Unsupported,
                host_kinds: &["third"],
                setup_scopes: &[],
                budget: EventBudgetPolicy {
                    lifecycle_ms: 1000,
                    observer_ms: 1000,
                },
                runtime_sources: &[],
            };
            &THIRD
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            &[]
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            panic!("unselected adapter observed")
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            panic!("unselected adapter admitted")
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            Ladder::Admitted
        }
        fn classify(&self, _: &HookInput) -> ContractObservation {
            unreachable!()
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            unreachable!()
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            unreachable!()
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            unreachable!()
        }
        fn setup(
            &self,
            _: &crate::harness::adapter::SetupRequest,
            _: &CallBudget,
        ) -> Result<SetupOutcome, SetupFailure> {
            unreachable!()
        }
        fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
            unreachable!()
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            unreachable!()
        }
    }
}

/// Catches retaining a closed Claude/Codex dispatch or granting check-in on startup.
#[test]
fn adapter_launch_keeps_native_argv_guards_and_managed_launch_only_provenance() {
    let s = Scratch::new();
    s.harness("fourth", "fourth-fixture", &[]);
    let (registry, adapter) = fourth_adapter::registry(true, false);
    let host = FakeHost::new();
    let seats = seats();
    let handoff = handoff();
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let parts = LaunchParts {
        env: &s.env,
        host: &host,
        seats: &seats,
        handoff: &handoff,
        clock: &clock,
        record_dir: Some(&s.root),
        shell_probe: &probe,
    };
    let harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        registry.agent("fourth").unwrap(),
    ));
    let out =
        execute_with_registry(&registry, &request(harness, &["hello 'world'"]), &parts).unwrap();
    assert_eq!(
        host.submitted()[0].argv,
        ["--fourth-owned", "hello 'world'"]
    );
    assert_eq!(out.report["harness"], "fourth");
    assert_eq!(out.report["fourth"]["mode"], "fixture");
    assert_eq!(out.report["codex"], Value::Null);
    assert_eq!(out.report["binding"]["provenance"], "managed_launch");
    assert_eq!(out.report["binding"]["state"], "launched, not checked in");
    assert_eq!(seats.recorded.lock().unwrap()[0].harness.as_str(), "fourth");
    assert_eq!(s.records()[0]["harness"], "fourth");
    assert_eq!(adapter.status_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        code(execute_with_registry(
            &registry,
            &request(harness, &["refuse"]),
            &parts
        )),
        ErrorCode::InvalidRequest
    );
    assert_eq!(host.submitted().len(), 1);
    assert_eq!(adapter.status_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn adapter_launch_injected_missing_provider_never_inspects_or_starts() {
    let s = Scratch::new();
    let (registry, adapter) = fourth_adapter::registry(false, false);
    let host = FakeHost::new();
    let seats = seats();
    let handoff = handoff();
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        registry.agent("fourth").unwrap(),
    ));
    assert_eq!(
        code(execute_with_registry(
            &registry,
            &request(harness, &[]),
            &LaunchParts {
                env: &s.env,
                host: &host,
                seats: &seats,
                handoff: &handoff,
                clock: &clock,
                record_dir: Some(&s.root),
                shell_probe: &probe
            }
        )),
        ErrorCode::UnsupportedHarness
    );
    assert_eq!(adapter.status_calls.load(Ordering::SeqCst), 0);
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.sequence.load(Ordering::SeqCst), 1);
    assert!(host.submitted().is_empty());
    assert!(s.records().is_empty());
}

#[test]
fn adapter_launch_disabled_configuration_never_allocates_or_starts() {
    let s = Scratch::new();
    s.harness("fourth", "fourth-fixture", &[]);
    let (registry, _) = fourth_adapter::registry(true, true);
    let host = FakeHost::new();
    let seats = seats();
    let handoff = handoff();
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        registry.agent("fourth").unwrap(),
    ));
    assert_eq!(
        code(execute_with_registry(
            &registry,
            &request(harness, &[]),
            &LaunchParts {
                env: &s.env,
                host: &host,
                seats: &seats,
                handoff: &handoff,
                clock: &clock,
                record_dir: Some(&s.root),
                shell_probe: &probe
            }
        )),
        ErrorCode::MissingHook
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
}

/// A status for a different config scope cannot qualify the selected launch.
#[test]
fn adapter_launch_selected_scope_mismatch_refuses_before_seat() {
    let s = Scratch::new();
    s.harness("fourth", "fourth-fixture", &[]);
    let (registry, _) = fourth_adapter::registry_with_scope(true, false, true);
    let host = FakeHost::new();
    let seats = seats();
    let handoff = handoff();
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        registry.agent("fourth").unwrap(),
    ));
    assert_eq!(
        code(execute_with_registry(
            &registry,
            &request(harness, &[]),
            &LaunchParts {
                env: &s.env,
                host: &host,
                seats: &seats,
                handoff: &handoff,
                clock: &clock,
                record_dir: Some(&s.root),
                shell_probe: &probe
            }
        )),
        ErrorCode::Conflict
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
}

/// Replacing the admitted executable during setup inspection refuses before allocation.
#[test]
fn adapter_launch_executable_change_during_preparation_never_allocates() {
    let s = Scratch::new();
    s.harness("fourth", "fourth-fixture", &[]);
    let (registry, adapter) = fourth_adapter::registry(true, false);
    adapter.mutate_binary.store(true, Ordering::SeqCst);
    let host = FakeHost::new();
    let seats = seats();
    let handoff = handoff();
    let clock = Clock0(AtomicU64::new(1));
    let probe = FakeProbe;
    let harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        registry.agent("fourth").unwrap(),
    ));
    assert_eq!(
        code(execute_with_registry(
            &registry,
            &request(harness, &[]),
            &LaunchParts {
                env: &s.env,
                host: &host,
                seats: &seats,
                handoff: &handoff,
                clock: &clock,
                record_dir: Some(&s.root),
                shell_probe: &probe
            }
        )),
        ErrorCode::Conflict
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted().is_empty());
}

/// Concrete preparation must reject a setup status captured before a config mutation.
#[test]
fn adapter_launch_preparation_rejects_stale_native_setup_status() {
    use crate::harness::adapter::{
        AdmissionRequest, InstallEnvironment, LaunchRequest as AdapterLaunchRequest, SetupStatus,
        StatusRequest,
    };
    for (harness, word, version, file_name) in [
        (
            ContextHarness::Claude,
            "claude",
            "2.1.283 (Claude Code)",
            "settings.json",
        ),
        (
            ContextHarness::Codex,
            "codex",
            "codex-cli 0.159.2",
            "hooks.json",
        ),
    ] {
        let s = Scratch::new();
        let schemas = committed_codex_schemas();
        s.harness(word, version, if word == "codex" { &schemas } else { &[] });
        s.setup(harness);
        let environment = s.env.snapshot();
        let budget = CallBudget {
            deadline: MonoInstant(environment.clock.monotonic_now().0 + 30_000),
            cancellation: Cancellation::default(),
        };
        let registry = crate::harness::registry::builtins();
        let registration = registry.by_id(registry.agent(word).unwrap()).unwrap();
        let admitted = registration
            .admit(
                &AdmissionRequest {
                    installed: registration.observe_install(
                        &InstallEnvironment {
                            clock: environment.clock.clone(),
                            path: environment.path.clone(),
                            config_root: environment.config_roots.get(word).cloned(),
                            state_dir: environment.state_dir.clone(),
                        },
                        &budget,
                    ),
                    input: None,
                    runtime_candidate: None,
                },
                &budget,
            )
            .unwrap();
        let request = AdapterLaunchRequest {
            argv: vec![],
            environment,
            native_binary: None,
        };
        let probe = FakeProbe;
        let scope = registration
            .launch_policy()
            .unwrap()
            .resolve_scope(&request, &probe, &budget)
            .unwrap();
        let SetupStatus::Detailed(status) = registration.status(
            &StatusRequest {
                scope: scope.setup.clone(),
                environment: request.environment.clone(),
                native_binary: None,
            },
            &budget,
        ) else {
            panic!("owned native status required")
        };
        let file = request
            .environment
            .config_roots
            .get(word)
            .unwrap()
            .join(file_name);
        let mut settings: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        settings["unrelated"] = json!("changed between status and preparation");
        fs::write(&file, serde_json::to_vec(&settings).unwrap()).unwrap();
        match registration.prepare_launch(&request, &scope, &admitted, &status, &probe, &budget) {
            Err(error) => assert_eq!(error.code, ErrorCode::Conflict, "{word}"),
            Ok(_) => panic!("stale {word} setup status must refuse native preparation"),
        }
    }
}

/// Selected native status must not start a probe after deadline or cancellation.
#[test]
fn adapter_launch_native_status_exhausted_budget_never_probes() {
    native_status_budget_regression(false);
}

/// Selected native status must stop its observation within the remaining budget.
#[test]
fn adapter_launch_native_status_short_budget_bounds_probe() {
    native_status_budget_regression(true);
}

fn native_status_budget_regression(short: bool) {
    use crate::harness::adapter::{SetupStatus, StatusRequest};
    for word in ["claude", "codex"] {
        for cancelled in [false, true] {
            if short && cancelled {
                continue;
            }
            let s = Scratch::new();
            let marker = s.root.join("status-probed");
            let binary = s.root.join("bin").join(word);
            fs::write(
                &binary,
                format!(
                    "#!/bin/sh\nprintf started > '{}'\n/bin/sleep 1\nprintf '{}\\n'\n",
                    marker.display(),
                    if word == "claude" {
                        "2.1.283 (Claude Code)"
                    } else {
                        "codex-cli 0.159.3"
                    }
                ),
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let environment = s.env.snapshot();
            let cancellation = Cancellation::default();
            if cancelled {
                cancellation.cancel();
            }
            let budget = CallBudget {
                deadline: MonoInstant(
                    environment.clock.monotonic_now().0
                        + if short {
                            50
                        } else if cancelled {
                            30_000
                        } else {
                            0
                        },
                ),
                cancellation,
            };
            let registry = crate::harness::registry::builtins();
            let registration = registry.by_id(registry.agent(word).unwrap()).unwrap();
            let scope = registration
                .resolve_setup_scope(&Default::default(), &environment)
                .unwrap();
            let began = std::time::Instant::now();
            let status = registration.status(
                &StatusRequest {
                    scope,
                    environment,
                    native_binary: None,
                },
                &budget,
            );
            let elapsed = began.elapsed();
            assert!(
                elapsed < std::time::Duration::from_millis(700),
                "{word} renewed probe budget: {elapsed:?}"
            );
            assert!(!marker.exists(), "{word} status invoked diagnostic flags");
            if let SetupStatus::Detailed(status) = status {
                assert_eq!(
                    status.admitted,
                    Some(short),
                    "{word} declared-contract admission did not respect executable availability and remaining budget"
                );
            } else {
                assert!(
                    !short,
                    "live executable-only observation must declare its contract"
                );
                assert!(
                    matches!(status, SetupStatus::Failed(_)),
                    "{word} must refuse the observation"
                );
            }
        }
    }
}

/// Kills passing an explicit non-executable path into guarded preparation.
#[test]
fn versionless_launch_rejects_unusable_explicit_binary_before_host_calls() {
    let s = Scratch::new();
    s.harness("claude", "2.1.284 (Claude Code)", b"");
    s.setup_claude();
    let missing = s.root.join("missing");
    let directory = s.root.join("directory");
    fs::create_dir(&directory).unwrap();
    let plain = s.root.join("plain");
    fs::write(&plain, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
    for path in [&missing, &directory, &plain] {
        let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
        let mut req = request(ContextHarness::Claude, &[]);
        req.harness_binary = Some(path.display().to_string());
        assert_eq!(
            code(s.launch(&host, &seats, &handoff, req)),
            ErrorCode::UnsupportedHarness
        );
        assert_eq!(host.sequence.load(Ordering::SeqCst), 1);
        assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
        assert!(host.submitted().is_empty());
    }
}

/// Kills launch-time executable probing and uncaptured form acceptance.
#[test]
fn versionless_launch_selects_wrapper_and_refuses_resume_without_start() {
    for harness in [ContextHarness::Claude, ContextHarness::Codex] {
        let s = Scratch::new();
        s.harness(
            harness.as_str(),
            if harness == ContextHarness::Codex {
                "codex-cli 0.158.0"
            } else {
                "2.1.284 (Claude Code)"
            },
            b"",
        );
        s.setup(harness);
        let log = s.root.join("wrapper.log");
        let wrapper = s.root.join("bin").join(harness.as_str());
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 93\n",
                log.display()
            ),
        )
        .unwrap();
        let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
        let out = s
            .launch(&host, &seats, &handoff, request(harness, &[]))
            .unwrap();
        assert_eq!(out.exit, 0);
        assert_eq!(
            out.report["harness_version"]["binary"],
            wrapper.display().to_string()
        );
        assert!(out.report["harness_version"]["version"].is_null());
        assert_eq!(
            out.report["harness_version"]["admission"],
            "contract_declared"
        );
        assert_eq!(host.submitted().len(), 1);
        assert!(!log.exists());
        if harness == ContextHarness::Codex {
            let refused = s.launch(
                &host,
                &seats,
                &handoff,
                request(harness, &["resume", "session"]),
            );
            assert!(refused.is_err());
            assert_eq!(host.submitted().len(), 1);
            assert!(!log.exists());
        }
    }
}

/// Kills reading another harness's options or ignoring the configured process
/// environment. Re-exec keeps environment changes out of parallel lib tests.
#[test]
fn configured_launch_options_read_matching_harness_environment() {
    const CHILD: &str = "HT_LAUNCH_OPTIONS_ENV_PROBE";
    if std::env::var_os(CHILD).is_some() {
        assert_eq!(
            request(ContextHarness::Codex, &["exec", "PROMPT"])
                .with_process_options()
                .unwrap()
                .argv,
            ["--no-daemon", "exec", "PROMPT"],
        );
        assert_eq!(
            request(ContextHarness::Claude, &["PROMPT"])
                .with_process_options()
                .unwrap()
                .argv,
            ["--model", "claude model", "PROMPT"],
        );
        return;
    }
    use crate::test_support::spawn::SpawnOwned;
    let mut command = crate::test_support::spawn::command(std::env::current_exe().unwrap());
    command
        .args([
            "cli::launch::tests::configured_launch_options_read_matching_harness_environment",
            "--exact",
        ])
        .env(CHILD, "1")
        .env("HERDR_THREADS_CODEX_OPTS", "--no-daemon")
        .env("HERDR_THREADS_CLAUDE_OPTS", "--model 'claude model'")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn_owned().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "options environment probe timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

/// Catches accidental legacy-key fallback for providers without configured options.
#[test]
fn stage_a_options_declaration_is_optional_and_provider_owned() {
    let registry = crate::harness::registry::builtins();
    for (name, expected) in [
        ("claude", Some("HERDR_THREADS_CLAUDE_OPTS")),
        ("codex", Some("HERDR_THREADS_CODEX_OPTS")),
        ("hermes", None),
    ] {
        let registration = registry.by_id(registry.agent(name).unwrap()).unwrap();
        assert_eq!(
            registration.launch_policy().unwrap().native_options_env(),
            expected
        );
    }
}

/// Catches unsafe declarations reaching getenv, implicit brand fallbacks, and lost literal tokens.
#[test]
fn stage_a_registered_options_validate_keys_and_preserve_literal_tokens() {
    use crate::harness::registry::OccupantHarness;
    let key128: &'static str =
        Box::leak(format!("HERDR_THREADS_{}_OPTS", "A".repeat(109)).into_boxed_str());
    let key129: &'static str =
        Box::leak(format!("HERDR_THREADS_{}_OPTS", "A".repeat(110)).into_boxed_str());
    for (key, valid) in [
        ("HERDR_THREADS_FOURTH_9_OPTS", true),
        (key128, true),
        (key129, false),
        ("HERDR_THREADS__OPTS", false),
        ("HERDR_THREADS_x_OPTS", false),
        ("HERDR_THREADS_É_OPTS", false),
        ("HERDR_THREADS_X=Y_OPTS", false),
        ("HERDR_THREADS_X\n_OPTS", false),
        ("HERDR_THREADS_$(ID)_OPTS", false),
        ("PATH", false),
        ("LC_ALL", false),
        ("HERDR_THREADS_CODEX_HOME", false),
    ] {
        let (registry, _) = fourth_adapter::registry_with_options(true, false, false, Some(key));
        let harness = ContextHarness::from(OccupantHarness::Agent(
            registry.agent("synthetic_fourth").unwrap(),
        ));
        let result = request(harness, &["--model", "caller", ""])
            .with_configured_options_with_registry(
                &registry,
                Some("--model 'configured' '' '$HOME' '$(id)' '*' '~'".into()),
            );
        if valid {
            let launch = result.unwrap();
            let scratch = Scratch::new();
            scratch.harness("fourth", "fourth-fixture", &[]);
            let (host, seats, handoff) = (FakeHost::new(), seats(), handoff());
            let clock = Clock0(AtomicU64::new(1));
            let probe = FakeProbe;
            let parts = LaunchParts {
                env: &scratch.env,
                host: &host,
                seats: &seats,
                handoff: &handoff,
                clock: &clock,
                record_dir: None,
                shell_probe: &probe,
            };
            assert_eq!(
                code(execute_with_registry(&registry, &launch, &parts)),
                ErrorCode::InvalidRequest
            );
            assert!(host.submitted().is_empty());
            let submitted = request(harness, &["--model", "caller"])
                .with_configured_options_with_registry(
                    &registry,
                    Some("--model configured '$HOME' '$(id)' '*' '~'".into()),
                )
                .unwrap();
            execute_with_registry(&registry, &submitted, &parts).unwrap();
            assert_eq!(
                host.submitted()[0].argv,
                [
                    "--fourth-owned",
                    "--model",
                    "configured",
                    "$HOME",
                    "$(id)",
                    "*",
                    "~",
                    "--model",
                    "caller"
                ]
            );
            assert_eq!(
                launch.argv,
                [
                    "--model",
                    "configured",
                    "",
                    "$HOME",
                    "$(id)",
                    "*",
                    "~",
                    "--model",
                    "caller",
                    ""
                ]
            );
        } else {
            assert!(
                matches!(result, Err(RunError::Api(ref error)) if error.code == ErrorCode::InvalidRequest)
            );
            assert!(
                native_options_env(&registry, harness)
                    .unwrap_err()
                    .detail
                    .len()
                    < 128
            );
        }
    }
    let (registry, _) = fourth_adapter::registry(true, false);
    let harness = ContextHarness::from(OccupantHarness::Agent(registry.agent("fourth").unwrap()));
    assert_eq!(
        request(harness, &["caller"])
            .with_configured_options_with_registry(&registry, Some("'malformed".into()))
            .unwrap()
            .argv,
        ["caller"]
    );
    for name in ["third", "hermes"] {
        let builtin = crate::harness::registry::builtins();
        let selected = if name == "third" { &registry } else { builtin };
        let harness = ContextHarness::from(OccupantHarness::Agent(selected.agent(name).unwrap()));
        let result = request(harness, &[])
            .with_configured_options_with_registry(selected, Some("'malformed".into()));
        if name == "third" {
            assert!(
                matches!(result, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnsupportedHarness)
            );
        } else {
            assert!(result.unwrap().argv.is_empty());
        }
    }
    assert!(
        request(ContextHarness::Human, &[])
            .with_configured_options(None)
            .is_err()
    );
    let absent = ContextHarness::from(OccupantHarness::Agent(registry.agent("third").unwrap()));
    assert!(native_options_env(crate::harness::registry::builtins(), absent).is_err());
}
