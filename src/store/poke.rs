//! Soft-deadline poke candidates and bookkeeping (spec §10).
//!
//! A receipt is due for a poke when `soft_point <= now < effective_deadline`,
//! where the soft point is evaluated lazily on every call from the receipt's
//! current effective deadline (so an extension moves it with no re-arm event)
//! and its frozen window. Candidates come only from seats a reservation could
//! accept (`wake::reservable_seat_sql`): never a person's seat, an unresolved
//! or targetless one, or one under a recovery hold. Candidates come only from
//! physical rows (`receipts` and `receipt_state`), so `mark_soft_poked` always updates the row the
//! candidate came from; `soft_poked_at` is read through the effective receipt
//! projection. A receipt whose seat has an active catch-up row on its thread
//! is not a candidate until the row ends.
use super::{
    connection::{api_error, store_error},
    effective::{self, EffectiveReceipt, EffectiveReceiptState, ReceiptSource, SoftPoked},
    receipts, wake,
};
use crate::{
    notification::policy::soft_point,
    ports::{PokeDue, PokeReceipt, PokeSource},
    protocol::{
        ids::{MessageId, SeatId, ThreadId},
        results::{ApiError, ErrorCode},
        summary::SummarySettings,
        time::UtcMillis,
    },
};
use rusqlite::{Connection, Transaction, params};
use std::collections::{BTreeMap, BTreeSet};

/// Rows examined per physical source per call. Bounded work per tick; rows
/// that are not yet due are excluded in SQL by the (conservative) frozen soft
/// point, which an extension can only move later.
const SCAN_ROWS: i64 = 512;

type EffectiveDeadline<'a> = &'a dyn Fn(&EffectiveReceipt) -> Result<Option<i64>, ApiError>;

/// Seats with due pokes, soonest effective deadline first, at most `limit`.
/// `limit` truncates after the bounded scan (`SCAN_ROWS` per source), so a
/// larger limit adds no store work; the scheduler applies its in-memory
/// admission before its own per-tick cap.
pub fn due_pokes(
    conn: &Connection,
    now: i64,
    settings: &SummarySettings,
    limit: u16,
) -> Result<Vec<PokeDue>, ApiError> {
    due_pokes_with(conn, now, settings, limit, &|receipt| {
        receipts::effective_deadline(conn, receipt)
    })
}

/// [`due_pokes`] with the effective deadline supplied by the caller: the seam
/// tests use to move a deadline independently of the catch-up lifecycle.
pub fn due_pokes_with(
    conn: &Connection,
    now: i64,
    settings: &SummarySettings,
    limit: u16,
    effective: EffectiveDeadline<'_>,
) -> Result<Vec<PokeDue>, ApiError> {
    collect(conn, now, settings, None, limit, effective)
}

/// The seat's due pokes, if any.
pub fn due_pokes_for_seat(
    conn: &Connection,
    seat: &SeatId,
    now: i64,
    settings: &SummarySettings,
) -> Result<Option<PokeDue>, ApiError> {
    Ok(collect(conn, now, settings, Some(seat), 1, &|receipt| {
        receipts::effective_deadline(conn, receipt)
    })?
    .pop())
}

fn collect(
    conn: &Connection,
    now: i64,
    settings: &SummarySettings,
    seat: Option<&SeatId>,
    limit: u16,
    effective: EffectiveDeadline<'_>,
) -> Result<Vec<PokeDue>, ApiError> {
    let seat_filter = seat.map(SeatId::as_str);
    let mut keys = BTreeSet::new();
    for table in ["receipt_state", "receipts"] {
        let reservable = wake::reservable_seat_sql(&format!("{table}.seat_id"));
        let mut statement = conn
            .prepare_cached(&format!(
                "SELECT message_id, seat_id FROM {table} \
                 WHERE state='pending' AND warning_message_id IS NULL \
                 AND deadline_at IS NOT NULL AND available_at IS NOT NULL \
                 AND soft_poked_at IS NULL AND (?3 IS NULL OR seat_id=?3) AND {reservable} \
                 AND deadline_at - (1.0-?1)*(deadline_at-available_at) - 1.0 <= ?2 \
                 ORDER BY deadline_at, message_id, seat_id LIMIT {SCAN_ROWS}"
            ))
            .map_err(store_error)?;
        let rows = statement
            .query_map(params![settings.soft_fraction, now, seat_filter], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(store_error)?;
        for row in rows {
            keys.insert(row.map_err(store_error)?);
        }
    }
    let mut by_seat: BTreeMap<String, Vec<PokeReceipt>> = BTreeMap::new();
    for (message, seat_id) in keys {
        let Some(receipt) = effective::effective_receipt(conn, &message, &seat_id)? else {
            continue;
        };
        if receipt.state != EffectiveReceiptState::Pending || receipt.warning_message_id.is_some() {
            continue;
        }
        // One physical row owns the mark: the effective receipt's own source.
        if effective::receipt_soft_poked_at(conn, &message, &seat_id, receipt.source)?
            != SoftPoked::Unpoked
        {
            continue;
        }
        let (Some(deadline_at), Some(available_at)) = (receipt.deadline_at, receipt.available_at)
        else {
            continue;
        };
        let Some(effective_deadline) = effective(&receipt)? else {
            continue;
        };
        let soft = soft_point(
            effective_deadline,
            deadline_at.saturating_sub(available_at),
            settings.soft_fraction,
        );
        // Past the effective deadline the hard warning owns the receipt.
        if !(soft <= now && now < effective_deadline) {
            continue;
        }
        let active_catch_up: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM catch_up WHERE seat_id=?1 AND thread_id=?2 AND state='active')",
                params![seat_id, receipt.thread_id],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if active_catch_up {
            continue;
        }
        let corrupt = |_| api_error(ErrorCode::StoreCorrupt, "invalid poke receipt identity");
        by_seat
            .entry(seat_id.clone())
            .or_default()
            .push(PokeReceipt {
                message: MessageId::parse(message).map_err(corrupt)?,
                seat: SeatId::parse(seat_id).map_err(corrupt)?,
                thread: ThreadId::parse(receipt.thread_id.clone()).map_err(corrupt)?,
                source: match receipt.source {
                    ReceiptSource::Manifest => PokeSource::ReceiptState,
                    ReceiptSource::Physical => PokeSource::Receipts,
                },
                effective_deadline,
            });
    }
    let mut due: Vec<PokeDue> = by_seat
        .into_values()
        .map(|mut receipts| {
            receipts.sort_by(|a, b| {
                (a.effective_deadline, a.message.as_str())
                    .cmp(&(b.effective_deadline, b.message.as_str()))
            });
            PokeDue {
                seat: receipts[0].seat.clone(),
                receipts,
            }
        })
        .collect();
    due.sort_by(|a, b| {
        (a.receipts[0].effective_deadline, a.seat.as_str())
            .cmp(&(b.receipts[0].effective_deadline, b.seat.as_str()))
    });
    due.truncate(usize::from(limit));
    Ok(due)
}

/// Sets `soft_poked_at` on exactly `receipts`, each on the row it came from,
/// once: an already-poked row keeps its first mark. Returns rows changed.
pub fn mark_soft_poked(
    tx: &Transaction<'_>,
    receipts: &[PokeReceipt],
    now: UtcMillis,
) -> Result<usize, ApiError> {
    let mut changed = 0;
    for receipt in receipts {
        let sql = match receipt.source {
            PokeSource::ReceiptState => {
                "UPDATE receipt_state SET soft_poked_at=?1 WHERE message_id=?2 AND seat_id=?3 AND soft_poked_at IS NULL"
            }
            PokeSource::Receipts => {
                "UPDATE receipts SET soft_poked_at=?1 WHERE message_id=?2 AND seat_id=?3 AND soft_poked_at IS NULL"
            }
        };
        changed += tx
            .execute(
                sql,
                params![now.0, receipt.message.as_str(), receipt.seat.as_str()],
            )
            .map_err(store_error)?;
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "../../tests/store/poke.rs"]
mod tests;
