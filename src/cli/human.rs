//! Human-readable terminal presentation of selected command results.
//!
//! This is a presentation layer only. Daemon page fitting, byte budgets, and
//! every machine consumer (hooks, ready commands, models running the CLI
//! through a tool, `--json`) keep using the selected machine encoder. The CLI
//! chooses this renderer only when stdout is a terminal (or `--human` is
//! given) and the machine `--json` form was not requested; `--machine` forces
//! the established `key: value` text form on a terminal.
//!
//! Peer-supplied text (topics, goals, previews, bodies) is untrusted: every
//! control character, C1 control, line separator and bidi override is shown
//! as a visible escape so it cannot drive the terminal.

use crate::host::observation::SeatHostLabels;
use crate::protocol::{
    ids::{HostTargetId, ThreadId},
    output::{OutputSpec, detail_argv, format_command_argv, selected_result},
    pagination::Page,
    results::{
        AckResult, CheckInContextDisposition, CheckInResult, CommandResult, InboxItem,
        MembershipStatus, MessageContent, MessageDetails, MessageKind, MessageSummary, Participant,
        PendingReceipt, SearchHit, SeatSummary, ThreadDetails, ThreadSummary,
    },
    service::EventAuthor,
    time::UtcMillis,
};
use crate::view::escape::{Context, display_width, escape_for_terminal, pad_to_width};
use std::{cell::RefCell, collections::HashMap};

const TOPIC_COLUMN: usize = 48;

/// Thread topics known to the inbox renderer: `thread -> (topic, clipped)`.
type InboxTopics = HashMap<ThreadId, (String, bool)>;

thread_local! {
    static INBOX_TOPICS: RefCell<Option<InboxTopics>> = const { RefCell::new(None) };
    static SEAT_LABELS: RefCell<Option<HashMap<HostTargetId, SeatHostLabels>>> = const { RefCell::new(None) };
}

/// Make one advisory host snapshot available while rendering a seat list.
pub fn with_seat_labels<T>(labels: Vec<SeatHostLabels>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<HashMap<HostTargetId, SeatHostLabels>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SEAT_LABELS.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let map = labels
        .into_iter()
        .map(|label| (label.target.clone(), label))
        .collect();
    let _restore = Restore(SEAT_LABELS.with(|cell| cell.borrow_mut().replace(map)));
    f()
}

/// Run `f` with `topics` available to the human inbox renderer, which then
/// adds a TOPIC column (a thread not in the map shows `-`). The wire inbox
/// carries no topic, so the CLI reads them separately for a person only.
pub fn with_inbox_topics<T>(topics: InboxTopics, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<InboxTopics>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INBOX_TOPICS.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(INBOX_TOPICS.with(|cell| cell.borrow_mut().replace(topics)));
    f()
}

/// Render a result for a person at a terminal. Returns `None` for result
/// kinds without a dedicated human form; callers then emit the machine text.
pub fn render(result: &CommandResult, spec: &OutputSpec) -> Option<String> {
    let selected = selected_result(result, spec);
    let mut out = String::new();
    match &selected {
        CommandResult::Inbox(page) => inbox(page, spec, &mut out),
        CommandResult::Directory(page) => directory(page, &mut out),
        CommandResult::Seats(page) => seats(page, &mut out),
        CommandResult::History(page) => transcript(page, &mut out),
        CommandResult::Thread(details) => thread(details, &mut out),
        CommandResult::Participants(page) => participants(page, &mut out),
        CommandResult::Message(details) => message(details, &mut out),
        CommandResult::PendingReceipts(page) => pending(page, &mut out),
        CommandResult::Search(search) => {
            if search.matches.items.is_empty() {
                out.push_str("No matches.\n");
            }
            for hit in &search.matches.items {
                match hit {
                    SearchHit::Topic(summary) => out.push_str(&format!(
                        "topic  {}  {}\n",
                        summary.thread.as_str(),
                        one_line(&summary.topic_data, summary.topic_omitted, usize::MAX)
                    )),
                    SearchHit::Body(summary) => {
                        out.push_str(&format!("body   {}  ", summary.thread.as_str()));
                        chat_line(summary, &mut out);
                    }
                }
            }
            more(&search.matches, &mut out);
        }
        CommandResult::CheckedIn(check) => checked_in(check, spec, &mut out),
        CommandResult::SeatResolved(seat) => {
            out.push_str(&format!("You are seat {}.\n", seat.as_str()))
        }
        CommandResult::ThreadCreated(thread) => {
            out.push_str(&format!("Created thread {}.\n", thread.as_str()))
        }
        CommandResult::Invitation(invitation) => {
            out.push_str(&format!("Invited (invitation {}).\n", invitation.as_str()))
        }
        CommandResult::AlreadyJoined(joined) => out.push_str(&format!(
            "Seat {} has already joined thread {}; no invitation sent.\n",
            joined.seat.as_str(),
            joined.thread.as_str()
        )),
        CommandResult::Accepted(accepted) => {
            out.push_str(&format!(
                "Accepted invitation {}.\n",
                accepted.invitation.as_str()
            ));
            if let Some(thread) = &accepted.summary_available {
                out.push_str(&format!(
                    "summary available: herdr-threads summary {}\n",
                    thread.as_str()
                ));
            }
        }
        CommandResult::RequiredAccepted(required) => out.push_str(&format!(
            "Accepted required membership in thread {} (invitation {}).\n",
            required.thread.as_str(),
            required.invitation.as_str()
        )),
        CommandResult::MessageSent(message) => {
            out.push_str(&format!("Sent message {}.\n", message.as_str()))
        }
        CommandResult::Acknowledged(ack) => acknowledged(ack, &mut out),
        CommandResult::Left(thread) => out.push_str(&format!("Left thread {}.\n", thread.as_str())),
        CommandResult::TopicChanged(thread) => {
            out.push_str(&format!("Changed topic of thread {}.\n", thread.as_str()))
        }
        CommandResult::Archived(thread) => {
            out.push_str(&format!("Archived thread {}.\n", thread.as_str()))
        }
        CommandResult::Reopened(thread) => {
            out.push_str(&format!("Reopened thread {}.\n", thread.as_str()))
        }
        CommandResult::ContinuityReattached(reattached) => out.push_str(&format!(
            "Reattached seat {} (binding generation {}).\n",
            reattached.seat.as_str(),
            reattached.binding_generation
        )),
        CommandResult::OperatorRebound(seat) => {
            out.push_str(&format!("Rebound seat {}.\n", seat.as_str()))
        }
        CommandResult::OperatorRetired(seat) => out.push_str(&format!(
            "Seat {} retired; its pending obligations settle as recipient-retired.\n",
            seat.as_str()
        )),
        CommandResult::OperatorFreshSeat(seat) => {
            out.push_str(&format!("Created fresh seat {}.\n", seat.as_str()))
        }
        CommandResult::OperatorInvited(invitation) => out.push_str(&format!(
            "Invited (operator invitation {}).\n",
            invitation.as_str()
        )),
        _ => return None,
    }
    Some(out)
}

fn seats(page: &Page<SeatSummary>, out: &mut String) {
    if page.items.is_empty() {
        out.push_str("No seats in this page.\n");
        more(page, out);
        return;
    }
    out.push_str(
        "Current host labels are advisory; unavailable labels do not change seat identity.\n",
    );
    let rows = SEAT_LABELS.with(|cell| {
        let labels = cell.borrow();
        page.items
            .iter()
            .map(|seat| {
                let target = seat.target.as_ref();
                let current =
                    if seat.continuity == crate::protocol::results::ContinuityStatus::Resolved {
                        target.and_then(|target| labels.as_ref()?.get(target))
                    } else {
                        None
                    };
                let labeled = |id: &str, name: Option<&str>| {
                    format!(
                        "{} ({})",
                        escape_for_terminal(id, Context::SingleLine),
                        name.map_or_else(|| "unnamed".to_owned(), |name| one_line(name, false, 40))
                    )
                };
                let state = match seat.continuity {
                    crate::protocol::results::ContinuityStatus::Resolved => "resolved",
                    crate::protocol::results::ContinuityStatus::Unresolved => "unresolved",
                    crate::protocol::results::ContinuityStatus::Retired => "retired",
                };
                vec![
                    seat.seat.as_str().to_owned(),
                    state.to_owned(),
                    timestamp(seat.created_at),
                    current.map_or_else(
                        || "-".to_owned(),
                        |name| labeled(&name.workspace_id, name.workspace_label.as_deref()),
                    ),
                    current.map_or_else(
                        || "-".to_owned(),
                        |name| labeled(&name.tab_id, name.tab_label.as_deref()),
                    ),
                    match (target, current) {
                        (Some(_), Some(name)) => {
                            labeled(name.target.as_str(), name.pane_label.as_deref())
                        }
                        (Some(target), None) => format!("{} (unavailable)", target.as_str()),
                        (None, None) => "-".to_owned(),
                        (None, Some(_)) => unreachable!(),
                    },
                ]
            })
            .collect::<Vec<_>>()
    });
    table(
        &["SEAT", "STATE", "CREATED", "SPACE", "TAB", "PANE"],
        &rows,
        out,
    );
    more(page, out);
}

#[cfg(test)]
mod seat_list_tests {
    use super::*;
    use crate::protocol::{
        ids::SeatId,
        pagination::{Consistency, StopReason},
        results::ContinuityStatus,
    };

    #[test]
    fn labels_only_describe_resolved_targets_present_in_current_snapshot() {
        let mut rows: Vec<_> = [
            ContinuityStatus::Resolved,
            ContinuityStatus::Unresolved,
            ContinuityStatus::Retired,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, continuity)| SeatSummary {
            seat: SeatId::new(format!("s{i}")),
            continuity,
            target: Some(HostTargetId::new("w4:p1")),
            generation: 1,
            created_at: UtcMillis(0),
            retired_at: (i == 2).then_some(UtcMillis(1)),
        })
        .collect();
        rows.push(SeatSummary {
            seat: SeatId::new("s3"),
            continuity: ContinuityStatus::Resolved,
            target: Some(HostTargetId::new("w4:p2")),
            generation: 1,
            created_at: UtcMillis(0),
            retired_at: None,
        });
        let page = Page {
            items: rows,
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 4,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        };
        let labels = vec![SeatHostLabels {
            target: HostTargetId::new("w4:p1"),
            workspace_id: "w4".into(),
            workspace_label: Some("Space".into()),
            tab_id: "w4:t1".into(),
            tab_label: Some("Tab".into()),
            pane_label: Some("Pane\nForged".into()),
        }];
        let text = with_seat_labels(labels, || {
            let mut out = String::new();
            seats(&page, &mut out);
            out
        });
        assert!(text.contains("w4 (Space)"));
        assert!(text.contains("w4:t1 (Tab)"));
        assert!(text.contains("w4:p1 (Pane\\nForged)"));
        assert_eq!(text.matches("w4 (Space)").count(), 1);
        assert_eq!(text.matches("w4:p1 (unavailable)").count(), 2);
        assert!(text.contains("w4:p2 (unavailable)"));
    }
}

fn inbox(page: &Page<InboxItem>, spec: &OutputSpec, out: &mut String) {
    if page.items.is_empty() {
        out.push_str("Inbox is empty.\n");
        more(page, out);
        return;
    }
    INBOX_TOPICS.with(|cell| {
        let topics = cell.borrow();
        let rows: Vec<Vec<String>> = page
            .items
            .iter()
            .map(|item| {
                let mut row = vec![item.thread.as_str().to_owned()];
                if let Some(topics) = topics.as_ref() {
                    row.push(topics.get(&item.thread).map_or_else(
                        || "-".to_owned(),
                        |(topic, omitted)| one_line(topic, *omitted, TOPIC_COLUMN),
                    ));
                }
                row.extend([
                    count(item.pending_receipts, item.pending_receipts_has_more),
                    count(item.invitations, item.invitations_has_more),
                    count(item.warnings, item.warnings_has_more),
                    item.pending_requirement.as_ref().map_or_else(
                        || "-".to_owned(),
                        |required| {
                            format!(
                                "required inv={} rev={}",
                                required.invitation.as_str(),
                                required.revision
                            )
                        },
                    ),
                ]);
                row
            })
            .collect();
        let mut headers = vec!["THREAD"];
        if topics.is_some() {
            headers.push("TOPIC");
        }
        headers.extend(["PENDING", "INVITATIONS", "WARNINGS", "MEMBERSHIP"]);
        table(&headers, &rows, out);
    });
    for item in &page.items {
        if let Some(required) = &item.pending_requirement {
            let revision = required.revision.to_string();
            out.push_str(&format!(
                "accept-required: {}\n",
                format_command_argv(&detail_argv(
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
                ))
            ));
        }
    }
    more(page, out);
}

fn directory(page: &Page<ThreadSummary>, out: &mut String) {
    if page.items.is_empty() {
        out.push_str("No threads.\n");
        more(page, out);
        return;
    }
    let rows: Vec<Vec<String>> = page
        .items
        .iter()
        .map(|summary| {
            vec![
                summary.thread.as_str().to_owned(),
                thread_state(summary).to_owned(),
                summary.message_count.to_string(),
                summary.joined_count.to_string(),
                timestamp(summary.created_at),
                one_line(&summary.topic_data, summary.topic_omitted, TOPIC_COLUMN),
            ]
        })
        .collect();
    table(
        &["THREAD", "STATE", "MESSAGES", "JOINED", "CREATED", "TOPIC"],
        &rows,
        out,
    );
    more(page, out);
}

fn transcript(page: &Page<MessageSummary>, out: &mut String) {
    out.push_str(&super::irc::render_page(
        page,
        &mut super::irc::NoLookup,
        &super::irc::Style::plain(),
    ));
}

fn chat_line(summary: &MessageSummary, out: &mut String) {
    let marker = match summary.kind {
        MessageKind::Ordinary => "",
        MessageKind::Info => "[info] ",
        MessageKind::Warn => "[warn] ",
    };
    out.push_str(&format!(
        "#{} {} {}{}: {}{}\n",
        summary.sequence,
        timestamp(summary.created_at),
        author(summary),
        summary.author_markers(),
        marker,
        one_line(&summary.preview_data, summary.preview_omitted, usize::MAX)
    ));
    if summary.preview_omitted
        && let Some(argv) = &summary.preview_detail_argv
    {
        out.push_str(&format!(
            "    full message ({}): {}\n",
            summary.message.as_str(),
            format_command_argv(argv)
        ));
    }
}

fn author(summary: &MessageSummary) -> String {
    let id = match (&summary.event_author, &summary.author) {
        (Some(EventAuthor::Programmatic(service)), _) => Some(service.as_str().to_owned()),
        (Some(EventAuthor::BuiltIn), None) => Some("system".to_owned()),
        (Some(EventAuthor::Native(seat)), _) | (_, Some(seat)) => Some(seat.as_str().to_owned()),
        (None, None) => None,
    };
    match (&summary.actor_label, id) {
        (Some(label), Some(id)) => format!("{} ({id})", one_line(label, false, 32)),
        (Some(label), None) => one_line(label, false, 32),
        (None, Some(id)) => id,
        (None, None) => "system".to_owned(),
    }
}

fn thread(details: &ThreadDetails, out: &mut String) {
    let summary = &details.summary;
    out.push_str(&format!(
        "Thread {}  [{}]\n",
        summary.thread.as_str(),
        thread_state(summary)
    ));
    out.push_str(&format!(
        "Topic:    {}\n",
        one_line(&summary.topic_data, summary.topic_omitted, usize::MAX)
    ));
    out.push_str(&format!(
        "Goal:     {}\n",
        one_line(&details.goal_data, false, usize::MAX)
    ));
    out.push_str(&format!("Created:  {}\n", timestamp(details.created_at)));
    out.push_str(&format!(
        "Messages: {} ({} ordinary, {} system)\n",
        summary.message_count, summary.ordinary_count, summary.system_count
    ));
    out.push_str(&format!("Participants ({}):\n", details.participant_count));
    for participant in &details.participants.items {
        participant_line(participant, out);
    }
    if details.participants.has_more
        && let Some(argv) = &details.participants.next_argv
    {
        out.push_str(&format!("  more: {}\n", format_command_argv(argv)));
    }
    out.push_str(&format!(
        "Pending receipts: {}",
        details.pending_receipt_count
    ));
    if details.pending_receipt_count > 0 {
        out.push_str(&format!(
            "  (list: {})",
            format_command_argv(&details.pending_receipts_argv)
        ));
    }
    out.push('\n');
}

fn participants(page: &Page<Participant>, out: &mut String) {
    if page.items.is_empty() {
        out.push_str("No participants.\n");
    }
    for participant in &page.items {
        participant_line(participant, out);
    }
    more(page, out);
}

fn participant_line(participant: &Participant, out: &mut String) {
    let state = match participant.effective_state {
        MembershipStatus::Invited => "invited",
        MembershipStatus::Joined => "joined",
        MembershipStatus::Left => "left",
        MembershipStatus::Retired => "retired",
    };
    out.push_str(&format!("  - {} {state}", participant.seat.as_str()));
    if let Some(at) = participant.joined_at
        && participant.effective_state == MembershipStatus::Joined
    {
        out.push_str(&format!(" since {}", timestamp(at)));
    }
    if participant.requirement.is_some() {
        out.push_str(" (required)");
    }
    if participant.is_self {
        out.push_str(" (you)");
    }
    out.push('\n');
}

fn message(details: &MessageDetails, out: &mut String) {
    let summary = &details.summary;
    out.push_str(&format!(
        "Message {} in thread {} (#{})\n",
        summary.message.as_str(),
        summary.thread.as_str(),
        summary.sequence
    ));
    out.push_str(&format!(
        "From {} at {}\n\n",
        author(summary),
        timestamp(summary.created_at)
    ));
    match &details.content {
        MessageContent::Ordinary {
            body_data,
            body_complete,
            body_next_argv,
            ..
        } => {
            out.push_str(&multi_line(body_data));
            if !body_data.ends_with('\n') {
                out.push('\n');
            }
            if !body_complete && let Some(argv) = body_next_argv {
                out.push_str(&format!("\n(continued: {})\n", format_command_argv(argv)));
            }
        }
        MessageContent::System { event, .. } => {
            let kind = match event.kind {
                MessageKind::Ordinary => "event",
                MessageKind::Info => "info",
                MessageKind::Warn => "warning",
            };
            out.push_str(&format!(
                "[{kind}] {}\n",
                one_line(&event.event_json.to_string(), false, usize::MAX)
            ));
        }
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

fn pending(page: &Page<PendingReceipt>, out: &mut String) {
    if page.items.is_empty() {
        out.push_str("No pending receipts.\n");
        more(page, out);
        return;
    }
    let rows: Vec<Vec<String>> = page
        .items
        .iter()
        .map(|receipt| {
            vec![
                receipt.message.as_str().to_owned(),
                receipt.thread.as_str().to_owned(),
                receipt_sender(receipt).to_owned(),
                timestamp(receipt.decision_at),
                receipt.deadline.map_or_else(|| "-".to_owned(), timestamp)
                    + if receipt.overdue { " (overdue)" } else { "" }
                    + &receipt.deferred_until.map_or_else(String::new, |until| {
                        format!(
                            " deferred: recipient catching up (until {})",
                            &timestamp(until)[11..]
                        )
                    }),
            ]
        })
        .collect();
    table(
        &["MESSAGE", "THREAD", "FROM", "SENT", "DEADLINE"],
        &rows,
        out,
    );
    more(page, out);
}

fn checked_in(check: &CheckInResult, spec: &OutputSpec, out: &mut String) {
    out.push_str(&format!("Checked in as seat {}.\n", check.seat.as_str()));
    let disposition = match check.context_disposition {
        CheckInContextDisposition::Current => "current",
        CheckInContextDisposition::Historical => "historical",
    };
    out.push_str(&format!("Context: {disposition}"));
    if let Some(through) = &check.offered_through {
        out.push_str(&format!(
            ", offered through {}",
            one_line(through, false, usize::MAX)
        ));
    }
    out.push('\n');
    if check.warning_count > 0 || check.warning_count_has_more {
        out.push_str(&format!(
            "Warnings: {}\n",
            count(check.warning_count, check.warning_count_has_more)
        ));
    }
    if !check.warnings.items.is_empty() || check.warnings.has_more {
        out.push_str("Warning history:\n");
        for warning in &check.warnings.items {
            out.push_str(&format!(
                "  {} {}#{}\n",
                warning.warning.as_str(),
                warning.thread.as_str(),
                warning.sequence
            ));
        }
        more(&check.warnings, out);
    }
    if let Some(notices) = check.notices.summary() {
        out.push_str(&notices);
        out.push('\n');
    }
    if check.notices.has_more {
        out.push_str(&format!(
            "notices.more: {}\n",
            format_command_argv(&detail_argv(
                spec,
                &["warnings", "--seat", check.seat.as_str()]
            ))
        ));
    }
    inbox(&check.inbox, spec, out);
}

fn acknowledged(ack: &AckResult, out: &mut String) {
    let ids = |list: &[crate::protocol::ids::MessageId]| {
        list.iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if ack.acknowledged.is_empty() && ack.already_acknowledged.is_empty() {
        out.push_str("Nothing to acknowledge.\n");
        return;
    }
    if !ack.acknowledged.is_empty() {
        out.push_str(&format!(
            "Acknowledged {} message(s): {}.\n",
            ack.acknowledged.len(),
            ids(&ack.acknowledged)
        ));
    }
    if !ack.already_acknowledged.is_empty() {
        out.push_str(&format!(
            "Already acknowledged: {}.\n",
            ids(&ack.already_acknowledged)
        ));
    }
}

fn more<T>(page: &Page<T>, out: &mut String) {
    if page.has_more
        && let Some(argv) = &page.next_argv
    {
        out.push_str(&format!("more: {}\n", format_command_argv(argv)));
    }
}

fn thread_state(summary: &ThreadSummary) -> &'static str {
    match (summary.archived, summary.orphaned) {
        (true, _) => "archived",
        (false, true) => "orphaned",
        (false, false) => "open",
    }
}

fn count(value: u64, has_more: bool) -> String {
    if has_more {
        format!("{value}+")
    } else {
        value.to_string()
    }
}

fn table(headers: &[&str], rows: &[Vec<String>], out: &mut String) {
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(display_width(cell));
        }
    }
    let line = |cells: Vec<&str>, out: &mut String| {
        let last = cells.len() - 1;
        for (index, cell) in cells.into_iter().enumerate() {
            if index < last {
                out.push_str(&pad_to_width(cell, widths[index] + 2));
            } else {
                out.push_str(cell);
            }
        }
        out.push('\n');
    };
    line(headers.to_vec(), out);
    for row in rows {
        line(row.iter().map(String::as_str).collect(), out);
    }
}

/// A single display line: controls escaped, clipped to `max` characters with
/// an ellipsis, and marked when the selected encoder already omitted text.
pub(crate) fn one_line(text: &str, omitted: bool, max: usize) -> String {
    let mut out = String::new();
    for (shown, ch) in text.chars().enumerate() {
        if shown >= max {
            out.push('…');
            return out;
        }
        let mut buffer = [0; 4];
        out.push_str(&escape_for_terminal(
            ch.encode_utf8(&mut buffer),
            Context::SingleLine,
        ));
    }
    if omitted {
        out.push('…');
    }
    out
}

/// Body text keeps its line breaks and tabs; every other control is escaped.
pub(crate) fn multi_line(text: &str) -> String {
    escape_for_terminal(text, Context::MultiLine).into_owned()
}

/// `YYYY-MM-DD HH:MMZ` (UTC) for a Unix-millisecond timestamp.
fn timestamp(at: UtcMillis) -> String {
    let secs = at.0.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60
    )
}

#[cfg(test)]
#[path = "../../tests/cli/human.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/cli/golden_contract.rs"]
mod golden_contract;
