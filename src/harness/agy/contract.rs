//! The agy hook payload contract. agy payloads carry no event name, so the registered event
//! (argv) is injected as a synthetic `$event` discriminator before the shared classifier runs.
use crate::harness::{
    adapter::{ContractDescriptor, ContractDomain, ContractObservation, HookInput},
    contract::{
        self, Classification, EventClass, EventContract, HarnessContract, JsonType, Malformed,
        field,
    },
    evidence::{AttributionHolding, EvidenceEvent, EvidenceOrigin},
};
use serde_json::Value;

/// The synthetic discriminator key: protojson camelCase never emits `$`.
pub const DISCRIMINATOR: &str = "$event";

macro_rules! common_fields {
    ($($field:expr),* $(,)?) => {
        &[
            $($field,)*
            field("artifactDirectoryPath", JsonType::String, false),
            field("modelName", JsonType::String, false),
            field("workspacePaths", JsonType::Array, false),
        ]
    };
}
pub static CONTRACT: HarnessContract = HarnessContract {
    harness: "agy",
    discriminator: DISCRIMINATOR,
    events: &[
        EventContract {
            event: "SessionStart",
            class: EventClass::Lifecycle,
            fields: common_fields![
                field("conversationId", JsonType::String, true),
                field("transcriptPath", JsonType::String, true),
            ],
        },
        EventContract {
            event: "PreInvocation",
            class: EventClass::Other,
            fields: common_fields![
                field("conversationId", JsonType::String, true),
                field("invocationNum", JsonType::Number, true),
                field("initialNumSteps", JsonType::Number, true),
                field("transcriptPath", JsonType::String, false),
            ],
        },
        EventContract {
            event: "Stop",
            class: EventClass::Other,
            fields: common_fields![
                field("conversationId", JsonType::String, true),
                field("terminationReason", JsonType::String, true),
                field("error", JsonType::String, false),
                field("fullyIdle", JsonType::Bool, false),
                field("executionNum", JsonType::Number, false),
                field("transcriptPath", JsonType::String, false),
            ],
        },
    ],
};

pub static DESCRIPTORS: [ContractDescriptor; 1] = [ContractDescriptor {
    domain: ContractDomain::Native,
    domain_id: "native_payload",
    origin: EvidenceOrigin::NativePayload,
    events: &[
        EvidenceEvent {
            native_event: "SessionStart",
            milestone: Some("lifecycle"),
            always_send: true,
        },
        EvidenceEvent {
            native_event: "PreInvocation",
            milestone: Some("turn"),
            always_send: false,
        },
        EvidenceEvent {
            native_event: "Stop",
            milestone: None,
            always_send: false,
        },
    ],
    required_milestones: &["lifecycle"],
    qualifications: &[],
    holding: AttributionHolding::Never,
    resumed_unavailable_reason: None,
    contract: &CONTRACT,
}];

/// Classify against [`CONTRACT`]. Non-JSON and over-size input pass through to the shared
/// classifier unchanged. An object that already has a `$event` key is malformed (the key is
/// reserved), and so is one with no registered event to inject.
pub fn classify(input: &HookInput) -> ContractObservation {
    let classification = classify_bytes(input);
    ContractObservation {
        domain: ContractDomain::Native,
        classification,
    }
}

fn classify_bytes(input: &HookInput) -> Classification {
    let registered = input.registered_event.as_deref();
    let passthrough = || contract::classify(&CONTRACT, registered, &input.bytes);
    if input.bytes.len() > contract::MAX_PAYLOAD {
        return passthrough();
    }
    let Ok(Value::Object(mut object)) = serde_json::from_slice::<Value>(&input.bytes) else {
        return passthrough();
    };
    if object.contains_key(DISCRIMINATOR) {
        return Classification::Malformed(Malformed::NotObject);
    }
    let Some(registered) = registered else {
        return Classification::Malformed(Malformed::UnknownEvent);
    };
    object.insert(DISCRIMINATOR.into(), Value::String(registered.into()));
    match serde_json::to_vec(&Value::Object(object)) {
        Ok(bytes) => contract::classify(&CONTRACT, Some(registered), &bytes),
        Err(_) => Classification::Malformed(Malformed::NotJson),
    }
}
