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
fn codex_fresh_session_is_attributed_from_the_rollout_head() {
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
        assert_eq!(note.version.as_deref(), Some("0.159.3"), "{note:?}");
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
