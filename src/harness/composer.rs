//! The composer reader shared by the host observation and the composer stash,
//! so the classification of a pane and the stash can never disagree.
//!
//! Extraction rules come from the poke spike
//! (`docs/evidence/poke-spike/findings.md`, Q2), applied to the text of
//! `agent read --source detection`:
//!
//! - Claude: the rows between the last two `─` rules. The first row starts
//!   `❯ `, continuation rows are indented two spaces. Empty is `❯` alone or a
//!   captured `Try "…"` placeholder; another placeholder-shaped row is
//!   unreadable. Any other Claude text is `Unsafe` ([`CLAUDE_NOT_KNOWN_EMPTY`]),
//!   never `Text`: Claude draws a prompt suggestion in the composer after a turn
//!   and the detection text cannot tell it from a typed draft.
//! - Codex: a `› ` row, then two-space rows until a blank row (the footer
//!   follows it). Empty is `› Ask Codex to do anything`.
//!
//! A soft wrap cannot be told from a hard newline (rows are measured in display
//! columns, not characters), an image placeholder does
//! not survive a retype, and a pasted-text placeholder stands for text that is
//! not in the composer rows, so each of those makes the composer `Unsafe`:
//! its text is known but must not be stashed. Herdr's detection text carries no
//! trailing whitespace, so a composer row whose raw text ends in whitespace is
//! `Unsafe` too: the whitespace cannot be restored on a retype.
use std::ops::Range;

use unicode_width::UnicodeWidthChar;

use crate::{ports::HostUiState, protocol::authority::Harness};

/// A row this close to the pane width may be a soft wrap (findings Q2).
const WRAP_MARGIN: usize = 12;
/// The shortest run of `─` taken for Claude's composer rule.
const MIN_RULE: usize = 10;
const CLAUDE_PROMPT: &str = "❯";
const CODEX_PROMPT: &str = "›";
/// Claude's empty-composer placeholder rows, exact text. Only a captured
/// placeholder may be listed: `docs/evidence/poke-spike/findings.md` (Q2 table)
/// records `Try "how do I log an error?"`.
const CLAUDE_PLACEHOLDERS: &[&str] = &["Try \"how do I log an error?\""];
const CODEX_EMPTY: &str = "Ask Codex to do anything";
/// Why Claude composer text is never `Text`.
pub const CLAUDE_NOT_KNOWN_EMPTY: &str = "Claude composer text may be a prompt suggestion";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerRead {
    /// A recognized composer holding no typed text.
    Empty,
    /// Typed text, rows joined by `\n`, safe to stash and retype.
    Text(String),
    /// Typed text that must not be stashed, and why.
    Unsafe { text: String, reason: &'static str },
    /// No recognizable composer, or one whose emptiness cannot be decided (a
    /// placeholder-shaped row that is not a captured placeholder).
    Unreadable,
}

/// Read the composer out of a `--source detection` pane text. `pane_width` is
/// the pane's column count when known; Claude's composer rule spans the pane,
/// so its length stands in when the width is unknown. Codex has no rule: an
/// unknown width makes typed text `Unsafe`.
pub fn read_composer(harness: Harness, detection: &str, pane_width: Option<u16>) -> ComposerRead {
    // Claude draws the gap after `❯` as a no-break space (captures).
    let detection = detection.replace('\u{a0}', " ");
    let raw: Vec<&str> = detection.lines().collect();
    let lines: Vec<&str> = raw.iter().map(|line| line.trim_end()).collect();
    let (rows, body, width, empty): (Vec<String>, Range<usize>, Option<usize>, bool) = match harness
    {
        Harness::Claude => match claude_rows(&lines) {
            Some((rows, body, rule)) => match claude_empty(&rows) {
                Some(empty) => (
                    rows,
                    body,
                    pane_width.map(usize::from).or(Some(rule)),
                    empty,
                ),
                None => return ComposerRead::Unreadable,
            },
            None => return ComposerRead::Unreadable,
        },
        Harness::Codex => match codex_rows(&lines) {
            Some((rows, body)) => {
                let empty = rows.len() == 1 && rows[0] == CODEX_EMPTY;
                (rows, body, pane_width.map(usize::from), empty)
            }
            None => return ComposerRead::Unreadable,
        },
        Harness::Human | Harness::Agent(_) => return ComposerRead::Unreadable,
    };
    if empty {
        return ComposerRead::Empty;
    }
    let prefix = match harness {
        Harness::Codex => CODEX_PROMPT,
        Harness::Claude => CLAUDE_PROMPT,
        _ => return ComposerRead::Unreadable,
    };
    // The first row's prompt glyph and following space are not typed text.
    let text = rows.join("\n");
    if text.is_empty() {
        return ComposerRead::Empty;
    }
    let trailing_whitespace = body.clone().any(|index| {
        let trimmed = lines[index];
        raw[index] != trimmed && !trimmed.is_empty() && trimmed != prefix
    });
    if trailing_whitespace {
        return ComposerRead::Unsafe {
            text,
            reason: "trailing whitespace in a composer row",
        };
    }
    if text.contains("[Image #") {
        return ComposerRead::Unsafe {
            text,
            reason: "image placeholder in the composer",
        };
    }
    if text.contains("[Pasted text") {
        return ComposerRead::Unsafe {
            text,
            reason: "pasted-text placeholder in the composer",
        };
    }
    let Some(width) = width else {
        return ComposerRead::Unsafe {
            text,
            reason: "pane width unknown",
        };
    };
    let mut near_wrap = false;
    for (index, row) in rows.iter().enumerate() {
        let Some(columns) = display_width(row) else {
            return ComposerRead::Unsafe {
                text,
                reason: "composer row holds a character of unknown display width",
            };
        };
        let lead = if index == 0 {
            prefix.chars().count() + 1
        } else {
            2
        };
        near_wrap |= lead + columns + WRAP_MARGIN >= width;
    }
    if near_wrap {
        return ComposerRead::Unsafe {
            text,
            reason: "composer row within 12 columns of the pane width (soft wrap)",
        };
    }
    // Claude draws a prompt suggestion in the composer after a turn, and the
    // detection text cannot tell it from a typed draft (ht-jf3). Without captured
    // styling, any Claude text that is not a known empty marker is not known
    // empty: never stashed, so a poke over it is skipped. Ordinary wakes do not
    // consult the composer (TRUST-POLICY A4).
    if harness == Harness::Claude {
        return ComposerRead::Unsafe {
            text,
            reason: CLAUDE_NOT_KNOWN_EMPTY,
        };
    }
    ComposerRead::Text(text)
}

/// Display columns of a composer row, `None` when a character's rendered
/// width cannot be determined: a control character, or an emoji variation
/// selector (U+FE0F) or zero-width joiner (U+200D), whose sequences render
/// wider than the sum of their parts. East Asian ambiguous-width characters
/// count two columns (`width_cjk`): overestimating only makes the guard stricter.
fn display_width(row: &str) -> Option<usize> {
    row.chars()
        .map(|c| match c {
            '\u{fe0f}' | '\u{200d}' => None,
            c => c.width_cjk(),
        })
        .sum()
}

fn is_rule(line: &str) -> bool {
    line.chars().count() >= MIN_RULE && line.chars().all(|c| c == '─')
}

/// The composer's rows (prompt glyph and indent removed), the line range of
/// its body, and the rule width.
fn claude_rows(lines: &[&str]) -> Option<(Vec<String>, Range<usize>, usize)> {
    let mut rules = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_rule(line))
        .map(|(index, _)| index)
        .rev();
    let bottom = rules.next()?;
    let top = rules.next()?;
    let body = &lines[top + 1..bottom];
    let first = body.first()?;
    let first = if *first == CLAUDE_PROMPT {
        String::new()
    } else {
        first.strip_prefix("❯ ")?.to_owned()
    };
    let mut rows = vec![first];
    for line in &body[1..] {
        rows.push(line.strip_prefix("  ")?.to_owned());
    }
    // Trailing empty continuation rows are padding, not typed text.
    while rows.len() > 1 && rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    let rule = lines[bottom].chars().count();
    Some((rows, top + 1..bottom, rule))
}

/// `Some(true)`: empty; `Some(false)`: typed text; `None`: a placeholder-shaped
/// row that is not a captured placeholder, so emptiness cannot be decided.
fn claude_empty(rows: &[String]) -> Option<bool> {
    match rows {
        [only] if only.is_empty() || CLAUDE_PLACEHOLDERS.contains(&only.as_str()) => Some(true),
        [only] if only.starts_with("Try \"") => None,
        _ => Some(false),
    }
}

/// The rows of the last `› ` composer: its first row and two-space rows until
/// a blank row.
fn codex_rows(lines: &[&str]) -> Option<(Vec<String>, Range<usize>)> {
    let start = lines
        .iter()
        .rposition(|line| line.strip_prefix("› ").is_some() || *line == CODEX_PROMPT)?;
    let first = lines[start]
        .strip_prefix("› ")
        .unwrap_or_default()
        .to_owned();
    let mut rows = vec![first];
    let mut end = start + 1;
    for line in &lines[start + 1..] {
        if line.is_empty() {
            break;
        }
        match line.strip_prefix("  ") {
            Some(row) => rows.push(row.to_owned()),
            None => break,
        }
        end += 1;
    }
    Some((rows, start..end))
}

/// The UI state a pane shows, from Herdr's `agent_status` and a composer read
/// (`None`: the read was not made or failed). Anything unreadable is
/// `Unknown`, which every wake and poke path already skips.
pub fn observed_ui(agent_status: Option<&str>, composer: Option<&ComposerRead>) -> HostUiState {
    match (agent_status, composer) {
        (Some("blocked"), _) => HostUiState::ApprovalOrQuestion,
        (_, None | Some(ComposerRead::Unreadable)) => HostUiState::Unknown,
        (Some("idle" | "done"), Some(ComposerRead::Empty)) => HostUiState::Idle,
        (Some("idle" | "done"), Some(ComposerRead::Text(_) | ComposerRead::Unsafe { .. })) => {
            HostUiState::HumanInput
        }
        // A poke into a running turn merges with a draft (findings Q5).
        (Some("working"), Some(ComposerRead::Empty)) => HostUiState::ActiveTurn,
        _ => HostUiState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! capture {
        ($name:literal) => {
            include_str!(concat!(
                "../../docs/evidence/poke-spike/captures/",
                $name,
                ".read-detection.txt"
            ))
        };
    }

    const CLAUDE_WIDTH: Option<u16> = Some(80);
    const CODEX_WIDTH: Option<u16> = Some(100);

    fn claude(text: &str) -> ComposerRead {
        read_composer(Harness::Claude, text, CLAUDE_WIDTH)
    }
    fn codex(text: &str) -> ComposerRead {
        read_composer(Harness::Codex, text, CODEX_WIDTH)
    }

    #[test]
    fn claude_empty_composer_is_empty() {
        assert_eq!(claude(capture!("claude-q1-empty")), ComposerRead::Empty);
    }

    fn not_known_empty(text: &str) -> ComposerRead {
        ComposerRead::Unsafe {
            text: text.into(),
            reason: CLAUDE_NOT_KNOWN_EMPTY,
        }
    }

    #[test]
    fn claude_text_is_preserved_but_not_known_empty() {
        assert_eq!(
            claude(capture!("claude-q1-q2-single")),
            not_known_empty("hello world one")
        );
        assert_eq!(
            claude(capture!("claude-q2-multiline-shiftenter")),
            not_known_empty("line one\nline two")
        );
        assert_eq!(
            claude(capture!("claude-q2-multiline-paste")),
            not_known_empty("ccc\nddd")
        );
    }

    #[test]
    fn claude_text_is_not_known_empty() {
        let rule = "─".repeat(80);
        let suggestion = format!("{rule}\n❯ herdr-threads pending-receipts\n{rule}\n  footer\n");
        let two_rows = format!("{rule}\n❯ first draft row\n  second draft row\n{rule}\n");
        for (read, text) in [
            (claude(capture!("claude-q1-q2-single")), "hello world one"),
            (claude(&suggestion), "herdr-threads pending-receipts"),
            (claude(&two_rows), "first draft row\nsecond draft row"),
        ] {
            assert_eq!(read, not_known_empty(text));
            assert_eq!(
                observed_ui(Some("idle"), Some(&read)),
                HostUiState::HumanInput
            );
        }
    }

    #[test]
    fn claude_known_empty_markers_are_idle() {
        let rule = "─".repeat(80);
        let placeholder = format!("{rule}\n❯ Try \"how do I log an error?\"\n{rule}\n  footer\n");
        for screen in [capture!("claude-q1-empty"), placeholder.as_str()] {
            let read = claude(screen);
            assert_eq!(read, ComposerRead::Empty);
            assert_eq!(observed_ui(Some("idle"), Some(&read)), HostUiState::Idle);
        }
    }

    #[test]
    fn wide_characters_count_their_display_columns() {
        let rule = "─".repeat(80);
        let wide = "漢".repeat(40);
        assert!(matches!(
            claude(&format!("{rule}\n❯ {wide}\n{rule}\n")),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
        // 2 + 45 x 2 + 12 >= 100; the old char count scored 59.
        let wide = "漢".repeat(45);
        assert!(matches!(
            codex(&format!("› {wide}\n\n  footer\n")),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
        assert_eq!(
            codex("› 漢字\n\n  footer\n"),
            ComposerRead::Text("漢字".into())
        );
    }

    #[test]
    fn emoji_row_near_the_width_is_unsafe() {
        let rule = "─".repeat(80);
        let emoji = "😀".repeat(34);
        assert!(matches!(
            claude(&format!("{rule}\n❯ {emoji}\n{rule}\n")),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
        let emoji = "😀".repeat(44);
        assert!(matches!(
            codex(&format!("› {emoji}\n\n  footer\n")),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
    }

    #[test]
    fn undeterminable_width_is_unsafe() {
        const UNKNOWN: &str = "composer row holds a character of unknown display width";
        for row in ["❤\u{fe0f}", "👨\u{200d}👩", "a\u{7}b"] {
            assert!(
                matches!(
                    codex(&format!("› {row}\n\n  footer\n")),
                    ComposerRead::Unsafe { reason, .. } if reason == UNKNOWN
                ),
                "{row:?}"
            );
        }
        let rule = "─".repeat(80);
        assert!(matches!(
            claude(&format!("{rule}\n❯ ❤\u{fe0f}\n{rule}\n")),
            ComposerRead::Unsafe { reason, .. } if reason == UNKNOWN
        ));
    }

    #[test]
    fn claude_image_placeholder_is_unsafe_with_its_text() {
        assert_eq!(
            claude(capture!("claude-q2-image-placeholder")),
            ComposerRead::Unsafe {
                text: "before [Image #1]".into(),
                reason: "image placeholder in the composer"
            }
        );
    }

    #[test]
    fn claude_width_comes_from_the_rule_when_the_pane_width_is_unknown() {
        let single = capture!("claude-q1-q2-single");
        // The rule supplied the width: the reason is not "pane width unknown".
        assert_eq!(
            read_composer(Harness::Claude, single, None),
            not_known_empty("hello world one")
        );
        // A known narrow pane makes the same row a possible soft wrap.
        assert!(matches!(
            read_composer(Harness::Claude, single, Some(24)),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
    }

    #[test]
    fn claude_placeholder_hint_is_empty() {
        let rule = "─".repeat(80);
        let screen = format!("{rule}\n❯ Try \"how do I log an error?\"\n{rule}\n  footer\n");
        assert_eq!(claude(&screen), ComposerRead::Empty);
    }

    #[test]
    fn claude_placeholder_lookalike_is_unreadable() {
        let rule = "─".repeat(80);
        for row in [
            "Try \"how do I deploy this?\"",
            "Try \"how do I log an error?\" now",
        ] {
            let screen = format!("{rule}\n❯ {row}\n{rule}\n  footer\n");
            let read = claude(&screen);
            assert_eq!(read, ComposerRead::Unreadable, "{row}");
            for status in ["idle", "working"] {
                assert_eq!(observed_ui(Some(status), Some(&read)), HostUiState::Unknown);
            }
        }
    }

    #[test]
    fn trailing_whitespace_row_is_unsafe() {
        let unsafe_ws = |read: ComposerRead| {
            assert!(
                matches!(read, ComposerRead::Unsafe { reason, .. } if reason == "trailing whitespace in a composer row"),
                "{read:?}"
            );
        };
        let rule = "─".repeat(80);
        unsafe_ws(claude(&format!("{rule}\n❯ hello  \n{rule}\n  footer\n")));
        unsafe_ws(claude(&format!(
            "{rule}\n❯ hello\n  second \n{rule}\n  footer\n"
        )));
        unsafe_ws(codex("› hello \n\n  footer\n"));
        // Empty rows with trailing nbsp / padding stay empty.
        assert_eq!(
            claude(&format!("{rule}\n❯\u{a0}\n{rule}\n  footer\n")),
            ComposerRead::Empty
        );
        assert_eq!(
            codex("› Ask Codex to do anything  \n\n  footer\n"),
            ComposerRead::Empty
        );
        // Whitespace on the footer row, outside the composer, is not the composer's:
        // the reason is not trailing whitespace.
        assert_eq!(
            claude(&format!("{rule}\n❯ hello\n{rule}\n  footer  \n")),
            not_known_empty("hello")
        );
    }

    #[test]
    fn claude_row_near_the_pane_width_is_unsafe() {
        let rule = "─".repeat(80);
        let long = "x".repeat(70);
        let screen = format!("{rule}\n❯ {long}\n{rule}\n  footer\n");
        assert!(matches!(
            claude(&screen),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
        // 2 (prompt) + 65 + 12 = 79 < 80: no wrap reason; Claude text is not known empty.
        let fits = "x".repeat(65);
        let screen = format!("{rule}\n❯ {fits}\n{rule}\n  footer\n");
        assert_eq!(claude(&screen), not_known_empty(&fits));
        // The same boundary on Codex, where fitting text is `Text`.
        let codex_read = |count: usize| {
            let row = "x".repeat(count);
            read_composer(Harness::Codex, &format!("› {row}\n\n  footer\n"), Some(80))
        };
        assert_eq!(codex_read(65), ComposerRead::Text(fits));
        assert!(matches!(
            codex_read(70),
            ComposerRead::Unsafe { reason, .. } if reason.contains("soft wrap")
        ));
    }

    #[test]
    fn claude_pasted_text_placeholder_is_unsafe() {
        let rule = "─".repeat(80);
        let screen = format!("{rule}\n❯ [Pasted text #1 +20 lines]\n{rule}\n");
        assert!(matches!(
            claude(&screen),
            ComposerRead::Unsafe { reason, .. } if reason.contains("pasted-text")
        ));
    }

    #[test]
    fn codex_empty_single_and_multiline() {
        assert_eq!(codex(capture!("codex-q6-workers")), ComposerRead::Empty);
        assert_eq!(
            codex(capture!("codex-q1-q2-single")),
            ComposerRead::Text("hello world one".into())
        );
        assert_eq!(
            codex(capture!("codex-q2-multiline-paste")),
            ComposerRead::Text("ccc\nddd".into())
        );
    }

    #[test]
    fn codex_image_placeholder_is_unsafe() {
        assert!(matches!(
            codex(capture!("codex-q2-image-placeholder")),
            ComposerRead::Unsafe { reason, .. } if reason.contains("image")
        ));
    }

    #[test]
    fn codex_text_with_unknown_width_is_unsafe_but_empty_is_still_empty() {
        assert_eq!(
            read_composer(Harness::Codex, capture!("codex-q1-q2-single"), None),
            ComposerRead::Unsafe {
                text: "hello world one".into(),
                reason: "pane width unknown"
            }
        );
        assert_eq!(
            read_composer(Harness::Codex, capture!("codex-q6-workers"), None),
            ComposerRead::Empty
        );
    }

    #[test]
    fn codex_composer_is_the_last_prompt_row_not_a_history_row() {
        let screen =
            "› an earlier user message\n\n• reply\n\n› Ask Codex to do anything\n\n  footer\n";
        assert_eq!(codex(screen), ComposerRead::Empty);
    }

    #[test]
    fn unrelated_text_and_other_harnesses_are_unreadable() {
        assert_eq!(
            claude("just a shell prompt\n$ ls\n"),
            ComposerRead::Unreadable
        );
        assert_eq!(
            codex("just a shell prompt\n$ ls\n"),
            ComposerRead::Unreadable
        );
        assert_eq!(
            read_composer(Harness::Human, capture!("claude-q1-empty"), CLAUDE_WIDTH),
            ComposerRead::Unreadable
        );
        // Claude rules without a prompt row between them.
        let rule = "─".repeat(80);
        assert_eq!(
            claude(&format!("{rule}\nnot a prompt\n{rule}\n")),
            ComposerRead::Unreadable
        );
    }

    #[test]
    fn observed_ui_covers_every_status_and_read_outcome() {
        use HostUiState::*;
        let text = ComposerRead::Text("draft".into());
        let unsafe_text = ComposerRead::Unsafe {
            text: "[Image #1]".into(),
            reason: "image",
        };
        let table: [(Option<&str>, Option<&ComposerRead>, HostUiState); 20] = [
            (Some("idle"), Some(&ComposerRead::Empty), Idle),
            (Some("done"), Some(&ComposerRead::Empty), Idle),
            (Some("idle"), Some(&text), HumanInput),
            (Some("done"), Some(&unsafe_text), HumanInput),
            (Some("working"), Some(&ComposerRead::Empty), ActiveTurn),
            (Some("working"), Some(&text), Unknown),
            (Some("working"), Some(&unsafe_text), Unknown),
            (
                Some("blocked"),
                Some(&ComposerRead::Empty),
                ApprovalOrQuestion,
            ),
            (Some("blocked"), None, ApprovalOrQuestion),
            (Some("idle"), None, Unknown),
            (Some("idle"), Some(&ComposerRead::Unreadable), Unknown),
            (Some("working"), Some(&ComposerRead::Unreadable), Unknown),
            (Some("unknown"), Some(&ComposerRead::Empty), Unknown),
            (Some("other"), Some(&text), Unknown),
            (None, Some(&ComposerRead::Empty), Unknown),
            (None, None, Unknown),
            (Some("done"), None, Unknown),
            (Some("done"), Some(&text), HumanInput),
            (Some("working"), None, Unknown),
            (
                Some("blocked"),
                Some(&ComposerRead::Unreadable),
                ApprovalOrQuestion,
            ),
        ];
        for (status, read, expected) in table {
            assert_eq!(observed_ui(status, read), expected, "{status:?} {read:?}");
        }
    }
}
