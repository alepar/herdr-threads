use super::*;
use crate::ports::{
    CorrelatedStartup, EvidenceKind, ExecutionEvidence, HostSnapshot, IncarnationEvidence,
    ObservationProvenance, PromptOutcome, SafeWakeTarget,
};
use crate::protocol::{
    ids::{HostBootId, HostCallId},
    time::{Cancellation, UtcMillis},
};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
struct FakeHost {
    capability: NativeLaunchCapability,
    observations: Mutex<Vec<HostObservation>>,
    submitted: Mutex<Vec<NativeLaunchRequest>>,
    outcome: NativeLaunchOutcome,
    correlation_override: Option<CorrelatedStartup>,
    launch_error: Option<ErrorCode>,
    pane_agent: Mutex<Option<crate::ports::PaneAgentObservation>>,
    pane_agent_reads: AtomicUsize,
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
        self.capability
    }
    fn observe_current_target(
        &self,
        _target: &HostTargetId,
        _context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observations.lock().unwrap().remove(0))
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
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn observe_pane_agent(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<Option<crate::ports::PaneAgentObservation>, ApiError> {
        assert_eq!(target.as_str(), "w4:p9", "the guard reads the bound pane");
        self.pane_agent_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.pane_agent.lock().unwrap().clone())
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        assert!(context.budget.deadline.0 <= 30_000);
        self.submitted.lock().unwrap().push(request.clone());
        if let Some(code) = &self.launch_error {
            return Err(error(code.clone(), "response lost after possible start"));
        }
        Ok(match &self.outcome {
            NativeLaunchOutcome::ObservedStartup { diagnostic, .. } => {
                NativeLaunchOutcome::ObservedStartup {
                    correlation: self
                        .correlation_override
                        .clone()
                        .unwrap_or(CorrelatedStartup {
                            process_hint: false,
                            seat: request.seat.clone(),
                            agent_name: request.agent_name(),
                            harness: request.harness,
                            target: request.target.clone(),
                            terminal: request.expected_terminal.clone(),
                            argv: request.argv.clone(),
                            host_boot: context.expected_boot.clone().unwrap(),
                            epoch: context.expected_epoch.unwrap(),
                            expected_generation: request.expected_generation,
                            expected_incarnation: request.expected_incarnation.clone(),
                            submitted_at_mono: MonoInstant(2),
                            completed_at_mono: MonoInstant(3),
                        }),
                    diagnostic: diagnostic.clone(),
                }
            }
            NativeLaunchOutcome::OutcomeUnknown => NativeLaunchOutcome::OutcomeUnknown,
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
struct FakeSeats {
    calls: AtomicUsize,
    failure: Option<ErrorCode>,
    open: Option<OpenBinding>,
}
impl LaunchSeatResolver for FakeSeats {
    fn resolve_for_launch(
        &self,
        _: &HostTargetId,
        _: &HostObservation,
        _: &CallBudget,
    ) -> Result<SeatId, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = &self.failure {
            return Err(error(code.clone(), "held or unresolved"));
        }
        Ok(SeatId::new("seat_1"))
    }
    fn open_binding(&self, _: &SeatId, _: &CallBudget) -> Result<Option<OpenBinding>, ApiError> {
        Ok(self.open.clone())
    }
}
struct FakeHooks(Option<ConfiguredHook>);
impl LaunchHookInspector for FakeHooks {
    fn configured_hook(
        &self,
        _: Harness,
        _: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError> {
        Ok(self.0.clone())
    }
}
fn observation(sequence: u64) -> HostObservation {
    HostObservation {
        focused: false,
        target: HostTargetId::new("pane_1"),
        host_boot: HostBootId::new("boot_1"),
        epoch: 1,
        generation: 4,
        observed_at_utc: UtcMillis(0),
        observed_at_mono: MonoInstant(sequence),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("term_1")),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc_1".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("call_{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(sequence),
        completed_at_mono: MonoInstant(sequence),
    }
}
fn fixture() -> (FakeHost, FakeSeats, FakeHooks, TestClock, CallBudget) {
    let first = observation(1);
    let second = observation(2);
    let mut started = second.clone();
    started.occupancy = StructuralOccupancy::Occupied;
    started.ui = HostUiState::Unknown;
    started.completed_at_mono = MonoInstant(3);
    (
        FakeHost {
            capability: NativeLaunchCapability::HostGuardedStart,
            observations: Mutex::new(vec![first, second.clone()]),
            submitted: Mutex::new(vec![]),
            outcome: NativeLaunchOutcome::ObservedStartup {
                correlation: correlation_for_empty_args(Harness::Codex),
                diagnostic: started,
            },
            correlation_override: None,
            launch_error: None,
            pane_agent: Mutex::new(None),
            pane_agent_reads: AtomicUsize::new(0),
        },
        FakeSeats {
            calls: AtomicUsize::new(0),
            failure: None,
            open: None,
        },
        FakeHooks(Some(ConfiguredHook {
            scope: "codex".into(),
            path: "/path with spaces/hook".into(),
            fingerprint: "sha256:abc".into(),
        })),
        TestClock(AtomicU64::new(0)),
        CallBudget {
            deadline: MonoInstant(60_000),
            cancellation: Cancellation::default(),
        },
    )
}
fn request(harness: Harness, argv: &[&str]) -> ManagedLaunchRequest {
    ManagedLaunchRequest {
        target: HostTargetId::new("pane_1"),
        harness,
        argv: argv.iter().map(|s| (*s).into()).collect(),
        name_hint: None,
    }
}

fn correlation_for_empty_args(harness: Harness) -> CorrelatedStartup {
    let request = NativeLaunchRequest {
        process_hint: false,
        seat: SeatId::new("seat_1"),
        target: HostTargetId::new("pane_1"),
        harness,
        argv: Vec::new(),
        configured_hook: ConfiguredHook {
            scope: "codex".into(),
            path: "/path with spaces/hook".into(),
            fingerprint: "sha256:abc".into(),
        },
        expected_terminal: TerminalId::new("term_1"),
        expected_generation: 4,
        expected_incarnation: "inc_1".into(),
        name_hint: None,
    };
    CorrelatedStartup {
        process_hint: false,
        seat: request.seat.clone(),
        agent_name: request.agent_name(),
        harness: request.harness,
        target: request.target.clone(),
        terminal: request.expected_terminal.clone(),
        argv: request.argv.clone(),
        host_boot: HostBootId::new("boot_1"),
        epoch: 1,
        expected_generation: request.expected_generation,
        expected_incarnation: request.expected_incarnation,
        submitted_at_mono: MonoInstant(2),
        completed_at_mono: MonoInstant(3),
    }
}

#[test]
fn launch_requires_a_resolved_empty_target() {
    let (host, seats, hooks, clock, budget) = fixture();
    host.observations.lock().unwrap()[0].occupancy = StructuralOccupancy::Occupied;
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Codex, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::TargetUnsafe
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted.lock().unwrap().is_empty());

    let (host, seats, hooks, clock, budget) = fixture();
    let seats = FakeSeats {
        failure: Some(ErrorCode::TargetUnresolved),
        ..seats
    };
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Codex, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::TargetUnresolved
    );
    assert!(host.submitted.lock().unwrap().is_empty());
}

#[test]
fn final_recheck_rejects_change_and_missing_hook() {
    let (host, seats, hooks, clock, budget) = fixture();
    host.observations.lock().unwrap()[1].generation = 5;
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleHostObservation
    );
    assert!(host.submitted.lock().unwrap().is_empty());

    let (host, seats, hooks, clock, budget) = fixture();
    host.observations.lock().unwrap()[1].ui = HostUiState::ApprovalOrQuestion;
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::TargetUnsafe
    );
    assert!(host.submitted.lock().unwrap().is_empty());

    let (host, seats, hooks, clock, budget) = fixture();
    host.observations.lock().unwrap()[1].observation_sequence = 1;
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleHostObservation
    );

    let (host, seats, _, clock, budget) = fixture();
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &FakeHooks(None),
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::MissingHook
    );
    assert!(host.submitted.lock().unwrap().is_empty());
}

#[test]
fn native_argument_forms_preserve_caller_options_and_transport_only() {
    let (host, seats, hooks, clock, budget) = fixture();
    let result = launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(
            Harness::Codex,
            &[
                "--model",
                "model with spaces",
                "--sandbox",
                "workspace-write",
            ],
        ),
        &budget,
    )
    .unwrap();
    assert!(matches!(
        result,
        NativeLaunchOutcome::ObservedStartup { .. }
    ));
    let sent = &host.submitted.lock().unwrap()[0];
    assert_eq!(
        sent.argv,
        [
            "--model",
            "model with spaces",
            "--sandbox",
            "workspace-write",
        ]
    );
    assert_eq!(sent.seat, SeatId::new("seat_1"));
    assert_eq!(sent.configured_hook.path, "/path with spaces/hook");

    let (host, seats, hooks, clock, budget) = fixture();
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(
            Harness::Claude,
            &["--permission-mode", "plan", "--model", "sonnet model"],
        ),
        &budget,
    )
    .unwrap();
    assert_eq!(
        host.submitted.lock().unwrap()[0].argv,
        ["--permission-mode", "plan", "--model", "sonnet model"]
    );
}

#[test]
fn conflicting_codex_mode_and_unsupported_adapter_never_start() {
    let (host, seats, hooks, clock, budget) = fixture();
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Codex, &["--daemon"]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    assert!(host.submitted.lock().unwrap().is_empty());
    let (mut host, seats, hooks, clock, budget) = fixture();
    host.capability = NativeLaunchCapability::Unsupported;
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );

    let (host, seats, hooks, clock, budget) = fixture();
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &["--no-daemon", "--model", "a b"]),
        &budget,
    )
    .unwrap();
    assert_eq!(
        host.submitted.lock().unwrap()[0].argv,
        ["--no-daemon", "--model", "a b"]
    );
}

#[test]
fn uncertain_transport_stays_unknown_and_does_not_resolve_again() {
    let (mut host, seats, hooks, clock, budget) = fixture();
    host.outcome = NativeLaunchOutcome::OutcomeUnknown;
    assert!(matches!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap(),
        NativeLaunchOutcome::OutcomeUnknown
    ));
    assert_eq!(seats.calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.submitted.lock().unwrap().len(), 1);

    let (mut host, seats, hooks, clock, budget) = fixture();
    host.launch_error = Some(ErrorCode::HostUnavailable);
    assert!(matches!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap(),
        NativeLaunchOutcome::OutcomeUnknown
    ));
    assert_eq!(host.submitted.lock().unwrap().len(), 1);

    let (host, seats, hooks, clock, budget) = fixture();
    budget.cancellation.cancel();
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Claude, &[]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::DeadlineExceeded
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn guarded_start_delegates_unknown_availability_for_both_harnesses() {
    for harness in [Harness::Codex, Harness::Claude] {
        let (host, seats, hooks, clock, budget) = fixture();
        for observed in host.observations.lock().unwrap().iter_mut() {
            observed.occupancy = StructuralOccupancy::Unknown;
            observed.ui = HostUiState::Unknown;
        }
        let result = launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(harness, &[]),
            &budget,
        );
        assert!(
            result.is_ok(),
            "guarded start should reach host for {harness:?}: {result:?}"
        );
        assert_eq!(seats.calls.load(Ordering::SeqCst), 1);
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }
}

#[test]
fn guarded_start_rejects_known_blocked_state_for_both_harnesses() {
    for harness in [Harness::Codex, Harness::Claude] {
        for ui in [
            HostUiState::ActiveTurn,
            HostUiState::ApprovalOrQuestion,
            HostUiState::HumanInput,
        ] {
            let (host, seats, hooks, clock, budget) = fixture();
            host.observations.lock().unwrap()[0].ui = ui;
            assert_eq!(
                launch_managed(
                    &host,
                    &seats,
                    &hooks,
                    &clock,
                    request(harness, &[]),
                    &budget
                )
                .unwrap_err()
                .code,
                ErrorCode::TargetUnsafe
            );
            assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
            assert!(host.submitted.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn unchanged_empty_response_never_proves_startup_for_both_harnesses() {
    for harness in [Harness::Codex, Harness::Claude] {
        let (mut host, seats, hooks, clock, budget) = fixture();
        let NativeLaunchOutcome::ObservedStartup { diagnostic, .. } = &mut host.outcome else {
            panic!()
        };
        diagnostic.occupancy = StructuralOccupancy::EmptyShell;
        diagnostic.ui = HostUiState::Idle;
        let result = launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(harness, &[]),
            &budget,
        )
        .unwrap();
        assert_eq!(result, NativeLaunchOutcome::OutcomeUnknown);
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }
}

#[test]
fn mismatched_startup_correlation_stays_unknown_for_both_harnesses() {
    for harness in [Harness::Codex, Harness::Claude] {
        for mismatch in [
            "name",
            "harness",
            "target",
            "argv",
            "epoch",
            "generation",
            "incarnation",
            "late",
        ] {
            let (mut host, seats, hooks, clock, budget) = fixture();
            let mut correlation = correlation_for_empty_args(harness);
            match mismatch {
                "name" => correlation.agent_name = "another-agent".into(),
                "harness" => {
                    correlation.harness = if harness == Harness::Codex {
                        Harness::Claude
                    } else {
                        Harness::Codex
                    }
                }
                "target" => correlation.target = HostTargetId::new("another-pane"),
                "argv" => correlation.argv.push("--unexpected".into()),
                "epoch" => correlation.epoch += 1,
                "generation" => correlation.expected_generation += 1,
                "incarnation" => correlation.expected_incarnation = "other-incarnation".into(),
                "late" => correlation.completed_at_mono = budget.deadline,
                _ => unreachable!(),
            }
            host.correlation_override = Some(correlation);
            let result = launch_managed(
                &host,
                &seats,
                &hooks,
                &clock,
                request(harness, &[]),
                &budget,
            )
            .unwrap();
            assert_eq!(
                result,
                NativeLaunchOutcome::OutcomeUnknown,
                "{harness:?} {mismatch}"
            );
            assert_eq!(host.submitted.lock().unwrap().len(), 1);
        }
    }
}

#[test]
fn unrelated_native_invocation_identity_cannot_authorize_launch() {
    for harness in [Harness::Codex, Harness::Claude] {
        let (host, seats, hooks, clock, budget) = fixture();
        host.observations.lock().unwrap()[0].incarnation = IncarnationEvidence::Verified {
            identity: "inc_1".into(),
            evidence_kind: EvidenceKind::NativeInvocation,
        };
        assert_eq!(
            launch_managed(
                &host,
                &seats,
                &hooks,
                &clock,
                request(harness, &[]),
                &budget
            )
            .unwrap_err()
            .code,
            ErrorCode::TargetUnresolved
        );
        assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
        assert!(host.submitted.lock().unwrap().is_empty());
    }
}

#[test]
fn producer_bound_coherent_enumeration_can_authorize_both_launches() {
    for harness in [Harness::Codex, Harness::Claude] {
        let (host, seats, hooks, clock, budget) = fixture();
        for observation in host.observations.lock().unwrap().iter_mut() {
            observation.incarnation = IncarnationEvidence::Verified {
                identity: "inc_1".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            };
        }
        assert!(matches!(
            launch_managed(
                &host,
                &seats,
                &hooks,
                &clock,
                request(harness, &[]),
                &budget
            )
            .unwrap(),
            NativeLaunchOutcome::ObservedStartup { .. }
        ));
        assert_eq!(seats.calls.load(Ordering::SeqCst), 1);
        assert_eq!(host.submitted.lock().unwrap().len(), 1);
    }
}

struct OwnedHooks(Vec<String>);
impl LaunchHookInspector for OwnedHooks {
    fn configured_hook(
        &self,
        _: Harness,
        _: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError> {
        Ok(Some(ConfiguredHook {
            scope: "session".into(),
            path: "codex:-c".into(),
            fingerprint: "sha256:owned".into(),
        }))
    }
    fn launch_configuration(
        &self,
        harness: Harness,
        budget: &CallBudget,
    ) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        Ok(self
            .configured_hook(harness, budget)?
            .map(|hook| LaunchHookConfiguration {
                hook,
                argv: self.0.clone(),
            }))
    }
}

/// Interactive Codex places owned configuration before unchanged caller arguments.
/// Kills dropping owned hooks or adding daemon-mode arguments.
#[test]
fn owned_configuration_precedes_caller_arguments_without_daemon_flags() {
    let (host, seats, _, clock, budget) = fixture();
    let owned = OwnedHooks(vec!["-c".into(), "hooks.SessionStart=[owned]".into()]);
    let _ = launch_managed(
        &host,
        &seats,
        &owned,
        &clock,
        request(Harness::Codex, &["--model", "m", "--", "prompt"]),
        &budget,
    );
    let submitted = host.submitted.lock().unwrap();
    assert_eq!(
        submitted[0].argv,
        [
            "-c",
            "hooks.SessionStart=[owned]",
            "--model",
            "m",
            "--",
            "prompt"
        ]
    );
    assert_eq!(submitted[0].configured_hook.fingerprint, "sha256:owned");
}

/// Kills: a caller `-c hooks.*` override (in any spelling) replacing the owned
/// hook inside Codex's session layer; a prompt after `--` is not an option.
#[test]
fn caller_codex_hook_overrides_are_refused_before_seat_resolution() {
    for argv in [
        &["-c", "hooks.SessionStart=[]"][..],
        &["--config", " hooks.PreToolUse=[]"][..],
        &["--config=hooks.SubagentStart=[]"][..],
        &["-chooks.SessionStart=[]"][..],
    ] {
        let (host, seats, hooks, clock, budget) = fixture();
        assert_eq!(
            launch_managed(
                &host,
                &seats,
                &hooks,
                &clock,
                request(Harness::Codex, argv),
                &budget
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest,
            "{argv:?}"
        );
        assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
        assert!(host.submitted.lock().unwrap().is_empty());
    }
    let (host, seats, hooks, clock, budget) = fixture();
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &["--", "-c", "hooks.x"]),
        &budget,
    )
    .unwrap();
    assert_eq!(host.submitted.lock().unwrap().len(), 1);
}

fn owned() -> Vec<String> {
    ["-c", "hooks.SessionStart=[o]", "-c", "allow=[s]"]
        .map(String::from)
        .to_vec()
}

fn compose(argv: &[&str]) -> Result<Vec<String>, ApiError> {
    compose_native_argv(
        Harness::Codex,
        argv.iter().map(|a| a.to_string()).collect(),
        owned(),
    )
}

/// The argument array for each supported Codex form (native-codex-matrix-1
/// P4/P5): only caller daemon flags, and
/// the owned `-c` pairs at the level of the subcommand that reads them
/// (`hook-placement-probe`: `exec` ignores root-level `hooks.*`). Kills:
/// injecting daemon arguments, and placing the
/// owned hooks before `exec`/`resume`.
#[test]
fn codex_argv_composition_per_form() {
    let o = ["-c", "hooks.SessionStart=[o]", "-c", "allow=[s]"];
    let cases: &[(&[&str], Vec<&str>)] = &[
        // interactive, no prompt
        (&[], o.to_vec()),
        // interactive with options and a prompt
        (
            &["-m", "exec", "--search", "fix it"],
            [&o[..], &["-m", "exec", "--search", "fix it"]].concat(),
        ),
        // interactive with a prompt after `--` that looks like a subcommand
        (&["--", "exec"], [&o[..], &["--", "exec"]].concat()),
        // exec
        (
            &["exec", "--json", "-s", "read-only", "PROMPT"],
            [&["exec"][..], &o, &["--json", "-s", "read-only", "PROMPT"]].concat(),
        ),
        // exec with root options and the caller's own top-level --no-daemon
        (
            &["--no-daemon", "-C", "/p q", "exec", "--", "PROMPT"],
            [
                &["--no-daemon", "-C", "/p q", "exec"][..],
                &o,
                &["--", "PROMPT"],
            ]
            .concat(),
        ),
        // exec resume
        (
            &[
                "exec",
                "-s",
                "read-only",
                "resume",
                "--json",
                "ID",
                "PROMPT",
            ],
            [
                &["exec", "-s", "read-only", "resume"][..],
                &o,
                &["--json", "ID", "PROMPT"],
            ]
            .concat(),
        ),
        // exec resume --last
        (
            &["exec", "resume", "--last"],
            [&["exec", "resume"][..], &o, &["--last"]].concat(),
        ),
        // exec whose prompt is the word "resume" after `--`
        (
            &["exec", "--", "resume"],
            [&["exec"][..], &o, &["--", "resume"]].concat(),
        ),
        // interactive whose prompt is a refused subcommand name after `--`
        (&["--", "update"], [&o[..], &["--", "update"]].concat()),
        // exec whose prompt is the word "fork" after `--`
        (
            &["exec", "--", "fork"],
            [&["exec"][..], &o, &["--", "fork"]].concat(),
        ),
        // exec resume behind a value-taking exec option (0.159.2
        // `--thread-source`): the hooks go after `resume`, not after `exec`
        (
            &["exec", "--thread-source", "X", "resume", "ID"],
            [&["exec", "--thread-source", "X", "resume"][..], &o, &["ID"]].concat(),
        ),
        // image in the `=` form is a single self-contained argument
        (
            &["exec", "--image=a.png", "PROMPT"],
            [&["exec"][..], &o, &["--image=a.png", "PROMPT"]].concat(),
        ),
        // image option after `--` is prompt text
        (
            &["exec", "--", "-i", "x"],
            [&["exec"][..], &o, &["--", "-i", "x"]].concat(),
        ),
    ];
    for (caller, expected) in cases {
        assert_eq!(compose(caller).unwrap(), *expected, "{caller:?}");
        let argv = compose(caller).unwrap();
        assert_eq!(
            argv.iter().filter(|a| *a == "--no-daemon").count(),
            caller.iter().filter(|arg| **arg == "--no-daemon").count(),
            "{caller:?}"
        );
    }
}

/// Kills: starting a Codex form that cannot carry the owned hooks (so the
/// agent would run without check-in while launch reports success), a
/// `--no-daemon` Codex would reject after the subcommand, and a misread
/// argument list placing the hooks at the wrong level.
#[test]
fn codex_argv_refuses_unconfigurable_forms() {
    for caller in [
        &["login"][..],
        &["fork", "ID"],
        &["e", "PROMPT"],
        &["review"],
        &["exec", "review"],
        &["exec", "help"],
        &["exec", "fork", "ID"],
        &["exec", "-m", "gpt", "fork", "ID"],
        &["mcp", "list"],
        &["agents"],
        &["plugin"],
        &["remote-control"],
        &["update"],
        &["doctor"],
        &["queue"],
        &["archive"],
        &["delete"],
        &["migrate-rollouts"],
        &["unarchive"],
        &["exec-server"],
        &["-m", "gpt", "update"],
        &["--no-daemon", "doctor"],
        &["exec", "--no-daemon", "PROMPT"],
        &["exec", "PROMPT", "--no-daemon"],
        &["resume", "--no-daemon"],
        &["--no-daemon", "--no-daemon"],
        &["--unknown-valued", "v", "exec", "PROMPT"],
        &["exec", "-c", "hooks.PreToolUse=[]"],
        &["--daemon", "exec"],
        // whole-table hook overrides replace the owned hooks just the same
        &["-c", "hooks={}"],
        &["exec", "-c", "hooks = []"],
        &["--config=hooks={}"],
        &["--config", " hooks ={}"],
        &["-chooks={}"],
        &["-c=hooks.SessionStart=[]"],
        // multi-value image option: arity would be guessed
        &["-i", "a.png", "exec", "PROMPT"],
        &["exec", "--image", "a.png", "PROMPT"],
    ] {
        assert_eq!(
            compose(caller).unwrap_err().code,
            ErrorCode::InvalidRequest,
            "{caller:?}"
        );
    }
}

/// A key merely starting with `hooks` is not the hooks table.
#[test]
fn codex_config_hooks_lookalike_key_is_not_an_override() {
    assert!(compose(&["-c", "hooksmith=1", "exec", "P"]).is_ok());
    assert!(compose(&["-c", "x=hooks.y", "exec", "P"]).is_ok());
}

/// Unsupported subcommands are refused before seat resolution or host start.
#[test]
fn unsupported_codex_subcommand_never_resolves_or_starts() {
    let (host, seats, hooks, clock, budget) = fixture();
    assert_eq!(
        launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Codex, &["fork", "ID"]),
            &budget
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
    assert!(host.submitted.lock().unwrap().is_empty());
}

/// Claude's owned arguments (none today) come first; caller bytes unchanged.
#[test]
fn claude_argv_is_owned_then_caller() {
    assert_eq!(
        compose_native_argv(
            Harness::Claude,
            vec!["-p".into(), "exec".into()],
            vec!["--x".into()]
        )
        .unwrap(),
        ["--x", "-p", "exec"]
    );
}

/// A scoped policy must remain the policy the capability probe checked.
#[test]
fn scoped_codex_policy_refuses_caller_policy_overrides() {
    let owned = vec![
        "-c".into(),
        "sandbox_workspace_write.network_access=true".into(),
    ];
    for caller in [
        vec!["-c", "features.network_proxy.enabled=false", "exec", "P"],
        vec!["-c", "model=\"other\"", "exec", "P"],
        vec!["--config=sandbox_mode=\"danger-full-access\"", "exec", "P"],
        vec!["--enable", "network_proxy", "exec", "P"],
        vec!["-p", "other", "exec", "P"],
        vec!["-sdanger-full-access", "exec", "P"],
        vec!["--dangerously-bypass-approvals-and-sandbox", "exec", "P"],
        vec!["--yolo", "exec", "P"],
        vec!["--dangerously-bypass-hook-trust", "exec", "P"],
        vec!["--remote=unix:///private/tmp/other.sock", "exec", "P"],
        vec!["--worktree", "exec", "P"],
        vec!["--search", "exec", "P"],
    ] {
        assert_eq!(
            compose_native_argv(
                Harness::Codex,
                caller.into_iter().map(str::to_owned).collect(),
                owned.clone(),
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(
        compose_native_argv(
            Harness::Codex,
            vec![
                "-c".into(),
                "sandbox_workspace_write.network_access=false".into()
            ],
            vec!["-c".into(), "hooks.SessionStart=[owned]".into()],
        )
        .is_ok(),
        "existing non-scoped launch policy remains caller-controlled"
    );
    assert!(
        compose_native_argv(
            Harness::Codex,
            vec!["-c".into(), "model=\"other\"".into()],
            vec![
                "-c".into(),
                "sandbox_workspace_write.network_access=false".into()
            ],
        )
        .is_ok(),
        "a network-disabled policy is not a scoped socket allowance"
    );
    assert!(
        compose_native_argv(
            Harness::Codex,
            vec!["--yolo".into(), "exec".into(), "P".into()],
            vec!["-c".into(), "hooks.SessionStart=[owned]".into()],
        )
        .is_ok(),
        "the scoped guard does not change existing non-scoped launch behavior"
    );
}

#[test]
fn scoped_codex_policy_requires_one_explicit_absolute_cwd() {
    assert_eq!(
        scoped_codex_cwd(&["-C".into(), "/repo".into(), "exec".into(), "P".into()]).unwrap(),
        std::path::PathBuf::from("/repo")
    );
    for argv in [
        vec!["exec", "P"],
        vec!["-C", "relative", "exec", "P"],
        vec!["-C", "/one", "-C", "/two", "exec", "P"],
        vec!["exec", "--", "-C", "/repo"],
    ] {
        assert!(
            scoped_codex_cwd(&argv.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err()
        );
    }
}

/// Explicit caller daemon arguments remain intact; the owned configuration
/// cannot inject an obsolete daemon argument.
#[test]
fn caller_daemon_flag_is_preserved_without_injection() {
    let owned = vec![
        "--no-daemon".into(),
        "-c".into(),
        "hooks.SessionStart=[owned]".into(),
    ];
    assert_eq!(
        compose_native_argv(Harness::Codex, Vec::new(), owned.clone()).unwrap(),
        ["-c", "hooks.SessionStart=[owned]"]
    );
    assert_eq!(
        compose_native_argv(
            Harness::Codex,
            vec!["--no-daemon".into(), "exec".into(), "PROMPT".into()],
            owned
        )
        .unwrap(),
        [
            "--no-daemon",
            "exec",
            "-c",
            "hooks.SessionStart=[owned]",
            "PROMPT"
        ]
    );
}

/// Kills: the `-c`/`-i` refusal scanning a prompt or the arguments after `--`
/// as if they were options; options before them are still refused.
#[test]
fn config_refusal_ignores_positionals_after_options() {
    assert!(compose(&["exec", "use -c here"]).is_ok());
    assert!(compose(&["--", "-c"]).is_ok());
    assert!(compose(&["exec", "--", "-c", "hooks.x=1"]).is_ok());
    assert!(compose(&["exec", "-m", "gpt", "run -i now"]).is_ok());
    assert!(compose(&["-c", "hooks.SessionStart=[]", "exec"]).is_err());
    assert!(compose(&["exec", "PROMPT", "-c", "hooks.x=1"]).is_err());
}

fn bound_fixture(
    provenance: &str,
    agent_kind: Option<&str>,
) -> (FakeHost, FakeSeats, FakeHooks, TestClock, CallBudget) {
    let (host, mut seats, hooks, clock, budget) = fixture();
    seats.open = Some(OpenBinding {
        target: HostTargetId::new("w4:p9"),
        provenance: provenance.into(),
    });
    *host.pane_agent.lock().unwrap() = agent_kind.map(|kind| crate::ports::PaneAgentObservation {
        kind: Some(kind.into()),
        agent_session: None,
    });
    (host, seats, hooks, clock, budget)
}

/// Kills: a launch that starts a second agent for a seat whose bound agent is
/// still live in its pane (the name-suffix retry would otherwise allow it).
#[test]
fn launch_refused_when_bound_claude_agent_is_live_elsewhere() {
    launch_refused_for_live_bound_agent("claude");
}

#[test]
fn launch_refused_when_bound_codex_agent_is_live_elsewhere() {
    launch_refused_for_live_bound_agent("codex");
}

/// TRUST-POLICY A4 (ht-5n6): an agent `launch` started that has not checked
/// in yet (`managed_launch` binding) is the seat's bound agent too. Kills: a
/// second launch for the seat while the first launched agent is live.
#[test]
fn launch_refused_when_launched_agent_is_live_before_check_in() {
    for kind in ["claude", "codex"] {
        let (host, seats, hooks, clock, budget) = bound_fixture("managed_launch", Some(kind));
        let err = launch_managed(
            &host,
            &seats,
            &hooks,
            &clock,
            request(Harness::Codex, &[]),
            &budget,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::TargetUnsafe, "{kind}");
        assert!(host.submitted.lock().unwrap().is_empty());
    }
}

fn launch_refused_for_live_bound_agent(kind: &str) {
    let (host, seats, hooks, clock, budget) = bound_fixture("cooperative_top_level", Some(kind));
    let err = launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Claude, &[]),
        &budget,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::TargetUnsafe);
    for needle in ["seat_1", "w4:p9", "second agent", kind] {
        assert!(err.detail.contains(needle), "{needle}: {}", err.detail);
    }
    assert_eq!(host.pane_agent_reads.load(Ordering::SeqCst), 1);
    assert!(host.submitted.lock().unwrap().is_empty());
}

#[test]
fn launch_proceeds_when_bound_pane_shows_no_agent() {
    let (host, seats, hooks, clock, budget) = bound_fixture("cooperative_top_level", None);
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &[]),
        &budget,
    )
    .unwrap();
    assert_eq!(host.pane_agent_reads.load(Ordering::SeqCst), 1);
    assert_eq!(host.submitted.lock().unwrap().len(), 1);
}

/// A detected kind outside claude/codex is not a live bound agent.
#[test]
fn launch_proceeds_when_bound_pane_agent_kind_is_unrecognized() {
    let (host, seats, hooks, clock, budget) = bound_fixture("cooperative_top_level", Some("pi"));
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &[]),
        &budget,
    )
    .unwrap();
    assert_eq!(host.submitted.lock().unwrap().len(), 1);
}

#[test]
fn launch_proceeds_for_human_bound_seat() {
    let (host, seats, hooks, clock, budget) = bound_fixture("operator_human", Some("claude"));
    launch_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &[]),
        &budget,
    )
    .unwrap();
    assert_eq!(host.pane_agent_reads.load(Ordering::SeqCst), 0);
    assert_eq!(host.submitted.lock().unwrap().len(), 1);
}

/// Kills: managed launch of the top-level Codex `resume` form, which no live
/// capture shows loading the owned hooks.
#[test]
fn codex_top_level_resume_form_is_refused_until_captured() {
    for caller in [
        &["resume"][..],
        &["-m", "m", "resume", "ID"],
        &["--remote", "ADDR", "resume"],
        &["--remote-auth-token-env", "VAR", "resume", "ID"],
    ] {
        let err = compose(caller).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest, "{caller:?}");
        assert!(err.detail.contains("no live capture"), "{}", err.detail);
        assert!(err.detail.contains("`resume`"), "{}", err.detail);
    }
    assert!(compose(&["exec", "resume", "ID"]).is_ok());
}

#[test]
fn handoff_preparation_runs_all_guards_without_native_submission() {
    let (host, seats, hooks, clock, budget) = fixture();
    let prepared = prepare_managed(
        &host,
        &seats,
        &hooks,
        &clock,
        request(Harness::Codex, &[]),
        &budget,
    )
    .unwrap();
    assert_eq!(prepared.request.seat, SeatId::new("seat_1"));
    assert!(host.submitted.lock().unwrap().is_empty());
    assert_eq!(seats.calls.load(Ordering::SeqCst), 1);
    let outcome = submit_prepared(&host, &clock, prepared).unwrap();
    assert!(matches!(
        outcome,
        NativeLaunchOutcome::ObservedStartup { .. }
    ));
    assert_eq!(host.submitted.lock().unwrap().len(), 1);
}

// Synthetic registration selects a policy through the same generic accessor as builtins.
pub(crate) fn process_hint_registry(required: bool) -> &'static crate::harness::registry::Registry {
    use crate::harness::registry::{Registration, Registry};
    let adapter = Box::leak(Box::new(HintAdapter(required)));
    Box::leak(Box::new(
        Registry::new(Box::leak(
            vec![Registration::new(adapter)].into_boxed_slice(),
        ))
        .unwrap(),
    ))
}
struct HintAdapter(bool);
impl crate::harness::adapter::HarnessAdapter for HintAdapter {
    type Admission = crate::harness::operational::ClaudeContract;
    fn metadata(&self) -> &'static crate::harness::adapter::AdapterMetadata {
        use crate::harness::adapter::*;
        static META: AdapterMetadata = AdapterMetadata {
            id: "hinted",
            display_label: "Hint fixture",
            context_spelling: "Hinted",
            context_aliases: &[],
            executable: ExecutableLookup::Path("codex"),
            host_kinds: &["codex"],
            setup_scopes: &[SetupScopeKind::ConfigRoot],
            budget: EventBudgetPolicy {
                lifecycle_ms: 1000,
                observer_ms: 1000,
            },
            runtime_sources: &["installed_probe"],
        };
        &META
    }
    fn contracts(&self) -> &'static [crate::harness::adapter::ContractDescriptor] {
        &[]
    }
    fn observe_install(
        &self,
        e: &crate::harness::adapter::InstallEnvironment,
        b: &CallBudget,
    ) -> crate::harness::adapter::InstallObservation {
        crate::harness::claude::ClaudeAdapter.observe_install(e, b)
    }
    fn admit(
        &self,
        r: &crate::harness::adapter::AdmissionRequest,
        b: &CallBudget,
    ) -> crate::harness::adapter::AdmissionDecision<Self::Admission> {
        crate::harness::claude::ClaudeAdapter.admit(r, b)
    }
    fn version_ladder(
        &self,
        r: &crate::harness::adapter::RuntimeIdentity,
    ) -> crate::harness::state::Ladder {
        crate::harness::claude::ClaudeAdapter.version_ladder(r)
    }
    fn classify(
        &self,
        r: &crate::harness::adapter::HookInput,
    ) -> crate::harness::adapter::ContractObservation {
        crate::harness::claude::ClaudeAdapter.classify(r)
    }
    fn decode(
        &self,
        a: &Self::Admission,
        r: &crate::harness::adapter::HookInput,
    ) -> Result<crate::harness::adapter::DecodedEvent, crate::harness::adapter::DecodeFailure> {
        crate::harness::claude::ClaudeAdapter.decode(a, r)
    }
    fn encode(
        &self,
        a: &Self::Admission,
        r: &crate::harness::adapter::DecodedEvent,
        o: &crate::harness::adapter::NeutralOffer,
    ) -> Result<crate::harness::adapter::EncodedOutput, crate::harness::adapter::EncodeFailure>
    {
        crate::harness::claude::ClaudeAdapter.encode(a, r, o)
    }
    fn attribute_runtime(
        &self,
        r: &crate::harness::adapter::HookInput,
        b: &CallBudget,
    ) -> crate::harness::adapter::RuntimeAttribution {
        crate::harness::claude::ClaudeAdapter.attribute_runtime(r, b)
    }
    fn setup(
        &self,
        r: &crate::harness::adapter::SetupRequest,
        b: &CallBudget,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        crate::harness::claude::ClaudeAdapter.setup(r, b)
    }
    fn status(
        &self,
        r: &crate::harness::adapter::StatusRequest,
        b: &CallBudget,
    ) -> crate::harness::adapter::SetupStatus {
        crate::harness::claude::ClaudeAdapter.status(r, b)
    }
    fn unsetup(
        &self,
        r: &crate::harness::adapter::UnsetupRequest,
        b: &CallBudget,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        crate::harness::claude::ClaudeAdapter.unsetup(r, b)
    }
    fn launch_policy(&self) -> Option<&dyn crate::harness::adapter::LaunchPolicy> {
        if self.0 {
            Some(&RequiredHintPolicy)
        } else {
            Some(&DefaultHintPolicy)
        }
    }
}
struct DefaultHintPolicy;
struct RequiredHintPolicy;
macro_rules! hint_policy {
 ($ty:ty $(,$hint:item)?) => {
 impl crate::harness::adapter::LaunchPolicy for $ty {
    $($hint)?
    fn resolve_scope(&self,r:&crate::harness::adapter::LaunchRequest,_:&dyn CodexShellProbe,_:&CallBudget)->Result<crate::harness::adapter::LaunchScope,ApiError> { Ok(crate::harness::adapter::LaunchScope {setup:crate::harness::adapter::ResolvedSetupScope::ConfigRoot(r.environment.cwd.clone()),working_directory:r.environment.cwd.clone(),config_source:"fixture"}) }
    fn validate_native_argv(&self,_:&[String])->Result<(),ApiError>{Ok(())}
    fn compose_argv(&self,caller:Vec<String>,owned:Vec<String>)->Result<Vec<String>,ApiError>{Ok([owned,caller].concat())}
    fn prepare_launch(&self,_:&crate::harness::adapter::LaunchRequest,_:&crate::harness::adapter::LaunchScope,_:&crate::harness::registry::AdmittedHandle,_:&crate::harness::adapter::LocalSetupStatus,_:&dyn CodexShellProbe,_:&CallBudget)->Result<crate::harness::adapter::LaunchPreparation,ApiError>{panic!("generic preparation uses supplied owned hook")}
    fn configuration_fingerprint(&self,_:&crate::harness::adapter::LaunchRequest,_:&crate::harness::adapter::LaunchScope)->Result<String,ApiError>{Ok("hint-fixture".into())}
    fn expected_host_kinds(&self)-> &'static [&'static str]{ &["codex"] }
 }
 };
}
hint_policy!(DefaultHintPolicy);
hint_policy!(
    RequiredHintPolicy,
    fn requires_process_hint(&self) -> bool {
        true
    }
);
#[test]
fn process_hint_policy_reaches_generic_native_request() {
    for required in [false, true] {
        for supplied in [false, true] {
            let registry = process_hint_registry(required);
            let (host, seats, hooks, clock, budget) = fixture();
            let harness = Harness::Agent(registry.agent("hinted").unwrap());
            let mut managed = request(harness, &["space arg", "apostrophe's arg"]);
            managed.name_hint = Some("hint-worker".into());
            let prepared = prepare_managed_with_registry(
                registry,
                &host,
                &seats,
                &hooks,
                &clock,
                managed,
                &budget,
                supplied.then(|| vec!["space arg".into(), "apostrophe's arg".into()]),
            )
            .unwrap();
            assert_eq!(
                prepared.request.process_hint, required,
                "selected policy mode was lost"
            );
            assert_eq!(prepared.request.argv, ["space arg", "apostrophe's arg"]);
            assert_eq!(prepared.request.agent_name(), "hint-worker");
            assert_eq!(prepared.request.target.as_str(), "pane_1");
            assert_eq!(prepared.request.configured_hook.fingerprint, "sha256:abc");
            assert_eq!(prepared.request.expected_incarnation, "inc_1");
        }
    }
}

struct Task48OwnedHooks {
    hook: ConfiguredHook,
    argv: Vec<String>,
}
impl LaunchHookInspector for Task48OwnedHooks {
    fn configured_hook(
        &self,
        _: Harness,
        _: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError> {
        Ok(Some(self.hook.clone()))
    }
    fn launch_configuration(
        &self,
        _: Harness,
        _: &CallBudget,
    ) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        Ok(Some(LaunchHookConfiguration {
            hook: self.hook.clone(),
            argv: self.argv.clone(),
        }))
    }
}

#[test]
fn task48_actual_owned_preparation_preserves_empty_data() {
    let (host, seats, hooks, clock, budget) = fixture();
    let owned = Task48OwnedHooks {
        hook: hooks.0.unwrap(),
        argv: vec!["--model".into(), "owned-model".into()],
    };
    let caller = [
        "--model",
        "caller-model",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
    ];
    let prepared = prepare_managed_with_registry(
        crate::harness::registry::builtins(),
        &host,
        &seats,
        &owned,
        &clock,
        request(Harness::Claude, &caller),
        &budget,
        None,
    );
    eprintln!(
        "task48 actual Claude owned preparation: result={:?} seat_calls={} remaining_observations={}",
        prepared.as_ref().map(|p| &p.request),
        seats.calls.load(Ordering::SeqCst),
        host.observations.lock().unwrap().len()
    );
    let prepared =
        prepared.expect("actual generic preparation must preserve the captured empty tools token");
    let expected: Vec<String> = owned
        .argv
        .iter()
        .cloned()
        .chain(caller.into_iter().map(str::to_owned))
        .collect();
    assert_eq!(prepared.request.argv, expected);
    assert_eq!(
        prepared
            .request
            .argv
            .iter()
            .filter(|arg| arg.is_empty())
            .count(),
        1
    );
    assert_eq!(prepared.request.configured_hook, owned.hook);
    assert!(host.submitted.lock().unwrap().is_empty());
}

#[test]
fn task48_composed_overflow_refuses_before_seat_work() {
    let mut observed = vec![];
    for (label, additions) in [
        ("count", vec!["owned".to_owned(); 65]),
        (
            "single",
            vec!["x".repeat(NativeLaunchRequest::MAX_ARG_BYTES + 1)],
        ),
        (
            "total",
            vec!["x".repeat(NativeLaunchRequest::MAX_ARG_BYTES); 2],
        ),
    ] {
        for supplied in [false, true] {
            let (host, seats, hooks, clock, budget) = fixture();
            let registry = process_hint_registry(false);
            let harness = Harness::Agent(registry.agent("hinted").unwrap());
            let owned = Task48OwnedHooks {
                hook: hooks.0.unwrap(),
                argv: additions.clone(),
            };
            let caller = request(harness, &["caller"]);
            let override_argv = supplied.then(|| [additions.clone(), caller.argv.clone()].concat());
            let result = prepare_managed_with_registry(
                registry,
                &host,
                &seats,
                &owned,
                &clock,
                caller,
                &budget,
                override_argv,
            );
            let seat_calls = seats.calls.load(Ordering::SeqCst);
            let observation_calls = 2 - host.observations.lock().unwrap().len();
            eprintln!(
                "task48 actual composed {label} override={supplied}: result={:?} seat_calls={seat_calls} observation_calls={observation_calls}",
                result.as_ref().map(|p| &p.request)
            );
            observed.push((
                label,
                supplied,
                result.map(|p| p.request),
                seat_calls,
                observation_calls,
                host.submitted.lock().unwrap().len(),
            ));
        }
    }
    for (label, supplied, result, seat_calls, observation_calls, submissions) in observed {
        assert_eq!(result.unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(
            seat_calls, 0,
            "{label} override={supplied}: lexical refusal preceded seat work"
        );
        assert_eq!(
            observation_calls, 0,
            "{label} override={supplied}: lexical refusal preceded host work"
        );
        assert_eq!(submissions, 0);
    }
}

fn v1(harness: Harness, argv: &[&str]) -> Result<Vec<String>, ApiError> {
    compose_bootstrap_v1_argv(harness, argv.iter().map(|a| a.to_string()).collect())
}

/// Frozen V1 bootstrap composition keeps the original caller order and forms
/// (empty owned argv), independently of today's composer; baseline equality
/// with the unperturbed current composer is a golden, not the RED.
#[test]
fn frozen_v1_bootstrap_composition_keeps_original_caller_forms() {
    let admitted: &[(Harness, &[&str])] = &[
        (Harness::Codex, &["--model", "fixed", "P"]),
        (
            Harness::Codex,
            &["--no-daemon", "-c", "a=1", "-c", "a=1", "P"],
        ),
        (Harness::Codex, &["--no-daemon", "exec", "--json", "P"]),
        (Harness::Codex, &["exec", "resume", "--last", "P"]),
        (Harness::Codex, &["--image=x.png", "--", "resume"]),
        (Harness::Codex, &["-c", "", "--model", "\t", "P"]),
        (Harness::Claude, &["--settings=x", "--tools", "", "P"]),
    ];
    for (harness, argv) in admitted {
        let caller: Vec<String> = argv.iter().map(|a| a.to_string()).collect();
        assert_eq!(v1(*harness, argv).unwrap(), caller, "{argv:?}");
        assert_eq!(
            compose_native_argv(*harness, caller.clone(), vec![]).unwrap(),
            caller,
            "tripwire: today's composer differs from V1 for {argv:?}"
        );
    }
    let refused: &[&[&str]] = &[
        &["--daemon", "P"],
        &["--no-daemon=1", "P"],
        &["-c", "hooks.SessionStart=[x]", "P"],
        &["--config=hooks={}", "P"],
        &["-chooks.Stop=1", "P"],
        &["-i", "x.png", "P"],
        &["resume", "P"],
        &["login"],
        &["exec", "fork", "P"],
        &["first", "second"],
        &["--no-daemon", "--no-daemon", "P"],
        &["exec", "--no-daemon", "P"],
    ];
    // Tripwire: today's composer must still agree with frozen V1 on every
    // admitted and refused form. A failure here means today's launch
    // composition changed: introduce bootstrap plan version 2 for new plans;
    // never edit V1 (retained V1 reports are validated against it).
    for argv in refused {
        assert!(v1(Harness::Codex, argv).is_err(), "{argv:?}");
        assert!(
            compose_native_argv(
                Harness::Codex,
                argv.iter().map(|a| a.to_string()).collect(),
                vec![]
            )
            .is_err(),
            "tripwire: today's composer admits V1-refused {argv:?}"
        );
    }
    assert!(v1(Harness::Human, &["P"]).is_err());
    let hermes = crate::harness::registry::builtins()
        .agent("hermes")
        .ok()
        .map(crate::harness::registry::OccupantHarness::Agent);
    if let Some(hermes) = hermes {
        assert!(v1(hermes, &["P"]).is_err(), "Hermes was never a V1 target");
    }
}

/// Simulated present-day composer drift is observed by the current composer
/// only; frozen V1 composition still accepts the formerly admitted argument.
#[test]
fn frozen_v1_bootstrap_composition_ignores_current_composer_drift() {
    let caller: Vec<String> = ["--model", "fixed", "P"].map(String::from).to_vec();
    let _drift = current_drift::arm(None, Some("fixed"));
    assert!(compose_native_argv(Harness::Codex, caller.clone(), vec![]).is_err());
    assert_eq!(
        compose_bootstrap_v1_argv(Harness::Codex, caller.clone()).unwrap(),
        caller
    );
    drop(_drift);
    assert!(compose_native_argv(Harness::Codex, caller, vec![]).is_ok());
}
