//! Captured native composer grammar and safe input recipe.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
};
use std::ops::Range;
/// The shortest run of `─` taken for Claude's composer rule.
const MIN_RULE: usize = 10;
const CLAUDE_PROMPT: &str = "❯";
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

pub(super) struct NativeComposer;
impl ComposerPolicy for NativeComposer {
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
                reason: composer::CLAUDE_NOT_KNOWN_EMPTY,
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
                poke_during_turn: recipe.profile.poke_during_turn,
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
