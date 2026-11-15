use super::{PiAdmission, witness};
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
pub enum PiEvent {
    SessionStart,
    TurnStart,
    SessionEnd,
}

impl PiEvent {
    pub const ALL: [Self; 3] = [Self::SessionStart, Self::TurnStart, Self::SessionEnd];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::TurnStart => "TurnStart",
            Self::SessionEnd => "SessionEnd",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }
}

pub(super) fn decode(_: &PiAdmission, input: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
    let invalid = |detail: &str| DecodeFailure::Invalid(format!("pi: {detail}"));
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
    let event = PiEvent::parse(named).ok_or_else(|| invalid("unknown event"))?;
    let session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|id| witness::valid_session(id))
        .ok_or_else(|| invalid("missing or invalid session_id"))?;
    if payload["role"].as_str() != Some("top_level")
        || payload["mode"].as_str() != Some("tui")
        || payload["pid"]
            .as_u64()
            .filter(|pid| *pid > 0 && *pid <= u64::from(u32::MAX))
            .is_none()
        || payload["cwd"]
            .as_str()
            .filter(|cwd| !cwd.is_empty() && !cwd.chars().any(char::is_control))
            .is_none()
    {
        return Err(invalid("invalid TUI owner claim"));
    }
    let role = EventRole::TopLevel;
    let (intent, context_source, skill_pointer) = match event {
        PiEvent::SessionStart => {
            let source = payload
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("missing source"))?;
            let kind = match source {
                "startup" | "reload" => EventKind::Startup,
                "new" => EventKind::Clear,
                "resume" => EventKind::Resume,
                "compact" | "tree" => EventKind::Compact,
                "fork" => EventKind::Fork,
                _ => return Err(invalid("unknown source")),
            };
            (EventIntent::Lifecycle(kind), source.to_owned(), true)
        }
        // A turn start checks in like a tool boundary (Current mode).
        PiEvent::TurnStart => (EventIntent::Current, event.as_str().to_owned(), false),
        PiEvent::SessionEnd => (EventIntent::Observer, event.as_str().to_owned(), false),
    };
    Ok(DecodedEvent {
        harness: builtins()
            .agent("pi")
            .map_err(|_| invalid("adapter not registered"))?,
        role,
        native_session: Some(session.to_owned()),
        event_id: uuid::Uuid::new_v4().to_string(),
        intent,
        metadata: EventMetadata {
            turn_end: false,
            skill_pointer,
            callback_deadline: None,
            context_source,
            capability: Capability::ContractValidatedInput,
            domain: ContractDomain::Native,
            native_event: event.as_str().into(),
            shape_fields: vec![],
            agent_id: None,
            enrollment_witness: None,
        },
        delivery: DeliveryEligibility::Context,
        runtime: RuntimeAttribution::Unavailable {
            diagnostic: "pi payload carries no runtime version".into(),
        },
        binding_event: (event == PiEvent::SessionEnd).then_some(
            crate::harness::adapter::BindingEventDecode {
                kind: crate::protocol::commands::BindingEventKind::SessionEnd,
                agent_id: None,
                child_session: None,
                claim_session: None,
            },
        ),
    })
}

pub(super) fn encode(
    _: &PiAdmission,
    event: &DecodedEvent,
    offer: &NeutralOffer,
) -> Result<EncodedOutput, EncodeFailure> {
    let text = &offer.fixed_guidance;
    if text.len() > MAX_CONTEXT {
        return Err(EncodeFailure::Invalid("context exceeds budget".into()));
    }
    let _ = event;
    Ok(EncodedOutput::ObserverOnly {
        bytes: text.as_bytes().to_vec(),
    })
}
