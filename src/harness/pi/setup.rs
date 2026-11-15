//! Digest-owned extension and shared skill. Settings and permission configuration are untouched.
use crate::harness::{
    adapter::*,
    owned_files::{self, FileSet, Rendered},
    setup::legacy,
};
use crate::ports::ConfiguredHook;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub const EXTENSION_PATH: &str = "extensions/herdr-threads/index.js";
pub const SKILL_PATH: &str = "skills/herdr-threads/SKILL.md";
pub const SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/integrations/pi/extension/herdr-threads.js"
));
const LAUNCH_MARKER: &str = "const LAUNCH = null // herdr-threads:launch";
const MANIFEST_KIND: &str = "pi";
fn invalid(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Invalid(format!("pi: {}", detail.into()))
}
fn state_dir(env: &SetupEnvironment) -> Result<&Path, SetupFailure> {
    env.state_dir
        .as_deref()
        .ok_or_else(|| invalid("state directory unknown"))
}
pub(super) fn root_of(scope: &ResolvedSetupScope) -> Result<&Path, SetupFailure> {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => Ok(root),
        _ => Err(invalid("expected an agent directory")),
    }
}
pub fn default_config_root(env: &SetupEnvironment) -> Option<PathBuf> {
    if let Some(value) = env
        .declared
        .get("PI_CODING_AGENT_DIR")
        .filter(|v| !v.is_empty())
    {
        let path = PathBuf::from(value);
        if let Ok(tail) = path.strip_prefix("~") {
            return env.home.as_ref().map(|home| PathBuf::from(home).join(tail));
        }
        return Some(if path.is_absolute() {
            path
        } else {
            env.cwd.join(path)
        });
    }
    env.home
        .as_ref()
        .map(|home| PathBuf::from(home).join(".pi/agent"))
}
pub fn manifest_file(root: &Path, env: &SetupEnvironment) -> Result<PathBuf, SetupFailure> {
    Ok(legacy::manifest_path(state_dir(env)?, MANIFEST_KIND, root))
}
fn render(env: &SetupEnvironment, source: &str) -> Result<Vec<Rendered>, SetupFailure> {
    let executable = env
        .executable
        .to_str()
        .ok_or_else(|| invalid("executable is not UTF-8"))?;
    let state = state_dir(env)?
        .to_str()
        .ok_or_else(|| invalid("state directory is not UTF-8"))?;
    let host = env
        .host_endpoint
        .as_deref()
        .and_then(Path::to_str)
        .ok_or_else(|| invalid("host endpoint unavailable"))?;
    let harness = crate::harness::registry::builtins()
        .agent("pi")
        .map_err(|_| invalid("adapter unavailable"))?;
    let mut argv = crate::cli::hook::installed_event_argv(
        executable,
        Some(state),
        Some(host),
        crate::harness::registry::OccupantHarness::Agent(harness).into(),
        "SessionStart",
    )
    .map_err(|_| invalid("invalid hook command"))?;
    argv.truncate(argv.len() - 3);
    if source.lines().filter(|line| *line == LAUNCH_MARKER).count() != 1 {
        return Err(invalid("extension launch marker unavailable"));
    }
    let line = format!(
        "const LAUNCH = {} // herdr-threads:launch (written by setup pi)",
        json!({"argv":argv})
    );
    let extension = source.replacen(LAUNCH_MARKER, &line, 1);
    Ok(vec![
        Rendered {
            rel: EXTENSION_PATH,
            bytes: extension.into_bytes(),
        },
        Rendered {
            rel: SKILL_PATH,
            bytes: crate::cli::skill::SKILL_MD.as_bytes().to_vec(),
        },
    ])
}
// Empty metadata preserves the adapter's five-field manifest without adding a policy field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Metadata {}
type Manifest = owned_files::Manifest<Metadata>;
pub use crate::harness::owned_files::{FileAction, FileState, InstallReport, RemoveReport};
fn file_set<'a>(root: &'a Path, manifest: &'a Path) -> FileSet<'a> {
    FileSet {
        root,
        manifest,
        directories: &[
            ("extensions", false),
            ("extensions/herdr-threads", true),
            ("skills", false),
            ("skills/herdr-threads", true),
        ],
        invalid,
    }
}
fn read_manifest(path: &Path) -> Result<Option<Manifest>, SetupFailure> {
    owned_files::read_manifest(path, invalid)
}
/// Write or replace the two owned files. Every conflict is found before anything is written.
pub fn install(
    root: &Path,
    env: &SetupEnvironment,
    source: &str,
) -> Result<InstallReport, SetupFailure> {
    let rendered = render(env, source)?;
    let manifest = manifest_file(root, env)?;
    file_set(root, &manifest).install(&rendered, Metadata {})
}
pub fn remove(root: &Path, env: &SetupEnvironment) -> Result<RemoveReport, SetupFailure> {
    let manifest = manifest_file(root, env)?;
    file_set(root, &manifest).remove::<Metadata>()
}

#[derive(Debug, Clone)]
pub struct InstallStatus {
    pub root: PathBuf,
    pub plugin: FileState,
    pub skill: FileState,
    pub plugin_digest: Option<String>,
    pub diagnostics: Vec<SetupDiagnostic>,
}
impl InstallStatus {
    pub fn installed(&self) -> bool {
        self.plugin == FileState::Installed && self.skill == FileState::Installed
    }
}

fn warning(code: &str, text: &str) -> SetupDiagnostic {
    SetupDiagnostic::new(code, DiagnosticSeverity::Warning, text)
}

/// The installation state of `root`. Never fails: what cannot be read is a diagnostic.
pub fn install_status(root: &Path, env: &SetupEnvironment, source: &str) -> InstallStatus {
    let mut status = InstallStatus {
        root: root.to_path_buf(),
        plugin: FileState::NotInstalled,
        skill: FileState::NotInstalled,
        plugin_digest: None,
        diagnostics: Vec::new(),
    };
    let manifest = match manifest_file(root, env).and_then(|path| read_manifest(&path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            status.diagnostics.push(warning(
                "pi_manifest_unreadable",
                &format!("pi ownership record unreadable: {error}"),
            ));
            None
        }
    };
    let rendered = manifest.as_ref().and_then(|_| render(env, source).ok());
    let mut states = [FileState::NotInstalled; 2];
    for (index, rel) in [EXTENSION_PATH, SKILL_PATH].into_iter().enumerate() {
        let (state, digest) = owned_files::inspect_file(
            root,
            rel,
            manifest.as_ref().map(|m| m.files.as_slice()),
            rendered.as_deref(),
        );
        states[index] = state;
        if rel == EXTENSION_PATH {
            status.plugin_digest = digest;
        }
    }
    [status.plugin, status.skill] = states;
    for (what, state, path) in [
        ("plugin", status.plugin, EXTENSION_PATH),
        ("skill", status.skill, SKILL_PATH),
    ] {
        let path = root.join(path);
        let text = match state {
            FileState::Unowned => format!(
                "{} is not owned by herdr-threads; setup will not overwrite it. Move it aside or \
                 remove it, then run `herdr-threads setup pi`",
                path.display()
            ),
            FileState::Missing => format!(
                "the pi {what} {} is missing; re-run `herdr-threads setup pi`",
                path.display()
            ),
            FileState::Modified => format!(
                "the pi {what} {} was edited; restore or remove it, then re-run \
                 `herdr-threads setup pi`",
                path.display()
            ),
            FileState::Stale => format!(
                "the pi {what} differs from this herdr-threads build or instance; re-run \
                 `herdr-threads setup pi`"
            ),
            FileState::NotInstalled | FileState::Installed => continue,
        };
        status
            .diagnostics
            .push(warning(&format!("pi_{what}_{}", state.as_str()), &text));
    }
    status
}

fn projection(status: &InstallStatus) -> Value {
    json!({"installed":status.installed(),"config_root":status.root.display().to_string(),
        "hooks_file":status.root.join(EXTENSION_PATH).display().to_string(),
        "extension":{"path":status.root.join(EXTENSION_PATH).display().to_string(),"state":status.plugin.as_str()},
        "skill":{"path":status.root.join(SKILL_PATH).display().to_string(),"state":status.skill.as_str()}})
}
pub(super) fn setup(request: &SetupRequest) -> Result<SetupOutcome, SetupFailure> {
    let root = root_of(&request.scope)?;
    let mut env = request.environment.clone();
    env.executable = request.executable.clone();
    legacy::prepare_state(&crate::cli::setup::SetupEnv::from_snapshot(&env))
        .map_err(legacy::adapter_failure)?;
    let report = install(root, &env, SOURCE)?;
    let status = install_status(root, &env, SOURCE);
    let changed = report.changed();
    let mut projection = projection(&status);
    projection["action"] = json!(if changed {
        "installed"
    } else {
        "already_installed"
    });
    Ok(SetupOutcome {
        actions: vec![if changed {
            SetupAction::InstalledOwned
        } else {
            SetupAction::Unchanged
        }],
        diagnostic: String::new(),
        projection,
        diagnostics: status.diagnostics,
    })
}
pub(super) fn status(request: &StatusRequest) -> SetupStatus {
    let root = match root_of(&request.scope) {
        Ok(root) => root,
        Err(e) => return SetupStatus::Failed(e),
    };
    let status = install_status(root, &request.environment, SOURCE);
    let installed = status.installed();
    let projection = projection(&status);
    let repairable = |state| matches!(state, FileState::Missing | FileState::Stale);
    let repair = (repairable(status.plugin) || repairable(status.skill))
        && !matches!(status.plugin, FileState::Modified | FileState::Unowned)
        && !matches!(status.skill, FileState::Modified | FileState::Unowned);
    SetupStatus::Detailed(Box::new(LocalSetupStatus {
        scope: request.scope.clone(),
        installed,
        enabled: None,
        admitted: None,
        observed: None,
        fingerprint: installed.then(|| status.plugin_digest.clone()).flatten(),
        configured_hook: installed.then(|| ConfiguredHook {
            scope: "user".into(),
            path: root.join(EXTENSION_PATH).display().to_string(),
            fingerprint: status.plugin_digest.clone().unwrap_or_default(),
        }),
        diagnostics: status.diagnostics,
        repairs: if repair {
            vec![LocalRepair::RepairOwned]
        } else {
            vec![]
        },
        projection,
    }))
}
pub(super) fn unsetup(request: &UnsetupRequest) -> Result<RemovalOutcome, SetupFailure> {
    let root = root_of(&request.scope)?;
    let report = remove(root, &request.environment)?;
    let projection = json!({"action":if report.recorded{"removed"}else{"not_installed"},"removed":report.removed.iter().map(|p|p.display().to_string()).collect::<Vec<_>>(),"residue":report.residue.iter().map(|p|p.display().to_string()).collect::<Vec<_>>(),"incomplete":!report.residue.is_empty()});
    Ok(RemovalOutcome {
        actions: vec![if report.recorded {
            SetupAction::RemovedOwned
        } else {
            SetupAction::Unchanged
        }],
        diagnostic: String::new(),
        residue: report.residue,
        projection,
        diagnostics: vec![],
    })
}
pub(super) fn status_extras(request: &StatusRequest, value: &mut Value) {
    if let Ok(root) = root_of(&request.scope) {
        value["manifest"] = json!(
            manifest_file(root, &request.environment)
                .ok()
                .map(|p| p.display().to_string())
        );
    }
}
pub(super) fn report_lines(value: &Value) -> Vec<String> {
    ["extension", "skill"]
        .into_iter()
        .filter_map(|key| {
            Some(format!(
                "{key} {}: {}",
                value[key]["path"].as_str()?,
                value[key]["state"].as_str()?
            ))
        })
        .collect()
}
pub(super) fn doctor_projection(request: &StatusRequest) -> Option<DoctorProjection> {
    let SetupStatus::Detailed(status) = status(request) else {
        return None;
    };
    let present = crate::cli::hook::resolve_on_path("pi", request.environment.path.as_deref())
        .is_some()
        || root_of(&request.scope).ok()?.is_dir();
    Some(DoctorProjection {
        hooks: status.projection.clone(),
        limitations: if present && !status.installed {
            vec!["pi extension unavailable; run herdr-threads setup pi".into()]
        } else {
            vec![]
        },
        safe_repairs: if present {
            status.repairs.clone()
        } else {
            vec![]
        },
        status: Some(SetupStatus::Detailed(status)),
        manual_repairs: vec![],
        repair_options: Default::default(),
    })
}
pub(super) fn doctor_text(value: &Value, form: DoctorTextForm) -> String {
    match form {
        DoctorTextForm::Summary => format!(
            "pi: extension {}, skill {}\n",
            value["extension"]["state"].as_str().unwrap_or("unknown"),
            value["skill"]["state"].as_str().unwrap_or("unknown")
        ),
        DoctorTextForm::Debug => report_lines(value)
            .into_iter()
            .map(|line| format!("hooks.pi.{line}\n"))
            .collect(),
    }
}
