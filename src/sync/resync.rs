//! `sync resync`: the restart-mode rebuild of a damaged store.
//!
//! Responsibility (Resync)
//! - Owns: the request preconditions and their order, the atomic marker
//!   writes, the step-0 phase transitions (backup by `VACUUM INTO`, moving
//!   the old files into `orig/`, choosing `backup_db`), the plan handed to
//!   composition, and deleting the marker when salvage ends.
//! - Hides: the marker format (`resync.json` v1), the backup directory layout
//!   (`backups/<unix_ms>/{herdr-threads.db,orig/}`), and that a rebuild is a
//!   restart against a missing store.
//! - Must not: delete the old database or the backups, swap a store under a
//!   running daemon, or admit salvaged rows except through
//!   `JournalPort::ingest` (which verifies every entry).
//! - Relationships: [`request`] is called by the operator-gated CLI path and
//!   uses the daemon's existing graceful stop (the shutdown token).
//!   [`resume_at_start`] is step 0 of `run_elected_impl`; `app.rs` turns the
//!   [`ResyncPlan`] into dial targets, the rx pin and the salvage job.
//! - Lifecycle: the marker file is the persisted state. Each phase is
//!   restart-safe, and `moved`/`salvaging` never back up a second time.
//! - Errors: request refusals are kept apart: `InvalidRequest` (unknown
//!   node), `HostUnavailable` (no path to the node, the wire code of the
//!   Unavailable class) and `StoreFull` (no room for the backup). A failed
//!   vacuum is logged and the rebuild continues. A start failure is a
//!   [`StartError`].
//!
//! This is the one sync module that opens SQLite itself: step 0 runs before
//! any store exists, so the backup reads the old file directly.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::daemon::paths::NodePaths;
use crate::node::{NodeTag, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::store::sync_dial_targets::DialTarget;
use crate::sync::coordinator::SyncHandle;

/// The marker's file name inside the node directory.
pub const MARKER_FILE: &str = "resync.json";

/// The marker format version this build reads and writes.
const MARKER_VERSION: u32 = 1;

/// Free space required beyond the database and its WAL before a request is
/// accepted.
const HEADROOM_BYTES: u64 = 64 * 1024 * 1024;

const MIB: u64 = 1024 * 1024;

/// How long [`request`] waits for the coordinator's link snapshot.
const REQUEST_BUDGET_MS: u64 = 5_000;

const BACKUPS_DIR: &str = "backups";
const ORIG_DIR: &str = "orig";
const DB_FILE: &str = crate::daemon::paths::DATABASE_FILE;
const VACUUM_PARTIAL: &str = "herdr-threads.db.partial";

/// Where a rebuild stands. Closed: a new step is a new phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResyncPhase {
    /// Asked for; nothing backed up or moved yet.
    Requested,
    /// The old files are in `orig/`; the next start builds a fresh store.
    Moved,
    /// The fresh store is up and the backup is being read back in.
    Salvaging,
}

/// A dial target to re-add after the rebuild, so the path to `from` survives
/// the loss of the store that held it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResyncTarget {
    pub ssh_dest: String,
    pub remote_command: Option<String>,
}

/// `<node_dir>/resync.json` v1: the persisted state of one rebuild.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResyncMarker {
    pub v: u32,
    /// The peer the rebuilt store is pulled from.
    pub from: NodeUuid,
    /// The nodes whose logs must come from `from`; `None` means every log.
    pub logs: Option<Vec<NodeUuid>>,
    /// Provenance of the request (`operator:local-user:<uid>`).
    pub operator: String,
    /// Unix ms of the request.
    pub requested_at: i64,
    pub phase: ResyncPhase,
    /// `backups/<unix_ms>`, relative to the node directory; set from the
    /// moment the backup directory exists.
    pub backup_dir: Option<String>,
    pub targets: Vec<ResyncTarget>,
}

impl ResyncMarker {
    /// The marker of a node directory; `None` when no rebuild is pending.
    pub fn read(paths: &NodePaths) -> Result<Option<Self>, StartError> {
        let path = marker_path(paths);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(StartError::io("read the resync marker", error)),
        };
        let marker: Self = serde_json::from_slice(&bytes)
            .map_err(|error| StartError::Marker(format!("malformed {MARKER_FILE}: {error}")))?;
        if marker.v != MARKER_VERSION {
            return Err(StartError::Marker(format!(
                "unsupported {MARKER_FILE} version {}",
                marker.v
            )));
        }
        Ok(Some(marker))
    }

    /// Replaces the marker atomically: temp file, fsync, rename, directory
    /// fsync.
    pub fn write(&self, paths: &NodePaths) -> Result<(), StartError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| StartError::Marker(format!("encode {MARKER_FILE}: {error}")))?;
        atomic_write(&marker_path(paths), &bytes)
            .map_err(|error| StartError::io("write the resync marker", error))
    }

    /// Deletes the marker: salvage ended. The backups stay.
    pub fn finish(paths: &NodePaths) -> Result<(), StartError> {
        match fs::remove_file(marker_path(paths)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StartError::io("remove the resync marker", error)),
        }
        sync_dir(&paths.node_dir).map_err(|error| StartError::io("sync the node directory", error))
    }
}

/// What step 0 hands to composition: whom to rebuild from and where the old
/// store's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResyncPlan {
    pub from: NodeUuid,
    /// The nodes whose logs must come from `from`; `None` means every log.
    pub logs: Option<Vec<NodeUuid>>,
    /// The vacuumed copy if the vacuum succeeded, else `orig/herdr-threads.db`.
    pub backup_db: PathBuf,
}

/// A start-time failure of the rebuild; the daemon refuses to start.
#[derive(Debug)]
pub enum StartError {
    Io {
        what: &'static str,
        source: io::Error,
    },
    /// The marker is unreadable or from another format version.
    Marker(String),
    /// A failpoint fired (test builds only).
    Api(ApiError),
}

impl StartError {
    fn io(what: &'static str, source: io::Error) -> Self {
        Self::Io { what, source }
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { what, source } => write!(f, "resync: could not {what}: {source}"),
            Self::Marker(detail) => write!(f, "resync: {detail}"),
            Self::Api(error) => write!(f, "resync: {}", error.detail),
        }
    }
}

impl std::error::Error for StartError {}

impl From<ApiError> for StartError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl From<StartError> for io::Error {
    fn from(error: StartError) -> Self {
        let kind = match &error {
            StartError::Io { source, .. } => source.kind(),
            StartError::Marker(_) => io::ErrorKind::InvalidData,
            StartError::Api(_) => io::ErrorKind::Other,
        };
        io::Error::new(kind, error.to_string())
    }
}

/// The store reads [`request`] needs, behind a narrow port.
pub trait ResyncStore: Send + Sync {
    /// The tag of `node`; `None` when the directory does not know it or it is
    /// this node itself.
    fn peer_tag(&self, node: NodeUuid, budget: &CallBudget) -> Result<Option<NodeTag>, ApiError>;

    /// The enabled dial targets that learned `node`.
    fn targets_reaching(
        &self,
        node: NodeUuid,
        budget: &CallBudget,
    ) -> Result<Vec<DialTarget>, ApiError>;
}

/// What [`request`] needs from the running daemon besides the store.
pub struct RequestEnv<'a> {
    pub clock: &'a Arc<dyn Clock>,
    /// The token the wire `Stop` cancels: the daemon's graceful stop.
    pub shutdown: &'a Cancellation,
}

/// Asks for a rebuild: checks the preconditions in order, writes the marker
/// and triggers the daemon's graceful stop (`env.shutdown`). The next start runs [`resume_at_start`].
pub async fn request(
    store: &dyn ResyncStore,
    paths: &NodePaths,
    sync: &SyncHandle,
    from: NodeUuid,
    logs: Option<Vec<NodeUuid>>,
    operator: &str,
    env: &RequestEnv<'_>,
) -> Result<(), ApiError> {
    let RequestEnv { clock, shutdown } = env;
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + REQUEST_BUDGET_MS),
        cancellation: Cancellation::default(),
    };
    let link_active = sync
        .snapshot(&budget)
        .await
        .map(|snapshot| snapshot.links.iter().any(|link| link.node == from));
    prepare_request(
        store,
        paths,
        link_active,
        from,
        logs,
        operator,
        clock.as_ref(),
        &budget,
        &free_bytes,
    )?;
    shutdown.cancel();
    Ok(())
}

/// The checks and the marker write of [`request`]. `link_active` is the
/// coordinator's answer, consulted only at the "no path" step.
#[allow(clippy::too_many_arguments)]
fn prepare_request(
    store: &dyn ResyncStore,
    paths: &NodePaths,
    link_active: Result<bool, ApiError>,
    from: NodeUuid,
    logs: Option<Vec<NodeUuid>>,
    operator: &str,
    clock: &dyn Clock,
    budget: &CallBudget,
    free_space: &dyn Fn(&Path) -> io::Result<u64>,
) -> Result<(), ApiError> {
    let tag = store
        .peer_tag(from, budget)?
        .ok_or_else(|| ApiError::new(ErrorCode::InvalidRequest, "unknown node"))?;
    let targets = store.targets_reaching(from, budget)?;
    if targets.is_empty() && !link_active? {
        return Err(ApiError::new(
            ErrorCode::HostUnavailable,
            format!("no path to {tag}"),
        ));
    }
    let needed = database_bytes(paths).saturating_add(HEADROOM_BYTES);
    let free = free_space(&paths.node_dir)
        .map_err(|error| ApiError::new(ErrorCode::StoreBusy, format!("free space: {error}")))?;
    if free < needed {
        return Err(ApiError::new(
            ErrorCode::StoreFull,
            format!(
                "resync needs {} MiB free for the backup",
                needed.div_ceil(MIB)
            ),
        ));
    }
    let pending = |detail: String| ApiError::new(ErrorCode::Conflict, detail);
    match ResyncMarker::read(paths) {
        Ok(Some(marker)) if marker.phase != ResyncPhase::Requested => {
            return Err(pending("a resync is already in progress".to_owned()));
        }
        Ok(_) => {}
        Err(error) => return Err(pending(error.to_string())),
    }
    let marker = ResyncMarker {
        v: MARKER_VERSION,
        from,
        logs,
        operator: operator.to_owned(),
        requested_at: clock.utc_now().0,
        phase: ResyncPhase::Requested,
        backup_dir: None,
        targets: targets
            .into_iter()
            .map(|target| ResyncTarget {
                ssh_dest: target.ssh_dest,
                remote_command: target.remote_command,
            })
            .collect(),
    };
    marker
        .write(paths)
        .map_err(|error| ApiError::new(ErrorCode::StoreBusy, error.to_string()))
}

/// Step 0 of the daemon start: advances a pending rebuild and returns its
/// plan; `None` when no rebuild is pending.
///
/// `requested`: back up by `VACUUM INTO`, move the old files into `orig/`,
/// record `moved`. `moved` and `salvaging` go straight to the plan. Nothing
/// is deleted.
pub fn resume_at_start(
    paths: &NodePaths,
    clock: &dyn Clock,
) -> Result<Option<ResyncPlan>, StartError> {
    let Some(mut marker) = ResyncMarker::read(paths)? else {
        return Ok(None);
    };
    if marker.phase == ResyncPhase::Requested {
        let relative = match &marker.backup_dir {
            Some(relative) => relative.clone(),
            None => {
                let relative = format!("{BACKUPS_DIR}/{}", clock.utc_now().0);
                create_private_dir(&paths.node_dir.join(&relative))
                    .map_err(|error| StartError::io("create the backup directory", error))?;
                // Recorded before any file moves, so a restart reuses this
                // directory instead of starting a second backup.
                marker.backup_dir = Some(relative.clone());
                marker.write(paths)?;
                relative
            }
        };
        let dir = paths.node_dir.join(&relative);
        vacuum_backup(paths, &dir);
        move_old_files(paths, &dir)?;
        marker.phase = ResyncPhase::Moved;
        marker.write(paths)?;
        failpoint!("resync.after_moved", paths.node_dir.display().to_string());
    }
    let Some(relative) = marker.backup_dir.as_deref() else {
        return Err(StartError::Marker(format!(
            "{MARKER_FILE} is past `requested` without a backup directory"
        )));
    };
    let dir = paths.node_dir.join(relative);
    let vacuumed = dir.join(DB_FILE);
    let backup_db = if vacuumed.is_file() {
        vacuumed
    } else {
        dir.join(ORIG_DIR).join(DB_FILE)
    };
    Ok(Some(ResyncPlan {
        from: marker.from,
        logs: marker.logs,
        backup_db,
    }))
}

/// `VACUUM INTO` a copy of the old database inside `dir`. A failure is
/// logged and the rebuild continues with the original files as the backup.
fn vacuum_backup(paths: &NodePaths, dir: &Path) {
    let target = dir.join(DB_FILE);
    if target.exists() {
        return;
    }
    let partial = dir.join(VACUUM_PARTIAL);
    let result = (|| -> Result<(), String> {
        let _ = fs::remove_file(&partial);
        let source = Connection::open_with_flags(
            &paths.database_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| error.to_string())?;
        source
            .execute("VACUUM INTO ?1", [partial.to_string_lossy().as_ref()])
            .map_err(|error| error.to_string())?;
        drop(source);
        File::open(&partial)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        fs::rename(&partial, &target).map_err(|error| error.to_string())?;
        sync_dir(dir).map_err(|error| error.to_string())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        eprintln!(
            "resync: backup of {} by VACUUM INTO failed, continuing with the original files: {error}",
            paths.database_path.display()
        );
    }
}

/// Renames the database and its `-wal`/`-shm` into `<dir>/orig/`, then
/// fsyncs. A file already moved by an interrupted earlier pass is skipped.
fn move_old_files(paths: &NodePaths, dir: &Path) -> Result<(), StartError> {
    let orig = dir.join(ORIG_DIR);
    create_private_dir(&orig)
        .map_err(|error| StartError::io("create the orig directory", error))?;
    for suffix in ["", "-wal", "-shm"] {
        let name = format!("{DB_FILE}{suffix}");
        let from = paths.node_dir.join(&name);
        match fs::rename(&from, orig.join(&name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StartError::io("move the old store aside", error)),
        }
    }
    sync_dir(&orig)
        .and_then(|()| sync_dir(&paths.node_dir))
        .map_err(|error| StartError::io("sync the moved files", error))
}

fn marker_path(paths: &NodePaths) -> PathBuf {
    paths.node_dir.join(MARKER_FILE)
}

/// Size of the database plus its WAL; absent files count as zero.
fn database_bytes(paths: &NodePaths) -> u64 {
    let size =
        |name: String| fs::metadata(paths.node_dir.join(name)).map_or(0, |metadata| metadata.len());
    size(DB_FILE.to_owned()).saturating_add(size(format!("{DB_FILE}-wal")))
}

/// Bytes available to an unprivileged writer on the filesystem of `dir`.
fn free_bytes(dir: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `path` is a valid C string and `stat` is a writable statvfs.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statvfs returned 0, so the struct is initialised.
    let stat = unsafe { stat.assume_init() };
    #[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::tag_for;
    use crate::protocol::time::UtcMillis;
    use crate::store::sync_dial_targets::DialTarget;
    use crate::sync::sync_store::MemorySyncStore;
    use crate::test_support::failpoints::Failpoint;
    use crate::test_support::isolation::TestIsolation;
    use std::os::unix::fs::PermissionsExt;

    const NOW: i64 = 1_700_000_000_000;

    struct FixedClock(i64);
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }

    fn peer() -> NodeUuid {
        NodeUuid::from_bytes([7; 16])
    }

    struct FakeStore {
        known: bool,
        targets: Vec<DialTarget>,
    }

    impl ResyncStore for FakeStore {
        fn peer_tag(&self, node: NodeUuid, _: &CallBudget) -> Result<Option<NodeTag>, ApiError> {
            Ok(self.known.then(|| tag_for(node, 0)))
        }
        fn targets_reaching(
            &self,
            _: NodeUuid,
            _: &CallBudget,
        ) -> Result<Vec<DialTarget>, ApiError> {
            Ok(self.targets.clone())
        }
    }

    fn target(dest: &str, remote_command: Option<&str>) -> DialTarget {
        let store = MemorySyncStore::new();
        store.add_target(dest);
        let mut row = store.targets().remove(0);
        row.remote_command = remote_command.map(str::to_owned);
        row
    }

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        }
    }

    fn node(label: &str) -> (TestIsolation, NodePaths) {
        let iso = TestIsolation::new(label);
        let paths = crate::test_support::resolve_seeded(iso.state_root());
        (iso, paths)
    }

    /// A real SQLite database with one row, as the damaged store.
    fn seed_database(paths: &NodePaths) {
        let conn = Connection::open(&paths.database_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE t(x TEXT); INSERT INTO t VALUES ('old-row');",
        )
        .unwrap();
        // Keep the WAL on disk: a live writer leaves -wal/-shm behind.
        std::mem::forget(conn);
    }

    fn run_request(
        store: &FakeStore,
        paths: &NodePaths,
        link: Result<bool, ApiError>,
        free: u64,
    ) -> Result<(), ApiError> {
        prepare_request(
            store,
            paths,
            link,
            peer(),
            None,
            "operator:local-user:501",
            &FixedClock(NOW),
            &budget(),
            &move |_| Ok(free),
        )
    }

    fn backups(paths: &NodePaths) -> Vec<PathBuf> {
        match fs::read_dir(paths.node_dir.join(BACKUPS_DIR)) {
            Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn requested_marker(paths: &NodePaths) -> ResyncMarker {
        let marker = ResyncMarker {
            v: 1,
            from: peer(),
            logs: Some(vec![NodeUuid::from_bytes([9; 16])]),
            operator: "operator:local-user:501".to_owned(),
            requested_at: NOW,
            phase: ResyncPhase::Requested,
            backup_dir: None,
            targets: vec![ResyncTarget {
                ssh_dest: "peer.example".to_owned(),
                remote_command: None,
            }],
        };
        marker.write(paths).unwrap();
        marker
    }

    fn row_of(db: &Path) -> String {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn request_refusals_in_order() {
        let (_iso, paths) = node("resync-refusals");
        seed_database(&paths);
        let plenty = u64::MAX;
        // Unknown node wins over every later failure (no path, no space).
        let unknown = FakeStore {
            known: false,
            targets: Vec::new(),
        };
        let error = run_request(&unknown, &paths, Ok(false), 0).unwrap_err();
        assert_eq!(
            (error.code, error.detail.as_str()),
            (ErrorCode::InvalidRequest, "unknown node")
        );
        // Known but unreachable wins over no space.
        let known = FakeStore {
            known: true,
            targets: Vec::new(),
        };
        let error = run_request(&known, &paths, Ok(false), 0).unwrap_err();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert_eq!(error.detail, format!("no path to {}", tag_for(peer(), 0)));
        // A failed snapshot is reported only at the path step.
        let busy = ApiError::new(ErrorCode::StoreBusy, "sync coordinator did not answer");
        let error = run_request(&known, &paths, Err(busy), plenty).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreBusy);
        // A path (an active link) but no space.
        let error = run_request(&known, &paths, Ok(true), 0).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreFull);
        assert_eq!(error.detail, "resync needs 65 MiB free for the backup");
        // No refusal wrote a marker.
        assert_eq!(ResyncMarker::read(&paths).unwrap(), None);
        // A learned dial target is a path as well.
        let reachable = FakeStore {
            known: true,
            targets: vec![target("peer.example", None)],
        };
        run_request(&reachable, &paths, Ok(false), plenty).unwrap();
    }

    #[test]
    fn marker_round_trip_and_atomic_write() {
        let (_iso, paths) = node("resync-marker");
        assert_eq!(ResyncMarker::read(&paths).unwrap(), None);
        let marker = requested_marker(&paths);
        assert_eq!(ResyncMarker::read(&paths).unwrap(), Some(marker.clone()));
        let text = fs::read_to_string(marker_path(&paths)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["phase"], "requested");
        assert_eq!(value["backup_dir"], serde_json::Value::Null);
        assert_eq!(value["from"], peer().to_string());
        assert_eq!(value["targets"][0]["ssh_dest"], "peer.example");
        let mode = fs::metadata(marker_path(&paths))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // Replacing leaves exactly one marker file and no temp sibling.
        let mut moved = marker;
        moved.phase = ResyncPhase::Moved;
        moved.write(&paths).unwrap();
        assert_eq!(
            ResyncMarker::read(&paths).unwrap().unwrap().phase,
            ResyncPhase::Moved
        );
        let leftovers: Vec<_> = fs::read_dir(&paths.node_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("resync.json"))
            .collect();
        assert_eq!(leftovers, vec![MARKER_FILE.to_owned()]);
        // An unknown version is a start failure, not a silent reset.
        fs::write(marker_path(&paths), text.replace("\"v\":1", "\"v\":2")).unwrap();
        assert!(matches!(
            ResyncMarker::read(&paths),
            Err(StartError::Marker(_))
        ));
        ResyncMarker::finish(&paths).unwrap();
        assert_eq!(ResyncMarker::read(&paths).unwrap(), None);
    }

    #[test]
    fn request_writes_requested_marker_with_reaching_targets() {
        let (_iso, paths) = node("resync-request-marker");
        let store = FakeStore {
            known: true,
            targets: vec![target(
                "peer.example",
                Some("exec herdr-threads peer --stdio"),
            )],
        };
        run_request(&store, &paths, Ok(false), u64::MAX).unwrap();
        let marker = ResyncMarker::read(&paths).unwrap().unwrap();
        assert_eq!(marker.phase, ResyncPhase::Requested);
        assert_eq!(marker.from, peer());
        assert_eq!(marker.logs, None);
        assert_eq!(marker.requested_at, NOW);
        assert_eq!(marker.operator, "operator:local-user:501");
        assert_eq!(
            marker.targets,
            vec![ResyncTarget {
                ssh_dest: "peer.example".to_owned(),
                remote_command: Some("exec herdr-threads peer --stdio".to_owned()),
            }]
        );
        // Moved/salvaging markers are never overwritten by a second request.
        let mut moved = marker;
        moved.phase = ResyncPhase::Moved;
        moved.write(&paths).unwrap();
        let error = run_request(&store, &paths, Ok(false), u64::MAX).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(
            ResyncMarker::read(&paths).unwrap().unwrap().phase,
            ResyncPhase::Moved
        );
    }

    #[test]
    fn no_marker_means_no_plan() {
        let (_iso, paths) = node("resync-none");
        assert_eq!(resume_at_start(&paths, &FixedClock(NOW)).unwrap(), None);
        assert!(backups(&paths).is_empty());
    }

    #[test]
    fn requested_backs_up_moves_and_sets_moved() {
        let (_iso, paths) = node("resync-requested");
        seed_database(&paths);
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        let dir = paths.node_dir.join(BACKUPS_DIR).join(NOW.to_string());
        assert_eq!(backups(&paths), vec![dir.clone()]);
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // The vacuumed copy is the plan's backup and holds the data.
        assert_eq!(plan.backup_db, dir.join(DB_FILE));
        assert_eq!(row_of(&plan.backup_db), "old-row");
        assert_eq!(plan.from, peer());
        assert_eq!(plan.logs, Some(vec![NodeUuid::from_bytes([9; 16])]));
        // The old files moved, intact, into orig/; the node dir has no store.
        assert!(!paths.database_path.exists());
        assert!(!paths.node_dir.join(format!("{DB_FILE}-wal")).exists());
        assert_eq!(row_of(&dir.join(ORIG_DIR).join(DB_FILE)), "old-row");
        let marker = ResyncMarker::read(&paths).unwrap().unwrap();
        assert_eq!(marker.phase, ResyncPhase::Moved);
        assert_eq!(
            marker.backup_dir.as_deref(),
            Some(format!("{BACKUPS_DIR}/{NOW}").as_str())
        );
    }

    #[test]
    fn moved_never_backs_up_twice() {
        let (_iso, paths) = node("resync-moved");
        seed_database(&paths);
        requested_marker(&paths);
        let first = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        // A later start, with a different clock and a fresh (empty) store
        // file in place: no second backup, the same plan.
        fs::write(&paths.database_path, b"new store").unwrap();
        let second = resume_at_start(&paths, &FixedClock(NOW + 5_000))
            .unwrap()
            .unwrap();
        assert_eq!(second, first);
        assert_eq!(backups(&paths).len(), 1);
        assert_eq!(fs::read(&paths.database_path).unwrap(), b"new store");
        // Salvaging resumes the same way.
        let mut marker = ResyncMarker::read(&paths).unwrap().unwrap();
        marker.phase = ResyncPhase::Salvaging;
        marker.write(&paths).unwrap();
        let third = resume_at_start(&paths, &FixedClock(NOW + 9_000))
            .unwrap()
            .unwrap();
        assert_eq!(third, first);
        assert_eq!(backups(&paths).len(), 1);
    }

    #[test]
    fn failpoint_after_moved_then_second_call_reaches_plan_one_backup() {
        let (_iso, paths) = node("resync-failpoint");
        seed_database(&paths);
        requested_marker(&paths);
        let scope = paths.node_dir.display().to_string();
        let armed = Failpoint::error("resync.after_moved", scope, ErrorCode::StoreBusy);
        let error = resume_at_start(&paths, &FixedClock(NOW)).unwrap_err();
        assert!(matches!(error, StartError::Api(_)), "{error}");
        assert_eq!(armed.fired(), 1);
        // The crash left the `moved` marker and the moved files behind.
        assert_eq!(
            ResyncMarker::read(&paths).unwrap().unwrap().phase,
            ResyncPhase::Moved
        );
        assert!(!paths.database_path.exists());
        let plan = resume_at_start(&paths, &FixedClock(NOW + 1_000))
            .unwrap()
            .unwrap();
        assert_eq!(backups(&paths).len(), 1);
        assert_eq!(row_of(&plan.backup_db), "old-row");
    }

    #[test]
    fn failed_vacuum_logged_plan_uses_orig_db() {
        let (_iso, paths) = node("resync-vacuum-fail");
        // Not a SQLite file: the read-only open or the vacuum fails.
        fs::write(&paths.database_path, b"this is not a database at all").unwrap();
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        let dir = paths.node_dir.join(BACKUPS_DIR).join(NOW.to_string());
        assert!(!dir.join(DB_FILE).exists());
        assert!(!dir.join(VACUUM_PARTIAL).exists());
        assert_eq!(plan.backup_db, dir.join(ORIG_DIR).join(DB_FILE));
        assert_eq!(
            fs::read(&plan.backup_db).unwrap(),
            b"this is not a database at all"
        );
        assert!(!paths.database_path.exists());
        assert_eq!(
            ResyncMarker::read(&paths).unwrap().unwrap().phase,
            ResyncPhase::Moved
        );
    }

    #[test]
    fn restart_inside_requested_reuses_the_backup_directory() {
        let (_iso, paths) = node("resync-requested-restart");
        seed_database(&paths);
        let mut marker = requested_marker(&paths);
        // A crash after the directory was recorded but before anything moved.
        let relative = format!("{BACKUPS_DIR}/{NOW}");
        create_private_dir(&paths.node_dir.join(&relative)).unwrap();
        marker.backup_dir = Some(relative);
        marker.write(&paths).unwrap();
        let plan = resume_at_start(&paths, &FixedClock(NOW + 7_000))
            .unwrap()
            .unwrap();
        assert_eq!(backups(&paths).len(), 1);
        assert_eq!(row_of(&plan.backup_db), "old-row");
    }

    #[test]
    fn free_bytes_reports_the_filesystem() {
        let (_iso, paths) = node("resync-free");
        assert!(free_bytes(&paths.node_dir).unwrap() > 0);
        assert!(free_bytes(&paths.node_dir.join("missing")).is_err());
    }
}
