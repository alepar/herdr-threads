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
    let observed = crate::harness::setup::legacy::Observed {
        binary: binary.clone(),
        version,
        recipe: admitted.recipe(),
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
        Some(policy.configuration_fingerprint(&adapter_request, &scope)?)
    } else {
        None
    };
    let preparation = registration
        .prepare_launch(&adapter_request, &scope, &admitted, &status, parts.shell_probe, &local_budget)
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
            || policy.configuration_fingerprint(&adapter_request, &scope)? != config_fingerprint
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
        "admission": "contract_declared",
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
            "admission": "contract_declared",
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
