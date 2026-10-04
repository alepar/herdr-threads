//! Captured native composer grammar and safe input recipe.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
};
use std::ops::Range;
const CODEX_PROMPT: &str = "›";
const CODEX_EMPTY: &str = "Ask Codex to do anything";
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

pub(super) struct NativeComposer;
impl ComposerPolicy for NativeComposer {
    fn read(&self, detection: &str, pane_width: Option<u16>) -> ComposerRead {
        let detection = detection.replace('\u{a0}', " ");
        let raw: Vec<&str> = detection.lines().collect();
        let lines: Vec<&str> = raw.iter().map(|line| line.trim_end()).collect();
        let Some((rows, body)) = codex_rows(&lines) else {
            return ComposerRead::Unreadable;
        };
        let empty = rows.len() == 1 && rows[0] == CODEX_EMPTY;
        composer::validate_rows(
            &raw,
            &lines,
            rows,
            body,
            pane_width.map(usize::from),
            empty,
            CODEX_PROMPT,
        )
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
