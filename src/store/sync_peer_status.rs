//! What this node last knew of each peer node (ht-xg88 node status).
//!
//! Responsibility (sync_peer_status)
//! - Owns: the `sync_peer_status` table: [`record`] (the coordinator's
//!   throttled shadow of its live link state) and [`list`].
//! - Hides: the SQL and the merge rule (a row of a new boot replaces the
//!   version; a missing version or position keeps the stored one).
//! - Must not: decide reachability (`sync_contacts::reach` does) or feed any
//!   routing, dial or seat decision; every field is display data.
//! - Role: written through `SyncStorePort::record_peer_status`, read by
//!   `store::sync_status::nodes` for `peer list`.

use rusqlite::{Connection, Transaction, params};
use uuid::Uuid;

use super::connection::{api_error, store_error};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};

/// One peer node's last-known status, on this node's wall clock (unix ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerStatus {
    pub node: NodeUuid,
    /// The peer's self-reported daemon version (`node_info`).
    pub version: Option<String>,
    /// The capabilities negotiated with the peer.
    pub caps: Vec<String>,
    pub boot: Uuid,
    /// Since when the node has an active link; `None` while it has none.
    pub linked_since: Option<u64>,
    pub last_linked_at: u64,
    /// `(acked, head)`: the peer holds `acked` of this node's active log,
    /// whose head is `head`.
    pub tx: Option<(u64, u64)>,
    /// `(have, head)`: this node holds `have` of the peer's active log, whose
    /// advertised head is `head`.
    pub rx: Option<(u64, u64)>,
    /// The last frame the peer's coordinator sent (not ping/pong).
    pub app_seen_at: Option<u64>,
}

fn ms(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "peer status value overflow"))
}

fn unsigned(value: Option<i64>) -> Result<Option<u64>, ApiError> {
    value
        .map(|v| {
            u64::try_from(v)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative peer status value"))
        })
        .transpose()
}

/// Upserts `rows`. A row whose boot differs from the stored one replaces the
/// version; otherwise a missing version, position or app contact keeps the
/// stored value. `now` stamps `updated_at`.
pub fn record(tx: &Transaction<'_>, rows: &[PeerStatus], now: u64) -> Result<(), ApiError> {
    let mut statement = tx
        .prepare_cached(
            "INSERT INTO sync_peer_status(node_uuid,version,caps,boot,linked_since,last_linked_at,\
               tx_acked,tx_head,rx_have,rx_head,app_seen_at,updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) \
             ON CONFLICT(node_uuid) DO UPDATE SET \
               version = CASE WHEN excluded.boot <> boot THEN excluded.version \
                              ELSE coalesce(excluded.version, version) END, \
               caps = excluded.caps, boot = excluded.boot, \
               linked_since = excluded.linked_since, \
               last_linked_at = max(last_linked_at, excluded.last_linked_at), \
               tx_acked = coalesce(excluded.tx_acked, tx_acked), \
               tx_head = coalesce(excluded.tx_head, tx_head), \
               rx_have = coalesce(excluded.rx_have, rx_have), \
               rx_head = coalesce(excluded.rx_head, rx_head), \
               app_seen_at = max(coalesce(app_seen_at, 0), coalesce(excluded.app_seen_at, 0)), \
               updated_at = excluded.updated_at",
        )
        .map_err(store_error)?;
    let now = ms(now)?;
    for row in rows {
        let version = row
            .version
            .as_deref()
            .filter(|v| !v.is_empty() && v.len() <= crate::sync::frame::MAX_NODE_VERSION_BYTES);
        statement
            .execute(params![
                row.node.to_string(),
                version,
                row.caps.join(" "),
                row.boot.to_string(),
                row.linked_since.map(ms).transpose()?,
                ms(row.last_linked_at)?,
                row.tx.map(|(a, _)| ms(a)).transpose()?,
                row.tx.map(|(_, h)| ms(h)).transpose()?,
                row.rx.map(|(a, _)| ms(a)).transpose()?,
                row.rx.map(|(_, h)| ms(h)).transpose()?,
                row.app_seen_at.map(ms).transpose()?,
                now
            ])
            .map_err(store_error)?;
    }
    Ok(())
}

/// Every stored row, by node.
pub fn list(conn: &Connection) -> Result<Vec<PeerStatus>, ApiError> {
    let mut statement = conn
        .prepare_cached(
            "SELECT node_uuid,version,caps,boot,linked_since,last_linked_at,tx_acked,tx_head,\
               rx_have,rx_head,app_seen_at FROM sync_peer_status ORDER BY node_uuid",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
                [
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                ],
                row.get::<_, Option<i64>>(10)?,
            ))
        })
        .map_err(store_error)?;
    let corrupt = |what: &str| api_error(ErrorCode::StoreCorrupt, format!("malformed peer {what}"));
    let mut out = Vec::new();
    for row in rows {
        let (node, version, caps, boot, linked_since, last_linked_at, positions, app) =
            row.map_err(store_error)?;
        let [tx_acked, tx_head, rx_have, rx_head] = positions;
        let pair = |a: Option<i64>, b: Option<i64>| -> Result<Option<(u64, u64)>, ApiError> {
            Ok(unsigned(a)?.zip(unsigned(b)?))
        };
        out.push(PeerStatus {
            node: node.parse().map_err(|_| corrupt("node"))?,
            version,
            caps: caps.split_whitespace().map(str::to_owned).collect(),
            boot: Uuid::parse_str(&boot).map_err(|_| corrupt("boot"))?,
            linked_since: unsigned(linked_since)?,
            last_linked_at: unsigned(Some(last_linked_at))?.unwrap_or(0),
            tx: pair(tx_acked, tx_head)?,
            rx: pair(rx_have, rx_head)?,
            app_seen_at: unsigned(app)?,
        });
    }
    Ok(out)
}
