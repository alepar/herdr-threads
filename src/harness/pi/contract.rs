use crate::harness::{
    adapter::{ContractDescriptor, ContractDomain, ContractObservation, HookInput},
    contract::{self, EventClass, EventContract, HarnessContract, JsonType, field},
    evidence::{AttributionHolding, EvidenceEvent, EvidenceOrigin},
};

pub const DISCRIMINATOR: &str = "event";

macro_rules! common_fields {
    ($($field:expr),* $(,)?) => {
        &[
            field("event", JsonType::String, true),
            field("session_id", JsonType::String, true),
            field("cwd", JsonType::String, true),
            field("role", JsonType::String, true),
            field("mode", JsonType::String, true),
            field("pid", JsonType::Number, true),
            $($field,)*
        ]
    };
}

pub static CONTRACT: HarnessContract = HarnessContract {
    harness: "pi",
    discriminator: DISCRIMINATOR,
    events: &[
        EventContract {
            event: "SessionStart",
            class: EventClass::Lifecycle,
            fields: common_fields![field("source", JsonType::String, true)],
        },
        EventContract {
            event: "TurnStart",
            class: EventClass::Tool,
            fields: common_fields![],
        },
        EventContract {
            event: "TurnEnd",
            class: EventClass::Other,
            fields: common_fields![
                field("source", JsonType::String, true),
                field("idle", JsonType::Bool, true)
            ],
        },
        EventContract {
            event: "SessionEnd",
            class: EventClass::Other,
            fields: common_fields![],
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
            native_event: "TurnStart",
            milestone: Some("turn"),
            always_send: false,
        },
        EvidenceEvent {
            native_event: "SessionEnd",
            milestone: None,
            always_send: false,
        },
        EvidenceEvent {
            native_event: "TurnEnd",
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

pub fn classify(input: &HookInput) -> ContractObservation {
    ContractObservation {
        domain: ContractDomain::Native,
        classification: contract::classify(
            &CONTRACT,
            input.registered_event.as_deref(),
            &input.bytes,
        ),
    }
}
