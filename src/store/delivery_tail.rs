//! The local delivery tail: the one writer of the delivery-contract rows a
//! message or invitation leaves for local seats.
//!
//! Responsibility
//! - Owns: the `send_attention` work job of a message with explicit ACK
//!   receipts or warnings (`work:send:<message>`), the delivery-row publication write
//!   (`lazy_delivery::note_published`), the invitation attention write, and
//!   [`DeliveryTail::attention`], the single `wake_work` attention upsert and
//!   the one locality guard (a no-op for a seat with `is_local = 0`).
//! - Does not: decide suppression (the catch-up decision is made before this
//!   runs; a suppressed recipient rides `send_manifests.suppressed_seats`),
//!   project receipts (the job does, for local and replicated sends alike),
//!   or touch the dispatch side of `wake_work` (`wake.rs` reservations and
//!   completions).
//! - New delivery-table writes (a row the wake, lazy or attention kicks key
//!   on) go here; handlers and materializers never write `wake_work` or send
//!   manifests directly, and never for a remote seat.
//! - Role: the seam that makes a replicated fact wake exactly as a local one
//!   does, and never wakes anyone for a seat this node does not host.

use rusqlite::{Connection, Transaction, params};

use crate::protocol::ids::{MessageId, SeatId};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{api_error, store_error};
use crate::store::lazy_delivery;

/// Reason bit for new message or invitation attention.
const REASON_NEW_ATTENTION: i64 = 1;

/// Namespace for the local delivery writes; see the module docs.
pub(crate) struct DeliveryTail;

impl DeliveryTail {
    /// Writes what publishing `message` (sealed preparation `prep`) leaves for
    /// its local recipients (main's D9 model): when the send has explicit ACK
    /// receipts or unavailable-recipient warnings, the `send_attention` job
    /// whose high water is `ack_recipients + warnings + 1` (the job projects
    /// receipts and attention, skipping the seats listed in
    /// `send_manifests.suppressed_seats`); when it staged delivery rows, the
    /// publication of those rows (`lazy_delivery::note_published`: the
    /// publication `decision_seq` and one `wake_work` row per local recipient
    /// whose nudge threshold admits the row's priority). Every row of one
    /// preparation carries the message priority, so the priority is read from
    /// the rows, not passed.
    pub(crate) fn message(
        tx: &Transaction<'_>,
        message: &MessageId,
        prep: &str,
        ack_recipients: i64,
        warnings: i64,
    ) -> Result<(), ApiError> {
        if ack_recipients > 0 || warnings > 0 {
            let high_water = ack_recipients
                .checked_add(warnings)
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| {
                    api_error(ErrorCode::SequenceExhausted, "send work position exhausted")
                })?;
            tx.execute(
                "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'send_attention',?2,?3)",
                params![format!("work:send:{}", message.as_str()), prep, high_water],
            )
            .map_err(store_error)?;
        }
        let has_rows: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM lazy_recipients WHERE preparation_id=?1)",
                [prep],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if has_rows {
            lazy_delivery::note_published(tx, prep)?;
        }
        Ok(())
    }

    /// The attention a new invitation gives its invitee: every invitation is
    /// priority 0 and confirming (main D6), so it always wakes. A no-op for a
    /// remote invitee.
    pub(crate) fn invitation(tx: &Connection, invitee: &SeatId) -> Result<(), ApiError> {
        Self::attention(tx, invitee.as_str(), REASON_NEW_ATTENTION)
    }

    /// The one `wake_work` attention upsert: sets `reason_bits` and advances
    /// `attention_version` so the dispatcher re-derives the seat's reasons.
    /// A no-op for a seat with `is_local = 0`.
    pub(crate) fn attention(tx: &Connection, seat: &str, reason_bits: i64) -> Result<(), ApiError> {
        Self::upsert(tx, seat, reason_bits, false)
    }

    /// [`attention`](Self::attention) that also skips a retired seat.
    pub(crate) fn attention_unless_retired(
        tx: &Connection,
        seat: &str,
        reason_bits: i64,
    ) -> Result<(), ApiError> {
        Self::upsert(tx, seat, reason_bits, true)
    }

    fn upsert(
        tx: &Connection,
        seat: &str,
        reason_bits: i64,
        skip_retired: bool,
    ) -> Result<(), ApiError> {
        let sql = if skip_retired {
            "INSERT INTO wake_work(seat_id,reason_bits,attention_version) SELECT id,?2,1 FROM seats WHERE id=?1 AND is_local=1 AND state!='retired' ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|?2,attention_version=attention_version+1"
        } else {
            "INSERT INTO wake_work(seat_id,reason_bits,attention_version) SELECT id,?2,1 FROM seats WHERE id=?1 AND is_local=1 ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|?2,attention_version=attention_version+1"
        };
        tx.execute(sql, params![seat, reason_bits])
            .map_err(store_error)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/store/delivery_tail.rs"]
mod tests;
