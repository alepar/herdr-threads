//! opencode decode and encode (opencode design D3) over the recorded plugin payloads in
//! `tests/fixtures/opencode/payloads/`, and the byte-locked hook output shape
//! (`tests/fixtures/opencode/encode/`). Included from `src/harness/opencode/decode.rs`.
use super::*;
use crate::harness::{
    adapter::{ContractObservation, HarnessAdapter},
    contract::Classification,
    opencode::{OpencodeAdapter, OpencodeAdmission},
};
use std::path::PathBuf;

const TOP: &str = "ses_1a2B3c4D5e6F7g8H9i0J";
const CHILD: &str = "ses_9z8Y7x6W5v4U3t2S1r0Q";

fn fixture(path: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/opencode")
        .join(path);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn input(bytes: Vec<u8>, event: &str) -> HookInput {
    HookInput {
        bytes,
        registered_event: Some(event.into()),
    }
}

fn decoded(bytes: Vec<u8>, event: &str) -> Result<DecodedEvent, DecodeFailure> {
    OpencodeAdapter.decode(&OpencodeAdmission, &input(bytes, event))
}

fn payload(value: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

fn kind(event: &DecodedEvent) -> Option<EventKind> {
    match event.intent {
        EventIntent::Lifecycle(kind) => Some(kind),
        _ => None,
    }
}

#[test]
fn recorded_payloads_classify_under_their_registered_events() {
    for (file, event) in [
        ("payloads/session-start-startup.json", "SessionStart"),
        ("payloads/session-start-clear.json", "SessionStart"),
        ("payloads/session-start-resume.json", "SessionStart"),
        ("payloads/session-start-compact.json", "SessionStart"),
        ("payloads/user-prompt-submit.json", "UserPromptSubmit"),
        ("payloads/subagent-start.json", "SubagentStart"),
    ] {
        let ContractObservation {
            domain,
            classification,
        } = OpencodeAdapter.classify(&input(fixture(file), event));
        assert_eq!(domain, ContractDomain::Native, "{file}");
        assert_eq!(classification, Classification::Ok { event }, "{file}");
    }
}

#[test]
fn session_start_sources_decode_to_top_level_lifecycles() {
    for (file, want, source) in [
        ("startup", EventKind::Startup, "startup"),
        ("clear", EventKind::Clear, "clear"),
        ("resume", EventKind::Resume, "resume"),
        ("compact", EventKind::Compact, "compact"),
    ] {
        let event = decoded(
            fixture(&format!("payloads/session-start-{file}.json")),
            "SessionStart",
        )
        .unwrap();
        assert_eq!(kind(&event), Some(want), "{file}");
        assert_eq!(event.role, EventRole::TopLevel, "{file}");
        assert_eq!(event.metadata.context_source, source, "{file}");
        assert_eq!(event.metadata.native_event, "SessionStart");
        assert!(event.metadata.skill_pointer);
        assert!(matches!(event.delivery, DeliveryEligibility::Context));
        assert!(event.binding_event.is_none());
        assert_eq!(event.harness.as_str(), "opencode");
    }
    // The resume source is the one C1 continuity keys on.
    let resume = decoded(
        fixture("payloads/session-start-resume.json"),
        "SessionStart",
    )
    .unwrap();
    assert_eq!(
        resume.metadata.context_source,
        crate::protocol::commands::ContinuityCheckIn::RESUME_SOURCE
    );
    // A startup, clear and resume enroll; a compact re-entry does not.
    use crate::harness::adapter::{EnrollmentKind, EnrollmentPolicy};
    let policy = OpencodeAdapter.enrollment_policy();
    assert_eq!(policy, EnrollmentPolicy::Lifecycle);
    for file in ["startup", "clear", "resume"] {
        let event = decoded(
            fixture(&format!("payloads/session-start-{file}.json")),
            "SessionStart",
        )
        .unwrap();
        assert_eq!(event.enrollment(policy), Some(EnrollmentKind::Lifecycle));
    }
    let compact = decoded(
        fixture("payloads/session-start-compact.json"),
        "SessionStart",
    )
    .unwrap();
    assert_eq!(compact.enrollment(policy), None);
    assert!(compact.can_check_in());
}

#[test]
fn user_prompt_submit_is_a_current_turn_check_in() {
    let event = decoded(
        fixture("payloads/user-prompt-submit.json"),
        "UserPromptSubmit",
    )
    .unwrap();
    assert!(matches!(event.intent, EventIntent::Current));
    assert_eq!(event.role, EventRole::TopLevel);
    assert_eq!(event.native_session.as_deref(), Some(TOP));
    assert_eq!(event.metadata.native_event, "UserPromptSubmit");
    assert_eq!(event.metadata.context_source, "UserPromptSubmit");
    assert!(!event.metadata.skill_pointer);
    assert!(event.can_check_in());
}

#[test]
fn subagent_start_is_a_child_lifecycle_with_no_binding_event() {
    let event = decoded(fixture("payloads/subagent-start.json"), "SubagentStart").unwrap();
    assert_eq!(kind(&event), Some(EventKind::Startup));
    assert_eq!(event.role, EventRole::Subagent);
    assert_eq!(event.native_session.as_deref(), Some(CHILD));
    assert_eq!(event.metadata.native_event, "SubagentStart");
    assert!(event.metadata.agent_id.is_none());
    assert!(event.binding_event.is_none());
    // A child never checks in: its restriction is rendered locally.
    assert!(!event.can_check_in());
}

#[test]
fn the_role_is_declared_by_the_parent_field() {
    let base = |parent: serde_json::Value| {
        payload(
            serde_json::json!({"event": "UserPromptSubmit", "session_id": CHILD,
            "parent_session_id": parent, "cwd": "/w"}),
        )
    };
    for parent in [serde_json::Value::Null, serde_json::json!("")] {
        let event = decoded(base(parent.clone()), "UserPromptSubmit").unwrap();
        assert_eq!(event.role, EventRole::TopLevel, "{parent}");
    }
    let child = decoded(base(serde_json::json!(TOP)), "UserPromptSubmit").unwrap();
    assert_eq!(child.role, EventRole::Subagent);
    // An unknown role is refused, never read as top-level.
    for parent in [
        serde_json::json!("?"),
        serde_json::json!(CHILD),
        serde_json::json!("ses_"),
        serde_json::json!(7),
    ] {
        assert!(
            decoded(base(parent.clone()), "UserPromptSubmit").is_err(),
            "{parent}"
        );
    }
}

#[test]
fn malformed_payloads_are_refused() {
    let cases = [
        // The payload names another event than the registered one.
        (
            serde_json::json!({"event": "SessionStart", "source": "startup", "session_id": TOP, "cwd": "/w"}),
            "UserPromptSubmit",
        ),
        // Unknown event and unknown source.
        (
            serde_json::json!({"event": "Stop", "session_id": TOP, "cwd": "/w"}),
            "Stop",
        ),
        (
            serde_json::json!({"event": "SessionStart", "source": "fork", "session_id": TOP, "cwd": "/w"}),
            "SessionStart",
        ),
        (
            serde_json::json!({"event": "SessionStart", "session_id": TOP, "cwd": "/w"}),
            "SessionStart",
        ),
        // Session ids outside `ses_[0-9A-Za-z]{1,64}`.
        (
            serde_json::json!({"event": "UserPromptSubmit", "session_id": "abc", "cwd": "/w"}),
            "UserPromptSubmit",
        ),
        (
            serde_json::json!({"event": "UserPromptSubmit", "session_id": "ses_a-b", "cwd": "/w"}),
            "UserPromptSubmit",
        ),
        (
            serde_json::json!({"event": "UserPromptSubmit", "cwd": "/w"}),
            "UserPromptSubmit",
        ),
        // A child start names its parent.
        (
            serde_json::json!({"event": "SubagentStart", "session_id": CHILD, "cwd": "/w"}),
            "SubagentStart",
        ),
    ];
    for (value, event) in cases {
        assert!(decoded(payload(value.clone()), event).is_err(), "{value}");
    }
    assert!(decoded(b"not json".to_vec(), "SessionStart").is_err());
}

fn offer(text: &str) -> NeutralOffer {
    NeutralOffer {
        fixed_guidance: text.into(),
        peer_data: serde_json::Value::Null,
        ready_argv: vec![],
    }
}

/// The hook's stdout is the context text itself, with no envelope: the plugin injects it
/// verbatim. Byte-locked against `tests/fixtures/opencode/encode/context.txt`.
#[test]
fn encode_is_the_context_text_verbatim() {
    let text = String::from_utf8(fixture("encode/context.txt")).unwrap();
    let event = decoded(
        fixture("payloads/session-start-startup.json"),
        "SessionStart",
    )
    .unwrap();
    match OpencodeAdapter
        .encode(&OpencodeAdmission, &event, &offer(&text))
        .unwrap()
    {
        EncodedOutput::ContextBearing { bytes } => {
            assert_eq!(bytes, fixture("encode/context.txt"))
        }
        EncodedOutput::ObserverOnly { .. } => panic!("top-level lifecycle bears context"),
    }
    match OpencodeAdapter
        .encode(&OpencodeAdmission, &event, &offer(""))
        .unwrap()
    {
        EncodedOutput::ContextBearing { bytes } => assert!(bytes.is_empty()),
        EncodedOutput::ObserverOnly { .. } => panic!("top-level lifecycle bears context"),
    }
    assert!(
        OpencodeAdapter
            .encode(&OpencodeAdmission, &event, &offer(&"x".repeat(4097)))
            .is_err()
    );
    let child = decoded(fixture("payloads/subagent-start.json"), "SubagentStart").unwrap();
    match OpencodeAdapter
        .encode(&OpencodeAdmission, &child, &offer("child restriction\n"))
        .unwrap()
    {
        EncodedOutput::ObserverOnly { bytes } => assert_eq!(bytes, b"child restriction\n"),
        EncodedOutput::ContextBearing { .. } => panic!("a child's text is observer-only"),
    }
}
