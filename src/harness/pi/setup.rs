//! Digest-owned extension and shared skill. Settings and permission configuration are untouched.
use crate::harness::{
    adapter::*,
    setup::{fingerprint, legacy, write_replacement},
};
use crate::ports::ConfiguredHook;
use crate::protocol::results::{ApiError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub const EXTENSION_PATH: &str = "extensions/herdr-threads/index.js";
pub const SKILL_PATH: &str = "skills/herdr-threads/SKILL.md";
pub const SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/integrations/pi/extension/herdr-threads.js"
));
const LAUNCH_MARKER: &str = "const LAUNCH = null // herdr-threads:launch";
const MANIFEST_KIND: &str = "pi";
const MAX_BYTES: u64 = 1_048_576;
fn invalid(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Invalid(format!("pi: {}", detail.into()))
}
fn conflict(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Api(ApiError::new(ErrorCode::Conflict, detail.into()))
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
struct Rendered {
    rel: &'static str,
    bytes: Vec<u8>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    path: String,
    digest: String,
    /// An in-flight replacement's digest: the file may hold either until setup finalizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
}
impl Entry {
    fn owns(&self, digest: &str) -> bool {
        self.digest == digest || self.pending.as_deref() == Some(digest)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    version: u8,
    config_root: String,
    binary_version: String,
    files: Vec<Entry>,
    /// Directories setup created, outermost first.
    created_dirs: Vec<String>,
}

enum ReadFailure {
    Unsafe,
    TooLarge,
    Io(io::Error),
}
impl ReadFailure {
    fn describe(&self, path: &Path) -> String {
        match self {
            Self::Unsafe => format!("{} is a symlink or not a regular file", path.display()),
            Self::TooLarge => format!("{} exceeds the size bound", path.display()),
            Self::Io(error) => format!("could not read {}: {error}", path.display()),
        }
    }
}

/// A bounded, symlink-refusing read; `None` when the file is absent.
fn read_regular(path: &Path) -> Result<Option<Vec<u8>>, ReadFailure> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadFailure::Io(error)),
    };
    if !meta.file_type().is_file() {
        return Err(ReadFailure::Unsafe);
    }
    if meta.len() > MAX_BYTES {
        return Err(ReadFailure::TooLarge);
    }
    fs::read(path).map(Some).map_err(ReadFailure::Io)
}

fn read_manifest(path: &Path) -> Result<Option<Manifest>, SetupFailure> {
    let bytes = match read_regular(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(failure) => return Err(invalid(failure.describe(path))),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|_| invalid(format!("{} is damaged", path.display())))?;
    if manifest.version != 1 {
        return Err(invalid(format!(
            "{} has an unknown version",
            path.display()
        )));
    }
    Ok(Some(manifest))
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), SetupFailure> {
    let bytes = serde_json::to_vec(manifest).map_err(|error| invalid(error.to_string()))?;
    write_replacement(path, &bytes, true)
        .map_err(|error| SetupFailure::Io(io::Error::other(format!("{error:?}"))))
}

// --------------------------------------------------------------- install

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Created,
    Updated,
    Unchanged,
}
impl FileAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub files: Vec<(&'static str, FileAction)>,
    pub created_dirs: Vec<PathBuf>,
}
impl InstallReport {
    pub fn changed(&self) -> bool {
        !self.created_dirs.is_empty()
            || self
                .files
                .iter()
                .any(|(_, action)| *action != FileAction::Unchanged)
    }
}

/// Make `path` a directory, creating missing ancestors outermost first. `strict` refuses a
/// symlink at `path` itself (setup's own `skills/herdr-threads`); shared directories such as
/// `plugins/` may be symlinked.
fn ensure_dir(path: &Path, strict: bool, created: &mut Vec<PathBuf>) -> Result<(), SetupFailure> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let ok = if strict {
                meta.file_type().is_dir()
            } else {
                fs::metadata(path).is_ok_and(|meta| meta.is_dir())
            };
            if ok {
                Ok(())
            } else {
                Err(conflict(format!(
                    "{} exists but is not a plain directory; nothing was changed",
                    path.display()
                )))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                ensure_dir(parent, false, created)?;
            }
            fs::create_dir(path).map_err(SetupFailure::Io)?;
            created.push(path.to_path_buf());
            Ok(())
        }
        Err(error) => Err(SetupFailure::Io(error)),
    }
}

/// Write or replace the two owned files. Every conflict is found before anything is written.
pub fn install(
    root: &Path,
    env: &SetupEnvironment,
    source: &str,
) -> Result<InstallReport, SetupFailure> {
    let rendered = render(env, source)?;
    let manifest_path = manifest_file(root, env)?;
    let recorded = read_manifest(&manifest_path)?;
    if let Some(recorded) = &recorded
        && recorded.config_root != root.to_string_lossy()
    {
        return Err(conflict(format!(
            "{} records another config root; nothing was changed",
            manifest_path.display()
        )));
    }
    let mut plan = Vec::new();
    for file in &rendered {
        let path = root.join(file.rel);
        let target = fingerprint(&file.bytes);
        let entry = recorded
            .as_ref()
            .and_then(|m| m.files.iter().find(|e| e.path == file.rel));
        let current = read_regular(&path).map_err(|failure| {
            conflict(format!("{}; nothing was changed", failure.describe(&path)))
        })?;
        // The digest of the bytes on disk, which may be the record's earlier pending one.
        let mut held = target.clone();
        let action = match current {
            None => FileAction::Created,
            Some(bytes) => match entry {
                Some(entry) if entry.owns(&fingerprint(&bytes)) => {
                    if bytes == file.bytes {
                        FileAction::Unchanged
                    } else {
                        held = fingerprint(&bytes);
                        FileAction::Updated
                    }
                }
                _ => {
                    return Err(conflict(format!(
                        "{} was edited or is not owned by herdr-threads; it was not overwritten \
                         and nothing was changed. Move it aside or remove it, then set up again",
                        path.display()
                    )));
                }
            },
        };
        plan.push((file, action, held, target));
    }
    let files_changed = plan
        .iter()
        .any(|(_, action, _, _)| *action != FileAction::Unchanged);
    let mut created_dirs = Vec::new();
    if files_changed || recorded.is_none() {
        ensure_dir(&root.join("extensions"), false, &mut created_dirs)?;
        ensure_dir(
            &root.join("extensions/herdr-threads"),
            true,
            &mut created_dirs,
        )?;
        ensure_dir(&root.join("skills"), false, &mut created_dirs)?;
        ensure_dir(&root.join("skills/herdr-threads"), true, &mut created_dirs)?;
        let mut all_created = recorded
            .as_ref()
            .map(|m| m.created_dirs.clone())
            .unwrap_or_default();
        all_created.extend(
            created_dirs
                .iter()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        let mut manifest = Manifest {
            version: 1,
            config_root: root.to_string_lossy().into_owned(),
            binary_version: env!("CARGO_PKG_VERSION").into(),
            files: plan
                .iter()
                .map(|(file, _, held, target)| Entry {
                    path: file.rel.into(),
                    digest: held.clone(),
                    pending: (held != target).then(|| target.clone()),
                })
                .collect(),
            created_dirs: all_created,
        };
        // Intent first: a crash mid-write leaves files the manifest still recognizes.
        write_manifest(&manifest_path, &manifest)?;
        for (file, action, _, _) in &plan {
            if *action != FileAction::Unchanged {
                write_replacement(&root.join(file.rel), &file.bytes, true)
                    .map_err(|error| SetupFailure::Io(io::Error::other(format!("{error:?}"))))?;
            }
        }
        for entry in &mut manifest.files {
            if let Some(pending) = entry.pending.take() {
                entry.digest = pending;
            }
        }
        write_manifest(&manifest_path, &manifest)?;
    }
    Ok(InstallReport {
        files: plan
            .iter()
            .map(|(file, action, _, _)| (file.rel, *action))
            .collect(),
        created_dirs,
    })
}

// --------------------------------------------------------------- removal

#[derive(Debug, Clone, Default)]
pub struct RemoveReport {
    pub removed: Vec<PathBuf>,
    pub residue: Vec<PathBuf>,
    /// A record existed.
    pub recorded: bool,
}

/// Remove only files that still match their recorded digest, then the directories setup created
/// that are now empty. An edited file is residue.
pub fn remove(root: &Path, env: &SetupEnvironment) -> Result<RemoveReport, SetupFailure> {
    let manifest_path = manifest_file(root, env)?;
    let Some(mut manifest) = read_manifest(&manifest_path)? else {
        return Ok(RemoveReport::default());
    };
    if manifest.config_root != root.to_string_lossy() {
        return Err(conflict(format!(
            "{} records another config root; nothing was changed",
            manifest_path.display()
        )));
    }
    let mut report = RemoveReport {
        recorded: true,
        ..Default::default()
    };
    let mut kept = Vec::new();
    for entry in std::mem::take(&mut manifest.files) {
        let path = root.join(&entry.path);
        match read_regular(&path) {
            Ok(None) => {}
            Ok(Some(bytes)) if entry.owns(&fingerprint(&bytes)) => {
                fs::remove_file(&path).map_err(SetupFailure::Io)?;
                report.removed.push(path);
            }
            _ => {
                report.residue.push(path);
                kept.push(entry);
            }
        }
    }
    for dir in manifest.created_dirs.iter().rev() {
        // `remove_dir` refuses a non-empty directory: that is the emptiness check.
        let _ = fs::remove_dir(dir);
    }
    if kept.is_empty() {
        fs::remove_file(&manifest_path).map_err(SetupFailure::Io)?;
    } else {
        manifest.files = kept;
        manifest.created_dirs.retain(|dir| Path::new(dir).exists());
        write_manifest(&manifest_path, &manifest)?;
    }
    Ok(report)
}

// ---------------------------------------------------------------- status

/// One owned file's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// No record (setup not run here).
    NotInstalled,
    /// No record, but a file sits at the owned path.
    Unowned,
    /// Recorded, but gone.
    Missing,
    /// Recorded, but its bytes differ from the record.
    Modified,
    /// Matches the record, but this build or instance renders other bytes.
    Stale,
    Installed,
}
impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Unowned => "unowned",
            Self::Missing => "missing",
            Self::Modified => "modified",
            Self::Stale => "stale",
            Self::Installed => "installed",
        }
    }
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
        let path = root.join(rel);
        let bytes = read_regular(&path);
        let digest = match &bytes {
            Ok(Some(bytes)) => Some(fingerprint(bytes)),
            _ => None,
        };
        let present = !matches!(bytes, Ok(None));
        states[index] = match &manifest {
            None if present => FileState::Unowned,
            None => FileState::NotInstalled,
            Some(manifest) => match (
                manifest.files.iter().find(|entry| entry.path == rel),
                &digest,
            ) {
                (_, None) if !present => FileState::Missing,
                (Some(entry), Some(digest))
                    if entry.pending.is_none() && entry.digest == *digest =>
                {
                    let target = rendered
                        .as_ref()
                        .and_then(|files| files.iter().find(|file| file.rel == rel))
                        .map(|file| fingerprint(&file.bytes));
                    if target.as_deref() == Some(digest.as_str()) {
                        FileState::Installed
                    } else {
                        FileState::Stale
                    }
                }
                (None, _) if present => FileState::Unowned,
                _ => FileState::Modified,
            },
        };
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
