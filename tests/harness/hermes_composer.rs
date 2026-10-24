//! Hermes composer policy over recorded Hermes 0.22.5 `agent read` captures.
//! Included from `src/harness/hermes/composer.rs`; the recipe is
//! `docs/evidence/hermes-composer/README.md`.
use super::*;
use crate::{harness::composer::read_composer, protocol::authority::Harness};

macro_rules! capture {
    ($name:literal) => {
        (
            include_str!(concat!(
                "../../docs/evidence/hermes-composer/captures/hermes-0.22.5-",
                $name,
                ".read-detection.txt"
            )),
            include_str!(concat!(
                "../../docs/evidence/hermes-composer/captures/hermes-0.22.5-",
                $name,
                ".recent-ansi.txt"
            )),
        )
    };
}

fn hermes() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("hermes")
            .unwrap(),
    )
}

/// The registered policy's detection read, and the styled read it asks for, as the
/// native host combines them.
fn read((detection, ansi): (&str, &str)) -> ComposerRead {
    let plain = read_composer(hermes(), detection, None);
    if !HermesComposer.wants_styled(&plain) {
        return plain;
    }
    HermesComposer
        .read_styled(&sgr::parse(ansi).expect("a styled capture"), None)
        .unwrap_or(plain)
}

// Catches the Hermes idle wake never firing: the placeholder must read empty.
#[test]
fn placeholder_reads_empty_at_both_widths() {
    for capture in [
        capture!("idle-empty"),
        capture!("after-turn"),
        capture!("narrow-idle-empty"),
    ] {
        let plain = read_composer(hermes(), capture.0, None);
        assert!(
            matches!(&plain, ComposerRead::Unsafe { reason, .. } if *reason == HERMES_NOT_KNOWN_EMPTY),
            "{plain:?}"
        );
        assert_eq!(read(capture), ComposerRead::Empty);
    }
}

// Catches a wake typed over the person's draft.
#[test]
fn typed_drafts_are_never_empty() {
    assert_eq!(
        read(capture!("draft")),
        ComposerRead::Text("please keep this draft".into())
    );
    assert_eq!(
        read(capture!("history-suggestion")),
        ComposerRead::Text("please".into())
    );
    assert_eq!(
        read(capture!("multiline-draft")),
        ComposerRead::Text("please keep this draft\nsecond line".into())
    );
}

// Catches a nudge into a busy session: Herdr reports Hermes `idle` mid-turn.
#[test]
fn busy_screens_are_unreadable() {
    for capture in [
        capture!("busy"),
        capture!("busy-draft"),
        capture!("narrow-busy"),
    ] {
        assert_eq!(read(capture), ComposerRead::Unreadable);
    }
}

// Catches a wake line typed into an overlay or the first-run consent screen.
#[test]
fn modals_and_overlays_are_unreadable() {
    for capture in [
        capture!("telemetry-modal"),
        capture!("palette"),
        capture!("model-picker"),
        capture!("model-picker-loading"),
    ] {
        assert_eq!(read(capture), ComposerRead::Unreadable);
    }
}

fn screen(composer: &str, footer: &str) -> String {
    let rule = "─".repeat(80);
    format!("output\n{rule}\n{composer}\n{rule}\n{footer}\n")
}

const IDLE_FOOTER: &str = " ☤ model · 1m  / commands · @ files · ! shell";

// Hermes drops the key hints whenever the footer's left side leaves no room for them.
#[test]
fn the_footer_hints_decide_nothing() {
    assert_eq!(
        HermesComposer.read(&screen("❯", IDLE_FOOTER), None),
        ComposerRead::Empty
    );
    assert_eq!(
        HermesComposer.read(&screen("❯", " ☤ model · 36.6K/500K · 2m"), None),
        ComposerRead::Empty
    );
    assert_eq!(
        HermesComposer.read(&format!("{}\n", screen("❯", "").trim_end()), None),
        ComposerRead::Unreadable,
        "a blank footer (model picker, palette)"
    );
}

#[test]
fn a_panel_above_the_top_rule_is_unreadable() {
    let rule = "─".repeat(80);
    let panel =
        format!("╭─ Confirm ─╮\n│ 1) yes    │\n╰───────────╯\n{rule}\n❯\n{rule}\n{IDLE_FOOTER}\n");
    assert_eq!(HermesComposer.read(&panel, None), ComposerRead::Unreadable);
}

// A busy top rule must not let an older full-width rule further up stand in for it.
#[test]
fn an_older_rule_never_stands_in_for_a_busy_top_rule() {
    let rule = "─".repeat(80);
    let busy = format!(
        "{rule}\n❯ earlier\n{rule}\n── ⠹ thinking · 3s {}\n❯\n{rule}\n{IDLE_FOOTER}\n",
        "─".repeat(61)
    );
    assert_eq!(HermesComposer.read(&busy, None), ComposerRead::Unreadable);
}

#[test]
fn styled_mixed_or_continuation_italics_are_unsafe() {
    let rule = "─".repeat(80);
    let mixed = format!(
        "{rule}\n\x1b[38;5;230m❯ \x1b[0mtyp\x1b[3m\x1b[38;5;136med\x1b[0m\n{rule}\n{IDLE_FOOTER}\n"
    );
    let read = HermesComposer.read_styled(&sgr::parse(&mixed).unwrap(), None);
    assert!(
        matches!(&read, Some(ComposerRead::Unsafe { reason, .. }) if *reason == HERMES_MIXED_STYLE),
        "{read:?}"
    );
    let continuation = format!(
        "{rule}\n\x1b[38;5;230m❯ \x1b[0mfirst\n\x1b[3m\x1b[38;5;136msecond\x1b[0m\n{rule}\n{IDLE_FOOTER}\n"
    );
    assert!(matches!(
        HermesComposer.read_styled(&sgr::parse(&continuation).unwrap(), None),
        Some(ComposerRead::Unsafe { .. })
    ));
}

#[test]
fn stashing_is_unsupported() {
    assert_eq!(
        HermesComposer.capabilities(Some("0.22.5")),
        PokeCapabilities::NONE
    );
}
