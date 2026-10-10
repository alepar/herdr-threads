//! Hook-pull delivery wire (delivery-policy spec DP4): `DeliveryPull` and
//! `DeliveryConfirm` with their replies. Both are ordinary one-shot
//! `Command`s on protocol version 6 behind the not-yet-advertised capability
//! [`super::capabilities::DELIVERY_PULL`]; the daemon answers `Unsupported`
//! until the handlers land.
use crate::protocol::{
    authority::CallerClaim,
    ids::{MessageId, OperationId},
};
use serde::{Deserialize, Serialize};

/// At most this many `marker_prefix` arguments.
pub const MAX_MARKER_PREFIX_ARGS: usize = 8;
/// At most this many bytes across all `marker_prefix` arguments.
pub const MAX_MARKER_PREFIX_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPull {
    pub claim: CallerClaim,
    /// `attention_version` of the last Page/Unchanged reply this conversation saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_attention_version: Option<u64>,
    /// argv the truncation marker's commands start with (`cli::hook::cli_prefix`);
    /// empty = bare program. At most 8 args and 4096 bytes in total, validated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_prefix: Vec<String>,
}
impl DeliveryPull {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.marker_prefix.len() > MAX_MARKER_PREFIX_ARGS
            || self.marker_prefix.iter().map(String::len).sum::<usize>() > MAX_MARKER_PREFIX_BYTES
        {
            return Err("delivery pull marker prefix exceeds 8 args or 4096 bytes");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum DeliveryPullReply {
    Unchanged { attention_version: u64 },
    Page(DeliveryPage),
    Refused { reason: DeliveryRefusalReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPage {
    pub attention_version: u64,
    /// `delivery::frame` output: HEADER first, never starts with '/'.
    pub text: String,
    /// Non-truncated message/lazy ids now in the ledger.
    pub offered: Vec<MessageId>,
    /// Shown cut; never confirmable.
    pub truncated: Vec<MessageId>,
    /// Stopped at a page bound.
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryRefusalReason {
    NoBinding,
    SessionMismatch,
    Held,
    Unresolved,
    NoHookPull,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryConfirm {
    pub claim: CallerClaim,
    /// 1..=`MAX_BATCH_ITEMS` (100), no duplicates.
    pub ids: Vec<MessageId>,
    /// Accountable transaction and idempotent replay key.
    pub operation: OperationId,
}
impl DeliveryConfirm {
    pub fn validate(&self) -> Result<(), &'static str> {
        let mut seen = std::collections::BTreeSet::new();
        if self.ids.is_empty()
            || self.ids.len() > crate::protocol::commands::MAX_BATCH_ITEMS
            || !self.ids.iter().all(|id| seen.insert(id))
        {
            return Err("invalid delivery confirm batch");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryConfirmReport {
    pub results: Vec<DeliveryConfirmItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryConfirmItem {
    pub id: MessageId,
    pub result: DeliveryConfirmOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<DeliveryConfirmReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryConfirmOutcome {
    Settled,
    AlreadySettled,
    NotOffered,
    RefusedTerminal,
    StaleGeneration,
    Retryable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryConfirmReason {
    Unknown,
    NotAddressed,
    Truncated,
    NoHookPull,
    Busy,
}

#[cfg(test)]
#[path = "../../tests/protocol/delivery.rs"]
mod tests;
