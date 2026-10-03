use super::*;

#[test]
fn bounded_collection_routes_parse_with_exact_filters() {
    type Case = (Vec<&'static str>, fn(&WireCommand) -> bool);
    let cases: Vec<Case> = vec![
        (
            vec![
                "herdr-threads",
                "thread",
                "participants",
                "t1",
                "--limit",
                "7",
            ],
            |c| matches!(c,WireCommand::Participants(q) if q.thread.as_str()=="t1" && q.page.limit==7),
        ),
        (
            vec![
                "herdr-threads",
                "delivery",
                "recipients",
                "m1",
                "--limit",
                "7",
            ],
            |c| matches!(c,WireCommand::Recipients(q) if q.message.as_str()=="m1" && q.page.limit==7),
        ),
        (
            vec!["herdr-threads", "seat", "retirements", "--limit", "7"],
            |c| matches!(c,WireCommand::RetirementJobs(q) if q.page.limit==7),
        ),
        (
            vec![
                "herdr-threads",
                "thread",
                "list",
                "--seat",
                "s1",
                "--joined",
                "--search",
                "topic",
                "--limit",
                "7",
            ],
            |c| matches!(c,WireCommand::Directory(q) if q.membership.as_ref().is_some_and(|v|v.as_str()=="s1") && q.membership_filter==DirectoryMembership::Joined && q.topic_contains.as_deref()==Some("topic") && q.page.limit==7),
        ),
        (
            vec!["herdr-threads", "inbox", "--seat", "s1", "--limit", "7"],
            |c| matches!(c,WireCommand::Inbox(q) if q.seat.as_ref().is_some_and(|v|v.as_str()=="s1") && q.page.limit==7),
        ),
    ];
    for (argv, predicate) in cases {
        let parsed = parse_argv(argv).unwrap();
        let CliAction::Wire(command) = parsed.action else {
            panic!("expected read route")
        };
        assert!(predicate(&command));
    }
}

#[test]
fn overdue_uses_global_context_and_typed_diagnostics_read_without_operation_key() {
    use crate::protocol::{
        commands::Command,
        output::{OutputFormat, OutputSpec},
        pagination::{Cursor, CursorDirection, CursorScope},
        results::{CommandResult, Health},
    };
    struct ReadStub(Option<(Command, OutputSpec)>);
    impl CliBackend for ReadStub {
        fn call(
            &mut self,
            command: Command,
            output: &OutputSpec,
        ) -> Result<CommandResult, ApiError> {
            self.0 = Some((command, output.clone()));
            Ok(CommandResult::Health(Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "test".into(),
                1,
            )))
        }
    }
    let cursor = Cursor {
        instance: "instance-1".into(),
        scope: CursorScope::Diagnostics,
        scope_key: "overdue".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 3,
        high_water_ordinal: 20,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .unwrap();
    let parsed = parse_argv(vec![
        "herdr-threads".into(),
        "--state-dir".into(),
        "/tmp/state with space".into(),
        "--host-endpoint".into(),
        "/tmp/host.sock".into(),
        "--json".into(),
        "overdue".into(),
        "--cursor".into(),
        cursor.clone(),
        "--limit".into(),
        "3".into(),
        "--max-bytes".into(),
        "600".into(),
    ])
    .unwrap();
    let mut stub = ReadStub(None);
    dispatch(parsed, &mut stub, None, None).unwrap();
    let (command, output) = stub.0.unwrap();
    assert_eq!(
        output.context.state_dir.as_deref(),
        Some("/tmp/state with space")
    );
    assert_eq!(output.context.host.unwrap().as_str(), "/tmp/host.sock");
    assert_eq!(output.format, OutputFormat::Json);
    let Command::Diagnostics(query) = command else {
        panic!("expected diagnostics read")
    };
    assert_eq!(query.seat, None);
    assert_eq!(query.thread, None);
    assert_eq!(query.page.cursor.as_deref(), Some(cursor.as_str()));
    assert_eq!((query.page.limit, query.page.max_bytes), (3, 600));
}

#[test]
fn filtered_diagnostics_continuation_parses_both_scopes() {
    let cursor = crate::protocol::pagination::Cursor {
        instance: "i".into(),
        scope: crate::protocol::pagination::CursorScope::Diagnostics,
        scope_key: "*".into(),
        filter_digest: "digest".into(),
        direction: crate::protocol::pagination::CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 2,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .unwrap();
    let parsed = parse_argv(vec![
        "herdr-threads".into(),
        "diagnostics".into(),
        "--seat".into(),
        "s".into(),
        "--thread".into(),
        "t".into(),
        "--cursor".into(),
        cursor.clone(),
        "--limit".into(),
        "2".into(),
        "--max-bytes".into(),
        "4096".into(),
    ])
    .unwrap();
    assert!(matches!(parsed.action,
        CliAction::Wire(Command::Diagnostics(ref q))
        if q.seat.as_ref().is_some_and(|s| s.as_str()=="s")
        && q.thread.as_ref().is_some_and(|t| t.as_str()=="t")
        && q.page.cursor.as_deref()==Some(cursor.as_str())
        && q.page.limit==2
        && q.page.max_bytes==4096
    ));
}

#[test]
fn overdue_rejects_invalid_cursor_and_page_bounds() {
    use crate::protocol::results::ErrorCode;
    assert_eq!(
        parse_argv(["herdr-threads", "overdue", "--cursor", "bad"])
            .unwrap_err()
            .code,
        ErrorCode::InvalidCursor
    );
    assert_eq!(
        parse_argv(["herdr-threads", "overdue", "--limit", "0"])
            .unwrap_err()
            .code,
        ErrorCode::InvalidBudget
    );
    assert_eq!(
        parse_argv(["herdr-threads", "overdue", "--max-bytes", "1"])
            .unwrap_err()
            .code,
        ErrorCode::InvalidBudget
    );
}

#[test]
fn read_routes_select_typed_operations_and_explicit_context() {
    let parsed = parse_argv([
        "herdr-threads",
        "--state-dir",
        "/tmp/state with space",
        "--host-endpoint",
        "/tmp/herdr.sock",
        "--json",
        "pending-receipts",
        "--seat",
        "s1",
        "--thread",
        "t1",
        "--limit",
        "7",
        "--max-bytes",
        "900",
    ])
    .unwrap();
    assert_eq!(
        parsed.output.context.state_dir.as_deref(),
        Some("/tmp/state with space")
    );
    assert_eq!(
        parsed.output.context.host.unwrap().as_str(),
        "/tmp/herdr.sock"
    );
    assert_eq!(
        parsed.output.format,
        crate::protocol::output::OutputFormat::Json
    );
    assert!(
        matches!(parsed.action, CliAction::Wire(crate::protocol::commands::Command::PendingReceipts(q)) if
        q.seat.as_ref().unwrap().as_str() == "s1" && q.thread.as_ref().unwrap().as_str() == "t1" && q.page.limit == 7 && q.page.max_bytes == 900)
    );
}

// P7 (native Claude demo 3): a wrapper that pins --state-dir/--host-endpoint
// plus a ready command that names the same values must run; conflicting
// values are refused as invalid arguments naming the flag and both values.
// Kills: clap's "cannot be used multiple times" on identical repeats, and
// silently picking one of two different state dirs.
#[test]
fn identical_repeated_global_paths_are_accepted_and_conflicts_refused() {
    let parsed = parse_argv([
        "herdr-threads",
        "--state-dir",
        "/tmp/s",
        "--host-endpoint",
        "/tmp/h.sock",
        "--state-dir",
        "/tmp/s",
        "inbox",
        "--host-endpoint",
        "/tmp/h.sock",
    ])
    .unwrap();
    assert_eq!(parsed.output.context.state_dir.as_deref(), Some("/tmp/s"));
    assert_eq!(parsed.output.context.host.as_deref(), Some("/tmp/h.sock"));
    let single = parse_argv(["herdr-threads", "--state-dir", "/tmp/s", "inbox"]).unwrap();
    assert_eq!(single.output.context.state_dir.as_deref(), Some("/tmp/s"));
    assert_eq!(single.output.context.host, None);
    for (flag, a, b) in [
        ("--state-dir", "/tmp/s", "/tmp/t"),
        ("--host-endpoint", "/tmp/a.sock", "/tmp/b.sock"),
    ] {
        let error = parse_argv(["herdr-threads", flag, a, flag, b, "inbox"]).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.detail);
        assert!(
            error.detail.contains(flag) && error.detail.contains(a) && error.detail.contains(b),
            "{}",
            error.detail
        );
    }
}

#[test]
fn recent_history_rejects_conflicting_page_options() {
    assert!(
        parse_argv([
            "herdr-threads",
            "read",
            "t1",
            "--recent",
            "5",
            "--limit",
            "5"
        ])
        .is_err()
    );
    assert!(
        parse_argv([
            "herdr-threads",
            "read",
            "t1",
            "--cursor",
            "c2:abc",
            "--before",
            "9"
        ])
        .is_err()
    );
}

#[test]
fn recent_history_enforces_shared_page_limit_before_dispatch() {
    use crate::protocol::{commands::Command, results::ErrorCode};

    for (count, expected) in [
        ("0", ErrorCode::InvalidBudget),
        ("101", ErrorCode::InvalidBudget),
        ("65535", ErrorCode::InvalidBudget),
        ("65536", ErrorCode::InvalidRequest),
    ] {
        let result = parse_argv(["herdr-threads", "read", "t1", "--recent", count]);
        assert_eq!(result.unwrap_err().code, expected, "--recent {count}");
    }

    for count in ["1", "100"] {
        let parsed = parse_argv(["herdr-threads", "read", "t1", "--recent", count]).unwrap();
        let CliAction::Wire(Command::History(query)) = parsed.action else {
            panic!("expected history query for --recent {count}")
        };
        let expected: u16 = count.parse().unwrap();
        assert_eq!(query.page.limit, expected);
        assert!(
            matches!(query.initial, Some(crate::protocol::commands::HistoryRange::Recent { count }) if count == expected)
        );
    }
}

#[test]
fn cursor_and_output_budget_errors_keep_typed_codes() {
    use crate::protocol::results::ErrorCode;
    let cursor = parse_argv(["herdr-threads", "inbox", "--cursor", "bad"]).unwrap_err();
    assert_eq!(cursor.code, ErrorCode::InvalidCursor);
    let budget = parse_argv(["herdr-threads", "inbox", "--max-bytes", "999999"]).unwrap_err();
    assert_eq!(budget.code, ErrorCode::InvalidBudget);
    let body_budget = parse_argv(["herdr-threads", "body", "m1", "--max-bytes", "1"]).unwrap_err();
    assert_eq!(body_budget.code, ErrorCode::InvalidBudget);
}

#[test]
fn inline_body_preserves_unicode_and_newlines_without_echo() {
    let parsed = parse_argv(["herdr-threads", "send", "t1", "--body", "hello\n雪"]).unwrap();
    assert!(
        matches!(parsed.action, CliAction::Mutation(MutationSpec::Send { ref body, .. }) if
        body == "hello\n雪")
    );
}

fn claim() -> crate::protocol::authority::CallerClaim {
    use crate::protocol::{
        authority::{CallerClaim, Harness},
        ids::*,
    };
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("legacy-fixture"),
        binding_generation: 0,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("native-1"),
        execution: ExecutionId::new("execution-1"),
        target: HostTargetId::new("pane-1"),
    }
}

#[test]
fn exact_ack_materializes_one_typed_batch() {
    let parsed = parse_argv(["herdr-threads", "ack", "m1", "m2"]).unwrap();
    let CliAction::Mutation(spec) = parsed.action else {
        panic!("expected mutation")
    };
    let command = spec
        .into_command(
            Some(claim()),
            crate::protocol::ids::OperationId::new("op-1"),
        )
        .unwrap();
    assert!(
        matches!(command, crate::protocol::commands::Command::Ack(ack) if
        ack.messages.iter().map(|id| id.as_str()).collect::<Vec<_>>() == ["m1", "m2"])
    );
}

#[test]
fn operator_invite_materializes_only_operator_variant() {
    let parsed = parse_argv([
        "herdr-threads",
        "invite",
        "t1",
        "--seat",
        "s1",
        "--deadline",
        "4",
        "--operator",
    ])
    .unwrap();
    let CliAction::Mutation(spec) = parsed.action else {
        panic!("expected mutation")
    };
    let command = spec
        .into_command(None, crate::protocol::ids::OperationId::new("op-2"))
        .unwrap();
    assert!(
        matches!(command, crate::protocol::commands::Command::OperatorOrphanInvite(invite) if invite.deadline_millis == Some(4000))
    );
}

#[test]
fn documented_read_table_dispatches_to_stub_client() {
    use crate::protocol::{
        commands::Command,
        results::{CommandResult, Health},
    };
    struct Stub(Vec<Command>);
    impl CliBackend for Stub {
        fn call(
            &mut self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.0.push(command);
            Ok(CommandResult::Health(Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "test".into(),
                1,
            )))
        }
    }
    let rows: &[(&[&str], &str)] = &[
        (&["thread", "list"], "directory"),
        (&["thread", "show", "t1"], "thread"),
        (&["inbox"], "inbox"),
        (&["warnings", "--seat", "s1"], "warnings"),
        (&["pending-receipts", "--seat", "s1"], "pending_receipts"),
        (&["read", "t1", "--after", "3"], "history"),
        (&["body", "m1"], "message"),
        (&["search", "雪", "--thread", "t1"], "search"),
        (&["seat", "list"], "seats"),
        (&["seat", "inspect", "s1"], "seat_inspect"),
        (&["delivery", "inspect", "m1"], "delivery_inspect"),
    ];
    let mut stub = Stub(vec![]);
    for &(args, expected) in rows {
        let parsed =
            parse_argv(std::iter::once("herdr-threads").chain(args.iter().copied())).unwrap();
        dispatch(parsed, &mut stub, None, None).unwrap();
        let command = stub.0.last().unwrap();
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["kind"], expected, "{args:?}");
    }
}

#[test]
fn warnings_route_requires_seat_and_valid_page() {
    use crate::protocol::commands::Command;
    assert!(parse_argv(["herdr-threads", "warnings"]).is_err());
    assert!(parse_argv(["herdr-threads", "warnings", "--seat", "s1", "--limit", "0"]).is_err());
    let parsed = parse_argv(["herdr-threads", "warnings", "--seat", "s1", "--limit", "7"]).unwrap();
    assert!(
        matches!(parsed.action,CliAction::Wire(Command::Warnings(ref query)) if query.seat.as_str()=="s1" && query.page.limit==7)
    );
}

#[test]
fn revised_directory_accept_and_body_routes_are_typed() {
    use crate::protocol::commands::{Command, DirectoryMembership};
    let list = parse_argv([
        "herdr-threads",
        "thread",
        "list",
        "--invited",
        "--search",
        "snow 雪",
    ])
    .unwrap();
    assert!(
        matches!(list.action, CliAction::Wire(Command::Directory(q)) if q.membership_filter == DirectoryMembership::Invited && q.topic_contains.as_deref() == Some("snow 雪"))
    );
    let body = parse_argv([
        "herdr-threads",
        "body",
        "m1",
        "--offset",
        "21",
        "--max-bytes",
        "500",
    ])
    .unwrap();
    assert!(
        matches!(body.action, CliAction::Wire(Command::Message(q)) if q.body.offset == Some(21) && q.body.max_bytes == 500)
    );
    let accept = parse_argv(["herdr-threads", "accept", "t1"]).unwrap();
    let CliAction::Mutation(spec) = accept.action else {
        panic!("expected mutation")
    };
    assert!(
        matches!(spec.into_command(Some(claim()), crate::protocol::ids::OperationId::new("op3")).unwrap(), Command::Accept(a) if a.thread.as_str() == "t1")
    );
}

#[test]
fn required_acceptance_names_the_displayed_episode_and_revision() {
    use crate::protocol::commands::Command;
    let parsed = parse_argv([
        "herdr-threads", "accept-required", "thread-1", "--invitation", "invite-1",
        "--requirement", "requirement-1", "--revision", "4",
    ]).unwrap();
    let CliAction::Mutation(spec) = parsed.action else { panic!("expected mutation") };
    let Command::AcceptRequired(accept) = spec.into_command(
        Some(claim()), crate::protocol::ids::OperationId::new("accept-op")
    ).unwrap() else { panic!("required acceptance must keep its wire kind") };
    assert_eq!(accept.thread.as_str(), "thread-1");
    assert_eq!(accept.invitation.as_str(), "invite-1");
    assert_eq!(accept.requirement.as_str(), "requirement-1");
    assert_eq!(accept.expected_revision, 4);
    assert!(parse_argv(["herdr-threads", "accept-required", "thread-1", "--invitation", "invite-1", "--requirement", "requirement-1", "--revision", "0"]).is_err());
}

#[test]
fn operator_service_recovery_uses_observed_boot_and_generation() {
    use crate::protocol::commands::Command;
    let inspect = parse_argv(["herdr-threads", "service", "inspect"]).unwrap();
    assert!(matches!(inspect.action, CliAction::Wire(Command::ServiceInspect)));
    let boot = "00000000-0000-4000-8000-000000000001";
    let disconnect = parse_argv(["herdr-threads", "service", "disconnect", "--expected-boot", boot, "--expected-generation", "7"]).unwrap();
    assert!(matches!(disconnect.action, CliAction::Wire(Command::ServiceDisconnect(request)) if request.expected_boot == boot && request.expected_generation == 7));
    assert!(parse_argv(["herdr-threads", "service", "disconnect", "--expected-boot", boot]).is_err());
}

#[test]
fn ordinary_seat_resolution_requires_durable_mutation_key() {
    let parsed = parse_argv(["herdr-threads", "seat", "resolve", "--pane", "p1"]).unwrap();
    let CliAction::Mutation(MutationSpec::Resolve(pane)) = parsed.action else {
        panic!("expected durable resolve")
    };
    assert_eq!(pane.as_str(), "p1");
    let command = MutationSpec::Resolve(pane)
        .into_command(None, crate::protocol::ids::OperationId::new("op-resolve"))
        .unwrap();
    assert!(
        matches!(command, crate::protocol::commands::Command::ResolveSeat(resolve) if
        resolve.target.as_str() == "p1" && resolve.operation.as_str() == "op-resolve")
    );
}

#[test]
fn mutation_is_not_submitted_without_durable_operation_key() {
    use crate::protocol::{
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CommandResult},
    };
    struct Stub;
    impl CliBackend for Stub {
        fn call(&mut self, _: Command, _: &OutputSpec) -> Result<CommandResult, ApiError> {
            panic!("submitted without key")
        }
    }
    let parsed = parse_argv(["herdr-threads", "seat", "resolve", "--pane", "p1"]).unwrap();
    assert!(dispatch(parsed, &mut Stub, None, None).is_err());
}

#[test]
fn parser_rejects_operator_and_source_conflicts() {
    for argv in [
        vec!["herdr-threads", "ack", "m1", "--operator"],
        vec!["herdr-threads", "accept", "t1", "--operator"],
        vec!["herdr-threads", "check-in", "--operator"],
        vec![
            "herdr-threads",
            "seat",
            "resolve",
            "--pane",
            "p1",
            "--new-seat",
        ],
        vec!["herdr-threads", "seat", "rebind", "s1", "--pane", "p1"],
        vec!["herdr-threads", "send", "t1", "--body", "x", "--stdin"],
        vec!["herdr-threads", "send", "t1", "--body", "x", "--file", "y"],
        vec![
            "herdr-threads",
            "body",
            "m1",
            "--offset",
            "1",
            "--cursor",
            "bad",
        ],
    ] {
        assert!(parse_argv(argv.clone()).is_err(), "{argv:?}");
    }
}

#[test]
fn send_rejects_duplicate_and_excess_explicit_recipients() {
    assert!(
        parse_argv([
            "herdr-threads",
            "send",
            "t1",
            "--body",
            "hello",
            "--require-ack",
            "s1",
            "--require-ack",
            "s1"
        ])
        .is_err()
    );
    let mut argv = vec![
        "herdr-threads".to_string(),
        "send".into(),
        "t1".into(),
        "--body".into(),
        "hello".into(),
    ];
    for index in 0..=crate::protocol::commands::MAX_BATCH_ITEMS {
        argv.push("--require-ack".into());
        argv.push(format!("s{index}"));
    }
    assert!(parse_argv(argv).is_err());
}

#[test]
fn send_accepts_multi_and_repeated_require_ack_with_following_options() {
    let parsed = parse_argv([
        "herdr-threads",
        "send",
        "t1",
        "--body",
        "hello",
        "--require-ack",
        "s1",
        "s2",
        "--require-ack",
        "s3",
        "s4",
        "--deadline",
        "7",
    ])
    .unwrap();
    let CliAction::Mutation(spec) = parsed.action else {
        panic!("expected send mutation")
    };
    let command = spec
        .into_command(
            Some(claim()),
            crate::protocol::ids::OperationId::new("op-seats"),
        )
        .unwrap();
    let crate::protocol::commands::Command::SendMessage(send) = command else {
        panic!("expected typed send request")
    };
    assert_eq!(
        send.invited_recipients
            .iter()
            .map(|seat| seat.as_str())
            .collect::<Vec<_>>(),
        ["s1", "s2", "s3", "s4"]
    );
    assert_eq!(send.deadline_millis, Some(7000));
}

#[test]
fn file_body_preserves_exact_utf8_and_rejects_invalid_bytes() {
    let path = std::env::temp_dir().join(format!("herdr-cli-body-{}", std::process::id()));
    std::fs::write(&path, "first\n雪\n").unwrap();
    let parsed = parse_argv([
        "herdr-threads",
        "send",
        "t1",
        "--file",
        path.to_str().unwrap(),
    ])
    .unwrap();
    assert!(
        matches!(parsed.action, CliAction::Mutation(MutationSpec::Send { body, .. }) if body == "first\n雪\n")
    );
    std::fs::write(&path, [0xff]).unwrap();
    assert!(
        parse_argv([
            "herdr-threads",
            "send",
            "t1",
            "--file",
            path.to_str().unwrap()
        ])
        .is_err()
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn stdin_body_reads_raw_utf8_once_with_bound() {
    let mut input = "line one\n雪\n".as_bytes();
    let body = crate::cli::input::read_body_from(None, None, true, &mut input).unwrap();
    assert_eq!(body, "line one\n雪\n");
    let bytes = vec![b'x'; crate::cli::input::MAX_BODY_BYTES + 1];
    let mut oversized = bytes.as_slice();
    assert!(crate::cli::input::read_body_from(None, None, true, &mut oversized).is_err());
}

#[test]
fn body_bound_includes_json_escaping_before_submission() {
    let body = "\0".repeat(200_000);
    assert!(crate::cli::input::read_body(Some(body), None, false).is_err());
}

#[test]
fn cli_body_limit_is_the_store_limit_and_refuses_before_any_journaling() {
    assert_eq!(
        crate::cli::input::MAX_BODY_BYTES,
        crate::store::messages::MAX_BODY_BYTES
    );
    let limit = crate::cli::input::MAX_BODY_BYTES;
    assert!(crate::cli::input::read_body(Some("x".repeat(limit)), None, false).is_ok());
    // Parsing fails before any runner can record a durable intent.
    let oversized = "x".repeat(limit + 1);
    assert!(parse_argv(["herdr-threads", "send", "t1", "--body", oversized.as_str()]).is_err());
    let within = "x".repeat(limit);
    assert!(parse_argv(["herdr-threads", "send", "t1", "--body", within.as_str()]).is_ok());
}

#[test]
fn local_recovery_reference_is_not_an_ack_id() {
    assert!(parse_argv(["herdr-threads", "ack", "local:R_123"]).is_err());
    assert!(matches!(
        parse_argv(["herdr-threads", "retry", "local:R_123"])
            .unwrap()
            .action,
        CliAction::Retry(_)
    ));
    assert!(parse_argv(["herdr-threads", "retry", "m1"]).is_err());
}

#[test]
fn create_uses_supplied_topic_as_default_goal() {
    let parsed = parse_argv(["herdr-threads", "thread", "create", "--topic", "Orbit 雪"]).unwrap();
    let CliAction::Mutation(MutationSpec::Create { topic, goal }) = parsed.action else {
        panic!("expected create")
    };
    assert_eq!(topic, "Orbit 雪");
    assert_eq!(goal, topic);
}

#[test]
fn documented_mutations_dispatch_exact_typed_command_to_stub() {
    use crate::protocol::{
        commands::Command,
        ids::OperationId,
        output::OutputSpec,
        results::{ApiError, CommandResult, Health},
    };
    struct Stub(Vec<Command>);
    impl CliBackend for Stub {
        fn call(&mut self, command: Command, _: &OutputSpec) -> Result<CommandResult, ApiError> {
            self.0.push(command);
            Ok(CommandResult::Health(Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "test".into(),
                1,
            )))
        }
    }
    let rows: &[(&[&str], &str)] = &[
        (&["thread", "create", "--topic", "topic"], "create_thread"),
        (&["thread", "topic", "t1", "--set", "new"], "set_topic"),
        (&["invite", "t1", "--seat", "s1"], "invite"),
        (&["accept", "t1"], "accept"),
        (&["leave", "t1"], "leave"),
        (
            &["send", "t1", "--body", "hello\n雪", "--require-ack", "s1"],
            "send_message",
        ),
        (&["ack", "m1", "m2"], "ack"),
        (&["archive", "t1"], "archive"),
        (&["reopen", "t1"], "reopen"),
        (&["check-in"], "check_in"),
        (&["seat", "resolve", "--pane", "p1"], "resolve_seat"),
        (
            &["seat", "rebind", "s1", "--pane", "p1", "--operator"],
            "operator_rebind",
        ),
        (
            &[
                "seat",
                "resolve",
                "--pane",
                "p2",
                "--new-seat",
                "--operator",
            ],
            "operator_fresh_seat",
        ),
        (
            &["invite", "t1", "--seat", "s1", "--operator"],
            "operator_orphan_invite",
        ),
        (
            &["seat", "retire", "s1", "--operator"],
            "operator_retire",
        ),
        (
            &[
                "seat", "rebind", "s1", "--pane", "p1", "--replace", "s2", "--operator",
            ],
            "operator_replace",
        ),
    ];
    let mut stub = Stub(vec![]);
    for &(args, expected) in rows {
        let parsed =
            parse_argv(std::iter::once("herdr-threads").chain(args.iter().copied())).unwrap();
        assert!(matches!(parsed.action, CliAction::Mutation(_)), "{args:?}");
        dispatch(
            parsed,
            &mut stub,
            Some(claim()),
            Some(OperationId::new("op-row")),
        )
        .unwrap();
        let command = stub.0.last().unwrap();
        assert_eq!(
            serde_json::to_value(command).unwrap()["kind"],
            expected,
            "{args:?}"
        );
    }
}

#[test]
fn continuation_argv_round_trips_context_filter_format_and_bounds() {
    use crate::protocol::pagination::{Cursor, CursorDirection, CursorScope};
    let cursor = Cursor {
        instance: "instance-1".into(),
        scope: CursorScope::Directory,
        scope_key: "directory".into(),
        filter_digest: "filter-1".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 20,
        high_water_ordinal: 99,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .unwrap();
    let argv = vec![
        "herdr-threads".into(),
        "--state-dir".into(),
        "/tmp/state with space".into(),
        "--host-endpoint".into(),
        "/tmp/host.sock".into(),
        "--json".into(),
        "thread".into(),
        "list".into(),
        "--all".into(),
        "--search".into(),
        "雪's topic".into(),
        "--cursor".into(),
        cursor.clone(),
        "--limit".into(),
        "3".into(),
        "--max-bytes".into(),
        "600".into(),
    ];
    let parsed = parse_argv(argv).unwrap();
    assert_eq!(
        parsed.output.context.state_dir.as_deref(),
        Some("/tmp/state with space")
    );
    assert_eq!(
        parsed.output.context.host.unwrap().as_str(),
        "/tmp/host.sock"
    );
    assert_eq!(
        parsed.output.format,
        crate::protocol::output::OutputFormat::Json
    );
    assert!(
        matches!(parsed.action, CliAction::Wire(crate::protocol::commands::Command::Directory(q)) if
        q.membership_filter == crate::protocol::commands::DirectoryMembership::All &&
        q.topic_contains.as_deref() == Some("雪's topic") && q.page.cursor.as_deref() == Some(&cursor) &&
        q.page.limit == 3 && q.page.max_bytes == 600)
    );
}

#[test]
fn compound_and_local_page_continuations_keep_route_and_cursor() {
    use crate::protocol::pagination::{Cursor, CursorDirection, CursorScope};
    let routes: &[(&[&str], CursorScope)] = &[
        (&["thread", "show", "t1"], CursorScope::Participants),
        (&["seat", "inspect", "s1"], CursorScope::Diagnostics),
        (&["delivery", "inspect", "m1"], CursorScope::Recipients),
        (&["pending-ops"], CursorScope::LocalIntents),
        (&["view"], CursorScope::Directory),
    ];
    for &(route, scope) in routes {
        let cursor = Cursor {
            instance: "instance-1".into(),
            scope,
            scope_key: "key".into(),
            filter_digest: "digest".into(),
            direction: CursorDirection::Ascending,
            order_version: 1,
            last_examined_key: None,
            after_ordinal: 3,
            high_water_ordinal: 20,
            scope_revision: None,
            filter_revision: None,
            search: None,
            inbox: None,
            attention: None,
            binding: None,
        }
        .encode()
        .unwrap();
        let argv = std::iter::once("herdr-threads".to_string())
            .chain([
                "--state-dir".to_string(),
                "/tmp/state".into(),
                "--host-endpoint".into(),
                "/tmp/h.sock".into(),
            ])
            .chain(route.iter().map(|s| (*s).to_string()))
            .chain([
                "--cursor".to_string(),
                cursor.clone(),
                "--limit".into(),
                "2".into(),
                "--max-bytes".into(),
                "700".into(),
            ]);
        let parsed = parse_argv(argv).unwrap();
        let page = match parsed.action {
            CliAction::Wire(command) => command.page().unwrap().clone(),
            CliAction::PendingOps(page) | CliAction::View { page, .. } => page,
            other => panic!("wrong continuation route: {other:?}"),
        };
        assert_eq!(page.cursor.as_deref(), Some(cursor.as_str()), "{route:?}");
        assert_eq!((page.limit, page.max_bytes), (2, 700), "{route:?}");
    }
}

#[test]
fn local_command_table_uses_local_backend_without_wire_call() {
    use crate::protocol::{
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CommandResult, Health},
    };
    struct Stub(Vec<LocalAction>);
    impl CliBackend for Stub {
        fn call(&mut self, _: Command, _: &OutputSpec) -> Result<CommandResult, ApiError> {
            panic!("wire call")
        }
        fn local(
            &mut self,
            action: LocalAction,
            _: &OutputSpec,
        ) -> Result<CommandResult, ApiError> {
            self.0.push(action);
            Ok(CommandResult::Health(Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "test".into(),
                1,
            )))
        }
    }
    let rows: &[&[&str]] = &[
        &["pending-ops", "--limit", "3"],
        &["retry", "local:R_123"],
        &["view", "--once", "--limit", "3"],
        &["daemon", "health"],
        &["daemon", "ensure"],
        &["daemon", "stop"],
        &["doctor"],
    ];
    let mut stub = Stub(vec![]);
    for args in rows {
        let parsed =
            parse_argv(std::iter::once("herdr-threads").chain(args.iter().copied())).unwrap();
        dispatch(parsed, &mut stub, None, None).unwrap();
    }
    assert_eq!(stub.0.len(), rows.len());
}

#[test]
fn local_composition_error_names_skill() {
    use crate::protocol::{
        commands::Command,
        output::OutputSpec,
        results::{ApiError, CommandResult},
    };
    struct Stub;
    impl CliBackend for Stub {
        fn call(&mut self, _: Command, _: &OutputSpec) -> Result<CommandResult, ApiError> {
            panic!("wire call")
        }
    }
    let parsed = parse_argv(["herdr-threads", "--skill"]).unwrap();
    let err = dispatch(parsed, &mut Stub, None, None).unwrap_err();
    assert!(err.detail.contains("skill"), "{}", err.detail);
    assert!(err.detail.contains("launch") && err.detail.contains("setup"), "{}", err.detail);
}

#[test]
fn help_names_exact_ack_separate_accept_and_optional_summary() {
    use clap::CommandFactory;
    let help = Cli::command().render_long_help().to_string();
    assert!(help.contains("ACK exact message IDs"));
    assert!(help.contains("Accept invitations separately"));
    assert!(help.contains("cheap subagents"));
}
#[test]
fn cooperative_selection_requires_explicit_complete_context() {
    let parsed = crate::cli::commands::parse_argv([
        "herdr-threads", "--cooperative-seat", "seat-1", "--cooperative-target", "w:p1",
        "--cooperative-harness", "codex", "--cooperative-role", "top-level",
        "check-in", "--lifecycle-event", "launch-1",
    ]).unwrap();
    let selection = parsed.cooperative.unwrap();
    assert_eq!(selection.seat.as_str(), "seat-1");
    assert_eq!(selection.target.as_str(), "w:p1");
    assert_eq!(selection.harness, crate::harness::context::Harness::Codex);
    assert_eq!(selection.role, crate::harness::context::Role::TopLevel);
    assert!(crate::cli::commands::parse_argv([
        "herdr-threads", "--cooperative-seat", "seat-1", "check-in",
        "--lifecycle-event", "launch-1",
    ]).is_err());
}

// Demo-1 P1: the hook's digest lists `ITEM@THREAD`; a model passed that form to
// `accept` and got a misleading `current invitation missing (not_found)`.
// Kills: removing the digest-form check from thread or message arguments (the
// value parses as an opaque ID and reaches the service), or an error that does
// not name the bare ID to pass instead.
#[test]
fn digest_item_form_is_refused_as_invalid_arguments_naming_the_bare_id() {
    let inv = "invitation-1@thread-9";
    for (argv, bare) in [
        (vec!["herdr-threads", "accept", inv], "herdr-threads accept thread-9"),
        (
            vec![
                "herdr-threads",
                "accept-required",
                inv,
                "--invitation",
                "invitation-1",
                "--requirement",
                "requirement-1",
                "--revision",
                "1",
            ],
            "`thread-9`",
        ),
        (vec!["herdr-threads", "read", "msg-1@thread-9"], "`thread-9`"),
        (
            vec!["herdr-threads", "thread", "participants", inv],
            "`thread-9`",
        ),
        (
            vec!["herdr-threads", "ack", "msg-2", "msg-1@thread-9"],
            "herdr-threads ack msg-1",
        ),
    ] {
        let Err(error) = parse_argv(argv.clone()) else {
            panic!("{argv:?} parsed")
        };
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{argv:?}");
        assert!(error.detail.contains("attention digest item"), "{}", error.detail);
        assert!(error.detail.contains(bare), "{}", error.detail);
    }
    // Bare IDs still parse.
    let parsed = parse_argv(vec!["herdr-threads", "accept", "thread-9"]).unwrap();
    assert!(matches!(
        parsed.action,
        CliAction::Mutation(MutationSpec::Accept(ref t)) if t.as_str() == "thread-9"
    ));
    assert!(parse_argv(vec!["herdr-threads", "ack", "msg-1"]).is_ok());
}

// Native codex matrix O1 (repeats demo 3 P4): models guess
// `herdr-threads participants THREAD`; it is an alias of `thread
// participants`. Kills: removing the alias or routing it elsewhere.
#[test]
fn top_level_participants_aliases_thread_participants() {
    let alias = parse_argv(["herdr-threads", "participants", "t1", "--limit", "7"]).unwrap();
    let full = parse_argv([
        "herdr-threads",
        "thread",
        "participants",
        "t1",
        "--limit",
        "7",
    ])
    .unwrap();
    assert_eq!(alias, full);
    assert!(
        matches!(&alias.action, CliAction::Wire(WireCommand::Participants(q)) if q.thread.as_str() == "t1" && q.caller.is_none())
    );
}

#[test]
fn read_follow_parses_its_options_and_refuses_page_selectors() {
    let parsed = parse_argv(["herdr-threads", "read", "thread-1", "--follow"]).unwrap();
    assert_eq!(
        parsed.action,
        CliAction::Follow(FollowRequest {
            thread: ThreadId::new("thread-1"),
            recent: FOLLOW_DEFAULT_RECENT,
            after: None,
            no_system: false,
        })
    );
    let parsed = parse_argv([
        "herdr-threads", "read", "thread-1", "--follow", "--recent", "30", "--no-system",
    ])
    .unwrap();
    let CliAction::Follow(request) = parsed.action else {
        panic!("expected follow")
    };
    assert_eq!((request.recent, request.no_system), (30, true));
    let parsed =
        parse_argv(["herdr-threads", "read", "thread-1", "--follow", "--after", "7"]).unwrap();
    let CliAction::Follow(request) = parsed.action else {
        panic!("expected follow")
    };
    assert_eq!((request.recent, request.after), (0, Some(7)));
    for refused in [
        vec!["herdr-threads", "read", "thread-1", "--follow", "--before", "3"],
        vec!["herdr-threads", "read", "thread-1", "--follow", "--limit", "3"],
        vec!["herdr-threads", "read", "thread-1", "--follow", "--cursor", "c2:x"],
        vec!["herdr-threads", "read", "thread-1", "--follow", "--recent", "101"],
        vec!["herdr-threads", "read", "thread-1", "--no-system"],
    ] {
        assert!(parse_argv(refused.clone()).is_err(), "{refused:?}");
    }
}

#[test]
fn exit_status_help_names_both_exit_3_remedies() {
    use crate::daemon::remedy::{RemedyContext, remedy};
    use crate::protocol::results::ErrorClass;
    let help = exit_status_help();
    assert!(
        help.contains(&format!(
            "  3  daemon or host unavailable; {}",
            remedy(Some(ErrorClass::Unavailable), &RemedyContext::Exit3)
        )),
        "{help}"
    );
    assert!(
        help.contains(&format!(
            "(version mismatch: {})",
            remedy(Some(ErrorClass::VersionSkew), &RemedyContext::Exit3)
        )),
        "{help}"
    );
}

#[test]
fn seat_retire_requires_operator_flag() {
    let error = parse_argv(["herdr-threads", "seat", "retire", "s1"]).unwrap_err();
    assert!(error.detail.contains("operator required"), "{}", error.detail);
    let parsed = parse_argv(["herdr-threads", "seat", "retire", "s1", "--operator"]).unwrap();
    assert!(
        matches!(&parsed.action, CliAction::Mutation(MutationSpec::Retire(seat)) if seat.as_str() == "s1"),
        "{:?}",
        parsed.action
    );
}

#[test]
fn seat_rebind_replace_parses_to_operator_replace() {
    let parsed = parse_argv([
        "herdr-threads",
        "seat",
        "rebind",
        "s1",
        "--pane",
        "p1",
        "--replace",
        "s2",
        "--operator",
    ])
    .unwrap();
    assert!(
        matches!(&parsed.action, CliAction::Mutation(MutationSpec::Replace { seat, pane, replace })
            if seat.as_str() == "s1" && pane.as_str() == "p1" && replace.as_str() == "s2"),
        "{:?}",
        parsed.action
    );
    // Without --replace the same form stays a plain rebind; --replace needs --operator.
    let plain = parse_argv(["herdr-threads", "seat", "rebind", "s1", "--pane", "p1", "--operator"])
        .unwrap();
    assert!(matches!(
        plain.action,
        CliAction::Mutation(MutationSpec::Rebind { .. })
    ));
    assert!(
        parse_argv(["herdr-threads", "seat", "rebind", "s1", "--pane", "p1", "--replace", "s2"])
            .is_err()
    );
}
