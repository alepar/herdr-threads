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
    /// A running turn shows `Working (Ns • esc to interrupt)` among the rows above the composer
    /// (`tests/fixtures/screen_turn_running/codex/`). A readable composer without it is idle;
    /// anything else cannot be told.
    fn screen_turn_running(&self, screen: &str) -> Option<bool> {
        let screen = screen.replace('\u{a0}', " ");
        let running = screen
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .rev()
            .take(WORKING_TAIL_ROWS)
            .any(|line| {
                line.trim_start_matches('•')
                    .trim_start()
                    .starts_with("Working (")
                    && line.contains("esc to interrupt")
            });
        if running {
            return Some(true);
        }
        (self.read(&screen, None) != ComposerRead::Unreadable).then_some(false)
    }
}

/// How many trailing non-empty rows can hold the working status line and the composer.
const WORKING_TAIL_ROWS: usize = 8;

#[cfg(test)]
mod tests {
    use super::NativeComposer;
    use crate::{
        harness::{
            adapter::ComposerPolicy,
            composer::{ComposerRead, read_composer},
        },
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

    macro_rules! running_fixture {
        ($name:literal) => {
            include_str!(concat!(
                "../../../tests/fixtures/screen_turn_running/codex/",
                $name,
                ".txt"
            ))
        };
    }

    // Catches a working turn read as idle (a wake typed into it) or an idle composer read as
    // running (a stale busy never cleared).
    #[test]
    fn screen_turn_running_recognizes_the_working_line() {
        assert_eq!(
            NativeComposer.screen_turn_running(running_fixture!("running")),
            Some(true)
        );
        assert_eq!(
            NativeComposer.screen_turn_running(running_fixture!("idle")),
            Some(false)
        );
        assert_eq!(
            NativeComposer.screen_turn_running("not a codex screen"),
            None
        );
    }

    // Codex has no captured approval screen, so it keeps the `None` default (no wake).
    #[test]
    fn screen_blocked_is_unsupported() {
        assert_eq!(
            NativeComposer.screen_blocked("› Ask Codex to do anything\n"),
            None
        );
    }
}
