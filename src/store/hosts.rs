//! The store's access to `herdr_hosts` and its attached-host view.
//!
//! Responsibility: this module owns the `herdr_hosts` rows' addressing side
//! (attach upsert, row lookup, the remembered-host list) and [`HostIndex`], the
//! store's read view of which Herdr servers are attached. It hides the SQL and
//! the lock behind the index map. It does not own the per-host observation and
//! identity columns (`store::seats`), host reachability, or sessions
//! (`HostSessions` in `service`); the store never reads `herdr_hosts` or
//! `HostSessions` to decide whether a host is attached, it asks [`HostIndex`].
//! New per-host addressing behaviour goes here. Role: the seam the claim,
//! store, wake-scope and `HostSessions` work builds on; it is inert until a
//! caller migrates to it.

use std::{collections::BTreeMap, sync::RwLock};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::connection::{api_error, store_error};
use crate::ports::HostId;
use crate::protocol::{
    ids::HostLocator,
    results::{ApiError, ErrorCode},
};

/// One `herdr_hosts` row as the addressing seam sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRow {
    pub id: HostId,
    pub locator: HostLocator,
    pub created_at: i64,
    pub last_attached_at: Option<i64>,
    pub host_epoch: i64,
}

/// Upserts the host row of `locator` for this node, stamping `last_attached_at`
/// to `now`, and returns its id with the persisted host epoch (0 for a new
/// row). One `BEGIN IMMEDIATE`; a second call returns the same id.
pub fn attach_host(
    conn: &mut Connection,
    locator: &HostLocator,
    now: i64,
) -> Result<(HostId, i64), ApiError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let (host_id, epoch): (i64, i64) = tx
        .query_row(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at,last_attached_at) \
             VALUES ((SELECT id FROM local_node),?1,?2,?2) \
             ON CONFLICT(endpoint) DO UPDATE SET last_attached_at=excluded.last_attached_at \
             RETURNING host_id,host_epoch",
            params![locator.as_str(), now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok((HostId::from_row(host_id), epoch))
}

const ROW_COLUMNS: &str = "host_id,endpoint,created_at,last_attached_at,host_epoch";

/// Reads one row; a legacy endpoint that is not a [`HostLocator`] (the interim
/// unnamed host) yields `None` for its locator.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<HostRow>> {
    let endpoint: String = row.get(1)?;
    let id = HostId::from_row(row.get(0)?);
    let (created_at, last_attached_at, host_epoch) = (row.get(2)?, row.get(3)?, row.get(4)?);
    Ok(HostLocator::parse(endpoint).ok().map(|locator| HostRow {
        id,
        locator,
        created_at,
        last_attached_at,
        host_epoch,
    }))
}

/// The row of `id`, or `None` when it does not exist or its endpoint is not a
/// canonical locator.
pub fn host_row(conn: &Connection, id: HostId) -> Result<Option<HostRow>, ApiError> {
    let found = conn
        .query_row(
            &format!("SELECT {ROW_COLUMNS} FROM herdr_hosts WHERE host_id=?1"),
            [id.get()],
            read_row,
        )
        .optional()
        .map_err(store_error)?;
    Ok(found.flatten())
}

/// The hosts this node has attached before, most recently attached first, at
/// most `limit`. Rows whose endpoint is not a canonical locator are skipped.
pub fn remembered(conn: &Connection, limit: usize) -> Result<Vec<HostRow>, ApiError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ROW_COLUMNS} FROM herdr_hosts \
             ORDER BY last_attached_at DESC, host_id DESC"
        ))
        .map_err(store_error)?;
    let rows = stmt.query_map([], read_row).map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        if out.len() >= limit {
            break;
        }
        if let Some(row) = row.map_err(store_error)? {
            out.push(row);
        }
    }
    Ok(out)
}

/// HostIndex: the store's read view of which Herdr servers are attached.
///
/// Owns: the `HostLocator -> HostId` map of attached hosts and
/// [`HostIndex::attached_id`] (a miss is `HostUnavailable`).
/// Hides: the lock behind the map.
/// Must not: be written by anything except `HostSessions`, hold a host whose
/// workers are not yet running, or be consulted for reachability.
/// Role: held by `StoreContext` as an `Arc` and shared with `HostSessions`. The
/// store depends on it; it depends on nothing in `service`, so the store never
/// reaches up to the registry. Created before `StoreContext`, lives for the
/// daemon; entries are only added (no detach).
#[derive(Debug, Default)]
pub struct HostIndex {
    by_locator: RwLock<BTreeMap<HostLocator, HostId>>,
}

impl HostIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// The id of the attached host at `locator`.
    pub fn attached_id(&self, locator: &HostLocator) -> Result<HostId, ApiError> {
        self.by_locator
            .read()
            .ok()
            .and_then(|map| map.get(locator).copied())
            .ok_or_else(|| api_error(ErrorCode::HostUnavailable, "Herdr server not attached"))
    }

    pub fn contains(&self, id: HostId) -> bool {
        self.by_locator
            .read()
            .is_ok_and(|map| map.values().any(|known| *known == id))
    }

    /// Every attached host id, ascending.
    pub fn ids(&self) -> Vec<HostId> {
        let mut ids: Vec<HostId> = self
            .by_locator
            .read()
            .map(|map| map.values().copied().collect())
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// Adds an attached host. `HostSessions` only, after the host's workers run.
    // No production caller until the HostSessions leaf (ht-5fr.2.7).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish(&self, locator: HostLocator, id: HostId) {
        if let Ok(mut map) = self.by_locator.write() {
            map.insert(locator, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::time::UtcMillis;
    use crate::store::schema;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
        db
    }

    fn loc(text: &str) -> HostLocator {
        HostLocator::parse(text).unwrap()
    }

    #[test]
    fn attach_host_inserts_then_restamps_same_id_and_epoch() {
        let mut db = db();
        let a = loc("/run/a.sock");
        let (id, epoch) = attach_host(&mut db, &a, 100).unwrap();
        assert_eq!(epoch, 0);
        db.execute(
            "UPDATE herdr_hosts SET host_epoch=7 WHERE host_id=?1",
            [id.get()],
        )
        .unwrap();
        let (again, epoch) = attach_host(&mut db, &a, 250).unwrap();
        assert_eq!(again, id);
        assert_eq!(epoch, 7);
        let row = host_row(&db, id).unwrap().unwrap();
        assert_eq!(row.last_attached_at, Some(250));
        assert_eq!(row.created_at, 100);
        let count: i64 = db
            .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(host_row(&db, HostId::from_row(999)).unwrap(), None);
    }

    #[test]
    fn remembered_orders_by_last_attached_desc_with_limit() {
        let mut db = db();
        let (a, _) = attach_host(&mut db, &loc("/run/a.sock"), 10).unwrap();
        let (b, _) = attach_host(&mut db, &loc("/run/b.sock"), 20).unwrap();
        let (c, _) = attach_host(&mut db, &loc("/run/c.sock"), 30).unwrap();
        attach_host(&mut db, &loc("/run/a.sock"), 40).unwrap();
        let all: Vec<HostId> = remembered(&db, 10).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(all, vec![a, c, b]);
        let two: Vec<HostId> = remembered(&db, 2).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(two, vec![a, c]);
    }

    #[test]
    fn host_index_miss_is_host_unavailable() {
        let index = HostIndex::new();
        let a = loc("/run/a.sock");
        let error = index.attached_id(&a).unwrap_err();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert_eq!(error.detail, "Herdr server not attached");
        assert!(index.ids().is_empty());
        index.publish(a.clone(), HostId::from_row(3));
        index.publish(loc("/run/b.sock"), HostId::from_row(2));
        assert_eq!(index.attached_id(&a).unwrap(), HostId::from_row(3));
        assert!(index.contains(HostId::from_row(2)));
        assert!(!index.contains(HostId::from_row(9)));
        assert_eq!(index.ids(), vec![HostId::from_row(2), HostId::from_row(3)]);
    }
}
