use super::*;
use crate::cli::output::{Presentation, PresentationGuard, emitted_bytes, write_selected};
use crate::protocol::{
    ids::{InvitationId, MessageId, SeatId, ThreadId},
    output::{ContinuationContext, OutputFormat, encode_selected},
    pagination::{Consistency, StopReason},
    results::{AckResult, MembershipStatus},
};

fn page<T>(items: Vec<T>) -> Page<T> {
    Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 1,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

fn text() -> OutputSpec {
    OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext::default(),
    }
}

fn summary(seq: u64, preview: &str) -> MessageSummary {
    MessageSummary {
        message: MessageId::new(format!("msg-{seq}")),
        thread: ThreadId::new("thread-Ab12Cd34"),
        author: Some(SeatId::new("seat-Xy98Zw76")),
        event_author: None,
        kind: MessageKind::Ordinary,
        sequence: seq,
        // 2026-09-30 12:34:56Z
        created_at: UtcMillis(1_790_771_696_000),
        actor_label: None,
        preview_data: preview.into(),
        preview_omitted: false,
        preview_detail_argv: None,
    }
}

fn human(result: &CommandResult) -> String {
    // 2026-09-30 13:00:00Z: the day of the fixture timestamps.
    crate::protocol::output::with_render_now(UtcMillis(1_790_773_200_000), || {
        render(result, &text()).expect("human form")
    })
}

#[test]
fn inbox_is_a_table_with_counts() {
    let out = human(&CommandResult::Inbox(page(vec![InboxItem {
        thread: ThreadId::new("thread-Ab12Cd34"),
        invitations: 1,
        invitations_has_more: false,
        pending_receipts: 3,
        pending_receipts_has_more: true,
        warnings: 0,
        warnings_has_more: false,
        pending_requirement: None,
    }])));
    assert_eq!(
        out,
        "THREAD           PENDING  INVITATIONS  WARNINGS  MEMBERSHIP\n\
         thread-Ab12Cd34  3+       1            0         -\n"
    );
    assert_eq!(
        human(&CommandResult::Inbox(page(Vec::new()))),
        "Inbox is empty.\n"
    );
}

#[test]
fn history_is_an_irc_transcript_with_escaped_peer_text() {
    let mut labelled = summary(2, "line one\nline two \u{1b}[31mred\u{202e}");
    labelled.actor_label = Some("reviewer".into());
    let mut clipped = summary(3, "long");
    clipped.preview_omitted = true;
    let mut history = page(vec![summary(1, "hello"), labelled, clipped]);
    history.has_more = true;
    history.next_argv = Some(vec![
        "herdr-threads".into(),
        "read".into(),
        "--cursor".into(),
        "c1".into(),
    ]);
    let out = human(&CommandResult::History(history));
    assert_eq!(
        out,
        "[12:34] <seat-Xy98Zw76> hello\n\
         [12:34] <seat-Xy98Zw76> line one\n\
         \x20                       line two \\u{001b}[31mred\\u{202e}\n\
         [12:34] <seat-Xy98Zw76> long…\n\
         \x20                       … (full message: herdr-threads body msg-3)\n\
         more: herdr-threads read --cursor c1\n"
    );
    assert!(!out.contains('\u{1b}'));
}

#[test]
fn mutations_are_one_line_confirmations() {
    assert_eq!(
        human(&CommandResult::MessageSent(MessageId::new("msg-Q1w2E3r4"))),
        "Sent message msg-Q1w2E3r4.\n"
    );
    assert_eq!(
        human(&CommandResult::Accepted(InvitationId::new("inv-Q1w2E3r4"))),
        "Accepted invitation inv-Q1w2E3r4.\n"
    );
    assert_eq!(
        human(&CommandResult::Invitation(InvitationId::new(
            "inv-Q1w2E3r4"
        ))),
        "Invited (invitation inv-Q1w2E3r4).\n"
    );
    assert_eq!(
        human(&CommandResult::Acknowledged(AckResult {
            acknowledged: vec![MessageId::new("msg-1"), MessageId::new("msg-2")],
            already_acknowledged: vec![MessageId::new("msg-0")],
        })),
        "Acknowledged 2 message(s): msg-1, msg-2.\nAlready acknowledged: msg-0.\n"
    );
}

#[test]
fn participants_are_a_list_marking_self() {
    let participant = |seat: &str, state, is_self| Participant {
        seat: SeatId::new(seat),
        is_self,
        requirement: None,
        episode: 1,
        joined: state == MembershipStatus::Joined,
        retired: false,
        physical_state: state,
        effective_state: state,
        joined_at: None,
        left_at: None,
        retirement_cutover: None,
        cleanup_state: None,
        accepted_invitation: None,
    };
    let out = human(&CommandResult::Participants(page(vec![
        participant("seat-A", MembershipStatus::Joined, true),
        participant("seat-B", MembershipStatus::Invited, false),
    ])));
    assert_eq!(out, "  - seat-A joined (you)\n  - seat-B invited\n");
}

#[test]
fn unknown_kinds_fall_back_to_machine_text() {
    let result = CommandResult::Diagnostics(page(Vec::new()));
    assert!(render(&result, &text()).is_none());
}

#[test]
fn presentation_is_machine_unless_selected_and_never_for_json() {
    let result = CommandResult::MessageSent(MessageId::new("msg-1"));
    let machine = encode_selected(&result, &text()).unwrap();
    // Default (not a terminal): the established machine form.
    {
        let _guard = PresentationGuard::enter(Presentation::Auto, &text());
        assert_eq!(emitted_bytes(&result, &text()).unwrap(), machine);
    }
    {
        let _guard = PresentationGuard::enter(Presentation::Human, &text());
        assert_eq!(
            emitted_bytes(&result, &text()).unwrap(),
            b"Sent message msg-1.\n"
        );
        let mut out = Vec::new();
        write_selected(&result, &text(), 256, &mut out).unwrap();
        assert_eq!(out, b"Sent message msg-1.\n");
    }
    // The guard restores the machine form when the run ends.
    assert_eq!(emitted_bytes(&result, &text()).unwrap(), machine);
    let json = OutputSpec::default();
    let _guard = PresentationGuard::enter(Presentation::Human, &json);
    assert_eq!(
        emitted_bytes(&result, &json).unwrap(),
        encode_selected(&result, &json).unwrap()
    );
}

#[test]
fn terminal_auto_selects_human_and_machine_flag_overrides() {
    let result = CommandResult::Left(ThreadId::new("thread-1"));
    crate::cli::output::set_stdout_is_terminal(true);
    {
        let _guard = PresentationGuard::enter(Presentation::Auto, &text());
        assert_eq!(
            emitted_bytes(&result, &text()).unwrap(),
            b"Left thread thread-1.\n"
        );
    }
    {
        let _guard = PresentationGuard::enter(Presentation::Machine, &text());
        assert_eq!(
            emitted_bytes(&result, &text()).unwrap(),
            encode_selected(&result, &text()).unwrap()
        );
    }
    crate::cli::output::set_stdout_is_terminal(false);
}

#[test]
fn timestamps_are_utc_minutes() {
    assert_eq!(timestamp(UtcMillis(0)), "1970-01-01 00:00Z");
    assert_eq!(timestamp(UtcMillis(951_782_400_000)), "2000-02-29 00:00Z");
}

#[test]
fn presentation_flags_parse_and_conflict_with_json() {
    use crate::cli::commands::parse_argv;
    let parsed = |args: &[&str]| parse_argv(args.iter().copied());
    assert_eq!(
        parsed(&["herdr-threads", "inbox"]).unwrap().presentation,
        Presentation::Auto
    );
    assert_eq!(
        parsed(&["herdr-threads", "--human", "inbox"])
            .unwrap()
            .presentation,
        Presentation::Human
    );
    assert_eq!(
        parsed(&["herdr-threads", "inbox", "--machine"])
            .unwrap()
            .presentation,
        Presentation::Machine
    );
    assert!(parsed(&["herdr-threads", "--json", "--human", "inbox"]).is_err());
    assert!(parsed(&["herdr-threads", "--json", "--machine", "inbox"]).is_err());
    assert!(parsed(&["herdr-threads", "--human", "--machine", "inbox"]).is_err());
}

#[test]
fn table_columns_align_by_display_width() {
    let rows = vec![
        vec!["漢字漢字".to_owned(), "x".to_owned()],
        vec!["ab".to_owned(), "y".to_owned()],
    ];
    let mut out = String::new();
    table(&["NAME", "V"], &rows, &mut out);
    let lines: Vec<&str> = out.lines().collect();
    let second = |line: &str, mark: char| {
        crate::view::escape::display_width(&line[..line.find(mark).unwrap()])
    };
    assert_eq!(second(lines[1], 'x'), second(lines[2], 'y'), "{out}");
    assert_eq!(second(lines[1], 'x'), second(lines[0], 'V'), "{out}");
}
