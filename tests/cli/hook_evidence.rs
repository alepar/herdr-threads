//! The hook's harness evidence note (ht-xoc.4): classification, the
//! per-session gate table, transport and the capability check.
use super::*;
use crate::{
    harness::contract::{contract_for, contract_id},
    protocol::{
        results::{ApiError, ErrorCode},
        time::{Cancellation, MonoInstant},
    },
    test_support::{
        counting_client::{CallKind, CountingLocalClient, DaemonVintage},
        isolation::TestIsolation,
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

const NOW: u64 = 5_000 * HEARTBEAT_MS;

/// Strict synthetic callback data; no native imports, profile reads or runtime probes.
fn hermes_callback() -> Value {
    let mut payload: Value =
        serde_json::from_slice(include_bytes!("../fixtures/hermes/envelopes.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    payload["started_at"] = now.into();
    payload["deadline_at"] = (now + 1200).into();
    payload["observation_order"]["observed_at_millis"] = now.into();
    payload
}

#[test]
fn hermes_structural_domains_reach_real_recorder_without_qualification_conflation() {
    use crate::{
        harness::adapter::{ContractDomain, HookInput, RuntimeAttribution},
        ports::StorePort as _,
        protocol::{
            capabilities::HARNESS_EVIDENCE_V2, commands::HarnessEvidenceOutcomeV2 as Outcome,
            results::HarnessEvidenceV2Recorded,
        },
    };
    let iso = TestIsolation::new("hev-hermes-domains");
    let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::new());
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(iso.path("store.db"), clock.clone()),
            "i",
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let recorder = crate::daemon::harness_evidence::HarnessEvidenceRecorderV2::new(
        store.clone(),
        None,
        clock.clone(),
    );
    let notes = Arc::new(Mutex::new(Vec::new()));
    let captured = notes.clone();
    let client = Arc::new(CountingLocalClient::scripted(
        move |command| {
            let Command::HarnessEvidenceV2(note) = command else {
                panic!("unexpected command {command:?}")
            };
            captured.lock().unwrap().push(note.clone());
            recorder.record(note, &budget()).map(|verified| {
                CommandResult::HarnessEvidenceV2Recorded(HarnessEvidenceV2Recorded { verified })
            })
        },
        DaemonVintage::Current,
    ));
    let registry = crate::harness::registry::builtins();
    let registration = registry.by_id(registry.agent("hermes").unwrap()).unwrap();
    let send = |payload: &Value, state: Option<&Path>, at| {
        run_registered(
            registration,
            Some("pre_llm_call"),
            &serde_json::to_vec(payload).unwrap(),
            state,
            at,
            (&budget(), clock.as_ref()),
            |_| {
                Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.into()]),
                ))
            },
        )
    };
    let native = registration
        .contracts()
        .iter()
        .find(|d| d.domain == ContractDomain::Native)
        .unwrap();
    let bridge = registration
        .contracts()
        .iter()
        .find(|d| d.domain == ContractDomain::Bridge)
        .unwrap();
    let mut calls = 0;
    // Both missing and type-wrong paths must be accepted by the real recorder's allowlist.
    for (index, field, broken_domain, missing) in [
        (0, "deadline_at", bridge, false),
        (1, "deadline_at", bridge, true),
        (2, "shape.session_id.type", native, false),
        (3, "shape.session_id.type", native, true),
    ] {
        let mut payload = hermes_callback();
        payload["session_id"] = format!("broken-{index}").into();
        payload["role_association"]["session_id"] = payload["session_id"].clone();
        let (object, key) = if field == "deadline_at" {
            (payload.as_object_mut().unwrap(), "deadline_at")
        } else {
            (
                payload["shape"]["session_id"].as_object_mut().unwrap(),
                "type",
            )
        };
        if missing {
            object.remove(key);
        } else {
            object.insert(key.into(), serde_json::json!([]));
        }
        assert_eq!(
            send(&payload, Some(iso.state_root()), NOW + index),
            Delivery::Sent(Some(false)),
            "{field}/missing={missing}"
        );
        calls += 2;
        assert_eq!(client.total_calls(), calls);
        let captured = notes.lock().unwrap();
        for (note, descriptor) in captured[captured.len() - 2..].iter().zip([native, bridge]) {
            assert_eq!(note.domain, descriptor.domain_id);
            assert_eq!(note.origin, descriptor.origin);
            assert_eq!(note.contract_id, descriptor.contract_id_v2().unwrap());
            assert_eq!(note.event, "pre_llm_call");
            assert_eq!(
                note.outcome,
                if descriptor.domain == broken_domain.domain {
                    Outcome::Violation {
                        field: field.into(),
                    }
                } else {
                    Outcome::Ok
                }
            );
            assert_eq!(note.runtime, None);
            assert_eq!(
                note.unavailable_reason.as_deref(),
                Some("startup_callback_unavailable")
            );
            assert!(note.qualifications.is_empty());
        }
        drop(captured);
        let diagnostics = store.contract_diagnostics("hermes", &budget()).unwrap();
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.session_id == format!("broken-{index}"))
            .unwrap();
        assert_eq!(
            diagnostic.contract_id,
            broken_domain.contract_id_v2().unwrap()
        );
        assert_eq!(diagnostic.event, "pre_llm_call");
        assert_eq!(diagnostic.field, field);
        assert_eq!(diagnostics.len(), (index + 1) as usize);
        assert_eq!(
            send(&payload, Some(iso.state_root()), NOW + index + 10),
            Delivery::Sent(Some(false))
        );
        calls += 1; // Unqualified Ok in the other domain still needs its milestone.
        assert_eq!(
            client.total_calls(),
            calls,
            "bounded per-session diagnostic send"
        );
        let captured = notes.lock().unwrap();
        assert_ne!(captured.last().unwrap().domain, broken_domain.domain_id);
        assert_eq!(captured.last().unwrap().outcome, Outcome::Ok);
    }
    for (index, callback) in [
        Value::Null,
        serde_json::json!(3),
        serde_json::json!("on_session_start"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut payload = hermes_callback();
        payload["session_id"] = format!("selector-{index}").into();
        if callback.is_null() {
            payload.as_object_mut().unwrap().remove("callback");
        } else {
            payload["callback"] = callback;
        }
        assert_eq!(send(&payload, None, NOW), Delivery::Sent(Some(false)));
        calls += 2;
        assert_eq!(client.total_calls(), calls);
        let captured = notes.lock().unwrap();
        assert_eq!(captured[captured.len() - 2].outcome, Outcome::Malformed);
        assert_eq!(captured[captured.len() - 2].domain, native.domain_id);
        assert_eq!(
            captured.last().unwrap().outcome,
            Outcome::Violation {
                field: "callback".into()
            }
        );
        assert_eq!(captured.last().unwrap().domain, bridge.domain_id);
        drop(captured);
        let diagnostics = store.contract_diagnostics("hermes", &budget()).unwrap();
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.session_id == format!("selector-{index}"))
            .unwrap();
        assert_eq!(diagnostic.contract_id, bridge.contract_id_v2().unwrap());
        assert_eq!(diagnostic.field, "callback");
    }
    let sticky = store.contract_diagnostics("hermes", &budget()).unwrap();
    // Full decoder refusal must never invent a structural field violation.
    for refusal in ["expired", "role", "runtime", "schema", "callback_value"] {
        let mut payload = hermes_callback();
        match refusal {
            "expired" => {
                payload["started_at"] = 1.into();
                payload["deadline_at"] = 1201.into();
                payload["observation_order"]["observed_at_millis"] = 1.into();
            }
            "role" => payload["role_association"]["role"] = "unsupported".into(),
            "runtime" => payload["runtime_identity"]["source"] = "unsupported".into(),
            "schema" => payload["schema_version"] = 2.into(),
            "callback_value" => payload["reset_reason"] = "unsupported".into(),
            _ => unreachable!(),
        }
        payload["session_id"] = "broken-0".into();
        payload["role_association"]["session_id"] = "broken-0".into();
        let input = HookInput {
            bytes: serde_json::to_vec(&payload).unwrap(),
            registered_event: Some("pre_llm_call".into()),
        };
        assert!(matches!(
            registration.attribute_runtime(&input, &budget()),
            RuntimeAttribution::Unavailable { .. }
        ));
        assert_eq!(
            send(&payload, None, NOW),
            Delivery::Sent(Some(false)),
            "{refusal}"
        );
        calls += 2;
        assert_eq!(client.total_calls(), calls);
        for note in notes.lock().unwrap().iter().rev().take(2) {
            assert_eq!(note.outcome, Outcome::Ok, "{refusal}");
            assert_eq!(note.runtime, None);
            assert!(note.qualifications.is_empty());
        }
        assert_eq!(
            store.contract_diagnostics("hermes", &budget()).unwrap(),
            sticky,
            "success cannot clear or refresh sticky failure"
        );
    }
    for descriptor in [native, bridge] {
        assert_eq!(
            store
                .last_unattributed_v2("hermes", descriptor.domain_id, descriptor.origin, &budget())
                .unwrap()
                .unwrap()
                .0,
            "startup_callback_unavailable"
        );
    }
    assert!(
        store
            .harness_evidence_v2_all("hermes", 0, &budget())
            .unwrap()
            .is_empty(),
        "unavailable runtime creates no build or verified milestone"
    );
    assert_eq!(client.total_calls(), 28);
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    }
}

fn claude_id() -> String {
    contract_id(contract_for("claude").unwrap())
}

/// A transcript file whose entries say `version`.
fn transcript(iso: &TestIsolation, version: &str) -> String {
    let path = iso.path(format!("t-{version}.jsonl"));
    std::fs::write(
        &path,
        format!(
            "{{\"type\":\"user\",\"version\":\"{version}\",\"cwd\":\"/tmp\",\"message\":{{}}}}\n"
        ),
    )
    .unwrap();
    path.to_str().unwrap().to_owned()
}

fn start(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "SessionStart", "session_id": session,
        "transcript_path": transcript, "source": "startup"}))
    .unwrap()
}

fn tool(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse", "session_id": session,
        "transcript_path": transcript, "tool_name": "Bash", "tool_use_id": "toolu_1",
        "tool_input": {"command": "ls"}}))
    .unwrap()
}

/// A payload missing `tool_name`: a violation of the PreToolUse contract.
fn broken_tool(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse", "session_id": session,
        "transcript_path": transcript, "tool_use_id": "toolu_1"}))
    .unwrap()
}

/// A Codex rollout created by `creator`: one head `session_meta`, then
/// turns with no version field (the spike's observed resumed shape).
fn codex_rollout(iso: &TestIsolation, creator: &str) -> String {
    let path = iso.path(format!("rollout-{creator}.jsonl"));
    std::fs::write(
        &path,
        format!(
            "{{\"timestamp\":\"2026-09-29T10:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"019e0000-0000-7000-8000-000000000002\",\"cwd\":\"/tmp\",\"originator\":\"codex_cli_rs\",\"cli_version\":\"{creator}\",\"source\":\"cli\"}}}}\n\
             {{\"timestamp\":\"2026-09-30T09:00:01.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[]}}}}\n"
        ),
    )
    .unwrap();
    path.to_str().unwrap().to_owned()
}

fn codex_start(session: &str, transcript: &str, source: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "SessionStart", "session_id": session, "source": source,
        "transcript_path": transcript}))
    .unwrap()
}

fn codex_tool(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse", "session_id": session, "turn_id": "turn-1",
        "tool_name": "Bash", "tool_use_id": "call_1", "tool_input": {"command": "ls"},
        "transcript_path": transcript}))
    .unwrap()
}

fn codex_subagent(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "SubagentStart", "session_id": session, "turn_id": "turn-1",
        "cwd": "/tmp", "model": "gpt-5", "permission_mode": "default",
        "agent_id": "agent-1", "agent_type": "default", "transcript_path": transcript}))
    .unwrap()
}

/// Missing `tool_name`: a violation of the Codex PreToolUse contract.
fn codex_broken_tool(session: &str, transcript: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse", "session_id": session, "turn_id": "turn-1",
        "tool_use_id": "call_1", "tool_input": {"command": "ls"},
        "transcript_path": transcript}))
    .unwrap()
}

// -- classification ---------------------------------------------------------

#[test]
fn ok_payload_classifies_ok_under_its_registered_event() {
    let classified =
        classify_payload(Harness::Claude, Some("PreToolUse"), &tool("s1", "/x")).unwrap();
    assert_eq!(classified.event, "PreToolUse");
    assert_eq!(classified.outcome, HarnessEvidenceOutcome::Ok);
    assert_eq!(classified.session_id.as_deref(), Some("s1"));
    assert_eq!(classified.contract_id, claude_id());
}

#[test]
fn legacy_registration_takes_the_event_from_the_discriminator() {
    let classified = classify_payload(Harness::Claude, None, &tool("s1", "/x")).unwrap();
    assert_eq!(classified.event, "PreToolUse");
    assert_eq!(classified.outcome, HarnessEvidenceOutcome::Ok);
}

#[test]
fn missing_field_is_a_violation_naming_it() {
    let classified = classify_payload(
        Harness::Claude,
        Some("PreToolUse"),
        &broken_tool("s1", "/x"),
    )
    .unwrap();
    assert_eq!(
        classified.outcome,
        HarnessEvidenceOutcome::Violation {
            field: "tool_name".into()
        }
    );
    assert_eq!(classified.event, "PreToolUse");
}

#[test]
fn unparseable_payload_is_malformed_with_a_safe_event_name() {
    for stdin in [&b"not json"[..], b"", b"[1]"] {
        let registered = classify_payload(Harness::Claude, Some("SessionStart"), stdin).unwrap();
        assert_eq!(registered.outcome, HarnessEvidenceOutcome::Malformed);
        assert_eq!(registered.event, "SessionStart", "the registered event");
        let legacy = classify_payload(Harness::Claude, None, stdin).unwrap();
        assert_eq!(legacy.outcome, HarnessEvidenceOutcome::Malformed);
        assert_eq!(legacy.event, "unknown");
    }
    // A well-formed object that names no event: malformed without a
    // registration, a violation of the discriminator with one.
    let nameless = br#"{"x":1}"#;
    let legacy = classify_payload(Harness::Claude, None, nameless).unwrap();
    assert_eq!(
        (legacy.event.as_str(), legacy.outcome),
        ("unknown", HarnessEvidenceOutcome::Malformed)
    );
    let registered = classify_payload(Harness::Claude, Some("SessionStart"), nameless).unwrap();
    assert_eq!(
        registered.outcome,
        HarnessEvidenceOutcome::Violation {
            field: "hook_event_name".into()
        }
    );
    // An unknown discriminator that is a valid name keeps it; one that is
    // not a valid name falls back to "unknown".
    let named = br#"{"hook_event_name":"Compact"}"#;
    assert_eq!(
        classify_payload(Harness::Claude, None, named)
            .unwrap()
            .event,
        "Compact"
    );
    let bad = br#"{"hook_event_name":"Pre Tool"}"#;
    assert_eq!(
        classify_payload(Harness::Claude, None, bad).unwrap().event,
        "unknown"
    );
}

#[test]
fn oversized_session_id_is_dropped_and_a_human_has_no_contract() {
    let long = "s".repeat(257);
    let classified = classify_payload(Harness::Claude, None, &tool(&long, "/x")).unwrap();
    assert_eq!(classified.session_id, None);
    assert!(classify_payload(Harness::Human, None, &tool("s", "/x")).is_none());
}

#[test]
fn attribution_reads_the_transcript_and_reports_why_not() {
    let iso = TestIsolation::new("hev-attr");
    let path = transcript(&iso, "2.1.286");
    let attributed = evidence_for(Harness::Claude, Some("PreToolUse"), &tool("s", &path)).unwrap();
    assert_eq!(attributed.version.as_deref(), Some("2.1.286"));
    assert_eq!(attributed.unattributed_reason, None);
    let missing = evidence_for(
        Harness::Claude,
        Some("PreToolUse"),
        &tool("s", "/definitely/not/here.jsonl"),
    )
    .unwrap();
    assert_eq!(missing.version, None);
    assert_eq!(
        missing.unattributed_reason.as_deref(),
        Some("transcript not found")
    );
    let not_json = evidence_for(Harness::Claude, None, b"nope").unwrap();
    assert_eq!(
        not_json.unattributed_reason.as_deref(),
        Some("no transcript path in the payload")
    );
    let resume = serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "SessionStart", "session_id": "s", "source": "resume",
        "transcript_path": path}))
    .unwrap();
    let resumed = evidence_for(Harness::Claude, Some("SessionStart"), &resume).unwrap();
    assert_eq!(resumed.version, None, "a resume is never read");
    assert_eq!(
        resumed.unattributed_reason.as_deref(),
        Some("resume before first entry")
    );
}

// -- the gate table ---------------------------------------------------------

struct Fx {
    iso: TestIsolation,
    sent: Arc<Mutex<Vec<HarnessEvidence>>>,
    verified: Arc<AtomicBool>,
    client: Arc<CountingLocalClient>,
    connects: Mutex<u32>,
}

impl Fx {
    fn new(label: &str, vintage: DaemonVintage) -> Self {
        let sent: Arc<Mutex<Vec<HarnessEvidence>>> = Arc::default();
        let verified = Arc::new(AtomicBool::new(false));
        let (record, reply) = (Arc::clone(&sent), Arc::clone(&verified));
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| match command {
                Command::HarnessEvidence(note) => {
                    record.lock().unwrap().push(note.clone());
                    Ok(CommandResult::HarnessEvidenceRecorded {
                        verified: reply.load(Ordering::SeqCst),
                    })
                }
                other => panic!("unexpected command {other:?}"),
            },
            vintage,
        ));
        Self {
            iso: TestIsolation::new(label),
            sent,
            verified,
            client,
            connects: Mutex::new(0),
        }
    }

    fn hook(&self, event: &str, stdin: &[u8], now_ms: u64) -> Delivery {
        self.hook_as(Harness::Claude, event, stdin, now_ms)
    }

    fn hook_as(&self, harness: Harness, event: &str, stdin: &[u8], now_ms: u64) -> Delivery {
        run(
            harness,
            Some(event),
            stdin,
            Some(self.iso.state_root()),
            now_ms,
            &budget(),
            |budget| {
                *self.connects.lock().unwrap() += 1;
                let client = Arc::clone(&self.client);
                let capabilities = client.capabilities();
                let _ = budget;
                Some((client as Arc<dyn LocalClient>, capabilities))
            },
        )
    }

    fn notes(&self) -> Vec<HarnessEvidence> {
        self.sent.lock().unwrap().clone()
    }

    fn connects(&self) -> u32 {
        *self.connects.lock().unwrap()
    }

    fn gate(&self, harness: &str, session: &str) -> GateState {
        let path = gate_path(&gate_dir(self.iso.state_root()), harness, session);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn gate_files(&self) -> usize {
        std::fs::read_dir(gate_dir(self.iso.state_root())).map_or(0, |entries| entries.count())
    }
}

#[test]
fn session_start_is_always_sent() {
    let fx = Fx::new("hev-start", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.verified.store(true, Ordering::SeqCst);
    for i in 0..3 {
        assert_eq!(
            fx.hook("SessionStart", &start("s", &path), NOW + i),
            Delivery::Sent(Some(true))
        );
    }
    assert_eq!(fx.notes().len(), 3, "even once verified");
    let note = &fx.notes()[0];
    assert_eq!(note.harness, "claude");
    assert_eq!(note.version.as_deref(), Some("2.1.286"));
    assert_eq!(note.event, "SessionStart");
    assert_eq!(note.outcome, HarnessEvidenceOutcome::Ok);
    assert_eq!(note.session_id.as_deref(), Some("s"));
    assert_eq!(note.contract_id, claude_id());
}

#[test]
fn first_tool_ok_is_sent_once_while_unverified_then_hooks_stop_sending_notes() {
    let fx = Fx::new("hev-first-tool", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    // Unverified reply: one note per session, not one per tool call.
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW),
        Delivery::Sent(Some(false))
    );
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 1),
        Delivery::Suppressed
    );
    assert_eq!(fx.notes().len(), 1);
    // A second session is its own gate.
    assert_eq!(
        fx.hook("PreToolUse", &tool("s2", &path), NOW + 2),
        Delivery::Sent(Some(false))
    );
    assert_eq!(fx.gate_files(), 2);
}

#[test]
fn verified_reply_silences_later_ok_notes() {
    let fx = Fx::new("hev-verified", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.verified.store(true, Ordering::SeqCst);
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW),
        Delivery::Sent(Some(true))
    );
    let connects = fx.connects();
    for i in 1..5 {
        assert_eq!(
            fx.hook("PreToolUse", &tool("s", &path), NOW + i),
            Delivery::Suppressed
        );
    }
    assert_eq!(fx.notes().len(), 1);
    assert_eq!(
        fx.connects(),
        connects,
        "the gate decides before any socket I/O"
    );
}

#[test]
fn ok_sent_after_a_verified_session_start_is_suppressed_for_tools() {
    let fx = Fx::new("hev-start-verified", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.verified.store(true, Ordering::SeqCst);
    fx.hook("SessionStart", &start("s", &path), NOW);
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 1),
        Delivery::Suppressed
    );
}

#[test]
fn session_start_does_not_spend_the_first_tool_note_while_unverified() {
    let fx = Fx::new("hev-start-unverified", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.hook("SessionStart", &start("s", &path), NOW);
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 1),
        Delivery::Sent(Some(false)),
        "the tool payload is what completes verification"
    );
}

#[test]
fn subagent_ok_does_not_spend_the_tool_slot() {
    let fx = Fx::new("hev-subagent", DaemonVintage::Current);
    let path = codex_rollout(&fx.iso, "0.160.0");
    fx.hook_as(
        Harness::Codex,
        "SubagentStart",
        &codex_subagent("s", &path),
        NOW,
    );
    assert_eq!(
        fx.hook_as(
            Harness::Codex,
            "PreToolUse",
            &codex_tool("s", &path),
            NOW + 1
        ),
        Delivery::Sent(Some(false)),
        "a SubagentStart ok must not hold the PreToolUse ok back to the heartbeat"
    );
    assert_eq!(fx.gate("codex", "s").ok_sent_at_ms, Some(NOW + 1));
}

#[test]
fn subagent_ok_is_only_a_heartbeat() {
    let fx = Fx::new("hev-subagent-hb", DaemonVintage::Current);
    let path = codex_rollout(&fx.iso, "0.160.0");
    fx.hook_as(
        Harness::Codex,
        "SessionStart",
        &codex_start("s", &path, "startup"),
        NOW,
    );
    let connects = fx.connects();
    assert_eq!(
        fx.hook_as(
            Harness::Codex,
            "SubagentStart",
            &codex_subagent("s", &path),
            NOW + 1
        ),
        Delivery::Suppressed
    );
    assert_eq!(fx.connects(), connects, "no connect for a suppressed ok");
    assert!(matches!(
        fx.hook_as(
            Harness::Codex,
            "SubagentStart",
            &codex_subagent("s", &path),
            NOW + HEARTBEAT_MS
        ),
        Delivery::Sent(_)
    ));
}

#[test]
fn heartbeat_is_sent_after_an_hour_even_when_verified() {
    let fx = Fx::new("hev-heartbeat", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.verified.store(true, Ordering::SeqCst);
    fx.hook("PreToolUse", &tool("s", &path), NOW);
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + HEARTBEAT_MS - 1),
        Delivery::Suppressed
    );
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + HEARTBEAT_MS),
        Delivery::Sent(Some(true)),
        "an hour on, one ok note keeps the session in Health"
    );
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + HEARTBEAT_MS + 1),
        Delivery::Suppressed,
        "and the next heartbeat is an hour after that one"
    );
    assert_eq!(fx.notes().len(), 2);
}

#[test]
fn violation_is_sent_once_per_session_event_and_field() {
    let fx = Fx::new("hev-violation", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.verified.store(true, Ordering::SeqCst);
    // Verified does not silence a violation.
    fx.hook("PreToolUse", &tool("s", &path), NOW);
    assert_eq!(
        fx.hook("PreToolUse", &broken_tool("s", &path), NOW + 1),
        Delivery::Sent(Some(true))
    );
    assert_eq!(
        fx.hook("PreToolUse", &broken_tool("s", &path), NOW + 2),
        Delivery::Suppressed
    );
    // Another session reports it again.
    assert_eq!(
        fx.hook("PreToolUse", &broken_tool("s2", &path), NOW + 3),
        Delivery::Sent(Some(true))
    );
    let violations: Vec<_> = fx
        .notes()
        .into_iter()
        .filter(|n| matches!(n.outcome, HarnessEvidenceOutcome::Violation { .. }))
        .collect();
    assert_eq!(violations.len(), 2);
    assert_eq!(
        violations[0].outcome,
        HarnessEvidenceOutcome::Violation {
            field: "tool_name".into()
        }
    );
}

#[test]
fn malformed_is_sent_once_per_session_and_event() {
    let fx = Fx::new("hev-malformed", DaemonVintage::Current);
    // No session id (unparseable): the hook cannot dedupe, so it reports.
    assert_eq!(
        fx.hook("PreToolUse", b"garbage", NOW),
        Delivery::Sent(Some(false))
    );
    assert_eq!(
        fx.hook("PreToolUse", b"garbage", NOW + 1),
        Delivery::Sent(Some(false))
    );
    assert_eq!(fx.notes().len(), 2);
    // A session id and an event no contract declares: malformed, deduped by
    // (session, event). `run` is given no registration so the payload's own
    // discriminator names the event.
    let unknown = br#"{"hook_event_name":"Compact","session_id":"s"}"#;
    let send = |stdin: &[u8], now| {
        let client = Arc::clone(&fx.client);
        run(
            Harness::Claude,
            None,
            stdin,
            Some(fx.iso.state_root()),
            now,
            &budget(),
            move |_| {
                let capabilities = client.capabilities();
                Some((client as Arc<dyn LocalClient>, capabilities))
            },
        )
    };
    assert_eq!(send(unknown, NOW + 2), Delivery::Sent(Some(false)));
    assert_eq!(send(unknown, NOW + 3), Delivery::Suppressed);
    let other_event = br#"{"hook_event_name":"Stop","session_id":"s"}"#;
    assert_eq!(
        send(other_event, NOW + 4),
        Delivery::Sent(Some(false)),
        "another event is its own entry"
    );
    let last = fx.notes().pop().unwrap();
    assert_eq!(last.outcome, HarnessEvidenceOutcome::Malformed);
    assert_eq!(last.event, "Stop");
}

#[test]
fn no_session_id_sends_only_session_start_violation_and_malformed() {
    let gate = GateState::default();
    let ok = HarnessEvidenceOutcome::Ok;
    assert!(
        !gate.should_send("PreToolUse", &ok, false, NOW),
        "an ok tool note needs a session to be gated"
    );
    assert!(gate.should_send("SessionStart", &ok, false, NOW));
    let violation = HarnessEvidenceOutcome::Violation { field: "f".into() };
    assert!(gate.should_send("PreToolUse", &violation, false, NOW));
    assert!(gate.should_send("PreToolUse", &HarnessEvidenceOutcome::Malformed, false, NOW));
    // Through the whole path: an unparseable payload has no session id, so it
    // is sent and leaves no gate file behind.
    let fx = Fx::new("hev-no-session", DaemonVintage::Current);
    assert_eq!(
        fx.hook("PreToolUse", b"garbage", NOW),
        Delivery::Sent(Some(false))
    );
    assert_eq!(fx.gate_files(), 0, "no session id, no gate file");
}

#[test]
fn a_failed_send_does_not_close_the_gate() {
    let iso = TestIsolation::new("hev-failed");
    let path = transcript(&iso, "2.1.286");
    let refusing = Arc::new(CountingLocalClient::scripted(
        |_| Err(ApiError::new(ErrorCode::Unsupported, "no")),
        DaemonVintage::Current,
    ));
    let call = |now| {
        let client = Arc::clone(&refusing);
        run(
            Harness::Claude,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            now,
            &budget(),
            move |_| {
                let capabilities = client.capabilities();
                Some((client as Arc<dyn LocalClient>, capabilities))
            },
        )
    };
    assert_eq!(call(NOW), Delivery::Sent(None));
    assert_eq!(
        call(NOW + 1),
        Delivery::Sent(None),
        "an error leaves the gate open so the next event retries"
    );
    assert_eq!(refusing.calls(CallKind::HarnessEvidence), 2);
}

#[test]
fn capability_absent_daemon_gets_nothing() {
    let fx = Fx::new("hev-older", DaemonVintage::Older);
    let path = transcript(&fx.iso, "2.1.286");
    assert_eq!(
        fx.hook("SessionStart", &start("s", &path), NOW),
        Delivery::Unavailable
    );
    assert_eq!(
        fx.hook("PreToolUse", &broken_tool("s", &path), NOW),
        Delivery::Unavailable
    );
    assert_eq!(fx.client.total_calls(), 0, "no call besides capabilities");
    assert_eq!(fx.gate_files(), 0, "no gate file written");
    assert!(!gate_dir(fx.iso.state_root()).exists());
}

// Kills: an evidence client built without connect()'s protocol check, which
// would send a note a previous-protocol daemon drops at decode and so block
// until the call cap.
#[test]
fn evidence_skips_a_previous_protocol_daemon() {
    use crate::{
        daemon::{
            ownership::OwnerLock,
            paths::{InstancePaths, RuntimeContext},
        },
        protocol::wire::PROTOCOL_VERSION,
    };
    let iso = TestIsolation::new("hev-skew");
    let path = transcript(&iso, "2.1.286");
    let args = HookArgs {
        state_dir: Some(iso.state_root().to_path_buf()),
        host_endpoint: Some(iso.state_root().join("host.sock")),
        harness: Harness::Claude,
        event: Some("SessionStart".into()),
    };
    let context = RuntimeContext::explicit(
        iso.state_root().to_path_buf(),
        iso.state_root().join("host.sock"),
        None,
    )
    .unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let lock = OwnerLock::acquire(&paths).unwrap();
    // Bound but never accepted: a client that sent anyway would block.
    let listener = lock.bind_socket().unwrap();
    lock.publish_endpoint(&listener, "0.0.1", PROTOCOL_VERSION - 1)
        .unwrap();
    let started = Instant::now();
    report(
        &args,
        &start("s", &path),
        Some(iso.state_root()),
        started + Duration::from_secs(5),
        CALL_CAP,
        Arc::new(crate::app::SystemClock::new()),
    );
    assert!(
        started.elapsed() < CALL_CAP / 2,
        "evidence waited {:?} of its {CALL_CAP:?} cap",
        started.elapsed()
    );
    assert_eq!(
        std::fs::read_dir(gate_dir(iso.state_root())).map_or(0, |entries| entries.count()),
        0,
        "no gate file written"
    );
    drop(listener);
    drop(lock);
}

#[test]
fn no_daemon_is_silent_and_writes_no_gate() {
    let iso = TestIsolation::new("hev-nodaemon");
    let path = transcript(&iso, "2.1.286");
    let delivery = run(
        Harness::Claude,
        Some("PreToolUse"),
        &tool("s", &path),
        Some(iso.state_root()),
        NOW,
        &budget(),
        |_| None,
    );
    assert_eq!(delivery, Delivery::Unavailable);
    assert!(!gate_dir(iso.state_root()).exists());
}

#[test]
fn unattributed_payload_is_still_sent_with_its_reason() {
    let fx = Fx::new("hev-unattributed", DaemonVintage::Current);
    let delivered = fx.hook("PreToolUse", &tool("s", "/definitely/not/here.jsonl"), NOW);
    assert_eq!(delivered, Delivery::Sent(Some(false)));
    let note = &fx.notes()[0];
    assert_eq!(note.version, None);
    assert_eq!(
        note.unattributed_reason.as_deref(),
        Some("transcript not found")
    );
    assert!(Command::HarnessEvidence(note.clone()).validate().is_ok());
}

#[test]
fn every_note_the_hook_builds_passes_wire_validation() {
    let fx = Fx::new("hev-valid", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.hook("SessionStart", &start("s", &path), NOW);
    fx.hook("PreToolUse", &broken_tool("s", &path), NOW);
    fx.hook("PreToolUse", b"garbage", NOW);
    assert_eq!(fx.notes().len(), 3);
    for note in fx.notes() {
        assert_eq!(
            Command::HarnessEvidence(note.clone()).validate(),
            Ok(()),
            "{note:?}"
        );
    }
}

// -- the gate file ----------------------------------------------------------

#[test]
fn gate_file_is_private_bounded_and_named_by_the_session_hash() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new("hev-gate-file", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.hook("PreToolUse", &tool("sess-1", &path), NOW);
    let file = gate_path(&gate_dir(fx.iso.state_root()), "claude", "sess-1");
    let name = file.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(
        name.starts_with("claude-") && name.ends_with(".json"),
        "{name}"
    );
    assert_eq!(name.len(), "claude-".len() + 16 + ".json".len());
    assert!(!name.contains("sess-1"), "the session id is hashed");
    let meta = std::fs::metadata(&file).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert!(meta.len() <= MAX_GATE_BYTES);
    let dir_mode = std::fs::metadata(gate_dir(fx.iso.state_root()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
    let stored: GateState = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(stored.ok_sent_at_ms, Some(NOW));
    assert!(!stored.verified);
}

#[test]
fn a_foreign_or_oversized_gate_file_reads_as_a_fresh_gate() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new("hev-gate-bad", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.hook("PreToolUse", &tool("s", &path), NOW);
    let file = gate_path(&gate_dir(fx.iso.state_root()), "claude", "s");
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 1),
        Delivery::Suppressed
    );
    // World-readable: not trusted, so the gate is fresh and the note is sent.
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 2),
        Delivery::Sent(Some(false))
    );
    // An oversized file is not trusted either.
    std::fs::write(&file, vec![b' '; MAX_GATE_BYTES as usize + 1]).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        fx.hook("PreToolUse", &tool("s", &path), NOW + 3),
        Delivery::Sent(Some(false))
    );
}

#[test]
fn sent_list_stays_within_the_gate_bound() {
    let mut state = GateState::default();
    for i in 0..100 {
        state = state.after_send(
            "PreToolUse",
            &HarnessEvidenceOutcome::Violation {
                field: format!("tool_input.{}", "f".repeat(100) + &i.to_string()),
            },
            false,
            NOW,
        );
    }
    assert!(state.sent.len() <= 16);
    assert!(serde_json::to_vec(&state).unwrap().len() as u64 <= MAX_GATE_BYTES);
}

#[test]
fn prune_removes_at_most_sixteen_files_older_than_a_day() {
    let fx = Fx::new("hev-prune", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    fx.hook("PreToolUse", &tool("live", &path), NOW);
    let dir = gate_dir(fx.iso.state_root());
    let old = std::time::SystemTime::now() - Duration::from_millis(GATE_MAX_AGE_MS + 60_000);
    for i in 0..20 {
        let file = dir.join(format!("claude-{i:016x}.json"));
        std::fs::write(&file, b"{}").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    prune(&dir, now_ms);
    let left = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(left, 1 + 20 - PRUNE_PER_RUN, "16 pruned, the live one kept");
    prune(&dir, now_ms);
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "the rest go on the next run, the fresh gate stays"
    );
}

// -- Codex resume -------------------------------------------------------------

const CODEX_RESUMED: &str = "codex resume: rollout version is the creating CLI's";

#[test]
fn codex_resume_marks_the_gate_and_every_later_note_is_unattributed() {
    let fx = Fx::new("hev-codex-resume", DaemonVintage::Current);
    let rollout = codex_rollout(&fx.iso, "0.159.3");
    let delivered = fx.hook_as(
        Harness::Codex,
        "SessionStart",
        &codex_start("s", &rollout, "resume"),
        NOW,
    );
    assert!(matches!(delivered, Delivery::Sent(_)), "{delivered:?}");
    assert!(fx.gate("codex", "s").resumed);
    assert!(matches!(
        fx.hook_as(
            Harness::Codex,
            "PreToolUse",
            &codex_tool("s", &rollout),
            NOW + 1
        ),
        Delivery::Sent(_)
    ));
    assert!(matches!(
        fx.hook_as(
            Harness::Codex,
            "PreToolUse",
            &codex_broken_tool("s", &rollout),
            NOW + 2
        ),
        Delivery::Sent(_)
    ));
    let notes = fx.notes();
    assert_eq!(notes.len(), 3);
    for note in &notes {
        assert_eq!(note.version, None, "{note:?}");
        assert_eq!(note.unattributed_reason.as_deref(), Some(CODEX_RESUMED));
    }
}

#[test]
fn codex_resume_mark_is_written_without_a_daemon() {
    let fx = Fx::new("hev-codex-nodaemon", DaemonVintage::Current);
    let rollout = codex_rollout(&fx.iso, "0.159.3");
    let delivery = run(
        Harness::Codex,
        Some("SessionStart"),
        &codex_start("s", &rollout, "resume"),
        Some(fx.iso.state_root()),
        NOW,
        &budget(),
        |_| None,
    );
    assert_eq!(delivery, Delivery::Unavailable);
    assert_eq!(std::fs::read(gate_path(&gate_dir(fx.iso.state_root()), "codex", "s")).unwrap(),
        br#"{"verified":false,"ok_sent_at_ms":null,"heartbeat_at_ms":null,"sent":[],"resumed":true}"#);
    assert!(fx.gate("codex", "s").resumed);
    assert!(matches!(
        fx.hook_as(
            Harness::Codex,
            "PreToolUse",
            &codex_tool("s", &rollout),
            NOW + 1
        ),
        Delivery::Sent(_)
    ));
    let note = &fx.notes()[0];
    assert_eq!(note.version, None);
    assert_eq!(note.unattributed_reason.as_deref(), Some(CODEX_RESUMED));
}

#[test]
fn codex_fresh_session_creator_metadata_is_not_current_runtime() {
    let fx = Fx::new("hev-codex-fresh", DaemonVintage::Current);
    let rollout = codex_rollout(&fx.iso, "0.159.3");
    fx.hook_as(
        Harness::Codex,
        "SessionStart",
        &codex_start("s", &rollout, "startup"),
        NOW,
    );
    fx.hook_as(
        Harness::Codex,
        "PreToolUse",
        &codex_tool("s", &rollout),
        NOW + 1,
    );
    let notes = fx.notes();
    assert_eq!(notes.len(), 2);
    for note in &notes {
        assert_eq!(note.version, None, "{note:?}");
        assert_eq!(
            note.unattributed_reason.as_deref(),
            Some(crate::harness::attribution::Unattributed::CodexCreatorOnly.as_str())
        );
    }
    assert!(!fx.gate("codex", "s").resumed);
}

#[test]
fn claude_resume_is_unchanged() {
    let fx = Fx::new("hev-claude-resume", DaemonVintage::Current);
    let path = transcript(&fx.iso, "2.1.286");
    let resume = serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "SessionStart", "session_id": "s", "source": "resume",
        "transcript_path": path}))
    .unwrap();
    assert_eq!(
        fx.hook("SessionStart", &resume, NOW),
        Delivery::Sent(Some(false))
    );
    assert_eq!(
        fx.notes()[0].unattributed_reason.as_deref(),
        Some("resume before first entry")
    );
    assert!(!fx.gate("claude", "s").resumed);
    fx.hook("PreToolUse", &tool("s", &path), NOW + 1);
    assert_eq!(fx.notes()[1].version.as_deref(), Some("2.1.286"));
}

#[test]
fn gate_file_without_resumed_reads_as_not_resumed() {
    let gate = serde_json::from_str::<GateState>(
        r#"{"verified":false,"ok_sent_at_ms":null,"heartbeat_at_ms":null,"sent":[]}"#,
    )
    .unwrap();
    assert!(!gate.resumed);
}

#[test]
fn codex_session_resumed_after_an_upgrade_records_nothing_for_either_version() {
    use crate::ports::StorePort as _;
    let iso = TestIsolation::new("hev-codex-e2e");
    let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::new());
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(
                iso.state_root().join("store.db"),
                Arc::clone(&clock),
            ),
            "i",
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let recorder = Arc::new(
        crate::daemon::harness_evidence::HarnessEvidenceRecorder::new(
            store.clone(),
            None,
            Arc::clone(&clock),
        ),
    );
    let client = Arc::new(CountingLocalClient::scripted(
        {
            let recorder = Arc::clone(&recorder);
            move |command| match command {
                Command::HarnessEvidence(note) => recorder
                    .record(note, &budget())
                    .map(|verified| CommandResult::HarnessEvidenceRecorded { verified }),
                other => panic!("unexpected command {other:?}"),
            }
        },
        DaemonVintage::Current,
    ));
    // The rollout names the creator (0.159.3); the process now writing it is
    // 0.160.0 and is visible nowhere.
    let rollout = codex_rollout(&iso, "0.159.3");
    let stdins = [
        ("SessionStart", codex_start("s", &rollout, "resume"), NOW),
        ("PreToolUse", codex_tool("s", &rollout), NOW + 1),
        ("PreToolUse", codex_broken_tool("s", &rollout), NOW + 2),
        (
            "PreToolUse",
            codex_tool("s", &rollout),
            NOW + 1 + HEARTBEAT_MS,
        ),
    ];
    for (event, stdin, now) in &stdins {
        let delivery = run(
            Harness::Codex,
            Some(event),
            stdin,
            Some(iso.state_root()),
            *now,
            &budget(),
            |_| {
                let capabilities = client.capabilities();
                Some((Arc::clone(&client) as Arc<dyn LocalClient>, capabilities))
            },
        );
        assert!(
            matches!(delivery, Delivery::Sent(_)),
            "{event}: {delivery:?}"
        );
    }
    assert!(
        store
            .harness_evidence_all("codex", &budget())
            .unwrap()
            .is_empty(),
        "no row for 0.159.3 or 0.160.0: nothing verified, no violation"
    );
    assert_eq!(
        store
            .last_unattributed("codex", &budget())
            .unwrap()
            .map(|(reason, _)| reason)
            .as_deref(),
        Some(CODEX_RESUMED)
    );
}

// Synthetic rich-only registration: projected metadata, never native Hermes proof.
mod rich_fixture {
    use crate::harness::adapter::*;
    use crate::harness::{claude::ClaudeAdapter, evidence::EvidenceOrigin};
    use crate::protocol::time::CallBudget;
    pub struct Adapter(pub u8);
    impl HarnessAdapter for Adapter {
        type Admission = ();
        fn legacy_contract_id(&self) -> Option<String> {
            (self.0 == 1).then(|| ClaudeAdapter.legacy_contract_id().unwrap())
        }
        fn metadata(&self) -> &'static AdapterMetadata {
            ClaudeAdapter.metadata()
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            static DOMAINS: std::sync::OnceLock<Vec<ContractDescriptor>> =
                std::sync::OnceLock::new();
            let domains = DOMAINS.get_or_init(|| {
                let mut native = ClaudeAdapter.contracts()[0];
                native.domain_id = "shape";
                native.origin = EvidenceOrigin::NativeShapeObservation;
                let mut bridge = native;
                bridge.domain = ContractDomain::Bridge;
                bridge.domain_id = "envelope";
                bridge.origin = EvidenceOrigin::BridgeEnvelope;
                vec![native, bridge]
            });
            static QUALIFIED: std::sync::OnceLock<Vec<ContractDescriptor>> =
                std::sync::OnceLock::new();
            if self.0 >= 2 {
                QUALIFIED.get_or_init(|| {
                    domains
                        .iter()
                        .map(|d| {
                            let mut d = *d;
                            d.qualifications = &["same_runtime"];
                            d
                        })
                        .collect()
                })
            } else {
                domains
            }
        }
        fn classify(&self, input: &HookInput) -> ContractObservation {
            let domain = if serde_json::from_slice::<serde_json::Value>(&input.bytes)
                .ok()
                .is_some_and(|p| p["bridge"] == true)
            {
                ContractDomain::Bridge
            } else {
                ContractDomain::Native
            };
            ContractObservation {
                domain,
                classification: ClaudeAdapter.classify(input).classification,
            }
        }
        fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution {
            if self.0 == 1 {
                RuntimeAttribution::Attributed(
                    RuntimeIdentity::stable_release("9.9.9", "native_transcript").unwrap(),
                )
            } else {
                ClaudeAdapter.attribute_runtime(input, budget)
            }
        }
        fn evidence_qualifications(
            &self,
            request: &EvidenceQualificationRequest<'_>,
            _: &CallBudget,
        ) -> Result<Vec<String>, String> {
            if self.0 < 2 {
                return Ok(vec![]);
            }
            assert_eq!(request.runtime.key, "release:2.1.286");
            assert_eq!(request.descriptor.domain_id, "shape");
            assert!(request.input.bytes.starts_with(b"{"));
            Ok(match self.0 {
                2 => vec!["same_runtime".into()],
                3 => vec!["same_runtime".into(), "same_runtime".into()],
                4 => vec!["undeclared".into()],
                5 => vec!["Malformed".into()],
                6 => vec!["same_runtime".into(); 9],
                _ => vec![],
            })
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            panic!("evidence must never probe")
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            panic!("evidence must never admit")
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            panic!("evidence must never consult admission")
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            panic!("classification independent of decode")
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            panic!("observer evidence cannot offer")
        }
        fn setup(&self, _: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            panic!("evidence cannot setup")
        }
        fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
            panic!("evidence cannot inspect installation")
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            panic!("evidence cannot remove")
        }
    }
    pub fn registration() -> crate::harness::registry::Registration {
        crate::harness::registry::Registration::new(&Adapter(0))
    }
}

// Kills legacy-only submission despite advertised rich capability, and session-only gates.
#[test]
fn v2_client_gates_isolate_runtime_domains_retry_and_never_downgrade_rich_evidence() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-v2-client");
    let first = transcript(&iso, "2.1.286");
    let second = transcript(&iso, "2.1.287");
    let notes = Arc::new(Mutex::new(Vec::new()));
    let captured = notes.clone();
    let client = Arc::new(CountingLocalClient::scripted(
        move |command| {
            captured.lock().unwrap().push(command.clone());
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded { verified: true },
            ))
        },
        DaemonVintage::Current,
    ));
    let registration = rich_fixture::registration();
    let send = |path: &str, session: &str, now| {
        run_registered(
            &registration,
            Some("PreToolUse"),
            &tool(session, path),
            Some(iso.state_root()),
            now,
            (&budget(), &crate::app::SystemClock::new()),
            |_| {
                Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.to_owned()]),
                ))
            },
        )
    };
    assert_eq!(send(&first, "s", NOW), Delivery::Sent(Some(true)));
    let captured = notes.lock().unwrap();
    assert!(
        matches!(&captured[0], Command::HarnessEvidenceV2(_)),
        "advertised v2 must select its own command: {:?}",
        captured[0]
    );
    drop(captured);
    assert_eq!(send(&first, "s", NOW + 1), Delivery::Suppressed);
    assert_eq!(send(&second, "s", NOW + 2), Delivery::Sent(Some(true)));
    assert_eq!(send(&first, "s2", NOW + 3), Delivery::Sent(Some(true)));
    assert_eq!(
        send(&first, "s", NOW + HEARTBEAT_MS),
        Delivery::Sent(Some(true))
    );
    assert_eq!(notes.lock().unwrap().len(), 4);
}

// Kills domain-shared suppression, raw payload projection, and authority-side calls.
#[test]
fn rich_observer_domains_are_separate_and_send_only_bounded_metadata() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-rich-domains");
    let path = transcript(&iso, "2.1.286");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let record = captured.clone();
    let client = Arc::new(CountingLocalClient::scripted(
        move |command| {
            command.validate().unwrap();
            record.lock().unwrap().push(command.clone());
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded { verified: false },
            ))
        },
        DaemonVintage::Current,
    ));
    let registration = rich_fixture::registration();
    let send = |event: &str, bridge: bool, now| {
        let mut payload: Value = serde_json::from_slice(&tool("s", &path)).unwrap();
        payload["bridge"] = bridge.into();
        payload["hook_event_name"] = event.into();
        payload["source"] = "startup".into();
        payload["tool_input"] = serde_json::json!({"command": "PRIVATE_BODY"});
        run_registered(
            &registration,
            Some(event),
            &serde_json::to_vec(&payload).unwrap(),
            Some(iso.state_root()),
            now,
            (&budget(), &crate::app::SystemClock::new()),
            |_| {
                Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.into()]),
                ))
            },
        )
    };
    assert_eq!(send("PreToolUse", false, NOW), Delivery::Sent(Some(false)));
    assert_eq!(send("PreToolUse", false, NOW + 1), Delivery::Suppressed);
    assert_eq!(
        send("PreToolUse", true, NOW + 2),
        Delivery::Sent(Some(false))
    );
    assert_eq!(
        send("SessionStart", false, NOW + 3),
        Delivery::Sent(Some(false))
    );
    assert_eq!(
        send("SessionStart", false, NOW + 4),
        Delivery::Sent(Some(false))
    );
    let notes = captured.lock().unwrap();
    assert_eq!(notes.len(), 4);
    let Command::HarnessEvidenceV2(shape) = &notes[0] else {
        panic!("no legacy projection")
    };
    let Command::HarnessEvidenceV2(envelope) = &notes[1] else {
        panic!("no legacy projection")
    };
    assert_eq!(shape.domain, "shape");
    assert_eq!(
        shape.origin,
        crate::harness::evidence::EvidenceOrigin::NativeShapeObservation
    );
    assert_eq!(envelope.domain, "envelope");
    assert_eq!(
        envelope.origin,
        crate::harness::evidence::EvidenceOrigin::BridgeEnvelope
    );
    assert_ne!(shape.contract_id, envelope.contract_id);
    let serialized = serde_json::to_string(&*notes).unwrap();
    for forbidden in [
        "PRIVATE_BODY",
        "tool_input",
        "transcript_path",
        "tool_use_id",
    ] {
        assert!(!serialized.contains(forbidden), "wire leaked {forbidden}");
    }
    assert_eq!(
        std::fs::read_dir(v2_gate_dir(iso.state_root()))
            .unwrap()
            .count(),
        2
    );
    assert!(
        !gate_dir(iso.state_root()).exists(),
        "rich observations must never rewrite legacy gates"
    );
    for entry in std::fs::read_dir(v2_gate_dir(iso.state_root())).unwrap() {
        let meta = entry.unwrap().metadata().unwrap();
        assert!(meta.len() <= MAX_GATE_BYTES);
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    // Script accepts only evidence; fixture panics for admission, native probe, decode and offer encoding.
    assert_eq!(client.total_calls(), 4);
}

// Kills old-daemon downgrade and any local advancement after wrong/refused/expired replies.
#[test]
fn rich_missing_capability_and_failed_transport_never_close_the_retry_gate() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-rich-retry");
    let path = transcript(&iso, "2.1.286");
    let registration = rich_fixture::registration();
    let clock = crate::app::SystemClock::new();
    let good = Arc::new(CountingLocalClient::scripted(
        |_| {
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded { verified: true },
            ))
        },
        DaemonVintage::Current,
    ));
    for caps in [
        Capabilities::none(),
        Capabilities::from_list([HARNESS_EVIDENCE.into()]),
    ] {
        assert_eq!(
            run_registered(
                &registration,
                Some("PreToolUse"),
                &tool("s", &path),
                Some(iso.state_root()),
                NOW,
                (&budget(), &clock),
                |_| Some((good.clone() as Arc<dyn LocalClient>, caps))
            ),
            Delivery::Unsupported
        );
    }
    assert_eq!(
        good.total_calls(),
        0,
        "rich adapter must never send a legacy command"
    );
    for reply in [
        Ok(CommandResult::HarnessEvidenceRecorded { verified: true }),
        Err(ApiError::new(ErrorCode::Unsupported, "refused")),
        Err(ApiError::new(ErrorCode::DeadlineExceeded, "timed out")),
    ] {
        let client = Arc::new(CountingLocalClient::scripted(
            move |_| reply.clone(),
            DaemonVintage::Current,
        ));
        for now in [NOW, NOW + 1] {
            assert_eq!(
                run_registered(
                    &registration,
                    Some("PreToolUse"),
                    &tool("s", &path),
                    Some(iso.state_root()),
                    now,
                    (&budget(), &clock),
                    |_| Some((
                        client.clone() as Arc<dyn LocalClient>,
                        Capabilities::from_list([HARNESS_EVIDENCE_V2.into()])
                    ))
                ),
                Delivery::Sent(None)
            );
        }
        assert_eq!(client.total_calls(), 2, "failed send must retry");
        assert!(!v2_gate_dir(iso.state_root()).exists());
    }
    let cancelled = budget();
    let token = cancelled.cancellation.clone();
    let late = Arc::new(CountingLocalClient::scripted(
        move |_| {
            token.cancel();
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded { verified: true },
            ))
        },
        DaemonVintage::Current,
    ));
    assert_eq!(
        run_registered(
            &registration,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            NOW,
            (&cancelled, &clock),
            |_| Some((
                late as Arc<dyn LocalClient>,
                Capabilities::from_list([HARNESS_EVIDENCE_V2.into()])
            ))
        ),
        Delivery::Sent(None)
    );
    assert!(!v2_gate_dir(iso.state_root()).exists());
    let expired = CallBudget {
        deadline: MonoInstant(0),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        run_registered(
            &registration,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            NOW,
            (&expired, &clock),
            |_| panic!("expired observer cannot connect")
        ),
        Delivery::Unavailable
    );
}

// Kills trusting a gate copied from another immutable key or a foreign version.
#[test]
fn v2_gate_rejects_key_replacement_and_legacy_bytes() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-rich-key");
    let path = transcript(&iso, "2.1.286");
    let registration = rich_fixture::registration();
    let client = Arc::new(CountingLocalClient::scripted(
        |_| {
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded { verified: true },
            ))
        },
        DaemonVintage::Current,
    ));
    let send = || {
        run_registered(
            &registration,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            NOW,
            (&budget(), &crate::app::SystemClock::new()),
            |_| {
                Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.into()]),
                ))
            },
        )
    };
    assert_eq!(send(), Delivery::Sent(Some(true)));
    let file = std::fs::read_dir(v2_gate_dir(iso.state_root()))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let bytes = std::fs::read(&file).unwrap();
    for (field, value) in [
        ("harness", "codex"),
        ("domain", "other"),
        ("origin", "bridge_envelope"),
        ("contract_id", "0123456789abcdef"),
        ("session_id", "other-session"),
        ("unavailable_reason", "no identity"),
    ] {
        let mut stored: Value = serde_json::from_slice(&bytes).unwrap();
        stored["key"][field] = value.into();
        std::fs::write(&file, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(
            send(),
            Delivery::Sent(Some(true)),
            "copied {field} gate cannot suppress another observation"
        );
    }
    for bad in [
        serde_json::to_vec(&GateState {
            verified: true,
            ..Default::default()
        })
        .unwrap(),
        vec![b' '; MAX_GATE_BYTES as usize + 1],
    ] {
        std::fs::write(&file, bad).unwrap();
        assert_eq!(send(), Delivery::Sent(Some(true)));
    }
    let mut old: Value = serde_json::from_slice(&bytes).unwrap();
    old["version"] = 1.into();
    std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(send(), Delivery::Sent(Some(true)));
}

// Kills a legacy sender bypassing its resolved adapter runtime attribution.
#[test]
fn legacy_projection_uses_the_resolved_adapter_attribution() {
    let iso = TestIsolation::new("hev-adapter-attribution");
    let path = transcript(&iso, "2.1.286");
    let registration = crate::harness::registry::Registration::new(&rich_fixture::Adapter(1));
    let note = Arc::new(Mutex::new(None));
    let captured = note.clone();
    let client = Arc::new(CountingLocalClient::scripted(
        move |command| {
            *captured.lock().unwrap() = Some(command.clone());
            Ok(CommandResult::HarnessEvidenceRecorded { verified: false })
        },
        DaemonVintage::Current,
    ));
    assert_eq!(
        run_registered(
            &registration,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            NOW,
            (&budget(), &crate::app::SystemClock::new()),
            |_| Some((
                client as Arc<dyn LocalClient>,
                Capabilities::from_list([HARNESS_EVIDENCE.into()])
            ))
        ),
        Delivery::Sent(Some(false))
    );
    let recorded = note.lock().unwrap();
    let Some(Command::HarnessEvidence(evidence)) = recorded.as_ref() else {
        panic!("legacy projection missing")
    };
    assert_eq!(evidence.version.as_deref(), Some("9.9.9"));
}

// Kills a late successful response being remembered after the invocation expired.
#[test]
fn legacy_expired_advisory_result_keeps_the_retry_open() {
    let iso = TestIsolation::new("hev-legacy-expired");
    let path = transcript(&iso, "2.1.286");
    let timing = budget();
    let token = timing.cancellation.clone();
    let client = Arc::new(CountingLocalClient::scripted(
        move |_| {
            token.cancel();
            Ok(CommandResult::HarnessEvidenceRecorded { verified: true })
        },
        DaemonVintage::Current,
    ));
    let registration = registration_for(Harness::Claude).unwrap();
    assert_eq!(
        run_registered(
            registration,
            Some("PreToolUse"),
            &tool("s", &path),
            Some(iso.state_root()),
            NOW,
            (&timing, &crate::app::SystemClock::new()),
            |_| Some((
                client as Arc<dyn LocalClient>,
                Capabilities::from_list([HARNESS_EVIDENCE.into()])
            ))
        ),
        Delivery::Sent(None)
    );
    assert!(!gate_dir(iso.state_root()).exists());
}

// Kills discarded adapter facts and accepting forged/unbounded/undeclared qualification tokens.
#[test]
fn rich_qualification_producer_reaches_client_and_invalid_facts_never_send() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-rich-qualified");
    let path = transcript(&iso, "2.1.286");
    for mode in [2, 3, 4, 5, 6, 7] {
        let registration = crate::harness::registry::Registration::new(Box::leak(Box::new(
            rich_fixture::Adapter(mode),
        )));
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| {
                let Command::HarnessEvidenceV2(note) = command else {
                    panic!("rich qualifier sent legacy evidence")
                };
                if !matches!(mode, 3..=6) {
                    assert_eq!(
                        note.qualifications,
                        if mode == 2 {
                            vec!["same_runtime".to_string()]
                        } else {
                            vec![]
                        }
                    );
                }
                Ok(CommandResult::HarnessEvidenceV2Recorded(
                    HarnessEvidenceV2Recorded {
                        verified: mode == 2,
                    },
                ))
            },
            DaemonVintage::Current,
        ));
        let delivery = run_registered(
            &registration,
            Some("PreToolUse"),
            &tool(&format!("s{mode}"), &path),
            Some(iso.state_root()),
            NOW,
            (&budget(), &crate::app::SystemClock::new()),
            |_| {
                Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.into()]),
                ))
            },
        );
        if matches!(mode, 3..=6) {
            assert_eq!(delivery, Delivery::Unsupported);
            assert_eq!(
                client.total_calls(),
                0,
                "invalid facts must refuse before send"
            );
        } else {
            assert_eq!(delivery, Delivery::Sent(Some(mode == 2)));
        }
    }
}

// Kills an unqualified observation spending the required qualified milestone's retry.
#[test]
fn absent_qualification_cannot_suppress_a_later_qualified_milestone() {
    use crate::protocol::{capabilities::HARNESS_EVIDENCE_V2, results::HarnessEvidenceV2Recorded};
    let iso = TestIsolation::new("hev-rich-qualification-retry");
    let path = transcript(&iso, "2.1.286");
    let client = Arc::new(CountingLocalClient::scripted(
        |command| {
            let Command::HarnessEvidenceV2(note) = command else {
                panic!("unexpected non-evidence command")
            };
            Ok(CommandResult::HarnessEvidenceV2Recorded(
                HarnessEvidenceV2Recorded {
                    verified: note.qualifications == ["same_runtime"],
                },
            ))
        },
        DaemonVintage::Current,
    ));
    for (mode, expected) in [(7, false), (2, true)] {
        let registration = crate::harness::registry::Registration::new(Box::leak(Box::new(
            rich_fixture::Adapter(mode),
        )));
        let mut payload: Value = serde_json::from_slice(&tool("s", &path)).unwrap();
        payload["qualifications"] = serde_json::json!(["same_runtime"]); // ignored JSON claim
        assert_eq!(
            run_registered(
                &registration,
                Some("PreToolUse"),
                &serde_json::to_vec(&payload).unwrap(),
                Some(iso.state_root()),
                NOW,
                (&budget(), &crate::app::SystemClock::new()),
                |_| Some((
                    client.clone() as Arc<dyn LocalClient>,
                    Capabilities::from_list([HARNESS_EVIDENCE_V2.into()])
                ))
            ),
            Delivery::Sent(Some(expected))
        );
    }
    assert_eq!(client.total_calls(), 2);
}
