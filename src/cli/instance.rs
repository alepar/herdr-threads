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
//!    when a socket exists there;
//! 4. only when the default is missing, the slower `herdr` queries
//!    (`plugin list --json` / `status server --json`).
//!
//! Nothing is cached: every invocation re-resolves, so a moved or restarted
//! Herdr is never answered from stale data. Two existing state roots are
//! refused as ambiguous rather than picked.

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
    /// server, whose socket is not the default path: skip the fast default.
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

/// The fast state-directory default: `Ok(None)` when no candidate exists
/// (fall back), `Err` when both Herdr state roots hold one (ambiguous).
pub fn default_state_dir(inputs: &InstanceInputs) -> Result<Option<(PathBuf, String)>, String> {
    let xdg = absolute(&inputs.xdg_state_home)
        .map(|dir| dir.join("herdr").join("plugins").join(PLUGIN_ID))
        .filter(|dir| dir.is_dir());
    let home = absolute(&inputs.home)
        .map(|dir| dir.join(".local/state/herdr/plugins").join(PLUGIN_ID))
        .filter(|dir| dir.is_dir());
    match (xdg, home) {
        (Some(xdg), Some(home)) if xdg != home => Err(format!(
            "both {} and {} exist; pass --state-dir to choose the Herdr plugin state directory",
            xdg.display(),
            home.display()
        )),
        (Some(xdg), _) => Ok(Some((xdg, "default (XDG_STATE_HOME)".into()))),
        (None, Some(home)) => Ok(Some((home, "default (~/.local/state)".into()))),
        (None, None) => Ok(None),
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
    let context = RuntimeContext::explicit(
        state,
        host,
        std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from),
    )?;
    let mut source = Map::new();
    source.insert("state_dir".into(), json!(state_how));
    source.insert("host_endpoint".into(), json!(host_how));
    Ok((context, Value::Object(source)))
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
}
