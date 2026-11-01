//! agy composer policy, read from `agent read --source detection` captures
//! (`tests/fixtures/agy/composer/`, recipe in its README).
//!
//! The live screen, bottom up: a footer line (`? for shortcuts …` when idle, blank-led
//! `… Gemini 3.1 Pro · low` while a draft is typed), a full-width `─` rule, the composer
//! rows (`>` first row, two-space continuation rows), a full-width rule. While a
//! subagent works, agy adds a status region between two more rules:
//!
//! ```text
//! ───────…      composer top
//! >
//! ───────…      composer bottom
//!   ● Agent(self: Probe Child)  Working · 1s
//! ───────…
//! ? for shortcuts …
//! ```
//!
//! Herdr reports such a pane idle (issue #2354), so any `Agent(<name>)` row in the live
//! region makes the screen `Unreadable`. A busy turn (`esc to cancel` footer), a
//! permission prompt, the shortcuts overlay and any other screen lack the idle footer or
//! the rule pair and are `Unreadable` too. Rows above the live region do not count.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
};

const PROMPT: &str = ">";
/// The shortest run of `─` taken for a rule.
const MIN_RULE: usize = 10;

pub(super) struct AgyComposer;

fn is_rule(line: &str) -> bool {
    line.chars().count() >= MIN_RULE && line.chars().all(|c| c == '─')
}

/// `● Agent(<name>)  <action> · <elapsed>`, with or without the bullet.
fn is_agent_row(line: &str) -> bool {
    let line = line.trim();
    let line = line.strip_prefix('●').map_or(line, str::trim_start);
    line.starts_with("Agent(") && line.contains(')')
}

/// The idle footer is `? for shortcuts …` (empty composer) or starts blank (a draft).
fn idle_footer(footer: &str) -> bool {
    footer.starts_with("? for shortcuts") || footer.starts_with(' ')
}

impl ComposerPolicy for AgyComposer {
    fn capabilities(&self, _: Option<&str>) -> PokeCapabilities {
        PokeCapabilities::NONE
    }
    fn read(&self, detection: &str, pane_width: Option<u16>) -> ComposerRead {
        let detection = detection.replace('\u{a0}', " ");
        let raw: Vec<&str> = detection.lines().collect();
        let lines: Vec<&str> = raw.iter().map(|line| line.trim_end()).collect();
        let Some(bottom) = lines.iter().rposition(|line| is_rule(line)) else {
            return ComposerRead::Unreadable;
        };
        let rule = lines[bottom].chars().count();
        // Exactly one footer line follows the bottom rule.
        let mut footer = lines[bottom + 1..].iter().filter(|line| !line.is_empty());
        match (footer.next(), footer.next()) {
            (Some(footer), None) if idle_footer(footer) => {}
            _ => return ComposerRead::Unreadable,
        }
        // Full-width rules only: the transcript's separators are shorter.
        let rules: Vec<usize> = (0..=bottom)
            .filter(|&index| is_rule(lines[index]) && lines[index].chars().count() == rule)
            .collect();
        let [.., top, _] = rules[..] else {
            return ComposerRead::Unreadable;
        };
        // The live region starts one rule above the composer's top rule, when there is one.
        let live = if rules.len() >= 3 {
            rules[rules.len() - 3]
        } else {
            top
        };
        if lines[live..].iter().any(|line| is_agent_row(line)) {
            return ComposerRead::Unreadable;
        }
        let body = &lines[top + 1..bottom];
        let Some(first) = body.first() else {
            return ComposerRead::Unreadable;
        };
        let first = if *first == PROMPT {
            String::new()
        } else if let Some(text) = first.strip_prefix("> ") {
            text.to_owned()
        } else {
            return ComposerRead::Unreadable;
        };
        let mut rows = vec![first];
        for line in &body[1..] {
            match line.strip_prefix("  ") {
                Some(row) => rows.push(row.to_owned()),
                None if line.is_empty() => rows.push(String::new()),
                None => return ComposerRead::Unreadable,
            }
        }
        // Trailing empty rows are padding (a cleared last line), not typed text.
        while rows.len() > 1 && rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        let empty = rows.iter().all(String::is_empty);
        composer::validate_rows(
            &raw,
            &lines,
            rows,
            top + 1..bottom,
            pane_width.map(usize::from).or(Some(rule)),
            empty,
            PROMPT,
        )
    }
    /// `ctrl+u` deletes back to the start of the line, and at a line start joins the
    /// row to the one above, so repeated presses clear every row (captures
    /// `multiline-after-ctrl-u-*`). Unused while stashing is unsupported.
    fn clear_key(&self) -> &'static str {
        "ctrl+u"
    }
    /// The saved text, retyped verbatim. Unused while stashing is unsupported.
    fn restore_text(&self, saved: &str) -> String {
        saved.into()
    }
    /// A swallowed Enter leaves the one-line wake in the composer, where it blocks every later
    /// wake (the composer reads `Text`); one `ctrl+u` clears a single row.
    fn clears_unsubmitted_wake(&self) -> bool {
        true
    }
    /// A permission prompt: the `↑/↓ Navigate` hint and a `> N.` option row among the last
    /// non-empty rows (`tests/fixtures/screen_blocked/agy/`). Otherwise a readable idle
    /// composer is not blocked; a busy turn or an overlay cannot be told.
    fn screen_blocked(&self, screen: &str) -> Option<bool> {
        let screen = screen.replace('\u{a0}', " ");
        let tail: Vec<&str> = screen
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .rev()
            .take(DIALOG_TAIL_ROWS)
            .collect();
        let hint = tail.iter().any(|line| line.starts_with("↑/↓ Navigate"));
        let option = tail.iter().any(|line| {
            line.strip_prefix("> ").is_some_and(|rest| {
                rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains(". ")
            })
        });
        if hint && option {
            return Some(true);
        }
        (self.read(&screen, None) != ComposerRead::Unreadable).then_some(false)
    }
    /// A generating turn replaces the `? for shortcuts` footer with `esc to cancel`, the last
    /// non-empty row (`tests/fixtures/screen_turn_running/agy/`). A readable composer without it
    /// is idle; anything else cannot be told.
    fn screen_turn_running(&self, screen: &str) -> Option<bool> {
        let screen = screen.replace('\u{a0}', " ");
        let footer = screen
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())?;
        // A permission prompt carries the same footer: it is blocked, not generating.
        if self.screen_blocked(&screen) == Some(true) {
            return None;
        }
        if footer.starts_with("esc to cancel") {
            return Some(true);
        }
        (self.read(&screen, None) != ComposerRead::Unreadable).then_some(false)
    }
}

/// How many trailing non-empty rows can hold a permission prompt's options and hint.
const DIALOG_TAIL_ROWS: usize = 8;

#[cfg(test)]
#[path = "../../../tests/harness/agy_composer.rs"]
mod agy_composer;
