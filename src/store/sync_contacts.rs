//! Per-node reachability: the persisted half of the contact book.
//!
//! Responsibility (SyncContacts)
//! - Owns: the `sync_node_contact` table, the `MAX(last_contact_at)` upsert
//!   ([`record`]), [`UNREACHABLE_AFTER_MS`] and [`reach`], the one rule that
//!   says whether a node is reachable.
//! - Hides: that gossip carries relative ages and never absolute times, and
//!   the threshold.
//! - Must not: compare wall clocks across nodes, or decide what to flush (the
//!   in-memory `sync::signals::ContactBook` does). Nothing else computes
//!   reachable or unreachable.
//! - Role: storage seam read by `store::queries` and the daemon-less `doctor`
//!   (they call [`reach`] directly) and written by `SyncStorePort`'s SQLite
//!   implementation. New reachability rules go in [`reach`].

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::connection::{api_error, store_error};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};

/// A node with no contact for longer than this (180 s) is unreachable.
pub const UNREACHABLE_AFTER_MS: u64 = 180_000;

/// How a contact was learned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactVia {
    /// A frame arrived from the node itself.
    Direct,
    /// Another node reported the contact in a `seen` frame.
    Relayed,
}

impl ContactVia {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Relayed => "relayed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "direct" => Some(Self::Direct),
            "relayed" => Some(Self::Relayed),
            _ => None,
        }
    }
}

/// The last time this node heard from `node`, on this node's wall clock
/// (unix ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeContact {
    pub node: NodeUuid,
    pub last_contact_at: u64,
    pub via: ContactVia,
}

/// Whether a node is currently reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeReach {
    Reachable,
    /// `since` is the last contact (unix ms); `None` when no contact was ever
    /// recorded.
    Unreachable {
        since: Option<u64>,
    },
}

fn ms(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| api_error(ErrorCode::InvalidRequest, "contact time overflow"))
}

/// Stores `contacts`, keeping for each node the later of the stored and the
/// given `last_contact_at` (and the `via` of the later one). `now` stamps
/// `updated_at`.
pub fn record(tx: &Transaction<'_>, contacts: &[NodeContact], now: u64) -> Result<(), ApiError> {
    let mut statement = tx
        .prepare_cached(
            "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) \
             VALUES (?1,?2,?3,?4) \
             ON CONFLICT(node_uuid) DO UPDATE SET \
               via = CASE WHEN excluded.last_contact_at > last_contact_at \
                          THEN excluded.via ELSE via END, \
               last_contact_at = max(last_contact_at, excluded.last_contact_at), \
               updated_at = excluded.updated_at",
        )
        .map_err(store_error)?;
    let now = ms(now)?;
    for contact in contacts {
        statement
            .execute(params![
                contact.node.to_string(),
                ms(contact.last_contact_at)?,
                contact.via.as_str(),
                now
            ])
            .map_err(store_error)?;
    }
    Ok(())
}

/// The stored contact of `node`, if any.
pub fn get(conn: &Connection, node: &NodeUuid) -> Result<Option<NodeContact>, ApiError> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT last_contact_at, via FROM sync_node_contact WHERE node_uuid = ?1",
            [node.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    row.map(|(at, via)| {
        Ok(NodeContact {
            node: *node,
            last_contact_at: u64::try_from(at)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative contact time"))?,
            via: ContactVia::parse(&via)
                .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "malformed contact via"))?,
        })
    })
    .transpose()
}

/// The reachability of `node` at `now_ms`. The local node is always
/// reachable; any other node is reachable while `now_ms - last_contact_at` is
/// at most [`UNREACHABLE_AFTER_MS`]; with no row it is unreachable since
/// `None`.
pub fn reach(conn: &Connection, node: &NodeUuid, now_ms: u64) -> Result<NodeReach, ApiError> {
    let is_self: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_node WHERE id = ?1)",
            [node.to_string()],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    if is_self {
        return Ok(NodeReach::Reachable);
    }
    Ok(match get(conn, node)? {
        Some(contact) if now_ms.saturating_sub(contact.last_contact_at) <= UNREACHABLE_AFTER_MS => {
            NodeReach::Reachable
        }
        Some(contact) => NodeReach::Unreachable {
            since: Some(contact.last_contact_at),
        },
        None => NodeReach::Unreachable { since: None },
    })
}
