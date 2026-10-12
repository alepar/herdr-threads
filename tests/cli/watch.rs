use super::*;
use crate::cli::{commands::parse_argv, CliAction, RunError};
use crate::protocol::{
    authority::CallerRole,
    commands::Command,
    ids::{ExecutionId, HostTargetId, InvitationId, ThreadId},
    pagination::{Consistency, Page, StopReason},
    results::{CapabilityList, CommandResult, ErrorCode, InboxBatchV2Item},
    time::CallBudget,
    watch::{
        ModAckReport, WatchCloseReason, WatchItem, WatchLine, WatchRefusal, WatchRefusalReason,
        WatchStatus, WatchStatusReason, WatchStatusState,
    },
};
use std::sync::Mutex;

fn action(argv: &[&str]) -> Result<CliAction, crate::protocol::results::ApiError> {
    parse_argv(argv.iter().copied()).map(|parsed| parsed.action)
}

#[test]
fn parses_watch_argv() {
    assert_eq!(
        action(&[
            "herdr-threads",
            "watch",
            "--harness",
            "claude",
            "--session",
            "sess-1"
        ])
        .unwrap(),
        CliAction::Watch(WatchRequest {
            harness: Harness::Claude,
            session: "sess-1".into()
        })
    );
}

#[test]
fn parses_watch_ack_argv() {
    assert_eq!(
        action(&[
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "sess-1",
            "--via",
            "append",
            "m1",
            "m2"
        ])
        .unwrap(),
        CliAction::WatchAck(WatchAckRequest {
            session: "sess-1".into(),
            via: ModDeliveryVia::Append,
            messages: vec![MessageId::new("m1"), MessageId::new("m2")],
        })
    );
}

#[test]
fn rejects_bad_watch_argv() {
    for argv in [
        vec!["herdr-threads", "watch"],
        vec![
            "herdr-threads",
            "watch",
            "--harness",
            "codex",
            "--session",
            "s",
        ],
        vec![
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "s",
            "--via",
            "context",
        ],
        vec![
            "herdr-threads",
            "watch",
            "ack",
            "--session",
            "s",
            "--via",
            "later",
            "m1",
        ],
    ] {
        assert!(action(&argv).is_err(), "{argv:?}");
    }
}

#[test]
fn stub_mod_files_follow_the_layout() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/claude/mod");
    let read = |path: &str| std::fs::read_to_string(root.join(path)).unwrap();
    let plugin: serde_json::Value =
        serde_json::from_str(&read(".claude-plugin/plugin.json")).unwrap();
    assert_eq!(plugin["name"], "herdr-threads");
    assert_eq!(plugin["types"], "./types/index.d.ts");
    let types = read("types/index.d.ts");
    assert!(types.contains("PluginState"));
    assert!(types.contains("'herdr-threads'"));
    let hooks: serde_json::Value = serde_json::from_str(&read("hooks/hooks.json")).unwrap();
    assert_eq!(hooks, serde_json::json!({"modules":["./register.js"]}));
    assert!(read("hooks/register.js").contains("export const register"));
}

// ---- helpers ----

type Handler = Box<dyn Fn(&Command) -> Result<CommandResult, ApiError> + Send + Sync>;

struct FakeClient {
    handler: Handler,
    calls: Mutex<Vec<Command>>,
    watch_capability: bool,
    presentation_capability: bool,
    /// The daemon also serves per-binding unread streams (`inbox.binding_cursors_v1`).
    cursors_capability: bool,
    /// Explicit Watch-mode agreement, independent of generic presentation support.
    watch_presentation_capability: bool,
    /// Makes the capability probe fail: `Err` is an outer (transport) failure,
    /// `Ok` a definitive daemon rejection.
    probe: Option<Result<ApiError, ApiError>>,
}

impl FakeClient {
    fn new(
        handler: impl Fn(&Command) -> Result<CommandResult, ApiError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            handler: Box::new(handler),
            calls: Mutex::new(Vec::new()),
            watch_capability: true,
            presentation_capability: false,
            cursors_capability: false,
            watch_presentation_capability: false,
            probe: None,
        }
    }

    fn calls(&self) -> Vec<Command> {
        self.calls.lock().unwrap().clone()
    }
}

impl LocalClient for FakeClient {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        self.watch_capability && name == MOD_WATCH
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::Capabilities) {
            return match self.call_definitive(command, budget) {
                Ok(answer) => answer,
                Err(error) => Err(error),
            };
        }
        self.calls.lock().unwrap().push(command.clone());
        (self.handler)(&command)
    }
    fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        if matches!(command, Command::Capabilities) {
            // Not recorded in `calls()`: the probe is not part of the stream's traffic.
            return match &self.probe {
                Some(Err(outer)) => Err(outer.clone()),
                Some(Ok(rejection)) => Ok(Err(rejection.clone())),
                None => Ok(Ok(CommandResult::Capabilities(CapabilityList {
                    capabilities: if self.watch_capability {
                        let mut list = vec![MOD_WATCH.to_owned()];
                        if self.presentation_capability {
                            list.push(
                                crate::protocol::capabilities::DELIVERY_PRESENTATION_V2.to_owned(),
                            );
                        }
                        if self.cursors_capability {
                            list.push(crate::protocol::capabilities::BINDING_CURSORS.to_owned());
                        }
                        if self.watch_presentation_capability {
                            list.push("mod.watch_presentation_v2".to_owned());
                        }
                        list
                    } else {
                        Vec::new()
                    },
                }))),
            };
        }
        self.call(command, budget).map(Ok)
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}

fn clock() -> crate::app::SystemClock {
    crate::app::SystemClock::new()
}

fn page<T>(items: Vec<T>, next: Option<&str>) -> Page<T> {
    Page {
        items,
        next_cursor: next.map(str::to_owned),
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: next.is_some(),
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

fn message(id: &str, body: &str, start: u64, end: u64, len: u64, ack: bool) -> InboxBatchV2Item {
    InboxBatchV2Item::Message {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: Some(SeatId::new("sender")),
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: body.into(),
        body_start: start,
        body_end: end,
        body_len: len,
        ack_candidate: ack.then(|| MessageId::new(id)),
    }
}

fn whole(id: &str, body: &str) -> InboxBatchV2Item {
    message(id, body, 0, body.len() as u64, body.len() as u64, true)
}

fn lazy(id: &str, body: &str) -> InboxBatchV2Item {
    InboxBatchV2Item::LazyMessage {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: body.into(),
        body_start: 0,
        body_end: body.len() as u64,
        body_len: body.len() as u64,
        wakes: false,
    }
}

fn invitation() -> InboxBatchV2Item {
    InboxBatchV2Item::Invitation {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        goal_data: Some("goal".into()),
        topic_revision: None,
        invitation: InvitationId::new("invitation-1"),
        confirmation: None,
        deadline_at: None,
        required_service: None,
    }
}

fn warning() -> InboxBatchV2Item {
    InboxBatchV2Item::Warning {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        warning: MessageId::new("w1"),
        sequence: 2,
        informational: false,
    }
}

/// A client whose inbox serves `pages` in order, each reached by the cursor
/// `p<n>` the page before it returned.
fn paged_client(pages: Vec<Vec<InboxBatchV2Item>>) -> FakeClient {
    let pages = Mutex::new(pages);
    FakeClient::new(move |command| {
        let Command::InboxBatchV2(query) = command else {
            panic!("unexpected command {command:?}");
        };
        let index = query
            .page
            .cursor
            .as_deref()
            .map_or(0, |cursor| cursor[1..].parse::<usize>().unwrap());
        let mut pages = pages.lock().unwrap();
        let total = pages.len();
        let items = std::mem::take(&mut pages[index]);
        let next = format!("p{}", index + 1);
        Ok(CommandResult::InboxBatchV2(page(
            items,
            (index + 1 < total).then_some(next.as_str()),
        )))
    })
}

fn lines(bytes: &[u8]) -> Vec<WatchLine> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn run_drain(
    client: &FakeClient,
    version: u64,
    state: &mut EmitState,
) -> (Result<(), DrainError>, Vec<WatchLine>) {
    let mut out = Vec::new();
    let result = drain(
        client,
        &clock(),
        &SeatId::new("me"),
        None,
        version,
        state,
        &mut out,
    );
    (result, lines(&out))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "htw-{tag}-{:08x}",
        uuid::Uuid::new_v4().as_fields().0
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn status_of(line: &WatchLine) -> &WatchStatus {
    match &line.item {
        WatchItem::Status(status) => status,
        other => panic!("not a status line: {other:?}"),
    }
}

fn message_of(line: &WatchLine) -> &WatchMessage {
    match &line.item {
        WatchItem::Message(message) | WatchItem::Lazy(message) => message,
        other => panic!("not a message line: {other:?}"),
    }
}

// ---- pre-checks ----

#[test]
fn env_off_exits_3_with_env_disabled_status_before_connecting() {
    // `precheck` takes no client, paths or socket: nothing can be contacted.
    for is_watch in [true, false] {
        let mut out = Vec::new();
        let result = precheck(Some("off"), Some("w1:p1"), false, is_watch, &mut out);
        assert!(matches!(result, Err(RunError::Exit(3))));
        let got = lines(&out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "status:0");
        assert_eq!(
            status_of(&got[0]),
            &WatchStatus {
                state: WatchStatusState::Refused,
                reason: Some(WatchStatusReason::EnvDisabled),
                exit: Some(3),
            }
        );
    }
    // Env off outranks a missing pane; any other value does not refuse.
    let mut out = Vec::new();
    assert!(precheck(Some("off"), None, false, true, &mut out).is_err());
    assert_eq!(
        status_of(&lines(&out)[0]).reason,
        Some(WatchStatusReason::EnvDisabled)
    );
    let mut out = Vec::new();
    assert!(precheck(Some("on"), Some("w1:p1"), false, true, &mut out).is_ok());
    assert!(precheck(None, Some("w1:p1"), false, true, &mut out).is_ok());
    assert!(out.is_empty());
}

#[test]
fn missing_pane_exits_3_with_no_pane_status_and_registers_nothing() {
    for pane in [None, Some("")] {
        let mut out = Vec::new();
        let result = precheck(None, pane, false, true, &mut out);
        assert!(matches!(result, Err(RunError::Exit(3))), "{pane:?}");
        let got = lines(&out);
        assert_eq!(got.len(), 1);
        assert_eq!(
            status_of(&got[0]),
            &WatchStatus {
                state: WatchStatusState::Refused,
                reason: Some(WatchStatusReason::NoPane),
                exit: Some(3),
            }
        );
    }
    // A cooperative selection stands in for the pane; `watch ack` has no pane check.
    let mut out = Vec::new();
    assert!(precheck(None, None, true, true, &mut out).is_ok());
    assert!(precheck(None, None, false, false, &mut out).is_ok());
    assert!(out.is_empty());
}

#[test]
fn missing_pane_through_run_in_pane_prints_one_status_and_exits_3() {
    if std::env::var_os(crate::protocol::watch::MOD_DELIVERY_ENV).is_some() {
        return;
    }
    let mut out = Vec::new();
    let result = crate::cli::run_in_pane(
        [
            "herdr-threads",
            "watch",
            "--harness",
            "claude",
            "--session",
            "s1",
        ],
        None,
        &mut out,
    );
    assert!(matches!(result, Err(RunError::Exit(3))));
    let got = lines(&out);
    assert_eq!(got.len(), 1);
    assert_eq!(status_of(&got[0]).reason, Some(WatchStatusReason::NoPane));
}

#[test]
fn unlocated_caller_exits_2_no_binding() {
    let root = temp_dir("loc");
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(clock());
    // The pane maps to no seat.
    let connection = LazyConnection::new(|| {
        Ok((
            uuid::Uuid::from_u128(1),
            FakeClient::new(|command| match command {
                Command::Seats(_) => Ok(CommandResult::Seats(page(vec![], None))),
                other => panic!("unexpected command {other:?}"),
            }),
        ))
    });
    let mut parsed = parse_argv([
        "herdr-threads",
        "watch",
        "--harness",
        "claude",
        "--session",
        "s1",
    ])
    .unwrap();
    let request = WatchRequest {
        harness: Harness::Claude,
        session: "s1".into(),
    };
    let mut out = Vec::new();
    let result = run_watch(
        &mut parsed,
        &request,
        Some("w1:p2"),
        &runtime,
        &paths,
        &connection,
        &clock,
        &mut out,
    );
    assert!(matches!(result, Err(RunError::Exit(2))), "{result:?}");
    let got = lines(&out);
    assert_eq!(got.len(), 1);
    assert_eq!(
        status_of(&got[0]),
        &WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::NoBinding),
            exit: Some(2),
        }
    );
    // `watch ack` in the same state prints nothing and exits 1.
    let mut parsed = parse_argv([
        "herdr-threads",
        "watch",
        "ack",
        "--session",
        "s1",
        "--via",
        "submit",
        "m1",
    ])
    .unwrap();
    let ack = WatchAckRequest {
        session: "s1".into(),
        via: ModDeliveryVia::Submit,
        messages: vec![MessageId::new("m1")],
    };
    let mut out = Vec::new();
    let result = run_ack(
        &mut parsed,
        &ack,
        Some("w1:p2"),
        &runtime,
        &paths,
        &connection,
        &clock,
        &mut out,
    );
    assert!(matches!(result, Err(RunError::Exit(1))));
    assert!(out.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unreachable_daemon_exits_1_daemon_unavailable() {
    let root = temp_dir("dead");
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(clock());
    let connection: LazyConnection<FakeClient, _> =
        LazyConnection::new(|| Err(crate::cli::daemon_unavailable()));
    let mut parsed = parse_argv([
        "herdr-threads",
        "watch",
        "--harness",
        "claude",
        "--session",
        "s1",
    ])
    .unwrap();
    let request = WatchRequest {
        harness: Harness::Claude,
        session: "s1".into(),
    };
    let mut out = Vec::new();
    let result = run_watch(
        &mut parsed,
        &request,
        Some("w1:p2"),
        &runtime,
        &paths,
        &connection,
        &clock,
        &mut out,
    );
    assert!(matches!(result, Err(RunError::Exit(1))));
    assert_eq!(
        status_of(&lines(&out)[0]).reason,
        Some(WatchStatusReason::DaemonUnavailable)
    );
    std::fs::remove_dir_all(root).unwrap();
}

// ---- drain ----

#[test]
fn drain_pages_until_exhausted_with_schema_kind_and_stable_ids() {
    let batch = |from: usize, count: usize| -> Vec<InboxBatchV2Item> {
        (from..from + count)
            .map(|n| whole(&format!("m{n:03}"), &format!("body {n}")))
            .collect()
    };
    let client = paged_client(vec![batch(0, 32), batch(32, 32), batch(64, 5)]);
    let mut state = EmitState::default();
    let (result, got) = run_drain(&client, 4, &mut state);
    assert!(result.is_ok());
    assert_eq!(got.len(), 69);
    for (n, line) in got.iter().enumerate() {
        assert_eq!(line.schema, 1);
        assert_eq!(line.id, format!("m{n:03}"));
        assert!(matches!(line.item, WatchItem::Message(_)));
        assert_eq!(message_of(line).body, format!("body {n}"));
        assert!(message_of(line).ack_required);
    }
    let calls = client.calls();
    assert_eq!(calls.len(), 3);
    for (index, call) in calls.iter().enumerate() {
        let Command::InboxBatchV2(query) = call else {
            panic!("{call:?}");
        };
        assert_eq!(query.seat, Some(SeatId::new("me")));
        assert_eq!(query.page.limit, 32);
        assert_eq!(query.page.max_bytes, 64 * 1024);
        assert_eq!(
            query.page.cursor,
            (index > 0).then(|| format!("p{index}")),
            "page {index}"
        );
    }
    // A second drain re-reads but prints nothing new: ids stream once per process.
    let (result, again) = run_drain(&paged_client(vec![batch(0, 3)]), 4, &mut state);
    assert!(result.is_ok());
    assert!(again.is_empty());
}

#[test]
fn body_over_8_kib_is_truncated_with_marker_and_hint_file_records_id() {
    let dir = temp_dir("trunc");
    let hint = dir.join("hint.ids");
    let exact = "a".repeat(8192);
    let over = "b".repeat(8193);
    // 8191 ASCII bytes then a 2-byte char straddling the 8192 cut.
    let straddle = format!("{}é{}", "c".repeat(8191), "d");
    assert_eq!(straddle.len(), 8194);
    let client = paged_client(vec![vec![
        whole("exact", &exact),
        whole("over", &over),
        whole("straddle", &straddle),
        lazy("lazy-over", &over),
    ]]);
    let mut state = EmitState::new(Some(hint.clone()));
    let (result, got) = run_drain(&client, 1, &mut state);
    assert!(result.is_ok());
    assert_eq!(got.len(), 4);

    let exact_line = message_of(&got[0]);
    assert!(!exact_line.truncated);
    assert_eq!(exact_line.body, exact);
    assert_eq!(exact_line.body_len, 8192);

    let over_line = message_of(&got[1]);
    assert!(over_line.truncated);
    assert_eq!(over_line.body_len, 8193);
    assert_eq!(
        over_line.body,
        format!(
            "{}{}",
            "b".repeat(8192),
            truncation_marker(&MessageId::new("over"))
        )
    );

    let straddle_line = message_of(&got[2]);
    assert!(straddle_line.truncated);
    assert_eq!(
        straddle_line.body,
        format!(
            "{}{}",
            "c".repeat(8191),
            truncation_marker(&MessageId::new("straddle"))
        )
    );

    assert!(matches!(got[3].item, WatchItem::Lazy(_)));
    assert!(message_of(&got[3]).truncated);

    let hinted = std::fs::read_to_string(&hint).unwrap();
    assert_eq!(
        hinted.lines().collect::<Vec<_>>(),
        ["over", "straddle", "lazy-over"],
        "only truncated ids are hinted"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hint_file_keeps_only_the_newest_1024_ids() {
    let dir = temp_dir("hint");
    let hint = dir.join("hint.ids");
    let state = EmitState::new(Some(hint.clone()));
    for n in 0..1100 {
        record_truncated(&state, &format!("m{n}"));
    }
    record_truncated(&state, "m1099"); // duplicate: no second line
    let text = std::fs::read_to_string(&hint).unwrap();
    let kept: Vec<&str> = text.lines().collect();
    assert_eq!(kept.len(), 1024);
    assert_eq!(kept[0], "m76");
    assert_eq!(kept[1023], "m1099");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn chunked_body_is_reassembled_before_emitting() {
    let client = paged_client(vec![
        vec![message("c1", "ab", 0, 2, 6, true)],
        vec![message("c1", "cd", 2, 4, 6, true)],
        vec![message("c1", "ef", 4, 6, 6, true)],
    ]);
    let mut state = EmitState::default();
    let (result, got) = run_drain(&client, 1, &mut state);
    assert!(result.is_ok());
    assert_eq!(got.len(), 1, "one line for the whole message");
    let line = message_of(&got[0]);
    assert_eq!(line.body, "abcdef");
    assert_eq!(line.body_len, 6);
    assert!(!line.truncated);

    // A long chunked body emits once its first 8 KiB are assembled and skips the rest.
    let client = paged_client(vec![
        vec![message("c2", &"x".repeat(5000), 0, 5000, 10_000, true)],
        vec![message("c2", &"y".repeat(4000), 5000, 9000, 10_000, true)],
        vec![message("c2", &"z".repeat(1000), 9000, 10_000, 10_000, true)],
    ]);
    let (result, got) = run_drain(&client, 1, &mut state);
    assert!(result.is_ok());
    assert_eq!(got.len(), 1);
    let line = message_of(&got[0]);
    assert!(line.truncated);
    assert_eq!(line.body_len, 10_000);
    assert_eq!(
        line.body,
        format!(
            "{}{}{}",
            "x".repeat(5000),
            "y".repeat(3192),
            truncation_marker(&MessageId::new("c2"))
        )
    );

    // A chunk that does not continue the assembled text is a contract break.
    let client = paged_client(vec![vec![
        message("c3", "ab", 0, 2, 6, true),
        message("c3", "ef", 4, 6, 6, true),
    ]]);
    let (result, got) = run_drain(&client, 1, &mut state);
    assert!(matches!(result, Err(DrainError::Protocol)));
    assert!(got.is_empty());
}

#[test]
fn lazy_rows_stream_as_lazy_without_ack_required() {
    let client = paged_client(vec![vec![
        lazy("l1", "lazy body"),
        whole("m1", "ordinary"),
        message("m2", "no candidate", 0, 12, 12, false),
    ]]);
    let mut state = EmitState::default();
    let (_, got) = run_drain(&client, 1, &mut state);
    assert_eq!(got.len(), 3);
    assert!(matches!(got[0].item, WatchItem::Lazy(_)));
    assert!(!message_of(&got[0]).ack_required);
    assert_eq!(got[0].id, "l1");
    assert!(matches!(got[1].item, WatchItem::Message(_)));
    assert!(message_of(&got[1]).ack_required);
    assert!(matches!(got[2].item, WatchItem::Message(_)));
    assert!(!message_of(&got[2]).ack_required, "no ack_candidate");
    assert_eq!(message_of(&got[1]).sender, Some(SeatId::new("sender")));
    assert_eq!(message_of(&got[1]).thread_name.as_deref(), Some("topic"));
    assert_eq!(message_of(&got[1]).sender_name.as_deref(), Some("sender"));
    assert_eq!(message_of(&got[0]).sender_name.as_deref(), Some("service"));
}

#[test]
fn message_lines_carry_the_thread_topic_and_sender_names() {
    let item = |id: &str, topic: &str, sender: Option<&str>| {
        let mut it = whole(id, "body");
        if let InboxBatchV2Item::Message {
            topic_data,
            sender: s,
            ..
        } = &mut it
        {
            *topic_data = topic.into();
            *s = sender.map(SeatId::new);
        }
        it
    };
    let long = "x".repeat(200);
    let client = paged_client(vec![vec![
        item("n1", "release plan\nsecond\u{2028}line", Some("alice")),
        item("n2", &long, None),
        item("n3", "\n\t", Some("alice")),
    ]]);
    let mut state = EmitState::default();
    let (_, got) = run_drain(&client, 1, &mut state);
    assert_eq!(got.len(), 3);
    let text = serde_json::to_string(&got[0]).unwrap();
    assert!(
        text.contains(r#""thread_name":"release plan second line""#),
        "{text}"
    );
    assert!(text.contains(r#""sender_name":"alice""#), "{text}");
    let expected = format!("{}…", "x".repeat(120));
    assert_eq!(
        message_of(&got[1]).thread_name.as_deref(),
        Some(expected.as_str())
    );
    let text = serde_json::to_string(&got[1]).unwrap();
    assert!(text.contains(r#""sender_name":"service""#), "{text}");
    assert_eq!(message_of(&got[2]).thread_name, None);
}

#[test]
fn invitations_and_warnings_collapse_into_one_attention_line_per_version() {
    let items = || vec![invitation(), warning(), invitation(), whole("m1", "hi")];
    let mut state = EmitState::default();
    let (_, first) = run_drain(&paged_client(vec![items()]), 5, &mut state);
    assert_eq!(
        first
            .iter()
            .map(|line| line.id.as_str())
            .collect::<Vec<_>>(),
        ["invitation:invitation-1", "m1", "attention:5"]
    );
    let WatchItem::Invitation(invitation) = &first[0].item else {
        panic!("invitation body missing")
    };
    assert!(invitation.body.contains("goal: goal"));
    assert!(!invitation.truncated);
    assert_eq!(
        first[2].item,
        WatchItem::Attention(crate::protocol::watch::WatchAttention {
            presentation_epoch: None,
            attention_version: 5,
            text: crate::notification::policy::MARKER.to_owned(),
        })
    );
    // Same version again: nothing new. A newer version: one more attention line.
    let (_, same) = run_drain(&paged_client(vec![items()]), 5, &mut state);
    assert!(same.is_empty());
    let (_, newer) = run_drain(&paged_client(vec![items()]), 6, &mut state);
    assert_eq!(newer.len(), 1);
    assert_eq!(newer[0].id, "attention:6");
    // Messages alone never produce an attention line.
    let mut fresh = EmitState::default();
    let (_, plain) = run_drain(&paged_client(vec![vec![whole("m9", "x")]]), 7, &mut fresh);
    assert_eq!(plain.len(), 1);
    assert_eq!(plain[0].id, "m9");
}

#[test]
fn the_attention_line_follows_the_command_prefix() {
    let mut state = EmitState::new(None).with_command_prefix(vec![
        "/opt/ht".into(),
        "--state-dir".into(),
        "/s".into(),
    ]);
    let (_, got) = run_drain(&paged_client(vec![vec![invitation()]]), 5, &mut state);
    assert_eq!(
        got.iter().map(|line| line.id.as_str()).collect::<Vec<_>>(),
        ["invitation:invitation-1", "attention:5"]
    );
    let WatchItem::Invitation(invitation) = &got[0].item else {
        panic!("invitation body missing")
    };
    assert!(invitation
        .body
        .contains("next: /opt/ht --state-dir /s accept t1"));
    assert_eq!(
        got[1].item,
        WatchItem::Attention(crate::protocol::watch::WatchAttention {
            presentation_epoch: None,
            attention_version: 5,
            text: "herdr-threads: attention pending; run /opt/ht --state-dir /s inbox".to_owned(),
        })
    );
}

#[test]
fn a_drain_that_finds_nothing_pending_retracts_the_last_attention_line() {
    let mut state = EmitState::default();
    let ids = |got: &[WatchLine]| got.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
    let (_, got) = run_drain(&paged_client(vec![vec![invitation()]]), 5, &mut state);
    assert_eq!(ids(&got), ["invitation:invitation-1", "attention:5"]);
    let (_, got) = run_drain(&paged_client(vec![vec![whole("m1", "hi")]]), 6, &mut state);
    assert_eq!(ids(&got), ["m1", "attention_cleared:6"]);
    assert_eq!(
        got[1].item,
        WatchItem::AttentionCleared(crate::protocol::watch::WatchAttentionCleared {
            presentation_epoch: None,
            attention_version: 6
        })
    );
    // Already retracted: nothing more.
    let (_, got) = run_drain(&paged_client(vec![vec![]]), 6, &mut state);
    assert!(got.is_empty(), "{got:?}");
    // Attention comes back at a version already seen: printed again.
    let (_, got) = run_drain(&paged_client(vec![vec![warning()]]), 6, &mut state);
    assert_eq!(ids(&got), ["attention:6"]);
    let (_, got) = run_drain(&paged_client(vec![vec![warning()]]), 6, &mut state);
    assert!(got.is_empty(), "{got:?}");
    let (_, got) = run_drain(&paged_client(vec![vec![]]), 7, &mut state);
    assert_eq!(ids(&got), ["attention_cleared:7"]);
}

/// ht-j16.34: a warning the daemon marks informational (another seat's
/// overdue transition, or a clear) raises no attention line; one that wakes
/// the seat still does, and an informational one alone retracts it.
#[test]
fn informational_warnings_raise_no_attention_line() {
    let notice = || InboxBatchV2Item::Warning {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        warning: MessageId::new("w-notice"),
        sequence: 3,
        informational: true,
    };
    let ids = |got: &[WatchLine]| got.iter().map(|l| l.id.clone()).collect::<Vec<_>>();
    let mut state = EmitState::default();
    let (_, got) = run_drain(&paged_client(vec![vec![notice()]]), 4, &mut state);
    assert!(got.is_empty(), "{got:?}");
    let (_, got) = run_drain(
        &paged_client(vec![vec![notice(), warning()]]),
        5,
        &mut state,
    );
    assert_eq!(ids(&got), ["attention:5"]);
    let (_, got) = run_drain(&paged_client(vec![vec![notice()]]), 6, &mut state);
    assert_eq!(ids(&got), ["attention_cleared:6"]);
}

#[test]
fn a_run_without_attention_never_retracts() {
    let mut state = EmitState::default();
    let (_, first) = run_drain(&paged_client(vec![vec![whole("m9", "x")]]), 1, &mut state);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].id, "m9");
    let (_, second) = run_drain(&paged_client(vec![vec![]]), 2, &mut state);
    assert!(second.is_empty(), "{second:?}");
}

#[test]
fn drain_surfaces_daemon_errors_and_closed_stdout() {
    let failing = FakeClient::new(|_| Err(ApiError::host_unavailable("down")));
    let mut state = EmitState::default();
    let (result, got) = run_drain(&failing, 1, &mut state);
    assert!(matches!(result, Err(DrainError::Api)));
    assert!(got.is_empty());

    let client = paged_client(vec![vec![whole("m1", "x")]]);
    let result = drain(
        &client,
        &clock(),
        &SeatId::new("me"),
        None,
        1,
        &mut EmitState::default(),
        &mut ClosedStdout,
    );
    assert!(matches!(result, Err(DrainError::Out)));
}

struct ClosedStdout;

impl Write for ClosedStdout {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---- liveness ----

#[test]
fn parent_death_ends_watch() {
    assert!(parent_alive(500, || 500));
    assert!(!parent_alive(500, || 1), "reparented to init");
    assert!(!parent_alive(500, || 501), "parent pid changed");
    assert!(!parent_alive(1, || 1), "already orphaned at start");
}

// ---- session against a fake watch server ----

fn claim() -> CallerClaim {
    CallerClaim {
        instance: uuid::Uuid::from_u128(1).to_string(),
        seat: SeatId::new("me"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: crate::protocol::authority::Harness::Claude,
        native_session: NativeSessionId::new("sess-1"),
        execution: ExecutionId::new(uuid::Uuid::from_u128(2).to_string()),
        target: HostTargetId::new("w1:p1"),
        agent_id: None,
        liveness: None,
    }
}

#[test]
fn watch_registers_with_its_own_session_claim() {
    use crate::protocol::{authority::LivenessFingerprint, ids::AgentId};
    // The seat context on disk may name another session, a subagent or a stale role: the claim
    // the watch child registers names the `--session` it serves, as a top-level Claude caller.
    let mut stale = claim();
    stale.native_session = NativeSessionId::new("some-other-session");
    stale.role = CallerRole::Subagent;
    stale.agent_id = Some(AgentId::new("agent-7"));
    let own = own_session_claim(stale, "sess-2", Some(LivenessFingerprint::TEST))
        .ok()
        .unwrap();
    assert_eq!(own.native_session, NativeSessionId::new("sess-2"));
    assert_eq!(own.role, CallerRole::TopLevel);
    assert_eq!(own.agent_id, None);
    assert_eq!(own.harness, crate::protocol::authority::Harness::Claude);
    assert_eq!(own.liveness, Some(LivenessFingerprint::TEST));
    assert_eq!(own.seat, SeatId::new("me"), "the seat comes from the pane");
    // Two sessions of one seat register two different binding keys.
    let a = own_session_claim(claim(), "sess-a", None).ok().unwrap();
    let b = own_session_claim(claim(), "sess-b", None).ok().unwrap();
    assert_ne!(
        crate::store::bindings::BindingKey::from_claim(&a),
        crate::store::bindings::BindingKey::from_claim(&b)
    );
    // Without a fresh fingerprint the claim keeps what the context recorded.
    let mut recorded = claim();
    recorded.liveness = Some(LivenessFingerprint::TEST);
    assert_eq!(
        own_session_claim(recorded, "sess-a", None)
            .ok()
            .unwrap()
            .liveness,
        Some(LivenessFingerprint::TEST)
    );
}

fn write_frame(stream: &mut UnixStream, value: &impl serde::Serialize) {
    let body = serde_json::to_vec(value).unwrap();
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    stream.write_all(&framed).unwrap();
}

fn read_request(stream: &mut UnixStream) -> WatchWireRequest {
    WatchWireRequest::decode(&read_raw_request(stream)).unwrap()
}

fn read_raw_request(stream: &mut UnixStream) -> Vec<u8> {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).unwrap();
    let mut body = vec![0u8; u32::from_be_bytes(prefix) as usize];
    stream.read_exact(&mut body).unwrap();
    body
}

fn reply(request: &WatchWireRequest, outcome: WatchOutcome) -> WatchReply {
    WatchReply {
        version: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        instance: request.expected_instance.clone(),
        daemon_boot: uuid::Uuid::from_u128(3).to_string(),
        outcome,
    }
}

fn accepted(version: u64) -> WatchOutcome {
    WatchOutcome::Accepted(crate::protocol::watch::WatchAccepted {
        presentation_lease: None,
        attention_version: version,
    })
}

struct Server {
    dir: PathBuf,
    socket: PathBuf,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Accept one watch connection and hand it to `script` after reading the
    /// registration frame.
    fn start(script: impl FnOnce(&mut UnixStream, WatchWireRequest) + Send + 'static) -> Self {
        let dir = temp_dir("srv");
        let socket = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            script(&mut stream, request);
        });
        Self {
            dir,
            socket,
            handle: Some(handle),
        }
    }

    /// Preserve the actual first-frame bytes before any typed decoder can normalize them.
    fn start_raw(script: impl FnOnce(&mut UnixStream, Vec<u8>) + Send + 'static) -> Self {
        let dir = temp_dir("raw-srv");
        let socket = dir.join("s");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = read_raw_request(&mut stream);
            script(&mut stream, request);
        });
        Self {
            dir,
            socket,
            handle: Some(handle),
        }
    }

    fn finish(mut self) {
        self.handle.take().unwrap().join().unwrap();
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn session<'a>(
    client: &'a FakeClient,
    clock: &'a crate::app::SystemClock,
    socket: &'a Path,
) -> WatchSession<'a> {
    WatchSession {
        client,
        clock,
        socket,
        instance: uuid::Uuid::from_u128(1),
        claim: claim(),
        hint_path: None,
    }
}

fn empty_inbox() -> FakeClient {
    FakeClient::new(|command| match command {
        Command::InboxBatchV2(_) => Ok(CommandResult::InboxBatchV2(page(vec![], None))),
        other => panic!("unexpected command {other:?}"),
    })
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Q3HistoricalWatch {
    claim: CallerClaim,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Q3HistoricalWatchWire {
    version: u16,
    request_id: String,
    expected_instance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_boot: Option<String>,
    watch: Q3HistoricalWatch,
}

#[test]
fn q3_absent_watch_opt_in_preserves_historical_bytes() {
    let historical = serde_json::to_vec(&Q3HistoricalWatch { claim: claim() }).unwrap();
    let decoded: WireWatch = serde_json::from_slice(&historical).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), historical);
    let accepted: crate::protocol::watch::WatchAccepted =
        serde_json::from_str(r#"{"attention_version":7}"#).unwrap();
    assert_eq!(
        serde_json::to_vec(&accepted).unwrap(),
        br#"{"attention_version":7}"#
    );
}

#[test]
fn q3_false_watch_opt_in_normalizes_to_historical_bytes() {
    let historical = serde_json::to_vec(&Q3HistoricalWatch { claim: claim() }).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&historical).unwrap();
    value["presentation_v2"] = serde_json::json!(false);
    let decoded = serde_json::from_value::<WireWatch>(value);
    assert!(decoded.is_ok(), "explicit false must decode: {decoded:?}");
    assert_eq!(serde_json::to_vec(&decoded.unwrap()).unwrap(), historical);
}

#[test]
fn q3_true_watch_opt_in_is_explicit_and_strictly_typed() {
    let mut value = serde_json::json!({"claim": claim(), "presentation_v2": true});
    let decoded = serde_json::from_value::<WireWatch>(value.clone());
    assert!(decoded.is_ok(), "explicit true must decode: {decoded:?}");
    assert_eq!(serde_json::to_value(decoded.unwrap()).unwrap(), value);
    for invalid in [
        serde_json::json!(null),
        serde_json::json!(1),
        serde_json::json!("true"),
    ] {
        value["presentation_v2"] = invalid;
        assert!(serde_json::from_value::<WireWatch>(value.clone()).is_err());
    }
    value["presentation_v2"] = serde_json::json!(true);
    value["unrecognized_mode"] = serde_json::json!(true);
    assert!(serde_json::from_value::<WireWatch>(value.clone()).is_err());
    assert!(
        serde_json::from_value::<Q3HistoricalWatch>(
            serde_json::json!({"claim": claim(), "presentation_v2": true})
        )
        .is_err(),
        "strict old peers really reject the opt-in field"
    );
}

fn q3_negotiated_session(
    presentation: bool,
    cursors: bool,
    channel: bool,
) -> (Vec<u8>, Vec<Command>) {
    let captured = Arc::new(Mutex::new(None));
    let capture = captured.clone();
    let lease = "0123456789abcdef0123456789abcdef";
    let server = Server::start_raw(move |stream, bytes| {
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let request = Q3HistoricalWatchWire {
            version: PROTOCOL_VERSION,
            request_id: value["request_id"].as_str().unwrap().into(),
            expected_instance: value["expected_instance"].as_str().unwrap().into(),
            expected_boot: None,
            watch: Q3HistoricalWatch { claim: claim() },
        };
        *capture.lock().unwrap() = Some(bytes);
        // An unsolicited valid lease must not override a missing capability leg.
        write_frame(
            stream,
            &WatchReply {
                version: request.version,
                request_id: request.request_id,
                instance: request.expected_instance,
                daemon_boot: uuid::Uuid::from_u128(3).to_string(),
                outcome: WatchOutcome::Accepted(crate::protocol::watch::WatchAccepted {
                    presentation_lease: Some(lease.into()),
                    attention_version: 7,
                }),
            },
        );
    });
    let mut client = FakeClient::new(|command| match command {
        Command::InboxBatchV2(_) => Ok(CommandResult::InboxBatchV2(page(vec![], None))),
        Command::DeliveryPullV2(_) => Ok(CommandResult::DeliveryPulledV2(
            crate::protocol::delivery::DeliveryPullReplyV2::Unchanged {
                attention_version: 7,
            },
        )),
        other => panic!("unexpected negotiation command {other:?}"),
    });
    client.presentation_capability = presentation;
    client.cursors_capability = cursors;
    client.watch_presentation_capability = channel;
    let clock = clock();
    let mut out = Vec::new();
    let result = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    server.finish();
    assert_eq!(result, 0);
    assert!(
        lines(&out)
            .iter()
            .all(|line| matches!(line.item, WatchItem::Status(_))),
        "a drain signal or capability must not manufacture model content"
    );
    let bytes = captured.lock().unwrap().take().unwrap();
    (bytes, client.calls())
}

#[test]
fn q3_missing_watch_mode_capability_keeps_old_wire_and_consumer() {
    let (bytes, calls) = q3_negotiated_session(true, true, false);
    let historical: Q3HistoricalWatchWire = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&historical).unwrap(), bytes);
    assert!(
        matches!(calls.as_slice(), [Command::InboxBatchV2(_)]),
        "{calls:?}"
    );
}

#[test]
fn q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer() {
    let (bytes, calls) = q3_negotiated_session(false, true, true);
    let historical: Q3HistoricalWatchWire = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&historical).unwrap(), bytes);
    assert!(
        matches!(calls.as_slice(), [Command::InboxBatchV2(_)]),
        "{calls:?}"
    );
}

#[test]
fn q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer() {
    let (bytes, calls) = q3_negotiated_session(true, false, true);
    let historical: Q3HistoricalWatchWire = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&historical).unwrap(), bytes);
    assert!(
        matches!(calls.as_slice(), [Command::InboxBatchV2(_)]),
        "{calls:?}"
    );
    let Command::InboxBatchV2(query) = &calls[0] else {
        unreachable!()
    };
    assert!(
        query.claim.is_none(),
        "historical seat stream remains available without cursors"
    );
}

#[test]
fn q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease() {
    let (bytes, calls) = q3_negotiated_session(true, true, true);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["watch"]["presentation_v2"], true);
    assert!(serde_json::from_slice::<Q3HistoricalWatchWire>(&bytes).is_err());
    assert!(
        matches!(calls.as_slice(), [Command::DeliveryPullV2(pull)]
        if pull.presentation_lease.as_deref() == Some("0123456789abcdef0123456789abcdef")),
        "{calls:?}"
    );
}

#[test]
fn q3_watch_discovery_failure_never_opens_a_registration_socket() {
    let dir = temp_dir("probe-failure");
    let socket = dir.join("s");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    for probe in [
        Err(ApiError::host_unavailable("lost")),
        Ok(ApiError::store_busy("busy")),
    ] {
        let mut client = empty_inbox();
        client.presentation_capability = true;
        client.cursors_capability = true;
        client.watch_presentation_capability = true;
        client.probe = Some(probe);
        let clock = clock();
        let mut out = Vec::new();
        assert_eq!(
            run_session(session(&client, &clock, &socket), &mut out, || true),
            1
        );
        assert_eq!(
            status_of(&lines(&out)[0]).reason,
            Some(WatchStatusReason::DaemonUnavailable)
        );
        assert!(client.calls().is_empty());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    drop(listener);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn q3_missing_mod_watch_capability_never_opens_a_registration_socket() {
    let dir = temp_dir("probe-unsupported");
    let socket = dir.join("s");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut client = empty_inbox();
    client.watch_capability = false;
    client.presentation_capability = true;
    client.cursors_capability = true;
    client.watch_presentation_capability = true;
    let clock = clock();
    let mut out = Vec::new();
    assert_eq!(
        run_session(session(&client, &clock, &socket), &mut out, || true),
        3
    );
    assert_eq!(
        status_of(&lines(&out)[0]).reason,
        Some(WatchStatusReason::Unsupported)
    );
    assert!(client.calls().is_empty());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(listener);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refusal_prints_status_line_and_exits_with_contract_code() {
    use WatchRefusalReason::*;
    let table = [
        (NoBinding, 2),
        (SessionMismatch, 2),
        (Held, 2),
        (Unresolved, 2),
        (Cooldown, 2),
        (Busy, 2),
        (Stopping, 2),
        (NotClaude, 3),
        (Disabled, 3),
    ];
    for (reason, code) in table {
        let server = Server::start(move |stream, request| {
            // The registration carries the claim, with the --session native session.
            assert_eq!(request.watch.claim.native_session.as_str(), "sess-1");
            assert_eq!(request.watch.claim.seat.as_str(), "me");
            assert_eq!(request.version, PROTOCOL_VERSION);
            assert!(request.expected_boot.is_none());
            write_frame(
                stream,
                &reply(
                    &request,
                    WatchOutcome::Refused(WatchRefusal {
                        reason,
                        detail: None,
                    }),
                ),
            );
        });
        let client = empty_inbox();
        let clock = clock();
        let mut out = Vec::new();
        let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
        assert_eq!(exit, code, "{reason:?}");
        let got = lines(&out);
        assert_eq!(got.len(), 1, "{reason:?}: {got:?}");
        assert_eq!(got[0].id, "status:0");
        assert_eq!(
            status_of(&got[0]),
            &WatchStatus {
                state: WatchStatusState::Refused,
                reason: Some(WatchStatusReason::from(reason)),
                exit: Some(code),
            },
            "{reason:?}"
        );
        assert!(client.calls().is_empty(), "a refused watch drains nothing");
        server.finish();
    }
}

#[test]
fn close_frame_prints_closing_status_and_exits_with_close_code() {
    let table = [
        (WatchCloseReason::BindingChanged, 0),
        (WatchCloseReason::Replaced, 3),
        (WatchCloseReason::Disabled, 3),
        (WatchCloseReason::Stalled, 0),
        (WatchCloseReason::Retired, 0),
        (WatchCloseReason::Unresolved, 0),
        (WatchCloseReason::Stopping, 0),
    ];
    for (reason, code) in table {
        let server = Server::start(move |stream, request| {
            write_frame(stream, &reply(&request, accepted(1)));
            write_frame(stream, &WatchFrame::Close { reason });
        });
        let client = empty_inbox();
        let clock = clock();
        let mut out = Vec::new();
        let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
        assert_eq!(exit, code, "{reason:?}");
        let got = lines(&out);
        assert_eq!(got.len(), 2, "{reason:?}: {got:?}");
        assert_eq!(
            status_of(&got[0]),
            &WatchStatus {
                state: WatchStatusState::Connected,
                reason: None,
                exit: None
            }
        );
        assert_eq!(got[1].id, "status:1");
        assert_eq!(
            status_of(&got[1]),
            &WatchStatus {
                state: WatchStatusState::Closing,
                reason: Some(reason.into()),
                exit: Some(code),
            }
        );
        server.finish();
    }
}

#[test]
fn attention_frame_triggers_another_drain() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
        write_frame(stream, &WatchFrame::Attention { version: 2 });
        write_frame(
            stream,
            &WatchFrame::Close {
                reason: WatchCloseReason::Stalled,
            },
        );
    });
    // First drain: m1. Second drain (after Attention): m1 again plus m2.
    let round = Mutex::new(0usize);
    let client = FakeClient::new(move |command| {
        assert!(matches!(command, Command::InboxBatchV2(_)));
        let mut round = round.lock().unwrap();
        *round += 1;
        let items = if *round == 1 {
            vec![whole("m1", "one")]
        } else {
            vec![whole("m1", "one"), whole("m2", "two")]
        };
        Ok(CommandResult::InboxBatchV2(page(items, None)))
    });
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    assert_eq!(exit, 0);
    assert_eq!(
        client.calls().len(),
        2,
        "one drain after Accepted, one per Attention"
    );
    let ids: Vec<String> = lines(&out).into_iter().map(|line| line.id).collect();
    assert_eq!(ids, ["status:0", "m1", "m2", "status:1"]);
    server.finish();
}

#[test]
fn stream_eof_prints_stream_ended_and_exits_0() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
        // Dropping the stream ends it without a Close frame (daemon restart).
    });
    let client = empty_inbox();
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    assert_eq!(exit, 0);
    let got = lines(&out);
    assert_eq!(got.len(), 2);
    assert_eq!(
        status_of(&got[1]),
        &WatchStatus {
            state: WatchStatusState::Closing,
            reason: Some(WatchStatusReason::StreamEnded),
            exit: Some(0),
        }
    );
    server.finish();
}

#[test]
fn stdout_closed_exits_0_quietly() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
        // Hold the stream until the client leaves.
        let mut sink = [0u8; 1];
        let _ = stream.read(&mut sink);
    });
    let client = empty_inbox();
    let clock = clock();
    let exit = run_session(
        session(&client, &clock, &server.socket),
        &mut ClosedStdout,
        || true,
    );
    assert_eq!(exit, 0);
    server.finish();
    // A refusal into a closed stdout is quiet too.
    let exit = run_session(
        WatchSession {
            socket: Path::new("/nonexistent/htw.sock"),
            ..session(&client, &clock, Path::new("/nonexistent/htw.sock"))
        },
        &mut ClosedStdout,
        || true,
    );
    assert_eq!(exit, 0);
}

#[test]
fn dead_parent_stops_the_stream_loop_without_a_status_line() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
        let mut sink = [0u8; 1];
        let _ = stream.read(&mut sink); // returns when the client closes
    });
    let client = empty_inbox();
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || false);
    assert_eq!(exit, 0);
    let got = lines(&out);
    assert_eq!(
        got.len(),
        1,
        "only `connected`; nothing after the parent died"
    );
    server.finish();
}

/// Runs `run_session` against a client whose capability probe is rigged and
/// returns the exit code and the single status line it printed.
fn run_probe(client: &FakeClient) -> (i32, WatchStatus) {
    let clock = clock();
    let mut out = Vec::new();
    // No server exists: the capability check precedes any connection.
    let exit = run_session(
        session(client, &clock, Path::new("/nonexistent/htw.sock")),
        &mut out,
        || true,
    );
    let got = lines(&out);
    assert_eq!(got.len(), 1, "exactly one status line: {got:?}");
    (exit, status_of(&got[0]).clone())
}

#[test]
fn failed_capability_probe_exits_retryable_daemon_unavailable() {
    let mut client = empty_inbox();
    client.probe = Some(Err(ApiError::host_unavailable("daemon booting")));
    let (exit, status) = run_probe(&client);
    let retryable = WatchStatusReason::DaemonUnavailable.exit_code();
    assert_eq!(exit, retryable);
    assert_ne!(exit, 3, "exit 3 stops the mod for the whole session");
    assert_eq!(
        status,
        WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::DaemonUnavailable),
            exit: Some(retryable),
        }
    );
}

#[test]
fn busy_or_timed_out_probe_is_retryable() {
    for code in [
        ErrorCode::StoreBusy,
        ErrorCode::ServiceBusy,
        ErrorCode::DeadlineExceeded,
        ErrorCode::DaemonBootChanged,
    ] {
        let mut client = empty_inbox();
        client.probe = Some(Ok(ApiError::new(code.clone(), "probe rejected")));
        let (exit, status) = run_probe(&client);
        assert_ne!(exit, 3, "{code:?} must not stop the mod");
        assert_eq!(exit, WatchStatusReason::DaemonUnavailable.exit_code());
        assert_eq!(
            status.reason,
            Some(WatchStatusReason::DaemonUnavailable),
            "{code:?}"
        );
    }
}

#[test]
fn definitive_unknown_wire_version_probe_exits_3_unsupported() {
    let mut client = empty_inbox();
    client.probe = Some(Ok(ApiError::new(
        ErrorCode::UnknownWireVersion,
        "older daemon",
    )));
    let (exit, status) = run_probe(&client);
    assert_eq!(exit, 3);
    assert_eq!(status.reason, Some(WatchStatusReason::Unsupported));
}

#[test]
fn missing_mod_watch_capability_exits_3_unsupported() {
    let mut client = empty_inbox();
    client.watch_capability = false;
    let clock = clock();
    let mut out = Vec::new();
    // No server exists: the capability check precedes any connection.
    let exit = run_session(
        session(&client, &clock, Path::new("/nonexistent/htw.sock")),
        &mut out,
        || true,
    );
    assert_eq!(exit, 3);
    let got = lines(&out);
    assert_eq!(got.len(), 1);
    assert_eq!(
        status_of(&got[0]),
        &WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::Unsupported),
            exit: Some(3),
        }
    );
}

#[test]
fn unknown_wire_version_reply_exits_3_unsupported() {
    let server = Server::start(|stream, request| {
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id.clone(),
            instance: request.expected_instance.clone(),
            daemon_boot: uuid::Uuid::from_u128(3).to_string(),
            result: Err(ApiError::new(ErrorCode::UnknownWireVersion, "old")),
        };
        write_frame(stream, &response);
    });
    let client = empty_inbox();
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    assert_eq!(exit, 3);
    assert_eq!(
        status_of(&lines(&out)[0]).reason,
        Some(WatchStatusReason::Unsupported)
    );
    server.finish();
}

#[test]
fn unreachable_socket_exits_1_daemon_unavailable() {
    let client = empty_inbox();
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(
        session(&client, &clock, Path::new("/nonexistent/htw.sock")),
        &mut out,
        || true,
    );
    assert_eq!(exit, 1);
    assert_eq!(
        status_of(&lines(&out)[0]).reason,
        Some(WatchStatusReason::DaemonUnavailable)
    );
}

// ---- watch ack ----

fn ack_request(ids: &[&str]) -> WatchAckRequest {
    WatchAckRequest {
        session: "sess-1".into(),
        via: ModDeliveryVia::Context,
        messages: ids.iter().map(|id| MessageId::new(*id)).collect(),
    }
}

fn settled(id: &str) -> ModAckItem {
    ModAckItem {
        id: MessageId::new(id),
        result: ModAckOutcome::Settled,
        reason: None,
    }
}

fn ack_lines(bytes: &[u8]) -> Vec<ModAckItem> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn watch_ack_prints_one_line_per_id_and_exits_0() {
    let client = FakeClient::new(|command| {
        let Command::AckModDelivered(ack) = command else {
            panic!("{command:?}");
        };
        Ok(CommandResult::ModDeliveryAcked(ModAckReport {
            results: ack
                .messages
                .iter()
                .enumerate()
                .map(|(n, id)| ModAckItem {
                    id: id.clone(),
                    result: if n == 0 {
                        ModAckOutcome::Settled
                    } else {
                        ModAckOutcome::Retryable
                    },
                    reason: (n != 0).then_some(ModAckReason::Busy),
                })
                .collect(),
        }))
    });
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request(&["m1", "m2"]),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 0);
    assert_eq!(
        ack_lines(&out),
        vec![
            settled("m1"),
            ModAckItem {
                id: MessageId::new("m2"),
                result: ModAckOutcome::Retryable,
                reason: Some(ModAckReason::Busy),
            }
        ]
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 1);
    let Command::AckModDelivered(sent) = &calls[0] else {
        panic!();
    };
    assert_eq!(sent.via, ModDeliveryVia::Context);
    assert_eq!(sent.messages, [MessageId::new("m1"), MessageId::new("m2")]);
    assert_eq!(sent.claim, claim());
    assert!(uuid::Uuid::parse_str(sent.operation.as_str()).is_ok());
}

#[test]
fn watch_ack_refuses_hinted_truncated_ids_locally_and_sends_the_rest() {
    let dir = temp_dir("ack");
    let hint = dir.join("hint.ids");
    std::fs::write(&hint, "m2\nunrelated\n").unwrap();
    let client = FakeClient::new(|command| {
        let Command::AckModDelivered(ack) = command else {
            panic!("{command:?}");
        };
        Ok(CommandResult::ModDeliveryAcked(ModAckReport {
            results: ack.messages.iter().map(|id| settled(id.as_str())).collect(),
        }))
    });
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request(&["m1", "m2", "m3"]),
        claim(),
        Some(&hint),
        &mut out,
    );
    assert_eq!(exit, 0);
    assert_eq!(
        ack_lines(&out),
        vec![
            ModAckItem {
                id: MessageId::new("m2"),
                result: ModAckOutcome::RefusedTerminal,
                reason: Some(ModAckReason::Truncated),
            },
            settled("m1"),
            settled("m3"),
        ]
    );
    let Command::AckModDelivered(sent) = &client.calls()[0] else {
        panic!();
    };
    assert_eq!(sent.messages, [MessageId::new("m1"), MessageId::new("m3")]);
    // All ids hinted: nothing is sent at all.
    let idle = FakeClient::new(|command| panic!("unexpected {command:?}"));
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &idle,
        &clock(),
        &ack_request(&["m2"]),
        claim(),
        Some(&hint),
        &mut out,
    );
    assert_eq!(exit, 0);
    assert_eq!(ack_lines(&out).len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn watch_ack_error_prints_nothing_and_exits_1() {
    let client = FakeClient::new(|_| Err(ApiError::host_unavailable("daemon went away")));
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request(&["m1", "m2"]),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 1);
    assert!(out.is_empty());

    // A report that misses a requested id is incomplete: exit 1 (the mod retries that id).
    let partial = FakeClient::new(|_| {
        Ok(CommandResult::ModDeliveryAcked(ModAckReport {
            results: vec![settled("m1")],
        }))
    });
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &partial,
        &clock(),
        &ack_request(&["m1", "m2"]),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 1);
    assert_eq!(ack_lines(&out), vec![settled("m1")]);
}

// ---- watch ack chunking (daemon batch limit) ----

fn many_ids(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("m{n}")).collect()
}

fn ack_request_for(ids: &[String]) -> WatchAckRequest {
    WatchAckRequest {
        session: "sess-1".into(),
        via: ModDeliveryVia::Context,
        messages: ids.iter().map(|id| MessageId::new(id.as_str())).collect(),
    }
}

/// Answers like the daemon: a batch over `MAX_BATCH_ITEMS` is rejected whole,
/// anything else settles. `fail` decides per call number (0-based) whether
/// the call fails with the given error instead.
fn limit_enforcing(fail: impl Fn(usize) -> Option<ApiError> + Send + Sync + 'static) -> FakeClient {
    let seen = std::sync::atomic::AtomicUsize::new(0);
    FakeClient::new(move |command| {
        let Command::AckModDelivered(ack) = command else {
            panic!("{command:?}");
        };
        let call = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if ack.messages.len() > crate::protocol::commands::MAX_BATCH_ITEMS {
            return Err(ApiError::invalid_request("too many messages in one batch"));
        }
        if let Some(error) = fail(call) {
            return Err(error);
        }
        Ok(CommandResult::ModDeliveryAcked(ModAckReport {
            results: ack.messages.iter().map(|id| settled(id.as_str())).collect(),
        }))
    })
}

fn sent_acks(client: &FakeClient) -> Vec<AckModDelivered> {
    client
        .calls()
        .into_iter()
        .map(|command| {
            let Command::AckModDelivered(ack) = command else {
                panic!("{command:?}");
            };
            ack
        })
        .collect()
}

#[test]
fn watch_ack_over_the_batch_limit_sends_chunks_and_settles_every_id() {
    let ids = many_ids(250);
    let client = limit_enforcing(|_| None);
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request_for(&ids),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 0);
    let got = ack_lines(&out);
    assert_eq!(got.len(), 250);
    assert!(
        got.iter()
            .zip(&ids)
            .all(|(item, id)| item == &settled(id.as_str())),
        "one settled line per id, in request order"
    );
    let sent = sent_acks(&client);
    assert_eq!(
        sent.iter()
            .map(|ack| ack.messages.len())
            .collect::<Vec<_>>(),
        [100, 100, 50]
    );
    let operations: HashSet<&str> = sent.iter().map(|ack| ack.operation.as_str()).collect();
    assert_eq!(operations.len(), 3, "each chunk has its own operation id");
    assert!(sent
        .iter()
        .all(|ack| ack.via == ModDeliveryVia::Context && ack.claim == claim()));
}

#[test]
fn exactly_the_batch_limit_is_one_call() {
    let ids = many_ids(100);
    let client = limit_enforcing(|_| None);
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request_for(&ids),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 0);
    assert_eq!(ack_lines(&out).len(), 100);
    assert_eq!(sent_acks(&client).len(), 1);
}

#[test]
fn one_over_is_two_calls() {
    let ids = many_ids(101);
    let client = limit_enforcing(|_| None);
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request_for(&ids),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 0);
    assert_eq!(ack_lines(&out).len(), 101);
    assert_eq!(
        sent_acks(&client)
            .iter()
            .map(|ack| ack.messages.len())
            .collect::<Vec<_>>(),
        [100, 1]
    );
}

#[test]
fn a_failed_chunk_marks_only_its_ids_retryable() {
    for (error, reason) in [
        (
            ApiError::host_unavailable("daemon went away"),
            ModAckReason::Unreachable,
        ),
        (ApiError::store_busy("locked"), ModAckReason::Busy),
        (ApiError::service_busy("queue full"), ModAckReason::Busy),
    ] {
        let ids = many_ids(250);
        let failure = error.clone();
        let client = limit_enforcing(move |call| (call == 1).then(|| failure.clone()));
        let mut out = Vec::new();
        let exit = run_ack_lines(
            &client,
            &clock(),
            &ack_request_for(&ids),
            claim(),
            None,
            &mut out,
        );
        assert_eq!(exit, 0, "{error:?}: every id has a line");
        let got = ack_lines(&out);
        assert_eq!(got.len(), 250);
        for (n, item) in got.iter().enumerate() {
            assert_eq!(item.id.as_str(), ids[n]);
            if (100..200).contains(&n) {
                assert_eq!(item.result, ModAckOutcome::Retryable, "{:?}", item.id);
                assert_eq!(item.reason, Some(reason), "{:?}", item.id);
            } else {
                assert_eq!(item, &settled(&ids[n]));
            }
        }
    }
}

#[test]
fn every_chunk_failing_prints_nothing_and_exits_1() {
    let ids = many_ids(150);
    let client = limit_enforcing(|_| Some(ApiError::host_unavailable("daemon went away")));
    let mut out = Vec::new();
    let exit = run_ack_lines(
        &client,
        &clock(),
        &ack_request_for(&ids),
        claim(),
        None,
        &mut out,
    );
    assert_eq!(exit, 1);
    assert!(out.is_empty());
    assert_eq!(sent_acks(&client).len(), 2);
}

#[test]
fn invitation_policy_lazy_is_bookkeeping_while_timed_and_legacy_raise_attention() {
    use crate::protocol::commands::InvitationConfirmation::{Lazy, RequireAck};
    for (mode, waking) in [(Some(Lazy), false), (Some(RequireAck), true), (None, true)] {
        let mut item = invitation();
        if let InboxBatchV2Item::Invitation { confirmation, .. } = &mut item {
            *confirmation = mode;
        }
        let classified = crate::delivery::assemble::Assembler::default()
            .push(item.clone())
            .unwrap();
        assert!(matches!(classified,
            crate::delivery::assemble::Assembled::Invitation(ref item) if item.content.wakes == waking));
        let mut state = EmitState::default();
        let (_, lines) = run_drain(&paged_client(vec![vec![item]]), 5, &mut state);
        assert_eq!(lines.len(), 1 + usize::from(waking), "{mode:?}: {lines:?}");
        if waking {
            assert_eq!(lines[1].id, "attention:5");
        }
    }
}

// Kills: dropping a Lazy invitation from the actual watch consumer output.
#[test]
fn watch_carries_a_passive_invitation_as_invitation_content_never_a_message() {
    let mut item = invitation();
    if let InboxBatchV2Item::Invitation {
        confirmation,
        topic_data,
        ..
    } = &mut item
    {
        *confirmation = Some(crate::protocol::commands::InvitationConfirmation::Lazy);
        *topic_data = "review the passive handoff".into();
    }
    let mut state = EmitState::default();
    let (_, lines) = run_drain(&paged_client(vec![vec![item]]), 5, &mut state);
    assert_eq!(
        lines.len(),
        1,
        "the passive pending item reaches the channel"
    );
    let value = serde_json::to_value(&lines[0]).unwrap();
    assert_eq!(value["kind"], "invitation");
    assert_eq!(value["wakes"], false);
    assert!(value["body"]
        .as_str()
        .unwrap()
        .contains("review the passive handoff"));
    assert!(value["body"].as_str().unwrap().contains("accept"));
    assert!(value.get("ack_required").is_none(), "not a message receipt");
}

/// Either missing negotiation leg must retain the historical consumer.
#[test]
fn presentation_watch_requires_both_capability_and_valid_accepted_lease() {
    for (capability, lease) in [
        (false, Some("0123456789abcdef0123456789abcdef")),
        (true, None),
        (true, Some("not-a-presentation-lease")),
    ] {
        let server = Server::start(move |stream, request| {
            write_frame(
                stream,
                &reply(
                    &request,
                    WatchOutcome::Accepted(crate::protocol::watch::WatchAccepted {
                        attention_version: 7,
                        presentation_lease: lease.map(str::to_owned),
                    }),
                ),
            );
        });
        let mut client = FakeClient::new(|command| match command {
            Command::InboxBatchV2(_) => Ok(CommandResult::InboxBatchV2(page(
                vec![whole("historical-item", "historical body")],
                None,
            ))),
            other => panic!("inactive negotiation issued {other:?}"),
        });
        client.presentation_capability = capability;
        client.cursors_capability = true;
        let clock = clock();
        let mut out = Vec::new();
        assert_eq!(
            run_session(session(&client, &clock, &server.socket), &mut out, || true),
            0
        );
        assert_eq!(
            lines(&out)
                .into_iter()
                .map(|line| line.id)
                .collect::<Vec<_>>(),
            ["status:0", "historical-item", "status:1"],
            "capability={capability}, lease={lease:?}"
        );
        assert!(matches!(
            client.calls().as_slice(),
            [Command::InboxBatchV2(_)]
        ));
        assert_eq!(inbox_claims(&client), [Some(claim())]);
        server.finish();
    }
}

#[test]
fn capable_watch_streams_ordered_v2_then_explicitly_confirms_captured_controls() {
    use crate::protocol::delivery::{
        DeliveryPageV2, DeliveryPresentationConfirmationReplyV2,
        DeliveryPresentationConfirmationV2, DeliveryPullReplyV2, PresentationItemV2,
        PresentationRenderSelection,
    };
    let lease = "0123456789abcdef0123456789abcdef";
    let server = Server::start(move |stream, request| {
        write_frame(
            stream,
            &reply(
                &request,
                WatchOutcome::Accepted(crate::protocol::watch::WatchAccepted {
                    presentation_lease: Some(lease.into()),
                    attention_version: 7,
                }),
            ),
        );
    });
    let item = PresentationItemV2::Notice {
        warning: MessageId::new("watch-v2-notice"),
        thread: ThreadId::new("watch-v2-thread"),
        body: "canonical notice".into(),
    };
    let bytes = crate::delivery::presentation::measure(
        &crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
        std::slice::from_ref(&item),
    )
    .unwrap()
    .1;
    let mut client = FakeClient::new(move |command| match command {
        Command::InboxBatchV2(_) => Ok(CommandResult::InboxBatchV2(page(vec![], None))),
        Command::DeliveryPullV2(pull) => {
            assert_eq!(pull.claim, claim());
            assert_eq!(pull.presentation_lease.as_deref(), Some(lease));
            Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(
                DeliveryPageV2 {
                    attention_version: 7,
                    text: String::new(),
                    offered: vec![],
                    truncated: vec![],
                    more: false,
                    presentation_token: Some("11111111111111111111111111111111".into()),
                    render: PresentationRenderSelection::WatchJsonLinesV1,
                    output_bytes: bytes as u32,
                    items: vec![item.clone()],
                    emitted_controls: vec![],
                },
            )))
        }
        Command::DeliveryPresentationConfirmV2(confirm) => {
            assert_eq!(confirm.claim, claim());
            assert!(confirm.emitted_controls.is_empty());
            assert_eq!(confirm.presentation_lease.as_deref(), Some(lease));
            Ok(CommandResult::DeliveryPresentationConfirmedV2(
                DeliveryPresentationConfirmationReplyV2::empty(
                    DeliveryPresentationConfirmationV2::Confirmed,
                ),
            ))
        }
        other => panic!("unexpected {other:?}"),
    });
    client.presentation_capability = true;
    client.cursors_capability = true;
    let clock = clock();
    let mut out = Vec::new();
    assert_eq!(
        run_session(session(&client, &clock, &server.socket), &mut out, || true),
        0
    );
    let calls = client.calls();
    assert!(
        matches!(
            calls.as_slice(),
            [
                Command::DeliveryPullV2(_),
                Command::DeliveryPresentationConfirmV2(_)
            ]
        ),
        "capable watch must use explicit V2 emission, got {calls:?}"
    );
    let ids = lines(&out)
        .into_iter()
        .map(|line| line.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["status:0", "notice:watch-v2-notice", "status:1"]);
    server.finish();
}

struct PresentationTestConn(std::collections::VecDeque<WatchFrame>);
impl WatchConn for PresentationTestConn {
    fn next_frame(&mut self, _: std::time::Duration) -> std::io::Result<Option<WatchFrame>> {
        Ok(self.0.pop_front())
    }
}
fn presentation_notice_page(more: bool) -> crate::protocol::delivery::DeliveryPageV2 {
    use crate::protocol::delivery::{
        DeliveryPageV2, PresentationItemV2, PresentationRenderSelection,
    };
    let items = vec![PresentationItemV2::Notice {
        warning: MessageId::new("watch-v2-notice"),
        thread: ThreadId::new("watch-v2-thread"),
        body: "canonical notice".into(),
    }];
    let size = crate::delivery::presentation::measure(
        &crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
        &items,
    )
    .unwrap()
    .1;
    DeliveryPageV2 {
        attention_version: 1,
        text: String::new(),
        offered: vec![],
        truncated: vec![],
        more,
        presentation_token: Some("11111111111111111111111111111111".into()),
        render: PresentationRenderSelection::WatchJsonLinesV1,
        output_bytes: size as u32,
        items,
        emitted_controls: vec![],
    }
}
#[test]
fn presentation_watch_failed_flush_never_confirms_or_writes_success_hint() {
    struct FailSecondFlush {
        flushes: usize,
        writes: usize,
        fail_write: bool,
    }
    impl std::io::Write for FailSecondFlush {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.fail_write && self.writes == 2 {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            if !self.fail_write && self.flushes == 2 {
                Err(std::io::ErrorKind::BrokenPipe.into())
            } else {
                Ok(())
            }
        }
    }
    for fail_write in [false, true] {
        let client = FakeClient::new(|command| match command {
            Command::DeliveryPullV2(_) => Ok(CommandResult::DeliveryPulledV2(
                crate::protocol::delivery::DeliveryPullReplyV2::Page(presentation_notice_page(
                    false,
                )),
            )),
            other => panic!("failed emission must not confirm {other:?}"),
        });
        let mut conn = PresentationTestConn(Default::default());
        let clock = clock();
        let mut state = EmitState::default();
        let mut out = FailSecondFlush {
            flushes: 0,
            writes: 0,
            fail_write,
        };
        assert_eq!(
            watch_presentation_loop(
                &mut conn,
                &client,
                &clock,
                &claim(),
                "0123456789abcdef0123456789abcdef",
                &mut state,
                &mut out,
                &mut || true
            ),
            0
        );
        assert!(matches!(
            client.calls().as_slice(),
            [Command::DeliveryPullV2(_)]
        ));
        assert_eq!(out.flushes, if fail_write { 1 } else { 2 });
    }
}
#[test]
fn presentation_watch_close_wins_before_next_more_quantum() {
    let client = FakeClient::new(|command| match command {
        Command::DeliveryPullV2(_) => Ok(CommandResult::DeliveryPulledV2(
            crate::protocol::delivery::DeliveryPullReplyV2::Page(presentation_notice_page(true)),
        )),
        Command::DeliveryPresentationConfirmV2(confirm) => {
            assert!(confirm.emitted_controls.is_empty());
            Ok(CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2::empty(
                    crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed,
                ),
            ))
        }
        other => panic!("{other:?}"),
    });
    let mut conn = PresentationTestConn(
        [WatchFrame::Close {
            reason: WatchCloseReason::Replaced,
        }]
        .into(),
    );
    let clock = clock();
    let mut state = EmitState::default();
    let mut out = Vec::new();
    assert_eq!(
        watch_presentation_loop(
            &mut conn,
            &client,
            &clock,
            &claim(),
            "0123456789abcdef0123456789abcdef",
            &mut state,
            &mut out,
            &mut || true
        ),
        3
    );
    assert!(
        matches!(
            client.calls().as_slice(),
            [
                Command::DeliveryPullV2(_),
                Command::DeliveryPresentationConfirmV2(_)
            ]
        ),
        "no second quantum after replacement"
    );
    assert_eq!(
        status_of(lines(&out).last().unwrap()).reason,
        Some(WatchStatusReason::Replaced)
    );
}
#[test]
fn presentation_watch_foreign_acknowledgement_is_error_not_success() {
    let client = FakeClient::new(|command| match command {
        Command::DeliveryPullV2(_) => Ok(CommandResult::DeliveryPulledV2(
            crate::protocol::delivery::DeliveryPullReplyV2::Page(presentation_notice_page(true)),
        )),
        Command::DeliveryPresentationConfirmV2(confirm) => {
            assert!(confirm.emitted_controls.is_empty());
            Ok(CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                    status:
                        crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed,
                    acknowledged_controls: vec![
                        crate::protocol::delivery::PresentationControlStampV2::Attention {
                            epoch: "1".into(),
                            version: 1,
                        },
                    ],
                },
            ))
        }
        other => panic!("{other:?}"),
    });
    let mut conn = PresentationTestConn(Default::default());
    let clock = clock();
    let mut state = EmitState::default();
    let mut out = Vec::new();
    assert_eq!(
        watch_presentation_loop(
            &mut conn,
            &client,
            &clock,
            &claim(),
            "0123456789abcdef0123456789abcdef",
            &mut state,
            &mut out,
            &mut || true
        ),
        1
    );
    assert!(matches!(
        client.calls().as_slice(),
        [
            Command::DeliveryPullV2(_),
            Command::DeliveryPresentationConfirmV2(_)
        ]
    ));
    assert_eq!(
        status_of(lines(&out).last().unwrap()).reason,
        Some(WatchStatusReason::Error)
    );
}

#[test]
fn presentation_watch_refused_confirmation_never_records_truncation_hint() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmationReplyV2 as Reply,
        DeliveryPresentationConfirmationV2 as Status, PresentationItemV2,
    };
    for refusal in [
        None,
        Some(Status::StaleBinding),
        Some(Status::NotCurrentPage),
    ] {
        let dir = temp_dir("presentation-refused-hint");
        let hint = dir.join("hint");
        let client = FakeClient::new(move |command| match command {
            Command::DeliveryPullV2(_) => {
                let mut p = presentation_notice_page(true);
                let assembled = Assembler::default()
                    .push(message(
                        "refused-trunc",
                        &"x".repeat(8192),
                        0,
                        8192,
                        9000,
                        true,
                    ))
                    .unwrap();
                let crate::delivery::Assembled::Item(item) = assembled else {
                    panic!("supported truncated message")
                };
                p.items.push(PresentationItemV2::Message {
                    id: item.id.clone(),
                    lazy: false,
                    content: item.message,
                });
                p.truncated.push(item.id);
                p.output_bytes = crate::delivery::presentation::measure(
                    &crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                    &p.items,
                )
                .unwrap()
                .1 as u32;
                Ok(CommandResult::DeliveryPulledV2(
                    crate::protocol::delivery::DeliveryPullReplyV2::Page(p),
                ))
            }
            Command::DeliveryPresentationConfirmV2(_) => match refusal {
                Some(status) => Ok(CommandResult::DeliveryPresentationConfirmedV2(
                    Reply::empty(status),
                )),
                None => Err(ApiError::unsupported("lost confirmation response")),
            },
            other => panic!("{other:?}"),
        });
        let mut conn = PresentationTestConn(Default::default());
        let clock = clock();
        let mut state = EmitState::new(Some(hint.clone()));
        let mut out = Vec::new();
        assert_eq!(
            watch_presentation_loop(
                &mut conn,
                &client,
                &clock,
                &claim(),
                "0123456789abcdef0123456789abcdef",
                &mut state,
                &mut out,
                &mut || true
            ),
            1
        );
        assert!(
            !hint.exists(),
            "failed/stale confirmation cannot write success hint"
        );
        assert_eq!(client.calls().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[test]
fn presentation_watch_new_owner_physically_reemits_before_own_confirmation() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmationReplyV2 as Reply,
        DeliveryPresentationConfirmationV2 as Status,
    };
    let client = FakeClient::new(|command| match command {
        Command::DeliveryPullV2(_) => Ok(CommandResult::DeliveryPulledV2(
            crate::protocol::delivery::DeliveryPullReplyV2::Page(presentation_notice_page(false)),
        )),
        Command::DeliveryPresentationConfirmV2(_) => Ok(
            CommandResult::DeliveryPresentationConfirmedV2(Reply::empty(Status::Confirmed)),
        ),
        other => panic!("{other:?}"),
    });
    let clock = clock();
    for lease in [
        "0123456789abcdef0123456789abcdef",
        "22222222222222222222222222222222",
    ] {
        let mut conn = PresentationTestConn(Default::default());
        let mut state = EmitState::default();
        let mut out = Vec::new();
        assert_eq!(
            watch_presentation_loop(
                &mut conn,
                &client,
                &clock,
                &claim(),
                lease,
                &mut state,
                &mut out,
                &mut || true
            ),
            0
        );
        assert!(
            lines(&out)
                .iter()
                .any(|line| line.id == "notice:watch-v2-notice"),
            "new owner emits replay independently"
        );
    }
    let calls = client.calls();
    assert_eq!(calls.len(), 4);
    for (pair, lease) in calls.chunks(2).zip([
        "0123456789abcdef0123456789abcdef",
        "22222222222222222222222222222222",
    ]) {
        assert!(
            matches!(&pair[0],Command::DeliveryPullV2(p) if p.presentation_lease.as_deref()==Some(lease))
        );
        assert!(
            matches!(&pair[1],Command::DeliveryPresentationConfirmV2(p) if p.presentation_lease.as_deref()==Some(lease)&&p.emitted_controls.is_empty())
        );
    }
}

/// Every `InboxBatchV2` query the client recorded.
fn inbox_claims(client: &FakeClient) -> Vec<Option<CallerClaim>> {
    client
        .calls()
        .into_iter()
        .filter_map(|command| match command {
            Command::InboxBatchV2(query) => Some(query.claim),
            _ => None,
        })
        .collect()
}

/// Kills: a watch drain that reads the seat (`claim: None`), which never carries a message a
/// sibling binding already settled.
#[test]
fn watch_drain_reads_the_registering_bindings_own_stream() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
    });
    let mut client = empty_inbox();
    client.cursors_capability = true;
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    assert_eq!(exit, 0);
    let claims = inbox_claims(&client);
    assert!(!claims.is_empty(), "the drain after Accepted ran");
    for read in claims {
        assert_eq!(read, Some(claim()), "the page is the binding's own stream");
    }
    server.finish();
}

/// Control: a daemon without `inbox.binding_cursors_v1` keeps the seat read.
#[test]
fn watch_drain_reads_the_seat_without_the_binding_cursors_capability() {
    let server = Server::start(|stream, request| {
        write_frame(stream, &reply(&request, accepted(1)));
    });
    let client = empty_inbox();
    let clock = clock();
    let mut out = Vec::new();
    let exit = run_session(session(&client, &clock, &server.socket), &mut out, || true);
    assert_eq!(exit, 0);
    let claims = inbox_claims(&client);
    assert!(!claims.is_empty());
    for read in claims {
        assert_eq!(read, None);
    }
    server.finish();
}
