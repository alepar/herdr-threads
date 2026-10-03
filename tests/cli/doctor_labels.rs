//! ht-p03.27: doctor's `hooks.claude.observed` label and the daemon log path.
//! Each test names the mutation it kills.

use super::*;

/// Kills: labelling every non-Supported harness "(cooperative mode)" (the
/// Wave 26 bug), or dropping the label from the one state it describes.
#[test]
fn cooperative_mode_label_only_for_cooperative() {
    let none: [String; 0] = [];
    assert_eq!(
        claude_observed_text(HarnessState::Cooperative, &none, &none),
        CLAUDE_NOT_OBSERVABLE
    );
    assert!(CLAUDE_NOT_OBSERVABLE.contains("(cooperative mode)"));
    for state in [
        HarnessState::Unsupported,
        HarnessState::Supported,
        HarnessState::Unknown,
    ] {
        let text = claude_observed_text(state, &none, &none);
        assert!(!text.contains("cooperative"), "{state:?}: {text}");
    }
    assert_eq!(
        claude_observed_text(HarnessState::Supported, &none, &none),
        "observed"
    );
    assert_eq!(
        claude_observed_text(HarnessState::Unknown, &none, &none),
        "unknown"
    );
}

/// Kills: an Unsupported (refused) claude printing a generic or cooperative
/// line instead of the daemon's own refusal reason, and a not-installed claude
/// losing its reason (it sits in notes, not limitations).
#[test]
fn unsupported_prints_its_own_reason() {
    let refused = vec![
        "host unavailable: server not running".to_owned(),
        "harness claude unsupported: claude 9.9.9: no recipe admits it".to_owned(),
    ];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &refused, &[]),
        "not observable: claude 9.9.9: no recipe admits it"
    );
    let absent = vec![
        "harness claude not installed: no executable `claude` on the daemon's PATH".to_owned(),
    ];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &[], &absent),
        "not observable: no executable `claude` on the daemon's PATH"
    );
    // The codex line must not be mistaken for claude's reason.
    let codex_only = vec!["harness codex unsupported: codex 1.0: refused".to_owned()];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &codex_only, &[]),
        "not observable: the daemon did not admit claude"
    );
}

/// Kills: doctor's text form omitting the daemon log it already knows (the
/// path was only in the JSON report).
#[test]
fn doctor_text_prints_the_daemon_log_path() {
    let report = json!({
        "version": "0",
        "protocol_version": 1,
        "context": {"ok": true, "state_dir": "/s", "source": {}, "host_endpoint": "/h"},
        "state_dir": {"exists": true, "safe": true},
        "instance_dir": "/s/instances/i",
        "daemon_log": "/s/instances/i/logs/daemon.log",
        "daemon": {"state": "degraded"},
        "hooks": {"claude": {"setup": {}, "installed": {}}, "codex": {"setup": {}, "installed": {}}},
        "result": "degraded",
    });
    let text = render_text(&report);
    assert!(
        text.contains("daemon_log: /s/instances/i/logs/daemon.log\n"),
        "{text}"
    );
    let mut no_log = report.clone();
    no_log["daemon_log"] = Value::Null;
    assert!(!render_text(&no_log).contains("daemon_log:"));
}
