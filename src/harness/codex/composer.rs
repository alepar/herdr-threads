//! Captured native composer grammar and safe input recipe.
use crate::harness::{
    adapter::ComposerPolicy,
    composer::{self, ComposerRead},
    recipe::PokeCapabilities,
};
use std::ops::Range;
const CODEX_PROMPT: &str = "›";
/// Codex's empty-composer placeholder, exact text: the main conversation's (Codex 0.163
/// `chatwidget/constructor.rs` `PLACEHOLDER`; captured in `docs/evidence/ansi-composer/`).
/// A `/side` chat's placeholder (`SIDE_PLACEHOLDER`, see `CODEX_SIDE_PLACEHOLDER`) is deliberately
/// absent: the side chat is not the wake target (the root session is), so a side composer reads as
/// text and a native wake owed meanwhile is refused before sending and retried on the refusal
/// backoff until the side chat closes; a queue wake classifies it as an idle placeholder
/// (`is_idle_placeholder`). The sub-agent view's placeholder is absent too: Codex blocks
/// a typed submit there.
const CODEX_PLACEHOLDERS: &[&str] = &["Ask Codex to do anything"];
/// The `/side` chat's empty composer (Codex 0.163 `SIDE_PLACEHOLDER`); absent from
/// `CODEX_PLACEHOLDERS` on purpose, see there.
const CODEX_SIDE_PLACEHOLDER: &str = "Ask a follow-up question";
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

/// Codex's side-by-side layout: a wide terminal shows the root conversation and the `/side` chat
/// in two panes split by `│` (observed live on 0.172.0, ht-z41e.43), so the composer row reads
/// `›  ...  │› Ask a follow-up question`. The divider is the column that holds a `│` on at least
/// three rows and on at least half of the non-blank rows, and the screen is split when a row to
/// its right starts with the side pane's own composer prompt. The two composers need not share a
/// row: each pane draws its composer below its own content and the narrower one wraps its footer
/// onto more rows (ht-z41e.53). The screen is cut at the divider's column into the root pane and
/// the side pane (a row without the divider there keeps its text in the root pane). `None` for an
/// ordinary single-pane screen.
fn split_panes(detection: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = detection.lines().collect();
    let mut dividers = std::collections::BTreeMap::<usize, usize>::new();
    for line in &lines {
        for (column, _) in line.chars().enumerate().filter(|&(_, c)| c == '│') {
            *dividers.entry(column).or_default() += 1;
        }
    }
    let non_blank = lines.iter().filter(|line| !line.trim().is_empty()).count();
    // The leftmost of the best-supported columns.
    let (column, rows) = dividers
        .into_iter()
        .rev()
        .max_by_key(|&(column, rows)| (rows, std::cmp::Reverse(column)))?;
    if rows < 3 || rows * 2 < non_blank {
        return None;
    }
    let (mut root, mut side, mut side_composer) = (Vec::new(), Vec::new(), false);
    for line in lines {
        if line.chars().nth(column) == Some('│') {
            root.push(line.chars().take(column).collect::<String>());
            let right = line.chars().skip(column + 1).collect::<String>();
            side_composer |= right.starts_with(CODEX_PROMPT);
            side.push(right);
        } else {
            root.push(line.to_owned());
            side.push(String::new());
        }
    }
    side_composer.then(|| (root.join("\n"), side.join("\n")))
}

pub(super) struct NativeComposer;
impl NativeComposer {
    /// One single-pane screen.
    fn read_pane(&self, detection: &str, pane_width: Option<u16>) -> ComposerRead {
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
}
impl ComposerPolicy for NativeComposer {
    fn read(&self, detection: &str, pane_width: Option<u16>) -> ComposerRead {
        let Some((root, side)) = split_panes(detection) else {
            return self.read_pane(detection, pane_width);
        };
        // The panes are narrower than the terminal: no width rule applies to either half. Typed
        // text in the root pane is the read; an empty root pane reads as the side pane (its
        // placeholder is what a queue wake admits, native typing refuses it as text).
        match self.read_pane(&root, None) {
            ComposerRead::Empty => self.read_pane(&side, None),
            other => other,
        }
    }
    /// The `/side` chat's empty composer (Codex 0.163 `SIDE_PLACEHOLDER`): a queue wake starts the
    /// root session's turn and leaves the side view untouched (spike S2/S4). The exact text is
    /// matched whatever `Unsafe` reason the read carries (the native host reads with no pane width).
    fn is_idle_placeholder(&self, text: &str) -> bool {
        text == CODEX_SIDE_PLACEHOLDER
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

    // Catches a /side chat's placeholder read as an empty composer (a wake typed into the side chat).
    #[test]
    fn captured_side_placeholder_is_not_empty_and_a_side_draft_is_text() {
        let read = |text| read_composer(Harness::Codex, text, Some(120));
        assert_eq!(read(capture!("empty")), ComposerRead::Empty);
        assert_eq!(
            read(capture!("side")),
            ComposerRead::Text("Ask a follow-up question".into())
        );
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

    // Catches the side-by-side layout (a wide terminal, captured live on 0.172.0, ht-z41e.43) read
    // as typed text: the divider and the other pane's text made every split screen non-empty, so a
    // queue wake beside an open /side chat was always refused. Root draft, side draft and a
    // placeholder-shaped draft in the root pane stay text.
    #[test]
    fn side_by_side_panes_read_per_pane() {
        let screen = |root: &str, side: &str| {
            let row = |l: &str, r: &str| format!("{l:<79}│{r}\n");
            let mut text = String::new();
            text += &row("• Accepted.", "• Waiting for background terminal (1m …");
            text += &row("", "  └ sleep 100");
            text += &row(root, side);
            text += &row("", "");
            text += &row(
                "  GPT-6.2-Sol default · ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-",
                "  GPT-6.2-Sol default · ~/AleCode/",
            );
            text += &row("  config-side-wake", "  ← for agents · ? for shortcuts");
            text
        };
        let read =
            |root: &str, side: &str| read_composer(Harness::Codex, &screen(root, side), Some(160));
        let side = ComposerRead::Text("Ask a follow-up question".into());
        let is_side = |read: ComposerRead| match read {
            ComposerRead::Text(t) | ComposerRead::Unsafe { text: t, .. } => {
                NativeComposer.is_idle_placeholder(&t)
            }
            _ => false,
        };
        // Empty root pane, side placeholder: the side placeholder (a queue wake admits it).
        assert!(is_side(read("›", "› Ask a follow-up question")), "{side:?}");
        // Empty root pane (its own placeholder), side placeholder: same.
        assert!(is_side(read(
            "› Ask Codex to do anything",
            "› Ask a follow-up question"
        )));
        // Root draft: text, never the side placeholder.
        let draft = read("› root draft", "› Ask a follow-up question");
        assert!(!is_side(draft.clone()));
        assert!(matches!(
            draft,
            ComposerRead::Text(ref t) | ComposerRead::Unsafe { text: ref t, .. } if t == "root draft"
        ));
        // Side draft: text, not the placeholder.
        let side_draft = read("›", "› side draft");
        assert!(!is_side(side_draft.clone()));
        assert!(matches!(
            side_draft,
            ComposerRead::Text(ref t) | ComposerRead::Unsafe { text: ref t, .. } if t == "side draft"
        ));
        // A single-pane draft that merely contains a divider is still read as one pane.
        assert!(!is_side(read_composer(
            Harness::Codex,
            "› a │ b\n\n  footer\n",
            Some(120)
        )));
    }

    // Catches the wide side-by-side layout (a 155-column pane, captured live on the shared Herdr
    // server, ht-z41e.53) refused as typed text: the narrower side pane wraps its footer onto more
    // rows, so its composer sits on a different row than the root's, no row carried both
    // prompts, the screen read as one pane and the divider `│` read as the root composer's text.
    #[test]
    fn side_by_side_panes_read_per_pane_when_the_composers_sit_on_different_rows() {
        const ROOT_WIDTH: usize = 103;
        // The root pane: history, then its composer row and a two-row footer. The side pane: its
        // forked history, then its composer row `side_composer` and a four-row footer.
        let screen = |root_composer: &str, side_composer: &str, side_offset: isize| {
            let mut root = vec![String::new(); 36];
            root[0] = "  >_ OpenAI Codex (v0.172.0)".into();
            root[10] = "• Accepted.".into();
            root[12] = "  Worked for 5s • 13:40".into();
            root[15] = root_composer.into();
            root[17] =
                "  GPT-6.2-Sol default · ~/AleCode/herdr-threads/.worktrees/super-auto-per-seat-"
                    .into();
            root[18] =
                "  config-side-wake/.worktrees/super-auto-per-seat-config-side-wake · Accept test"
                    .into();
            let mut side = vec![String::new(); 36];
            side[0] = " Run side-…×          +".into();
            side[2] = "› Side of wake-e2e: run inbox".into();
            side[10] = "• SIDE-QUEUE-DONE".into();
            let at = usize::try_from(15 + side_offset).unwrap();
            side[at] = side_composer.into();
            side[at + 2] = "  GPT-6.2-Sol default · ~/AleCode/".into();
            side[at + 3] = "  herdr-threads/.worktrees/super-auto-".into();
            side[at + 4] = "  per-seat-config-side-wake/.worktrees…".into();
            side[at + 5] = "  ← for agents · ? for shortcuts".into();
            root.iter()
                .zip(&side)
                .map(|(l, r)| format!("{l:<ROOT_WIDTH$}│{r}\n"))
                .collect::<String>()
        };
        let is_side = |read: ComposerRead| match read {
            ComposerRead::Text(t) | ComposerRead::Unsafe { text: t, .. } => {
                NativeComposer.is_idle_placeholder(&t)
            }
            _ => false,
        };
        let text_of = |read: ComposerRead| match read {
            ComposerRead::Text(t) | ComposerRead::Unsafe { text: t, .. } => t,
            other => panic!("no text in {other:?}"),
        };
        let read = |text: &str| read_composer(Harness::Codex, text, Some(155));
        for offset in [-2, 0, 2, 3] {
            let at = format!("side composer {offset} rows from the root's");
            let idle = screen("›", "› Ask a follow-up question", offset);
            assert!(is_side(read(&idle)), "{at}: {:?}", read(&idle));
            // A root draft stays text and is never the side placeholder.
            let draft = read(&screen(
                "› root draft",
                "› Ask a follow-up question",
                offset,
            ));
            assert_eq!(text_of(draft), "root draft", "{at}");
            // A side draft stays text and is not the placeholder.
            let side_draft = read(&screen("›", "› side draft", offset));
            assert_eq!(text_of(side_draft), "side draft", "{at}");
        }
    }

    // Catches a side draft, a sub-agent row or the main placeholder classified as the side placeholder.
    #[test]
    fn side_placeholder_is_the_only_idle_placeholder() {
        let text_of = |r: ComposerRead| match r {
            ComposerRead::Text(t) | ComposerRead::Unsafe { text: t, .. } => t,
            other => panic!("no text in {other:?}"),
        };
        for width in [None, Some(120)] {
            let read = |text| read_composer(Harness::Codex, text, width);
            let side = text_of(read(capture!("side")));
            assert_eq!(side, "Ask a follow-up question");
            assert!(NativeComposer.is_idle_placeholder(&side), "{width:?}");
            for text in [
                text_of(read(capture!("draft"))),
                text_of(read(capture!("side-draft"))),
                text_of(read(
                    "› Viewing sub-agent — direct input is disabled\n\n  footer\n",
                )),
                "Ask Codex to do anything".to_owned(),
            ] {
                assert!(
                    !NativeComposer.is_idle_placeholder(&text),
                    "{width:?} {text:?}"
                );
            }
        }
        assert!(matches!(
            read_composer(Harness::Codex, capture!("side"), None),
            ComposerRead::Unsafe { .. }
        ));
        assert_eq!(
            read_composer(Harness::Codex, "unknown layout", None),
            ComposerRead::Unreadable
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
