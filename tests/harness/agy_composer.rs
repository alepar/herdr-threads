//! agy composer policy over recorded `agent read --source detection` captures.
//! Included from `src/harness/agy/composer.rs`; the recipe is `tests/fixtures/agy/composer/README.md`.
use super::*;
use crate::{harness::registry::builtins, protocol::authority::Harness};

macro_rules! capture {
    ($name:literal) => {
        include_str!(concat!(
            "../fixtures/agy/composer/",
            $name,
            ".read-detection.txt"
        ))
    };
}

/// The captured pane was 119 columns wide (its rules are 119 `─`).
const WIDTH: Option<u16> = Some(119);

fn read(screen: &str) -> ComposerRead {
    AgyComposer.read(screen, WIDTH)
}

#[test]
fn text_captures_read_to_their_rows() {
    assert_eq!(read(capture!("empty")), ComposerRead::Empty);
    assert_eq!(
        read(capture!("draft")),
        ComposerRead::Text("hello draft".into())
    );
    assert_eq!(
        read(capture!("multiline")),
        ComposerRead::Text("first row\nsecond row".into())
    );
    // ctrl+u cleared the last row's text (an empty row remains), then joined it.
    for name in [
        capture!("multiline-after-ctrl-u-1"),
        capture!("multiline-after-ctrl-u-2"),
    ] {
        assert_eq!(read(name), ComposerRead::Text("first row".into()));
    }
}

#[test]
fn width_unknown_falls_back_to_the_rule() {
    assert_eq!(
        AgyComposer.read(capture!("draft"), None),
        ComposerRead::Text("hello draft".into())
    );
}

#[test]
fn busy_prompt_overlay_and_subagent_screens_are_unreadable() {
    for (name, screen) in [
        ("busy", capture!("busy")),
        ("permission-prompt", capture!("permission-prompt")),
        ("help-overlay", capture!("help-overlay")),
        ("subagent-generating", capture!("subagent-generating")),
        ("subagent-tool-running", capture!("subagent-tool-running")),
    ] {
        assert_eq!(read(screen), ComposerRead::Unreadable, "{name}");
    }
}

#[test]
fn an_agent_row_outside_the_live_region_does_not_count() {
    assert_eq!(read(capture!("old-agent-row")), ComposerRead::Empty);
}

#[test]
fn an_agent_row_without_its_bullet_still_counts() {
    let screen = capture!("subagent-generating").replace("● Agent(", "Agent(");
    assert!(screen.contains("  Agent(self: Probe Child)"));
    assert_eq!(read(&screen), ComposerRead::Unreadable);
}

#[test]
fn typed_agent_text_is_a_draft() {
    let screen = capture!("draft").replace("hello draft", "Agent(x) helper");
    assert_eq!(read(&screen), ComposerRead::Text("Agent(x) helper".into()));
}

#[test]
fn unknown_screens_are_unreadable() {
    for screen in [
        "",
        "plain shell output\n$ ",
        "─────────────────────────\n> x\n─────────────────────────\n",
        // The idle footer is required.
        &capture!("empty").replace("? for shortcuts", "esc to cancel"),
        // More footer rows than a footer or statusline: an overlay below the box.
        &format!("{}one\ntwo\nthree\n", capture!("empty")),
        // The composer's first row must start with the prompt.
        &capture!("draft").replace("> hello draft", "hello draft"),
        // A continuation row must be indented two spaces.
        &capture!("multiline").replace("  second row", "second row"),
    ] {
        assert_eq!(read(screen), ComposerRead::Unreadable, "{screen:?}");
    }
}

/// A user-configured statusline replaces the default footer (ht-rot9).
const STATUSLINE: &str =
    "████░░░░░░░░░░░░░░░░ 18%/1m | Fake Model (Medium) | wk:100%/100% | default | !sbx";

/// `screen` with its last line (agy's default footer) replaced by the statusline.
fn with_statusline(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let (_, body) = lines.split_last().unwrap();
    format!("{}\n{STATUSLINE}\n", body.join("\n"))
}

#[test]
fn a_statusline_footer_reads_like_the_default_one() {
    assert_eq!(read(capture!("statusline-empty")), ComposerRead::Empty);
    assert_eq!(
        read(capture!("statusline-draft")),
        ComposerRead::Text("hello draft".into())
    );
    // The idle composer with agy's footer and the statusline stacked.
    let stacked = format!("{}{STATUSLINE}\n", capture!("empty"));
    assert_eq!(read(&stacked), ComposerRead::Empty);
}

// Catches a running turn read as an empty composer once the `esc to cancel` footer is gone.
#[test]
fn a_spinner_above_the_box_is_a_running_turn_whatever_the_footer() {
    let default_busy = with_statusline(capture!("busy"));
    assert!(!default_busy.contains("esc to cancel"));
    for (name, screen) in [
        ("statusline-busy", capture!("statusline-busy")),
        ("busy with a statusline", default_busy.as_str()),
    ] {
        assert_eq!(read(screen), ComposerRead::Unreadable, "{name}");
        assert_eq!(
            AgyComposer.screen_turn_running(screen),
            Some(true),
            "{name}"
        );
        assert_eq!(AgyComposer.screen_blocked(screen), None, "{name}");
    }
    let running_command =
        capture!("statusline-busy").replace("Generating...", "Running command...");
    assert_eq!(read(&running_command), ComposerRead::Unreadable);
    assert_eq!(
        AgyComposer.screen_turn_running(capture!("statusline-empty")),
        Some(false)
    );
    assert_eq!(
        AgyComposer.screen_blocked(capture!("statusline-empty")),
        Some(false)
    );
}

#[test]
fn overlay_prompt_and_subagent_screens_stay_unreadable_with_a_statusline() {
    assert_eq!(
        read(capture!("statusline-help-overlay")),
        ComposerRead::Unreadable
    );
    for (name, screen) in [
        ("permission-prompt", capture!("permission-prompt")),
        ("subagent-generating", capture!("subagent-generating")),
        ("subagent-tool-running", capture!("subagent-tool-running")),
    ] {
        assert_eq!(
            read(&with_statusline(screen)),
            ComposerRead::Unreadable,
            "{name}"
        );
    }
}

#[test]
fn trailing_whitespace_in_a_row_is_unsafe() {
    let screen = capture!("draft").replace("hello draft", "hello draft  ");
    assert!(matches!(read(&screen), ComposerRead::Unsafe { .. }));
}

#[test]
fn registry_forwards_to_the_agy_policy() {
    let registry = builtins();
    let harness = Harness::Agent(registry.agent("agy").unwrap());
    assert_eq!(
        crate::harness::composer::read_composer(harness, capture!("draft"), WIDTH),
        ComposerRead::Text("hello draft".into())
    );
}

#[test]
fn capabilities_are_none_and_keys_come_from_the_captures() {
    assert_eq!(AgyComposer.capabilities(None), PokeCapabilities::NONE);
    assert_eq!(
        AgyComposer.capabilities(Some("1.3.2")),
        PokeCapabilities::NONE
    );
    assert_eq!(AgyComposer.clear_key(), "ctrl+u");
    assert_eq!(AgyComposer.restore_text("a\nb"), "a\nb");
}

#[test]
fn herdr_reports_the_agy_kind() {
    let list = include_str!("../fixtures/agy/composer/agent-list.txt");
    let list: serde_json::Value = serde_json::from_str(list).unwrap();
    let kind = list["result"]["agents"][0]["agent"].as_str().unwrap();
    assert!(crate::harness::agy::METADATA.host_kinds.contains(&kind));
    assert_eq!(kind, "agy");
}

// Catches a permission prompt read as typeable, or an idle composer read as blocked.
#[test]
fn screen_blocked_recognizes_the_permission_prompt_and_idle_composers() {
    let approval = include_str!("../fixtures/screen_blocked/agy/approval.txt");
    let idle = include_str!("../fixtures/screen_blocked/agy/idle.txt");
    assert_eq!(AgyComposer.screen_blocked(approval), Some(true));
    assert_eq!(AgyComposer.screen_blocked(idle), Some(false));
    assert_eq!(AgyComposer.screen_blocked(capture!("busy")), None);
    assert_eq!(AgyComposer.screen_blocked(capture!("draft")), Some(false));
}

// Catches a generating turn read as idle (a wake typed into it), an idle composer read as running
// (a stale busy never cleared), and a permission prompt read as either.
#[test]
fn screen_turn_running_recognizes_the_cancel_footer() {
    let running = include_str!("../fixtures/screen_turn_running/agy/running.txt");
    let idle = include_str!("../fixtures/screen_turn_running/agy/idle.txt");
    assert_eq!(AgyComposer.screen_turn_running(running), Some(true));
    assert_eq!(AgyComposer.screen_turn_running(idle), Some(false));
    assert_eq!(
        AgyComposer.screen_turn_running(capture!("permission-prompt")),
        None
    );
}
