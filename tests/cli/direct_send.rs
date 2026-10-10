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
                capabilities: if self.supported {
                    herdr_threads::protocol::capabilities::ADVERTISED
                        .iter()
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
        let context = StoreContext::new(root.join("store.db"), clock.clone());
        let db = context.open_writer().unwrap();
        let instance = uuid::Uuid::from_u128(1);
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES(?1,0,'b',1)",
            [instance.to_string()],
        )
        .unwrap();
        for seat in ["seat-a", "seat-b"] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES(?1,?2,'resolved','native',?1,1,1,0)", rusqlite::params![seat, instance.to_string()]).unwrap();
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES(?1,?2,'b',1,1,0,'fresh','term-'||?2,'inc','coherent_enumeration',1)", rusqlite::params![instance.to_string(), seat]).unwrap();
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
        let contexts =
            ContextJournal::open(&directory, instance, "seat-a", Duration::from_secs(1)).unwrap();
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
        "UPDATE seats SET state='retired', retired_at=1, retired_seq=(SELECT decision_seq+1 FROM host_instances LIMIT 1), generation=generation+1 WHERE id='seat-b'",
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
                    "UPDATE seats SET state='retired', retired_at=1, retired_seq=(SELECT decision_seq+1 FROM host_instances LIMIT 1), generation=generation+1 WHERE id='seat-b'",
                    [],
                )
                .unwrap();
            }
            "unresolved" => {
                f.db.execute("UPDATE seats SET state='unresolved' WHERE id='seat-b'", [])
                    .unwrap();
            }
            "held" => {
                f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES(?1,'seat-b','b',1,'operator')", [uuid::Uuid::from_u128(1).to_string()]).unwrap();
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
    assert!(f.run(&["retry", &recovery], &mut vec![]).is_err());
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
