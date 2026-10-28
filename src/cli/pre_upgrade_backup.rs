//! Hidden `herdr-threads internal pre-upgrade-backup --state-dir DIR --target-version TAG` for
//! scripts/install.sh: before an upgrade replaces anything, copy the node store (`VACUUM
//! INTO`, consistent while the daemon runs), `node.key` and `node.json` into
//! `<parent>/herdr-threads-backups/pre-v<TAG>-<YYYYmmddTHHMMSSZ>` (0700, files 0600), check
//! the copy, then prune older entries of that directory
//! (`docs/superpowers/specs/2026-10-28-installer-pre-upgrade-backup-design.md`).
//!
//! - Owns: the backup root and entry name grammar, staging and atomic rename, the copy's
//!   check and the retention policy.
//! - Must not: print or format `node.key`'s bytes, follow or remove a symlink, touch an entry
//!   off the grammar (the manual `herdr-threads-pre-v*-backup-*` directories beside the state
//!   directory are never read), or write to a backup it only reads (`immutable=1`).
//!
//! Local only; never contacts the daemon.
use crate::daemon::paths::{DATABASE_FILE, node_dir};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

/// The installer-owned backup directory, a sibling of the state directory.
pub const ROOT: &str = "herdr-threads-backups";
const ENTRY_PREFIX: &str = "pre-v";
const STAGING_PREFIX: &str = ".staging-";
const KEEP_NEWEST: usize = 3;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// What a run did, for the caller's output.
#[derive(Debug)]
pub enum Outcome {
    /// No `node/herdr-threads.db` under the state directory.
    NoStore,
    Taken {
        backup: PathBuf,
        schema: i64,
        prune: PruneReport,
    },
}

#[derive(Debug, Default)]
pub struct PruneReport {
    pub pruned: Vec<PathBuf>,
    /// Kept because the schema could not be read: (path, reason).
    pub unreadable: Vec<(PathBuf, String)>,
    /// Deletions that failed: (path, reason).
    pub failed: Vec<(PathBuf, String)>,
}

/// The CLI entry: run, print one line per event, exit 1 on a failed backup.
pub fn run<W: Write>(
    state_dir: &Path,
    version: &str,
    writer: &mut W,
) -> Result<(), super::RunError> {
    match backup_and_prune(state_dir, version, SystemTime::now()) {
        Ok(Outcome::NoStore) => {
            writeln!(
                writer,
                "herdr-threads: no store in {}; no backup taken",
                state_dir.display()
            )?;
        }
        Ok(Outcome::Taken {
            backup,
            schema,
            prune,
        }) => {
            writeln!(
                writer,
                "herdr-threads: backed up {} to {} (schema {schema}, integrity ok)",
                state_dir.display(),
                backup.display()
            )?;
            for path in &prune.pruned {
                writeln!(
                    writer,
                    "herdr-threads: pruned old backup {}",
                    path.display()
                )?;
            }
            writer.flush()?;
            for (path, reason) in &prune.unreadable {
                eprintln!(
                    "herdr-threads: warning: kept {}: cannot read its store schema ({reason})",
                    path.display()
                );
            }
            for (path, reason) in &prune.failed {
                eprintln!(
                    "herdr-threads: warning: could not prune {}: {reason}",
                    path.display()
                );
            }
        }
        Err(reason) => {
            writer.flush()?;
            eprintln!("herdr-threads: error: pre-upgrade backup failed: {reason}");
            return Err(super::RunError::Exit(1));
        }
    }
    writer.flush()?;
    Ok(())
}

/// Take the backup of `state_dir`'s node store, then prune the backup root. An error means
/// no backup exists (the staging directory is removed); pruning never fails the run.
pub fn backup_and_prune(
    state_dir: &Path,
    version: &str,
    now: SystemTime,
) -> Result<Outcome, String> {
    let source = node_dir(state_dir).join(DATABASE_FILE);
    match fs::symlink_metadata(&source) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Err(format!("{} is not a regular file", source.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Outcome::NoStore),
        Err(error) => return Err(format!("{}: {error}", source.display())),
    }
    let tag = version.strip_prefix('v').unwrap_or(version);
    if !valid_tag(tag) {
        return Err(format!("invalid version {version:?}"));
    }
    let root = state_dir
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", state_dir.display()))?
        .join(ROOT);
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(format!("{} is not a directory", root.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_private_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        }
        Err(error) => return Err(format!("{}: {error}", root.display())),
    }
    sweep_stale_staging(&root);
    let staging = root.join(format!("{STAGING_PREFIX}{}", std::process::id()));
    create_private_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let result = (|| {
        let schema = copy_store(&source, &staging.join(DATABASE_FILE))?;
        for name in [
            crate::node::identity::KEY_FILE,
            crate::node::json::NODE_JSON_FILE,
        ] {
            copy_private(&node_dir(state_dir).join(name), &staging.join(name))?;
        }
        sync_dir(&staging)?;
        let backup = free_name(&root, tag, now)?;
        fs::rename(&staging, &backup).map_err(|error| format!("{}: {error}", backup.display()))?;
        sync_dir(&root)?;
        Ok((backup, schema))
    })();
    let (backup, schema) = match result {
        Ok(taken) => taken,
        Err(reason) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(reason);
        }
    };
    let prune = prune(&root, &backup);
    Ok(Outcome::Taken {
        backup,
        schema,
        prune,
    })
}

/// Remove staging directories a killed run left behind: `.staging-<pid>` real directories
/// whose pid is not a live process (or is this one, from an earlier run with a reused pid).
fn sweep_stale_staging(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix(STAGING_PREFIX))
            .and_then(|pid| pid.parse::<i32>().ok())
        else {
            continue;
        };
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if pid > 0 && pid as u32 != std::process::id() && process_alive(pid) {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
    }
}

fn process_alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

/// `VACUUM INTO` the live store, then check the copy: `integrity_check` must be exactly
/// `ok`. Returns the copy's `user_version`.
fn copy_store(source: &Path, target: &Path) -> Result<i64, String> {
    let describe = |error: rusqlite::Error| format!("{}: {error}", source.display());
    let connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(describe)?;
    connection.busy_timeout(BUSY_TIMEOUT).map_err(describe)?;
    let target_text = target
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", target.display()))?;
    // VACUUM INTO reads one consistent snapshot (a single read transaction of its own).
    connection
        .execute("VACUUM INTO ?1", [target_text])
        .map_err(describe)?;
    drop(connection);
    set_private(target)?;
    sync_file(target)?;
    check_copy(target)
}

/// The copy's check: `integrity_check` must be exactly `ok`. Returns its `user_version`.
fn check_copy(target: &Path) -> Result<i64, String> {
    let copy = Connection::open_with_flags(
        target,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("{}: {error}", target.display()))?;
    let check: String = copy
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("integrity check of the copy: {error}"))?;
    if check != "ok" {
        return Err(format!("the copy failed its integrity check: {check}"));
    }
    copy.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("schema of the copy: {error}"))
}

/// Copy `from` to `to` byte for byte (never through a string), 0600, fsynced. A missing
/// `from` is skipped.
fn copy_private(from: &Path, to: &Path) -> Result<(), String> {
    match fs::symlink_metadata(from) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Err(format!("{} is not a regular file", from.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("{}: {error}", from.display())),
    }
    fs::copy(from, to).map_err(|error| format!("{}: {error}", from.display()))?;
    set_private(to)?;
    sync_file(to)
}

fn set_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(())
}

fn sync_file(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)?;
    // The umask can only narrow the mode; never leave it wider than 0700.
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), String> {
    sync_file(path)
}

/// `<root>/pre-v<tag>-<YYYYmmddTHHMMSSZ>`, with `-2`, `-3`, … when that name is taken.
fn free_name(root: &Path, tag: &str, now: SystemTime) -> Result<PathBuf, String> {
    let millis = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| "the clock is before 1970".to_string())?
        .as_millis();
    let stamp: String = crate::harness::manifest::format_rfc3339_utc(
        i64::try_from(millis).map_err(|_| "the clock is out of range".to_string())?,
    )
    .chars()
    .filter(|c| !matches!(c, '-' | ':'))
    .collect();
    let base = format!("{ENTRY_PREFIX}{tag}-{stamp}");
    for n in 1..1000 {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let path = root.join(name);
        if fs::symlink_metadata(&path).is_err() {
            return Ok(path);
        }
    }
    Err(format!(
        "no free backup name for {base} in {}",
        root.display()
    ))
}

/// The sort key of an entry name on the grammar
/// `pre-v[0-9A-Za-z.+-]+-[0-9]{8}T[0-9]{6}Z(-[0-9]+)?`: its stamp digits and suffix number.
/// `None` for any other name.
fn entry_key(name: &str) -> Option<(u64, u32)> {
    let rest = name.strip_prefix(ENTRY_PREFIX)?;
    if !rest.is_ascii() {
        return None;
    }
    let (rest, suffix) = match rest.rsplit_once('-') {
        Some((head, n))
            if head.ends_with('Z') && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (head, n.parse::<u32>().ok()?)
        }
        _ => (rest, 1),
    };
    // `<tag>-YYYYmmddTHHMMSSZ`: the stamp is the last 16 bytes, after a '-'.
    let split = rest.len().checked_sub(17)?;
    let (tag, stamp) = rest.split_at(split);
    let stamp = stamp.strip_prefix('-')?;
    if !valid_tag(tag) {
        return None;
    }
    let bytes = stamp.as_bytes();
    if bytes[8] != b'T' || bytes[15] != b'Z' {
        return None;
    }
    let digits = format!("{}{}", &stamp[..8], &stamp[9..15]);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, suffix))
}

/// One backup entry as pruning sees it.
#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    key: (u64, u32),
    name: String,
    schema: Result<i64, String>,
}

/// Keep the backup just taken, the newest 3, the newest per schema and every entry whose
/// schema cannot be read; remove the rest. Only entries on the grammar are considered.
fn prune(root: &Path, taken: &Path) -> PruneReport {
    let mut report = PruneReport::default();
    let Ok(entries) = fs::read_dir(root) else {
        return report;
    };
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(key) = entry_key(&name) else {
            continue;
        };
        let path = entry.path();
        // symlink_metadata: a symlink is never a candidate, whatever it points at.
        if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let schema = user_version(&path.join(DATABASE_FILE));
        candidates.push(Candidate {
            path,
            key,
            name,
            schema,
        });
    }
    // Newest first; the name breaks remaining ties, so the order is total.
    candidates.sort_by(|a, b| b.key.cmp(&a.key).then_with(|| b.name.cmp(&a.name)));
    let mut schemas = HashSet::new();
    for (index, candidate) in candidates.into_iter().enumerate() {
        let newest_of_schema = match &candidate.schema {
            Err(reason) => {
                report
                    .unreadable
                    .push((candidate.path.clone(), reason.clone()));
                true
            }
            Ok(schema) => schemas.insert(*schema),
        };
        if newest_of_schema || index < KEEP_NEWEST || candidate.path == taken {
            continue;
        }
        match fs::remove_dir_all(&candidate.path) {
            Ok(()) => report.pruned.push(candidate.path),
            Err(error) => report.failed.push((candidate.path, error.to_string())),
        }
    }
    report
}

/// `PRAGMA user_version` of a backup store, opened read-only and immutable: no `-wal` or
/// `-shm` is created and nothing is written.
fn user_version(store: &Path) -> Result<i64, String> {
    if !fs::symlink_metadata(store).is_ok_and(|meta| meta.is_file()) {
        return Err(format!("no {DATABASE_FILE}"));
    }
    let text = store.to_str().ok_or("path is not UTF-8")?;
    let uri = format!("file:{}?mode=ro&immutable=1", uri_escape(text));
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())?;
    // A file that is not SQLite opens fine and fails on the first read.
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

/// Percent-escape the characters a SQLite URI path treats specially.
fn uri_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3f"),
            '#' => out.push_str("%23"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "pre_upgrade_backup_tests.rs"]
mod tests;
