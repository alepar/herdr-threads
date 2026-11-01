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
//!    `node/herdr-threads.db`, or a legacy `instances/<id>/` with the database) and the XDG directory does not; the
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
    fs, io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt},
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
    /// server, whose socket is not the default path: skip every fast default
    /// (socket, state directory, registry). Herdr keys the plugin registry and
    /// plugin state on the XDG home alone, so the state directory would match,
    /// but the `herdr` query decides it rather than a guess.
    pub non_default_server: bool,
    /// The `herdr` executable on PATH, for the slow fallback only.
    pub herdr: Option<PathBuf>,
    /// `HERDR_ENV` is set: the caller runs beside a Herdr (SA1 step 4).
    pub herdr_env: bool,
    /// The caller is a lone non-Pane site (a Named seat, `HERDR_THREADS_SEAT_NAME`, or a
    /// herdr-core terminal, `HC_TERMINAL_ID`): with no Herdr configured or on PATH it gets the
    /// standalone default state dir, beside a Herdr the command group's store, and its Herdr
    /// host endpoint resolves lazily (fast sources only, never a `herdr` query).
    pub non_pane_site: bool,
    /// The lone non-Pane site's claim host: a herdr-core terminal's core socket
    /// (`HC_TERMINAL_SOCKET`) or a named seat's reserved locator, ahead of a Herdr
    /// `HERDR_SOCKET_PATH` that may be stale in its environment.
    pub site_host: Option<PathBuf>,
}

fn non_empty(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl InstanceInputs {
    pub fn from_process(state_flag: Option<PathBuf>, host_flag: Option<PathBuf>) -> Self {
        let path = std::env::var_os("PATH");
        let caller = super::caller_location::CallerEnv::from_process();
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
            herdr_env: caller.herdr_env(),
            non_pane_site: caller.non_pane_site(),
            site_host: caller
                .non_pane_locator(super::caller_location::EnvMode::Cli)
                .and_then(|site| site.site().claim_host_locator().map(PathBuf::from)),
        }
    }

    /// A Herdr is configured for this process: a pane or session env, or a named server.
    fn herdr_configured(&self) -> bool {
        self.herdr_env || self.non_default_server
    }

    /// The Herdr host endpoint of a non-Pane caller, resolved at the one point that needs it (a
    /// handoff into a Herdr pane): the fast sources first, then the `herdr status server` query
    /// that discovery skips for a named site.
    pub(crate) fn lazy_host_endpoint(&self) -> io::Result<PathBuf> {
        let inputs = Self {
            non_pane_site: false,
            // A herdr-core terminal's core socket is not a Herdr server.
            site_host: None,
            ..self.clone()
        };
        resolve_host_endpoint(&inputs)
            .map(|(host, _)| host)
            .map_err(|error| io::Error::new(io::ErrorKind::NotFound, error))
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
/// (`non_default_server`) gets no fast default: the slower `herdr` query decides, so a
/// caller that set only `HERDR_SESSION` never lands in the default directory when that
/// query fails. (Herdr's plugin state is per XDG home, not per session.)
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

/// The standalone default state dir, whether or not it exists yet: the single definition of
/// that path (discovery and Herdr-less `setup` use it). `None` with no usable HOME/XDG root;
/// `Err` when both roots already hold a store (ambiguous, as for [`default_state_dir`]).
pub fn standalone_state_dir(inputs: &InstanceInputs) -> Result<Option<PathBuf>, String> {
    let (xdg, home) = state_candidates(absolute(&inputs.home), absolute(&inputs.xdg_state_home));
    Ok(choose_state_dir(xdg, home)?.map(|(dir, _, _)| dir))
}

/// Where the default state dir would be created. `None` for a named Herdr session. For a
/// non-Pane site (`non_pane_site`) also `None` unless no Herdr is configured (`HERDR_ENV`, `HERDR_SESSION`,
/// `HERDR_CONFIG_PATH` unset) and no `herdr` is on PATH: beside a Herdr the command group's
/// store is found by the `herdr plugin list` query (or refused), never split off a second store.
fn default_state_dir_to_create(inputs: &InstanceInputs) -> Result<Option<PathBuf>, String> {
    if inputs.non_default_server {
        return Ok(None);
    }
    if inputs.non_pane_site && (inputs.herdr_configured() || inputs.herdr.is_some()) {
        return Ok(None);
    }
    standalone_state_dir(inputs)
}

/// Create `dir` owner-only (the daemon refuses a group/other-writable state root, so the leaf
/// is 0700 whatever the umask).
fn create_state_dir(dir: &Path) -> io::Result<()> {
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// The context for `peer --stdio`, the ssh-side pump. Like [`resolve_context`], except that a
/// node never used locally (no `--state-dir`, no `HERDR_PLUGIN_STATE_DIR`, no default state dir
/// yet) creates the default state dir, owner-only, so the inbound link starts its daemon there
/// as a first local use would. An explicit state dir is never replaced: its error is returned.
pub fn resolve_peer_context(inputs: &InstanceInputs) -> io::Result<RuntimeContext> {
    let first = match resolve_context(inputs) {
        Ok((context, _)) => return Ok(context),
        Err(error) => error,
    };
    if inputs.state_flag.is_some() || inputs.env_state.is_some() {
        return Err(first);
    }
    let Some(dir) = default_state_dir_to_create(inputs).ok().flatten() else {
        return Err(first);
    };
    if dir.exists() {
        return Err(first);
    }
    create_state_dir(&dir)?;
    let created = InstanceInputs {
        state_flag: Some(dir),
        ..inputs.clone()
    };
    resolve_context(&created).map(|(context, _)| context)
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
/// (Herdr 0.9.1). Herdr keeps one registry per XDG config home, shared by every session;
/// it is still not read for a named session or `HERDR_CONFIG_PATH`, whose state directory
/// came from the `herdr` query rather than this home. File reads only, no subprocess.
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
    if !(how.starts_with("default") || how.starts_with("legacy")) || has_standalone_marker(state) {
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
        None => match default_state_dir_to_create(inputs) {
            // A non-Pane site with no Herdr configured or on PATH: the standalone default dir
            // (created by the context resolution). Otherwise the one `herdr plugin list` query.
            Ok(Some(dir)) if inputs.non_pane_site => Ok((dir, STANDALONE_HOW.into())),
            _ => detect_state_dir(&inputs.detect()),
        },
    }
}

/// The source label of a state directory chosen as the standalone default (a Named site, or
/// Herdr-less `setup`).
pub const STANDALONE_HOW: &str = "standalone default";

/// The standalone install marker, `<state>/node/install.json`: written by Herdr-less `setup`
/// (and so by the installer), it says no Herdr plugin owns this state dir, so it is no leftover.
const INSTALL_MARKER: &str = "install.json";
const STANDALONE_MARKER_BODY: &str = "{\"mode\":\"standalone\"}";

fn install_marker_path(state: &Path) -> PathBuf {
    crate::daemon::paths::node_dir(state).join(INSTALL_MARKER)
}

/// True when `state` carries the standalone install marker.
pub fn has_standalone_marker(state: &Path) -> bool {
    fs::read(install_marker_path(state)).is_ok_and(|bytes| {
        serde_json::from_slice::<Value>(&bytes).is_ok_and(|v| v["mode"] == "standalone")
    })
}

/// Write the standalone marker (owner-only; the node dir is created 0700 when absent).
pub fn write_standalone_marker(state: &Path) -> io::Result<()> {
    let node = crate::daemon::paths::node_dir(state);
    crate::daemon::paths::ensure_private_dir(&node)?;
    let path = install_marker_path(state);
    if fs::read(&path).is_ok_and(|bytes| bytes == STANDALONE_MARKER_BODY.as_bytes()) {
        return Ok(());
    }
    fs::write(&path, STANDALONE_MARKER_BODY)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
}

/// The state dir of a Herdr-less `setup`: no flag, no `HERDR_PLUGIN_STATE_DIR`, no Herdr
/// configured and no `herdr` on PATH. `None` when any of those holds (the usual resolution
/// applies); otherwise the standalone default, which is also what named-site discovery resolves.
pub fn herdrless_state_dir(inputs: &InstanceInputs) -> Option<Result<(PathBuf, String), String>> {
    if inputs.state_flag.is_some()
        || inputs.env_state.is_some()
        || inputs.herdr.is_some()
        || inputs.herdr_configured()
    {
        return None;
    }
    Some(match standalone_state_dir(inputs) {
        Ok(Some(dir)) => Ok((dir, STANDALONE_HOW.into())),
        Ok(None) => Err("neither XDG_STATE_HOME nor HOME is an absolute path".into()),
        Err(error) => Err(error),
    })
}

/// The claim host: the Herdr server of the calling pane. An explicit user flag on an interactive
/// command wins, then the pane's own `HERDR_SOCKET_PATH`, then the host baked into an installed
/// argv at setup, which is only a fallback (and the daemon attach hint): a user-level hook runs in
/// panes of any Herdr server, so the pane it runs in decides. Hooks pass no `explicit` (their argv
/// is always installed). Spellings are returned as given; `RuntimeContext` canonicalizes.
pub fn claim_host(
    explicit: Option<&Path>,
    pane_env: Option<&Path>,
    installed: Option<&Path>,
) -> Option<PathBuf> {
    explicit.or(pane_env).or(installed).map(Path::to_path_buf)
}

/// The host endpoint and where it came from.
pub fn resolve_host_endpoint(inputs: &InstanceInputs) -> Result<(PathBuf, String), String> {
    // A named seat's claim host is always its reserved locator; an explicit `--host-endpoint`
    // beside a name names the Herdr server a handoff creates its target on (`lazy_host_endpoint`).
    if let Some(host) = inputs
        .site_host
        .as_ref()
        .filter(|host| host.as_os_str() == crate::protocol::ids::NAMED_HOST_LOCATOR)
    {
        return Ok((host.clone(), "the named site reserved locator".into()));
    }
    if let Some(host) = &inputs.host_flag {
        return Ok((host.clone(), "--host-endpoint".into()));
    }
    if let Some(host) = &inputs.site_host {
        return Ok((host.clone(), "the terminal site core socket".into()));
    }
    if let Some(host) = &inputs.env_host {
        return Ok((host.clone(), "HERDR_SOCKET_PATH".into()));
    }
    match default_host_endpoint(inputs) {
        Some(found) => Ok(found),
        // Lazy for a non-Pane site: no `herdr status server` query on every call; a command
        // that needs the host fails later, in `RuntimeContext::require_host`.
        None if inputs.non_pane_site => {
            Err("Herdr host endpoint not resolved for a named site".into())
        }
        None => detect_host_endpoint(&inputs.detect()),
    }
}

/// The state directory (required) and the claim host (optional) resolved into
/// a runtime context, plus `{"state_dir": how, "host_endpoint": how}`; the host
/// source is `"none"` when no Herdr server resolved. A failure to find the
/// state directory names the flag and variable to use; a call that needs the
/// host fails later, in [`RuntimeContext::require_host`].
pub fn resolve_context(inputs: &InstanceInputs) -> io::Result<(RuntimeContext, Value)> {
    resolve_context_with_selected_state(inputs).map(|(context, source, _)| (context, source))
}

pub(crate) fn resolve_context_with_selected_state(
    inputs: &InstanceInputs,
) -> io::Result<(RuntimeContext, Value, PathBuf)> {
    resolve_context_with_selected_paths(inputs)
        .map(|(context, source, state, _)| (context, source, state))
}

/// Preserve exact selected spellings for local asset ownership while the runtime
/// context uses canonical paths. Each source is resolved only once.
pub(crate) fn resolve_context_with_selected_paths(
    inputs: &InstanceInputs,
) -> io::Result<(RuntimeContext, Value, PathBuf, Option<PathBuf>)> {
    let (state, state_how) = resolve_state_dir(inputs).map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("state directory unknown: {error}; use --state-dir or HERDR_PLUGIN_STATE_DIR"),
        )
    })?;
    let (host, host_how) = match resolve_host_endpoint(inputs) {
        Ok((host, how)) => (Some(host), how),
        Err(_) => (None, "none".to_owned()),
    };
    if state_how == STANDALONE_HOW {
        create_state_dir(&state)?;
    }
    let selected_state = state.clone();
    let selected_host = host.clone();
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
    Ok((
        context,
        Value::Object(source),
        selected_state,
        selected_host,
    ))
}

#[cfg(test)]
#[path = "../../tests/cli/instance_named.rs"]
mod instance_named;

#[cfg(test)]
#[path = "../../tests/daemon/terminal_site.rs"]
mod terminal_site;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, os::unix::net::UnixListener};

    fn fresh_inputs(home: &Path) -> InstanceInputs {
        InstanceInputs {
            home: Some(home.to_path_buf()),
            ..InstanceInputs::default()
        }
    }

    #[test]
    fn peer_context_on_a_fresh_node_creates_an_owner_only_default_state_dir() {
        let root = scratch();
        let context = resolve_peer_context(&fresh_inputs(&root)).unwrap();
        let state = root.join(".local/state/herdr/plugins/herdr-threads");
        assert_eq!(context.state_dir, state.canonicalize().unwrap());
        let mode = fs::metadata(&state).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "owner-only whatever the umask");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn peer_context_never_replaces_an_explicit_state_dir() {
        let root = scratch();
        for inputs in [
            InstanceInputs {
                state_flag: Some(PathBuf::from("rel/state")),
                ..fresh_inputs(&root)
            },
            InstanceInputs {
                env_state: Some(PathBuf::from("rel/state")),
                ..fresh_inputs(&root)
            },
        ] {
            assert!(resolve_peer_context(&inputs).is_err());
            assert!(
                !root.join(".local").exists(),
                "no default state dir is created"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

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

    /// The standalone marker exempts a fast-default state dir from the leftover check, and the
    /// Herdr-less state dir is the standalone default that named discovery also resolves.
    #[test]
    fn standalone_marker_is_no_leftover_and_herdrless_dir_is_the_default() {
        let dir = scratch();
        let mut inputs = InstanceInputs {
            home: Some(dir.join("home")),
            ..InstanceInputs::default()
        };
        let state = standalone_state_dir(&inputs).unwrap().unwrap();
        assert_eq!(
            herdrless_state_dir(&inputs).unwrap().unwrap(),
            (state.clone(), STANDALONE_HOW.into())
        );
        // A Herdr on PATH, a flag or a configured Herdr leave the usual resolution.
        inputs.herdr = Some(dir.join("herdr"));
        assert!(herdrless_state_dir(&inputs).is_none());
        inputs.herdr = None;
        inputs.state_flag = Some(dir.join("s"));
        assert!(herdrless_state_dir(&inputs).is_none());
        inputs.state_flag = None;

        fs::create_dir_all(&state).unwrap();
        let registry = dir.join("home/.config/herdr/plugins.json");
        fs::create_dir_all(registry.parent().unwrap()).unwrap();
        fs::write(&registry, "[]").unwrap();
        let fast = "default (~/.local/state)";
        assert!(leftover_state_dir(&inputs, &state, fast).is_some());
        assert!(!has_standalone_marker(&state));
        write_standalone_marker(&state).unwrap();
        write_standalone_marker(&state).unwrap();
        assert!(has_standalone_marker(&state));
        assert_eq!(
            fs::read_to_string(state.join("node/install.json")).unwrap(),
            r#"{"mode":"standalone"}"#
        );
        assert_eq!(leftover_state_dir(&inputs, &state, fast), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unresolved_host_is_none_and_reported_as_such() {
        let dir = scratch();
        let state = dir.join("state");
        fs::create_dir(&state).unwrap();
        let inputs = InstanceInputs {
            home: Some(dir.join("home")),
            state_flag: Some(state.clone()),
            non_default_server: true,
            ..InstanceInputs::default()
        };
        let (context, source, _, selected_host) =
            resolve_context_with_selected_paths(&inputs).unwrap();
        assert_eq!(context.host_endpoint, None);
        assert_eq!(selected_host, None);
        assert_eq!(source["host_endpoint"], "none");
        assert_eq!(source["state_dir"], "--state-dir");
        assert_eq!(
            context.require_host().unwrap_err().to_string(),
            "host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH"
        );
        fs::remove_dir_all(dir).unwrap();
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
        let (context, source, selected, selected_host) =
            resolve_context_with_selected_paths(&inputs).unwrap();
        assert_eq!(selected_host, Some(dir.join("host.sock")));
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
        assert_eq!(context.host_endpoint, Some(socket));
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
