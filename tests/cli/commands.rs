use super::*;

#[test]
fn doctor_debug_and_fix_are_scoped_to_doctor() {
    let plain = parse_argv(["herdr-threads", "doctor"]).unwrap();
    assert!(matches!(plain.action, CliAction::Doctor { debug: false, fix: false, .. }));
    let debug = parse_argv(["herdr-threads", "doctor", "--debug"]).unwrap();
    assert!(matches!(debug.action, CliAction::Doctor { debug: true, fix: false, .. }));
    let fix = parse_argv(["herdr-threads", "--json", "doctor", "fix"]).unwrap();
    assert!(matches!(fix.action, CliAction::Doctor { debug: false, fix: true, .. }));
    assert_eq!(fix.output.format, OutputFormat::Json);
    assert!(parse_argv(["herdr-threads", "doctor", "repair"]).is_err());
}

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
        (
            vec!["herdr-threads", "warnings", "--active", "t1", "--limit", "7"],
            |c| matches!(c,WireCommand::ActiveWarnings(q) if q.thread.as_str()=="t1" && q.page.limit==7),
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
    dispatch(freeze_thread_id(parsed), &mut stub, None, None).unwrap();
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
        "human",
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
        (&["thread", "list", "--all"], "directory"),
        (&["thread", "show", "t1"], "thread"),
        (&["inbox", "--seat", "s1"], "inbox"),
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
        dispatch(freeze_thread_id(parsed), &mut stub, None, None).unwrap();
        let command = stub.0.last().unwrap();
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["kind"], expected, "{args:?}");
    }
}

#[test]
fn warnings_route_defaults_to_caller_and_requires_valid_page() {
    use crate::protocol::commands::Command;
    assert!(parse_argv(["herdr-threads", "warnings"]).unwrap().caller_read_default);
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
    let disconnect = parse_argv(["herdr-threads", "human", "service", "disconnect", "--expected-boot", boot, "--expected-generation", "7"]).unwrap();
    assert!(matches!(disconnect.action, CliAction::Wire(Command::ServiceDisconnect(request)) if request.expected_boot == boot && request.expected_generation == 7));
    assert!(parse_argv(["herdr-threads", "human", "service", "disconnect", "--expected-boot", boot]).is_err());
}

#[test]
fn service_help_explains_recovery_workflow_and_safety_boundary() {
    fn help(args: &[&str]) -> String {
        match parse_argv_or_informational(args.iter().copied()).unwrap_err() {
            ParseFailure::Informational(text) => text,
            other => panic!("expected help, got {other:?}"),
        }
    }
    let root = help(&["herdr-threads", "--help"]);
    assert!(root.contains("service") && root.contains("Inspect or disconnect"));
    let group = help(&["herdr-threads", "service", "--help"]);
    assert!(group.contains("live service connection"));
    assert!(group.contains("service inspect"));
    assert!(group.contains("service disconnect"));
    let inspect = help(&["herdr-threads", "service", "inspect", "--help"]);
    assert!(inspect.contains("does not attest agent liveness"));
    let disconnect = help(&["herdr-threads", "service", "disconnect", "--help"]);
    assert!(disconnect.contains("--expected-boot"));
    assert!(disconnect.contains("--expected-generation"));
    assert!(disconnect.contains("service inspect"));
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
    assert!(dispatch(freeze_thread_id(parsed), &mut Stub, None, None).is_err());
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
    let CliAction::Mutation(MutationSpec::Create { topic, goal, .. }) = parsed.action else {
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
        (&["seat", "resolve", "--pane", "w1:p1"], "resolve_seat"),
        (
            &["seat", "rebind", "s1", "--pane", "w1:p1", "--operator"],
            "operator_rebind",
        ),
        (
            &[
                "seat",
                "resolve",
                "--pane",
                "w1:p2",
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
                "seat", "rebind", "s1", "--pane", "w1:p1", "--replace", "s2", "--operator",
            ],
            "operator_replace",
        ),
    ];
    let mut stub = Stub(vec![]);
    for &(args, expected) in rows {
        let parsed =
            parse_argv(std::iter::once("herdr-threads").chain(expected.starts_with("operator_").then_some("human")).chain(args.iter().copied())).unwrap();
        assert!(matches!(parsed.action, CliAction::Mutation(_)), "{args:?}");
        dispatch(
            freeze_thread_id(parsed),
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
        dispatch(freeze_thread_id(parsed), &mut stub, None, None).unwrap();
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
    let err = dispatch(freeze_thread_id(parsed), &mut Stub, None, None).unwrap_err();
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
        let error = if argv[1] == "ack" {
            parse_argv(argv.clone()).unwrap_err()
        } else {
            let mut parsed = parse_argv(argv.clone()).unwrap();
            let selector = parsed.thread_selector.clone().unwrap();
            crate::cli::threads::resolve_cli_threads(&mut parsed, |_| {
                Err::<ThreadId,_>(crate::cli::threads::selector_error(ApiError::not_found("thread not found"), &selector))
            }).unwrap_err()
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

#[test]
fn copied_compact_and_persisted_ids_parse_as_command_inputs() {
    for thread in ["tAb12Cd34", "thread-Ab12Cd34"] {
        let accept = parse_argv(["herdr-threads", "accept", thread]).unwrap();
        assert!(matches!(
            accept.action,
            CliAction::Mutation(MutationSpec::Accept(ref id)) if id.as_str() == thread
        ));
        assert!(parse_argv(["herdr-threads", "read", thread]).is_ok());
        assert!(parse_argv(["herdr-threads", "send", thread, "--body", "hi"]).is_ok());
    }
    for message in ["mAb12Cd34", "msg-Ab12Cd34"] {
        assert!(parse_argv(["herdr-threads", "ack", message]).is_ok());
        assert!(parse_argv(["herdr-threads", "body", message]).is_ok());
    }
    for message in [
        "eAb12Cd34",
        "event-Ab12Cd34",
        "nAb12Cd34",
        "notify-Ab12Cd34",
        "wbc121d12-9d30-8510-b93c-e7cd26e890f1",
        "warning-bc121d12-9d30-8510-b93c-e7cd26e890f1",
    ] {
        assert!(parse_argv(["herdr-threads", "body", message]).is_ok());
    }
    for (invitation, requirement) in [
        ("iAb12Cd34", "qAb12Cd34"),
        ("inv-Ab12Cd34", "requirement-Ab12Cd34"),
    ] {
        assert!(
            parse_argv([
                "herdr-threads",
                "accept-required",
                "tAb12Cd34",
                "--invitation",
                invitation,
                "--requirement",
                requirement,
                "--revision",
                "4",
            ])
            .is_ok()
        );
    }
    for seat in ["sAb12Cd34", "seat-Ab12Cd34"] {
        assert!(
            parse_argv(["herdr-threads", "human", "seat", "retire", seat, "--operator"]).is_ok()
        );
    }
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

// Kills: follow routed differently, lost selectors/options, or history-only flags admitted.
#[test]
fn top_level_follow_matches_read_follow_for_exact_names_and_picker() {
    let cases: &[&[&str]] = &[
        &[],
        &["thread-1"],
        &["review channel", "--recent", "0"],
        &[
            "review channel",
            "--recent",
            "30",
            "--no-system",
            "--max-bytes",
            "512",
        ],
        &["thread-1", "--after", "7"],
        &["--recent", "9", "--no-system", "--max-bytes", "512"],
        &["--after", "7"],
    ];
    for executable in ["herdr-threads", "ht"] {
        for args in cases {
            for format in [None, Some("--human"), Some("--machine"), Some("--json")] {
                let mut alias = vec![executable];
                alias.extend(format);
                alias.push("follow");
                alias.extend_from_slice(args);
                let mut full = vec![executable];
                full.extend(format);
                full.extend(["read", "--follow"]);
                full.extend_from_slice(args);
                let parsed = parse_argv(alias.clone())
                    .unwrap_or_else(|error| panic!("{alias:?}: {error:?}"));
                assert_eq!(parsed, parse_argv(full).unwrap(), "{alias:?}");
                if let Some(thread) = args.first().filter(|arg| !arg.starts_with("--")) {
                    assert_eq!(parsed.thread_selector.as_deref(), Some(*thread));
                    assert!(matches!(parsed.action, CliAction::Follow(_)));
                } else {
                    assert!(matches!(
                        parsed.action,
                        CliAction::Picker(PickerRequest { follow: true, .. })
                    ));
                }
            }
        }
    }
}

#[test]
fn top_level_follow_refuses_history_only_or_conflicting_ranges() {
    for args in [
        vec!["--before", "3"],
        vec!["--limit", "3"],
        vec!["--cursor", "c3:x"],
        vec!["--recent", "101"],
        vec!["--recent", "2", "--after", "7"],
    ] {
        for thread in [None, Some("thread-1")] {
            let mut argv = vec!["herdr-threads", "follow"];
            argv.extend(thread);
            argv.extend_from_slice(&args);
            assert!(parse_argv(argv.clone()).is_err(), "{argv:?}");
        }
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
    let error = parse_argv(["herdr-threads", "human", "seat", "retire", "s1"]).unwrap_err();
    assert!(error.detail.contains("operator required"), "{}", error.detail);
    let parsed = parse_argv(["herdr-threads", "human", "seat", "retire", "s1", "--operator"]).unwrap();
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
        "human",
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
    let plain = parse_argv(["herdr-threads", "human", "seat", "rebind", "s1", "--pane", "p1", "--operator"])
        .unwrap();
    assert!(matches!(
        plain.action,
        CliAction::Mutation(MutationSpec::Rebind { .. })
    ));
    assert!(
        parse_argv(["herdr-threads", "human", "seat", "rebind", "s1", "--pane", "p1", "--replace", "s2"])
            .is_err()
    );
}

#[test]
fn send_relays_user_flag_reaches_the_command_and_is_off_by_default() {
    let send_command = |extra: &[&str]| {
        let mut argv = vec!["herdr-threads", "send", "t1", "--body", "x"];
        argv.extend_from_slice(extra);
        let parsed = parse_argv(argv).unwrap();
        let CliAction::Mutation(spec) = parsed.action else {
            panic!("expected send mutation")
        };
        let flag = matches!(spec, MutationSpec::Send { relays_user, .. } if relays_user);
        let command = spec
            .into_command(
                Some(claim()),
                crate::protocol::ids::OperationId::new("op-relay"),
            )
            .unwrap();
        let crate::protocol::commands::Command::SendMessage(send) = command else {
            panic!("expected typed send request")
        };
        (flag, send.relays_user)
    };
    assert_eq!(send_command(&["--relays-user"]), (true, true));
    assert_eq!(send_command(&[]), (false, false));
}

// Catches the CLI dropping classification or incorrectly requiring relay locally.
#[test]
fn user_intent_send_cli_parses_and_dispatches() {
    use crate::protocol::summary::UserIntent;
    struct SendStub(Option<SendMessage>);
    impl CliBackend for SendStub {
        fn call(&mut self, command: WireCommand, _: &OutputSpec) -> Result<CommandResult, ApiError> {
            let WireCommand::SendMessage(send) = command else { panic!("expected send command") };
            self.0 = Some(send);
            Ok(CommandResult::MessageSent(MessageId::new("sent")))
        }
    }
    let dispatch_send = |argv: Vec<&str>, expected, relay| {
        let parsed = parse_argv(argv).unwrap();
        assert!(matches!(&parsed.action, CliAction::Mutation(MutationSpec::Send { user_intent, relays_user, .. })
            if *user_intent == expected && *relays_user == relay));
        let mut stub = SendStub(None);
        dispatch(freeze_thread_id(parsed), &mut stub, Some(claim()), Some(OperationId::new("intent-send"))).unwrap();
        let sent = stub.0.unwrap();
        assert_eq!(sent.user_intent, expected);
        assert_eq!(sent.relays_user, relay);
    };
    for (spelling, intent) in [
        ("query", UserIntent::Query),
        ("request", UserIntent::Request),
        ("rule", UserIntent::Rule),
    ] {
        for relay in [false, true] {
            let mut argv = vec!["herdr-threads", "send", "t1", "--body", "x", "--user-intent", spelling];
            if relay { argv.push("--relays-user"); }
            dispatch_send(argv, Some(intent), relay);
        }
    }
    dispatch_send(vec!["herdr-threads", "send", "t1", "--body", "x"], None, false);
    for invalid in ["instruction", "Query", "REQUEST", ""] {
        assert!(parse_argv(["herdr-threads", "send", "t1", "--body", "x", "--user-intent", invalid]).is_err(),
            "accepted invalid user intent {invalid:?}");
    }
}

#[test]
fn send_help_names_relays_user() {
    use clap::CommandFactory;
    let mut cli = Cli::command();
    let help = cli
        .find_subcommand_mut("send")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(help.contains("--relays-user"));
    assert!(help.contains("input from your user"));
    assert!(help.contains("--user-intent"));
    assert!(help.contains("ordinary humans may set it directly"));
    assert!(help.contains("agents require --relays-user"));
    assert!(help.contains("query|request|rule"));
    assert!(help.contains("Missing intent stays unclassified"));
    assert!(help.contains("classification grants no permission"));
}

#[test]
fn contract_id_and_harness_version_parse_as_local_actions() {
    use crate::harness::context::Harness;
    let parsed = parse_argv(["herdr-threads", "contract-id"]).unwrap();
    assert_eq!(parsed.action, CliAction::ContractId { harness: None });
    let parsed = parse_argv(["herdr-threads", "contract-id", "--harness", "codex"]).unwrap();
    assert_eq!(
        parsed.action,
        CliAction::ContractId {
            harness: Some(Harness::Codex)
        }
    );
    let parsed = parse_argv([
        "herdr-threads",
        "harness-version",
        "normalize",
        "codex",
        "codex-cli 0.158.0",
    ])
    .unwrap();
    assert_eq!(
        parsed.action,
        CliAction::HarnessVersionNormalize {
            harness: Harness::Codex,
            raw: "codex-cli 0.158.0".into()
        }
    );
    assert!(parse_argv(["herdr-threads", "contract-id", "--harness", "gemini"]).is_err());
    assert!(
        parse_argv(["herdr-threads", "harness-version", "normalize", "gemini", "1.0.0"]).is_err()
    );
}

#[test]
fn contract_id_runs_without_a_daemon_and_prints_exact_documents() {
    use crate::harness::{claude, codex, contract::contract_id};
    let run = |args: &[&str]| -> Result<String, String> {
        let mut out = Vec::new();
        crate::cli::run(
            std::iter::once("herdr-threads").chain(args.iter().copied()),
            &mut out,
        )
        .map_err(|e| e.to_string())?;
        Ok(String::from_utf8(out).unwrap())
    };
    let claude_id = contract_id(&claude::CONTRACT);
    let codex_id = contract_id(&codex::CONTRACT);
    assert_eq!(
        run(&["contract-id"]).unwrap(),
        format!("claude {claude_id}\ncodex {codex_id}\n")
    );
    assert_eq!(
        run(&["contract-id", "--harness", "codex"]).unwrap(),
        format!("codex {codex_id}\n")
    );
    let doc: serde_json::Value = serde_json::from_str(&run(&["contract-id", "--json"]).unwrap()).unwrap();
    let keys: Vec<&str> = doc.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["claude", "codex", "normalize"]);
    assert_eq!(doc["claude"], claude_id);
    assert_eq!(doc["codex"], codex_id);
    assert_eq!(
        run(&["harness-version", "normalize", "codex", "codex-cli 0.158.0"]).unwrap(),
        "0.158.0\n"
    );
    let json = run(&[
        "harness-version",
        "normalize",
        "claude",
        "2.1.286 (Claude Code)",
        "--json",
    ])
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        doc,
        serde_json::json!({"harness":"claude","raw":"2.1.286 (Claude Code)","version":"2.1.286"})
    );
    let err = run(&["harness-version", "normalize", "codex", "codex-cli 0.160.0-alpha.1"]).unwrap_err();
    assert!(
        err.contains("unrecognized codex version: codex-cli 0.160.0-alpha.1"),
        "{err}"
    );
}

/// A local selector cannot silently choose an adapter, use a native launch
/// profile, or pass through unsupported scope/option metadata.
#[test]
fn local_profile_and_options_are_validated_before_dispatch() {
    for command in ["setup", "unsetup", "setup-status"] {
        let bare = parse_argv(["herdr-threads", command, "--profile", "work"]).unwrap_err();
        assert!(bare.detail.contains("explicitly selected adapter"));
        for harness in ["claude", "codex"] {
            let named = parse_argv(["herdr-threads", command, harness, "--profile", "work"]).unwrap_err();
            assert!(named.detail.contains("named profile is unsupported"));
        }
    }
    let doctor = parse_argv(["herdr-threads", "doctor", "--profile", "work"]).unwrap_err();
    assert!(doctor.detail.contains("explicitly selected adapter"));
    let doctor = parse_argv(["herdr-threads", "doctor", "--harness", "codex", "--profile", "work"]).unwrap_err();
    assert!(doctor.detail.contains("named profile is unsupported"));
    assert!(parse_argv(["herdr-threads", "setup", "unknown"]).is_err());
    assert!(parse_argv(["herdr-threads", "setup", "codex", "--disable-prompt-suggestions"]).is_err());
    assert!(parse_argv(["herdr-threads", "setup", "claude", "--disable-prompt-suggestions", "--keep-prompt-suggestions"]).is_err());
    let bare = parse_argv(["herdr-threads", "setup", "--disable-prompt-suggestions"]).unwrap();
    assert!(matches!(bare.action, CliAction::SetupAll(_, crate::cli::setup::PromptSuggestionPolicy::Disable)));
}

#[test]
fn rejection_requires_exact_invitation_and_bounded_nonblank_reason() {
    let parsed = parse_argv([
        "herdr-threads", "reject", "t1", "--invitation", "iv1", "--reason",
        "Outside my reviewer role",
    ]);
    assert!(parsed.is_ok(), "ordinary invitation rejection must parse: {parsed:?}");
    assert!(parse_argv(["herdr-threads", "reject", "t1", "--reason", "irrelevant"]).is_err());
    assert!(parse_argv(["herdr-threads", "reject", "t1", "--invitation", "iv1"]).is_err());
    assert!(parse_argv(["herdr-threads", "reject", "t1", "--invitation", "iv1", "--reason", " \n "]).is_err());
    let too_long = "x".repeat(4097);
    assert!(parse_argv(["herdr-threads", "reject", "t1", "--invitation", "iv1", "--reason", &too_long]).is_err());
}

#[test]
fn thread_names_public_create_and_selectors() {
    let named = parse_argv(["ht", "thread", "create", "--topic", "Announcements", "--name", "team café"]);
    assert!(named.is_ok(), "optional names must parse: {named:?}");
    for argv in [
        vec!["ht", "thread", "show", "team café"],
        vec!["ht", "thread", "name", "team café"],
        vec!["ht", "thread", "rename", "team café", "new name"],
        vec!["ht", "read", "team café"],
        vec!["ht", "summary", "team café"],
    ] {
        assert!(parse_argv(argv.clone()).is_ok(), "selector must parse: {argv:?}");
    }
}

#[test]
fn thread_names_every_selector_freezes_one_literal_id() {
    use crate::cli::threads::{resolve_cli_threads, selector_mut};
    let cases = [
        vec!["ht","thread","show","review"], vec!["ht","thread","topic","review"],
        vec!["ht","thread","topic","review","--set","topic"], vec!["ht","thread","participants","review"],
        vec!["ht","participants","review"], vec!["ht","invite","review","--seat","s1"],
        vec!["ht","reject","review","--invitation","v1","--reason","Outside my role"],
        vec!["ht","accept","review"], vec!["ht","accept-required","review","--invitation","v1","--requirement","q1","--revision","1"],
        vec!["ht","leave","review"], vec!["ht","send","review","--body","ready"],
        vec!["ht","archive","review"], vec!["ht","reopen","review"], vec!["ht","read","review"],
        vec!["ht","read","review","--follow"], vec!["ht","pending-receipts","--thread","review"],
        vec!["ht","diagnostics","--thread","review"], vec!["ht","warnings","--active","review"],
        vec!["ht","search","release","--thread","review"], vec!["ht","summary","review"],
        vec!["ht","thread","name","review"], vec!["ht","thread","name","review","--set","new"],
        vec!["ht","thread","name","review","--clear"], vec!["ht","thread","rename","review","new"],
    ];
    for argv in cases {
        let mut parsed = parse_argv(argv.clone()).unwrap();
        let mut calls = 0;
        resolve_cli_threads(&mut parsed, |selector| {
            assert_eq!(selector,"review", "{argv:?}"); calls += 1;
            Ok::<_,ApiError>(ThreadId::new("tFREEZE01"))
        }).unwrap();
        assert_eq!(calls,1,"{argv:?}");
        assert_eq!(selector_mut(&mut parsed.action).unwrap().as_str(),"tFREEZE01","{argv:?}");

    }
}

#[test]
fn thread_names_validate_bytes_controls_and_exact_spaces() {
    for name in ["".to_owned(), "x".repeat(129), "é".repeat(65), "line\nname".into(), "tab\tname".into(), "delete\u{7f}".into(), "unicode\u{85}".into(), "line\u{2028}name".into(), "paragraph\u{2029}name".into()] {
        assert!(parse_argv(["ht","thread","create","--topic","topic","--name",&name]).is_err(),"{name:?}");
        assert!(parse_argv(["ht","thread","name","t1","--set",&name]).is_err(),"{name:?}");
    }
    for name in ["x".repeat(128), "é".repeat(64), "  exact spaces  ".into(), "m1@t1".into()] {
        let parsed = parse_argv(["ht","thread","create","--topic","topic","--name",&name]).unwrap();
        assert!(matches!(parsed.action,CliAction::Mutation(MutationSpec::Create {name:Some(value),..}) if value==name));
        assert!(parse_argv(["ht","read",&name]).is_ok());
    }
    assert!(parse_argv(["ht","thread","name","t1","--set","x","--clear"]).is_err());
}

// Typed dispatch fixtures explicitly freeze exact IDs; runtime uses the daemon.
fn freeze_thread_id(mut parsed: ParsedCli) -> ParsedCli {
    crate::cli::threads::resolve_cli_threads(&mut parsed, |id| Ok::<_,ApiError>(ThreadId::new(id))).unwrap();
    parsed
}

#[test]
fn thread_names_unresolved_dispatch_refuses_before_any_backend_call() {
    struct Never;
    impl CliBackend for Never {
        fn call(&mut self,_:WireCommand,_:&OutputSpec)->Result<CommandResult,ApiError> {panic!("unresolved selector must not dispatch")}
    }
    for selector in ["review","team café","tIDSHAPED"] {
        let parsed = parse_argv(["ht","read",selector]).unwrap();
        assert!(super::dispatch(parsed,&mut Never,None,None).is_err());
    }
}

#[test]
fn handoff_parser_requires_one_thread_explicit_pane_and_one_body() {
    let base = ["ht", "handoff", "--new-thread", "--pane", "bob", "--kind", "codex"];
    let mut good = base.to_vec();
    good.extend(["--", "durable task"]);
    assert!(parse_argv(good).is_ok());
    for tail in [vec![], vec!["--", "one", "two"], vec!["--thread", "review", "--", "task"]] {
        let mut args = base.to_vec(); args.extend(tail);
        assert!(parse_argv(args).is_err());
    }
    assert!(parse_argv(["ht", "handoff", "--new-thread", "--kind", "codex", "--", "task"]).is_err());
    for option in ["--thread-name", "--topic", "--goal"] {
        assert!(parse_argv(["ht", "handoff", "--thread", "review", "--pane", "bob", "--kind", "codex", option, "value", "--", "task"]).is_err());
    }
}
#[test]
fn handoff_parser_preserves_native_elements_and_distinct_names() {
    let parsed = parse_argv(["ht", "handoff", "--new-thread", "--thread-name", "review", "--name", "worker", "--pane", "bob", "--kind", "codex", "--agent-arg=-a", "--agent-arg=on-request", "--agent-arg=literal spaces", "--", "one durable body"]).unwrap();
    let CliAction::Handoff(request) = parsed.action else { panic!("not handoff"); };
    assert_eq!(request.launch.argv, ["-a", "on-request", "literal spaces"]);
    assert_eq!(request.launch.name.as_deref(), Some("worker"));
    assert_eq!(request.thread_name.as_deref(), Some("review"));
    assert_eq!(request.body, "one durable body");
}

#[test]
fn handoff_thread_name_uses_shared_canonical_resolution() {
    let mut parsed=parse_argv(["ht","handoff","--thread","review channel","--pane","bob","--kind","codex","--","task"]).unwrap();
    assert_eq!(parsed.thread_selector.as_deref(),Some("review channel"));
    crate::cli::threads::resolve_cli_threads::<ApiError>(&mut parsed, |selector| { assert_eq!(selector,"review channel"); Ok(ThreadId::new("tFrozen")) }).unwrap();
    assert!(parsed.thread_selector.is_none());
    assert!(matches!(parsed.action,CliAction::Handoff(request) if request.thread.as_ref().unwrap().as_str()=="tFrozen"));
}

#[test]
fn handoff_new_tab_grammar() {
    for target in [vec!["--new-tab", "peer"], vec!["--new-tab", "peer", "--space", "work", "--cwd", "/tmp"]] {
        let mut argv = vec!["herdr-threads", "handoff", "--new-thread", "--kind", "codex"];
        argv.extend(target);
        argv.extend(["--agent-arg=--model", "--agent-arg=literal $HOME", "--", "quoted work"]);
        assert!(parse_argv(argv).is_ok(), "valid new-tab grammar must parse");
    }
}
#[test]
fn handoff_new_tab_conflicts_and_required_fields() {
    for extra in [vec!["--existing"], vec!["--tab", "t1"], vec!["--pane", "w1:p1"], vec!["--seat", "peer"]] {
        let mut argv=vec!["herdr-threads", "handoff", "--new-tab", "peer", "--new-thread", "--kind", "codex"];
        argv.extend(extra); argv.extend(["--", "work"]);
        assert!(parse_argv(argv.clone()).is_err(), "accepted conflict: {argv:?}");
    }
    for argv in [
        vec!["herdr-threads", "handoff", "--new-tab", "peer", "--new-thread", "--", "work"],
        vec!["herdr-threads", "handoff", "--new-tab", "peer", "--kind", "codex", "--", "work"],
        vec!["herdr-threads", "handoff", "--new-tab", "peer", "--new-thread", "--kind", "codex"],
        vec!["herdr-threads", "handoff", "--pane", "w1:p1", "--cwd", "/tmp", "--new-thread", "--kind", "codex", "--", "work"],
    ] { assert!(parse_argv(argv.clone()).is_err(), "accepted missing/conflict: {argv:?}"); }
}

#[test]
fn handoff_existing_target_matrix_and_no_launch_options() {
    for target in [vec!["--pane", "w1:p1"], vec!["--seat", "peer"], vec!["--space", "work", "--tab", "tab", "--pane", "peer"]] {
        let mut argv=vec!["herdr-threads","handoff","--existing","--thread","selected"];
        argv.extend(target); argv.extend(["--","work"]);
        assert!(parse_argv(argv).is_ok());
    }
    for target in [vec![],vec!["--space","work"],vec!["--tab","tab"],vec!["--seat","peer","--pane","w1:p1"],vec!["--seat","peer","--space","work"],vec!["--pane","w1:p1","--kind","codex"],vec!["--pane","w1:p1","--agent-arg=--model"],vec!["--pane","w1:p1","--harness-binary","/bin/codex"],vec!["--pane","w1:p1","--name","peer"],vec!["--pane","w1:p1","--cwd","/tmp"]] {
        let mut argv=vec!["herdr-threads","handoff","--existing","--thread","selected"];
        argv.extend(target); argv.extend(["--","work"]);
        assert!(parse_argv(argv.clone()).is_err(),"accepted: {argv:?}");
    }
    let parsed=parse_argv(["herdr-threads","handoff","--new-tab","peer","--new-thread","--kind","codex","--agent-arg=--model","--agent-arg=$HOME","--agent-arg=--model","--","literal body"]).unwrap();
    let CliAction::TopologyHandoff(request)=parsed.action else {panic!("wrong route")};
    assert_eq!(request.argv,vec!["--model","$HOME","--model"]);
    assert_eq!(request.body,"literal body");
    assert!(parsed.thread_selector.is_none());
}

// Removing immediate actor routing would reject honest Human invocations and
// accept ordinary Agent person/account actions; these are real parser calls.
#[test]
fn actor_prerequisite_human_grammar_preserves_routing_and_format() {
    let parsed = parse_argv(["ht", "human", "--state-dir", "state space", "--host-endpoint=host space", "--machine", "me", "init"]).unwrap();
    assert!(matches!(parsed.action, CliAction::MeInit { .. }));
    assert_eq!(parsed.output.context.state_dir.as_deref(), Some("state space"));
    assert_eq!(parsed.output.context.host.as_deref(), Some("host space"));
    for argv in [
        vec!["ht", "me", "init"],
        vec!["ht", "--human", "me", "init"],
        vec!["ht", "seat", "retire", "seat-1", "--operator"],
        vec!["ht", "service", "disconnect", "--expected-boot", "00000000-0000-4000-8000-000000000001", "--expected-generation", "7"],
        vec!["ht", "seat", "rebind", "seat-1", "--pane", "pane", "--operator"],
        vec!["ht", "seat", "resolve", "--pane", "pane", "--new-seat", "--operator"],
        vec!["ht", "invite", "thread-1", "--seat", "seat-1", "--operator"],
    ] {
        let failure = parse_argv(argv).unwrap_err();
        assert!(failure.detail.contains("immediate human namespace"), "{failure:?}");
    }
    let failure = parse_argv(["ht binary", "--state-dir", "state space", "--host-endpoint", "host space", "human", "me", "init"]).unwrap_err();
    assert!(failure.detail.contains("'ht binary' human --state-dir 'state space' --host-endpoint 'host space' me init"), "{failure:?}");
    assert!(parse_argv(["ht", "--state-dir", "human", "--human", "inbox"]).is_ok());
    assert!(parse_argv(["ht", "human", "--state-dir", "a", "me", "init", "--state-dir", "b"]).unwrap_err().detail.contains("conflicting values"));
}

#[test]
fn actor_prerequisite_human_refuses_cooperative_agent_selectors() {
    let failure = parse_argv(["ht", "human", "--cooperative-seat", "seat", "--cooperative-target", "pane", "--cooperative-harness", "codex", "--cooperative-role", "top-level", "ack", "message"]).unwrap_err();
    assert!(failure.detail.contains("cannot be mixed with cooperative agent selectors"), "{failure:?}");
}

#[test]
fn actor_prerequisite_real_os_parser_keeps_human_data_and_opaque_guidance() {
    use crate::cli::actor_route::InvocationActor;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let parsed = parse_argv(["ht", "--state-dir", "human", "--human", "inbox"]).unwrap();
    assert_eq!(parsed.actor, InvocationActor::Agent);
    let parsed = parse_argv(["ht", "human", "--json", "inbox"]).unwrap();
    assert_eq!(parsed.actor, InvocationActor::Human);
    let argv = vec![OsString::from_vec(vec![b'h', 0xff]), "--state-dir".into(), "state space".into(), "human".into(), "me".into(), "init".into()];
    let error = parse_argv(argv).unwrap_err();
    assert!(error.detail.contains("opaque argv cannot be rendered as UTF-8"), "{error:?}");
}

#[test]
fn human_topology_recovery_accepts_exact_attempt_qualified_grammar() {
    for suffix in [vec!["--created-pane", "w1:p2"], vec!["--not-created"], vec!["--cancel", "--reason", "inspected abandonment"]] {
        let mut argv = vec!["ht", "human", "--state-dir", "/quoted state", "--host-endpoint", "/quoted host.sock", "--json", "handoff", "recover", "local:1", "--attempt", "7"];
        argv.extend(suffix);
        let parsed = parse_argv(argv).expect("approved human recovery grammar must parse");
        assert_eq!(parsed.actor, super::super::actor_route::InvocationActor::Human);
        assert_eq!(parsed.output.format, OutputFormat::Json);
    }
}

#[test]
fn human_topology_recovery_rejects_missing_extra_and_invalid_assertions() {
    let cases = [vec!["--not-created"], vec!["--attempt", "0", "--not-created"], vec!["--attempt", "1"], vec!["--attempt", "1", "--not-created", "--cancel", "--reason", "x"], vec!["--attempt", "1", "--cancel"], vec!["--attempt", "1", "--cancel", "--reason", "   "], vec!["--attempt", "1", "--not-created", "--reason", "x"]];
    for suffix in cases {
        let mut argv = vec!["ht", "human", "handoff", "recover", "local:1"];
        argv.extend(suffix);
        assert!(parse_argv(argv.clone()).is_err(), "unexpected accepted argv: {argv:?}");
    }
    for reason in ["x".repeat(4097), "é".repeat(2049)] {
        assert!(parse_argv(["ht", "human", "handoff", "recover", "local:1", "--attempt", "1", "--cancel", "--reason", reason.as_str()]).is_err());
    }
    assert!(parse_argv(["ht", "handoff", "recover", "local:1", "--attempt", "1", "--not-created"]).is_err());
    assert!(parse_argv(["ht", "--json", "human", "handoff", "recover", "local:1", "--attempt", "1", "--not-created"]).is_err());
}

#[test]
fn legacy_handoff_body_word_recover_remains_data() {
    let parsed = parse_argv(["ht", "handoff", "--pane", "w1:p2", "--new-thread", "--kind", "codex", "--", "recover"]).unwrap();
    let CliAction::Handoff(request) = parsed.action else { panic!("legacy launch route changed") };
    assert_eq!(request.body, "recover");
}

#[test]
fn human_topology_recovery_help_and_errors_use_only_canonical_route() {
    let ParseFailure::Informational(help)=parse_argv_or_informational(["ht","human","handoff","recover","--help"]).unwrap_err() else {panic!("expected help")};
    assert!(help.contains("ht human handoff recover"),"{help}");assert!(help.contains("--attempt"));assert!(!help.contains("_topology-recover"));
    let error=parse_argv(["ht","human","handoff","recover","local:1","--not-created"]).unwrap_err();assert!(error.detail.contains("human handoff recover"),"{error:?}");assert!(!error.detail.contains("_topology-recover"));
    for argv in [vec!["ht","human","_topology-recover","local:1","--attempt","1","--not-created"],vec!["ht","--state-dir","human","handoff","recover","local:1","--attempt","1","--not-created"],vec!["ht","handoff","human","recover","local:1","--attempt","1","--not-created"],vec!["ht","handoff","recover","local:1","--attempt","1","--not-created","--operator"]] { assert!(parse_argv(argv).is_err()); }
    let parsed=parse_argv(["ht","human","--state-dir","human","handoff","recover","local:1","--attempt","1","--not-created"]).unwrap();assert_eq!(parsed.output.context.state_dir.as_deref(),Some("human"));
    let parsed=parse_argv(["ht","handoff","--pane","w1:p2","--thread","t1","--kind","codex","--agent-arg=recover","--","human handoff recover local:1"]).unwrap();let CliAction::Handoff(request)=parsed.action else {panic!("legacy route changed")};assert_eq!(request.body,"human handoff recover local:1");assert_eq!(request.launch.argv,vec!["recover"]);
}

#[test]
fn actor_route_immediate_namespace_and_output_are_independent() {
    for executable in ["herdr-threads", "ht", "/private/a space/ht"] {
        for args in [vec!["inbox"], vec!["send", "t1", "--body", "human --operator --human"], vec!["ack", "m1"], vec!["accept", "t1"], vec!["join", "t1"], vec!["leave", "t1"], vec!["check-in"], vec!["invite", "t1", "--seat", "s1"], vec!["reject", "t1", "--invitation", "i1", "--reason", "reason"], vec!["accept-required", "t1", "--invitation", "i1", "--requirement", "r1", "--revision", "1"]] {
            let ordinary = parse_argv(std::iter::once(executable).chain(args.iter().copied())).unwrap();
            let human = parse_argv([executable, "human"].into_iter().chain(args.iter().copied())).unwrap();
            assert_eq!(ordinary.action, human.action);
            assert_eq!(ordinary.actor, crate::cli::actor_route::InvocationActor::Agent);
            assert_eq!(human.actor, crate::cli::actor_route::InvocationActor::Human);
        }
    }
}
#[test]
fn actor_route_legacy_person_operator_forms_require_human() {
    for args in [vec!["me", "init"], vec!["me", "init", "--operator"], vec!["invite", "t1", "--seat", "s1", "--operator"], vec!["seat", "retire", "s1", "--operator"]] {
        let error = parse_argv(std::iter::once("ht").chain(args.iter().copied())).unwrap_err();
        assert!(error.detail.contains("human"), "{}", error.detail);
        assert!(parse_argv(["ht", "human"].into_iter().chain(args)).is_ok());
    }
}
#[test]
fn permission_cli_inputs_are_bounded_and_command_scoped() {
    assert!(parse_argv(["ht", "setup", "--permissions", "--with-permissions", "--permission-installed-binary", "/private/a space/$binary"]).is_ok());
    assert!(parse_argv(["ht", "setup", "--permission-link-path", "/private/ht"]).is_err());
}

#[test]
fn ordinary_catalog_exports_positive_syntax_contract() {
    let catalog = ordinary_catalog();
    for prefix in [&["send"][..], &["join"], &["launch"], &["seat", "resolve"], &["service", "inspect"], &["--skill"], &["thread", "rename"], &["--version"], &["--help"]] {
        assert!(catalog.families.iter().any(|f| f.prefix == prefix), "missing {prefix:?}");
    }
    assert!(catalog.output_flags.contains(&"--human"));
    assert!(catalog.routing_forms.iter().any(|f| f.contains(&RoutingToken::StateDirectory) && f.contains(&RoutingToken::HostEndpoint)));
}
#[test]
fn ordinary_catalog_excludes_person_and_operator_families() {
    let catalog = ordinary_catalog();
    assert!(!catalog.families.is_empty());
    for forbidden in [&["human"][..], &["me"], &["me", "init"], &["seat"], &["seat", "rebind"], &["seat", "retire"], &["service"], &["service", "disconnect"]] {
        assert!(!catalog.families.iter().any(|f| f.prefix == forbidden));
    }
}

/// Kills an agent-grantable self-permission: config-changing commands must be escalating,
/// never ordinary families, and each must still parse as a command.
#[test]
fn ordinary_catalog_escalates_self_granting_commands() {
    let catalog = ordinary_catalog();
    let expected: [&[&str]; 4] = [&["setup"], &["unsetup"], &["doctor", "fix"], &["internal", "installer-integrations"]];
    assert_eq!(catalog.escalating, &expected[..]);
    for prefix in expected {
        assert!(!catalog.families.iter().any(|f| f.prefix == prefix), "{prefix:?}");
    }
    assert!(catalog.families.iter().any(|f| f.prefix == ["setup-status"]));
    assert!(catalog.families.iter().any(|f| f.prefix == ["doctor"]));
}

#[test]
fn actor_route_pinned_globals_follow_human() {
    use crate::cli::actor_route::InvocationActor;
    let parsed = parse_argv(["ht", "human", "--state-dir", "/private/state", "--host-endpoint", "/private/host.sock", "--json", "me", "init"]).unwrap();
    assert_eq!(parsed.actor, InvocationActor::Human);
    assert_eq!(parsed.output.context.state_dir.as_deref(), Some("/private/state"));
    assert_eq!(parsed.output.context.host.as_deref(), Some("/private/host.sock"));
    assert_eq!(parsed.output.format, OutputFormat::Json);
    for prefix in [vec!["--json"], vec!["--state-dir", "/private/a space"], vec!["--state-dir=/private/state", "--host-endpoint=/private/host.sock"]] {
        let argv = ["ht"].into_iter().chain(prefix.iter().copied()).chain(["human", "me", "init"]);
        let error = parse_argv(argv).unwrap_err();
        assert!(error.detail.contains("ht human"), "{}", error.detail);
        for token in &prefix { assert!(error.detail.contains(token), "{}", error.detail); }
    }
    assert!(parse_argv(["ht", "human", "--state-dir", "/private/s", "me", "init", "--state-dir", "/private/s"]).is_ok());
    assert!(parse_argv(["ht", "human", "--state-dir", "/private/s", "me", "init", "--state-dir", "/private/other"]).unwrap_err().detail.contains("conflicting values"));
    for args in [vec!["--human", "inbox"], vec!["inbox", "--human"], vec!["human", "--human", "inbox"]] {
        let parsed = parse_argv(["ht"].into_iter().chain(args)).unwrap();
        assert_eq!(parsed.presentation, crate::cli::output::Presentation::Human);
    }
    for flag in ["--json", "--machine"] {
        assert!(parse_argv(["ht", "human", flag, "inbox"]).is_ok());
        assert!(parse_argv(["ht", flag, "inbox"]).is_ok());
    }
}

#[test]
fn actor_route_administrative_and_discovery_matrix() {
    use crate::cli::actor_route::InvocationActor;
    let boot = "00000000-0000-4000-8000-000000000001";
    let admin = ["service", "disconnect", "--expected-boot", boot, "--expected-generation", "7"];
    assert!(parse_argv(["ht"].into_iter().chain(admin)).unwrap_err().detail.contains("ht human service disconnect"));
    assert!(matches!(parse_argv(["ht", "human"].into_iter().chain(admin)).unwrap().action, CliAction::Wire(WireCommand::ServiceDisconnect(_))));
    for args in [vec!["service", "inspect"], vec!["seat", "list"], vec!["seat", "inspect", "s1"], vec!["seat", "resolve", "--pane", "p1"], vec!["read"], vec!["follow"], vec!["launch", "--kind", "codex", "--pane", "p1", "--", "human", "--operator", "--human"]] {
        assert_eq!(parse_argv(["ht"].into_iter().chain(args)).unwrap().actor, InvocationActor::Agent);
    }
    for args in [vec!["seat", "resolve", "--pane", "p1", "--new-seat", "--operator"], vec!["seat", "rebind", "s1", "--pane", "p1", "--operator"], vec!["seat", "rebind", "s1", "--pane", "p1", "--replace", "s2", "--operator"]] {
        let human = parse_argv(["ht", "human"].into_iter().chain(args.iter().copied())).unwrap();
        assert_eq!(human.actor, InvocationActor::Human);
        assert!(parse_argv(["ht"].into_iter().chain(args)).unwrap_err().detail.contains("human"));
    }
    for args in [vec!["seat", "rebind", "s1", "--pane", "p1"], vec!["seat", "retire", "s1"]] {
        let error = parse_argv(["ht"].into_iter().chain(args)).unwrap_err();
        assert!(error.detail.contains("human") && error.detail.contains("operator"));
    }
    assert!(parse_argv(["ht", "--cooperative-harness", "human", "inbox"]).unwrap_err().detail.contains("namespace"));
    assert!(parse_argv(["ht", "human", "--cooperative-seat", "s1", "--cooperative-target", "p1", "--cooperative-harness", "codex", "--cooperative-role", "top-level", "inbox"]).unwrap_err().detail.contains("cannot be mixed"));
}

#[test]
fn actor_route_payload_tokens_never_declare_actor() {
    use crate::cli::actor_route::InvocationActor;
    let body = "human --operator --human; ht human me init";
    assert!(matches!(parse_argv(["ht", "send", "t1", "--body", body]).unwrap().action, CliAction::Mutation(MutationSpec::Send { body: value, .. }) if value == body));
    for args in [vec!["thread", "create", "--topic", body, "--name", "human"], vec!["search", "human"], vec!["--state-dir", "human", "inbox"], vec!["handoff", "--thread", "t1", "--pane", "p1", "--kind", "codex", "--", body]] {
        assert_eq!(parse_argv(["ht"].into_iter().chain(args)).unwrap().actor, InvocationActor::Agent);
    }
}

#[test]
#[cfg(unix)]
fn actor_route_os_string_identity_is_preserved() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use crate::cli::actor_route::InvocationActor;
    let opaque = OsString::from_vec(b"ht\xff".to_vec());
    let parsed = parse_argv([opaque.clone(), "human".into(), "inbox".into()]).unwrap();
    assert_eq!(parsed.actor, InvocationActor::Human);
    for argv in [vec![opaque.clone(), OsString::from_vec(b"human\xff".to_vec()), "inbox".into()], vec![opaque.clone(), "send".into(), "t1".into(), "--body".into(), OsString::from_vec(vec![255])]] {
        assert!(parse_argv(argv).is_err());
    }
    let error = parse_argv([opaque, "me".into(), "init".into()]).unwrap_err();
    assert!(error.detail.contains("opaque argv"));
    assert!(!error.detail.contains('\u{fffd}'));
    assert!(matches!(parse_argv_or_informational(["ht"]), Err(ParseFailure::Usage(_))));
    for argv in [vec!["ht", "--version"], vec!["/private/a space/ht", "--version"], vec!["ht", "human", "--help"]] {
        assert!(matches!(parse_argv_or_informational(argv), Err(ParseFailure::Informational(_))));
    }
    let Err(ParseFailure::Informational(help)) = parse_argv_or_informational(["ht", "human", "--help"]) else { panic!("help") };
    assert!(help.contains("immediate `human`"));
}

#[test]
fn permission_cli_inputs_preserve_installer_spellings() {
    let binary = "/private/a space/$binary;`literal`'quoted'";
    for verb in ["setup", "unsetup", "setup-status"] {
        let parsed = parse_argv(["ht", verb, "--permissions", "--permission-installed-binary", binary, "--permission-link-path", "/private/link", "--permission-alias-path", "/private/alias"]).unwrap();
        assert!(parsed.permissions.permissions);
        assert_eq!(parsed.permissions.permission_installed_binary.as_deref(), Some(binary));
        assert_eq!(parsed.permissions.permission_link_path.as_deref(), Some("/private/link"));
        assert_eq!(parsed.permissions.permission_alias_path.as_deref(), Some("/private/alias"));
        assert_eq!(parse_argv(["ht", verb]).unwrap().permissions, PermissionCliInputs::default());
        for flag in ["--with-permissions", "--without-permissions"] {
            assert_eq!(parse_argv(["ht", verb, flag]).is_ok(), verb == "setup");
        }
    }
    for flag in ["--with-permissions", "--without-permissions"] {
        let parsed = parse_argv(["ht", "internal", "installer-integrations", "--confirm-missing", flag, "--permission-installed-binary", binary]).unwrap();
        assert!(matches!(parsed.action, CliAction::InstallerIntegrations { confirm_missing: true }));
        assert_eq!(parsed.permissions.with_permissions, flag == "--with-permissions");
        assert_eq!(parsed.permissions.without_permissions, flag == "--without-permissions");
        assert_eq!(parsed.permissions.permission_installed_binary.as_deref(), Some(binary));
    }
    for args in [vec!["setup", "--with-permissions", "--without-permissions"], vec!["internal", "installer-integrations", "--with-permissions", "--without-permissions"], vec!["inbox", "--permissions"], vec!["setup", "--permission-alias-path", "/private/alias"], vec!["setup", "--permission-installed-binary", "/private/a", "--permission-installed-binary", "/private/a"]] {
        assert!(parse_argv(["ht"].into_iter().chain(args)).is_err());
    }
    for path in ["".into(), "relative".into(), "/private/line\nfeed".into(), "/private/tab\t".into(), format!("/{}", "x".repeat(4096))] {
        for flag in ["--permission-installed-binary", "--permission-link-path", "--permission-alias-path"] {
            assert!(parse_argv(["ht", "setup", flag, &path]).is_err(), "{flag}: {path:?}");
        }
    }
    let maximum = format!("/{}", "x".repeat(4095));
    assert_eq!(parse_argv(["ht", "setup", "--permission-installed-binary", &maximum]).unwrap().permissions.permission_installed_binary, Some(maximum));
}

#[test]
fn ordinary_catalog_routing_and_output_forms_parse_without_actor_escalation() {
    let catalog = ordinary_catalog();
    for executable in ["ht", "herdr-threads", "/private/a space/herdr-threads"] {
        for routing in catalog.routing_forms {
            let pinned: Vec<&str> = routing.iter().map(|token| match token {
                RoutingToken::Literal(token) => *token,
                RoutingToken::StateDirectory => "/private/a space/state",
                RoutingToken::HostEndpoint => "/private/a space/host.sock",
            }).collect();
            for flag in catalog.output_flags {
                for position in catalog.output_positions {
                    let mut argv = vec![executable];
                    argv.extend(&pinned);
                    if *position == OutputPosition::BeforeFamily { argv.push(flag); }
                    argv.extend(["send", "t1", "--body", "human --operator; ht human me init"]);
                    if *position == OutputPosition::AfterArguments { argv.push(flag); }
                    let parsed = parse_argv(argv).unwrap();
                    assert_eq!(parsed.actor, crate::cli::actor_route::InvocationActor::Agent);
                    assert!(matches!(parsed.action, CliAction::Mutation(MutationSpec::Send { body, .. }) if body == "human --operator; ht human me init"));
                }
            }
        }
    }
    let resolve = catalog.families.iter().find(|f| f.prefix == ["seat", "resolve"]).unwrap();
    assert_eq!(resolve.human_options, ["--operator", "--new-seat"]);
    assert_eq!(catalog.families.iter().find(|f| f.prefix == ["invite"]).unwrap().human_options, ["--operator"]);
}

#[test]
fn inbox_v2_continuation_validates_without_changing_own_text_action() {
    use crate::protocol::{ids::{ExecutionId,MessageId,SeatId}, pagination::{InboxBatchV2CursorState as State, InboxBatchV2Source as Source, InboxBatchV2BodyPosition,SeatAttentionCursorState,ReceiptAttentionCursorState}};
    let state = State {
        seat: SeatId::new("s"),
        binding_generation: Some(1),
        execution: Some(ExecutionId::new("execution")),
        source: Source::Lazy,
        lazy_after_ordinal: 0,
        lazy_high_water_ordinal: 2,
        publication_decision_high_water: 4,
        body: Some(InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset: 2,
            body_len: 10,
        }),
        attention: SeatAttentionCursorState {
            invitation_after_seq: 0,
            invitation_after_ordinal: 0,
            invitations_done: false,
            has_pending_invitation: false,
            invitation_frontier: Some((0, 2)),
            receipts: Some(ReceiptAttentionCursorState {
                physical_after: 0,
                manifest_after: 0,
                physical_high_water: 2,
                manifest_high_water: 2,
                next_manifest: false,
            }),
            receipts_done: false,
            has_pending_receipt: false,
            receipt_frontier_seq: Some(4),
            physical_warning_after: 0,
            physical_warning_high_water: 4,
            manifest_warning_after: 0,
            manifest_warning_high_water: 4,
            next_manifest_warning: false,
            latest_warning_seq: None,
            latest_warning_offset: Some(0),
        },
    };
    let raw=state.encode("fixture-instance").unwrap();
    let parsed=parse_argv(["ht","inbox","--cursor",&raw]).unwrap();
    let CliAction::Wire(WireCommand::Inbox(query))=parsed.action else {panic!("must retain own Inbox placeholder")};
    assert!(query.seat.is_none());
    assert_eq!(query.page.cursor.as_deref(),Some(raw.as_str()));
    assert!(WireCommand::Inbox(query.clone()).validate().is_err(),"legacy wire still rejects ib2");
    assert!(WireCommand::InboxBatch(query.clone()).validate().is_err());
    assert!(WireCommand::InboxBatchV2(query).validate().is_ok());
    for prefix in [vec!["ht","read","t"],vec!["ht","search","needle"],vec!["ht","seat","list"]] {let mut args=prefix;args.extend(["--cursor",&raw]);assert!(parse_argv(args).is_err(),"non-Inbox route must reject v2 cursor");}
    for invalid in ["ib2:","ib2:broken!","ib2:A"] {assert!(parse_argv(["ht","inbox","--cursor",invalid]).is_err());}
}

#[test]
fn public_join_parses_and_freezes_thread_selector() {
    let mut parsed = parse_argv(["herdr-threads", "join", "shared work"]).unwrap();
    assert_eq!(parsed.thread_selector.as_deref(), Some("shared work"));
    crate::cli::threads::resolve_cli_threads::<crate::protocol::results::ApiError>(
        &mut parsed, |_| Ok(ThreadId::new("tResolved")),
    ).unwrap();
    let CliAction::Mutation(spec) = parsed.action else { panic!("mutation expected") };
    assert_eq!(crate::cli::threads::selector_mut(&mut CliAction::Mutation(spec)).unwrap().as_str(), "tResolved");
}

#[test]
fn registry_selectors_preserve_legacy_argv_and_never_default_unknown_to_claude() {
    // A parser caller must not gain a Claude claim just because it bypassed clap.
    let mut cli = Cli::try_parse_from([
        "herdr-threads",
        "--cooperative-seat",
        "s1",
        "--cooperative-target",
        "w1:p1",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
        "inbox",
    ])
    .unwrap();
    cli.cooperative_harness = Some("missing-adapter".into());
    assert!(
        parse_cli(cli).is_err(),
        "unknown identity must not become Claude"
    );
    let parsed = parse_argv([
        "herdr-threads",
        "launch",
        "--pane",
        "w1:p1",
        "--kind",
        "codex",
        "--",
        "exec",
        "--config",
        "key=value with spaces",
        "prompt",
    ])
    .unwrap();
    assert!(matches!(parsed.action, CliAction::Launch(request)
        if request.harness.as_str() == "codex"
        && request.argv == ["exec", "--config", "key=value with spaces", "prompt"]));
    for name in ["human", "Human", "missing-adapter"] {
        for argv in [
            vec!["herdr-threads", "contract-id", "--harness", name],
            vec![
                "herdr-threads",
                "harness-version",
                "normalize",
                name,
                "0.1.0",
            ],
            vec!["herdr-threads", "launch", "--pane", "w1:p1", "--kind", name],
            vec![
                "herdr-threads",
                "handoff",
                "--pane",
                "w1:p1",
                "--kind",
                name,
                "--new-thread",
                "--",
                "body",
            ],
            vec!["herdr-threads", "setup", name],
            vec!["herdr-threads", "unsetup", name],
            vec!["herdr-threads", "setup-status", name],
            vec!["herdr-threads", "doctor", "--harness", name],
            vec![
                "herdr-threads",
                "--cooperative-seat",
                "s1",
                "--cooperative-target",
                "w1:p1",
                "--cooperative-harness",
                name,
                "--cooperative-role",
                "top-level",
                "inbox",
            ],
        ] {
            let error = parse_argv(argv).unwrap_err();
            assert!(error.detail.contains("possible values"), "{error:?}");
        }
        let error = harness_arg(name, crate::harness::registry::builtins()).unwrap_err();
        assert!(error.detail.contains("select a registered agent harness"));
    }
    for (harness, serialized) in [
        (crate::harness::context::Harness::Codex, "\"Codex\""),
        (crate::harness::context::Harness::Claude, "\"Claude\""),
        (crate::harness::context::Harness::Human, "\"Human\""),
    ] {
        assert_eq!(serde_json::to_string(&harness).unwrap(), serialized);
        assert_eq!(
            serde_json::from_str::<crate::harness::context::Harness>(serialized).unwrap(),
            harness
        );
    }
    assert!(matches!(
        parse_argv(["herdr-threads", "human", "me", "init"]).unwrap().action,
        CliAction::MeInit { operator: false }
    ));
}

// Metadata-only author fixture: selecting it must never call runtime operations.
static SELECTOR_INSTALL_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct SelectorAdapter(u8);
impl crate::harness::adapter::HarnessAdapter for SelectorAdapter {
    type Admission = ();
    fn metadata(&self) -> &'static crate::harness::adapter::AdapterMetadata {
        use crate::harness::adapter::*;
        static METADATA: AdapterMetadata = AdapterMetadata {
            id: "selectorfixture",
            display_label: "Synthetic SelectorFixture",
            context_spelling: "SelectorFixture",
            context_aliases: &[],
            executable: ExecutableLookup::Unsupported,
            host_kinds: &[],
            setup_scopes: &[SetupScopeKind::ConfigRoot, SetupScopeKind::Profile],
            budget: EventBudgetPolicy {
                lifecycle_ms: 1,
                observer_ms: 1,
            },
            runtime_sources: &[],
        };
        static FOURTH: AdapterMetadata = AdapterMetadata {
            id: "fourth",
            display_label: "Fourth",
            context_spelling: "Fourth",
            context_aliases: &[],
            executable: ExecutableLookup::Unsupported,
            host_kinds: &[],
            setup_scopes: &[SetupScopeKind::ConfigRoot],
            budget: EventBudgetPolicy {
                lifecycle_ms: 1,
                observer_ms: 1,
            },
            runtime_sources: &[],
        };
        if self.0 != 0 { &FOURTH } else { &METADATA }
    }
    fn canary_strategy(&self) -> Option<&dyn crate::harness::adapter::CanaryStrategy> {
        if self.0 == 2 { Some(&InvalidDiscoveryStrategy) } else { None }
    }
    fn contracts(&self) -> &'static [crate::harness::adapter::ContractDescriptor] {
        &[]
    }
    fn observe_install(
        &self,
        _: &crate::harness::adapter::InstallEnvironment,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::InstallObservation {
        SELECTOR_INSTALL_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        panic!("selector must not probe")
    }
    fn admit(
        &self,
        _: &crate::harness::adapter::AdmissionRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::AdmissionDecision<()> {
        panic!("selector must not admit")
    }
    fn version_ladder(
        &self,
        _: &crate::harness::adapter::RuntimeIdentity,
    ) -> crate::harness::adapter::Ladder {
        panic!("selector must not inspect version")
    }
    fn classify(
        &self,
        _: &crate::harness::adapter::HookInput,
    ) -> crate::harness::adapter::ContractObservation {
        panic!("selector must not classify")
    }
    fn decode(
        &self,
        _: &(),
        _: &crate::harness::adapter::HookInput,
    ) -> Result<crate::harness::adapter::DecodedEvent, crate::harness::adapter::DecodeFailure> {
        panic!("selector must not decode")
    }
    fn encode(
        &self,
        _: &(),
        _: &crate::harness::adapter::DecodedEvent,
        _: &crate::harness::adapter::NeutralOffer,
    ) -> Result<crate::harness::adapter::EncodedOutput, crate::harness::adapter::EncodeFailure>
    {
        panic!("selector must not encode")
    }
    fn attribute_runtime(
        &self,
        _: &crate::harness::adapter::HookInput,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::RuntimeAttribution {
        panic!("selector must not attribute")
    }
    fn setup(
        &self,
        _: &crate::harness::adapter::SetupRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        panic!("selector must not set up")
    }
    fn status(
        &self,
        _: &crate::harness::adapter::StatusRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> crate::harness::adapter::SetupStatus {
        panic!("selector must not inspect status")
    }
    fn unsetup(
        &self,
        _: &crate::harness::adapter::UnsetupRequest,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        panic!("selector must not remove")
    }
}

#[test]
fn injected_registry_drives_agent_selector_choices_without_native_operations() {
    use crate::harness::registry::{Registration, Registry};
    use clap::FromArgMatches;
    static ADAPTER: SelectorAdapter = SelectorAdapter(0);
    static FOURTH: SelectorAdapter = SelectorAdapter(1);
    let registrations =
        Box::leak(vec![Registration::new(&ADAPTER), Registration::new(&FOURTH)].into_boxed_slice());
    let registry = Registry::new(registrations).unwrap();
    for (name, spelling) in [
        ("selectorfixture", "SelectorFixture"),
        ("fourth", "Fourth"),
    ] {
        for mut argv in [
            vec![
                "herdr-threads",
                "launch",
                "--pane",
                "w1:p1",
                "--kind",
                "selectorfixture",
                "--",
                "literal prompt",
            ],
            vec![
                "herdr-threads",
                "handoff",
                "--pane",
                "w1:p1",
                "--kind",
                "selectorfixture",
                "--new-thread",
                "--",
                "durable body",
            ],
            vec!["herdr-threads", "contract-id", "--harness", "selectorfixture"],
            vec![
                "herdr-threads",
                "harness-version",
                "normalize",
                "selectorfixture",
                "development identity",
            ],
            vec![
                "herdr-threads",
                "--cooperative-seat",
                "s1",
                "--cooperative-target",
                "w1:p1",
                "--cooperative-harness",
                "selectorfixture",
                "--cooperative-role",
                "top-level",
                "inbox",
            ],
        ] {
            for value in &mut argv {
                if *value == "selectorfixture" {
                    *value = name;
                }
            }
            let matches = command_for_registry(&registry)
                .try_get_matches_from(argv)
                .unwrap();
            let cli = Cli::from_arg_matches(&matches).unwrap();
            let parsed = parse_cli_in_registry(cli, &registry).unwrap();
            let selected = match parsed.action {
                CliAction::Launch(request) => request.harness,
                CliAction::Handoff(request) => request.launch.harness,
                CliAction::ContractId { harness } => harness.unwrap(),
                CliAction::HarnessVersionNormalize { harness, .. } => harness,
                _ => parsed.cooperative.unwrap().harness,
            };
            assert_eq!(
                selected.occupant(),
                crate::harness::registry::OccupantHarness::Agent(registry.agent(name).unwrap())
            );
            assert_eq!(
                serde_json::to_string(&selected).unwrap(),
                format!("\"{spelling}\"")
            );
        }
        for verb in ["setup", "unsetup", "setup-status"] {
            let matches = command_for_registry(&registry)
                .try_get_matches_from(["herdr-threads", verb, name])
                .unwrap();
            let parsed =
                parse_cli_in_registry(Cli::from_arg_matches(&matches).unwrap(), &registry).unwrap();
            assert!(
                matches!(parsed.action, CliAction::Setup(request) if request.harness.as_str() == name)
            );
        }
        let matches = command_for_registry(&registry)
            .try_get_matches_from(["herdr-threads", "doctor", "--harness", name])
            .unwrap();
        assert!(matches!(
            parse_cli_in_registry(Cli::from_arg_matches(&matches).unwrap(), &registry)
                .unwrap()
                .action,
            CliAction::Doctor { .. }
        ));
    }
    assert!(
        crate::harness::registry::builtins()
            .agent("selectorfixture")
            .is_err(),
        "fixture is never production registration"
    );
}

#[test]
fn doctor_action_retains_selected_registered_profile() {
    use crate::harness::registry::{Registration, Registry};
    use clap::FromArgMatches;
    static ADAPTER: SelectorAdapter = SelectorAdapter(0);
    let registry = Registry::new(Box::leak(vec![Registration::new(&ADAPTER)].into_boxed_slice())).unwrap();
    let matches = command_for_registry(&registry).try_get_matches_from([
        "herdr-threads", "doctor", "--harness", "selectorfixture", "--profile", "work"
    ]).unwrap();
    let parsed = parse_cli_in_registry(Cli::from_arg_matches(&matches).unwrap(), &registry).unwrap();
    let action = format!("{:?}", parsed.action);
    assert!(action.contains("selectorfixture"), "{action}");
    assert!(action.contains("work"), "{action}");
}

// Catches hard-coded discovery, reordering, native observation, and synthesized strategies.
#[test]
fn adapters_discovery_is_bounded_deterministic_and_never_probes_installation() {
    use crate::harness::registry::{Registration, Registry};
    use clap::FromArgMatches;
    static THIRD: SelectorAdapter = SelectorAdapter(0);
    static FOURTH: SelectorAdapter = SelectorAdapter(1);
    let registry = Registry::new(Box::leak(vec![
        Registration::new(&FOURTH),
        Registration::new(&crate::harness::claude::ClaudeAdapter),
        Registration::new(&THIRD),
        Registration::new(&crate::harness::codex::CodexAdapter),
    ].into_boxed_slice())).unwrap();
    let matches = command_for_registry(&registry)
        .try_get_matches_from(["herdr-threads", "adapters", "--json"]).unwrap();
    let parsed = parse_cli_in_registry(Cli::from_arg_matches(&matches).unwrap(), &registry).unwrap();
    assert!(matches!(parsed.action, CliAction::Adapters));
    assert_eq!(parsed.output.format, OutputFormat::Json);
    let bytes = adapter_discovery_output(&parsed, &registry).unwrap().expect("local dispatch output");
    let value: serde_json::Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    let entries = value["adapters"].as_array().unwrap();
    assert_eq!(entries.iter().map(|e|e["id"].as_str().unwrap()).collect::<Vec<_>>(),
        ["fourth", "claude", "selectorfixture", "codex"]);
    assert_eq!(entries[0], serde_json::json!({"id":"fourth", "display_name":"Fourth",
        "host_kinds":[], "setup_scopes":["config_root"], "legacy_contract_id":null,
        "contracts":[], "canary_strategy":null}));
    assert_eq!(entries[2]["canary_strategy"], serde_json::Value::Null);
    assert_eq!(entries[1]["contracts"][0]["domain"], "native_payload");
    assert_eq!(entries[1]["contracts"][0]["required_milestones"], serde_json::json!(["lifecycle","tool"]));
    assert_eq!(entries[1]["canary_strategy"]["npm_package"], "@anthropic-ai/claude-code");
    assert_eq!(entries[3]["canary_strategy"]["npm_package"], "@openai/codex");
    assert_eq!(bytes, crate::harness::discovery::render(&registry).unwrap());
    assert_eq!(SELECTOR_INSTALL_CALLS.load(std::sync::atomic::Ordering::SeqCst),0);
    struct CountBackend(usize);
    impl CliBackend for CountBackend {
        fn call(&mut self,_:WireCommand,_:&OutputSpec)->Result<CommandResult,ApiError> {
            self.0 += 1; Err(ApiError::unsupported("unexpected daemon call"))
        }
    }
    let mut backend=CountBackend(0);
    assert!(dispatch(parsed,&mut backend,None,None).is_err());
    assert_eq!(backend.0,0);

}

struct InvalidDiscoveryStrategy;
impl crate::harness::adapter::CanaryStrategy for InvalidDiscoveryStrategy {
    fn descriptor(&self) -> crate::harness::adapter::CanaryDescriptor {
        use crate::harness::adapter::*;
        CanaryDescriptor { kind:CanaryKind::ExactRuntime, candidate_kind:CandidateKind::ExactBuild,
            npm_package:None, model_key_env:None, companion:"scripts/canary/adapters/../bad.py".into(),
            artifact_schema_version:1 }
    }
}
// A malformed adapter-owned provider must refuse local discovery, never expose a supported strategy.
#[test]
fn adapters_discovery_refuses_invalid_provider_before_daemon_dispatch() {
    use crate::harness::registry::{Registration, Registry};
    static BAD: SelectorAdapter = SelectorAdapter(2);
    let registry=Registry::new(Box::leak(vec![Registration::new(&BAD)].into_boxed_slice())).unwrap();
    let parsed=parse_argv(["ht","adapters","--json"]).unwrap();
    assert!(adapter_discovery_output(&parsed,&registry).is_err());
}
