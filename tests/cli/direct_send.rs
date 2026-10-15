//! Direct CLI composition against the real accountable service/store, with reply loss.
use herdr_threads::{
    app::SystemClock,
    cli::{RunError, commands::parse_argv, journal::Journal, run_cooperative},
    harness::context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    ports::{LocalClient, LocalService},
    protocol::{
        commands::Command,
        results::{ApiError, CapabilityList, CommandResult},
        time::{CallBudget, Clock},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

/// The Herdr server of the isolated fixture: the namespace's `host_endpoint`.
fn isolated_host() -> herdr_threads::protocol::ids::HostLocator {
    herdr_threads::protocol::ids::HostLocator::parse("/isolated-host.sock").unwrap()
}

struct Fixture {
    root: PathBuf,
    journal: Journal,
    contexts: ContextJournal,
    db: rusqlite::Connection,
    client: Client,
}
struct Client {
    service: DomainService,
    supported: bool,
    omit_creation_policy: bool,
    /// Exact discovery reply for scoped mixed-capability peer controls.
    capability_override: Option<Vec<String>>,
    lose: Mutex<Option<&'static str>>,
    calls: Mutex<Vec<Command>>,
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
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        if *self.lose.lock().unwrap() == Some("unavailable") {
            return Err(ApiError::host_unavailable("injected unavailable daemon"));
        }
        if matches!(command, Command::Capabilities) {
            return Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: if let Some(capabilities) = &self.capability_override {
                    capabilities.clone()
                } else if self.supported {
                    herdr_threads::protocol::capabilities::ADVERTISED
                        .iter()
                        .filter(|name| {
                            !self.omit_creation_policy
                                || **name != "direct_threads.ensure_creation_policy_v1"
                        })
                        .map(|name| (*name).into())
                        .collect()
                } else {
                    vec![]
                },
            }));
        }
        let phase = match &command {
            Command::EnsureDirectThread(_) => "ensure",
            Command::SendMessage(_) => "send",
            _ => "read",
        };
        if phase == "send" && *self.lose.lock().unwrap() == Some("before_send") {
            *self.lose.lock().unwrap() = None;
            return Err(ApiError::unknown_outcome(
                "injected crash after progress sync",
            ));
        }
        let result = self.service.handle(
            command,
            herdr_threads::test_support::peer_identity(501),
            budget,
        )?;
        let mut lose = self.lose.lock().unwrap();
        if *lose == Some(phase) {
            *lose = None;
            return Err(ApiError::unknown_outcome("injected lost committed reply"));
        }
        Ok(result)
    }
}
impl Fixture {
    fn new(human: bool) -> Self {
        let root = std::env::temp_dir().join(format!("ht-direct-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let instance = herdr_threads::node::NodeUuid::from_u128(1);
        let context = StoreContext::new(root.join("store.db"), clock.clone())
            .with_test_host()
            .with_node_seed(herdr_threads::test_support::node_seed_for(instance));
        let db = context.open_writer().unwrap();
        db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        for seat in ["seat-a", "seat-b"] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES(?1,?2,'resolved','native',?1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", rusqlite::params![seat, instance.to_string()]).unwrap();
            db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts WHERE node_id=?1),?2,'b',1,1,0,'fresh','term-'||?2,'inc','coherent_enumeration',1)", rusqlite::params![instance.to_string(), seat]).unwrap();
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES(?1,1,?1,'b',1,1,?2,'session','00000000-0000-4000-8000-000000000002',?3,0,'term-'||?1,'inc')", rusqlite::params![seat, if human {"human"} else {"codex"}, if human {"operator_human"} else {"cooperative_top_level"}]).unwrap();
        }
        let store = Arc::new(
            SqliteStore::new(context, instance.to_string(), StoreSettings::default()).unwrap(),
        );
        let service = DomainService::new(instance.to_string(), store, clock)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
        let directory = root.join("contexts");
        std::fs::create_dir(&directory).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let contexts = ContextJournal::open(&directory, instance, "seat-a", Duration::from_secs(1))
            .map(|journal| journal.with_host(isolated_host()))
            .unwrap();
        contexts
            .install_reattached(OccupantContext {
                format_version: 1,
                instance,
                seat: "seat-a".into(),
                target: "seat-a".into(),
                harness: if human {
                    Harness::Human
                } else {
                    Harness::Codex
                },
                binding_generation: 1,
                execution: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap(),
                session: SessionReference::Native("session".into()),
                role: Role::TopLevel,
            })
            .unwrap();
        let journal = Journal::open(root.join("intents")).unwrap();
        Self {
            root,
            journal,
            contexts,
            db,
            client: Client {
                service,
                supported: true,
                omit_creation_policy: false,
                capability_override: None,
                lose: Mutex::new(None),
                calls: Mutex::new(vec![]),
            },
        }
    }
    fn run(&self, args: &[&str], writer: &mut impl Write) -> Result<(), RunError> {
        self.run_with_journal(args, writer, &self.journal)
    }
    fn run_with_journal(
        &self,
        args: &[&str],
        writer: &mut impl Write,
        journal: &Journal,
    ) -> Result<(), RunError> {
        let state = self.root.to_str().unwrap();
        let mut argv = vec!["ht"];
        if args.first() == Some(&"human") {
            argv.push("human");
        }
        argv.extend([
            "--state-dir",
            state,
            "--host-endpoint",
            "/isolated-host.sock",
        ]);
        argv.extend(args.iter().copied().filter(|arg| *arg != "human"));
        run_cooperative(
            parse_argv(argv)?,
            journal,
            &self.contexts,
            None,
            Role::TopLevel,
            &self.client,
            &SystemClock::new(),
            writer,
        )
    }
    fn count(&self, table: &str) -> i64 {
        self.db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    fn recovery(&self) -> String {
        self.journal.page(&Default::default()).unwrap().items[0]
            .recovery_ref
            .as_str()
            .to_owned()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct LostOutput;
impl Write for LostOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("lost output"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Kills: admitting a new ordinary direct compound from Direct/Ensure support
// alone, mutating the pair before send-time receipt semantics are negotiated.
#[test]

fn modern_ordinary_direct_send_requires_receipt_capability_before_any_effect() {
    use herdr_threads::protocol::{
        capabilities::{DIRECT_THREADS_V1, ENSURE_CREATION_POLICY_V1},
        results::ErrorCode,
    };
    for flags in [
        vec!["--nudge"],
        vec!["--require-ack", "seat-b", "--deadline", "30"],
    ] {
        let mut f = Fixture::new(false);
        f.client.capability_override = Some(vec![
            DIRECT_THREADS_V1.into(),
            ENSURE_CREATION_POLICY_V1.into(),
        ]);
        let effects = [
            "direct_threads",
            "threads",
            "invitations",
            "memberships",
            "messages",
            "operations",
            "send_preparations",
            "send_manifests",
            "prepared_recipients",
            "lazy_recipients",
            "receipts",
            "receipt_state",
            "wake_work",
        ];
        let before = effects.map(|table| f.count(table));
        let mut args = vec!["send", "--seat", "seat-b", "--body", "modern ordinary"];
        args.extend(flags);
        let mut output = Vec::new();
        let result = f.run(&args, &mut output);
        assert!(
            matches!(result, Err(RunError::Api(ref error)) if error.code == ErrorCode::Unsupported),
            "Direct/Ensure without send-time receipts must refuse the new ordinary compound: {result:?}"
        );
        assert!(output.is_empty());
        assert!(
            f.journal
                .page(&Default::default())
                .unwrap()
                .items
                .is_empty()
        );
        assert_eq!(effects.map(|table| f.count(table)), before);
        assert!(
            matches!(
                f.client.calls.lock().unwrap().as_slice(),
                [Command::Capabilities]
            ),
            "refusal must precede journal, inspection, Ensure and Send"
        );
    }
}

#[test]
fn modern_ordinary_direct_send_with_receipt_capability_commits_exact_ordinary_body() {
    use herdr_threads::protocol::{
        capabilities::{DIRECT_THREADS_V1, ENSURE_CREATION_POLICY_V1, SEND_TIME_RECEIPTS_V1},
        commands::{DeliveryMode, InvitationConfirmation},
    };
    let mut f = Fixture::new(false);
    f.client.capability_override = Some(vec![
        DIRECT_THREADS_V1.into(),
        ENSURE_CREATION_POLICY_V1.into(),
        SEND_TIME_RECEIPTS_V1.into(),
    ]);
    let mut output = Vec::new();
    f.run(
        &[
            "send",
            "--seat",
            "seat-b",
            "--body",
            "modern ordinary",
            "--nudge",
        ],
        &mut output,
    )
    .unwrap();
    assert!(!output.is_empty());
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(f.count("lazy_recipients"), 0);
    let message: String = f.db.query_row(
        "SELECT id FROM messages WHERE body='modern ordinary' AND kind='ordinary' AND delivery_mode='ordinary'",
        [], |row| row.get(0),
    ).unwrap();
    let receipt = herdr_threads::store::effective::effective_receipt(&f.db, &message, "seat-b")
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.state,
        herdr_threads::store::effective::EffectiveReceiptState::Pending
    );
    assert!(receipt.deadline_at.is_some());
    let calls = f.client.calls.lock().unwrap();
    assert!(calls.iter().any(|command| matches!(command,
        Command::EnsureDirectThread(ensure) if ensure.creation_policy.as_ref().is_some_and(|policy|
            policy.confirmation == InvitationConfirmation::Lazy && policy.deadline_millis.is_none())
    )));
    let sends = calls
        .iter()
        .filter_map(|command| match command {
            Command::SendMessage(send) => Some(send),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].delivery_mode, DeliveryMode::Ordinary);
    assert_eq!(sends[0].body, "modern ordinary");
    assert_eq!(
        sends[0].direct_recipient.as_ref().unwrap().as_str(),
        "seat-b"
    );
}

#[test]
fn modern_lazy_direct_send_is_exempt_from_receipt_capability() {
    use herdr_threads::protocol::{
        capabilities::{DIRECT_THREADS_V1, ENSURE_CREATION_POLICY_V1, LAZY_SEND},
        commands::{DeliveryMode, InvitationConfirmation},
    };
    let mut f = Fixture::new(false);
    f.client.capability_override = Some(vec![
        DIRECT_THREADS_V1.into(),
        ENSURE_CREATION_POLICY_V1.into(),
        LAZY_SEND.into(),
    ]);
    f.run(
        &["send", "--seat", "seat-b", "--body", "modern lazy"],
        &mut Vec::new(),
    )
    .unwrap();
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(f.count("lazy_recipients"), 1);
    assert_eq!(f.count("receipts"), 0);
    assert_eq!(f.count("prepared_recipients"), 0);
    assert_eq!(f.count("receipt_state"), 0);
    let calls = f.client.calls.lock().unwrap();
    assert!(calls.iter().any(|command| matches!(command,
        Command::EnsureDirectThread(ensure) if ensure.creation_policy.as_ref().is_some_and(|policy|
            policy.confirmation == InvitationConfirmation::Lazy && policy.deadline_millis.is_none())
    )));
    let sends = calls
        .iter()
        .filter_map(|command| match command {
            Command::SendMessage(send) => Some(send),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].delivery_mode, DeliveryMode::Lazy);
    assert_eq!(sends[0].body, "modern lazy");
    assert_eq!(
        sends[0].direct_recipient.as_ref().unwrap().as_str(),
        "seat-b"
    );
    assert!(sends[0].invited_recipients.is_empty());
    assert!(sends[0].deadline_millis.is_none());
}

fn record_historical_direct_receipt_control(
    f: &Fixture,
    mode: herdr_threads::protocol::commands::DeliveryMode,
    body: &str,
) -> herdr_threads::cli::journal::IntentRef {
    use herdr_threads::{
        cli::{
            direct_send::{Plan, Request},
            journal::{IntentScope, SemanticMutation},
        },
        protocol::{
            commands::DeliveryMode,
            handoff::HandoffNamespace,
            ids::{OperationId, SeatId},
        },
    };
    let context = f.contexts.current().unwrap().unwrap();
    let claim = f.contexts.claim(&context).unwrap();
    let target = SeatId::new("seat-b");
    let plan = Plan {
        creation_policy: None,
        namespace: HandoffNamespace {
            instance: claim.instance.clone(),
            state_dir: f.root.clone(),
            host_endpoint: PathBuf::from("/isolated-host.sock"),
        },
        request: Request {
            target: target.clone(),
            delivery_mode: mode,
            body: body.into(),
            require_ack: if mode == DeliveryMode::Ordinary {
                vec![target]
            } else {
                vec![]
            },
            deadline_millis: (mode == DeliveryMode::Ordinary).then_some(30_000),
            relays_user: false,
            user_intent: None,
        },
        ensure_key: OperationId::new(uuid::Uuid::new_v4().to_string()),
        send_key: OperationId::new(uuid::Uuid::new_v4().to_string()),
    };
    let scope = IntentScope::Cooperative {
        host: isolated_host(),
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let semantic =
        SemanticMutation::freeze(SemanticMutation::DirectSend(Box::new(plan)), claim).unwrap();
    assert!(
        !serde_json::to_string(&semantic)
            .unwrap()
            .contains("creation_policy")
    );
    f.journal.record(scope, semantic, 0).unwrap()
}

#[test]
fn pending_historical_ordinary_direct_retry_requires_receipts_without_rewriting_original() {
    use herdr_threads::{
        cli::journal::SemanticMutation,
        protocol::{
            capabilities::{DIRECT_THREADS_V1, ENSURE_CREATION_POLICY_V1},
            commands::DeliveryMode,
            results::ErrorCode,
        },
    };
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new(false);
    let reference =
        record_historical_direct_receipt_control(&f, DeliveryMode::Ordinary, "retained ordinary");
    let original = f.journal.load(&reference).unwrap();
    assert_eq!(
        f.journal.page(&Default::default()).unwrap().items[0].status,
        herdr_threads::protocol::results::IntentStatus::Pending
    );
    let original_bytes = serde_json::to_vec(&original.semantic).unwrap();
    assert_eq!(
        original.header.semantic_digest,
        format!("{:x}", Sha256::digest(&original_bytes))
    );
    let intent_path = std::fs::read_dir(f.root.join("intents"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "intent"))
        .unwrap();
    let stored_original = std::fs::read(&intent_path).unwrap();
    let before = [
        f.count("direct_threads"),
        f.count("invitations"),
        f.count("messages"),
        f.count("operations"),
        f.count("send_preparations"),
        f.count("prepared_recipients"),
    ];
    f.client.capability_override = Some(vec![
        DIRECT_THREADS_V1.into(),
        ENSURE_CREATION_POLICY_V1.into(),
    ]);
    let result = f.run(&["retry", &reference.recovery_ref()], &mut Vec::new());
    assert!(
        matches!(result, Err(RunError::Api(ref error)) if error.code == ErrorCode::Unsupported),
        "pending historical ordinary retry must refuse before additional effects: {result:?}"
    );
    assert_eq!(
        [
            f.count("direct_threads"),
            f.count("invitations"),
            f.count("messages"),
            f.count("operations"),
            f.count("send_preparations"),
            f.count("prepared_recipients")
        ],
        before
    );
    assert!(matches!(
        f.client.calls.lock().unwrap().as_slice(),
        [Command::Capabilities]
    ));
    assert_eq!(std::fs::read(&intent_path).unwrap(), stored_original);
    let retained = f.journal.load(&reference).unwrap();
    assert_eq!(
        serde_json::to_vec(&retained.semantic).unwrap(),
        original_bytes
    );
    assert_eq!(retained.semantic, original.semantic);
    assert_eq!(
        retained.header.semantic_digest,
        original.header.semantic_digest
    );
    let SemanticMutation::Frozen { claim, mutation } = &retained.semantic else {
        panic!("historical frozen claim");
    };
    let SemanticMutation::DirectSend(plan) = mutation.as_ref() else {
        panic!("historical direct plan");
    };
    assert!(plan.creation_policy.is_none());
    assert_eq!(claim, original.semantic.frozen_claim().unwrap());

    // The same immutable historical original remains executable on a capable peer.
    f.client.capability_override = None;
    f.client.calls.lock().unwrap().clear();
    f.run(&["retry", &reference.recovery_ref()], &mut Vec::new())
        .unwrap();
    let calls = f.client.calls.lock().unwrap();
    let ensure = calls
        .iter()
        .find_map(|command| match command {
            Command::EnsureDirectThread(value) => Some(value),
            _ => None,
        })
        .unwrap();
    let send = calls
        .iter()
        .find_map(|command| match command {
            Command::SendMessage(value) => Some(value),
            _ => None,
        })
        .unwrap();
    assert!(ensure.creation_policy.is_none());
    assert_eq!(ensure.operation, plan.ensure_key);
    assert_eq!(send.operation, plan.send_key);
    assert_eq!(&ensure.claim, claim);
    assert_eq!(&send.claim, claim);
    assert_eq!(send.body, plan.request.body);
    assert_eq!(send.delivery_mode, plan.request.delivery_mode);
    assert_eq!(send.invited_recipients, plan.request.require_ack);
    assert_eq!(send.deadline_millis, plan.request.deadline_millis);
}

#[test]
fn historical_lazy_direct_retry_is_exempt_from_receipt_capability() {
    use herdr_threads::protocol::{
        capabilities::{DIRECT_THREADS_V1, LAZY_SEND},
        commands::DeliveryMode,
    };
    let mut f = Fixture::new(false);
    let reference =
        record_historical_direct_receipt_control(&f, DeliveryMode::Lazy, "retained lazy");
    f.client.capability_override = Some(vec![DIRECT_THREADS_V1.into(), LAZY_SEND.into()]);
    f.run(&["retry", &reference.recovery_ref()], &mut Vec::new())
        .unwrap();
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(f.count("lazy_recipients"), 1);
    assert_eq!(f.count("receipts"), 0);
    assert_eq!(f.count("receipt_state"), 0);
    let calls = f.client.calls.lock().unwrap();
    assert!(calls.iter().any(|command| matches!(command, Command::EnsureDirectThread(ensure) if ensure.creation_policy.is_none())));
    assert!(calls.iter().any(|command| matches!(command, Command::SendMessage(send) if send.delivery_mode == DeliveryMode::Lazy && send.body == "retained lazy")));
}

#[test]
fn committed_direct_ordinary_output_retry_does_not_require_receipt_capability() {
    use herdr_threads::protocol::{
        capabilities::{DIRECT_THREADS_V1, ENSURE_CREATION_POLICY_V1},
        commands::DeliveryMode,
    };
    for historical in [false, true] {
        let mut f = Fixture::new(false);
        let result = if historical {
            let reference = record_historical_direct_receipt_control(
                &f,
                DeliveryMode::Ordinary,
                "committed ordinary",
            );
            f.run(&["retry", &reference.recovery_ref()], &mut LostOutput)
        } else {
            f.run(
                &[
                    "send",
                    "--seat",
                    "seat-b",
                    "--body",
                    "committed ordinary",
                    "--nudge",
                ],
                &mut LostOutput,
            )
        };
        assert!(
            matches!(result, Err(RunError::Io(ref error)) if error.to_string().contains("lost output"))
        );
        let recovery = f.recovery();
        let reference = f.journal.resolve_recovery_ref(&recovery).unwrap();
        let retained = f.journal.load(&reference).unwrap();
        assert_eq!(
            f.journal.page(&Default::default()).unwrap().items[0].status,
            herdr_threads::protocol::results::IntentStatus::CommittedAwaitingOutput
        );
        let original_commands = f.client.calls.lock().unwrap().clone();
        let before = [
            f.count("direct_threads"),
            f.count("invitations"),
            f.count("messages"),
            f.count("operations"),
            f.count("send_manifests"),
            f.count("prepared_recipients"),
        ];
        let message: String =
            f.db.query_row(
                "SELECT id FROM messages WHERE body='committed ordinary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        f.client.capability_override = Some(vec![
            DIRECT_THREADS_V1.into(),
            ENSURE_CREATION_POLICY_V1.into(),
        ]);
        f.client.calls.lock().unwrap().clear();
        let mut output = Vec::new();
        f.run(&["retry", &recovery], &mut output).unwrap();
        assert!(String::from_utf8(output).unwrap().contains(&message));
        assert_eq!(
            [
                f.count("direct_threads"),
                f.count("invitations"),
                f.count("messages"),
                f.count("operations"),
                f.count("send_manifests"),
                f.count("prepared_recipients")
            ],
            before
        );
        assert!(
            f.journal
                .page(&Default::default())
                .unwrap()
                .items
                .is_empty()
        );
        for command in f.client.calls.lock().unwrap().iter() {
            if let Command::EnsureDirectThread(ensure) = command {
                assert_eq!(ensure.creation_policy.is_none(), historical);
                assert_eq!(Some(&ensure.claim), retained.semantic.frozen_claim());
                assert!(original_commands.contains(command));
            }
            if let Command::SendMessage(send) = command {
                assert_eq!(Some(&send.claim), retained.semantic.frozen_claim());
                assert!(original_commands.contains(command));
            }
        }
    }
}

#[test]
fn unsupported_daemon_refuses_before_compound_and_pair_effects() {
    let mut f = Fixture::new(false);
    f.client.supported = false;
    let error = f
        .run(
            &["send", "--seat", "seat-b", "--body", "hello"],
            &mut vec![],
        )
        .unwrap_err();
    assert!(format!("{error:?}").contains("Unsupported"));
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 0);
    assert!(
        !f.client
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::EnsureDirectThread(_) | Command::SendMessage(_)))
    );
}

#[test]
fn lost_committed_replies_restart_with_exact_keys_body_and_lazy_audience() {
    for phase in ["ensure", "before_send", "send"] {
        let f = Fixture::new(false);
        *f.client.lose.lock().unwrap() = Some(phase);
        assert!(
            f.run(
                &["send", "--seat", "seat-b", "--body", "frozen body"],
                &mut vec![]
            )
            .is_err()
        );
        let recovery = f.recovery();
        assert_eq!(f.count("direct_threads"), 1);
        let reopened = Journal::open(f.root.join("intents")).unwrap();
        assert_eq!(reopened.page(&Default::default()).unwrap().items.len(), 1);
        let mut output = vec![];
        f.run_with_journal(&["retry", &recovery], &mut output, &reopened)
            .unwrap();
        assert_eq!(f.count("direct_threads"), 1);
        assert_eq!(f.count("invitations"), 1);
        assert_eq!(f.count("lazy_recipients"), 1);
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM messages WHERE body='frozen body'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(
            f.journal
                .page(&Default::default())
                .unwrap()
                .items
                .is_empty()
        );
        let calls = f.client.calls.lock().unwrap();
        let ensures: Vec<_> = calls
            .iter()
            .filter_map(|c| {
                if let Command::EnsureDirectThread(v) = c {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(ensures.len(), 2);
        assert_eq!(ensures[0], ensures[1]);
        let sends: Vec<_> = calls
            .iter()
            .filter_map(|c| {
                if let Command::SendMessage(v) = c {
                    Some(v)
                } else {
                    None
                }
            })
            .collect();
        assert!(!sends.is_empty());
        for send in &sends {
            assert_eq!(*send, sends[0]);
            assert!(send.invited_recipients.is_empty());
            assert_eq!(send.direct_recipient.as_ref().unwrap().as_str(), "seat-b");
        }
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains(sends[0].thread.as_str()));
    }
}

fn assert_direct_send_frozen_claim_survives_changed_caller(
    fingerprint: Option<herdr_threads::protocol::authority::LivenessFingerprint>,
) {
    use herdr_threads::{cli::journal::SemanticMutation, protocol::authority::LivenessFingerprint};
    use sha2::{Digest, Sha256};

    let mut f = Fixture::new(false);
    let original_context = f.contexts.current().unwrap().unwrap();
    let mut expected =
        herdr_threads::harness::bridge::caller_claim(&original_context, isolated_host()).unwrap();
    expected.liveness = fingerprint;
    let open_contexts = || {
        ContextJournal::open(
            &f.root.join("contexts"),
            original_context.instance,
            &original_context.seat,
            Duration::from_secs(1),
        )
        .map(|journal| journal.with_host(isolated_host()))
        .unwrap()
    };
    if let Some(fingerprint) = fingerprint {
        f.contexts = open_contexts()
            .with_agent_caller(original_context.clone(), fingerprint)
            .unwrap();
    }
    // The legacy handle keeps the historical None representation without editing an intent.
    *f.client.lose.lock().unwrap() = Some("before_send");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "frozen caller body"],
            &mut vec![],
        )
        .is_err()
    );
    let recovery = f.recovery();
    let reopened = Journal::open(f.root.join("intents")).unwrap();
    let reference = reopened.resolve_recovery_ref(&recovery).unwrap();
    let original = reopened.load(&reference).unwrap();
    let SemanticMutation::Frozen { claim, mutation } = &original.semantic else {
        panic!("direct send must retain its original frozen claim")
    };
    assert!(matches!(mutation.as_ref(), SemanticMutation::DirectSend(_)));
    assert_eq!(claim, &expected);
    let original_bytes = serde_json::to_vec(&original.semantic).unwrap();
    assert_eq!(
        original.header.semantic_digest,
        format!("{:x}", Sha256::digest(&original_bytes))
    );

    // Replace the environment-derived session, execution and fingerprint at the same seat.
    let mut successor = original_context.clone();
    successor.session = SessionReference::Native("successor-session".into());
    successor.execution = uuid::Uuid::from_u128(3);
    f.contexts = open_contexts()
        .with_agent_caller(successor, LivenessFingerprint { pid: 2, start: 2 })
        .unwrap();
    let current = f.contexts.current().unwrap().unwrap();
    let current_claim = f.contexts.claim(&current).unwrap();
    assert_ne!(current_claim.native_session, expected.native_session);
    assert_ne!(current_claim.execution, expected.execution);
    assert_ne!(current_claim.liveness, expected.liveness);

    // Commit the original send, but retain the durable intent through the real output seam.
    let error = f
        .run_with_journal(&["retry", &recovery], &mut LostOutput, &reopened)
        .unwrap_err();
    assert!(matches!(error, RunError::Io(ref e) if e.to_string().contains("lost output")));
    let retained = reopened.load(&reference).unwrap();
    assert_eq!(retained.semantic, original.semantic);
    assert_eq!(
        serde_json::to_vec(&retained.semantic).unwrap(),
        original_bytes
    );
    assert_eq!(
        serde_json::to_value(&retained.header).unwrap(),
        serde_json::to_value(&original.header).unwrap()
    );
    f.run_with_journal(&["retry", &recovery], &mut vec![], &reopened)
        .unwrap();
    assert!(reopened.page(&Default::default()).unwrap().items.is_empty());
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(f.count("invitations"), 1);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='frozen caller body'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    let calls = f.client.calls.lock().unwrap();
    let ensures: Vec<_> = calls
        .iter()
        .filter_map(|command| match command {
            Command::EnsureDirectThread(request) => Some(request),
            _ => None,
        })
        .collect();
    let sends: Vec<_> = calls
        .iter()
        .filter_map(|command| match command {
            Command::SendMessage(request) => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(ensures.len(), 3);
    assert_eq!(sends.len(), 3);
    for request in &ensures {
        assert_eq!(*request, ensures[0]);
        assert_eq!(request.claim, expected);
    }
    for request in &sends {
        assert_eq!(*request, sends[0]);
        assert_eq!(request.claim, expected);
        assert_eq!(request.body, "frozen caller body");
    }
}

#[test]
fn frozen_some_claim_survives_direct_send_durable_retry_with_changed_caller() {
    assert_direct_send_frozen_claim_survives_changed_caller(Some(
        herdr_threads::protocol::authority::LivenessFingerprint::TEST,
    ));
}

#[test]
fn historical_none_claim_survives_direct_send_durable_retry_with_changed_caller() {
    assert_direct_send_frozen_claim_survives_changed_caller(None);
}

#[test]
fn completed_output_loss_replays_history_without_new_effects() {
    let f = Fixture::new(false);
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "hello"],
            &mut LostOutput
        )
        .is_err()
    );
    let recovery = f.recovery();
    let before = [
        f.count("messages"),
        f.count("invitations"),
        f.count("operations"),
    ];
    let expected_thread = f.journal.page(&Default::default()).unwrap().items[0]
        .thread
        .clone()
        .unwrap();
    let expected_message: String =
        f.db.query_row("SELECT id FROM messages WHERE body='hello'", [], |r| {
            r.get(0)
        })
        .unwrap();
    f.db.execute("UPDATE threads SET archived=1", []).unwrap();
    f.db.execute(
        "UPDATE seats SET state='retired', retired_at=1, retired_seq=(SELECT decision_seq+1 FROM local_node LIMIT 1), generation=generation+1 WHERE id='seat-b'",
        [],
    )
    .unwrap();
    f.db.execute("UPDATE seats SET generation=2 WHERE id='seat-a'", [])
        .unwrap();
    f.db.execute("UPDATE occupant_bindings SET harness='human', native_session='successor' WHERE seat_id='seat-a'", []).unwrap();
    let original = f.client.calls.lock().unwrap().clone();
    let mut output = vec![];
    f.run(&["retry", &recovery], &mut output).unwrap();
    assert_eq!(
        [
            f.count("messages"),
            f.count("invitations"),
            f.count("operations")
        ],
        before
    );
    let calls = f.client.calls.lock().unwrap();
    for command in calls
        .iter()
        .filter(|c| matches!(c, Command::EnsureDirectThread(_) | Command::SendMessage(_)))
    {
        assert!(
            original.contains(command),
            "historical retry must use exact original children"
        );
    }
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.contains(expected_thread.as_str()));
    assert!(rendered.contains(&expected_message));
}

#[test]
fn invalid_target_and_caller_refuse_before_intent_or_pair() {
    for case in [
        "self",
        "missing",
        "retired",
        "unresolved",
        "held",
        "stale-caller",
    ] {
        let f = Fixture::new(false);
        match case {
            "retired" => {
                f.db.execute(
                    "UPDATE seats SET state='retired', retired_at=1, retired_seq=(SELECT decision_seq+1 FROM local_node LIMIT 1), generation=generation+1 WHERE id='seat-b'",
                    [],
                )
                .unwrap();
            }
            "unresolved" => {
                f.db.execute("UPDATE seats SET state='unresolved' WHERE id='seat-b'", [])
                    .unwrap();
            }
            "held" => {
                f.db.execute("INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ((SELECT min(host_id) FROM herdr_hosts WHERE node_id=?1),'seat-b','b',1,'operator')", [herdr_threads::node::NodeUuid::from_u128(1).to_string()]).unwrap();
            }
            "stale-caller" => {
                f.db.execute("UPDATE seats SET generation=2 WHERE id='seat-a'", [])
                    .unwrap();
            }
            _ => {}
        }
        let target = match case {
            "self" => "seat-a",
            "missing" => "missing",
            _ => "seat-b",
        };
        assert!(
            f.run(&["send", "--seat", target, "--body", "hello"], &mut vec![])
                .is_err(),
            "{case}"
        );
        assert!(
            f.journal
                .page(&Default::default())
                .unwrap()
                .items
                .is_empty(),
            "{case}"
        );
        assert_eq!(f.count("direct_threads"), 0, "{case}");
    }
}

#[test]
fn resolved_unbound_peer_is_a_valid_lazy_target() {
    let f = Fixture::new(false);
    f.db.execute("DELETE FROM occupant_bindings WHERE seat_id='seat-b'", [])
        .unwrap();
    f.run(
        &["send", "--seat", "seat-b", "--body", "hello"],
        &mut vec![],
    )
    .unwrap();
    assert_eq!(f.count("lazy_recipients"), 1);
    assert_eq!(f.count("receipts"), 0);
}

#[test]
fn nudge_and_explicit_target_ack_use_ordinary_delivery() {
    for flags in [
        vec!["--nudge"],
        vec!["--require-ack", "seat-b", "seat-b", "--deadline", "30"],
    ] {
        let f = Fixture::new(false);
        let mut args = vec!["send", "--seat", "seat-b", "--body", "hello"];
        args.extend(flags);
        f.run(&args, &mut vec![]).unwrap();
        assert_eq!(f.count("lazy_recipients"), 0);
        let message: String =
            f.db.query_row("SELECT id FROM messages WHERE body='hello'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let recipients: Vec<(String, i64)> = f.db.prepare("SELECT pr.seat_id, pr.ack_required FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1").unwrap().query_map([&message], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(recipients, vec![("seat-b".into(), 1)]);
        let receipt = herdr_threads::store::effective::effective_receipt(&f.db, &message, "seat-b")
            .unwrap()
            .unwrap();
        assert_eq!(
            receipt.state,
            herdr_threads::store::effective::EffectiveReceiptState::Pending
        );
        assert_eq!(receipt.seat_id, "seat-b");
        assert_eq!(f.count("invitations"), 1);
    }
}

#[test]
fn human_direct_send_and_original_actor_retry_preserve_no_receipts() {
    let f = Fixture::new(true);
    assert!(
        f.run(
            &[
                "human", "send", "--seat", "seat-b", "--body", "hello", "--nudge"
            ],
            &mut LostOutput
        )
        .is_err()
    );
    let recovery = f.recovery();
    let before = f.client.calls.lock().unwrap().len();
    assert!(f.run(&["retry", &recovery], &mut vec![]).is_err());
    assert_eq!(f.client.calls.lock().unwrap().len(), before);
    let mut successor = f.contexts.current().unwrap().unwrap();
    successor.harness = Harness::Codex;
    successor.binding_generation = 2;
    successor.session = SessionReference::Native("successor".into());
    f.contexts.install_reattached(successor).unwrap();
    f.run(&["human", "retry", &recovery], &mut vec![]).unwrap();
    assert_eq!(f.count("receipts"), 0);
}

#[test]
fn direct_file_body_is_frozen_across_send_failure() {
    let f = Fixture::new(false);
    let path = f.root.join("body.txt");
    std::fs::write(&path, "file body\n雪").unwrap();
    *f.client.lose.lock().unwrap() = Some("ensure");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--file", path.to_str().unwrap()],
            &mut vec![]
        )
        .is_err()
    );
    std::fs::write(&path, "changed defaults").unwrap();
    f.run(&["retry", &f.recovery()], &mut vec![]).unwrap();
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='file body\n雪'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn staged_failure_retains_body_and_exposes_one_exact_retry() {
    let f = Fixture::new(false);
    *f.client.lose.lock().unwrap() = Some("before_send");
    let error = f
        .run(
            &["send", "--seat", "seat-b", "--body", "recover me"],
            &mut vec![],
        )
        .unwrap_err();
    let page = f.journal.page(&Default::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].status,
        herdr_threads::protocol::results::IntentStatus::Submitted
    );
    assert_eq!(
        serde_json::to_value(page.items[0].kind).unwrap(),
        "direct_send"
    );
    let thread = page.items[0].thread.as_ref().unwrap();
    let detail = format!("{error:?}");
    assert!(detail.contains(thread.as_str()));
    assert!(detail.contains(&page.items[0].recovery_ref.as_str().to_string()));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='recover me'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    f.run(&["retry", &f.recovery()], &mut vec![]).unwrap();
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='recover me'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn contradictory_saved_send_is_refused_without_publishing_another_message() {
    let f = Fixture::new(false);
    *f.client.lose.lock().unwrap() = Some("before_send");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "original"],
            &mut vec![]
        )
        .is_err()
    );
    let recovery = f.recovery();
    let path = std::fs::read_dir(f.root.join("intents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|ext| ext == "progress"))
        .unwrap();
    let mut progress: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    progress["send"]["body"] = "replaced".into();
    std::fs::write(&path, serde_json::to_vec(&progress).unwrap()).unwrap();
    assert!(f.run(&["retry", &recovery], &mut vec![]).is_err());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body IN ('original','replaced')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(f.count("invitations"), 1);
}

#[test]
fn direct_agent_user_intent_requires_relay_before_local_effects() {
    let f = Fixture::new(false);
    assert!(
        f.run(
            &[
                "send",
                "--seat",
                "seat-b",
                "--body",
                "hello",
                "--user-intent",
                "request"
            ],
            &mut vec![]
        )
        .is_err()
    );
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 0);
    f.run(
        &[
            "send",
            "--seat",
            "seat-b",
            "--body",
            "hello",
            "--user-intent",
            "request",
            "--relays-user",
        ],
        &mut vec![],
    )
    .unwrap();
    assert_eq!(f.db.query_row("SELECT count(*) FROM messages WHERE body='hello' AND user_intent='request' AND relays_user=1", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn completed_retry_unavailable_retains_original_recovery_files() {
    let f = Fixture::new(false);
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "hello"],
            &mut LostOutput
        )
        .is_err()
    );
    let recovery = f.recovery();
    let before = [
        f.count("messages"),
        f.count("invitations"),
        f.count("operations"),
    ];
    *f.client.lose.lock().unwrap() = Some("unavailable");
    let error = f.run(&["retry", &recovery], &mut vec![]).unwrap_err();
    let RunError::Api(api) = error else {
        panic!("unavailable API error expected")
    };
    let argv = api.restart_argv.as_ref().unwrap();
    assert_eq!(argv.last(), Some(&recovery));
    assert!(argv.contains(&f.root.to_string_lossy().into_owned()));
    assert!(argv.contains(&"/isolated-host.sock".to_owned()));
    assert_eq!(api.detail.matches("recover with ").count(), 1);
    assert_eq!(f.recovery(), recovery);
    assert_eq!(
        [
            f.count("messages"),
            f.count("invitations"),
            f.count("operations")
        ],
        before
    );
    *f.client.lose.lock().unwrap() = None;
    f.run(&["retry", &recovery], &mut vec![]).unwrap();
}

#[test]
fn consistently_edited_selected_thread_is_refused_at_canonical_ensure() {
    let f = Fixture::new(false);
    *f.client.lose.lock().unwrap() = Some("before_send");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "original"],
            &mut vec![]
        )
        .is_err()
    );
    let recovery = f.recovery();
    let path = std::fs::read_dir(f.root.join("intents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|ext| ext == "progress"))
        .unwrap();
    let mut progress: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    progress["thread"] = "edited-thread".into();
    progress["send"]["thread"] = "edited-thread".into();
    progress["selection"]["thread"] = "edited-thread".into();
    std::fs::write(&path, serde_json::to_vec(&progress).unwrap()).unwrap();
    let before = [
        f.count("messages"),
        f.count("operations"),
        f.count("invitations"),
    ];
    f.client.calls.lock().unwrap().clear();
    assert!(f.run(&["retry", &recovery], &mut vec![]).is_err());
    let calls = f.client.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Command::EnsureDirectThread(_)))
    );
    assert!(!calls.iter().any(|c| matches!(c, Command::SendMessage(_))));
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(
        [
            f.count("messages"),
            f.count("operations"),
            f.count("invitations")
        ],
        before
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='original'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(f.recovery(), recovery);
}

#[test]
fn changed_retry_namespace_refuses_before_any_client_call() {
    let f = Fixture::new(false);
    *f.client.lose.lock().unwrap() = Some("ensure");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "original"],
            &mut vec![]
        )
        .is_err()
    );
    let recovery = f.recovery();
    let state = f.root.to_str().unwrap();
    let parsed = parse_argv([
        "ht",
        "--state-dir",
        state,
        "--host-endpoint",
        "/different-host.sock",
        "retry",
        &recovery,
    ])
    .unwrap();
    f.client.calls.lock().unwrap().clear();
    assert!(
        run_cooperative(
            parsed,
            &f.journal,
            &f.contexts,
            None,
            Role::TopLevel,
            &f.client,
            &SystemClock::new(),
            &mut vec![]
        )
        .is_err()
    );
    assert!(f.client.calls.lock().unwrap().is_empty());
    assert_eq!(f.recovery(), recovery);
}

#[test]
fn deterministic_send_refusal_retains_staged_thread_body_and_retry() {
    let f = Fixture::new(false);
    *f.client.lose.lock().unwrap() = Some("before_send");
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "retained"],
            &mut vec![]
        )
        .is_err()
    );
    let recovery = f.recovery();
    let thread = f.journal.page(&Default::default()).unwrap().items[0]
        .thread
        .clone()
        .unwrap();
    f.db.execute("UPDATE threads SET archived=1", []).unwrap();
    let error = f.run(&["retry", &recovery], &mut vec![]).unwrap_err();
    let rendered = format!("{error:?}");
    assert!(rendered.contains(thread.as_str()));
    assert!(rendered.contains(&recovery));
    assert_eq!(f.recovery(), recovery);
    assert_eq!(f.count("direct_threads"), 1);
    assert_eq!(f.count("invitations"), 1);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE body='retained'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let original = f
        .journal
        .load(&f.journal.resolve_recovery_ref(&recovery).unwrap())
        .unwrap();
    let encoded = serde_json::to_string(&original.semantic).unwrap();
    assert!(encoded.contains("retained"));
}

#[test]
fn public_completed_retry_missing_endpoint_retains_files_and_one_frozen_hint() {
    let f = Fixture::new(false);
    assert!(
        f.run(
            &["send", "--seat", "seat-b", "--body", "hello"],
            &mut LostOutput
        )
        .is_err()
    );
    let recovery = f.recovery();
    let runtime = herdr_threads::daemon::paths::RuntimeContext::explicit(
        f.root.clone(),
        Some("/isolated-host.sock".into()),
        None,
    )
    .unwrap();
    let paths = herdr_threads::daemon::paths::NodePaths::resolve(&runtime.state_dir).unwrap();
    std::fs::create_dir_all(&paths.node_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&paths.node_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    herdr_threads::test_support::write_node_hint(
        &paths,
        herdr_threads::node::NodeUuid::from_u128(1),
    );
    let public_root = paths.node_dir.join("intents");
    let _public = Journal::open(&public_root).unwrap();
    for entry in std::fs::read_dir(f.root.join("intents")).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, public_root.join(path.file_name().unwrap())).unwrap();
    }
    let snapshot = || {
        let mut files: Vec<_> = std::fs::read_dir(&public_root)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(&path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let before_files = snapshot();
    let before_effects = [
        f.count("messages"),
        f.count("operations"),
        f.count("invitations"),
    ];
    assert!(!paths.descriptor_path.exists());
    let expected = vec![
        "herdr-threads".to_string(),
        "--state-dir".into(),
        f.root.to_string_lossy().into_owned(),
        "--host-endpoint".into(),
        "/isolated-host.sock".into(),
        "retry".into(),
        recovery.clone(),
    ];
    let command = expected
        .iter()
        .map(|v| shlex::try_quote(v).unwrap().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    for malformed in [false, true] {
        if malformed {
            std::fs::write(&paths.descriptor_path, b"{").unwrap();
            std::fs::set_permissions(
                &paths.descriptor_path,
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        for caller in [None, Some("seat-a")] {
            let mut output = vec![];
            let error = herdr_threads::cli::run_in_pane(
                [
                    "ht",
                    "--state-dir",
                    f.root.to_str().unwrap(),
                    "--host-endpoint",
                    "/isolated-host.sock",
                    "retry",
                    &recovery,
                ],
                caller,
                &mut output,
            )
            .unwrap_err();
            match error {
                RunError::Api(api) if !malformed => {
                    assert_eq!(
                        api.code,
                        herdr_threads::protocol::results::ErrorCode::HostUnavailable
                    );
                    assert_eq!(api.restart_argv.as_ref(), Some(&expected));
                    assert_eq!(
                        api.detail
                            .matches(&format!("recover with {command}"))
                            .count(),
                        1
                    );
                }
                RunError::Io(error) if malformed => {
                    assert_eq!(
                        error
                            .to_string()
                            .matches(&format!("recover with {command}"))
                            .count(),
                        1
                    );
                }
                error => panic!("unexpected public boundary error: {error:?}"),
            }
            assert!(output.is_empty());
            assert_eq!(snapshot(), before_files);
            assert_eq!(
                [
                    f.count("messages"),
                    f.count("operations"),
                    f.count("invitations")
                ],
                before_effects
            );
        }
    }
}

#[test]
fn old_direct_peer_with_lazy_support_refuses_creation_policy_before_any_effect() {
    let mut f = Fixture::new(false);
    f.client.omit_creation_policy = true;
    let result = f.run(
        &["send", "--seat", "seat-b", "--body", "hello"],
        &mut vec![],
    );
    assert!(
        matches!(result, Err(RunError::Api(ref e)) if e.code == herdr_threads::protocol::results::ErrorCode::Unsupported),
        "old direct peer unexpectedly accepted modern creation policy: {result:?}"
    );
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(f.count("direct_threads"), 0);
    assert_eq!(f.count("invitations"), 0);
    assert_eq!(f.count("messages"), 0);
    assert!(
        !f.client
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::EnsureDirectThread(_) | Command::SendMessage(_)))
    );
}

#[test]
fn modern_direct_send_creates_lazy_pending_invitation_without_invitation_wake() {
    let f = Fixture::new(false);
    f.run(
        &["send", "--seat", "seat-b", "--body", "hello"],
        &mut vec![],
    )
    .unwrap();
    let row: (Option<String>, String, Option<String>) =
        f.db.query_row(
            "SELECT confirmation,state,inviter_seat_id FROM invitations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (Some("lazy".into()), "pending".into(), Some("seat-a".into()))
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM wake_work WHERE seat_id='seat-b' AND reason_bits&1!=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

fn ensure_command(
    human: bool,
    policy: Option<herdr_threads::protocol::commands::EnsureCreationPolicy>,
    operation: &str,
) -> Command {
    use herdr_threads::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::EnsureDirectThread,
        ids::*,
    };
    Command::EnsureDirectThread(EnsureDirectThread {
        creation_policy: policy,
        recipient: SeatId::new("seat-b"),
        operation: OperationId::new(operation),
        claim: CallerClaim {
            host: isolated_host(),
            instance: herdr_threads::node::NodeUuid::from_u128(1).to_string(),
            seat: SeatId::new("seat-a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: if human {
                Harness::Human
            } else {
                Harness::Codex
            },
            native_session: NativeSessionId::new("session"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000002"),
            target: HostTargetId::new("seat-a"),
            agent_id: None,
            liveness: None,
        },
    })
}
fn initial_policy(timed: bool) -> herdr_threads::protocol::commands::EnsureCreationPolicy {
    use herdr_threads::protocol::commands::{EnsureCreationPolicy, InvitationConfirmation};
    EnsureCreationPolicy {
        confirmation: if timed {
            InvitationConfirmation::RequireAck
        } else {
            InvitationConfirmation::Lazy
        },
        deadline_millis: if timed { Some(11000) } else { None },
    }
}
fn ensure_call(f: &Fixture, command: Command) -> Result<CommandResult, ApiError> {
    f.client.call(
        command,
        &CallBudget {
            deadline: herdr_threads::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
    )
}
#[test]
fn human_initial_ensure_require_ack_refuses_before_pair_invitation_or_obligation_effects() {
    let f = Fixture::new(true);
    let obligations = f.count("seat_availability");
    let operations = f.count("operations");
    let result = ensure_call(
        &f,
        ensure_command(true, Some(initial_policy(true)), "human-timed"),
    );
    assert!(
        result
            .is_err_and(|e| e.code == herdr_threads::protocol::results::ErrorCode::InvalidRequest),
        "Human issuer gained a new timed invitation mode"
    );
    assert_eq!(f.count("direct_threads"), 0);
    assert_eq!(f.count("invitations"), 0);
    assert_eq!(f.count("seat_availability"), obligations);
    assert_eq!(f.count("operations"), operations);
}
#[test]
fn explicit_initial_ensure_policy_applies_once_and_existing_episode_is_immutable() {
    for timed in [false, true] {
        let f = Fixture::new(false);
        let first = ensure_call(
            &f,
            ensure_command(false, Some(initial_policy(timed)), "first-policy"),
        )
        .unwrap();
        let row: (Option<String>,i64,i64,i64,Option<String>)=f.db.query_row("SELECT confirmation,frozen_duration_ms,created_at,deadline_at,inviter_seat_id FROM invitations",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(
            row.0.as_deref(),
            Some(if timed { "require_ack" } else { "lazy" })
        );
        assert_eq!(row.4.as_deref(), Some("seat-a"));
        if timed {
            assert_eq!(row.1, 11000);
            assert_eq!(row.3 - row.2, 11000);
        }
        let second = ensure_call(
            &f,
            ensure_command(false, Some(initial_policy(!timed)), "second-policy"),
        )
        .unwrap();
        assert_eq!(ensured_thread(&first), ensured_thread(&second));
        let after: (Option<String>,i64,i64,i64,Option<String>)=f.db.query_row("SELECT confirmation,frozen_duration_ms,created_at,deadline_at,inviter_seat_id FROM invitations",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(row, after);
        assert_eq!(f.count("invitations"), 1);
    }
}
#[test]
fn human_lazy_and_historical_ensure_and_existing_requested_policy_keep_old_episode() {
    for policy in [None, Some(initial_policy(false))] {
        let f = Fixture::new(true);
        let first = ensure_call(&f, ensure_command(true, policy, "first-human")).unwrap();
        let second = ensure_call(
            &f,
            ensure_command(true, Some(initial_policy(true)), "reuse-human"),
        )
        .unwrap();
        assert_eq!(ensured_thread(&first), ensured_thread(&second));
        assert_eq!(f.count("invitations"), 1);
    }
}

#[test]
fn modern_ensure_returns_actual_canonical_policy_and_selection_outcome() {
    let f = Fixture::new(false);
    let first = ensure_call(
        &f,
        ensure_command(false, Some(initial_policy(true)), "actual-first"),
    )
    .unwrap();
    let first = serde_json::to_value(first).unwrap();
    assert_eq!(first["kind"], "direct_thread_ensured");
    assert_eq!(first["data"]["outcome"], "created");
    assert_eq!(first["data"]["invitation"]["confirmation"], "require_ack");
    let second = ensure_call(
        &f,
        ensure_command(false, Some(initial_policy(false)), "actual-reuse"),
    )
    .unwrap();
    let second = serde_json::to_value(second).unwrap();
    assert_eq!(second["kind"], "direct_thread_ensured");
    assert_eq!(second["data"]["outcome"], "existing_pending");
    assert_eq!(second["data"]["invitation"], first["data"]["invitation"]);
    assert_eq!(second["data"]["thread"], first["data"]["thread"]);
    let replay = ensure_call(
        &f,
        ensure_command(false, Some(initial_policy(false)), "actual-reuse"),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), second);
}

fn ensured_thread(result: &CommandResult) -> herdr_threads::protocol::ids::ThreadId {
    let value = serde_json::to_value(result).unwrap();
    herdr_threads::protocol::ids::ThreadId::new(if value["kind"] == "thread_created" {
        value["data"].as_str().unwrap()
    } else {
        value["data"]["thread"].as_str().unwrap()
    })
}
#[test]
fn modern_ensure_joined_rejected_and_left_use_actual_canonical_lifecycle() {
    use herdr_threads::protocol::{
        commands::{Accept, Leave, Reject},
        ids::*,
    };
    for state in ["joined", "rejected", "left"] {
        let f = Fixture::new(false);
        let first = ensure_call(
            &f,
            ensure_command(false, Some(initial_policy(false)), "initial-lifecycle"),
        )
        .unwrap();
        let thread = ensured_thread(&first);
        let Command::EnsureDirectThread(initial) = ensure_command(false, None, "claim-only") else {
            unreachable!()
        };
        let mut recipient = initial.claim;
        recipient.seat = SeatId::new("seat-b");
        recipient.target = HostTargetId::new("seat-b");
        if state == "rejected" {
            let invitation: String =
                f.db.query_row("SELECT id FROM invitations", [], |r| r.get(0))
                    .unwrap();
            ensure_call(
                &f,
                Command::Reject(Reject {
                    thread: thread.clone(),
                    invitation: InvitationId::new(invitation),
                    reason: "declined".into(),
                    operation: OperationId::new("decline"),
                    claim: recipient,
                }),
            )
            .unwrap();
        } else {
            ensure_call(
                &f,
                Command::Accept(Accept {
                    thread: thread.clone(),
                    operation: OperationId::new("accept"),
                    claim: recipient.clone(),
                }),
            )
            .unwrap();
            if state == "left" {
                ensure_call(
                    &f,
                    Command::Leave(Leave {
                        thread: thread.clone(),
                        operation: OperationId::new("leave"),
                        claim: recipient,
                    }),
                )
                .unwrap();
            }
        }
        let result = ensure_call(
            &f,
            ensure_command(false, Some(initial_policy(true)), "ensure-after-lifecycle"),
        );
        if state == "joined" {
            let actual = serde_json::to_value(result.unwrap()).unwrap();
            assert_eq!(actual["data"]["outcome"], "already_joined");
            assert_eq!(actual["data"]["invitation"], serde_json::Value::Null);
        } else {
            assert!(result.is_err(), "{state} participant was resurrected");
        }
        assert_eq!(f.count("direct_threads"), 1);
        assert_eq!(f.count("invitations"), 1);
    }
}

#[test]
fn historical_ensure_omits_new_result_and_modern_reuse_reports_old_timed_origin_honestly() {
    let f = Fixture::new(false);
    let historical = ensure_call(&f, ensure_command(false, None, "historical-initial")).unwrap();
    let historical = serde_json::to_value(&historical).unwrap();
    assert_eq!(historical["kind"], "thread_created");
    assert!(historical["data"].is_string());
    let row: (i64, Option<String>) =
        f.db.query_row(
            "SELECT deadline_at,inviter_seat_id FROM invitations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let modern = ensure_call(
        &f,
        ensure_command(false, Some(initial_policy(false)), "modern-old-pair"),
    )
    .unwrap();
    let modern = serde_json::to_value(modern).unwrap();
    assert_eq!(modern["kind"], "direct_thread_ensured");
    assert_eq!(modern["data"]["outcome"], "existing_pending");
    assert_eq!(modern["data"]["invitation"]["confirmation"], "require_ack");
    assert_eq!(modern["data"]["invitation"]["effective_deadline_at"], row.0);
    assert_eq!(
        modern["data"]["invitation"]["inviter"],
        serde_json::Value::Null
    );
    assert_eq!(row.1, None);
    let replay = ensure_call(&f, ensure_command(false, None, "historical-initial")).unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), historical);
}

#[test]
fn json_output_uses_the_versioned_send_envelope_with_the_thread() {
    let f = Fixture::new(false);
    let mut direct = vec![];
    f.run(
        &["send", "--seat", "seat-b", "--body", "hello", "--json"],
        &mut direct,
    )
    .unwrap();
    let direct: serde_json::Value = serde_json::from_slice(&direct).unwrap();
    let thread = direct["result"]["thread"].as_str().unwrap().to_owned();
    let mut ordinary = vec![];
    f.run(
        &["send", &thread, "--body", "again", "--json"],
        &mut ordinary,
    )
    .unwrap();
    let ordinary: serde_json::Value = serde_json::from_slice(&ordinary).unwrap();
    assert_eq!(direct["version"], ordinary["version"]);
    assert_eq!(direct["result"]["kind"], ordinary["result"]["kind"]);
    let message: String =
        f.db.query_row("SELECT id FROM messages WHERE body='hello'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(direct["result"]["data"], message.as_str());
    assert_eq!(direct["result"]["selection"]["outcome"], "created");
    assert_eq!(
        direct["result"]["selection"]["invitation"]["confirmation"],
        "lazy"
    );
    assert_eq!(
        serde_json::from_value::<CommandResult>(direct["result"].clone()).unwrap(),
        CommandResult::MessageSent(herdr_threads::protocol::ids::MessageId::new(message))
    );
}
