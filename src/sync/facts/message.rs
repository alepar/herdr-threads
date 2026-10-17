//! Fact kinds of the message entity: `message.sent` (fact-kinds design
//! section 8).
//!
//! `message.sent` carries the complete audience the author fixed, so a
//! receiver never recomputes it. The handler checks the origin's authority
//! first (the author seat's home node), then calls `messages::materialize_message`,
//! the one materializer of a published ordinary message (`src` holds no SQL
//! here).

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{
    ReceiptObligation, RecipientSpec, ReferencedSeats, message_key, seat_key, thread_key,
};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::{MessageId, SeatId, ThreadId};
use crate::protocol::priority::DeliveryPriority;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::summary::{AuthorRole, UserIntent};
use crate::store::materialize::{AuthorRule, require_author};
use crate::store::messages;
use crate::sync::EntityKey;

/// The most `needs` keys a `message.sent` names: the thread, the author, then
/// recipients while the total stays within it (section 3).
pub const NEEDS_LIMIT: usize = 256;

/// A payload over this many bytes is refused before authoring (the journal
/// caps a payload at 512 KiB; section 3 "Payload size").
pub const AUTHOR_PAYLOAD_LIMIT: usize = 448 * 1024;

/// The `message.sent` v1 payload.
///
/// `recipients` is the complete audience the author fixed: sorted by seat ID,
/// the author excluded. A recipient with `receipt: ack` owes a receipt;
/// `deadline_at` is `sent_at + duration` when the author's availability
/// evidence made the receipt timed, else `None`.
///
/// Main's priority model (reconcile amendment §1.1): `priority` is the
/// message's immutable delivery priority (`messages.priority`, required: a
/// defaulted priority would silently change wake behaviour); `require_ack`
/// says the send named explicit ACK recipients (`--require-ack`), which
/// decides the recorded `delivery_mode` (`ordinary` iff true). An `ack`
/// recipient exists only for an explicit ACK recipient that is not a human
/// (D9), so a send whose explicit ACK recipients are all humans has
/// `require_ack` and no `ack` recipient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSentPayload {
    pub message: MessageId,
    pub thread: ThreadId,
    pub author: SeatId,
    pub body: String,
    /// Send-time claim, frozen on the authoring node. Absent on v2.0.0 facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_role: Option<AuthorRole>,
    /// 0..=9, 0 most urgent; checked by [`MessageSentPayload::check`].
    pub priority: u8,
    #[serde(default)]
    pub require_ack: bool,
    #[serde(default)]
    pub relays_user: bool,
    #[serde(default)]
    pub user_intent: Option<UserIntent>,
    /// The author's `decision.utc` in Unix ms; the row's `decision_at`.
    pub sent_at: i64,
    /// The frozen receipt duration in ms (`0` when nobody owes a receipt).
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub recipients: Vec<RecipientSpec>,
}

impl ReferencedSeats for MessageSentPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        std::iter::once(self.author.clone())
            .chain(self.recipients.iter().map(|r| r.seat.clone()))
            .collect()
    }
}

impl MessageSentPayload {
    /// The payload rules every node checks before authority (amendment
    /// §1.1); a failure is `InvalidRequest`, so the entry is `Rejected`:
    /// `priority <= 9`; `require_ack` implies priority 0 (D3); an `ack`
    /// recipient requires `require_ack`; a `none` recipient has no deadline.
    pub fn check(&self) -> Result<(), ApiError> {
        let invalid = |detail: &str| Err(ApiError::new(ErrorCode::InvalidRequest, detail));
        if self.priority > 9 {
            return invalid("message.sent priority out of range");
        }
        if self.require_ack && self.priority != 0 {
            return invalid("message.sent require_ack needs priority 0");
        }
        for recipient in &self.recipients {
            match recipient.receipt {
                ReceiptObligation::Ack if !self.require_ack => {
                    return invalid("message.sent ack recipient without require_ack");
                }
                ReceiptObligation::None if recipient.deadline_at.is_some() => {
                    return invalid("message.sent deadline on a recipient that owes no receipt");
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The checked priority.
    pub fn delivery_priority(&self) -> Result<DeliveryPriority, ApiError> {
        DeliveryPriority::new(self.priority)
            .map_err(|detail| ApiError::new(ErrorCode::InvalidRequest, detail))
    }

    /// The recorded `delivery_mode`: `ordinary` iff the send named explicit
    /// ACK recipients (main's rule).
    pub fn recorded_mode(&self) -> &'static str {
        if self.require_ack { "ordinary" } else { "lazy" }
    }

    /// Refuses a payload the journal could not carry (`InvalidRequest`
    /// "audience too large to replicate"), before anything is authored.
    pub fn validate_for_author(&self) -> Result<(), ApiError> {
        let size = serde_json::to_vec(self)
            .map_err(|e| {
                ApiError::new(
                    ErrorCode::InvalidRequest,
                    format!("fact payload encode: {e}"),
                )
            })?
            .len();
        if size > AUTHOR_PAYLOAD_LIMIT {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "audience too large to replicate",
            ));
        }
        Ok(())
    }
}

/// The `message.sent` v1 kind.
pub struct MessageSent;

impl FactKind for MessageSent {
    const NAME: &'static str = "message.sent";
    const MAJOR: u32 = 1;
    type Payload = MessageSentPayload;

    fn creates(payload: &MessageSentPayload) -> Option<EntityKey> {
        message_key(&payload.message).ok()
    }

    /// The thread, the author, then recipients while the total stays within
    /// [`NEEDS_LIMIT`]. A recipient dropped from `needs` is still ordered
    /// before the message by the author's deps vector; only a parked creator
    /// differs, and there a missing seat row is a replayable `Rejected`.
    fn needs(payload: &MessageSentPayload) -> Vec<EntityKey> {
        let mut keys: Vec<EntityKey> = thread_key(&payload.thread)
            .into_iter()
            .chain(seat_key(&payload.author))
            .collect();
        for recipient in &payload.recipients {
            if keys.len() >= NEEDS_LIMIT {
                break;
            }
            keys.extend(seat_key(&recipient.seat));
        }
        keys
    }

    /// Authority: the origin is the author's home node. Membership of the
    /// author and recipients is not re-checked (accepted limit,
    /// TRUST-POLICY).
    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: MessageSentPayload,
    ) -> Result<(), FactError> {
        payload.check().map_err(FactError::Rejected)?;
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.author.clone()),
            )?;
            messages::materialize_message(tx, m, &payload)
        })
    }
}
