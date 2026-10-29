//! The dial targets: which remote machines this node dials to link with.
//!
//! Responsibility (DialTargets)
//! - Owns: the `sync_dial_targets` rows and their rules: control-character and
//!   leading-`-` rejection, explicit-beats-discovered precedence,
//!   forget-disables-only, the [`MAX_DIAL_TARGETS`] cap, the `stopped` <->
//!   reason invariant, [`clear_stale_stops`] by local software version, and
//!   applying a [`DialTransition`] to the state, attempts, `next_attempt_at`
//!   and error columns.
//! - Hides: the table shape and the precedence `ON CONFLICT` forms.
//! - Must not: compute backoff delays or classify link errors (the Dialer
//!   decides the transition; this module only records it), and must never
//!   delete a row. Callers never write the table directly.
//! - Errors: `InvalidRequest` (control characters, a leading `-`, the cap
//!   reached) is kept apart from [`Discovered::Skipped`] (discovery over the
//!   cap, not an error) and from store errors.
//! - Role: storage seam for the Dialer. Used by `SyncStorePort`'s SQLite
//!   implementation (`enabled`, `record_transition`), by the operator
//!   `peer add/forget/list` commands and by discovery. New target columns or
//!   sources go here.
//!
//! Every write takes the caller's transaction: the cap check and the insert
//! are one decision, so the caller holds a write transaction.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::connection::{api_error, store_error};
use super::sync_link_events::{self, LinkEvent, LinkEventKind, LinkOrigin};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::sync::frame::{CloseClass, CloseReason};

/// Most enabled dial targets a node holds.
pub const MAX_DIAL_TARGETS: usize = 64;
/// Longest `ssh_dest` and `remote_command` the table accepts, in characters.
pub const MAX_TARGET_TEXT: usize = 1024;
/// Longest stored `last_error`, in characters.
pub const MAX_LAST_ERROR_CHARS: usize = 512;
/// How long a target whose remote has no herdr-threads installed waits before
/// the next dial (30 minutes). A fixed rule, not a computed backoff.
pub const NOT_INSTALLED_RETRY_MS: i64 = 30 * 60 * 1000;
/// The local software version, for [`clear_stale_stops`].
pub const LOCAL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The row id of one dial target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetId(pub i64);

/// How a target came to exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialSource {
    Explicit,
    HerdrMachine,
    HerdrRemoteProcess,
}

impl DialSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::HerdrMachine => "herdr_machine",
            Self::HerdrRemoteProcess => "herdr_remote_process",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "explicit" => Self::Explicit,
            "herdr_machine" => Self::HerdrMachine,
            "herdr_remote_process" => Self::HerdrRemoteProcess,
            _ => return None,
        })
    }
}

/// The sources discovery may create (an explicit target is never discovered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoverySource {
    HerdrMachine,
    HerdrRemoteProcess,
}

impl From<DiscoverySource> for DialSource {
    fn from(source: DiscoverySource) -> Self {
        match source {
            DiscoverySource::HerdrMachine => Self::HerdrMachine,
            DiscoverySource::HerdrRemoteProcess => Self::HerdrRemoteProcess,
        }
    }
}

/// Where a target is in its dial cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialState {
    Idle,
    Dialing,
    Linked,
    Backoff,
    NotInstalled,
    Stopped,
}

impl DialState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Dialing => "dialing",
            Self::Linked => "linked",
            Self::Backoff => "backoff",
            Self::NotInstalled => "not_installed",
            Self::Stopped => "stopped",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "idle" => Self::Idle,
            "dialing" => Self::Dialing,
            "linked" => Self::Linked,
            "backoff" => Self::Backoff,
            "not_installed" => Self::NotInstalled,
            "stopped" => Self::Stopped,
            _ => return None,
        })
    }
}

/// The [`CloseReason`]s whose class is [`CloseClass::StopDialing`]: the only
/// reasons a target stops on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EnvelopeUnsupported,
    SelfLink,
    AuthFailed,
    NodeCollision,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EnvelopeUnsupported => "envelope_unsupported",
            Self::SelfLink => "self_link",
            Self::AuthFailed => "auth_failed",
            Self::NodeCollision => "node_collision",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "envelope_unsupported" => Self::EnvelopeUnsupported,
            "self_link" => Self::SelfLink,
            "auth_failed" => Self::AuthFailed,
            "node_collision" => Self::NodeCollision,
            _ => return None,
        })
    }
}

/// A [`CloseReason`] of class `Retry` is not a stop reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotStopping(pub CloseReason);

impl TryFrom<CloseReason> for StopReason {
    type Error = NotStopping;

    fn try_from(reason: CloseReason) -> Result<Self, NotStopping> {
        if reason.class() != CloseClass::StopDialing {
            return Err(NotStopping(reason));
        }
        match reason {
            CloseReason::EnvelopeUnsupported => Ok(Self::EnvelopeUnsupported),
            CloseReason::SelfLink => Ok(Self::SelfLink),
            CloseReason::AuthFailed => Ok(Self::AuthFailed),
            CloseReason::NodeCollision => Ok(Self::NodeCollision),
            other => Err(NotStopping(other)),
        }
    }
}

/// What the Dialer observed, for [`record_transition`] to store. Closed: the
/// Dialer chooses the variant and its numbers, this module applies them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialTransition {
    /// A dial attempt started.
    Dialing,
    /// The handshake completed with `node`.
    Linked { node: NodeUuid },
    /// The attempt failed; dial again at `next_at` (unix ms).
    Backoff {
        attempts: u32,
        next_at: i64,
        error: String,
    },
    /// The remote has no herdr-threads; retry in [`NOT_INSTALLED_RETRY_MS`].
    NotInstalled,
    /// The remote closed with a stop-class reason; dial again only after the
    /// operator re-adds the target or the local `version` changes.
    Stopped { reason: StopReason, version: String },
}

/// One `sync_dial_targets` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialTarget {
    pub target_id: TargetId,
    pub ssh_dest: String,
    pub source: DialSource,
    pub enabled: bool,
    pub remote_command: Option<String>,
    pub learned_node: Option<NodeUuid>,
    pub state: DialState,
    pub stopped: Option<(StopReason, Option<String>)>,
    pub attempts: u32,
    pub next_attempt_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_error_at: Option<i64>,
    pub last_linked_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl DialTarget {
    /// The last error while it is current: the target is not linked and the
    /// error is not older than its last link. A healthy link shows none; the
    /// history is `peer events`.
    pub fn current_error(&self) -> Option<&str> {
        let newer = match (self.last_error_at, self.last_linked_at) {
            (Some(error), Some(linked)) => error >= linked,
            (_, None) => true,
            (None, Some(_)) => false,
        };
        (self.state != DialState::Linked && newer)
            .then_some(self.last_error.as_deref())
            .flatten()
    }

    /// Applies `transition` to the row's state columns. The one definition of
    /// what each transition means; [`record_transition`] stores its result and
    /// the in-memory test store runs it too.
    pub fn apply(&mut self, transition: &DialTransition, now: i64) {
        self.updated_at = now;
        if !matches!(transition, DialTransition::Stopped { .. }) {
            self.stopped = None;
        }
        match transition {
            DialTransition::Dialing => {
                self.state = DialState::Dialing;
                self.next_attempt_at = None;
            }
            DialTransition::Linked { node } => {
                self.state = DialState::Linked;
                self.learned_node = Some(*node);
                self.attempts = 0;
                self.next_attempt_at = None;
                self.last_linked_at = Some(now);
            }
            DialTransition::Backoff {
                attempts,
                next_at,
                error,
            } => {
                self.state = DialState::Backoff;
                self.attempts = *attempts;
                self.next_attempt_at = Some(*next_at);
                self.last_error = Some(truncate_chars(error, MAX_LAST_ERROR_CHARS));
                self.last_error_at = Some(now);
            }
            DialTransition::NotInstalled => {
                self.state = DialState::NotInstalled;
                self.next_attempt_at = Some(now.saturating_add(NOT_INSTALLED_RETRY_MS));
            }
            DialTransition::Stopped { reason, version } => {
                self.state = DialState::Stopped;
                self.stopped = Some((*reason, Some(version.clone())));
                self.next_attempt_at = None;
            }
        }
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// What [`upsert_discovered`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovered {
    /// A new, enabled row.
    Inserted(TargetId),
    /// The destination already has a row, whatever its source or state; it was
    /// left exactly as it was.
    Present(TargetId),
    /// The destination is new but the node already holds
    /// [`MAX_DIAL_TARGETS`] enabled targets.
    Skipped,
}

fn invalid(detail: &'static str) -> ApiError {
    api_error(ErrorCode::InvalidRequest, detail)
}

fn check_text(text: &str, what: &'static str) -> Result<(), ApiError> {
    if text.chars().any(char::is_control) {
        return Err(invalid("dial target contains control characters"));
    }
    if text.is_empty() || text.chars().count() > MAX_TARGET_TEXT {
        return Err(invalid(match what {
            "dest" => "dial target destination length out of range",
            _ => "dial target remote command length out of range",
        }));
    }
    Ok(())
}

fn check_dest(dest: &str) -> Result<(), ApiError> {
    check_text(dest, "dest")?;
    if dest.starts_with('-') {
        return Err(invalid("dial target must not start with '-'"));
    }
    Ok(())
}

fn enabled_count(tx: &Connection) -> Result<usize, ApiError> {
    tx.query_row(
        "SELECT count(*) FROM sync_dial_targets WHERE enabled = 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n as usize)
    .map_err(store_error)
}

fn enabled_row(tx: &Connection, dest: &str) -> Result<Option<(i64, bool)>, ApiError> {
    tx.query_row(
        "SELECT target_id, enabled FROM sync_dial_targets WHERE ssh_dest = ?1",
        [dest],
        |row| Ok((row.get(0)?, row.get::<_, i64>(1)? == 1)),
    )
    .optional()
    .map_err(store_error)
}

/// The operator adds (or re-adds) `dest`: the row becomes `explicit`, enabled
/// and `idle` with attempts, stop and retry time cleared, and `remote_command`
/// replaced (`None` clears an override). Re-adding is the operator's "changed".
/// A target whose link is up and whose command is unchanged stays `linked`: the
/// live link is not re-dialed, so nothing would record `linked` again.
///
/// `InvalidRequest` for a bad destination or command, and "too many dial
/// targets (64)" when the add would make more than [`MAX_DIAL_TARGETS`]
/// enabled rows (re-adding an enabled target never grows the count).
pub fn add_explicit(
    tx: &Transaction<'_>,
    dest: &str,
    remote_command: Option<&str>,
    now: i64,
) -> Result<TargetId, ApiError> {
    check_dest(dest)?;
    if let Some(command) = remote_command {
        check_text(command, "command")?;
    }
    let already_enabled = matches!(enabled_row(tx, dest)?, Some((_, true)));
    if !already_enabled && enabled_count(tx)? >= MAX_DIAL_TARGETS {
        return Err(invalid("too many dial targets (64)"));
    }
    let id: i64 = tx
        .query_row(
            "INSERT INTO sync_dial_targets(ssh_dest,source,enabled,remote_command,created_at,updated_at) \
             VALUES (?1,'explicit',1,?2,?3,?3) \
             ON CONFLICT(ssh_dest) DO UPDATE SET source='explicit', enabled=1, \
               state=CASE WHEN state='linked' AND remote_command IS excluded.remote_command \
                 THEN 'linked' ELSE 'idle' END, \
               remote_command=excluded.remote_command, attempts=0, \
               stopped_reason=NULL, stopped_version=NULL, next_attempt_at=NULL, \
               updated_at=excluded.updated_at \
             RETURNING target_id",
            params![dest, remote_command, now],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    Ok(TargetId(id))
}

/// Discovery found `dest`: inserts an enabled row when the destination is
/// absent. An existing row is never touched (not its source, `enabled`,
/// `remote_command` or state), so discovery neither overwrites an explicit
/// target nor re-enables a forgotten one.
pub fn upsert_discovered(
    tx: &Transaction<'_>,
    dest: &str,
    source: DiscoverySource,
    now: i64,
) -> Result<Discovered, ApiError> {
    check_dest(dest)?;
    if let Some((id, _)) = enabled_row(tx, dest)? {
        return Ok(Discovered::Present(TargetId(id)));
    }
    if enabled_count(tx)? >= MAX_DIAL_TARGETS {
        return Ok(Discovered::Skipped);
    }
    let id: i64 = tx
        .query_row(
            "INSERT INTO sync_dial_targets(ssh_dest,source,enabled,created_at,updated_at) \
             VALUES (?1,?2,1,?3,?3) RETURNING target_id",
            params![dest, DialSource::from(source).as_str(), now],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    Ok(Discovered::Inserted(TargetId(id)))
}

/// Discovery's process scan no longer sees a `herdr --remote` client for a
/// destination: deletes each enabled `herdr_remote_process` row that never
/// linked and whose destination is not in `seen` (every destination the pass
/// found), so a one-off client (a test harness's `herdr --remote test-only`, a
/// typo) is not dialed forever (ht-hvyf). Explicit, `herdr_machine`, forgotten
/// and once-linked rows are kept. An unseen row mid-dial (`dialing`, or
/// `linked` not yet recorded) is kept for now, since its link may be up before
/// the Dialer records it, and counted in [`Retired::deferred`] so the caller
/// tries again. A later sighting adds a deleted row again.
pub fn retire_unseen_remote_processes(
    tx: &Transaction<'_>,
    seen: &[&str],
) -> Result<Retired, ApiError> {
    let mut candidates = tx
        .prepare(
            "SELECT target_id, ssh_dest, state IN ('dialing','linked') FROM sync_dial_targets \
             WHERE source='herdr_remote_process' AND enabled=1 \
               AND learned_node_uuid IS NULL AND last_linked_at IS NULL",
        )
        .map_err(store_error)?;
    let unseen = candidates
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?
        .into_iter()
        .filter(|(_, dest, _)| !seen.contains(&dest.as_str()))
        .collect::<Vec<_>>();
    drop(candidates);
    let mut retired = Retired::default();
    for (id, _, mid_dial) in unseen {
        if mid_dial {
            retired.deferred += 1;
            continue;
        }
        tx.execute("DELETE FROM sync_dial_targets WHERE target_id=?1", [id])
            .map_err(store_error)?;
        retired.deleted += 1;
    }
    Ok(retired)
}

/// What [`retire_unseen_remote_processes`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retired {
    /// Rows deleted.
    pub deleted: usize,
    /// Unseen rows kept because they were mid-dial: retirement is due again.
    pub deferred: usize,
}

/// Disables the target whose destination or learned node uuid is
/// `dest_or_node`. Nothing is deleted; the row keeps its history and a later
/// [`add_explicit`] enables it again. Returns the rows disabled (0 when there
/// is no such target or it was already disabled).
pub fn forget(tx: &Transaction<'_>, dest_or_node: &str, now: i64) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE sync_dial_targets SET enabled=0, updated_at=?2 \
         WHERE enabled=1 AND (ssh_dest=?1 OR learned_node_uuid=?1)",
        params![dest_or_node, now],
    )
    .map_err(store_error)
}

const COLUMNS: &str = "target_id,ssh_dest,source,enabled,remote_command,learned_node_uuid,state,\
    stopped_reason,stopped_version,attempts,next_attempt_at,last_error,last_error_at,\
    last_linked_at,created_at,updated_at";

fn corrupt(column: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("malformed sync_dial_targets.{column}").into(),
    )
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<DialTarget> {
    let source: String = row.get(2)?;
    let learned: Option<String> = row.get(5)?;
    let state: String = row.get(6)?;
    let stopped_reason: Option<String> = row.get(7)?;
    let stopped_version: Option<String> = row.get(8)?;
    let stopped = match stopped_reason {
        Some(text) => Some((
            StopReason::parse(&text).ok_or_else(|| corrupt("stopped_reason"))?,
            stopped_version,
        )),
        None => None,
    };
    Ok(DialTarget {
        target_id: TargetId(row.get(0)?),
        ssh_dest: row.get(1)?,
        source: DialSource::parse(&source).ok_or_else(|| corrupt("source"))?,
        enabled: row.get::<_, i64>(3)? == 1,
        remote_command: row.get(4)?,
        learned_node: learned
            .map(|text| text.parse().map_err(|_| corrupt("learned_node_uuid")))
            .transpose()?,
        state: DialState::parse(&state).ok_or_else(|| corrupt("state"))?,
        stopped,
        attempts: row.get::<_, i64>(9)? as u32,
        next_attempt_at: row.get(10)?,
        last_error: row.get(11)?,
        last_error_at: row.get(12)?,
        last_linked_at: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn select(conn: &Connection, filter: &str) -> Result<Vec<DialTarget>, ApiError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {COLUMNS} FROM sync_dial_targets {filter} ORDER BY target_id"
        ))
        .map_err(store_error)?;
    statement
        .query_map([], read_row)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)
}

/// Every target, enabled or not, by id.
pub fn list(conn: &Connection) -> Result<Vec<DialTarget>, ApiError> {
    select(conn, "")
}

/// The enabled targets, by id.
pub fn enabled(conn: &Connection) -> Result<Vec<DialTarget>, ApiError> {
    select(conn, "WHERE enabled = 1")
}

/// Stores what the Dialer observed for `target` (see [`DialTarget::apply`]).
/// `InvalidRequest` when the target does not exist: a caller bug, or a row
/// discovery retired while its dial task was still running.
pub fn record_transition(
    tx: &Transaction<'_>,
    target: TargetId,
    transition: &DialTransition,
    now: i64,
) -> Result<(), ApiError> {
    let mut row = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM sync_dial_targets WHERE target_id = ?1"),
            [target.0],
            read_row,
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| invalid("unknown dial target"))?;
    let was_linked = row.state == DialState::Linked;
    row.apply(transition, now);
    // Link up and down are the coordinator's events; these are the dial
    // outcomes only it cannot see.
    let event = match transition {
        DialTransition::Backoff { error, .. } if !was_linked => {
            Some((LinkEventKind::DialFailed, Some(error.clone())))
        }
        DialTransition::NotInstalled => Some((LinkEventKind::NotInstalled, None)),
        DialTransition::Stopped { reason, .. } => {
            Some((LinkEventKind::Stopped, Some(reason.as_str().to_owned())))
        }
        _ => None,
    };
    if let Some((kind, detail)) = event {
        sync_link_events::record(
            tx,
            &LinkEvent {
                at: now,
                kind,
                origin: LinkOrigin::Dialed,
                node: row.learned_node,
                dest: Some(row.ssh_dest.clone()),
                detail,
            },
        )?;
    }
    let (reason, version) = match &row.stopped {
        Some((reason, version)) => (Some(reason.as_str()), version.as_deref()),
        None => (None, None),
    };
    tx.execute(
        "UPDATE sync_dial_targets SET state=?2, stopped_reason=?3, stopped_version=?4, \
         attempts=?5, next_attempt_at=?6, last_error=?7, last_error_at=?8, \
         learned_node_uuid=?9, last_linked_at=?10, updated_at=?11 WHERE target_id=?1",
        params![
            target.0,
            row.state.as_str(),
            reason,
            version,
            i64::from(row.attempts),
            row.next_attempt_at,
            row.last_error,
            row.last_error_at,
            row.learned_node.map(|node| node.to_string()),
            row.last_linked_at,
            row.updated_at,
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Run once at start: resets to `idle` every `stopped` target whose
/// `stopped_version` is not `version`, because a local upgrade is a change that
/// may fix the cause (`envelope_unsupported`). Returns the rows reset.
pub fn clear_stale_stops(tx: &Transaction<'_>, version: &str) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE sync_dial_targets SET state='idle', stopped_reason=NULL, stopped_version=NULL, \
         attempts=0, next_attempt_at=NULL WHERE state='stopped' AND stopped_version IS NOT ?1",
        [version],
    )
    .map_err(store_error)
}
