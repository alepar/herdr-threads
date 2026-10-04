//! IRC-style human transcript of a thread: `[HH:MM] <nick> message`.
//!
//! Used by the human form of `read` and by `read --follow`. A nick is the
//! author seat's Herdr space/tab/pane label relative to the live caller,
//! otherwise the short seat ID, with the seat's current binding harness as a
//! suffix (`alice·claude`) when known. System events (joins, accepts, leaves,
//! ACKs, warnings) become `-!-` notice lines. Bodies are shown in full,
//! wrapped under the message column; very long bodies are folded with a hint
//! naming the command that prints the whole body.
//!
//! Every peer-supplied string (pane labels, bodies, topics, event fields) is
//! escaped with the same rules as the rest of the human renderer, so a peer
//! cannot drive the terminal. The only escape sequences this module emits are
//! its own SGR colors, and only when [`Style::color`] is set (a terminal).

use super::human::{multi_line, one_line};
use crate::protocol::{
    ids::SeatId,
    output::{format_command_argv, other_day, render_now},
    pagination::Page,
    results::{MessageKind, MessageSummary},
    service::EventAuthor,
    time::UtcMillis,
};
use crate::view::escape::display_width;
use serde_json::Value;

/// Longest nick shown, in characters (an ellipsis marks clipping).
const NICK_MAX: usize = 32;
/// Most ACKed message IDs listed on one notice line.
const ACK_IDS_SHOWN: usize = 3;

/// Presentation knobs. [`Style::plain`] is deterministic for tests and pipes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    /// Wrap column (characters).
    pub width: usize,
    /// Per-nick ANSI colors; only ever set for a terminal.
    pub color: bool,
    /// Most display lines one message body may take before it is folded.
    pub max_lines: usize,
    /// Show the viewer's local wall clock (`true`) or UTC (`false`).
    pub local_time: bool,
}

impl Style {
    pub fn plain() -> Self {
        Self {
            width: 100,
            color: false,
            max_lines: 20,
            local_time: false,
        }
    }
}

/// A display name for a seat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nick {
    /// Pane name, label or short seat ID (untrusted; escaped on display).
    pub name: String,
    /// The seat's current binding harness (`claude`, `codex`, `human`).
    pub harness: Option<String>,
}

impl Nick {
    pub fn seat(seat: &SeatId) -> Self {
        Self {
            name: short_seat(seat.as_str()),
            harness: None,
        }
    }

    /// `name·harness`, escaped and clipped. The harness suffix is left out
    /// when the name already says it (`mad-tea-hatter-codex`).
    pub fn display(&self) -> String {
        let bound = if self.name.contains('/') {
            (NICK_MAX + 1) * 3 + 2
        } else {
            NICK_MAX
        };
        let escaped = crate::view::escape::escape_for_terminal(
            &self.name,
            crate::view::escape::Context::SingleLine,
        );
        let name = one_line(&escaped, false, bound);
        match &self.harness {
            Some(harness)
                if !harness.is_empty()
                    && !self.name.to_lowercase().contains(&harness.to_lowercase()) =>
            {
                format!("{name}·{}", one_line(harness, false, 12))
            }
            _ => name,
        }
    }
}

/// Display a pane relative to the live caller's canonical parent IDs.
/// Shared with handoff; labels are presentation only and never locate a seat.
pub fn relative_pane_nick(
    pane: &crate::host::observation::SeatHostLabels,
    caller: Option<&crate::host::observation::SeatHostLabels>,
) -> String {
    let label = |name: &Option<String>, id: &str| {
        let raw = name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(id);
        let escaped =
            crate::view::escape::escape_for_terminal(raw, crate::view::escape::Context::SingleLine);
        one_line(&escaped, false, NICK_MAX)
    };
    let mut parts = Vec::new();
    let same_space = caller.is_some_and(|caller| caller.workspace_id == pane.workspace_id);
    if !same_space {
        parts.push(label(&pane.workspace_label, &pane.workspace_id));
    }
    if !same_space || caller.is_none_or(|caller| caller.tab_id != pane.tab_id) {
        parts.push(label(&pane.tab_label, &pane.tab_id));
    }
    parts.push(label(&pane.pane_label, pane.target.as_str()));
    parts.join("/")
}

/// A seat ID short enough for a nick: compact IDs are kept whole; persisted
/// UUID-suffixed IDs keep eight suffix characters.
pub fn short_seat(seat: &str) -> String {
    match seat.split_once('-') {
        Some((prefix, suffix)) if suffix.chars().count() > 8 => {
            format!("{prefix}-{}", suffix.chars().take(8).collect::<String>())
        }
        _ => seat.to_owned(),
    }
}

/// The full content behind a summary whose preview was clipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Full {
    Text {
        data: String,
        complete: bool,
        more_argv: Option<Vec<String>>,
    },
    Event(Value),
}

/// Nick and body lookups. The defaults need no daemon or host: the short
/// seat ID and the (possibly clipped) preview.
pub trait Lookup {
    fn nick(&mut self, seat: &SeatId) -> Nick {
        Nick::seat(seat)
    }
    fn full(&mut self, _summary: &MessageSummary) -> Option<Full> {
        None
    }
}

/// [`Lookup`] with only the offline defaults.
pub struct NoLookup;
impl Lookup for NoLookup {}

const NICK_COLORS: [&str; 6] = ["31", "32", "33", "35", "36", "34"];

fn paint(text: &str, code: &str, style: &Style) -> String {
    if style.color {
        format!("\u{1b}[{code}m{text}\u{1b}[0m")
    } else {
        text.to_owned()
    }
}

fn nick_color(name: &str) -> &'static str {
    let hash = name
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    NICK_COLORS[hash as usize % NICK_COLORS.len()]
}

/// `HH:MM` for a Unix-millisecond timestamp, in UTC or local time; a time on
/// another date than the render-time "now" (earlier or later, in the same
/// zone) is `MM-DD HH:MM`.
pub fn clock(at: UtcMillis, local: bool) -> String {
    let secs = at.0.div_euclid(1000);
    if local && let Some(shown) = local_clock(secs) {
        let now = local_clock(render_now().0.div_euclid(1000));
        let other_day = now.is_some_and(|now| now.date != shown.date);
        return shown.format(other_day);
    }
    let rem = secs.rem_euclid(86_400);
    let time = format!("{:02}:{:02}", rem / 3600, (rem % 3600) / 60);
    match other_day(at) {
        Some((month, day)) => format!("{month:02}-{day:02} {time}"),
        None => time,
    }
}

/// A wall-clock reading: its civil date `(year, month, day)` and time.
struct WallClock {
    date: (i32, i32, i32),
    hour: i32,
    minute: i32,
}

impl WallClock {
    fn format(&self, with_date: bool) -> String {
        let time = format!("{:02}:{:02}", self.hour, self.minute);
        if with_date {
            format!("{:02}-{:02} {time}", self.date.1, self.date.2)
        } else {
            time
        }
    }
}

fn local_clock(secs: i64) -> Option<WallClock> {
    let time = libc::time_t::try_from(secs).ok()?;
    // SAFETY: localtime_r writes only into the zeroed `tm` we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::localtime_r(&time, &mut tm) };
    if result.is_null() {
        return None;
    }
    Some(WallClock {
        date: (tm.tm_year, tm.tm_mon + 1, tm.tm_mday),
        hour: tm.tm_hour,
        minute: tm.tm_min,
    })
}

/// The author nick of a message, or `None` for a built-in system event.
fn author(summary: &MessageSummary, lookup: &mut dyn Lookup) -> Option<Nick> {
    match (&summary.event_author, &summary.author) {
        (Some(EventAuthor::Programmatic(service)), _) => Some(Nick {
            name: summary
                .actor_label
                .clone()
                .unwrap_or_else(|| service.as_str().to_owned()),
            harness: None,
        }),
        (Some(EventAuthor::Native(seat)), _) | (_, Some(seat)) => Some(lookup.nick(seat)),
        (Some(EventAuthor::BuiltIn), None) | (None, None) => {
            summary.actor_label.as_ref().map(|label| Nick {
                name: label.clone(),
                harness: None,
            })
        }
    }
}

/// Render one message as IRC lines (each ending in `\n`).
pub fn render_message(summary: &MessageSummary, lookup: &mut dyn Lookup, style: &Style) -> String {
    let time = format!("[{}]", clock(summary.created_at, style.local_time));
    let time_shown = paint(&time, "2", style);
    let mut out = String::new();
    match summary.kind {
        MessageKind::Ordinary => {
            let nick =
                author(summary, lookup).map_or_else(|| "system".to_owned(), |nick| nick.display());
            let markers = summary.author_markers();
            let prefix_plain = format!("{time} <{nick}>{markers} ");
            let prefix_shown = format!(
                "{time_shown} <{}>{markers} ",
                paint(&nick, nick_color(&nick), style)
            );
            let (body, hint) = ordinary_body(summary, lookup);
            emit(
                &prefix_plain,
                &prefix_shown,
                &body,
                hint,
                summary,
                style,
                &mut out,
            );
        }
        // IRC view (user decision 2026-10-01): of the informational notices
        // only channel joins and leaves are shown; ACKs, invitations, topic
        // and other bookkeeping stay in --json and the machine form.
        // Warnings are always shown.
        MessageKind::Info if !is_join_or_leave(summary) => return String::new(),
        MessageKind::Info | MessageKind::Warn => {
            let text = system_text(summary, lookup);
            let marker = if summary.kind == MessageKind::Warn {
                paint("-!-", "1;31", style)
            } else {
                paint("-!-", "1;34", style)
            };
            let prefix_plain = format!("{time} -!- ");
            let prefix_shown = format!("{time_shown} {marker} ");
            emit(
                &prefix_plain,
                &prefix_shown,
                &text,
                None,
                summary,
                style,
                &mut out,
            );
        }
    }
    out
}

/// A one-off `-!-` notice line from the follower itself (reconnects, the
/// thread topic). `text` is trusted presentation text, but is escaped anyway.
pub fn notice(text: &str, at: Option<UtcMillis>, style: &Style) -> String {
    let time = at.map_or_else(
        || "[--:--]".to_owned(),
        |at| format!("[{}]", clock(at, style.local_time)),
    );
    format!(
        "{} {} {}\n",
        paint(&time, "2", style),
        paint("-!-", "1;34", style),
        one_line(text, false, usize::MAX)
    )
}

/// The ordinary body to show, plus a fold/continuation hint when the shown
/// text is not the whole body.
fn ordinary_body(summary: &MessageSummary, lookup: &mut dyn Lookup) -> (String, Option<String>) {
    if !summary.preview_omitted {
        return (summary.preview_data.clone(), None);
    }
    match lookup.full(summary) {
        Some(Full::Text {
            data,
            complete,
            more_argv,
        }) => {
            let hint = (!complete).then(|| {
                more_argv.as_ref().map_or_else(
                    || body_hint(summary),
                    |argv| format!("continued: {}", format_command_argv(argv)),
                )
            });
            (data, hint)
        }
        _ => (
            format!("{}…", summary.preview_data),
            Some(body_hint(summary)),
        ),
    }
}

fn body_hint(summary: &MessageSummary) -> String {
    match &summary.preview_detail_argv {
        Some(argv) => format!("full message: {}", format_command_argv(argv)),
        None => format!(
            "full message: herdr-threads body {}",
            summary.message.as_str()
        ),
    }
}

/// Wrap `body` under the message column and fold it past `style.max_lines`.
fn emit(
    prefix_plain: &str,
    prefix_shown: &str,
    body: &str,
    hint: Option<String>,
    summary: &MessageSummary,
    style: &Style,
    out: &mut String,
) {
    let prefix_width = display_width(prefix_plain);
    let indent = if prefix_width <= style.width / 2 {
        prefix_width
    } else {
        8
    };
    let avail = style.width.saturating_sub(indent).max(10);
    let first_avail = style.width.saturating_sub(prefix_width).max(10);
    let safe = multi_line(body).replace('\t', "    ");
    let safe = safe.trim_end_matches('\n');
    let mut lines = Vec::new();
    for (index, paragraph) in safe.split('\n').enumerate() {
        let first = if index == 0 { first_avail } else { avail };
        wrap(paragraph, first, avail, &mut lines);
    }
    let pad = " ".repeat(indent);
    let total = lines.len();
    let mut hint = hint;
    let shown = if total > style.max_lines.max(2) {
        let keep = style.max_lines.max(2) - 1;
        let folded = total - keep;
        hint = Some(match hint {
            Some(existing) => format!("{folded} more lines; {existing}"),
            None => format!("{folded} more lines; {}", body_hint(summary)),
        });
        keep
    } else {
        total
    };
    for (index, line) in lines.iter().take(shown).enumerate() {
        if index == 0 {
            out.push_str(prefix_shown);
        } else {
            out.push_str(&pad);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if let Some(hint) = hint {
        out.push_str(&pad);
        out.push_str(&paint(&format!("… ({hint})"), "2", style));
        out.push('\n');
    }
}

/// Greedy word wrap of one paragraph; words longer than a line are split.
fn wrap(paragraph: &str, first: usize, rest: usize, lines: &mut Vec<String>) {
    let mut current = String::new();
    let mut width = 0;
    let mut limit = first;
    let start = lines.len();
    let flush = |current: &mut String, width: &mut usize, lines: &mut Vec<String>| {
        lines.push(std::mem::take(current));
        *width = 0;
    };
    for word in paragraph.split(' ') {
        let len = display_width(word);
        let needed = if width == 0 { len } else { width + 1 + len };
        if needed <= limit {
            if width > 0 {
                current.push(' ');
            }
            current.push_str(word);
            width = needed;
            continue;
        }
        if width > 0 {
            flush(&mut current, &mut width, lines);
            limit = rest;
        }
        let mut chars = word.chars().peekable();
        while chars.peek().is_some() {
            let mut piece = String::new();
            let mut piece_len = 0;
            while let Some(&next) = chars.peek() {
                let next_width = display_width(next.encode_utf8(&mut [0; 4]));
                if piece_len + next_width > limit && !piece.is_empty() {
                    break;
                }
                piece.push(next);
                piece_len += next_width;
                chars.next();
            }
            current.push_str(&piece);
            width = piece_len;
            if chars.peek().is_some() {
                flush(&mut current, &mut width, lines);
                limit = rest;
            }
        }
    }
    if width > 0 || lines.len() == start {
        lines.push(current);
    }
}

/// Field of a structured event as a string, if present.
fn field<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event.get(key).and_then(Value::as_str)
}

fn seat_nick(event: &Value, key: &str, lookup: &mut dyn Lookup) -> String {
    field(event, key).map_or_else(
        || "someone".to_owned(),
        |seat| lookup.nick(&SeatId::new(seat)).display(),
    )
}

/// Whether an informational event is a channel join or leave.
fn is_join_or_leave(summary: &MessageSummary) -> bool {
    serde_json::from_str::<Value>(&summary.preview_data)
        .ok()
        .and_then(|event| field(&event, "action").map(str::to_owned))
        .is_some_and(|action| matches!(action.as_str(), "accept" | "accept_required" | "leave"))
}

/// The `-!-` text of a system event.
fn system_text(summary: &MessageSummary, lookup: &mut dyn Lookup) -> String {
    let parsed = if summary.preview_omitted {
        match lookup.full(summary) {
            Some(Full::Event(value)) => Some(value),
            Some(Full::Text { data, .. }) => serde_json::from_str(&data).ok(),
            None => serde_json::from_str(&summary.preview_data).ok(),
        }
    } else {
        serde_json::from_str(&summary.preview_data).ok()
    };
    let Some(event) = parsed else {
        let marker = if summary.kind == MessageKind::Warn {
            "warning"
        } else {
            "notice"
        };
        let mut text = format!("{marker}: {}", summary.preview_data);
        if summary.preview_omitted {
            text.push('…');
        }
        return text;
    };
    // An event recorded by a seat or service: who did it.
    let actor = |lookup: &mut dyn Lookup| {
        author(summary, lookup).map_or_else(|| "someone".to_owned(), |nick| nick.display())
    };
    if let Some(action) = field(&event, "action") {
        return match action {
            "create_thread" => {
                let who = if field(&event, "actor_seat").is_some() {
                    seat_nick(&event, "actor_seat", lookup)
                } else {
                    actor(lookup)
                };
                match field(&event, "goal") {
                    Some(goal) => format!("{who} created the thread (goal: {goal})"),
                    None => format!("{who} created the thread"),
                }
            }
            "invite" => format!(
                "{} invited {}",
                seat_nick(&event, "actor_seat", lookup),
                seat_nick(&event, "seat", lookup)
            ),
            "service_invite" => {
                let required = event.get("required").and_then(Value::as_bool) == Some(true);
                format!(
                    "{} invited {}{}",
                    actor(lookup),
                    seat_nick(&event, "seat", lookup),
                    if required { " (required)" } else { "" }
                )
            }
            "operator_orphan_invite" => {
                format!("operator invited {}", seat_nick(&event, "seat", lookup))
            }
            "accept" | "accept_required" => {
                format!("{} joined", seat_nick(&event, "seat", lookup))
            }
            "leave" => format!("{} left", seat_nick(&event, "seat", lookup)),
            "archive" => format!(
                "{} archived the thread",
                seat_nick(&event, "actor_seat", lookup)
            ),
            "reopen" => format!(
                "{} reopened the thread",
                seat_nick(&event, "actor_seat", lookup)
            ),
            "set_topic" => format!(
                "{} changed the topic to: {}",
                seat_nick(&event, "actor_seat", lookup),
                field(&event, "topic").unwrap_or("")
            ),
            "release_requirement" => format!(
                "{} released the required membership of {}",
                actor(lookup),
                seat_nick(&event, "seat", lookup)
            ),
            other => format!("{} {other}", actor(lookup)),
        };
    }
    if let Some(kind) = field(&event, "event") {
        return match kind {
            "ack" => {
                let who = seat_nick(&event, "seat", lookup);
                match event.get("messages").and_then(Value::as_array) {
                    Some(ids) => {
                        let mut list: Vec<&str> = ids
                            .iter()
                            .filter_map(Value::as_str)
                            .take(ACK_IDS_SHOWN)
                            .collect();
                        let extra = ids.len().saturating_sub(list.len());
                        let more = if extra > 0 {
                            format!(" (+{extra} more)")
                        } else {
                            String::new()
                        };
                        if list.is_empty() {
                            list.push("a message");
                        }
                        format!("{who} ACKed {}{more}", list.join(", "))
                    }
                    None => match event.get("count").and_then(Value::as_u64) {
                        Some(count) => format!("{who} ACKed {count} messages"),
                        None => format!("{who} ACKed"),
                    },
                }
            }
            "recipient_unavailable" => {
                format!("{} is unavailable", seat_nick(&event, "seat", lookup))
            }
            "system_notify" => match event.get("data") {
                Some(Value::String(text)) => format!("{}: {text}", actor(lookup)),
                Some(data) => format!("{}: {data}", actor(lookup)),
                None => format!("{}: notice", actor(lookup)),
            },
            "operator_service_disconnect" => "a service was disconnected".to_owned(),
            other => format!("{other}: {event}"),
        };
    }
    if let Some(obligation) = field(&event, "obligation") {
        let what = if obligation == "invitation" {
            "invitation"
        } else {
            "ACK"
        };
        return format!(
            "warning: {} is overdue on an {what}",
            seat_nick(&event, "seat", lookup)
        );
    }
    format!("notice: {event}")
}

/// A history page as an IRC transcript, oldest first. `more:` names the
/// command for the next (older or newer) page when there is one.
pub fn render_page(page: &Page<MessageSummary>, lookup: &mut dyn Lookup, style: &Style) -> String {
    let mut out = String::new();
    if page.items.is_empty() {
        out.push_str("No messages.\n");
    }
    let mut items: Vec<&MessageSummary> = page.items.iter().collect();
    items.sort_by_key(|summary| summary.sequence);
    for summary in items {
        out.push_str(&render_message(summary, lookup, style));
    }
    if page.has_more
        && let Some(argv) = &page.next_argv
    {
        out.push_str(&format!("more: {}\n", format_command_argv(argv)));
    }
    out
}

#[cfg(test)]
#[path = "../../tests/cli/irc.rs"]
mod tests;
