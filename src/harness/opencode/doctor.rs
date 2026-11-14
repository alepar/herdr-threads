//! opencode doctor (opencode design D6, D8): the `hooks.opencode` projection from the setup
//! status, the plugin heartbeat, other installed copies of the plugin and same-named skills.
//!
//! Heartbeat (the plugin's contract): the plugin writes `<state>/opencode/plugin-heartbeat.json`
//! (the `--state-dir` of its launch line; none when the line is unrendered) at the end of its
//! init, after its hooks are set up (code before a failing init also runs, so an earlier write
//! proves nothing), and rewrites it, fields accumulating, when consent or a capability changes:
//! `{"schema":1,"plugin":"herdr-threads","pid":N,"role":"active"|"inert","directory":"...",
//!   "launch":bool,"permissions":"consented"|"not_consented",
//!   "capabilities":{"append","wake","status","session_get","toast": bool},
//!   "loaded_at":ms,"at":ms,"permission":{"granted":[..],"denied":[..]}?,
//!   "degraded":["append"|"append_row"|"wake"]?}`. Unknown fields are ignored.
//! No heartbeat with the plugin installed is "never loaded"; a `loaded_at` older than the
//! installed file is "not loaded since setup"; role `inert` is a second loaded copy; a `false`
//! capability or a `degraded` entry is a degraded capability; `permission.denied` lists the
//! allow keys a user `deny` rule kept the plugin from adding.
use super::setup::{
    self, ALLOW_KEYS, InstallStatus, PLUGIN_PATH, PLUGIN_SOURCE, heartbeat_path, xdg_config_dir,
};
use crate::{
    cli::doctor::scalar,
    harness::adapter::{
        DoctorProjection, DoctorTextForm, SetupEnvironment, SetupStatus, StatusRequest,
    },
    protocol::time::CallBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_HEARTBEAT_BYTES: u64 = 65_536;
/// Project directories walked from the cwd up to the worktree root, at most.
const MAX_PROJECT_LEVELS: usize = 32;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Heartbeat {
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub launch: Option<bool>,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(default)]
    pub loaded_at: Option<u64>,
    #[serde(default)]
    pub at: Option<u64>,
    #[serde(default)]
    pub permission: Option<PermissionResult>,
    #[serde(default)]
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct PermissionResult {
    #[serde(default)]
    pub granted: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
}

/// What the heartbeat file says, if anything.
#[derive(Debug, Clone)]
pub enum HeartbeatRead {
    Absent,
    Unreadable(String),
    Present(Heartbeat),
}

pub fn read_heartbeat(path: &Path) -> HeartbeatRead {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return HeartbeatRead::Absent,
        Err(error) => return HeartbeatRead::Unreadable(error.to_string()),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_HEARTBEAT_BYTES {
        return HeartbeatRead::Unreadable("not a bounded regular file".into());
    }
    match std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(heartbeat) => HeartbeatRead::Present(heartbeat),
        Err(error) => HeartbeatRead::Unreadable(error),
    }
}

fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    u64::try_from(modified.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// opencode's config directories other than `root`: `OPENCODE_CONFIG_DIR`, the XDG one,
/// `~/.opencode` and every `.opencode/` from the cwd up to the worktree root.
fn other_config_dirs(root: &Path, env: &SetupEnvironment) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = env
        .declared
        .get("OPENCODE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
    {
        dirs.push(PathBuf::from(dir));
    }
    dirs.extend(xdg_config_dir(env));
    if let Some(home) = env.home.as_deref().filter(|home| !home.is_empty()) {
        dirs.push(Path::new(home).join(".opencode"));
    }
    dirs.extend(project_dirs(env).iter().map(|dir| dir.join(".opencode")));
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        if dir != root && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// The cwd and its ancestors up to the first one holding `.git` (inclusive), bounded.
fn project_dirs(env: &SetupEnvironment) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !env.cwd.is_absolute() {
        return out;
    }
    for dir in env.cwd.ancestors().take(MAX_PROJECT_LEVELS) {
        out.push(dir.to_path_buf());
        if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
            break;
        }
    }
    out
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Other installed copies of the plugin that opencode would also load
/// (`{plugin,plugins}/herdr-threads.{js,ts}` in every config directory).
pub fn plugin_copies(root: &Path, env: &SetupEnvironment) -> Vec<PathBuf> {
    let owned = root.join(PLUGIN_PATH);
    let mut dirs = vec![root.to_path_buf()];
    dirs.extend(other_config_dirs(root, env));
    let mut out = Vec::new();
    for dir in dirs {
        for sub in ["plugin", "plugins"] {
            for ext in ["js", "ts"] {
                let path = dir.join(sub).join(format!("herdr-threads.{ext}"));
                if path != owned && exists(&path) {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// Same-named `herdr-threads` skills opencode also discovers; it keeps one of them, not by
/// location (probe item 4).
pub fn skill_copies(root: &Path, env: &SetupEnvironment) -> Vec<PathBuf> {
    let owned = root.join(setup::SKILL_PATH);
    let mut skill_dirs = Vec::new();
    for dir in std::iter::once(root.to_path_buf()).chain(other_config_dirs(root, env)) {
        skill_dirs.push(dir.join("skill"));
        skill_dirs.push(dir.join("skills"));
    }
    let mut bases = Vec::new();
    if let Some(home) = env.home.as_deref().filter(|home| !home.is_empty()) {
        bases.push(PathBuf::from(home));
    }
    bases.extend(project_dirs(env));
    for base in bases {
        skill_dirs.push(base.join(".claude").join("skills"));
        skill_dirs.push(base.join(".agents").join("skills"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in skill_dirs {
        let path = dir.join("herdr-threads").join("SKILL.md");
        if path != owned && exists(&path) && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// What each capability's absence costs (design D8).
fn capability_effect(name: &str) -> &'static str {
    match name {
        "append" => "messages arrive through the turn hook only",
        "append_row" => "appends are acked without the stored message id",
        "wake" => "no idle wake (wake=no_wake)",
        "status" => "busy state unknown, no idle wake",
        "session_get" => "subagent parent lookup unavailable",
        "toast" => "no toast for a hidden woken turn's prompt",
        _ => "degraded",
    }
}

fn heartbeat_json(path: Option<&Path>, read: &HeartbeatRead, status: &InstallStatus) -> Value {
    let owned = status.root.join(PLUGIN_PATH);
    let mut value = json!({
        "path": path.map(|p| p.display().to_string()),
    });
    match read {
        HeartbeatRead::Absent => value["state"] = json!("absent"),
        HeartbeatRead::Unreadable(error) => {
            value["state"] = json!("unreadable");
            value["error"] = json!(error);
        }
        HeartbeatRead::Present(heartbeat) => {
            let stale = match (heartbeat.loaded_at, mtime_ms(&owned)) {
                (Some(at), Some(written)) => at < written,
                _ => false,
            };
            let state = if stale {
                "stale"
            } else if heartbeat.role.as_deref() == Some("inert") {
                "inert"
            } else {
                "loaded"
            };
            value["state"] = json!(state);
            value["pid"] = json!(heartbeat.pid);
            value["role"] = json!(heartbeat.role);
            value["launch"] = json!(heartbeat.launch);
            value["permissions"] = json!(heartbeat.permissions);
            value["loaded_at"] = json!(heartbeat.loaded_at);
            value["at"] = json!(heartbeat.at);
            value["capabilities"] = json!(heartbeat.capabilities);
            value["degraded"] = json!(heartbeat.degraded);
            value["denied"] = json!(
                heartbeat
                    .permission
                    .as_ref()
                    .map(|p| p.denied.clone())
                    .unwrap_or_default()
            );
        }
    }
    value
}

fn limitations_of(hooks: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let root = scalar(&hooks["config_root"]);
    let plugin = hooks["plugin"]["state"].as_str().unwrap_or_default();
    let skill = hooks["skill"]["state"].as_str().unwrap_or_default();
    match plugin {
        "not_installed" => {
            out.push("opencode plugin is not installed: run `herdr-threads setup opencode`".into())
        }
        "unowned" => out.push(format!(
            "opencode plugin {} is not owned by herdr-threads: move it aside, then run `herdr-threads setup opencode`",
            scalar(&hooks["plugin"]["path"])
        )),
        "missing" | "modified" | "stale" => out.push(format!(
            "opencode plugin is {plugin}: run `herdr-threads setup opencode`"
        )),
        _ => {}
    }
    if matches!(skill, "missing" | "modified" | "stale" | "unowned") {
        out.push(format!(
            "opencode skill is {}: run `herdr-threads setup opencode`",
            skill.replace('_', "-")
        ));
    }
    let heartbeat = &hooks["heartbeat"];
    if plugin == "installed" {
        match heartbeat["state"].as_str() {
            Some("absent") => out.push(format!(
                "opencode plugin installed, never loaded: start opencode (a load failure is logged only in opencode's log, \"failed to load plugin\"); config root {root}"
            )),
            Some("stale") => out.push(
                "opencode plugin not loaded since setup wrote it: restart opencode".into(),
            ),
            Some("inert") => out.push(
                "opencode loaded a second herdr-threads plugin copy, which stays inert; remove the extra copy"
                    .into(),
            ),
            Some("unreadable") => out.push(format!(
                "opencode plugin heartbeat unreadable: {}",
                scalar(&heartbeat["error"])
            )),
            _ => {}
        }
    }
    let mut lost: Vec<String> = heartbeat["capabilities"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, present)| **present == json!(false))
        .map(|(name, _)| name.clone())
        .collect();
    for name in heartbeat["degraded"].as_array().into_iter().flatten() {
        let name = scalar(name);
        if !lost.contains(&name) {
            lost.push(name);
        }
    }
    for name in lost {
        out.push(format!(
            "opencode plugin degraded: no {name} ({})",
            capability_effect(&name)
        ));
    }
    for key in heartbeat["denied"].as_array().into_iter().flatten() {
        out.push(format!(
            "opencode permission rule denies \"{}\": the plugin adds no allow for it",
            scalar(key)
        ));
    }
    for copy in hooks["copies"].as_array().into_iter().flatten() {
        out.push(format!(
            "second herdr-threads plugin copy installed: {} (only one instance runs; remove it)",
            scalar(copy)
        ));
    }
    out
}

pub(super) fn doctor_projection(
    request: &StatusRequest,
    _: &Value,
    _: &CallBudget,
) -> Option<DoctorProjection> {
    let SetupStatus::Detailed(status) = setup::status(request) else {
        return None;
    };
    let root = setup::root_of(&request.scope).ok()?.to_path_buf();
    let env = &request.environment;
    let install = setup::install_status(&root, env, PLUGIN_SOURCE);
    let heartbeat = env.state_dir.as_deref().map(heartbeat_path);
    let read = heartbeat
        .as_deref()
        .map_or(HeartbeatRead::Absent, read_heartbeat);
    let mut hooks = status.projection.clone();
    hooks["heartbeat"] = heartbeat_json(heartbeat.as_deref(), &read, &install);
    hooks["copies"] = json!(
        plugin_copies(&root, env)
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );
    hooks["skill_copies"] = json!(
        skill_copies(&root, env)
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );
    // Without opencode on PATH or its config root, a missing plugin is no degradation.
    let present = crate::cli::hook::resolve_on_path("opencode", env.path.as_deref()).is_some()
        || root.is_dir();
    let limitations = if present {
        limitations_of(&hooks)
    } else {
        Vec::new()
    };
    let safe_repairs = if present {
        status.repairs.clone()
    } else {
        Vec::new()
    };
    Some(DoctorProjection {
        status: Some(SetupStatus::Detailed(status)),
        hooks,
        limitations,
        manual_repairs: Vec::new(),
        safe_repairs,
        repair_options: Default::default(),
    })
}

/// Text for `report.hooks.opencode`.
pub(super) fn doctor_text(hooks: &Value, form: DoctorTextForm) -> String {
    if !hooks["plugin"].is_object() {
        return String::new();
    }
    let state = |value: &Value| scalar(value).replace('_', "-");
    match form {
        DoctorTextForm::Summary => {
            let mut out = format!(
                "opencode: plugin {}, skill {}, permissions {}, heartbeat {}\n",
                state(&hooks["plugin"]["state"]),
                state(&hooks["skill"]["state"]),
                state(&hooks["permissions"]["state"]),
                state(&hooks["heartbeat"]["state"]),
            );
            for copy in hooks["skill_copies"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "opencode: same-named skill {} (opencode keeps one copy at random)\n",
                    scalar(copy)
                ));
            }
            out
        }
        DoctorTextForm::Debug => {
            let mut out = String::new();
            for key in ["plugin", "skill"] {
                out.push_str(&format!(
                    "hooks.opencode.{key}: {} {}\n",
                    scalar(&hooks[key]["path"]),
                    state(&hooks[key]["state"]),
                ));
            }
            out.push_str(&format!(
                "hooks.opencode.permissions: {} ({})\n",
                state(&hooks["permissions"]["state"]),
                ALLOW_KEYS.join(", "),
            ));
            let version = &hooks["harness_version"];
            out.push_str(&format!(
                "hooks.opencode.installed: {} (binary {})\n",
                scalar(&version["admission"]),
                scalar(&version["binary"]),
            ));
            let heartbeat = &hooks["heartbeat"];
            out.push_str(&format!(
                "hooks.opencode.heartbeat: {} {} pid={} loaded_at={} permissions={}\n",
                scalar(&heartbeat["path"]),
                state(&heartbeat["state"]),
                scalar(&heartbeat["pid"]),
                scalar(&heartbeat["loaded_at"]),
                scalar(&heartbeat["permissions"]),
            ));
            for key in ["copies", "skill_copies"] {
                for copy in hooks[key].as_array().into_iter().flatten() {
                    out.push_str(&format!("hooks.opencode.{key}: {}\n", scalar(copy)));
                }
            }
            out
        }
    }
}
