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

const MAX_LAUNCH_MILLIS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLaunchRequest {
    pub target: HostTargetId,
    pub harness: Harness,
    /// Native agent arguments; their order and bytes are retained.
    pub argv: Vec<String>,
    /// Codex only: the pane's shell resolves `codex` to a wrapper (function
    /// or alias) that already passes `--no-daemon`, so the composed argv
    /// must not carry one (Codex refuses a repeated `--no-daemon`).
    pub shell_passes_no_daemon: bool,
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

/// Codex options (root, `exec` and `resume` levels) that consume the next
/// argument as their value. Any other `-`/`--` option is taken as a switch;
/// `--flag=value` spellings never consume the next argument. `-i/--image`
/// takes one or more values, so its separated spelling is refused
/// ([`CODEX_MULTI_VALUE_OPTIONS`]) rather than guessing its arity.
pub(super) const CODEX_VALUE_OPTIONS: &[&str] = &[
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

/// Codex options taking a variable number of values (codex-cli 0.159.2
/// `-i, --image <FILE>...`): the separated spelling would swallow a
/// following subcommand or prompt, so only `--image=FILE` (or the option
/// after `--`) is accepted.
const CODEX_MULTI_VALUE_OPTIONS: &[&str] = &["-i", "--image"];

/// Codex subcommands a managed launch cannot configure: refused rather than
/// started without the owned hooks. `exec` is a handled form; `resume` is refused until captured.
/// Covers every top-level subcommand and alias `codex --help` lists for
/// codex-cli 0.159.2 (plus older names), so a bare first positional naming a
/// Codex subcommand is never mistaken for an interactive prompt; a prompt
/// that is such a word goes after `--`.
pub(super) const CODEX_UNSUPPORTED_SUBCOMMANDS: &[&str] = &[
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

/// `codex exec` subcommands other than `resume` (codex-cli 0.159.2
/// `codex exec --help`): refused, since no evidence shows they read the
/// exec-level owned hooks.
pub(super) const CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS: &[&str] = &["fork", "review", "help"];

/// Where a managed Codex launch places the owned `-c` configuration. Codex
/// 0.159.2 `exec` ignores root-level `hooks.*` overrides
/// (`native-codex-matrix-1/hook-placement-probe`), so each subcommand form
/// carries them at its own level; `--no-daemon` is a top-level flag and
/// always precedes the subcommand.
///
/// Evidence per form:
/// - `Interactive`: `codex --no-daemon -c hooks.* [PROMPT]`, the launch line
///   `setup codex` prints (root-level session overrides);
/// - `Exec`: `codex --no-daemon exec -c hooks.* ... PROMPT`
///   (`hook-placement-probe/exec.jsonl`, `codex-158-live-hook-capture/run1.sh`);
/// - `ExecResume`: `codex --no-daemon exec ... resume ... -c hooks.* ID PROMPT`
///   (`codex-158-live-hook-capture/run3.sh`, SessionStart resume captured);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexLaunchForm {
    Interactive,
    Exec,
    ExecResume,
}

/// The explicit working directory whose project configuration a scoped
/// sandbox probe can reproduce. An implicit pane cwd is not inferred from
/// the coordinator process or saved pane paths.
pub fn scoped_codex_cwd(argv: &[String]) -> Result<std::path::PathBuf, ApiError> {
    let options_end = argv
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(argv.len());
    let mut found = None;
    let mut index = 0;
    while index < options_end {
        let arg = argv[index].as_str();
        let cwd = if arg == "-C" || arg == "--cd" {
            argv.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--cd=")
                .or_else(|| arg.strip_prefix("-C="))
        };
        if let Some(cwd) = cwd {
            let path = std::path::PathBuf::from(cwd);
            if found.is_some() || !path.is_absolute() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "scoped Codex sandbox validation needs one absolute -C directory",
                ));
            }
            found = Some(path);
            index += if arg == "-C" || arg == "--cd" { 2 } else { 1 };
        } else {
            index += if CODEX_VALUE_OPTIONS.contains(&arg) {
                2
            } else {
                1
            };
        }
    }
    found.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "scoped Codex sandbox validation needs an explicit -C /absolute/project/path to match project policy",
        )
    })
}

/// The next positional argument at or after `start`: its index and whether
/// it follows `--` (and so is a prompt, never a subcommand).
fn next_positional(argv: &[String], start: usize) -> Option<(usize, bool)> {
    let mut index = start;
    while index < argv.len() {
        let arg = argv[index].as_str();
        if arg == "--" {
            return (index + 1 < argv.len()).then_some((index + 1, true));
        }
        if arg.len() > 1 && arg.starts_with('-') {
            index += if CODEX_VALUE_OPTIONS.contains(&arg) {
                2
            } else {
                1
            };
            continue;
        }
        return Some((index, false));
    }
    None
}

/// The caller's Codex form and the index at which the owned configuration is
/// inserted (right after the subcommand that must carry it).
fn codex_form(argv: &[String]) -> Result<(CodexLaunchForm, usize, Option<usize>), ApiError> {
    let Some((first, after_separator)) = next_positional(argv, 0) else {
        return Ok((CodexLaunchForm::Interactive, 0, None));
    };
    if after_separator {
        return Ok((CodexLaunchForm::Interactive, 0, None));
    }
    match argv[first].as_str() {
        "exec" => match next_positional(argv, first + 1) {
            Some((second, false)) if argv[second] == "resume" => {
                Ok((CodexLaunchForm::ExecResume, second + 1, Some(first)))
            }
            // Only `exec resume` has evidence of reading its own hook level;
            // every other exec subcommand (0.159.2: `fork`, `review`, `help`)
            // is refused rather than started with unverified hook placement.
            Some((second, false))
                if CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS.contains(&argv[second].as_str()) =>
            {
                Err(error(
                    ErrorCode::InvalidRequest,
                    "managed launch supports Codex `exec` and `exec resume` only; this exec \
                     subcommand cannot carry the owned hook configuration",
                ))
            }
            _ => Ok((CodexLaunchForm::Exec, first + 1, Some(first))),
        },
        "resume" => Err(error(
            ErrorCode::InvalidRequest,
            "managed launch refuses the Codex `resume` form: no live capture shows it loading the owned hooks (TRUST-POLICY Accepted limits); run `codex resume` by hand in the pane, or use `exec resume`",
        )),
        word if CODEX_UNSUPPORTED_SUBCOMMANDS.contains(&word) => Err(error(
            ErrorCode::InvalidRequest,
            "managed launch supports Codex interactive, `exec` and `exec resume` only; \
             this subcommand cannot carry the owned hook configuration",
        )),
        _ => {
            // Interactive with a prompt: Codex takes one prompt, so a second
            // positional means the arguments were misread (an unknown option
            // taking a value before a subcommand). Refuse rather than place
            // the owned hooks where the subcommand would ignore them.
            if next_positional(argv, first + 1).is_some() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "ambiguous Codex arguments: more than one positional argument before a \
                     recognised subcommand; put the prompt after `--`",
                ));
            }
            Ok((CodexLaunchForm::Interactive, 0, None))
        }
    }
}

/// The native argument array a managed launch submits: for Claude the owned
/// arguments then the caller's; for Codex `--no-daemon` exactly once at the
/// top level, then the caller's arguments byte for byte and in order with the
/// owned configuration inserted at the level of the caller's subcommand
/// ([`CodexLaunchForm`]). An owned `--no-daemon` is dropped, never
/// duplicated. Unsupported subcommands, conflicting daemon modes, a
/// misplaced `--no-daemon` and caller `hooks.*` overrides are refused.
pub fn compose_native_argv(
    harness: Harness,
    caller: Vec<String>,
    owned: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    compose_native_argv_with(harness, caller, owned, false)
}

/// [`compose_native_argv`] for a pane whose shell wrapper may already pass
/// `--no-daemon`. With `shell_passes_no_daemon` the composed Codex argv
/// carries no `--no-daemon` at all (the wrapper supplies the single one), so
/// a caller's own top-level `--no-daemon` is dropped too; every other check
/// and placement is unchanged.
pub fn compose_native_argv_with(
    harness: Harness,
    caller: Vec<String>,
    owned: Vec<String>,
    shell_passes_no_daemon: bool,
) -> Result<Vec<String>, ApiError> {
    if harness != Harness::Codex {
        return Ok(owned.into_iter().chain(caller).collect());
    }
    let options_end = caller
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(caller.len());
    let options = &caller[..options_end];
    if options.iter().any(|arg| {
        arg == "--daemon" || arg.starts_with("--daemon=") || arg.starts_with("--no-daemon=")
    }) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "conflicting Codex daemon mode",
        ));
    }
    // Codex applies repeated `-c` values in order within the session layer,
    // so a caller hook override (a `hooks.*` key or the whole `hooks` table)
    // would silently replace the owned hook.
    let scoped_socket_policy = owned
        .iter()
        .any(|arg| arg == "sandbox_workspace_write.network_access=true");
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
            return Err(error(
                ErrorCode::InvalidRequest,
                "caller Codex hooks override would replace the owned hook configuration",
            ));
        }
        if scoped_socket_policy
            && (value.is_some()
                || (!previous_takes_value
                    && matches!(
                        arg.as_str(),
                        "-c" | "--config"
                            | "-p"
                            | "--profile"
                            | "-s"
                            | "--sandbox"
                            | "--enable"
                            | "--disable"
                            | "--add-dir"
                            | "--remote"
                            | "--remote-auth-token-env"
                            | "--worktree"
                            | "--approve-for-me"
                            | "--dangerously-bypass-approvals-and-sandbox"
                            | "--yolo"
                            | "--dangerously-bypass-hook-trust"
                            | "--full-auto"
                            | "--search"
                    ))
                || (!previous_takes_value
                    && [
                        "--profile=",
                        "--sandbox=",
                        "--enable=",
                        "--disable=",
                        "--add-dir=",
                        "--remote=",
                        "--remote-auth-token-env=",
                    ]
                    .iter()
                    .any(|prefix| arg.starts_with(prefix)))
                || (!previous_takes_value && (arg.starts_with("-s") || arg.starts_with("-p"))))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "caller Codex policy or profile override would differ from the measured scoped sandbox policy",
            ));
        }
        if !previous_takes_value && CODEX_MULTI_VALUE_OPTIONS.contains(&arg.as_str()) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "ambiguous Codex arguments: put images as --image=FILE or before --",
            ));
        }
        previous_is_config = !previous_takes_value && (arg == "-c" || arg == "--config");
        previous_takes_value = !previous_takes_value && CODEX_VALUE_OPTIONS.contains(&arg.as_str());
    }
    let (_, insert_at, subcommand) = codex_form(&caller)?;
    let no_daemon: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, arg)| *arg == "--no-daemon")
        .map(|(index, _)| index)
        .collect();
    if no_daemon.len() > 1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "duplicate Codex --no-daemon",
        ));
    }
    if let (Some(&at), Some(subcommand)) = (no_daemon.first(), subcommand)
        && at > subcommand
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "Codex --no-daemon is a top-level flag; it must precede the subcommand",
        ));
    }
    let owned = owned.into_iter().filter(|arg| arg != "--no-daemon");
    let mut argv = Vec::with_capacity(caller.len() + 8);
    let (caller, insert_at) = match (shell_passes_no_daemon, no_daemon.first()) {
        (true, Some(&at)) => {
            // The wrapper's flag is the single one; removing a top-level
            // switch never changes the subcommand form.
            let mut caller = caller;
            caller.remove(at);
            (
                caller,
                if at < insert_at {
                    insert_at - 1
                } else {
                    insert_at
                },
            )
        }
        (true, None) => (caller, insert_at),
        (false, _) => {
            if no_daemon.is_empty() {
                argv.push("--no-daemon".to_owned());
            }
            (caller, insert_at)
        }
    };
    let mut caller = caller.into_iter();
    argv.extend(caller.by_ref().take(insert_at));
    argv.extend(owned);
    argv.extend(caller);
    Ok(argv)
}

/// Whether a Codex `-c` value sets the `hooks` table or a key under it: the
/// key is the text before the first `=`, trimmed.
fn overrides_hooks(value: &str) -> bool {
    let key = value.split('=').next().unwrap_or("").trim();
    key == "hooks" || key.starts_with("hooks.")
}

/// Fully guarded preparation; must be submitted immediately after the durable gate.
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
    // Refuse an unconfigurable form before any host or seat work.
    compose_native_argv_with(
        request.harness,
        request.argv.clone(),
        Vec::new(),
        request.shell_passes_no_daemon,
    )?;
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
    if let Some(bound) = seats.open_binding(&seat, &budget)?
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
            .filter(|kind| crate::protocol::authority::is_harness_agent_kind(kind))
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
    let configuration = hooks
        .launch_configuration(request.harness, &budget)?
        .ok_or_else(|| error(ErrorCode::MissingHook, "supported hook is not configured"))?;
    let hook = configuration.hook;
    // Owned configuration at the caller's subcommand level (Codex) or first
    // (Claude); the caller's arguments keep their bytes and order.
    let argv = compose_native_argv_with(
        request.harness,
        request.argv,
        configuration.argv,
        request.shell_passes_no_daemon,
    )?;
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
mod tests;
