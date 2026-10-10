//! Pure decode and encode of agy hook payloads and `injectSteps` output.
use super::{AgyAdmission, paths};
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
use serde_json::{Value, json};

/// Hook-injected context is bounded like every other adapter's (`cli::hook::MAX_CONTEXT`).
const MAX_CONTEXT: usize = 4096;
/// One fixed line ahead of a lifecycle context that a `PreInvocation` (not `SessionStart`) carried.
pub const PRE_INVOCATION_LIFECYCLE_PREFIX: &str = "herdr-threads: this conversation is the pane's top-level session; any earlier herdr-threads subagent notice no longer applies.\n";

/// How the context text is injected into the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionStep {
    /// `{"injectSteps":[{"systemMessage":{"systemMessage":"T"}}]}`
    SystemMessage,
    /// `{"injectSteps":[{"userMessage":"T"}]}`
    UserMessage,
}
impl InjectionStep {
    fn render(self, text: &str) -> Value {
        match self {
            Self::SystemMessage => {
                json!({"injectSteps": [{"systemMessage": {"systemMessage": text}}]})
            }
            Self::UserMessage => json!({"injectSteps": [{"userMessage": text}]}),
        }
    }
}

/// Role is always `Unknown` here: the local witnesses (L2b) decide it, so a missing witness is
/// silent by construction.
pub(super) fn decode(_: &AgyAdmission, input: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
    let invalid = |detail: &str| DecodeFailure::Invalid(format!("agy: {detail}"));
    let registered = input
        .registered_event
        .as_deref()
        .ok_or_else(|| invalid("hook registered without --event"))?;
    let event =
        paths::AgyEvent::parse(registered).ok_or_else(|| invalid("unknown registered event"))?;
    let payload: Value =
        serde_json::from_slice(&input.bytes).map_err(|_| invalid("payload is not JSON"))?;
    let conversation = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .filter(|id| paths::valid_conversation_id(id))
        .ok_or_else(|| invalid("missing or invalid conversationId"))?;
    let (intent, delivery, skill_pointer) = match event {
        paths::AgyEvent::SessionStart => (
            EventIntent::Lifecycle(EventKind::Startup),
            DeliveryEligibility::Context,
            true,
        ),
        paths::AgyEvent::PreInvocation | paths::AgyEvent::Stop => (
            EventIntent::Observer,
            DeliveryEligibility::ObserverOnly,
            false,
        ),
    };
    Ok(DecodedEvent {
        harness: builtins()
            .agent("agy")
            .map_err(|_| invalid("adapter not registered"))?,
        role: EventRole::Unknown,
        native_session: Some(conversation.to_owned()),
        event_id: uuid::Uuid::new_v4().to_string(),
        intent,
        metadata: EventMetadata {
            skill_pointer,
            callback_deadline: None,
            context_source: event.as_str().into(),
            capability: Capability::ContractValidatedInput,
            domain: ContractDomain::Native,
            native_event: event.as_str().into(),
            shape_fields: vec![],
        },
        delivery,
        runtime: RuntimeAttribution::Unavailable {
            diagnostic: "agy payload carries no runtime version".into(),
        },
    })
}

pub(super) fn encode(
    admitted: &AgyAdmission,
    event: &DecodedEvent,
    offer: &NeutralOffer,
) -> Result<EncodedOutput, EncodeFailure> {
    let text = &offer.fixed_guidance;
    if text.len() > MAX_CONTEXT {
        return Err(EncodeFailure::Invalid("context exceeds budget".into()));
    }
    let bytes = if text.is_empty() {
        vec![]
    } else {
        let lifecycle_from_turn_start = event.metadata.native_event == "PreInvocation"
            && matches!(event.intent, EventIntent::Lifecycle(_));
        let text = if lifecycle_from_turn_start {
            format!("{PRE_INVOCATION_LIFECYCLE_PREFIX}{text}")
        } else {
            text.clone()
        };
        serde_json::to_vec(&admitted.step.render(&text))
            .map_err(|error| EncodeFailure::Invalid(error.to_string()))?
    };
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
#[path = "../../../tests/harness/agy_decode.rs"]
mod tests;
