//! The gate-only record of a plain local `launch` onto a herdr-core (TRUST-POLICY H3).
//!
//! Responsibility
//! - Owns: the `local_launch_gates` row of one launch: opened before its terminal exists,
//!   completed with the creation and the allocated seat in one update, the retention of old
//!   rows, and the launch gate's read of it, [`gate_view`].
//! - Hides: the SQL and the JSON form of the recorded `CreatedTarget`.
//! - Must not: decide whether a gate opens (the daemon's gate handler does, on this view and a
//!   same-response `terminal.get`), or stand in for a handoff's bootstrap Send: the caller
//!   reads a bootstrap first.
//!
//! Core and harness-neutral.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    ports::{CreatedTarget, LaunchGateView},
    protocol::{handoff::BootstrapAttempt, ids::SeatId, results::ApiError},
    store::{connection::store_error, peer_requests::GATE_ATTEMPT},
};

/// Rows older than this are dropped when a new gate opens: a shim waits 30 s at most (ms).
pub const RETENTION_MS: i64 = 3_600_000;

/// Records `id` before its terminal exists, so the shim waits rather than being refused as
/// unknown. A repeat changes nothing.
pub fn open(tx: &Transaction<'_>, id: &str, now: i64) -> Result<(), ApiError> {
    tx.execute(
        "DELETE FROM local_launch_gates WHERE opened_at < ?1",
        [now.saturating_sub(RETENTION_MS)],
    )
    .map_err(store_error)?;
    tx.execute(
        "INSERT OR IGNORE INTO local_launch_gates(id, opened_at) VALUES(?1, ?2)",
        params![id, now],
    )
    .map_err(store_error)?;
    Ok(())
}

/// The creation and seat of an open record, in one write: the view releases the gate on this
/// row, so the seat always precedes the gate. False for no open row.
pub fn record(
    tx: &Transaction<'_>,
    id: &str,
    created: &CreatedTarget,
    seat: &SeatId,
) -> Result<bool, ApiError> {
    let text = serde_json::to_string(created)
        .map_err(|e| ApiError::store_corrupt(format!("local launch gate creation: {e}")))?;
    let changed = tx
        .execute(
            "UPDATE local_launch_gates SET created=?2, seat=?3 WHERE id=?1 AND created IS NULL",
            params![id, text, seat.as_str()],
        )
        .map_err(store_error)?;
    Ok(changed == 1)
}

/// The gate's read; `None` for no row. A plain launch has no first message, so a recorded
/// creation is the release. Nothing cancels a record: a launcher that dies leaves the shim to
/// its deadline.
pub fn gate_view(
    db: &Connection,
    id: &str,
    attempt: BootstrapAttempt,
) -> Result<Option<LaunchGateView>, ApiError> {
    let found: Option<Option<String>> = db
        .query_row(
            "SELECT created FROM local_launch_gates WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(created) = found else {
        return Ok(None);
    };
    let created = created
        .map(|text| serde_json::from_str::<CreatedTarget>(&text))
        .transpose()
        .map_err(|e| ApiError::store_corrupt(format!("local launch gate record: {e}")))?;
    Ok(Some(LaunchGateView {
        send_staged: created.is_some(),
        created,
        current_attempt: attempt.get() == GATE_ATTEMPT,
        cancelled: false,
    }))
}
