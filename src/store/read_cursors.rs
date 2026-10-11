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

/// The highest `messages.sequence` of `thread` (0 when it has none): the head a cursor jumps to.
pub fn thread_head(db: &Connection, thread: &ThreadId) -> Result<u64, ApiError> {
    let head: Option<i64> = db
        .query_row(
            "SELECT MAX(sequence) FROM messages WHERE thread_id=?1",
            params![thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    u64::try_from(head.unwrap_or(0))
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))
}

/// Where a binding enrolled now would start in `thread`: the highest cursor any open or closed
/// binding of `seat` holds there, else the thread head (the rule of [`init_for_new_binding`], for
/// a read that must not enroll).
pub fn seed_preview(db: &Connection, seat: &SeatId, thread: &ThreadId) -> Result<u64, ApiError> {
    let max: Option<i64> = db
        .query_row(
            "SELECT MAX(c.seq) FROM binding_cursors c JOIN occupant_bindings o ON o.binding_id=c.binding_id \
             WHERE o.seat_id=?1 AND c.thread_id=?2",
            params![seat.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    match max {
        Some(seq) => u64::try_from(seq)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative read cursor")),
        None => thread_head(db, thread),
    }
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

/// The rows a seat's ordinary inbox delivers, as one query fragment (the single copy; the unread
/// walk, `has_unread` and `advance_through_displayed` all use it). Ordinary-mode messages reach a
/// seat through `prepared_recipients` (a message the seat sent itself, or any other kind, never
/// does). Bind `?1` = the seat. Aliases: `pr` recipient row, `sm` manifest (published), `m` message.
pub const RECEIVABLE_FROM: &str = "prepared_recipients pr \
    JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id \
    JOIN messages m ON m.id=sm.message_id";
pub const RECEIVABLE_WHERE: &str =
    "pr.seat_id=?1 AND m.kind='ordinary' AND m.delivery_mode='ordinary'";
/// Appended to `RECEIVABLE_WHERE`: unread by a binding, i.e. in a thread the seat has joined, past the binding's cursor
/// (a missing cursor row counts as 0). Bind `?2` = the binding.
pub const UNREAD_EXTRA: &str = "AND EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=m.thread_id AND mb.seat_id=pr.seat_id AND mb.state='joined') \
    AND m.sequence > COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=?2 AND c.thread_id=m.thread_id),0)";

pub(crate) fn seat_of(db: &Connection, binding: &BindingId) -> Result<Option<SeatId>, ApiError> {
    db.query_row(
        "SELECT seat_id FROM occupant_bindings WHERE binding_id=?1",
        [binding.as_str()],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(store_error)
    .map(|seat| seat.map(SeatId::new))
}

/// Whether `binding` has unread mail (spec §2 Inbox per binding). Serves wake and the mod channel.
/// The ordinary part: receivable messages in joined threads past the binding's cursor.
/// The lazy part is the seat's pending lazy rows (today's rule); ht-54r.22 replaces it with
/// `lazy_delivery::binding_lazy_unread`, keeping this ordinary side.
pub fn has_unread(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    let Some(seat) = seat_of(db, binding)? else {
        return Ok(false);
    };
    let ordinary: bool = db
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} {UNREAD_EXTRA})"),
            params![seat.as_str(), binding.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if ordinary {
        return Ok(true);
    }
    super::lazy_delivery::binding_has_unread_lazy(db, binding)
}

/// A change detector for the binding's unread ordinary mail (the mod channel fingerprint): the
/// count of unread receivable messages (at most `cap`, the oldest) and the newest of their
/// rowids. It moves when the binding's cursor advances or a message arrives for it, and never
/// when another binding's cursor moves.
pub fn binding_unread_stats(
    db: &Connection,
    binding: &BindingId,
    cap: i64,
) -> Result<(u64, i64), ApiError> {
    let Some(seat) = seat_of(db, binding)? else {
        return Ok((0, 0));
    };
    let (count, newest): (i64, i64) = db
        .query_row(
            &format!("SELECT count(*),COALESCE(max(rid),0) FROM (SELECT m.rowid AS rid FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} {UNREAD_EXTRA} ORDER BY m.rowid LIMIT ?3)"),
            params![seat.as_str(), binding.as_str(), cap],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((u64::try_from(count).unwrap_or(0), newest))
}

/// Advance `binding`'s cursor in `thread` over the longest prefix of messages past the cursor
/// whose sequences are all in `displayed`. Messages the seat cannot receive in that thread (its
/// own sends, kinds the inbox never shows) count as displayed. Monotonic. Returns the cursor.
// ht-54r.22 adds lazy-mode messages to the receivable set.
pub fn advance_through_displayed(
    tx: &Connection,
    binding: &BindingId,
    thread: &ThreadId,
    displayed: &[u64],
) -> Result<u64, ApiError> {
    let current = cursor_for(tx, binding, thread)?.unwrap_or(0);
    let Some(seat) = seat_of(tx, binding)? else {
        return Ok(current);
    };
    let mut stmt = tx
        .prepare(&format!(
            "SELECT x.sequence, EXISTS(SELECT 1 FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} AND m.id=x.id) \
             OR EXISTS(SELECT 1 FROM lazy_recipients lr WHERE lr.message_id=x.id AND lr.seat_id=?1) \
             FROM messages x WHERE x.thread_id=?2 AND x.sequence>?3 ORDER BY x.sequence"
        ))
        .map_err(store_error)?;
    let rows = stmt
        .query_map(
            params![seat.as_str(), thread.as_str(), to_i64(current)?],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, bool>(1)?)),
        )
        .map_err(store_error)?;
    let mut reached = current;
    for row in rows {
        let (sequence, receivable) = row.map_err(store_error)?;
        let sequence = u64::try_from(sequence)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?;
        if receivable && !displayed.contains(&sequence) {
            break;
        }
        reached = sequence;
    }
    drop(stmt);
    if reached > current {
        advance(tx, binding, thread, reached)?;
    }
    Ok(reached)
}

#[cfg(test)]
#[path = "../../tests/store/read_cursors.rs"]
mod tests;
