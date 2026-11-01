//! N2-turn: per-binding hook turn state. Edges come from each harness's own hook entry (the
//! registered decode, then `turn_state_edge`), the way `cli::hook` reports them.
use herdr_threads::{
    harness::{
        adapter::{AdmissionRequest, HookInput, InstallObservation, TurnEdge},
        registry::builtins,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint},
        commands::TurnStateReport,
        ids::{AgentId, ExecutionId, HostLocator, HostTargetId, NativeSessionId},
    },
    protocol::{ids::SeatId, time::CallBudget, time::MonoInstant},
    service::dispatch::wake_hint_subject,
    service::turn_state::{SCREEN_READ_GAP_MS, STALE_BUSY_MS, TurnKey, TurnState, TurnStateBook},
};

/// The edge `harness`'s hook entry reports for `payload` registered as `event`.
fn hook_edge(harness: &str, event: &str, payload: serde_json::Value) -> Option<TurnEdge> {
    let registry = builtins();
    let registration = registry.by_id(registry.agent(harness).unwrap()).unwrap();
    let admitted = registration
        .admit(
            &AdmissionRequest {
                installed: InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &CallBudget {
                deadline: MonoInstant(30_000),
                cancellation: Default::default(),
            },
        )
        .unwrap();
    let decoded = registration
        .decode(
            &admitted,
            &HookInput {
                registered_event: Some(event.into()),
                bytes: serde_json::to_vec(&payload).unwrap(),
            },
        )
        .unwrap();
    registration.turn_state_edge(&decoded)
}

fn key() -> TurnKey {
    TurnKey {
        seat: SeatId::new("s"),
        generation: 1,
    }
}

fn codex(event: &str, extra: serde_json::Value) -> Option<TurnEdge> {
    let mut payload = serde_json::json!({
        "hook_event_name": event, "session_id": "sess", "turn_id": "t1"});
    payload
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    hook_edge("codex", event, payload)
}

#[test]
fn codex_turn_start_busy_stop_idle() {
    let book = TurnStateBook::default();
    assert_eq!(
        codex("UserPromptSubmit", serde_json::json!({})),
        Some(TurnEdge::Busy)
    );
    assert_eq!(
        codex("Stop", serde_json::json!({"stop_hook_active": false})),
        Some(TurnEdge::Idle)
    );
    assert_eq!(
        codex(
            "PreToolUse",
            serde_json::json!({"tool_name": "Bash", "tool_use_id": "u"})
        ),
        None
    );
    book.record(&key(), TurnEdge::Busy, 0);
    assert_eq!(book.state(&key()), TurnState::Busy);
    book.record(&key(), TurnEdge::Idle, 1);
    assert_eq!(book.state(&key()), TurnState::Idle);
}

#[test]
fn agy_turn_start_busy_stop_idle() {
    let payload = |event: &str| {
        serde_json::json!({"hook_event_name": event,
            "conversationId": "0b3ae1c4-77a0-4f55-9d5b-1d9a2e6f0c11"})
    };
    assert_eq!(
        hook_edge("agy", "PreInvocation", payload("PreInvocation")),
        Some(TurnEdge::Busy)
    );
    assert_eq!(
        hook_edge("agy", "Stop", payload("Stop")),
        Some(TurnEdge::Idle)
    );
    assert_eq!(
        hook_edge("agy", "SessionStart", payload("SessionStart")),
        None
    );
}

#[test]
fn claude_stop_failure_clears_busy() {
    let payload = |event: &str| serde_json::json!({"hook_event_name": event, "session_id": "sess"});
    let book = TurnStateBook::default();
    let busy = hook_edge("claude", "UserPromptSubmit", payload("UserPromptSubmit"));
    assert_eq!(busy, Some(TurnEdge::Busy));
    book.record(&key(), busy.unwrap(), 0);
    assert_eq!(book.state(&key()), TurnState::Busy);
    let failed = hook_edge("claude", "StopFailure", payload("StopFailure"));
    assert_eq!(failed, Some(TurnEdge::Idle));
    book.record(&key(), failed.unwrap(), 1);
    assert_eq!(book.state(&key()), TurnState::Idle);
    assert_eq!(
        hook_edge("claude", "Stop", payload("Stop")),
        Some(TurnEdge::Idle)
    );
}

#[test]
fn unknown_after_restart_never_idle() {
    let book = TurnStateBook::default();
    book.record(&key(), TurnEdge::Idle, 0);
    assert_eq!(book.state(&key()), TurnState::Idle);
    // A restarted daemon has a fresh book: nothing is known, and nothing clears it but a hook.
    let restarted = TurnStateBook::default();
    assert_eq!(restarted.state(&key()), TurnState::Unknown);
    assert!(!restarted.observe_screen(&key(), Some(false), 10 * STALE_BUSY_MS));
    assert!(!restarted.observe_screen(&key(), Some(false), 20 * STALE_BUSY_MS));
    assert_eq!(restarted.state(&key()), TurnState::Unknown);
    restarted.record(&key(), TurnEdge::Busy, 0);
    assert_eq!(restarted.state(&key()), TurnState::Busy);
}

#[test]
fn screen_idle_accepted_only_when_busy() {
    let book = TurnStateBook::default();
    assert!(!book.record(&key(), TurnEdge::ScreenIdle, 0), "unknown");
    assert_eq!(book.state(&key()), TurnState::Unknown);
    book.record(&key(), TurnEdge::Idle, 0);
    assert!(!book.record(&key(), TurnEdge::ScreenIdle, 1), "idle");
    book.record(&key(), TurnEdge::Busy, 2);
    assert!(book.record(&key(), TurnEdge::ScreenIdle, 3), "busy");
    assert_eq!(book.state(&key()), TurnState::Idle);
}

#[test]
fn stale_busy_clears_after_two_spaced_idle_reads() {
    let book = TurnStateBook::default();
    book.record(&key(), TurnEdge::Busy, 0);
    // Younger than the stale bound: reads do not even count.
    assert!(!book.observe_screen(&key(), Some(false), STALE_BUSY_MS - 1));
    let first = STALE_BUSY_MS;
    assert!(!book.observe_screen(&key(), Some(false), first));
    // A second read inside the gap, a running read, or an unreadable one never clears.
    assert!(!book.observe_screen(&key(), Some(false), first + SCREEN_READ_GAP_MS - 1));
    assert!(!book.observe_screen(&key(), Some(true), first + SCREEN_READ_GAP_MS));
    assert!(!book.observe_screen(&key(), Some(false), first + SCREEN_READ_GAP_MS + 1));
    assert_eq!(book.state(&key()), TurnState::Busy);
    // The running read restarted the count: two idle reads a gap apart now clear it.
    assert!(book.observe_screen(&key(), Some(false), first + 2 * SCREEN_READ_GAP_MS + 1));
    assert_eq!(book.state(&key()), TurnState::Idle);
    // A harness whose recognizer answers None keeps a stale busy.
    book.record(&key(), TurnEdge::Busy, 0);
    for step in 0..4 {
        assert!(!book.observe_screen(&key(), None, STALE_BUSY_MS + step * SCREEN_READ_GAP_MS));
    }
    assert_eq!(book.state(&key()), TurnState::Busy);
}

#[test]
fn codex_continued_stop_stays_busy_then_idle() {
    let book = TurnStateBook::default();
    // UserPromptSubmit: busy.
    book.record(
        &key(),
        codex("UserPromptSubmit", serde_json::json!({})).unwrap(),
        0,
    );
    // The first Stop was answered with a continuation (decision:block): the turn goes on.
    book.answered_turn_end(&key(), true, 10);
    assert_eq!(book.state(&key()), TurnState::Busy);
    // A p0 arriving now finds the binding busy: the wake condition (idle only) fails.
    assert_ne!(book.state(&key()), TurnState::Idle);
    // The Stop that ends the continued turn (stop_hook_active) is the idle edge.
    let ending = codex("Stop", serde_json::json!({"stop_hook_active": true})).unwrap();
    assert_eq!(ending, TurnEdge::Idle);
    book.record(&key(), ending, 20);
    assert_eq!(book.state(&key()), TurnState::Idle);
}

#[test]
fn codex_continued_stop_with_a_missed_start_is_busy() {
    let book = TurnStateBook::default();
    book.answered_turn_end(&key(), true, 10);
    assert_eq!(book.state(&key()), TurnState::Busy);
}

#[test]
fn codex_stop_without_continuation_idle() {
    let book = TurnStateBook::default();
    book.record(&key(), TurnEdge::Busy, 0);
    book.answered_turn_end(&key(), false, 10);
    assert_eq!(book.state(&key()), TurnState::Idle);
}

fn claim(role: CallerRole, liveness: Option<LivenessFingerprint>) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        host: HostLocator::parse("/h.sock").unwrap(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("sess"),
        execution: ExecutionId::new("e"),
        target: HostTargetId::new("alice"),
        agent_id: None,
        liveness,
    }
}

// Kills: verifying the hint for the reporting hook process (the peer) when the claim fingerprints
// the harness, so the binding dies with the hook; no fallback for a harness that cannot be
// fingerprinted; a subagent claim or a claim with no peer pid holding a wake.
#[test]
fn turn_edge_reestablishes_the_hint_after_a_restart() {
    let harness = LivenessFingerprint {
        pid: 4242,
        start: 9,
    };
    let top = claim(CallerRole::TopLevel, Some(harness));
    assert_eq!(wake_hint_subject(&top, Some(77)), Some(4242));
    let unfingerprinted = claim(CallerRole::TopLevel, None);
    assert_eq!(wake_hint_subject(&unfingerprinted, Some(77)), Some(77));
    assert_eq!(wake_hint_subject(&unfingerprinted, None), None);
    let subagent = claim(CallerRole::Subagent, Some(harness));
    assert_eq!(wake_hint_subject(&subagent, Some(77)), None);
    let mut with_agent = top.clone();
    with_agent.agent_id = Some(AgentId::new("a"));
    assert_eq!(wake_hint_subject(&with_agent, Some(77)), None);
}

// Kills: serializing an absent hint (an older daemon's `deny_unknown_fields` would refuse the
// report), or requiring the field on decode (an older hook's report would not decode).
#[test]
fn turn_state_report_without_hint_keeps_its_bytes() {
    let report = TurnStateReport {
        claim: claim(CallerRole::TopLevel, None),
        edge: TurnEdge::Busy,
        tmux_hint: None,
    };
    let text = serde_json::to_string(&report).unwrap();
    assert!(!text.contains("tmux_hint"), "{text}");
    assert_eq!(
        serde_json::from_str::<TurnStateReport>(&text).unwrap(),
        report
    );
    let hinted = TurnStateReport {
        tmux_hint: Some("tmux:/s:1:%1".into()),
        ..report
    };
    let text = serde_json::to_string(&hinted).unwrap();
    assert!(text.contains("\"tmux_hint\":\"tmux:/s:1:%1\""), "{text}");
    assert_eq!(
        serde_json::from_str::<TurnStateReport>(&text).unwrap(),
        hinted
    );
}
