//! Hermes composer policy, read from `agent read` captures of the default (modern)
//! prompt_toolkit layout of Hermes 0.22.5 (`docs/evidence/hermes-composer/`).
//!
//! The live screen, bottom up: one footer line (model, context, cwd and, when they fit,
//! key hints), a full-width `─` rule, the composer rows (`❯ ` first row, unindented continuation rows), a top rule
//! of the same width (`───…`, the session title may sit at its right end):
//!
//! ```text
//! ─────────────────────────────────────── Count slowly from 1 to 40 ─
//! ❯ Type / to browse commands, or Ctrl+P for the palette
//! ───────────────────────────────────────────────────────────────────
//!  ☤ gpt-6.1-sol · 16.5K/500K · … · …/work  / commands · @ files · ! shell
//! ```
//!
//! Herdr reports a Hermes pane `idle` throughout a turn, so the screen alone must tell
//! a busy one apart: while a turn or a command runs the top rule carries the live status
//! (`── ⠹ <activity> · 18s ───`) and the composer reads `☤ ❯ type to redirect this turn`.
//! The model picker and command palette blank the footer; the palette and the other
//! overlay prompts are bordered panels right above the top rule, and an approval, clarify,
//! sudo or secret prompt comes inside a turn (live status). Any of these, the telemetry
//! consent screen (no composer) and any other screen are `Unreadable`. The footer's key
//! hints decide nothing: Hermes drops them whenever they do not fit.
//!
//! The empty composer shows a placeholder: a random example prompt from Hermes' localized
//! catalog, drawn italic in the skin's dim colour; typed text has the default rendition.
//! The detection text cannot tell the two apart (`HERMES_NOT_KNOWN_EMPTY`); the styled
//! read can.
use std::ops::Range;

use unicode_width::UnicodeWidthStr;

use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
    sgr,
};

const PROMPT: &str = "❯";
/// The shortest run of `─` taken for the bottom rule.
const MIN_RULE: usize = 10;
/// Why detection-text Hermes composer text is not known to be a draft.
const HERMES_NOT_KNOWN_EMPTY: &str = "Hermes composer text may be a placeholder";
/// Why styled Hermes composer text is neither a placeholder nor a draft.
const HERMES_MIXED_STYLE: &str = "Hermes composer text mixes placeholder and typed styles";

pub(super) struct HermesComposer;

fn is_rule(line: &str) -> bool {
    line.chars().count() >= MIN_RULE && line.chars().all(|c| c == '─')
}

/// An idle top rule: no live status at its left (`── ⠹ …` starts with two), so at least
/// three `─`, an optional session title, a closing `─`, the bottom rule's width.
fn is_top_rule(line: &str, width: usize) -> bool {
    line.starts_with("───") && line.ends_with('─') && line.width() == width
}

/// A bordered overlay panel (`│ … │`, `╰──╯`) sits right above the top rule.
fn is_panel_row(line: &str) -> bool {
    line.starts_with(['│', '╰', '╭', '┃', '┗', '└'])
}

/// The composer rows of an idle screen, their line range and the rule width.
fn hermes_rows(lines: &[&str]) -> Option<(Vec<String>, Range<usize>, usize)> {
    let bottom = lines.iter().rposition(|line| is_rule(line))?;
    let width = lines[bottom].width();
    // Exactly one footer line follows the bottom rule; the model picker and the command
    // palette leave it blank.
    let mut footer = lines[bottom + 1..].iter().filter(|line| !line.is_empty());
    let (Some(_), None) = (footer.next(), footer.next()) else {
        return None;
    };
    // The first `─`-led line above the bottom rule must be the idle top rule: a busy top
    // rule (live status) must not let an older rule further up stand in for it.
    let top = lines[..bottom]
        .iter()
        .rposition(|line| line.starts_with('─'))?;
    if !is_top_rule(lines[top], width) || top + 1 == bottom {
        return None;
    }
    if top > 0 && is_panel_row(lines[top - 1]) {
        return None;
    }
    let body = top + 1..bottom;
    let first = lines[body.start];
    let first = if first == PROMPT {
        String::new()
    } else {
        first.strip_prefix("❯ ")?.to_owned()
    };
    let mut rows = vec![first];
    rows.extend(
        lines[body.start + 1..bottom]
            .iter()
            .map(|row| (*row).to_owned()),
    );
    Some((rows, body, width))
}

/// How the composer's non-blank characters are drawn.
enum RowStyle {
    /// No character: an empty composer.
    Blank,
    /// Every character italic: the placeholder.
    Placeholder,
    /// Every character in the default rendition: typed text.
    Plain,
    Mixed,
}

/// The style of `rows`, each read from its screen line (past `❯ ` on the first row).
/// `None` when the prompt glyph is italic: the busy composer draws it so.
fn hermes_row_style(
    screen: &[sgr::StyledLine],
    rows: &[String],
    body: &Range<usize>,
) -> Option<RowStyle> {
    if screen[body.start].styles.first()?.is_italic() {
        return None;
    }
    let mut style = RowStyle::Blank;
    for (index, (row, line)) in rows.iter().zip(&screen[body.clone()]).enumerate() {
        let lead = if index == 0 { 2 } else { 0 };
        let cells = line.text.chars().zip(&line.styles).skip(lead);
        for (_, cell) in cells
            .take(row.chars().count())
            .filter(|(c, _)| !c.is_whitespace())
        {
            // The placeholder is drawn on the first row only.
            let placeholder = index == 0 && cell.is_italic();
            style = match (style, placeholder, cell.is_plain()) {
                (RowStyle::Blank | RowStyle::Placeholder, true, _) => RowStyle::Placeholder,
                (RowStyle::Blank | RowStyle::Plain, _, true) => RowStyle::Plain,
                _ => return Some(RowStyle::Mixed),
            };
        }
    }
    Some(style)
}

impl ComposerPolicy for HermesComposer {
    fn capabilities(&self, _: Option<&str>) -> PokeCapabilities {
        PokeCapabilities::NONE
    }
    fn read(&self, detection: &str, _pane_width: Option<u16>) -> ComposerRead {
        let detection = detection.replace('\u{a0}', " ");
        let lines: Vec<&str> = detection.lines().map(str::trim_end).collect();
        let Some((rows, _, _)) = hermes_rows(&lines) else {
            return ComposerRead::Unreadable;
        };
        if rows.iter().all(String::is_empty) {
            return ComposerRead::Empty;
        }
        ComposerRead::Unsafe {
            text: rows.join("\n"),
            reason: HERMES_NOT_KNOWN_EMPTY,
        }
    }
    fn wants_styled(&self, detection: &ComposerRead) -> bool {
        matches!(
            detection,
            ComposerRead::Unsafe {
                reason: HERMES_NOT_KNOWN_EMPTY,
                ..
            }
        )
    }
    /// Italic first-row text is the placeholder (empty), unstyled text a draft, anything
    /// else unsafe.
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
        let (rows, body, width) = hermes_rows(&lines)?;
        let empty = match hermes_row_style(&screen, &rows, &body)? {
            RowStyle::Blank | RowStyle::Placeholder => true,
            RowStyle::Plain => false,
            RowStyle::Mixed => {
                return Some(ComposerRead::Unsafe {
                    text: rows.join("\n"),
                    reason: HERMES_MIXED_STYLE,
                });
            }
        };
        Some(composer::validate_rows(
            &raw,
            &lines,
            rows,
            body,
            pane_width.map(usize::from).or(Some(width)),
            empty,
            PROMPT,
        ))
    }
    /// Unused while stashing is unsupported.
    fn clear_key(&self) -> &'static str {
        "ctrl+u"
    }
    /// The saved text, retyped verbatim. Unused while stashing is unsupported.
    fn restore_text(&self, saved: &str) -> String {
        saved.into()
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/hermes_composer.rs"]
mod hermes_composer;
