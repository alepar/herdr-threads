//! Native TUI captures, synthetic provider, sanitized private project paths.
use crate::harness::{adapter::ComposerPolicy, composer::ComposerRead, registry::builtins, sgr};

fn policy() -> &'static dyn ComposerPolicy {
    builtins()
        .by_id(builtins().agent("pi").unwrap())
        .unwrap()
        .composer_policy()
        .expect("pi needs a native composer policy")
}
macro_rules! capture {
    ($name:literal, $suffix:literal) => {
        include_str!(concat!("../fixtures/pi/composer/", $name, $suffix))
    };
}
fn styled(text: &str) -> ComposerRead {
    policy()
        .read_styled(&sgr::parse(text).expect("captured ANSI"), None)
        .unwrap_or(ComposerRead::Unreadable)
}

#[test]
fn pi_composer_requires_styled_empty_default_editor() {
    let read = policy().read(capture!("idle-empty", ".txt"), None);
    assert_ne!(
        read,
        ComposerRead::Empty,
        "plain blanks cannot distinguish spaces"
    );
    assert!(policy().wants_styled(&read));
    assert_eq!(styled(capture!("idle-empty", ".ansi")), ComposerRead::Empty);
    assert_eq!(
        policy().read_styled(
            &sgr::parse(capture!("tmux-empty", ".ansi")).unwrap(),
            Some(120)
        ),
        Some(ComposerRead::Empty)
    );
    let mixed = capture!("tmux-empty", ".ansi").replacen(&"─".repeat(120), &"─".repeat(119), 1);
    assert_eq!(
        policy().read_styled(&sgr::parse(&mixed).unwrap(), Some(120)),
        None
    );
    assert_eq!(
        policy().read_styled(
            &sgr::parse(capture!("tmux-empty", ".ansi")).unwrap(),
            Some(118)
        ),
        None
    );
    assert_eq!(
        policy().screen_blocked(capture!("idle-empty", ".txt")),
        Some(false)
    );
    assert_eq!(
        policy().screen_turn_running(capture!("idle-empty", ".txt")),
        Some(false)
    );
    assert!(!policy().clears_unsubmitted_wake());
}

#[test]
fn pi_composer_refuses_visible_whitespace_multiline_and_wrapped_drafts() {
    for text in [
        capture!("idle-draft", ".ansi"),
        capture!("one-space-end", ".ansi"),
        capture!("four-spaces-end", ".ansi"),
        capture!("multiline", ".ansi"),
        capture!("empty-multiline", ".ansi"),
        capture!("wrapped-end", ".ansi"),
        capture!("wrapped-home", ".ansi"),
    ] {
        assert_ne!(styled(text), ComposerRead::Empty);
    }
}

#[test]
fn pi_composer_refuses_builtin_and_inline_selectors_busy_or_unknown_screens() {
    for text in [
        capture!("builtin-model-picker", ".txt"),
        capture!("builtin-settings-picker", ".txt"),
        capture!("extension-inline-select", ".txt"),
        "unrecognized editor",
    ] {
        assert_eq!(policy().read(text, None), ComposerRead::Unreadable);
        assert_ne!(policy().screen_blocked(text), Some(false));
    }
    for text in [capture!("busy-text", ".txt"), capture!("busy-tool", ".txt")] {
        assert_eq!(policy().read(text, None), ComposerRead::Unreadable);
        assert_eq!(policy().screen_turn_running(text), Some(true));
    }
    let idle = capture!("idle-empty", ".txt");
    assert_eq!(
        policy().read(&format!("{idle}\nforeign footer"), None),
        ComposerRead::Unreadable
    );
}

#[test]
fn pi_composer_captures_screen_observability_limits_without_inventing_a_veto() {
    // These real states have the same composer bytes as idle. Qualification and
    // TRUST-POLICY disclose the limits; a terminal parser has no missing text to recover.
    for text in [
        capture!("one-space-home", ".ansi"),
        capture!("four-spaces-home", ".ansi"),
        capture!("extension-overlay", ".ansi"),
    ] {
        assert_eq!(styled(text), ComposerRead::Empty);
    }
}

#[test]
fn pi_composer_accepts_captured_postcompaction_unknown_usage_without_weakening_guards() {
    let plain = capture!("postcompact-unknown", ".txt");
    let ansi = capture!("postcompact-unknown", ".ansi");
    assert_eq!(policy().screen_blocked(plain), Some(false));
    assert_eq!(policy().screen_turn_running(plain), Some(false));
    assert!(policy().wants_styled(&policy().read(plain, Some(120))));
    assert_eq!(styled(ansi), ComposerRead::Empty);
    assert_ne!(
        styled(&ansi.replace("?/100k", "?%/100k")),
        ComposerRead::Empty
    );
    assert_ne!(
        styled(&ansi.replace("?/100k", "unknown/100k")),
        ComposerRead::Empty
    );
    for text in [
        capture!("idle-draft", ".ansi"),
        capture!("one-space-end", ".ansi"),
        capture!("four-spaces-end", ".ansi"),
        capture!("multiline", ".ansi"),
        capture!("empty-multiline", ".ansi"),
        capture!("wrapped-home", ".ansi"),
        capture!("builtin-model-picker", ".ansi"),
        capture!("builtin-settings-picker", ".ansi"),
        capture!("extension-inline-select", ".ansi"),
    ] {
        assert_ne!(
            styled(&text.replace("2.0%/100k", "?/100k")),
            ComposerRead::Empty
        );
    }
    let malformed = ansi.replacen(&"─".repeat(119), &"─".repeat(118), 1);
    assert_ne!(styled(&malformed), ComposerRead::Empty);
}
