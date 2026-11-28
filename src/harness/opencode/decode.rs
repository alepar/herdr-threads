//! Pure decode and encode of the opencode plugin's hook payloads (opencode design D3).
//!
//! Output shape, for the plugin author: the hook's stdout is the context text itself, UTF-8,
//! with no envelope. Empty stdout means "inject nothing". Non-empty stdout is added verbatim as
//! one synthetic text part of the triggering user message (`chat.message`), or appended to the
//! bound session for a `compact` re-entry. A non-zero exit or a timeout also means "inject
//! nothing". The bytes are at most 4096.
use super::{OpencodeAdmission, witness};
use crate::harness::{
    Capability,
    adapter::{
        ContractDomain, DecodeFailure, DecodedEvent, DeliveryEligibility, EncodeFailure,
        EncodedOutput, EventIntent, EventMetadata, EventRole, HookInput, NeutralOffer,
        RuntimeAttribution,
    },
    context::EventKind,
    registry::builtins,
};
use serde_json::Value;

/// Hook-injected context is bounded like every other adapter's (`cli::hook::MAX_CONTEXT`).
const MAX_CONTEXT: usize = 4096;

/// The registered events of the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpencodeEvent {
    SessionStart,
    UserPromptSubmit,
    SubagentStart,
}

impl OpencodeEvent {
    pub const ALL: [Self; 3] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::SubagentStart,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SubagentStart => "SubagentStart",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }
}

pub(super) fn decode(
    _: &OpencodeAdmission,
    input: &HookInput,
) -> Result<DecodedEvent, DecodeFailure> {
    let invalid = |detail: &str| DecodeFailure::Invalid(format!("opencode: {detail}"));
    if input.bytes.len() > crate::harness::contract::MAX_PAYLOAD {
        return Err(invalid("payload too large"));
    }
    let payload: Value =
        serde_json::from_slice(&input.bytes).map_err(|_| invalid("payload is not JSON"))?;
    let named = payload
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing event"))?;
    if input
        .registered_event
        .as_deref()
        .is_some_and(|registered| registered != named)
    {
        return Err(invalid("payload event differs from the registered event"));
    }
    let event = OpencodeEvent::parse(named).ok_or_else(|| invalid("unknown event"))?;
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| witness::valid_session_id(id))
        .ok_or_else(|| invalid("missing or invalid session_id"))?;
    // The role is payload-declared from opencode's native `parentID`: a parent makes a child,
    // none (absent, null or empty) a top-level session.
    let parent = match payload.get("parent_session_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(parent)) if parent.is_empty() => None,
        Some(Value::String(parent)) if witness::valid_session_id(parent) && parent != session => {
            Some(parent.as_str())
        }
        Some(_) => return Err(invalid("invalid parent_session_id")),
    };
    let role = if parent.is_some() {
        EventRole::Subagent
    } else {
        EventRole::TopLevel
    };
    let (intent, context_source, skill_pointer) = match event {
        OpencodeEvent::SessionStart => {
            let source = payload
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("missing source"))?;
            let kind = match source {
                "startup" => EventKind::Startup,
                "clear" => EventKind::Clear,
                "resume" => EventKind::Resume,
                "compact" => EventKind::Compact,
                _ => return Err(invalid("unknown source")),
            };
            (EventIntent::Lifecycle(kind), source.to_owned(), true)
        }
        // A turn start checks in like a tool boundary (Current mode).
        OpencodeEvent::UserPromptSubmit => (EventIntent::Current, event.as_str().to_owned(), false),
        OpencodeEvent::SubagentStart => {
            if parent.is_none() {
                return Err(invalid("SubagentStart without parent_session_id"));
            }
            (
                EventIntent::Lifecycle(EventKind::Startup),
                event.as_str().to_owned(),
                false,
            )
        }
    };
    Ok(DecodedEvent {
        harness: builtins()
            .agent("opencode")
            .map_err(|_| invalid("adapter not registered"))?,
        role,
        native_session: Some(session.to_owned()),
        event_id: uuid::Uuid::new_v4().to_string(),
        intent,
        metadata: EventMetadata {
            native_turn_id: None,
            turn_end: false,
            auxiliary_session: false,
            skill_pointer,
            callback_deadline: None,
            context_source,
            capability: Capability::ContractValidatedInput,
            domain: ContractDomain::Native,
            native_event: event.as_str().into(),
            shape_fields: vec![],
            // A child session reaches the CLI through the plugin's session variables, not as a
            // hook-registered subagent binding.
            agent_id: None,
            enrollment_witness: None,
        },
        delivery: DeliveryEligibility::Context,
        runtime: RuntimeAttribution::Unavailable {
            diagnostic: "opencode payload carries no runtime version".into(),
        },
        binding_event: None,
    })
}

pub(super) fn encode(
    _: &OpencodeAdmission,
    event: &DecodedEvent,
    offer: &NeutralOffer,
) -> Result<EncodedOutput, EncodeFailure> {
    let text = &offer.fixed_guidance;
    if text.len() > MAX_CONTEXT {
        return Err(EncodeFailure::Invalid("context exceeds budget".into()));
    }
    let bytes = text.as_bytes().to_vec();
    if matches!(event.delivery, DeliveryEligibility::Context)
        && !matches!(event.intent, EventIntent::Observer)
        && matches!(event.role, EventRole::TopLevel)
    {
        Ok(EncodedOutput::ContextBearing { bytes })
    } else {
        Ok(EncodedOutput::ObserverOnly { bytes })
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/opencode_decode.rs"]
mod tests;
