//! Native composer-state parsing over captured pane text (no live Herdr).
use super::{composer_state_from_read, composer_state_from_text};
use crate::notification::policy::MARKER;
use crate::ports::AgentComposerState;

#[test]
fn composer_holding_marker_reads_holding_prompt() {
    let pane = format!(
        "previous output\n\n╭──────────────╮\n│ > {MARKER}   │\n╰──────────────╯\n  ? for shortcuts\n"
    );
    assert_eq!(
        composer_state_from_text(Some(&pane)),
        AgentComposerState::HoldingPrompt
    );
    let body = serde_json::json!({"result":{"read":{"text":pane}}}).to_string();
    assert_eq!(
        composer_state_from_read(Some(&body)),
        AgentComposerState::HoldingPrompt
    );
}

#[test]
fn cleared_composer_reads_submitted() {
    // The submitted prompt is history above the agent's reply and the empty box.
    let pane = format!(
        "> {MARKER}\n\n● Checking the inbox\n  running herdr-threads inbox\n\n╭──────────────╮\n│ >            │\n╰──────────────╯\n  ? for shortcuts\n"
    );
    assert_eq!(
        composer_state_from_text(Some(&pane)),
        AgentComposerState::Submitted
    );
}

#[test]
fn read_error_reads_unknown() {
    assert_eq!(composer_state_from_text(None), AgentComposerState::Unknown);
    assert_eq!(composer_state_from_read(None), AgentComposerState::Unknown);
    assert_eq!(
        composer_state_from_read(Some("not json")),
        AgentComposerState::Unknown
    );
}

fn read_wrapper(pane: &str) -> String {
    serde_json::json!({"result":{"read":{"text":pane}}}).to_string()
}

#[test]
fn wrapped_marker_in_narrow_claude_box_reads_holding_prompt() {
    // A 40-column Claude box wraps its own content: the marker is split over
    // two box lines, so no single line contains it whole.
    let pane = "previous output\n\n\
        ╭──────────────────────────────────────╮\n\
        │ > herdr-threads: attention pending; run │\n\
        │ herdr-threads inbox                  │\n\
        ╰──────────────────────────────────────╯\n\
        \x20 ? for shortcuts\n";
    assert_eq!(
        composer_state_from_text(Some(pane)),
        AgentComposerState::HoldingPrompt
    );
    assert_eq!(
        composer_state_from_read(Some(&read_wrapper(pane))),
        AgentComposerState::HoldingPrompt
    );
}

#[test]
fn empty_or_blank_text_reads_unknown() {
    // A successful read with nothing in it proves nothing.
    for text in ["", "\n\n", "   \n"] {
        assert_eq!(
            composer_state_from_text(Some(text)),
            AgentComposerState::Unknown,
            "{text:?}"
        );
    }
    assert_eq!(
        composer_state_from_read(Some(&read_wrapper(""))),
        AgentComposerState::Unknown
    );
}

#[test]
fn codex_composer_holding_marker_reads_holding_prompt() {
    let pane = format!(
        "• Ran herdr-threads inbox\n\n› {MARKER}\n\n  ⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit   100% context left\n"
    );
    assert_eq!(
        composer_state_from_text(Some(&pane)),
        AgentComposerState::HoldingPrompt
    );
    assert_eq!(
        composer_state_from_read(Some(&read_wrapper(&pane))),
        AgentComposerState::HoldingPrompt
    );
}

#[test]
fn codex_cleared_composer_with_marker_in_history_reads_submitted() {
    // The marker is the 4th non-empty line from the bottom (inside the tail
    // window) but it is history above the cleared composer.
    let pane = format!(
        "› {MARKER}\n\n• Working (2s • esc to interrupt)\n\n› \n\n  ⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit   100% context left\n"
    );
    assert_eq!(
        composer_state_from_text(Some(&pane)),
        AgentComposerState::Submitted
    );
    assert_eq!(
        composer_state_from_read(Some(&read_wrapper(&pane))),
        AgentComposerState::Submitted
    );
}

#[test]
fn marker_fifth_from_bottom_reads_submitted() {
    // Pins `COMPOSER_TAIL_LINES = 4`: in the just-submitted, no-output Claude
    // layout the marker is the 5th non-empty line from the bottom, so a window
    // of 5 under the whole-line rule would read it as still holding.
    let pane = format!(
        "> {MARKER}\n\n╭──────────────╮\n│ >            │\n╰──────────────╯\n  ? for shortcuts\n"
    );
    assert_eq!(
        composer_state_from_text(Some(&pane)),
        AgentComposerState::Submitted
    );
    assert_eq!(
        composer_state_from_read(Some(&read_wrapper(&pane))),
        AgentComposerState::Submitted
    );
}
