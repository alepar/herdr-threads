//! The opencode hook payload contract (opencode design D3). The plugin runs the registered
//! `hook-event opencode <Event>` command with one JSON object on stdin:
//! `{"event": E, "source"?: S, "session_id": ID, "parent_session_id"?: P, "cwd": DIR}`.
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
            $($field,)*
        ]
    };
}

pub static CONTRACT: HarnessContract = HarnessContract {
    harness: "opencode",
    discriminator: DISCRIMINATOR,
    events: &[
        EventContract {
            event: "SessionStart",
            class: EventClass::Lifecycle,
            fields: common_fields![
                field("source", JsonType::String, true),
                field("parent_session_id", JsonType::StringOrNull, false),
            ],
        },
        EventContract {
            event: "UserPromptSubmit",
            class: EventClass::Tool,
            fields: common_fields![field("parent_session_id", JsonType::StringOrNull, false)],
        },
        EventContract {
            event: "SubagentStart",
            class: EventClass::Lifecycle,
            fields: common_fields![field("parent_session_id", JsonType::String, true)],
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
            native_event: "UserPromptSubmit",
            milestone: Some("turn"),
            always_send: false,
        },
        EvidenceEvent {
            native_event: "SubagentStart",
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
