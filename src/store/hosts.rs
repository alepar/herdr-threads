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
use crate::ports::{HostId, HostKind};
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
    /// The stored `herdr_hosts.kind` text, as written (possibly a kind this
    /// build does not know; see [`HostRow::host_kind`]).
    pub kind: String,
    /// When a Herdr attach last stamped the kind; `None` while it is the
    /// column default.
    pub kind_probed_at: Option<i64>,
}

impl HostRow {
    /// The row's kind as this build reads it; `Unknown` for foreign text.
    pub fn host_kind(&self) -> HostKind {
        HostKind::from_stored(&self.kind)
    }

    /// Whether this build may attach the row (G4: its kind is implemented).
    /// The built-in named host is implemented but never dialled, so it is not
    /// attachable through reattach.
    pub fn attachable(&self) -> bool {
        self.host_kind().is_implemented() && self.host_kind() != HostKind::Named
    }
}

/// Upserts the host row of `locator` for this node, stamping `last_attached_at`
/// to `now`, and returns its id with the persisted host epoch (0 for a new
/// row). The attach writes the probed `kind` and `kind_probed_at=now`. A row
/// whose stored kind this build does not implement is refused with
/// `HostUnavailable` and left untouched (G4: never dialled as Herdr), and so is
/// a row whose implemented stored kind differs from `kind` (H1b: the endpoint
/// changed identity, nothing is rewritten silently). One `BEGIN IMMEDIATE`; a
/// second call returns the same id.
pub fn attach_host(
    conn: &mut Connection,
    locator: &HostLocator,
    now: i64,
    kind: HostKind,
) -> Result<(HostId, i64), ApiError> {
    let Some(kind_name) = kind.stored_name() else {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            format!("host {} has no attachable kind", locator.as_str()),
        ));
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let stored: Option<String> = tx
        .query_row(
            "SELECT kind FROM herdr_hosts WHERE endpoint=?1",
            [locator.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(stored) = stored {
        if !HostKind::from_stored(&stored).is_implemented() {
            return Err(api_error(
                ErrorCode::HostUnavailable,
                format!(
                    "host {} has kind '{stored}', which this build does not implement",
                    locator.as_str()
                ),
            ));
        }
        if stored != kind_name {
            return Err(api_error(
                ErrorCode::HostUnavailable,
                format!(
                    "host {} is stored as kind '{stored}' but answers as '{kind_name}'",
                    locator.as_str()
                ),
            ));
        }
    }
    let (host_id, epoch): (i64, i64) = tx
        .query_row(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at,last_attached_at,kind,kind_probed_at) \
             VALUES ((SELECT id FROM local_node),?1,?2,?2,?3,?2) \
             ON CONFLICT(endpoint) DO UPDATE SET last_attached_at=excluded.last_attached_at, \
                 kind=excluded.kind, kind_probed_at=excluded.kind_probed_at \
             RETURNING host_id,host_epoch",
            params![locator.as_str(), now, kind_name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok((HostId::from_row(host_id), epoch))
}

const ROW_COLUMNS: &str =
    "host_id,endpoint,created_at,last_attached_at,host_epoch,kind,kind_probed_at";

/// Reads one row; a legacy endpoint that is not a [`HostLocator`] (the interim
/// unnamed host) yields `None` for its locator.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<HostRow>> {
    let endpoint: String = row.get(1)?;
    let id = HostId::from_row(row.get(0)?);
    let (created_at, last_attached_at, host_epoch) = (row.get(2)?, row.get(3)?, row.get(4)?);
    let (kind, kind_probed_at) = (row.get(5)?, row.get(6)?);
    Ok(HostLocator::parse(endpoint).ok().map(|locator| HostRow {
        id,
        locator,
        created_at,
        last_attached_at,
        host_epoch,
        kind,
        kind_probed_at,
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

/// A host row this build never attaches: its stored kind is outside
/// [`HostKind::IMPLEMENTED`]. Raw endpoint text, since a foreign kind need not
/// use a Herdr locator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnattachableHost {
    pub id: HostId,
    pub endpoint: String,
    pub kind: String,
}

/// The rows whose kind this build does not implement, by `host_id`. This is
/// what `doctor` names; reattach skips the same rows.
pub fn unattachable(conn: &Connection) -> Result<Vec<UnattachableHost>, ApiError> {
    let mut stmt = conn
        .prepare("SELECT host_id,endpoint,kind FROM herdr_hosts ORDER BY host_id")
        .map_err(store_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(UnattachableHost {
                id: HostId::from_row(row.get(0)?),
                endpoint: row.get(1)?,
                kind: row.get(2)?,
            })
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(store_error)?;
        if !HostKind::from_stored(&row.kind).is_implemented() {
            out.push(row);
        }
    }
    Ok(out)
}

/// The claim host of a claim the store builds itself from a binding row (archival ticket, mod
/// presentation refresh): the endpoint of the Herdr server `seat` sits on when it is a canonical
/// locator, else the interim unnamed host's locator. Internal claims never leave the
/// store and are not compared with a fence, so this names the server, it does not route.
pub(crate) fn seat_claim_host(db: &Connection, seat: &str) -> HostLocator {
    let endpoint: Option<String> = db
        .query_row(
            "SELECT h.endpoint FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1",
            [seat],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    endpoint
        .and_then(|endpoint| HostLocator::parse(endpoint).ok())
        .unwrap_or_else(|| {
            HostLocator::parse(crate::store::schema::UNNAMED_HOST_ENDPOINT)
                .expect("the placeholder is a canonical locator")
        })
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

    /// Adds an attached host. `HostSessions` only (until it lands, the daemon's
    /// single-host composition), after the host's workers run.
    pub(crate) fn publish(&self, locator: HostLocator, id: HostId) {
        if let Ok(mut map) = self.by_locator.write() {
            map.insert(locator, id);
        }
    }

    /// Test hook: publishes `id` at `locator` as `HostSessions` would, for fixtures outside the
    /// crate that attach hosts through the store port.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_for_test(&self, locator: HostLocator, id: HostId) {
        self.publish(locator, id);
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
        let (id, epoch) = attach_host(&mut db, &a, 100, HostKind::Herdr).unwrap();
        assert_eq!(epoch, 0);
        db.execute(
            "UPDATE herdr_hosts SET host_epoch=7 WHERE host_id=?1",
            [id.get()],
        )
        .unwrap();
        let (again, epoch) = attach_host(&mut db, &a, 250, HostKind::Herdr).unwrap();
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
        let (a, _) = attach_host(&mut db, &loc("/run/a.sock"), 10, HostKind::Herdr).unwrap();
        let (b, _) = attach_host(&mut db, &loc("/run/b.sock"), 20, HostKind::Herdr).unwrap();
        let (c, _) = attach_host(&mut db, &loc("/run/c.sock"), 30, HostKind::Herdr).unwrap();
        attach_host(&mut db, &loc("/run/a.sock"), 40, HostKind::Herdr).unwrap();
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
