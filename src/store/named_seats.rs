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
    ids::{NativeSessionId, SeatId, SeatName},
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

    /// `seat`'s connected top-level non-human bindings: the keys of their hook turn state, which
    /// the tmux wake gate combines per pane. Default: none known.
    fn top_level_bindings(
        &self,
        _seat: &SeatId,
        _budget: &CallBudget,
    ) -> Result<Vec<TopLevelBinding>, ApiError> {
        Ok(Vec::new())
    }
}

/// One connected top-level binding of a seat: the key fields of its turn state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopLevelBinding {
    pub generation: u64,
    pub harness: String,
    pub native_session: NativeSessionId,
}

/// The rows behind [`NamedSeatSource::top_level_bindings`]; a session id that does not parse is
/// skipped.
pub fn top_level_bindings(
    conn: &Connection,
    seat: &SeatId,
) -> Result<Vec<TopLevelBinding>, ApiError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT generation,harness,native_session FROM occupant_bindings WHERE seat_id=?1 \
             AND ended_at IS NULL AND liveness='connected' AND role='top_level' \
             AND harness<>'human' ORDER BY ordinal",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map([seat.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (generation, harness, session) = row.map_err(store_error)?;
        if let (Ok(generation), Ok(native_session)) =
            (u64::try_from(generation), NativeSessionId::parse(session))
        {
            out.push(TopLevelBinding {
                generation,
                harness,
                native_session,
            });
        }
    }
    Ok(out)
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

    fn top_level_bindings(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Vec<TopLevelBinding>, ApiError> {
        let db = self.context().open_query(budget.clone())?;
        top_level_bindings(&db, seat)
    }
}
