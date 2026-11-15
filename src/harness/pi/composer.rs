//! Default native TUI grammar, captured with a scripted provider. Unknown layouts refuse.
use crate::harness::{
    adapter::ComposerPolicy, composer::ComposerRead, recipe::PokeCapabilities, sgr,
};

const NEED_STYLED: &str = "pi empty composer requires styled cursor observation";
pub(super) struct PiComposer;

fn rule(line: &str) -> bool {
    line.chars().count() >= 10 && line.chars().all(|c| c == '─')
}

/// Two footer rows follow the last full-width border. Extra custom footer/status
/// rows are deliberately unsupported, rather than searched past as transcript.
fn bottom(lines: &[&str], width: Option<u16>) -> Option<usize> {
    let end = lines.iter().rposition(|line| !line.is_empty())? + 1;
    let bottom = end.checked_sub(3)?;
    let border = lines[bottom];
    // Herdr's native screen leaves the final column free; tmux's native screen
    // uses every column. Both were captured from the default Pi renderer.
    let columns = border.chars().count();
    if !rule(border)
        || width.is_some_and(|w| columns != usize::from(w) && columns + 1 != usize::from(w))
    {
        return None;
    }
    let cwd = lines[bottom + 1];
    if !(cwd.starts_with('/') || cwd.starts_with('~') || cwd.as_bytes().get(1) == Some(&b':')) {
        return None;
    }
    let stats = lines[bottom + 2];
    let after = if let Some((before, after)) = stats.split_once("%/") {
        let percent = before.split_whitespace().last()?;
        if percent.parse::<f64>().ok().is_none_or(|n| !n.is_finite()) {
            return None;
        }
        after
    } else {
        // Native post-compaction usage is unknown until a measured response.
        // Its default token is ?/capacity, without a percent sign.
        let (before, after) = stats.split_once("?/")?;
        if !before.is_empty() && !before.ends_with(char::is_whitespace) {
            return None;
        }
        after
    };
    if after.split_whitespace().count() < 2 {
        return None;
    }
    Some(bottom)
}

fn editor_row(lines: &[&str], width: Option<u16>) -> Option<usize> {
    let bottom = bottom(lines, width)?;
    let top = bottom.checked_sub(2)?;
    (rule(lines[top]) && lines[top] == lines[bottom]).then_some(top + 1)
}

impl ComposerPolicy for PiComposer {
    fn capabilities(&self, _: Option<&str>) -> PokeCapabilities {
        PokeCapabilities::NONE
    }
    fn read(&self, detection: &str, width: Option<u16>) -> ComposerRead {
        let lines: Vec<_> = detection.lines().map(str::trim_end).collect();
        let Some(row) = editor_row(&lines, width) else {
            return ComposerRead::Unreadable;
        };
        ComposerRead::Unsafe {
            text: lines[row].into(),
            reason: NEED_STYLED,
        }
    }
    fn wants_styled(&self, read: &ComposerRead) -> bool {
        matches!(read, ComposerRead::Unsafe { text, reason: NEED_STYLED } if text.is_empty())
    }
    fn read_styled(&self, screen: &[sgr::StyledLine], width: Option<u16>) -> Option<ComposerRead> {
        let lines: Vec<_> = screen.iter().map(|line| line.text.trim_end()).collect();
        let row = editor_row(&lines, width)?;
        let body = &screen[row];
        // Plain detection loses typed spaces. Require the observed default
        // reverse-video cursor at column zero, with nothing but plain padding.
        // Spaces with the cursor moved home are screen-equivalent to empty;
        // TRUST-POLICY records that accepted native-screen limit explicitly.
        let inverse = sgr::parse("\x1b[7m ")?.first()?.styles[0];
        let empty = body.text.chars().all(|c| c == ' ')
            && body.styles.first() == Some(&inverse)
            && body.styles.iter().skip(1).all(sgr::Style::is_plain);
        Some(if empty {
            ComposerRead::Empty
        } else {
            ComposerRead::Unreadable
        })
    }
    fn clear_key(&self) -> &'static str {
        "ctrl+u"
    }
    fn restore_text(&self, saved: &str) -> String {
        saved.into()
    }
    fn recognizes_blocked_screens(&self) -> bool {
        true
    }
    fn screen_blocked(&self, screen: &str) -> Option<bool> {
        let lines: Vec<_> = screen.lines().map(str::trim_end).collect();
        editor_row(&lines, None).map(|_| false)
    }
    fn screen_turn_running(&self, screen: &str) -> Option<bool> {
        let lines: Vec<_> = screen.lines().map(str::trim_end).collect();
        let bottom = bottom(&lines, None)?;
        let top = bottom.checked_sub(2)?;
        let header = lines[top];
        if let Some(rest) = header.strip_prefix("── ")
            && let Some((spinner, border)) = rest.split_once(" Working ")
            && spinner.chars().count() == 1
            && spinner.chars().all(|c| "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c))
            && rule(border)
        {
            return Some(true);
        }
        editor_row(&lines, None).map(|_| false)
    }
}
