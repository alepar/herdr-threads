use super::*;
use crate::cli::{CliAction, RunError, commands::parse_argv};
use crate::protocol::watch::{
    WatchItem, WatchLine, WatchStatus, WatchStatusReason, WatchStatusState,
};

fn action(argv: &[&str]) -> Result<CliAction, crate::protocol::results::ApiError> {
    parse_argv(argv.iter().copied()).map(|parsed| parsed.action)
}

#[test]
fn parses_watch_argv() {
    assert_eq!(
        action(&[
            "herdr-threads",
            "watch",
            "--harness",
            "claude",
            "--session",
            "sess-1"
        ])
        .unwrap(),
        CliAction::Watch(WatchRequest {
            harness: Harness::Claude,
            session: "sess-1".into()
        })
    );
}

#[test]
fn parses_watch_ack_argv() {
    assert_eq!(
        action(&[
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "sess-1",
            "--via",
            "append",
            "m1",
            "m2"
        ])
        .unwrap(),
        CliAction::WatchAck(WatchAckRequest {
            session: "sess-1".into(),
            via: ModDeliveryVia::Append,
            messages: vec![MessageId::new("m1"), MessageId::new("m2")],
        })
    );
}

#[test]
fn rejects_bad_watch_argv() {
    for argv in [
        vec!["herdr-threads", "watch"],
        vec![
            "herdr-threads",
            "watch",
            "--harness",
            "codex",
            "--session",
            "s",
        ],
        vec![
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "s",
            "--via",
            "context",
        ],
        vec![
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "s",
            "--via",
            "later",
            "m1",
        ],
    ] {
        assert!(action(&argv).is_err(), "{argv:?}");
    }
}

#[test]
fn watch_stub_prints_one_status_line_and_exits_3() {
    let mut buf = Vec::new();
    let result = crate::cli::run_in_pane(
        [
            "herdr-threads",
            "watch",
            "--harness",
            "claude",
            "--session",
            "s1",
        ],
        None,
        &mut buf,
    );
    assert!(matches!(result, Err(RunError::Exit(3))));
    let text = String::from_utf8(buf).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.ends_with('\n'));
    let line: WatchLine = serde_json::from_str(text.trim_end()).unwrap();
    assert_eq!(line.id, "status:0");
    assert_eq!(
        line.item,
        WatchItem::Status(WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::Unsupported),
            exit: Some(3),
        })
    );
}

#[test]
fn watch_ack_stub_exits_nonzero_without_lines() {
    let mut buf = Vec::new();
    let error = crate::cli::run_in_pane(
        [
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "s1",
            "--via",
            "submit",
            "m1",
        ],
        None,
        &mut buf,
    )
    .unwrap_err();
    assert_ne!(error.exit_code(), 0);
    assert!(buf.is_empty());
}

#[test]
fn stub_mod_files_follow_the_layout() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/claude/mod");
    let read = |path: &str| std::fs::read_to_string(root.join(path)).unwrap();
    let plugin: serde_json::Value =
        serde_json::from_str(&read(".claude-plugin/plugin.json")).unwrap();
    assert_eq!(plugin["name"], "herdr-threads");
    assert_eq!(plugin["types"], "./types/index.d.ts");
    let types = read("types/index.d.ts");
    assert!(types.contains("PluginState"));
    assert!(types.contains("'herdr-threads'"));
    let hooks: serde_json::Value = serde_json::from_str(&read("hooks/hooks.json")).unwrap();
    assert_eq!(hooks, serde_json::json!({"modules":["./register.js"]}));
    assert!(read("hooks/register.js").contains("export const register"));
}
