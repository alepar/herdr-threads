//! The stale-install finding for an install that lacks a required hook event (spec §4 Protocol).
//! Each adapter declares its own required events (`HarnessAdapter::required_hook_events`); setup
//! installs them and `doctor`/`setup-status` flag an install that lacks one.
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
