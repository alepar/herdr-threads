//! Per-binding read cursors (spec §2 Cursors). `seq` is the last delivered
//! `messages.sequence` of the thread for that binding. Contract module (ht-54r.1); ht-54r.7 owns
//! the inbox semantics and `seed` precedence, ht-54r.22 makes `has_unread` lazy-aware.
use crate::{
    protocol::{
        ids::{BindingId, SeatId, ThreadId},
        results::{ApiError, ErrorCode},
    },
    store::connection::{api_error, store_error},
};
use rusqlite::{Connection, OptionalExtension, params};

/// How a new or re-adopted binding's cursors start. Precedence when several apply (ht-54r.7):
/// re-adoption > lineage copy > seat maximum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedMode {
    /// Seat maximum per joined thread, the thread head when the seat has none (decision 5).
    Fresh,
    /// Copy the predecessor's cursors (spec §1 Lineage), then fill missing threads as Fresh.
    Lineage(BindingId),
    /// A migrated or reopened binding keeps its cursors; only missing joined threads are filled.
    Readopt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedOutcome {
    pub first_contact: bool,
    pub threads: usize,
}

fn to_i64(seq: u64) -> Result<i64, ApiError> {
    i64::try_from(seq).map_err(|_| api_error(ErrorCode::InvalidRequest, "cursor out of range"))
}

/// Start the cursors of `binding` according to `mode`. `first_contact` is true only for a
/// fresh binding with no predecessor to inherit from.
pub fn seed(tx: &Connection, binding: &BindingId, mode: SeedMode) -> Result<SeedOutcome, ApiError> {
    let first_contact = matches!(mode, SeedMode::Fresh);
    let mut threads = 0;
    if let SeedMode::Lineage(from) = &mode {
        threads += copy_lineage(tx, from, binding)?;
    }
    threads += init_for_new_binding(tx, binding)?;
    Ok(SeedOutcome {
        first_contact,
        threads,
    })
}

pub fn cursor_for(
    db: &Connection,
    binding: &BindingId,
    thread: &ThreadId,
) -> Result<Option<u64>, ApiError> {
    let seq: Option<i64> = db
        .query_row(
            "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
            params![binding.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    seq.map(|v| {
        u64::try_from(v).map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative read cursor"))
    })
    .transpose()
}

/// Monotonic: never moves a cursor backwards. Returns whether it moved.
pub fn advance(
    tx: &Connection,
    binding: &BindingId,
    thread: &ThreadId,
    seq: u64,
) -> Result<bool, ApiError> {
    let changed = tx
        .execute(
            "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,?2,?3) \
             ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=excluded.seq \
             WHERE excluded.seq>binding_cursors.seq",
            params![binding.as_str(), thread.as_str(), to_i64(seq)?],
        )
        .map_err(store_error)?;
    Ok(changed > 0)
}

/// Give `binding` a cursor for every thread its seat has joined and it lacks: the seat's
/// highest existing cursor for the thread, else the thread head, else 0. Existing cursors of the
/// binding are kept. Returns the number of cursors written.
pub fn init_for_new_binding(tx: &Connection, binding: &BindingId) -> Result<usize, ApiError> {
    tx.execute(
        "INSERT OR IGNORE INTO binding_cursors(binding_id,thread_id,seq) \
         SELECT ?1, m.thread_id, COALESCE(\
             (SELECT MAX(c.seq) FROM binding_cursors c JOIN occupant_bindings o ON o.binding_id=c.binding_id \
               WHERE o.seat_id=b.seat_id AND c.thread_id=m.thread_id AND c.binding_id<>?1),\
             (SELECT MAX(x.sequence) FROM messages x WHERE x.thread_id=m.thread_id),0) \
         FROM occupant_bindings b JOIN memberships m ON m.seat_id=b.seat_id AND m.state='joined' \
         WHERE b.binding_id=?1",
        params![binding.as_str()],
    )
    .map_err(store_error)
}

/// A seat joined `thread`: every open binding of the seat gets a cursor at the join point (the
/// thread head), keeping a higher cursor it already has. Returns the number of bindings touched.
pub fn init_for_join(tx: &Connection, seat: &SeatId, thread: &ThreadId) -> Result<usize, ApiError> {
    let head: Option<i64> = tx
        .query_row(
            "SELECT MAX(sequence) FROM messages WHERE thread_id=?1",
            params![thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    tx.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) \
         SELECT binding_id, ?2, ?3 FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND true \
         ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=MAX(binding_cursors.seq, excluded.seq)",
        params![seat.as_str(), thread.as_str(), head.unwrap_or(0)],
    )
    .map_err(store_error)
}

/// Copy the predecessor's cursors to the successor, keeping any higher cursor the successor
/// already has. Returns the number of cursors written.
pub fn copy_lineage(tx: &Connection, from: &BindingId, to: &BindingId) -> Result<usize, ApiError> {
    tx.execute(
        "INSERT INTO binding_cursors(binding_id,thread_id,seq) \
         SELECT ?2, thread_id, seq FROM binding_cursors WHERE binding_id=?1 AND true \
         ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=MAX(binding_cursors.seq, excluded.seq)",
        params![from.as_str(), to.as_str()],
    )
    .map_err(store_error)
}

/// Contract stub (graph pass): always false until ht-54r.7 implements it.
pub fn has_unread(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    let _ = (db, binding);
    Ok(false)
}

#[cfg(test)]
#[path = "../../tests/store/read_cursors.rs"]
mod tests;
