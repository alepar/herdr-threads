//! agy `setup` / `setup-status` / `unsetup` (D7): the owned plugin directory plus the owned
//! `permissions.allow` member `command(herdr-threads)` in `antigravity-cli/settings.json`, which
//! the shared JSON engine manages in its allow-member-only mode.
use super::{paths::AgyRoots, plugin};
use crate::{
    cli::setup::SetupEnv,
    harness::{
        adapter::{
            DiagnosticSeverity, LocalRepair, LocalSetupStatus, RemovalOutcome, ResolvedSetupScope,
            SetupAction, SetupDiagnostic, SetupEnvironment, SetupFailure, SetupOutcome,
            SetupRequest, SetupStatus, StatusRequest, UnsetupRequest,
        },
        setup::{
            NativeObservation, OwnedAllowSpec, SettingsKind, SettingsKindSpec, SetupError,
            inspect_user_settings, install_user_settings, legacy, read_settings_manifest,
            remove_user_settings,
        },
    },
    protocol::results::{ApiError, ErrorCode},
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

/// The owned `permissions.allow` member.
pub const ALLOW_RULE: &str = "command(herdr-threads)";
/// The consent question an interactive `setup agy` asks when nothing is granted.
pub const AGY_PERMISSION_QUESTION: &str = "Let agy agents run herdr-threads commands without prompting? \
    The allow entry command(herdr-threads) also covers person and setup commands [Y/n] ";
/// The non-interactive advice when nothing is granted (nothing was changed).
pub const AGY_PERMISSION_ADVICE: &str = "agy asks before each herdr-threads command: re-run \
    `herdr-threads setup agy --with-permissions` to add the allow entry command(herdr-threads) (it covers every \
    herdr-threads command, person and setup commands included), or --without-permissions to keep it this way";
const SETTINGS_MANIFEST_KIND: &str = "agy-user";

/// agy's user settings file in the shared engine: allow-member-only, and agy rewrites the file.
pub static SETTINGS_KIND: SettingsKindSpec = SettingsKindSpec {
    harness: "agy",
    file_name: "settings.json",
    hook_groups: |_| Vec::new(),
    owned_allow: Some(OwnedAllowSpec {
        rule: ALLOW_RULE,
        retired: &[],
        native_rewrites: true,
    }),
    adoption_note: "",
    legacy_upgrade_warning: "",
    permission_rules: crate::harness::setup::no_permission_rules,
};
pub const fn settings_kind() -> SettingsKind {
    SettingsKind::new(&SETTINGS_KIND)
}

/// Who holds the allow member now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowEntryState {
    /// Setup added it; unsetup removes it.
    Owned,
    /// The user's file held it before setup; unsetup leaves it.
    PreExisting,
    /// A recorded grant whose member agy rewrote away, or a file that cannot be inspected.
    Missing,
    /// No record and no member: consent was not given.
    NotGranted,
}
impl AllowEntryState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::PreExisting => "pre_existing",
            Self::Missing => "missing",
            Self::NotGranted => "not_granted",
        }
    }
}

fn roots_of(scope: &ResolvedSetupScope) -> Result<AgyRoots, SetupFailure> {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => Ok(AgyRoots::from_config_root(root.clone())),
        ResolvedSetupScope::Profile { .. } => Err(SetupFailure::Invalid(
            "agy: named profile is unsupported".into(),
        )),
    }
}

fn state_dir(env: &SetupEnvironment) -> Result<&Path, SetupFailure> {
    env.state_dir
        .as_deref()
        .ok_or_else(|| SetupFailure::Invalid("agy: state directory unknown".into()))
}

fn settings_manifest(roots: &AgyRoots, env: &SetupEnvironment) -> Result<PathBuf, SetupFailure> {
    Ok(legacy::manifest_path(
        state_dir(env)?,
        SETTINGS_MANIFEST_KIND,
        &roots.settings,
    ))
}

fn settings_failure(error: SetupError, roots: &AgyRoots, manifest: &Path) -> SetupFailure {
    let file = roots.settings.display();
    match error {
        SetupError::Conflict => SetupFailure::Api(ApiError::new(
            ErrorCode::Conflict,
            format!(
                "{file} changed during setup, or its ownership record ({}) does not match it; \
                 nothing was changed. Run `herdr-threads unsetup agy` and set up again",
                manifest.display()
            ),
        )),
        SetupError::Invalid => SetupFailure::Invalid(format!(
            "agy: {file} is not a JSON object with an array `permissions.allow` (or it or its \
             ownership record is a symlink or damaged); nothing was changed"
        )),
        SetupError::TooLarge => SetupFailure::Invalid(format!(
            "agy: {file} or its ownership record exceeds the setup size bound; nothing was changed"
        )),
        SetupError::Io => SetupFailure::Io(std::io::Error::other(format!(
            "agy: could not read or replace {file} or its ownership record"
        ))),
    }
}

/// The allow member's state, from the recorded ownership and the file as it is now.
pub fn allow_entry_state(roots: &AgyRoots, env: &SetupEnvironment) -> AllowEntryState {
    inspect_allow(roots, env).map_or(AllowEntryState::Missing, |(state, _)| state)
}

/// [`allow_entry_state`], and why the file could not be inspected when it could not.
fn inspect_allow(
    roots: &AgyRoots,
    env: &SetupEnvironment,
) -> Result<(AllowEntryState, Option<String>), SetupFailure> {
    use crate::harness::setup::AllowRuleOwnership;
    let manifest = settings_manifest(roots, env)?;
    match read_settings_manifest(&manifest) {
        Ok(None) => {
            return Ok((
                if rule_in_file(&roots.settings) {
                    AllowEntryState::PreExisting
                } else {
                    AllowEntryState::NotGranted
                },
                None,
            ));
        }
        Ok(Some(_)) => {}
        Err(error) => {
            return Ok((
                AllowEntryState::Missing,
                Some(format!(
                    "{} ownership record cannot be inspected ({error:?})",
                    roots.settings.display()
                )),
            ));
        }
    }
    let inspection = match inspect_user_settings(
        settings_kind(),
        &roots.settings,
        &manifest,
        NativeObservation::Unknown,
    ) {
        Ok(inspection) => inspection,
        // A recorded member whose file is gone (or unreadable) is missing, with the reason.
        Err(error) => {
            return Ok((
                AllowEntryState::Missing,
                Some(format!(
                    "{} cannot be inspected ({error:?})",
                    roots.settings.display()
                )),
            ));
        }
    };
    let state = match inspection.allow_rule {
        Some(rule) if inspection.installed && rule.present => match rule.ownership {
            AllowRuleOwnership::PreExisting => AllowEntryState::PreExisting,
            AllowRuleOwnership::Owned => AllowEntryState::Owned,
            _ => AllowEntryState::Missing,
        },
        _ => AllowEntryState::Missing,
    };
    Ok((state, None))
}

/// Whether the user's own `settings.json` holds the allow member; any error reads as no.
fn rule_in_file(path: &Path) -> bool {
    let Ok(bytes) = crate::harness::setup::config_bytes(path) else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes).is_ok_and(|value| {
        value["permissions"]["allow"]
            .as_array()
            .is_some_and(|allow| allow.iter().any(|rule| rule == ALLOW_RULE))
    })
}

/// Add the owned allow member, creating `settings.json` as `{}` when absent.
fn install_allow(roots: &AgyRoots, env: &SetupEnvironment) -> Result<(), SetupFailure> {
    let kind = settings_kind();
    let file = &roots.settings;
    let manifest = settings_manifest(roots, env)?;
    let map = |error| settings_failure(error, roots, &manifest);
    let recorded = read_settings_manifest(&manifest).map_err(map)?;
    let mut created = legacy::OwnedFile::new(file.clone(), manifest.clone(), b"{}");
    if recorded.is_none() {
        if let Some(grandparent) = file.parent().and_then(Path::parent) {
            std::fs::create_dir_all(grandparent).map_err(SetupFailure::Io)?;
        }
        created.prepare().map_err(legacy::adapter_failure)?;
    }
    let base = match &recorded {
        Some(recorded) => recorded.original_bytes.clone(),
        None => std::fs::read(file).map_err(SetupFailure::Io)?,
    };
    match install_user_settings(kind, file, &manifest, &[], &base) {
        Ok(_) => {
            let mut warnings = Vec::new();
            created.record(&mut warnings);
            Ok(())
        }
        Err(error) => {
            created.undo();
            Err(map(error))
        }
    }
}

/// Remove only the owned member (a pre-existing one stays), then the file when setup created it
/// and it is back to its created bytes.
fn remove_allow(roots: &AgyRoots, env: &SetupEnvironment) -> Result<bool, SetupFailure> {
    let manifest = settings_manifest(roots, env)?;
    let map = |error| settings_failure(error, roots, &manifest);
    if read_settings_manifest(&manifest).map_err(map)?.is_none() {
        return Ok(false);
    }
    if !roots.settings.exists() {
        // agy removed the whole file: nothing of ours is left to take out.
        std::fs::remove_file(&manifest).map_err(SetupFailure::Io)?;
        return Ok(true);
    }
    remove_user_settings(settings_kind(), &roots.settings, &manifest).map_err(map)?;
    legacy::delete_created(&roots.settings, &manifest, b"{}");
    Ok(true)
}

/// The executable observation the bare commands' summary line reads.
fn harness_version(native_binary: Option<&Path>, env: &SetupEnvironment) -> Value {
    let binary = native_binary
        .map(Path::to_path_buf)
        .or_else(|| crate::cli::hook::resolve_on_path("agy", env.path.as_deref()));
    match binary {
        Some(binary) => json!({
            "admission": "contract_declared",
            "binary": binary.display().to_string(),
            "version": Value::Null,
            "recipe": super::RECIPE,
        }),
        None => json!({
            "admission": "unavailable",
            "version": Value::Null,
            "refusal": "installed agy executable not found on PATH",
        }),
    }
}

fn plugin_json(status: &plugin::PluginStatus) -> Value {
    json!({
        "path": status.path.display().to_string(),
        "installed": status.installed,
        "current": status.current,
        "enabled": status.enabled,
        "version": status.version,
    })
}
fn allow_json(roots: &AgyRoots, state: AllowEntryState) -> Value {
    json!({
        "file": roots.settings.display().to_string(),
        "rule": ALLOW_RULE,
        "state": state.as_str(),
    })
}

fn permissions_json(roots: &AgyRoots, env: &SetupEnvironment, state: AllowEntryState) -> Value {
    json!({
        "state": if matches!(state, AllowEntryState::Owned | AllowEntryState::PreExisting) {
            "installed"
        } else {
            "not_installed"
        },
        "rule": ALLOW_RULE,
        "file": roots.settings.display().to_string(),
        "pre_existing": state == AllowEntryState::PreExisting,
        "manifest": settings_manifest(roots, env).ok().map(|path| path.display().to_string()),
    })
}

fn status_diagnostics(
    plugin: &plugin::PluginStatus,
    allow: AllowEntryState,
    allow_note: Option<String>,
) -> Vec<SetupDiagnostic> {
    let mut diagnostics = plugin.diagnostics.clone();
    if allow == AllowEntryState::Missing {
        let detail = allow_note
            .map(|note| format!(" ({note})"))
            .unwrap_or_default();
        diagnostics.push(SetupDiagnostic::new(
            "agy_allow_entry_missing",
            DiagnosticSeverity::Warning,
            &format!(
                "the allow entry {ALLOW_RULE} is missing from antigravity-cli/settings.json{detail}; \
                 re-run `herdr-threads setup agy`"
            ),
        ));
    }
    diagnostics
}

/// `Ask`: keep a recorded grant current, record the user's own member; `false` when nothing is
/// granted (nothing written).
fn ask_allow(roots: &AgyRoots, env: &SetupEnvironment) -> Result<bool, SetupFailure> {
    let manifest = settings_manifest(roots, env)?;
    let recorded = read_settings_manifest(&manifest)
        .map_err(|error| settings_failure(error, roots, &manifest))?
        .is_some();
    if recorded || rule_in_file(&roots.settings) {
        install_allow(roots, env)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Ask on a terminal when setup only advised; a yes (the default) grants.
pub(super) fn settle_permissions(
    scope: &ResolvedSetupScope,
    env: &SetupEnvironment,
    report: &mut Value,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<(), SetupFailure> {
    if report["permissions"]["action"] != "advised" {
        return Ok(());
    }
    let roots = roots_of(scope)?;
    write!(writer, "{AGY_PERMISSION_QUESTION}").map_err(SetupFailure::Io)?;
    writer.flush().map_err(SetupFailure::Io)?;
    let yes = crate::harness::claude::setup::permission_answer(reader).map_err(SetupFailure::Io)?;
    if yes {
        install_allow(&roots, env)?;
        let state = allow_entry_state(&roots, env);
        report["permissions"] = permissions_json(&roots, env, state);
        report["permissions"]["action"] = json!("granted");
        report["allow_entry"] = allow_json(&roots, state);
    } else {
        report["permissions"]["action"] = json!("declined");
        if let Some(permissions) = report["permissions"].as_object_mut() {
            permissions.remove("note");
        }
    }
    Ok(())
}

pub(super) fn setup(request: &SetupRequest) -> Result<SetupOutcome, SetupFailure> {
    let roots = roots_of(&request.scope)?;
    let mut environment = request.environment.clone();
    environment.executable = request.executable.clone();
    let env = &environment;
    state_dir(env)?;
    super::delivery::clear_downgrade(env);
    legacy::prepare_state(&SetupEnv::from_snapshot(env)).map_err(legacy::adapter_failure)?;
    let policy = crate::cli::setup::PermissionPolicy::from_options(&request.options);
    let before = allow_entry_state(&roots, env);
    let report = plugin::install(&roots, env)?;
    // The allow member stays owned by the shared settings engine: its write is already a
    // compare-and-swap against the validated bytes, so there is no permission backend or
    // write guard here (a lock file would be left in `~/.gemini` that unsetup never removes).
    let (action, note, failed) = match policy {
        crate::cli::setup::PermissionPolicy::Grant => {
            install_allow(&roots, env)?;
            ("granted", false, None)
        }
        crate::cli::setup::PermissionPolicy::Decline => (
            "removed",
            false,
            remove_allow(&roots, env)
                .err()
                .map(|error| error.to_string()),
        ),
        crate::cli::setup::PermissionPolicy::Ask => match ask_allow(&roots, env) {
            Ok(true) => ("kept", false, None),
            Ok(false) => ("advised", true, None),
            Err(error) => ("kept", false, Some(error.to_string())),
        },
    };
    let plugin = plugin::status(&roots, env);
    let allow = allow_entry_state(&roots, env);
    let changed = report.changed() || before != allow;
    let mut permissions = permissions_json(&roots, env, allow);
    permissions["action"] = json!(action);
    if note {
        permissions["note"] = json!(AGY_PERMISSION_ADVICE);
    }
    if let Some(error) = failed {
        permissions["state"] = json!("error");
        permissions["error"] = json!(error);
    }
    let mut projection = json!({
        "action": if changed { "installed" } else { "already_installed" },
        "installed": plugin.installed && plugin.current,
        "harness_version": harness_version(request.native_binary.as_deref(), env),
        "hooks_file": roots.plugin_dir.join(plugin::HOOKS_JSON).display().to_string(),
        "plugin": plugin_json(&plugin),
        "allow_entry": allow_json(&roots, allow),
        "permissions": permissions,
        "files": report.files.iter().map(|(rel, action)| json!({"path": rel, "action": action.as_str()})).collect::<Vec<_>>(),
    });
    let diagnostics = status_diagnostics(&plugin, allow, None);
    projection["warnings"] = json!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.text.clone())
            .collect::<Vec<_>>()
    );
    Ok(SetupOutcome {
        actions: vec![if changed {
            SetupAction::InstalledOwned
        } else {
            SetupAction::Unchanged
        }],
        diagnostic: String::new(),
        projection,
        diagnostics,
    })
}

pub(super) fn status(request: &StatusRequest) -> SetupStatus {
    let roots = match roots_of(&request.scope) {
        Ok(roots) => roots,
        Err(error) => return SetupStatus::Failed(error),
    };
    let env = &request.environment;
    let plugin = plugin::status(&roots, env);
    let (allow, allow_note) =
        inspect_allow(&roots, env).unwrap_or((AllowEntryState::Missing, None));
    let installed = plugin.installed && plugin.current;
    let mut projection = json!({
        "installed": installed,
        "harness_version": harness_version(request.native_binary.as_deref(), env),
        "hooks_file": roots.plugin_dir.join(plugin::HOOKS_JSON).display().to_string(),
        "plugin": plugin_json(&plugin),
        "allow_entry": allow_json(&roots, allow),
        "permissions": permissions_json(&roots, env, allow),
    });
    if env.state_dir.is_none() {
        projection["error"] =
            json!("state directory unknown: use --state-dir or HERDR_PLUGIN_STATE_DIR");
    }
    let needs_repair = !installed || allow == AllowEntryState::Missing;
    SetupStatus::Detailed(Box::new(LocalSetupStatus {
        scope: request.scope.clone(),
        installed,
        enabled: plugin.enabled,
        admitted: None,
        observed: None,
        fingerprint: plugin.hook.as_ref().map(|hook| hook.fingerprint.clone()),
        configured_hook: plugin.hook.clone(),
        diagnostics: status_diagnostics(&plugin, allow, allow_note),
        repairs: if needs_repair {
            vec![LocalRepair::RepairOwned]
        } else {
            Vec::new()
        },
        projection,
    }))
}

pub(super) fn unsetup(request: &UnsetupRequest) -> Result<RemovalOutcome, SetupFailure> {
    let roots = roots_of(&request.scope)?;
    let env = &request.environment;
    state_dir(env)?;
    let plugin = plugin::remove(&roots, env)?;
    let allow_removed = remove_allow(&roots, env)?;
    let changed = !plugin.removed.is_empty() || allow_removed;
    let incomplete = !plugin.residue.is_empty();
    let projection = json!({
        "action": if changed { "removed" } else { "not_installed" },
        "removed": plugin.removed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "residue": plugin.residue.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "allow_entry_removed": allow_removed,
        "incomplete": incomplete,
    });
    let diagnostics = plugin
        .residue
        .iter()
        .map(|path| {
            SetupDiagnostic::new(
                "agy_plugin_residue",
                DiagnosticSeverity::Warning,
                &format!("{} was edited and was left in place", path.display()),
            )
        })
        .collect();
    Ok(RemovalOutcome {
        actions: vec![if changed {
            SetupAction::RemovedOwned
        } else {
            SetupAction::Unchanged
        }],
        diagnostic: if incomplete {
            "owned removal incomplete; edited files were left in place".into()
        } else {
            String::new()
        },
        residue: plugin.residue,
        projection,
        diagnostics,
    })
}

/// `setup-status` only: where the ownership records live.
pub(super) fn setup_status_extras(request: &StatusRequest, projection: &mut Value) {
    let Ok(roots) = roots_of(&request.scope) else {
        return;
    };
    let env = &request.environment;
    projection["manifests"] = json!({
        "plugin": plugin::manifest_file(&roots, env).ok().map(|p| p.display().to_string()),
        "allow_entry": settings_manifest(&roots, env).ok().map(|p| p.display().to_string()),
    });
}

pub(super) fn setup_report_lines(report: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(path) = report["plugin"]["path"].as_str() {
        lines.push(format!("plugin {path}"));
    }
    if let Some(state) = report["allow_entry"]["state"].as_str() {
        let rule = report["allow_entry"]["rule"].as_str().unwrap_or(ALLOW_RULE);
        lines.push(format!("allow entry {rule}: {}", state.replace('_', "-")));
    }
    if let Some(action) = report["permissions"]["action"].as_str() {
        lines.push(format!("permissions: {action}"));
    }
    lines
}

#[cfg(test)]
#[path = "../../../tests/harness/agy_setup.rs"]
mod tests;
