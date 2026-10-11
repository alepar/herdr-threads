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
    use std::collections::BTreeSet;

    /// The callback names the embedded Hermes bridge registers: the names in its `HOOKS` tuple (the
    /// bridge registers every one through `ctx.register_hook(name, ...)` in a loop).
    fn hermes_bridge_hooks() -> BTreeSet<String> {
        let source =
            String::from_utf8(include_bytes!("../../integrations/hermes/__init__.py").to_vec())
                .unwrap();
        let start = source.find("\nHOOKS = (").expect("bridge HOOKS tuple") + "\nHOOKS = (".len();
        let end = start + source[start..].find(')').expect("bridge HOOKS tuple end");
        source[start..end]
            .split(',')
            .map(|name| name.trim().trim_matches('"'))
            .filter(|name| !name.is_empty() && !name.starts_with('#'))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn installed_groups_equal_the_required_sets() {
        let sorted = |names: &[&str]| -> Vec<String> {
            let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            names.sort_unstable();
            names
        };
        // Claude installs exactly the required set.
        let claude: Vec<&str> = crate::harness::claude::declared_hook_groups("ht hook claude")
            .into_iter()
            .map(|(event, _)| event)
            .collect();
        assert_eq!(sorted(&claude), sorted(CLAUDE_REQUIRED_HOOK_EVENTS));
        // So does Codex: every owned hook is required and every required one is owned.
        let codex: Vec<&str> = crate::harness::codex::DECLARATION
            .owned_hooks
            .iter()
            .map(|hook| hook.event)
            .collect();
        assert_eq!(sorted(&codex), sorted(CODEX_REQUIRED_HOOK_EVENTS));
        // And Hermes: the bridge registers exactly the required callbacks. `on_session_end`
        // fires every turn, so it is never one of them.
        let bridge = hermes_bridge_hooks();
        let required: BTreeSet<String> = HERMES_REQUIRED_HOOK_EVENTS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(bridge, required);
        assert!(!required.contains("on_session_end"));
        assert_eq!(
            required_hook_events("claude"),
            Some(CLAUDE_REQUIRED_HOOK_EVENTS)
        );
        assert_eq!(required_hook_events("human"), None);
    }
}
