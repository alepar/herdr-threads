//! Presentation contract (ht-p03.17): one golden file per (result type, mode)
//! under `tests/cli/golden/`, the contract table in `docs/agent-usage.md`, and
//! a check that every continuation label the table lists is printed.
//!
//! `HT_BLESS=1 cargo test golden_files_match` rewrites the golden files.

use crate::cli::human::render as human_render;
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness},
    ids::{
        ExecutionId, HostTargetId, InvitationId, MessageId, NativeSessionId, RequirementId, SeatId,
        ServiceAuthorId, ThreadId,
    },
    output::{ContinuationContext, OutputFormat, OutputSpec, encode_selected, with_render_now},
    pagination::{Consistency, Page, StopReason},
    results::{
        AckResult, AlreadyJoined, CheckInContextDisposition, CheckInResult, CommandResult,
        InboxItem, MembershipStatus, MessageContent, MessageDetails, MessageKind, MessageSummary,
        NoticeOffer, Participant, PendingReceipt, SearchHit, SearchPage, ThreadDetails,
        ThreadSummary, WarningRef,
    },
    service::{RequiredMembership, RequirementState},
    time::UtcMillis,
};
use std::{collections::BTreeSet, path::PathBuf};

mod fixtures {
    use super::*;

    /// Unix milliseconds of a UTC civil date and time.
    pub fn ms(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> UtcMillis {
        // Howard Hinnant's days-from-civil algorithm.
        let y = year - i64::from(month <= 2);
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let mp = (month + 9) % 12;
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        UtcMillis(((days * 24 + hour) * 60 + minute) * 60_000)
    }

    /// `2026-10-01T09:30Z`: the same day as the render-time now.
    pub fn today() -> UtcMillis {
        ms(2026, 10, 1, 9, 30)
    }

    /// `2026-09-28T08:45Z`: an earlier day.
    pub fn old() -> UtcMillis {
        ms(2026, 9, 28, 8, 45)
    }

    /// `2026-10-03T09:30Z`: a later day.
    pub fn later() -> UtcMillis {
        ms(2026, 10, 3, 9, 30)
    }

    /// The render-time now every golden is rendered under.
    pub fn now() -> UtcMillis {
        ms(2026, 10, 1, 12, 0)
    }

    pub fn argv(parts: &[&str]) -> Vec<String> {
        std::iter::once("herdr-threads")
            .chain(parts.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    pub fn page<T>(items: Vec<T>, more: Option<&[&str]>) -> Page<T> {
        Page {
            items,
            next_cursor: more.map(|_| "cursor-1".to_owned()),
            next_argv: more.map(argv),
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: more.is_some(),
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }

    pub fn summary(sequence: u64, author: &str, at: UtcMillis, preview: &str) -> MessageSummary {
        MessageSummary {
            message: MessageId::new(format!("msg-{sequence}")),
            thread: ThreadId::new("t-1"),
            author: Some(SeatId::new(author)),
            event_author: None,
            author_role: None,
            relays_user: false,
            author_role_backfilled: false,
            kind: MessageKind::Ordinary,
            sequence,
            created_at: at,
            actor_label: None,
            preview_data: preview.to_owned(),
            preview_omitted: false,
            preview_detail_argv: None,
        }
    }

    /// A system event row whose preview is the structured event JSON.
    pub fn event(sequence: u64, at: UtcMillis, json: &str) -> MessageSummary {
        let mut row = summary(sequence, "seat-a", at, json);
        row.kind = MessageKind::Info;
        row.author = None;
        row
    }

    pub fn required() -> RequiredMembership {
        RequiredMembership {
            requirement: RequirementId::new("req-1"),
            revision: 2,
            invitation: InvitationId::new("inv-1"),
            thread: ThreadId::new("t-1"),
            seat: SeatId::new("seat-b"),
            issuer: ServiceAuthorId::new("svc-1"),
            state: RequirementState::Pending,
            accepted_by: None,
            accepted_at: None,
        }
    }

    pub fn inbox_item(thread: &str, receipts: u64, more: bool, requirement: bool) -> InboxItem {
        InboxItem {
            thread: ThreadId::new(thread),
            invitations: u64::from(requirement),
            invitations_has_more: false,
            pending_receipts: receipts,
            pending_receipts_has_more: more,
            warnings: 0,
            warnings_has_more: false,
            pending_requirement: requirement.then(required),
        }
    }

    pub fn inbox_page() -> Page<InboxItem> {
        page(
            vec![
                inbox_item("t-1", 2, false, true),
                inbox_item("t-2", 1, true, false),
            ],
            Some(&["inbox", "--cursor", "cursor-1"]),
        )
    }

    pub fn participant(seat: &str, state: MembershipStatus, is_self: bool) -> Participant {
        Participant {
            seat: SeatId::new(seat),
            is_self,
            requirement: (seat == "seat-b").then(required),
            episode: 1,
            joined: state == MembershipStatus::Joined,
            retired: false,
            physical_state: state,
            effective_state: state,
            joined_at: (state == MembershipStatus::Joined).then(old),
            left_at: None,
            retirement_cutover: None,
            cleanup_state: None,
            accepted_invitation: None,
        }
    }

    pub fn participants_page() -> Page<Participant> {
        page(
            vec![
                participant("seat-a", MembershipStatus::Joined, true),
                participant("seat-b", MembershipStatus::Invited, false),
            ],
            Some(&["participants", "t-1", "--cursor", "cursor-1"]),
        )
    }

    pub fn thread_summary(thread: &str, topic: &str, omitted: bool) -> ThreadSummary {
        ThreadSummary {
            thread: ThreadId::new(thread),
            managed_owner: None,
            topic_data: topic.to_owned(),
            topic_omitted: omitted,
            topic_detail_argv: omitted.then(|| argv(&["thread", "show", thread])),
            archived: false,
            orphaned: false,
            message_count: 7,
            created_at: old(),
            ordinary_count: 5,
            system_count: 2,
            joined_count: 2,
        }
    }

    pub fn pending_receipts_page() -> Page<PendingReceipt> {
        let receipt = |sequence: u64, deadline: Option<UtcMillis>, overdue: bool| PendingReceipt {
            message: MessageId::new(format!("msg-{sequence}")),
            thread: ThreadId::new("t-1"),
            seat: SeatId::new("seat-a"),
            sequence,
            sender: SeatId::new("seat-b"),
            decision_at: old(),
            available_at: None,
            deadline,
            overdue,
            effective_deadline: None,
            deferred_until: None,
        };
        page(
            vec![
                receipt(3, Some(old()), true),
                receipt(4, Some(today()), false),
                receipt(5, Some(later()), false),
                receipt(6, None, false),
            ],
            Some(&["pending-receipts", "--cursor", "cursor-1"]),
        )
    }

    pub fn history_page() -> Page<MessageSummary> {
        let mut clipped = summary(2, "seat-b", today(), "a long reply that was clipped");
        clipped.preview_omitted = true;
        clipped.preview_detail_argv = Some(argv(&["body", "msg-2"]));
        let mut warn = summary(3, "seat-a", today(), "deadline missed");
        warn.kind = MessageKind::Warn;
        let deadline = event(
            4,
            today(),
            r#"{"action":"deadline_set","seat":"seat-b","message":"msg-2","deadline":"2026-10-03T09:30Z","constraint":"reply","revision":3,"detail":"please answer"}"#,
        );
        page(
            vec![
                summary(1, "seat-a", old(), "hello there"),
                clipped,
                warn,
                deadline,
            ],
            Some(&["history", "t-1", "--cursor", "cursor-1"]),
        )
    }

    pub fn thread_details() -> ThreadDetails {
        ThreadDetails {
            summary: thread_summary("t-1", "Plan the release", true),
            goal_data: "Ship the release by Friday".to_owned(),
            created_at: old(),
            participant_count: 2,
            participants: participants_page(),
            pending_receipt_count: 2,
            pending_receipts_argv: argv(&["pending-receipts", "--thread", "t-1"]),
        }
    }

    pub fn warning(sequence: u64) -> WarningRef {
        WarningRef {
            warning: MessageId::new(format!("warn-{sequence}")),
            thread: ThreadId::new("t-1"),
            sequence,
            event_seq: sequence + 100,
        }
    }

    pub fn checked_in() -> CheckInResult {
        CheckInResult {
            context_disposition: CheckInContextDisposition::Current,
            context: CallerClaim {
                instance: "instance-1".into(),
                seat: SeatId::new("seat-a"),
                binding_generation: 3,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("native-1"),
                execution: ExecutionId::new("execution-1"),
                target: HostTargetId::new("pane-1"),
            },
            seat: SeatId::new("seat-a"),
            offered_through: Some("evt-9".to_owned()),
            warning_count: 2,
            warning_count_has_more: false,
            warnings: page(
                vec![warning(4), warning(5)],
                Some(&["warnings", "--seat", "seat-a", "--cursor", "cursor-1"]),
            ),
            notices: NoticeOffer {
                items: vec![warning(6)],
                has_more: true,
            },
            inbox: inbox_page(),
        }
    }

    pub fn message_details() -> MessageDetails {
        MessageDetails {
            summary: summary(2, "seat-b", today(), "Here is the first part"),
            content: MessageContent::Ordinary {
                body_data: "Here is the first part\nof a longer body".to_owned(),
                body_offset: 0,
                body_total_bytes: 400,
                body_complete: false,
                body_next_cursor: Some("cursor-1".to_owned()),
                body_next_argv: Some(argv(&["body", "msg-2", "--offset", "39"])),
            },
        }
    }

    pub fn already_joined() -> AlreadyJoined {
        AlreadyJoined {
            thread: ThreadId::new("t-1"),
            seat: SeatId::new("seat-a"),
        }
    }

    pub fn search() -> SearchPage {
        SearchPage {
            matches: page(
                vec![
                    SearchHit::Topic(thread_summary("t-2", "Release notes", false)),
                    SearchHit::Body(summary(1, "seat-a", old(), "release candidate is ready")),
                ],
                Some(&["search", "release", "--cursor", "cursor-1"]),
            ),
            examined_candidates: 2,
            examined_utf8_bytes: 100,
        }
    }

    pub fn acknowledged() -> AckResult {
        AckResult {
            acknowledged: vec![MessageId::new("msg-1"), MessageId::new("msg-2")],
            already_acknowledged: vec![MessageId::new("msg-0")],
        }
    }
}
use fixtures::*;

fn text_spec() -> OutputSpec {
    OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext::default(),
    }
}

fn compact(result: &CommandResult) -> String {
    with_render_now(now(), || {
        String::from_utf8(encode_selected(result, &text_spec()).expect("encodes")).expect("utf8")
    })
}

fn human(result: &CommandResult) -> String {
    with_render_now(now(), || {
        human_render(result, &text_spec()).expect("human form")
    })
}

/// Every contract row: the result type, the mode, and its fixture.
fn rows() -> Vec<(&'static str, &'static str, CommandResult)> {
    let mut rows = vec![
        ("history", "compact", CommandResult::History(history_page())),
        ("inbox", "compact", CommandResult::Inbox(inbox_page())),
        (
            "pending_receipts",
            "compact",
            CommandResult::PendingReceipts(pending_receipts_page()),
        ),
        (
            "participants",
            "compact",
            CommandResult::Participants(participants_page()),
        ),
        ("thread", "compact", CommandResult::Thread(thread_details())),
        (
            "checked_in",
            "compact",
            CommandResult::CheckedIn(checked_in()),
        ),
        (
            "message",
            "compact",
            CommandResult::Message(message_details()),
        ),
        (
            "already_joined",
            "compact",
            CommandResult::AlreadyJoined(already_joined()),
        ),
    ];
    rows.extend([
        ("inbox", "human", CommandResult::Inbox(inbox_page())),
        (
            "directory",
            "human",
            CommandResult::Directory(page(
                vec![
                    thread_summary("t-1", "Plan the release", false),
                    thread_summary("t-2", "Release notes", true),
                ],
                Some(&["threads", "--cursor", "cursor-1"]),
            )),
        ),
        ("history", "human", CommandResult::History(history_page())),
        ("thread", "human", CommandResult::Thread(thread_details())),
        (
            "participants",
            "human",
            CommandResult::Participants(participants_page()),
        ),
        (
            "message",
            "human",
            CommandResult::Message(message_details()),
        ),
        (
            "pending_receipts",
            "human",
            CommandResult::PendingReceipts(pending_receipts_page()),
        ),
        ("search", "human", CommandResult::Search(search())),
        (
            "checked_in",
            "human",
            CommandResult::CheckedIn(checked_in()),
        ),
        (
            "already_joined",
            "human",
            CommandResult::AlreadyJoined(already_joined()),
        ),
        (
            "acknowledged",
            "human",
            CommandResult::Acknowledged(acknowledged()),
        ),
    ]);
    rows
}

fn render_row(mode: &str, result: &CommandResult) -> String {
    match mode {
        "compact" => compact(result),
        "human" => human(result),
        other => panic!("unknown mode {other}"),
    }
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cli/golden")
}

#[test]
fn golden_files_match() {
    let bless = std::env::var_os("HT_BLESS").is_some();
    let mut failures = Vec::new();
    for (kind, mode, result) in rows() {
        let path = golden_dir().join(format!("{kind}.{mode}.txt"));
        let actual = render_row(mode, &result);
        if bless {
            std::fs::create_dir_all(golden_dir()).expect("golden dir");
            std::fs::write(&path, &actual).expect("write golden");
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => failures.push(format!(
                "{kind}.{mode}: golden differs (HT_BLESS=1 rewrites)\n--- expected\n{expected}--- actual\n{actual}"
            )),
            Err(error) => failures.push(format!(
                "{kind}.{mode}: no golden at {} ({error}); HT_BLESS=1 writes it",
                path.display()
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// One parsed row of the contract table in `docs/agent-usage.md`.
struct ContractRow {
    kind: String,
    mode: String,
    labels: Vec<String>,
}

fn strip_ticks(cell: &str) -> String {
    cell.trim().trim_matches('`').to_owned()
}

fn contract_table() -> Vec<ContractRow> {
    let doc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/agent-usage.md"),
    )
    .expect("docs/agent-usage.md");
    let start = "<!-- presentation-contract:start -->";
    let end = "<!-- presentation-contract:end -->";
    let from = doc.find(start).expect("contract start marker") + start.len();
    let to = doc.find(end).expect("contract end marker");
    doc[from..to]
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|'))
        .skip(2) // header and separator
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
            assert_eq!(cells.len(), 4, "contract row needs four columns: {line}");
            let labels = cells[3]
                .split('`')
                .enumerate()
                .filter(|(index, _)| index % 2 == 1)
                .map(|(_, label)| label.to_owned())
                .collect();
            ContractRow {
                kind: strip_ticks(cells[0]),
                mode: strip_ticks(cells[1]),
                labels,
            }
        })
        .collect()
}

#[test]
fn contract_table_lists_every_golden() {
    let table: BTreeSet<(String, String)> = contract_table()
        .into_iter()
        .map(|row| (row.kind, row.mode))
        .collect();
    // `kind.mode.txt` files are the presentation contract; a golden without a
    // mode (`read_transcript_names.txt`) belongs to another test.
    let goldens: BTreeSet<(String, String)> = std::fs::read_dir(golden_dir())
        .expect("golden dir")
        .filter_map(|entry| {
            let name = entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("utf8");
            let stem = name.strip_suffix(".txt").expect("golden is .txt");
            let (kind, mode) = stem.rsplit_once('.')?;
            Some((kind.to_owned(), mode.to_owned()))
        })
        .collect();
    let fixtures: BTreeSet<(String, String)> = rows()
        .into_iter()
        .map(|(kind, mode, _)| (kind.to_owned(), mode.to_owned()))
        .collect();
    assert_eq!(table, goldens, "table rows and golden files differ");
    assert_eq!(table, fixtures, "table rows and fixtures differ");
    assert_eq!(table.len(), 19);
}

#[test]
fn every_contract_continuation_appears_in_compact_output() {
    let fixtures = rows();
    for row in contract_table() {
        let (_, _, result) = fixtures
            .iter()
            .find(|(kind, mode, _)| *kind == row.kind && *mode == row.mode)
            .unwrap_or_else(|| panic!("no fixture for {}.{}", row.kind, row.mode));
        let out = render_row(&row.mode, result);
        for label in &row.labels {
            assert!(
                out.contains(&format!("{label} herdr-threads ")),
                "{}.{}: label `{label}` is not followed by a command in:\n{out}",
                row.kind,
                row.mode
            );
        }
    }
}

// ---- one assertion per continuation fix -----------------------------------

#[test]
fn human_checked_in_keeps_warnings_continuation_and_offered_through() {
    let out = human(&CommandResult::CheckedIn(checked_in()));
    assert!(out.contains("offered through evt-9"), "{out}");
    assert!(out.contains("current"), "{out}");
    assert!(out.contains("warn-4 t-1#4"), "{out}");
    assert!(
        out.contains("more: herdr-threads warnings --seat seat-a --cursor cursor-1\n"),
        "{out}"
    );
    assert!(
        out.contains("notices.more: herdr-threads warnings --seat seat-a\n"),
        "{out}"
    );
}

#[test]
fn compact_thread_keeps_topic_detail_argv() {
    let out = compact(&CommandResult::Thread(thread_details()));
    assert!(
        out.contains("topic: Plan the release…\ntopic.more: herdr-threads thread show t-1\n"),
        "{out}"
    );
    // A thread whose topic is whole prints no continuation.
    let mut whole = thread_details();
    whole.summary = thread_summary("t-1", "Plan the release", false);
    assert!(!compact(&CommandResult::Thread(whole)).contains("topic.more:"));
}

#[test]
fn human_inbox_shows_invitation_id_and_accept_command() {
    let out = human(&CommandResult::Inbox(inbox_page()));
    assert!(out.contains("required inv=inv-1 rev=2"), "{out}");
    assert!(
        out.contains(
            "accept-required: herdr-threads accept-required t-1 --invitation inv-1 \
             --requirement req-1 --revision 2\n"
        ),
        "{out}"
    );
    // Only the required item gets an accept command.
    assert_eq!(out.matches("accept-required:").count(), 1, "{out}");
}

#[test]
fn human_inbox_shows_topics_when_known() {
    let topics = std::collections::HashMap::from([(
        ThreadId::new("t-1"),
        ("Plan the release".to_owned(), false),
    )]);
    let out =
        crate::cli::human::with_inbox_topics(topics, || human(&CommandResult::Inbox(inbox_page())));
    let header = out.lines().next().unwrap();
    assert!(header.contains("TOPIC"), "{out}");
    let row = |thread: &str| out.lines().find(|line| line.starts_with(thread)).unwrap();
    assert!(row("t-1").contains("Plan the release"), "{out}");
    // A thread that is not in the map shows a dash in the topic column.
    assert!(row("t-2").split_whitespace().nth(1) == Some("-"), "{out}");
    // Without a map there is no topic column.
    assert!(!human(&CommandResult::Inbox(inbox_page())).contains("TOPIC"));
}

#[test]
fn event_row_keeps_deadline_constraint_revision() {
    let out = compact(&CommandResult::History(history_page()));
    assert!(
        out.contains(
            "#4 deadline_set seat-b msg-2 constraint=reply deadline=2026-10-03T09:30Z \
             revision=3: please answer\n"
        ),
        "{out}"
    );
    // At most four extra fields, then a count of the rest.
    let many = event(
        1,
        today(),
        r#"{"action":"x","a":1,"b":2,"c":3,"d":4,"e":5,"f":6}"#,
    );
    let out = compact(&CommandResult::History(page(vec![many], None)));
    assert!(out.contains("#1 x a=1 b=2 c=3 d=4 +2 fields\n"), "{out}");
}

#[test]
fn compact_already_joined_has_a_specific_form() {
    assert_eq!(
        compact(&CommandResult::AlreadyJoined(already_joined())),
        "already_joined t-1 seat-a: no invitation sent\n"
    );
}

#[test]
fn older_timestamps_carry_a_date() {
    let out = compact(&CommandResult::History(history_page()));
    assert!(
        out.contains("#1 msg-1 seat-a 09-28 08:45Z: hello there\n"),
        "{out}"
    );
    // A later date is as ambiguous as an earlier one.
    let out = compact(&CommandResult::PendingReceipts(pending_receipts_page()));
    assert!(out.contains("due 09-28 08:45Z overdue"), "{out}");
    assert!(out.contains("due 10-03 09:30Z\n"), "{out}");
    let out = human(&CommandResult::History(history_page()));
    assert!(out.contains("[09-28 08:45] <seat-a> hello there"), "{out}");
}

#[test]
fn todays_timestamps_stay_bare() {
    let out = compact(&CommandResult::History(history_page()));
    assert!(out.contains("#2 msg-2 seat-b 09:30Z [more:"), "{out}");
    let out = compact(&CommandResult::PendingReceipts(pending_receipts_page()));
    assert!(
        out.contains("msg-4 t-1#4 from seat-b due 09:30Z\n"),
        "{out}"
    );
    let out = human(&CommandResult::History(history_page()));
    assert!(out.contains("[09:30] <seat-b> a long reply"), "{out}");
}

/// A client that answers an inbox and a directory read and counts calls.
struct TopicClient {
    calls: std::sync::Mutex<Vec<crate::protocol::commands::Command>>,
}

impl crate::ports::LocalClient for TopicClient {
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
        use crate::protocol::commands::Command;
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::Inbox(_) => Ok(CommandResult::Inbox(inbox_page())),
            Command::Directory(_) => Ok(CommandResult::Directory(page(
                vec![thread_summary("t-1", "Plan the release", false)],
                None,
            ))),
            other => panic!("unexpected command {other:?}"),
        }
    }
}

fn run_inbox(presentation: crate::cli::output::Presentation) -> (String, Vec<String>) {
    use crate::protocol::commands::{Command, DirectoryMembership, InboxQuery};
    let client = TopicClient {
        calls: Default::default(),
    };
    let spec = text_spec();
    let _guard = crate::cli::output::PresentationGuard::enter(presentation, &spec);
    let query = InboxQuery {
        seat: Some(SeatId::new("seat-a")),
        page: Default::default(),
    };
    let mut out = Vec::new();
    crate::cli::run_wire(
        Command::Inbox(query),
        &spec,
        &client,
        &|| crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &mut out,
    )
    .expect("inbox runs");
    let calls = client.calls.into_inner().unwrap();
    let directory: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Command::Directory(query) => Some(query),
            _ => None,
        })
        .collect();
    for query in &directory {
        assert_eq!(query.membership, Some(SeatId::new("seat-a")));
        assert_eq!(query.membership_filter, DirectoryMembership::All);
        assert_eq!(query.topic_contains, None);
        assert_eq!(query.page.limit, 50);
    }
    let kinds = calls
        .iter()
        .map(|call| match call {
            Command::Inbox(_) => "inbox".to_owned(),
            Command::Directory(_) => "directory".to_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    (String::from_utf8(out).unwrap(), kinds)
}

#[test]
fn human_inbox_fetches_topics_once() {
    let (out, calls) = run_inbox(crate::cli::output::Presentation::Human);
    assert_eq!(calls, ["inbox", "directory"], "{out}");
    assert!(
        out.contains("TOPIC") && out.contains("Plan the release"),
        "{out}"
    );
    // Machine mode makes no extra call and prints no topic column.
    let (out, calls) = run_inbox(crate::cli::output::Presentation::Machine);
    assert_eq!(calls, ["inbox"], "{out}");
    assert!(!out.contains("Plan the release"), "{out}");
}
