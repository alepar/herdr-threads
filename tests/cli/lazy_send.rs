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
            instance: uuid::Uuid::from_u128(1),
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
    calls: Mutex<Vec<Command>>,
}
impl Client {
    fn new(capability: Result<bool, ErrorCode>) -> Self {
        Self {
            capability,
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
            Command::Capabilities => match self.capability {
                Ok(supported) => Ok(CommandResult::Capabilities(CapabilityList {
                    capabilities: if supported {
                        vec!["send.lazy_v1".into()]
                    } else {
                        vec![]
                    },
                })),
                Err(ErrorCode::Unsupported) => Err(ApiError::unsupported("old daemon")),
                Err(ErrorCode::InvalidRequest) => Err(ApiError::invalid_request("old daemon")),
                _ => panic!("unexpected fixture capability"),
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
fn lazy_send_flag_freezes_lazy_mode() {
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
                "--lazy",
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
    assert_eq!(send.delivery_mode, DeliveryMode::Lazy);
    assert!(send.relays_user);
    assert_eq!(
        send.user_intent,
        Some(herdr_threads::protocol::summary::UserIntent::Rule)
    );
    assert_eq!(send.claim, caller_claim(&f.context).unwrap());
    assert_eq!(
        serde_json::to_value(saved.semantic).unwrap()["mutation"]["delivery_mode"],
        "lazy"
    );
}
#[test]
fn lazy_send_rejects_ack_seats_panes_and_deadline() {
    for options in [
        vec!["--require-ack", "s1"],
        vec!["--require-ack-pane", "p1"],
        vec!["--deadline", "0"],
        vec!["--deadline", "10"],
        vec!["--require-ack-pane", "p1", "--space", "workspace"],
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
        assert!(
            error.detail.contains("--lazy cannot be combined"),
            "{error:?}"
        );
    }
}
#[test]
fn lazy_send_unsupported_daemon_records_no_intent() {
    for capability in [
        Ok(false),
        Err(ErrorCode::Unsupported),
        Err(ErrorCode::InvalidRequest),
    ] {
        for lazy_option in [vec![], vec!["--lazy"]] {
            let f = Fixture::new();
            let client = Client::new(capability.clone());
            let mut argv = vec!["ht", "send", "t1", "--body", "announcement"];
            argv.extend(lazy_option);
            let error = f.run(&argv, &client, &mut Vec::new()).unwrap_err();
            let RunError::Api(error) = error else {
                panic!("not API refusal")
            };
            assert_eq!(error.code, ErrorCode::Unsupported);
            assert!(error.detail.contains("send.lazy_v1") && error.detail.contains("upgrade"));
            assert_eq!(*client.calls.lock().unwrap(), vec![Command::Capabilities]);
            assert!(
                f.journal
                    .page(&Default::default())
                    .unwrap()
                    .items
                    .is_empty()
            );
            assert!(
                !std::fs::read_dir(f.root.join("intents")).unwrap().any(|e| e
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "intent"))
            );
        }
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
            Some(caller_claim(&f.context).unwrap()),
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
    let client = Client::new(Ok(false));
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
fn lazy_send_retry_preserves_frozen_claim_and_mode() {
    let f = Fixture::new();
    let client = Client::new(Ok(true));
    assert!(
        f.run(
            &["ht", "send", "t1", "--body", "announcement", "--lazy"],
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
    let old = Client::new(Ok(false));
    assert!(
        f.run(&["ht", "retry", recovery], &old, &mut Vec::new())
            .is_err()
    );
    assert_eq!(*old.calls.lock().unwrap(), vec![Command::Capabilities]);
    assert_eq!(f.journal.load(&reference).unwrap().semantic, saved.semantic);
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
    assert_eq!(sends[1].delivery_mode, DeliveryMode::Lazy);
    assert_eq!(sends[1].claim, caller_claim(&f.context).unwrap());
    assert_eq!(
        saved.header.scope,
        IntentScope::Cooperative {
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
            instance: f.context.instance.to_string(),
            seat: caller_claim(&f.context).unwrap().seat,
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

#[test]
fn lazy_send_default_and_nudge_ack_selection() {
    use herdr_threads::cli::commands::MutationSpec;
    for (options, expected) in [
        (vec![], DeliveryMode::Lazy),
        (vec!["--lazy"], DeliveryMode::Lazy),
        (vec!["--nudge"], DeliveryMode::Ordinary),
        (vec!["--require-ack", "s1"], DeliveryMode::Ordinary),
        (vec!["--require-ack-pane", "p1"], DeliveryMode::Ordinary),
        (
            vec!["--require-ack", "s1", "--deadline", "5"],
            DeliveryMode::Ordinary,
        ),
        (vec!["--nudge", "--deadline", "5"], DeliveryMode::Ordinary),
    ] {
        let mut argv = vec!["ht", "send", "t1", "--body", "announcement"];
        argv.extend(options);
        let parsed = parse_argv(argv).unwrap();
        let CliAction::Mutation(MutationSpec::Send { delivery_mode, .. }) = parsed.action else {
            panic!("not send")
        };
        assert_eq!(delivery_mode, expected);
    }
}
#[test]
fn lazy_send_default_refuses_deadline_and_explicit_mode_conflicts() {
    for options in [
        vec!["--deadline", "5"],
        vec!["--lazy", "--deadline", "5"],
        vec!["--lazy", "--require-ack", "s1"],
        vec!["--lazy", "--require-ack-pane", "p1"],
        vec!["--lazy", "--nudge"],
    ] {
        let mut argv = vec!["ht", "send", "t1", "--body", "announcement"];
        argv.extend(options);
        assert!(parse_argv(argv).is_err());
    }
}
