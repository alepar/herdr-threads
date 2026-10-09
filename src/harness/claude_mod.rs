//! The Claude delivery mod as an owned install (spec D8, ht-j16.7).
//!
//! `setup claude` writes the embedded mod files ([`MOD_FILES`]) to
//! `<state>/claude-mod/herdr-threads/` and appends that directory to
//! `env.CLAUDE_CODE_PLUGIN_DIRS` in the user's Claude `settings.json`
//! (`:`-joined, an existing value preserved). The key is its own owned edit,
//! like [`super::prompt_suggestion`]: a private manifest under
//! `<state>/setup/` records the key's previous value, the directories setup
//! appended and copied in from the shell, and the file's exact bytes before
//! and after the write. The file is replaced atomically after an
//! unchanged-baseline check; a value edited by hand since keeps the user's
//! entries. The hook manifest ([`super::setup::OwnershipManifest`]) is not
//! touched.
//!
//! Managed policy is honored: a truthy `disableSideloadFlags` in a managed
//! settings file makes Claude Code refuse `CLAUDE_CODE_PLUGIN_DIRS`, so the
//! write is skipped ([`ManagedPolicy`]). The documented file, the
//! `managed-settings.d/*.json` drop-ins and the server-managed cache are all
//! checked, and a policy setup cannot read or parse is treated as unsafe
//! (not known to be absent), so the write is skipped too.
use super::{
    claude::{
        DISABLE_SIDELOAD_FLAGS_KEY, MANAGED_SETTINGS_DROPIN_DIR, MANAGED_SETTINGS_LINUX,
        MANAGED_SETTINGS_MACOS, MOD_MIN_VERSION, PLUGIN_DIRS_ENV, SERVER_MANAGED_SETTINGS_CACHE,
    },
    setup::{SetupError, config_bytes, publish_manifest, write_replacement},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

/// The embedded mod: the published path and the bytes. Exactly the files
/// Claude Code loads (ht-j16.1 contract decision 7); never the README,
/// tsconfig or tests.
pub const MOD_FILES: [(&str, &str); 4] = [
    (
        ".claude-plugin/plugin.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/claude/mod/.claude-plugin/plugin.json"
        )),
    ),
    (
        "hooks/hooks.json",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/claude/mod/hooks/hooks.json"
        )),
    ),
    (
        "hooks/register.js",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/claude/mod/hooks/register.js"
        )),
    ),
    (
        "types/index.d.ts",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/integrations/claude/mod/types/index.d.ts"
        )),
    ),
];

/// Where the mod files live under the plugin state directory.
pub fn mod_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("claude-mod").join("herdr-threads")
}

/// A digest of the embedded file set (paths and bytes).
pub fn files_fingerprint() -> String {
    let mut digest = Sha256::new();
    for (path, content) in MOD_FILES {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update((content.len() as u64).to_le_bytes());
        digest.update(content.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// Whether every embedded file exists in `dir` with the embedded bytes.
pub fn files_current(dir: &Path) -> bool {
    MOD_FILES.iter().all(|(path, content)| {
        let file = dir.join(path);
        file.symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_file())
            && fs::read(&file).is_ok_and(|bytes| bytes == content.as_bytes())
    })
}

/// Write the files that differ from the embedded set, each by
/// write-to-temp-then-rename (mode 0644, directories 0755). Returns the
/// published paths written; an unchanged set writes nothing (a needless
/// rewrite would reload the module in running sessions).
pub fn write_files(dir: &Path) -> Result<Vec<&'static str>, SetupError> {
    let mut written = Vec::new();
    for (path, content) in MOD_FILES {
        let file = dir.join(path);
        let unchanged = file
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_file())
            && fs::read(&file).is_ok_and(|bytes| bytes == content.as_bytes());
        if unchanged {
            continue;
        }
        let parent = file.parent().ok_or(SetupError::Invalid)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(parent)
            .map_err(|_| SetupError::Io)?;
        let temp = parent.join(format!(".herdr-mod-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut handle = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&temp)
                .map_err(|_| SetupError::Io)?;
            handle
                .set_permissions(fs::Permissions::from_mode(0o644))
                .map_err(|_| SetupError::Io)?;
            handle
                .write_all(content.as_bytes())
                .and_then(|_| handle.sync_all())
                .map_err(|_| SetupError::Io)?;
            fs::rename(&temp, &file).map_err(|_| SetupError::Io)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        written.push(path);
    }
    Ok(written)
}

/// Delete the mod directory (and its empty `claude-mod` parent). A symlink
/// at the location is left alone. Returns whether anything was removed.
pub fn remove_files(state_dir: &Path) -> Result<bool, SetupError> {
    let dir = mod_dir(state_dir);
    let Ok(meta) = dir.symlink_metadata() else {
        return Ok(false);
    };
    if !meta.file_type().is_dir() {
        return Err(SetupError::Invalid);
    }
    fs::remove_dir_all(&dir).map_err(|_| SetupError::Io)?;
    if let Some(parent) = dir.parent() {
        let _ = fs::remove_dir(parent);
    }
    Ok(true)
}

// ------------------------------------------------------------ version gate

/// `Some(true)` when `version` (`major.minor.patch`, optionally followed by
/// a suffix) is at least [`MOD_MIN_VERSION`], `Some(false)` when older,
/// `None` when it does not parse. Older versions never load the mod: hooks
/// plus the native wake fallback apply.
pub fn version_supported(version: &str) -> Option<bool> {
    let mut parts = version.trim().split(['.', ' ', '-']);
    let mut next = || parts.next()?.parse::<u32>().ok();
    let found = (next()?, next()?, next()?);
    Some(found >= MOD_MIN_VERSION)
}

// ---------------------------------------------------------- managed policy

/// The managed settings files and drop-in directories to read. Injectable so
/// tests point them into a temp dir.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManagedPolicySources {
    pub files: Vec<PathBuf>,
    /// Directories of `*.json` drop-ins (`managed-settings.d`), merged by
    /// Claude Code in file-name order.
    pub dropin_dirs: Vec<PathBuf>,
}

/// Test-support builds read the file list (`:`-separated) from this variable
/// instead of the platform paths, so a CLI test can place a managed policy
/// inside its scratch dir. Production builds never read it.
pub const TEST_MANAGED_SETTINGS_ENV: &str = "HERDR_THREADS_TEST_MANAGED_SETTINGS";

/// Companion of [`TEST_MANAGED_SETTINGS_ENV`] for drop-in directories
/// (`:`-separated; unset means none). Read only when the files variable is
/// set, so no test ever reads the host's real drop-in directory.
pub const TEST_MANAGED_SETTINGS_DIRS_ENV: &str = "HERDR_THREADS_TEST_MANAGED_SETTINGS_DIRS";

impl ManagedPolicySources {
    /// The documented managed settings file of this platform, its
    /// `managed-settings.d` drop-in directory and, when the Claude config dir
    /// is known, the cached server-managed settings. Only these are checked:
    /// policy delivered by other means (a macOS configuration profile, for
    /// one) is not visible to setup.
    pub fn platform(claude_config_dir: Option<&Path>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(files) = std::env::var_os(TEST_MANAGED_SETTINGS_ENV) {
            return Self {
                files: std::env::split_paths(&files).collect(),
                dropin_dirs: std::env::var_os(TEST_MANAGED_SETTINGS_DIRS_ENV)
                    .map(|dirs| std::env::split_paths(&dirs).collect())
                    .unwrap_or_default(),
            };
        }
        let documented = Path::new(if cfg!(target_os = "macos") {
            MANAGED_SETTINGS_MACOS
        } else {
            MANAGED_SETTINGS_LINUX
        });
        let mut files = vec![documented.to_path_buf()];
        if let Some(dir) = claude_config_dir {
            files.push(dir.join(SERVER_MANAGED_SETTINGS_CACHE));
        }
        let dropin_dirs = documented
            .parent()
            .map(|parent| vec![parent.join(MANAGED_SETTINGS_DROPIN_DIR)])
            .unwrap_or_default();
        Self { files, dropin_dirs }
    }

    /// A policy setup cannot read is not known to be absent: a read error
    /// other than not-found, or content that is not a JSON object, is
    /// [`ManagedPolicy::Unverifiable`] and blocks the write like a set flag.
    /// A set flag is reported in preference to an unreadable source.
    pub fn check(&self) -> ManagedPolicy {
        let mut judged: Vec<(PathBuf, Judged)> = Vec::new();
        for file in &self.files {
            judged.push((file.clone(), judge_file(file)));
        }
        for dir in &self.dropin_dirs {
            match dropin_files(dir) {
                Ok(files) => {
                    for file in files {
                        let verdict = judge_file(&file);
                        judged.push((file, verdict));
                    }
                }
                Err(()) => judged.push((dir.clone(), Judged::Unreadable)),
            }
        }
        if let Some((source, _)) = judged.iter().find(|(_, v)| matches!(v, Judged::Flag)) {
            return ManagedPolicy::DisableSideloadFlags {
                source: source.clone(),
            };
        }
        match judged.iter().find(|(_, v)| matches!(v, Judged::Unreadable)) {
            Some((source, _)) => ManagedPolicy::Unverifiable {
                source: source.clone(),
            },
            None => ManagedPolicy::None,
        }
    }
}

enum Judged {
    Absent,
    Clear,
    Flag,
    Unreadable,
}

fn judge_file(file: &Path) -> Judged {
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Judged::Absent,
        Err(_) => return Judged::Unreadable,
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.is_object() => {
            if value.get(DISABLE_SIDELOAD_FLAGS_KEY).is_some_and(truthy) {
                Judged::Flag
            } else {
                Judged::Clear
            }
        }
        _ => Judged::Unreadable,
    }
}

/// The `*.json` regular files of a drop-in directory sorted by file name; a
/// missing directory has none, any other read error is `Err`.
fn dropin_files(dir: &Path) -> Result<Vec<PathBuf>, ()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.map_err(|_| ())?.path();
        if path.extension().is_some_and(|ext| ext == "json") && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !matches!(text.trim(), "" | "0" | "false"),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Null => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedPolicy {
    None,
    /// A managed settings file sets `disableSideloadFlags`: Claude Code
    /// refuses to start with `CLAUDE_CODE_PLUGIN_DIRS`.
    DisableSideloadFlags {
        source: PathBuf,
    },
    /// A policy file or drop-in directory could not be read, or a policy
    /// file is not a JSON object: setup cannot rule out
    /// `disableSideloadFlags`, so it is treated as set.
    Unverifiable {
        source: PathBuf,
    },
}

// ---------------------------------------------------------------- settings

fn object(bytes: &[u8]) -> Result<Value, SetupError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| SetupError::Invalid)?;
    if !value.is_object() {
        return Err(SetupError::Invalid);
    }
    Ok(value)
}

fn split_dirs(value: &str) -> Vec<String> {
    value
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `env.CLAUDE_CODE_PLUGIN_DIRS` of a parsed settings object: `Ok(None)`
/// when absent, `Invalid` when `env` or the key has another type.
fn plugin_dirs_value(settings: &Value) -> Result<Option<String>, SetupError> {
    match settings.get("env") {
        None => Ok(None),
        Some(Value::Object(env)) => match env.get(PLUGIN_DIRS_ENV) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(SetupError::Invalid),
        },
        Some(_) => Err(SetupError::Invalid),
    }
}

fn serialize(value: &Value) -> Result<Vec<u8>, SetupError> {
    // Compact, like every other owned settings edit: a later hook install
    // or removal then sees the bytes it would have written itself.
    let bytes = serde_json::to_vec(value).map_err(|_| SetupError::Invalid)?;
    if bytes.len() > 1_048_576 {
        return Err(SetupError::TooLarge);
    }
    Ok(bytes)
}

/// The settings value the file holds now (`None` when absent or the file is
/// missing or unreadable). Best effort, for status.
pub fn settings_value(settings: &Path) -> Option<String> {
    let bytes = config_bytes(settings).ok()?;
    plugin_dirs_value(&object(&bytes).ok()?).ok().flatten()
}

// ---------------------------------------------------------------- manifest

/// What setup recorded when it wrote the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModManifest {
    pub version: u8,
    pub path: String,
    /// The key's value before setup wrote it; `None` when it was absent.
    pub previous: Option<String>,
    /// The mod directory setup appended.
    pub appended: String,
    /// Directories copied in from the shell's own `CLAUDE_CODE_PLUGIN_DIRS`.
    pub copied_from_shell: Vec<String>,
    /// The file's bytes before the write, restored exactly when the file
    /// still holds `after`.
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub files_fingerprint: String,
}

/// The recorded edit, if any.
pub fn recorded(manifest_path: &Path) -> Result<Option<ModManifest>, SetupError> {
    let Ok(meta) = manifest_path.symlink_metadata() else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        return Err(SetupError::Invalid);
    }
    // Two copies of a settings file of at most 1 MiB, as JSON byte arrays.
    if meta.len() > 16_777_216 {
        return Err(SetupError::TooLarge);
    }
    let bytes = fs::read(manifest_path).map_err(|_| SetupError::Io)?;
    let manifest: ModManifest = serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
    if manifest.version != 1 {
        return Err(SetupError::Invalid);
    }
    Ok(Some(manifest))
}

// ----------------------------------------------------------------- install

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModAction {
    /// Files written and the directory appended to the key.
    Installed,
    /// Recorded and in the key already; some file bytes were rewritten.
    Upgraded,
    /// Recorded, in the key, files unchanged: nothing written.
    AlreadyInstalled,
    /// The key already names the directory but setup did not record writing
    /// it: left as the user's.
    AlreadyPresent,
    /// Managed policy forbids the key: nothing written (hooks-only).
    SkippedManagedPolicy,
}

impl ModAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Upgraded => "upgraded",
            Self::AlreadyInstalled => "already_installed",
            Self::AlreadyPresent => "already_present",
            Self::SkippedManagedPolicy => "skipped_managed_policy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    pub action: ModAction,
    pub policy: ManagedPolicy,
    /// Published paths whose bytes were (re)written.
    pub files_written: Vec<&'static str>,
    /// The key's value after the call (`None` when not written or absent).
    pub value: Option<String>,
    /// Shell directories this call copied into the value.
    pub copied_from_shell: Vec<String>,
    /// A record from an earlier run exists (its edit may still be in the
    /// key, which managed policy then makes Claude Code refuse).
    pub recorded: bool,
}

pub struct InstallInput<'a> {
    pub settings: &'a Path,
    pub manifest: &'a Path,
    pub state_dir: &'a Path,
    /// The directories of the shell's `CLAUDE_CODE_PLUGIN_DIRS` the person
    /// accepted into the written value.
    pub shell_dirs: &'a [String],
    pub policy: &'a ManagedPolicySources,
}

/// Write the mod files and append the mod directory to the key. See the
/// module docs. `settings` must exist.
pub fn install(input: &InstallInput) -> Result<InstallOutcome, SetupError> {
    let settings_text = input.settings.to_str().ok_or(SetupError::Invalid)?;
    let policy = input.policy.check();
    let earlier = recorded(input.manifest)?;
    if earlier
        .as_ref()
        .is_some_and(|manifest| manifest.path != settings_text)
    {
        return Err(SetupError::Conflict);
    }
    if policy != ManagedPolicy::None {
        return Ok(InstallOutcome {
            action: ModAction::SkippedManagedPolicy,
            policy,
            files_written: Vec::new(),
            value: None,
            copied_from_shell: Vec::new(),
            recorded: earlier.is_some(),
        });
    }
    let dir = mod_dir(input.state_dir);
    let dir_text = dir.to_str().ok_or(SetupError::Invalid)?.to_owned();
    // A ':' inside the path would split into two entries.
    if dir_text.contains(':') {
        return Err(SetupError::Invalid);
    }
    let files_written = write_files(&dir)?;
    let current = config_bytes(input.settings)?;
    let mut value = object(&current)?;
    let existing = plugin_dirs_value(&value)?;
    let mut dirs = existing.as_deref().map(split_dirs).unwrap_or_default();
    if dirs.contains(&dir_text) {
        let action = match (&earlier, files_written.is_empty()) {
            (None, _) => ModAction::AlreadyPresent,
            (Some(_), true) => ModAction::AlreadyInstalled,
            (Some(_), false) => ModAction::Upgraded,
        };
        return Ok(InstallOutcome {
            action,
            policy,
            files_written,
            value: existing,
            copied_from_shell: Vec::new(),
            recorded: earlier.is_some(),
        });
    }
    let mut copied = Vec::new();
    for shell in input.shell_dirs {
        if !shell.is_empty() && !dirs.contains(shell) && !copied.contains(shell) {
            copied.push(shell.clone());
        }
    }
    dirs.extend(copied.iter().cloned());
    dirs.push(dir_text.clone());
    let new_value = dirs.join(":");
    let map = value.as_object_mut().ok_or(SetupError::Invalid)?;
    let env = map
        .entry("env")
        .or_insert_with(|| Value::Object(Default::default()));
    env.as_object_mut()
        .ok_or(SetupError::Invalid)?
        .insert(PLUGIN_DIRS_ENV.into(), Value::String(new_value.clone()));
    let written = serialize(&value)?;
    let manifest = ModManifest {
        version: 1,
        path: settings_text.into(),
        previous: existing,
        appended: dir_text,
        copied_from_shell: copied.clone(),
        before: current.clone(),
        after: written.clone(),
        files_fingerprint: files_fingerprint(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|_| SetupError::Invalid)?;
    if earlier.is_some() {
        write_replacement(input.manifest, &manifest_bytes, true)?;
    } else {
        publish_manifest(input.manifest, &manifest_bytes, false)?;
    }
    let published = (|| {
        if config_bytes(input.settings)? != current {
            return Err(SetupError::Conflict);
        }
        write_replacement(input.settings, &written, false)
    })();
    if let Err(error) = published {
        // Nothing was written: drop a record this run created, keep an earlier one.
        if earlier.is_none() {
            let _ = fs::remove_file(input.manifest);
        }
        return Err(error);
    }
    Ok(InstallOutcome {
        action: ModAction::Installed,
        policy,
        files_written,
        value: Some(new_value),
        copied_from_shell: copied,
        recorded: true,
    })
}

// ------------------------------------------------------------------ revert

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOutcome {
    /// Setup recorded no edit here (the mod files, if any, were removed).
    NotRecorded,
    /// The appended and copied directories were removed from the key (the
    /// key restored when nothing user-written remained).
    Reverted,
    /// The key no longer names the mod directory (changed by hand) or the
    /// file is gone: left alone, record dropped.
    LeftChanged,
}

impl RevertOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRecorded => "not_recorded",
            Self::Reverted => "reverted",
            Self::LeftChanged => "left_changed",
        }
    }
}

/// Undo [`install`]: remove the appended and shell-copied directories from
/// the key, restoring the file's bytes exactly when it still holds what
/// setup wrote, delete the mod files, then drop the record.
pub fn revert(
    settings: &Path,
    manifest_path: &Path,
    state_dir: &Path,
) -> Result<RevertOutcome, SetupError> {
    let outcome = lift(settings, manifest_path)?.unwrap_or(RevertOutcome::NotRecorded);
    remove_files(state_dir)?;
    Ok(outcome)
}

/// [`revert`] without deleting the mod files: undo the settings edit and
/// drop the record. `None` when setup recorded no edit. Used to take the
/// edit out of the way while the hook installer rewrites the file, so its
/// own byte-exact restore still holds; [`install`] then puts the edit back.
pub fn lift(settings: &Path, manifest_path: &Path) -> Result<Option<RevertOutcome>, SetupError> {
    let Some(manifest) = recorded(manifest_path)? else {
        return Ok(None);
    };
    if manifest.path != settings.to_str().ok_or(SetupError::Invalid)? {
        return Err(SetupError::Conflict);
    }
    let outcome = revert_key(settings, &manifest)?;
    fs::remove_file(manifest_path).map_err(|_| SetupError::Io)?;
    Ok(Some(outcome))
}

fn revert_key(settings: &Path, manifest: &ModManifest) -> Result<RevertOutcome, SetupError> {
    if settings.symlink_metadata().is_err() {
        return Ok(RevertOutcome::LeftChanged);
    }
    let current = config_bytes(settings)?;
    let mut value = object(&current)?;
    let existing = plugin_dirs_value(&value)?;
    let dirs = existing.as_deref().map(split_dirs).unwrap_or_default();
    if !dirs.contains(&manifest.appended) {
        return Ok(RevertOutcome::LeftChanged);
    }
    let restored = if current == manifest.after {
        manifest.before.clone()
    } else {
        let remaining: Vec<String> = dirs
            .into_iter()
            .filter(|dir| *dir != manifest.appended && !manifest.copied_from_shell.contains(dir))
            .collect();
        let previous_dirs = manifest.previous.as_deref().map(split_dirs);
        // `env` existed before setup wrote the key, judged by the baseline.
        let env_existed = serde_json::from_slice::<Value>(&manifest.before)
            .ok()
            .is_some_and(|before| before.get("env").is_some());
        let map = value.as_object_mut().ok_or(SetupError::Invalid)?;
        let env = map
            .get_mut("env")
            .and_then(Value::as_object_mut)
            .ok_or(SetupError::Invalid)?;
        if remaining.is_empty() {
            env.remove(PLUGIN_DIRS_ENV);
        } else if previous_dirs.as_ref() == Some(&remaining) {
            // Nothing but the user's original entries: its exact spelling.
            if let Some(previous) = &manifest.previous {
                env.insert(PLUGIN_DIRS_ENV.into(), Value::String(previous.clone()));
            }
        } else {
            env.insert(PLUGIN_DIRS_ENV.into(), Value::String(remaining.join(":")));
        }
        if env.is_empty() && !env_existed {
            map.remove("env");
        }
        serialize(&value)?
    };
    if config_bytes(settings)? != current {
        return Err(SetupError::Conflict);
    }
    write_replacement(settings, &restored, false)?;
    Ok(RevertOutcome::Reverted)
}

// ------------------------------------------------------------------ status

/// What `setup-status` reads from disk about the mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// All embedded files exist with the embedded bytes.
    pub files_current: bool,
    /// Some mod file exists (maybe an older version).
    pub files_present: bool,
    /// The settings key names the mod directory.
    pub settings_value_contains_mod_dir: bool,
    /// The key's current value.
    pub settings_value: Option<String>,
    /// Setup recorded writing the key.
    pub recorded: bool,
}

impl Inspection {
    /// Installed: current files and the key names the directory.
    pub fn installed(&self) -> bool {
        self.files_current && self.settings_value_contains_mod_dir
    }
}

pub fn inspect(settings: &Path, manifest_path: &Path, state_dir: &Path) -> Inspection {
    let dir = mod_dir(state_dir);
    let settings_value = settings_value(settings);
    let dir_text = dir.to_str().unwrap_or_default();
    Inspection {
        files_current: files_current(&dir),
        files_present: dir.symlink_metadata().is_ok(),
        settings_value_contains_mod_dir: settings_value
            .as_deref()
            .is_some_and(|value| split_dirs(value).iter().any(|d| d == dir_text)),
        settings_value,
        recorded: recorded(manifest_path).ok().flatten().is_some(),
    }
}

/// The directories of a shell `CLAUDE_CODE_PLUGIN_DIRS` value.
pub fn shell_dirs(value: &str) -> Vec<String> {
    split_dirs(value)
}

#[cfg(test)]
#[path = "../../tests/harness/claude_mod.rs"]
mod tests;
