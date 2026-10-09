//! Managed native launch policy. Application composition supplies seat and hook producers.

use crate::ports::{
    ConfiguredHook, CorrelatedStartup, HostCallContext, HostObservation, HostPort, HostUiState,
    NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, StructuralOccupancy,
};
use crate::protocol::{
    authority::Harness,
    ids::{HostTargetId, SeatId, TerminalId},
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock, MonoInstant},
};

pub use super::codex::launch::{CodexShellProbe, SHELL_PROBE_TIMEOUT, SystemShellProbe};

const MAX_LAUNCH_MILLIS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLaunchRequest {
    pub target: HostTargetId,
    pub harness: Harness,
    /// Native agent arguments; their order and bytes are retained.
    pub argv: Vec<String>,
    /// Readable Herdr agent name wanted (`launch --name`, else the pane
    /// label); see [`NativeLaunchRequest::agent_name`].
    pub name_hint: Option<String>,
}

/// The seat's open (not ended) binding, as launch needs it for the live-agent guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenBinding {
    pub target: HostTargetId,
    pub provenance: String,
}

/// The ordinary service resolver must enforce durable recovery holds and ownership.
/// It may allocate the explicit target only through the standard guarded seat path.
pub trait LaunchSeatResolver: Send + Sync {
    fn resolve_for_launch(
        &self,
        target: &HostTargetId,
        observation: &HostObservation,
        budget: &CallBudget,
    ) -> Result<SeatId, ApiError>;

    /// The pane's already resolved seat, without resolving or allocating
    /// one. The default reports none.
    fn existing_seat(
        &self,
        _target: &HostTargetId,
        _budget: &CallBudget,
    ) -> Result<Option<SeatId>, ApiError> {
        Ok(None)
    }

    /// The seat's open binding, if any (TRUST-POLICY A4 launch guard). The
    /// default reports none.
    fn open_binding(
        &self,
        _seat: &SeatId,
        _budget: &CallBudget,
    ) -> Result<Option<OpenBinding>, ApiError> {
        Ok(None)
    }

    /// TRUST-POLICY A3 `managed_launch` (ht-5n6): report one accepted,
    /// correlated startup so the daemon can open an unregistered binding on a
    /// seat with none. Called only after an accepted `ObservedStartup`, never
    /// on `OutcomeUnknown`. `Ok(None)`: the daemon does not record managed
    /// launches (an older daemon); the default records nothing.
    fn record_managed_launch(
        &self,
        _startup: &CorrelatedStartup,
        _budget: &CallBudget,
    ) -> Result<Option<crate::protocol::results::ManagedLaunchRecord>, ApiError> {
        Ok(None)
    }
}

/// The daemon command an accepted, correlated startup becomes: the seat, pane
/// and harness Herdr started, and the structural evidence it was started in.
pub fn managed_launch_command(
    startup: &CorrelatedStartup,
) -> crate::protocol::commands::RecordManagedLaunch {
    crate::protocol::commands::RecordManagedLaunch {
        seat: startup.seat.clone(),
        target: startup.target.clone(),
        harness: startup.harness,
        terminal: startup.terminal.clone(),
        incarnation: startup.expected_incarnation.clone(),
        host_boot: startup.host_boot.clone(),
        target_generation: startup.expected_generation,
    }
}

/// The owned hook configuration a managed launch adds: the installed hook's
/// descriptor plus the native arguments that load it. Claude's project-scoped
/// hooks load from its working directory, so they add no arguments; Codex's
/// session-scoped hooks are `-c hooks.*` overrides (and the sandbox socket
/// allowance) that exist only on the launch line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchHookConfiguration {
    pub hook: ConfiguredHook,
    /// Owned native arguments, placed by [`compose_native_argv`].
    pub argv: Vec<String>,
}

/// Setup inspection returns only a descriptor for an already installed owned hook.
pub trait LaunchHookInspector: Send + Sync {
    fn configured_hook(
        &self,
        harness: Harness,
        budget: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError>;

    /// The descriptor plus the owned launch arguments. The default adds no
    /// arguments (an installed on-disk configuration).
    fn launch_configuration(
        &self,
        harness: Harness,
        budget: &CallBudget,
    ) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        Ok(self
            .configured_hook(harness, budget)?
            .map(|hook| LaunchHookConfiguration {
                hook,
                argv: Vec::new(),
            }))
    }
}

fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

fn launch_target(
    observation: &HostObservation,
    target: &HostTargetId,
    capability: NativeLaunchCapability,
) -> Result<(TerminalId, String), ApiError> {
    if &observation.target != target || !observation.is_fresh_structure() {
        return Err(error(
            ErrorCode::StaleHostObservation,
            "target observation is not fresh",
        ));
    }
    let availability_ok = match capability {
        NativeLaunchCapability::HostGuardedStart => {
            observation.occupancy != StructuralOccupancy::Occupied
                && matches!(observation.ui, HostUiState::Idle | HostUiState::Unknown)
        }
        NativeLaunchCapability::Unsupported => false,
    };
    if !availability_ok || observation.occupant.is_some() {
        return Err(error(
            ErrorCode::TargetUnsafe,
            "target availability is unsafe for native launch",
        ));
    }
    let proof = observation.verified_structural_proof().ok_or_else(|| {
        error(
            ErrorCode::TargetUnresolved,
            "target has no verified structural proof",
        )
    })?;
    Ok((proof.terminal().clone(), proof.incarnation().to_owned()))
}

#[cfg(test)]
pub(crate) use super::codex::launch::{
    CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS, CODEX_UNSUPPORTED_SUBCOMMANDS, CODEX_VALUE_OPTIONS,
};
pub use super::codex::launch::{CodexLaunchForm, scoped_codex_cwd};

pub fn compose_native_argv(
    harness: Harness,
    caller: Vec<String>,
    owned: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(token) = current_drift::rejected_token()
        && caller.iter().any(|arg| arg == token)
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "simulated current composer drift refuses this argument",
        ));
    }
    let registration = launch_registration(super::registry::builtins(), harness)?;
    #[allow(unused_mut)]
    let mut argv = registration
        .launch_policy()
        .expect("checked provider")
        .compose_argv(caller, owned)?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(extra) = current_drift::appended_arg() {
        argv.push(extra.to_owned());
    }
    Ok(argv)
}
/// Test-only, thread-local simulation of a later change to today's mutable
/// launch composition (the prompt renderer and [`compose_native_argv`]). It
/// models an upgrade without touching retained data; frozen V1 composition
/// ([`compose_bootstrap_v1_argv`]) never reads it.
#[cfg(any(test, feature = "test-support"))]
pub mod current_drift {
    use std::cell::Cell;

    thread_local! {
        static PROMPT_SUFFIX: Cell<Option<&'static str>> = const { Cell::new(None) };
        static REJECTED_TOKEN: Cell<Option<&'static str>> = const { Cell::new(None) };
        static APPENDED_ARG: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    /// Restores the unperturbed current composition on drop.
    pub struct Armed(());
    impl Drop for Armed {
        fn drop(&mut self) {
            PROMPT_SUFFIX.with(|cell| cell.set(None));
            REJECTED_TOKEN.with(|cell| cell.set(None));
            APPENDED_ARG.with(|cell| cell.set(None));
        }
    }

    /// Today's prompt renderer appends `prompt_suffix`; today's composer
    /// refuses any caller argument equal to `rejected_token`.
    pub fn arm(prompt_suffix: Option<&'static str>, rejected_token: Option<&'static str>) -> Armed {
        PROMPT_SUFFIX.with(|cell| cell.set(prompt_suffix));
        REJECTED_TOKEN.with(|cell| cell.set(rejected_token));
        Armed(())
    }

    /// Today's composer accepts every argument but appends `appended` to
    /// its output: a different, still admitted native argv.
    pub fn arm_output(appended: &'static str) -> Armed {
        APPENDED_ARG.with(|cell| cell.set(Some(appended)));
        Armed(())
    }

    pub(crate) fn appended_arg() -> Option<&'static str> {
        APPENDED_ARG.with(Cell::get)
    }

    pub(crate) fn prompt_suffix() -> Option<&'static str> {
        PROMPT_SUFFIX.with(Cell::get)
    }

    pub(crate) fn rejected_token() -> Option<&'static str> {
        REJECTED_TOKEN.with(Cell::get)
    }
}

/// Frozen V1 bootstrap launch composition: the native argv a version-1 Root
/// bootstrap (`BootstrapPlan.version == 1`, the only plan version) produced
/// from its caller arguments plus generated prompt, with the empty owned argv
/// that production hook inspection supplies. Retained successful launch
/// reports are validated against this, never against today's registry or
/// provider grammar, so a later change to [`compose_native_argv`] cannot
/// invalidate an already successful historical launch.
///
/// The original acceptance grammar is kept (Codex daemon-mode, hook-override,
/// image-arity, subcommand-form and `--no-daemon` guards); with empty owned
/// argv every admitted form returns the caller arguments unchanged. Only the
/// original V1 native targets (Codex, Claude) are accepted. Do not edit this
/// to follow current launch behavior: it is a compatibility record.
pub fn compose_bootstrap_v1_argv(
    harness: Harness,
    caller: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    if harness == Harness::Claude {
        return Ok(caller);
    }
    if harness != Harness::Codex {
        return Err(error(
            ErrorCode::UnsupportedHarness,
            "not a version-1 bootstrap native target",
        ));
    }
    v1::codex_guard(&caller)?;
    Ok(caller)
}

/// The V1 Codex acceptance grammar, frozen with its original tables.
mod v1 {
    use super::error;
    use crate::protocol::results::{ApiError, ErrorCode};

    const VALUE_OPTIONS: &[&str] = &[
        "-c",
        "--config",
        "--enable",
        "--disable",
        "-i",
        "--image",
        "--remote",
        "--remote-auth-token-env",
        "--thread-source",
        "-m",
        "--model",
        "--local-provider",
        "-p",
        "--profile",
        "-s",
        "--sandbox",
        "-a",
        "--ask-for-approval",
        "-C",
        "--cd",
        "--add-dir",
        "--output-schema",
        "--color",
        "-o",
        "--output-last-message",
    ];
    const MULTI_VALUE_OPTIONS: &[&str] = &["-i", "--image"];
    const UNSUPPORTED_SUBCOMMANDS: &[&str] = &[
        "agents",
        "e",
        "review",
        "login",
        "logout",
        "mcp",
        "mcp-server",
        "app-server",
        "app",
        "completion",
        "sandbox",
        "debug",
        "apply",
        "a",
        "fork",
        "cloud",
        "cloud-tasks",
        "features",
        "help",
        "plugin",
        "remote-control",
        "update",
        "doctor",
        "queue",
        "archive",
        "delete",
        "migrate-rollouts",
        "unarchive",
        "exec-server",
        "responses-api-proxy",
        "stdio-to-uds",
        "execpolicy",
        "generate-ts",
    ];
    const EXEC_UNSUPPORTED_SUBCOMMANDS: &[&str] = &["fork", "review", "help"];

    fn invalid(detail: &str) -> ApiError {
        error(ErrorCode::InvalidRequest, detail)
    }

    fn next_positional(argv: &[String], start: usize) -> Option<(usize, bool)> {
        let mut index = start;
        while index < argv.len() {
            let arg = argv[index].as_str();
            if arg == "--" {
                return (index + 1 < argv.len()).then_some((index + 1, true));
            }
            if arg.len() > 1 && arg.starts_with('-') {
                index += if VALUE_OPTIONS.contains(&arg) { 2 } else { 1 };
                continue;
            }
            return Some((index, false));
        }
        None
    }

    /// The index of the subcommand, if any, for an admitted V1 form.
    fn form(argv: &[String]) -> Result<Option<usize>, ApiError> {
        let Some((first, after_separator)) = next_positional(argv, 0) else {
            return Ok(None);
        };
        if after_separator {
            return Ok(None);
        }
        match argv[first].as_str() {
            "exec" => match next_positional(argv, first + 1) {
                Some((second, false))
                    if argv[second] != "resume"
                        && EXEC_UNSUPPORTED_SUBCOMMANDS.contains(&argv[second].as_str()) =>
                {
                    Err(invalid("V1 bootstrap refuses this Codex exec subcommand"))
                }
                _ => Ok(Some(first)),
            },
            "resume" => Err(invalid("V1 bootstrap refuses the Codex resume form")),
            word if UNSUPPORTED_SUBCOMMANDS.contains(&word) => {
                Err(invalid("V1 bootstrap refuses this Codex subcommand"))
            }
            _ if next_positional(argv, first + 1).is_some() => Err(invalid(
                "ambiguous Codex arguments: more than one positional argument before a \
                 recognised subcommand",
            )),
            _ => Ok(None),
        }
    }

    fn overrides_hooks(value: &str) -> bool {
        let key = value.split('=').next().unwrap_or("").trim();
        key == "hooks" || key.starts_with("hooks.")
    }

    pub(super) fn codex_guard(caller: &[String]) -> Result<(), ApiError> {
        let options_end = caller
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(caller.len());
        let options = &caller[..options_end];
        if options.iter().any(|arg| {
            arg == "--daemon" || arg.starts_with("--daemon=") || arg.starts_with("--no-daemon=")
        }) {
            return Err(invalid("conflicting Codex daemon mode"));
        }
        let mut previous_is_config = false;
        let mut previous_takes_value = false;
        for arg in options {
            let value = if previous_is_config {
                Some(arg.as_str())
            } else {
                arg.strip_prefix("--config=").or_else(|| {
                    arg.strip_prefix("-c")
                        .filter(|rest| !rest.is_empty())
                        .map(|rest| rest.strip_prefix('=').unwrap_or(rest))
                })
            };
            if value.is_some_and(overrides_hooks) {
                return Err(invalid(
                    "caller Codex hooks override would replace the owned hook configuration",
                ));
            }
            if !previous_takes_value && MULTI_VALUE_OPTIONS.contains(&arg.as_str()) {
                return Err(invalid(
                    "ambiguous Codex arguments: put images as --image=FILE or before --",
                ));
            }
            previous_is_config = !previous_takes_value && (arg == "-c" || arg == "--config");
            previous_takes_value = !previous_takes_value && VALUE_OPTIONS.contains(&arg.as_str());
        }
        let subcommand = form(caller)?;
        let no_daemon: Vec<usize> = options
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "--no-daemon")
            .map(|(index, _)| index)
            .collect();
        if no_daemon.len() > 1 {
            return Err(invalid("duplicate Codex --no-daemon"));
        }
        if let (Some(&at), Some(subcommand)) = (no_daemon.first(), subcommand)
            && at > subcommand
        {
            return Err(invalid(
                "Codex --no-daemon is a top-level flag; it must precede the subcommand",
            ));
        }
        Ok(())
    }
}

pub fn launch_registration(
    registry: &super::registry::Registry,
    harness: Harness,
) -> Result<&'static super::registry::Registration, ApiError> {
    let super::registry::OccupantHarness::Agent(id) = harness else {
        return Err(error(
            ErrorCode::UnsupportedHarness,
            "launch starts agents only; a person uses `herdr-threads me init`",
        ));
    };
    let registration = registry
        .by_id(id)
        .map_err(|err| error(ErrorCode::UnsupportedHarness, &err.to_string()))?;
    if registration.launch_policy().is_none() {
        return Err(error(
            ErrorCode::UnsupportedHarness,
            &format!("{}: launch is unsupported", id.as_str()),
        ));
    }
    Ok(registration)
}

pub struct PreparedLaunch {
    pub request: NativeLaunchRequest,
    context: HostCallContext,
    current: HostObservation,
}

/// Resolves the seat before direct native start. A successful return proves only
/// host-observed startup; check-in and receipts remain separate operations.
pub fn launch_managed(
    host: &dyn HostPort,
    seats: &dyn LaunchSeatResolver,
    hooks: &dyn LaunchHookInspector,
    clock: &dyn Clock,
    request: ManagedLaunchRequest,
    caller_budget: &CallBudget,
) -> Result<NativeLaunchOutcome, ApiError> {
    let prepared = prepare_managed(host, seats, hooks, clock, request, caller_budget)?;
    submit_prepared(host, clock, prepared)
}

pub fn prepare_managed(
    host: &dyn HostPort,
    seats: &dyn LaunchSeatResolver,
    hooks: &dyn LaunchHookInspector,
    clock: &dyn Clock,
    request: ManagedLaunchRequest,
    caller_budget: &CallBudget,
) -> Result<PreparedLaunch, ApiError> {
    prepare_managed_with_registry(
        super::registry::builtins(),
        host,
        seats,
        hooks,
        clock,
        request,
        caller_budget,
        None,
    )
}

/// TRUST-POLICY A4: refuse a launch onto a seat whose agent binding Herdr
/// still reports live in its pane.
fn refuse_second_agent(
    registry: &super::registry::Registry,
    host: &dyn HostPort,
    seats: &dyn LaunchSeatResolver,
    seat: &SeatId,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    if let Some(bound) = seats.open_binding(seat, budget)?
        && crate::protocol::authority::AGENT_BINDING_PROVENANCES
            .contains(&bound.provenance.as_str())
    {
        let observed = host.observe_pane_agent(
            &bound.target,
            &HostCallContext {
                budget: budget.clone(),
                expected_boot: None,
                expected_epoch: None,
            },
        )?;
        if let Some(kind) = observed
            .and_then(|agent| agent.kind)
            .filter(|kind| registry.by_host_kind(kind).is_some())
        {
            return Err(error(
                ErrorCode::TargetUnsafe,
                &format!(
                    "seat {} is bound to a live {kind} agent in pane {}; launch refuses to start a second agent for the seat (TRUST-POLICY A4). Use that pane, or retire/rebind the seat as the operator",
                    seat.as_str(),
                    bound.target.as_str()
                ),
            ));
        }
    }
    Ok(())
}

/// The seat guards `prepare_managed_with_registry` applies before hook
/// configuration, for callers whose own preparation found the hook missing:
/// a fresh target read, then the A4 refusal for the pane's already resolved
/// seat. It never resolves or allocates a seat.
pub fn launch_seat_guards(
    registry: &super::registry::Registry,
    host: &dyn HostPort,
    seats: &dyn LaunchSeatResolver,
    clock: &dyn Clock,
    target: &HostTargetId,
    caller_budget: &CallBudget,
) -> Result<(), ApiError> {
    let capability = host.native_launch_capability();
    if capability == NativeLaunchCapability::Unsupported {
        return Err(error(
            ErrorCode::Unsupported,
            "native guarded launch is unavailable",
        ));
    }
    let budget = CallBudget {
        deadline: MonoInstant(
            caller_budget
                .deadline
                .0
                .min(clock.monotonic_now().0.saturating_add(MAX_LAUNCH_MILLIS)),
        ),
        cancellation: caller_budget.cancellation.clone(),
    };
    let first = host.observe_current_target(
        target,
        &HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        },
    )?;
    launch_target(&first, target, capability)?;
    match seats.existing_seat(target, &budget)? {
        Some(seat) => refuse_second_agent(registry, host, seats, &seat, &budget),
        None => Ok(()),
    }
}

/// The application passes adapter-prepared argv; compatibility callers use provider composition.
#[allow(clippy::too_many_arguments)]
pub fn prepare_managed_with_registry(
    registry: &super::registry::Registry,
    host: &dyn HostPort,
    seats: &dyn LaunchSeatResolver,
    hooks: &dyn LaunchHookInspector,
    clock: &dyn Clock,
    request: ManagedLaunchRequest,
    caller_budget: &CallBudget,
    native_argv: Option<Vec<String>>,
) -> Result<PreparedLaunch, ApiError> {
    let registration = launch_registration(registry, request.harness)?;
    let policy = registration.launch_policy().expect("checked provider");
    if caller_budget.is_exhausted(clock) {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "launch budget exhausted",
        ));
    }
    let capability = host.native_launch_capability();
    if capability == NativeLaunchCapability::Unsupported {
        return Err(error(
            ErrorCode::Unsupported,
            "native guarded launch is unavailable",
        ));
    }
    // Refuse native grammar before any host or seat work.
    policy.validate_native_argv(&request.argv)?;
    // All launch work shares one finite absolute deadline and cancellation token.
    let budget = CallBudget {
        deadline: MonoInstant(
            caller_budget
                .deadline
                .0
                .min(clock.monotonic_now().0.saturating_add(MAX_LAUNCH_MILLIS)),
        ),
        cancellation: caller_budget.cancellation.clone(),
    };
    // A composed form is refused before any seat work.
    let configured = match hooks.launch_configuration(request.harness, &budget)? {
        Some(configuration) => {
            // Owned configuration at the caller's subcommand level (Codex) or
            // first (Claude); the caller's arguments keep their bytes and order.
            let argv = match native_argv {
                Some(argv) => argv,
                None => policy.compose_argv(request.argv.clone(), configuration.argv)?,
            };
            crate::ports::validate_native_argv(&argv)
                .map_err(|detail| error(ErrorCode::InvalidRequest, detail))?;
            Some((configuration.hook, argv))
        }
        None => None,
    };
    let first = host.observe_current_target(
        &request.target,
        &HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        },
    )?;
    let (terminal, incarnation) = launch_target(&first, &request.target, capability)?;
    if budget.is_exhausted(clock) {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "launch preflight expired",
        ));
    }
    let seat = seats.resolve_for_launch(&request.target, &first, &budget)?;
    refuse_second_agent(registry, host, seats, &seat, &budget)?;
    // A missing hook is reported only after the seat guards (TRUST-POLICY A4).
    let (hook, argv) = configured
        .ok_or_else(|| error(ErrorCode::MissingHook, "supported hook is not configured"))?;
    if budget.is_exhausted(clock) {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "launch preflight expired",
        ));
    }
    let current = host.observe_current_target(
        &request.target,
        &HostCallContext {
            budget: budget.clone(),
            expected_boot: Some(first.host_boot.clone()),
            expected_epoch: Some(first.epoch),
        },
    )?;
    let (current_terminal, current_incarnation) =
        launch_target(&current, &request.target, capability)?;
    if current.host_boot != first.host_boot
        || current.epoch != first.epoch
        || current.connection_epoch != first.connection_epoch
        || current.generation != first.generation
        || current_terminal != terminal
        || current_incarnation != incarnation
        || current.observation_sequence <= first.observation_sequence
    {
        return Err(error(
            ErrorCode::StaleHostObservation,
            "empty target changed before launch",
        ));
    }
    if budget.is_exhausted(clock) {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "launch preflight expired",
        ));
    }
    let native_request = NativeLaunchRequest {
        process_hint: policy.requires_process_hint(),
        seat,
        target: request.target,
        harness: request.harness,
        argv,
        configured_hook: hook,
        expected_terminal: current_terminal,
        expected_generation: current.generation,
        expected_incarnation: current_incarnation,
        name_hint: request.name_hint,
    };
    native_request
        .validate()
        .map_err(|detail| error(ErrorCode::InvalidRequest, detail))?;
    let native_context = HostCallContext {
        budget: budget.clone(),
        expected_boot: Some(current.host_boot.clone()),
        expected_epoch: Some(current.epoch),
    };
    Ok(PreparedLaunch {
        request: native_request,
        context: native_context,
        current,
    })
}

/// Called only after preparation and any caller's durable possible-start fence.
pub fn submit_prepared(
    host: &dyn HostPort,
    clock: &dyn Clock,
    prepared: PreparedLaunch,
) -> Result<NativeLaunchOutcome, ApiError> {
    match submit_prepared_with_evidence(host, clock, prepared) {
        Err(failure)
            if matches!(
                failure.error.code,
                ErrorCode::HostUnavailable
                    | ErrorCode::DeadlineExceeded
                    | ErrorCode::Cancelled
                    | ErrorCode::UnknownOutcome
            ) =>
        {
            Ok(NativeLaunchOutcome::OutcomeUnknown)
        }
        result => result.map_err(|failure| failure.error),
    }
}

pub fn submit_prepared_with_evidence(
    host: &dyn HostPort,
    clock: &dyn Clock,
    prepared: PreparedLaunch,
) -> Result<NativeLaunchOutcome, crate::ports::NativeLaunchFailure> {
    let PreparedLaunch {
        request: native_request,
        context: native_context,
        current,
    } = prepared;
    let budget = &native_context.budget;
    let outcome = host.launch_native_with_evidence(native_request.clone(), &native_context);
    // Once the host start call begins, a lost response or deadline cannot
    // establish that no agent was started. Do not advertise a safe retry.
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(err)
            if err.submission == crate::ports::NativeSubmission::Possible
                && matches!(
                    err.error.code,
                    ErrorCode::HostUnavailable
                        | ErrorCode::DeadlineExceeded
                        | ErrorCode::Cancelled
                        | ErrorCode::UnknownOutcome
                ) =>
        {
            return Ok(NativeLaunchOutcome::OutcomeUnknown);
        }
        Err(err) => return Err(err),
    };
    if budget.is_exhausted(clock) {
        return Ok(NativeLaunchOutcome::OutcomeUnknown);
    }
    match outcome {
        NativeLaunchOutcome::ObservedStartup {
            correlation,
            diagnostic,
        } if correlation.matches_request(&native_request, &native_context)
            && diagnostic.target == current.target
            && diagnostic.host_boot == current.host_boot
            && diagnostic.epoch == current.epoch
            && diagnostic.terminal == current.terminal
            && diagnostic.occupancy == StructuralOccupancy::Occupied
            && diagnostic.completed_at_mono >= correlation.completed_at_mono =>
        {
            Ok(NativeLaunchOutcome::ObservedStartup {
                correlation,
                diagnostic,
            })
        }
        NativeLaunchOutcome::ObservedStartup { .. } | NativeLaunchOutcome::OutcomeUnknown => {
            Ok(NativeLaunchOutcome::OutcomeUnknown)
        }
    }
}

#[cfg(test)]
#[path = "../../tests/harness/launch.rs"]
pub(crate) mod tests;

/// Resolve only the adapter-declared config variable, retaining the existing pane-shell fallback.
pub(crate) fn native_scope(
    request: &super::adapter::LaunchRequest,
    id: &str,
    variable: &str,
    probe: &dyn CodexShellProbe,
    budget: &CallBudget,
) -> Result<super::adapter::LaunchScope, ApiError> {
    if budget.is_exhausted(request.environment.clock.as_ref()) {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "launch budget exhausted",
        ));
    }
    let pane = probe
        .pane_shell_env_bounded(variable, request.environment.clock.as_ref(), budget)
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    let source = if pane.is_some() {
        "pane_shell"
    } else {
        "launcher"
    };
    let root = pane
        .or_else(|| request.environment.config_roots.get(id).cloned())
        .ok_or_else(|| error(ErrorCode::MissingHook, "native config root unavailable"))?;
    Ok(super::adapter::LaunchScope {
        setup: super::adapter::ResolvedSetupScope::ConfigRoot(root),
        working_directory: request.environment.cwd.clone(),
        config_source: source,
    })
}

pub(crate) fn owned_launch_hook(
    status: &super::adapter::LocalSetupStatus,
) -> Result<ConfiguredHook, ApiError> {
    status.configured_hook.clone().filter(|_| status.installed && status.enabled != Some(false) && status.admitted != Some(false))
        .ok_or_else(|| {
            let file = status.projection["settings"].as_str().or_else(|| status.projection["hooks_file"].as_str()).unwrap_or("the selected scope");
            error(ErrorCode::MissingHook, &format!("supported hook is not configured in {file}; run `herdr-threads setup` first with the config directory the agent uses"))
        })
}

/// Fingerprint the concrete adapter's selected files and actual owned hook inspection.
pub(crate) fn native_configuration_fingerprint(
    request: &super::adapter::LaunchRequest,
    scope: &super::adapter::LaunchScope,
    harness: super::context::Harness,
    files: &[&str],
) -> Result<String, ApiError> {
    use sha2::{Digest, Sha256};
    let hook = native_configuration_hook(request, scope, harness)?;
    let super::adapter::ResolvedSetupScope::ConfigRoot(root) = &scope.setup else {
        return Err(error(
            ErrorCode::InvalidRequest,
            "legacy launch requires a config root",
        ));
    };
    let mut hash = Sha256::new();
    hash.update(hook.fingerprint.as_bytes());
    for file in files {
        hash.update(file.as_bytes());
        match std::fs::read(root.join(file)) {
            Ok(bytes) => {
                hash.update([1]);
                hash.update(bytes);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(err) => return Err(error(ErrorCode::Conflict, &err.to_string())),
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Reinspect the descriptor consumed by concrete preparation, independently of the status snapshot.
pub(crate) fn native_configuration_hook(
    request: &super::adapter::LaunchRequest,
    scope: &super::adapter::LaunchScope,
    harness: super::context::Harness,
) -> Result<ConfiguredHook, ApiError> {
    let env = super::setup::legacy::scoped_legacy_environment(
        harness,
        &scope.setup,
        &request.environment,
    )
    .map_err(|err| error(ErrorCode::Conflict, &err.to_string()))?;
    let (_, inspection) = crate::cli::setup::user_inspection(harness, &env)
        .map_err(|err| error(ErrorCode::Conflict, &err))?;
    let hook = inspection
        .configured_hook
        .ok_or_else(|| error(ErrorCode::Conflict, "owned launch hooks changed"))?;
    Ok(hook)
}
