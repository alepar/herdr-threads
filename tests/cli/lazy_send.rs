use herdr_threads::{
    app::SystemClock,
    cli::{
        RunError,
        commands::{CliAction, parse_argv},
        journal::{IntentScope, Journal, SemanticMutation},
        run_cooperative,
    },
    harness::{
        bridge::caller_claim,
        context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    },
    ports::LocalClient,
    protocol::{
        commands::{Command, DeliveryMode},
        ids::{MessageId, OperationId},
        priority::DeliveryPriority,
        results::{ApiError, CapabilityList, CommandResult, ErrorCode},
        time::CallBudget,
    },
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

struct Fixture {
    root: PathBuf,
    journal: Journal,
    contexts: ContextJournal,
    context: OccupantContext,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp/ht-big.4.1")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let directory = root.join("contexts");
        std::fs::create_dir(&directory).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let context = OccupantContext {
            format_version: 1,
            instance: herdr_threads::node::NodeUuid::from_u128(1),
            seat: "seat-1".into(),
            target: "pane-1".into(),
            harness: Harness::Codex,
            binding_generation: 7,
            execution: uuid::Uuid::from_u128(2),
            session: SessionReference::Native("original-session".into()),
            role: Role::TopLevel,
        };
        let contexts = ContextJournal::open(
            &directory,
            context.instance,
            &context.seat,
            Duration::from_secs(1),
        )
        .map(|journal| journal.with_host(herdr_threads::test_support::test_host_locator()))
        .unwrap();
        contexts.install_reattached(context.clone()).unwrap();
        let journal = Journal::open(root.join("intents")).unwrap();
        Self {
            root,
            journal,
            contexts,
            context,
        }
    }
    fn run<W: Write>(
        &self,
        argv: &[&str],
        client: &Client,
        writer: &mut W,
    ) -> Result<(), RunError> {
        run_cooperative(
            parse_argv(argv.iter().copied()).unwrap(),
            &self.journal,
            &self.contexts,
            None,
            Role::TopLevel,
            client,
            &SystemClock::new(),
            writer,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
struct Client {
    capability: Result<bool, ErrorCode>,
    send_time_capability: bool,
    calls: Mutex<Vec<Command>>,
}
impl Client {
    fn new(capability: Result<bool, ErrorCode>) -> Self {
        Self {
            capability,
            send_time_capability: true,
            calls: Mutex::new(vec![]),
        }
    }
}
impl LocalClient for Client {
    fn call_with_output(
        &self,
        command: Command,
        _: &herdr_threads::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::Capabilities => match self.capability.clone() {
                Ok(supported) => Ok(CommandResult::Capabilities(CapabilityList {
                    capabilities: if supported {
                        let mut names = vec!["send.lazy_v1".into()];
                        if self.send_time_capability {
                            names.push("messages.send_time_receipts_v1".into());
                        }
                        names
                    } else {
                        vec![]
                    },
                })),
                Err(ErrorCode::Unsupported) => Err(ApiError::unsupported("old daemon")),
                Err(ErrorCode::InvalidRequest) => Err(ApiError::invalid_request("old daemon")),
                Err(code) => Err(ApiError::new(code, "probe failed")),
            },
            Command::SendMessage(_) => Ok(CommandResult::MessageSent(MessageId::new("published"))),
            _ => panic!("unexpected command: {command:?}"),
        }
    }
}
struct LostOutput;
impl Write for LostOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "lost output"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn default_send_freezes_priority_one_with_the_relayed_intent() {
    let f = Fixture::new();
    let client = Client::new(Ok(true));
    assert!(
        f.run(
            &[
                "ht",
                "send",
                "t1",
                "--body",
                "announcement",
                "--relays-user",
                "--user-intent",
                "rule"
            ],
            &client,
            &mut LostOutput
        )
        .is_err()
    );
    let pending = f.journal.page(&Default::default()).unwrap();
    assert_eq!(pending.items.len(), 1);
    let reference = f
        .journal
        .resolve_recovery_ref(pending.items[0].recovery_ref.as_str())
        .unwrap();
    let saved = f.journal.load(&reference).unwrap();
    let command = saved.semantic.to_command(saved.operation, None).unwrap();
    let Command::SendMessage(send) = command else {
        panic!("not send")
    };
    assert_eq!(send.delivery_mode, DeliveryMode::Ordinary);
    assert_eq!(send.priority, Some(DeliveryPriority::DEFAULT));
    assert!(send.relays_user);
    assert_eq!(
        send.user_intent,
        Some(herdr_threads::protocol::summary::UserIntent::Rule)
    );
    assert_eq!(
        send.claim,
        caller_claim(&f.context, herdr_threads::test_support::test_host_locator()).unwrap()
    );
    let json = serde_json::to_value(saved.semantic).unwrap();
    assert_eq!(json["mutation"]["priority"], 1);
    assert!(json["mutation"].get("delivery_mode").is_none());
}
#[test]
fn removed_lazy_flag_points_to_the_default_priority_before_the_body_is_read() {
    for options in [
        vec![],
        vec!["--require-ack", "s1"],
        vec!["--require-ack-pane", "p1"],
        vec!["--deadline", "10"],
        vec!["--nudge"],
    ] {
        let mut argv = vec![
            "ht",
            "send",
            "t1",
            "--file",
            "/nonexistent/lazy-body",
            "--lazy",
        ];
        argv.extend(options);
        let error = parse_argv(argv).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(error.detail.contains("--lazy was removed"), "{error:?}");
        assert!(error.detail.contains("priority 1"), "{error:?}");
    }
}
#[test]
fn ordinary_send_omits_mode_and_preserves_digest() {
    let legacy = r#"{"kind":"send_message","thread":"t1","body":"announcement","invited_recipients":[],"deadline_millis":null}"#;
    let semantic: SemanticMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(serde_json::to_string(&semantic).unwrap(), legacy);
    let f = Fixture::new();
    let reference = f
        .journal
        .record(
            IntentScope::Native {
                instance: "i".into(),
                seat: herdr_threads::protocol::ids::SeatId::new("s1"),
            },
            semantic,
            1,
        )
        .unwrap();
    assert_eq!(
        f.journal.load(&reference).unwrap().header.semantic_digest,
        "ace51db3e8dc24bd12b0abf5a61f9f4a52cfbfe559bc734c2b13c1c68667b1d0"
    );
    let parsed = parse_argv(["ht", "send", "t1", "--body", "announcement", "--nudge"]).unwrap();
    let CliAction::Mutation(spec) = parsed.action else {
        panic!("not mutation")
    };
    let command = spec
        .into_command(
            Some(
                caller_claim(&f.context, herdr_threads::test_support::test_host_locator()).unwrap(),
            ),
            OperationId::new("op1"),
        )
        .unwrap();
    let Command::SendMessage(ref send) = command else {
        panic!("not send")
    };
    assert_eq!(send.delivery_mode, DeliveryMode::Ordinary);
    assert!(
        serde_json::to_value(command).unwrap()["args"]
            .as_object()
            .unwrap()
            .get("delivery_mode")
            .is_none()
    );
    let client = Client::new(Ok(true));
    f.run(
        &["ht", "send", "t1", "--body", "announcement", "--nudge"],
        &client,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(matches!(
        client.calls.lock().unwrap().as_slice(),
        [Command::SendMessage(_)]
    ));
}
#[test]
fn priority_send_retry_preserves_frozen_claim_and_shape() {
    let f = Fixture::new();
    let client = Client::new(Ok(true));
    assert!(
        f.run(
            &["ht", "send", "t1", "--body", "announcement"],
            &client,
            &mut LostOutput
        )
        .is_err()
    );
    let pending = f.journal.page(&Default::default()).unwrap();
    let recovery = pending.items[0].recovery_ref.as_str();
    let reference = f.journal.resolve_recovery_ref(recovery).unwrap();
    let saved = f.journal.load(&reference).unwrap();
    let mut successor = f.context.clone();
    successor.harness = Harness::Claude;
    successor.binding_generation += 1;
    successor.execution = uuid::Uuid::from_u128(3);
    successor.session = SessionReference::Native("successor-session".into());
    f.contexts.install_reattached(successor).unwrap();
    f.run(&["ht", "retry", recovery], &client, &mut Vec::new())
        .unwrap();
    let calls = client.calls.lock().unwrap();
    let sends: Vec<_> = calls
        .iter()
        .filter_map(|c| {
            if let Command::SendMessage(s) = c {
                Some(s)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(sends.len(), 2);
    assert_eq!(sends[0], sends[1]);
    assert_eq!(sends[1].delivery_mode, DeliveryMode::Ordinary);
    assert_eq!(sends[1].priority, Some(DeliveryPriority::DEFAULT));
    assert_eq!(
        sends[1].claim,
        caller_claim(&f.context, herdr_threads::test_support::test_host_locator()).unwrap()
    );
    assert_eq!(
        saved.header.scope,
        IntentScope::Cooperative {
            host: herdr_threads::test_support::test_host_locator(),
            instance: f.context.instance.to_string(),
            seat: sends[1].claim.seat.clone()
        }
    );
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn lazy_send_journal_rejects_obligations_before_recording() {
    let f = Fixture::new();
    for extra in [
        serde_json::json!({"invited_recipients": ["s1"]}),
        serde_json::json!({"deadline_millis": 0}),
        serde_json::json!({"deadline_millis": 1000}),
    ] {
        let mut value = serde_json::json!({"kind": "send_message", "delivery_mode": "lazy", "thread": "t1", "body": "announcement", "invited_recipients": [], "deadline_millis": null});
        for (key, value_override) in extra.as_object().unwrap() {
            value[key] = value_override.clone();
        }
        let semantic = serde_json::from_value(value).unwrap();
        let scope = IntentScope::Cooperative {
            host: herdr_threads::test_support::test_host_locator(),
            instance: f.context.instance.to_string(),
            seat: caller_claim(&f.context, herdr_threads::test_support::test_host_locator())
                .unwrap()
                .seat,
        };
        assert!(f.journal.record(scope, semantic, 1).is_err());
    }
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
}

fn parsed_priority(options: &[&str]) -> u8 {
    use herdr_threads::cli::commands::MutationSpec;
    let mut argv = vec!["ht", "send", "t1", "--body", "announcement"];
    argv.extend(options);
    let parsed = parse_argv(argv).unwrap();
    let CliAction::Mutation(MutationSpec::Send { priority, .. }) = parsed.action else {
        panic!("not send")
    };
    priority.get()
}
fn parse_error(options: &[&str]) -> ApiError {
    let mut argv = vec!["ht", "send", "t1", "--file", "/nonexistent/body"];
    argv.extend(options);
    parse_argv(argv).unwrap_err()
}

#[test]
fn send_priority_flag_matrix() {
    for (options, expected) in [
        (vec![], 1),
        (vec!["-p", "0"], 0),
        (vec!["--priority", "9"], 9),
        (vec!["-p", "5"], 5),
        (vec!["--nudge"], 0),
        (vec!["--require-ack", "s1"], 0),
        (vec!["--require-ack-pane", "p1"], 0),
        (vec!["--require-ack", "s1", "-p", "0"], 0),
        (vec!["--require-ack", "s1", "--nudge"], 0),
        (vec!["--require-ack", "s1", "--deadline", "5"], 0),
    ] {
        assert_eq!(parsed_priority(&options), expected, "{options:?}");
    }
}

#[test]
fn send_priority_bounds_and_conflicts_are_usage_errors() {
    for options in [
        vec!["-p", "10"],
        vec!["-p", "-1"],
        vec!["-p", "x"],
        vec!["--nudge", "-p", "2"],
        vec!["-p", "2", "--nudge"],
    ] {
        let mut argv = vec!["ht", "send", "t1", "--body", "announcement"];
        argv.extend(options.clone());
        assert!(parse_argv(argv).is_err(), "{options:?}");
    }
}

#[test]
fn ack_with_a_lower_urgency_and_deadline_without_ack_are_refused_before_the_body_is_read() {
    // `--file` names a missing path: reaching the body read would be a different error.
    let ack = parse_error(&["--require-ack", "s1", "-p", "2"]);
    assert_eq!(ack.code, ErrorCode::InvalidRequest);
    assert_eq!(
        ack.detail,
        "--require-ack implies priority 0; drop --priority"
    );
    for options in [
        vec!["--deadline", "5"],
        vec!["--nudge", "--deadline", "5"],
        vec!["-p", "0", "--deadline", "5"],
    ] {
        let error = parse_error(&options);
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{options:?}");
        assert_eq!(
            error.detail, "--deadline requires --require-ack recipients",
            "{options:?}"
        );
    }
}

#[test]
fn direct_ack_target_rule_is_kept() {
    let error = parse_argv([
        "ht",
        "send",
        "--seat",
        "s-target",
        "--body",
        "x",
        "--require-ack",
        "s-other",
    ])
    .unwrap_err();
    assert!(
        error
            .detail
            .contains("direct ACK requests may name only the target seat"),
        "{error:?}"
    );
}

#[test]
fn ordinary_historical_retry_refuses_unsupported_without_rewriting_intent() {
    for capability in [
        Ok(false),
        Err(ErrorCode::Unsupported),
        Err(ErrorCode::InvalidRequest),
    ] {
        let f = Fixture::new();
        let legacy: SemanticMutation = serde_json::from_str(r#"{"kind":"send_message","thread":"t1","body":"historical body","invited_recipients":[],"deadline_millis":null}"#).unwrap();
        let reference = f
            .journal
            .record(
                IntentScope::Cooperative {
                    host: herdr_threads::test_support::test_host_locator(),
                    instance: f.context.instance.to_string(),
                    seat: herdr_threads::protocol::ids::SeatId::new(f.context.seat.clone()),
                },
                SemanticMutation::freeze(
                    legacy,
                    caller_claim(&f.context, herdr_threads::test_support::test_host_locator())
                        .unwrap(),
                )
                .unwrap(),
                0,
            )
            .unwrap();
        let original = f.journal.load(&reference).unwrap();
        let client = Client::new(capability);
        let result = f.run(
            &["ht", "retry", &reference.recovery_ref()],
            &client,
            &mut Vec::new(),
        );
        assert!(
            matches!(result,Err(RunError::Api(error)) if error.code==ErrorCode::Unsupported && error.detail.contains("messages.send_time_receipts_v1"))
        );
        assert_eq!(*client.calls.lock().unwrap(), vec![Command::Capabilities]);
        let retained = f.journal.load(&reference).unwrap();
        assert_eq!(
            retained.header.semantic_digest,
            original.header.semantic_digest
        );
        assert_eq!(retained.operation, original.operation);
        assert_eq!(
            serde_json::to_value(retained.semantic).unwrap(),
            serde_json::to_value(original.semantic).unwrap()
        );
    }
}

fn sent(client: &Client) -> Vec<herdr_threads::protocol::commands::SendMessage> {
    client
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Command::SendMessage(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn send_carries_the_priority_and_no_delivery_mode_key() {
    for n in [0u8, 1, 2, 9] {
        let f = Fixture::new();
        let client = Client::new(Ok(false));
        f.run(
            &["ht", "send", "t1", "--body", "p", "-p", &n.to_string()],
            &client,
            &mut Vec::new(),
        )
        .unwrap();
        // No capability round trip: a send that carries its priority needs neither lazy nor receipts probes.
        let calls = client.calls.lock().unwrap().clone();
        assert!(
            matches!(calls.as_slice(), [Command::SendMessage(_)]),
            "{calls:?}"
        );
        let sends = sent(&client);
        assert_eq!(sends[0].priority.map(|p| p.get()), Some(n));
        let json = serde_json::to_value(&sends[0]).unwrap();
        assert!(json.get("delivery_mode").is_none(), "{json}");
        assert_eq!(json["priority"], n);
    }
}

#[test]
fn default_send_records_priority_one() {
    let f = Fixture::new();
    let client = Client::new(Ok(false));
    assert!(
        f.run(
            &["ht", "send", "t1", "--body", "default"],
            &client,
            &mut LostOutput
        )
        .is_err()
    );
    let pending = f.journal.page(&Default::default()).unwrap();
    let reference = f
        .journal
        .resolve_recovery_ref(pending.items[0].recovery_ref.as_str())
        .unwrap();
    let saved = f.journal.load(&reference).unwrap();
    let json = serde_json::to_value(&saved.semantic).unwrap();
    assert_eq!(json["mutation"]["priority"], 1);
    assert!(json["mutation"].get("delivery_mode").is_none());
}

#[test]
fn priority_retry_keeps_the_frozen_priority_whatever_the_daemon_now_advertises() {
    let f = Fixture::new();
    let client = Client::new(Ok(false));
    assert!(
        f.run(
            &["ht", "send", "t1", "--body", "frozen", "-p", "3"],
            &client,
            &mut LostOutput
        )
        .is_err()
    );
    let pending = f.journal.page(&Default::default()).unwrap();
    let recovery = pending.items[0].recovery_ref.as_str().to_owned();
    // Retry against a daemon that no longer advertises the capability: same request, byte for byte.
    let downgraded = Client::new(Ok(false));
    f.run(&["ht", "retry", &recovery], &downgraded, &mut Vec::new())
        .unwrap();
    let first = sent(&client);
    let again = sent(&downgraded);
    assert_eq!(first.len(), 1);
    assert_eq!(again, first);
    assert_eq!(again[0].priority.map(|p| p.get()), Some(3));
}

#[test]
fn pre_upgrade_lazy_intent_retries_in_its_recorded_shape_on_a_priority_daemon() {
    let f = Fixture::new();
    let legacy: SemanticMutation = serde_json::from_str(r#"{"kind":"send_message","delivery_mode":"lazy","thread":"t1","body":"old","invited_recipients":[],"deadline_millis":null}"#).unwrap();
    let reference = f
        .journal
        .record(
            IntentScope::Cooperative {
                host: herdr_threads::test_support::test_host_locator(),
                instance: f.context.instance.to_string(),
                seat: herdr_threads::protocol::ids::SeatId::new(f.context.seat.clone()),
            },
            SemanticMutation::freeze(
                legacy,
                caller_claim(&f.context, herdr_threads::test_support::test_host_locator()).unwrap(),
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let client = Client::new(Ok(true));
    f.run(
        &["ht", "retry", &reference.recovery_ref()],
        &client,
        &mut Vec::new(),
    )
    .unwrap();
    let sends = sent(&client);
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].delivery_mode, DeliveryMode::Lazy);
    assert_eq!(sends[0].priority, None);
}
