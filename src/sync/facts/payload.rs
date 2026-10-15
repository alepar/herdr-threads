//! Shared payload value types and the v1 `EntityKey` spellings.
//!
//! Responsibility
//! - Owns: [`SeatKind`], [`RecipientSpec`], [`ReceiptObligation`],
//!   [`SettleHow`], [`AnswerOutcome`], [`RegisterField`] and [`SettleBatch`]
//!   (closed enums or plain structs, snake_case, additive fields default, never
//!   `deny_unknown_fields`, times as origin wall-clock Unix ms `i64`); the key
//!   builders [`seat_key`] ... [`answer_key`]; the [`ReferencedSeats`] contract
//!   FactRoute reads to publish the seats a payload names.
//! - Hides: the composite key spellings (`m<id>/s<id>`, `i<id>/answer`), so
//!   ApplyEngine and the suppression query treat keys as opaque.
//! - Must not: carry local positions (`decision_seq`, sequence, episode,
//!   revision, `host_id`) or attribution.
//! - Role: used by the kind modules, by command authoring and by the
//!   materializers; depends only on `EntityKey` and the ID types. New shared
//!   payload types and key spellings go here; callers never format
//!   `creates`/`needs` keys inline.

use serde::{Deserialize, Serialize};

use crate::protocol::ids::{InvitationId, MessageId, SeatId, ThreadId};
use crate::sync::{EntityKey, EnvelopeError};

/// A seat's published kind; it fixes the ACK expectation of remote recipients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatKind {
    Agent,
    Human,
}

/// Whether a recipient owes a receipt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptObligation {
    #[default]
    None,
    Ack,
}

/// One recipient of a `message.sent`, as the author fixed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipientSpec {
    pub seat: SeatId,
    #[serde(default)]
    pub receipt: ReceiptObligation,
    /// Origin wall-clock Unix ms; `None` when the receipt is not timed.
    #[serde(default)]
    pub deadline_at: Option<i64>,
}

/// How a receipt was settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettleHow {
    Acked,
    Displayed,
    ModDelivered,
    Policy,
    Retired,
}

/// The answer to an invitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerOutcome {
    Accepted,
    Rejected,
}

/// Who issued an invitation. A `seat` inviter is a seat of some node and the
/// invitation names it; `service` (a registered service author) and `operator`
/// (an operator command) have no seat, so the fact's authority is the signing
/// origin node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviterKind {
    #[default]
    Seat,
    Service,
    Operator,
}

impl InviterKind {
    /// The `invitations.inviter_kind` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Seat => "seat",
            Self::Service => "service",
            Self::Operator => "operator",
        }
    }
}

/// An invitation's confirmation mode: `lazy` (no deadline, never wakes) or
/// `require_ack`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    Lazy,
    RequireAck,
}

impl Confirmation {
    /// The `invitations.confirmation` spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lazy => "lazy",
            Self::RequireAck => "require_ack",
        }
    }
}

/// The LWW registers of a thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisterField {
    Topic,
    Name,
    Archived,
}

/// The authoring command's batch of settlements (`batch.id` is its operation
/// ID, at most 64 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettleBatch {
    pub id: String,
    /// True on the batch's last settled receipt of each thread.
    pub last_in_thread: bool,
}

/// The seats a payload names; FactRoute publishes every local one before
/// authoring. Each kind payload implements it.
pub trait ReferencedSeats {
    fn referenced_seats(&self) -> Vec<SeatId>;
}

impl ReferencedSeats for RecipientSpec {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

impl ReferencedSeats for [RecipientSpec] {
    fn referenced_seats(&self) -> Vec<SeatId> {
        self.iter().map(|r| r.seat.clone()).collect()
    }
}

/// `s<id>`: a seat (`seat.created` creates it).
pub fn seat_key(seat: &SeatId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(seat.as_str())
}

/// `t<id>` or `d<id>`: a thread (`thread.created` / `thread.direct` creates it).
pub fn thread_key(thread: &ThreadId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(thread.as_str())
}

/// `i<id>`: an invitation (`invitation.created` creates it).
pub fn invitation_key(invitation: &InvitationId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(invitation.as_str())
}

/// `m<id>`: a message (`message.sent` creates it).
pub fn message_key(message: &MessageId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(message.as_str())
}

/// `m<id>/s<id>`: the settlement of one message for one seat
/// (`receipt.settled` creates it; catch-up suppression asks for it).
pub fn receipt_settle_key(message: &MessageId, seat: &SeatId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(format!("{}/{}", message.as_str(), seat.as_str()))
}

/// `i<id>/answer`: the answer to one invitation (`invitation.answered`
/// creates it).
pub fn answer_key(invitation: &InvitationId) -> Result<EntityKey, EnvelopeError> {
    EntityKey::new(format!("{}/answer", invitation.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    fn round<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T, json: &str) {
        assert_eq!(serde_json::to_string(v).unwrap(), json);
        assert_eq!(&serde_json::from_str::<T>(json).unwrap(), v);
    }

    #[test]
    fn value_types_json_round_trip() {
        round(&SeatKind::Agent, "\"agent\"");
        round(&SeatKind::Human, "\"human\"");
        round(&ReceiptObligation::None, "\"none\"");
        round(&ReceiptObligation::Ack, "\"ack\"");
        for (v, s) in [
            (SettleHow::Acked, "acked"),
            (SettleHow::Displayed, "displayed"),
            (SettleHow::ModDelivered, "mod_delivered"),
            (SettleHow::Policy, "policy"),
            (SettleHow::Retired, "retired"),
        ] {
            round(&v, &format!("\"{s}\""));
        }
        round(&InviterKind::Seat, "\"seat\"");
        round(&InviterKind::Service, "\"service\"");
        round(&InviterKind::Operator, "\"operator\"");
        round(&Confirmation::Lazy, "\"lazy\"");
        round(&Confirmation::RequireAck, "\"require_ack\"");
        round(&AnswerOutcome::Accepted, "\"accepted\"");
        round(&AnswerOutcome::Rejected, "\"rejected\"");
        round(&RegisterField::Topic, "\"topic\"");
        round(&RegisterField::Name, "\"name\"");
        round(&RegisterField::Archived, "\"archived\"");
        round(
            &RecipientSpec {
                seat: SeatId::new("sAbc12345678"),
                receipt: ReceiptObligation::Ack,
                deadline_at: Some(1_700_000_000_000),
            },
            r#"{"seat":"sAbc12345678","receipt":"ack","deadline_at":1700000000000}"#,
        );
        round(
            &SettleBatch {
                id: "o1".into(),
                last_in_thread: true,
            },
            r#"{"id":"o1","last_in_thread":true}"#,
        );
    }

    #[test]
    fn unknown_fields_ignored_and_additive_fields_default() {
        let r: RecipientSpec = serde_json::from_str(r#"{"seat":"s1","future":{"x":1}}"#).unwrap();
        assert_eq!(r.receipt, ReceiptObligation::None);
        assert_eq!(r.deadline_at, None);
        let b: SettleBatch = serde_json::from_str(r#"{"id":"o","new":true}"#).unwrap();
        assert_eq!(
            b,
            SettleBatch {
                id: "o".into(),
                last_in_thread: false
            }
        );
        let empty: SettleBatch = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, SettleBatch::default());
        assert!(serde_json::from_str::<SeatKind>("\"robot\"").is_err());
    }

    #[test]
    fn key_builder_spellings() {
        let seat = SeatId::new("sAbc12345678");
        let msg = MessageId::new("mXyz98765432");
        let inv = InvitationId::new("iQrs11112222");
        assert_eq!(seat_key(&seat).unwrap().as_str(), "sAbc12345678");
        assert_eq!(
            thread_key(&ThreadId::new("tAbc12345678")).unwrap().as_str(),
            "tAbc12345678"
        );
        assert_eq!(
            thread_key(&ThreadId::new("dAbc12345678")).unwrap().as_str(),
            "dAbc12345678"
        );
        assert_eq!(invitation_key(&inv).unwrap().as_str(), "iQrs11112222");
        assert_eq!(message_key(&msg).unwrap().as_str(), "mXyz98765432");
        assert_eq!(
            receipt_settle_key(&msg, &seat).unwrap().as_str(),
            "mXyz98765432/sAbc12345678"
        );
        assert_eq!(answer_key(&inv).unwrap().as_str(), "iQrs11112222/answer");
        // An ID outside the key grammar is an error, not a malformed key.
        assert!(seat_key(&SeatId::new("s#bad")).is_err());
    }

    #[test]
    fn referenced_seats_lists_recipients_in_order() {
        let specs = [
            RecipientSpec {
                seat: SeatId::new("sA"),
                receipt: ReceiptObligation::None,
                deadline_at: None,
            },
            RecipientSpec {
                seat: SeatId::new("sB"),
                receipt: ReceiptObligation::Ack,
                deadline_at: Some(5),
            },
        ];
        assert_eq!(
            specs[..].referenced_seats(),
            vec![SeatId::new("sA"), SeatId::new("sB")]
        );
        assert_eq!(specs[1].referenced_seats(), vec![SeatId::new("sB")]);
    }
}
