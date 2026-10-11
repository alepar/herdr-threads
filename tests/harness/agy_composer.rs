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
        // A second footer line is not the idle layout.
        &format!("{}extra status line\n", capture!("empty")),
        // The composer's first row must start with the prompt.
        &capture!("draft").replace("> hello draft", "hello draft"),
        // A continuation row must be indented two spaces.
        &capture!("multiline").replace("  second row", "second row"),
    ] {
        assert_eq!(read(screen), ComposerRead::Unreadable, "{screen:?}");
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
