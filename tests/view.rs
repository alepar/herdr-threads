use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use herdr_threads::protocol::{
    commands::{Command, DiagnosticsQuery, DirectoryQuery},
    ids::ThreadId,
    output::{OutputFormat, OutputSpec},
    pagination::{Consistency, Page, PageRequest, StopReason},
    results::{CommandResult, Diagnostic, Health, ThreadSummary},
    time::UtcMillis,
};
use herdr_threads::view::*;

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

struct Reader {
    calls: Vec<Command>,
    unavailable: bool,
}
impl ViewReader for Reader {
    fn read(
        &mut self,
        command: Command,
        _: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        self.calls.push(command.clone());
        match command {
            Command::Health if !self.unavailable => {
                let mut health = Health::unknown("instance".into(), "b".into(), "v1".into(), 1);
                health.limitations.push("read worker delayed".into());
                Ok(CommandResult::Health(health))
            }
            Command::Directory(_) if !self.unavailable => {
                Ok(CommandResult::Directory(page(vec![ThreadSummary {
                    last_activity: None,
                    name: None,
                    thread: ThreadId::new("t1"),
                    managed_owner: None,
                    topic_data: "urgent\n\u{001b}[31m\u{0085}\u{2028}".into(),
                    topic_omitted: false,
                    topic_detail_argv: None,
                    archived: false,
                    orphaned: true,
                    message_count: 3,
                    created_at: UtcMillis(0),
                    ordinary_count: 2,
                    system_count: 1,
                    joined_count: 0,
                }])))
            }
            Command::Diagnostics(_) if !self.unavailable => {
                Ok(CommandResult::Diagnostics(page(vec![Diagnostic {
                    subject: "overdue_receipt:m1".into(),
                    detail_data: "seat s1".into(),
                }])))
            }
            _ => Err(herdr_threads::protocol::results::ApiError::host_unavailable("down")),
        }
    }
}

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(100),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn view_reads_only_health_directory_and_overdue_diagnostics() {
    let mut reader = Reader {
        calls: vec![],
        unavailable: false,
    };
    let request = PageRequest::default();
    let bytes = render_view(
        &mut reader,
        &request,
        &OutputSpec::default(),
        &budget(),
        &TestClock,
    )
    .unwrap();
    assert!(matches!(
        reader.calls.as_slice(),
        [
            Command::Health,
            Command::Diagnostics(DiagnosticsQuery {
                seat: None,
                thread: None,
                ..
            }),
            Command::Directory(DirectoryQuery { .. })
        ]
    ));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("degraded"));
    assert!(text.contains("overdue_receipt:m1"));
    assert!(text.contains("t1"));
    assert!(text.contains("urgent\\n\\u001b"));
    assert!(!text.contains('\u{001b}'));
    assert!(!text.contains('\u{0085}'));
    assert!(!text.contains('\u{2028}'));
    assert!(text.contains("refresh"));
    assert!(text.contains("health: herdr-threads --json daemon health"));
}

#[test]
fn emitted_view_continuation_round_trips_real_parser() {
    use herdr_threads::protocol::pagination::{Cursor, CursorDirection, CursorScope};
    let cursor = Cursor {
        instance: "instance".into(),
        scope: CursorScope::Directory,
        scope_key: "all".into(),
        filter_digest: "none".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 205,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .unwrap();
    let request = PageRequest {
        cursor: None,
        limit: 7,
        max_bytes: 4096,
    };
    let spec = OutputSpec {
        context: herdr_threads::protocol::output::ContinuationContext {
            state_dir: Some("/tmp/state with space".into()),
            host: Some("host';$".into()),
        },
        ..Default::default()
    };
    let argv = view_argv(&spec, &request, Some(&cursor));
    let parsed = herdr_threads::cli::commands::parse_argv(argv.clone()).unwrap();
    assert_eq!(
        parsed.output.context.state_dir.as_deref(),
        Some("/tmp/state with space")
    );
    assert!(
        matches!(parsed.action, herdr_threads::cli::commands::CliAction::View { page, .. } if page.cursor.as_deref() == Some(cursor.as_str()) && page.limit == 7 && page.max_bytes == 4096)
    );

    let mut reader = Reader {
        calls: vec![],
        unavailable: false,
    };
    let mut snapshot = load_snapshot(&mut reader, &request, &spec, &budget());
    let threads = snapshot.threads.as_mut().unwrap();
    threads.has_more = true;
    threads.stop_reason = StopReason::Rows;
    threads.next_cursor = Some(cursor.clone());
    threads.next_argv = Some(vec!["herdr-threads".into(), "thread".into(), "list".into()]);
    let encoded = String::from_utf8(render_snapshot(&snapshot, &request, &spec).unwrap()).unwrap();
    let link = encoded
        .lines()
        .find_map(|line| line.strip_prefix("next view: "))
        .expect("rendered next view link");
    assert!(link.contains("--cursor"));
}

#[test]
fn view_reports_full_actual_minimum_without_truncating() {
    let mut reader = Reader {
        calls: vec![],
        unavailable: false,
    };
    let request = PageRequest::default();
    assert!(
        !render_view(
            &mut reader,
            &request,
            &OutputSpec::default(),
            &budget(),
            &TestClock
        )
        .unwrap()
        .is_empty()
    );
    let small = PageRequest {
        max_bytes: 256,
        ..request
    };
    let error = render_view(
        &mut reader,
        &small,
        &OutputSpec::default(),
        &budget(),
        &TestClock,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        herdr_threads::protocol::results::ErrorCode::InvalidBudget
    );
    let minimum = error.required_minimum_bytes.unwrap();
    assert!(minimum > 256 && minimum <= 65_536);
    let exact = PageRequest {
        max_bytes: minimum,
        ..small.clone()
    };
    let bytes = render_view(
        &mut reader,
        &exact,
        &OutputSpec::default(),
        &budget(),
        &TestClock,
    )
    .unwrap();
    assert_eq!(bytes.len(), minimum as usize);
    let below = PageRequest {
        max_bytes: minimum - 1,
        ..small
    };
    assert_eq!(
        render_view(
            &mut reader,
            &below,
            &OutputSpec::default(),
            &budget(),
            &TestClock
        )
        .unwrap_err()
        .code,
        herdr_threads::protocol::results::ErrorCode::InvalidBudget
    );
}

#[test]
fn unavailable_snapshot_is_deterministic_and_stays_bounded() {
    let mut reader = Reader {
        calls: vec![],
        unavailable: true,
    };
    let request = PageRequest::default();
    let snapshot = load_snapshot(
        &mut reader,
        &request,
        &OutputSpec {
            format: OutputFormat::Text,
            ..Default::default()
        },
        &budget(),
    );
    let a = render_snapshot(&snapshot, &request, &OutputSpec::default()).unwrap();
    let b = render_snapshot(&snapshot, &request, &OutputSpec::default()).unwrap();
    assert_eq!(a, b);
    assert!(String::from_utf8(a).unwrap().contains("unavailable"));
}

struct PagedReader {
    rows: Vec<ThreadSummary>,
    directory_calls: usize,
}
impl ViewReader for PagedReader {
    fn read(
        &mut self,
        command: Command,
        _: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        use herdr_threads::protocol::pagination::{Cursor, CursorDirection, CursorScope};
        match command {
            Command::Health => Ok(CommandResult::Health(Health::unknown(
                "instance".into(),
                "b".into(),
                "v1".into(),
                1,
            ))),
            Command::Diagnostics(_) => Ok(CommandResult::Diagnostics(page(vec![]))),
            Command::Directory(query) => {
                self.directory_calls += 1;
                let source_cursor = query
                    .page
                    .cursor
                    .as_deref()
                    .map(|encoded| Cursor::decode(encoded).unwrap());
                let start = source_cursor
                    .as_ref()
                    .map(|cursor| cursor.after_ordinal as usize)
                    .unwrap_or(0);
                let high_water = source_cursor
                    .as_ref()
                    .map(|cursor| cursor.high_water_ordinal as usize)
                    .unwrap_or(self.rows.len());
                let child_cap = if query.page.max_bytes >= 20_000 {
                    80
                } else {
                    5
                };
                let end = (start + usize::from(query.page.limit).min(child_cap)).min(high_water);
                let has_more = end < high_water;
                let next_cursor = has_more.then(|| {
                    Cursor {
                        instance: "instance".into(),
                        scope: CursorScope::Directory,
                        scope_key: "all".into(),
                        filter_digest: "none".into(),
                        direction: CursorDirection::Ascending,
                        order_version: 1,
                        last_examined_key: None,
                        after_ordinal: end as u64,
                        high_water_ordinal: high_water as u64,
                        scope_revision: None,
                        filter_revision: None,
                        search: None,
                        inbox: None,
                        attention: None,
                        binding: None,
                    }
                    .encode()
                    .unwrap()
                });
                Ok(CommandResult::Directory(Page {
                    items: self.rows[start..end].to_vec(),
                    next_cursor,
                    next_argv: has_more
                        .then(|| vec!["herdr-threads".into(), "thread".into(), "list".into()]),
                    high_water_ordinal: high_water as u64,
                    scope_revision: None,
                    has_more,
                    stop_reason: if has_more {
                        StopReason::Rows
                    } else {
                        StopReason::Complete
                    },
                    consistency: Consistency::BoundedLive,
                }))
            }
            _ => unreachable!(),
        }
    }
}

struct HeavyHealthReader(PagedReader);
impl ViewReader for HeavyHealthReader {
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        if matches!(command, Command::Health) {
            let mut health = Health::unknown("instance".into(), "b".into(), "v1".into(), 1);
            health.limitations.push("h".repeat(32_000));
            return Ok(CommandResult::Health(health));
        }
        self.0.read(command, output, budget)
    }
}

#[test]
fn legal_child_at_hard_cap_is_refit_as_progressing_whole_view() {
    use herdr_threads::protocol::pagination::Cursor;
    let rows: Vec<_> = (0..100)
        .map(|i| ThreadSummary {
            last_activity: None,
            name: None,
            thread: ThreadId::new(format!("t{i}")),
            managed_owner: None,
            topic_data: "é control\n and quoted ' topic".repeat(12),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: false,
            message_count: 0,
            created_at: UtcMillis(0),
            ordinary_count: 0,
            system_count: 0,
            joined_count: 0,
        })
        .collect();
    let mut reader = HeavyHealthReader(PagedReader {
        rows,
        directory_calls: 0,
    });
    let request = PageRequest {
        cursor: None,
        limit: 100,
        max_bytes: 65_536,
    };
    let spec = OutputSpec::default();
    let oversized = load_snapshot(&mut reader, &request, &spec, &budget());
    let child = herdr_threads::protocol::output::encode_selected(
        &CommandResult::Directory(oversized.threads.clone().unwrap()),
        &OutputSpec {
            format: OutputFormat::Text,
            ..spec.clone()
        },
    )
    .unwrap();
    assert!(child.len() <= 65_536, "legal source child page");
    assert_eq!(
        render_snapshot(&oversized, &request, &spec)
            .unwrap_err()
            .code,
        herdr_threads::protocol::results::ErrorCode::InvalidBudget
    );
    let bytes = render_view(&mut reader, &request, &spec, &budget(), &TestClock).unwrap();
    assert!(bytes.len() <= 65_536);
    assert!(reader.0.directory_calls > 1);
    let text = String::from_utf8(bytes).unwrap();
    let selected = text
        .lines()
        .filter_map(|line| line.strip_prefix("item: "))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("thread").is_some())
        .count();
    assert!(selected > 0 && selected < 100);
    let next = text
        .lines()
        .find_map(|line| line.strip_prefix("next view: "))
        .unwrap();
    let cursor = Cursor::decode(next.split_whitespace().last().unwrap()).unwrap();
    assert_eq!(cursor.after_ordinal, selected as u64);
}

#[test]
fn fitted_view_reaches_every_row_through_source_cursors() {
    let rows: Vec<_> = (0..205)
        .map(|i| ThreadSummary {
            last_activity: None,
            name: None,
            thread: ThreadId::new(format!("t{i}")),
            managed_owner: None,
            topic_data: "é control\n and quoted ' topic".repeat(4),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: false,
            message_count: 0,
            created_at: UtcMillis(0),
            ordinary_count: 0,
            system_count: 0,
            joined_count: 0,
        })
        .collect();
    let mut reader = PagedReader {
        rows,
        directory_calls: 0,
    };
    let spec = OutputSpec::default();
    let mut request = PageRequest {
        cursor: None,
        limit: 100,
        max_bytes: 2560,
    };
    let mut seen = vec![];
    let mut first_page = true;
    loop {
        let prior_calls = reader.directory_calls;
        let bytes = render_view(&mut reader, &request, &spec, &budget(), &TestClock).unwrap();
        assert!(reader.directory_calls - prior_calls <= 100);
        assert!(bytes.len() <= request.max_bytes as usize);
        let text = String::from_utf8(bytes).unwrap();
        for line in text.lines().filter_map(|line| line.strip_prefix("item: ")) {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            if let Some(thread) = value.get("thread").and_then(|v| v.as_str()) {
                seen.push(thread.to_string());
            }
        }
        let next = text
            .lines()
            .find_map(|line| line.strip_prefix("next view: "));
        match next {
            Some(command) => {
                let cursor = command.split_whitespace().last().unwrap();
                request.cursor = Some(cursor.to_string());
                if first_page {
                    let mut appended = reader.rows.last().unwrap().clone();
                    appended.thread = ThreadId::new("appended");
                    reader.rows.push(appended);
                    first_page = false;
                }
            }
            None => break,
        }
    }
    assert_eq!(seen, (0..205).map(|i| format!("t{i}")).collect::<Vec<_>>());
    assert!(reader.directory_calls > 0);
}

#[test]
fn rendered_actions_execute_as_shell_commands() {
    let mut reader = PagedReader {
        rows: (0..2)
            .map(|i| ThreadSummary {
                last_activity: None,
                name: None,
                thread: ThreadId::new(format!("t{i}")),
                managed_owner: None,
                topic_data: "topic".into(),
                topic_omitted: false,
                topic_detail_argv: None,
                archived: false,
                orphaned: false,
                message_count: 0,
                created_at: UtcMillis(0),
                ordinary_count: 0,
                system_count: 0,
                joined_count: 0,
            })
            .collect(),
        directory_calls: 0,
    };
    let spec = OutputSpec {
        context: herdr_threads::protocol::output::ContinuationContext {
            state_dir: Some("/tmp/space ' $;\u{0085}\u{2028}".into()),
            host: Some("host';$".into()),
        },
        ..Default::default()
    };
    let request = PageRequest {
        cursor: None,
        limit: 1,
        max_bytes: 8192,
    };
    let text = String::from_utf8(
        render_view(&mut reader, &request, &spec, &budget(), &TestClock).unwrap(),
    )
    .unwrap();
    for label in [
        "refresh: ",
        "health: ",
        "overdue: ",
        "thread t0: ",
        "next view: ",
    ] {
        let command = text
            .lines()
            .find_map(|line| line.strip_prefix(label))
            .expect(label);
        let script = format!("herdr-threads() {{ printf '%s\\0' \"$@\"; }}; {command}");
        for shell in ["bash", "zsh"] {
            let output = std::process::Command::new(shell)
                .arg("-c")
                .arg(&script)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let mut argv = vec!["herdr-threads".to_string()];
            let mut parts: Vec<_> = output.stdout.split(|byte| *byte == 0).collect();
            assert_eq!(parts.pop(), Some(&b""[..]));
            argv.extend(
                parts
                    .into_iter()
                    .map(|part| String::from_utf8(part.to_vec()).unwrap()),
            );
            let parsed = herdr_threads::cli::commands::parse_argv(argv).unwrap();
            assert_eq!(parsed.output, spec);
        }
    }
}

struct CancellingReader {
    inner: PagedReader,
    cancellation: Cancellation,
}
impl ViewReader for CancellingReader {
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        let result = self.inner.read(command.clone(), output, budget);
        if matches!(command, Command::Directory(_)) {
            self.cancellation.cancel();
        }
        result
    }
}

#[test]
fn cancellation_after_first_directory_read_prevents_retry() {
    let cancellation = Cancellation::default();
    let mut reader = CancellingReader {
        inner: PagedReader {
            rows: (0..5)
                .map(|i| ThreadSummary {
                    last_activity: None,
                    name: None,
                    thread: ThreadId::new(format!("t{i}")),
                    managed_owner: None,
                    topic_data: "x".repeat(200),
                    topic_omitted: false,
                    topic_detail_argv: None,
                    archived: false,
                    orphaned: false,
                    message_count: 0,
                    created_at: UtcMillis(0),
                    ordinary_count: 0,
                    system_count: 0,
                    joined_count: 0,
                })
                .collect(),
            directory_calls: 0,
        },
        cancellation: cancellation.clone(),
    };
    let result = render_view(
        &mut reader,
        &PageRequest {
            cursor: None,
            limit: 5,
            max_bytes: 2048,
        },
        &OutputSpec::default(),
        &CallBudget {
            deadline: MonoInstant(100),
            cancellation,
        },
        &TestClock,
    );
    assert_eq!(
        result.unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::Cancelled
    );
    assert_eq!(reader.inner.directory_calls, 1);
}

struct MinRejectReader(PagedReader);
impl ViewReader for MinRejectReader {
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        if let Command::Directory(query) = &command
            && query.page.max_bytes < 400
        {
            return Err(herdr_threads::protocol::results::ApiError::invalid_budget(
                "directory item needs 400 bytes",
            )
            .with_required_minimum_bytes(400));
        }
        self.0.read(command, output, budget)
    }
}

#[test]
fn minimum_includes_view_when_first_child_row_is_rejected() {
    let row = ThreadSummary {
        last_activity: None,
        name: None,
        thread: ThreadId::new("t0"),
        managed_owner: None,
        topic_data: "topic".into(),
        topic_omitted: false,
        topic_detail_argv: None,
        archived: false,
        orphaned: false,
        message_count: 0,
        created_at: UtcMillis(0),
        ordinary_count: 0,
        system_count: 0,
        joined_count: 0,
    };
    let mut reader = MinRejectReader(PagedReader {
        rows: vec![row],
        directory_calls: 0,
    });
    let request = PageRequest {
        cursor: None,
        limit: 1,
        max_bytes: 256,
    };
    let error = render_view(
        &mut reader,
        &request,
        &OutputSpec::default(),
        &budget(),
        &TestClock,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        herdr_threads::protocol::results::ErrorCode::InvalidBudget
    );
    let minimum = error.required_minimum_bytes.unwrap();
    assert!(minimum > 400 && minimum <= 65_536);
    let exact = PageRequest {
        max_bytes: minimum,
        ..request
    };
    assert_eq!(
        render_view(
            &mut reader,
            &exact,
            &OutputSpec::default(),
            &budget(),
            &TestClock
        )
        .unwrap()
        .len(),
        minimum as usize
    );
}

struct MutableClock(std::sync::Arc<std::sync::atomic::AtomicU64>);
impl Clock for MutableClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}
struct ExpiringReader {
    inner: PagedReader,
    now: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl ViewReader for ExpiringReader {
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        let result = self.inner.read(command.clone(), output, budget);
        if matches!(command, Command::Directory(_)) {
            self.now.store(1, std::sync::atomic::Ordering::SeqCst);
        }
        result
    }
}

#[test]
fn deadline_after_first_directory_read_prevents_retry() {
    let now = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut reader = ExpiringReader {
        inner: PagedReader {
            rows: vec![],
            directory_calls: 0,
        },
        now: now.clone(),
    };
    let result = render_view(
        &mut reader,
        &PageRequest::default(),
        &OutputSpec::default(),
        &CallBudget {
            deadline: MonoInstant(1),
            cancellation: Cancellation::default(),
        },
        &MutableClock(now),
    );
    assert_eq!(
        result.unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::DeadlineExceeded
    );
    assert_eq!(reader.inner.directory_calls, 1);
}

#[test]
fn overdue_discovery_stays_independent_of_directory_continuation() {
    use herdr_threads::protocol::pagination::{Cursor, CursorDirection, CursorScope};
    let cursor = |scope| {
        Cursor {
            instance: "instance".into(),
            scope,
            scope_key: "all".into(),
            filter_digest: "none".into(),
            direction: CursorDirection::Ascending,
            order_version: 1,
            last_examined_key: None,
            after_ordinal: 1,
            high_water_ordinal: 205,
            scope_revision: None,
            filter_revision: None,
            search: None,
            inbox: None,
            attention: None,
            binding: None,
        }
        .encode()
        .unwrap()
    };
    let directory_cursor = cursor(CursorScope::Directory);
    let overdue_cursor = cursor(CursorScope::Diagnostics);
    let mut reader = Reader {
        calls: vec![],
        unavailable: false,
    };
    let request = PageRequest {
        cursor: None,
        limit: 1,
        max_bytes: 8192,
    };
    let spec = OutputSpec::default();
    let mut snapshot = load_snapshot(&mut reader, &request, &spec, &budget());
    let threads = snapshot.threads.as_mut().unwrap();
    threads.has_more = true;
    threads.stop_reason = StopReason::Rows;
    threads.next_cursor = Some(directory_cursor.clone());
    threads.next_argv = Some(vec![
        "herdr-threads".into(),
        "thread".into(),
        "list".into(),
        "--cursor".into(),
        directory_cursor,
    ]);
    let overdue = snapshot.overdue.as_mut().unwrap();
    overdue.has_more = true;
    overdue.stop_reason = StopReason::Rows;
    overdue.next_cursor = Some(overdue_cursor.clone());
    overdue.next_argv = Some(vec![
        "herdr-threads".into(),
        "overdue".into(),
        "--cursor".into(),
        overdue_cursor,
    ]);
    let text = String::from_utf8(render_snapshot(&snapshot, &request, &spec).unwrap()).unwrap();
    assert!(text.contains("next: herdr-threads overdue --cursor"));
    assert!(text.contains("next view: herdr-threads --json view"));
    snapshot.overdue = Err(herdr_threads::protocol::results::ApiError::host_unavailable("down"));
    let unavailable =
        String::from_utf8(render_snapshot(&snapshot, &request, &spec).unwrap()).unwrap();
    assert!(unavailable.contains("overdue: herdr-threads --json overdue"));
    assert!(unavailable.contains("next view: herdr-threads --json view"));
}

/// Kills: omitting `--once` from the rendered refresh/continuation argv
/// (the one-shot CLI rejects a bare `view` as unsupported).
#[test]
fn refresh_and_continuation_argv_select_the_one_shot_view() {
    let spec = OutputSpec::default();
    let request = PageRequest::default();
    {
        let argv = view_argv(&spec, &request, None);
        let parsed = herdr_threads::cli::commands::parse_argv(argv.clone()).unwrap();
        assert!(
            matches!(
                parsed.action,
                herdr_threads::cli::commands::CliAction::View { once: true, .. }
            ),
            "{argv:?}"
        );
    }
}

mod escaping {
    use super::*;
    use herdr_threads::cli::{
        human,
        irc::{self, NoLookup, Style},
    };
    use herdr_threads::protocol::{
        ids::{MessageId, SeatId, ThreadId},
        output::{ContinuationContext, OutputFormat, OutputSpec},
        results::{CommandResult, MessageKind, MessageSummary, ThreadSummary},
    };
    use herdr_threads::view::escape::{
        Context::{MultiLine, SingleLine},
        display_width, escape_for_terminal, is_format_char, pad_to_width,
    };

    fn text_spec() -> OutputSpec {
        OutputSpec {
            format: OutputFormat::Text,
            context: ContinuationContext::default(),
        }
    }

    fn thread(id: &str, topic: &str) -> ThreadSummary {
        ThreadSummary {
            last_activity: None,
            name: None,
            thread: ThreadId::new(id),
            managed_owner: None,
            topic_data: topic.into(),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: false,
            message_count: 1,
            created_at: UtcMillis(1_790_771_696_000),
            ordinary_count: 1,
            system_count: 0,
            joined_count: 1,
        }
    }

    fn message(body: &str) -> MessageSummary {
        MessageSummary {
            message: MessageId::new("msg-1"),
            thread: ThreadId::new("thread-Ab12Cd34"),
            author: Some(SeatId::new("seat-Alice001")),
            event_author: None,
            author_role: None,
            relays_user: false,
            user_intent: None,
            author_role_backfilled: false,
            kind: MessageKind::Ordinary,
            sequence: 1,
            created_at: UtcMillis(1_790_771_696_000),
            actor_label: None,
            preview_data: body.into(),
            preview_omitted: false,
            preview_detail_argv: None,
        }
    }

    #[test]
    fn escaping_table() {
        // (class, input, MultiLine output, SingleLine output)
        let rows: &[(&str, &str, &str, &str)] = &[
            ("plain", "a b·é漢", "a b·é漢", "a b·é漢"),
            ("C0 NUL", "\u{0}", "\\u{0000}", "\\u{0000}"),
            ("C0 ESC", "\u{1b}[2J", "\\u{001b}[2J", "\\u{001b}[2J"),
            ("C0 BEL", "\u{7}", "\\u{0007}", "\\u{0007}"),
            ("newline", "a\nb", "a\nb", "a\\nb"),
            ("carriage return", "a\rb", "a\\rb", "a\\rb"),
            ("tab", "a\tb", "a\tb", "a\\tb"),
            ("DEL", "\u{7f}", "\\u{007f}", "\\u{007f}"),
            ("C1 CSI", "\u{9b}", "\\u{009b}", "\\u{009b}"),
            ("C1 NEL", "\u{85}", "\\u{0085}", "\\u{0085}"),
            ("LS", "\u{2028}", "\\u{2028}", "\\u{2028}"),
            ("PS", "\u{2029}", "\\u{2029}", "\\u{2029}"),
            ("LRM", "\u{200e}", "\\u{200e}", "\\u{200e}"),
            ("RLM", "\u{200f}", "\\u{200f}", "\\u{200f}"),
            ("ALM", "\u{61c}", "\\u{061c}", "\\u{061c}"),
            ("LRE", "\u{202a}", "\\u{202a}", "\\u{202a}"),
            ("RLE", "\u{202b}", "\\u{202b}", "\\u{202b}"),
            ("PDF", "\u{202c}", "\\u{202c}", "\\u{202c}"),
            ("LRO", "\u{202d}", "\\u{202d}", "\\u{202d}"),
            ("RLO", "\u{202e}", "\\u{202e}", "\\u{202e}"),
            ("LRI", "\u{2066}", "\\u{2066}", "\\u{2066}"),
            ("RLI", "\u{2067}", "\\u{2067}", "\\u{2067}"),
            ("FSI", "\u{2068}", "\\u{2068}", "\\u{2068}"),
            ("PDI", "\u{2069}", "\\u{2069}", "\\u{2069}"),
            ("Cf soft hyphen", "a\u{ad}b", "a\\u{00ad}b", "a\\u{00ad}b"),
            ("Cf ZWSP", "\u{200b}", "\\u{200b}", "\\u{200b}"),
            ("Cf BOM", "\u{feff}", "\\u{feff}", "\\u{feff}"),
            ("Cf word joiner", "\u{2060}", "\\u{2060}", "\\u{2060}"),
            ("Cf tag", "\u{e0001}", "\\u{e0001}", "\\u{e0001}"),
            ("Cf astral", "\u{1d173}", "\\u{1d173}", "\\u{1d173}"),
        ];
        for (class, input, multi, single) in rows {
            assert_eq!(
                escape_for_terminal(input, MultiLine),
                *multi,
                "{class} multi"
            );
            assert_eq!(
                escape_for_terminal(input, SingleLine),
                *single,
                "{class} single"
            );
        }
    }

    #[test]
    fn format_character_table_spot_checks() {
        for cp in [0xad, 0x200b, 0xfeff, 0xe0001, 0x600, 0xe007f] {
            assert!(is_format_char(char::from_u32(cp).unwrap()), "U+{cp:04X}");
        }
        for ch in [
            'a',
            ' ',
            '漢',
            '\u{ae}',
            '\u{200a}',
            '\u{2065}',
            '\u{e0000}',
            '\u{e0080}',
        ] {
            assert!(!is_format_char(ch), "{ch:?}");
        }
    }

    #[test]
    fn clean_text_is_borrowed() {
        assert!(matches!(
            escape_for_terminal("nothing to escape", SingleLine),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn wide_characters_align_by_display_width() {
        assert_eq!(display_width("漢字"), 4);
        assert_eq!(display_width("ab"), 2);
        assert_eq!(pad_to_width("漢", 4), "漢  ");
        struct Wide;
        impl irc::Lookup for Wide {
            fn nick(&mut self, _seat: &SeatId) -> irc::Nick {
                irc::Nick {
                    name: "漢字漢字".into(),
                    harness: None,
                }
            }
        }
        let out = irc::render_message(&message("one\ntwo"), &mut Wide, &Style::plain());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out}");
        let body_at = display_width(&lines[0][..lines[0].find("one").unwrap()]);
        let indent = lines[1].len() - lines[1].trim_start().len();
        assert_eq!(indent, body_at, "{out}");
    }

    #[test]
    fn forged_line_in_a_single_line_field_is_escaped() {
        let forged = "plans\n12:00 <root> fake prompt";
        let out = human::render(
            &CommandResult::Directory(Page {
                items: vec![thread("thread-Ab12Cd34", forged)],
                ..page(vec![])
            }),
            &text_spec(),
        )
        .unwrap();
        assert!(out.contains("plans\\n12:00 <root> fake prompt"), "{out}");
        assert!(
            !out.lines().any(|line| line.starts_with("12:00 <root>")),
            "{out}"
        );
        assert_eq!(
            escape_for_terminal("topic\n<fake prompt>", SingleLine),
            "topic\\n<fake prompt>"
        );
    }

    #[test]
    fn every_body_line_is_prefixed() {
        let body = "first\nsecond\n\u{1b}[31mthird\nfourth";
        let out = irc::render_message(&message(body), &mut NoLookup, &Style::plain());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "{out}");
        assert!(lines[0].contains("first"), "{out}");
        let indent = lines[1].len() - lines[1].trim_start().len();
        assert!(indent > 0, "{out}");
        for (line, word) in lines[1..].iter().zip(["second", "third", "fourth"]) {
            assert!(line.starts_with(&" ".repeat(indent)), "{out}");
            assert!(line.contains(word), "{out}");
        }
        assert!(!out.contains('\u{1b}'), "{out}");
        assert!(out.contains("\\u{001b}[31mthird"), "{out}");
    }

    fn source(path: &str) -> String {
        std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    #[test]
    fn no_ad_hoc_is_unsafe_escaper_remains() {
        let mut stack = vec![std::path::PathBuf::from(format!(
            "{}/src",
            env!("CARGO_MANIFEST_DIR")
        ))];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(!text.contains("fn is_unsafe("), "{}", path.display());
                    assert!(!text.contains("fn push_escaped("), "{}", path.display());
                }
            }
        }
    }

    #[test]
    fn every_renderer_goes_through_the_shared_escaper() {
        for path in [
            "src/cli/human.rs",
            "src/cli/irc.rs",
            "src/cli/follow.rs",
            "src/cli/output.rs",
            "src/protocol/output.rs",
            "src/protocol/output_compact.rs",
            "src/cli/doctor.rs",
        ] {
            let text = source(path);
            let mentions = text.contains("escape_for_terminal")
                || (path == "src/cli/irc.rs" && text.contains("one_line"));
            assert!(mentions, "{path} does not use escape_for_terminal");
        }
    }
}
