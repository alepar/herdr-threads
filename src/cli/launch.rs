//! `launch --pane PANE --kind claude|codex [-- AGENT_ARG...]`:
//! the application composition of the managed native launch policy
//! (`harness::launch::launch_managed`).
//!
//! Preflight, in order:
//! 1. resolve the selected executable and declare its registered contract,
//!    without invoking diagnostic flags or inferring runtime identity;
//! 2. inside the policy: a fresh explicit-target read that must be an
//!    available shell, the pane's seat resolved through the daemon's ordinary
//!    guarded `seat resolve` path (a recovery hold refuses), the owned
//!    user-level hook installation inspected (`setup` must have installed it
//!    in the Claude settings / Codex hooks.json this environment resolves;
//!    Codex commands use approved outside-sandbox execution, without requiring
//!    a network allowance), and a last fenced recheck;
//! 3. Herdr's guarded `agent.start` with the argument array: the caller's
//!    arguments byte for byte and in order (the owned configuration is on
//!    disk, so launch adds none), and Codex `--no-daemon` exactly once at the
//!    top level (`harness::launch::compose_native_argv`), or not at all when
//!    the pane shell's `codex` wrapper already passes it ([`CodexShellProbe`]).
//!
//! Launch never registers, accepts or ACKs. After an accepted, correlated
//! startup it asks the daemon to record a `managed_launch` binding on a seat
//! with no open binding (TRUST-POLICY A3): an unregistered occupant record
//! that authorizes nothing but a wake prompt to the launched harness, so an
//! agent that never checks in before its first turn (Codex 0.159.3 TUI) is
//! still woken for pending work; its first lifecycle check-in replaces it.
//! Invitations and messages that were committed before launch stay pending
//! and are surfaced by the SessionStart hook's check-in, or by `inbox`, even
//! when the agent's initial prompt is lost. Each submitted start is appended
//! to a private launch record in the instance directory.

#[cfg(test)]
use super::setup::{self, SetupRequest, SetupVerb};
use super::{
    RunError,
    journal::{IntentScope, Journal, SemanticMutation},
    retry,
    setup::SetupEnv,
};
use crate::{
    client::local::LocalSocketClient,
    harness::{
        context::Harness as ContextHarness,
        launch::{
            LaunchHookInspector, LaunchSeatResolver, ManagedLaunchRequest, OpenBinding,
            managed_launch_command, submit_prepared, submit_prepared_with_evidence,
        },
    },
    host::observation::PaneName,
    ports::{
        ConfiguredHook, CorrelatedStartup, HostObservation, HostPort, LocalClient,
        NativeLaunchOutcome,
    },
    protocol::{
        authority::Harness,
        commands::{Command, InboxQuery},
        ids::{HostTargetId, SeatId},
        output::OutputSpec,
        pagination::PageRequest,
        results::{ApiError, CommandResult, ErrorCode, ManagedLaunchRecord},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Mutex,
};

pub use crate::harness::codex::launch::{
    CODEX_WRAPPER_NO_DAEMON, codex_profile, wrapper_passes_no_daemon,
};
pub use crate::harness::launch::{CodexShellProbe, SHELL_PROBE_TIMEOUT, SystemShellProbe};

/// `launch --help` epilogue.
pub const LAUNCH_HELP: &str = "Target:
  --pane names one existing Herdr pane at its interactive shell prompt; launch never
  creates, splits or picks a pane and never types into an occupied one. Herdr's
  guarded `agent start` refuses a busy pane.

Preflight (nothing is started when any step refuses):
  - the selected executable must resolve; its registered contract is declared without
    invoking --version, --help or schema probes, and runtime metadata remains unknown;
  - the pane's seat is resolved like `seat resolve --pane`; a recovery-held target
    needs `seat rebind ... --operator` or a fresh seat first;
  - the owned user-level hooks must be set up (`herdr-threads setup claude|codex`) in the
    Claude settings / Codex hooks.json of the CLAUDE_CONFIG_DIR / CODEX_HOME the agent
    will use (an absolute one the pane's shell exports, else launch's own, else HOME;
    the report's config_dir names it); Codex CLI commands use approved
    outside-sandbox execution, without a network/socket allowance;
  - a last fresh read of the pane just before Herdr starts the agent.

Agent name: --name NAME, else the pane's Herdr label, else s<short seat id>
for a new compact ID (seat-<short seat id> for a persisted ID),
fitted to Herdr's [a-z][a-z0-9_-]{0,31}. If another live agent holds it, launch
retries once with -<short seat id> appended.

Arguments after `--` are passed to the agent unchanged and in order; the owned
configuration is on disk, so launch adds no hook arguments. Codex gets `--no-daemon`
exactly once before any subcommand; when the pane shell's `codex` function or alias
already passes it (probed with `$SHELL -ic`, 3 s bound), launch adds none and reports
`codex_wrapper`. Other Codex subcommands, an explicit `--daemon`, a
`--no-daemon` after the subcommand and a caller `-c hooks.*` override are refused. No
auto-approve flag is added.

Launch is not receipt: it never checks in, accepts or ACKs. Invitations and messages
sent before launch stay pending; the agent's SessionStart hook shows them, and
`herdr-threads inbox --seat SEAT` lists them even when the initial prompt is lost.
After an observed start, a seat with no open binding gets a `managed_launch` binding
(report `binding`; `seat inspect` shows it as launched, not checked in): it lets the
daemon wake the idle agent for pending work before its first check-in (Codex runs no
SessionStart hook until its first turn), and the agent's first check-in replaces it.

A harness that exits right after the start (for example Codex refusing its arguments)
fails launch at once with invalid_request and the pane's last lines. Codex launches report
`codex`: the effective CODEX_HOME, its config.toml and the selected profile (-p/--profile,
else `profile` in config.toml, else default).

Exit status: 0 startup observed; 5 start may have happened but was not confirmed:
inspect the pane (`herdr agent get` / `herdr agent read`) before launching again.";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaunchRequest {
    pub target: HostTargetId,
    pub harness: ContextHarness,
    pub harness_binary: Option<String>,
    /// Caller native arguments, byte for byte and in order.
    pub argv: Vec<String>,
    /// `--name`: the Herdr agent name wanted (sanitized to Herdr's rules).
    pub name: Option<String>,
    /// The target pane's Herdr label (else its single-pane tab's label),
    /// filled in by the composition when `--name` is absent. Display only.
    pub pane_label: Option<String>,
}

impl LaunchRequest {
    /// The agent name hint and where it came from: `--name`, else the pane
    /// label, else none (the adapter then uses the short seat id).
    pub fn name_hint(&self) -> (Option<String>, &'static str) {
        let usable = |name: &Option<String>| {
            name.as_deref()
                .and_then(crate::ports::sanitize_agent_name)
                .is_some()
        };
        if usable(&self.name) {
            (self.name.clone(), "name")
        } else if usable(&self.pane_label) {
            (self.pane_label.clone(), "pane_label")
        } else {
            (None, "seat")
        }
    }
}

/// The label naming `target` for its agent: the pane's own label, else the
/// label of its tab when the tab holds only this pane.
pub fn pane_label(target: &HostTargetId, panes: &[PaneName]) -> Option<String> {
    let pane = panes.iter().find(|pane| &pane.target == target)?;
    pane.label.clone().or_else(|| {
        (pane.tab_pane_count == 1)
            .then(|| pane.tab_label.clone())
            .flatten()
    })
}

fn api(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

fn policy_harness(harness: ContextHarness) -> Harness {
    harness.into()
}

struct PreparedHookInspector(ConfiguredHook);
impl LaunchHookInspector for PreparedHookInspector {
    fn configured_hook(
        &self,
        _: Harness,
        _: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError> {
        Ok(Some(self.0.clone()))
    }
}

/// The daemon's ordinary guarded `seat resolve --pane` (the same journaled
/// intent the CLI command records), so a recovery hold or unresolved
/// continuity claim refuses exactly as it does for `seat resolve`.
pub struct DaemonSeatResolver<'a> {
    client: &'a LocalSocketClient,
    journal: &'a Journal,
    instance: uuid::Uuid,
    clock: &'a dyn Clock,
}

impl<'a> DaemonSeatResolver<'a> {
    pub fn new(
        client: &'a LocalSocketClient,
        journal: &'a Journal,
        instance: uuid::Uuid,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            client,
            journal,
            instance,
            clock,
        }
    }
}

impl LaunchSeatResolver for DaemonSeatResolver<'_> {
    fn resolve_for_launch(
        &self,
        target: &HostTargetId,
        _observation: &HostObservation,
        budget: &CallBudget,
    ) -> Result<SeatId, ApiError> {
        let output = OutputSpec::default();
        let (_, result) = retry::run_new_api_to_writer_discarding_rejection(
            self.journal,
            IntentScope::ServiceAllocation {
                instance: self.instance.to_string(),
                target: target.clone(),
            },
            SemanticMutation::ResolveSeat {
                target: target.clone(),
            },
            self.clock.utc_now().0,
            || {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "ordinary resolution has no native claim",
                ))
            },
            |command| {
                self.client
                    .call_with_output_definitive(command, &output, budget)
            },
            &output,
            &mut io::sink(),
        )
        .map_err(|failure| match failure {
            retry::RetryFailure::Local(error) => api(
                ErrorCode::InvalidRequest,
                format!("local intent journal: {error}"),
            ),
            retry::RetryFailure::Submit(error) => error,
        })?;
        match result {
            CommandResult::SeatResolved(seat) => Ok(seat),
            _ => Err(api(
                ErrorCode::TargetUnresolved,
                "service returned no resolved seat",
            )),
        }
    }

    fn open_binding(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<OpenBinding>, ApiError> {
        open_binding_of(self.client, seat, budget)
    }

    fn record_managed_launch(
        &self,
        startup: &CorrelatedStartup,
        budget: &CallBudget,
    ) -> Result<Option<ManagedLaunchRecord>, ApiError> {
        record_managed_launch_with(self.client, startup, budget)
    }
}

/// Send one accepted startup to a daemon that records managed launches; an
/// older daemon (no `seat.managed_launch` capability) is never sent the
/// command it cannot decode.
fn record_managed_launch_with(
    client: &LocalSocketClient,
    startup: &CorrelatedStartup,
    budget: &CallBudget,
) -> Result<Option<ManagedLaunchRecord>, ApiError> {
    if !client
        .capabilities(budget)
        .supports(crate::protocol::capabilities::SEAT_MANAGED_LAUNCH)
    {
        return Ok(None);
    }
    match client.call(
        Command::RecordManagedLaunch(managed_launch_command(startup)),
        budget,
    )? {
        CommandResult::ManagedLaunchRecorded(record) => Ok(Some(record)),
        _ => Err(api(
            ErrorCode::InvalidRequest,
            "service returned no managed launch record",
        )),
    }
}

/// The seat's open binding from one `SeatInspect` call (`limit: 1`): the
/// daemon's answer does not depend on how long the seat's history is.
fn open_binding_of<C: LocalClient + ?Sized>(
    client: &C,
    seat: &SeatId,
    budget: &CallBudget,
) -> Result<Option<OpenBinding>, ApiError> {
    let result = client.call(
        Command::SeatInspect(crate::protocol::commands::SeatInspectQuery {
            seat: seat.clone(),
            page: PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            },
        }),
        budget,
    )?;
    let CommandResult::SeatInspect(inspection) = result else {
        return Err(api(
            ErrorCode::InvalidRequest,
            "service returned no seat inspection",
        ));
    };
    Ok(inspection.open_binding.map(|bound| OpenBinding {
        target: bound.target,
        provenance: bound.provenance,
    }))
}

/// Remembers the seat the policy resolved, for the report and the record.
struct RecordingResolver<'a> {
    inner: &'a dyn LaunchSeatResolver,
    seat: Mutex<Option<SeatId>>,
}
impl LaunchSeatResolver for RecordingResolver<'_> {
    fn resolve_for_launch(
        &self,
        target: &HostTargetId,
        observation: &HostObservation,
        budget: &CallBudget,
    ) -> Result<SeatId, ApiError> {
        let seat = self.inner.resolve_for_launch(target, observation, budget)?;
        if let Ok(mut slot) = self.seat.lock() {
            *slot = Some(seat.clone());
        }
        Ok(seat)
    }

    fn open_binding(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<OpenBinding>, ApiError> {
        self.inner.open_binding(seat, budget)
    }

    fn record_managed_launch(
        &self,
        startup: &CorrelatedStartup,
        budget: &CallBudget,
    ) -> Result<Option<ManagedLaunchRecord>, ApiError> {
        self.inner.record_managed_launch(startup, budget)
    }
}

/// The report's `binding` block for an accepted startup: whether the daemon
/// opened the seat's `managed_launch` binding (TRUST-POLICY A3), or why not.
/// Never a launch failure: the agent started either way.
fn binding_report(
    seats: &dyn LaunchSeatResolver,
    startup: &CorrelatedStartup,
    clock: &dyn Clock,
) -> Value {
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5_000)),
        cancellation: Cancellation::default(),
    };
    match seats.record_managed_launch(startup, &budget) {
        Ok(Some(record)) if record.recorded => json!({
            "recorded": true,
            "state": MANAGED_LAUNCH_STATE,
            "provenance": record.provenance,
            "binding_generation": record.binding_generation,
        }),
        Ok(Some(record)) => json!({
            "recorded": false,
            "state": if record.provenance == crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE {
                MANAGED_LAUNCH_STATE
            } else {
                "checked in"
            },
            "provenance": record.provenance,
            "binding_generation": record.binding_generation,
            "note": "the seat already had an open binding; it was left unchanged",
        }),
        Ok(None) => json!({
            "recorded": false,
            "note": "this daemon does not record managed launches (no seat.managed_launch \
                     capability): the seat stays unbound until the agent checks in, so a lost \
                     initial prompt is not recovered by a wake before then",
        }),
        Err(error) => json!({
            "recorded": false,
            "note": format!(
                "the launch binding was not recorded ({:?}: {}); the agent started and \
                 registers at its first check-in",
                error.code, error.detail
            ),
        }),
    }
}

/// How `launch`, `seat inspect` and `me init` name a `managed_launch` binding.
pub const MANAGED_LAUNCH_STATE: &str = "launched, not checked in";

/// Pending handoff of the launched seat, read after launch (never mutated).
pub trait HandoffReader {
    fn pending(&self, seat: &SeatId, budget: &CallBudget) -> Result<Value, ApiError>;
}

impl<C: LocalClient + ?Sized> HandoffReader for C {
    fn pending(&self, seat: &SeatId, budget: &CallBudget) -> Result<Value, ApiError> {
        let result = self.call(
            Command::Inbox(InboxQuery {
                seat: Some(seat.clone()),
                page: PageRequest {
                    cursor: None,
                    limit: 16,
                    max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
                },
            }),
            budget,
        )?;
        let CommandResult::Inbox(page) = result else {
            return Err(api(ErrorCode::InvalidRequest, "service returned no inbox"));
        };
        let invitations: u64 = page.items.iter().map(|item| item.invitations).sum();
        let receipts: u64 = page.items.iter().map(|item| item.pending_receipts).sum();
        Ok(json!({
            "threads": page.items.iter().map(|item| item.thread.as_str()).collect::<Vec<_>>(),
            "pending_invitations": invitations,
            "pending_receipts": receipts,
            "has_more": page.has_more,
        }))
    }
}

/// Everything one launch composes; the host and daemon ports are injected.
pub struct LaunchParts<'a> {
    pub env: &'a SetupEnv,
    pub host: &'a dyn HostPort,
    pub seats: &'a dyn LaunchSeatResolver,
    pub handoff: &'a dyn HandoffReader,
    pub clock: &'a dyn Clock,
    /// Directory holding the private launch record (`launches.jsonl`).
    pub record_dir: Option<&'a Path>,
    /// How the pane shell resolves `codex` (consulted for Codex only).
    pub shell_probe: &'a dyn CodexShellProbe,
}

/// The finished report and the process exit status (0 observed startup,
/// 5 possible start).
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchReport {
    pub report: Value,
    pub exit: i32,
}

fn append_record(dir: &Path, record: &Value) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
    line.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(dir.join("launches.jsonl"))?;
    file.write_all(&line)
}

/// Adapter-local report keys are additive; canonical launch evidence keeps its own fields.
fn append_adapter_projection(report: &mut Value, projection: &Value) {
    if let (Some(report), Some(projection)) = (report.as_object_mut(), projection.as_object()) {
        for (key, value) in projection {
            report.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

/// Run the managed launch preflight and start; errors are refusals before
/// any start was submitted.
pub fn execute(request: &LaunchRequest, parts: &LaunchParts<'_>) -> Result<LaunchReport, RunError> {
    execute_with_registry(crate::harness::registry::builtins(), request, parts)
}

/// Injectable registry entrypoint uses exactly the same guarded composition as production.
pub fn execute_with_registry(
    registry: &crate::harness::registry::Registry,
    request: &LaunchRequest,
    parts: &LaunchParts<'_>,
) -> Result<LaunchReport, RunError> {
    execute_guarded_inner(registry, request, parts, false, false, &mut |_| Ok(()))
}

/// A preflight uses the exact launch guards but does not submit or record a start.
/// The final gate persists compound state immediately before native submission.
pub enum LaunchBoundary<'a> {
    BeforeSubmit(&'a crate::ports::NativeLaunchRequest),
    RefusedBeforeStart,
}

pub fn execute_guarded(
    request: &LaunchRequest,
    parts: &LaunchParts<'_>,
    preflight: bool,
    boundary: &mut dyn FnMut(LaunchBoundary<'_>) -> Result<(), ApiError>,
) -> Result<LaunchReport, RunError> {
    execute_guarded_inner(
        crate::harness::registry::builtins(),
        request,
        parts,
        preflight,
        true,
        boundary,
    )
}

fn execute_guarded_inner(
    registry: &crate::harness::registry::Registry,
    request: &LaunchRequest,
    parts: &LaunchParts<'_>,
    preflight: bool,
    typed_evidence: bool,
    boundary: &mut dyn FnMut(LaunchBoundary<'_>) -> Result<(), ApiError>,
) -> Result<LaunchReport, RunError> {
    use crate::harness::adapter::{
        AdmissionRequest, ExecutableLookup, InstallEnvironment, InstallObservation,
        LaunchRequest as AdapterLaunchRequest, ResolvedSetupScope, SetupStatus, StatusRequest,
    };
    let registration =
        crate::harness::launch::launch_registration(registry, policy_harness(request.harness))?;
    let policy = registration.launch_policy().expect("checked provider");
    let word = registration.metadata().id;
    // Snapshot starts one decreasing wall-clock budget before any native probe.
    let mut adapter_request = AdapterLaunchRequest {
        argv: request.argv.clone(),
        environment: parts.env.snapshot(),
        native_binary: request
            .harness_binary
            .as_ref()
            .map(std::path::PathBuf::from),
    };
    let local_clock = adapter_request.environment.clock.clone();
    let local_budget = CallBudget {
        deadline: MonoInstant(local_clock.monotonic_now().0.saturating_add(30_000)),
        cancellation: Cancellation::default(),
    };
    policy.validate_native_argv(&request.argv)?;
    let scope = policy.resolve_scope(&adapter_request, parts.shell_probe, &local_budget)?;
    let root = match &scope.setup {
        ResolvedSetupScope::ConfigRoot(root) => root,
        ResolvedSetupScope::Profile { home, .. } => home,
    };
    adapter_request
        .environment
        .config_roots
        .insert(word.into(), root.clone());
    let ExecutableLookup::Path(executable_name) = registration.metadata().executable else {
        return Err(api(
            ErrorCode::UnsupportedHarness,
            format!("{word}: executable lookup is unsupported"),
        )
        .into());
    };
    let binary = crate::cli::hook::resolve_on_path(
        executable_name,
        adapter_request.environment.path.as_deref(),
    )
    .ok_or_else(|| {
        api(
            ErrorCode::UnsupportedHarness,
            format!("no executable `{executable_name}` on PATH"),
        )
    })?;
    if let Some(explicit) = &adapter_request.native_binary {
        if !explicit.is_absolute() {
            return Err(api(
                ErrorCode::InvalidRequest,
                "--harness-binary must be an absolute path",
            )
            .into());
        }
        use std::os::unix::fs::PermissionsExt;
        if !fs::metadata(explicit)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            return Err(api(
                ErrorCode::UnsupportedHarness,
                "selected harness binary is not an executable file",
            )
            .into());
        }
        if fs::canonicalize(explicit).ok() != fs::canonicalize(&binary).ok() {
            return Err(api(
                ErrorCode::InvalidRequest,
                "admitted executable must match the native command on PATH",
            )
            .into());
        }
    }
    let binary_identity = crate::harness::BinaryIdentity::observe(&binary).ok_or_else(|| {
        api(
            ErrorCode::UnsupportedHarness,
            "launch executable identity unavailable",
        )
    })?;
    let install_env = InstallEnvironment {
        clock: local_clock.clone(),
        path: adapter_request.environment.path.clone(),
        config_root: Some(root.clone()),
        state_dir: adapter_request.environment.state_dir.clone(),
    };
    let probe_budget = CallBudget {
        deadline: MonoInstant(
            local_budget
                .deadline
                .0
                .min(local_clock.monotonic_now().0.saturating_add(5_000)),
        ),
        cancellation: local_budget.cancellation.clone(),
    };
    let (admitted, prelaunch, version) = if policy.uses_prelaunch_observation() {
        (
            None,
            Some(registration.observe_prelaunch(
                &adapter_request,
                &scope,
                &binary,
                &local_budget,
            )?),
            None,
        )
    } else {
        let installed = registration.observe_install(&install_env, &probe_budget);
        let version = match &installed {
            InstallObservation::Available {
                identity,
                binary: observed,
            } => {
                if fs::canonicalize(observed).ok() != fs::canonicalize(&binary).ok() {
                    return Err(api(
                        ErrorCode::Conflict,
                        "installation observation executable mismatch",
                    )
                    .into());
                }
                identity.release_version.clone()
            }
            InstallObservation::CodexWitness(version) => Some(version.as_str().to_owned()),
            InstallObservation::ExecutableAvailable { binary: observed } => {
                if fs::canonicalize(observed).ok() != fs::canonicalize(&binary).ok() {
                    return Err(api(
                        ErrorCode::Conflict,
                        "installation observation executable mismatch",
                    )
                    .into());
                }
                None
            }
            InstallObservation::NotRequested => {
                return Err(api(
                    ErrorCode::UnsupportedHarness,
                    "launch requires executable availability",
                )
                .into());
            }
            InstallObservation::Unavailable { diagnostic } => {
                return Err(api(ErrorCode::UnsupportedHarness, diagnostic).into());
            }
            InstallObservation::Unsupported(operation) => {
                return Err(api(ErrorCode::UnsupportedHarness, operation.to_string()).into());
            }
        };
        let admitted = registration
            .admit(
                &AdmissionRequest {
                    installed,
                    input: None,
                    runtime_candidate: None,
                },
                &local_budget,
            )
            .map_err(|err| api(ErrorCode::UnsupportedHarness, err.to_string()))?;
        (Some(admitted), None, version)
    };
    let observed = crate::harness::setup::legacy::Observed {
        binary: binary.clone(),
        version,
        recipe: admitted
            .as_ref()
            .map_or("prelaunch_api_profile", |h| h.recipe()),
    };
    let status = match registration.status(
        &StatusRequest {
            scope: scope.setup.clone(),
            environment: adapter_request.environment.clone(),
            native_binary: Some(binary.clone()),
        },
        &local_budget,
    ) {
        SetupStatus::Detailed(status) => status,
        SetupStatus::Failed(error) => {
            return Err(api(ErrorCode::Conflict, error.to_string()).into());
        }
        _ => {
            return Err(api(
                ErrorCode::MissingHook,
                format!("{word}: launch setup status is unavailable"),
            )
            .into());
        }
    };
    let config_fingerprint = if status.installed
        && status.configured_hook.is_some()
        && status.enabled != Some(false)
        && status.admitted != Some(false)
    {
        Some(if let Some(handle) = &prelaunch {
            registration.recheck_prelaunch(&adapter_request, &scope, handle, &local_budget)?
        } else {
            policy.configuration_fingerprint(&adapter_request, &scope)?
        })
    } else {
        None
    };
    let preparation = if let Some(handle) = &prelaunch {
        registration.prepare_prelaunch(&adapter_request, &scope, handle, &status, parts.shell_probe, &local_budget)
    } else {
        registration.prepare_launch(&adapter_request, &scope, admitted.as_ref().expect("ordinary launch handle"), &status, parts.shell_probe, &local_budget)
    }
        .map_err(|mut error| {
            if error.code == ErrorCode::MissingHook {
                error.detail = format!(
                    "the owned {word} hooks are not installed in {} ({}): run `herdr-threads setup {word}` first, with the CLAUDE_CONFIG_DIR / CODEX_HOME the agent uses (nothing was started)",
                    root.display(), error.detail
                );
            }
            error
        })?;
    // The native host carries argv only; providers cannot silently request an environment it cannot submit.
    if !preparation.environment_overrides.is_empty() {
        return Err(api(
            ErrorCode::UnsupportedHarness,
            "native host cannot apply launch environment overrides",
        )
        .into());
    }
    if !registration
        .metadata()
        .host_kinds
        .iter()
        .any(|kind| policy.expected_host_kinds().contains(kind))
    {
        return Err(api(
            ErrorCode::UnsupportedHarness,
            "launch policy has no registered host kind",
        )
        .into());
    }
    let config_fingerprint = config_fingerprint.ok_or_else(|| {
        api(
            ErrorCode::MissingHook,
            "owned launch configuration unavailable",
        )
    })?;
    let inspector = PreparedHookInspector(preparation.hook.clone());
    let seats = RecordingResolver {
        inner: parts.seats,
        seat: Mutex::new(None),
    };
    let remaining = local_budget
        .deadline
        .0
        .saturating_sub(local_clock.monotonic_now().0);
    let budget = CallBudget {
        deadline: MonoInstant(parts.clock.monotonic_now().0.saturating_add(remaining)),
        cancellation: local_budget.cancellation.clone(),
    };
    let codex_wrapper = preparation.wrapper_warning;
    let config_dir_source = scope.config_source;
    let (name_hint, name_source) = request.name_hint();
    let recheck_configuration = || -> Result<(), ApiError> {
        if local_budget.is_exhausted(local_clock.as_ref()) || budget.is_exhausted(parts.clock) {
            return Err(api(
                ErrorCode::DeadlineExceeded,
                "launch budget exhausted before submission",
            ));
        }
        if crate::harness::BinaryIdentity::observe(&binary).as_ref() != Some(&binary_identity)
            || crate::cli::hook::resolve_on_path(
                executable_name,
                adapter_request.environment.path.as_deref(),
            )
            .and_then(|path| crate::harness::BinaryIdentity::observe(&path))
            .as_ref()
                != Some(&binary_identity)
            || (if let Some(handle) = &prelaunch {
                registration.recheck_prelaunch(&adapter_request, &scope, handle, &local_budget)?
            } else {
                policy.configuration_fingerprint(&adapter_request, &scope)?
            }) != config_fingerprint
        {
            return Err(api(
                ErrorCode::Conflict,
                "launch executable or selected configuration changed before submission",
            ));
        }
        Ok(())
    };
    recheck_configuration()?;
    // 2-3. Policy: fresh read, seat, owned hooks, recheck, guarded start.
    let prepared = crate::harness::launch::prepare_managed_with_registry(
        registry,
        parts.host,
        &seats,
        &inspector,
        parts.clock,
        ManagedLaunchRequest {
            target: request.target.clone(),
            harness: policy_harness(request.harness),
            argv: request.argv.clone(),
            shell_passes_no_daemon: false,
            name_hint: name_hint.clone(),
        },
        &budget,
        Some(preparation.argv.clone()),
    );
    let seat = seats.seat.lock().ok().and_then(|slot| slot.clone());
    let prepared = prepared?;
    if preflight {
        return Ok(LaunchReport {
            report: json!({"seat": prepared.request.seat}),
            exit: 0,
        });
    }
    boundary(LaunchBoundary::BeforeSubmit(&prepared.request))?;
    if let Err(error) = recheck_configuration() {
        boundary(LaunchBoundary::RefusedBeforeStart)?;
        return Err(error.into());
    }
    let outcome = if typed_evidence {
        match submit_prepared_with_evidence(parts.host, parts.clock, prepared) {
            Ok(outcome) => outcome,
            Err(failure) => {
                if failure.submission == crate::ports::NativeSubmission::NotSubmitted {
                    boundary(LaunchBoundary::RefusedBeforeStart)?;
                }
                return Err(failure.error.into());
            }
        }
    } else {
        submit_prepared(parts.host, parts.clock, prepared)?
    };
    let (status, argv, agent_name, exit) = match &outcome {
        NativeLaunchOutcome::ObservedStartup { correlation, .. } => (
            "started",
            Some(correlation.argv.clone()),
            Some(correlation.agent_name.clone()),
            0,
        ),
        NativeLaunchOutcome::OutcomeUnknown => ("outcome_unknown", None, None, 5),
    };
    // TRUST-POLICY A3 `managed_launch`: only an accepted, correlated startup
    // is reported to the daemon, never an unconfirmed one.
    let binding = match &outcome {
        NativeLaunchOutcome::ObservedStartup { correlation, .. } => {
            Some(binding_report(&seats, correlation, parts.clock))
        }
        NativeLaunchOutcome::OutcomeUnknown => None,
    };
    // Unconfirmed: either name may now be live in the pane.
    let agent_name_candidates = match (&outcome, &seat) {
        (NativeLaunchOutcome::OutcomeUnknown, Some(seat)) => {
            Some(crate::ports::launch_agent_names(seat, name_hint.as_deref()).to_vec())
        }
        _ => None,
    };
    let handoff_budget = CallBudget {
        deadline: MonoInstant(parts.clock.monotonic_now().0.saturating_add(5_000)),
        cancellation: Cancellation::default(),
    };
    let handoff = match &seat {
        Some(seat) => match parts.handoff.pending(seat, &handoff_budget) {
            Ok(summary) => summary,
            Err(error) => json!({"unavailable": format!("{:?}: {}", error.code, error.detail)}),
        },
        None => Value::Null,
    };
    let mut warnings = Vec::new();
    if let Some(name) = &request.name
        && name_source == "name"
        && crate::ports::sanitize_agent_name(name).as_deref() != Some(name.as_str())
    {
        warnings.push(format!(
            "--name `{name}` was fitted to Herdr's agent-name rule [a-z][a-z0-9_-]{{0,31}}"
        ));
    }
    let config_dir = json!({ "path": root.display().to_string(), "source": config_dir_source });
    let codex = preparation
        .report
        .get("codex")
        .cloned()
        .unwrap_or(Value::Null);
    let mut record = json!({
        "at_utc_ms": parts.clock.utc_now().0,
        "pane": request.target.as_str(),
        "seat": seat.as_ref().map(SeatId::as_str),
        "harness": word,
        "outcome": status,
        "agent_name": agent_name,
        "agent_name_source": name_source,
        "agent_name_candidates": agent_name_candidates,
        "argv": argv,
        "caller_argv": request.argv,
        "codex_wrapper": codex_wrapper,
        "config_dir": config_dir,
        "codex": codex,
        "harness_version": observed.version,
        "admission": if prelaunch.is_some() { "prelaunch_observed" } else { "contract_declared" },
        "recipe": observed.recipe,
        "binding": binding,
    });
    append_adapter_projection(&mut record, &preparation.report);
    if let Some(dir) = parts.record_dir
        && let Err(error) = append_record(dir, &record)
    {
        warnings.push(format!("the launch record could not be written: {error}"));
    }
    let seat_word = seat.as_ref().map_or("SEAT", SeatId::as_str).to_owned();
    let mut report = json!({
        "outcome": status,
        "pane": request.target.as_str(),
        "seat": seat.as_ref().map(SeatId::as_str),
        "harness": word,
        "agent_name": agent_name,
        "agent_name_source": name_source,
        "argv": argv,
        "codex_wrapper": codex_wrapper,
        "config_dir": config_dir,
        "codex": codex,
        "harness_version": {
            "admission": if prelaunch.is_some() { "prelaunch_observed" } else { "contract_declared" },
            "binary": observed.binary.display().to_string(),
            "version": observed.version,
            "recipe": observed.recipe,
        },
        "handoff": handoff,
        "binding": binding,
        "receipt": "launch is not receipt: nothing was checked in, accepted or ACKed",
        "next": format!(
            "the agent's SessionStart hook shows pending invitations and messages; if its \
             initial prompt is lost, the idle agent is woken for pending work, and \
             `herdr-threads inbox --seat {seat_word}` still lists it"
        ),
        "warnings": warnings,
    });
    append_adapter_projection(&mut report, &preparation.report);
    if let Some(candidates) = agent_name_candidates {
        report["agent_name_candidates"] = json!(candidates);
    }
    if exit != 0 {
        report["note"] = json!(
            "the start may have happened but was not confirmed: inspect the pane with `herdr \
             agent get` / `herdr agent read` before launching again"
        );
    }
    Ok(LaunchReport { report, exit })
}

#[cfg(test)]
#[path = "../../tests/cli/launch.rs"]
mod tests;

#[cfg(all(test, feature = "test-support"))]
#[allow(dead_code)]
mod task23_tests {
    use super::*;
    use crate::ports::{
        CorrelatedStartup, EvidenceKind, ExecutionEvidence, HostCallContext, HostSnapshot,
        HostUiState, IncarnationEvidence, NativeLaunchCapability, NativeLaunchRequest,
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
                    process_hint: request.process_hint,
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

    // Real shared guarded launch with source-shaped Python APIs and real locked
    // asset producer. These fixtures do not invoke installed Hermes or Herdr.
    fn hermes_fixture() -> Scratch {
        hermes_fixture_selected("default")
    }
    fn hermes_fixture_selected(selected_profile: &str) -> Scratch {
        use crate::test_support::spawn::SpawnOwned;
        let mut scratch = Scratch::new();
        let integration = Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/hermes");
        let script = "import sys,json;from pathlib import Path;sys.path.insert(0,sys.argv[1]);from test_runtime_helper import RuntimeHelperTests;t=RuntimeHelperTests();t.setUp();t.make_api();t.tmp._finalizer.detach();print(json.dumps({'python':str(Path(sys.executable).resolve()),'native':str(t.native),'selected':str(t.selected),'home':str(t.home)}))";
        let mut command = crate::test_support::spawn::command("python3");
        command
            .args(["-I", "-B", "-c", script])
            .arg(integration)
            .env("TMPDIR", &scratch.root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let out = command.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        let python = value["python"].as_str().unwrap();
        let native = value["native"].as_str().unwrap();
        let bootstrap = format!(
            "import sys,runpy;sys.path.insert(0,{});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)",
            serde_json::to_string(native).unwrap()
        );
        let launcher = scratch.root.join("bin/hermes");
        fs::write(&launcher,format!("#!{python}\nimport json,sys\nprint(json.dumps([{},'-I','-c',{},'--count','--no-report',sys.argv[-3],'--profile',sys.argv[-1]]))\n",serde_json::to_string(python).unwrap(),serde_json::to_string(&bootstrap).unwrap())).unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        scratch.env.home = Some(scratch.root.as_os_str().to_owned());
        for (key, field) in [
            ("FIXTURE_HOME", "home"),
            ("FIXTURE_DEP", "selected"),
            ("HERMES_HOME", "home"),
            ("TASK23_NATIVE", "native"),
        ] {
            scratch
                .env
                .declared_environment
                .insert(key.into(), value[field].as_str().unwrap().into());
        }
        let env = scratch.env.snapshot();
        let budget = CallBudget {
            deadline: MonoInstant(10000),
            cancellation: Default::default(),
        };
        let observed = crate::harness::hermes::runtime::discover_selected_profile(
            &launcher,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/hermes/runtime_helper.py"),
            selected_profile,
            &env,
            &budget,
        );
        // Machine metadata binds the exact helper entry, so use the unchanged
        // captured dynamic helper argument, never a guessed bootstrap.
        let observed = observed.unwrap();
        crate::harness::hermes::assets::setup(
            &observed,
            env.state_dir.as_ref().unwrap(),
            &env.executable,
            env.host_endpoint.as_ref().unwrap(),
        )
        .unwrap();
        scratch
    }
    #[test]
    fn hermes_actual_generic_guarded_consumer_uses_launch_only_observation() {
        let s = hermes_fixture();
        let host = FakeHost::new();
        let seats = seats();
        let handoff = handoff();
        let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
        let report = s.launch(
            &host,
            &seats,
            &handoff,
            request(harness, &["--model", "model Ω", "-q", "first turn's text"]),
        );
        let report = report.expect("actual generic consumer refuses separate positive prelaunch");
        assert_eq!(report.exit, 0);
        let starts = host.submitted();
        assert_eq!(starts.len(), 1);
        assert!(starts[0].process_hint);
        assert_eq!(
            starts[0].argv,
            [
                "--profile",
                "default",
                "--cli",
                "chat",
                "--model",
                "model Ω",
                "--query",
                "first turn's text"
            ]
        );
        assert_eq!(seats.recorded.lock().unwrap().len(), 1);
        assert_eq!(report.report["hermes"]["callback_qualified"], false);
        assert_eq!(report.report["hermes"]["native_acceptance"], "unmet");
    }
    #[test]
    fn hermes_named_profile_actual_consumer_keeps_provider_prompt_bytes() {
        let s = hermes_fixture_selected("work");
        let host = FakeHost::new();
        let seats = seats();
        let handoff = handoff();
        let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
        let report = s
            .launch(
                &host,
                &seats,
                &handoff,
                request(
                    harness,
                    &[
                        "--profile",
                        "Work",
                        "--cli",
                        "--provider",
                        "provider's Ω",
                        "--query",
                        "first literal turn",
                    ],
                ),
            )
            .unwrap();
        assert_eq!(
            host.submitted()[0].argv,
            [
                "--profile",
                "work",
                "--cli",
                "chat",
                "--provider",
                "provider's Ω",
                "--query",
                "first literal turn"
            ]
        );
        assert_eq!(report.report["hermes"]["profile"], "work");
        assert_eq!(seats.recorded.lock().unwrap().len(), 1);
    }
    #[test]
    fn hermes_foreign_partial_api_and_settings_refuse_before_seat_start() {
        for kind in ["foreign", "partial", "api", "settings"] {
            let mut s = hermes_fixture();
            let home = Path::new(s.env.declared_environment["FIXTURE_HOME"].to_str().unwrap());
            match kind {
                "foreign" => fs::write(
                    home.join("plugins/herdr-threads/__init__.py"),
                    "# foreign bytes",
                )
                .unwrap(),
                "partial" => {
                    fs::remove_file(home.join("plugins/herdr-threads/bridge_config.json")).unwrap()
                }
                "api" => {
                    let native = Path::new(
                        s.env.declared_environment["TASK23_NATIVE"]
                            .to_str()
                            .unwrap(),
                    );
                    fs::remove_file(native.join("hermes_cli/plugins_dispatch.py")).unwrap();
                }
                _ => s.env.host_endpoint = Some(s.root.join("wrong-endpoint")),
            }
            let host = FakeHost::new();
            let seats = seats();
            let handoff = handoff();
            let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
            assert!(
                s.launch(&host, &seats, &handoff, request(harness, &[]))
                    .is_err(),
                "accepted {kind}"
            );
            assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
            assert!(host.submitted().is_empty());
            assert!(seats.recorded.lock().unwrap().is_empty());
            assert!(s.records().is_empty());
        }
    }
    #[test]
    fn hermes_disabled_and_unknown_selected_config_refuse_before_seat_start() {
        for failed in [false, true] {
            let s = hermes_fixture();
            let native = Path::new(
                s.env.declared_environment["TASK23_NATIVE"]
                    .to_str()
                    .unwrap(),
            );
            fs::write(native.join("hermes_cli/config.py"),if failed {
                "class FailedConfigRead(dict): pass\ndef load_config_readonly(): return FailedConfigRead({'plugins':{'enabled':['herdr-threads'],'disabled':[]}})\n"
            } else {"class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':[],'disabled':['herdr-threads']}}\n"}).unwrap();
            let host = FakeHost::new();
            let seats = seats();
            let handoff = handoff();
            let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
            assert_eq!(
                code(s.launch(&host, &seats, &handoff, request(harness, &[]))),
                ErrorCode::MissingHook
            );
            assert_eq!(seats.calls.load(Ordering::SeqCst), 0);
            assert!(host.submitted().is_empty());
            assert!(seats.recorded.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn hermes_pre_submit_config_generation_and_binary_drift_never_start() {
        for kind in ["config", "asset", "binary"] {
            let s = hermes_fixture();
            let host = FakeHost::new();
            let seats = seats();
            let handoff = handoff();
            let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
            let req = request(harness, &[]);
            let clock = Clock0(AtomicU64::new(1));
            let probe = FakeProbe(Ok("unused".into()));
            let path = match kind {
                "config" => Path::new(
                    s.env.declared_environment["TASK23_NATIVE"]
                        .to_str()
                        .unwrap(),
                )
                .join("hermes_cli/config.py"),
                "asset" => Path::new(s.env.declared_environment["FIXTURE_HOME"].to_str().unwrap())
                    .join("plugins/herdr-threads/__init__.py"),
                _ => s.root.join("bin/hermes"),
            };
            let mut at_boundary = false;
            let mut boundary = |event: LaunchBoundary<'_>| {
                if matches!(event, LaunchBoundary::BeforeSubmit(_)) {
                    at_boundary = true;
                    let mut bytes = fs::read(&path).unwrap();
                    bytes.extend_from_slice(b"\n# owned synthetic mutation\n");
                    fs::write(&path, bytes).unwrap();
                }
                Ok(())
            };
            let result = execute_guarded_inner(
                crate::harness::registry::builtins(),
                &req,
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
                false,
                &mut boundary,
            );
            assert!(
                at_boundary,
                "initial qualification did not reach final guard for {kind}"
            );
            if kind == "config" {
                // An inert comment is not effective-config drift. It must still
                // pass: prelaunch is observation, not continuous source freshness.
                assert_eq!(result.unwrap().exit, 0);
                assert_eq!(host.submitted().len(), 1);
            } else {
                assert!(result.is_err(), "{kind} drift reached start");
                assert!(host.submitted().is_empty());
                assert!(seats.recorded.lock().unwrap().is_empty());
            }
        }
    }
    #[test]
    fn hermes_effective_enablement_drift_at_final_boundary_is_refused() {
        let s = hermes_fixture();
        let host = FakeHost::new();
        let seats = seats();
        let handoff = handoff();
        let harness: ContextHarness = serde_json::from_str("\"Hermes\"").unwrap();
        let req = request(harness, &[]);
        let clock = Clock0(AtomicU64::new(1));
        let probe = FakeProbe(Ok("unused".into()));
        let config = Path::new(
            s.env.declared_environment["TASK23_NATIVE"]
                .to_str()
                .unwrap(),
        )
        .join("hermes_cli/config.py");
        let mut at_boundary = false;
        let mut boundary = |event: LaunchBoundary<'_>| {
            if matches!(event, LaunchBoundary::BeforeSubmit(_)) {
                at_boundary = true;
                fs::write(&config,"class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':[],'disabled':['herdr-threads']}}\n").unwrap();
            }
            Ok(())
        };
        assert!(
            execute_guarded_inner(
                crate::harness::registry::builtins(),
                &req,
                &LaunchParts {
                    env: &s.env,
                    host: &host,
                    seats: &seats,
                    handoff: &handoff,
                    clock: &clock,
                    record_dir: Some(&s.root),
                    shell_probe: &probe
                },
                false,
                false,
                &mut boundary
            )
            .is_err()
        );
        assert!(at_boundary);
        assert!(host.submitted().is_empty());
        assert!(seats.recorded.lock().unwrap().is_empty());
        assert!(s.records().is_empty());
    }
    #[test]
    fn prelaunch_handle_cannot_cross_registration_scope_or_captured_inputs() {
        use crate::harness::adapter::{
            LaunchRequest as AdapterRequest, SetupStatus, StatusRequest,
        };
        let s = hermes_fixture();
        let registry = crate::harness::registry::builtins();
        let reg = registry.by_id(registry.agent("hermes").unwrap()).unwrap();
        let codex = registry.by_id(registry.agent("codex").unwrap()).unwrap();
        let req = AdapterRequest {
            argv: vec![],
            environment: s.env.snapshot(),
            native_binary: Some(s.root.join("bin/hermes")),
        };
        let probe = FakeProbe(Ok("unused".into()));
        let budget = CallBudget {
            deadline: MonoInstant(30000),
            cancellation: Default::default(),
        };
        let scope = reg
            .launch_policy()
            .unwrap()
            .resolve_scope(&req, &probe, &budget)
            .unwrap();
        assert!(
            reg.observe_prelaunch(&req, &scope, &s.env.executable, &budget)
                .is_err()
        );
        let handle = reg
            .observe_prelaunch(&req, &scope, req.native_binary.as_ref().unwrap(), &budget)
            .unwrap();
        let SetupStatus::Detailed(status) = reg.status(
            &StatusRequest {
                scope: scope.setup.clone(),
                environment: req.environment.clone(),
                native_binary: req.native_binary.clone(),
            },
            &budget,
        ) else {
            panic!("status unavailable")
        };
        assert!(
            codex
                .prepare_prelaunch(&req, &scope, &handle, &status, &probe, &budget)
                .is_err()
        );
        let mut changed = req;
        changed.argv = vec!["--profile".into(), "work".into()];
        assert!(
            reg.prepare_prelaunch(&changed, &scope, &handle, &status, &probe, &budget)
                .is_err()
        );
        changed.argv.clear();
        changed.environment.host_endpoint = Some(s.root.join("other-host"));
        assert!(
            reg.recheck_prelaunch(&changed, &scope, &handle, &budget)
                .is_err()
        );
        changed.environment.host_endpoint = s.env.host_endpoint.clone();
        let mut other = scope.clone();
        other.working_directory = s.root.join("elsewhere");
        assert!(
            reg.prepare_prelaunch(&changed, &other, &handle, &status, &probe, &budget)
                .is_err()
        );
        let cancelled = CallBudget {
            deadline: MonoInstant(30000),
            cancellation: Default::default(),
        };
        cancelled.cancellation.cancel();
        assert!(
            reg.observe_prelaunch(
                &changed,
                &scope,
                changed.native_binary.as_ref().unwrap(),
                &cancelled
            )
            .is_err()
        );
    }
}
