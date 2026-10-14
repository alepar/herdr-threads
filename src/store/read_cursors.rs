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
    // Before the cursors are written, while "no cursor yet" still identifies the threads this call
    // inherits: take the out-of-order display marks, still above the inherited cursor, of the one
    // binding whose cursor is inherited (the seat maximum; ties go to the lowest binding id).
    tx.execute(
        "INSERT OR IGNORE INTO binding_row_displays(binding_id,message_id) \
         SELECT ?1, d.message_id FROM occupant_bindings b \
         JOIN occupant_bindings o ON o.seat_id=b.seat_id AND o.binding_id<>?1 \
         JOIN binding_row_displays d ON d.binding_id=o.binding_id \
         JOIN messages dm ON dm.id=d.message_id \
         JOIN memberships mb ON mb.seat_id=b.seat_id AND mb.thread_id=dm.thread_id AND mb.state='joined' \
         WHERE b.binding_id=?1 \
           AND NOT EXISTS(SELECT 1 FROM binding_cursors own WHERE own.binding_id=?1 AND own.thread_id=dm.thread_id) \
           AND o.binding_id=(SELECT c2.binding_id FROM binding_cursors c2 JOIN occupant_bindings o2 ON o2.binding_id=c2.binding_id \
                WHERE o2.seat_id=b.seat_id AND c2.thread_id=dm.thread_id AND c2.binding_id<>?1 \
                ORDER BY c2.seq DESC, c2.binding_id LIMIT 1) \
           AND dm.sequence>(SELECT c3.seq FROM binding_cursors c3 WHERE c3.binding_id=o.binding_id AND c3.thread_id=dm.thread_id)",
        params![binding.as_str()],
    )
    .map_err(store_error)?;
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
///
/// Lazy messages addressed to the seat past its furthest-read point that no completion has
/// displayed (a direct send's frozen invited target, ht-8y9; a row displayed out of order while a
/// cursor was held is never shown again) stay pending at seat level instead: they take the `legacy_pending`
/// path, unread for every top-level binding until one displays them. A cursor cannot do this,
/// because ordinary messages to an invitee interleave with them and were already offered as
/// pending receipts before the join; they must not be offered again.
pub fn init_for_join(tx: &Connection, seat: &SeatId, thread: &ThreadId) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE lazy_recipients SET legacy_pending=1 WHERE seat_id=?1 AND thread_id=?2 AND legacy_pending=0 AND state='pending' \
         AND message_id IN (SELECT m.id FROM messages m WHERE m.thread_id=?2 AND m.sequence>COALESCE(\
           (SELECT MAX(c.seq) FROM binding_cursors c JOIN occupant_bindings o ON o.binding_id=c.binding_id \
             WHERE o.seat_id=?1 AND c.thread_id=?2),0))",
        params![seat.as_str(), thread.as_str()],
    )
    .map_err(store_error)?;
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

/// Direct invitation acceptance retains every established cursor and seeds only
/// missing open-binding cursors from the captured invitation event sequence.
/// The eligibility transition invalidates each affected binding once even when
/// its cursor value did not change. The owner includes this in its one deciding
/// transaction, so any later binding/counter failure rolls all changes back.
pub(super) fn init_for_direct_accept(
    tx: &Connection,
    seat: &SeatId,
    thread: &ThreadId,
    floor: u64,
) -> Result<usize, ApiError> {
    let floor = to_i64(floor)?;
    let bindings: Vec<BindingId> = tx.prepare(
        "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY binding_id",
    ).map_err(store_error)?.query_map([seat.as_str()], |r| r.get::<_, String>(0))
        .map_err(store_error)?.map(|row| row.map(BindingId::new)).collect::<rusqlite::Result<_>>().map_err(store_error)?;
    for binding in &bindings {
        tx.execute(
            "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,?2,?3) ON CONFLICT(binding_id,thread_id) DO NOTHING",
            params![binding.as_str(), thread.as_str(), floor],
        ).map_err(store_error)?;
        super::presentation_reader::note_binding_reader_change(tx, binding)?;
    }
    Ok(bindings.len())
}

/// Copy the predecessor's cursors to the successor, keeping any higher cursor the successor
/// already has. Returns the number of cursors written.
pub fn copy_lineage(tx: &Connection, from: &BindingId, to: &BindingId) -> Result<usize, ApiError> {
    let copied = tx
        .execute(
            "INSERT INTO binding_cursors(binding_id,thread_id,seq) \
         SELECT ?2, thread_id, seq FROM binding_cursors WHERE binding_id=?1 AND true \
         ON CONFLICT(binding_id,thread_id) DO UPDATE SET seq=MAX(binding_cursors.seq, excluded.seq)",
            params![from.as_str(), to.as_str()],
        )
        .map_err(store_error)?;
    // The successor inherits the predecessor's out-of-order display marks with its cursors.
    tx.execute(
        "INSERT OR IGNORE INTO binding_row_displays(binding_id,message_id) \
         SELECT ?2, message_id FROM binding_row_displays WHERE binding_id=?1",
        params![from.as_str(), to.as_str()],
    )
    .map_err(store_error)?;
    Ok(copied)
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
/// The lazy part is this binding's own lazy-unread predicate (`lazy_delivery::binding_has_unread_lazy`).
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

/// Whether message `x` carries this binding's own display ACK (`?1` seat, `?4` binding): the walk
/// passes such a message without it being displayed again. The single copy, shared by the walk
/// and [`first_ordinary_walk_stop`].
const WALK_DISPLAY_ACKED: &str = "(EXISTS(SELECT 1 FROM receipt_state rs WHERE rs.message_id=x.id AND rs.seat_id=?1 AND rs.state='acked' \
    AND rs.ack_binding_id=?4 AND json_extract(rs.ack_observation,'$.action_provenance') IS NOT NULL) \
    OR EXISTS(SELECT 1 FROM receipts rc WHERE rc.message_id=x.id AND rc.seat_id=?1 AND rc.state='acked' \
    AND rc.ack_binding_id=?4 AND json_extract(rc.ack_observation,'$.action_provenance') IS NOT NULL))";

/// The first ordinary message of `thread` past `binding`'s cursor at which the
/// display walk ([`advance_through_displayed`]) stops when a display shows only lazy rows: a
/// receivable ordinary message the binding has not display-ACKed. `None` when there is none.
/// A display of lazy rows below it can be passed whole by the walk; a lazy row above it cannot
/// (ht-br0: the text accept's thread-scoped page shows only rows below this stop, so its
/// completion never leaves a shown row unread in the binding's own stream). Without a cursor it
/// starts at 0, not at the walk's displayed-episode floor: never higher than the walk's real stop,
/// so the page can only show fewer rows (an accept normally creates the cursor first).
pub fn first_ordinary_walk_stop(
    db: &Connection,
    binding: &BindingId,
    seat: &SeatId,
    thread: &ThreadId,
) -> Result<Option<u64>, ApiError> {
    let after = cursor_for(db, binding, thread)?.unwrap_or(0);
    let stop: Option<i64> = db
        .query_row(
            &format!(
                "SELECT MIN(x.sequence) FROM messages x WHERE x.thread_id=?2 AND x.sequence>?3 \
                 AND EXISTS(SELECT 1 FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} AND m.id=x.id) \
                 AND NOT {WALK_DISPLAY_ACKED}"
            ),
            params![seat.as_str(), thread.as_str(), to_i64(after)?, binding.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    stop.map(|v| {
        u64::try_from(v)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))
    })
    .transpose()
}

/// Advance `binding`'s cursor in `thread` over the longest prefix of messages past the cursor
/// whose sequences are all in `displayed` (or marked displayed by this binding, `binding_row_displays`). Messages the seat cannot receive in that thread (its
/// own sends, kinds the inbox never shows) count as displayed, and so does a message whose seat
/// receipt this same binding display-ACKed: `ack_binding_id` is the binding and the ACK
/// observation carries `action_provenance`, which only display settlements write (text-inbox
/// display, mod or hook-pull delivery). Another binding's ACK and an explicit `ack` do not count
/// (ht-54r F1). Monotonic. Returns the cursor.
// ht-54r.22 adds lazy-mode messages to the receivable set.
pub fn advance_through_displayed(
    tx: &Connection,
    binding: &BindingId,
    thread: &ThreadId,
    displayed: &[u64],
) -> Result<u64, ApiError> {
    advance_through_displayed_with_change(tx, binding, thread, displayed)
        .map(|(reached, _)| reached)
}

/// The display walk's reached sequence and whether its canonical cursor actually changed.
/// Owning operations coalesce the changed bit across their threads before invalidating private
/// presentation state; the low-level cursor helpers never perform that maintenance themselves.
pub(super) fn advance_through_displayed_with_change(
    tx: &Connection,
    binding: &BindingId,
    thread: &ThreadId,
    displayed: &[u64],
) -> Result<(u64, bool), ApiError> {
    let existing = cursor_for(tx, binding, thread)?;
    let Some(seat) = seat_of(tx, binding)? else {
        return Ok((existing.unwrap_or(0), false));
    };
    let current = match existing {
        Some(current) => current,
        None => {
            super::lazy_delivery::displayed_episode_floor(tx, binding, &seat, thread, displayed)?
                .unwrap_or(0)
        }
    };
    let mut stmt = tx
        .prepare(&format!(
            "SELECT x.sequence, EXISTS(SELECT 1 FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} AND m.id=x.id) \
             OR EXISTS(SELECT 1 FROM lazy_recipients lr WHERE lr.message_id=x.id AND lr.seat_id=?1), \
             {WALK_DISPLAY_ACKED}, \
             EXISTS(SELECT 1 FROM binding_row_displays d WHERE d.binding_id=?4 AND d.message_id=x.id) \
             FROM messages x WHERE x.thread_id=?2 AND x.sequence>?3 ORDER BY x.sequence"
        ))
        .map_err(store_error)?;
    let rows = stmt
        .query_map(
            params![
                seat.as_str(),
                thread.as_str(),
                to_i64(current)?,
                binding.as_str()
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            },
        )
        .map_err(store_error)?;
    let mut reached = current;
    for row in rows {
        let (sequence, receivable, display_acked, marked) = row.map_err(store_error)?;
        let sequence = u64::try_from(sequence)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?;
        // A row this binding already displayed out of order (its mark) is passed like one shown now.
        if receivable && !display_acked && !marked && !displayed.contains(&sequence) {
            break;
        }
        reached = sequence;
    }
    drop(stmt);
    let changed = reached > current && advance(tx, binding, thread, reached)?;
    mark_displayed_above(tx, binding, &seat, thread, displayed, reached)?;
    Ok((reached, changed))
}

/// Per-binding displayed marks (`binding_row_displays`; the unread predicates exclude them): a
/// delivery row this binding displayed that stays above its cursor, because a lower-sequence row
/// of a higher priority number is still undisplayed, is marked so it is not offered again while
/// the cursor catches up. The marks the cursor now passes are pruned. One function for both the
/// top-level completion and the cursor-only `AdvanceCursors` path (subagent streams, body reads).
fn mark_displayed_above(
    tx: &Connection,
    binding: &BindingId,
    seat: &SeatId,
    thread: &ThreadId,
    displayed: &[u64],
    reached: u64,
) -> Result<(), ApiError> {
    for &sequence in displayed.iter().filter(|s| **s > reached) {
        tx.execute(
            "INSERT OR IGNORE INTO binding_row_displays(binding_id,message_id) \
             SELECT ?1, x.id FROM messages x WHERE x.thread_id=?2 AND x.sequence=?3 \
             AND EXISTS(SELECT 1 FROM lazy_recipients lr WHERE lr.message_id=x.id AND lr.seat_id=?4)",
            params![
                binding.as_str(),
                thread.as_str(),
                to_i64(sequence)?,
                seat.as_str()
            ],
        )
        .map_err(store_error)?;
    }
    tx.execute(
        "DELETE FROM binding_row_displays WHERE binding_id=?1 AND message_id IN \
         (SELECT x.id FROM messages x WHERE x.thread_id=?2 AND x.sequence<=?3)",
        params![binding.as_str(), thread.as_str(), to_i64(reached)?],
    )
    .map_err(store_error)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/store/read_cursors.rs"]
mod tests;
