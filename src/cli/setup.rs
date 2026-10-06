//! `setup`, `unsetup` and `setup-status`: the public CLI over the owned
//! harness setup library (`harness::setup`, `harness::codex_config`). Local
//! only: these commands never contact or start the daemon.
//!
//! Setup is user-level, the way Herdr installs its own agent hooks:
//!
//! - Claude: the adapter's declared hook groups (SessionStart and Bash
//!   PreToolUse) and the permission allow rule `claude::HERDR_THREADS_ALLOW_RULE`
//!   (`Bash(herdr-threads *)`) go into `$CLAUDE_CONFIG_DIR/settings.json`
//!   (default `~/.claude/settings.json`), merged with the hooks already there.
//! - Codex: the declared hook groups (SessionStart, SubagentStart, Bash
//!   PreToolUse) go into `$CODEX_HOME/hooks.json` (default
//!   `~/.codex/hooks.json`). No sandbox allowance is installed. Historical
//!   config.toml ownership remains inspectable and removable by unsetup.
//!
//! Every write is owned: groups carry an installation marker, a private
//! manifest under `<state-dir>/setup/` records exactly what was added, a
//! foreign identical entry refuses, files are replaced atomically after an
//! unchanged-baseline check, re-running is idempotent, and `unsetup` removes
//! only what was added (byte for byte when nothing else changed).
//!
//! The Herdr instance (plugin state directory and Herdr server socket) is
//! `--state-dir` / `--host-endpoint`, else `HERDR_PLUGIN_STATE_DIR` /
//! `HERDR_SOCKET_PATH`, else Herdr's default locations when they exist, else
//! auto-detected through the `herdr` CLI ([`detect_state_dir`],
//! [`detect_host_endpoint`]); see [`super::instance`], which every command
//! shares. The installed hook command
//! records both, and the hook stays silent in any session that is not a pane
//! of that instance.
//!
//! Setup resolves the selected executable without invoking it and declares
//! the registered contract. Runtime metadata and native delivery remain unknown.
//! The installed hook command is exactly the hook entrypoint's
//! [`hook::installed_argv`], which its `parse_hook_argv` accepts.

use super::{RunError, hook};
#[cfg(test)]
use crate::harness::{prompt_suggestion, setup::shell_command};
use crate::{
    daemon::paths::RuntimeContext,
    harness::{claude, codex, context::Harness},
    protocol::{
        output::{OutputFormat, OutputSpec},
        results::{ApiError, ErrorCode},
    },
};
use serde_json::{Value, json};
#[cfg(test)]
use std::fs;
use std::{
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

/// Shared `--help` epilogue for the three setup commands.
pub const SETUP_HELP: &str =
    "No harness named: every harness. `setup` sets up every detected harness (each
of claude and codex found on PATH) and prints one line per harness: installed, already
installed or skipped (not on PATH);
`unsetup` removes both recorded installations (on PATH or not); `setup-status` reports
both. The Codex hook-trust reminder is printed once at the end. The exit status is that
of the first harness that failed; skipped harnesses are not failures.

Scope (user level, like Herdr's own agent hooks):
  claude  $CLAUDE_CONFIG_DIR/settings.json (default ~/.claude/settings.json), created as `{}`
          when absent. Adds the hook groups (SessionStart, Bash PreToolUse) beside the hooks
          already there, and the permission allow rule `Bash(herdr-threads *)`: Claude may then
          run any single `herdr-threads ...` command (including the ready commands the hook
          suggests) without asking; chained commands are still checked separately. A rule you
          already have is left as yours. Re-running setup on an older installation replaces its
          owned `Bash(export HERDR_THREADS_CALLER_CONTEXT=*)` rule, which allowed nothing in use.
  codex   $CODEX_HOME/hooks.json (default ~/.codex/hooks.json) for the hook groups
          (SessionStart, SubagentStart, Bash PreToolUse). Setup installs no sandbox socket,
          writable-root or network allowance. Run herdr-threads commands through Codex's
          approved outside-sandbox execution; a denied approval is a policy refusal.
          Historical owned allowances remain inspectable; unsetup removes only unchanged
          owned values and refuses edited ownership records. Codex runs user hooks only once
          you trust them: the next interactive `codex` start lists them for review (or use
          /hooks); Codex records their hashes in config.toml [hooks.state]. setup never writes trust.

Claude prompt suggestions: Claude shows a dim prompt suggestion in its input box after every
turn, which herdr-threads cannot tell from typed text, so it never pokes a Claude pane that shows
one. Unless `promptSuggestionEnabled` is already false, `setup claude` (and bare `setup`) asks
`Disable prompt suggestions? [y/N]` on a terminal and writes `promptSuggestionEnabled: false` only
on yes; without a terminal it only advises. --disable-prompt-suggestions / --keep-prompt-suggestions
decide without asking. The write is recorded; unsetup reverts it if setup set it.

Herdr instance: --state-dir and --host-endpoint, else HERDR_PLUGIN_STATE_DIR and
HERDR_SOCKET_PATH (set inside Herdr), else Herdr's defaults when they exist
($XDG_STATE_HOME or ~/.local/state, then herdr/plugins/herdr-threads; $XDG_CONFIG_HOME or
~/.config, then herdr/herdr.sock), else detected with the herdr CLI (`herdr plugin list`,
`herdr status server`).
Ambiguity is refused. The installed hook command is
`<this executable> --state-dir <state> --host-endpoint <socket> hook <harness>`, registered
for each event with `--event <EVENT>` appended (hooks installed before that keep working;
`doctor` suggests re-running setup); in any session that is not a pane of that Herdr
instance it prints nothing, runs no version probe and starts no daemon, and only sends a
short best-effort evidence note to a daemon already running (gate files under
<state>/harness/evidence).
Each harness's ownership manifest lives in <state>/setup/.

A hook file that already holds exactly this command's hook groups under another setup's
owner marker (for example a hooks.json copied with its trust into a second CODEX_HOME) is
adopted: setup records a manifest for it and leaves the file byte-identical (action
`adopted`), setup-status and doctor report it installed (adopted), and unsetup removes only
those groups from that file.

Setup resolves the selected executable without invoking --version, --help or schema probes.
Admission is contract_declared: the registered hook contract is configured, runtime metadata
is unknown. Installation does not prove hook delivery, native support or receipt.

Exit status:
  0  installed / removed / nothing to remove / status reported
  1  refused: the file changed concurrently, an owned entry was edited or removed by hand,
     the recorded allow rule was removed by hand, an unowned identical hook exists (for
     codex also in another Codex config layer), a recorded legacy allowance is partial or
     edited, or a file could not be written
  2  invalid arguments, undetectable Herdr instance, or invalid settings file (not a JSON
     object / not valid TOML, symlink, over 1 MiB)
  4  the named harness is missing from PATH or --harness-binary is not an executable file;
     with no harness named an absent harness is skipped";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupVerb {
    Install,
    Remove,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupRequest {
    pub scope: crate::harness::adapter::SetupScopeRequest,
    pub verb: SetupVerb,
    pub harness: Harness,
    pub harness_binary: Option<String>,
    /// `setup claude` only: what to do about Claude's prompt suggestions.
    pub prompt_suggestions: PromptSuggestionPolicy,
}

/// `setup claude`: whether to set Claude's [`claude::PROMPT_SUGGESTION_SETTING`]
/// to `false` (ht-6jt). Setup never changes it silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptSuggestionPolicy {
    /// Ask on an interactive terminal ([`settle_prompt_suggestions`]);
    /// otherwise only advise (report and warning, no change).
    #[default]
    Ask,
    /// `--disable-prompt-suggestions`: set it `false` without asking.
    Disable,
    /// `--keep-prompt-suggestions`: leave it, without asking or advising.
    Keep,
}

/// A person's pane identity (`me init`) has no hooks to set up.
const NO_HUMAN_SETUP: &str =
    "setup manages agent hooks only; a person uses `herdr-threads me init`";

fn harness_name(harness: Harness) -> &'static str {
    harness.as_str()
}

fn api(code: ErrorCode, detail: impl Into<String>) -> RunError {
    RunError::Api(ApiError::new(code, detail))
}

fn invalid(detail: impl Into<String>) -> RunError {
    api(ErrorCode::InvalidRequest, detail)
}

/// Everything the commands take from the process, injectable for tests.
#[derive(Debug, Clone)]
pub struct SetupEnv {
    pub home: Option<OsString>,
    pub declared_environment: std::collections::BTreeMap<String, OsString>,
    pub executable: PathBuf,
    pub state_dir: Option<PathBuf>,
    pub cwd: PathBuf,
    pub path: Option<OsString>,
    /// Codex's config home, resolved the way Codex does: a non-empty
    /// `CODEX_HOME`, else `$HOME/.codex`.
    pub codex_home: Option<PathBuf>,
    /// Claude's config directory: a non-empty `CLAUDE_CONFIG_DIR`, else
    /// `$HOME/.claude`.
    pub claude_config_dir: Option<PathBuf>,
    /// The Herdr host endpoint that, with the state directory, names the
    /// daemon instance the installed hook serves.
    pub host_endpoint: Option<PathBuf>,
    /// Where the state directory and host endpoint came from, or why they
    /// could not be detected.
    pub instance_source: Value,
}

/// How long one read-only `herdr` query may take during instance
/// auto-detection. Detection runs before almost every command, so a hung or
/// wedged Herdr must not hold the command for long: a healthy `herdr plugin
/// list --json` / `status server --json` answers in tens of milliseconds, and
/// a probe that misses this bound fails with the "pass --state-dir /
/// --host-endpoint" remedy instead of making the person wait out the old 5 s.
pub(crate) const AUTODETECT_PROBE_TIMEOUT: Duration = Duration::from_millis(2_000);
const HERDR_QUERY_LIMIT: u64 = 1 << 20;
pub const PLUGIN_ID: &str = "herdr-threads";

/// Inputs of Herdr instance detection, injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct DetectInputs {
    pub herdr: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
}

#[cfg(test)]
thread_local! {
    /// Unit tests that run fake `herdr` scripts in parallel with the rest of
    /// the suite get a generous probe; the bound test sets the real one.
    pub(crate) static PROBE_TIMEOUT: std::cell::Cell<Duration> =
        const { std::cell::Cell::new(Duration::from_secs(30)) };
}

fn probe_timeout() -> Duration {
    #[cfg(test)]
    return PROBE_TIMEOUT.with(std::cell::Cell::get);
    #[cfg(not(test))]
    AUTODETECT_PROBE_TIMEOUT
}

fn herdr_json(herdr: &Path, args: &[&str]) -> Result<Value, String> {
    let timeout = probe_timeout();
    let bytes = codex::bounded_output(herdr, args, HERDR_QUERY_LIMIT, timeout).map_err(|_| {
        format!(
            "`herdr {}` failed or did not answer within {} ms",
            args.join(" "),
            timeout.as_millis()
        )
    })?;
    if bytes.len() as u64 > HERDR_QUERY_LIMIT {
        return Err(format!("`herdr {}` output is too large", args.join(" ")));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("`herdr {}` did not print JSON", args.join(" ")))
}

/// The installed herdr-threads plugin's state directory: Herdr's plugin state
/// root (`$XDG_STATE_HOME/herdr/plugins`, else `~/.local/state/herdr/plugins`)
/// joined with the plugin id, for a plugin `herdr plugin list --json` reports
/// as installed and enabled. Refuses when the plugin is absent, disabled or
/// listed more than once, or when both state roots hold a store (ambiguous);
/// a legacy `~/.local/state` directory is chosen over `XDG_STATE_HOME` only when
/// it holds a store and the XDG directory does not
/// ([`super::instance::choose_state_dir`]).
pub fn detect_state_dir(inputs: &DetectInputs) -> Result<(PathBuf, String), String> {
    let herdr = inputs
        .herdr
        .as_deref()
        .ok_or("no `herdr` executable on PATH to detect the plugin state directory")?;
    let list = herdr_json(herdr, &["plugin", "list", "--json"])?;
    let plugins = list["result"]["plugins"]
        .as_array()
        .ok_or("`herdr plugin list --json` reported no plugin list")?;
    let ours: Vec<&Value> = plugins
        .iter()
        .filter(|plugin| plugin["plugin_id"] == PLUGIN_ID)
        .collect();
    match ours.as_slice() {
        [] => {
            return Err(format!(
                "the {PLUGIN_ID} Herdr plugin is not installed (`herdr plugin list`); install or \
                 link it, or pass --state-dir"
            ));
        }
        [plugin] if plugin["enabled"] == json!(false) => {
            return Err(format!(
                "the {PLUGIN_ID} Herdr plugin is disabled; enable it or pass --state-dir"
            ));
        }
        [_] => (),
        _ => {
            return Err(format!(
                "`herdr plugin list` reports {PLUGIN_ID} more than once; pass --state-dir"
            ));
        }
    }
    let (xdg, home) =
        super::instance::state_candidates(inputs.home.as_deref(), inputs.xdg_state_home.as_deref());
    match super::instance::choose_state_dir(xdg, home)? {
        Some((state, how, _)) => Ok((state, format!("herdr plugin list + {how}"))),
        None => Err("neither XDG_STATE_HOME nor HOME is an absolute path".into()),
    }
}

/// The socket of the running Herdr server, from `herdr status server --json`.
pub fn detect_host_endpoint(inputs: &DetectInputs) -> Result<(PathBuf, String), String> {
    let herdr = inputs
        .herdr
        .as_deref()
        .ok_or("no `herdr` executable on PATH to detect the Herdr server socket")?;
    let status = herdr_json(herdr, &["status", "server", "--json"])?;
    if status["running"] != json!(true) {
        return Err(
            "the Herdr server is not running (`herdr status server`); start Herdr or pass \
             --host-endpoint"
                .into(),
        );
    }
    let socket = status["socket"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("`herdr status server --json` reported no absolute socket path")?;
    Ok((socket, "herdr status server".into()))
}

fn capture_declared_environment(
    registry: &crate::harness::registry::Registry,
) -> std::collections::BTreeMap<String, OsString> {
    let names: std::collections::BTreeSet<_> = registry
        .registrations()
        .iter()
        .flat_map(|registration| registration.setup_environment_inputs())
        .copied()
        .collect();
    names
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
        .collect()
}

impl SetupEnv {
    pub(crate) fn from_process(output: &OutputSpec) -> Result<Self, RunError> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let path = std::env::var_os("PATH");
        let cwd = std::env::current_dir()?;
        let declared_environment =
            capture_declared_environment(crate::harness::registry::builtins());
        let codex_override = std::env::var_os("CODEX_HOME");
        let claude_override = std::env::var_os("CLAUDE_CONFIG_DIR");
        let mut inputs = super::instance::InstanceInputs::from_process(
            output.context.state_dir.as_ref().map(PathBuf::from),
            output.context.host.as_ref().map(PathBuf::from),
        );
        inputs.herdr = hook::resolve_on_path("herdr", path.as_deref());
        let mut source = serde_json::Map::new();
        let state_dir = match super::instance::resolve_state_dir(&inputs) {
            Ok((state, how)) => {
                if let Some(leftover) = super::instance::leftover_state_dir(&inputs, &state, &how) {
                    source.insert("state_dir_leftover".into(), json!(leftover));
                }
                source.insert("state_dir".into(), json!(how));
                Some(state)
            }
            Err(error) => {
                source.insert("state_dir_error".into(), json!(error));
                None
            }
        };
        let host_endpoint = match super::instance::resolve_host_endpoint(&inputs) {
            Ok((host, how)) => {
                source.insert("host_endpoint".into(), json!(how));
                Some(host)
            }
            Err(error) => {
                source.insert("host_endpoint_error".into(), json!(error));
                None
            }
        };
        let home = inputs.home.as_ref().map(|home| home.as_os_str().to_owned());
        Ok(Self {
            home: home.clone(),
            declared_environment,
            executable,
            state_dir,
            cwd,
            path,
            codex_home: codex_home_from(codex_override, home.clone()),
            claude_config_dir: claude_config_dir_from(claude_override, home.clone()),
            host_endpoint,
            instance_source: Value::Object(source),
        })
    }

    /// The environment of a command that already resolved its runtime
    /// context (`doctor`): the instance is that context's.
    pub(crate) fn for_context(context: &RuntimeContext) -> Self {
        let home = std::env::var_os("HOME");
        Self {
            home: home.clone(),
            declared_environment: capture_declared_environment(crate::harness::registry::builtins()),
            executable: std::env::current_exe()
                .and_then(|path| path.canonicalize())
                .unwrap_or_default(),
            state_dir: Some(context.state_dir.clone()),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            path: std::env::var_os("PATH"),
            codex_home: codex_home_from(std::env::var_os("CODEX_HOME"), home.clone()),
            claude_config_dir: claude_config_dir_from(
                std::env::var_os("CLAUDE_CONFIG_DIR"),
                home.clone(),
            ),
            host_endpoint: Some(context.host_endpoint.clone()),
            instance_source: json!({"state_dir": "context", "host_endpoint": "context"}),
        }
    }

    pub(crate) fn state_dir(&self) -> Result<&Path, RunError> {
        let state = self.state_dir.as_deref().ok_or_else(|| {
            invalid(format!(
                "state directory unknown: {}; use --state-dir or HERDR_PLUGIN_STATE_DIR",
                self.instance_source["state_dir_error"]
                    .as_str()
                    .unwrap_or("not detected")
            ))
        })?;
        if !state.is_absolute() {
            return Err(invalid("state directory must be an absolute path"));
        }
        Ok(state)
    }

    /// The canonical host endpoint (its directory resolved), as the daemon
    /// and the hook name the instance.
    pub(crate) fn host_endpoint(&self) -> Result<PathBuf, RunError> {
        let host = self.host_endpoint.clone().ok_or_else(|| {
            invalid(format!(
                "Herdr host endpoint unknown: {}; run inside Herdr, or pass --host-endpoint",
                self.instance_source["host_endpoint_error"]
                    .as_str()
                    .unwrap_or("not detected")
            ))
        })?;
        let context = RuntimeContext::explicit(self.state_dir()?.to_path_buf(), host, None)
            .map_err(|error| invalid(format!("invalid host endpoint: {error}")))?;
        Ok(context.host_endpoint)
    }

    pub(crate) fn hook_argv(&self, harness: Harness) -> Result<Vec<String>, RunError> {
        let executable = self
            .executable
            .to_str()
            .ok_or_else(|| invalid("herdr-threads executable path is not UTF-8"))?;
        let state = self
            .state_dir()?
            .to_str()
            .ok_or_else(|| invalid("state directory path is not UTF-8"))?;
        let host = self.host_endpoint()?;
        let host = host
            .to_str()
            .ok_or_else(|| invalid("host endpoint path is not UTF-8"))?;
        Ok(hook::installed_argv(
            executable,
            Some(state),
            Some(host),
            harness,
        ))
    }

    pub(crate) fn claude_settings(&self) -> Result<PathBuf, RunError> {
        self.claude_config_dir
            .as_ref()
            .map(|dir| dir.join("settings.json"))
            .ok_or_else(|| invalid("neither CLAUDE_CONFIG_DIR nor HOME is set"))
    }

    pub(crate) fn codex_file(&self, name: &str) -> Result<PathBuf, RunError> {
        self.codex_home
            .as_ref()
            .map(|dir| dir.join(name))
            .ok_or_else(|| invalid("neither CODEX_HOME nor HOME is set"))
    }
}

pub fn run<W: Write>(
    request: &SetupRequest,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let env = SetupEnv::from_process(output)?;
    let mut report = execute(request, &env)?;
    if request.verb == SetupVerb::Install
        && request.prompt_suggestions == PromptSuggestionPolicy::Ask
        && interactive()
    {
        let registry = crate::harness::registry::builtins();
        let registration = registry
            .by_id(
                registry
                    .agent(request.harness.as_str())
                    .map_err(|error| invalid(error.to_string()))?,
            )
            .map_err(|error| invalid(error.to_string()))?;
        registration
            .settle_setup_consent(
                &env.snapshot(),
                &mut report,
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )
            .map_err(adapter_run_error)?;
    }
    let bytes = match output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&json!({ "setup": report }))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => render_text(&report).into_bytes(),
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Run one setup command and return its report; errors carry the exit status.
fn adapter_run_error(error: crate::harness::adapter::SetupFailure) -> RunError {
    use crate::harness::adapter::SetupFailure;
    match error {
        SetupFailure::Api(e) => RunError::Api(e),
        SetupFailure::Io(e) => RunError::Io(e),
        other => invalid(other.to_string()),
    }
}
impl SetupEnv {
    pub(crate) fn from_snapshot(snapshot: &crate::harness::adapter::SetupEnvironment) -> Self {
        Self {
            home: snapshot.home.clone(),
            declared_environment: snapshot.declared.clone(),
            executable: snapshot.executable.clone(),
            state_dir: snapshot.state_dir.clone(),
            cwd: snapshot.cwd.clone(),
            path: snapshot.path.clone(),
            codex_home: snapshot.config_roots.get("codex").cloned(),
            claude_config_dir: snapshot.config_roots.get("claude").cloned(),
            host_endpoint: snapshot.host_endpoint.clone(),
            instance_source: snapshot.instance_source.clone(),
        }
    }
    pub fn snapshot(&self) -> crate::harness::adapter::SetupEnvironment {
        let mut config_roots = std::collections::BTreeMap::new();
        if let Some(root) = &self.codex_home {
            config_roots.insert("codex".into(), root.clone());
        }
        if let Some(root) = &self.claude_config_dir {
            config_roots.insert("claude".into(), root.clone());
        }
        crate::harness::adapter::SetupEnvironment {
            clock: std::sync::Arc::new(crate::app::SystemClock::new()),
            home: self.home.clone(),
            executable: self.executable.clone(),
            state_dir: self.state_dir.clone(),
            cwd: self.cwd.clone(),
            path: self.path.clone(),
            host_endpoint: self.host_endpoint.clone(),
            instance_source: self.instance_source.clone(),
            config_roots,
            declared: self.declared_environment.clone(),
        }
    }
}
pub fn execute(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let registry = crate::harness::registry::builtins();
    if request.harness == Harness::Human {
        return Err(invalid(NO_HUMAN_SETUP));
    }
    let registration = registry
        .by_id(
            registry
                .agent(request.harness.as_str())
                .map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?;
    let snapshot = env.snapshot();
    let mut options = crate::harness::adapter::SetupOptions::new();
    match request.prompt_suggestions {
        PromptSuggestionPolicy::Disable => {
            options.insert("disable-prompt-suggestions".into(), true);
        }
        PromptSuggestionPolicy::Keep => {
            options.insert("keep-prompt-suggestions".into(), true);
        }
        _ => {}
    }
    execute_registered(
        registration,
        request.verb,
        &request.scope,
        request.harness_binary.as_deref().map(Path::new),
        options,
        &snapshot,
    )
}
pub fn execute_registered(
    registration: &crate::harness::registry::Registration,
    verb: SetupVerb,
    scope: &crate::harness::adapter::SetupScopeRequest,
    native_binary: Option<&Path>,
    options: crate::harness::adapter::SetupOptions,
    environment: &crate::harness::adapter::SetupEnvironment,
) -> Result<Value, RunError> {
    execute_registered_with_expected_scope(
        registration,
        verb,
        scope,
        native_binary,
        options,
        environment,
        None,
    )
}

/// Installer reconciliation binds the final resolution to its inspected scope.
pub(crate) fn execute_registered_with_expected_scope(
    registration: &crate::harness::registry::Registration,
    verb: SetupVerb,
    scope: &crate::harness::adapter::SetupScopeRequest,
    native_binary: Option<&Path>,
    options: crate::harness::adapter::SetupOptions,
    environment: &crate::harness::adapter::SetupEnvironment,
    expected_scope: Option<&crate::harness::adapter::ResolvedSetupScope>,
) -> Result<Value, RunError> {
    use crate::harness::adapter::*;
    crate::harness::setup::validate_local_request(
        Some(registration),
        verb == SetupVerb::Install,
        scope,
        &options,
    )
    .map_err(adapter_run_error)?;
    if verb == SetupVerb::Install && !environment.executable.is_absolute() {
        return Err(invalid("owned executable must be an absolute path"));
    }
    if let Some(binary) = native_binary {
        if verb == SetupVerb::Remove {
            return Err(invalid(
                "--harness-binary is not used by unsetup: removal never depends on the harness version",
            ));
        }
        if !binary.is_absolute() {
            return Err(invalid("--harness-binary must be an absolute path"));
        }
    }
    if registration.setup_environment_inputs().len() > 32
        || environment.declared.len() > 32
        || environment
            .declared
            .values()
            .any(|value| value.len() > 16_384)
    {
        return Err(invalid(
            "declared adapter environment exceeds local request limits",
        ));
    }
    let scope = registration
        .resolve_setup_scope(scope, environment)
        .map_err(adapter_run_error)?;
    if expected_scope.is_some_and(|expected| expected != &scope) {
        return Err(invalid(
            "installer scope changed after ownership inspection; preserved",
        ));
    }
    let budget = crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(
            environment.clock.monotonic_now().0.saturating_add(30_000),
        ),
        cancellation: Default::default(),
    };
    match verb {
        SetupVerb::Install => registration
            .setup(
                &crate::harness::adapter::SetupRequest {
                    scope,
                    executable: environment.executable.clone(),
                    environment: environment.clone(),
                    native_binary: native_binary.map(Path::to_path_buf),
                    options,
                },
                &budget,
            )
            .map(|outcome| outcome.projection)
            .map_err(adapter_run_error),
        SetupVerb::Remove => registration
            .unsetup(
                &UnsetupRequest {
                    scope,
                    environment: environment.clone(),
                },
                &budget,
            )
            .map(|outcome| outcome.projection)
            .map_err(adapter_run_error),
        SetupVerb::Status => match registration.status(
            &StatusRequest {
                scope,
                environment: environment.clone(),
                native_binary: native_binary.map(Path::to_path_buf),
            },
            &budget,
        ) {
            SetupStatus::Detailed(status) => Ok(status.projection),
            SetupStatus::Failed(error) => Err(adapter_run_error(error)),
            SetupStatus::Unsupported(error) => Err(invalid(error.to_string())),
            _ => Err(invalid("adapter did not provide local setup status")),
        },
    }
}

// -------------------------------------------------------------- all harnesses

/// `setup`, `unsetup` or `setup-status` with no harness named: the per-harness
/// summary is rendered, then a nonzero status is returned (as
/// [`RunError::Exit`]) only when a harness failed.
pub fn run_all<W: Write>(
    verb: SetupVerb,
    prompt_suggestions: PromptSuggestionPolicy,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let env = SetupEnv::from_process(output)?;
    let mut report = execute_all(verb, prompt_suggestions, &env)?;
    if verb == SetupVerb::Install
        && prompt_suggestions == PromptSuggestionPolicy::Ask
        && interactive()
        && let Some(entries) = report["harnesses"].as_array_mut()
    {
        for entry in entries {
            if !entry["report"].is_object() {
                continue;
            }
            let registry = crate::harness::registry::builtins();
            if let Some(name) = entry["harness"].as_str()
                && let Ok(id) = registry.agent(name)
                && let Ok(registration) = registry.by_id(id)
            {
                registration
                    .settle_setup_consent(
                        &env.snapshot(),
                        &mut entry["report"],
                        &mut io::stdin().lock(),
                        &mut io::stderr(),
                    )
                    .map_err(adapter_run_error)?;
            }
        }
    }
    let bytes = match output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&json!({ "setup": report }))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => render_all_text(&report).into_bytes(),
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    if let Some(entries) = report["harnesses"].as_array() {
        for entry in entries {
            if entry["outcome"] == "failed" {
                eprintln!(
                    "herdr-threads: {} {}: {}",
                    verb_name(verb),
                    entry["harness"].as_str().unwrap_or_default(),
                    entry["error"].as_str().unwrap_or_default()
                );
            }
        }
    }
    match report["exit_status"].as_i64() {
        Some(0) | None => Ok(()),
        Some(code) => Err(RunError::Exit(code as i32)),
    }
}

fn verb_name(verb: SetupVerb) -> &'static str {
    match verb {
        SetupVerb::Install => "setup",
        SetupVerb::Remove => "unsetup",
        SetupVerb::Status => "setup-status",
    }
}

/// Run one setup command for every harness and return the combined report.
///
/// - `setup` installs for each harness found on PATH. One that is absent is
///   `skipped`, without failing the combined command.
/// - `unsetup` removes the recorded installation of both harnesses, whether
///   or not the harness is still on PATH (removal never depends on it).
/// - `setup-status` reports both harnesses.
///
/// `exit_status` is that of the first harness that `failed`, else 0. An
/// undetectable Herdr instance is refused up front (status 2) whenever a
/// harness would be written.
pub fn execute_all(
    verb: SetupVerb,
    prompt_suggestions: PromptSuggestionPolicy,
    env: &SetupEnv,
) -> Result<Value, RunError> {
    execute_all_registered(
        crate::harness::registry::builtins(),
        verb,
        prompt_suggestions,
        &env.snapshot(),
    )
}

pub fn execute_all_registered(
    registry: &crate::harness::registry::Registry,
    verb: SetupVerb,
    prompt_suggestions: PromptSuggestionPolicy,
    snapshot: &crate::harness::adapter::SetupEnvironment,
) -> Result<Value, RunError> {
    let env = &SetupEnv::from_snapshot(snapshot);
    let found: Vec<(
        &crate::harness::registry::Registration,
        Harness,
        Option<PathBuf>,
    )> = registry
        .registrations()
        .iter()
        .map(|registration| {
            let harness = crate::harness::registry::OccupantHarness::Agent(
                registry
                    .agent(registration.metadata().id)
                    .expect("validated registry identity"),
            )
            .into();
            (
                registration,
                harness,
                match registration.metadata().executable {
                    crate::harness::adapter::ExecutableLookup::Path(name) => {
                        hook::resolve_on_path(name, env.path.as_deref())
                    }
                    _ => None,
                },
            )
        })
        .collect();
    let eligible = |registration: &&crate::harness::registry::Registration| {
        registration
            .metadata()
            .setup_scopes
            .iter()
            .any(|scope| matches!(scope, crate::harness::adapter::SetupScopeKind::ConfigRoot))
            && !registration
                .metadata()
                .setup_scopes
                .iter()
                .any(|scope| matches!(scope, crate::harness::adapter::SetupScopeKind::Profile))
    };
    match verb {
        SetupVerb::Install => {
            if let Some((_, harness, _)) = found
                .iter()
                .find(|(registration, _, binary)| eligible(registration) && binary.is_some())
            {
                env.hook_argv(*harness)?;
            }
        }
        SetupVerb::Remove
            if found
                .iter()
                .any(|(registration, _, _)| eligible(registration)) =>
        {
            env.state_dir()?;
        }
        _ => {}
    }
    let mut entries = Vec::new();
    let mut exit_status = 0;
    let mut trust_reminder = Value::Null;
    for (registration, harness, binary) in found {
        let name = harness_name(harness);
        let mut entry = json!({"harness": name, "detected": binary.is_some()});
        if !eligible(&registration) {
            entry["outcome"] = json!("skipped");
            entry["reason"] = json!("requires explicit harness selection for a profile scope");
            entries.push(entry);
            continue;
        }
        if verb == SetupVerb::Install && binary.is_none() {
            entry["outcome"] = json!("skipped");
            entry["reason"] = json!(format!("no executable `{name}` on PATH"));
            entries.push(entry);
            continue;
        }
        let mut options = crate::harness::adapter::SetupOptions::new();
        for option in registration.setup_options() {
            let enabled = match option.name {
                "disable-prompt-suggestions" => {
                    prompt_suggestions == PromptSuggestionPolicy::Disable
                }
                "keep-prompt-suggestions" => prompt_suggestions == PromptSuggestionPolicy::Keep,
                _ => false,
            };
            if enabled {
                options.insert(option.name.into(), true);
            }
        }
        match execute_registered(
            registration,
            verb,
            &Default::default(),
            None,
            options,
            snapshot,
        ) {
            Ok(mut report) => {
                entry["outcome"] = match verb {
                    SetupVerb::Status => json!("status"),
                    _ => report["action"].clone(),
                };
                if verb == SetupVerb::Install
                    && let Some(trust) = report["trust"].as_object_mut()
                {
                    // Printed once for the whole run (`trust_reminder`).
                    if let Some(note) = trust.remove("note") {
                        trust_reminder = note;
                    }
                }
                entry["report"] = report;
            }
            Err(RunError::Api(error)) if error.code == ErrorCode::UnsupportedHarness => {
                entry["outcome"] = json!("refused");
                entry["reason"] = json!(error.detail);
            }
            Err(error) => {
                let status = error.exit_code();
                if exit_status == 0 {
                    exit_status = status;
                }
                entry["outcome"] = json!("failed");
                entry["error"] = json!(error.to_string());
                entry["exit_status"] = json!(status);
            }
        }
        entries.push(entry);
    }
    Ok(json!({
        "action": format!("{}_all", match verb {
            SetupVerb::Install => "install",
            SetupVerb::Remove => "remove",
            SetupVerb::Status => "status",
        }),
        "harnesses": entries,
        "trust_reminder": trust_reminder,
        "exit_status": exit_status,
    }))
}

/// One summary line per harness, its warnings, then the Codex trust
/// reminder once.
pub fn render_all_text(report: &Value) -> String {
    let mut out = String::new();
    let mut warnings = Vec::new();
    for entry in report["harnesses"].as_array().into_iter().flatten() {
        let name = scalar(&entry["harness"]);
        let inner = &entry["report"];
        let version = || {
            let observed = &inner["harness_version"];
            if observed["admission"] == "contract_declared" {
                format!("{name} contract declared ({})", scalar(&observed["recipe"]))
            } else {
                format!("executable unavailable: {}", scalar(&observed["refusal"]))
            }
        };
        let file = || {
            let mut files = vec![scalar(match &inner["settings"] {
                Value::Null => &inner["hooks_file"],
                settings => settings,
            })];
            if inner["sandbox"]["present"] == true {
                files.push(scalar(&inner["config_file"]));
            }
            files.join(", ")
        };
        let line = match entry["outcome"].as_str().unwrap_or_default() {
            "installed" => format!("installed: {}; {}", version(), file()),
            "already_installed" => format!("already installed: {}; {}", version(), file()),
            "adopted" => format!(
                "adopted the existing hook groups of another setup (owner {}), file unchanged: \
                 {}; {}",
                scalar(&inner["adopted"]),
                version(),
                file()
            ),
            "skipped" => format!("skipped: {}", scalar(&entry["reason"])),
            "refused" => format!("refused: {}", scalar(&entry["reason"])),
            "failed" => format!(
                "failed (exit {}): {}",
                scalar(&entry["exit_status"]),
                scalar(&entry["error"])
            ),
            "removed" => format!("removed the owned entries; {}", file()),
            "not_installed" => "nothing to remove (no installation recorded)".to_owned(),
            "status" => {
                let installed = if inner["installed"] == true && inner["adopted"].is_object() {
                    "installed (adopted from another setup)"
                } else if inner["installed"] == true {
                    "installed"
                } else {
                    "not installed"
                };
                let detected = if entry["detected"] == true {
                    version()
                } else {
                    format!("no executable `{name}` on PATH")
                };
                let mut line = format!("{installed}; {detected}; {}", file());
                if let Some(error) = inner["error"].as_str() {
                    line.push_str(&format!("; {}", scalar(&json!(error))));
                }
                line
            }
            other => other.to_owned(),
        };
        out.push_str(&format!("{name}: {line}\n"));
        if let Some(line) = prompt_suggestion_line(&inner["prompt_suggestions"]) {
            out.push_str(&format!("{name}: {line}\n"));
        }
        for warning in inner["warnings"].as_array().into_iter().flatten() {
            warnings.push(format!("warning: {name}: {}\n", scalar(warning)));
        }
    }
    for warning in warnings {
        out.push_str(&warning);
    }
    if report["action"] == "install_all" {
        out.push_str("installed is not observed: run `herdr-threads doctor` for native evidence\n");
    }
    if let Some(note) = report["trust_reminder"].as_str() {
        out.push_str(&format!("codex hook trust: {}\n", scalar(&json!(note))));
    }
    out
}

// ----------------------------------------------------- executable selection

/// Both stdin and stderr are terminals: setup may ask a question.
fn interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// The one-line summary `setup`/`unsetup` with no harness print for Claude's
/// prompt suggestions; `None` when there is nothing to say (the advice is a
/// warning).
fn prompt_suggestion_line(value: &Value) -> Option<String> {
    let setting = claude::PROMPT_SUGGESTION_SETTING;
    Some(match value {
        Value::String(outcome) => match outcome.as_str() {
            "reverted" => format!("prompt suggestions: reverted `{setting}` (setup had set it)"),
            "left_changed" => {
                format!(
                    "prompt suggestions: `{setting}` was changed by hand since setup; left as is"
                )
            }
            _ => return None,
        },
        Value::Object(_) => match value["action"].as_str()? {
            "disabled" => format!("prompt suggestions: disabled (`{setting}: false`)"),
            "already_disabled" => "prompt suggestions: already disabled".to_owned(),
            "kept" => format!(
                "prompt suggestions: kept ({}); Claude pokes skip a pane that shows one",
                scalar(&value["state"])
            ),
            _ => return None,
        },
        _ => return None,
    })
}

// ------------------------------------------------------------------ render

fn scalar(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Null => "none".into(),
        other => other.to_string(),
    };
    crate::view::escape::escape_for_terminal(&text, crate::view::escape::Context::SingleLine)
        .into_owned()
}

/// Compact `key: value` text form.
pub fn render_text(report: &Value) -> String {
    let mut out = String::new();
    let Some(map) = report.as_object() else {
        return out;
    };
    for (key, value) in map {
        match value {
            Value::Object(inner) => {
                for (sub, value) in inner {
                    out.push_str(&format!("{key}.{sub}: {}\n", scalar(value)));
                }
            }
            Value::Array(items) if key == "warnings" => {
                for item in items {
                    out.push_str(&format!("warning: {}\n", scalar(item)));
                }
            }
            Value::Array(items) => {
                let words: Vec<_> = items.iter().map(scalar).collect();
                out.push_str(&format!("{key}: {}\n", words.join(" ")));
            }
            other => out.push_str(&format!("{key}: {}\n", scalar(other))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn scalar_escapes_bidi_and_format_controls() {
        let text = scalar(&json!("ok\u{202e}evil\u{2066}x\u{200b}\u{1b}[31m\n"));
        for raw in ['\u{202e}', '\u{2066}', '\u{200b}', '\u{1b}', '\n'] {
            assert!(!text.contains(raw), "{raw:?} survived: {text:?}");
        }
        assert_eq!(
            text,
            crate::view::escape::escape_for_terminal(
                "ok\u{202e}evil\u{2066}x\u{200b}\u{1b}[31m\n",
                crate::view::escape::Context::SingleLine
            )
        );
        assert_eq!(scalar(&Value::Null), "none");
        assert_eq!(scalar(&json!(3)), "3");
        assert_eq!(scalar(&json!("plain")), "plain");
    }

    fn env(state: Option<&str>) -> SetupEnv {
        SetupEnv {
            home: None,
            declared_environment: Default::default(),
            executable: PathBuf::from("/opt/h t/herdr-threads"),
            state_dir: state.map(PathBuf::from),
            cwd: PathBuf::from("/"),
            path: None,
            codex_home: None,
            claude_config_dir: None,
            host_endpoint: Some(PathBuf::from("/tmp/herdr.sock")),
            instance_source: Value::Null,
        }
    }

    /// Hook argv contract: the command setup installs is exactly the hook
    /// entrypoint's `installed_argv`, and the entrypoint's own parser selects
    /// the hook path for it with the same state dir and harness. Kills:
    /// dropping the state dir, reordering it after the subcommand, renaming
    /// `hook`, or swapping the harness word.
    #[test]
    fn installed_hook_argv_round_trips_through_the_hook_entrypoint_parser() {
        for harness in [Harness::Claude, Harness::Codex] {
            let argv = env(Some("/s d")).hook_argv(harness).expect("argv");
            let word = harness_name(harness);
            let host = Path::new("/tmp")
                .canonicalize()
                .unwrap()
                .join("herdr.sock")
                .display()
                .to_string();
            assert_eq!(
                argv,
                [
                    "/opt/h t/herdr-threads",
                    "--state-dir",
                    "/s d",
                    "--host-endpoint",
                    host.as_str(),
                    "hook",
                    word
                ]
            );
            let os: Vec<OsString> = argv.iter().map(OsString::from).collect();
            let parsed = hook::parse_hook_argv(&os)
                .expect("selects the hook entrypoint")
                .expect("valid hook argv");
            assert_eq!(parsed.state_dir.as_deref(), Some(Path::new("/s d")));
            assert_eq!(parsed.host_endpoint.as_deref(), Some(Path::new(&host)));
            assert_eq!(parsed.harness, harness);
            assert_eq!(parsed.event, None);
            // The registered per-event form: the installed argv plus `--event NAME`.
            let mut evented = os.clone();
            evented.extend([OsString::from("--event"), OsString::from("SessionStart")]);
            let parsed = hook::parse_hook_argv(&evented)
                .expect("selects the hook entrypoint")
                .expect("valid evented hook argv");
            assert_eq!(parsed.harness, harness);
            assert_eq!(parsed.event.as_deref(), Some("SessionStart"));
            assert_eq!(parsed.state_dir.as_deref(), Some(Path::new("/s d")));
        }
        assert!(env(None).hook_argv(Harness::Claude).is_err());
    }

    /// CODEX_HOME resolution as Codex does it. Kills: ignoring CODEX_HOME,
    /// honouring an empty one, or dropping the `$HOME/.codex` fallback.
    #[test]
    fn codex_home_prefers_non_empty_codex_home_then_home_dot_codex() {
        let os = |v: &str| Some(std::ffi::OsString::from(v));
        assert_eq!(codex_home_from(os("/ch"), os("/h")), Some("/ch".into()));
        assert_eq!(codex_home_from(os(""), os("/h")), Some("/h/.codex".into()));
        assert_eq!(codex_home_from(None, os("/h")), Some("/h/.codex".into()));
        assert_eq!(codex_home_from(None, None), None);
    }

    /// Kills: dropping any accepted TOML spelling, matching a non-owned event,
    /// counting a commented line, or leaking the `[hooks]` table state into a
    /// later table.
    #[test]
    fn toml_scan_finds_owned_hook_events_in_every_spelling() {
        let text = "# [[hooks.PreToolUse]]\n[[hooks.SessionStart]]\nhooks = []\n\
                    [hooks]\n\"SubagentStart\" = []\nStop = []\n\
                    [other]\nPreToolUse = 1\n";
        assert_eq!(toml_hook_events(text), ["SessionStart", "SubagentStart"]);
        assert_eq!(
            toml_hook_events("hooks . PreToolUse = []\n[hooks.SessionStart.x]\n"),
            ["PreToolUse", "SessionStart"]
        );
        assert!(toml_hook_events("hooks.Stop = []\n").is_empty());
    }

    /// Kills: matching only the raw command, so a command whose path needs
    /// TOML/JSON escaping is missed, or flagging another installation's command
    /// as identical.
    #[test]
    fn layer_observation_matches_the_escaped_owned_command() {
        let dir = std::env::temp_dir().join(format!("ht-layer-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let command = r#"'/a"b\c/herdr-threads' '--state-dir' '/s' 'hook' 'codex'"#;
        let path = dir.join("config.toml");
        fs::write(
            &path,
            format!(
                "hooks.SessionStart = [{{hooks=[{{type=\"command\",command={}}}]}}]\n",
                serde_json::to_string(command).unwrap()
            ),
        )
        .unwrap();
        let layer = observe_codex_layer(&path, command).expect("present");
        assert!(layer.has_owned_command);
        assert_eq!(layer.events, ["SessionStart"]);
        let other = command.replace("'/s'", "'/t'");
        let layer = observe_codex_layer(&path, &other).expect("present");
        assert!(!layer.has_owned_command);
        assert!(layer.has_other_herdr_threads_hook);
        assert!(observe_codex_layer(&dir.join("hooks.json"), command).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: skipping CODEX_HOME, the system layer or project `.codex/`
    /// layers, or walking past the Git root.
    #[test]
    fn layer_paths_cover_codex_home_system_and_project_up_to_git_root() {
        let dir = std::env::temp_dir().join(format!("ht-walk-{}", uuid::Uuid::new_v4()));
        let sub = dir.join("repo").join("a");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir(dir.join("repo").join(".git")).unwrap();
        let paths = codex_layer_paths(Some(Path::new("/ch")), &sub);
        let expect = [
            PathBuf::from("/ch/config.toml"),
            PathBuf::from("/ch/hooks.json"),
            PathBuf::from("/etc/codex/config.toml"),
            PathBuf::from("/etc/codex/hooks.json"),
            sub.join(".codex/config.toml"),
            sub.join(".codex/hooks.json"),
            dir.join("repo/.codex/config.toml"),
            dir.join("repo/.codex/hooks.json"),
        ];
        assert_eq!(paths, expect);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: a recorded executable read wrongly (quotes in the path, spaces), which would make
    /// the moved-binary message name a wrong path or fire when the executable did not move.
    #[test]
    fn first_shell_word_reads_the_quoted_executable() {
        for exe in ["/a/b", "/tmp/space dir/it's here", "/q''x"] {
            let command = shell_command(&[exe.to_owned(), "hook".into(), "codex".into()]).unwrap();
            assert_eq!(
                first_shell_word(&command).as_deref(),
                Some(exe),
                "{command}"
            );
            let marked = format!("{command} # herdr-threads-owner:abc");
            assert_eq!(first_shell_word(&marked).as_deref(), Some(exe));
        }
        assert_eq!(first_shell_word("unquoted word"), None);
    }

    /// P2 (ht-p03.15). Kills: walking to `/` when no `.git` is found, which lists `.codex/`
    /// files Codex would not load (and `refuse_duplicate` then refuses on one), and a walk that
    /// stops short of an existing Git root.
    #[test]
    fn codex_layer_paths_stop_at_cwd_without_a_git_root() {
        let dir = std::env::temp_dir().join(format!("ht-walk-{}", uuid::Uuid::new_v4()));
        let cwd = dir.join("x/a/b");
        fs::create_dir_all(&cwd).unwrap();
        let layers = |root: &Path| {
            ["config.toml", "hooks.json"]
                .into_iter()
                .map(|name| root.join(".codex").join(name))
                .collect::<Vec<_>>()
        };
        let head = [
            PathBuf::from("/ch/config.toml"),
            PathBuf::from("/ch/hooks.json"),
            PathBuf::from("/etc/codex/config.toml"),
            PathBuf::from("/etc/codex/hooks.json"),
        ];
        let expect = |roots: &[PathBuf]| {
            head.iter()
                .cloned()
                .chain(roots.iter().flat_map(|root| layers(root)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            codex_layer_paths(Some(Path::new("/ch")), &cwd),
            expect(std::slice::from_ref(&cwd))
        );
        fs::create_dir(dir.join("x/.git")).unwrap();
        assert_eq!(
            codex_layer_paths(Some(Path::new("/ch")), &cwd),
            expect(&[cwd.clone(), dir.join("x/a"), dir.join("x")])
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// An isolated state directory and Claude config dir, with `settings`
    /// as the user settings, and an `advised` install report for them.
    fn suggestion_fixture(settings: &[u8]) -> (PathBuf, SetupEnv, Value) {
        let root = std::env::temp_dir().join(format!("setup-suggestions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("state/setup")).unwrap();
        fs::create_dir_all(root.join("claude")).unwrap();
        fs::write(root.join("claude/settings.json"), settings).unwrap();
        let mut env = env(Some(root.join("state").to_str().unwrap()));
        env.claude_config_dir = Some(root.join("claude"));
        let path = root.join("claude/settings.json");
        let mut warnings = Vec::new();
        let report = json!({
            "prompt_suggestions": prompt_suggestion_step(
                PromptSuggestionPolicy::Ask, &env, &path, &mut warnings
            )
            .unwrap(),
            "warnings": warnings,
        });
        (root, env, report)
    }

    fn settle(env: &SetupEnv, report: &mut Value, answer: &str) -> String {
        let mut out = Vec::new();
        settle_prompt_suggestions(env, report, &mut io::Cursor::new(answer), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Interactive yes: the explanation and `[y/N]` are printed, the key is
    /// written false and recorded (so unsetup can revert it), and the advice
    /// warning is dropped. Kills: writing without a yes, not recording.
    #[test]
    fn interactive_yes_disables_and_records() {
        let (root, env, mut report) = suggestion_fixture(b"{\"model\":\"x\"}");
        assert_eq!(report["prompt_suggestions"]["action"], "advised");
        assert_eq!(report["warnings"].as_array().unwrap().len(), 1);
        let asked = settle(&env, &mut report, "y\n");
        assert!(asked.contains(PROMPT_SUGGESTION_EXPLANATION), "{asked}");
        assert!(
            asked.ends_with("Disable prompt suggestions? [y/N] "),
            "{asked}"
        );
        assert_eq!(report["prompt_suggestions"]["action"], "disabled");
        assert_eq!(report["prompt_suggestions"]["set_by_setup"], true);
        assert_eq!(report["warnings"], json!([]));
        let settings = root.join("claude/settings.json");
        let value: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
        assert_eq!(
            value,
            json!({"model": "x", "promptSuggestionEnabled": false})
        );
        let manifest = prompt_suggestion_manifest(&env, &settings).unwrap();
        assert_eq!(
            prompt_suggestion::revert(&settings, &manifest).unwrap(),
            prompt_suggestion::RevertOutcome::Reverted
        );
        assert_eq!(fs::read(&settings).unwrap(), b"{\"model\":\"x\"}");
        fs::remove_dir_all(root).unwrap();
    }

    /// Interactive no (and the default: an empty answer or end of input)
    /// keeps the setting untouched. Kills: treating anything but y/yes as yes.
    #[test]
    fn interactive_no_and_default_keep() {
        for answer in ["n\n", "\n", "", "maybe\n"] {
            let (root, env, mut report) = suggestion_fixture(b"{}");
            settle(&env, &mut report, answer);
            assert_eq!(report["prompt_suggestions"]["action"], "kept", "{answer:?}");
            assert_eq!(report["warnings"], json!([]));
            assert_eq!(fs::read(root.join("claude/settings.json")).unwrap(), b"{}");
            assert!(
                !prompt_suggestion_manifest(&env, &root.join("claude/settings.json"))
                    .unwrap()
                    .exists()
            );
            fs::remove_dir_all(root).unwrap();
        }
        let (root, env, mut report) = suggestion_fixture(b"{}");
        settle(&env, &mut report, "YES\n");
        assert_eq!(report["prompt_suggestions"]["action"], "disabled");
        fs::remove_dir_all(root).unwrap();
    }

    /// Nothing is asked when the setting is already false or a flag decided.
    #[test]
    fn no_question_unless_advised() {
        let (root, env, mut report) = suggestion_fixture(b"{\"promptSuggestionEnabled\":false}");
        assert_eq!(report["prompt_suggestions"]["action"], "already_disabled");
        assert_eq!(settle(&env, &mut report, "y\n"), "");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod detect_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_herdr(dir: &Path, plugin_list: &str, status: &str) -> PathBuf {
        let bin = dir.join("herdr");
        fs::write(
            &bin,
            format!(
                "#!/bin/sh\ncase \"$1 $2\" in\n'plugin list') printf '%s' '{plugin_list}';;\n\
                 'status server') printf '%s' '{status}';;\n*) exit 3;;\nesac\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ht-detect-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        dir
    }

    const LISTED: &str = r#"{"result":{"plugins":[{"plugin_id":"herdr-threads","enabled":true}]}}"#;

    /// Kills: guessing a state directory without the plugin installed,
    /// ignoring XDG_STATE_HOME, accepting a disabled plugin, or silently
    /// picking one of two existing state roots.
    #[test]
    fn state_dir_detection_follows_herdr_plugin_state_root_and_refuses_ambiguity() {
        let dir = scratch();
        let home = dir.join("home");
        let xdg = dir.join("xdg");
        let mut inputs = DetectInputs {
            herdr: Some(fake_herdr(&dir, LISTED, "{}")),
            home: Some(home.clone()),
            xdg_state_home: None,
        };
        assert_eq!(
            detect_state_dir(&inputs).unwrap().0,
            home.join(".local/state/herdr/plugins/herdr-threads")
        );
        inputs.xdg_state_home = Some(xdg.clone());
        assert_eq!(
            detect_state_dir(&inputs).unwrap().0,
            xdg.join("herdr/plugins/herdr-threads")
        );
        fs::create_dir_all(xdg.join("herdr/plugins/herdr-threads")).unwrap();
        fs::create_dir_all(home.join(".local/state/herdr/plugins/herdr-threads")).unwrap();
        // Two directories without a store: the XDG one wins (the other is a stale leftover).
        assert_eq!(
            detect_state_dir(&inputs).unwrap().0,
            xdg.join("herdr/plugins/herdr-threads")
        );
        for root in [
            xdg.join("herdr/plugins/herdr-threads"),
            home.join(".local/state/herdr/plugins/herdr-threads"),
        ] {
            fs::create_dir_all(root.join("instances/abc")).unwrap();
            fs::write(root.join("instances/abc/threads.sqlite3"), "").unwrap();
        }
        assert!(detect_state_dir(&inputs).unwrap_err().contains("both"));
        inputs.herdr = Some(fake_herdr(&dir, r#"{"result":{"plugins":[]}}"#, "{}"));
        assert!(
            detect_state_dir(&inputs)
                .unwrap_err()
                .contains("not installed")
        );
        inputs.herdr = Some(fake_herdr(
            &dir,
            r#"{"result":{"plugins":[{"plugin_id":"herdr-threads","enabled":false}]}}"#,
            "{}",
        ));
        assert!(detect_state_dir(&inputs).unwrap_err().contains("disabled"));
        inputs.herdr = None;
        assert!(detect_state_dir(&inputs).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: using a stopped server's socket, a relative socket, or
    /// answering without asking Herdr.
    #[test]
    fn host_detection_uses_the_running_server_socket() {
        let dir = scratch();
        let inputs = DetectInputs {
            herdr: Some(fake_herdr(
                &dir,
                LISTED,
                r#"{"running":true,"socket":"/x/herdr.sock"}"#,
            )),
            ..DetectInputs::default()
        };
        assert_eq!(
            detect_host_endpoint(&inputs).unwrap().0,
            PathBuf::from("/x/herdr.sock")
        );
        let stopped = DetectInputs {
            herdr: Some(fake_herdr(
                &dir,
                LISTED,
                r#"{"running":false,"socket":"/x/herdr.sock"}"#,
            )),
            ..DetectInputs::default()
        };
        assert!(
            detect_host_endpoint(&stopped)
                .unwrap_err()
                .contains("not running")
        );
        let relative = DetectInputs {
            herdr: Some(fake_herdr(
                &dir,
                LISTED,
                r#"{"running":true,"socket":"herdr.sock"}"#,
            )),
            ..DetectInputs::default()
        };
        assert!(detect_host_endpoint(&relative).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: a trust key that does not match Codex's `hooks/list` spelling.
    #[test]
    fn trust_keys_use_codex_snake_case_event_names() {
        assert_eq!(snake("SessionStart"), "session_start");
        assert_eq!(snake("SubagentStart"), "subagent_start");
        assert_eq!(snake("PreToolUse"), "pre_tool_use");
    }
}

pub use crate::harness::setup::legacy::manifest_path;

#[cfg(test)]
use crate::harness::setup::legacy::first_shell_word;
pub(crate) use crate::harness::setup::legacy::user_inspection;

pub use crate::harness::claude::setup::{
    PROMPT_SUGGESTION_EXPLANATION, allow_rule_json, claude_config_dir_from, claude_paths,
    prompt_suggestion_manifest, prompt_suggestion_status, settle_prompt_suggestions,
};

#[cfg(test)]
use crate::harness::claude::setup::prompt_suggestion_step;

pub use crate::harness::codex::setup::{
    CODEX_SANDBOX_MEASURED_VERSIONS, CODEX_TRUST_NOTE, CODEX_WRITABLE_DIRS, CodexLayerFile,
    CodexPaths, codex_home_from, codex_layer_paths, codex_paths, codex_sandbox_note,
    observe_codex_layer,
};

pub(crate) use crate::harness::codex::setup::{
    codex_foreign_proxy_warnings, codex_missing_roots_warning, codex_trust_report,
    codex_unmeasured_allowance_warning,
};
#[cfg(test)]
use crate::harness::codex::setup::{snake, toml_hook_events};
