//! `sync resync`: the restart-mode rebuild of a damaged store.
//!
//! Responsibility (Resync)
//! - Owns: the request preconditions and their order, the atomic marker
//!   writes, the step-0 phase transitions (backup by `VACUUM INTO`, moving
//!   the old files into `orig/`, choosing `backup_db`), the plan handed to
//!   composition, the salvage reader (the backup read back, per log, into an
//!   ingest callback) and deleting the marker when every log was read back (a
//!   log stopped for lack of disk headroom keeps it).
//! - Hides: the marker format (`resync.json` v1), the backup directory layout
//!   (`backups/<unix_ms>/{herdr-threads.db,orig/}`), and that a rebuild is a
//!   restart against a missing store.
//! - Must not: delete the old database or the backups, swap a store under a
//!   running daemon, or admit salvaged rows except through
//!   `JournalPort::ingest` (which verifies every entry).
//! - Relationships: [`request`] is called by the operator-gated CLI path and
//!   uses the daemon's existing graceful stop (the shutdown token).
//!   [`resume_at_start`] is step 0 of `run_elected_impl`, run only by the
//!   elected daemon; `app.rs` turns the [`ResyncPlan`] into dial targets, the
//!   rx pin and the salvage job, which the incarnations worker runs through
//!   [`run_salvage`].
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
use crate::store::journal;
use crate::store::sync_dial_targets::DialTarget;
use crate::sync::coordinator::SyncHandle;
use crate::sync::envelope::{Deps, Envelope, StoredEntry};
use crate::sync::hlc::HlcStamp;
use crate::sync::journal_port::IngestOutcome;
use crate::sync::{BatchLimits, EntityKey, LogKey};

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

/// Most entries of one log handed to ingest at once.
const SALVAGE_BATCH_ENTRIES: usize = 256;

/// Most canonical bytes of one salvage batch (a single larger entry still
/// goes alone).
const SALVAGE_BATCH_BYTES: usize = 4 * 1024 * 1024;

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
        Self::finish_in(&paths.node_dir)
    }

    /// [`ResyncMarker::finish`] for a node directory.
    pub fn finish_in(node_dir: &Path) -> Result<(), StartError> {
        match fs::remove_file(node_dir.join(MARKER_FILE)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StartError::io("remove the resync marker", error)),
        }
        sync_dir(node_dir).map_err(|error| StartError::io("sync the node directory", error))
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

/// Records `salvaging` in the marker and returns it (its targets and
/// operator are what composition needs). The marker must exist: `app.rs`
/// calls this only with the plan [`resume_at_start`] just returned.
pub fn mark_salvaging(paths: &NodePaths) -> Result<ResyncMarker, StartError> {
    let mut marker = ResyncMarker::read(paths)?
        .ok_or_else(|| StartError::Marker(format!("{MARKER_FILE} vanished before salvage")))?;
    if marker.phase != ResyncPhase::Salvaging {
        marker.phase = ResyncPhase::Salvaging;
        marker.write(paths)?;
    }
    Ok(marker)
}

/// A log the salvage stopped before its end, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SalvageStop {
    pub log: LogKey,
    pub why: String,
}

/// What one salvage pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SalvageReport {
    /// Logs the backup holds.
    pub logs: usize,
    /// Entries ingest reported as stored or already held, over all logs.
    pub entries: u64,
    /// Logs stopped before their end.
    pub stopped: Vec<SalvageStop>,
    /// The backup could not be opened or listed: nothing was salvaged.
    pub unreadable: Option<String>,
    /// Cancelled before the end; the marker is kept for the next start.
    pub interrupted: bool,
    /// At least one log stopped for lack of disk headroom (it is also in
    /// `stopped`): its tail is not yet in the store, so the marker stays and
    /// a later pass continues.
    pub headroom: bool,
}

/// The ingest callback of a salvage: one batch of one log. The worker's
/// implementation holds a `FairWriter` background turn around
/// `JournalPort::ingest`.
pub type SalvageIngest<'a> =
    dyn FnMut(&LogKey, Vec<StoredEntry>) -> Result<IngestOutcome, ApiError> + 'a;

/// Reads the backup at `backup_db` read-only and hands every log's entries,
/// ascending in batches of [`SALVAGE_BATCH_ENTRIES`], to `ingest`, logs in
/// [`LogKey`] order.
///
/// The backup has one of two shapes, detected once by its `journal.raw`
/// column. With it (schema 5, this release), entries are served byte-exact by
/// [`journal::serve_entries`], unknown layouts as `Opaque`. Without it
/// (schema 4, 2.2.x and older: parsed columns only) every row is rebuilt from
/// its columns by [`legacy_entries`] as a known entry; ingest verifies it and
/// stores its recomputed canonical bytes, as delta 5 does for a live store.
///
/// A log stops at the first outcome other than `Stored` or at a read error
/// (a SQLite error, or a schema-4 row that does not decode), and the next log
/// goes on. `ingest` verifies every entry, so
/// corrupt rows are never admitted. A store error from `ingest` itself ends
/// the pass with that error: ingest is idempotent, so the caller runs the
/// whole pass again. A `Headroom` stop sets [`SalvageReport::headroom`].
pub fn salvage(
    backup_db: &Path,
    ingest: &mut SalvageIngest<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<SalvageReport, ApiError> {
    let mut report = SalvageReport::default();
    let logs = Connection::open_with_flags(
        backup_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| error.to_string())
    .and_then(|conn| {
        let vector = journal::vector(&conn).map_err(|error| error.detail)?;
        Ok((conn, vector))
    });
    let (conn, vector) = match logs {
        Ok(found) => found,
        Err(why) => {
            report.unreadable = Some(why);
            return Ok(report);
        }
    };
    report.logs = vector.0.len();
    let has_raw = match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('journal') WHERE name='raw')",
        [],
        |r| r.get::<_, bool>(0),
    ) {
        Ok(found) => found,
        Err(error) => {
            report.unreadable = Some(error.to_string());
            return Ok(report);
        }
    };
    let limits = BatchLimits {
        entries: SALVAGE_BATCH_ENTRIES,
        bytes: SALVAGE_BATCH_BYTES,
    };
    for log in vector.0.keys() {
        let mut from = 1u64;
        loop {
            if cancelled() {
                report.interrupted = true;
                return Ok(report);
            }
            let read = if has_raw {
                journal::serve_entries(&conn, log, from, limits)
            } else {
                legacy_entries(&conn, log, from, limits)
            };
            let batch = match read {
                Ok(batch) => batch,
                Err(error) => {
                    report.stopped.push(SalvageStop {
                        log: *log,
                        why: format!("read: {}", error.detail),
                    });
                    break;
                }
            };
            let Some(last) = batch.last().map(StoredEntry::counter) else {
                break;
            };
            let count = batch.len() as u64;
            let opaque: Vec<u64> = batch
                .iter()
                .filter(|entry| entry.is_opaque())
                .map(StoredEntry::counter)
                .collect();
            match ingest(log, batch)? {
                IngestOutcome::Stored { .. } => {
                    report.entries += count;
                    from = last.saturating_add(1);
                }
                other => {
                    report.headroom |= matches!(other, IngestOutcome::Headroom);
                    // A store that cannot take an opaque entry is named, not
                    // just "corrupt": the entry is in the backup and stays
                    // there.
                    let named = match &other {
                        IngestOutcome::Corrupt { counter, .. } if opaque.contains(counter) => {
                            format!(" (opaque entry at counter {counter})")
                        }
                        _ => String::new(),
                    };
                    report.stopped.push(SalvageStop {
                        log: *log,
                        why: format!("ingest at counter {from}: {}{named}", outcome_name(&other)),
                    });
                    break;
                }
            }
        }
    }
    Ok(report)
}

/// The schema-4 reader of [`salvage`]: the entries of `log` from counter
/// `from`, built from the projection columns (the backup has no `raw`), in
/// batches by the rules of [`journal::serve_entries`]: at most `limits.entries`,
/// stopping before the entry whose canonical size would pass `limits.bytes`,
/// always at least one. A row that does not decode is an error, which stops
/// the log.
fn legacy_entries(
    conn: &Connection,
    log: &LogKey,
    from: u64,
    limits: BatchLimits,
) -> Result<Vec<StoredEntry>, ApiError> {
    let bad = |what: &str| {
        ApiError::new(
            ErrorCode::StoreCorrupt,
            format!("schema-4 backup row: {what}"),
        )
    };
    let sql_err =
        |error: rusqlite::Error| ApiError::new(ErrorCode::StoreCorrupt, error.to_string());
    let mut statement = conn
        .prepare(
            "SELECT j.counter,j.prev_hash,j.hlc,j.kind,j.kind_version,j.deps,j.creates,j.needs, \
             j.payload,j.hash,j.signature FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter>=?3 ORDER BY j.counter",
        )
        .map_err(sql_err)?;
    let mut rows = statement
        .query(rusqlite::params![
            log.node.to_string(),
            log.incarnation,
            i64::try_from(from.max(1)).unwrap_or(i64::MAX)
        ])
        .map_err(sql_err)?;
    let mut out: Vec<StoredEntry> = Vec::new();
    let mut bytes = 0usize;
    while let Some(r) = rows.next().map_err(sql_err)? {
        let envelope = Envelope {
            log: *log,
            counter: u64::try_from(r.get::<_, i64>(0).map_err(sql_err)?)
                .map_err(|_| bad("counter"))?,
            prev_hash: r
                .get::<_, Vec<u8>>(1)
                .map_err(sql_err)?
                .try_into()
                .map_err(|_| bad("prev_hash"))?,
            hlc: HlcStamp::from_u64(
                u64::try_from(r.get::<_, i64>(2).map_err(sql_err)?).map_err(|_| bad("hlc"))?,
            ),
            kind: r.get(3).map_err(sql_err)?,
            kind_version: r.get(4).map_err(sql_err)?,
            deps: Deps::decode(&r.get::<_, Vec<u8>>(5).map_err(sql_err)?, log)
                .map_err(|_| bad("deps"))?,
            creates: r
                .get::<_, Option<String>>(6)
                .map_err(sql_err)?
                .map(EntityKey::new)
                .transpose()
                .map_err(|_| bad("creates"))?,
            needs: r
                .get::<_, String>(7)
                .map_err(sql_err)?
                .split('\n')
                .filter(|key| !key.is_empty())
                .map(EntityKey::new)
                .collect::<Result<_, _>>()
                .map_err(|_| bad("needs"))?,
            payload: r.get(8).map_err(sql_err)?,
            hash: r
                .get::<_, Vec<u8>>(9)
                .map_err(sql_err)?
                .try_into()
                .map_err(|_| bad("hash"))?,
            signature: r
                .get::<_, Vec<u8>>(10)
                .map_err(sql_err)?
                .try_into()
                .map_err(|_| bad("signature"))?,
        };
        let size = envelope.canonical_bytes().len();
        if out.len() >= limits.entries.max(1)
            || (!out.is_empty() && bytes.saturating_add(size) > limits.bytes)
        {
            break;
        }
        bytes = bytes.saturating_add(size);
        out.push(StoredEntry::from(envelope));
    }
    Ok(out)
}

fn outcome_name(outcome: &IngestOutcome) -> &'static str {
    match outcome {
        IngestOutcome::Stored { .. } => "stored",
        IngestOutcome::Gap { .. } => "gap",
        IngestOutcome::Fork { .. } => "fork",
        IngestOutcome::Frozen { .. } => "frozen",
        IngestOutcome::Corrupt { .. } => "corrupt",
        IngestOutcome::Oversize { .. } => "oversize opaque entry",
        IngestOutcome::Headroom => "no disk headroom",
    }
}

/// [`salvage`], then the end of the rebuild: the marker is deleted unless the
/// pass was interrupted or a log stopped for lack of disk headroom (that tail
/// would survive only in the backup). The backup directory is never touched.
pub fn run_salvage(
    node_dir: &Path,
    backup_db: &Path,
    ingest: &mut SalvageIngest<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<SalvageReport, ApiError> {
    let report = salvage(backup_db, ingest, cancelled)?;
    if !report.interrupted && !report.headroom {
        ResyncMarker::finish_in(node_dir)
            .map_err(|error| ApiError::new(ErrorCode::StoreBusy, error.to_string()))?;
    }
    Ok(report)
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
pub(crate) fn database_bytes(paths: &NodePaths) -> u64 {
    let size =
        |name: String| fs::metadata(paths.node_dir.join(name)).map_or(0, |metadata| metadata.len());
    size(DB_FILE.to_owned()).saturating_add(size(format!("{DB_FILE}-wal")))
}

/// Bytes available to an unprivileged writer on the filesystem of `dir`.
pub(crate) fn free_bytes(dir: &Path) -> io::Result<u64> {
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

    // -- Salvage -----------------------------------------------------------

    use crate::node::NodeIdentity;
    use crate::store::connection::StoreContext;
    use crate::store::{SqliteStore, StoreSettings};
    use crate::sync::journal_port::JournalPort;
    use crate::sync::memory_journal::MemoryJournal;
    use crate::sync::{LogHead, LogState};

    /// A foreign log of node `n`: its genesis and `extra` more entries.
    fn foreign(n: u8, extra: usize) -> (LogKey, Vec<StoredEntry>) {
        let journal = MemoryJournal::new(Arc::new(NodeIdentity::from_seed([n; 32])));
        journal.bootstrap();
        for i in 0..extra {
            journal
                .author_raw("test.kind", format!("{{\"i\":{i}}}").into_bytes())
                .unwrap();
        }
        let log = journal.self_log();
        let entries = journal
            .serve(
                &log,
                1,
                BatchLimits {
                    entries: usize::MAX,
                    bytes: usize::MAX,
                },
                &budget(),
            )
            .unwrap();
        (log, entries)
    }

    /// A real store at `db` holding the given foreign logs, closed again.
    fn build_store(db: &Path, logs: &[(LogKey, Vec<StoredEntry>)]) {
        let context = StoreContext::new(db.to_path_buf(), Arc::new(FixedClock(NOW)));
        let store = SqliteStore::new(
            context,
            crate::test_support::test_node(),
            StoreSettings::default(),
        )
        .unwrap();
        for (log, entries) in logs {
            for chunk in entries.chunks(SALVAGE_BATCH_ENTRIES) {
                let outcome = store.ingest(log, chunk.to_vec(), &budget()).unwrap();
                assert!(
                    matches!(outcome, IngestOutcome::Stored { .. }),
                    "{outcome:?}"
                );
            }
        }
    }

    fn stored(count: usize) -> IngestOutcome {
        IngestOutcome::Stored {
            stored: count,
            duplicates: 0,
            head: LogHead {
                counter: count as u64,
                head_hash: [0; 32],
                state: LogState::Active,
                frozen_at: None,
            },
        }
    }

    /// Three foreign logs; the first has more than one batch. A store also
    /// holds its own genesis log, so a backup lists four logs.
    fn three_logs() -> Vec<(LogKey, Vec<StoredEntry>)> {
        vec![
            foreign(41, SALVAGE_BATCH_ENTRIES + 43),
            foreign(42, 2),
            foreign(43, 1),
        ]
    }

    /// `(log, head counter)` of a backup, in `LogKey` order.
    fn backup_logs(db: &Path) -> Vec<(LogKey, u64)> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        journal::vector(&conn)
            .unwrap()
            .0
            .into_iter()
            .map(|(log, head)| (log, head.counter))
            .collect()
    }

    #[test]
    fn salvage_stops_log_at_non_stored_and_continues() {
        let (iso, _paths) = node("resync-salvage-stop");
        let db = iso.path("backup.db");
        let logs = three_logs();
        build_store(&db, &logs);
        let big = logs[0].0;
        let held = backup_logs(&db);
        assert_eq!(held.len(), 4);

        // (log, first counter, batch size) in call order.
        let mut calls: Vec<(LogKey, u64, usize)> = Vec::new();
        let mut ingest = |log: &LogKey, entries: Vec<StoredEntry>| {
            calls.push((*log, entries[0].counter(), entries.len()));
            // The big log's second batch is a gap: that log stops there.
            if *log == big && entries[0].counter() > 1 {
                return Ok(IngestOutcome::Gap {
                    stored: 0,
                    expected: 1,
                });
            }
            Ok(stored(entries.len()))
        };
        let report = salvage(&db, &mut ingest, &|| false).unwrap();

        let batch = SALVAGE_BATCH_ENTRIES;
        let mut expected: Vec<(LogKey, u64, usize)> = Vec::new();
        for (log, counter) in &held {
            if *log == big {
                expected.push((*log, 1, batch));
                expected.push((*log, batch as u64 + 1, 44));
            } else {
                expected.push((*log, 1, *counter as usize));
            }
        }
        assert_eq!(
            calls, expected,
            "logs in key order; a stopped log is not retried, the next goes on"
        );
        assert_eq!(report.logs, 4);
        assert_eq!(
            report.entries,
            held.iter().map(|(_, c)| c).sum::<u64>() - 44
        );
        assert!(!report.headroom, "a gap is not a headroom stop");
        assert_eq!(report.stopped.len(), 1);
        assert_eq!(report.stopped[0].log, big);
        assert!(
            report.stopped[0].why.contains("gap"),
            "{:?}",
            report.stopped
        );
        assert!(!report.interrupted && report.unreadable.is_none());
    }

    /// A store that cannot take an opaque entry stops the log there, and the
    /// stop names that entry; a corrupt known entry is only "corrupt".
    #[test]
    fn salvage_reports_opaque_stop_by_name() {
        let _registry = crate::sync::envelope::hold_layout_registry();
        use crate::sync::EnvelopeError;
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};

        let (iso, _paths) = node("resync-salvage-opaque-stop");
        let id = NodeIdentity::from_seed([45; 32]);
        let (log, mut entries) = foreign(45, 1);
        let two = entries[1].hash();
        let opaque = synthetic_entry(&id, &synthetic_unsigned(&id, log.incarnation, 3, two));
        entries.push(StoredEntry::from(opaque));
        let backup = iso.path("backup.db");
        build_store(&backup, &[(log, entries)]);

        for (counter, named) in [(3, true), (2, false)] {
            let mut ingest = |log: &LogKey, _: Vec<StoredEntry>| {
                Ok(IngestOutcome::Corrupt {
                    log: *log,
                    counter,
                    reason: EnvelopeError::Malformed { field: "raw" },
                })
            };
            let report = salvage(&backup, &mut ingest, &|| false).unwrap();
            let stops: Vec<_> = report.stopped.iter().filter(|s| s.log == log).collect();
            assert_eq!(stops.len(), 1, "{:?}", report.stopped);
            let why = &stops[0].why;
            assert!(why.contains("corrupt"), "{why}");
            assert_eq!(
                why.contains(&format!("opaque entry at counter {counter}")),
                named,
                "{why}"
            );
        }
    }

    /// A schema-4 backup (2.2.x shape: no `raw` column, `user_version` 4) of
    /// the known `logs`, copied out of a store this release built.
    fn schema4_backup(dst: &Path, logs: &[(LogKey, Vec<StoredEntry>)]) {
        use crate::store::schema;
        let built = dst.with_extension("built");
        build_store(&built, logs);
        let conn = Connection::open(dst).unwrap();
        schema::initialize_with_deltas(
            &conn,
            || crate::protocol::time::UtcMillis(0),
            Some(&crate::test_support::test_node_seed()),
            &schema::NODE_DELTAS[..3],
            false,
        )
        .unwrap();
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
        conn.execute("ATTACH DATABASE ?1 AS built", [built.to_str().unwrap()])
            .unwrap();
        conn.execute(
            "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,\
             contiguous_counter,head_hash,applied_through,blocked_on,state,frozen_at_counter,\
             skew_ahead_ms,first_seen_at) \
             SELECT log_ix,node_uuid,incarnation,node_uuid=(SELECT id FROM local_node),\
             pubkey,genesis_hash,contiguous_counter,head_hash,applied_through,blocked_on,state,\
             frozen_at_counter,skew_ahead_ms,first_seen_at FROM built.sync_logs \
             WHERE log_ix NOT IN (SELECT log_ix FROM main.sync_logs)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO journal(ingest_seq,log_ix,counter,prev_hash,hlc,kind,kind_version,deps,\
             creates,needs,payload,hash,signature,ingested_at,apply_state,applied_seq,parked_on,\
             apply_error) \
             SELECT ingest_seq,log_ix,counter,prev_hash,hlc,kind,kind_version,deps,creates,needs,\
             payload,hash,signature,ingested_at,apply_state,applied_seq,parked_on,apply_error \
             FROM built.journal WHERE kind IS NOT NULL \
             AND log_ix NOT IN (SELECT log_ix FROM main.sync_logs WHERE is_self=1)",
            [],
        )
        .unwrap();
        conn.execute("DETACH DATABASE built", []).unwrap();
    }

    fn has_raw_column(db: &Path) -> bool {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('journal') WHERE name='raw')",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn salvage_reads_a_schema_4_backup() {
        let (iso, _paths) = node("resync-salvage-schema4");
        let backup = iso.path("backup4.db");
        let logs = three_logs();
        let held: u64 = logs.iter().map(|(_, e)| e.len() as u64).sum();
        schema4_backup(&backup, &logs);
        assert!(!has_raw_column(&backup));
        let before = std::fs::read(&backup).unwrap();

        let mut got: std::collections::BTreeMap<LogKey, Vec<StoredEntry>> = Default::default();
        let mut ingest = |log: &LogKey, batch: Vec<StoredEntry>| {
            let count = batch.len();
            got.entry(*log).or_default().extend(batch);
            Ok(stored(count))
        };
        let report = salvage(&backup, &mut ingest, &|| false).unwrap();

        assert!(report.stopped.is_empty(), "{:?}", report.stopped);
        assert_eq!(report.entries, held, "the own genesis log is empty here");
        for (log, entries) in &logs {
            assert_eq!(&got[log], entries, "every entry, equal to the original");
        }
        assert_eq!(std::fs::read(&backup).unwrap(), before, "backup untouched");
        assert!(!has_raw_column(&backup));
    }

    #[test]
    fn salvage_batches_a_schema_4_backup_by_limits() {
        let (iso, _paths) = node("resync-salvage-schema4-limits");
        let backup = iso.path("backup4.db");
        let (log, entries) = foreign(46, 4);
        schema4_backup(&backup, &[(log, entries.clone())]);
        let conn = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let one = BatchLimits {
            entries: 2,
            bytes: usize::MAX,
        };
        assert_eq!(legacy_entries(&conn, &log, 2, one).unwrap(), entries[1..3]);
        let tiny = BatchLimits {
            entries: usize::MAX,
            bytes: 1,
        };
        assert_eq!(
            legacy_entries(&conn, &log, 1, tiny).unwrap(),
            entries[..1],
            "at least one entry, even over the byte limit"
        );
    }

    #[test]
    fn salvage_stops_a_schema_4_log_at_a_row_that_does_not_decode() {
        let (iso, _paths) = node("resync-salvage-schema4-bad");
        let backup = iso.path("backup4.db");
        let (log, entries) = foreign(47, 2);
        schema4_backup(&backup, &[(log, entries)]);
        {
            let conn = Connection::open(&backup).unwrap();
            conn.execute("DROP TRIGGER journal_envelope_immutable", [])
                .unwrap();
            conn.execute(
                "UPDATE journal SET deps=x'ff' WHERE counter=2 AND log_ix=\
                 (SELECT log_ix FROM sync_logs WHERE node_uuid=?1)",
                [log.node.to_string()],
            )
            .unwrap();
        }
        let mut seen = 0usize;
        let mut ingest = |_: &LogKey, batch: Vec<StoredEntry>| {
            seen += batch.len();
            Ok(stored(batch.len()))
        };
        // The batch read fails as a whole, so nothing of the log is handed on.
        let report = salvage(&backup, &mut ingest, &|| false).unwrap();
        let stops: Vec<_> = report.stopped.iter().filter(|s| s.log == log).collect();
        assert_eq!(stops.len(), 1, "{:?}", report.stopped);
        assert!(stops[0].why.starts_with("read: "), "{}", stops[0].why);
        assert_eq!(seen, 0);
    }

    #[test]
    fn salvage_reads_a_schema_5_backup_with_an_opaque_entry() {
        let _registry = crate::sync::envelope::hold_layout_registry();
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};

        let (iso, _paths) = node("resync-salvage-schema5-opaque");
        let id = NodeIdentity::from_seed([48; 32]);
        let (log, mut entries) = foreign(48, 0);
        let one = entries[0].hash();
        let opaque = synthetic_entry(&id, &synthetic_unsigned(&id, log.incarnation, 2, one));
        entries.push(StoredEntry::from(opaque.clone()));
        let three = synthetic_unsigned(&id, log.incarnation, 3, opaque.hash()).seal(&id);
        entries.push(StoredEntry::from(three));
        let backup = iso.path("backup5.db");
        build_store(&backup, &[(log, entries.clone())]);
        assert!(has_raw_column(&backup));

        let mut got: Vec<StoredEntry> = Vec::new();
        let mut ingest = |l: &LogKey, batch: Vec<StoredEntry>| {
            if *l == log {
                got.extend(batch.clone());
            }
            Ok(stored(batch.len()))
        };
        let report = salvage(&backup, &mut ingest, &|| false).unwrap();
        assert!(report.stopped.is_empty(), "{:?}", report.stopped);
        assert_eq!(got.len(), 3);
        assert!(!got[0].is_opaque() && !got[2].is_opaque());
        assert_eq!(
            got[1].as_opaque().map(|o| o.raw().to_vec()),
            Some(opaque.raw().to_vec()),
            "entry 2 arrives as Opaque, byte for byte"
        );
        assert_eq!(got, entries);
    }

    #[test]
    fn salvage_ingest_store_error_ends_the_pass_for_a_retry() {
        let (iso, _paths) = node("resync-salvage-error");
        let db = iso.path("backup.db");
        build_store(&db, &three_logs());
        let mut calls = 0;
        let mut ingest = |_: &LogKey, entries: Vec<StoredEntry>| {
            calls += 1;
            if calls == 2 {
                return Err(ApiError::new(ErrorCode::StoreBusy, "writer busy"));
            }
            Ok(stored(entries.len()))
        };
        let error = salvage(&db, &mut ingest, &|| false).unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreBusy);
        assert_eq!(calls, 2, "the pass ends at the error");
    }

    #[test]
    fn marker_removed_backup_kept() {
        let (_iso, paths) = node("resync-salvage-finish");
        build_store(&paths.database_path, &three_logs());
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        let marker = mark_salvaging(&paths).unwrap();
        assert_eq!(marker.phase, ResyncPhase::Salvaging);
        assert_eq!(
            ResyncMarker::read(&paths).unwrap().unwrap().phase,
            ResyncPhase::Salvaging,
            "the phase is persisted"
        );
        let mut ingest = |_: &LogKey, entries: Vec<StoredEntry>| Ok(stored(entries.len()));
        let report = run_salvage(&paths.node_dir, &plan.backup_db, &mut ingest, &|| false).unwrap();
        assert_eq!(report.entries, 300 + 3 + 2 + 1, "every entry of every log");
        assert!(
            ResyncMarker::read(&paths).unwrap().is_none(),
            "salvage ended: the marker is gone"
        );
        assert!(plan.backup_db.is_file(), "the backup stays");
        let dir = plan.backup_db.parent().unwrap();
        assert!(dir.join(ORIG_DIR).join(DB_FILE).is_file(), "orig/ stays");
    }

    #[test]
    fn headroom_stop_keeps_the_marker_then_a_salvage_with_headroom_removes_it() {
        let (_iso, paths) = node("resync-salvage-headroom");
        build_store(&paths.database_path, &three_logs());
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        mark_salvaging(&paths).unwrap();

        let mut full = |_: &LogKey, _: Vec<StoredEntry>| Ok(IngestOutcome::Headroom);
        let report = run_salvage(&paths.node_dir, &plan.backup_db, &mut full, &|| false).unwrap();
        assert!(report.headroom, "{report:?}");
        assert_eq!(report.entries, 0);
        assert_eq!(report.stopped.len(), 4, "every log stopped");
        assert!(
            report
                .stopped
                .iter()
                .all(|stop| stop.why.contains("no disk headroom")),
            "{:?}",
            report.stopped
        );
        let marker = ResyncMarker::read(&paths).unwrap().expect("marker kept");
        assert_eq!(marker.phase, ResyncPhase::Salvaging);

        let mut room = |_: &LogKey, entries: Vec<StoredEntry>| Ok(stored(entries.len()));
        let report = run_salvage(&paths.node_dir, &plan.backup_db, &mut room, &|| false).unwrap();
        assert!(!report.headroom);
        assert_eq!(report.entries, 300 + 3 + 2 + 1);
        assert!(ResyncMarker::read(&paths).unwrap().is_none());
        assert!(plan.backup_db.is_file(), "the backup stays");
    }

    #[test]
    fn headroom_on_one_log_still_reads_the_others() {
        let (_iso, paths) = node("resync-salvage-headroom-one");
        build_store(&paths.database_path, &three_logs());
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        let first = *backup_logs(&plan.backup_db)
            .first()
            .map(|(l, _)| l)
            .unwrap();
        let mut ingest = |log: &LogKey, entries: Vec<StoredEntry>| {
            if *log == first {
                Ok(IngestOutcome::Headroom)
            } else {
                Ok(stored(entries.len()))
            }
        };
        let report = run_salvage(&paths.node_dir, &plan.backup_db, &mut ingest, &|| false).unwrap();
        let first_count = backup_logs(&plan.backup_db)[0].1;
        let total: u64 = backup_logs(&plan.backup_db).iter().map(|(_, c)| c).sum();
        assert!(report.headroom);
        assert_eq!(report.entries, total - first_count, "the others were read");
        assert_eq!(report.stopped.len(), 1);
        assert!(ResyncMarker::read(&paths).unwrap().is_some(), "marker kept");
    }

    /// A backup whose log holds an opaque entry in the middle is carried
    /// across whole: the new store holds the same bytes at every position, the
    /// opaque one included, and the log is not cut at that position.
    #[test]
    fn salvage_carries_opaque_entries() {
        let _registry = crate::sync::envelope::hold_layout_registry();
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};

        let (iso, _paths) = node("resync-salvage-opaque");
        let id = NodeIdentity::from_seed([44; 32]);
        let (log, mut entries) = foreign(44, 1);
        let two = entries[1].hash();
        let opaque = synthetic_entry(&id, &synthetic_unsigned(&id, log.incarnation, 3, two));
        let opaque_hash = opaque.hash();
        entries.push(StoredEntry::from(opaque.clone()));
        let four = synthetic_unsigned(&id, log.incarnation, 4, opaque_hash).seal(&id);
        entries.push(StoredEntry::from(four));
        let backup = iso.path("backup.db");
        build_store(&backup, &[(log, entries.clone())]);

        let target = StoreContext::new(iso.path("new.db"), Arc::new(FixedClock(NOW)));
        let fresh = SqliteStore::new(
            target,
            crate::test_support::test_node(),
            StoreSettings::default(),
        )
        .unwrap();
        let mut ingest =
            |log: &LogKey, batch: Vec<StoredEntry>| fresh.ingest(log, batch, &budget());
        let report = salvage(&backup, &mut ingest, &|| false).unwrap();

        assert!(report.stopped.is_empty(), "{:?}", report.stopped);
        let carried = fresh
            .serve(&log, 1, BatchLimits::default(), &budget())
            .unwrap();
        assert_eq!(carried, entries, "every position, byte for byte");
        assert_eq!(
            carried[2].as_opaque().map(|o| o.raw().to_vec()),
            Some(opaque.raw().to_vec())
        );
    }

    #[test]
    fn interrupted_salvage_keeps_the_marker() {
        let (_iso, paths) = node("resync-salvage-interrupted");
        build_store(&paths.database_path, &three_logs());
        requested_marker(&paths);
        let plan = resume_at_start(&paths, &FixedClock(NOW)).unwrap().unwrap();
        let mut ingest = |_: &LogKey, entries: Vec<StoredEntry>| Ok(stored(entries.len()));
        let report = run_salvage(&paths.node_dir, &plan.backup_db, &mut ingest, &|| true).unwrap();
        assert!(report.interrupted);
        assert_eq!(report.entries, 0);
        assert!(
            ResyncMarker::read(&paths).unwrap().is_some(),
            "the next start salvages again"
        );
    }

    #[test]
    fn unreadable_backup_is_reported_and_the_marker_removed() {
        let (iso, paths) = node("resync-salvage-unreadable");
        requested_marker(&paths);
        let db = iso.path("not-a-database");
        fs::write(&db, b"this is not a database at all").unwrap();
        let mut ingest = |_: &LogKey, _: Vec<StoredEntry>| panic!("nothing to ingest");
        let report = run_salvage(&paths.node_dir, &db, &mut ingest, &|| false).unwrap();
        assert!(report.unreadable.is_some(), "{report:?}");
        assert!(ResyncMarker::read(&paths).unwrap().is_none());
    }

    #[test]
    fn free_bytes_reports_the_filesystem() {
        let (_iso, paths) = node("resync-free");
        assert!(free_bytes(&paths.node_dir).unwrap() > 0);
        assert!(free_bytes(&paths.node_dir.join("missing")).is_err());
    }
}
