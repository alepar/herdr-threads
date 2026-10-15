//! The local delivery tail: the one writer of the delivery-contract rows a
//! message or invitation leaves for local seats.
//!
//! Responsibility
//! - Owns: the `send_attention` work job of an ordinary message
//!   (`work:send:<message>`), the lazy publication write
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
use crate::store::materialize::MaterializeInput;

/// Reason bit for new message or invitation attention.
const REASON_NEW_ATTENTION: i64 = 1;

/// Namespace for the local delivery writes; see the module docs.
pub(crate) struct DeliveryTail;

impl DeliveryTail {
    /// Writes what publishing `message` (sealed preparation `prep`, delivery
    /// `mode`) leaves for its local recipients: for an ordinary message the
    /// `send_attention` job whose high water is `ack_recipients + warnings + 1`
    /// (the job projects receipts and attention, skipping the seats listed in
    /// `send_manifests.suppressed_seats`), for a lazy one the `wake_on: all`
    /// attention writes.
    ///
    /// `_m` is the apply input of the publication; it is reserved
    /// (suppression rides the manifest column), and every caller passes
    /// `Some`.
    pub(crate) fn message(
        tx: &Transaction<'_>,
        _m: Option<&MaterializeInput<'_>>,
        message: &MessageId,
        prep: &str,
        mode: &str,
        ack_recipients: i64,
        warnings: i64,
    ) -> Result<(), ApiError> {
        if mode == "ordinary" {
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
        if mode == "lazy" {
            lazy_delivery::note_published(tx, prep)?;
        }
        Ok(())
    }

    /// The attention a new invitation gives its invitee; written only when the
    /// invitation is `timed` (it has a deadline the wake worker must watch).
    pub(crate) fn invitation(
        tx: &Connection,
        invitee: &SeatId,
        timed: bool,
    ) -> Result<(), ApiError> {
        if timed {
            Self::attention(tx, invitee.as_str(), REASON_NEW_ATTENTION)?;
        }
        Ok(())
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
