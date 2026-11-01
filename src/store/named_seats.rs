//! The store's read of named seats for the built-in named host.
//!
//! Responsibility: [`NamedSeatSource`] is the narrow read `NamedHost` composes
//! (host -> store-read trait, never store -> host). It owns the SQL that lists
//! the live named seats of one host, keyed on the host id so no row can be
//! missing. It does not decide presence, closure or wake; that stays with the
//! host and reconcile. New named-seat reads go here.

use rusqlite::Connection;

use super::{SqliteStore, connection::store_error};
use crate::ports::HostId;
use crate::protocol::{
    ids::{SeatId, SeatName},
    results::ApiError,
    time::CallBudget,
};

/// Every non-retired local named seat of one host (resolved or unresolved).
pub trait NamedSeatSource: Send + Sync {
    /// The live named seats of `host`, by creation order. A stored name that
    /// does not parse as a [`SeatName`] is skipped: it is never observed.
    fn live_named(
        &self,
        host: &HostId,
        budget: &CallBudget,
    ) -> Result<Vec<(SeatId, SeatName)>, ApiError>;
}

/// The rows behind [`NamedSeatSource::live_named`].
pub fn live_named(conn: &Connection, host: &HostId) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT id,name FROM seats \
             WHERE host_id=?1 AND is_local=1 AND state<>'retired' AND name IS NOT NULL \
             ORDER BY ordinal",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map([host.get()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name) = row.map_err(store_error)?;
        if let (Ok(id), Ok(name)) = (SeatId::parse(id), SeatName::parse(name)) {
            out.push((id, name));
        }
    }
    Ok(out)
}

impl NamedSeatSource for SqliteStore {
    fn live_named(
        &self,
        host: &HostId,
        budget: &CallBudget,
    ) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        let db = self.context().open_query(budget.clone())?;
        live_named(&db, host)
    }
}
