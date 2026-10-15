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

use super::payload::{RecipientSpec, ReferencedSeats, message_key, seat_key, thread_key};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::commands::DeliveryMode;
use crate::protocol::ids::{MessageId, SeatId, ThreadId};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::summary::UserIntent;
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
/// evidence made the receipt timed, else `None`. `require_ack` is implied by
/// any `ack` recipient.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSentPayload {
    pub message: MessageId,
    pub thread: ThreadId,
    pub author: SeatId,
    pub body: String,
    #[serde(default)]
    pub mode: DeliveryMode,
    /// Reserved: no send command sets it yet.
    #[serde(default)]
    pub nudge: bool,
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

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: MessageSentPayload,
    ) -> Result<(), FactError> {
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
