//! Which Herdr instance a CLI command talks to, resolved the same way for
//! every command (setup, doctor, inbox, me init, thread, launch, ...).
//!
//! Per field, first match wins:
//! 1. the flag (`--state-dir` / `--host-endpoint`);
//! 2. the environment Herdr sets (`HERDR_PLUGIN_STATE_DIR` inside plugin
//!    actions; `HERDR_SOCKET_PATH`, which Herdr 0.9.1 also exports into every
//!    pane);
//! 3. a fast deterministic default, filesystem checks only, no subprocess:
//!    Herdr's plugin state root joined with the plugin id
//!    (`$XDG_STATE_HOME/herdr/plugins/herdr-threads`, else
//!    `~/.local/state/herdr/plugins/herdr-threads`) when that directory
//!    exists, and Herdr's default server socket
//!    (`$XDG_CONFIG_HOME/herdr/herdr.sock`, else `~/.config/herdr/herdr.sock`)
//!    when a socket exists there. With `XDG_STATE_HOME` set, a `~/.local/state`
//!    directory is a legacy location, chosen only when it holds a store (an
//!    `instances/<id>/` with the database) and the XDG directory does not; the
//!    source then says so. A named Herdr session (`HERDR_SESSION` /
//!    `HERDR_CONFIG_PATH`) gets neither fast default: its server, not the
//!    default one, decides, so both fields go through step 4;
//! 4. only when the default is missing, the slower `herdr` queries
//!    (`plugin list --json` / `status server --json`).
//!
//! A state directory from the fast default whose plugin Herdr's registry
//! (`$XDG_CONFIG_HOME/herdr/plugins.json`, a file read) does not list or lists
//! as disabled is a leftover: `setup` refuses, `doctor` warns, every other
//! command keeps working ([`leftover_state_dir`]).
//!
//! Nothing is cached: every invocation re-resolves, so a moved or restarted
//! Herdr is never answered from stale data. Two state roots that both hold a
//! store are refused as ambiguous rather than picked.

use super::{
    hook,
    setup::{DetectInputs, PLUGIN_ID, detect_host_endpoint, detect_state_dir},
};
use crate::daemon::paths::RuntimeContext;
use serde_json::{Map, Value, json};
use std::{
    ffi::OsString,
    io,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
};

/// Everything resolution reads, captured once from the process (or built
/// by tests with scratch values).
#[derive(Debug, Clone, Default)]
pub struct InstanceInputs {
    pub state_flag: Option<PathBuf>,
    pub host_flag: Option<PathBuf>,
    pub env_state: Option<PathBuf>,
    pub env_host: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    /// `HERDR_SESSION` / `HERDR_CONFIG_PATH` select a non-default Herdr
    /// server, whose socket is not the default path and which owns its own
    /// plugin registry: skip both fast defaults (socket and state directory).
    pub non_default_server: bool,
    /// The `herdr` executable on PATH, for the slow fallback only.
    pub herdr: Option<PathBuf>,
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl InstanceInputs {
    pub fn from_process(state_flag: Option<PathBuf>, host_flag: Option<PathBuf>) -> Self {
        let path = std::env::var_os("PATH");
        Self {
            state_flag,
            host_flag,
            env_state: non_empty(std::env::var_os("HERDR_PLUGIN_STATE_DIR")),
            env_host: non_empty(std::env::var_os("HERDR_SOCKET_PATH")),
            home: non_empty(std::env::var_os("HOME")),
            xdg_state_home: non_empty(std::env::var_os("XDG_STATE_HOME")),
            xdg_config_home: non_empty(std::env::var_os("XDG_CONFIG_HOME")),
            non_default_server: non_empty(std::env::var_os("HERDR_SESSION")).is_some()
                || non_empty(std::env::var_os("HERDR_CONFIG_PATH")).is_some(),
            herdr: hook::resolve_on_path("herdr", path.as_deref()),
        }
    }

    fn detect(&self) -> DetectInputs {
        DetectInputs {
            herdr: self.herdr.clone(),
            home: self.home.clone(),
            xdg_state_home: self.xdg_state_home.clone(),
        }
    }
}

fn absolute(dir: &Option<PathBuf>) -> Option<&Path> {
    dir.as_deref().filter(|dir| dir.is_absolute())
}

/// Herdr's plugin state root joined with the plugin id, under `base` (`$XDG_STATE_HOME` or
/// `~/.local/state`).
fn plugin_state_dir(base: &Path, local: bool) -> PathBuf {
    let base = if local {
        base.join(".local/state")
    } else {
        base.to_path_buf()
    };
    base.join("herdr").join("plugins").join(PLUGIN_ID)
}

/// The state directory the two Herdr plugin state roots resolve to. Herdr uses
/// `$XDG_STATE_HOME` when it is set, else `~/.local/state`; a `~/.local` directory next to a
/// set `XDG_STATE_HOME` is a legacy location, chosen only when it holds a store and the XDG
/// directory does not (an older install, from before XDG_STATE_HOME was set). A stale
/// `~/.local` directory without a store is never chosen over XDG. Both holding a store is
/// ambiguous. `Ok(None)`: no candidate directory exists (the caller falls back to Herdr);
/// the `bool` is whether the chosen directory exists.
pub(crate) fn choose_state_dir(
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<Option<(PathBuf, String, bool)>, String> {
    use crate::daemon::paths::holds_store;
    const LEGACY: &str = "legacy ~/.local/state (holds a store; XDG_STATE_HOME has none)";
    let xdg_choice = |xdg: PathBuf| {
        let exists = xdg.is_dir();
        Ok(Some((xdg, "XDG_STATE_HOME".to_owned(), exists)))
    };
    match (xdg, home) {
        (None, None) => Ok(None),
        (Some(xdg), None) => xdg_choice(xdg),
        (None, Some(home)) => {
            let exists = home.is_dir();
            Ok(Some((home, "~/.local/state".into(), exists)))
        }
        (Some(xdg), Some(home)) if xdg == home => xdg_choice(xdg),
        (Some(xdg), Some(home)) => {
            let (xdg_store, home_store) = (holds_store(&xdg), holds_store(&home));
            if xdg_store && home_store {
                return Err(format!(
                    "both {} and {} hold a store; pass --state-dir to choose the Herdr plugin \
                     state directory",
                    xdg.display(),
                    home.display()
                ));
            }
            if home_store {
                return Ok(Some((home, LEGACY.into(), true)));
            }
            xdg_choice(xdg)
        }
    }
}

/// The two candidate plugin state directories of `home` and `xdg_state_home`, each only when
/// the base is an absolute path.
pub(crate) fn state_candidates(
    home: Option<&Path>,
    xdg_state_home: Option<&Path>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    (
        xdg_state_home
            .filter(|dir| dir.is_absolute())
            .map(|dir| plugin_state_dir(dir, false)),
        home.filter(|dir| dir.is_absolute())
            .map(|dir| plugin_state_dir(dir, true)),
    )
}

/// The fast state-directory default: `Ok(None)` when no candidate exists (fall back),
/// `Err` when both Herdr state roots hold a store (ambiguous). A named Herdr session
/// (`non_default_server`) gets no fast default: its server, not the default one, owns the
/// plugin, so the slower `herdr` query decides.
pub fn default_state_dir(inputs: &InstanceInputs) -> Result<Option<(PathBuf, String)>, String> {
    if inputs.non_default_server {
        return Ok(None);
    }
    let (xdg, home) = state_candidates(absolute(&inputs.home), absolute(&inputs.xdg_state_home));
    Ok(choose_state_dir(xdg, home)?.and_then(|(dir, how, exists)| {
        exists.then(|| {
            let how = if how.starts_with("legacy") {
                how
            } else {
                format!("default ({how})")
            };
            (dir, how)
        })
    }))
}

/// What Herdr's plugin registry says about this plugin, read from a file only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRegistry {
    Installed,
    Disabled,
    NotInstalled,
    /// No registry file, or one this version does not understand: nothing is claimed.
    Unknown,
}

/// Herdr's plugin registry, `$XDG_CONFIG_HOME/herdr/plugins.json` (else
/// `~/.config/herdr/plugins.json`): a JSON array of plugins with `plugin_id` and `enabled`
/// (Herdr 0.9.1). A named Herdr session or `HERDR_CONFIG_PATH` may keep its own, so the
/// default registry is not read for them. File reads only, no subprocess.
pub fn plugin_registry(inputs: &InstanceInputs) -> PluginRegistry {
    if inputs.non_default_server {
        return PluginRegistry::Unknown;
    }
    let path = match absolute(&inputs.xdg_config_home) {
        Some(dir) => dir.join("herdr/plugins.json"),
        None => match absolute(&inputs.home) {
            Some(home) => home.join(".config/herdr/plugins.json"),
            None => return PluginRegistry::Unknown,
        },
    };
    let Ok(bytes) = std::fs::read(path) else {
        return PluginRegistry::Unknown;
    };
    let Ok(Value::Array(plugins)) = serde_json::from_slice::<Value>(&bytes) else {
        return PluginRegistry::Unknown;
    };
    if plugins
        .iter()
        .any(|plugin| !plugin["plugin_id"].is_string())
    {
        return PluginRegistry::Unknown;
    }
    match plugins
        .iter()
        .find(|plugin| plugin["plugin_id"] == PLUGIN_ID)
    {
        None => PluginRegistry::NotInstalled,
        Some(plugin) if plugin["enabled"] == json!(false) => PluginRegistry::Disabled,
        Some(_) => PluginRegistry::Installed,
    }
}

/// Why a state directory taken from the fast default (`how` starts with `default` or
/// `legacy`; flags, Herdr's environment and the `herdr plugin list` query already name an
/// installed plugin) may be a leftover: Herdr's registry says the plugin is not installed or
/// is disabled. `setup` refuses on it, `doctor` warns, other commands keep working.
pub fn leftover_state_dir(inputs: &InstanceInputs, state: &Path, how: &str) -> Option<String> {
    if !(how.starts_with("default") || how.starts_with("legacy")) {
        return None;
    }
    match plugin_registry(inputs) {
        PluginRegistry::NotInstalled => Some(format!(
            "plugin not installed (leftover state dir {}); install or link the {PLUGIN_ID} \
             Herdr plugin, or pass --state-dir",
            state.display()
        )),
        PluginRegistry::Disabled => Some(format!(
            "plugin disabled (leftover state dir {}); enable the {PLUGIN_ID} Herdr plugin, or \
             pass --state-dir",
            state.display()
        )),
        PluginRegistry::Installed | PluginRegistry::Unknown => None,
    }
}

/// The fast host default: Herdr's default server socket, when one exists.
pub fn default_host_endpoint(inputs: &InstanceInputs) -> Option<(PathBuf, String)> {
    if inputs.non_default_server {
        return None;
    }
    let (socket, how) = match absolute(&inputs.xdg_config_home) {
        Some(dir) => (dir.join("herdr/herdr.sock"), "default (XDG_CONFIG_HOME)"),
        None => (
            absolute(&inputs.home)?.join(".config/herdr/herdr.sock"),
            "default (~/.config)",
        ),
    };
    std::fs::symlink_metadata(&socket)
        .is_ok_and(|meta| meta.file_type().is_socket())
        .then(|| (socket, how.to_owned()))
}

/// The state directory and where it came from.
pub fn resolve_state_dir(inputs: &InstanceInputs) -> Result<(PathBuf, String), String> {
    if let Some(state) = &inputs.state_flag {
        return Ok((state.clone(), "--state-dir".into()));
    }
    if let Some(state) = &inputs.env_state {
        return Ok((state.clone(), "HERDR_PLUGIN_STATE_DIR".into()));
    }
    match default_state_dir(inputs)? {
        Some(found) => Ok(found),
        None => detect_state_dir(&inputs.detect()),
    }
}

/// The host endpoint and where it came from.
pub fn resolve_host_endpoint(inputs: &InstanceInputs) -> Result<(PathBuf, String), String> {
    if let Some(host) = &inputs.host_flag {
        return Ok((host.clone(), "--host-endpoint".into()));
    }
    if let Some(host) = &inputs.env_host {
        return Ok((host.clone(), "HERDR_SOCKET_PATH".into()));
    }
    match default_host_endpoint(inputs) {
        Some(found) => Ok(found),
        None => detect_host_endpoint(&inputs.detect()),
    }
}

/// Both fields resolved into a runtime context, plus `{"state_dir": how,
/// "host_endpoint": how}`. A failure names the flag and variable to use.
pub fn resolve_context(inputs: &InstanceInputs) -> io::Result<(RuntimeContext, Value)> {
    resolve_context_with_selected_state(inputs).map(|(context, source, _)| (context, source))
}

pub(crate) fn resolve_context_with_selected_state(
    inputs: &InstanceInputs,
) -> io::Result<(RuntimeContext, Value, PathBuf)> {
    let (state, state_how) = resolve_state_dir(inputs).map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("state directory unknown: {error}; use --state-dir or HERDR_PLUGIN_STATE_DIR"),
        )
    })?;
    let (host, host_how) = resolve_host_endpoint(inputs).map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Herdr host endpoint unknown: {error}; use --host-endpoint or HERDR_SOCKET_PATH"
            ),
        )
    })?;
    let selected_state = state.clone();
    let context = RuntimeContext::explicit(
        state,
        host,
        std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
    )?;
    let mut source = Map::new();
    if let Some(leftover) = leftover_state_dir(inputs, &context.state_dir, &state_how) {
        source.insert("state_dir_leftover".into(), json!(leftover));
    }
    source.insert("state_dir".into(), json!(state_how));
    source.insert("host_endpoint".into(), json!(host_how));
    Ok((context, Value::Object(source), selected_state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, os::unix::net::UnixListener};

    fn scratch() -> PathBuf {
        // Short, so socket paths stay under the sun_path limit.
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = PathBuf::from("/tmp").join(format!("ht-inst-{}", &id[..12]));
        fs::create_dir(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A `herdr` that records each call, so a test can prove the fast path
    /// spawned nothing.
    fn recording_herdr(dir: &Path, status: &str) -> PathBuf {
        let bin = dir.join("herdr");
        let log = dir.join("herdr.calls");
        fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\ncase \"$1 $2\" in\n\
                 'plugin list') printf '%s' '{{\"result\":{{\"plugins\":[{{\"plugin_id\":\"herdr-threads\",\"enabled\":true}}]}}}}';;\n\
                 'status server') printf '%s' '{status}';;\n*) exit 3;;\nesac\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn calls(dir: &Path) -> String {
        fs::read_to_string(dir.join("herdr.calls")).unwrap_or_default()
    }

    fn inputs(dir: &Path) -> InstanceInputs {
        InstanceInputs {
            home: Some(dir.join("home")),
            herdr: Some(recording_herdr(
                dir,
                r#"{"running":true,"socket":"/slow/herdr.sock"}"#,
            )),
            ..InstanceInputs::default()
        }
    }

    #[test]
    fn selected_state_capture_preserves_raw_spelling_and_canonical_identity() {
        let dir = scratch();
        let state = dir.join("state");
        fs::create_dir(&state).unwrap();
        let selected = dir.join("selected");
        std::os::unix::fs::symlink(&state, &selected).unwrap();
        let inputs = InstanceInputs {
            state_flag: Some(selected.clone()),
            host_flag: Some(dir.join("host.sock")),
            ..InstanceInputs::default()
        };
        let (context, source, raw) = resolve_context_with_selected_state(&inputs).unwrap();
        assert_eq!(raw, selected);
        assert_eq!(context.state_dir, state);
        assert_eq!(source["state_dir"], "--state-dir");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn selected_state_capture_queries_each_missing_field_once() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        inputs.herdr = Some(recording_herdr(
            &dir,
            &format!(
                r#"{{"running":true,"socket":"{}"}}"#,
                dir.join("host.sock").display()
            ),
        ));
        inputs.non_default_server = true;
        let (context, source, selected) = resolve_context_with_selected_state(&inputs).unwrap();
        assert_eq!(
            selected,
            dir.join("home/.local/state/herdr/plugins/herdr-threads")
        );
        assert_eq!(context.state_dir, selected);
        assert_eq!(source["host_endpoint"], "herdr status server");
        assert_eq!(calls(&dir), "plugin list --json\nstatus server --json\n");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Kills: requiring flags/env in a plain shell, spawning `herdr` when the
    /// defaults exist, or ignoring XDG_CONFIG_HOME for the socket.
    #[test]
    fn existing_defaults_resolve_without_any_subprocess() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        let state = dir.join("home/.local/state/herdr/plugins/herdr-threads");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(dir.join("home/.config/herdr")).unwrap();
        let socket = dir.join("home/.config/herdr/herdr.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        let (context, source) = resolve_context(&inputs).unwrap();
        assert_eq!(context.state_dir, state);
        assert_eq!(context.host_endpoint, socket);
        assert_eq!(source["state_dir"], "default (~/.local/state)");
        assert_eq!(source["host_endpoint"], "default (~/.config)");
        assert_eq!(calls(&dir), "", "fast path must not run herdr");

        // XDG_CONFIG_HOME moves Herdr's socket; nothing there -> slow query.
        inputs.xdg_config_home = Some(dir.join("xdg-config"));
        let (host, how) = resolve_host_endpoint(&inputs).unwrap();
        assert_eq!(host, PathBuf::from("/slow/herdr.sock"));
        assert_eq!(how, "herdr status server");
        assert_eq!(calls(&dir), "status server --json\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: a default overriding the flag or Herdr's env, a non-socket file
    /// taken as the server, or the default used for a named Herdr session.
    #[test]
    fn flags_then_env_win_and_non_sockets_or_sessions_fall_back() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        fs::create_dir_all(dir.join("home/.config/herdr")).unwrap();
        fs::write(dir.join("home/.config/herdr/herdr.sock"), "").unwrap();
        assert_eq!(default_host_endpoint(&inputs), None);
        let _listener = UnixListener::bind(dir.join("home/.config/herdr/x.sock")).unwrap();
        fs::remove_file(dir.join("home/.config/herdr/herdr.sock")).unwrap();
        fs::rename(
            dir.join("home/.config/herdr/x.sock"),
            dir.join("home/.config/herdr/herdr.sock"),
        )
        .unwrap();
        assert!(default_host_endpoint(&inputs).is_some());
        inputs.non_default_server = true;
        assert_eq!(default_host_endpoint(&inputs), None);
        inputs.non_default_server = false;

        inputs.env_host = Some(PathBuf::from("/env/herdr.sock"));
        inputs.env_state = Some(PathBuf::from("/env/state"));
        assert_eq!(
            resolve_host_endpoint(&inputs).unwrap().1,
            "HERDR_SOCKET_PATH"
        );
        assert_eq!(
            resolve_state_dir(&inputs).unwrap(),
            (PathBuf::from("/env/state"), "HERDR_PLUGIN_STATE_DIR".into())
        );
        inputs.host_flag = Some(PathBuf::from("/flag/h.sock"));
        inputs.state_flag = Some(PathBuf::from("/flag/state"));
        assert_eq!(resolve_host_endpoint(&inputs).unwrap().1, "--host-endpoint");
        assert_eq!(resolve_state_dir(&inputs).unwrap().1, "--state-dir");
        assert_eq!(calls(&dir), "");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: silently picking one of two existing state roots, or failing
    /// instead of asking Herdr when no default exists yet.
    #[test]
    fn state_default_refuses_ambiguity_and_falls_back_to_herdr_when_missing() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        let (state, how) = resolve_state_dir(&inputs).unwrap();
        assert_eq!(
            state,
            dir.join("home/.local/state/herdr/plugins/herdr-threads")
        );
        assert!(how.starts_with("herdr plugin list"), "{how}");
        assert_eq!(calls(&dir), "plugin list --json\n");

        inputs.xdg_state_home = Some(dir.join("xdg"));
        fs::create_dir_all(dir.join("xdg/herdr/plugins/herdr-threads")).unwrap();
        assert_eq!(
            resolve_state_dir(&inputs).unwrap().1,
            "default (XDG_STATE_HOME)"
        );
        fs::create_dir_all(dir.join("home/.local/state/herdr/plugins/herdr-threads")).unwrap();
        // A stale directory without a store never makes the XDG one ambiguous.
        assert_eq!(
            resolve_state_dir(&inputs).unwrap().1,
            "default (XDG_STATE_HOME)"
        );
        for root in [
            dir.join("xdg/herdr/plugins/herdr-threads"),
            dir.join("home/.local/state/herdr/plugins/herdr-threads"),
        ] {
            fs::create_dir_all(root.join("instances/abc")).unwrap();
            fs::write(root.join("instances/abc/threads.sqlite3"), "").unwrap();
        }
        let error = resolve_state_dir(&inputs).unwrap_err();
        assert!(error.contains("both"), "{error}");
        let error = resolve_context(&inputs).unwrap_err().to_string();
        assert!(error.contains("--state-dir"), "{error}");

        inputs.herdr = None;
        inputs.xdg_state_home = None;
        fs::remove_dir_all(dir.join("home")).unwrap();
        let error = resolve_context(&inputs).unwrap_err().to_string();
        assert!(error.contains("state directory unknown"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    fn store_in(state: &Path) {
        fs::create_dir_all(state.join("instances/abc")).unwrap();
        fs::write(state.join("instances/abc/threads.sqlite3"), "").unwrap();
    }

    /// Kills: the `~/.local` directory taken because it exists while `XDG_STATE_HOME` is set
    /// but its herdr-threads directory does not exist yet (Herdr would use the XDG root), and a
    /// stale `~/.local` directory (no store) making an existing XDG directory ambiguous.
    #[test]
    fn stale_local_state_dir_is_not_chosen_over_xdg() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        inputs.xdg_state_home = Some(dir.join("xdg"));
        let stale = dir.join("home/.local/state/herdr/plugins/herdr-threads");
        fs::create_dir_all(stale.join("setup")).unwrap();
        // XDG directory missing: no fast default; Herdr's answer is the XDG root.
        assert_eq!(default_state_dir(&inputs), Ok(None));
        let (state, how) = resolve_state_dir(&inputs).unwrap();
        assert_eq!(state, dir.join("xdg/herdr/plugins/herdr-threads"));
        assert_eq!(how, "herdr plugin list + XDG_STATE_HOME");
        // XDG directory present: chosen, the stale one is not ambiguous.
        fs::create_dir_all(dir.join("xdg/herdr/plugins/herdr-threads")).unwrap();
        assert_eq!(
            default_state_dir(&inputs).unwrap(),
            Some((
                dir.join("xdg/herdr/plugins/herdr-threads"),
                "default (XDG_STATE_HOME)".into()
            ))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: ignoring a store under `~/.local` after `XDG_STATE_HOME` was set (the daemon's
    /// threads would silently vanish), a source that does not say which directory and why, and
    /// picking one of two directories that both hold a store.
    #[test]
    fn legacy_local_state_dir_with_a_store_is_chosen() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        inputs.xdg_state_home = Some(dir.join("xdg"));
        let legacy = dir.join("home/.local/state/herdr/plugins/herdr-threads");
        store_in(&legacy);
        let why = "legacy ~/.local/state (holds a store; XDG_STATE_HOME has none)";
        inputs.host_flag = Some(dir.join("host.sock"));
        assert_eq!(
            default_state_dir(&inputs).unwrap(),
            Some((legacy.clone(), why.into()))
        );
        // The reason reaches `doctor` as the state directory's source.
        assert_eq!(resolve_context(&inputs).unwrap().1["state_dir"], why);
        // An XDG directory without a store does not displace it.
        fs::create_dir_all(dir.join("xdg/herdr/plugins/herdr-threads")).unwrap();
        assert_eq!(
            default_state_dir(&inputs).unwrap(),
            Some((legacy, why.into()))
        );
        // Both holding a store is ambiguous.
        store_in(&dir.join("xdg/herdr/plugins/herdr-threads"));
        assert!(default_state_dir(&inputs).unwrap_err().contains("both"));
        // Without XDG_STATE_HOME the `~/.local` directory is simply Herdr's root.
        inputs.xdg_state_home = None;
        assert_eq!(
            default_state_dir(&inputs).unwrap().unwrap().1,
            "default (~/.local/state)"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: a named Herdr session pairing the default plugin state root (and the default
    /// socket) with its own server: neither fast default may apply, so resolution falls through
    /// to the flags, Herdr's environment or the `herdr` query (the latter run in that session).
    #[test]
    fn herdr_session_disables_both_fast_defaults() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        let state = dir.join("home/.local/state/herdr/plugins/herdr-threads");
        fs::create_dir_all(&state).unwrap();
        fs::create_dir_all(dir.join("home/.config/herdr")).unwrap();
        let _listener = UnixListener::bind(dir.join("home/.config/herdr/herdr.sock")).unwrap();
        assert!(default_state_dir(&inputs).unwrap().is_some());
        assert!(default_host_endpoint(&inputs).is_some());
        inputs.non_default_server = true;
        assert_eq!(default_state_dir(&inputs), Ok(None));
        assert_eq!(default_host_endpoint(&inputs), None);
        // Resolution asks Herdr instead, for both fields.
        let (_, how) = resolve_state_dir(&inputs).unwrap();
        assert!(how.starts_with("herdr plugin list"), "{how}");
        let (host, how) = resolve_host_endpoint(&inputs).unwrap();
        assert_eq!(
            (host, how.as_str()),
            ("/slow/herdr.sock".into(), "herdr status server")
        );
        assert_eq!(calls(&dir), "plugin list --json\nstatus server --json\n");
        // Flags still win without any query.
        inputs.state_flag = Some(PathBuf::from("/flag/state"));
        assert_eq!(resolve_state_dir(&inputs).unwrap().1, "--state-dir");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Kills: a leftover state directory (plugin uninstalled or disabled) taken silently by the
    /// fast default, a registry check that fires for flags/env/queried directories, or one that
    /// claims anything when the registry file is missing or not understood.
    #[test]
    fn fast_state_dir_requires_an_installed_plugin() {
        let dir = scratch();
        let mut inputs = inputs(&dir);
        inputs.host_flag = Some(dir.join("host.sock"));
        let state = dir.join("home/.local/state/herdr/plugins/herdr-threads");
        fs::create_dir_all(&state).unwrap();
        let registry = dir.join("home/.config/herdr/plugins.json");
        fs::create_dir_all(registry.parent().unwrap()).unwrap();
        let leftover = |inputs: &InstanceInputs, how: &str| leftover_state_dir(inputs, &state, how);
        let fast = "default (~/.local/state)";

        // No registry file, or one that is not a list of plugins: nothing is claimed.
        assert_eq!(plugin_registry(&inputs), PluginRegistry::Unknown);
        assert_eq!(leftover(&inputs, fast), None);
        for text in ["not json", "{}", "[1]", "[{\"name\":\"x\"}]"] {
            fs::write(&registry, text).unwrap();
            assert_eq!(plugin_registry(&inputs), PluginRegistry::Unknown, "{text}");
        }

        fs::write(&registry, r#"[{"plugin_id":"other","enabled":true}]"#).unwrap();
        assert_eq!(plugin_registry(&inputs), PluginRegistry::NotInstalled);
        let message = leftover(&inputs, fast).unwrap();
        assert!(
            message.starts_with(&format!(
                "plugin not installed (leftover state dir {})",
                state.display()
            )),
            "{message}"
        );
        assert!(
            leftover(
                &inputs,
                "legacy ~/.local/state (holds a store; XDG_STATE_HOME has none)"
            )
            .is_some()
        );
        // Not from the fast default: flags, Herdr's env and `herdr plugin list` name the plugin.
        for how in [
            "--state-dir",
            "HERDR_PLUGIN_STATE_DIR",
            "herdr plugin list + ~/.local/state",
        ] {
            assert_eq!(leftover(&inputs, how), None, "{how}");
        }
        // The context report carries it for doctor.
        assert!(
            resolve_context(&inputs).unwrap().1["state_dir_leftover"]
                .as_str()
                .is_some_and(|m| m.starts_with("plugin not installed"))
        );

        fs::write(
            &registry,
            r#"[{"plugin_id":"herdr-threads","enabled":false}]"#,
        )
        .unwrap();
        assert_eq!(plugin_registry(&inputs), PluginRegistry::Disabled);
        assert!(
            leftover(&inputs, fast)
                .unwrap()
                .starts_with("plugin disabled")
        );
        fs::write(
            &registry,
            r#"[{"plugin_id":"herdr-threads","enabled":true}]"#,
        )
        .unwrap();
        assert_eq!(plugin_registry(&inputs), PluginRegistry::Installed);
        assert_eq!(leftover(&inputs, fast), None);
        assert!(resolve_context(&inputs).unwrap().1["state_dir_leftover"].is_null());

        // A named session keeps its own registry, which is not read.
        fs::write(&registry, "[]").unwrap();
        inputs.non_default_server = true;
        assert_eq!(plugin_registry(&inputs), PluginRegistry::Unknown);
        fs::remove_dir_all(&dir).unwrap();
    }
}
