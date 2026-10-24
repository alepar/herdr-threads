//! Captured native composer grammar and safe input recipe.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
};
use std::ops::Range;
const CODEX_PROMPT: &str = "›";
/// Codex's empty-composer placeholders, exact text: the main conversation's
/// and a `/side` conversation's (Codex 0.163 `chatwidget/constructor.rs`
/// `PLACEHOLDER`, `SIDE_PLACEHOLDER`; captured in
/// `docs/evidence/ansi-composer/`). The sub-agent view's placeholder is
/// deliberately absent: Codex blocks a typed submit there.
const CODEX_PLACEHOLDERS: &[&str] = &["Ask Codex to do anything", "Ask a follow-up question"];
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
        let empty = rows.len() == 1 && CODEX_PLACEHOLDERS.contains(&rows[0].as_str());
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
    use crate::{
        harness::composer::{ComposerRead, read_composer},
        protocol::authority::Harness,
    };

    macro_rules! capture {
        ($name:literal) => {
            include_str!(concat!(
                "../../../docs/evidence/ansi-composer/captures/codex-0.163.0-",
                $name,
                ".read-detection.txt"
            ))
        };
    }

    // Catches a `/side` conversation's placeholder refusing every wake as typed text.
    #[test]
    fn captured_side_placeholder_is_empty_and_a_side_draft_is_text() {
        let read = |text| read_composer(Harness::Codex, text, Some(120));
        assert_eq!(read(capture!("empty")), ComposerRead::Empty);
        assert_eq!(read(capture!("side")), ComposerRead::Empty);
        assert_eq!(
            read(capture!("draft")),
            ComposerRead::Text("hello draft".into())
        );
        assert_eq!(
            read(capture!("side-draft")),
            ComposerRead::Text("side draft".into())
        );
        assert_eq!(
            read("› Viewing sub-agent — direct input is disabled\n\n  footer\n"),
            ComposerRead::Text("Viewing sub-agent — direct input is disabled".into())
        );
    }
}
