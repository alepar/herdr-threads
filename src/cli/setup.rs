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

mod foreground;

use super::{RunError, hook};
use crate::{
    daemon::paths::{RuntimeContext, ensure_owned_state_root, ensure_private_dir, instance_dir},
    harness::{
        claude,
        claude_mod::{self, ManagedPolicy, ManagedPolicySources},
        codex, codex_config,
        context::Harness,
        prompt_suggestion::{self, SuggestionState},
        recipe,
        setup::{
            self as lib, AllowRuleInspection, AllowRuleOwnership, NativeObservation, SettingsKind,
            SetupError, adopt_user_settings, adoptable_user_settings, inspect_user_settings,
            inspect_user_settings_for, install_user_settings, read_settings_manifest,
            remove_user_settings, shared_command, shell_command,
        },
    },
    protocol::{
        output::{OutputFormat, OutputSpec},
        results::{ApiError, ErrorCode},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::{self, Read, Write},
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

Foreground agents (user-managed, read-only inspection): setup and setup-status report user
settings and manual advice for both harnesses. Claude: merge `disableAgentView: true` into
user settings.json, or set its env.CLAUDE_CODE_DISABLE_AGENT_VIEW to `1`. The inspected Codex has
no persistent config/environment daemon opt-out; features.daemon_auto_start=false still attaches
to an existing daemon. Use native --no-daemon, or HERDR_THREADS_CODEX_OPTS='--no-daemon'
when the selected binary/wrapper supports and forwards it. Setup never sets these choices;
configured settings/arguments do not prove foreground execution and overrides may change them.

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
    pub verb: SetupVerb,
    pub harness: Harness,
    pub harness_binary: Option<String>,
    /// `setup claude` only: what to do about Claude's prompt suggestions.
    pub prompt_suggestions: PromptSuggestionPolicy,
    /// `setup claude --hooks-only`: install or keep the hooks, and remove the
    /// delivery mod's `CLAUDE_CODE_PLUGIN_DIRS` path and files.
    pub hooks_only: bool,
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

/// Why setup offers to turn Claude's prompt suggestions off.
pub const PROMPT_SUGGESTION_EXPLANATION: &str = "Claude Code shows a dim prompt suggestion in \
its input box after every turn (on by default). herdr-threads cannot tell that suggestion from \
text you typed, so it never sends a soft-deadline reminder (poke) into a Claude pane that shows \
one; only the hard-deadline warning reaches it. With prompt suggestions off \
(`promptSuggestionEnabled: false` in Claude's user settings), an idle Claude pane reads empty and \
pokes reach it. unsetup reverts the setting if setup set it.";

/// The non-interactive advice (nothing was changed).
fn prompt_suggestion_advice(settings: &Path) -> String {
    format!(
        "{PROMPT_SUGGESTION_EXPLANATION} Prompt suggestions are on in {} and were left \
         unchanged: re-run `herdr-threads setup claude --disable-prompt-suggestions` to turn them \
         off (or turn off Prompt suggestions in Claude's /config), or pass \
         --keep-prompt-suggestions to keep them without this note",
        settings.display()
    )
}

/// A person's pane identity (`me init`) has no hooks to set up.
const NO_HUMAN_SETUP: &str =
    "setup manages agent hooks only; a person uses `herdr-threads me init`";

fn harness_name(harness: Harness) -> &'static str {
    match harness {
        Harness::Claude => "claude",
        Harness::Codex => "codex",
        Harness::Human => "human",
    }
}

fn api(code: ErrorCode, detail: impl Into<String>) -> RunError {
    RunError::Api(ApiError::new(code, detail))
}

fn invalid(detail: impl Into<String>) -> RunError {
    api(ErrorCode::InvalidRequest, detail)
}

fn failed(detail: impl Into<String>) -> RunError {
    RunError::Io(io::Error::other(detail.into()))
}

/// Everything the commands take from the process, injectable for tests.
#[derive(Debug, Clone)]
pub struct SetupEnv {
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

impl SetupEnv {
    pub(crate) fn from_process(output: &OutputSpec) -> Result<Self, RunError> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let path = std::env::var_os("PATH");
        let inputs = super::instance::InstanceInputs::from_process(
            output.context.state_dir.as_ref().map(PathBuf::from),
            output.context.host.as_ref().map(PathBuf::from),
        );
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
        Ok(Self {
            executable,
            state_dir,
            cwd: std::env::current_dir()?,
            path,
            codex_home: codex_home_from(std::env::var_os("CODEX_HOME"), std::env::var_os("HOME")),
            claude_config_dir: claude_config_dir_from(
                std::env::var_os("CLAUDE_CONFIG_DIR"),
                std::env::var_os("HOME"),
            ),
            host_endpoint,
            instance_source: Value::Object(source),
        })
    }

    /// The environment of a command that already resolved its runtime
    /// context (`doctor`): the instance is that context's.
    pub(crate) fn for_context(context: &RuntimeContext) -> Self {
        Self {
            executable: std::env::current_exe()
                .and_then(|path| path.canonicalize())
                .unwrap_or_default(),
            state_dir: Some(context.state_dir.clone()),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            path: std::env::var_os("PATH"),
            codex_home: codex_home_from(std::env::var_os("CODEX_HOME"), std::env::var_os("HOME")),
            claude_config_dir: claude_config_dir_from(
                std::env::var_os("CLAUDE_CONFIG_DIR"),
                std::env::var_os("HOME"),
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

    fn claude_settings(&self) -> Result<PathBuf, RunError> {
        self.claude_config_dir
            .as_ref()
            .map(|dir| dir.join("settings.json"))
            .ok_or_else(|| invalid("neither CLAUDE_CONFIG_DIR nor HOME is set"))
    }

    fn codex_file(&self, name: &str) -> Result<PathBuf, RunError> {
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
        && request.harness == Harness::Claude
        && request.prompt_suggestions == PromptSuggestionPolicy::Ask
        && interactive()
    {
        settle_prompt_suggestions(
            &env,
            &mut report,
            &mut io::stdin().lock(),
            &mut io::stderr(),
        )?;
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
pub fn execute(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    // A malformed argument is invalid (status 2), never an executable refusal.
    harness_binary(request, env)?;
    // A state directory left behind by an uninstalled plugin must not silently take new
    // installations (unsetup, status and every other command keep working on it).
    if request.verb == SetupVerb::Install
        && request.harness != Harness::Human
        && let Some(leftover) = env.instance_source["state_dir_leftover"].as_str()
    {
        return Err(invalid(format!("{leftover}; nothing was changed")));
    }
    let mut report = match (request.harness, request.verb) {
        (Harness::Claude, SetupVerb::Install) => claude_install(request, env),
        (Harness::Claude, SetupVerb::Remove) => claude_remove(env),
        (Harness::Claude, SetupVerb::Status) => claude_status(request, env),
        (Harness::Codex, SetupVerb::Install) => codex_install(request, env),
        (Harness::Codex, SetupVerb::Remove) => codex_remove(env),
        (Harness::Codex, SetupVerb::Status) => codex_status(request, env),
        (Harness::Human, _) => Err(api(ErrorCode::InvalidRequest, NO_HUMAN_SETUP)),
    }?;
    if request.verb != SetupVerb::Remove {
        foreground::attach(&mut report, request.harness, env);
    }
    Ok(report)
}

// -------------------------------------------------------------- all harnesses

/// Every harness the bare commands cover, in report order.
pub const HARNESSES: [Harness; 2] = [Harness::Claude, Harness::Codex];

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
        && let Some(entry) = report["harnesses"].as_array_mut().and_then(|entries| {
            entries
                .iter_mut()
                .find(|entry| entry["harness"] == "claude")
        })
        && entry["report"].is_object()
    {
        settle_prompt_suggestions(
            &env,
            &mut entry["report"],
            &mut io::stdin().lock(),
            &mut io::stderr(),
        )?;
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
    let found: Vec<(Harness, Option<PathBuf>)> = HARNESSES
        .iter()
        .map(|&harness| {
            (
                harness,
                hook::resolve_on_path(harness_name(harness), env.path.as_deref()),
            )
        })
        .collect();
    match verb {
        SetupVerb::Install if found.iter().any(|(_, binary)| binary.is_some()) => {
            env.hook_argv(Harness::Claude)?;
        }
        SetupVerb::Remove => {
            env.state_dir()?;
        }
        _ => {}
    }
    let mut entries = Vec::new();
    let mut exit_status = 0;
    let mut trust_reminder = false;
    for (harness, binary) in found {
        let name = harness_name(harness);
        let mut entry = json!({"harness": name, "detected": binary.is_some()});
        if verb == SetupVerb::Install && binary.is_none() {
            entry["outcome"] = json!("skipped");
            entry["reason"] = json!(format!("no executable `{name}` on PATH"));
            entries.push(entry);
            continue;
        }
        let request = SetupRequest {
            verb,
            harness,
            harness_binary: None,
            prompt_suggestions,
            hooks_only: false,
        };
        match execute(&request, env) {
            Ok(mut report) => {
                entry["outcome"] = match verb {
                    SetupVerb::Status => json!("status"),
                    _ => report["action"].clone(),
                };
                if harness == Harness::Codex
                    && verb == SetupVerb::Install
                    && let Some(trust) = report["trust"].as_object_mut()
                {
                    // Printed once for the whole run (`trust_reminder`).
                    trust.remove("note");
                    trust_reminder = true;
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
        "trust_reminder": if trust_reminder { json!(CODEX_TRUST_NOTE) } else { Value::Null },
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
        if let Some(line) = mod_line(&inner["mod"]) {
            out.push_str(&format!("{name}: {line}\n"));
        }
        if let Some(status) = inner["foreground"]["status"].as_str() {
            out.push_str(&format!(
                "{name}: foreground user settings: {status}; execution unknown\n"
            ));
            if status == "configured" {
                out.push_str(&format!(
                    "{name}: {} {}\n",
                    scalar(&inner["foreground"]["explanation"]),
                    scalar(&inner["foreground"]["advice"])
                ));
            }
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

fn harness_binary(request: &SetupRequest, env: &SetupEnv) -> Result<Option<PathBuf>, RunError> {
    match &request.harness_binary {
        Some(binary) => {
            let binary = PathBuf::from(binary);
            if !binary.is_absolute() {
                return Err(invalid("--harness-binary must be an absolute path"));
            }
            Ok(Some(binary))
        }
        None => Ok(hook::resolve_on_path(
            harness_name(request.harness),
            env.path.as_deref(),
        )),
    }
}

/// Executable selection and declared contract, without runtime metadata.
#[derive(Debug, Clone)]
pub(crate) struct Observed {
    pub(crate) binary: PathBuf,
    pub(crate) version: Option<String>,
    pub(crate) recipe: &'static str,
}

/// Resolve the selected wrapper without invoking it. The handle declares only
/// the registered input/setup contract, not runtime support or native delivery.
pub(crate) fn observe(
    request: &SetupRequest,
    env: &SetupEnv,
) -> Result<(Observed, Option<crate::harness::operational::CodexContract>), String> {
    use crate::harness::operational::{ClaudeContract, CodexContract};
    let name = harness_name(request.harness);
    let binary = harness_binary(request, env)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no executable `{name}` on PATH; pass --harness-binary"))?;
    use std::os::unix::fs::PermissionsExt;
    if !fs::metadata(&binary)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    {
        return Err(format!("{} is not an executable file", binary.display()));
    }
    let (recipe, contract) = match request.harness {
        Harness::Claude => {
            let _contract = ClaudeContract::registered();
            ("claude-hooks-2.1.283", None)
        }
        Harness::Codex => ("codex-hooks-v1", Some(CodexContract::registered())),
        Harness::Human => return Err(NO_HUMAN_SETUP.to_owned()),
    };
    Ok((
        Observed {
            binary,
            version: None,
            recipe,
        },
        contract,
    ))
}

fn observation_json(
    result: &Result<(Observed, Option<crate::harness::operational::CodexContract>), String>,
) -> Value {
    match result {
        Ok((observed, _)) => json!({
            "admission": "contract_declared",
            "binary": observed.binary.display().to_string(),
            "version": observed.version,
            "recipe": observed.recipe,
        }),
        Err(refusal) => json!({"admission": "unavailable", "version": null, "refusal": refusal}),
    }
}

pub(crate) fn refuse_executable(refusal: String) -> RunError {
    api(ErrorCode::UnsupportedHarness, refusal)
}

// ------------------------------------------------------------ owned files

/// Claude's config directory: a non-empty `CLAUDE_CONFIG_DIR`, else
/// `$HOME/.claude`.
pub fn claude_config_dir_from(
    config_dir: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    match config_dir.filter(|value| !value.is_empty()) {
        Some(value) => Some(PathBuf::from(value)),
        None => home
            .filter(|value| !value.is_empty())
            .map(|home| PathBuf::from(home).join(".claude")),
    }
}

/// Ownership-manifest location for one owned file: private plugin state,
/// keyed by the file's path, never beside the file.
pub fn manifest_path(state_dir: &Path, kind: &str, file: &Path) -> PathBuf {
    let digest = format!("{:x}", Sha256::digest(file.as_os_str().as_encoded_bytes()));
    state_dir
        .join("setup")
        .join(format!("{kind}-{}.json", &digest[..32]))
}

/// Records files (and their directory) setup created, so unsetup can delete
/// exactly those when they are back to their created state. Kept beside the
/// manifest.
fn created_marker_path(manifest: &Path) -> PathBuf {
    manifest.with_extension("created.json")
}

/// One file setup may create: its path, the bytes it is created with, and
/// whether this invocation created it or its directory.
struct OwnedFile {
    path: PathBuf,
    manifest: PathBuf,
    initial: &'static [u8],
    created_dir: bool,
    created_file: bool,
}

impl OwnedFile {
    fn new(path: PathBuf, manifest: PathBuf, initial: &'static [u8]) -> Self {
        Self {
            path,
            manifest,
            initial,
            created_dir: false,
            created_file: false,
        }
    }

    /// Create the file (and one missing directory level) with its initial
    /// bytes when absent. A non-directory parent refuses.
    fn prepare(&mut self) -> Result<(), RunError> {
        let dir = self.path.parent().expect("owned file parent");
        match fs::symlink_metadata(dir) {
            Ok(meta) if !meta.is_dir() && !fs::metadata(dir).is_ok_and(|m| m.is_dir()) => {
                return Err(invalid(format!(
                    "{} exists but is not a directory",
                    dir.display()
                )));
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(dir).map_err(|error| {
                    invalid(format!("could not create {}: {error}", dir.display()))
                })?;
                self.created_dir = true;
            }
            Err(error) => return Err(error.into()),
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&self.path)?
                    .write_all(self.initial)?;
                self.created_file = true;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Undo what [`prepare`](Self::prepare) created, unless a manifest now
    /// refers to it (a recorded preparation resumes).
    fn undo(&self) {
        if self.manifest.exists() {
            return;
        }
        if self.created_file && fs::read(&self.path).is_ok_and(|b| b == self.initial) {
            let _ = fs::remove_file(&self.path);
        }
        if self.created_dir
            && let Some(dir) = self.path.parent()
        {
            let _ = fs::remove_dir(dir);
        }
    }

    /// Record what was created; a warning when the record cannot be kept.
    fn record(&self, warnings: &mut Vec<String>) {
        if !(self.created_file || self.created_dir) {
            return;
        }
        let marker =
            json!({"settings_created": self.created_file, "dir_created": self.created_dir});
        let result = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode_private()
            .open(created_marker_path(&self.manifest))
            .and_then(|mut file| file.write_all(marker.to_string().as_bytes()));
        if result.is_err() {
            warnings.push(format!(
                "could not record that setup created {}; unsetup will leave it",
                self.path.display()
            ));
        }
    }
}

/// After removal: delete a file setup created once it is back to its
/// created bytes (and its directory when setup created it and it is empty).
fn delete_created(path: &Path, manifest: &Path, initial: &[u8]) -> bool {
    let marker_path = created_marker_path(manifest);
    let mut deleted = false;
    if let Ok(bytes) = fs::read(&marker_path) {
        let marker: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if marker["settings_created"] == json!(true)
            && fs::read(path).is_ok_and(|b| b == initial)
            && fs::remove_file(path).is_ok()
        {
            deleted = true;
            if marker["dir_created"] == json!(true)
                && let Some(dir) = path.parent()
            {
                // Only an empty directory is removed.
                let _ = fs::remove_dir(dir);
            }
        }
        let _ = fs::remove_file(&marker_path);
    }
    deleted
}

trait PrivateMode {
    fn mode_private(&mut self) -> &mut Self;
}
impl PrivateMode for fs::OpenOptions {
    fn mode_private(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}

/// The state directory, made ready to hold setup manifests.
fn prepare_state(env: &SetupEnv) -> Result<&Path, RunError> {
    let state = env.state_dir()?;
    // A detected state directory follows Herdr's plugin state layout, whose
    // parents Herdr creates on the plugin's first action: create them if
    // setup runs first. An explicit one must already have its parent.
    if env.instance_source["state_dir"]
        .as_str()
        .is_some_and(|source| source.starts_with("herdr plugin list"))
        && let Some(parent) = state.parent()
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    ensure_owned_state_root(state)?;
    ensure_private_dir(&state.join("setup"))?;
    Ok(state)
}

fn settings_error(
    error: SetupError,
    verb: SetupVerb,
    kind: SettingsKind,
    settings: &Path,
    manifest: &Path,
) -> RunError {
    let settings = settings.display();
    let harness = kind.harness();
    match (error, verb) {
        (SetupError::Invalid, _) => invalid(format!(
            "{settings} is not a JSON object with an object `hooks` (and, for Claude, \
             `permissions` with an array `allow`), or it or the ownership manifest is a symlink \
             or damaged; nothing was changed"
        )),
        (SetupError::TooLarge, _) => invalid(format!(
            "{settings} or its ownership manifest exceeds the setup size bound; nothing was changed"
        )),
        (SetupError::Conflict, SetupVerb::Remove) => api(
            ErrorCode::Conflict,
            format!(
                "the owned herdr-threads hook groups in {settings} were edited or removed, or the \
                 file changed during removal; nothing was changed. Restore the groups or remove \
                 them by hand, then delete the manifest {}",
                manifest.display()
            ),
        ),
        (SetupError::Conflict, _) => api(
            ErrorCode::Conflict,
            format!(
                "refused to install into {settings}: it changed during setup, already holds an \
                 unowned identical herdr-threads hook, holds an edited owned hook{rule}, or an \
                 existing installation uses a different hook command (executable, state \
                 directory or Herdr instance). Run `herdr-threads unsetup {harness}` (or remove \
                 the unowned hook) and set up again; nothing was changed",
                rule = if kind == SettingsKind::ClaudeUser {
                    format!(
                        ", lacks the recorded permission allow rule `{}` (removed by hand)",
                        claude::HERDR_THREADS_ALLOW_RULE
                    )
                } else {
                    String::new()
                }
            ),
        ),
        (SetupError::Io, _) => failed(format!(
            "could not read or replace {settings} or its ownership manifest; if a manifest was \
             recorded, re-run the same command to resume"
        )),
    }
}

fn adopted_warning(kind: SettingsKind, file: &Path, owner: &str) -> String {
    format!(
        "{} already held the herdr-threads hook groups of another setup (owner marker {owner}, \
         e.g. a copied {}); they were adopted in place and the file was not changed{}. \
         `herdr-threads unsetup {}` here removes only these groups from this file",
        file.display(),
        kind.file_name(),
        if kind == SettingsKind::CodexUser {
            ", so Codex's recorded trust for them stays valid"
        } else {
            ""
        },
        kind.harness()
    )
}

fn adoption_json(adoption: Option<&lib::Adoption>) -> Value {
    match adoption {
        None => Value::Null,
        Some(adoption) => json!({
            "owner": adoption.owner,
            "recorded": adoption.recorded,
            "note": if adoption.recorded {
                "the owned groups carry another setup's owner marker; this file's manifest \
                 adopted them in place"
            } else {
                "the groups carry another setup's owner marker and match this command exactly \
                 (a copied hook file); `setup` adopts them without changing the file"
            },
        }),
    }
}

/// With no manifest for this file, adopt another setup's exact groups so `unsetup` can remove
/// them; `None` when nothing is adoptable (or the hook command cannot be resolved).
fn adopt_for_removal(
    kind: SettingsKind,
    env: &SetupEnv,
    file: &Path,
    manifest: &Path,
) -> Result<Option<lib::OwnershipManifest>, RunError> {
    let map = |e| settings_error(e, SetupVerb::Remove, kind, file, manifest);
    let Ok(argv) = env.hook_argv(match kind {
        SettingsKind::ClaudeUser => Harness::Claude,
        SettingsKind::CodexUser => Harness::Codex,
    }) else {
        return Ok(None);
    };
    // An unreadable or unparsable file has nothing adoptable: unsetup reports not installed.
    if !matches!(adoptable_user_settings(kind, file, &argv), Ok(Some(_))) {
        return Ok(None);
    }
    prepare_state(env)?;
    adopt_user_settings(kind, file, manifest, &argv).map_err(map)
}

/// The recorded hook command shared by every owned group: the base command, without the
/// `--event <event>` pair each group registers it with.
fn recorded_command(manifest: &Path) -> Option<String> {
    shared_command(&read_settings_manifest(manifest).ok().flatten()?.owned)
}

/// Setup output when a recorded registration without `--event` is rewritten to the per-event form.
const EVENT_DOWNGRADE_WARNING: &str = "the hook commands now carry --event; a herdr-threads build \
     from before per-event registration rejects them, so to downgrade herdr-threads first run \
     `herdr-threads unsetup <harness>` with this build";

/// Codex trusts hooks by hash, so rewritten commands need review again.
const CODEX_RETRUST_WARNING: &str = "Codex trusts hooks by hash: the rewritten hook commands need \
     review again (the next interactive `codex` start, or /hooks) before Codex runs them";

/// Install the owned hook groups into one user-level hook file.
fn install_settings(
    kind: SettingsKind,
    verb: SetupVerb,
    env: &SetupEnv,
    file: &mut OwnedFile,
    warnings: &mut Vec<String>,
) -> Result<(lib::OwnershipManifest, bool, bool), RunError> {
    let argv = env.hook_argv(match kind {
        SettingsKind::ClaudeUser => Harness::Claude,
        SettingsKind::CodexUser => Harness::Codex,
    })?;
    let (path, manifest) = (file.path.clone(), file.manifest.clone());
    let map = |error| settings_error(error, verb, kind, &path, &manifest);
    let recorded = read_settings_manifest(&file.manifest).map_err(map)?;
    let before = recorded
        .is_some()
        .then(|| {
            inspect_user_settings(kind, &file.path, &file.manifest, NativeObservation::Unknown).ok()
        })
        .flatten();
    let already = before
        .as_ref()
        .is_some_and(|inspection| inspection.installed);
    let legacy = before
        .as_ref()
        .is_some_and(|inspection| inspection.legacy_event_registration);
    if recorded.is_none() {
        file.prepare()?;
    }
    let base = match &recorded {
        Some(recorded) => recorded.original_bytes.clone(),
        None => fs::read(&file.path)
            .map_err(|error| failed(format!("could not read {}: {error}", file.path.display())))?,
    };
    match install_user_settings(kind, &file.path, &file.manifest, &argv, &base) {
        Ok(installed) => {
            file.record(warnings);
            if legacy {
                let harness = kind.harness();
                warnings.push(EVENT_DOWNGRADE_WARNING.replace("<harness>", harness));
                if kind == SettingsKind::CodexUser {
                    warnings.push(CODEX_RETRUST_WARNING.to_owned());
                }
            }
            // Adopted now: this run recorded another setup's groups in place, writing no hook.
            let adopted = recorded.is_none() && installed.adopted;
            if adopted {
                warnings.push(adopted_warning(
                    kind,
                    &file.path,
                    &installed.installation_id,
                ));
            }
            Ok((installed, already, adopted))
        }
        Err(error) => {
            file.undo();
            if error == SetupError::Conflict
                && let Some(moved) = moved_binary_conflict(kind, &path, recorded.as_ref(), &argv)
            {
                return Err(moved);
            }
            Err(map(error))
        }
    }
}

/// The first word of a hook command built by `shell_command` (single-quoted, an embedded
/// quote written `'\''`).
fn first_shell_word(command: &str) -> Option<String> {
    let mut chars = command.strip_prefix('\'')?.chars().peekable();
    let mut word = String::new();
    while let Some(c) = chars.next() {
        if c != '\'' {
            word.push(c);
        } else if chars.clone().take(3).eq("\\''".chars()) {
            chars.nth(2);
            word.push('\'');
        } else {
            return Some(word);
        }
    }
    None
}

/// A re-setup refused because the recorded hook command names another executable than this
/// one (the binary was moved, reinstalled elsewhere or run from a copy): the generic conflict
/// message lists causes such as hand removal, which misleads here. Names both paths and the fix.
fn moved_binary_conflict(
    kind: SettingsKind,
    file: &Path,
    recorded: Option<&lib::OwnershipManifest>,
    argv: &[String],
) -> Option<RunError> {
    let recorded_command = recorded?.owned.first()?.group["hooks"][0]["command"].as_str()?;
    let recorded_exe = first_shell_word(recorded_command)?;
    let current_exe = argv.first()?;
    if &recorded_exe == current_exe {
        return None;
    }
    let harness = kind.harness();
    Some(api(
        ErrorCode::Conflict,
        format!(
            "{} already holds the herdr-threads hooks of an installation that runs \
             `{recorded_exe}`, but this is `{current_exe}` (the binary moved, or this is another \
             copy). Run `herdr-threads unsetup {harness}` (from either binary: it removes the \
             recorded groups whatever executable they name), then `herdr-threads setup \
             {harness}` from the binary you want to keep; nothing was changed",
            file.display()
        ),
    ))
}

// ------------------------------------------------------------------ claude

fn claude_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(env.state_dir()?, "claude-user", settings))
}

/// The user-level Claude installation's file and manifest paths.
pub fn claude_paths(env: &SetupEnv) -> Result<(PathBuf, PathBuf), RunError> {
    let settings = env.claude_settings()?;
    let manifest = claude_manifest(env, &settings)?;
    Ok((settings, manifest))
}

/// The installed hooks are recorded, complete and run this executable's
/// command: a re-run changes nothing.
fn hooks_current(env: &SetupEnv, settings: &Path, manifest: &Path) -> bool {
    let Ok(argv) = env.hook_argv(Harness::Claude) else {
        return false;
    };
    let Ok(expected) = shell_command(&argv) else {
        return false;
    };
    inspect_user_settings_for(
        SettingsKind::ClaudeUser,
        settings,
        manifest,
        NativeObservation::Unknown,
        Some(&argv),
    )
    .is_ok_and(|inspection| inspection.installed)
        && recorded_command(manifest)
            .is_some_and(|recorded| recorded.starts_with(&format!("{expected} # ")))
}

fn claude_install(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let settings = env.claude_settings()?;
    env.hook_argv(Harness::Claude)?;
    let (observed, _) = observe(request, env).map_err(refuse_executable)?;
    prepare_state(env)?;
    let manifest = claude_manifest(env, &settings)?;
    let mut file = OwnedFile::new(settings.clone(), manifest.clone(), b"{}");
    let mut warnings = Vec::new();
    // The hook installer's byte-exact restore needs the file as it left it.
    // When it is about to rewrite the hooks, take the mod's settings entry
    // out first (the files stay) and put it back afterwards.
    let mod_lifted = if hooks_current(env, &settings, &manifest) {
        false
    } else {
        let lifted = claude_mod::lift(&settings, &mod_manifest(env, &settings)?)
            .map_err(|error| mod_error(error, &settings))?;
        lifted.is_some()
    };
    // Everything after the lift runs in one closure: any error in it puts
    // the mod's settings entry back (one restore path).
    let mut mod_done = false;
    let result = (|| -> Result<Value, RunError> {
        let (installed, already, adopted) = install_settings(
            SettingsKind::ClaudeUser,
            request.verb,
            env,
            &mut file,
            &mut warnings,
        )?;
        let allow_rule = inspect_user_settings(
            SettingsKind::ClaudeUser,
            &settings,
            &manifest,
            NativeObservation::Unknown,
        )
        .ok()
        .and_then(|inspection| inspection.allow_rule);
        let command = shared_command(&installed.owned).unwrap_or_default();
        let prompt_suggestions =
            prompt_suggestion_step(request.prompt_suggestions, env, &settings, &mut warnings)?;
        let delivery_mod = mod_install_step(request, env, &settings, &mut warnings)?;
        mod_done = true;
        Ok(json!({
            "action": if adopted { "adopted" } else if already { "already_installed" } else { "installed" },
            "adopted": installed.adopted.then(|| installed.installation_id.clone()),
            "harness": "claude",
            "scope": "user",
            "settings": settings.display().to_string(),
            "manifest": manifest.display().to_string(),
            "created_settings": file.created_file,
            "instance": instance_json(env),
            "hook_argv": env.hook_argv(Harness::Claude)?,
            "command": command,
            "events": installed.owned.iter().map(|entry| entry.event.clone()).collect::<Vec<_>>(),
            "allow_rule": allow_rule_json(allow_rule.as_ref()),
            "harness_version": observation_json(&Ok((observed, None))),
            "prompt_suggestions": prompt_suggestions,
            "mod": delivery_mod,
            "observed": "unknown",
            "note": "installed is not observed: run `herdr-threads doctor` for native evidence. \
                     Claude sessions that run with another CLAUDE_CONFIG_DIR, or with \
                     --setting-sources excluding `user`, do not load these hooks",
            "warnings": warnings,
        }))
    })();
    if result.is_err() && mod_lifted && !mod_done {
        let _ = mod_install_step(request, env, &settings, &mut Vec::new());
    }
    result
}

/// Report form of the recorded allow rule; `null` when no manifest is recorded.
pub fn allow_rule_json(allow_rule: Option<&AllowRuleInspection>) -> Value {
    match allow_rule {
        None => Value::Null,
        Some(rule) => {
            let mut report = json!({
                "rule": rule.rule,
                "ownership": match rule.ownership {
                    AllowRuleOwnership::Owned => "owned",
                    AllowRuleOwnership::PreExisting => "pre_existing",
                    AllowRuleOwnership::NotRecorded => "not_recorded",
                    AllowRuleOwnership::Superseded => "superseded",
                },
                "present": rule.present,
            });
            if rule.ownership == AllowRuleOwnership::Superseded {
                report["note"] = json!(format!(
                    "recorded with the retired rule `{}`; re-run `herdr-threads setup claude` to \
                     replace it with `{}`",
                    claude::CALLER_CONTEXT_ALLOW_RULE,
                    rule.rule
                ));
            }
            report
        }
    }
}

/// Where setup records that it set Claude's prompt-suggestion key.
pub fn prompt_suggestion_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(
        env.state_dir()?,
        "claude-prompt-suggestion",
        settings,
    ))
}

fn prompt_suggestion_error(error: SetupError, settings: &Path) -> RunError {
    let settings = settings.display();
    match error {
        SetupError::Invalid => invalid(format!(
            "{settings} is not a JSON object, or the prompt-suggestion record is a symlink or \
             damaged; prompt suggestions were not changed"
        )),
        SetupError::TooLarge => invalid(format!(
            "{settings} or the prompt-suggestion record exceeds the setup size bound; prompt \
             suggestions were not changed"
        )),
        SetupError::Conflict => api(
            ErrorCode::Conflict,
            format!(
                "{settings} changed while setup edited `{}`, or the prompt-suggestion record \
                 names another file; prompt suggestions were not changed. Re-run the command",
                claude::PROMPT_SUGGESTION_SETTING
            ),
        ),
        SetupError::Io => failed(format!(
            "could not read or replace {settings} or the prompt-suggestion record"
        )),
    }
}

/// The report form of the prompt-suggestion setting after `action`.
fn prompt_suggestion_json(state: SuggestionState, action: &str, recorded: bool) -> Value {
    json!({
        "setting": claude::PROMPT_SUGGESTION_SETTING,
        "state": state.as_str(),
        "action": action,
        "set_by_setup": recorded,
    })
}

/// `setup claude`, after the hooks: apply the policy. `advised` means the
/// setting is on and nothing was changed (an interactive run then asks).
fn prompt_suggestion_step(
    policy: PromptSuggestionPolicy,
    env: &SetupEnv,
    settings: &Path,
    warnings: &mut Vec<String>,
) -> Result<Value, RunError> {
    let manifest = prompt_suggestion_manifest(env, settings)?;
    let error = |e| prompt_suggestion_error(e, settings);
    let recorded = prompt_suggestion::recorded(&manifest)
        .map_err(error)?
        .is_some();
    let state = prompt_suggestion::read_state(settings).map_err(error)?;
    if state == SuggestionState::Disabled {
        return Ok(prompt_suggestion_json(state, "already_disabled", recorded));
    }
    Ok(match policy {
        PromptSuggestionPolicy::Disable => {
            prompt_suggestion::disable(settings, &manifest).map_err(error)?;
            prompt_suggestion_json(SuggestionState::Disabled, "disabled", true)
        }
        PromptSuggestionPolicy::Keep => prompt_suggestion_json(state, "kept", recorded),
        PromptSuggestionPolicy::Ask => {
            warnings.push(prompt_suggestion_advice(settings));
            prompt_suggestion_json(state, "advised", recorded)
        }
    })
}

/// Both stdin and stderr are terminals: setup may ask a question.
fn interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// An interactive `setup claude` whose report says `advised`: explain, ask
/// `Disable prompt suggestions? [y/N]` on `out`, read one line from `input`,
/// and set the key `false` only on yes. The advice warning is dropped either
/// way: the person has answered.
pub fn settle_prompt_suggestions<R: io::BufRead, W: Write>(
    env: &SetupEnv,
    report: &mut Value,
    input: &mut R,
    out: &mut W,
) -> Result<(), RunError> {
    if report["prompt_suggestions"]["action"] != "advised" {
        return Ok(());
    }
    let settings = env.claude_settings()?;
    writeln!(out, "{PROMPT_SUGGESTION_EXPLANATION}")?;
    write!(out, "Disable prompt suggestions? [y/N] ")?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let yes = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    let advice = prompt_suggestion_advice(&settings);
    if let Some(warnings) = report["warnings"].as_array_mut() {
        warnings.retain(|warning| warning.as_str() != Some(advice.as_str()));
    }
    let suggestions = &mut report["prompt_suggestions"];
    if yes {
        let manifest = prompt_suggestion_manifest(env, &settings)?;
        prompt_suggestion::disable(&settings, &manifest)
            .map_err(|e| prompt_suggestion_error(e, &settings))?;
        suggestions["action"] = json!("disabled");
        suggestions["state"] = json!(SuggestionState::Disabled.as_str());
        suggestions["set_by_setup"] = json!(true);
    } else {
        suggestions["action"] = json!("kept");
    }
    Ok(())
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

// ------------------------------------------------------------ delivery mod

/// Where setup records that it wrote the delivery mod's settings entry.
pub fn mod_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(env.state_dir()?, "claude-mod", settings))
}

fn mod_error(error: SetupError, settings: &Path) -> RunError {
    let settings = settings.display();
    match error {
        SetupError::Invalid => invalid(format!(
            "{settings} is not a JSON object, its `env.{}` is not a string, or the delivery mod \
             record or directory is a symlink or damaged; the delivery mod was not changed",
            claude::PLUGIN_DIRS_ENV
        )),
        SetupError::TooLarge => invalid(format!(
            "{settings} or the delivery mod record exceeds the setup size bound; the delivery \
             mod was not changed"
        )),
        SetupError::Conflict => api(
            ErrorCode::Conflict,
            format!(
                "{settings} changed while setup edited `env.{}`, or the delivery mod record \
                 names another file; the delivery mod was not changed. Re-run the command",
                claude::PLUGIN_DIRS_ENV
            ),
        ),
        SetupError::Io => failed(format!(
            "could not read or replace {settings}, the delivery mod record or the mod files"
        )),
    }
}

/// The directories of the shell's own `CLAUDE_CODE_PLUGIN_DIRS`.
fn shell_plugin_dirs() -> Vec<String> {
    std::env::var(claude::PLUGIN_DIRS_ENV)
        .map(|value| claude_mod::shell_dirs(&value))
        .unwrap_or_default()
}

/// Ask whether to carry the shell's plugin directories into the settings
/// value (a settings `env` value replaces the shell value). Yes by default:
/// declining would silently drop them from Claude sessions.
pub fn ask_include_shell_dirs<R: io::BufRead, W: Write>(
    dirs: &[String],
    input: &mut R,
    out: &mut W,
) -> io::Result<bool> {
    write!(
        out,
        "{} is set in this shell ({}). A settings `env` value replaces the shell value in \
         Claude sessions: include those directories in the value setup writes? [Y/n] ",
        claude::PLUGIN_DIRS_ENV,
        dirs.join(":")
    )?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(!matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "n" | "no"
    ))
}

fn managed_policy_json(policy: &ManagedPolicy) -> Value {
    match policy {
        ManagedPolicy::None => json!({"state": "none"}),
        ManagedPolicy::DisableSideloadFlags { source } => json!({
            "state": "disable_side_load_flags",
            "source": source.display().to_string(),
        }),
        ManagedPolicy::Unverifiable { source } => json!({
            "state": "unverifiable",
            "source": source.display().to_string(),
        }),
    }
}

/// The install warning / setup-status advice for a policy that blocks the
/// write, `None` when it does not.
fn managed_policy_advice(policy: &ManagedPolicy, written: bool) -> Option<String> {
    match policy {
        ManagedPolicy::None => None,
        ManagedPolicy::DisableSideloadFlags { source } => Some(managed_policy_warning(
            &source.display().to_string(),
            written,
        )),
        ManagedPolicy::Unverifiable { source } => {
            let mut text = format!(
                "managed settings ({}) could not be read or parsed; setup cannot rule out {}, \
                 so the delivery mod was not installed (hooks only; native wake is the fallback)",
                source.display(),
                claude::DISABLE_SIDELOAD_FLAGS_KEY
            );
            if written {
                text.push_str(
                    "; to remove the path setup wrote earlier, run \
                     `herdr-threads setup claude --hooks-only`",
                );
            }
            Some(text)
        }
    }
}

fn managed_policy_warning(source: &str, written: bool) -> String {
    let tail = if written {
        "Claude Code will refuse to start with the path setup wrote earlier: run \
         `herdr-threads setup claude --hooks-only` to remove it"
    } else {
        "the delivery mod was not installed (hooks only; native wake is the fallback)"
    };
    format!(
        "managed settings ({source}) set {}: Claude Code refuses {}; {tail}",
        claude::DISABLE_SIDELOAD_FLAGS_KEY,
        claude::PLUGIN_DIRS_ENV
    )
}

/// `setup claude`, after the hooks and the prompt suggestion: write the
/// delivery mod and its settings entry (or, with `--hooks-only`, remove
/// them).
fn mod_install_step(
    request: &SetupRequest,
    env: &SetupEnv,
    settings: &Path,
    warnings: &mut Vec<String>,
) -> Result<Value, RunError> {
    let state = env.state_dir()?;
    let manifest = mod_manifest(env, settings)?;
    let error = |e| mod_error(e, settings);
    let dir = claude_mod::mod_dir(state).display().to_string();
    if request.hooks_only {
        let outcome = claude_mod::revert(settings, &manifest, state).map_err(error)?;
        return Ok(json!({
            "action": "removed_hooks_only",
            "settings_entry": outcome.as_str(),
            "dir": dir,
            "note": "hooks only: the delivery mod is not installed; native wake is the fallback",
        }));
    }
    let shell = shell_plugin_dirs();
    let accepted = if shell.is_empty() || !interactive() {
        // Non-interactive: include them, so they are not silently lost.
        shell.clone()
    } else if ask_include_shell_dirs(&shell, &mut io::stdin().lock(), &mut io::stderr())? {
        shell.clone()
    } else {
        Vec::new()
    };
    let sources = ManagedPolicySources::platform(env.claude_config_dir.as_deref());
    let outcome = claude_mod::install(&claude_mod::InstallInput {
        settings,
        manifest: &manifest,
        state_dir: state,
        shell_dirs: &accepted,
        policy: &sources,
    })
    .map_err(error)?;
    if let Some(advice) = managed_policy_advice(&outcome.policy, outcome.recorded) {
        warnings.push(advice);
    }
    let shell_report = if shell.is_empty() {
        Value::Null
    } else {
        let value = std::env::var(claude::PLUGIN_DIRS_ENV).unwrap_or_default();
        let settings_value = claude_mod::settings_value(settings).unwrap_or_default();
        let kept = claude_mod::shell_dirs(&settings_value);
        let lost: Vec<&String> = shell.iter().filter(|dir| !kept.contains(dir)).collect();
        if outcome.action != claude_mod::ModAction::SkippedManagedPolicy {
            warnings.push(format!(
                "{} is set in this shell ({value}); a settings `env` value replaces it in \
                 Claude sessions. {}",
                claude::PLUGIN_DIRS_ENV,
                if lost.is_empty() {
                    "Its directories are in the value setup wrote"
                } else {
                    "Its directories were not added to the settings value"
                }
            ));
        }
        json!({
            "value": value,
            "copied": outcome.copied_from_shell,
            "all_in_settings_value": lost.is_empty(),
        })
    };
    Ok(json!({
        "action": outcome.action.as_str(),
        "dir": dir,
        "files_written": outcome.files_written,
        "settings_value": claude_mod::settings_value(settings),
        "managed_policy": managed_policy_json(&outcome.policy),
        "shell_env": shell_report,
        "note": "an install does not prove any session loaded the mod",
    }))
}

/// `setup-status`'s `mod` object (spec D8): install state, managed policy,
/// shell value, version gate and the daemon's channel status.
fn mod_status(request: &SetupRequest, env: &SetupEnv, settings: &Path) -> Value {
    const NOTE: &str = "an install does not prove any session loaded the mod";
    let Ok(state) = env.state_dir() else {
        return json!({"installed": false, "error": "state directory unknown", "note": NOTE});
    };
    let Ok(manifest) = mod_manifest(env, settings) else {
        return json!({"installed": false, "error": "state directory unknown", "note": NOTE});
    };
    let inspection = claude_mod::inspect(settings, &manifest, state);
    let policy = ManagedPolicySources::platform(env.claude_config_dir.as_deref()).check();
    let (version, supported, reason) = claude_version_gate(request, env);
    let mut report = json!({
        "installed": inspection.installed(),
        "dir": claude_mod::mod_dir(state).display().to_string(),
        "files_current": inspection.files_current,
        "files_present": inspection.files_present,
        "recorded": inspection.recorded,
        "settings_value_contains_mod_dir": inspection.settings_value_contains_mod_dir,
        "managed_policy": managed_policy_json(&policy),
        "shell_env": std::env::var(claude::PLUGIN_DIRS_ENV).ok(),
        "claude_version": version,
        "claude_version_supported": supported,
        "daemon": mod_daemon_status(env),
        "note": NOTE,
    });
    if let Some(reason) = reason {
        report["claude_version_reason"] = json!(reason);
    }
    if supported == Some(false) {
        report["claude_version_note"] = json!("mod unsupported, native wake fallback");
    }
    if let Ok(setting) = std::env::var(crate::protocol::watch::MOD_DELIVERY_ENV)
        && setting.trim().eq_ignore_ascii_case("off")
    {
        // This process's own environment, not the daemon-side switch.
        report["session_override"] = json!("off");
    }
    if let Some(advice) = managed_policy_advice(
        &policy,
        inspection.settings_value_contains_mod_dir || inspection.recorded,
    ) {
        report["advice"] = json!(advice);
    }
    report
}

/// Bound on the `claude --version` run behind `setup-status`.
const CLAUDE_VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The installed Claude's `major.minor.patch`, whether it meets the mod's
/// minimum and, when that is unknown, why. Runs the resolved binary's
/// `--version` under a bound; `setup claude` (install) never does.
fn claude_version_gate(
    request: &SetupRequest,
    env: &SetupEnv,
) -> (Option<String>, Option<bool>, Option<&'static str>) {
    let Ok((observed, _)) = observe(request, env) else {
        return (None, None, Some("no claude executable found"));
    };
    let Ok(stdout) = crate::harness::codex::version_output_cancellable(
        &observed.binary,
        CLAUDE_VERSION_TIMEOUT,
        &crate::protocol::time::Cancellation::default(),
    ) else {
        return (None, None, Some("claude --version failed or timed out"));
    };
    let text = String::from_utf8_lossy(&stdout);
    let first = text.lines().next().unwrap_or_default().trim();
    let version = first
        .split_whitespace()
        .next()
        .filter(|token| claude_mod::version_supported(token).is_some());
    match version {
        Some(version) => (
            Some(version.to_owned()),
            claude_mod::version_supported(version),
            None,
        ),
        None => (None, None, Some("unrecognised claude --version output")),
    }
}

const CHANNEL_STATUS_UNAVAILABLE: &str = "channel status unavailable";

/// The daemon's mod channel status, read only from a daemon that is already
/// running (never started here).
fn mod_daemon_status(env: &SetupEnv) -> Value {
    let unavailable = |why: &str| json!({"status": CHANNEL_STATUS_UNAVAILABLE, "reason": why});
    let (Ok(state), Ok(host)) = (env.state_dir(), env.host_endpoint()) else {
        return unavailable("the Herdr instance could not be resolved");
    };
    let Ok(context) = RuntimeContext::explicit(state.to_path_buf(), host, None) else {
        return unavailable("the Herdr instance could not be resolved");
    };
    let Ok(paths) = crate::daemon::paths::InstancePaths::resolve_read_only(&context) else {
        return unavailable("the daemon's paths could not be resolved");
    };
    match super::doctor::probe_daemon(&paths) {
        Ok(super::doctor::Daemon::Reachable(_, Ok(report))) => match report.mod_channels {
            Some(status) => json!({
                "status": "reachable",
                "mod_delivery": status.mod_delivery,
                "live_channels": status.live_channels,
                "channels": status.channels.iter().map(|channel| json!({
                    "seat": channel.seat,
                    "harness": channel.harness,
                    "state": channel.state,
                })).collect::<Vec<_>>(),
            }),
            None => unavailable("the daemon keeps no channel registry"),
        },
        Ok(super::doctor::Daemon::Reachable(_, Err(why))) => unavailable(&why),
        Ok(super::doctor::Daemon::NotRunning) => unavailable("no daemon is running"),
        Ok(super::doctor::Daemon::Unreachable(why)) => unavailable(&why),
        Err(why) => unavailable(&why),
    }
}

/// One `mod:` summary line for the per-harness text report.
fn mod_line(value: &Value) -> Option<String> {
    if let Some(outcome) = value.as_str() {
        return Some(format!("mod: {outcome}"));
    }
    let object = value.as_object()?;
    if let Some(action) = object.get("action").and_then(Value::as_str) {
        let mut line = format!("mod: {action}");
        if let Some(source) = value["managed_policy"]["source"].as_str() {
            line.push_str(&format!(" (managed policy {})", scalar(&json!(source))));
        }
        return Some(line);
    }
    let installed = if value["installed"] == true {
        "installed"
    } else {
        "not installed"
    };
    let mut line = format!("mod: {installed}");
    if let Some(source) = value["managed_policy"]["source"].as_str() {
        line.push_str(&format!(
            "; managed policy {} blocks it",
            scalar(&json!(source))
        ));
    }
    match value["claude_version_supported"].as_bool() {
        Some(true) => line.push_str("; claude >= 2.1.287"),
        Some(false) => line.push_str("; mod unsupported, native wake fallback"),
        None => line.push_str("; claude version not observed"),
    }
    line.push_str(&format!(
        "; daemon: {}",
        scalar(&json!(
            value["daemon"]["status"]
                .as_str()
                .map(|status| if status == "reachable" {
                    format!(
                        "mod_delivery {}, {} live channel(s)",
                        scalar(&value["daemon"]["mod_delivery"]),
                        scalar(&value["daemon"]["live_channels"])
                    )
                } else {
                    status.to_owned()
                })
                .unwrap_or_default()
        ))
    ));
    if value["session_override"] == "off" {
        line.push_str("; session override HERDR_THREADS_MOD_DELIVERY=off");
    }
    line.push_str("; an install does not prove any session loaded the mod");
    Some(line)
}

fn instance_json(env: &SetupEnv) -> Value {
    json!({
        "state_dir": env.state_dir.as_ref().map(|p| p.display().to_string()),
        "host_endpoint": env.host_endpoint().ok().map(|p| p.display().to_string()),
        "source": env.instance_source,
    })
}

fn claude_remove(env: &SetupEnv) -> Result<Value, RunError> {
    let (settings, manifest) = claude_paths(env)?;
    let kind = SettingsKind::ClaudeUser;
    // The mod edit first, then the prompt suggestion, then the hooks: each
    // later step can still restore the file byte for byte (install order is
    // the reverse).
    let mod_reverted =
        claude_mod::revert(&settings, &mod_manifest(env, &settings)?, env.state_dir()?)
            .map_err(|error| mod_error(error, &settings))?;
    let reverted =
        prompt_suggestion::revert(&settings, &prompt_suggestion_manifest(env, &settings)?)
            .map_err(|error| prompt_suggestion_error(error, &settings))?;
    let mut report = json!({
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "manifest": manifest.display().to_string(),
        "prompt_suggestions": reverted.as_str(),
        "mod": mod_reverted.as_str(),
    });
    let mut recorded = read_settings_manifest(&manifest)
        .map_err(|e| settings_error(e, SetupVerb::Remove, kind, &settings, &manifest))?;
    if recorded.is_none() {
        recorded = adopt_for_removal(kind, env, &settings, &manifest)?;
    }
    let Some(recorded) = recorded else {
        report["action"] = json!("not_installed");
        report["note"] = json!(
            "no herdr-threads installation is recorded for these settings and this state \
             directory; nothing changed"
        );
        return Ok(report);
    };
    let allow_rule = match recorded.permission.as_ref() {
        Some(permission) if permission.pre_existing => "left_pre_existing",
        Some(_) => "removed",
        None => "not_recorded",
    };
    remove_user_settings(kind, &settings, &manifest)
        .map_err(|e| settings_error(e, SetupVerb::Remove, kind, &settings, &manifest))?;
    report["action"] = json!("removed");
    report["allow_rule"] = json!(allow_rule);
    if recorded.adopted {
        report["adopted"] = json!(recorded.installation_id);
        report["note"] = json!(
            "removed only the adopted hook groups (another setup's owner marker) from this \
             settings file; that setup's own settings file is unchanged"
        );
    }
    report["deleted_created_settings"] = json!(delete_created(&settings, &manifest, b"{}"));
    Ok(report)
}

/// Shared status of one user-level hook file installation.
fn settings_status(
    kind: SettingsKind,
    env: &SetupEnv,
    settings: &Path,
    report: &mut Value,
) -> Result<(), RunError> {
    let Ok(state) = env.state_dir() else {
        report["installed"] = json!(false);
        report["error"] = json!(format!(
            "state directory unknown: {}",
            env.instance_source["state_dir_error"]
                .as_str()
                .unwrap_or("use --state-dir or HERDR_PLUGIN_STATE_DIR")
        ));
        return Ok(());
    };
    let manifest = manifest_path(state, &format!("{}-user", kind.harness()), settings);
    report["manifest"] = json!(manifest.display().to_string());
    let argv = env
        .hook_argv(match kind {
            SettingsKind::ClaudeUser => Harness::Claude,
            SettingsKind::CodexUser => Harness::Codex,
        })
        .ok();
    let inspection = inspect_user_settings_for(
        kind,
        settings,
        &manifest,
        NativeObservation::Unknown,
        argv.as_deref(),
    )
    .map_err(|e| settings_error(e, SetupVerb::Status, kind, settings, &manifest))?;
    report["installed"] = json!(inspection.installed);
    report["adopted"] = adoption_json(inspection.adopted.as_ref());
    if kind == SettingsKind::ClaudeUser {
        report["allow_rule"] = allow_rule_json(inspection.allow_rule.as_ref());
    }
    report["observed"] = json!("unknown");
    if let Some(recorded) = read_settings_manifest(&manifest).ok().flatten() {
        report["recorded_phase"] = json!(match recorded.phase {
            lib::InstallPhase::Prepared => "prepared",
            lib::InstallPhase::Installed => "installed",
        });
    }
    if let Some(command) = recorded_command(&manifest) {
        report["command"] = json!(command);
    } else if let Some(adoption) = &inspection.adopted
        && let Some(argv) = &argv
        && let Ok(command) = shell_command(argv)
    {
        report["command"] = json!(format!(
            "{command} # herdr-threads-owner:{}",
            adoption.owner
        ));
    }
    if let Ok(argv) = env.hook_argv(match kind {
        SettingsKind::ClaudeUser => Harness::Claude,
        SettingsKind::CodexUser => Harness::Codex,
    }) && let Ok(expected) = shell_command(&argv)
    {
        report["current_command_matches"] = json!(
            report["command"]
                .as_str()
                .is_some_and(|recorded| recorded.starts_with(&format!("{expected} # ")))
        );
    }
    Ok(())
}

fn claude_status(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let settings = env.claude_settings()?;
    let mut report = json!({
        "action": "status",
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "instance": instance_json(env),
        "recipes": recipe::describe(claude::RECIPES),
        "harness_version": observation_json(&observe(request, env)),
    });
    settings_status(SettingsKind::ClaudeUser, env, &settings, &mut report)?;
    report["prompt_suggestions"] = prompt_suggestion_status(
        env,
        &settings,
        std::env::var_os(claude::PROMPT_SUGGESTION_ENV),
    );
    report["mod"] = mod_status(request, env, &settings);
    Ok(report)
}

/// Claude's prompt-suggestion setting as `setup-status` and `doctor` report
/// it: the user settings value, whether setup set it, and the per-session
/// environment override when one is set. Best effort: an unreadable file is
/// reported, never an error.
pub fn prompt_suggestion_status(
    env: &SetupEnv,
    settings: &Path,
    env_override: Option<OsString>,
) -> Value {
    let state = prompt_suggestion::read_state(settings);
    let recorded = prompt_suggestion_manifest(env, settings)
        .ok()
        .and_then(|manifest| prompt_suggestion::recorded(&manifest).ok().flatten())
        .is_some();
    let mut report = json!({
        "setting": claude::PROMPT_SUGGESTION_SETTING,
        "state": match &state {
            Ok(state) => state.as_str(),
            Err(_) => "unreadable",
        },
        "set_by_setup": recorded,
    });
    if let Some(value) = env_override {
        report["env_override"] = json!(format!(
            "{}={}",
            claude::PROMPT_SUGGESTION_ENV,
            value.to_string_lossy()
        ));
    }
    if state != Ok(SuggestionState::Disabled) {
        report["note"] = json!(
            "Claude soft-deadline pokes skip a pane that shows a prompt suggestion; run \
             `herdr-threads setup claude --disable-prompt-suggestions` to turn them off"
        );
    }
    report
}

// ------------------------------------------------------------------- codex

/// Codex's config home: a non-empty `CODEX_HOME`, else `$HOME/.codex`.
pub fn codex_home_from(
    codex_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    match codex_home.filter(|value| !value.is_empty()) {
        Some(value) => Some(PathBuf::from(value)),
        None => home
            .filter(|value| !value.is_empty())
            .map(|home| PathBuf::from(home).join(".codex")),
    }
}

/// Read bound per Codex config file; a larger file is reported, not parsed.
const CODEX_CONFIG_LIMIT: u64 = 1 << 20;

/// What one Codex config layer file already declares, observed read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexLayerFile {
    pub path: PathBuf,
    /// Owned events this file declares hooks for (hooks.json parsed; TOML by
    /// a conservative key scan).
    pub events: Vec<&'static str>,
    /// The exact herdr-threads hook command setup would pass is already here.
    pub has_owned_command: bool,
    /// Some herdr-threads Codex hook command (another executable or state
    /// directory) is here.
    pub has_other_herdr_threads_hook: bool,
    /// Present but not read (too large or unreadable).
    pub unreadable: Option<String>,
}

/// The Codex config files whose hooks Codex discovers as separate layers: `$CODEX_HOME/{config.toml,hooks.json}`,
/// the system `/etc/codex` pair, and each `.codex/` pair from `cwd` up to its
/// Git root, or only `cwd`'s when no ancestor holds a `.git` (Codex has no project
/// root then, so it loads no `.codex/` above the working directory; Codex loads
/// project layers only for trusted projects; they are listed regardless, since
/// trust is not ours to read).
pub fn codex_layer_paths(codex_home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = codex_home {
        dirs.push(home.to_path_buf());
    }
    dirs.push(PathBuf::from("/etc/codex"));
    let project_depth = cwd
        .ancestors()
        .position(|dir| dir.join(".git").exists())
        .unwrap_or(0);
    for dir in cwd.ancestors().take(project_depth + 1) {
        dirs.push(dir.join(".codex"));
    }
    let mut paths = Vec::new();
    for dir in dirs {
        for name in ["config.toml", "hooks.json"] {
            let path = dir.join(name);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn owned_events() -> impl Iterator<Item = &'static str> {
    codex::DECLARATION.owned_hooks.iter().map(|hook| hook.event)
}

/// Owned events a `config.toml` declares hooks for, by a conservative scan of
/// table headers and dotted keys (`[[hooks.E]]`, `[hooks.E]`, `hooks.E =`, and
/// `E =` inside `[hooks]`). Informational: Codex keeps these hooks either way.
fn toml_hook_events(text: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    let mut in_hooks_table = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let compact: String = line
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        if compact.starts_with('[') {
            in_hooks_table = compact == "[hooks]";
        }
        for event in owned_events() {
            let header = compact == format!("[[hooks.{event}]]")
                || compact.starts_with(&format!("[hooks.{event}]"))
                || compact.starts_with(&format!("[hooks.{event}."));
            let dotted = compact.starts_with(&format!("hooks.{event}="))
                || (in_hooks_table && compact.starts_with(&format!("{event}=")));
            if (header || dotted) && !found.contains(&event) {
                found.push(event);
            }
        }
    }
    found
}

fn json_hook_events(bytes: &[u8]) -> Vec<&'static str> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    owned_events()
        .filter(|event| {
            value["hooks"][*event]
                .as_array()
                .is_some_and(|groups| !groups.is_empty())
        })
        .collect()
}

/// Observe one Codex config file read-only; `None` when it does not exist.
pub fn observe_codex_layer(path: &Path, owned_command: &str) -> Option<CodexLayerFile> {
    let meta = fs::metadata(path).ok()?;
    let mut layer = CodexLayerFile {
        path: path.to_path_buf(),
        events: Vec::new(),
        has_owned_command: false,
        has_other_herdr_threads_hook: false,
        unreadable: None,
    };
    if !meta.is_file() || meta.len() > CODEX_CONFIG_LIMIT {
        layer.unreadable = Some("not a regular file within 1 MiB".into());
        return Some(layer);
    }
    let mut bytes = Vec::new();
    if let Err(error) =
        fs::File::open(path).and_then(|file| file.take(CODEX_CONFIG_LIMIT).read_to_end(&mut bytes))
    {
        layer.unreadable = Some(error.to_string());
        return Some(layer);
    }
    let text = String::from_utf8_lossy(&bytes);
    layer.events = if path.extension().is_some_and(|ext| ext == "json") {
        json_hook_events(&bytes)
    } else {
        toml_hook_events(&text)
    };
    // TOML basic and JSON strings escape `"` and `\` the same way; the shell
    // form itself uses only single quotes.
    let escaped = serde_json::to_string(owned_command).unwrap_or_default();
    let escaped = escaped.trim_matches('"');
    layer.has_owned_command = text.contains(owned_command) || text.contains(escaped);
    layer.has_other_herdr_threads_hook = !layer.has_owned_command
        && text.contains("herdr-threads")
        && text.contains("'hook' 'codex'");
    Some(layer)
}

/// Every existing Codex config layer file, observed read-only.
pub(crate) fn observe_codex_config(env: &SetupEnv, owned_command: &str) -> Vec<CodexLayerFile> {
    codex_layer_paths(env.codex_home.as_deref(), &env.cwd)
        .iter()
        .filter_map(|path| observe_codex_layer(path, owned_command))
        .collect()
}

fn codex_config_json(env: &SetupEnv, layers: &[CodexLayerFile]) -> Value {
    json!({
        "codex_home": env.codex_home.as_ref().map(|home| home.display().to_string()),
        "layers": layers.iter().map(|layer| json!({
            "path": layer.path.display().to_string(),
            "owned_events_with_hooks": layer.events,
            "herdr_threads_hook": if layer.has_owned_command {
                "identical"
            } else if layer.has_other_herdr_threads_hook {
                "different"
            } else {
                "none"
            },
            "unreadable": layer.unreadable,
        })).collect::<Vec<_>>(),
    })
}

/// Another config layer (not the owned `hooks.json`, whose unowned copies
/// the installer itself refuses) that already runs the exact hook command
/// would run it a second time beside the owned one: refuse instead.
pub(crate) fn refuse_duplicate(
    layers: &[CodexLayerFile],
    owned_command: &str,
    managed: &Path,
) -> Result<(), RunError> {
    match layers
        .iter()
        .find(|layer| layer.has_owned_command && layer.path != managed)
    {
        None => Ok(()),
        Some(layer) => Err(api(
            ErrorCode::Conflict,
            format!(
                "{} already configures the herdr-threads hook command `{owned_command}`. Codex \
                 loads hooks from every config layer, so the owned user-level hook would run it \
                 a second time. Remove it from that file, then set up again; nothing was changed",
                layer.path.display()
            ),
        )),
    }
}

pub(crate) fn codex_owned_command(env: &SetupEnv) -> Result<String, RunError> {
    shell_command(&env.hook_argv(Harness::Codex)?)
        .map_err(|_| invalid("the hook command cannot be quoted for Codex"))
}

/// Historical allowance explanation for inspection only; no new allowance is installed.
pub fn codex_sandbox_note() -> String {
    "A legacy owned config.toml allowance may enable sandbox networking, a daemon socket and client journal roots. Setup installs none of these values. Run commands through approved outside-sandbox execution; unsetup removes only unchanged owned values.".to_owned()
}

/// Codex versions on which the allowance's default-deny was measured: the
/// proxy started, only the allowlisted socket connected, and other Unix
/// sockets, loopback, external TCP and proxied HTTPS stayed denied (Codex
/// demo 2 and the ht-910 `codex sandbox` A/B/C/D check, both 0.159.2; the
/// ht-4is.8.15 no-model `codex sandbox` probe on 0.159.3, evidence in
/// docs/evidence/codex-1593-sandbox-probe/). The
/// dangerous half of the allowance is `network_access=true`: on a build that
/// ignored or did not enforce `features.network_proxy`, it would leave
/// workspace-write with unrestricted networking. Historical setup admitted
/// only measured versions; current setup installs no allowance.
pub const CODEX_SANDBOX_MEASURED_VERSIONS: &[&str] = codex::SANDBOX_MEASURED_VERSIONS;

/// The loud warning for a recorded sandbox allowance that outlived the
/// version gate: `network_access=true` stays in config.toml and applies to
/// every Codex session of this CODEX_HOME, so after an upgrade to (or a PATH
/// change toward) a Codex whose proxy default-deny was never measured, the
/// risk the gate exists for is live. `None` when no allowance is recorded or
/// the observed version is measured. `version` is `None` when no admitted
/// Codex version was observed.
pub(crate) fn codex_unmeasured_allowance_warning(
    env: &SetupEnv,
    version: Option<&str>,
) -> Option<String> {
    if version.is_some_and(|version| CODEX_SANDBOX_MEASURED_VERSIONS.contains(&version)) {
        return None;
    }
    let paths = codex_paths(env).ok()?;
    // An unreadable manifest is treated as recorded: the warning must not
    // go quiet because the ownership record is damaged.
    if let Ok(None) = codex_config::read_manifest(&paths.config_manifest) {
        return None;
    }
    let observed = match version {
        Some(version) => format!("Codex {version}"),
        None => "a Codex runtime whose metadata is unavailable".to_owned(),
    };
    Some(format!(
        "WARNING: sandbox_workspace_write.network_access=true is installed in {} for an \
         unmeasured Codex version ({observed}; default-deny measured on: {}). It applies to \
         every Codex session using this CODEX_HOME and may leave the workspace-write sandbox \
         with unrestricted networking. Run `herdr-threads unsetup codex` to remove it (then \
         `herdr-threads setup codex` reinstalls the hooks without it)",
        paths.config.display(),
        CODEX_SANDBOX_MEASURED_VERSIONS.join(", ")
    ))
}

/// The client-side directories a sandboxed herdr-threads command writes, which the allowance
/// adds to `sandbox_workspace_write.writable_roots`: `<instance>/intents` (the intent journal:
/// every mutation records its pending operation there before calling the daemon, for
/// recovery and `retry`) and `<instance>/contexts` (each seat's caller-context journal,
/// written by check-in). Nothing else: not the instance directory, the SQLite database, the
/// endpoint descriptor, the owner lock or the setup manifests. The paths are canonical (the
/// longest existing prefix resolved, for example `/tmp` to `/private/tmp`), matching the
/// paths the CLI opens. Computed only: nothing is created.
pub(crate) fn codex_sandbox_roots(env: &SetupEnv) -> Result<Vec<String>, String> {
    let state = env.state_dir().map_err(|error| error.to_string())?;
    let host = env.host_endpoint().map_err(|error| error.to_string())?;
    let context = RuntimeContext::explicit(state.to_path_buf(), host, None)
        .map_err(|error| format!("invalid host endpoint: {error}"))?;
    let instance = canonical_prefix(&instance_dir(&context));
    CODEX_WRITABLE_DIRS
        .iter()
        .map(|name| {
            instance
                .join(name)
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "the instance directory path is not UTF-8".to_owned())
        })
        .collect()
}

/// The instance subdirectories a sandboxed CLI writes (see [`codex_sandbox_roots`]).
pub const CODEX_WRITABLE_DIRS: [&str; 2] = ["intents", "contexts"];

/// `path` with its longest existing ancestor canonicalized and the rest appended.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The warning for a recorded allowance that lacks this instance's writable roots (an
/// installation from before they were part of it): sandboxed mutations then fail with EPERM.
pub(crate) fn codex_missing_roots_warning(env: &SetupEnv, version: Option<&str>) -> Option<String> {
    if !version.is_some_and(|version| CODEX_SANDBOX_MEASURED_VERSIONS.contains(&version)) {
        return None;
    }
    let paths = codex_paths(env).ok()?;
    let inspection = codex_config::inspect(&paths.config, &paths.config_manifest).ok()?;
    inspection.recorded.as_ref()?;
    let roots = codex_sandbox_roots(env).ok()?;
    if inspection.roots_present(&roots) {
        return None;
    }
    Some(format!(
        "the Codex sandbox allowance in {} does not list this instance's writable roots ({}) in sandbox_workspace_write.writable_roots: under the workspace-write sandbox herdr-threads send, ack, accept, leave, invite and check-in fail with `Operation not permitted`. Run commands through approved outside-sandbox execution; `herdr-threads unsetup codex` removes the legacy allowance",
        paths.config.display(),
        roots.join(", ")
    ))
}

/// One warning per `features.network_proxy` key the user's config.toml already held when the
/// allowance was installed (recorded in the manifest): the allowance turns
/// `sandbox_workspace_write.network_access` on, which makes those settings effective.
pub(crate) fn codex_foreign_proxy_warnings(env: &SetupEnv) -> Vec<String> {
    let Ok(paths) = codex_paths(env) else {
        return Vec::new();
    };
    let Ok(Some(manifest)) = codex_config::read_manifest(&paths.config_manifest) else {
        return Vec::new();
    };
    manifest
        .foreign_network_proxy
        .iter()
        .map(|key| {
            format!(
                "{key} was already set in {}; enabling network access for the sandbox makes it \
                 effective",
                paths.config.display()
            )
        })
        .collect()
}

/// The user-level Codex installation's hook file, config file and manifests.
pub struct CodexPaths {
    pub hooks: PathBuf,
    pub hooks_manifest: PathBuf,
    pub config: PathBuf,
    pub config_manifest: PathBuf,
}

pub fn codex_paths(env: &SetupEnv) -> Result<CodexPaths, RunError> {
    let hooks = env.codex_file("hooks.json")?;
    let config = env.codex_file("config.toml")?;
    let state = env.state_dir()?;
    Ok(CodexPaths {
        hooks_manifest: manifest_path(state, "codex-user", &hooks),
        config_manifest: manifest_path(state, "codex-config", &config),
        hooks,
        config,
    })
}

/// Codex's trust-key spelling of a hook event (`hooks.state` keys and
/// `hooks/list`): `SessionStart` -> `session_start`.
fn snake(event: &str) -> String {
    let mut out = String::new();
    for (index, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub const CODEX_TRUST_NOTE: &str = "Codex runs hooks from hooks.json only once you trust them. \
     The next interactive `codex` start lists the new hooks as needing review (or open /hooks); \
     trusting them makes Codex record each hook's hash in config.toml [hooks.state] under the \
     key `<hooks.json>:<event>:<group>:<hook>`. That is how Herdr's own hooks.json entry is \
     trusted: Herdr writes the hook, Codex's review records the trust. setup never writes trust. \
     `codex exec` cannot review: trust once interactively, or (scratch only) pass \
     --dangerously-bypass-hook-trust. Editing or moving a trusted group asks for review again";

/// The trust keys of the owned groups as they sit in `hooks.json` now, and
/// whether `config.toml` records a trust hash for each (the hash itself is
/// Codex's and is not verified here).
fn codex_trust_json(paths: &CodexPaths, command: Option<&str>) -> Value {
    let Some(command) = command else {
        return json!({"status": "unknown", "note": CODEX_TRUST_NOTE});
    };
    let hooks: Value = fs::read(&paths.hooks)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    let config = fs::read_to_string(&paths.config);
    let state = config
        .as_ref()
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok());
    let path = paths.hooks.display().to_string();
    let mut keys = Vec::new();
    for hook in codex::DECLARATION.owned_hooks {
        let Some(groups) = hooks["hooks"][hook.event].as_array() else {
            continue;
        };
        for (index, group) in groups.iter().enumerate() {
            let registered = group["hooks"][0]["command"].as_str();
            if registered == Some(command)
                || registered == Some(lib::event_command(command, hook.event).as_str())
            {
                let key = format!("{path}:{}:{index}:0", snake(hook.event));
                let recorded = state.as_ref().is_some_and(|doc| {
                    doc.get("hooks")
                        .and_then(|h| h.get("state"))
                        .and_then(|s| s.get(&key))
                        .and_then(|e| e.get("trusted_hash"))
                        .is_some_and(|hash| hash.is_str())
                });
                keys.push(json!({"key": key, "trust_recorded": recorded}));
            }
        }
    }
    let status = if keys.is_empty()
        || (config.is_ok() && state.is_none())
        || config
            .as_ref()
            .is_err_and(|error| error.kind() != io::ErrorKind::NotFound)
    {
        "unknown"
    } else if keys.iter().all(|key| key["trust_recorded"] == true) {
        // Codex owns the hashes and may change its matching rules. A record is
        // evidence of a prior review, not proof the present hook will run.
        "recorded_unverified"
    } else {
        "review_required"
    };
    json!({"hooks": keys, "status": status, "note": CODEX_TRUST_NOTE})
}

/// Read Codex's own recorded hook-review keys without claiming hash validity.
pub(crate) fn codex_trust_report(env: &SetupEnv) -> Value {
    let Ok(paths) = codex_paths(env) else {
        return json!({"status": "unknown", "note": CODEX_TRUST_NOTE});
    };
    let mut status = json!({});
    if settings_status(SettingsKind::CodexUser, env, &paths.hooks, &mut status).is_err()
        || status["installed"] != true
    {
        return json!({"status": "unknown", "note": CODEX_TRUST_NOTE});
    }
    codex_trust_json(&paths, status["command"].as_str())
}

fn allowance_error(error: codex_config::AllowanceError, config: &Path) -> RunError {
    match error {
        codex_config::AllowanceError::Foreign(key) => api(
            ErrorCode::Conflict,
            format!(
                "{} already sets `{key}` to a value the sandbox allowance does not use; setup \
                 never overwrites it. Remove or change that key yourself (or keep it and add the \
                 socket allowance by hand), then set up again; nothing was changed",
                config.display()
            ),
        ),
        codex_config::AllowanceError::Setup(SetupError::Conflict) => api(
            ErrorCode::Conflict,
            format!(
                "the recorded sandbox allowance in {} names another daemon socket or one of its \
                 keys was edited by hand, or the file changed during setup. Run `herdr-threads \
                 unsetup codex` and set up again",
                config.display()
            ),
        ),
        codex_config::AllowanceError::Setup(SetupError::Io) => failed(format!(
            "could not read or replace {} or its ownership manifest; re-run to resume",
            config.display()
        )),
        codex_config::AllowanceError::Setup(_) => invalid(format!(
            "{} is not valid TOML within 1 MiB, or it or its ownership manifest is a symlink or \
             damaged; nothing was changed",
            config.display()
        )),
    }
}

/// Historical ownership remains inspectable; setup never adds an allowance.
fn allowance_json(inspection: Option<&codex_config::AllowanceInspection>) -> Value {
    json!({
        "socket_path": inspection.and_then(|i| i.recorded.as_ref()).map(|m| &m.socket),
        "validation": "unvalidated",
        "omitted": "setup uses approved outside-sandbox commands and installs no sandbox allowance",
        "recorded": inspection.is_some_and(|i| i.recorded.is_some()),
        "present": inspection.is_some_and(|i| i.present),
    })
}

/// Attach the unmeasured-allowance warning to a `sandbox` report.
fn with_unmeasured(mut sandbox: Value, warning: Option<String>) -> Value {
    if let Some(warning) = warning {
        sandbox["unmeasured_installed"] = json!(true);
        sandbox["warning"] = json!(warning);
    }
    sandbox
}

fn codex_install(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    env.hook_argv(Harness::Codex)?;
    let paths = codex_paths(env)?;
    let (observed, contract) = observe(request, env).map_err(refuse_executable)?;
    let contract = contract.expect("Codex has a registered contract");
    // Validate the declared composition without manufacturing a version witness.
    lib::plan_codex_for_contract(&[], &env.hook_argv(Harness::Codex)?, &contract).map_err(
        |error| {
            settings_error(
                error,
                request.verb,
                SettingsKind::CodexUser,
                &paths.hooks,
                &paths.hooks_manifest,
            )
        },
    )?;
    let owned_command = codex_owned_command(env)?;
    let layers = observe_codex_config(env, &owned_command);
    refuse_duplicate(&layers, &owned_command, &paths.hooks)?;
    // Legacy allowances are left for owned unsetup. Incomplete or edited
    // records still refuse before hooks change, preserving the installer seam.
    // A dangling symlink is still an ownership record, never absence.
    if fs::symlink_metadata(&paths.config_manifest)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(allowance_error(
            codex_config::AllowanceError::Setup(SetupError::Invalid),
            &paths.config,
        ));
    }
    let inspection =
        codex_config::inspect(&paths.config, &paths.config_manifest).map_err(|error| {
            allowance_error(codex_config::AllowanceError::Setup(error), &paths.config)
        })?;
    if let Some(manifest) = &inspection.recorded
        && (manifest.phase != lib::InstallPhase::Installed || !inspection.present)
    {
        return Err(allowance_error(
            codex_config::AllowanceError::Setup(SetupError::Conflict),
            &paths.config,
        ));
    }
    prepare_state(env)?;
    let mut warnings = Vec::new();
    let mut hooks_file = OwnedFile::new(paths.hooks.clone(), paths.hooks_manifest.clone(), b"{}");
    let (installed, already, adopted) = install_settings(
        SettingsKind::CodexUser,
        request.verb,
        env,
        &mut hooks_file,
        &mut warnings,
    )?;
    for layer in &layers {
        if layer.has_other_herdr_threads_hook && layer.path != paths.hooks {
            warnings.push(format!(
                "{} holds a different herdr-threads Codex hook command (another executable, \
                 state directory or Herdr instance); both would run",
                layer.path.display()
            ));
        }
        if let Some(reason) = &layer.unreadable {
            warnings.push(format!(
                "{} was not inspected ({reason}); its hooks still run",
                layer.path.display()
            ));
        }
    }
    warnings.extend(codex_foreign_proxy_warnings(env));
    let unmeasured = codex_unmeasured_allowance_warning(env, None);
    let command = shared_command(&installed.owned);
    let inspection = codex_config::inspect(&paths.config, &paths.config_manifest).ok();
    Ok(json!({
        "action": if adopted {
            "adopted"
        } else if already {
            "already_installed"
        } else {
            "installed"
        },
        "adopted": installed.adopted.then(|| installed.installation_id.clone()),
        "harness": "codex",
        "scope": "user",
        "hooks_file": paths.hooks.display().to_string(),
        "config_file": paths.config.display().to_string(),
        "manifest": paths.hooks_manifest.display().to_string(),
        "config_manifest": paths.config_manifest.display().to_string(),
        "created_hooks_file": hooks_file.created_file,
        "created_config_file": false,
        "instance": instance_json(env),
        "hook_argv": env.hook_argv(Harness::Codex)?,
        "command": command,
        "events": installed.owned.iter().map(|entry| entry.event.clone()).collect::<Vec<_>>(),
        "sandbox": with_unmeasured(
            allowance_json(inspection.as_ref()),
            unmeasured
        ),
        "trust": codex_trust_json(&paths, command.as_deref()),
        "harness_version": observation_json(&Ok((observed, None))),
        "codex_config": codex_config_json(env, &layers),
        "observed": "unknown",
        "note": "installed is not observed: run `herdr-threads doctor` for native evidence. \
                 Codex sessions with another CODEX_HOME, or launched with \
                 --ignore-user-config, do not load these hooks",
        "warnings": warnings,
    }))
}

fn codex_remove(env: &SetupEnv) -> Result<Value, RunError> {
    let paths = codex_paths(env)?;
    let kind = SettingsKind::CodexUser;
    let mut report = json!({
        "harness": "codex",
        "scope": "user",
        "hooks_file": paths.hooks.display().to_string(),
        "config_file": paths.config.display().to_string(),
        "manifest": paths.hooks_manifest.display().to_string(),
        "config_manifest": paths.config_manifest.display().to_string(),
    });
    let map = |e| {
        settings_error(
            e,
            SetupVerb::Remove,
            kind,
            &paths.hooks,
            &paths.hooks_manifest,
        )
    };
    let mut recorded = read_settings_manifest(&paths.hooks_manifest).map_err(map)?;
    if recorded.is_none() {
        recorded = adopt_for_removal(kind, env, &paths.hooks, &paths.hooks_manifest)?;
    }
    let allowance = codex_config::read_manifest(&paths.config_manifest).map_err(|error| {
        allowance_error(codex_config::AllowanceError::Setup(error), &paths.config)
    })?;
    if recorded.is_none() && allowance.is_none() {
        report["action"] = json!("not_installed");
        report["note"] = json!(
            "no herdr-threads installation is recorded for this CODEX_HOME and this state \
             directory; nothing changed"
        );
        return Ok(report);
    }
    let mut warnings = Vec::new();
    if let Some(recorded) = &recorded {
        // Codex keys hook trust by group position: groups after ours under
        // the same event move up one and need review again.
        if let Ok(bytes) = fs::read(&paths.hooks)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        {
            for entry in &recorded.owned {
                let command = entry.group["hooks"][0]["command"].as_str();
                if let Some(groups) = value["hooks"][&entry.event].as_array()
                    && let Some(at) = groups
                        .iter()
                        .position(|g| g["hooks"][0]["command"].as_str() == command)
                    && at + 1 < groups.len()
                {
                    warnings.push(format!(
                        "{}: {} hook group(s) after the removed one move up; Codex asks to \
                         review them again",
                        entry.event,
                        groups.len() - at - 1
                    ));
                }
            }
        }
        if recorded.adopted {
            report["adopted"] = json!(recorded.installation_id);
            warnings.push(format!(
                "removing only the adopted hook groups (owner marker {}) from {}; the setup \
                 that wrote them keeps its own hooks.json. Codex trusts hooks by position, so \
                 any group that shifts in this file needs review again",
                recorded.installation_id,
                paths.hooks.display()
            ));
        }
        remove_user_settings(kind, &paths.hooks, &paths.hooks_manifest).map_err(map)?;
        report["deleted_created_hooks_file"] =
            json!(delete_created(&paths.hooks, &paths.hooks_manifest, b"{}"));
    }
    if allowance.is_some() {
        codex_config::remove(&paths.config, &paths.config_manifest).map_err(|error| {
            allowance_error(codex_config::AllowanceError::Setup(error), &paths.config)
        })?;
        report["deleted_created_config_file"] =
            json!(delete_created(&paths.config, &paths.config_manifest, b""));
    }
    report["action"] = json!("removed");
    report["hooks_removed"] = json!(recorded.is_some());
    report["allowance_removed"] = json!(allowance.is_some());
    report["note"] = json!(
        "Codex's own [hooks.state] trust entries for the removed hooks are Codex state and \
         are left in config.toml"
    );
    report["warnings"] = json!(warnings);
    Ok(report)
}

fn codex_status(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let observation = observe(request, env);
    let mut report = json!({
        "action": "status",
        "harness": "codex",
        "scope": "user",
        "instance": instance_json(env),
        "recipes": recipe::describe(codex::RECIPES),
        "harness_version": observation_json(&observation),
    });
    let hooks = env.codex_file("hooks.json")?;
    report["hooks_file"] = json!(hooks.display().to_string());
    settings_status(SettingsKind::CodexUser, env, &hooks, &mut report)?;
    let Ok(paths) = codex_paths(env) else {
        return Ok(report);
    };
    let inspection = codex_config::inspect(&paths.config, &paths.config_manifest).ok();
    let unmeasured = codex_unmeasured_allowance_warning(env, None);
    report["config_file"] = json!(paths.config.display().to_string());
    report["sandbox"] = with_unmeasured(allowance_json(inspection.as_ref()), unmeasured.clone());
    let warnings: Vec<String> = unmeasured
        .into_iter()
        .chain(codex_foreign_proxy_warnings(env))
        .collect();
    if !warnings.is_empty() {
        report["warnings"] = json!(warnings);
    }
    report["trust"] = codex_trust_json(&paths, report["command"].as_str());
    if let Ok(owned_command) = codex_owned_command(env) {
        let layers = observe_codex_config(env, &owned_command);
        report["duplicate_hook"] = json!(
            layers
                .iter()
                .any(|layer| layer.has_owned_command && layer.path != paths.hooks)
        );
        report["codex_config"] = codex_config_json(env, &layers);
    }
    Ok(report)
}

/// The owned user-level installation for `launch` and `doctor`: the
/// configured-hook descriptor when every declared hook (and, for Claude, the
/// allow rule) is installed in the files this environment resolves.
pub(crate) fn user_inspection(
    harness: Harness,
    env: &SetupEnv,
) -> Result<(PathBuf, lib::HookInspection), String> {
    let (kind, file) = match harness {
        Harness::Claude => (
            SettingsKind::ClaudeUser,
            env.claude_settings().map_err(|e| e.to_string())?,
        ),
        Harness::Codex => (
            SettingsKind::CodexUser,
            env.codex_file("hooks.json").map_err(|e| e.to_string())?,
        ),
        Harness::Human => return Err(NO_HUMAN_SETUP.to_owned()),
    };
    let state = env.state_dir().map_err(|e| e.to_string())?;
    let manifest = manifest_path(state, &format!("{}-user", kind.harness()), &file);
    let argv = env.hook_argv(harness).ok();
    let inspection = inspect_user_settings_for(
        kind,
        &file,
        &manifest,
        NativeObservation::Unknown,
        argv.as_deref(),
    )
    .map_err(|error| {
        format!(
            "the owned {} installation in {} cannot be inspected ({error:?}); run \
                 `herdr-threads setup-status {}`",
            kind.harness(),
            file.display(),
            kind.harness()
        )
    })?;
    Ok((file, inspection))
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

    /// The shell-directories question defaults to yes (declining would drop
    /// them from Claude sessions) and only an explicit no leaves them out.
    #[test]
    fn shell_dirs_question_defaults_to_include() {
        let dirs = vec!["/s1".to_owned(), "/s2".to_owned()];
        for (answer, include) in [
            ("\n", true),
            ("y\n", true),
            ("yes\n", true),
            ("n\n", false),
            ("No\n", false),
            ("", true),
        ] {
            let mut out = Vec::new();
            let got =
                ask_include_shell_dirs(&dirs, &mut io::Cursor::new(answer), &mut out).unwrap();
            assert_eq!(got, include, "{answer:?}");
            let asked = String::from_utf8(out).unwrap();
            assert!(
                asked.contains("/s1:/s2") && asked.contains("[Y/n]"),
                "{asked}"
            );
        }
    }

    fn env(state: Option<&str>) -> SetupEnv {
        SetupEnv {
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
