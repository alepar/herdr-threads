//! What this node last knew of each peer node (ht-xg88 node status).
//!
//! Responsibility (sync_peer_status)
//! - Owns: the `sync_peer_status` table: [`record`] (the coordinator's
//!   throttled shadow of its live link state) and [`list`].
//! - Hides: the SQL and the merge rule (a row of a new boot replaces every
//!   value; within a boot a missing version or position keeps the stored
//!   one).
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
    /// The peer's boot the values below belong to.
    pub boot: Uuid,
    /// `(acked, head)`: the peer holds `acked` of this node's active log,
    /// whose head is `head`.
    pub tx: Option<(u64, u64)>,
    /// `(have, head)`: this node holds `have` of the peer's active log, whose
    /// advertised head is `head`.
    pub rx: Option<(u64, u64)>,
    /// The last frame the peer's coordinator sent (not ping/pong).
    pub app_seen_at: Option<u64>,
}

/// A stored value: clamped to `i64::MAX` (display data never fails a
/// write).
fn ms(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn unsigned(value: Option<i64>) -> Result<Option<u64>, ApiError> {
    value
        .map(|v| {
            u64::try_from(v)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative peer status value"))
        })
        .transpose()
}

/// Upserts `rows`. A row of a new boot replaces every value (the old boot's
/// version and positions describe another run). Within one boot a missing
/// version or position keeps the stored one and app contact only moves
/// forward. `now` stamps `updated_at`.
pub fn record(tx: &Transaction<'_>, rows: &[PeerStatus], now: u64) -> Result<(), ApiError> {
    let mut statement = tx
        .prepare_cached(
            "INSERT INTO sync_peer_status(node_uuid,version,boot,tx_acked,tx_head,rx_have,rx_head,\
               app_seen_at,updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) \
             ON CONFLICT(node_uuid) DO UPDATE SET \
               version = CASE WHEN excluded.boot <> boot THEN excluded.version \
                              ELSE coalesce(excluded.version, version) END, \
               tx_acked = CASE WHEN excluded.boot <> boot OR excluded.tx_acked IS NOT NULL \
                               THEN excluded.tx_acked ELSE tx_acked END, \
               tx_head = CASE WHEN excluded.boot <> boot OR excluded.tx_acked IS NOT NULL \
                              THEN excluded.tx_head ELSE tx_head END, \
               rx_have = CASE WHEN excluded.boot <> boot OR excluded.rx_have IS NOT NULL \
                              THEN excluded.rx_have ELSE rx_have END, \
               rx_head = CASE WHEN excluded.boot <> boot OR excluded.rx_have IS NOT NULL \
                              THEN excluded.rx_head ELSE rx_head END, \
               app_seen_at = CASE WHEN excluded.boot <> boot OR app_seen_at IS NULL \
                                  THEN excluded.app_seen_at \
                                  ELSE max(app_seen_at, coalesce(excluded.app_seen_at, 0)) END, \
               boot = excluded.boot, \
               updated_at = excluded.updated_at",
        )
        .map_err(store_error)?;
    let now = ms(now);
    for row in rows {
        statement
            .execute(params![
                row.node.to_string(),
                row.version.as_deref(),
                row.boot.to_string(),
                row.tx.map(|(a, _)| ms(a)),
                row.tx.map(|(_, h)| ms(h)),
                row.rx.map(|(a, _)| ms(a)),
                row.rx.map(|(_, h)| ms(h)),
                row.app_seen_at.map(ms),
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
            "SELECT node_uuid,version,boot,tx_acked,tx_head,rx_have,rx_head,app_seen_at \
             FROM sync_peer_status ORDER BY node_uuid",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                [
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ],
                row.get::<_, Option<i64>>(7)?,
            ))
        })
        .map_err(store_error)?;
    let corrupt = |what: &str| api_error(ErrorCode::StoreCorrupt, format!("malformed peer {what}"));
    let mut out = Vec::new();
    for row in rows {
        let (node, version, boot, positions, app) = row.map_err(store_error)?;
        let [tx_acked, tx_head, rx_have, rx_head] = positions;
        let pair = |a: Option<i64>, b: Option<i64>| -> Result<Option<(u64, u64)>, ApiError> {
            Ok(unsigned(a)?.zip(unsigned(b)?))
        };
        out.push(PeerStatus {
            node: node.parse().map_err(|_| corrupt("node"))?,
            version,
            boot: Uuid::parse_str(&boot).map_err(|_| corrupt("boot"))?,
            tx: pair(tx_acked, tx_head)?,
            rx: pair(rx_have, rx_head)?,
            app_seen_at: unsigned(app)?,
        });
    }
    Ok(out)
}
