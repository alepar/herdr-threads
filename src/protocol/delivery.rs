//! Hook-pull delivery wire (delivery-policy spec DP4): `DeliveryPull` and
//! `DeliveryConfirm` with their replies. Both are ordinary one-shot
//! `Command`s on protocol version 6 behind the capability
//! [`super::capabilities::DELIVERY_PULL`] (`delivery.pull_v1`), which the daemon
//! advertises and serves.
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
    /// Accountable transaction key. A replay after the ids settled answers `not_offered`
    /// (the process-local offer ledger no longer holds them).
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

/// Fixed presentation streams; callers cannot create durable namespaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationConsumer {
    Hook,
    Watch,
}

impl PresentationConsumer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Watch => "watch",
        }
    }
}

/// A bounded opaque page/owner token, never a receipt or attention version.
pub fn valid_presentation_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPullV2 {
    pub render: PresentationRenderRequest,
    pub claim: CallerClaim,
    pub consumer: PresentationConsumer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_lease: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_prefix: Vec<String>,
}
impl DeliveryPullV2 {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_presentation_scope(self.consumer, self.presentation_lease.as_deref())?;
        if !matches!(
            (self.consumer, &self.render),
            (
                PresentationConsumer::Hook,
                PresentationRenderRequest::Hook { .. }
            ) | (
                PresentationConsumer::Watch,
                PresentationRenderRequest::WatchJsonLinesV1
            )
        ) {
            return Err("presentation render consumer mismatch");
        }
        DeliveryPull {
            claim: self.claim.clone(),
            seen_attention_version: None,
            marker_prefix: self.marker_prefix.clone(),
        }
        .validate()
    }
}

fn validate_presentation_scope(
    consumer: PresentationConsumer,
    lease: Option<&str>,
) -> Result<(), &'static str> {
    match (consumer, lease) {
        (PresentationConsumer::Hook, None) => Ok(()),
        (PresentationConsumer::Watch, Some(v)) if valid_presentation_token(v) => Ok(()),
        _ => Err("invalid presentation consumer lease"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPresentationConfirmV2 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitted_controls: Vec<PresentationControlStampV2>,
    pub claim: CallerClaim,
    pub consumer: PresentationConsumer,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_lease: Option<String>,
}
impl DeliveryPresentationConfirmV2 {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_presentation_scope(self.consumer, self.presentation_lease.as_deref())?;
        if !valid_presentation_token(&self.token) {
            return Err("invalid presentation token");
        }
        validate_control_stamps(&self.emitted_controls)?;
        Ok(())
    }
}

pub fn validate_control_stamps(stamps: &[PresentationControlStampV2]) -> Result<(), &'static str> {
    if stamps.len() > 32 {
        return Err("presentation control bound");
    }
    for (index, stamp) in stamps.iter().enumerate() {
        let epoch = match stamp {
            PresentationControlStampV2::Attention { epoch, .. }
            | PresentationControlStampV2::Cleared { epoch, .. } => epoch,
        };
        if !valid_presentation_epoch(epoch) {
            return Err("invalid presentation control epoch");
        }
        if stamps[..index].contains(stamp) {
            return Err("duplicate presentation control");
        }
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPresentationConfirmationReplyV2 {
    pub status: DeliveryPresentationConfirmationV2,
    pub acknowledged_controls: Vec<PresentationControlStampV2>,
}
impl DeliveryPresentationConfirmationReplyV2 {
    pub fn empty(status: DeliveryPresentationConfirmationV2) -> Self {
        Self {
            status,
            acknowledged_controls: Vec::new(),
        }
    }
    pub fn validate_for(
        &self,
        submitted: &[PresentationControlStampV2],
    ) -> Result<(), &'static str> {
        validate_control_stamps(submitted)?;
        validate_control_stamps(&self.acknowledged_controls)?;
        if self
            .acknowledged_controls
            .iter()
            .any(|stamp| !submitted.contains(stamp))
        {
            return Err("foreign acknowledged presentation control");
        }
        if matches!(
            self.status,
            DeliveryPresentationConfirmationV2::StaleBinding
                | DeliveryPresentationConfirmationV2::NotCurrentPage
        ) && !self.acknowledged_controls.is_empty()
        {
            return Err("stale confirmation acknowledges controls");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPresentationConfirmationV2 {
    Confirmed,
    AlreadyConfirmed,
    StaleBinding,
    NotCurrentPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum DeliveryPullReplyV2 {
    Unchanged { attention_version: u64 },
    Page(DeliveryPageV2),
    Refused { reason: DeliveryRefusalReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPageV2 {
    pub attention_version: u64,
    pub text: String,
    pub offered: Vec<MessageId>,
    pub truncated: Vec<MessageId>,
    pub more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_token: Option<String>,
    pub render: PresentationRenderSelection,
    pub output_bytes: u32,
    pub items: Vec<PresentationItemV2>,
    pub emitted_controls: Vec<PresentationControlStampV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationInvitation {
    pub id: crate::protocol::ids::InvitationId,
    pub content: crate::protocol::watch::WatchInvitation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationMessage {
    pub id: MessageId,
    pub lazy: bool,
    pub content: crate::protocol::watch::WatchMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationRenderRequest {
    Hook {
        confirmable_selector: u8,
        passive_selector: u8,
    },
    WatchJsonLinesV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationRenderSelection {
    Hook { selector: u8 },
    WatchJsonLinesV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationItemV2 {
    Message {
        id: MessageId,
        lazy: bool,
        content: crate::protocol::watch::WatchMessage,
    },
    Invitation {
        id: crate::protocol::ids::InvitationId,
        content: crate::protocol::watch::WatchInvitation,
    },
    Notice {
        warning: MessageId,
        thread: crate::protocol::ids::ThreadId,
        body: String,
    },
    Attention {
        epoch: String,
        attention_version: u64,
        text: String,
    },
    AttentionCleared {
        epoch: String,
        attention_version: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationControlStampV2 {
    Attention { epoch: String, version: u64 },
    Cleared { epoch: String, version: u64 },
}
/// Canonical decimal integers remain exact across JavaScript consumers.
pub fn valid_presentation_epoch(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}
