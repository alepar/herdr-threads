//! The peer link event history behind `peer events` (2.8.0).
//!
//! Responsibility (SyncLinkEvents)
//! - Owns: the `sync_link_events` table: [`record`] (with the repeat collapse
//!   and the retention prune) and the bounded, newest-first [`page`].
//! - Hides: the collapse rule (a repeat of the newest row's kind, origin,
//!   node, dest and detail bumps its `n` and `last_at`) and the retention
//!   bounds ([`MAX_ROWS`], [`MAX_AGE_MS`]).
//! - Must not: decide anything. The history is diagnostic display data: no
//!   dial, link, seat or binding decision reads it (N13).
//! - Role: written by `sync_dial_targets::record_transition` (outbound dial
//!   outcomes) and by `SyncStorePort::record_link_events` (the coordinator's
//!   link up and down); read by the `peer events` operator view.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::connection::store_error;
use crate::node::NodeUuid;
use crate::protocol::results::ApiError;

/// Rows kept: the newest 1000.
pub const MAX_ROWS: i64 = 1000;
/// Rows kept: at most 30 days old (by `last_at`).
pub const MAX_AGE_MS: i64 = 30 * 24 * 3_600_000;
/// Longest stored detail, in bytes (cut on a character boundary).
pub const MAX_DETAIL_BYTES: usize = 160;

/// What happened on a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkEventKind {
    /// A link finished its handshake.
    Up,
    /// A link ended; the detail is why.
    Down,
    /// A dial ended before a link was up; the detail is the error.
    DialFailed,
    /// The remote has no herdr-threads.
    NotInstalled,
    /// The remote closed with a stop-class reason; the detail is the reason.
    Stopped,
}

impl LinkEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::DialFailed => "dial_failed",
            Self::NotInstalled => "not_installed",
            Self::Stopped => "stopped",
        }
    }
}

/// Which side opened the link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkOrigin {
    Dialed,
    Inbound,
}

impl LinkOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dialed => "dialed",
            Self::Inbound => "inbound",
        }
    }
}

/// One event to record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkEvent {
    pub at: i64,
    pub kind: LinkEventKind,
    pub origin: LinkOrigin,
    pub node: Option<NodeUuid>,
    pub dest: Option<String>,
    pub detail: Option<String>,
}

/// One stored row, as [`page`] returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredLinkEvent {
    pub seq: i64,
    pub at: i64,
    pub last_at: i64,
    pub n: i64,
    pub node: Option<String>,
    pub dest: Option<String>,
    pub kind: String,
    pub origin: String,
    pub detail: Option<String>,
}

fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Stores `event`: a repeat of the newest row bumps it, else a new row; then
/// prunes past the retention bounds.
pub fn record(tx: &Transaction<'_>, event: &LinkEvent) -> Result<(), ApiError> {
    let node = event.node.map(|node| node.to_string());
    let detail = event
        .detail
        .as_deref()
        .map(|detail| cut(detail, MAX_DETAIL_BYTES));
    let dest = event.dest.as_deref().map(|dest| cut(dest, 1024));
    let newest: Option<i64> = tx
        .query_row(
            "SELECT seq FROM sync_link_events WHERE seq = (SELECT MAX(seq) FROM sync_link_events) \
             AND kind=?1 AND origin=?2 AND node_uuid IS ?3 AND dest IS ?4 AND detail IS ?5",
            params![
                event.kind.as_str(),
                event.origin.as_str(),
                node,
                dest,
                detail
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?;
    match newest {
        Some(seq) => {
            tx.execute(
                "UPDATE sync_link_events SET n = n + 1, last_at = MAX(last_at, ?2) WHERE seq = ?1",
                params![seq, event.at],
            )
            .map_err(store_error)?;
        }
        None => {
            tx.execute(
                "INSERT INTO sync_link_events(at,last_at,n,node_uuid,dest,kind,origin,detail) \
                 VALUES (?1,?1,1,?2,?3,?4,?5,?6)",
                params![
                    event.at,
                    node,
                    dest,
                    event.kind.as_str(),
                    event.origin.as_str(),
                    detail
                ],
            )
            .map_err(store_error)?;
        }
    }
    tx.execute(
        "DELETE FROM sync_link_events WHERE last_at < ?1 \
         OR seq <= (SELECT MAX(seq) FROM sync_link_events) - ?2",
        params![event.at.saturating_sub(MAX_AGE_MS), MAX_ROWS],
    )
    .map_err(store_error)?;
    Ok(())
}

/// At most `limit` rows before `before` (a `seq`; `None` is the newest),
/// newest first, narrowed to `node` when given; the second value says whether
/// older rows remain.
pub fn page(
    conn: &Connection,
    node: Option<NodeUuid>,
    before: Option<i64>,
    limit: u16,
) -> Result<(Vec<StoredLinkEvent>, bool), ApiError> {
    let node = node.map(|node| node.to_string());
    let mut statement = conn
        .prepare_cached(
            "SELECT seq,at,last_at,n,node_uuid,dest,kind,origin,detail FROM sync_link_events \
             WHERE (?1 IS NULL OR node_uuid = ?1) AND (?2 IS NULL OR seq < ?2) \
             ORDER BY seq DESC LIMIT ?3",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map(params![node, before, i64::from(limit) + 1], |row| {
            Ok(StoredLinkEvent {
                seq: row.get(0)?,
                at: row.get(1)?,
                last_at: row.get(2)?,
                n: row.get(3)?,
                node: row.get(4)?,
                dest: row.get(5)?,
                kind: row.get(6)?,
                origin: row.get(7)?,
                detail: row.get(8)?,
            })
        })
        .map_err(store_error)?;
    let mut out = rows.collect::<Result<Vec<_>, _>>().map_err(store_error)?;
    let more = out.len() > usize::from(limit);
    out.truncate(usize::from(limit));
    Ok((out, more))
}
