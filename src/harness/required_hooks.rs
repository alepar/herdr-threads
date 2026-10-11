//! Per-harness hook groups herdr-threads requires once session-keyed bindings land (spec §4
//! Protocol). Setup installs them and `doctor`/`setup-status` flag an install that lacks one.
pub const CLAUDE_REQUIRED_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "PreToolUse",
    "SubagentStart",
    "SubagentStop",
];
pub const CODEX_REQUIRED_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SubagentStart",
    "PreToolUse",
    "SubagentStop",
    "SessionEnd",
];
/// Hermes bridge callbacks. `on_session_end` fires after every turn and is NOT an end event;
/// `on_session_finalize` is the session teardown (spec §3).
pub const HERMES_REQUIRED_HOOK_EVENTS: &[&str] = &[
    "pre_llm_call",
    "post_tool_call",
    "on_session_start",
    "on_session_reset",
    "on_session_finalize",
    "subagent_start",
    "subagent_stop",
];
pub fn required_hook_events(harness: &str) -> Option<&'static [&'static str]> {
    match harness {
        "claude" => Some(CLAUDE_REQUIRED_HOOK_EVENTS),
        "codex" => Some(CODEX_REQUIRED_HOOK_EVENTS),
        "hermes" => Some(HERMES_REQUIRED_HOOK_EVENTS),
        _ => None,
    }
}

/// The stale-install finding of `harness`: `None` when nothing required is missing, else
/// `{harness, missing, fix, detail}` (setup-status and doctor report it; `doctor fix` re-runs setup).
pub fn stale_install_json(harness: &str, missing: &[String]) -> Option<serde_json::Value> {
    if missing.is_empty() {
        return None;
    }
    let fix = format!("herdr-threads setup {harness}");
    Some(serde_json::json!({
        "harness": harness,
        "missing": missing,
        "fix": fix,
        "detail": format!(
            "the installed {harness} integration lacks {}: re-run `{fix}` (agents started before the upgrade also need a restart)",
            missing.join(", ")
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_groups_are_a_subset_of_the_required_sets() {
        // Claude installs exactly the required set; Codex and Hermes stay a subset until ht-54r.11.
        let claude: Vec<&str> = crate::harness::claude::declared_hook_groups("ht hook claude")
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        let mut sorted = claude.clone();
        sorted.sort_unstable();
        let mut required = CLAUDE_REQUIRED_HOOK_EVENTS.to_vec();
        required.sort_unstable();
        assert_eq!(sorted, required);
        for hook in crate::harness::codex::DECLARATION.owned_hooks {
            assert!(
                CODEX_REQUIRED_HOOK_EVENTS.contains(&hook.event),
                "{}",
                hook.event
            );
        }
        assert!(HERMES_REQUIRED_HOOK_EVENTS.contains(&"on_session_finalize"));
        assert!(!HERMES_REQUIRED_HOOK_EVENTS.contains(&"on_session_end"));
        assert_eq!(
            required_hook_events("claude"),
            Some(CLAUDE_REQUIRED_HOOK_EVENTS)
        );
        assert_eq!(required_hook_events("human"), None);
    }
}
