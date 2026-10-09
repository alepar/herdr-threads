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
fn agent_nick_escapes_host_names_bounds_each_component_and_keeps_full_legacy_id() {
    let seat = SeatId::new("seat-0b5a1c2e-1111-2222-3333-444455556666");
    let mut labels = crate::host::observation::SeatHostLabels {
        terminal: "terminal".into(),
        incarnation: None,
        target: crate::protocol::ids::HostTargetId::new("w1:p1"),
        workspace_id: "w1".into(),
        workspace_label: Some("a/b\x1b\n".into()),
        tab_id: "t1".into(),
        tab_label: Some("must-not-appear".into()),
        pane_label: Some("alice\u{202e}".into()),
    };
    let nick = |labels: Option<&crate::host::observation::SeatHostLabels>| {
        Nick {
            name: agent_seat_nick(&seat, labels),
            harness: Some("codex".into()),
        }
        .display()
    };
    // Slash text follows the existing label convention; controls are visible.
    assert_eq!(
        nick(Some(&labels)),
        "a/b\\u{001b}\\n/alice\\u{202e}/seat-0b5a1c2e-1111-2222-3333-444455556666"
    );
    labels.workspace_label = Some("w".repeat(100));
    labels.pane_label = Some("p".repeat(100));
    assert_eq!(
        nick(Some(&labels)),
        "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww…/pppppppppppppppppppppppppppppppp…/seat-0b5a1c2e-1111-2222-3333-444455556666"
    );
    assert_eq!(nick(None), "seat-0b5a1c2e-1111-2222-3333-444455556666");
    labels.workspace_label = Some(String::new());
    labels.pane_label = None;
    assert_eq!(
        nick(Some(&labels)),
        "w1/w1:p1/seat-0b5a1c2e-1111-2222-3333-444455556666"
    );
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
    };
    let nick = Nick {
        name: relative_pane_nick(&labels, None),
        harness: None,
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
        };
        let formatted = relative_pane_nick(&labels, None);
        let nick = Nick {
            name: formatted.clone(),
            harness: None,
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
