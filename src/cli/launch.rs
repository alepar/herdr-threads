//! `launch --pane PANE --kind claude|codex [-- AGENT_ARG...]`:
//! the application composition of the managed native launch policy
//! (`harness::launch::launch_managed`).
//!
//! Preflight, in order:
//! 1. installed-version recipe gate: `<harness> --version` must be admitted by
//!    the admission ladder (the same observation `setup` and the hook use);
//! 2. inside the policy: a fresh explicit-target read that must be an
//!    available shell, the pane's seat resolved through the daemon's ordinary
//!    guarded `seat resolve` path (a recovery hold refuses), the owned
//!    user-level hook installation inspected (`setup` must have installed it
//!    in the Claude settings / Codex hooks.json this environment resolves;
//!    for Codex under a sandbox that needs it, the recorded config.toml
//!    socket allowance for this instance too), and a last fenced recheck;
//! 3. Herdr's guarded `agent.start` with the argument array: the caller's
//!    arguments byte for byte and in order (the owned configuration is on
//!    disk, so launch adds none), and Codex `--no-daemon` exactly once at the
//!    top level (`harness::launch::compose_native_argv`), or not at all when
//!    the pane shell's `codex` wrapper already passes it ([`CodexShellProbe`]).
//!
//! Launch never registers, accepts or ACKs. Invitations and messages that
//! were committed before launch stay pending and are surfaced by the
//! SessionStart hook's check-in, or by `inbox`, even when the agent's
//! initial prompt is lost. Each submitted start is appended to a private
//! launch record in the instance directory.

use super::{
    RunError,
    journal::{IntentScope, Journal, SemanticMutation},
    retry,
    setup::{self, SetupEnv, SetupRequest, SetupVerb},
};
use crate::{
    client::local::LocalSocketClient,
    harness::{
        codex,
        context::Harness as ContextHarness,
        launch::{
            LaunchHookConfiguration, LaunchHookInspector, LaunchSeatResolver, ManagedLaunchRequest,
            OpenBinding, launch_managed,
        },
    },
    host::observation::PaneName,
    ports::{ConfiguredHook, HostObservation, HostPort, LocalClient, NativeLaunchOutcome},
    protocol::{
        authority::Harness,
        commands::{Command, InboxQuery},
        ids::{HostTargetId, SeatId},
        output::OutputSpec,
        pagination::PageRequest,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
    process::{Command as Process, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

/// How the pane's interactive shell resolves `codex`. Herdr starts the
/// agent by name inside that shell, so a user function or alias wrapping
/// `codex` runs first and may already pass `--no-daemon` (Codex refuses the
/// flag twice). Injected so tests never run a real shell.
pub trait CodexShellProbe {
    /// The shell's description of `codex` (stdout only), or why it could
    /// not be obtained.
    fn resolve_codex(&self) -> Result<String, String>;

    /// The value the pane's interactive shell itself gives `var` (an `export`
    /// in its startup files), without the launcher's own value; `None` when
    /// the shell sets none or cannot be asked. Herdr's `agent.start` carries
    /// no environment, so the agent inherits whatever the pane shell has.
    fn pane_shell_env(&self, _var: &str) -> Option<String> {
        None
    }
}

/// The bound on the shell probe; on timeout launch keeps adding `--no-daemon`.
pub const SHELL_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Runs the user's `$SHELL` (else `/bin/zsh`) interactively, as the pane
/// does: zsh `whence -f codex 2>/dev/null || type codex`, otherwise
/// `type codex`. Stdout only; stdin and stderr are null.
pub struct SystemShellProbe {
    pub shell: std::path::PathBuf,
    pub timeout: Duration,
}

impl SystemShellProbe {
    pub fn from_process() -> Self {
        let shell = std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map_or_else(|| "/bin/zsh".into(), std::path::PathBuf::from);
        Self {
            shell,
            timeout: crate::protocol::time::external_bound(SHELL_PROBE_TIMEOUT),
        }
    }
}

impl SystemShellProbe {
    /// Runs `script` in the interactive shell (`-ic`), stdout only, bounded by
    /// the probe timeout. `unset` removes one inherited variable first.
    fn run_script(&self, script: &str, unset: Option<&str>) -> Result<String, String> {
        let mut command = Process::new(&self.shell);
        command.arg("-ic").arg(script);
        if let Some(var) = unset {
            command.env_remove(var);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{}: {error}", self.shell.display()))?;
        let mut stdout = child.stdout.take().ok_or("no shell stdout")?;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout.read_to_end(&mut bytes).map(|_| bytes);
            let _ = sender.send(result);
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the shell probe timed out".into());
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        if !status.success() {
            return Err(format!("the shell probe exited with {status}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let bytes = receiver
            .recv_timeout(remaining.max(Duration::from_millis(100)))
            .map_err(|_| "the shell probe output was not closed".to_owned())?
            .map_err(|error| error.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl CodexShellProbe for SystemShellProbe {
    fn resolve_codex(&self) -> Result<String, String> {
        let is_zsh = self
            .shell
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("zsh"));
        let script = if is_zsh {
            "whence -f codex 2>/dev/null || type codex"
        } else {
            "type codex"
        };
        self.run_script(script, None)
    }

    fn pane_shell_env(&self, var: &str) -> Option<String> {
        const BEGIN: &str = "HT_PANE_ENV_BEGIN";
        const END: &str = "HT_PANE_ENV_END";
        // Only a plain variable name is ever interpolated into the script.
        if var.is_empty() || !var.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            return None;
        }
        let script = format!("printf '%s' {BEGIN}\"${{{var}-}}\"{END}");
        let output = self.run_script(&script, Some(var)).ok()?;
        let value = output.split(BEGIN).nth(1)?.split(END).next()?;
        (!value.is_empty()).then(|| value.to_owned())
    }
}

/// Whether a shell's description of `codex` (a function body or alias)
/// passes `--no-daemon` as a word of its own. Comment lines are ignored;
/// `--no-daemon=...` is not the flag.
pub fn wrapper_passes_no_daemon(description: &str) -> bool {
    description
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(|line| {
            line.split(|c: char| {
                c.is_whitespace() || matches!(c, '\'' | '"' | '`' | ';' | '(' | ')' | '|' | '&')
            })
        })
        .any(|word| word == "--no-daemon")
}

/// The report text when the pane shell's `codex` wrapper already passes
/// `--no-daemon` and launch therefore adds none.
pub const CODEX_WRAPPER_NO_DAEMON: &str =
    "shell function or alias already passes --no-daemon; launch added none";

/// `launch --help` epilogue.
pub const LAUNCH_HELP: &str = "Target:
  --pane names one existing Herdr pane at its interactive shell prompt; launch never
  creates, splits or picks a pane and never types into an occupied one. Herdr's
  guarded `agent start` refuses a busy pane.

Preflight (nothing is started when any step refuses):
  - `<kind> --version` must not be refused by the admission ladder (unparsable, inside a
    known-broken range, or older than every recipe: exit 4); an optimistic or schema-matched
    version is admitted with its label;
  - the pane's seat is resolved like `seat resolve --pane`; a recovery-held target
    needs `seat rebind ... --operator` or a fresh seat first;
  - the owned user-level hooks must be set up (`herdr-threads setup claude|codex`) in the
    Claude settings / Codex hooks.json of the CLAUDE_CONFIG_DIR / CODEX_HOME the agent
    will use (an absolute one the pane's shell exports, else launch's own, else HOME;
    the report's config_dir names it); for Codex under a sandbox
    that refuses the daemon socket, the recorded config.toml allowance for this
    instance's socket too;
  - a last fresh read of the pane just before Herdr starts the agent.

Agent name: --name NAME, else the pane's Herdr label, else seat-<short seat id>,
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

A harness that exits right after the start (for example Codex refusing its arguments)
fails launch at once with invalid_request and the pane's last lines. Codex launches report
`codex`: the effective CODEX_HOME, its config.toml and the selected profile (-p/--profile,
else `profile` in config.toml, else default).

Exit status: 0 startup observed; 5 start may have happened but was not confirmed:
inspect the pane (`herdr agent get` / `herdr agent read`) before launching again.";

#[derive(Debug, Clone, PartialEq, Eq)]
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
    match harness {
        ContextHarness::Claude => Harness::Claude,
        ContextHarness::Codex => Harness::Codex,
        ContextHarness::Human => Harness::Human,
    }
}

fn harness_word(harness: ContextHarness) -> &'static str {
    match harness {
        ContextHarness::Claude => "claude",
        ContextHarness::Codex => "codex",
        ContextHarness::Human => "human",
    }
}

/// Whether the caller's Codex arguments leave the sandbox in a mode that
/// refuses the daemon socket (the default `workspace-write`, or an explicit
/// `read-only`/`workspace-write`). Only an explicit full-access mode or the
/// sandbox bypass flag makes the socket allowance unnecessary.
pub fn codex_sandbox_needs_allowance(argv: &[String]) -> bool {
    let options_end = argv.iter().position(|a| a == "--").unwrap_or(argv.len());
    let options = &argv[..options_end];
    let mut full_access = false;
    for (index, arg) in options.iter().enumerate() {
        let next = options.get(index + 1).map(String::as_str);
        let mode = match arg.as_str() {
            "-s" | "--sandbox" => next,
            "--dangerously-bypass-approvals-and-sandbox" => Some("danger-full-access"),
            "-c" | "--config" => next.and_then(|value| {
                value
                    .strip_prefix("sandbox_mode=")
                    .map(|v| v.trim_matches(|c| c == '"' || c == '\''))
            }),
            other => other
                .strip_prefix("--sandbox=")
                .or_else(|| other.strip_prefix("-s").filter(|rest| !rest.is_empty())),
        };
        if let Some(mode) = mode {
            // The last explicit choice wins, as it does for Codex.
            full_access = mode == "danger-full-access";
        }
    }
    !full_access
}

/// The owned hook configuration of one launch, read from the setup library:
/// the exact user-level installation (no extra arguments), plus for Codex the
/// recorded sandbox socket allowance when the caller's sandbox needs it.
pub struct SetupHookInspector {
    env: SetupEnv,
    harness: ContextHarness,
    codex_witness: Option<codex::InstalledVersion>,
    caller_argv: Vec<String>,
    warnings: Mutex<Vec<String>>,
}

impl SetupHookInspector {
    fn installed(&self) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        let (_, inspection) = setup::user_inspection(self.harness, &self.env)
            .map_err(|detail| api(ErrorCode::Conflict, detail))?;
        Ok(inspection
            .configured_hook
            .map(|hook| LaunchHookConfiguration {
                hook,
                argv: Vec::new(),
            }))
    }

    fn codex(&self) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        let witness = self
            .codex_witness
            .as_ref()
            .ok_or_else(|| api(ErrorCode::UnsupportedHarness, "Codex version unobserved"))?;
        let Some(configuration) = self.installed()? else {
            return Ok(None);
        };
        if !codex_sandbox_needs_allowance(&self.caller_argv) {
            return Ok(Some(configuration));
        }
        let socket = setup::codex_sandbox_socket(&self.env, witness);
        match socket {
            Ok(socket) if setup::codex_allowance_present(&self.env, &socket) => {
                Ok(Some(configuration))
            }
            Ok(socket) => Err(api(
                ErrorCode::Unsupported,
                format!(
                    "the Codex sandbox allowance for {socket} (socket and writable roots) is not \
                     installed in config.toml: run `herdr-threads setup codex` (or pass an explicit `-s \
                     danger-full-access` after `--`). Under the default workspace-write sandbox \
                     herdr-threads commands cannot reach the daemon (transport_denied), so \
                     launch refuses"
                ),
            )),
            Err(reason) => Err(api(
                ErrorCode::Unsupported,
                format!(
                    "the Codex sandbox socket allowance is unavailable: {reason}. Under the \
                     default workspace-write sandbox herdr-threads commands cannot reach the \
                     daemon (transport_denied), so launch refuses. Pass an explicit \
                     `-s danger-full-access` after `--` to launch without it"
                ),
            )),
        }
    }
}

impl LaunchHookInspector for SetupHookInspector {
    fn configured_hook(
        &self,
        harness: Harness,
        budget: &CallBudget,
    ) -> Result<Option<ConfiguredHook>, ApiError> {
        Ok(self
            .launch_configuration(harness, budget)?
            .map(|configuration| configuration.hook))
    }

    fn launch_configuration(
        &self,
        harness: Harness,
        _budget: &CallBudget,
    ) -> Result<Option<LaunchHookConfiguration>, ApiError> {
        if harness != policy_harness(self.harness) {
            return Err(api(ErrorCode::InvalidRequest, "launch harness mismatch"));
        }
        match self.harness {
            ContextHarness::Claude => self.installed(),
            ContextHarness::Codex => self.codex(),
            ContextHarness::Human => Err(api(
                ErrorCode::InvalidRequest,
                "launch starts agents only; a person uses `herdr-threads me init`",
            )),
        }
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
}

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

/// The config directory the agent will use and where that came from: the
/// pane shell's own `CODEX_HOME` / `CLAUDE_CONFIG_DIR` (an `export` in its
/// startup files; Herdr's `agent.start` carries no environment, so the agent
/// inherits the pane's) when it sets an absolute one, else the launcher's
/// resolved value (the pane then inherits the Herdr server's environment,
/// which launch cannot read). Returns the environment to inspect with.
fn effective_env(
    request: &LaunchRequest,
    env: &SetupEnv,
    probe: &dyn CodexShellProbe,
) -> (SetupEnv, &'static str) {
    let var = match request.harness {
        ContextHarness::Codex => "CODEX_HOME",
        ContextHarness::Claude => "CLAUDE_CONFIG_DIR",
        ContextHarness::Human => return (env.clone(), "launcher"),
    };
    let pane_dir = probe
        .pane_shell_env(var)
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_absolute());
    let Some(dir) = pane_dir else {
        return (env.clone(), "launcher");
    };
    let mut effective = env.clone();
    match request.harness {
        ContextHarness::Codex => effective.codex_home = Some(dir),
        _ => effective.claude_config_dir = Some(dir),
    }
    (effective, "pane_shell")
}

/// The Codex profile Codex applies: `-p/--profile` before `--` (the last one
/// wins), else the top-level `profile` key of `config.toml`, else none.
pub fn codex_profile(argv: &[String], config: Option<&str>) -> (String, &'static str) {
    let options_end = argv.iter().position(|a| a == "--").unwrap_or(argv.len());
    let options = &argv[..options_end];
    let mut chosen = None;
    for (index, arg) in options.iter().enumerate() {
        let value = match arg.as_str() {
            "-p" | "--profile" => options.get(index + 1).map(String::as_str),
            other => other
                .strip_prefix("--profile=")
                .or_else(|| other.strip_prefix("-p").filter(|rest| !rest.is_empty())),
        };
        if let Some(value) = value {
            chosen = Some(value.to_owned());
        }
    }
    if let Some(profile) = chosen {
        return (profile, "argv");
    }
    let from_config = config
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| doc.get("profile")?.as_str().map(str::to_owned));
    match from_config {
        Some(profile) => (profile, "config.toml"),
        None => ("default".to_owned(), "none"),
    }
}

/// The effective Codex home, its `config.toml` and the selected profile,
/// which answers "why was my Codex profile not applied?".
fn codex_report(env: &SetupEnv, argv: &[String]) -> Value {
    let home = env.codex_home.as_deref();
    let config_path = home.map(|home| home.join("config.toml"));
    let config = config_path
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok());
    let (profile, profile_source) = codex_profile(argv, config.as_deref());
    json!({
        "codex_home": home.map(|home| home.display().to_string()),
        "config_path": config_path.as_ref().map(|path| path.display().to_string()),
        "config_present": config.is_some(),
        "profile": profile,
        "profile_source": profile_source,
    })
}

/// Run the managed launch preflight and start; errors are refusals before
/// any start was submitted.
pub fn execute(request: &LaunchRequest, parts: &LaunchParts<'_>) -> Result<LaunchReport, RunError> {
    let word = harness_word(request.harness);
    let (effective, config_dir_source) = effective_env(request, parts.env, parts.shell_probe);
    let parts = &LaunchParts {
        env: &effective,
        ..*parts
    };
    // 1. Installed-version recipe gate.
    let setup_request = SetupRequest {
        verb: SetupVerb::Status,
        harness: request.harness,
        harness_binary: request.harness_binary.clone(),
        prompt_suggestions: Default::default(),
    };
    // Codex: warm the hook's persistent fingerprint cache here, outside the
    // hook's time budget, so a cold scan of an unlisted (schema-matched)
    // version is not repeated, and refused, by every budget-bound hook.
    // Best effort: without an owned state root the scan stays in memory.
    let codex_cache = match request.harness {
        ContextHarness::Codex => parts
            .env
            .state_dir
            .as_deref()
            .and_then(|state| crate::harness::codex_evidence::prepare(state).ok())
            .map(|private| crate::harness::codex_evidence::cache_path(&private)),
        ContextHarness::Claude | ContextHarness::Human => None,
    };
    let (observed, witness) =
        setup::observe_with_cache(&setup_request, parts.env, codex_cache.as_deref())
            .map_err(setup::refuse_version)?;
    let inspector = SetupHookInspector {
        env: parts.env.clone(),
        harness: request.harness,
        codex_witness: witness,
        caller_argv: request.argv.clone(),
        warnings: Mutex::new(Vec::new()),
    };
    let seats = RecordingResolver {
        inner: parts.seats,
        seat: Mutex::new(None),
    };
    let budget = CallBudget {
        deadline: MonoInstant(parts.clock.monotonic_now().0.saturating_add(40_000)),
        cancellation: Cancellation::default(),
    };
    // The pane's shell may wrap `codex` with its own `--no-daemon`; a probe
    // failure or timeout keeps launch adding the flag.
    let shell_passes_no_daemon = request.harness == ContextHarness::Codex
        && parts
            .shell_probe
            .resolve_codex()
            .is_ok_and(|description| wrapper_passes_no_daemon(&description));
    let codex_wrapper = shell_passes_no_daemon.then_some(CODEX_WRAPPER_NO_DAEMON);
    let (name_hint, name_source) = request.name_hint();
    // 2-3. Policy: fresh read, seat, owned hooks, recheck, guarded start.
    let outcome = launch_managed(
        parts.host,
        &seats,
        &inspector,
        parts.clock,
        ManagedLaunchRequest {
            target: request.target.clone(),
            harness: policy_harness(request.harness),
            argv: request.argv.clone(),
            shell_passes_no_daemon,
            name_hint: name_hint.clone(),
        },
        &budget,
    );
    let seat = seats.seat.lock().ok().and_then(|slot| slot.clone());
    let outcome = outcome.map_err(|mut error| {
        if error.code == ErrorCode::MissingHook {
            let file = setup::user_inspection(request.harness, parts.env)
                .map(|(file, _)| file.display().to_string())
                .unwrap_or_else(|detail| detail);
            error.detail = format!(
                "the owned {word} hooks are not installed in {file}: run `herdr-threads setup \
                 {word}` first, with the CLAUDE_CONFIG_DIR / CODEX_HOME the agent uses (nothing \
                 was started)"
            );
        }
        RunError::Api(error)
    })?;
    let (status, argv, agent_name, exit) = match &outcome {
        NativeLaunchOutcome::ObservedStartup { correlation, .. } => (
            "started",
            Some(correlation.argv.clone()),
            Some(correlation.agent_name.clone()),
            0,
        ),
        NativeLaunchOutcome::OutcomeUnknown => ("outcome_unknown", None, None, 5),
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
    let mut warnings = inspector
        .warnings
        .lock()
        .map(|w| w.clone())
        .unwrap_or_default();
    if let Some(name) = &request.name
        && name_source == "name"
        && crate::ports::sanitize_agent_name(name).as_deref() != Some(name.as_str())
    {
        warnings.push(format!(
            "--name `{name}` was fitted to Herdr's agent-name rule [a-z][a-z0-9_-]{{0,31}}"
        ));
    }
    let config_dir = json!({
        "path": match request.harness {
            ContextHarness::Codex => parts.env.codex_home.as_deref(),
            _ => parts.env.claude_config_dir.as_deref(),
        }
        .map(|dir| dir.display().to_string()),
        "source": config_dir_source,
    });
    let codex =
        (request.harness == ContextHarness::Codex).then(|| codex_report(parts.env, &request.argv));
    let record = json!({
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
        "recipe": observed.recipe,
    });
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
            "binary": observed.binary.display().to_string(),
            "version": observed.version,
            "recipe": observed.recipe,
        },
        "handoff": handoff,
        "receipt": "launch is not receipt: nothing was checked in, accepted or ACKed",
        "next": format!(
            "the agent's SessionStart hook shows pending invitations and messages; if its \
             initial prompt is lost, `herdr-threads inbox --seat {seat_word}` still lists them"
        ),
        "warnings": warnings,
    });
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
