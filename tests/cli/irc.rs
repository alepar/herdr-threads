use super::*;
use crate::protocol::{
    ids::{MessageId, ServiceAuthorId, ThreadId},
    pagination::{Consistency, StopReason},
};

/// 2026-09-30 13:00:00Z: the day of every fixture timestamp below, so times
/// render bare (older or later days carry a date; see the golden contract).
const NOW: UtcMillis = UtcMillis(1_790_773_200_000);

fn render_message(summary: &MessageSummary, lookup: &mut dyn Lookup, style: &Style) -> String {
    crate::protocol::output::with_render_now(NOW, || super::render_message(summary, lookup, style))
}

fn render_page(page: &Page<MessageSummary>, lookup: &mut dyn Lookup, style: &Style) -> String {
    crate::protocol::output::with_render_now(NOW, || super::render_page(page, lookup, style))
}

fn summary(seq: u64, author: &str, body: &str) -> MessageSummary {
    MessageSummary {
        message: MessageId::new(format!("msg-{seq}")),
        thread: ThreadId::new("thread-Ab12Cd34"),
        author: Some(SeatId::new(author)),
        event_author: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        kind: MessageKind::Ordinary,
        sequence: seq,
        // 2026-09-30 12:34:56Z
        created_at: UtcMillis(1_790_771_696_000),
        actor_label: None,
        preview_data: body.into(),
        preview_omitted: false,
        preview_detail_argv: Some(vec![
            "herdr-threads".into(),
            "body".into(),
            format!("msg-{seq}"),
        ]),
    }
}

fn event(seq: u64, kind: MessageKind, json: &str) -> MessageSummary {
    let mut summary = summary(seq, "seat-unused0", json);
    summary.author = None;
    summary.event_author = Some(EventAuthor::BuiltIn);
    summary.kind = kind;
    summary
}

/// Pane names and harnesses for a fixed set of seats.
struct Fixed(Vec<(&'static str, &'static str, Option<&'static str>)>);
impl Lookup for Fixed {
    fn nick(&mut self, seat: &SeatId) -> Nick {
        self.0
            .iter()
            .find(|(id, _, _)| *id == seat.as_str())
            .map_or_else(
                || Nick::seat(seat),
                |(_, name, harness)| Nick {
                    name: (*name).into(),
                    harness: harness.map(str::to_owned),
                    ids: None,
                },
            )
    }
}

fn party() -> Fixed {
    Fixed(vec![
        ("s001", "w/alice/s001", Some("claude")),
        ("s002", "w/mad-tea-hatter-codex/s002", Some("codex")),
        ("seat-Person01", "you", Some("human")),
    ])
}

fn render(summary: &MessageSummary, lookup: &mut dyn Lookup) -> String {
    render_message(summary, lookup, &Style::plain())
}

#[test]
fn agent_harness_is_never_appended_to_display_names() {
    let out = render(
        &summary(1, "s001", "Why is a raven like a writing-desk?"),
        &mut party(),
    );
    assert_eq!(
        out,
        "[12:34] <w/alice/s001> Why is a raven like a writing-desk?\n"
    );
    // A harness word already present in an advisory name is kept as name text.
    let out = render(
        &summary(2, "s002", "I haven't the slightest idea"),
        &mut party(),
    );
    assert_eq!(
        out,
        "[12:34] <w/mad-tea-hatter-codex/s002> I haven't the slightest idea\n"
    );
}

#[test]
fn agent_nick_is_workspace_tab_and_pane_label_escaped_and_bounded_per_component() {
    let seat = SeatId::new("seat-0b5a1c2e-1111-2222-3333-444455556666");
    let mut labels = crate::host::observation::SeatHostLabels {
        terminal: "terminal".into(),
        incarnation: None,
        target: crate::protocol::ids::HostTargetId::new("w1:p1"),
        workspace_id: "w1".into(),
        workspace_label: Some("a/b\x1b\n".into()),
        tab_id: "t1".into(),
        tab_label: Some("build".into()),
        pane_label: Some("alice\u{202e}".into()),
        session: None,
    };
    let nick = |labels: Option<&crate::host::observation::SeatHostLabels>| {
        Nick {
            name: agent_seat_nick(&seat, labels),
            harness: Some("codex".into()),
            ids: None,
        }
        .display()
    };
    // Slash text follows the existing label convention; controls are visible.
    assert_eq!(nick(Some(&labels)), "a/b\\u{001b}\\n/build/alice\\u{202e}");
    labels.workspace_label = Some("w".repeat(100));
    labels.tab_label = Some("t".repeat(100));
    labels.pane_label = Some("p".repeat(100));
    assert_eq!(
        nick(Some(&labels)),
        "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww…/tttttttttttttttttttttttttttttttt…/pppppppppppppppppppppppppppppppp…"
    );
    // No host read: only the canonical seat ID is honest.
    assert_eq!(nick(None), "seat-0b5a1c2e-1111-2222-3333-444455556666");
    // Unlabelled workspace and tab show their ids; an unlabelled pane is left out.
    labels.workspace_label = Some(String::new());
    labels.tab_label = None;
    labels.pane_label = None;
    assert_eq!(nick(Some(&labels)), "w1/t1");
    labels.pane_label = Some(String::new());
    assert_eq!(nick(Some(&labels)), "w1/t1");
}

#[test]
fn directory_agent_nick_drops_a_pane_that_is_only_its_id() {
    let target = crate::protocol::ids::HostTargetId::new("w1:p3");
    assert_eq!(
        agent_seat_nick_named("office", Some("build"), "reviewer", Some(&target)),
        "office/build/reviewer"
    );
    assert_eq!(
        agent_seat_nick_named("office", Some("build"), "w1:p3", Some(&target)),
        "office/build"
    );
    // A local seat's pane is an id only when it is its target: a pane
    // deliberately labelled like an id keeps its segment.
    for label in ["t2", "notes:t3", "w2:p9"] {
        assert_eq!(
            agent_seat_nick_named("office", Some("build"), label, Some(&target)),
            format!("office/build/{label}"),
        );
    }
    // A remote seat has no local target: an ID-shaped pane is still an id.
    for id in ["w1:p3", "core:t4", "t4"] {
        assert_eq!(
            agent_seat_nick_named("office", Some("w1:t2"), id, None),
            "office/w1:t2",
            "{id}"
        );
    }
    assert_eq!(
        agent_seat_nick_named("office", Some("build"), "reviewer", None),
        "office/build/reviewer"
    );
    // No tab level (`-`: an hc terminal without labels, a named seat): the
    // pane keeps its id, so two such seats never read the same.
    assert_eq!(
        agent_seat_nick_named("-", Some("-"), "core:t4", None),
        "-/-/core:t4"
    );
    assert_eq!(
        agent_seat_nick_named("office", Some("-"), "core:t5", None),
        "office/-/core:t5"
    );
    let named = crate::protocol::ids::HostTargetId::new("reviewer");
    assert_eq!(
        agent_seat_nick_named("-", Some("-"), "reviewer", Some(&named)),
        "-/-/reviewer"
    );
    // A peer that publishes no tab keeps the workspace and pane.
    assert_eq!(
        agent_seat_nick_named("office", None, "reviewer", Some(&target)),
        "office/reviewer"
    );
}

fn host_labels(
    target: &str,
    workspace: (&str, Option<&str>),
    tab: (&str, Option<&str>),
    pane: Option<&str>,
) -> crate::host::observation::SeatHostLabels {
    crate::host::observation::SeatHostLabels {
        terminal: target.into(),
        incarnation: None,
        target: crate::protocol::ids::HostTargetId::new(target),
        workspace_id: workspace.0.into(),
        workspace_label: workspace.1.map(str::to_owned),
        tab_id: tab.0.into(),
        tab_label: tab.1.map(str::to_owned),
        pane_label: pane.map(str::to_owned),
        session: None,
    }
}

/// hc fills `-` for a missing space or tab; an unnamed terminal must keep its
/// address there, not collapse every terminal to `-/-`.
#[test]
fn unnamed_hc_terminals_keep_their_address_and_differ() {
    let seat = SeatId::new("sQ7aAb12Cd34");
    let t4 = host_labels("core:t4", ("-", None), ("-", None), None);
    let t5 = host_labels("core:t5", ("-", None), ("-", None), None);
    assert_eq!(agent_seat_nick(&seat, Some(&t4)), "-/-/core:t4");
    assert_eq!(agent_seat_nick(&seat, Some(&t5)), "-/-/core:t5");
    let spaced = host_labels("core:t6", ("s1", Some("office")), ("-", None), None);
    assert_eq!(agent_seat_nick(&seat, Some(&spaced)), "office/-/core:t6");
    // A named hc terminal, and one in a labelled tab, follow the Herdr rule.
    let named = host_labels("core:t7", ("-", None), ("-", None), Some("builder"));
    assert_eq!(agent_seat_nick(&seat, Some(&named)), "-/-/builder");
    let tabbed = host_labels(
        "core:t8",
        ("s1", Some("office")),
        ("b1", Some("build")),
        None,
    );
    assert_eq!(agent_seat_nick(&seat, Some(&tabbed)), "office/build");
}

/// A named seat's labels (src/host/named.rs): synthetic `-` parents, its name
/// as both target and pane label.
#[test]
fn named_seats_render_dash_dash_name() {
    let seat = SeatId::new("sQ7aAb12Cd34");
    let labels = host_labels("reviewer", ("-", None), ("-", None), Some("reviewer"));
    assert_eq!(agent_seat_nick(&seat, Some(&labels)), "-/-/reviewer");
}

#[test]
fn show_ids_under_a_wide_nick_and_on_a_narrow_terminal() {
    let seat = "sQ7aAb12Cd34";
    let ids = Some(seat_ids_line(
        Some(&crate::protocol::ids::HostTargetId::new("w1:p3")),
        &SeatId::new(seat),
    ));
    // A nick wider than half the line: the body wraps at indent 8, so the ids
    // take a line of their own under the nick text.
    let mut wide = Fixed(vec![(
        seat,
        "a-long-workspace/a-long-tab/a-long-pane",
        Some("claude"),
    )]);
    let mut wide_ids = IdsLookup(&mut wide, ids.clone());
    let style = Style {
        width: 60,
        ..Style::plain()
    };
    assert_eq!(
        render_message(&summary(1, seat, "one\ntwo"), &mut wide_ids, &style),
        "[12:34] <a-long-workspace/a-long-tab/a-long-pane> one\n         w1:p3 sQ7aAb12Cd34\n        two\n"
    );
    // A terminal too narrow for the ids clips them with an ellipsis.
    let mut short = Fixed(vec![(seat, "w/t", Some("claude"))]);
    let mut short_ids = IdsLookup(&mut short, ids);
    let style = Style {
        width: 20,
        ..Style::plain()
    };
    let text = render_message(&summary(1, seat, "hi"), &mut short_ids, &style);
    assert_eq!(text, "[12:34] <w/t> hi\n         w1:p3 sQ7a…\n");
    assert!(
        text.lines().all(|line| line.chars().count() <= 20),
        "{text}"
    );
}

#[test]
fn show_ids_puts_pane_and_seat_ids_on_the_name_columns_second_line() {
    let seat = "sQ7aAb12Cd34";
    let mut lookup = Fixed(vec![(seat, "office/build/reviewer", Some("claude"))]);
    let ids = Some(seat_ids_line(
        Some(&crate::protocol::ids::HostTargetId::new("w1:p3")),
        &SeatId::new(seat),
    ));
    assert_eq!(ids.as_deref(), Some("w1:p3 sQ7aAb12Cd34"));
    let mut with_ids = IdsLookup(&mut lookup, ids.clone());
    // One-line body: the ids get their own line under the nick.
    assert_eq!(
        render(&summary(1, seat, "looks good"), &mut with_ids),
        "[12:34] <office/build/reviewer> looks good\n         w1:p3 sQ7aAb12Cd34\n"
    );
    // A longer body continues beside the ids, in the message column.
    assert_eq!(
        render(&summary(1, seat, "first\nsecond\nthird"), &mut with_ids),
        "[12:34] <office/build/reviewer> first\n         w1:p3 sQ7aAb12Cd34     second\n                                third\n"
    );
    // Ids wider than the nick take a line of their own.
    let mut narrow = Fixed(vec![(seat, "w/t", Some("claude"))]);
    let mut narrow_ids = IdsLookup(&mut narrow, ids);
    assert_eq!(
        render(&summary(1, seat, "a\nb"), &mut narrow_ids),
        "[12:34] <w/t> a\n         w1:p3 sQ7aAb12Cd34\n              b\n"
    );
    // Without a target only the seat id is shown; without the flag, nothing.
    assert_eq!(seat_ids_line(None, &SeatId::new(seat)), "sQ7aAb12Cd34");
    assert_eq!(
        render(&summary(1, seat, "looks good"), &mut lookup),
        "[12:34] <office/build/reviewer> looks good\n"
    );
}

struct IdsLookup<'a>(&'a mut dyn Lookup, Option<String>);

impl Lookup for IdsLookup<'_> {
    fn nick(&mut self, seat: &SeatId) -> Nick {
        Nick {
            ids: self.1.clone(),
            ..self.0.nick(seat)
        }
    }
    fn full(&mut self, summary: &MessageSummary) -> Option<Full> {
        self.0.full(summary)
    }
}

#[test]
fn unknown_seats_fall_back_to_the_short_seat_id() {
    let out = render(&summary(1, "seat-Zz99Yy88", "hi"), &mut NoLookup);
    assert_eq!(out, "[12:34] <seat-Zz99Yy88> hi\n");
    let compact = render(&summary(1, "sAb12Cd34", "hi"), &mut NoLookup);
    assert_eq!(compact, "[12:34] <sAb12Cd34> hi\n");
    assert_eq!(
        short_seat("seat-0b5a1c2e-1111-2222-3333-444455556666"),
        "seat-0b5a1c2e"
    );
    assert_eq!(short_seat("seat-Ab12Cd34"), "seat-Ab12Cd34");
    assert_eq!(short_seat("sAb12Cd34"), "sAb12Cd34");
    assert_eq!(short_seat("sQ7aAb12Cd34"), "sAb12Cd34");
    let tagged = render(&summary(1, "sQ7aAb12Cd34", "hi"), &mut NoLookup);
    assert_eq!(tagged, "[12:34] <sAb12Cd34> hi\n");
}

#[test]
fn programmatic_authors_use_their_label() {
    let mut message = summary(1, "seat-unused0", "graph says hi");
    message.author = None;
    message.event_author = Some(EventAuthor::Programmatic(ServiceAuthorId::new("svc-1")));
    message.actor_label = Some("herdr-graph".into());
    assert_eq!(
        render(&message, &mut NoLookup),
        "[12:34] <herdr-graph> graph says hi\n"
    );
}

#[test]
fn long_bodies_wrap_under_the_message_column_and_keep_line_breaks() {
    let style = Style {
        width: 50,
        ..Style::plain()
    };
    let body = "one two three four five six seven eight nine ten\nsecond paragraph";
    let out = render_message(&summary(1, "s001", body), &mut party(), &style);
    assert_eq!(
        out,
        "[12:34] <w/alice/s001> one two three four five six\n\
         \x20                      seven eight nine ten\n\
         \x20                      second paragraph\n"
    );
    for line in out.lines() {
        assert!(line.chars().count() <= 50, "{line:?}");
    }
    // On a narrow terminal a wide nick column falls back to a short indent.
    let narrow = Style {
        width: 40,
        ..Style::plain()
    };
    let out = render_message(&summary(1, "s001", body), &mut party(), &narrow);
    assert_eq!(
        out,
        "[12:34] <w/alice/s001> one two three\n\
         \x20       four five six seven eight nine\n\
         \x20       ten\n\
         \x20       second paragraph\n"
    );
    // A word longer than the column is split rather than overflowing.
    let out = render_message(&summary(2, "s001", &"x".repeat(80)), &mut party(), &narrow);
    assert!(out.lines().all(|line| line.chars().count() <= 40), "{out}");
    assert_eq!(out.matches('x').count(), 80);
}

#[test]
fn very_long_bodies_fold_with_a_hint() {
    let style = Style {
        max_lines: 3,
        ..Style::plain()
    };
    let body = (1..=10)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let out = render_message(&summary(7, "s001", &body), &mut party(), &style);
    assert_eq!(
        out,
        "[12:34] <w/alice/s001> line 1\n\
         \x20                      line 2\n\
         \x20                      … (8 more lines; full message: herdr-threads body msg-7)\n"
    );
}

#[test]
fn clipped_previews_use_the_full_body_from_the_lookup() {
    struct WithBody;
    impl Lookup for WithBody {
        fn full(&mut self, _: &MessageSummary) -> Option<Full> {
            Some(Full::Text {
                data: "the whole story".into(),
                complete: true,
                more_argv: None,
            })
        }
    }
    let mut clipped = summary(3, "seat-Zz99Yy88", "the who");
    clipped.preview_omitted = true;
    assert_eq!(
        render(&clipped, &mut WithBody),
        "[12:34] <seat-Zz99Yy88> the whole story\n"
    );
    // Offline, the clipped preview is marked and the hint names the command.
    assert_eq!(
        render(&clipped, &mut NoLookup),
        "[12:34] <seat-Zz99Yy88> the who…\n\
         \x20                       … (full message: herdr-threads body msg-3)\n"
    );
}

#[test]
fn system_events_are_notice_lines_with_nicks() {
    let mut lookup = party();
    let cases = [
        (
            r#"{"action":"accept","seat":"s001","invitation":"inv-1"}"#,
            "[12:34] -!- w/alice/s001 joined\n",
        ),
        (
            r#"{"action":"leave","seat":"s002"}"#,
            "[12:34] -!- w/mad-tea-hatter-codex/s002 left\n",
        ),
        (
            r#"{"action":"invite","actor_seat":"seat-Person01","seat":"s001","invitation":"inv-2","deadline_at":1}"#,
            "", // informational: hidden in the IRC view
        ),
        (
            r#"{"event":"ack","seat":"s002","messages":["msg-a","msg-b","msg-c","msg-d"],"decided_at":1}"#,
            "", // ACKs are hidden in the IRC view
        ),
        (r#"{"action":"archive","actor_seat":"seat-Person01"}"#, ""),
        (
            r#"{"action":"create_thread","goal":"tea","actor_seat":"seat-Person01"}"#,
            "",
        ),
        (
            r#"{"action":"set_topic","actor_seat":"s001","topic":"riddles"}"#,
            "",
        ),
    ];
    for (json, expected) in cases {
        assert_eq!(
            render(&event(1, MessageKind::Info, json), &mut lookup),
            expected,
            "{json}"
        );
    }
    let warning = event(
        2,
        MessageKind::Warn,
        r#"{"obligation":"receipt","seat":"s002","deadline_at":1}"#,
    );
    assert_eq!(
        render(&warning, &mut lookup),
        "[12:34] -!- warning: w/mad-tea-hatter-codex/s002 is overdue on an ACK\n"
    );
    // The recipient's wake disposition (ht-327) says whether it was ever prompted.
    let woken = event(
        2,
        MessageKind::Warn,
        r#"{"obligation":"receipt","seat":"s002","deadline_at":1,"wake":"unsafe","detail":"last wake unsafe"}"#,
    );
    assert_eq!(
        render(&woken, &mut lookup),
        "[12:34] -!- warning: w/mad-tea-hatter-codex/s002 is overdue on an ACK (last wake unsafe)\n"
    );
    // An unparseable (clipped) informational event is not a join or leave: hidden.
    let mut clipped = event(3, MessageKind::Info, "{\"action\":\"crea");
    clipped.preview_omitted = true;
    assert_eq!(render(&clipped, &mut NoLookup), "");
    // An unparseable warning is still shown, escaped.
    let mut clipped_warning = event(4, MessageKind::Warn, "{\"obligation\":\"rec");
    clipped_warning.preview_omitted = true;
    assert_eq!(
        render(&clipped_warning, &mut NoLookup),
        "[12:34] -!- warning: {\"obligation\":\"rec…\n"
    );
}

#[test]
fn register_events_name_their_actor_and_never_say_someone() {
    let mut lookup = party();
    let cases = [
        (
            r#"{"action":"archive","actor_seat":"s001"}"#,
            "w/alice/s001 archived the thread",
        ),
        (r#"{"action":"archive"}"#, "the thread was archived"),
        (
            r#"{"action":"reopen","actor_seat":"s001"}"#,
            "w/alice/s001 reopened the thread",
        ),
        (r#"{"action":"reopen"}"#, "the thread was reopened"),
        (
            r#"{"action":"set_topic","actor_seat":"s001","topic":"riddles"}"#,
            "w/alice/s001 changed the topic to: riddles",
        ),
        (
            r#"{"action":"set_topic","topic":"riddles"}"#,
            "the topic was changed to: riddles",
        ),
        (
            r#"{"action":"set_thread_name","actor_seat":"s001","name":"tea"}"#,
            "w/alice/s001 renamed the thread to: tea",
        ),
        (
            r#"{"action":"set_thread_name","actor_seat":"s001","name":null}"#,
            "w/alice/s001 cleared the thread name",
        ),
        (
            r#"{"action":"set_thread_name","name":"tea"}"#,
            "the thread was renamed to: tea",
        ),
        (
            r#"{"action":"set_thread_name","name":null}"#,
            "the thread name was cleared",
        ),
    ];
    for (json, expected) in cases {
        let text = system_text(&event(1, MessageKind::Info, json), &mut lookup);
        assert_eq!(text, expected, "{json}");
        assert!(!text.contains("someone"), "{json}");
    }
}

#[test]
fn peer_text_cannot_drive_the_terminal() {
    let hostile = Fixed(vec![(
        "seat-Evil0001",
        "evil\u{1b}]0;pwned\u{7}",
        Some("claude"),
    )]);
    let body = "hi \u{1b}[2J\u{1b}[31mred\u{9b}x\u{202e}rtl\rback";
    let out = render(&summary(1, "seat-Evil0001", body), &mut { hostile });
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(!out.contains('\u{9b}'), "{out:?}");
    assert!(!out.contains('\u{202e}'), "{out:?}");
    assert!(!out.contains('\r'), "{out:?}");
    assert!(!out.contains('\u{7}'), "{out:?}");
    assert!(out.contains("\\u{001b}[2J"), "{out:?}");
    // Event fields are peer text too.
    let topic = event(
        2,
        MessageKind::Info,
        r#"{"action":"set_topic","actor_seat":"seat-X","topic":"a\u001b[31mb"}"#,
    );
    let out = render(&topic, &mut NoLookup);
    assert!(!out.contains('\u{1b}'), "{out:?}");
}

#[test]
fn colors_are_only_emitted_when_selected() {
    let colored = Style {
        color: true,
        ..Style::plain()
    };
    let message = summary(1, "s001", "hello");
    let out = render_message(&message, &mut party(), &colored);
    assert!(out.contains("\u{1b}["), "{out:?}");
    assert!(out.contains("alice"), "{out:?}");
    // The same nick always gets the same color.
    assert_eq!(out, render_message(&message, &mut party(), &colored));
    assert!(!render_message(&message, &mut party(), &Style::plain()).contains('\u{1b}'));
}

#[test]
fn pages_render_oldest_first_with_a_continuation() {
    let page = Page {
        items: vec![
            summary(3, "s001", "third"),
            summary(1, "s001", "first"),
            event(2, MessageKind::Info, r#"{"action":"accept","seat":"s002"}"#),
        ],
        next_cursor: Some("c".into()),
        next_argv: Some(vec![
            "herdr-threads".into(),
            "read".into(),
            "--cursor".into(),
            "c".into(),
        ]),
        high_water_ordinal: 3,
        scope_revision: None,
        has_more: true,
        stop_reason: StopReason::Rows,
        consistency: Consistency::BoundedLive,
    };
    assert_eq!(
        render_page(&page, &mut party(), &Style::plain()),
        "[12:34] <w/alice/s001> first\n\
         [12:34] -!- w/mad-tea-hatter-codex/s002 joined\n\
         [12:34] <w/alice/s001> third\n\
         more: herdr-threads read --cursor c\n"
    );
}

#[test]
fn clock_is_hours_and_minutes() {
    crate::protocol::output::with_render_now(NOW, || {
        assert_eq!(clock(UtcMillis(1_790_771_696_000), false), "12:34");
        assert_eq!(clock(UtcMillis(1_790_771_696_000), true).len(), 5);
    });
    crate::protocol::output::with_render_now(UtcMillis(1), || {
        assert_eq!(clock(UtcMillis(0), false), "00:00");
    });
}

#[test]
fn follow_lines_mark_human_and_relayed_messages() {
    use crate::protocol::summary::AuthorRole;
    let mut human_row = summary(1, "seat-Person01", "hello");
    human_row.author_role = Some(AuthorRole::Human);
    assert_eq!(
        render(&human_row, &mut party()),
        "[12:34] <you·human> [human] hello\n"
    );
    let mut relayed = summary(2, "s001", "do it");
    relayed.author_role = Some(AuthorRole::Agent);
    relayed.relays_user = true;
    assert_eq!(
        render(&relayed, &mut party()),
        "[12:34] <w/alice/s001> [relays user] do it\n"
    );
    assert_eq!(
        render(&summary(3, "s001", "plain"), &mut party()),
        "[12:34] <w/alice/s001> plain\n"
    );
}

#[test]
fn relative_nick_long_parents_keep_pane_component() {
    let labels = crate::host::observation::SeatHostLabels {
        terminal: "test-terminal".into(),
        incarnation: None,
        target: crate::protocol::ids::HostTargetId::new("p"),
        workspace_id: "w".into(),
        workspace_label: Some("project".repeat(30)),
        tab_id: "t".into(),
        tab_label: Some("tryout".repeat(30)),
        pane_label: Some("alice".into()),
        session: None,
    };
    let nick = Nick {
        name: relative_pane_nick(&labels, None),
        harness: None,
        ids: None,
    };
    assert!(
        nick.display().ends_with("/alice"),
        "pane component must survive bounded display: {}",
        nick.display()
    );
}

#[test]
fn relative_nick_expanded_escaped_parents_reserve_pane_budget() {
    for (workspace, tab, pane) in [
        ("\x1b".repeat(100), "\u{202e}".repeat(100), "alice".into()),
        ("w".repeat(100), "t".repeat(100), "p".repeat(100)),
    ] {
        let labels = crate::host::observation::SeatHostLabels {
            terminal: "test-terminal".into(),
            incarnation: None,
            target: crate::protocol::ids::HostTargetId::new("pane"),
            workspace_id: "workspace".into(),
            workspace_label: Some(workspace),
            tab_id: "tab".into(),
            tab_label: Some(tab),
            pane_label: Some(pane),
            session: None,
        };
        let formatted = relative_pane_nick(&labels, None);
        let nick = Nick {
            name: formatted.clone(),
            harness: None,
            ids: None,
        }
        .display();
        assert!(
            nick.ends_with(if labels.pane_label.as_deref() == Some("alice") {
                "/alice"
            } else {
                "/pppppppppppppppppppppppppppppppp…"
            }),
            "pane needs its independent budget: {nick}"
        );
        assert_eq!(
            nick, formatted,
            "final nick clipping must not eat an already bounded component"
        );
        assert!(
            nick.chars().count() <= 3 * (32 + 1) + 2,
            "escaped path must be bounded: {nick}"
        );
        assert!(!nick.contains('\x1b') && !nick.contains('\u{202e}'));
    }
}

#[test]
fn user_intent_irc_markers_are_independent() {
    use crate::protocol::summary::{AuthorRole, UserIntent};
    for (role, relay, source) in [
        (AuthorRole::Human, false, "[human]"),
        (AuthorRole::Human, true, "[human] [relays user]"),
        (AuthorRole::Agent, true, "[relays user]"),
    ] {
        for intent in [
            None,
            Some(UserIntent::Query),
            Some(UserIntent::Request),
            Some(UserIntent::Rule),
        ] {
            let mut row = summary(1, "s001", "quoted rule");
            row.author_role = Some(role);
            row.relays_user = relay;
            row.user_intent = intent;
            let marks = format!(
                "{source}{}",
                intent.map_or(String::new(), |i| format!(" [{}]", i.as_str()))
            );
            let text = render(&row, &mut party());
            assert!(text.contains(&format!("> {marks} quoted rule\n")), "{text}");
        }
    }
    assert_eq!(
        render(&summary(1, "s001", "Always test."), &mut party()),
        "[12:34] <w/alice/s001> Always test.\n"
    );
}

#[test]
fn public_join_is_visible_in_human_read_and_follow() {
    let mut joined = summary(
        1,
        "s001",
        r#"{"action":"join","seat":"s001","generation":1,"observation":"cooperative_top_level"}"#,
    );
    joined.kind = MessageKind::Info;
    assert_eq!(
        render(&joined, &mut party()),
        "[12:34] -!- w/alice/s001 joined\n"
    );
}

struct RegisteredNick {
    seat: SeatId,
    nick: Nick,
}
impl Lookup for RegisteredNick {
    fn nick(&mut self, seat: &SeatId) -> Nick {
        if *seat == self.seat {
            self.nick.clone()
        } else {
            Nick::seat(seat)
        }
    }
}

/// Kills a renderer-only two-brand predicate, including suffix and ID clipping.
#[test]
fn registered_agents_render_messages_and_recipients_without_suffix_or_id_clipping() {
    let seat = SeatId::new("seat-0b5a1c2e-1111-2222-3333-444455556666");
    let mut labels = crate::host::observation::SeatHostLabels {
        terminal: "test".into(),
        incarnation: None,
        target: crate::protocol::ids::HostTargetId::new("w1:p1"),
        workspace_id: "w1".into(),
        workspace_label: Some("a/b\x1b\n".into()),
        tab_id: "t1".into(),
        tab_label: Some("build".into()),
        pane_label: Some("alice\u{202e}".into()),
        session: None,
    };
    for registration in crate::harness::registry::builtins().registrations() {
        let id = registration.metadata().id;
        for (workspace, pane, expected) in [
            (
                Some("a/b\x1b\n".to_owned()),
                Some("alice\u{202e}".to_owned()),
                "a/b\\u{001b}\\n/build/alice\\u{202e}",
            ),
            (
                Some("w".repeat(100)),
                Some("p".repeat(100)),
                "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww…/build/pppppppppppppppppppppppppppppppp…",
            ),
            (Some(String::new()), None, "w1/build"),
        ] {
            labels.workspace_label = workspace;
            labels.pane_label = pane;
            let mut lookup = RegisteredNick {
                seat: seat.clone(),
                nick: Nick {
                    name: agent_seat_nick(&seat, Some(&labels)),
                    harness: Some(id.into()),
                    ids: None,
                },
            };
            let text = render_message(
                &summary(1, seat.as_str(), "hello"),
                &mut lookup,
                &Style {
                    width: 1000,
                    ..Style::plain()
                },
            );
            assert_eq!(
                text,
                format!("[12:34] <{expected}> hello\n"),
                "registered {id}"
            );
            let warning = event(
                2,
                MessageKind::Warn,
                &format!(
                    "{{\"obligation\":\"receipt\",\"seat\":\"{}\"}}",
                    seat.as_str()
                ),
            );
            let text = render_message(
                &warning,
                &mut lookup,
                &Style {
                    width: 1000,
                    ..Style::plain()
                },
            );
            assert!(
                text.contains(&format!("{expected} is overdue")),
                "registered {id}: {text}"
            );
            assert!(!text.contains('·'), "registered {id}: {text}");
            assert!(!text.contains("seat-0b5a1c2e"), "registered {id}: {text}");
        }
        let mut lookup = RegisteredNick {
            seat: seat.clone(),
            nick: Nick {
                name: agent_seat_nick(&seat, None),
                harness: Some(id.into()),
                ids: None,
            },
        };
        assert_eq!(
            render(&summary(1, seat.as_str(), "hello"), &mut lookup),
            "[12:34] <seat-0b5a1c2e-1111-2222-3333-444455556666> hello\n",
            "registered {id}"
        );
    }
}

/// A plausible but unregistered spelling must not grant full-ID agent display.
#[test]
fn unregistered_and_human_nicks_retain_clipping_and_suffix_behavior() {
    let unknown = "not_registered_display_adapter";
    assert!(crate::harness::registry::builtins().agent(unknown).is_err());
    for (harness, expected) in [
        (None, "abcdefghijklmnopqrstuvwxyz012345…"),
        (Some("human"), "abcdefghijklmnopqrstuvwxyz012345…·human"),
        (Some("Human"), "abcdefghijklmnopqrstuvwxyz012345…·Human"),
        (
            Some(unknown),
            "abcdefghijklmnopqrstuvwxyz012345…·not_register…",
        ),
    ] {
        let mut lookup = RegisteredNick {
            seat: SeatId::new("sUnknown"),
            nick: Nick {
                name: "abcdefghijklmnopqrstuvwxyz0123456789".into(),
                harness: harness.map(str::to_owned),
                ids: None,
            },
        };
        assert_eq!(
            render(&summary(1, "sUnknown", "hello"), &mut lookup),
            format!("[12:34] <{expected}> hello\n"),
            "{harness:?}"
        );
    }
}
