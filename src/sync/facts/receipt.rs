//! Fact kinds of the receipt entity: `receipt.settled` (fact-kinds design
//! section 9).
//!
//! One fact per newly settled receipt, authored by the recipient seat's home
//! node. The handler checks that authority first (`AuthorRule::SeatHome`), then
//! calls `receipts::materialize_settlement`, the one materializer of a
//! settlement: the receipt row, the warning-condition close, the local
//! delivery bookkeeping and, on a batch's last receipt of a thread, the derived
//! `ack` info event (`src` holds no SQL here).
//!
//! Fork residual (root section 12, accepted): two branches of the seat's home
//! log (a wipe or restore past the own-history hold's cap) can each settle one
//! receipt with a different `how`. Each node keeps whichever it applied first
//! (the other finds the receipt no longer pending and is a no-op), so stores
//! can differ in `how`-derived attribution only; the receipt is `acked` on
//! every node.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{ReferencedSeats, SettleBatch, SettleHow, message_key, receipt_settle_key};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::{MessageId, SeatId};
use crate::store::materialize::{AuthorRule, require_author};
use crate::store::receipts;
use crate::sync::EntityKey;

/// The `receipt.settled` v1 payload.
///
/// `settled_at` is the author's `decision.utc` in Unix ms: the receipt's
/// `acked_at` on every node. `batch.id` is the authoring command's operation
/// ID (a digest when that exceeds 64 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptSettledPayload {
    pub message: MessageId,
    pub seat: SeatId,
    pub how: SettleHow,
    pub settled_at: i64,
    #[serde(default)]
    pub batch: SettleBatch,
}

impl ReferencedSeats for ReceiptSettledPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

/// The `receipt.settled` v1 kind. It is a held kind (`materialize::HELD_KINDS`):
/// `FactRoute::Journal` refuses it for a local seat during the restore-window
/// authoring hold.
pub struct ReceiptSettled;

impl FactKind for ReceiptSettled {
    const NAME: &'static str = "receipt.settled";
    const MAJOR: u32 = 1;
    type Payload = ReceiptSettledPayload;

    fn creates(payload: &ReceiptSettledPayload) -> Option<EntityKey> {
        receipt_settle_key(&payload.message, &payload.seat).ok()
    }

    fn needs(payload: &ReceiptSettledPayload) -> Vec<EntityKey> {
        message_key(&payload.message).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: ReceiptSettledPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.seat.clone()),
            )?;
            receipts::materialize_settlement(tx, m, &payload)
        })
    }
}
