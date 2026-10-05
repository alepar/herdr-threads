//! Compact machine text (ht-4is.8.18): the non-TTY default an agent's tool
//! call sees for the high-traffic reads. One line per row, page metadata only
//! as a single trailing `next:` command when there is more, and no JSON blobs.
//! `--json` keeps the full structured form; kinds without a form here use the
//! generic `key: value` encoder.
//!
//! Layout rules every form follows:
//! - the first line is the result kind (optionally followed by its subject);
//! - a row's service-generated fields (sequence, IDs, seat, time) come first;
//!   peer-supplied text only ever follows the first `": "` of a row and is
//!   escaped to one line, so it cannot fake a row, a field or a command line;
//! - a command line is `label: herdr-threads ...`, quoted for a POSIX shell;
//! - `body` is the one multi-line exception: its header row comes first and
//!   every body line after it is indented by two spaces, so no body line can
//!   start at column 0 where a row or a command line would.

use super::{
    OutputSpec, detail_argv, format_command_argv, needs_terminal_escape, other_day, text_json,
};
use crate::protocol::{
    pagination::Page,
    results::{
        CheckInResult, CommandResult, InboxBatchItem, InboxItem, MessageContent, MessageDetails,
        MessageKind, MessageSummary, Participant, PendingReceipt, ThreadDetails, WarningRef,
    },
    service::EventAuthor,
    time::UtcMillis,
};
use crate::view::escape::{Context, escape_for_terminal, push_u4};
use serde_json::Value;
use std::borrow::Cow;

/// Most IDs listed for one event row (`+N` counts the rest).
const EVENT_IDS_SHOWN: usize = 3;

pub(super) fn render(result: &CommandResult, spec: &OutputSpec) -> Option<String> {
    let mut out = String::new();
    match result {
        CommandResult::ThreadName(value) => {
            out.push_str(&format!(
                "thread_name {}: {}\n",
                value.thread.as_str(),
                value
                    .name
                    .as_deref()
                    .map_or_else(|| "unnamed".to_owned(), one_line)
            ));
        }
        CommandResult::History(page) => {
            out.push_str("history\n");
            for summary in &page.items {
                message_row(summary, spec, &mut out);
            }
            next("next", page, &mut out);
        }
        CommandResult::Inbox(page) => {
            out.push_str("inbox\n");
            inbox_rows(page, spec, &mut out);
            next("next", page, &mut out);
        }
        CommandResult::InboxBatch(page) => {
            if page.items.is_empty() && !page.has_more {
                out.push_str("empty\n");
            } else {
                for item in &page.items {
                    match item {
                        InboxBatchItem::Invitation {
                            thread,
                            topic_data,
                            invitation,
                            required_service,
                        } => {
                            out.push_str(&format!(
                                "invitation {} {}: {}\n",
                                invitation.as_str(),
                                thread.as_str(),
                                one_line(topic_data)
                            ));
                            if let Some(required) = required_service {
                                let revision = required.revision.to_string();
                                out.push_str("  accept-required: ");
                                out.push_str(&format_command_argv(&detail_argv(
                                    spec,
                                    &[
                                        "accept-required",
                                        thread.as_str(),
                                        "--invitation",
                                        invitation.as_str(),
                                        "--requirement",
                                        required.requirement.as_str(),
                                        "--revision",
                                        &revision,
                                    ],
                                )));
                            } else {
                                out.push_str("  accept: ");
                                out.push_str(&format_command_argv(&detail_argv(
                                    spec,
                                    &["accept", thread.as_str()],
                                )));
                            }
                            out.push('\n');
                        }
                        InboxBatchItem::Message {
                            thread,
                            topic_data,
                            message,
                            sequence,
                            sender,
                            author_role,
                            relays_user,
                            user_intent,
                            body,
                            body_start,
                            body_end,
                            body_len,
                            ..
                        } => {
                            out.push_str(&format!(
                                "message {} {}#{} from {}{} bytes {}..{}/{}: {}\n",
                                message.as_str(),
                                thread.as_str(),
                                sequence,
                                sender.as_ref().map_or("service", |s| s.as_str()),
                                crate::protocol::results::author_markers(
                                    MessageKind::Ordinary,
                                    *author_role,
                                    *relays_user,
                                    *user_intent,
                                ),
                                body_start,
                                body_end,
                                body_len,
                                one_line(topic_data)
                            ));
                            for line in body.split('\n') {
                                out.push_str("  ");
                                out.push_str(&one_line(line));
                                out.push('\n');
                            }
                            if body_end < body_len {
                                out.push_str("  read: ");
                                out.push_str(&format_command_argv(&detail_argv(
                                    spec,
                                    &["body", message.as_str()],
                                )));
                                out.push('\n');
                            }
                        }
                        InboxBatchItem::Warning {
                            thread,
                            topic_data,
                            warning,
                            sequence,
                        } => {
                            out.push_str(&format!(
                                "warning {} {}#{}: {}\n",
                                warning.as_str(),
                                thread.as_str(),
                                sequence,
                                one_line(topic_data)
                            ));
                            out.push_str("  read: ");
                            out.push_str(&format_command_argv(&detail_argv(
                                spec,
                                &["body", warning.as_str()],
                            )));
                            out.push('\n');
                        }
                    }
                }
                next("next", page, &mut out);
            }
        }
        CommandResult::ActiveWarnings(page) => {
            out.push_str("active_warnings\n");
            if page.items.is_empty() && !page.has_more {
                out.push_str("empty\n");
            }
            for warning in &page.items {
                warning_row(warning, &mut out);
                out.push_str("  body: ");
                out.push_str(&format_command_argv(&detail_argv(
                    spec,
                    &["body", warning.warning.as_str()],
                )));
                out.push('\n');
            }
            next("next", page, &mut out);
        }
        CommandResult::PendingReceipts(page) => {
            pending_receipts(page, &mut out);
            next("next", page, &mut out);
        }
        CommandResult::Participants(page) => {
            out.push_str("participants\n");
            for participant in &page.items {
                participant_row(participant, &mut out);
            }
            next("next", page, &mut out);
        }
        CommandResult::Thread(details) => thread(details, &mut out),
        CommandResult::CheckedIn(check) => checked_in(check, spec, &mut out),
        // Only the join hint needs a form; a bare accept keeps the generic one.
        CommandResult::Accepted(accepted) if accepted.summary_available.is_some() => {
            out.push_str("accepted\nvalue: ");
            out.push_str(&text_json(&Value::from(accepted.invitation.as_str())));
            out.push('\n');
            if let Some(thread) = &accepted.summary_available {
                out.push_str("summary available: ");
                out.push_str(&format_command_argv(&detail_argv(
                    spec,
                    &["summary", thread.as_str()],
                )));
                out.push('\n');
            }
        }
        CommandResult::Message(details) => message_body(details, &mut out),
        CommandResult::AlreadyJoined(joined) => out.push_str(&format!(
            "already_joined {} {}: no invitation sent\n",
            joined.thread.as_str(),
            joined.seat.as_str()
        )),
        _ => return None,
    }
    Some(out)
}

/// `label: COMMAND` once, only when the page has more.
fn next<T>(label: &str, page: &Page<T>, out: &mut String) {
    if page.has_more
        && let Some(argv) = &page.next_argv
    {
        out.push_str(label);
        out.push_str(": ");
        out.push_str(&format_command_argv(argv));
        out.push('\n');
    }
}

/// Peer text on one line: JSON-style escapes for controls, backslash, C1
/// controls and Unicode line separators; quotes stay as they are.
fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            ch if ch.is_control() || needs_terminal_escape(ch) => {
                push_u4(ch, &mut out);
            }
            ch => out.push(ch),
        }
    }
    out
}

/// A label as a single token: escaped, whitespace replaced.
fn token(text: &str) -> String {
    one_line(text)
        .chars()
        .map(|ch| if ch.is_whitespace() { '_' } else { ch })
        .collect()
}

/// `HH:MMZ` (UTC) of a timestamp on the render-time date, `MM-DD HH:MMZ` on
/// any other date (earlier or later: a bare time there is ambiguous).
fn clock(at: UtcMillis) -> String {
    let minutes = at.0.div_euclid(60_000).rem_euclid(24 * 60);
    let time = format!("{:02}:{:02}Z", minutes / 60, minutes % 60);
    match other_day(at) {
        Some((month, day)) => format!("{month:02}-{day:02} {time}"),
        None => time,
    }
}

/// The author column: a seat ID, a service ID or `system`, with any actor
/// label attached as `id(label)`.
fn author(summary: &MessageSummary) -> String {
    let id = match (&summary.event_author, &summary.author) {
        (Some(EventAuthor::Programmatic(service)), _) => Some(service.as_str().to_owned()),
        (Some(EventAuthor::Native(seat)), _) | (_, Some(seat)) => Some(seat.as_str().to_owned()),
        (Some(EventAuthor::BuiltIn), None) | (None, None) => None,
    };
    match (id, &summary.actor_label) {
        (Some(id), Some(label)) => format!("{id}({})", token(label)),
        (Some(id), None) => id,
        (None, Some(label)) => token(label),
        (None, None) => "system".to_owned(),
    }
}

/// `#SEQ MSG AUTHOR HH:MMZ: text`, with `[more: herdr-threads body MSG]`
/// before the text when the preview was clipped. System events are one short
/// `#SEQ EVENT IDS...` row.
fn message_row(summary: &MessageSummary, spec: &OutputSpec, out: &mut String) {
    if summary.kind != MessageKind::Ordinary
        && let Some(row) = event_row(summary)
    {
        out.push_str(&row);
        out.push('\n');
        return;
    }
    out.push_str(&format!(
        "#{} {} {} {}",
        summary.sequence,
        summary.message.as_str(),
        author(summary),
        clock(summary.created_at)
    ));
    if summary.kind == MessageKind::Warn {
        out.push_str(" warn");
    }
    out.push_str(&summary.author_markers());
    if summary.preview_omitted {
        let argv = summary
            .preview_detail_argv
            .clone()
            .unwrap_or_else(|| detail_argv(spec, &["body", summary.message.as_str()]));
        out.push_str(&format!(" [more: {}]", format_command_argv(&argv)));
    }
    out.push_str(": ");
    out.push_str(&one_line(&summary.preview_data));
    out.push('\n');
}

/// Whether `value` is a service-generated ID safe to print as a bare token.
fn id_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Most extra `key=value` fields one event row lists (`+N fields` counts the
/// rest).
const EVENT_FIELDS_SHOWN: usize = 4;

/// A scalar event value as one token: bare when it holds only characters
/// that cannot form a `": "` separator, else a quoted, escaped string whose
/// every `": "` is defused.
fn field_value(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    let bare = !text.is_empty()
        && text.len() <= 64
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'+' | b'/')
        });
    if bare {
        return Some(text);
    }
    let escaped = one_line(&text)
        .replace('"', "\\\"")
        .replace(": ", ":\\u0020");
    Some(format!("\"{escaped}\""))
}

/// `#SEQ NAME IDS...[ key=value...][: text]` for a structured system event,
/// or `None` when its preview is clipped or not a structured event (the full
/// row is used).
fn event_row(summary: &MessageSummary) -> Option<String> {
    if summary.preview_omitted {
        return None;
    }
    let event: Value = serde_json::from_str(&summary.preview_data).ok()?;
    let event = event.as_object()?;
    let field = |key: &str| event.get(key).and_then(Value::as_str);
    let (name_key, name) = field("action")
        .map(|name| ("action", name.to_owned()))
        .or_else(|| field("event").map(|name| ("event", name.to_owned())))
        .or_else(|| field("obligation").map(|what| ("obligation", format!("overdue_{what}"))))?;
    if !id_token(&name) {
        return None;
    }
    let mut row = format!("#{} {name}", summary.sequence);
    if summary.kind == MessageKind::Warn {
        row.push_str(" warn");
    }
    let mut ids: Vec<String> = Vec::new();
    fn push(ids: &mut Vec<String>, id: &str) {
        if id_token(id) && !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_owned());
        }
    }
    // Who did it, when the event does not name its actor itself.
    if !event.contains_key("actor_seat") {
        match (&summary.author, &summary.event_author) {
            (Some(seat), _) => push(&mut ids, seat.as_str()),
            (None, Some(EventAuthor::Programmatic(service))) => push(&mut ids, service.as_str()),
            _ => {}
        }
    }
    for key in ["actor_seat", "seat", "invitation", "message"] {
        if let Some(id) = field(key) {
            push(&mut ids, id);
        }
    }
    let (mut listed, mut extra) = (0usize, 0usize);
    for key in ["messages", "seats"] {
        if let Some(list) = event.get(key).and_then(Value::as_array) {
            for id in list.iter().filter_map(Value::as_str) {
                if listed < EVENT_IDS_SHOWN {
                    push(&mut ids, id);
                    listed += 1;
                } else {
                    extra += 1;
                }
            }
        }
    }
    for id in &ids {
        row.push(' ');
        row.push_str(id);
    }
    if extra > 0 {
        row.push_str(&format!(" +{extra}"));
    }
    // Every remaining scalar field (the name, the ids and the text fields are
    // already shown), sorted by key.
    let shown_elsewhere = [
        name_key,
        "actor_seat",
        "seat",
        "invitation",
        "message",
        "messages",
        "seats",
        "data",
        "topic",
        "goal",
        "detail",
    ];
    let mut rest: Vec<(&String, &Value)> = event
        .iter()
        .filter(|(key, _)| !shown_elsewhere.contains(&key.as_str()))
        .collect();
    rest.sort_by(|left, right| left.0.cmp(right.0));
    let mut listed_fields = 0usize;
    let mut dropped_fields = 0usize;
    for (key, value) in rest {
        match field_value(value) {
            Some(value) if id_token(key) && listed_fields < EVENT_FIELDS_SHOWN => {
                row.push_str(&format!(" {key}={value}"));
                listed_fields += 1;
            }
            _ => dropped_fields += 1,
        }
    }
    if dropped_fields > 0 {
        row.push_str(&format!(" +{dropped_fields} fields"));
    }
    for key in ["data", "topic", "goal", "detail"] {
        if let Some(text) = field(key) {
            row.push_str(": ");
            row.push_str(&one_line(text));
            break;
        }
    }
    Some(row)
}

/// A count with `+` when more may be pending than it says.
fn count(value: u64, more: bool) -> String {
    format!("{value}{}", if more { "+" } else { "" })
}

/// `THREAD receipts=N invitations=N warnings=N [required]`, zero counts left
/// out; a required invitation adds its exact accept command.
fn inbox_rows(page: &Page<InboxItem>, spec: &OutputSpec, out: &mut String) {
    if page.items.is_empty() && !page.has_more {
        out.push_str("empty\n");
    }
    for item in &page.items {
        out.push_str(item.thread.as_str());
        for (label, value, more) in [
            (
                "receipts",
                item.pending_receipts,
                item.pending_receipts_has_more,
            ),
            ("invitations", item.invitations, item.invitations_has_more),
            ("warnings", item.warnings, item.warnings_has_more),
        ] {
            if value > 0 || more {
                out.push_str(&format!(" {label}={}", count(value, more)));
            }
        }
        if let Some(required) = &item.pending_requirement {
            out.push_str(" required\n  accept-required: ");
            let revision = required.revision.to_string();
            out.push_str(&format_command_argv(&detail_argv(
                spec,
                &[
                    "accept-required",
                    item.thread.as_str(),
                    "--invitation",
                    required.invitation.as_str(),
                    "--requirement",
                    required.requirement.as_str(),
                    "--revision",
                    &revision,
                ],
            )));
        }
        out.push('\n');
    }
}

/// The sender column: the native seat, else the programmatic service author,
/// else `system`.
fn receipt_sender(receipt: &PendingReceipt) -> &str {
    match (&receipt.sender, &receipt.sender_author) {
        (Some(seat), _) => seat.as_str(),
        (None, Some(EventAuthor::Programmatic(service))) => service.as_str(),
        (None, _) => "system",
    }
}

/// `pending_receipts [SEAT]`, then `MSG THREAD#SEQ from SENDER [due HH:MMZ]
/// [overdue] [deferred: recipient catching up (until HH:MMZ)]`; the seat is on the header when every row shares it.
fn pending_receipts(page: &Page<PendingReceipt>, out: &mut String) {
    let shared = page
        .items
        .first()
        .map(|first| &first.seat)
        .filter(|seat| page.items.iter().all(|item| &item.seat == *seat));
    out.push_str("pending_receipts");
    if let Some(seat) = shared {
        out.push(' ');
        out.push_str(seat.as_str());
    }
    out.push('\n');
    if page.items.is_empty() && !page.has_more {
        out.push_str("none\n");
    }
    for item in &page.items {
        out.push_str(&format!(
            "{} {}#{} from {}",
            item.message.as_str(),
            item.thread.as_str(),
            item.sequence,
            receipt_sender(item)
        ));
        if shared.is_none() {
            out.push_str(&format!(" to {}", item.seat.as_str()));
        }
        if let Some(deadline) = item.deadline {
            out.push_str(&format!(" due {}", clock(deadline)));
        }
        if item.overdue {
            out.push_str(" overdue");
        }
        if let Some(until) = item.deferred_until {
            out.push_str(&format!(
                " deferred: recipient catching up (until {})",
                clock(until)
            ));
        }
        out.push('\n');
    }
}

fn snake<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// `SEAT STATE [self] [required invitation=I requirement=R revision=N]
/// [retired]`.
fn participant_row(participant: &Participant, out: &mut String) {
    out.push_str(participant.seat.as_str());
    out.push(' ');
    out.push_str(&snake(&participant.effective_state));
    if participant.physical_state != participant.effective_state {
        out.push_str(&format!(
            "(physical:{})",
            snake(&participant.physical_state)
        ));
    }
    if participant.is_self {
        out.push_str(" self");
    }
    if let Some(required) = &participant.requirement {
        out.push_str(&format!(
            " required invitation={} requirement={} revision={}",
            required.invitation.as_str(),
            required.requirement.as_str(),
            required.revision
        ));
        let state = snake(&required.state);
        if state != "pending" {
            out.push_str(&format!(" requirement_state={state}"));
        }
    }
    if participant.retired {
        out.push_str(" retired");
    }
    if let Some(cleanup) = &participant.cleanup_state {
        out.push_str(&format!(" cleanup:{}", snake(cleanup)));
    }
    out.push('\n');
}

fn thread(details: &ThreadDetails, out: &mut String) {
    let summary = &details.summary;
    out.push_str(&format!(
        "thread {} messages={} ordinary={} joined={} participants={} pending_receipts={}",
        summary.thread.as_str(),
        summary.message_count,
        summary.ordinary_count,
        summary.joined_count,
        details.participant_count,
        details.pending_receipt_count
    ));
    if summary.archived {
        out.push_str(" archived");
    }
    if summary.orphaned {
        out.push_str(" orphaned");
    }
    if let Some(owner) = &summary.managed_owner {
        out.push_str(&format!(" managed_by={}", owner.as_str()));
    }
    out.push('\n');
    if let Some(name) = &summary.name {
        out.push_str("name: ");
        out.push_str(&one_line(name));
        out.push('\n');
    }
    out.push_str("topic: ");
    out.push_str(&one_line(&summary.topic_data));
    if summary.topic_omitted {
        out.push('…');
    }
    out.push('\n');
    if summary.topic_omitted
        && let Some(argv) = &summary.topic_detail_argv
    {
        out.push_str("topic.more: ");
        out.push_str(&format_command_argv(argv));
        out.push('\n');
    }
    if details.goal_data != summary.topic_data {
        out.push_str("goal: ");
        out.push_str(&one_line(&details.goal_data));
        out.push('\n');
    }
    if details.pending_receipt_count > 0 {
        out.push_str("pending_receipts: ");
        out.push_str(&format_command_argv(&details.pending_receipts_argv));
        out.push('\n');
    }
    out.push_str("participants:\n");
    for participant in &details.participants.items {
        participant_row(participant, out);
    }
    next("participants.next", &details.participants, out);
}

fn warning_row(warning: &WarningRef, out: &mut String) {
    out.push_str(&format!(
        "{} {}#{}\n",
        warning.warning.as_str(),
        warning.thread.as_str(),
        warning.sequence
    ));
}

/// `checked_in SEAT HARNESS ROLE generation=N DISPOSITION`, the inbox rows,
/// the pending warning count, the first warning-history page (when there is
/// any) and any notices this offer carries.
fn checked_in(check: &CheckInResult, spec: &OutputSpec, out: &mut String) {
    let context = &check.context;
    out.push_str(&format!(
        "checked_in {} {} {} generation={} {}",
        check.seat.as_str(),
        snake(&context.harness),
        snake(&context.role),
        context.binding_generation,
        snake(&check.context_disposition)
    ));
    if let Some(through) = &check.offered_through {
        out.push_str(&format!(" offered_through={}", token(through)));
    }
    out.push('\n');
    out.push_str("inbox:\n");
    inbox_rows(&check.inbox, spec, out);
    next("inbox.next", &check.inbox, out);
    out.push_str(&format!(
        "warnings: {} pending\n",
        count(check.warning_count, check.warning_count_has_more)
    ));
    if !check.warnings.items.is_empty() || check.warnings.has_more {
        out.push_str("warning_history:\n");
        for warning in &check.warnings.items {
            warning_row(warning, out);
        }
        next("warning_history.next", &check.warnings, out);
    }
    if !check.notices.items.is_empty() || check.notices.has_more {
        out.push_str("notices:\n");
        for notice in &check.notices.items {
            warning_row(notice, out);
        }
        if check.notices.has_more {
            out.push_str("notices.more: ");
            out.push_str(&format_command_argv(&detail_argv(
                spec,
                &["warnings", "--seat", check.seat.as_str()],
            )));
            out.push('\n');
        }
    }
}

/// Whether a body character must be escaped to keep the text terminal-safe.
/// Newlines and tabs stay as they are.
fn body_escape(ch: char) -> bool {
    matches!(
        escape_for_terminal(ch.encode_utf8(&mut [0; 4]), Context::MultiLine),
        Cow::Owned(_)
    )
}

/// `body`: the header `#SEQ MSG AUTHOR HH:MMZ [warn] [bytes=FROM-TO/TOTAL]
/// [escaped]`, then the body verbatim with each non-empty line indented by
/// two spaces, then `more: COMMAND` only when the body continues.
///
/// A body is printed exactly unless it holds a character that could drive
/// the terminal (C0 other than newline and tab, C1, U+2028/U+2029). Such a
/// body is marked `escaped` on the header and printed with those characters
/// as `\uXXXX` and each backslash doubled, so the escapes stay unambiguous.
/// A system event prints `event: JSON` (one line) and its condition.
fn message_body(details: &MessageDetails, out: &mut String) {
    let summary = &details.summary;
    out.push_str(&format!(
        "#{} {} {} {}",
        summary.sequence,
        summary.message.as_str(),
        author(summary),
        clock(summary.created_at)
    ));
    if summary.kind == MessageKind::Warn {
        out.push_str(" warn");
    }
    match &details.content {
        MessageContent::Ordinary {
            body_data,
            body_offset,
            body_total_bytes,
            body_complete,
            body_next_argv,
            ..
        } => {
            out.push_str(&summary.author_markers());
            let end = body_offset + body_data.len() as u64;
            if *body_offset > 0 || !body_complete {
                out.push_str(&format!(" bytes={body_offset}-{end}/{body_total_bytes}"));
            }
            let escaped = body_data.chars().any(body_escape);
            if escaped {
                out.push_str(" escaped");
            }
            out.push('\n');
            let text = if escaped {
                let mut text = String::with_capacity(body_data.len());
                for ch in body_data.chars() {
                    match ch {
                        '\\' => text.push_str("\\\\"),
                        ch if body_escape(ch) => push_u4(ch, &mut text),
                        ch => text.push(ch),
                    }
                }
                text
            } else {
                body_data.clone()
            };
            for line in text.split('\n') {
                if !line.is_empty() {
                    out.push_str("  ");
                    out.push_str(line);
                }
                out.push('\n');
            }
            // The daemon always names the continuation of an incomplete body
            // (`body_next_argv`); a client never invents one.
            if !body_complete && let Some(argv) = body_next_argv {
                out.push_str("more: ");
                out.push_str(&format_command_argv(argv));
                out.push('\n');
            }
        }
        MessageContent::System {
            event,
            current_condition,
        } => {
            out.push('\n');
            out.push_str("event: ");
            out.push_str(&text_json(&event.event_json));
            out.push('\n');
            if let Some(source) = &event.source_message {
                out.push_str(&format!("source_message: {}\n", source.as_str()));
            }
            if let Some(source) = &event.source_invitation {
                out.push_str(&format!("source_invitation: {}\n", source.as_str()));
            }
            if let Some(condition) = current_condition {
                out.push_str(&format!(
                    "condition: {} {}\n",
                    token(&condition.state),
                    if condition.active {
                        "active"
                    } else {
                        "inactive"
                    }
                ));
            }
        }
    }
}

#[cfg(test)]
mod inbox_batch_tests {
    use super::*;
    use crate::protocol::{
        ids::{MessageId, ThreadId},
        pagination::{Consistency, StopReason},
    };

    #[test]
    fn body_fallback_names_message_id_for_partial_content_and_warning() {
        let page = Page {
            items: vec![
                InboxBatchItem::Message {
                    thread: ThreadId::new("full-thread-id"),
                    topic_data: "Topic".into(),
                    message: MessageId::new("full-message-id"),
                    sequence: 1,
                    sender: None,
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "part".into(),
                    body_start: 0,
                    body_end: 4,
                    body_len: 8,
                    ack_candidate: None,
                },
                InboxBatchItem::Warning {
                    thread: ThreadId::new("full-thread-id"),
                    topic_data: "Topic".into(),
                    warning: MessageId::new("full-warning-id"),
                    sequence: 2,
                },
            ],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 2,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        };
        let text = render(&CommandResult::InboxBatch(page), &OutputSpec::default()).unwrap();
        assert!(text.contains("body full-message-id"), "{text}");
        assert!(text.contains("body full-warning-id"), "{text}");
        assert!(!text.contains("read full-message-id"), "{text}");
        assert!(!text.contains("read full-warning-id"), "{text}");
    }

    #[test]
    fn active_warning_rows_keep_full_ids_and_copyable_detail_command() {
        let page = Page {
            items: vec![WarningRef {
                warning: MessageId::new("full-warning-id"),
                thread: ThreadId::new("full-thread-id"),
                sequence: 7,
                event_seq: 42,
            }],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 42,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        };
        let text = render(&CommandResult::ActiveWarnings(page), &OutputSpec::default()).unwrap();
        assert!(text.contains("full-warning-id full-thread-id#7"), "{text}");
        assert!(text.contains("body full-warning-id"), "{text}");
    }
}
