//! Owned Codex local setup backend.
use std::time::Duration;

use crate::cli::{
    RunError,
    setup::{PermissionPolicy, SetupEnv, SetupRequest, SetupVerb},
};
use crate::harness::launch::compose_native_argv;
use crate::harness::setup::legacy::*;
use crate::harness::setup::{
    EventGroups, HookInspection, InstallPhase, NativeObservation, OwnedEntry, OwnershipManifest,
    event_command, groups_fingerprint, merge, owned, uninstall_json,
};
use crate::ports::ConfiguredHook;
use crate::{
    daemon::paths::{RuntimeContext, node_dir},
    harness::{
        codex,
        codex::permissions::{CodexPermissionState, CodexPermissions},
        codex_config,
        context::Harness,
        permissions::PermissionConsent,
        recipe,
        setup::{self as lib, SetupError, remove_user_settings, shared_command, shell_command},
    },
    protocol::results::ErrorCode,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
/// A file under Codex's config root.
pub(crate) fn codex_file(env: &SetupEnv, name: &str) -> Result<PathBuf, RunError> {
    Ok(env.config_root("codex")?.join(name))
}

/// Codex's config root, when one resolved.
pub(crate) fn codex_root(env: &SetupEnv) -> Option<&Path> {
    env.config_roots.get("codex").map(PathBuf::as_path)
}

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
pub(crate) const CODEX_CONFIG_LIMIT: u64 = 1 << 20;

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
/// trust is not ours to read). Like Codex (`discover_project_layers` skips a `.codex/` equal to
/// `CODEX_HOME`), a `.codex/` that resolves to `CODEX_HOME` itself is not a project layer.
pub fn codex_layer_paths(codex_home: Option<&Path>, cwd: &Path) -> Vec<PathBuf> {
    let identity = |dir: &Path| lib::canonical_path(dir).unwrap_or_else(|_| dir.to_path_buf());
    let home_identity = codex_home.map(identity);
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
        let project = dir.join(".codex");
        if home_identity.as_ref() == Some(&identity(&project)) {
            continue;
        }
        dirs.push(project);
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

pub(crate) fn owned_events() -> impl Iterator<Item = &'static str> {
    codex::DECLARATION.owned_hooks.iter().map(|hook| hook.event)
}

/// Owned events a `config.toml` declares hooks for, by a conservative scan of
/// table headers and dotted keys (`[[hooks.E]]`, `[hooks.E]`, `hooks.E =`, and
/// `E =` inside `[hooks]`). Informational: Codex keeps these hooks either way.
pub(crate) fn toml_hook_events(text: &str) -> Vec<&'static str> {
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

pub(crate) fn json_hook_events(bytes: &[u8]) -> Vec<&'static str> {
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
    let contains = |command: &str| {
        let escaped = serde_json::to_string(command).unwrap_or_default();
        text.contains(command) || text.contains(escaped.trim_matches('"'))
    };
    let stable_commands = shlex::split(owned_command)
        .and_then(|argv| {
            lib::native_entries(codex::settings_kind(), &argv)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    layer.has_owned_command = contains(owned_command)
        || stable_commands.iter().any(|entry| {
            entry.group["hooks"][0]["command"]
                .as_str()
                .is_some_and(contains)
        });
    layer.has_other_herdr_threads_hook = !layer.has_owned_command
        && text.contains("herdr-threads")
        && (text.contains("'hook' 'codex'") || text.contains("'hook-event' 'codex'"));
    Some(layer)
}

/// Every existing Codex config layer file, observed read-only.
pub(crate) fn observe_codex_config(env: &SetupEnv, owned_command: &str) -> Vec<CodexLayerFile> {
    codex_layer_paths(codex_root(env), &env.cwd)
        .iter()
        .filter_map(|path| observe_codex_layer(path, owned_command))
        .collect()
}

pub(crate) fn codex_config_json(env: &SetupEnv, layers: &[CodexLayerFile]) -> Value {
    json!({
        "codex_home": codex_root(env).map(|home| home.display().to_string()),
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
    user_codex_home: Option<&Path>,
) -> Result<(), RunError> {
    let identity = |dir: &Path| lib::canonical_path(dir).unwrap_or_else(|_| dir.to_path_buf());
    match layers
        .iter()
        .find(|layer| layer.has_owned_command && layer.path != managed)
    {
        None => Ok(()),
        // The user-level home (`~/.codex`) read as a project layer: another installation, not
        // a stray copy. Codex loads it for sessions started under its parent once that project
        // and these hooks are trusted, so the refusal stays, without removal advice.
        Some(layer)
            if user_codex_home
                .is_some_and(|home| layer.path.parent().map(identity) == Some(identity(home))) =>
        {
            let dir = layer.path.parent().unwrap_or(&layer.path);
            let project = dir.parent().unwrap_or(dir);
            Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} is the user-level Codex home, which already has the herdr-threads hook; \
                     Codex also reads it as a project layer for sessions started in {}, which \
                     would run the hook twice once that project and its hooks are trusted. To \
                     set up {} anyway, run setup from another working directory; nothing was \
                     changed",
                    dir.display(),
                    project.display(),
                    managed.parent().unwrap_or(managed).display()
                ),
            ))
        }
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

/// What the Codex sandbox allowance enables, reported with it.
pub fn codex_sandbox_note() -> String {
    let measured = CODEX_SANDBOX_MEASURED_VERSIONS.join(" and ");
    format!(
        "Codex's default `-s workspace-write` sandbox refuses \
     connect() to the herdr-threads daemon socket (EPERM), so sandboxed herdr-threads commands \
     cannot reach the daemon (transport_denied). These three config.toml keys add this one \
     Unix socket to the sandbox's allowed sockets: sandbox_workspace_write.network_access=true is what makes Codex start its \
     network proxy (without it the proxy settings do nothing); features.network_proxy.enabled=true \
     turns on the proxy's enforcement; features.network_proxy.unix_sockets gains only the named \
     daemon socket (sockets already listed there stay allowed). No domain is allowed, so other network access from sandboxed commands stays \
     denied: measured on Codex {measured} only, the only versions setup writes this \
     allowance for. In the Codex demo and sandbox-probe runs other Unix sockets, \
     the Herdr server socket, loopback and external TCP were refused (EPERM) and proxied HTTPS \
     got 403. The socket path \
     is stable across daemon restarts but belongs to this state directory and Herdr instance. \
     workspace-write also refuses writes outside the workspace and tmp (EPERM), and every \
     mutation (send, ack, accept, leave, invite, check-in) records a pending-operation intent \
     and the caller's context locally first, so sandbox_workspace_write.writable_roots gains \
     exactly this instance's two client-side journal directories, <instance>/intents and \
     <instance>/contexts; the instance directory, the SQLite database and the daemon's files \
     stay read-only (measured in codex-sandbox-writes-probe)"
    )
}

/// Codex versions on which the allowance's default-deny was measured: the
/// proxy started, only the allowlisted socket connected, and other Unix
/// sockets, loopback, external TCP and proxied HTTPS stayed denied (Codex
/// demo 2 and the ht-910 `codex sandbox` A/B/C/D check, both 0.159.2; the
/// ht-4is.8.15 no-model `codex sandbox` probe on 0.159.3, evidence in
/// docs/evidence/codex-1593-sandbox-probe/). The
/// dangerous half of the allowance is `network_access=true`: on a build that
/// ignored or did not enforce `features.network_proxy`, it would leave
/// workspace-write with unrestricted networking. So setup writes the
/// allowance only for a measured version, never merely an admitted one.
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
    // go quiet because the ownership record is damaged. A record whose keys were all
    // removed by hand has nothing installed to warn about.
    if let Ok(None) = codex_config::read_manifest(&paths.config_manifest) {
        return None;
    }
    if codex_config::inspect(&paths.config, &paths.config_manifest)
        .is_ok_and(|inspection| inspection.removed)
    {
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
    let instance = canonical_prefix(&node_dir(&context.state_dir));
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
pub(crate) fn canonical_prefix(path: &Path) -> PathBuf {
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
        "the Codex sandbox allowance in {} does not list this instance's writable roots ({}) in sandbox_workspace_write.writable_roots: under the workspace-write sandbox herdr-threads send, ack, accept, leave, invite and check-in fail with `Operation not permitted`. Run `herdr-threads setup codex` to add them",
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
    let hooks = codex_file(env, "hooks.json")?;
    let config = codex_file(env, "config.toml")?;
    let state = env.state_dir()?;
    Ok(CodexPaths {
        hooks_manifest: manifest_path(state, "codex-user", &hooks),
        config_manifest: manifest_path(state, "codex-config", &config),
        hooks,
        config,
    })
}

/// A recorded historical allowance that is interrupted or edited: setup no longer writes
/// allowances, but it still refuses to change the hooks beside one. A complete allowance, or one
/// whose keys were all removed by hand (nothing of the user's at stake), is no reason to refuse.
fn allowance_blocks_hooks(inspection: &codex_config::AllowanceInspection) -> bool {
    inspection.recorded.as_ref().is_some_and(|manifest| {
        manifest.phase != lib::InstallPhase::Installed
            || !(inspection.present || inspection.removed)
    })
}

/// The installer's check of the historical owned allowance beside the hook file: a recorded
/// allowance that is partial or edited refuses before hooks are inspected.
pub(crate) fn installer_allowance_precheck(env: &SetupEnv) -> Result<(), RunError> {
    let failure = |detail: String| RunError::Io(io::Error::other(detail));
    let paths = codex_paths(env)?;
    let allowance = codex_config::inspect(&paths.config, &paths.config_manifest)
        .map_err(|error| failure(format!("invalid Codex allowance ownership: {error:?}")))?;
    if allowance_blocks_hooks(&allowance) {
        return Err(failure(
            "recorded Codex allowance is partial or edited; preserved".into(),
        ));
    }
    Ok(())
}

/// Codex's trust-key spelling of a hook event (`hooks.state` keys and
/// `hooks/list`): `SessionStart` -> `session_start`.
pub(crate) fn snake(event: &str) -> String {
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
pub(crate) fn codex_trust_json(paths: &CodexPaths, command: Option<&str>) -> Value {
    let Some(command) = command else {
        return json!({"status": "unknown", "note": CODEX_TRUST_NOTE});
    };
    let recorded = lib::read_settings_manifest(&paths.hooks_manifest)
        .ok()
        .flatten();
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
            if recorded.as_ref().map_or_else(
                || {
                    registered == Some(command)
                        || registered == Some(lib::event_command(command, hook.event).as_str())
                },
                |manifest| {
                    manifest
                        .owned
                        .iter()
                        .any(|entry| entry.event == hook.event && &entry.group == group)
                },
            ) {
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
    if settings_status(
        crate::harness::codex::settings_kind(),
        env,
        &paths.hooks,
        &mut status,
    )
    .is_err()
        || status["installed"] != true
    {
        return json!({"status": "unknown", "note": CODEX_TRUST_NOTE});
    }
    codex_trust_json(&paths, status["command"].as_str())
}

pub(crate) fn allowance_error(error: codex_config::AllowanceError, config: &Path) -> RunError {
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

pub(crate) fn allowance_json(inspection: Option<&codex_config::AllowanceInspection>) -> Value {
    let mut report = json!({
        "socket_path": inspection.and_then(|i| i.recorded.as_ref()).map(|m| &m.socket),
        "validation": "unvalidated",
        "omitted": "setup uses approved outside-sandbox commands and installs no sandbox allowance",
        "recorded": inspection.is_some_and(|i| i.recorded.is_some()),
        "present": inspection.is_some_and(|i| i.present),
    });
    // Only a record whose keys were all removed by hand says so; owned unsetup retires it.
    if inspection.is_some_and(|i| i.removed) {
        report["removed_by_hand"] = json!(true);
    }
    report
}

/// Attach the unmeasured-allowance warning to a `sandbox` report.
pub(crate) fn with_unmeasured(mut sandbox: Value, warning: Option<String>) -> Value {
    if let Some(warning) = warning {
        sandbox["unmeasured_installed"] = json!(true);
        sandbox["warning"] = json!(warning);
    }
    sandbox
}

/// The permission component of this CODEX_HOME and state directory.
fn permission_component(env: &SetupEnv) -> Result<CodexPermissions, RunError> {
    let home = codex_root(env).ok_or_else(|| invalid("neither CODEX_HOME nor HOME is set"))?;
    let rules = home.join("rules").join("herdr-threads.rules");
    Ok(CodexPermissions::new(
        home,
        manifest_path(env.state_dir()?, "codex-permissions", &rules),
    ))
}

fn permission_error(error: SetupError, rules: &Path) -> RunError {
    let rules = rules.display();
    match error {
        SetupError::Conflict => api(
            ErrorCode::Conflict,
            format!(
                "{rules} was not written by herdr-threads or was edited since, or changed while \
                 an update was interrupted; review it and remove it (or move it aside) to let \
                 setup manage it. Nothing was changed"
            ),
        ),
        SetupError::Invalid | SetupError::TooLarge => invalid(format!(
            "cannot manage {rules}: CODEX_HOME must be a directory you own that is not group- \
             or world-writable; nothing was changed"
        )),
        SetupError::Io => failed(format!(
            "could not update {rules} (another setup may hold its lock); re-run the command"
        )),
    }
}

/// A rules file reached through a symbolic link to a directory, with no ownership record of its
/// own. Nothing is ever written through the link: the root that holds the real file manages it.
enum SharedRules {
    /// The real file is owned and unedited, recorded at `owner`.
    Owned { owner: PathBuf },
    /// The real file is not installed.
    Missing { canonical: PathBuf },
    /// The real file is foreign, edited or mid-update.
    Unowned { canonical: PathBuf },
}

/// The default-root remedy for a shared rules file.
const SHARED_REMEDY: &str = "set up the root that holds the real file first (default root: `env -u \
     CODEX_HOME herdr-threads setup codex --with-permissions`). The rules file was not changed";

impl SharedRules {
    fn json(&self, rules: &Path) -> Value {
        match self {
            Self::Owned { owner } => json!({
                "state": "shared",
                "rules_file": rules.display().to_string(),
                "managed_at": owner.display().to_string(),
                "note": format!("shared via symlink; managed at {}", owner.display()),
            }),
            Self::Missing { canonical } | Self::Unowned { canonical } => json!({
                "state": "shared_unmanaged",
                "rules_file": rules.display().to_string(),
                "managed_at": canonical.display().to_string(),
                "note": self.advice(rules),
            }),
        }
    }

    fn advice(&self, rules: &Path) -> String {
        match self {
            Self::Owned { owner } => format!("shared via symlink; managed at {}", owner.display()),
            Self::Missing { canonical } => format!(
                "{} is shared through a symlink to {}, which is not installed; {SHARED_REMEDY}",
                rules.display(),
                canonical.display()
            ),
            Self::Unowned { canonical } => format!(
                "{} is shared through a symlink to {}, which herdr-threads does not own \
                 unedited; {SHARED_REMEDY}",
                rules.display(),
                canonical.display()
            ),
        }
    }
}

/// Whether `permissions` is a shared rules file (see [`SharedRules`]); `None` when no directory
/// below the config root on its path is a symbolic link to elsewhere. A record of the lexical path
/// itself (an earlier release wrote through the link) never makes it writable here: like any
/// other root's record, it can only show the real file owned.
fn shared_rules(
    env: &SetupEnv,
    permissions: &CodexPermissions,
) -> Result<Option<SharedRules>, RunError> {
    let home = codex_root(env).ok_or_else(|| invalid("neither CODEX_HOME nor HOME is set"))?;
    let rules = permissions.rules_file();
    let canonical_of = |file: &Path| {
        Some(
            lib::canonical_path(file.parent()?)
                .ok()?
                .join(file.file_name()?),
        )
    };
    let Some(canonical) = lib::shared_through_symlink(home, rules)? else {
        return Ok(
            managed_elsewhere(env, rules, &canonical_of)?.map(|owner| SharedRules::Owned { owner })
        );
    };
    // Any record of the real file installed and unedited owns it, whichever root made it (a
    // linked profile's record, made before its link, resolves to the same file), so no listing
    // order decides. The verdict names the real file, never the lexical path of the record that
    // validated. Otherwise an interrupted or unreadable record leaves it unowned.
    let mut unsettled = false;
    for manifest in lib::manifests_of_kind(env.state_dir()?, "codex-permissions") {
        let Some(file) = CodexPermissions::recorded_file(&manifest) else {
            continue;
        };
        if canonical_of(&file).as_ref() != Some(&canonical) {
            continue;
        }
        match CodexPermissions::at(file, manifest).status() {
            Ok(CodexPermissionState::Installed) => {
                return Ok(Some(SharedRules::Owned { owner: canonical }));
            }
            Ok(CodexPermissionState::Missing | CodexPermissionState::Edited) => {}
            _ => unsettled = true,
        }
    }
    if unsettled {
        return Ok(Some(SharedRules::Unowned { canonical }));
    }
    Ok(Some(
        if fs::symlink_metadata(&canonical)
            .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        {
            SharedRules::Missing { canonical }
        } else {
            SharedRules::Unowned { canonical }
        },
    ))
}

/// A rules file reached without a symbolic link and with no record of its own, that a record
/// made through another root's link (an earlier release recorded the link path) manages, installed
/// and unedited: the recorded path. Never adopted or migrated; this root writes and removes nothing.
fn managed_elsewhere(
    env: &SetupEnv,
    rules: &Path,
    canonical_of: &dyn Fn(&Path) -> Option<PathBuf>,
) -> Result<Option<PathBuf>, RunError> {
    let state = env.state_dir()?;
    if fs::symlink_metadata(manifest_path(state, "codex-permissions", rules)).is_ok()
        || fs::symlink_metadata(rules).is_err()
    {
        return Ok(None);
    }
    let Some(canonical) = canonical_of(rules) else {
        return Ok(None);
    };
    Ok(lib::manifests_of_kind(state, "codex-permissions")
        .into_iter()
        .filter_map(|manifest| {
            let file = CodexPermissions::recorded_file(&manifest)?;
            (file != rules && canonical_of(&file).as_ref() == Some(&canonical))
                .then_some((file, manifest))
        })
        .find(|(file, manifest)| {
            CodexPermissions::at(file.clone(), manifest.clone())
                .status()
                .is_ok_and(|state| state == CodexPermissionState::Installed)
        })
        .map(|(file, _)| file))
}

/// The permission component's report: a shared rules file's when it is one.
fn permission_report(env: &SetupEnv, permissions: &CodexPermissions) -> Value {
    match shared_rules(env, permissions) {
        Ok(Some(shared)) => shared.json(permissions.rules_file()),
        Ok(None) => match permissions.status() {
            Ok(state) => permissions_json(permissions, state),
            Err(_) => json!({"state": "unreadable"}),
        },
        Err(_) => json!({"state": "unreadable"}),
    }
}

fn permissions_json(component: &CodexPermissions, state: CodexPermissionState) -> Value {
    let mut report = json!({
        "state": match state {
            CodexPermissionState::Missing => "not_installed",
            CodexPermissionState::Installed => "installed",
            CodexPermissionState::Edited => "edited",
            CodexPermissionState::Foreign => "foreign",
            CodexPermissionState::Pending => "interrupted",
        },
        "rules_file": component.rules_file().display().to_string(),
    });
    match state {
        CodexPermissionState::Edited | CodexPermissionState::Foreign => {
            report["note"] = json!(
                "herdr-threads leaves this rules file alone: it was edited or not written by \
                 setup"
            );
        }
        CodexPermissionState::Pending => {
            report["note"] = json!(
                "a permission update was interrupted; re-run `herdr-threads setup codex` (or \
                 unsetup) to settle it"
            );
        }
        _ => {}
    }
    report
}

/// The permission component as doctor reports it; best effort.
pub(crate) fn permission_status_json(env: &SetupEnv) -> Value {
    match permission_component(env) {
        Ok(permissions) => permission_report(env, &permissions),
        Err(_) => json!({"state": "unreadable"}),
    }
}

/// The permission grant as the installer's permissions component sees it. A rules file shared
/// through a symlink is reported, never written: owned at its real location it is `Shared`,
/// absent there `SharedMissing`, anything else a failure that never advises removing it.
pub(crate) fn installer_permission_state(
    env: &SetupEnv,
) -> Result<crate::harness::adapter::InstallerPermissionState, RunError> {
    use crate::harness::adapter::InstallerPermissionState as State;
    let permissions = permission_component(env)?;
    let rules = permissions.rules_file();
    match shared_rules(env, &permissions)? {
        Some(shared @ SharedRules::Owned { .. }) => return Ok(State::Shared(shared.advice(rules))),
        Some(shared @ SharedRules::Missing { .. }) => {
            return Ok(State::SharedMissing(shared.advice(rules)));
        }
        Some(shared @ SharedRules::Unowned { .. }) => {
            return Err(api(ErrorCode::Conflict, shared.advice(rules)));
        }
        None => {}
    }
    permissions
        .status()
        .map(|state| {
            if state == CodexPermissionState::Installed {
                State::Granted
            } else {
                State::Missing
            }
        })
        .map_err(|error| permission_error(error, rules))
}

/// Ask on a terminal when setup only advised; a yes (the default) grants.
pub fn settle_permissions<R: io::BufRead + ?Sized, W: io::Write + ?Sized>(
    env: &SetupEnv,
    report: &mut Value,
    input: &mut R,
    out: &mut W,
) -> Result<(), RunError> {
    if report["permissions"]["action"] != "advised" {
        return Ok(());
    }
    write!(
        out,
        "{}",
        crate::harness::claude::setup::permission_question("codex")
    )?;
    out.flush()?;
    let yes = crate::harness::claude::setup::permission_answer(input)?;
    let permissions = permission_component(env)?;
    if yes {
        let state = permissions
            .apply(PermissionConsent::Granted)
            .map_err(|error| permission_error(error, permissions.rules_file()))?;
        report["permissions"] = permissions_json(&permissions, state);
        report["permissions"]["action"] = json!("granted");
    } else {
        report["permissions"]["action"] = json!("declined");
        if let Some(note) = report["permissions"].as_object_mut() {
            note.remove("note");
        }
    }
    Ok(())
}

fn codex_install(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    env.hook_argv(Harness::Codex)?;
    let paths = codex_paths(env)?;
    let (observed, contract) = observe(request, env).map_err(refuse_executable)?;
    let contract = contract.expect("Codex has a registered contract");
    // Validate the declared composition without manufacturing a version witness.
    plan_codex_for_contract(&[], &env.hook_argv(Harness::Codex)?, &contract).map_err(|error| {
        settings_error(
            error,
            request.verb,
            crate::harness::codex::settings_kind(),
            &paths.hooks,
            &paths.hooks_manifest,
        )
    })?;
    let owned_command = codex_owned_command(env)?;
    let layers = observe_codex_config(env, &owned_command);
    let user_codex_home = codex_home_from(None, env.home.clone());
    refuse_duplicate(
        &layers,
        &owned_command,
        &paths.hooks,
        user_codex_home.as_deref(),
    )?;
    // Legacy allowances are left for owned unsetup. Incomplete or edited
    // records still refuse before hooks change, preserving the installer seam;
    // one whose keys were all removed by hand does not.
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
    if allowance_blocks_hooks(&inspection) {
        return Err(allowance_error(
            codex_config::AllowanceError::Setup(SetupError::Conflict),
            &paths.config,
        ));
    }
    prepare_state(env)?;
    let mut warnings = Vec::new();
    let mut hooks_file = OwnedFile::new(paths.hooks.clone(), paths.hooks_manifest.clone(), b"{}");
    let (installed, already, adopted) = install_settings(
        crate::harness::codex::settings_kind(),
        request.verb,
        env,
        &mut hooks_file,
        &mut warnings,
    )?;
    let permissions = permission_component(env)?;
    // After the hooks (which create CODEX_HOME). The rules file is independent of the hooks: a problem with it is reported, never a
    // reason to leave the hooks out.
    let permission_report = match shared_rules(env, &permissions) {
        // Like the rules file's own errors, never a reason to fail after the hooks.
        Err(error) if request.permissions != PermissionPolicy::Grant => json!({
            "state": "error",
            "error": error.to_string(),
        }),
        Err(error) => return Err(error),
        // A shared rules file is never written through its link: reported as it is, and an
        // explicit grant the real file does not hold cannot be honoured here.
        Ok(Some(shared))
            if request.permissions == PermissionPolicy::Grant
                && !matches!(shared, SharedRules::Owned { .. }) =>
        {
            return Err(api(
                ErrorCode::Conflict,
                shared.advice(permissions.rules_file()),
            ));
        }
        Ok(Some(shared)) => shared.json(permissions.rules_file()),
        Ok(None) => {
            let (result, action) = match request.permissions {
                PermissionPolicy::Grant => {
                    (permissions.apply(PermissionConsent::Granted), "granted")
                }
                PermissionPolicy::Decline => (permissions.remove(), "removed"),
                PermissionPolicy::Ask => (permissions.apply(PermissionConsent::Undecided), "kept"),
            };
            match result {
                // An explicit grant that cannot be honoured fails the command; otherwise the
                // rules file never keeps the hooks out.
                Err(error) if request.permissions == PermissionPolicy::Grant => {
                    return Err(permission_error(error, permissions.rules_file()));
                }
                Ok(state) => {
                    let mut report = permissions_json(&permissions, state);
                    report["action"] = json!(if action == "kept"
                        && state == CodexPermissionState::Missing
                    {
                        "advised"
                    } else {
                        action
                    });
                    if report["action"] == "advised" {
                        report["note"] = json!(
                            crate::harness::claude::setup::PERMISSION_ADVICE
                                .replace("<harness>", "codex")
                        );
                    }
                    report
                }
                Err(error) => json!({
                    "state": "error",
                    "error": permission_error(error, permissions.rules_file()).to_string(),
                }),
            }
        }
    };
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
    // Like the rules file, the shell startup files never keep the hooks out.
    let (pane_env, pane_env_warnings) = super::pane_env::install_report(env, request.pane_env);
    warnings.extend(pane_env_warnings);
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
        "commands": lib::commands_json(&installed.owned),
        "events": installed.owned.iter().map(|entry| entry.event.clone()).collect::<Vec<_>>(),
        "sandbox": with_unmeasured(
            allowance_json(inspection.as_ref()),
            unmeasured
        ),
        "trust": codex_trust_json(&paths, command.as_deref()),
        "permissions": permission_report,
        "pane_env": pane_env,
        "harness_version": observation_json::<codex::CodexContract>(&Ok((observed, None))),
        "codex_config": codex_config_json(env, &layers),
        "observed": "unknown",
        "note": "installed is not observed: run `herdr-threads doctor` for native evidence. \
                 Codex sessions with another CODEX_HOME, or launched with \
                 --ignore-user-config, do not load these hooks",
        "warnings": warnings,
    }))
}

pub(crate) fn codex_remove(env: &SetupEnv) -> Result<Value, RunError> {
    let paths = codex_paths(env)?;
    let kind = crate::harness::codex::settings_kind();
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
    // Never let the permission component keep the hooks installed: its failure is reported.
    let permissions = permission_component(env)?;
    report["permissions"] = match shared_rules(env, &permissions) {
        Err(error) => json!({"state": "error", "error": error.to_string()}),
        // Shared through a symlink: removed only at the root that holds the real file.
        Ok(Some(shared)) => shared.json(permissions.rules_file()),
        Ok(None) => match permissions.remove() {
            Ok(state) => permissions_json(&permissions, state),
            Err(error) => json!({
                "state": "error",
                "error": permission_error(error, permissions.rules_file()).to_string(),
            }),
        },
    };
    // Independent of the hooks too: removed even when no hook installation is recorded.
    report["pane_env"] = super::pane_env::remove_report(env);
    let recorded =
        recorded_for_removal(kind, env, &paths.hooks, &paths.hooks_manifest, &mut report)?;
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
                "removing only the hook groups with adopted installation identity {} from {}; the setup \
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

fn codex_status(
    env: &SetupEnv,
    observation: Result<(Observed, Option<crate::harness::codex::CodexContract>), String>,
) -> Result<Value, RunError> {
    let mut report = json!({
        "action": "status",
        "harness": "codex",
        "scope": "user",
        "instance": instance_json(env),
        "recipes": recipe::describe(codex::RECIPES),
        "harness_version": observation_json(&observation),
    });
    let hooks = codex_file(env, "hooks.json")?;
    report["hooks_file"] = json!(hooks.display().to_string());
    settings_status(
        crate::harness::codex::settings_kind(),
        env,
        &hooks,
        &mut report,
    )?;
    let Ok(paths) = codex_paths(env) else {
        return Ok(report);
    };
    let inspection = codex_config::inspect(&paths.config, &paths.config_manifest).ok();
    let unmeasured = codex_unmeasured_allowance_warning(env, None);
    report["config_file"] = json!(paths.config.display().to_string());
    report["sandbox"] = with_unmeasured(allowance_json(inspection.as_ref()), unmeasured.clone());
    let doubled = env
        .hook_argv(Harness::Codex)
        .ok()
        .and_then(|argv| {
            let bytes = fs::read(&paths.hooks).ok()?;
            lib::doubled_events(crate::harness::codex::settings_kind(), &bytes, &argv).ok()
        })
        .unwrap_or_default();
    let warnings: Vec<String> = unmeasured
        .into_iter()
        .chain(codex_foreign_proxy_warnings(env))
        .chain((!doubled.is_empty()).then(|| lib::doubled_warning(&paths.hooks, &doubled)))
        .collect();
    if !warnings.is_empty() {
        report["warnings"] = json!(warnings);
    }
    report["trust"] = codex_trust_json(&paths, report["command"].as_str());
    if let Ok(permissions) = permission_component(env) {
        report["permissions"] = permission_report(env, &permissions);
    }
    report["pane_env"] = super::pane_env::status_report(env);
    if let Ok(owned_command) = codex_owned_command(env) {
        let layers = observe_codex_config(env, &owned_command);
        report["duplicate_hook"] = json!(
            !doubled.is_empty()
                || layers
                    .iter()
                    .any(|layer| layer.has_owned_command && layer.path != paths.hooks)
        );
        report["codex_config"] = codex_config_json(env, &layers);
    }
    Ok(report)
}

pub(crate) fn setup(
    request: &crate::harness::adapter::SetupRequest,
) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
    legacy_adapter_setup(
        Harness::Codex,
        request,
        codex_install,
        crate::cli::setup::PromptSuggestionPolicy::Ask,
        false,
    )
}
pub(crate) fn status(
    request: &crate::harness::adapter::StatusRequest,
    budget: &crate::protocol::time::CallBudget,
) -> crate::harness::adapter::SetupStatus {
    legacy_adapter_status(Harness::Codex, request, |legacy, env| {
        let timeout = Duration::from_millis(
            budget
                .deadline
                .0
                .saturating_sub(request.environment.clock.monotonic_now().0),
        )
        .min(Duration::from_secs(5));
        codex_status(
            env,
            observe_bounded(legacy, env, None, timeout, &budget.cancellation),
        )
    })
}
pub(crate) fn unsetup(
    request: &crate::harness::adapter::UnsetupRequest,
) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure> {
    legacy_adapter_unsetup(Harness::Codex, request, codex_remove)
}

pub(crate) fn observe(
    request: &SetupRequest,
    env: &SetupEnv,
) -> Result<(Observed, Option<crate::harness::codex::CodexContract>), String> {
    observe_with_cache(request, env, None)
}
pub(crate) fn observe_with_cache(
    request: &SetupRequest,
    env: &SetupEnv,
    codex_cache: Option<&Path>,
) -> Result<(Observed, Option<crate::harness::codex::CodexContract>), String> {
    observe_bounded(
        request,
        env,
        codex_cache,
        Duration::from_secs(5),
        &crate::protocol::time::Cancellation::default(),
    )
}

fn observe_bounded(
    request: &SetupRequest,
    env: &SetupEnv,
    _codex_cache: Option<&Path>,
    timeout: Duration,
    cancellation: &crate::protocol::time::Cancellation,
) -> Result<(Observed, Option<crate::harness::codex::CodexContract>), String> {
    if timeout.is_zero() || cancellation.is_cancelled() {
        return Err("native status observation budget exhausted or cancelled".into());
    }
    let binary = harness_binary(request, env)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no executable `codex` on PATH; pass --harness-binary".to_owned())?;
    use std::os::unix::fs::PermissionsExt;
    if !std::fs::metadata(&binary)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    {
        return Err(format!("{} is not an executable file", binary.display()));
    }
    Ok((
        Observed {
            binary,
            version: None,
            recipe: "codex-hooks-v1",
        },
        Some(crate::harness::codex::CodexContract::registered()),
    ))
}

// ------------------------------------------------------- session plan

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSetupPlan {
    pub base_fingerprint: String,
    pub events: Vec<EventGroups>,
    /// The owned session overrides for a launch with no caller arguments.
    pub launch_argv: Vec<String>,
    /// The session `-c hooks.*=...` overrides alone.
    pub session_config: Vec<String>,
    pub owned: Vec<OwnedEntry>,
}

impl CodexSetupPlan {
    /// Compose owned overrides with caller arguments using the launch rule.
    pub fn launch_argv_for(&self, caller: Vec<String>) -> Result<Vec<String>, SetupError> {
        compose_native_argv(
            crate::protocol::authority::Harness::Codex,
            caller,
            self.session_config.clone(),
        )
        .map_err(|_| SetupError::Invalid)
    }
}

/// `existing` is the session-flags layer the printed launch line owns (the
/// public CLI passes it empty): Codex discovers the hooks of config.toml,
/// hooks.json and trusted project layers separately and runs them alongside
/// these `-c` values (hooks/list on 0.157.1, 0.158.0 and 0.159.2), so copying
/// them in would register them twice. Each session override includes the
/// supplied existing groups. hooks.state is untouched and must be measured separately.
/// Internal composition shared by contract planning and historical diagnostics.
fn plan_codex(
    existing: &[EventGroups],
    hook_argv: &[String],
) -> Result<CodexSetupPlan, SetupError> {
    let command = shell_command(hook_argv)?;
    let native = lib::native_entries(codex::settings_kind(), hook_argv)?;
    if existing.len() > 64 {
        return Err(SetupError::TooLarge);
    }
    let mut events = existing.to_vec();
    let mut seen = std::collections::BTreeSet::new();
    for event in &events {
        if !valid_event(&event.event)
            || !seen.insert(event.event.clone())
            || event.groups.len() > 128
        {
            return Err(SetupError::Invalid);
        }
    }
    let mut owned_entries = Vec::new();
    // Exactly the adapter's declared context-only hook groups. The Bash
    // PreToolUse group carries tool-boundary attention context; the gated
    // invocation rewrite is never part of any installed group.
    for hook in codex::DECLARATION.owned_hooks {
        let name = hook.event;
        let mut group = json!({"hooks":[{"type":"command","command":native.as_ref().and_then(|entries| entries.iter().find(|e| e.event == name)).and_then(|entry| entry.group["hooks"][0]["command"].as_str()).map(str::to_owned).unwrap_or_else(|| event_command(&command, name))}]});
        if let Some(matcher) = hook.matcher {
            group["matcher"] = json!(matcher);
        }
        let entry = owned(name, group)?;
        if let Some(event) = events.iter_mut().find(|event| event.event == name) {
            merge(&mut event.groups, &entry)?;
        } else {
            events.push(EventGroups {
                event: name.into(),
                groups: vec![entry.group.clone()],
            });
        }
        owned_entries.push(entry);
    }
    let mut session_config: Vec<String> = Vec::new();
    for entry in &owned_entries {
        let event = events
            .iter()
            .find(|e| e.event == entry.event)
            .ok_or(SetupError::Invalid)?;
        session_config.push("-c".into());
        session_config.push(format!(
            "hooks.{}={}",
            event.event,
            toml_inline(&json!(event.groups))?
        ));
    }
    if session_config.iter().map(String::len).sum::<usize>() > 65536 {
        return Err(SetupError::TooLarge);
    }
    let mut plan = CodexSetupPlan {
        base_fingerprint: groups_fingerprint(existing)?,
        events,
        launch_argv: Vec::new(),
        session_config,
        owned: owned_entries,
    };
    plan.launch_argv = plan.launch_argv_for(Vec::new())?;
    Ok(plan)
}

/// Compose the registered Codex setup contract without executable metadata.
/// The handle declares registration, never runtime support or native delivery.
pub fn plan_codex_for_contract(
    existing: &[EventGroups],
    hook_argv: &[String],
    _: &codex::CodexContract,
) -> Result<CodexSetupPlan, SetupError> {
    plan_codex(existing, hook_argv)
}

/// Diagnostic/fixture compatibility for historical installed-version callers.
pub fn plan_codex_for_version(
    existing: &[EventGroups],
    hook_argv: &[String],
    version: &codex::InstalledVersion,
) -> Result<CodexSetupPlan, SetupError> {
    if !codex::DECLARATION.recipes.contains(version.recipe()) {
        return Err(SetupError::Invalid);
    }
    plan_codex(existing, hook_argv)
}
fn valid_event(s: &str) -> bool {
    !s.is_empty() && s.len() < 64 && s.chars().all(|c| c.is_ascii_alphanumeric())
}
/// Serialize the bounded JSON-compatible hook schema into TOML inline values; null is rejected.
fn toml_inline(v: &Value) -> Result<String, SetupError> {
    match v {
        Value::String(s) => serde_json::to_string(s).map_err(|_| SetupError::Invalid),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Array(items) => {
            let values: Result<Vec<_>, _> = items.iter().map(toml_inline).collect();
            Ok(format!("[{}]", values?.join(",")))
        }
        Value::Object(map) => {
            let values: Result<Vec<_>, _> = map
                .iter()
                .map(|(k, v)| {
                    Ok(format!(
                        "{}={}",
                        serde_json::to_string(k).map_err(|_| SetupError::Invalid)?,
                        toml_inline(v)?
                    ))
                })
                .collect();
            Ok(format!("{{{}}}", values?.join(",")))
        }
        Value::Null => Err(SetupError::Invalid),
    }
}

/// Typed Codex counterpart of exact-entry uninstall. Missing/edited entries are conservatively refused.
pub fn uninstall_codex(
    existing: &[EventGroups],
    entries: &[OwnedEntry],
) -> Result<Vec<EventGroups>, SetupError> {
    let mut root = serde_json::Map::new();
    let mut seen = std::collections::BTreeSet::new();
    for event in existing {
        if !valid_event(&event.event) || !seen.insert(event.event.clone()) {
            return Err(SetupError::Invalid);
        }
        root.insert(event.event.clone(), json!(event.groups));
    }
    let bytes = serde_json::to_vec(&json!({"hooks":root})).map_err(|_| SetupError::Invalid)?;
    let removed: Value = serde_json::from_slice(&uninstall_json(&bytes, entries)?)
        .map_err(|_| SetupError::Invalid)?;
    existing
        .iter()
        .map(|event| {
            Ok(EventGroups {
                event: event.event.clone(),
                groups: removed["hooks"][&event.event]
                    .as_array()
                    .ok_or(SetupError::Invalid)?
                    .clone(),
            })
        })
        .collect()
}

/// Codex hooks are session `-c` arguments; this records ownership without a global write.
pub fn codex_session_manifest(plan: &CodexSetupPlan) -> Result<OwnershipManifest, SetupError> {
    Ok(OwnershipManifest {
        version: 1,
        harness: "codex".into(),
        scope: "session".into(),
        path: "codex:-c".into(),
        original_bytes: Vec::new(),
        installation_base_bytes: Vec::new(),
        installation_id: String::new(),
        owned: plan.owned.clone(),
        installed_fingerprint: groups_fingerprint(&plan.events)?,
        phase: InstallPhase::Installed,
        superseded: Vec::new(),
        permission: None,
        superseded_permission: None,
        adopted: false,
    })
}

pub fn inspect_codex_session(
    events: &[EventGroups],
    manifest: &OwnershipManifest,
    observed: NativeObservation,
) -> Result<HookInspection, SetupError> {
    if manifest.version != 1
        || manifest.harness != "codex"
        || manifest.scope != "session"
        || manifest.path != "codex:-c"
    {
        return Err(SetupError::Invalid);
    }
    let owned_hooks = codex::DECLARATION.owned_hooks;
    let declaration_valid = manifest.owned.len() == owned_hooks.len()
        && owned_hooks
            .iter()
            .zip(&manifest.owned)
            .all(|(hook, entry)| {
                entry.event == hook.event
                    && entry.group["hooks"]
                        .as_array()
                        .is_some_and(|hooks| hooks.len() == 1)
                    && entry.group["hooks"][0]["type"] == "command"
                    && entry.group["hooks"][0]["command"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    && entry.group.get("matcher").and_then(Value::as_str) == hook.matcher
                    && entry.group.get("matcher").is_none_or(Value::is_string)
            });
    let installed = declaration_valid
        && manifest.owned.iter().all(|entry| {
            events
                .iter()
                .find(|event| event.event == entry.event)
                .is_some_and(|event| event.groups.contains(&entry.group))
                && owned(&entry.event, entry.group.clone())
                    .is_ok_and(|actual| actual.fingerprint == entry.fingerprint)
        });
    let current_fingerprint = if installed {
        Some(groups_fingerprint(events)?)
    } else {
        None
    };
    Ok(HookInspection {
        installed,
        observed,
        configured_hook: current_fingerprint.map(|fingerprint| ConfiguredHook {
            scope: "session".into(),
            path: "codex:-c".into(),
            fingerprint,
        }),
        allow_rule: None,
        adopted: None,
        legacy_event_registration: false,
        missing_required_events: Vec::new(),
    })
}
/// Check the typed reader's exact structural input before future session overrides are published.
pub fn verify_codex_base(plan: &CodexSetupPlan, current: &[EventGroups]) -> Result<(), SetupError> {
    if groups_fingerprint(current)? == plan.base_fingerprint {
        Ok(())
    } else {
        Err(SetupError::Conflict)
    }
}
