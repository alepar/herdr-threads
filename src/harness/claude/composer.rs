//! Captured native composer grammar and safe input recipe.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
    sgr,
};
use std::ops::Range;
/// The shortest run of `─` taken for Claude's composer rule.
const MIN_RULE: usize = 10;
const CLAUDE_PROMPT: &str = "❯";
/// Why Claude composer text is never `Text`.
pub(crate) const CLAUDE_NOT_KNOWN_EMPTY: &str = "Claude composer text may be a prompt suggestion";
/// Claude's empty-composer placeholder rows, exact text. Only a captured
/// placeholder may be listed: `docs/evidence/poke-spike/findings.md` (Q2 table)
/// records `Try "how do I log an error?"`.
const CLAUDE_PLACEHOLDERS: &[&str] = &["Try \"how do I log an error?\""];

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

/// Why styled Claude composer text is neither empty nor a draft.
const CLAUDE_MIXED_STYLE: &str = "Claude composer text mixes dim and other styles";

/// How the composer's non-blank characters are drawn.
enum RowStyle {
    /// No character: an empty composer.
    Blank,
    /// Every character dim and nothing else: a placeholder or prompt suggestion.
    Dim,
    /// Every character in the default rendition: typed text.
    Plain,
    Mixed,
}

/// The style of `rows`, each read from its screen line past the two-column
/// lead (`❯ ` or the continuation indent). `None` when the prompt glyph is
/// dim: a dimmed composer may hold typed text drawn dim.
fn claude_row_style(
    screen: &[sgr::StyledLine],
    rows: &[String],
    body: &Range<usize>,
) -> Option<RowStyle> {
    if screen[body.start].styles.first()?.is_dim() {
        return None;
    }
    let mut style = RowStyle::Blank;
    for (row, line) in rows.iter().zip(&screen[body.clone()]) {
        let cells = line.text.chars().zip(&line.styles).skip(2);
        for (_, cell) in cells
            .take(row.chars().count())
            .filter(|(c, _)| !c.is_whitespace())
        {
            style = match (style, cell.is_dim_only(), cell.is_plain()) {
                (RowStyle::Blank | RowStyle::Dim, true, _) => RowStyle::Dim,
                (RowStyle::Blank | RowStyle::Plain, _, true) => RowStyle::Plain,
                _ => return Some(RowStyle::Mixed),
            };
        }
    }
    Some(style)
}

pub(super) struct NativeComposer;
impl ComposerPolicy for NativeComposer {
    /// Detection text cannot tell a suggestion or an uncaptured placeholder
    /// from a draft; the styled screen can.
    fn wants_styled(&self, detection: &ComposerRead) -> bool {
        matches!(
            detection,
            ComposerRead::Unreadable
                | ComposerRead::Unsafe {
                    reason: CLAUDE_NOT_KNOWN_EMPTY,
                    ..
                }
        )
    }
    /// Dim-only text is a placeholder or prompt suggestion (empty), unstyled
    /// text a draft, anything else unsafe.
    fn read_styled(
        &self,
        screen: &[sgr::StyledLine],
        pane_width: Option<u16>,
    ) -> Option<ComposerRead> {
        let screen: Vec<sgr::StyledLine> = screen
            .iter()
            .map(|line| sgr::StyledLine {
                text: line.text.replace('\u{a0}', " "),
                styles: line.styles.clone(),
            })
            .collect();
        let raw: Vec<&str> = screen.iter().map(|line| line.text.as_str()).collect();
        let lines: Vec<&str> = raw.iter().map(|line| line.trim_end()).collect();
        let (rows, body, rule) = claude_rows(&lines)?;
        let empty = match claude_row_style(&screen, &rows, &body)? {
            RowStyle::Blank | RowStyle::Dim => true,
            RowStyle::Plain => false,
            RowStyle::Mixed => {
                return Some(ComposerRead::Unsafe {
                    text: rows.join("\n"),
                    reason: CLAUDE_MIXED_STYLE,
                });
            }
        };
        Some(composer::validate_rows(
            &raw,
            &lines,
            rows,
            body,
            pane_width.map(usize::from).or(Some(rule)),
            empty,
            CLAUDE_PROMPT,
        ))
    }
    fn read(&self, detection: &str, pane_width: Option<u16>) -> ComposerRead {
        let detection = detection.replace('\u{a0}', " ");
        let raw: Vec<&str> = detection.lines().collect();
        let lines: Vec<&str> = raw.iter().map(|line| line.trim_end()).collect();
        let Some((rows, body, rule)) = claude_rows(&lines) else {
            return ComposerRead::Unreadable;
        };
        let Some(empty) = claude_empty(&rows) else {
            return ComposerRead::Unreadable;
        };
        let read = composer::validate_rows(
            &raw,
            &lines,
            rows,
            body,
            pane_width.map(usize::from).or(Some(rule)),
            empty,
            CLAUDE_PROMPT,
        );
        // Detection text cannot distinguish suggestions from drafts (ht-jf3).
        // Only captured empty markers may admit a poke; text is never stashed.
        match read {
            ComposerRead::Text(text) => ComposerRead::Unsafe {
                text,
                reason: CLAUDE_NOT_KNOWN_EMPTY,
            },
            read => read,
        }
    }
    fn capabilities(&self, installed: Option<&str>) -> PokeCapabilities {
        let Some(installed) = installed else {
            return PokeCapabilities::NONE;
        };
        super::recipe_for(installed)
            .map(|recipe| PokeCapabilities {
                composer_stash: recipe.profile.composer_stash,
            })
            .unwrap_or(PokeCapabilities::NONE)
    }
    fn clear_key(&self) -> &'static str {
        "ctrl+u"
    }
    fn restore_text(&self, saved: &str) -> String {
        saved.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{harness::sgr, protocol::authority::Harness};

    macro_rules! capture {
        ($name:literal) => {
            include_str!(concat!(
                "../../../docs/evidence/ansi-composer/captures/claude-2.1.309-",
                $name
            ))
        };
    }

    fn detection(text: &str) -> ComposerRead {
        composer::read_composer(Harness::Claude, text, None)
    }
    fn styled(screen: &str) -> Option<ComposerRead> {
        NativeComposer.read_styled(&sgr::parse(screen).expect("a styled capture"), None)
    }

    // Catches a dim prompt suggestion still refusing the fallback wake.
    #[test]
    fn captured_suggestion_and_placeholders_read_empty_when_styled() {
        for (plain, ansi) in [
            (
                capture!("suggestion.read-detection.txt"),
                capture!("suggestion.recent-ansi.txt"),
            ),
            (
                capture!("placeholder-uncaptured.read-detection.txt"),
                capture!("placeholder-uncaptured.visible-ansi.txt"),
            ),
        ] {
            let read = detection(plain);
            assert!(NativeComposer.wants_styled(&read), "{read:?}");
            assert_eq!(styled(ansi), Some(ComposerRead::Empty));
        }
        assert_eq!(
            detection(capture!("suggestion.read-detection.txt")),
            ComposerRead::Unsafe {
                text: "run the tests".into(),
                reason: CLAUDE_NOT_KNOWN_EMPTY
            }
        );
        // The captured placeholder is empty without a styled read.
        let read = detection(capture!("placeholder.read-detection.txt"));
        assert_eq!(read, ComposerRead::Empty);
        assert!(!NativeComposer.wants_styled(&read));
        assert_eq!(
            styled(capture!("placeholder.recent-ansi.txt")),
            Some(ComposerRead::Empty)
        );
    }

    // Catches a typed draft read as a suggestion.
    #[test]
    fn captured_unstyled_text_is_a_draft() {
        for (ansi, text) in [
            (capture!("draft.recent-ansi.txt"), "hello draft"),
            (capture!("typed-prefix.recent-ansi.txt"), "run"),
            (
                capture!("multiline-draft.recent-ansi.txt"),
                "line one\nline two",
            ),
        ] {
            assert_eq!(styled(ansi), Some(ComposerRead::Text(text.into())));
        }
    }

    fn screen(composer: &str) -> String {
        let rule = format!("\x1b[38;2;136;136;136m{}\x1b[0m", "─".repeat(80));
        format!("{rule}\r\n{composer}\r\n{rule}\r\n  footer\r\n")
    }

    #[test]
    fn mixed_or_dimmed_composers_are_never_empty() {
        let mixed = screen("❯\u{a0}typed \x1b[2mghost\x1b[0m");
        assert!(matches!(
            styled(&mixed),
            Some(ComposerRead::Unsafe { text, reason: CLAUDE_MIXED_STYLE }) if text == "typed ghost"
        ));
        let coloured = screen("❯\u{a0}\x1b[2;38;5;1mred dim\x1b[0m");
        assert!(matches!(
            styled(&coloured),
            Some(ComposerRead::Unsafe {
                reason: CLAUDE_MIXED_STYLE,
                ..
            })
        ));
        // A dim prompt glyph: the whole composer may be drawn dim, a draft too.
        assert_eq!(
            styled(&screen("\x1b[2m❯\u{a0}draft drawn dim\x1b[0m")),
            None
        );
        // No composer on the styled screen: the detection read stands.
        assert_eq!(styled("\x1b[1mWelcome\x1b[0m\r\n"), None);
        // Blank composer and a dim multi-row placeholder are empty.
        assert_eq!(styled(&screen("❯")), Some(ComposerRead::Empty));
        let rows = "❯\u{a0}\x1b[2mfirst\r\n  second\x1b[0m";
        assert_eq!(styled(&screen(rows)), Some(ComposerRead::Empty));
    }

    #[test]
    fn only_undecidable_detection_reads_ask_for_styles() {
        for read in [
            ComposerRead::Empty,
            ComposerRead::Text("x".into()),
            ComposerRead::Unsafe {
                text: "x".into(),
                reason: "image placeholder in the composer",
            },
        ] {
            assert!(!NativeComposer.wants_styled(&read), "{read:?}");
        }
    }
}
