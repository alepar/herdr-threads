//! Persisted no-wake reports (`no_wake_reports`, NodeDelta 0012).
//!
//! Responsibility: one row per (observer, seat) per no-wake episode, so a sender is told
//! `wake=no_wake` once across daemon restarts and mod channel flaps. Owns the SQL of the table:
//! insert-once, delete on a `Composer` read, delete on retire of either seat. Does not decide
//! what a no-wake episode is (the daemon's `wake_status` classification does). New reads and
//! writes of this table go here.

use rusqlite::{Connection, params};

use super::{SqliteStore, connection::store_error};
use crate::protocol::{ids::SeatId, results::ApiError, time::CallBudget};

/// Records `seat` as reported to `observer`; true when the row is new (the episode's first
/// report), false when it was already there.
pub fn insert_once(conn: &Connection, observer: &SeatId, seat: &SeatId) -> Result<bool, ApiError> {
    conn.execute(
        "INSERT OR IGNORE INTO no_wake_reports(observer,seat_id) VALUES (?1,?2)",
        params![observer.as_str(), seat.as_str()],
    )
    .map(|changed| changed == 1)
    .map_err(store_error)
}

/// Ends the episode of `seat` for `observer` (a read found the seat's wake `Composer`).
pub fn delete_pair(conn: &Connection, observer: &SeatId, seat: &SeatId) -> Result<(), ApiError> {
    conn.execute(
        "DELETE FROM no_wake_reports WHERE observer=?1 AND seat_id=?2",
        params![observer.as_str(), seat.as_str()],
    )
    .map(|_| ())
    .map_err(store_error)
}

/// Drops every row naming `seat` as observer or as reported seat (it retired).
pub fn delete_for_retired(conn: &Connection, seat: &SeatId) -> Result<(), ApiError> {
    conn.execute(
        "DELETE FROM no_wake_reports WHERE observer=?1 OR seat_id=?1",
        [seat.as_str()],
    )
    .map(|_| ())
    .map_err(store_error)
}

/// The seats of `observer` that have a row.
fn reported(conn: &Connection, observer: &SeatId) -> Result<Vec<String>, ApiError> {
    let mut statement = conn
        .prepare("SELECT seat_id FROM no_wake_reports WHERE observer=?1")
        .map_err(store_error)?;
    let rows = statement
        .query_map([observer.as_str()], |row| row.get::<_, String>(0))
        .map_err(store_error)?;
    rows.collect::<Result<_, _>>().map_err(store_error)
}

/// Applies one `wake_status` read for `observer` inside a write transaction: every seat of
/// `persist` (a seat whose host has no wake path) gets its row, every seat of `clear` (a
/// `Composer` read) loses it. Returns the `persist` seats whose row is new.
pub fn apply(
    conn: &Connection,
    observer: &SeatId,
    persist: &[SeatId],
    clear: &[SeatId],
) -> Result<Vec<SeatId>, ApiError> {
    for seat in clear {
        delete_pair(conn, observer, seat)?;
    }
    let mut fresh = Vec::new();
    for seat in persist {
        if insert_once(conn, observer, seat)? {
            fresh.push(seat.clone());
        }
    }
    Ok(fresh)
}

/// Whether [`apply`] would change anything for `observer`, judged on a read connection, so a
/// read that changes nothing (most do) never takes the writer.
pub fn would_change(
    conn: &Connection,
    observer: &SeatId,
    persist: &[SeatId],
    clear: &[SeatId],
) -> Result<bool, ApiError> {
    let rows = reported(conn, observer)?;
    let has = |seat: &SeatId| rows.iter().any(|row| row == seat.as_str());
    Ok(persist.iter().any(|seat| !has(seat)) || clear.iter().any(has))
}

impl SqliteStore {
    /// `StorePort::no_wake_reports_apply`.
    pub(super) fn no_wake_reports_apply_impl(
        &self,
        observer: &SeatId,
        persist: &[SeatId],
        clear: &[SeatId],
        budget: &CallBudget,
    ) -> Result<Vec<SeatId>, ApiError> {
        {
            let db = self.context().open_query(budget.clone())?;
            if !would_change(&db, observer, persist, clear)? {
                return Ok(Vec::new());
            }
        }
        let mut writer = self.writer(budget)?;
        self.context().execute_decision(
            &mut writer,
            |_| Ok(()),
            |tx, _, ()| apply(tx, observer, persist, clear),
        )
    }
}
