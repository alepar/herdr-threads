//! `herdr-threads update`: upgrade the installer-managed installation this executable belongs
//! to (`docs/superpowers/specs/2026-10-28-update-command-design.md`).
//!
//! `update` resolves the release through the releases API, downloads and verifies it twice
//! (`SHA256SUMS` and the API's asset digest, then `VERSION`, `TARGET` and the executable), checks
//! the live daemons, keeps the previous package as `P.prev`, and runs the verified archive's own
//! `scripts/install.sh` against a `file://` mirror of the verified files. The installer does the
//! backup, the daemon stop and replace, the swap and setup reconciliation; nothing of it is
//! reimplemented here. `update` then judges the result itself and, when the store schema is
//! unchanged, rolls a failed update back by reinstalling `P.prev` the same way.
//!
//! - Owns: the `P.update.lock` lock, the `P.update.json` journal (an interrupted run is
//!   finished by the next one), `P.prev` and the work directory beside `P`.
//! - Must not: prompt, restore a store backup (it prints where the backup is), or judge a
//!   daemon this installation does not own.
use super::RunError;
use crate::{
    daemon::paths::{DATABASE_FILE, node_dir},
    update::{
        check,
        install::{Installation, Marker, store_state_dirs},
        release::{self, Release, Version},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// How long a second `update` waits for the lock (an orphaned installer holds it).
const LOCK_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateRequest {
    /// `--version`: the release to install instead of the latest.
    pub version: Option<String>,
    pub dry_run: bool,
    pub check: bool,
    /// `--setup` (`Some(true)`) or `--no-setup` (`Some(false)`).
    pub setup: Option<bool>,
    pub without_permissions: bool,
    pub no_aisw_profiles: bool,
    pub no_herdr: bool,
}

fn say<W: Write>(writer: &mut W, line: &str) -> io::Result<()> {
    writeln!(writer, "herdr-threads: {line}")?;
    writer.flush()
}

/// One `herdr-threads: error: ...` line on stderr, exit 1.
fn refuse(reason: impl std::fmt::Display) -> RunError {
    eprintln!("herdr-threads: error: {reason}");
    RunError::Exit(1)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Test builds may skip comparing an executable's `--version` with the package version, so a
/// test can package one build under several versions.
fn exe_version_checked() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if std::env::var_os("HT_TEST_UPDATE_IGNORE_EXE_VERSION").is_some() {
        return false;
    }
    true
}

fn lock_wait() -> Duration {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(ms) = std::env::var("HT_TEST_UPDATE_LOCK_WAIT_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    LOCK_WAIT
}

pub fn run<W: Write>(request: &UpdateRequest, writer: &mut W) -> Result<(), RunError> {
    let exe = std::env::current_exe()?;
    let installation = Installation::current().ok_or_else(|| {
        refuse(format!(
            "not an installer-managed install ({}); use scripts/install.sh",
            exe.display()
        ))
    })?;
    let installed = installation
        .marker
        .version
        .as_deref()
        .and_then(Version::parse)
        .unwrap_or_else(Version::current);
    let target = match &request.version {
        Some(text) => Some(
            Version::parse(text)
                .ok_or_else(|| refuse(format!("--version {text:?} is not vX.Y.Z")))?,
        ),
        None => None,
    };
    let _lock = Lock::acquire(&installation.sibling(".update.lock"))?;
    let journal_path = installation.sibling(".update.json");
    if let Some(journal) = Journal::read(&journal_path).map_err(refuse)? {
        if request.dry_run || request.check {
            return Err(refuse(format!(
                "an interrupted update to {} needs finishing: run herdr-threads update",
                journal.target
            )));
        }
        return resume(&installation, journal, &journal_path, writer);
    }
    if request.check {
        return run_check(&installation, installed, writer);
    }
    let platform = release::platform().map_err(refuse)?;
    let parent = installation
        .prefix
        .parent()
        .ok_or_else(|| refuse("the installation directory has no parent"))?;
    let mut work = WorkDir::create(parent)?;
    let release = release::fetch_release(
        &release::api_base(),
        target,
        work.path(),
        release::API_TIMEOUT_SECS,
    )
    .map_err(|error| refuse(format!("{error}; nothing changed")))?;
    if release.version == installed {
        return Ok(say(writer, &format!("up to date: {installed}"))?);
    }
    if release.version < installed {
        return Err(refuse(format!(
            "downgrade refused: {} is older than the installed {installed}; nothing changed",
            release.version
        )));
    }
    let verified = verify(&release, &platform, work.path())
        .map_err(|reason| refuse(format!("{reason}; nothing changed")))?;
    let states = preflight(&installation).map_err(refuse)?;
    let plan = Plan {
        previous: installed,
        target: release.version,
        states,
        flags: install_flags(request, &installation.marker),
    };
    if request.dry_run {
        return Ok(print_plan(
            &installation,
            &plan,
            &platform,
            &verified,
            writer,
        )?);
    }
    // From here to the judge, a terminal Ctrl-C, a closed pane or a tool timeout must not stop
    // the update halfway; an interrupted run (kill -9) is finished by the next one.
    ignore_signals();
    let mut journal = Journal {
        version: JOURNAL_VERSION,
        target: plan.target.to_string(),
        previous: plan.previous.to_string(),
        work: work.path().to_path_buf(),
        flags: plan.flags.clone(),
        states: plan.states.clone(),
        installer_pid: None,
    };
    journal.write(&journal_path)?;
    if let Err(error) = keep_previous(&installation) {
        let _ = fs::remove_file(&journal_path);
        return Err(refuse(format!(
            "could not keep the previous package: {error}; nothing changed"
        )));
    }
    // From here the journal owns the work directory: the run's conclusion removes it.
    work.keep();
    let log = installation.sibling(".prev").join("update.log");
    let outcome = run_installer(
        &installation,
        &verified.install_script,
        &verified.mirror,
        plan.target,
        &plan.flags,
        &log,
        work.path(),
        &mut |pid| {
            journal.installer_pid = Some(pid);
            let _ = journal.write(&journal_path);
        },
    );
    let outcome = match outcome {
        Ok(installed) => Run::Installed(installed),
        Err(error) => Run::Failed(error),
    };
    conclude(
        &installation,
        &journal,
        &journal_path,
        outcome,
        &log,
        writer,
    )
}

/// `update --check`: the update check now, in the foreground.
fn run_check<W: Write>(
    installation: &Installation,
    installed: Version,
    writer: &mut W,
) -> Result<(), RunError> {
    let nodes: Vec<PathBuf> = crate::update::install::default_state_dirs()
        .iter()
        .map(|dir| node_dir(dir))
        .filter(|node| node.is_dir())
        .collect();
    let scratch;
    let first = match nodes.first() {
        Some(node) => node.clone(),
        None => {
            let parent = installation
                .prefix
                .parent()
                .ok_or_else(|| refuse("the installation directory has no parent"))?;
            scratch = WorkDir::create(parent)?;
            scratch.path().to_path_buf()
        }
    };
    let now = now_ms();
    let outcome = check::run_check(&first, installed, now, release::API_TIMEOUT_SECS)
        .map_err(|error| refuse(format!("update check failed: {error}")))?;
    let latest = outcome.latest();
    for node in nodes.iter().skip(1) {
        check::record_latest(node, latest, now);
    }
    match outcome {
        check::Outcome::Newer(latest) => say(
            writer,
            &format!("{latest} available (installed {installed}): herdr-threads update"),
        )?,
        check::Outcome::UpToDate(_) => say(writer, &format!("up to date: {installed}"))?,
    }
    Ok(())
}

// -- plan --------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DaemonState {
    /// No live daemon at preflight.
    None,
    /// A live daemon started from this installation.
    Owned,
    /// A live daemon started from elsewhere (a dev checkout, another prefix).
    Foreign,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StateInfo {
    dir: PathBuf,
    /// `PRAGMA user_version` at preflight; `None` when it could not be read.
    user_version: Option<i64>,
    daemon: DaemonState,
    pid: Option<u32>,
    exe: Option<String>,
}

struct Plan {
    previous: Version,
    target: Version,
    states: Vec<StateInfo>,
    flags: Vec<String>,
}

/// The installer flags: the request's, plus the opt-outs the marker recorded.
fn install_flags(request: &UpdateRequest, marker: &Marker) -> Vec<String> {
    let mut flags = Vec::new();
    match request.setup {
        Some(true) => flags.push("--setup".to_owned()),
        Some(false) => flags.push("--no-setup".to_owned()),
        None => {}
    }
    if request.without_permissions || marker.without_permissions {
        flags.push("--without-permissions".to_owned());
    }
    if request.no_aisw_profiles || marker.no_aisw_profiles {
        flags.push("--no-aisw-profiles".to_owned());
    }
    if request.no_herdr || marker.no_herdr {
        flags.push("--no-herdr".to_owned());
    }
    flags
}

fn print_plan<W: Write>(
    installation: &Installation,
    plan: &Plan,
    platform: &str,
    verified: &Verified,
    writer: &mut W,
) -> io::Result<()> {
    say(
        writer,
        &format!(
            "would update {} -> {} ({platform}, sha256 {} verified)",
            plan.previous, plan.target, verified.sha256
        ),
    )?;
    for state in &plan.states {
        let schema = state
            .user_version
            .map_or_else(|| "unknown".to_owned(), |v| v.to_string());
        say(
            writer,
            &format!("would back up {} (schema {schema})", state.dir.display()),
        )?;
        match (state.daemon, state.pid) {
            (DaemonState::Owned, Some(pid)) => say(
                writer,
                &format!("would restart daemon pid {pid} ({})", state.dir.display()),
            )?,
            (DaemonState::Foreign, Some(pid)) => say(
                writer,
                &format!(
                    "would stop foreign daemon pid {pid} ({}, runs {}); not restarted",
                    state.dir.display(),
                    state.exe.as_deref().unwrap_or("?")
                ),
            )?,
            _ => {}
        }
    }
    let setup = if plan.flags.iter().any(|flag| flag == "--no-setup") {
        "would skip harness setup (--no-setup)"
    } else if plan.flags.iter().any(|flag| flag == "--setup") {
        "would reconcile harness integrations, adding missing ones (--setup)"
    } else {
        "would reconcile owned harness integrations (no missing ones added; --setup adds)"
    };
    say(writer, setup)?;
    say(
        writer,
        &format!(
            "would keep {} as {}",
            installation.prefix.display(),
            installation.sibling(".prev").display()
        ),
    )?;
    say(writer, "dry run; nothing changed")
}

// -- verification ------------------------------------------------------------------------

struct Verified {
    sha256: String,
    /// The extracted target's `scripts/install.sh`.
    install_script: PathBuf,
    /// `file://` release base laid out as `download/<tag>/<asset>`.
    mirror: PathBuf,
}

fn verify(release: &Release, platform: &str, work: &Path) -> Result<Verified, String> {
    let asset_name = release::asset_name(platform);
    let asset = release
        .asset(&asset_name)
        .ok_or_else(|| format!("{} has no {asset_name}", release.version.tag()))?;
    let sums = release
        .asset(release::SUMS_ASSET)
        .ok_or_else(|| format!("{} has no SHA256SUMS", release.version.tag()))?;
    let (Some(asset_digest), Some(sums_digest)) = (&asset.sha256, &sums.sha256) else {
        return Err(format!(
            "the release API gives no sha256 digest for {} assets",
            release.version.tag()
        ));
    };
    let download = work.join("download");
    let mirror = work.join("mirror");
    let dir = mirror.join("download").join(release.version.tag());
    fs::create_dir_all(&download).map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join(&asset_name);
    let sums_path = dir.join(release::SUMS_ASSET);
    release::download(&sums.url, &sums_path, release::API_TIMEOUT_SECS)
        .map_err(|error| error.to_string())?;
    release::download(&asset.url, &archive, release::DOWNLOAD_TIMEOUT_SECS)
        .map_err(|error| error.to_string())?;
    let sums_hash = release::sha256_file(&sums_path).map_err(|e| e.to_string())?;
    if &sums_hash != sums_digest {
        return Err(format!(
            "SHA256SUMS digest mismatch: release API {sums_digest}, downloaded {sums_hash}"
        ));
    }
    let sums_text = fs::read_to_string(&sums_path).map_err(|e| e.to_string())?;
    let listed = release::sums_entry(&sums_text, &asset_name)
        .ok_or_else(|| format!("SHA256SUMS has no entry for {asset_name}"))?;
    let actual = release::sha256_file(&archive).map_err(|e| e.to_string())?;
    if actual != listed || &actual != asset_digest {
        return Err(format!(
            "checksum mismatch for {asset_name}: SHA256SUMS {listed}, release API {asset_digest}, downloaded {actual}"
        ));
    }
    let extract = work.join("extract");
    fs::create_dir_all(&extract).map_err(|e| e.to_string())?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&extract)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("tar did not run: {error}"))?;
    if !status.success() {
        return Err(format!("could not extract {asset_name}"));
    }
    let entries: Vec<_> = fs::read_dir(&extract)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    let package = extract.join("herdr-threads");
    if entries.len() != 1
        || entries[0] != "herdr-threads"
        || !fs::symlink_metadata(&package).is_ok_and(|meta| meta.is_dir())
    {
        return Err(format!(
            "{asset_name} does not hold exactly one herdr-threads/ directory"
        ));
    }
    let read = |name: &str| {
        fs::read_to_string(package.join(name))
            .map(|text| text.trim().to_owned())
            .map_err(|_| format!("the archive has no {name}"))
    };
    let version = read("VERSION")?;
    if version != release.version.to_string() {
        return Err(format!(
            "the archive's VERSION is {version}, not {}",
            release.version
        ));
    }
    let target = read("TARGET")?;
    if target != platform {
        return Err(format!("the archive's TARGET is {target}, not {platform}"));
    }
    let install_script = package.join("scripts").join("install.sh");
    if !install_script.is_file() {
        return Err("the archive has no scripts/install.sh".to_owned());
    }
    let binary = package.join("bin").join("herdr-threads");
    let printed = exe_version(&binary)
        .map_err(|error| format!("the downloaded executable does not run here: {error}"))?;
    if exe_version_checked() && printed != release.version.to_string() {
        return Err(format!(
            "the downloaded executable reports {printed}, not {}",
            release.version
        ));
    }
    Ok(Verified {
        sha256: actual,
        install_script,
        mirror,
    })
}

/// The version an executable prints for `--version` (`herdr-threads X.Y.Z`).
fn exe_version(binary: &Path) -> Result<String, String> {
    let output = default_signals(&mut Command::new(binary))
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("exit status {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .trim()
        .strip_prefix("herdr-threads ")
        .unwrap_or(text.trim())
        .to_owned())
}

// -- preflight ---------------------------------------------------------------------------

fn user_version(state: &Path) -> Option<i64> {
    let path = node_dir(state).join(DATABASE_FILE);
    let connection = rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .ok()
}

/// The live daemon of `state` (pid, executable), from its `endpoint.json`.
fn live_daemon(state: &Path) -> Option<(u32, String)> {
    let bytes = fs::read(node_dir(state).join("endpoint.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let pid = u32::try_from(value.get("pid")?.as_u64()?).ok()?;
    let alive = i32::try_from(pid).is_ok_and(|pid| unsafe { libc::kill(pid, 0) } == 0);
    if !alive {
        return None;
    }
    let output = Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let command = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !command.contains("herdr-threads") {
        return None;
    }
    let exe = command
        .split_once(" daemon run")
        .map_or(command.as_str(), |(exe, _)| exe)
        .to_owned();
    Some((pid, exe))
}

fn preflight(installation: &Installation) -> Result<Vec<StateInfo>, String> {
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut states = Vec::new();
    for dir in store_state_dirs() {
        let (daemon, pid, exe) = match live_daemon(&dir) {
            None => (DaemonState::None, None, None),
            Some((pid, exe)) if installation.owns_executable(Path::new(&exe)) => {
                (DaemonState::Owned, Some(pid), Some(exe))
            }
            Some((pid, exe)) => (DaemonState::Foreign, Some(pid), Some(exe)),
        };
        if daemon == DaemonState::Owned {
            // `ensure` heals what it can (a dead boot with a renumbered socket device) and
            // otherwise names why this executable cannot reach the daemon.
            daemon_command(&current, &dir, "ensure").map_err(|error| {
                format!(
                    "cannot reach the daemon in {}: {error}; nothing changed",
                    dir.display()
                )
            })?;
        }
        states.push(StateInfo {
            user_version: user_version(&dir),
            dir,
            daemon,
            pid,
            exe,
        });
    }
    Ok(states)
}

/// `<exe> --state-dir <state> daemon <verb>`; the last stderr line on failure.
fn daemon_command(exe: &Path, state: &Path, verb: &str) -> Result<(), String> {
    let output = default_signals(&mut Command::new(exe))
        .arg("--state-dir")
        .arg(state)
        .args(["daemon", verb])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("failed")
        .trim()
        .to_owned())
}

/// `<exe> --state-dir <state> internal store-check`; the last stderr line on failure.
fn store_command(exe: &Path, state: &Path) -> Result<(), String> {
    let output = default_signals(&mut Command::new(exe))
        .arg("--state-dir")
        .arg(state)
        .args(["internal", "store-check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(stderr
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("failed")
        .trim()
        .to_owned())
}

// -- the run -----------------------------------------------------------------------------

/// The serialized run: target, previous version, work directory and preflight states.
/// The journal format; a release that changes it bumps this and keeps reading older ones.
const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    #[serde(default)]
    version: u32,
    target: String,
    previous: String,
    work: PathBuf,
    flags: Vec<String>,
    states: Vec<StateInfo>,
    /// The installer's pid, which leads its own process group (`setsid`), once it runs.
    #[serde(default)]
    installer_pid: Option<u32>,
}

impl Journal {
    /// The pending journal, if any. One that exists but cannot be read is an error, never
    /// silently "no interrupted run".
    fn read(path: &Path) -> Result<Option<Self>, String> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        };
        serde_json::from_slice(&bytes).map(Some).map_err(|error| {
            format!(
                "cannot parse {} ({error}); check the installation, then remove it",
                path.display()
            )
        })
    }

    fn write(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(self).map_err(io::Error::other)?)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    }
}

/// Children get the default dispositions back: a daemon started during the update must stay
/// stoppable by SIGTERM.
fn default_signals(command: &mut Command) -> &mut Command {
    unsafe {
        command.pre_exec(|| {
            for signal in [libc::SIGINT, libc::SIGHUP, libc::SIGTERM] {
                libc::signal(signal, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    command
}

fn ignore_signals() {
    for signal in [libc::SIGINT, libc::SIGHUP, libc::SIGTERM] {
        unsafe {
            libc::signal(signal, libc::SIG_IGN);
        }
    }
}

/// Copy `P` to `P.prev` through `P.prev.tmp`; the old `P.prev` is renamed aside first, so a
/// rename never targets a non-empty directory and an interrupted copy never replaces it.
fn keep_previous(installation: &Installation) -> io::Result<()> {
    let prev = installation.sibling(".prev");
    let tmp = installation.sibling(".prev.tmp");
    let old = installation.sibling(".prev.old");
    remove_tree(&tmp)?;
    remove_tree(&old)?;
    copy_tree(&installation.prefix, &tmp)?;
    if fs::symlink_metadata(&prev).is_ok() {
        fs::rename(&prev, &old)?;
    }
    fs::rename(&tmp, &prev)?;
    remove_tree(&old)
}

fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Copy a package tree: directories, regular files with their modes, symlinks as symlinks.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        fs::copy(from, to)?;
        fs::set_permissions(to, fs::Permissions::from_mode(meta.permissions().mode()))
    }
}

/// How the installer step of the run being concluded went.
enum Run {
    Installed(InstallOutcome),
    /// The installer could not be run.
    Failed(String),
    /// An earlier, interrupted run is being concluded: its installer outcome is unknown.
    Interrupted,
}

/// The installer's exit status and final status line.
struct InstallOutcome {
    status: Option<i32>,
    last_line: String,
}

#[allow(clippy::too_many_arguments)]
fn run_installer(
    installation: &Installation,
    script: &Path,
    mirror: &Path,
    version: Version,
    flags: &[String],
    log: &Path,
    work: &Path,
    on_spawn: &mut dyn FnMut(u32),
) -> Result<InstallOutcome, String> {
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map_err(|error| format!("cannot open {}: {error}", log.display()))?;
    let stderr = file.try_clone().map_err(|error| error.to_string())?;
    let bin_dir = installation
        .bin_dir()
        .ok_or_else(|| "cannot tell the bin directory (no HOME)".to_owned())?;
    let mut command = Command::new("bash");
    command
        .arg(script)
        .arg("--version")
        .arg(version.tag())
        .arg("--release-url")
        .arg(file_url(mirror))
        .arg("--prefix")
        .arg(&installation.prefix)
        .arg("--bin-dir")
        .arg(bin_dir)
        .args(flags)
        // The installer's own extraction runs on P's filesystem, not a noexec /tmp.
        .env("TMPDIR", work)
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(stderr);
    // Its own session, so no terminal signal reaches it; its pid is its process group, which
    // the journal records so a later update can tell an orphaned installer is still running.
    default_signals(&mut command);
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("the installer did not run: {error}"))?;
    on_spawn(child.id());
    let status = child
        .wait()
        .map_err(|error| format!("the installer did not finish: {error}"))?;
    let text = fs::read_to_string(log).unwrap_or_default();
    Ok(InstallOutcome {
        status: status.code(),
        // The final status line; the indented hint lines after it are dropped.
        last_line: text
            .lines()
            .rfind(|line| !line.trim().is_empty() && !line.starts_with(' '))
            .unwrap_or("")
            .trim()
            .to_owned(),
    })
}

fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut url = String::from("file://");
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

/// `internal store-check`: open the existing node store of `state` at this build's schema,
/// applying its migrations, with no daemon. Exit 1 when it cannot.
pub fn store_check<W: Write>(state: &Path, writer: &mut W) -> Result<(), RunError> {
    let path = node_dir(state).join(DATABASE_FILE);
    if !path.is_file() {
        return Err(refuse(format!("no node store at {}", path.display())));
    }
    let checked = (|| {
        let conn = rusqlite::Connection::open(&path).map_err(|error| error.to_string())?;
        conn.busy_timeout(Duration::from_secs(10))
            .map_err(|error| error.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| error.to_string())?;
        crate::store::schema::initialize_with(
            &conn,
            || crate::protocol::time::UtcMillis(now_ms()),
            None,
        )
        .map_err(|error| error.detail.clone())?;
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())
    })();
    match checked {
        Ok(schema) => Ok(say(writer, &format!("store ok (schema {schema})"))?),
        Err(reason) => Err(refuse(format!("store check failed: {reason}"))),
    }
}

/// Judge an installation against `target`: the marker, the executable, every daemon this
/// installation owns, and every store without a daemon, opened at the target schema with no
/// daemon (so a migration failure shows now, and the state directory stays daemon-less).
fn judge(installation: &Installation, target: &str, states: &[StateInfo]) -> Result<(), String> {
    let marker = Marker::read(&installation.prefix)
        .map_err(|error| format!("cannot read the installation marker: {error}"))?;
    let installed = marker.version.unwrap_or_default();
    if installed != target {
        return Err(format!("the installation is {installed}, not {target}"));
    }
    let binary = installation.binary();
    let printed = exe_version(&binary)
        .map_err(|error| format!("the installed executable does not run: {error}"))?;
    if exe_version_checked() && printed != target {
        return Err(format!("the installed executable reports {printed}"));
    }
    for state in states {
        match state.daemon {
            DaemonState::Foreign => {}
            DaemonState::Owned => daemon_command(&binary, &state.dir, "ensure")
                .map_err(|e| format!("the daemon in {} did not start: {e}", state.dir.display()))?,
            DaemonState::None => store_command(&binary, &state.dir)
                .map_err(|e| format!("the store in {} did not open: {e}", state.dir.display()))?,
        }
    }
    Ok(())
}

fn finish(installation: &Installation, journal: &Journal, journal_path: &Path) {
    if let Some(target) = Version::parse(&journal.target) {
        for state in &journal.states {
            check::record_latest(&node_dir(&state.dir), target, now_ms());
        }
    }
    let _ = fs::remove_file(journal_path);
    if journal
        .work
        .starts_with(installation.prefix.parent().unwrap_or(Path::new("/")))
        && journal
            .work
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(WORK_PREFIX))
    {
        let _ = fs::remove_dir_all(&journal.work);
    }
}

/// Judge the run and complete it, roll it back or report.
fn conclude<W: Write>(
    installation: &Installation,
    journal: &Journal,
    journal_path: &Path,
    outcome: Run,
    log: &Path,
    writer: &mut W,
) -> Result<(), RunError> {
    let judged = judge(installation, &journal.target, &journal.states);
    if judged.is_ok() {
        {
            let backups = backups_in(log);
            let backup = if backups.is_empty() {
                String::new()
            } else {
                format!("; backup {}", backups.join(", "))
            };
            say(
                writer,
                &format!(
                    "updated {} -> {} (sha256 verified twice{backup})",
                    journal.previous, journal.target
                ),
            )?;
            let running = journal
                .states
                .iter()
                .filter(|state| state.daemon == DaemonState::Owned)
                .count();
            let daemons = if running == 0 {
                String::new()
            } else {
                format!(
                    "daemon {} running in {running} state dir(s); ",
                    journal.target
                )
            };
            say(
                writer,
                &format!(
                    "{daemons}previous package kept in {}",
                    installation.sibling(".prev").display()
                ),
            )?;
            for state in &journal.states {
                if let (DaemonState::Foreign, Some(pid)) = (state.daemon, state.pid)
                    && live_daemon(&state.dir).is_none_or(|(now, _)| now != pid)
                {
                    say(
                        writer,
                        &format!(
                            "foreign daemon pid {pid} ({}) was stopped and not restarted",
                            state.dir.display()
                        ),
                    )?;
                }
            }
            finish(installation, journal, journal_path);
            // The judge passed, but anything the installer did not finish is reported: its
            // status line and the log, exit 3. `--no-herdr` alone is the asked-for state.
            return match outcome {
                Run::Interrupted
                | Run::Installed(InstallOutcome {
                    status: Some(0), ..
                }) => Ok(()),
                Run::Installed(InstallOutcome {
                    status: Some(3),
                    last_line,
                }) if last_line.contains("not linked: --no-herdr") => Ok(()),
                Run::Installed(InstallOutcome { last_line, .. }) => {
                    say(writer, &last_line)?;
                    say(writer, &format!("log: {}", log.display()))?;
                    Err(RunError::Exit(3))
                }
                Run::Failed(run_error) => {
                    say(writer, &format!("{run_error}; log: {}", log.display()))?;
                    Err(RunError::Exit(3))
                }
            };
        }
    }
    let error = judged.err().unwrap_or_default();
    let reason = match &outcome {
        Run::Interrupted => format!("the earlier update was interrupted: {error}"),
        Run::Failed(run_error) => format!("{run_error}: {error}"),
        Run::Installed(installed)
            if installed.last_line.is_empty() || installed.status == Some(0) =>
        {
            error
        }
        Run::Installed(installed) => format!("{error} (installer: {})", installed.last_line),
    };
    let marker = Marker::read(&installation.prefix).unwrap_or_default();
    if marker.version.as_deref() == Some(journal.previous.as_str()) {
        // The installer stopped before the swap; restart what it may have stopped.
        let binary = installation.binary();
        let mut restarted = Vec::new();
        for state in &journal.states {
            if state.daemon == DaemonState::Owned && live_daemon(&state.dir).is_none() {
                match daemon_command(&binary, &state.dir, "ensure") {
                    Ok(()) => restarted.push(state.dir.display().to_string()),
                    Err(e) => eprintln!(
                        "herdr-threads: error: could not restart the daemon in {}: {e}",
                        state.dir.display()
                    ),
                }
            }
        }
        finish(installation, journal, journal_path);
        let restarted = if restarted.is_empty() {
            String::new()
        } else {
            format!(
                "; daemon restarted with {} in {}",
                journal.previous,
                restarted.join(", ")
            )
        };
        return Err(refuse(format!(
            "{reason}; {} unchanged{restarted}; log: {}",
            journal.previous,
            log.display()
        )));
    }
    // After the swap. Roll back only while every store schema is unchanged.
    let unchanged = journal.states.iter().all(|state| {
        state.user_version.is_some() && user_version(&state.dir) == state.user_version
    });
    let script = journal
        .work
        .join("extract")
        .join("herdr-threads")
        .join("scripts")
        .join("install.sh");
    if unchanged && script.is_file() {
        match roll_back(installation, journal, journal_path, &script) {
            Ok(rollback_log) => {
                finish(installation, journal, journal_path);
                return Err(refuse(format!(
                    "{reason}; rolled back to {}; log: {} (rollback: {})",
                    journal.previous,
                    log.display(),
                    rollback_log.display()
                )));
            }
            Err(rollback_error) => {
                eprintln!("herdr-threads: error: rollback failed: {rollback_error}");
            }
        }
    }
    eprintln!(
        "herdr-threads: error: {reason}; no automatic rollback{}",
        if unchanged {
            ""
        } else {
            " (the store schema advanced)"
        }
    );
    for backup in backups_in(log) {
        eprintln!("herdr-threads: backup: {backup}");
    }
    eprintln!(
        "herdr-threads: restore: docs/install.md, Pre-upgrade backup; previous package: {}; log: {}",
        installation.sibling(".prev").display(),
        log.display()
    );
    finish(installation, journal, journal_path);
    Err(RunError::Exit(1))
}

/// The backup entries the installer named in its log (`backed up S to ENTRY (...)`).
fn backups_in(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("herdr-threads: backed up ")?;
            let (_, entry) = rest.split_once(" to ")?;
            Some(entry.split(" (").next().unwrap_or(entry).to_owned())
        })
        .collect()
}

/// Reinstall `P.prev` with the target's installer, then judge it.
fn roll_back(
    installation: &Installation,
    journal: &Journal,
    journal_path: &Path,
    script: &Path,
) -> Result<PathBuf, String> {
    let previous =
        Version::parse(&journal.previous).ok_or("the journal's previous version is invalid")?;
    let platform = release::platform()?;
    let asset = release::asset_name(&platform);
    let root = journal.work.join("rollback");
    let _ = remove_tree(&root);
    let stage = root.join("stage");
    let dir = root.join("mirror").join("download").join(previous.tag());
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    copy_tree(&installation.sibling(".prev"), &stage.join("herdr-threads"))
        .map_err(|error| format!("cannot stage the previous package: {error}"))?;
    let archive = dir.join(&asset);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .arg("herdr-threads")
        .env("COPYFILE_DISABLE", "1")
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("tar did not run: {error}"))?;
    if !status.success() {
        return Err("could not pack the previous package".to_owned());
    }
    let hash = release::sha256_file(&archive).map_err(|e| e.to_string())?;
    fs::write(dir.join(release::SUMS_ASSET), format!("{hash}  {asset}\n"))
        .map_err(|e| e.to_string())?;
    let log = installation.sibling(".prev").join("rollback.log");
    let outcome = run_installer(
        installation,
        script,
        &root.join("mirror"),
        previous,
        &journal.flags,
        &log,
        &journal.work,
        &mut |pid| {
            let mut recorded = journal.clone();
            recorded.installer_pid = Some(pid);
            let _ = recorded.write(journal_path);
        },
    )?;
    judge(installation, &journal.previous, &journal.states).map_err(|error| {
        format!(
            "{error} (installer: {}); log: {}",
            outcome.last_line,
            log.display()
        )
    })?;
    Ok(log)
}

/// Finish a run an earlier `update` journaled and did not conclude.
fn resume<W: Write>(
    installation: &Installation,
    journal: Journal,
    journal_path: &Path,
    writer: &mut W,
) -> Result<(), RunError> {
    // An installer orphaned by a killed update may still be running in its own process group:
    // wait for it, then judge what it left.
    if let Some(pid) = journal
        .installer_pid
        .and_then(|pid| i32::try_from(pid).ok())
    {
        let started = Instant::now();
        let alive = || unsafe { libc::kill(-pid, 0) } == 0;
        if alive() {
            eprintln!("herdr-threads: waiting for an earlier update's installer (pid {pid})");
        }
        while alive() {
            if started.elapsed() >= lock_wait() {
                return Err(refuse(format!(
                    "an earlier update's installer is still running (pid {pid}); try again later"
                )));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    ignore_signals();
    say(
        writer,
        &format!(
            "finishing an interrupted update {} -> {}",
            journal.previous, journal.target
        ),
    )?;
    let log = installation.sibling(".prev").join("update.log");
    conclude(
        installation,
        &journal,
        journal_path,
        Run::Interrupted,
        &log,
        writer,
    )
}

// -- lock and work directory ---------------------------------------------------------------

/// The exclusive `flock` on `P.update.lock`, held for the whole run. Close-on-exec: no child
/// (above all a daemon the installer starts) inherits it.
struct Lock {
    /// Held open for the lock's lifetime; closing it releases the `flock`.
    _file: fs::File,
}

impl Lock {
    fn acquire(path: &Path) -> Result<Self, RunError> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        let started = Instant::now();
        let mut announced = false;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _file: file });
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error.into());
            }
            if started.elapsed() >= lock_wait() {
                return Err(refuse(
                    "an earlier update is still running; try again later",
                ));
            }
            if !announced {
                eprintln!("herdr-threads: waiting for an earlier update to finish");
                announced = true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

const WORK_PREFIX: &str = ".herdr-threads-update.";

/// The run's work directory beside `P` (same filesystem, executable); removed on drop
/// unless kept for the journal.
struct WorkDir {
    path: PathBuf,
    keep: bool,
}

impl WorkDir {
    fn create(parent: &Path) -> Result<Self, RunError> {
        let path = parent.join(format!("{WORK_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|error| refuse(format!("cannot create {}: {error}", path.display())))?;
        Ok(Self { path, keep: false })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// From here the journal owns the directory: it is removed when the run concludes.
    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::release::Asset;

    /// A release of `version` for `platform` under a fresh directory, published as `file://`
    /// assets with correct digests; `tweak` edits the package tree before it is packed.
    fn fixture(version: &str, platform: &str, tweak: impl FnOnce(&Path)) -> (Release, PathBuf) {
        let root = crate::test_support::scratch::temp_path(format!(
            "update-verify-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let package = root.join("stage").join("herdr-threads");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::create_dir_all(package.join("scripts")).unwrap();
        fs::write(package.join("VERSION"), format!("{version}\n")).unwrap();
        fs::write(package.join("TARGET"), format!("{platform}\n")).unwrap();
        fs::write(package.join("scripts/install.sh"), "#!/bin/sh\n").unwrap();
        let binary = package.join("bin/herdr-threads");
        fs::write(
            &binary,
            format!("#!/bin/sh\necho 'herdr-threads {version}'\n"),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        tweak(&root.join("stage"));
        let asset = release::asset_name(platform);
        let published = root.join("published");
        fs::create_dir_all(&published).unwrap();
        let archive = published.join(&asset);
        let entries: Vec<_> = fs::read_dir(root.join("stage"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(root.join("stage"))
                .args(&entries)
                .status()
                .unwrap()
                .success()
        );
        let hash = release::sha256_file(&archive).unwrap();
        let sums = published.join(release::SUMS_ASSET);
        fs::write(&sums, format!("{hash}  {asset}\n")).unwrap();
        let release = Release {
            version: Version::parse(version).unwrap(),
            assets: vec![
                Asset {
                    name: asset,
                    sha256: Some(hash),
                    url: file_url(&archive),
                },
                Asset {
                    name: release::SUMS_ASSET.to_owned(),
                    sha256: Some(release::sha256_file(&sums).unwrap()),
                    url: file_url(&sums),
                },
            ],
        };
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();
        (release, work)
    }

    const PLATFORM: &str = "linux-x86_64";

    fn refusal(release: &Release, work: &Path) -> String {
        match verify(release, PLATFORM, work) {
            Ok(_) => panic!("verified"),
            Err(reason) => reason,
        }
    }

    #[test]
    fn a_correct_release_verifies() {
        let (release, work) = fixture("2.7.1", PLATFORM, |_| {});
        let verified = verify(&release, PLATFORM, &work).unwrap();
        assert!(verified.install_script.is_file());
        assert_eq!(Some(&verified.sha256), release.assets[0].sha256.as_ref());
    }

    #[test]
    fn each_digest_disagreement_refuses() {
        let (mut release, work) = fixture("2.7.1", PLATFORM, |_| {});
        release.assets[0].sha256 = Some("0".repeat(64));
        assert!(refusal(&release, &work).contains("checksum mismatch"));

        let (mut release, work) = fixture("2.7.1", PLATFORM, |_| {});
        release.assets[1].sha256 = Some("0".repeat(64));
        assert!(refusal(&release, &work).contains("SHA256SUMS digest mismatch"));

        let (mut release, work) = fixture("2.7.1", PLATFORM, |_| {});
        release.assets[0].sha256 = None;
        assert!(refusal(&release, &work).contains("gives no sha256 digest"));

        let (release, work) = fixture("2.7.1", PLATFORM, |_| {});
        let sums = release.assets[1]
            .url
            .trim_start_matches("file://")
            .to_owned();
        fs::write(&sums, format!("{}  other.tar.gz\n", "a".repeat(64))).unwrap();
        let mut release = release;
        release.assets[1].sha256 = Some(release::sha256_file(Path::new(&sums)).unwrap());
        assert!(refusal(&release, &work).contains("SHA256SUMS has no entry"));

        let (mut release, work) = fixture("2.7.1", PLATFORM, |_| {});
        release.assets.remove(0);
        assert!(refusal(&release, &work).contains("has no herdr-threads-linux-x86_64.tar.gz"));
    }

    #[test]
    fn each_archive_content_mismatch_refuses() {
        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::write(stage.join("herdr-threads/VERSION"), "2.7.0\n").unwrap();
        });
        assert!(refusal(&release, &work).contains("VERSION is 2.7.0"));

        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::write(stage.join("herdr-threads/TARGET"), "macos-aarch64\n").unwrap();
        });
        assert!(refusal(&release, &work).contains("TARGET is macos-aarch64"));

        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::remove_file(stage.join("herdr-threads/scripts/install.sh")).unwrap();
        });
        assert!(refusal(&release, &work).contains("no scripts/install.sh"));

        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::write(stage.join("extra"), "x").unwrap();
        });
        assert!(refusal(&release, &work).contains("exactly one herdr-threads/ directory"));

        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::write(
                stage.join("herdr-threads/bin/herdr-threads"),
                "#!/bin/sh\necho 'herdr-threads 9.9.9'\n",
            )
            .unwrap();
        });
        assert!(refusal(&release, &work).contains("executable reports 9.9.9"));

        let (release, work) = fixture("2.7.1", PLATFORM, |stage| {
            fs::write(
                stage.join("herdr-threads/bin/herdr-threads"),
                "#!/bin/sh\nexit 1\n",
            )
            .unwrap();
        });
        assert!(refusal(&release, &work).contains("does not run here"));
    }

    #[test]
    fn an_unreadable_journal_refuses_instead_of_reading_as_none() {
        let path = crate::test_support::scratch::temp_path(format!(
            "update-journal-{}",
            uuid::Uuid::new_v4().simple()
        ));
        assert_eq!(Journal::read(&path).unwrap().map(|j| j.target), None);
        fs::write(&path, b"{not json").unwrap();
        assert!(Journal::read(&path).unwrap_err().contains("cannot parse"));
        fs::write(
            &path,
            br#"{"target":"2.7.1","previous":"2.7.0","work":"/w","flags":[],"states":[]}"#,
        )
        .unwrap();
        let journal = Journal::read(&path).unwrap().unwrap();
        assert_eq!((journal.version, journal.installer_pid), (0, None));
    }

    #[test]
    fn flags_replay_the_marker_opt_outs() {
        let marker = Marker {
            no_herdr: true,
            without_permissions: true,
            ..Marker::default()
        };
        assert_eq!(
            install_flags(&UpdateRequest::default(), &marker),
            ["--without-permissions", "--no-herdr"]
        );
        let request = UpdateRequest {
            setup: Some(true),
            no_aisw_profiles: true,
            ..UpdateRequest::default()
        };
        assert_eq!(
            install_flags(&request, &Marker::default()),
            ["--setup", "--no-aisw-profiles"]
        );
    }

    #[test]
    fn file_urls_escape_unsafe_bytes() {
        assert_eq!(file_url(Path::new("/a b/c")), "file:///a%20b/c");
    }

    #[test]
    fn backups_are_read_from_the_installer_log() {
        let log = crate::test_support::scratch::temp_path(format!(
            "update-log-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::write(
            &log,
            "herdr-threads: x\nherdr-threads: backed up /s to /b/pre-v2.7.0-1 (schema 4, integrity ok)\n",
        )
        .unwrap();
        assert_eq!(backups_in(&log), ["/b/pre-v2.7.0-1"]);
    }
}
