//! Codex's pane environment: an owned, backed-up block in the user's shell
//! startup files that exports `CODEX_EXEC_SERVER_URL=` (empty) inside Herdr
//! panes.
//!
//! Codex attaches its TUI to a shared app-server daemon by default, and a
//! daemon client runs hooks and tools with the daemon's environment: the
//! `HERDR_PANE_ID` of whichever pane started it. Codex skips the implicit
//! daemon whenever `CODEX_EXEC_SERVER_URL` is set, read as presence so an
//! empty value counts, and its exec server treats an empty value as ordinary
//! local execution (only `none` disables it). The empty export therefore keeps
//! every Codex started in a Herdr pane inside that pane, whatever wrapper
//! starts it. Historical evidence (rust-v0.162.1 source, never a gate):
//! `codex-rs/tui/src/daemon_startup.rs` `exclusion()`,
//! `codex-rs/tui/src/startup_orchestration.rs`, `codex-rs/tui/src/lib.rs`
//! `app_server_target_for_launch` and
//! `codex-rs/exec-server/src/environment_provider.rs`. Herdr has no per-pane
//! environment setting and its agent start carries argv only, so the export
//! goes in the shell startup files, guarded by `HERDR_ENV=1`.
//!
//! Targets ([`targets`]): zsh `${ZDOTDIR:-$HOME}/.zshenv` (read by every zsh)
//! and bash `$HOME/.bashrc` get a marked block appended; fish gets a whole
//! drop-in, `${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/herdr-threads.fish`.
//! A private manifest under `<state>/setup/` records each file, whether setup
//! created it and the exact block written; removal takes a block out only
//! while it is byte-identical to the record. Symlinks, special files and files
//! over 1 MiB are never followed or written. Every change of an existing file
//! keeps a timestamped pre-image beside it first and is published through the
//! staged temp-and-rename replacement ([`write_user_config`],
//! [`remove_user_config`]); a file this run created gets no pre-image.
//!
//! The block is advisory foreground configuration, never execution proof or
//! attribution authority (TRUST-POLICY "Foreground harness execution").
use crate::store::connection::DurableSync;
use crate::{
    cli::{
        RunError,
        setup::{PaneEnvPolicy, SetupEnv},
    },
    harness::setup::{
        SetupError, config_bytes, legacy::manifest_path, note_created_user_config,
        remove_user_config, write_replacement, write_user_config,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Write},
    ops::Range,
    path::{Path, PathBuf},
};

/// The variable the block exports.
pub const VARIABLE: &str = "CODEX_EXEC_SERVER_URL";
const BEGIN: &str = "# >>> herdr-threads managed block";
const END: &str = "# <<< herdr-threads managed block <<<";
/// The block appended to `.zshenv` / `.bashrc`.
pub const POSIX_BLOCK: &str = "# >>> herdr-threads managed block (codex pane environment; \
     `herdr-threads unsetup codex` removes it) >>>\n\
     if [ \"${HERDR_ENV-}\" = 1 ]; then export CODEX_EXEC_SERVER_URL=; fi\n\
     # <<< herdr-threads managed block <<<\n";
/// The line to add by hand to a zsh or bash startup file.
pub const POSIX_LINE: &str = "if [ \"${HERDR_ENV-}\" = 1 ]; then export CODEX_EXEC_SERVER_URL=; fi";
/// The whole fish drop-in.
pub const FISH_FILE: &str = "# Managed by herdr-threads (codex pane environment): keeps Codex \
     inside its Herdr pane.\n\
     # `herdr-threads unsetup codex` removes this file.\n\
     if test \"$HERDR_ENV\" = 1; set -gx CODEX_EXEC_SERVER_URL \"\"; end\n";
/// The line to add by hand to a fish configuration file.
pub const FISH_LINE: &str = "if test \"$HERDR_ENV\" = 1; set -gx CODEX_EXEC_SERVER_URL \"\"; end";
/// The process environment the targets resolve from (Codex's setup environment inputs).
pub const ENVIRONMENT_INPUTS: [&str; 3] = ["SHELL", "ZDOTDIR", "XDG_CONFIG_HOME"];
const LIMIT: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
        }
    }
    /// What setup writes: a block in a shared file, or fish's whole drop-in.
    fn content(self) -> &'static str {
        match self {
            Self::Fish => FISH_FILE,
            _ => POSIX_BLOCK,
        }
    }
    fn manual(self) -> &'static str {
        match self {
            Self::Fish => FISH_LINE,
            _ => POSIX_LINE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub shell: Shell,
    pub path: PathBuf,
}

fn dir_var(value: Option<&OsStr>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// The startup files to manage: the login shell's (`basename $SHELL`, when
/// zsh, bash or fish), plus every one of the three whose startup files already
/// exist (zsh: `.zshenv` or `.zshrc`; bash: `.bashrc`, `.bash_profile` or
/// `.bash_login`; fish: its config directory). Empty or relative variables count
/// as unset; no HOME, no targets.
///
/// Herdr starts pane shells as login shells. A login bash reads the first existing of
/// `~/.bash_profile`, `~/.bash_login` and `~/.profile` (bash's own order) and not
/// `~/.bashrc` unless that file sources it, so bash gets two targets: `~/.bashrc` (non-login
/// interactive shells) and that login file, `~/.bash_profile` when none exists (so an
/// existing `~/.profile` is never shadowed). Reading the idempotent block twice is harmless.
/// zsh reads `${ZDOTDIR:-$HOME}/.zshenv` in every shell, login or not; setup uses its own
/// `ZDOTDIR`, which a Herdr server started without it does not share.
pub fn targets(
    home: Option<&OsStr>,
    zdotdir: Option<&OsStr>,
    xdg_config_home: Option<&OsStr>,
    shell: Option<&OsStr>,
) -> Vec<Target> {
    let Some(home) = dir_var(home) else {
        return Vec::new();
    };
    let login = shell
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    let exists = |path: &Path| path.symlink_metadata().is_ok();
    let mut found = Vec::new();
    let zdot = dir_var(zdotdir).unwrap_or_else(|| home.clone());
    if login == Some("zsh") || exists(&zdot.join(".zshenv")) || exists(&zdot.join(".zshrc")) {
        found.push(Target {
            shell: Shell::Zsh,
            path: zdot.join(".zshenv"),
        });
    }
    let bashrc = home.join(".bashrc");
    let login_files = [".bash_profile", ".bash_login", ".profile"].map(|name| home.join(name));
    if login == Some("bash")
        || exists(&bashrc)
        || exists(&login_files[0])
        || exists(&login_files[1])
    {
        found.push(Target {
            shell: Shell::Bash,
            path: bashrc,
        });
        let login_file = login_files
            .iter()
            .find(|path| exists(path))
            .unwrap_or(&login_files[0])
            .clone();
        found.push(Target {
            shell: Shell::Bash,
            path: login_file,
        });
    }
    let fish = dir_var(xdg_config_home)
        .unwrap_or_else(|| home.join(".config"))
        .join("fish");
    if login == Some("fish") || exists(&fish) {
        found.push(Target {
            shell: Shell::Fish,
            path: fish.join("conf.d").join("herdr-threads.fish"),
        });
    }
    found
}

fn find(bytes: &[u8], needle: &str, from: usize) -> Option<usize> {
    let needle = needle.as_bytes();
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

enum Found {
    Absent,
    /// From the begin marker's line start through the end marker and its newline.
    Block(Range<usize>),
    /// A begin marker without an end marker.
    Unterminated,
}

fn block_range(bytes: &[u8]) -> Found {
    let mut from = 0;
    let begin = loop {
        let Some(at) = find(bytes, BEGIN, from) else {
            return Found::Absent;
        };
        if at == 0 || bytes[at - 1] == b'\n' {
            break at;
        }
        from = at + 1;
    };
    let Some(end) = find(bytes, END, begin) else {
        return Found::Unterminated;
    };
    let end = end + END.len();
    Found::Block(begin..end + usize::from(bytes.get(end) == Some(&b'\n')))
}

enum Current {
    Missing,
    Bytes(Vec<u8>),
    Skipped(String),
}

fn read(path: &Path) -> Current {
    match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Current::Missing,
        Err(error) => Current::Skipped(format!("it cannot be inspected ({error})")),
        Ok(meta) if meta.file_type().is_symlink() => {
            Current::Skipped("it is a symlink, which setup never follows".into())
        }
        Ok(meta) if !meta.is_file() => Current::Skipped("it is not a regular file".into()),
        Ok(meta) if meta.len() > LIMIT => Current::Skipped("it is larger than 1 MiB".into()),
        Ok(_) => match config_bytes(path) {
            Ok(bytes) => Current::Bytes(bytes),
            Err(SetupError::TooLarge) => Current::Skipped("it is larger than 1 MiB".into()),
            Err(_) => Current::Skipped("it cannot be read".into()),
        },
    }
}

/// What a startup file holds of setup's content.
enum Holds {
    /// Exactly what setup writes now.
    Current,
    /// A different block (fish: a drop-in setup recorded writing). `owned`: it is
    /// byte-identical to what setup recorded writing, so an older version of setup's own.
    Other {
        range: Range<usize>,
        owned: bool,
    },
    Absent,
    /// Not setup's to change: the reason.
    Foreign(String),
}

fn holds(shell: Shell, bytes: &[u8], recorded: Option<&Record>) -> Holds {
    if shell == Shell::Fish {
        return if bytes == FISH_FILE.as_bytes() {
            Holds::Current
        } else if let Some(record) = recorded {
            Holds::Other {
                range: 0..bytes.len(),
                owned: record.block.as_bytes() == bytes,
            }
        } else {
            Holds::Foreign("it was not written by herdr-threads".into())
        };
    }
    match block_range(bytes) {
        Found::Block(range) if &bytes[range.clone()] == POSIX_BLOCK.as_bytes() => Holds::Current,
        Found::Block(range) => Holds::Other {
            owned: recorded.is_some_and(|record| record.block.as_bytes() == &bytes[range.clone()]),
            range,
        },
        Found::Absent => Holds::Absent,
        Found::Unterminated => Holds::Foreign(
            "it has a herdr-threads managed block begin marker without its end marker".into(),
        ),
    }
}

/// One startup file setup wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub shell: Shell,
    pub path: String,
    /// Setup created the file: removal deletes it when nothing else is left.
    pub created: bool,
    /// The exact block (fish: the whole file) setup wrote.
    pub block: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u8,
    pub files: Vec<Record>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: 1,
            files: Vec::new(),
        }
    }
}

impl Manifest {
    fn get(&self, path: &Path) -> Option<&Record> {
        self.files
            .iter()
            .find(|record| Path::new(&record.path) == path)
    }
    fn put(&mut self, record: Record) {
        self.files.retain(|old| old.path != record.path);
        self.files.push(record);
    }
}

fn read_manifest(path: &Path) -> Result<Manifest, SetupError> {
    let meta = match path.symlink_metadata() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Manifest::default()),
        Err(_) => return Err(SetupError::Io),
        Ok(meta) => meta,
    };
    if !meta.is_file() {
        return Err(SetupError::Invalid);
    }
    if meta.len() > LIMIT {
        return Err(SetupError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| SetupError::Io)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| SetupError::Invalid)?;
    if manifest.version != 1 {
        return Err(SetupError::Invalid);
    }
    Ok(manifest)
}

/// Write the manifest (private, staged replacement), or delete it once it records nothing.
fn save_manifest(path: &Path, manifest: &Manifest) -> Result<(), SetupError> {
    if manifest.files.is_empty() {
        return match fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(SetupError::Io),
            _ => Ok(()),
        };
    }
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path.parent().ok_or(SetupError::Invalid)?)
        .map_err(|_| SetupError::Io)?;
    let bytes = serde_json::to_vec(manifest).map_err(|_| SetupError::Invalid)?;
    write_replacement(path, &bytes, true)
}

/// Create `path` with `bytes` (mode 0644, before umask); a file that appeared meanwhile is a
/// conflict. The run notes the file as created, so it keeps no pre-image of it.
fn create(path: &Path, bytes: &[u8]) -> Result<(), SetupError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(path)
        .map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => SetupError::Conflict,
            _ => SetupError::Io,
        })?;
    note_created_user_config(path);
    file.write_all(bytes)
        .and_then(|()| file.durable_sync())
        .and_then(|()| fs::File::open(path.parent().unwrap_or(Path::new("/")))?.durable_sync())
        .map_err(|_| SetupError::Io)
}

/// Delete a file setup created that still holds exactly what it was created with: there is
/// nothing of the user's to keep, so no pre-image (as the hook files' created-file deletion).
fn delete_created(path: &Path, current: &[u8]) -> Result<(), SetupError> {
    if config_bytes(path)? != current {
        return Err(SetupError::Conflict);
    }
    fs::remove_file(path).map_err(|_| SetupError::Io)?;
    fs::File::open(path.parent().ok_or(SetupError::Invalid)?)
        .and_then(|dir| dir.durable_sync())
        .map_err(|_| SetupError::Io)
}

fn describe(error: &SetupError) -> String {
    match error {
        SetupError::Conflict => "it changed while setup was writing it; re-run the command",
        SetupError::Io => "it could not be written",
        SetupError::Invalid => "it is a symlink or not a regular file",
        SetupError::TooLarge => "it is larger than 1 MiB",
    }
    .into()
}

/// One startup file's state after an operation (or as inspected).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    /// Written now (status: present exactly as setup writes it).
    Installed,
    /// Already held exactly what setup writes; nothing written.
    AlreadyInstalled,
    /// Holds an older block setup recorded writing (status only; setup refreshes it).
    Outdated,
    /// Holds a block that differs from what setup recorded writing: left alone unless
    /// `--with-pane-env` replaces it.
    Edited,
    /// Holds exactly the current block, but setup has no record of writing it (for example
    /// pasted by hand): reported, never adopted without consent, and unsetup leaves it.
    Unrecorded,
    NotInstalled,
    Removed,
    /// The block was edited since setup wrote it: unsetup leaves it.
    LeftChanged,
    Skipped(String),
    Error(String),
}

impl FileState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::AlreadyInstalled => "already_installed",
            Self::Outdated => "outdated",
            Self::Edited => "edited",
            Self::Unrecorded => "present_unrecorded",
            Self::NotInstalled => "not_installed",
            Self::Removed => "removed",
            Self::LeftChanged => "left_changed",
            Self::Skipped(_) => "skipped",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub shell: Shell,
    pub path: PathBuf,
    pub state: FileState,
}

/// How far [`PaneEnv::apply`] may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No consent: refresh only blocks setup recorded writing.
    Ask,
    /// A typed yes to the terminal question: also add missing blocks and adopt an unrecorded
    /// current one, but never replace an edited block.
    Consent,
    /// `--with-pane-env`: everything, including replacing an edited block.
    Grant,
}

/// The component over its startup files and its manifest.
pub struct PaneEnv {
    pub targets: Vec<Target>,
    pub manifest: PathBuf,
}

impl PaneEnv {
    /// Each target's state, read only; files setup recorded elsewhere are included.
    pub fn status(&self) -> Result<Vec<FileReport>, SetupError> {
        let manifest = read_manifest(&self.manifest)?;
        let mut targets = self.targets.clone();
        for record in &manifest.files {
            if !targets.iter().any(|t| t.path == Path::new(&record.path)) {
                targets.push(Target {
                    shell: record.shell,
                    path: PathBuf::from(&record.path),
                });
            }
        }
        Ok(targets
            .into_iter()
            .map(|target| {
                let state = match read(&target.path) {
                    Current::Missing => FileState::NotInstalled,
                    Current::Skipped(reason) => FileState::Skipped(reason),
                    Current::Bytes(bytes) => {
                        match holds(target.shell, &bytes, manifest.get(&target.path)) {
                            Holds::Current
                                if manifest
                                    .get(&target.path)
                                    .is_some_and(|r| r.block == target.shell.content()) =>
                            {
                                FileState::Installed
                            }
                            Holds::Current => FileState::Unrecorded,
                            Holds::Other { owned: true, .. } => FileState::Outdated,
                            Holds::Other { owned: false, .. } => FileState::Edited,
                            Holds::Absent => FileState::NotInstalled,
                            Holds::Foreign(reason) => FileState::Skipped(reason),
                        }
                    }
                };
                FileReport {
                    shell: target.shell,
                    path: target.path,
                    state,
                }
            })
            .collect())
    }

    /// Bring every target up to date under `mode`. An older block setup recorded writing is
    /// refreshed in place in every mode. A file without the block gets it, and an unrecorded
    /// current block is adopted (recorded, so removal owns it), only with consent
    /// ([`Mode::Consent`] or [`Mode::Grant`]). A block that differs from setup's record is
    /// replaced only under [`Mode::Grant`] (`--with-pane-env`).
    pub fn apply(&self, mode: Mode) -> Result<Vec<FileReport>, SetupError> {
        let consent = mode != Mode::Ask;
        let grant = mode == Mode::Grant;
        let mut manifest = read_manifest(&self.manifest)?;
        let mut reports = Vec::new();
        for target in &self.targets {
            let content = target.shell.content();
            let state = match read(&target.path) {
                Current::Skipped(reason) => FileState::Skipped(reason),
                Current::Missing if !consent => FileState::NotInstalled,
                Current::Missing => {
                    let parent = target.path.parent().ok_or(SetupError::Invalid)?;
                    if target.shell == Shell::Fish {
                        // conf.d under an existing (or the login shell's) fish config directory.
                        let _ = fs::create_dir_all(parent);
                    }
                    if parent.is_dir() {
                        self.write(&mut manifest, target, None, content.as_bytes())
                    } else {
                        FileState::Skipped(format!(
                            "its directory {} does not exist",
                            parent.display()
                        ))
                    }
                }
                Current::Bytes(bytes) => {
                    match holds(target.shell, &bytes, manifest.get(&target.path)) {
                        Holds::Current
                            if !consent
                                && manifest
                                    .get(&target.path)
                                    .is_none_or(|r| r.block != target.shell.content()) =>
                        {
                            FileState::Unrecorded
                        }
                        Holds::Current => self.record_current(&mut manifest, target),
                        Holds::Other { owned: false, .. } if !grant => FileState::Edited,
                        Holds::Other { range, .. } => {
                            let mut next = bytes.clone();
                            next.splice(range, content.bytes());
                            self.write(&mut manifest, target, Some(&bytes), &next)
                        }
                        Holds::Absent if !consent => FileState::NotInstalled,
                        Holds::Absent => {
                            let mut next = bytes.clone();
                            if !next.is_empty() && !next.ends_with(b"\n") {
                                next.push(b'\n');
                            }
                            next.extend_from_slice(content.as_bytes());
                            self.write(&mut manifest, target, Some(&bytes), &next)
                        }
                        Holds::Foreign(reason) => FileState::Skipped(reason),
                    }
                }
            };
            reports.push(FileReport {
                shell: target.shell,
                path: target.path.clone(),
                state,
            });
        }
        Ok(reports)
    }

    fn record(target: &Target, created: bool) -> Record {
        Record {
            shell: target.shell,
            path: target.path.display().to_string(),
            created,
            block: target.shell.content().into(),
        }
    }

    fn record_current(&self, manifest: &mut Manifest, target: &Target) -> FileState {
        let created = manifest.get(&target.path).is_some_and(|r| r.created);
        let record = Self::record(target, created);
        if manifest.get(&target.path) != Some(&record) {
            manifest.put(record);
            if let Err(error) = save_manifest(&self.manifest, manifest) {
                return FileState::Error(format!(
                    "its ownership record could not be written ({})",
                    describe(&error)
                ));
            }
        }
        FileState::AlreadyInstalled
    }

    /// Record the write, then publish `bytes` over the validated `current` (pre-image kept,
    /// staged replacement) or create the file; a failed write drops the new record again.
    fn write(
        &self,
        manifest: &mut Manifest,
        target: &Target,
        current: Option<&[u8]>,
        bytes: &[u8],
    ) -> FileState {
        let created = current.is_none() || manifest.get(&target.path).is_some_and(|r| r.created);
        let previous = manifest.clone();
        manifest.put(Self::record(target, created));
        let written = save_manifest(&self.manifest, manifest).and_then(|()| match current {
            Some(current) => write_user_config(&target.path, current, bytes),
            None => create(&target.path, bytes),
        });
        match written {
            Ok(()) => FileState::Installed,
            Err(error) => {
                *manifest = previous;
                let _ = save_manifest(&self.manifest, manifest);
                FileState::Error(describe(&error))
            }
        }
    }

    /// Take out every recorded block that is still byte-identical; delete a file setup
    /// created once nothing else is left in it, and a recorded fish drop-in that still holds
    /// exactly the recorded content (a pre-image kept when setup did not create it). A record is dropped unless its file could
    /// not be written (so a re-run can finish it).
    pub fn remove(&self) -> Result<Vec<FileReport>, SetupError> {
        let mut manifest = read_manifest(&self.manifest)?;
        let mut kept = Vec::new();
        let mut reports = Vec::new();
        for record in &manifest.files {
            let path = PathBuf::from(&record.path);
            let state = match read(&path) {
                Current::Missing => FileState::NotInstalled,
                Current::Skipped(reason) => FileState::Skipped(reason),
                Current::Bytes(bytes) => {
                    let rest = if record.shell == Shell::Fish {
                        (bytes == record.block.as_bytes()).then(Vec::new)
                    } else {
                        match block_range(&bytes) {
                            Found::Block(range)
                                if bytes[range.clone()] == *record.block.as_bytes() =>
                            {
                                let mut rest = bytes.clone();
                                rest.drain(range);
                                Some(rest)
                            }
                            Found::Absent => {
                                reports.push(FileReport {
                                    shell: record.shell,
                                    path,
                                    state: FileState::NotInstalled,
                                });
                                continue;
                            }
                            _ => None,
                        }
                    };
                    match rest {
                        None => FileState::LeftChanged,
                        Some(rest) => {
                            let removed = if rest.trim_ascii().is_empty()
                                && (record.created || record.shell == Shell::Fish)
                            {
                                if record.created && bytes == record.block.as_bytes() {
                                    delete_created(&path, &bytes)
                                } else {
                                    remove_user_config(&path, &bytes)
                                }
                            } else {
                                write_user_config(&path, &bytes, &rest)
                            };
                            match removed {
                                Ok(()) => FileState::Removed,
                                Err(error) => FileState::Error(describe(&error)),
                            }
                        }
                    }
                }
            };
            if matches!(state, FileState::Error(_)) {
                kept.push(record.clone());
            }
            reports.push(FileReport {
                shell: record.shell,
                path,
                state,
            });
        }
        manifest.files = kept;
        save_manifest(&self.manifest, &manifest)?;
        Ok(reports)
    }
}

// ------------------------------------------------------------- setup glue

fn declared<'a>(
    declared: &'a std::collections::BTreeMap<String, std::ffi::OsString>,
    name: &str,
) -> Option<&'a OsStr> {
    declared.get(name).map(|value| value.as_os_str())
}

fn env_targets(env: &SetupEnv) -> Vec<Target> {
    let var = |name| declared(&env.declared_environment, name);
    targets(
        env.home.as_deref(),
        var("ZDOTDIR"),
        var("XDG_CONFIG_HOME"),
        var("SHELL"),
    )
}

/// The component of this HOME, shell environment and state directory.
pub(crate) fn component(env: &SetupEnv) -> Result<PaneEnv, RunError> {
    let home = env.home.as_deref();
    Ok(PaneEnv {
        targets: env_targets(env),
        manifest: manifest_path(
            env.state_dir()?,
            "codex-pane-env",
            home.map_or(Path::new(""), Path::new),
        ),
    })
}

/// Whether some target startup file holds exactly what setup writes (read only, without
/// the manifest): foreground inspection and the launch report read it.
pub(crate) fn installed_in(env: &SetupEnv) -> bool {
    installed(&env_targets(env))
}

fn installed(targets: &[Target]) -> bool {
    targets.iter().any(|target| {
        matches!(read(&target.path), Current::Bytes(bytes)
            if matches!(holds(target.shell, &bytes, None), Holds::Current))
    })
}

/// Foreground inspection hook ([`crate::harness::adapter::ForegroundAdvice`]): a present
/// block configures the foreground choice. Advisory only: wrappers, other shells and starts
/// outside a Herdr pane are not covered, so `execution` stays unknown.
pub(crate) fn inspect_foreground_environment(
    environment: &crate::harness::adapter::SetupEnvironment,
    report: &mut Value,
) {
    let var = |name| declared(&environment.declared, name);
    let targets = targets(
        environment.home.as_deref(),
        var("ZDOTDIR"),
        var("XDG_CONFIG_HOME"),
        var("SHELL"),
    );
    let installed = installed(&targets);
    report["pane_env"] = json!(if installed {
        "installed"
    } else {
        "not_installed"
    });
    if installed {
        report["status"] = json!("configured");
        // Setup's own change only when its ownership record names a file holding the block.
        let manifest = environment.state_dir.as_deref().map(|state| {
            manifest_path(
                state,
                "codex-pane-env",
                environment.home.as_deref().map_or(Path::new(""), Path::new),
            )
        });
        if manifest.is_some_and(|manifest| owned_current(&targets, &manifest)) {
            report["modified_by_setup"] = json!(true);
        }
    }
}

/// Whether the ownership record names a target that holds exactly the current block.
fn owned_current(targets: &[Target], manifest: &Path) -> bool {
    let Ok(manifest) = read_manifest(manifest) else {
        return false;
    };
    targets.iter().any(|target| {
        manifest
            .get(&target.path)
            .is_some_and(|record| record.block == target.shell.content())
            && matches!(read(&target.path), Current::Bytes(bytes)
                if matches!(holds(target.shell, &bytes, None), Holds::Current))
    })
}

fn file_list(targets: &[Target]) -> String {
    targets
        .iter()
        .map(|target| target.path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

const WHY: &str = "Codex's shared app-server daemon runs tools and hooks with the environment \
     of the pane that started it, so herdr-threads would attribute them to the wrong pane. \
     Exporting CODEX_EXEC_SERVER_URL= (empty) inside Herdr panes keeps Codex inside its own \
     pane; it applies only inside Herdr panes and takes effect in newly opened panes.";

/// The files an advised report talks about, by what setup could not do for them.
pub(crate) struct AdviceFiles<'a> {
    /// No block yet: `--with-pane-env` (or a terminal yes) adds it.
    pub missing: &'a [Target],
    /// A block that differs from setup's record: only `--with-pane-env` replaces it.
    pub edited: &'a [Target],
    /// Never written by setup (symlink, special, oversize, foreign drop-in, ...).
    pub skipped: &'a [Target],
}

/// The advice when setup did not configure every file: the lines to add by hand first (so a
/// truncated rendering never cuts them), then what setup can do for each file it named. The
/// why lives once in the foreground advice, not here.
pub(crate) fn advice(files: &AdviceFiles<'_>, zdotdir: Option<&Path>) -> String {
    let mut text =
        format!("Add to your shell startup file: zsh/bash `{POSIX_LINE}`; fish `{FISH_LINE}`.");
    if files.missing.is_empty() && files.edited.is_empty() && files.skipped.is_empty() {
        text.push_str(" No zsh, bash or fish startup file was found for this account.");
    }
    if !files.missing.is_empty() {
        text.push_str(&format!(
            " `herdr-threads setup codex --with-pane-env` adds the block to {}",
            file_list(files.missing)
        ));
        // Created only when no bash login file exists; bash then reads it instead of a
        // ~/.profile that appears later.
        if let Some(created) = files.missing.iter().find(|target| {
            target.path.file_name() == Some(OsStr::new(".bash_profile"))
                && target.path.symlink_metadata().is_err()
        }) {
            text.push_str(&format!(
                " (creating {}, which a login bash then reads instead of a later ~/.profile)",
                created.path.display()
            ));
        }
        text.push('.');
    }
    if !files.edited.is_empty() {
        text.push_str(&format!(
            " Your edited block in {} is kept; `--with-pane-env` replaces it.",
            file_list(files.edited)
        ));
    }
    if !files.skipped.is_empty() {
        text.push_str(&format!(
            " Setup cannot change {}: add the line there yourself.",
            file_list(files.skipped)
        ));
    }
    if let Some(dir) = zdotdir {
        text.push_str(&format!(
            " zsh files follow this shell's ZDOTDIR ({}); a Herdr server started without it \
             reads ~/.zshenv.",
            dir.display()
        ));
    }
    text
}

/// `ZDOTDIR` when it is set and differs from HOME (the note then says so).
fn distinct_zdotdir(env: &SetupEnv) -> Option<PathBuf> {
    let zdot = dir_var(declared(&env.declared_environment, "ZDOTDIR"))?;
    (dir_var(env.home.as_deref()).as_deref() != Some(zdot.as_path())).then_some(zdot)
}

/// The consent question, naming the files a yes writes and those whose existing block it
/// records as setup-owned.
pub(crate) fn question(missing: &[Target], adopt: &[Target]) -> String {
    let adopt = if adopt.is_empty() {
        String::new()
    } else {
        format!(
            ", and record the existing block in {} as setup-owned",
            file_list(adopt)
        )
    };
    format!(
        "{WHY}\nAdd a guarded `{VARIABLE}=` block to {}{adopt}? [y/N] ",
        file_list(missing)
    )
}

fn files_json(reports: &[FileReport], removing: bool) -> Value {
    json!(
        reports
            .iter()
            .map(|report| {
                let mut file = json!({
                    "shell": report.shell.as_str(),
                    "path": report.path.display().to_string(),
                    "state": report.state.as_str(),
                });
                match &report.state {
                    FileState::Skipped(reason) if removing => {
                        file["reason"] = json!(format!(
                            "{reason}; remove the herdr-threads managed block from it yourself"
                        ));
                    }
                    FileState::Skipped(reason) => {
                        file["reason"] = json!(format!(
                            "{reason}; add this line to it yourself: {}",
                            report.shell.manual()
                        ));
                    }
                    FileState::Error(reason) => file["reason"] = json!(reason),
                    _ => {}
                }
                file
            })
            .collect::<Vec<_>>()
    )
}

fn error_json(error: &str) -> Value {
    json!({"variable": VARIABLE, "state": "error", "error": error, "files": []})
}

/// Warnings for files skipped or failed by an operation.
fn file_warnings(reports: &[FileReport]) -> Vec<String> {
    reports
        .iter()
        .filter_map(|report| match &report.state {
            FileState::Skipped(reason) | FileState::Error(reason) => Some(format!(
                "pane environment: {} not changed: {reason}; add this line to it yourself: {}",
                report.path.display(),
                report.shell.manual()
            )),
            _ => None,
        })
        .collect()
}

fn manifest_error(component: &PaneEnv, error: &SetupError) -> String {
    format!(
        "the ownership record {} is unreadable or not a regular file ({}); nothing was changed",
        component.manifest.display(),
        match error {
            SetupError::Io => "I/O error",
            SetupError::TooLarge => "too large",
            _ => "invalid",
        }
    )
}

/// `setup codex`: grant writes every target, ask keeps (refreshes) what setup owns and
/// otherwise advises, advise only reports, decline removes. Never an error: a problem is
/// reported in the object and as a warning.
pub(crate) fn install_report(env: &SetupEnv, policy: PaneEnvPolicy) -> (Value, Vec<String>) {
    match policy {
        PaneEnvPolicy::Ask => report_with(env, Some(Mode::Ask)),
        PaneEnvPolicy::Grant => report_with(env, Some(Mode::Grant)),
        PaneEnvPolicy::Advise => report_with(env, None),
        PaneEnvPolicy::Decline => {
            let report = match component(env) {
                Ok(component) => remove_json(&component),
                Err(error) => error_json(&error.to_string()),
            };
            let warnings = report["error"]
                .as_str()
                .map(|e| format!("pane environment: {e}"))
                .into_iter()
                .collect();
            (report, warnings)
        }
    }
}

/// Apply `mode`, or with `None` only report (the installer's advise policy).
fn report_with(env: &SetupEnv, mode: Option<Mode>) -> (Value, Vec<String>) {
    let component = match component(env) {
        Ok(component) => component,
        Err(error) => {
            let error = error.to_string();
            return (
                error_json(&error),
                vec![format!("pane environment: {error}")],
            );
        }
    };
    let reports = match mode {
        None => component.status(),
        Some(mode) => component.apply(mode),
    };
    let reports = match reports {
        Ok(reports) => reports,
        Err(error) => {
            let error = manifest_error(&component, &error);
            return (
                error_json(&error),
                vec![format!("pane environment: {error}")],
            );
        }
    };
    let any = |state: FileState| reports.iter().any(|report| report.state == state);
    let present =
        any(FileState::Installed) || any(FileState::AlreadyInstalled) || any(FileState::Unrecorded);
    let of = |keep: &dyn Fn(&FileState) -> bool| -> Vec<Target> {
        reports
            .iter()
            .filter(|report| keep(&report.state))
            .map(|report| Target {
                shell: report.shell,
                path: report.path.clone(),
            })
            .collect()
    };
    let missing = of(&|state| *state == FileState::NotInstalled);
    let edited = of(&|state| *state == FileState::Edited);
    let skipped = of(&|state| matches!(state, FileState::Skipped(_) | FileState::Error(_)));
    let action = match mode {
        Some(Mode::Grant | Mode::Consent) if any(FileState::Installed) => "installed",
        Some(Mode::Grant | Mode::Consent) if present => "already_installed",
        // Without consent a file still lacking the block is offered, even beside a kept one.
        Some(Mode::Ask) | None if present && missing.is_empty() => "kept",
        _ => "advised",
    };
    let warnings = file_warnings(&reports);
    let mut report = json!({
        "action": action,
        "variable": VARIABLE,
        "files": files_json(&reports, false),
    });
    if action == "advised" {
        report["note"] = json!(advice(
            &AdviceFiles {
                missing: &missing,
                edited: &edited,
                skipped: &skipped,
            },
            distinct_zdotdir(env).as_deref()
        ));
    }
    (report, warnings)
}

fn remove_json(component: &PaneEnv) -> Value {
    match component.remove() {
        Ok(reports) => {
            let any = |state: FileState| reports.iter().any(|report| report.state == state);
            json!({
                "action": if any(FileState::Removed) {
                    "removed"
                } else if any(FileState::LeftChanged) {
                    "left_changed"
                } else {
                    "not_installed"
                },
                "variable": VARIABLE,
                "files": files_json(&reports, true),
            })
        }
        Err(error) => error_json(&manifest_error(component, &error)),
    }
}

/// `unsetup codex`: remove the recorded blocks; never an error.
pub(crate) fn remove_report(env: &SetupEnv) -> Value {
    match component(env) {
        Ok(component) => remove_json(&component),
        Err(error) => error_json(&error.to_string()),
    }
}

/// `setup-status codex`: each target's state, read only.
pub(crate) fn status_report(env: &SetupEnv) -> Value {
    let component = match component(env) {
        Ok(component) => component,
        Err(error) => return error_json(&error.to_string()),
    };
    match component.status() {
        Ok(reports) => json!({
            "state": if reports
                .iter()
                .any(|r| matches!(r.state, FileState::Installed | FileState::Unrecorded))
            {
                "installed"
            } else {
                "not_installed"
            },
            "variable": VARIABLE,
            "files": files_json(&reports, false),
        }),
        Err(error) => error_json(&manifest_error(&component, &error)),
    }
}

/// Ask on a terminal when setup only advised and some file could take the block; only a
/// typed yes writes. Called only for an interactive `setup` without a pane-env flag.
pub fn settle<R: io::BufRead + ?Sized, W: io::Write + ?Sized>(
    env: &SetupEnv,
    report: &mut Value,
    input: &mut R,
    out: &mut W,
) -> Result<(), RunError> {
    if report["pane_env"]["action"] != "advised"
        || !report["pane_env"]["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file["state"] == "not_installed"))
    {
        return Ok(());
    }
    let component = component(env)?;
    let in_state = |state: &str| -> Vec<Target> {
        component
            .targets
            .iter()
            .filter(|target| {
                report["pane_env"]["files"].as_array().is_some_and(|files| {
                    files.iter().any(|file| {
                        file["state"] == state && file["path"].as_str() == target.path.to_str()
                    })
                })
            })
            .cloned()
            .collect()
    };
    // A yes also adopts an unrecorded current block, so the question names those files too.
    write!(
        out,
        "{}",
        question(&in_state("not_installed"), &in_state("present_unrecorded"))
    )?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let yes = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    if yes {
        // A yes adds missing blocks and keeps an edited one: only --with-pane-env replaces it.
        let (pane_env, warnings) = report_with(env, Some(Mode::Consent));
        report["pane_env"] = pane_env;
        if !report["warnings"].is_array() {
            report["warnings"] = json!([]);
        }
        if let Some(list) = report["warnings"].as_array_mut() {
            list.extend(warnings.into_iter().map(Value::from));
        }
        crate::cli::setup::foreground::attach(report, crate::harness::context::Harness::Codex, env);
    } else {
        report["pane_env"]["action"] = json!("declined");
        if let Some(object) = report["pane_env"].as_object_mut() {
            object.remove("note");
        }
    }
    Ok(())
}

/// The one-line text summary; `None` when there is nothing to say.
pub(crate) fn line(value: &Value) -> Option<String> {
    let files = || {
        value["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|file| {
                format!(
                    "{} ({})",
                    file["path"].as_str().unwrap_or_default(),
                    file["state"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let text = match (value["action"].as_str(), value["state"].as_str()) {
        (_, Some("error")) => format!("error: {}", value["error"].as_str().unwrap_or_default()),
        (Some("installed"), _) => format!("installed (`{VARIABLE}=` in Herdr panes): {}", files()),
        (Some("already_installed" | "kept"), _) => format!("installed: {}", files()),
        (Some("advised"), _) => {
            format!("advised: {}", value["note"].as_str().unwrap_or_default())
        }
        (Some("declined"), _) => "declined; no startup file changed".to_owned(),
        (Some("removed"), _) => format!("removed: {}", files()),
        (Some("left_changed"), _) => format!("left in place (edited since setup): {}", files()),
        (Some("not_installed"), _) => return None,
        (None, Some("installed")) => format!("installed: {}", files()),
        (None, Some("not_installed")) => "not installed".to_owned(),
        _ => return None,
    };
    Some(format!(
        "pane environment: {}",
        crate::cli::setup::scalar(&json!(text))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::setup::user_config_backups;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn dir() -> Dir {
        let path =
            crate::test_support::scratch::temp_path(format!("pane-env-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Dir(path)
    }
    fn zsh(d: &Path) -> (PaneEnv, PathBuf) {
        let path = d.join(".zshenv");
        (
            PaneEnv {
                targets: vec![Target {
                    shell: Shell::Zsh,
                    path: path.clone(),
                }],
                manifest: d.join("state/setup/m.json"),
            },
            path,
        )
    }
    fn states(reports: &[FileReport]) -> Vec<FileState> {
        reports.iter().map(|r| r.state.clone()).collect()
    }

    /// Kills: a changed block text (the unsetup byte match and docs depend on it) or a fish
    /// drop-in that does not end with the manual line.
    #[test]
    fn block_text_is_exact() {
        assert_eq!(
            POSIX_BLOCK,
            "# >>> herdr-threads managed block (codex pane environment; `herdr-threads unsetup \
             codex` removes it) >>>\nif [ \"${HERDR_ENV-}\" = 1 ]; then export \
             CODEX_EXEC_SERVER_URL=; fi\n# <<< herdr-threads managed block <<<\n"
        );
        assert!(POSIX_BLOCK.contains(&format!("\n{POSIX_LINE}\n")));
        assert!(FISH_FILE.ends_with(&format!("{FISH_LINE}\n")));
    }

    /// Kills: ignoring $SHELL, ZDOTDIR or XDG_CONFIG_HOME, missing an existing startup file,
    /// or honouring a relative/empty HOME.
    #[test]
    fn targets_follow_shell_and_existing_files() {
        let d = dir();
        let home = d.0.join("home");
        let zdot = d.0.join("zdot");
        let xdg = d.0.join("xdg");
        fs::create_dir_all(&home).unwrap();
        let t = |shell: &str| {
            targets(
                Some(home.as_os_str()),
                Some(zdot.as_os_str()),
                Some(xdg.as_os_str()),
                Some(OsStr::new(shell)),
            )
        };
        assert_eq!(
            t("/bin/zsh"),
            [Target {
                shell: Shell::Zsh,
                path: zdot.join(".zshenv")
            }]
        );
        // Login bash (Herdr panes) reads the first existing login file; with none,
        // ~/.bash_profile, which would be created.
        let bash = |login: &str| {
            vec![
                Target {
                    shell: Shell::Bash,
                    path: home.join(".bashrc"),
                },
                Target {
                    shell: Shell::Bash,
                    path: home.join(login),
                },
            ]
        };
        assert_eq!(t("/usr/local/bin/bash"), bash(".bash_profile"));
        // ~/.profile alone does not select bash (sh and zsh users have one too) ...
        fs::write(home.join(".profile"), "").unwrap();
        assert!(t("/usr/bin/tcsh").is_empty());
        // ... but is never shadowed: it is the login file when it is the first existing.
        assert_eq!(t("/bin/bash"), bash(".profile"));
        fs::write(home.join(".bash_login"), "").unwrap();
        assert_eq!(t("/usr/bin/tcsh"), bash(".bash_login"));
        fs::write(home.join(".bash_profile"), "").unwrap();
        assert_eq!(t("/bin/bash"), bash(".bash_profile"));
        for name in [".profile", ".bash_login", ".bash_profile"] {
            fs::remove_file(home.join(name)).unwrap();
        }
        assert_eq!(
            t("/opt/fish"),
            [Target {
                shell: Shell::Fish,
                path: xdg.join("fish/conf.d/herdr-threads.fish")
            }]
        );
        fs::write(home.join(".bashrc"), "").unwrap();
        fs::create_dir_all(&zdot).unwrap();
        fs::write(zdot.join(".zshrc"), "").unwrap();
        let found: Vec<Shell> = t("").iter().map(|t| t.shell).collect();
        assert_eq!(found, [Shell::Zsh, Shell::Bash, Shell::Bash]);
        fs::create_dir_all(xdg.join("fish")).unwrap();
        let found: Vec<Shell> = t("/bin/sh").iter().map(|t| t.shell).collect();
        assert_eq!(found, [Shell::Zsh, Shell::Bash, Shell::Bash, Shell::Fish]);
        // No (or a relative) HOME: nothing.
        assert!(targets(None, None, None, Some(OsStr::new("zsh"))).is_empty());
        assert!(targets(Some(OsStr::new("rel")), None, None, Some(OsStr::new("zsh"))).is_empty());
        // ZDOTDIR and XDG_CONFIG_HOME empty or unset: HOME and ~/.config.
        let defaults = targets(
            Some(home.as_os_str()),
            Some(OsStr::new("")),
            None,
            Some(OsStr::new("zsh")),
        );
        assert_eq!(defaults[0].path, home.join(".zshenv"));
        let fish = targets(
            Some(home.as_os_str()),
            None,
            Some(OsStr::new("relative")),
            Some(OsStr::new("fish")),
        );
        assert_eq!(
            fish.last().unwrap().path,
            home.join(".config/fish/conf.d/herdr-threads.fish")
        );
    }

    /// Kills: a backup of a file setup created, a second write or backup on re-run, a lost
    /// trailing newline, a pre-image other than the original bytes, or writing past a begin
    /// marker that has no end marker.
    #[test]
    fn install_into_missing_existing_and_unterminated_files() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        let _session = crate::harness::setup::BackupSession::start();
        assert_eq!(
            states(&component.apply(Mode::Grant).unwrap()),
            [FileState::Installed]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), POSIX_BLOCK);
        assert!(user_config_backups(&path).is_empty(), "nothing to back up");
        drop(_session);
        // Idempotent: no write, no backup.
        assert_eq!(
            states(&component.apply(Mode::Grant).unwrap()),
            [FileState::AlreadyInstalled]
        );
        assert!(user_config_backups(&path).is_empty());
        fs::remove_file(&path).unwrap();
        fs::remove_file(&component.manifest).unwrap();

        // No trailing newline: one is added before the block; the old bytes are backed up once.
        fs::write(&path, "export A=1").unwrap();
        component.apply(Mode::Grant).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("export A=1\n{POSIX_BLOCK}")
        );
        assert_eq!(user_config_backups(&path), [b"export A=1".to_vec()]);

        // A begin marker without its end: skipped, untouched.
        let other = d.0.join("other");
        fs::create_dir(&other).unwrap();
        let (component, path) = zsh(&other);
        fs::write(&path, format!("{BEGIN} x\n")).unwrap();
        let reports = component.apply(Mode::Grant).unwrap();
        assert!(matches!(reports[0].state, FileState::Skipped(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{BEGIN} x\n"));
        assert!(user_config_backups(&path).is_empty());
    }

    /// Kills: refreshing a block the user edited without explicit consent, not refreshing
    /// setup's own older block, or adding a missing block on the default policy.
    #[test]
    fn an_owned_older_block_is_refreshed_and_an_edited_one_needs_consent() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        let old_block = format!("{BEGIN} (old) >>>\nexport X=1\n{END}\n");
        let old = format!("a\n{old_block}b\n");
        fs::write(&path, &old).unwrap();
        // Unrecorded and different: edited, left alone by the default policy.
        assert_eq!(
            states(&component.apply(Mode::Ask).unwrap()),
            [FileState::Edited]
        );
        assert_eq!(states(&component.status().unwrap()), [FileState::Edited]);
        assert_eq!(fs::read_to_string(&path).unwrap(), old);
        // Recorded as setup's own older block: refreshed in place by the default policy.
        let mut manifest = Manifest::default();
        manifest.put(Record {
            shell: Shell::Zsh,
            path: path.display().to_string(),
            created: false,
            block: old_block,
        });
        save_manifest(&component.manifest, &manifest).unwrap();
        assert_eq!(states(&component.status().unwrap()), [FileState::Outdated]);
        assert_eq!(
            states(&component.apply(Mode::Ask).unwrap()),
            [FileState::Installed]
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("a\n{POSIX_BLOCK}b\n")
        );
        assert_eq!(user_config_backups(&path), [old.into_bytes()]);
        // An edited block is replaced only with consent.
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("URL=;", "URL=x;");
        fs::write(&path, &edited).unwrap();
        assert_eq!(
            states(&component.apply(Mode::Ask).unwrap()),
            [FileState::Edited]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        assert_eq!(
            states(&component.apply(Mode::Grant).unwrap()),
            [FileState::Installed]
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("a\n{POSIX_BLOCK}b\n")
        );
        // The default policy never adds a missing block.
        fs::create_dir(d.0.join("fresh")).unwrap();
        let (fresh, fresh_path) = zsh(&d.0.join("fresh"));
        fs::write(&fresh_path, "x\n").unwrap();
        assert_eq!(
            states(&fresh.apply(Mode::Ask).unwrap()),
            [FileState::NotInstalled]
        );
        assert_eq!(fs::read_to_string(&fresh_path).unwrap(), "x\n");
        assert!(!fresh.manifest.exists());
    }

    /// Kills: following a symlink, writing a FIFO or an oversize file, or recording a skip.
    #[test]
    fn symlinked_special_and_oversize_targets_are_skipped() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        let real = d.0.join("real");
        fs::write(&real, "x\n").unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        let reports = component.apply(Mode::Grant).unwrap();
        assert!(
            matches!(&reports[0].state, FileState::Skipped(reason) if reason.contains("symlink"))
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "x\n");
        assert!(!component.manifest.exists());
        fs::remove_file(&path).unwrap();

        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: the NUL-terminated pathname remains alive for this call.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let reports = component.apply(Mode::Grant).unwrap();
        assert!(
            matches!(&reports[0].state, FileState::Skipped(reason) if reason.contains("regular"))
        );
        fs::remove_file(&path).unwrap();

        fs::File::create(&path).unwrap().set_len(LIMIT + 1).unwrap();
        let reports = component.apply(Mode::Grant).unwrap();
        assert!(
            matches!(&reports[0].state, FileState::Skipped(reason) if reason.contains("1 MiB"))
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), LIMIT + 1);
        assert!(!component.manifest.exists());
        let warnings = file_warnings(&reports);
        assert!(warnings[0].contains(POSIX_LINE), "{warnings:?}");
    }

    /// Kills: unsetup leaving the block, losing the user's surrounding lines, removing an
    /// edited block, or keeping a stale record.
    #[test]
    fn revert_is_exact_and_leaves_an_edited_block() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        fs::write(&path, "before\n").unwrap();
        component.apply(Mode::Grant).unwrap();
        fs::write(
            &path,
            format!("{}after\n", fs::read_to_string(&path).unwrap()),
        )
        .unwrap();
        assert_eq!(states(&component.remove().unwrap()), [FileState::Removed]);
        assert_eq!(fs::read_to_string(&path).unwrap(), "before\nafter\n");
        assert!(!component.manifest.exists());
        assert_eq!(component.remove().unwrap(), []);
        // Without a session each mutation keeps its pre-image: the original, then the
        // edited file with the block.
        assert_eq!(
            user_config_backups(&path),
            [
                b"before\n".to_vec(),
                format!("before\n{POSIX_BLOCK}after\n").into_bytes()
            ]
        );

        component.apply(Mode::Grant).unwrap();
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("CODEX_EXEC_SERVER_URL=;", "CODEX_EXEC_SERVER_URL=x;");
        fs::write(&path, &edited).unwrap();
        assert_eq!(
            states(&component.remove().unwrap()),
            [FileState::LeftChanged]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        assert!(!component.manifest.exists());
    }

    /// Kills: leaving behind a file setup created, deleting user content left in it, or a
    /// pre-image of a created file that still holds exactly the block.
    #[test]
    fn a_created_file_is_deleted_when_only_whitespace_is_left() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        component.apply(Mode::Grant).unwrap();
        assert_eq!(states(&component.remove().unwrap()), [FileState::Removed]);
        assert!(!path.exists());
        assert!(user_config_backups(&path).is_empty());
        component.apply(Mode::Grant).unwrap();
        fs::write(&path, format!("\n{POSIX_BLOCK}\n")).unwrap();
        assert_eq!(states(&component.remove().unwrap()), [FileState::Removed]);
        assert!(!path.exists());
        assert_eq!(
            user_config_backups(&path),
            [format!("\n{POSIX_BLOCK}\n").into_bytes()]
        );
        // A created file with other content left keeps the content.
        component.apply(Mode::Grant).unwrap();
        fs::write(&path, format!("{POSIX_BLOCK}keep\n")).unwrap();
        component.remove().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep\n");
    }

    /// Kills: replacing a foreign drop-in, leaving setup's, or deleting an edited one.
    #[test]
    fn fish_drop_in_is_created_and_deleted_whole() {
        let d = dir();
        let path = d.0.join("fish/conf.d/herdr-threads.fish");
        let component = PaneEnv {
            targets: vec![Target {
                shell: Shell::Fish,
                path: path.clone(),
            }],
            manifest: d.0.join("state/setup/m.json"),
        };
        assert_eq!(
            states(&component.apply(Mode::Grant).unwrap()),
            [FileState::Installed]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), FISH_FILE);
        assert_eq!(states(&component.status().unwrap()), [FileState::Installed]);
        assert_eq!(states(&component.remove().unwrap()), [FileState::Removed]);
        assert!(!path.exists());
        assert!(user_config_backups(&path).is_empty());
        // A drop-in setup did not write is never replaced; an edited one is left.
        fs::write(&path, "set -gx X 1\n").unwrap();
        assert!(matches!(
            component.apply(Mode::Grant).unwrap()[0].state,
            FileState::Skipped(_)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "set -gx X 1\n");
        fs::remove_file(&path).unwrap();
        component.apply(Mode::Grant).unwrap();
        fs::write(&path, "edited\n").unwrap();
        assert_eq!(states(&component.status().unwrap()), [FileState::Edited]);
        assert_eq!(
            states(&component.remove().unwrap()),
            [FileState::LeftChanged]
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "edited\n");
    }

    /// Kills: publishing over a file that changed after its baseline read, or keeping the
    /// record of a write that did not happen.
    #[test]
    fn a_concurrent_change_is_refused_and_unrecorded() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        fs::write(&path, "x\n").unwrap();
        let mut manifest = Manifest::default();
        let state = component.write(
            &mut manifest,
            &component.targets[0],
            Some(b"y\n"),
            POSIX_BLOCK.as_bytes(),
        );
        assert!(matches!(state, FileState::Error(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");
        assert!(manifest.files.is_empty());
        assert!(!component.manifest.exists());
        assert!(user_config_backups(&path).is_empty());
    }

    /// Kills: a launch report that reads another HOME's or another shell's files, or one that
    /// ignores the declared ZDOTDIR; and an adapter that does not capture the inputs.
    #[test]
    fn installed_in_reads_the_declared_environment() {
        use crate::harness::adapter::HarnessAdapter;
        for name in ENVIRONMENT_INPUTS {
            assert!(
                crate::harness::codex::CodexAdapter
                    .setup_environment_inputs()
                    .contains(&name),
                "{name}"
            );
        }
        let d = dir();
        let zdot = d.0.join("zdot");
        fs::create_dir(&zdot).unwrap();
        let env = SetupEnv {
            home: Some(d.0.clone().into_os_string()),
            declared_environment: [
                ("SHELL".to_owned(), "/bin/zsh".into()),
                ("ZDOTDIR".to_owned(), zdot.clone().into_os_string()),
            ]
            .into(),
            executable: PathBuf::from("/x/herdr-threads"),
            state_dir: Some(d.0.join("state")),
            cwd: PathBuf::from("/"),
            path: None,
            config_roots: Default::default(),
            host_endpoint: None,
            instance_source: Value::Null,
        };
        fs::write(d.0.join(".zshenv"), POSIX_BLOCK).unwrap();
        assert!(!installed_in(&env), "HOME/.zshenv is not the ZDOTDIR file");
        fs::write(zdot.join(".zshenv"), POSIX_BLOCK).unwrap();
        assert!(installed_in(&env));
    }

    fn scratch_env(root: &Path) -> SetupEnv {
        SetupEnv {
            home: Some(root.join("home").into_os_string()),
            declared_environment: [("SHELL".to_owned(), "/bin/bash".into())].into(),
            executable: PathBuf::from("/x/herdr-threads"),
            state_dir: Some(root.join("state")),
            cwd: PathBuf::from("/"),
            path: None,
            config_roots: [("codex".to_owned(), root.join("codex"))].into(),
            host_endpoint: None,
            instance_source: Value::Null,
        }
    }

    /// The terminal question: only a typed yes writes, and the foreground reading follows the
    /// answer; anything else declines without a write. Kills: a default-yes question, a write
    /// on decline, a stale `required` foreground warning after a yes, or asking when a block
    /// is already there.
    #[test]
    fn settle_writes_only_on_a_typed_yes() {
        for (answer, writes) in [
            ("y\n", true),
            ("YES\n", true),
            ("\n", false),
            ("n\n", false),
        ] {
            let d = dir();
            fs::create_dir(d.0.join("home")).unwrap();
            let bashrc = d.0.join("home/.bashrc");
            fs::write(&bashrc, "x\n").unwrap();
            let env = scratch_env(&d.0);
            let (pane_env, warnings) = install_report(&env, PaneEnvPolicy::Ask);
            assert_eq!(pane_env["action"], "advised");
            assert!(warnings.is_empty());
            let mut report = json!({"pane_env": pane_env, "warnings": []});
            crate::cli::setup::foreground::attach(
                &mut report,
                crate::harness::context::Harness::Codex,
                &env,
            );
            assert_eq!(report["foreground"]["status"], "required");
            let mut out = Vec::new();
            settle(&env, &mut report, &mut answer.as_bytes(), &mut out).unwrap();
            let asked = String::from_utf8(out).unwrap();
            assert!(asked.ends_with("[y/N] "), "{asked}");
            assert!(asked.contains(&bashrc.display().to_string()), "{asked}");
            if writes {
                assert_eq!(report["pane_env"]["action"], "installed", "{answer:?}");
                assert_eq!(
                    fs::read_to_string(&bashrc).unwrap(),
                    format!("x\n{POSIX_BLOCK}")
                );
                assert_eq!(report["foreground"]["status"], "configured");
                assert!(
                    !report["warnings"]
                        .to_string()
                        .contains("Foreground user settings"),
                    "{report}"
                );
                // Installed now: a second settle asks nothing.
                let mut out = Vec::new();
                let (kept, _) = install_report(&env, PaneEnvPolicy::Ask);
                let mut again = json!({"pane_env": kept});
                settle(&env, &mut again, &mut "y\n".as_bytes(), &mut out).unwrap();
                assert!(out.is_empty());
                assert_eq!(again["pane_env"]["action"], "kept");
            } else {
                assert_eq!(report["pane_env"]["action"], "declined", "{answer:?}");
                assert!(report["pane_env"].get("note").is_none());
                assert_eq!(fs::read_to_string(&bashrc).unwrap(), "x\n");
                assert!(user_config_backups(&bashrc).is_empty());
            }
        }
    }

    /// Kills: a terminal yes replacing a block the user edited (only --with-pane-env may),
    /// or skipping the file that lacks the block beside it.
    #[test]
    fn a_terminal_yes_adds_missing_blocks_and_keeps_an_edited_one() {
        let d = dir();
        let home = d.0.join("home");
        fs::create_dir(&home).unwrap();
        let mut env = scratch_env(&d.0);
        env.declared_environment
            .insert("SHELL".to_owned(), "/bin/zsh".into());
        let zshenv = home.join(".zshenv");
        let bashrc = home.join(".bashrc");
        let edited = format!("a\n{}", POSIX_BLOCK.replace("URL=;", "URL=; export X=1;"));
        fs::write(&zshenv, &edited).unwrap();
        fs::write(&bashrc, "b\n").unwrap();
        fs::write(home.join(".bash_profile"), POSIX_BLOCK).unwrap();
        let (pane_env, _) = install_report(&env, PaneEnvPolicy::Ask);
        assert_eq!(pane_env["action"], "advised", "{pane_env}");
        let mut report = json!({"pane_env": pane_env, "warnings": []});
        let mut out = Vec::new();
        settle(&env, &mut report, &mut "y\n".as_bytes(), &mut out).unwrap();
        let asked = String::from_utf8(out).unwrap();
        assert!(asked.contains(&bashrc.display().to_string()), "{asked}");
        assert!(!asked.contains(&zshenv.display().to_string()), "{asked}");
        // The unrecorded block a yes adopts is named too.
        let profile = home.join(".bash_profile");
        assert!(
            asked.contains(&format!(
                "and record the existing block in {} as setup-owned? [y/N] ",
                profile.display()
            )),
            "{asked}"
        );
        assert_eq!(report["pane_env"]["action"], "installed", "{report}");
        let states: Vec<_> = report["pane_env"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["state"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(states, ["edited", "installed", "already_installed"]);
        assert_eq!(fs::read_to_string(&zshenv).unwrap(), edited);
        assert_eq!(
            fs::read_to_string(&bashrc).unwrap(),
            format!("b\n{POSIX_BLOCK}")
        );
        // --with-pane-env does replace it.
        let (granted, _) = install_report(&env, PaneEnvPolicy::Grant);
        assert_eq!(granted["files"][0]["state"], "installed");
        assert_eq!(
            fs::read_to_string(&zshenv).unwrap(),
            format!("a\n{POSIX_BLOCK}")
        );
    }

    /// Kills: a plain setup adopting a hand-pasted block as owned, so unsetup would remove it.
    #[test]
    fn plain_setup_never_adopts_a_hand_pasted_block() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        fs::write(&path, format!("mine\n{POSIX_BLOCK}")).unwrap();
        assert_eq!(
            states(&component.apply(Mode::Ask).unwrap()),
            [FileState::Unrecorded]
        );
        assert_eq!(
            states(&component.status().unwrap()),
            [FileState::Unrecorded]
        );
        assert!(!component.manifest.exists());
        assert_eq!(component.remove().unwrap(), []);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("mine\n{POSIX_BLOCK}")
        );
        // A consented run adopts it; unsetup then owns it.
        assert_eq!(
            states(&component.apply(Mode::Consent).unwrap()),
            [FileState::AlreadyInstalled]
        );
        assert_eq!(states(&component.status().unwrap()), [FileState::Installed]);
        assert_eq!(states(&component.remove().unwrap()), [FileState::Removed]);
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine\n");
    }

    /// Kills: the hand-add lines cut off by the installer's 600-character rendering cap, the
    /// why repeated beside the foreground warning, or a silent ZDOTDIR mismatch.
    #[test]
    fn advice_leads_with_the_manual_lines_and_names_a_distinct_zdotdir() {
        let missing = [Target {
            shell: Shell::Zsh,
            path: PathBuf::from("/h/zdot/.zshenv"),
        }];
        let files = AdviceFiles {
            missing: &missing,
            edited: &[],
            skipped: &[],
        };
        let note = advice(&files, Some(Path::new("/h/zdot")));
        let head: String = note.chars().take(600).collect();
        assert!(
            head.contains(POSIX_LINE) && head.contains(FISH_LINE),
            "{note}"
        );
        assert!(note.contains("ZDOTDIR (/h/zdot)"), "{note}");
        assert!(!note.contains("wrong pane"), "{note}");
        assert!(!advice(&files, None).contains("ZDOTDIR"));
        assert!(!note.contains("No zsh, bash or fish"), "{note}");
        // Nothing missing because every file is edited or skipped: name them, never claim
        // that no startup file exists.
        let edited = [Target {
            shell: Shell::Zsh,
            path: PathBuf::from("/h/.zshenv"),
        }];
        let skipped = [Target {
            shell: Shell::Bash,
            path: PathBuf::from("/h/.bashrc"),
        }];
        let note = advice(
            &AdviceFiles {
                missing: &[],
                edited: &edited,
                skipped: &skipped,
            },
            None,
        );
        assert!(!note.contains("No zsh, bash or fish"), "{note}");
        assert!(
            note.contains(
                "Your edited block in /h/.zshenv is kept; `--with-pane-env` replaces it."
            ),
            "{note}"
        );
        assert!(
            note.contains("Setup cannot change /h/.bashrc: add the line there yourself."),
            "{note}"
        );
        assert!(!note.contains("adds the block"), "{note}");
        let none = advice(
            &AdviceFiles {
                missing: &[],
                edited: &[],
                skipped: &[],
            },
            None,
        );
        assert!(
            none.contains("No zsh, bash or fish startup file was found"),
            "{none}"
        );
        // A ~/.bash_profile setup would create is named with its shadowing effect.
        let d = dir();
        let profile = [Target {
            shell: Shell::Bash,
            path: d.0.join(".bash_profile"),
        }];
        let note = advice(
            &AdviceFiles {
                missing: &profile,
                edited: &[],
                skipped: &[],
            },
            None,
        );
        assert!(note.contains("instead of a later ~/.profile"), "{note}");
        fs::write(d.0.join(".bash_profile"), "").unwrap();
        let note = advice(
            &AdviceFiles {
                missing: &profile,
                edited: &[],
                skipped: &[],
            },
            None,
        );
        assert!(!note.contains("~/.profile"), "{note}");
        let d = dir();
        let mut env = scratch_env(&d.0);
        env.declared_environment
            .insert("ZDOTDIR".to_owned(), d.0.join("home").into_os_string());
        assert_eq!(distinct_zdotdir(&env), None, "ZDOTDIR equal to HOME");
        env.declared_environment
            .insert("ZDOTDIR".to_owned(), d.0.join("z").into_os_string());
        assert_eq!(distinct_zdotdir(&env), Some(d.0.join("z")));
    }

    /// Kills: unsetup advice that tells the user to add the line to a file it could not
    /// remove the block from.
    #[test]
    fn unsetup_advice_for_a_skipped_file_says_remove() {
        let d = dir();
        let (component, path) = zsh(&d.0);
        component.apply(Mode::Grant).unwrap();
        let real = d.0.join("real");
        fs::rename(&path, &real).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        let report = remove_json(&component);
        let reason = report["files"][0]["reason"].as_str().unwrap();
        assert!(
            reason.contains("remove the herdr-threads managed block"),
            "{reason}"
        );
        assert!(!reason.contains("add this line"), "{reason}");
    }

    /// The installer's advise policy reports but never writes, even an owned outdated block.
    /// Kills: a write (or a recorded adoption) from the installer path.
    #[test]
    fn advise_policy_never_writes() {
        let d = dir();
        fs::create_dir(d.0.join("home")).unwrap();
        let bashrc = d.0.join("home/.bashrc");
        let profile = d.0.join("home/.bash_profile");
        fs::write(&bashrc, "x\n").unwrap();
        let env = scratch_env(&d.0);
        let (report, _) = install_report(&env, PaneEnvPolicy::Advise);
        assert_eq!(report["action"], "advised");
        assert!(report["note"].as_str().unwrap().contains("--with-pane-env"));
        assert_eq!(fs::read_to_string(&bashrc).unwrap(), "x\n");
        assert!(!profile.exists());
        fs::write(&bashrc, format!("x\n{POSIX_BLOCK}")).unwrap();
        // One file still lacks it: still advised, naming only that file.
        let (report, _) = install_report(&env, PaneEnvPolicy::Advise);
        assert_eq!(report["action"], "advised");
        let note = report["note"].as_str().unwrap();
        assert!(note.contains(&profile.display().to_string()), "{note}");
        assert!(!note.contains(&bashrc.display().to_string()), "{note}");
        fs::write(&profile, POSIX_BLOCK).unwrap();
        let (report, _) = install_report(&env, PaneEnvPolicy::Advise);
        assert_eq!(report["action"], "kept");
        assert_eq!(report["files"][0]["state"], "present_unrecorded");
        assert!(
            !component(&env).unwrap().manifest.exists(),
            "nothing recorded"
        );
    }

    /// Kills: a foreground reading that ignores the block, or one that claims configured
    /// for an edited block.
    #[test]
    fn foreground_reads_configured_only_for_the_current_block() {
        let d = dir();
        let home = d.0.join("home");
        fs::create_dir(&home).unwrap();
        let environment = crate::harness::adapter::SetupEnvironment {
            home: Some(home.clone().into_os_string()),
            state_dir: Some(d.0.join("state")),
            declared: [("SHELL".to_owned(), "/bin/zsh".into())].into(),
            ..Default::default()
        };
        let read = || {
            let mut report = json!({"status": "required", "modified_by_setup": false});
            inspect_foreground_environment(&environment, &mut report);
            report
        };
        assert_eq!(
            read(),
            json!({"status": "required", "modified_by_setup": false, "pane_env": "not_installed"})
        );
        // Pasted by hand: configured, but not setup's change.
        fs::write(home.join(".zshenv"), format!("x\n{POSIX_BLOCK}")).unwrap();
        assert_eq!(
            read(),
            json!({"status": "configured", "modified_by_setup": false, "pane_env": "installed"})
        );
        // Recorded by setup (a consented adoption): setup's change.
        let env = SetupEnv::from_snapshot(&environment);
        let component = component(&env).unwrap();
        component.apply(Mode::Consent).unwrap();
        assert_eq!(
            read(),
            json!({"status": "configured", "modified_by_setup": true, "pane_env": "installed"})
        );
        fs::write(
            home.join(".zshenv"),
            format!("x\n{}", POSIX_BLOCK.replace("URL=;", "URL=y;")),
        )
        .unwrap();
        assert_eq!(read()["status"], "required");
        assert_eq!(read()["modified_by_setup"], false);
    }
}
