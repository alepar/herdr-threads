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
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
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
        human(&CommandResult::Accepted(
            InvitationId::new("inv-Q1w2E3r4").into()
        )),
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
fn history_marks_human_and_relayed_messages_after_the_author() {
    use crate::protocol::summary::AuthorRole;
    let mut human_row = summary(1, "from the human");
    human_row.author_role = Some(AuthorRole::Human);
    let mut relayed = summary(2, "relayed");
    relayed.author_role = Some(AuthorRole::Agent);
    relayed.relays_user = true;
    let mut both = summary(3, "both [human]");
    both.author_role = Some(AuthorRole::Human);
    both.relays_user = true;
    let mut service = summary(4, "svc");
    service.author_role = Some(AuthorRole::Service);
    let out = human(&CommandResult::History(page(vec![
        human_row,
        relayed,
        both,
        service,
        summary(5, "plain"),
    ])));
    assert_eq!(
        out,
        "[12:34] <seat-Xy98Zw76> [human] from the human\n\
         [12:34] <seat-Xy98Zw76> [relays user] relayed\n\
         [12:34] <seat-Xy98Zw76> [human] [relays user] both [human]\n\
         [12:34] <seat-Xy98Zw76> svc\n\
         [12:34] <seat-Xy98Zw76> plain\n"
    );
}

// Kills: a join hint missing from the human renderer, or one printed for an
// accept whose thread holds no full chunk.
#[test]
fn accepted_prints_the_summary_hint_only_when_present() {
    use crate::protocol::results::AcceptedInvitation;
    let hinted = CommandResult::Accepted(AcceptedInvitation {
        invitation: InvitationId::new("inv-Q1w2E3r4"),
        summary_available: Some(ThreadId::new("thr-9")),
    });
    assert_eq!(
        human(&hinted),
        "Accepted invitation inv-Q1w2E3r4.\nsummary available: herdr-threads summary thr-9\n"
    );
    assert!(
        !human(&CommandResult::Accepted(
            InvitationId::new("inv-Q1w2E3r4").into()
        ))
        .contains("summary")
    );
}

#[test]
fn pending_table_carries_the_deferral_line() {
    use crate::protocol::{results::PendingReceipt, time::UtcMillis};
    let receipt = |deferred_until: Option<i64>| PendingReceipt {
        message: MessageId::new("m1"),
        thread: ThreadId::new("t1"),
        seat: SeatId::new("s1"),
        sequence: 4,
        sender: Some(SeatId::new("S1")),
        sender_author: None,
        decision_at: UtcMillis(0),
        available_at: Some(UtcMillis(0)),
        deadline: Some(UtcMillis(12 * 3_600_000)),
        overdue: false,
        effective_deadline: deferred_until.map(UtcMillis),
        deferred_until: deferred_until.map(UtcMillis),
    };
    let deferred = human(&CommandResult::PendingReceipts(page(vec![receipt(Some(
        12 * 3_600_000 + 5 * 60_000,
    ))])));
    assert!(
        deferred.contains("1970-01-01 12:00Z deferred: recipient catching up (until 12:05Z)"),
        "{deferred}"
    );
    let plain = human(&CommandResult::PendingReceipts(page(vec![receipt(None)])));
    assert!(!plain.contains("deferred"), "{plain}");
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

#[test]
fn thread_names_human_directory_keeps_name_beside_canonical_id() {
    let summary: crate::protocol::results::ThreadSummary =
        serde_json::from_value(serde_json::json!({
            "thread":"t-human", "name":"team café", "topic_data":"topic", "topic_omitted":false,
            "topic_detail_argv":null,"archived":true,"orphaned":false,"message_count":0,
            "created_at":0,"ordinary_count":0,"system_count":0,"joined_count":1
        }))
        .unwrap();
    let rendered = human(&CommandResult::Directory(page(vec![summary])));
    assert!(rendered.contains("t-human (team café)"), "{rendered}");
    let unnamed = CommandResult::ThreadName(crate::protocol::results::ThreadNameResult {
        thread: ThreadId::new("t-human"),
        name: None,
    });
    assert_eq!(human(&unnamed), "Thread t-human name: unnamed.\n");
}

// Removing the direct lifecycle actor guard would reach pane/config/service
// lookup before refusing a root person operation.
#[test]
fn human_namespace_init_preserves_provenance() {
    use crate::cli::actor_route::InvocationActor;
    let root = std::env::temp_dir().join(format!("human-init-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let mut parsed = crate::cli::commands::parse_argv(["ht", "human", "me", "init"]).unwrap();
    parsed.actor = InvocationActor::Agent;
    let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
        std::sync::Arc::new(crate::app::SystemClock::new());
    let mut output = Vec::new();
    let error =
        crate::cli::me::run_me_init(parsed, false, None, &runtime, &paths, &clock, &mut output)
            .unwrap_err();
    assert!(
        matches!(error, crate::cli::RunError::Api(ref e)
        if e.code == crate::protocol::results::ErrorCode::InvalidRequest
        && e.detail.contains("ht human me init")),
        "{error:?}"
    );
    assert!(
        !paths.instance_dir.exists(),
        "root refusal creates no person state"
    );
    assert!(output.is_empty());
    assert!(crate::cli::commands::parse_argv(["ht", "me", "init"]).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

struct HumanLifecycle {
    root: std::path::PathBuf,
    paths: crate::daemon::paths::InstancePaths,
    instance: uuid::Uuid,
    selection: crate::cli::commands::CooperativeSelection,
    client: HumanClient,
}

struct HumanClient {
    generation: std::sync::atomic::AtomicU64,
    hold: std::sync::atomic::AtomicBool,
    lose_check_in: std::sync::atomic::AtomicBool,
    calls: std::sync::Mutex<Vec<crate::protocol::commands::Command>>,
}

impl crate::ports::LocalClient for HumanClient {
    fn call_with_output(
        &self,
        command: crate::protocol::commands::Command,
        _: &OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.call(command, budget)
    }
    fn call(
        &self,
        command: crate::protocol::commands::Command,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        use crate::protocol::{commands::Command, results::*};
        use std::sync::atomic::Ordering::SeqCst;
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::SeatInspect(q) => Ok(CommandResult::SeatInspect(SeatInspection {
                summary: SeatSummary {
                    seat: q.seat,
                    continuity: ContinuityStatus::Resolved,
                    target: Some(crate::protocol::ids::HostTargetId::new("w1:p1")),
                    generation: self.generation.load(SeqCst),
                    created_at: UtcMillis(0),
                    retired_at: None,
                },
                mapping: MappingStatus {
                    state: ContinuityStatus::Resolved,
                    target: Some(crate::protocol::ids::HostTargetId::new("w1:p1")),
                    detail_argv: None,
                },
                hold: self.hold.load(SeqCst).then(|| HoldSummary {
                    target: crate::protocol::ids::HostTargetId::new("w1:p1"),
                    reason_data: "restore repair".into(),
                    detail_argv: vec![],
                }),
                retirement: None,
                open_binding: None,
                history: page(vec![]),
            })),
            Command::CheckIn(c) | Command::OperatorCheckIn(c) => {
                if self.lose_check_in.load(SeqCst) {
                    return Err(ApiError::unknown_outcome("private check-in response lost"));
                }
                let mut claim = c.claim;
                claim.binding_generation = self.generation.load(SeqCst)
                    + u64::from(matches!(
                        c.mode,
                        crate::protocol::commands::CheckInMode::Lifecycle { .. }
                    ));
                self.generation.store(claim.binding_generation, SeqCst);
                Ok(CommandResult::CheckedIn(CheckInResult {
                    seat: claim.seat.clone(),
                    context: claim,
                    context_disposition: CheckInContextDisposition::Current,
                    offered_through: None,
                    warning_count: 0,
                    warning_count_has_more: false,
                    warnings: page(vec![]),
                    notices: Default::default(),
                    inbox: page(vec![]),
                }))
            }
            _ => Err(ApiError::unknown_outcome(
                "private communication response lost",
            )),
        }
    }
}

impl HumanLifecycle {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("human-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            root.join("state"),
            root.join("host.sock"),
            None,
        )
        .unwrap();
        Self {
            root,
            paths: crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap(),
            instance: uuid::Uuid::new_v4(),
            selection: crate::cli::commands::CooperativeSelection {
                seat: SeatId::new("seat-person"),
                target: crate::protocol::ids::HostTargetId::new("w1:p1"),
                harness: crate::harness::context::Harness::Human,
                role: crate::harness::context::Role::TopLevel,
            },
            client: HumanClient {
                generation: 1.into(),
                hold: false.into(),
                lose_check_in: false.into(),
                calls: Default::default(),
            },
        }
    }
    fn contexts(&self) -> crate::harness::context::ContextJournal {
        crate::cli::seat_contexts(&self.paths, self.instance, &self.selection.seat).unwrap()
    }
    fn run(&self, args: &[&str], operator: bool) -> Result<(), crate::cli::RunError> {
        let mut argv = vec!["ht", "human"];
        argv.extend_from_slice(args);
        let mut parsed = crate::cli::commands::parse_argv(argv).unwrap();
        if operator {
            let crate::cli::commands::CliAction::Mutation(
                crate::cli::commands::MutationSpec::CheckInLifecycle { operator, .. },
            ) = &mut parsed.action
            else {
                panic!("lifecycle required")
            };
            *operator = true;
        }
        crate::cli::run_selected(
            parsed,
            &self.selection,
            &self.paths,
            self.instance,
            &self.client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        )
    }
    fn init(&self) {
        self.run(&["check-in", "--lifecycle-event", "person-first"], false)
            .unwrap();
    }
}
impl Drop for HumanLifecycle {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

// An agent harness, a regenerated pending identity or a fresh Current
// lifecycle would violate the saved Human binding and exact request replay.
#[test]
fn human_namespace_pending_and_current_check_in_keep_identity() {
    use std::sync::atomic::Ordering::SeqCst;
    let f = HumanLifecycle::new();
    f.client.lose_check_in.store(true, SeqCst);
    assert!(
        f.run(&["check-in", "--lifecycle-event", "person-first"], false)
            .is_err()
    );
    let pending = f.contexts().pending().unwrap().unwrap();
    assert_eq!(
        pending.context.harness,
        crate::harness::context::Harness::Human
    );
    assert_eq!(pending.context.binding_generation, 1);
    f.client.lose_check_in.store(false, SeqCst);
    f.init();
    let current = f.contexts().current().unwrap().unwrap();
    assert_eq!(current.harness, crate::harness::context::Harness::Human);
    assert_eq!(current.execution, pending.context.execution);
    assert_eq!(current.session, pending.context.session);
    assert_eq!(current.binding_generation, 2);
    f.run(&["check-in"], false).unwrap();
    assert_eq!(f.contexts().current().unwrap().unwrap(), current);
    let calls = f.client.calls.lock().unwrap();
    let requests: Vec<_> = calls
        .iter()
        .filter(|c| matches!(c, crate::protocol::commands::Command::CheckIn(_)))
        .collect();
    assert_eq!(
        requests[0], requests[1],
        "lost response replay preserves complete frozen request"
    );
}

// Each retained communication intent must carry the same Human binding,
// without inventing an agent harness or operator repair label.
#[test]
fn human_namespace_communication_preserves_claim() {
    use crate::cli::journal::{IntentScope, Journal, OriginalActor, classify_original_actor};
    let operations: &[&[&str]] = &[
        &[
            "send",
            "thread-test",
            "--body",
            "human is peer text",
            "--relays-user",
        ],
        &["invite", "thread-test", "--seat", "seat-recipient"],
        &["accept", "thread-test"],
        &["ack", "message-test"],
    ];
    for args in operations {
        let f = HumanLifecycle::new();
        f.init();
        let context = f.contexts().current().unwrap().unwrap();
        let expected = crate::harness::bridge::caller_claim(&context).unwrap();
        assert_eq!(expected.harness.cooperative_provenance(), "operator_human");
        assert!(f.run(args, false).is_err());
        let journal = Journal::open(f.paths.instance_dir.join("intents")).unwrap();
        let pending = journal
            .page(&crate::protocol::pagination::PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            })
            .unwrap()
            .items;
        assert_eq!(
            pending.len(),
            1,
            "uncertain communication retains its intent"
        );
        let reference = journal
            .resolve_recovery_ref(pending[0].recovery_ref.as_str())
            .unwrap();
        let saved = journal.load(&reference).unwrap();
        assert_eq!(
            saved.header.scope,
            IntentScope::Cooperative {
                instance: f.instance.to_string(),
                seat: f.selection.seat.clone()
            }
        );
        assert_eq!(
            classify_original_actor(&saved.header.scope, &saved.semantic).unwrap(),
            OriginalActor::HumanOrOperator
        );
        let calls = f.client.calls.lock().unwrap();
        let command = calls.last().unwrap();
        assert_eq!(saved.semantic.frozen_claim(), Some(&expected));
        use crate::protocol::commands::Command;
        let emitted_claim = match command {
            Command::SendMessage(c) => &c.claim,
            Command::Invite(c) => &c.claim,
            Command::Accept(c) => &c.claim,
            Command::Ack(c) => &c.claim,
            other => panic!("unexpected communication: {other:?}"),
        };
        assert_eq!(emitted_claim, &expected);
    }
}

// Removing the context mismatch refusal would silently seed Human over an
// agent before a local-account operator check-in has been requested.
#[test]
fn human_namespace_agent_takeover_requires_operator() {
    let f = HumanLifecycle::new();
    let execution = uuid::Uuid::new_v4();
    let agent = crate::harness::context::OccupantContext {
        format_version: 1,
        instance: f.instance,
        seat: f.selection.seat.as_str().into(),
        target: "w1:p1".into(),
        harness: crate::harness::context::Harness::Codex,
        binding_generation: 1,
        execution,
        session: crate::harness::context::SessionReference::PluginContext(execution),
        role: crate::harness::context::Role::TopLevel,
    };
    f.contexts().install_reattached(agent.clone()).unwrap();
    assert!(
        f.run(&["check-in", "--lifecycle-event", "person-first"], false)
            .is_err()
    );
    assert_eq!(f.contexts().current().unwrap(), Some(agent));
    let journal = crate::cli::journal::Journal::open(f.paths.instance_dir.join("intents")).unwrap();
    assert!(
        journal
            .page(&crate::protocol::pagination::PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
            })
            .unwrap()
            .items
            .is_empty()
    );
    f.run(&["check-in", "--lifecycle-event", "person-override"], true)
        .unwrap();
    let context = f.contexts().current().unwrap().unwrap();
    assert_eq!(context.harness, crate::harness::context::Harness::Human);
    assert_ne!(context.execution, execution);
    assert!(matches!(
        f.client.calls.lock().unwrap().last(),
        Some(crate::protocol::commands::Command::OperatorCheckIn(_))
    ));
}

// A stale generation or canonical restore hold must stop communication
// rather than reseeding its session/execution to pass authorization.
#[test]
fn human_namespace_stale_and_hold_guards() {
    use std::sync::atomic::Ordering::SeqCst;
    for held in [false, true] {
        let f = HumanLifecycle::new();
        f.init();
        let before = f.contexts().current().unwrap();
        if held {
            f.client.hold.store(true, SeqCst);
        } else {
            f.client.generation.store(99, SeqCst);
        }
        f.client.calls.lock().unwrap().clear();
        assert!(
            f.run(&["send", "thread-test", "--body", "person mail"], false)
                .is_err()
        );
        assert_eq!(f.contexts().current().unwrap(), before);
        assert_eq!(
            f.client.calls.lock().unwrap().len(),
            1,
            "canonical inspection stops mutation"
        );
        let journal =
            crate::cli::journal::Journal::open(f.paths.instance_dir.join("intents")).unwrap();
        assert!(
            journal
                .page(&crate::protocol::pagination::PageRequest {
                    cursor: None,
                    limit: 10,
                    max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES
                })
                .unwrap()
                .items
                .is_empty()
        );
    }
}
