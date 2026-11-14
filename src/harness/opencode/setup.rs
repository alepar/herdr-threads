//! opencode `setup` / `setup-status` / `unsetup` (opencode design D6): two owned files under the
//! opencode config root, `plugins/herdr-threads.js` (the embedded plugin with its launch line
//! rendered) and `skills/herdr-threads/SKILL.md` (the shared skill plus an opencode section).
//! `opencode.json` and every other user file are never read or written.
//!
//! Ownership is a private manifest `<state>/setup/opencode-<sha256(config root)[..32]>.json`
//! recording each file's digest, the directories setup created and the consent variant the
//! plugin was rendered with. A file at an owned path with no record or a foreign digest is a
//! conflict and is never overwritten; unsetup removes only files that still match their record.
//!
//! Two placeholder lines of the plugin are rendered, each replaced whole:
//! - [`LAUNCH_MARKER`] becomes `const LAUNCH = {"argv":[...]}` plus [`LAUNCH_SUFFIX`]: the hook
//!   invocation prefix (executable, `--state-dir`, `--host-endpoint`) the plugin appends
//!   `hook-event opencode <Event>` or `watch ...` to. The plugin derives its heartbeat file from
//!   the `--state-dir` value ([`heartbeat_path`]).
//! - [`GRANT_MARKER`] becomes [`GRANT_GRANTED`] with consent and stays as is without: the consent
//!   to add the `herdr-threads *` / `ht *` bash allow keys in the plugin's `config` hook.
//!
//! A source without a marker line is installed with that line's substitution skipped.
use crate::{
    cli::setup::{PermissionPolicy, SetupEnv},
    harness::{
        adapter::{
            DiagnosticSeverity, LocalRepair, LocalSetupStatus, RemovalOutcome, ResolvedSetupScope,
            SetupAction, SetupDiagnostic, SetupEnvironment, SetupFailure, SetupOutcome,
            SetupRequest, SetupStatus, StatusRequest, UnsetupRequest,
        },
        context::Harness,
        registry::{OccupantHarness, builtins},
        setup::{fingerprint, legacy, write_replacement},
    },
    ports::ConfiguredHook,
    protocol::results::{ApiError, ErrorCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

/// The embedded plugin source (opaque here: only its launch marker line is rendered).
pub const PLUGIN_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/integrations/opencode/plugin/herdr-threads.js"
));
/// The plugin's path under the config root.
pub const PLUGIN_PATH: &str = "plugins/herdr-threads.js";
/// The skill's path under the config root.
pub const SKILL_PATH: &str = "skills/herdr-threads/SKILL.md";
/// The source line that setup replaces with the rendered launch object.
pub const LAUNCH_MARKER: &str = "const LAUNCH = null // herdr-threads:launch";
pub const LAUNCH_PREFIX: &str = "const LAUNCH = ";
pub const LAUNCH_SUFFIX: &str = " // herdr-threads:launch (written by setup opencode)";
/// The bash permission keys the plugin adds with consent.
pub const ALLOW_KEYS: [&str; 2] = ["herdr-threads *", "ht *"];
/// The consent placeholder line, and what setup writes in its place with consent.
pub const GRANT_MARKER: &str = "const GRANT_PERMISSIONS = false // herdr-threads:permissions";
pub const GRANT_GRANTED: &str =
    "const GRANT_PERMISSIONS = true // herdr-threads:permissions (written by setup opencode)";
/// The heartbeat file under `<state>/`, as the plugin writes it.
pub const HEARTBEAT_FILE: [&str; 2] = ["opencode", "plugin-heartbeat.json"];
const MANIFEST_KIND: &str = "opencode";
const MAX_BYTES: u64 = 1_048_576;

/// The opencode section of the installed skill (D6: its shell tool and the session variables).
pub const OPENCODE_SKILL_GUIDANCE: &str = "\
- Run herdr-threads commands with the bash tool, each as one plain `herdr-threads ...` command. \
The herdr-threads plugin sets HERDR_THREADS_OPENCODE_SESSION, HERDR_THREADS_OPENCODE_PARENT and \
HERDR_THREADS_OPENCODE_PID on every shell call; never set or clear them. A \
`session_claim_required` naming HERDR_THREADS_OPENCODE_SESSION means the plugin is not loaded \
(`--pure`, or setup not run): run `herdr-threads setup opencode` and restart opencode.
- Thread message bodies arrive as hidden messages in your session, framed as described under \
\"Trust model\" above.
- Subagents never ACK, accept or send. Only the top-level agent works threads.
";

/// The consent question an interactive `setup opencode` asks when nothing is granted.
pub const OPENCODE_PERMISSION_QUESTION: &str = "Let opencode agents run herdr-threads commands \
    without prompting? The plugin allows \"herdr-threads *\" and \"ht *\" in opencode's bash \
    permissions, which also covers person and setup commands [Y/n] ";
/// The non-interactive advice when nothing is granted.
pub const OPENCODE_PERMISSION_ADVICE: &str = "opencode asks before each herdr-threads command \
    until you choose \"Allow always\": re-run `herdr-threads setup opencode --with-permissions` to \
    have the plugin allow \"herdr-threads *\" and \"ht *\" (every herdr-threads command, person and \
    setup commands included), or --without-permissions to keep it this way";

// ------------------------------------------------------------------ paths

/// A non-empty absolute path from a declared variable.
fn absolute(value: Option<&std::ffi::OsString>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`: opencode's global config dir.
pub fn xdg_config_dir(env: &SetupEnvironment) -> Option<PathBuf> {
    absolute(env.declared.get("XDG_CONFIG_HOME"))
        .map(|dir| dir.join("opencode"))
        .or_else(|| {
            env.home
                .as_deref()
                .filter(|home| !home.is_empty())
                .map(|home| Path::new(home).join(".config").join("opencode"))
        })
}

/// The default config root: `OPENCODE_CONFIG_DIR`, else [`xdg_config_dir`]. An entry in the
/// environment's `config_roots` (the internal installer's `--config-root`) wins over both.
pub fn default_config_root(env: &SetupEnvironment) -> Option<PathBuf> {
    absolute(env.declared.get("OPENCODE_CONFIG_DIR")).or_else(|| xdg_config_dir(env))
}

pub(super) fn root_of(scope: &ResolvedSetupScope) -> Result<&Path, SetupFailure> {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => Ok(root),
        ResolvedSetupScope::Profile { .. } => Err(SetupFailure::Invalid(
            "opencode: named profile is unsupported".into(),
        )),
    }
}

fn invalid(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Invalid(format!("opencode: {}", detail.into()))
}
fn conflict(detail: impl Into<String>) -> SetupFailure {
    SetupFailure::Api(ApiError::new(ErrorCode::Conflict, detail.into()))
}

fn state_dir(env: &SetupEnvironment) -> Result<&Path, SetupFailure> {
    env.state_dir
        .as_deref()
        .ok_or_else(|| invalid("state directory unknown"))
}

/// The plugin heartbeat: `<state>/opencode/plugin-heartbeat.json`.
pub fn heartbeat_path(state: &Path) -> PathBuf {
    HEARTBEAT_FILE
        .iter()
        .fold(state.to_path_buf(), |path, part| path.join(part))
}

/// The manifest file of this config root.
pub fn manifest_file(root: &Path, env: &SetupEnvironment) -> Result<PathBuf, SetupFailure> {
    Ok(legacy::manifest_path(state_dir(env)?, MANIFEST_KIND, root))
}

// ---------------------------------------------------------------- render

/// The rendered launch object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    pub argv: Vec<String>,
}

/// The hook invocation prefix `[exe, --state-dir, S, --host-endpoint, H]`, validated by the hook
/// argv parser.
pub fn launch_argv(env: &SetupEnvironment) -> Result<Vec<String>, SetupFailure> {
    let executable = env
        .executable
        .to_str()
        .ok_or_else(|| invalid("herdr-threads executable path is not UTF-8"))?;
    let state = state_dir(env)?
        .to_str()
        .ok_or_else(|| invalid("state directory path is not UTF-8"))?;
    let host = env
        .host_endpoint
        .as_deref()
        .ok_or_else(|| invalid("Herdr host endpoint unknown"))?
        .to_str()
        .ok_or_else(|| invalid("host endpoint path is not UTF-8"))?;
    let harness: Harness = OccupantHarness::Agent(
        builtins()
            .agent(super::METADATA.id)
            .map_err(|error| SetupFailure::Invalid(error.to_string()))?,
    )
    .into();
    let mut argv = crate::cli::hook::installed_event_argv(
        executable,
        Some(state),
        Some(host),
        harness,
        "SessionStart",
    )
    .map_err(|_| invalid("hook command is not valid"))?;
    // Drop `hook-event opencode SessionStart`: the plugin appends its own event or `watch`.
    argv.truncate(argv.len().saturating_sub(3));
    Ok(argv)
}

pub fn launch(env: &SetupEnvironment) -> Result<Launch, SetupFailure> {
    Ok(Launch {
        argv: launch_argv(env)?,
    })
}

/// `source` with every line that starts with [`LAUNCH_MARKER`] replaced by the rendered launch
/// line and, with `permissions`, every line that starts with [`GRANT_MARKER`] replaced by
/// [`GRANT_GRANTED`]; a source without a marker is left as is there.
pub fn render_plugin(source: &str, launch: &Launch, permissions: bool) -> String {
    let json = serde_json::to_string(launch).expect("a launch object serializes");
    let mut out = String::with_capacity(source.len() + json.len() + GRANT_GRANTED.len());
    for segment in source.split_inclusive('\n') {
        let newline = if segment.ends_with('\n') { "\n" } else { "" };
        if segment.starts_with(LAUNCH_MARKER) {
            out.push_str(LAUNCH_PREFIX);
            out.push_str(&json);
            out.push_str(LAUNCH_SUFFIX);
            out.push_str(newline);
        } else if permissions && segment.starts_with(GRANT_MARKER) {
            out.push_str(GRANT_GRANTED);
            out.push_str(newline);
        } else {
            out.push_str(segment);
        }
    }
    out
}

/// Whether an installed plugin carries the rendered grant line.
pub fn installed_grant(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.lines().any(|line| line == GRANT_GRANTED))
}

/// The launch object an installed plugin carries; `None` without a rendered line.
pub fn installed_launch(bytes: &[u8]) -> Option<Launch> {
    let text = std::str::from_utf8(bytes).ok()?;
    let line = text.lines().find(|line| line.starts_with(LAUNCH_PREFIX))?;
    let json = line
        .strip_prefix(LAUNCH_PREFIX)?
        .strip_suffix(LAUNCH_SUFFIX)?;
    serde_json::from_str(json).ok()
}

/// The installed skill: the core skill text, then the opencode section.
pub fn skill_md() -> String {
    format!(
        "{}\n## opencode\n{OPENCODE_SKILL_GUIDANCE}",
        crate::cli::skill::SKILL_MD
    )
}

/// One owned file: its path relative to the config root and its exact bytes.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub rel: &'static str,
    pub bytes: Vec<u8>,
}

pub fn render(
    env: &SetupEnvironment,
    permissions: bool,
    source: &str,
) -> Result<Vec<Rendered>, SetupFailure> {
    Ok(vec![
        Rendered {
            rel: PLUGIN_PATH,
            bytes: render_plugin(source, &launch(env)?, permissions).into_bytes(),
        },
        Rendered {
            rel: SKILL_PATH,
            bytes: skill_md().into_bytes(),
        },
    ])
}

// -------------------------------------------------------------- manifest

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
    /// The consent variant the plugin was rendered with.
    permissions: bool,
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

/// The recorded consent of this config root; `None` without a record.
pub fn recorded_permissions(root: &Path, env: &SetupEnvironment) -> Option<bool> {
    manifest_file(root, env)
        .and_then(|path| read_manifest(&path))
        .ok()
        .flatten()
        .map(|manifest| manifest.permissions)
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
    pub permissions_changed: bool,
}
impl InstallReport {
    pub fn changed(&self) -> bool {
        self.permissions_changed
            || !self.created_dirs.is_empty()
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
    permissions: bool,
    source: &str,
) -> Result<InstallReport, SetupFailure> {
    let rendered = render(env, permissions, source)?;
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
        let action = match current {
            None => FileAction::Created,
            Some(bytes) => match entry {
                Some(entry) if entry.owns(&fingerprint(&bytes)) => {
                    if bytes == file.bytes {
                        FileAction::Unchanged
                    } else {
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
        let held = match (action, entry) {
            (FileAction::Updated, Some(entry)) => entry.digest.clone(),
            _ => target.clone(),
        };
        plan.push((file, action, held, target));
    }
    let permissions_changed = recorded
        .as_ref()
        .is_some_and(|m| m.permissions != permissions);
    let files_changed = plan
        .iter()
        .any(|(_, action, _, _)| *action != FileAction::Unchanged);
    let mut created_dirs = Vec::new();
    if files_changed || permissions_changed || recorded.is_none() {
        ensure_dir(&root.join("plugins"), false, &mut created_dirs)?;
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
            permissions,
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
        permissions_changed,
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
    /// The recorded consent; `None` without a record.
    pub permissions: Option<bool>,
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
        permissions: None,
        plugin_digest: None,
        diagnostics: Vec::new(),
    };
    let manifest = match manifest_file(root, env).and_then(|path| read_manifest(&path)) {
        Ok(manifest) => manifest,
        Err(error) => {
            status.diagnostics.push(warning(
                "opencode_manifest_unreadable",
                &format!("opencode ownership record unreadable: {error}"),
            ));
            None
        }
    };
    let rendered = manifest
        .as_ref()
        .and_then(|m| render(env, m.permissions, source).ok());
    status.permissions = manifest.as_ref().map(|m| m.permissions);
    let mut states = [FileState::NotInstalled; 2];
    for (index, rel) in [PLUGIN_PATH, SKILL_PATH].into_iter().enumerate() {
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
        if rel == PLUGIN_PATH {
            status.plugin_digest = digest;
        }
    }
    [status.plugin, status.skill] = states;
    for (what, state, path) in [
        ("plugin", status.plugin, PLUGIN_PATH),
        ("skill", status.skill, SKILL_PATH),
    ] {
        let path = root.join(path);
        let text = match state {
            FileState::Unowned => format!(
                "{} is not owned by herdr-threads; setup will not overwrite it. Move it aside or \
                 remove it, then run `herdr-threads setup opencode`",
                path.display()
            ),
            FileState::Missing => format!(
                "the opencode {what} {} is missing; re-run `herdr-threads setup opencode`",
                path.display()
            ),
            FileState::Modified => format!(
                "the opencode {what} {} was edited; restore or remove it, then re-run \
                 `herdr-threads setup opencode`",
                path.display()
            ),
            FileState::Stale => format!(
                "the opencode {what} differs from this herdr-threads build or instance; re-run \
                 `herdr-threads setup opencode`"
            ),
            FileState::NotInstalled | FileState::Installed => continue,
        };
        status.diagnostics.push(warning(
            &format!("opencode_{what}_{}", state.as_str()),
            &text,
        ));
    }
    status
}

// ------------------------------------------------------------ projections

fn harness_version(native_binary: Option<&Path>, env: &SetupEnvironment) -> Value {
    let binary = native_binary
        .map(Path::to_path_buf)
        .or_else(|| crate::cli::hook::resolve_on_path("opencode", env.path.as_deref()));
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
            "refusal": "installed opencode executable not found on PATH",
        }),
    }
}

fn permissions_state(permissions: Option<bool>) -> &'static str {
    match permissions {
        Some(true) => "granted",
        _ => "not_granted",
    }
}

fn permissions_json(permissions: Option<bool>) -> Value {
    json!({
        "state": permissions_state(permissions),
        "rules": ALLOW_KEYS,
    })
}

fn status_projection(
    status: &InstallStatus,
    native: Option<&Path>,
    env: &SetupEnvironment,
) -> Value {
    json!({
        "installed": status.installed(),
        "config_root": status.root.display().to_string(),
        "harness_version": harness_version(native, env),
        "hooks_file": status.root.join(PLUGIN_PATH).display().to_string(),
        "plugin": {
            "path": status.root.join(PLUGIN_PATH).display().to_string(),
            "state": status.plugin.as_str(),
        },
        "skill": {
            "path": status.root.join(SKILL_PATH).display().to_string(),
            "state": status.skill.as_str(),
        },
        "permissions": permissions_json(status.permissions),
    })
}

// ----------------------------------------------------------- entry points

/// The consent `Ask` keeps: the recorded variant, else none (advised).
fn ask_consent(root: &Path, env: &SetupEnvironment) -> Option<bool> {
    recorded_permissions(root, env)
}

pub(super) fn setup(request: &SetupRequest) -> Result<SetupOutcome, SetupFailure> {
    setup_with(request, PLUGIN_SOURCE)
}

pub(super) fn setup_with(
    request: &SetupRequest,
    source: &str,
) -> Result<SetupOutcome, SetupFailure> {
    let root = root_of(&request.scope)?;
    let mut environment = request.environment.clone();
    environment.executable = request.executable.clone();
    let env = &environment;
    state_dir(env)?;
    legacy::prepare_state(&SetupEnv::from_snapshot(env)).map_err(legacy::adapter_failure)?;
    let before = recorded_permissions(root, env);
    let (permissions, action) = match PermissionPolicy::from_options(&request.options) {
        PermissionPolicy::Grant => (true, "granted"),
        PermissionPolicy::Decline => (
            false,
            if before == Some(true) {
                "removed"
            } else {
                "declined"
            },
        ),
        PermissionPolicy::Ask => match ask_consent(root, env) {
            Some(consent) => (consent, "kept"),
            None => (false, "advised"),
        },
    };
    let report = install(root, env, permissions, source)?;
    let status = install_status(root, env, source);
    let changed = report.changed();
    let mut projection = status_projection(&status, request.native_binary.as_deref(), env);
    projection["action"] = json!(if changed {
        "installed"
    } else {
        "already_installed"
    });
    projection["permissions"]["action"] = json!(action);
    if action == "advised" {
        projection["permissions"]["note"] = json!(OPENCODE_PERMISSION_ADVICE);
    }
    projection["files"] = json!(
        report
            .files
            .iter()
            .map(|(rel, action)| json!({"path": rel, "action": action.as_str()}))
            .collect::<Vec<_>>()
    );
    projection["warnings"] = json!(
        status
            .diagnostics
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
        diagnostics: status.diagnostics,
    })
}

/// Ask on a terminal when setup only advised; a yes (the default) re-renders the plugin with
/// the grant.
pub(super) fn settle_permissions(
    scope: &ResolvedSetupScope,
    env: &SetupEnvironment,
    report: &mut Value,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<(), SetupFailure> {
    settle_permissions_with(scope, env, report, reader, writer, PLUGIN_SOURCE)
}

pub(super) fn settle_permissions_with(
    scope: &ResolvedSetupScope,
    env: &SetupEnvironment,
    report: &mut Value,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    source: &str,
) -> Result<(), SetupFailure> {
    if report["permissions"]["action"] != "advised" {
        return Ok(());
    }
    let root = root_of(scope)?;
    write!(writer, "{OPENCODE_PERMISSION_QUESTION}").map_err(SetupFailure::Io)?;
    writer.flush().map_err(SetupFailure::Io)?;
    let yes = crate::harness::claude::setup::permission_answer(reader).map_err(SetupFailure::Io)?;
    if yes {
        install(root, env, true, source)?;
        report["permissions"] = permissions_json(Some(true));
        report["permissions"]["action"] = json!("granted");
    } else {
        report["permissions"]["action"] = json!("declined");
        if let Some(permissions) = report["permissions"].as_object_mut() {
            permissions.remove("note");
        }
    }
    Ok(())
}

pub(super) fn status(request: &StatusRequest) -> SetupStatus {
    status_with(request, PLUGIN_SOURCE)
}

pub(super) fn status_with(request: &StatusRequest, source: &str) -> SetupStatus {
    let root = match root_of(&request.scope) {
        Ok(root) => root,
        Err(error) => return SetupStatus::Failed(error),
    };
    let env = &request.environment;
    let status = install_status(root, env, source);
    let installed = status.installed();
    let mut projection = status_projection(&status, request.native_binary.as_deref(), env);
    if env.state_dir.is_none() {
        projection["error"] =
            json!("state directory unknown: use --state-dir or HERDR_PLUGIN_STATE_DIR");
    }
    // Only what setup can fix without overwriting a foreign or edited file.
    let repairable = |state| matches!(state, FileState::Missing | FileState::Stale);
    let needs_repair = (repairable(status.plugin) || repairable(status.skill))
        && !matches!(status.plugin, FileState::Modified | FileState::Unowned)
        && !matches!(status.skill, FileState::Modified | FileState::Unowned);
    SetupStatus::Detailed(Box::new(LocalSetupStatus {
        scope: request.scope.clone(),
        installed,
        enabled: None,
        admitted: None,
        observed: None,
        fingerprint: installed.then(|| status.plugin_digest.clone()).flatten(),
        configured_hook: installed
            .then(|| {
                status
                    .plugin_digest
                    .clone()
                    .map(|fingerprint| ConfiguredHook {
                        scope: "user".into(),
                        path: root.join(PLUGIN_PATH).to_string_lossy().into_owned(),
                        fingerprint,
                    })
            })
            .flatten(),
        diagnostics: status.diagnostics,
        repairs: if needs_repair {
            vec![LocalRepair::RepairOwned]
        } else {
            Vec::new()
        },
        projection,
    }))
}

pub(super) fn unsetup(request: &UnsetupRequest) -> Result<RemovalOutcome, SetupFailure> {
    let root = root_of(&request.scope)?;
    let env = &request.environment;
    state_dir(env)?;
    let report = remove(root, env)?;
    let changed = report.recorded;
    let incomplete = !report.residue.is_empty();
    let projection = json!({
        "action": if changed { "removed" } else { "not_installed" },
        "hooks_file": root.join(PLUGIN_PATH).display().to_string(),
        "removed": report.removed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "residue": report.residue.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "incomplete": incomplete,
    });
    let diagnostics = report
        .residue
        .iter()
        .map(|path| {
            warning(
                "opencode_residue",
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
        residue: report.residue,
        projection,
        diagnostics,
    })
}

/// `setup-status` only: where the ownership record lives.
pub(super) fn setup_status_extras(request: &StatusRequest, projection: &mut Value) {
    let Ok(root) = root_of(&request.scope) else {
        return;
    };
    projection["manifest"] = json!(
        manifest_file(root, &request.environment)
            .ok()
            .map(|p| p.display().to_string())
    );
}

pub(super) fn setup_report_lines(report: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for key in ["plugin", "skill"] {
        if let Some(path) = report[key]["path"].as_str() {
            let state = report[key]["state"].as_str().unwrap_or("unknown");
            lines.push(format!("{key} {path}: {}", state.replace('_', "-")));
        }
    }
    if let Some(state) = report["permissions"]["state"].as_str() {
        let action = report["permissions"]["action"]
            .as_str()
            .map(|action| format!(" ({action})"))
            .unwrap_or_default();
        lines.push(format!("permissions: {}{action}", state.replace('_', "-")));
    }
    lines
}

#[cfg(test)]
#[path = "../../../tests/harness/opencode_setup.rs"]
mod tests;
