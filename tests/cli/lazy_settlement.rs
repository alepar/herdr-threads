use herdr_threads::{
    cli::{
        commands::parse_argv,
        journal::{Journal, SemanticMutation},
        run_cooperative,
    },
    harness::context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    ports::LocalClient,
    protocol::{
        commands::Command,
        ids::*,
        output::OutputSpec,
        pagination::{Consistency, Page, StopReason},
        results::{AckResult, ApiError, CommandResult, ErrorCode, InboxBatchV2Item},
        time::CallBudget,
    },
};
use std::{path::PathBuf, sync::Mutex};
struct Fixture {
    root: PathBuf,
    journal: Journal,
    contexts: ContextJournal,
}
impl Fixture {
    fn new(human: bool) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp/ht-big.5.2")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let contexts = ContextJournal::open(
            &root,
            uuid::Uuid::from_u128(1),
            "seat",
            std::time::Duration::from_millis(20),
        )
        .unwrap();
        let execution = uuid::Uuid::from_u128(2);
        contexts
            .install_reattached(OccupantContext {
                format_version: 1,
                instance: uuid::Uuid::from_u128(1),
                seat: "seat".into(),
                target: "pane".into(),
                harness: if human {
                    Harness::Human
                } else {
                    Harness::Codex
                },
                binding_generation: 1,
                execution,
                session: SessionReference::PluginContext(execution),
                role: Role::TopLevel,
            })
            .unwrap();
        let journal = Journal::open(root.join("intents")).unwrap();
        // Existing partial ordinary proof lets selective cleanup be observed;
        // legacy whole-body ordinary display itself does not persist a file.
        if !human {
            let claim =
                herdr_threads::harness::bridge::caller_claim(&contexts.current().unwrap().unwrap())
                    .unwrap();
            journal
                .record_displayed_chunk(&claim, &MessageId::new("ordinary"), 0, 2, 4)
                .unwrap();
        }
        Self {
            root,
            journal,
            contexts,
        }
    }
    fn run(&self, client: &Client, human: bool) -> Result<(), herdr_threads::cli::RunError> {
        let argv = if human {
            vec!["ht", "human", "inbox"]
        } else {
            vec!["ht", "inbox"]
        };
        run_cooperative(
            parse_argv(argv).unwrap(),
            &self.journal,
            &self.contexts,
            None,
            Role::TopLevel,
            client,
            &herdr_threads::app::SystemClock::new(),
            &mut Vec::new(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
struct Client {
    calls: Mutex<Vec<Command>>,
    fail_ack: bool,
    fail_lazy: bool,
    root: PathBuf,
    first_durable: Mutex<Vec<String>>,
}
impl Client {
    fn new(f: &Fixture, fail_ack: bool, fail_lazy: bool) -> Self {
        Self {
            calls: Mutex::new(vec![]),
            fail_ack,
            fail_lazy,
            root: f.root.join("intents"),
            first_durable: Mutex::new(vec![]),
        }
    }
}
fn page() -> CommandResult {
    CommandResult::InboxBatchV2(Page {
        items: vec![
            InboxBatchV2Item::LazyMessage {
                thread: ThreadId::new("thread"),
                message: MessageId::new("lazy"),
                sequence: 1,
                topic_data: "topic".into(),
                sender: Some(SeatId::new("sender")),
                author_role: None,
                relays_user: false,
                user_intent: None,
                author_role_backfilled: false,
                body: "body".into(),
                body_start: 0,
                body_end: 4,
                body_len: 4,
            },
            InboxBatchV2Item::Message {
                thread: ThreadId::new("thread"),
                message: MessageId::new("ordinary"),
                sequence: 2,
                topic_data: "topic".into(),
                sender: Some(SeatId::new("sender")),
                author_role: None,
                relays_user: false,
                user_intent: None,
                author_role_backfilled: false,
                body: "body".into(),
                body_start: 0,
                body_end: 4,
                body_len: 4,
                ack_candidate: Some(MessageId::new("ordinary")),
            },
        ],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 2,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    })
}
impl LocalClient for Client {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        let mut calls = self.calls.lock().unwrap();
        if matches!(
            c,
            Command::AckDisplayed(_) | Command::CompleteInboxDelivery(_)
        ) && !calls.iter().any(|c| {
            matches!(
                c,
                Command::AckDisplayed(_) | Command::CompleteInboxDelivery(_)
            )
        }) {
            *self.first_durable.lock().unwrap() = std::fs::read_dir(&self.root)
                .unwrap()
                .filter_map(|e| {
                    let p = e.unwrap().path();
                    if p.extension().is_some_and(|e| e == "intent") {
                        Some(std::fs::read_to_string(p).unwrap())
                    } else {
                        None
                    }
                })
                .collect();
        }
        calls.push(c.clone());
        drop(calls);
        match c {
            Command::Capabilities => Ok(CommandResult::Capabilities(
                herdr_threads::protocol::results::CapabilityList {
                    capabilities: vec![
                        herdr_threads::protocol::capabilities::INBOX_BATCH_V2.into(),
                    ],
                },
            )),
            Command::InboxBatchV2(_) => Ok(page()),
            Command::AckDisplayed(a) => {
                assert_eq!(a.messages, vec![MessageId::new("ordinary")]);
                if self.fail_ack {
                    Err(ApiError::new(ErrorCode::UnknownOutcome, "ack lost"))
                } else {
                    Ok(CommandResult::Acknowledged(AckResult {
                        acknowledged: a.messages,
                        already_acknowledged: vec![],
                    }))
                }
            }
            Command::CompleteInboxDelivery(a) => {
                assert_eq!(a.messages, vec![MessageId::new("lazy")]);
                if self.fail_lazy {
                    Err(ApiError::new(ErrorCode::UnknownOutcome, "completion lost"))
                } else {
                    Ok(CommandResult::InboxDeliveryCompleted(a.messages))
                }
            }
            _ => panic!("unexpected {c:?}"),
        }
    }
}
// Kills: omitting completion submission after fully flushed lazy body.
#[test]
fn lazy_mixed_intents_both_durable_before_submit() {
    let f = Fixture::new(false);
    let c = Client::new(&f, false, false);
    f.run(&c, false).unwrap();
    assert_eq!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, Command::CompleteInboxDelivery(_)))
            .count(),
        1
    );
    let durable = c.first_durable.lock().unwrap();
    assert_eq!(durable.len(), 2);
    assert!(
        durable
            .iter()
            .any(|s| s.contains("complete_inbox_delivery"))
    );
    assert!(durable.iter().any(|s| s.contains("ack_displayed")));
}
// Kills: missing exact semantic tag / failing to rehydrate frozen completion.
#[test]
fn lazy_completion_semantic_deserializes() {
    let s: SemanticMutation =
        serde_json::from_str(r#"{"kind":"complete_inbox_delivery","messages":["lazy"]}"#).unwrap();
    assert_eq!(
        serde_json::to_value(s).unwrap()["kind"],
        "complete_inbox_delivery"
    );
}

fn intent_files(f: &Fixture) -> Vec<PathBuf> {
    std::fs::read_dir(f.root.join("intents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "intent"))
        .collect()
}
fn progress_files(f: &Fixture, prefix: &str) -> Vec<PathBuf> {
    std::fs::read_dir(f.root.join("intents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name().unwrap().to_string_lossy().starts_with(prefix)
                && p.extension().is_some_and(|e| e == "progress")
        })
        .collect()
}
// Kills: fail-fast peer settlement, or clearing failed proofs.
#[test]
fn lazy_mixed_ack_failure_still_completes_lazy() {
    let f = Fixture::new(false);
    let c = Client::new(&f, true, false);
    let error = f.run(&c, false).unwrap_err().to_string();
    assert!(error.contains("local:1"));
    assert!(!error.contains("local:2"));
    assert_eq!(intent_files(&f).len(), 1);
    assert!(progress_files(&f, "display-lazy-").is_empty());
    assert_eq!(progress_files(&f, "display-").len(), 1);
    assert!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::CompleteInboxDelivery(_)))
    );
}
#[test]
fn lazy_mixed_completion_failure_still_acks_ordinary() {
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    let error = f.run(&c, false).unwrap_err().to_string();
    assert!(error.contains("local:2"));
    assert!(!error.contains("local:1"));
    assert_eq!(intent_files(&f).len(), 1);
    assert_eq!(progress_files(&f, "display-lazy-").len(), 1);
    assert_eq!(progress_files(&f, "display-").len(), 1);
}
#[test]
fn lazy_mixed_both_fail_exact_retry_refs() {
    let f = Fixture::new(false);
    let c = Client::new(&f, true, true);
    let error = f.run(&c, false).unwrap_err().to_string();
    assert!(error.contains("local:1"));
    assert!(error.contains("local:2"));
    assert_eq!(intent_files(&f).len(), 2);
    assert_eq!(progress_files(&f, "display-").len(), 2);
}
// Real allocator boundary exhausts only the second write: no failpoint/race.
#[test]
fn lazy_partial_record_failure_reports_existing_ref() {
    let f = Fixture::new(false);
    std::fs::write(
        f.root.join("intents/next-ordinal"),
        format!("{}\n", u64::MAX - 1),
    )
    .unwrap();
    let c = Client::new(&f, false, false);
    let error = f.run(&c, false).unwrap_err().to_string();
    assert!(error.contains("local:18446744073709551615"));
    assert_eq!(intent_files(&f).len(), 1);
    assert_eq!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(
                c,
                Command::AckDisplayed(_) | Command::CompleteInboxDelivery(_)
            ))
            .count(),
        0
    );
    assert_eq!(progress_files(&f, "display-").len(), 2);
}
#[test]
fn lazy_completion_lost_reply_retry_and_success_cleanup() {
    use herdr_threads::cli::retry;
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    f.run(&c, false).unwrap_err();
    let reference = f.journal.resolve_recovery_ref("local:2").unwrap();
    let pending = f.journal.load(&reference).unwrap();
    let original = c
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|c| matches!(c, Command::CompleteInboxDelivery(_)))
        .unwrap()
        .clone();
    let claim = pending.semantic.frozen_claim().unwrap().clone();
    f.journal
        .record_lazy_displayed_chunk(&claim, &MessageId::new("unrelated"), 0, 4, 4)
        .unwrap();
    let mut submitted = vec![];
    // Two transport-uncertain replays must preserve the entire command.
    for _ in 0..2 {
        assert!(
            retry::run_retry_api_to_writer(
                &f.journal,
                &reference,
                &pending.header.scope,
                || panic!("must not replace frozen claim"),
                |command| {
                    submitted.push(command);
                    Err(ApiError::new(
                        ErrorCode::UnknownOutcome,
                        "reply lost after commit",
                    ))
                },
                &OutputSpec::default(),
                &mut Vec::new()
            )
            .is_err()
        );
    }
    assert_eq!(submitted, vec![original.clone(), original.clone()]);
    assert!(f.journal.load(&reference).is_ok());
    retry::run_retry_api_to_writer(
        &f.journal,
        &reference,
        &pending.header.scope,
        || panic!("must not replace frozen claim"),
        |command| {
            assert_eq!(command, original);
            Ok(CommandResult::InboxDeliveryCompleted(vec![MessageId::new(
                "lazy",
            )]))
        },
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(f.journal.load(&reference).is_err());
    assert_eq!(progress_files(&f, "display-lazy-").len(), 1);
    assert!(
        !f.journal
            .record_lazy_displayed_chunk(&claim, &MessageId::new("lazy"), 2, 4, 4)
            .unwrap()
    ); // cleared exact successful chain
}
#[test]
fn lazy_cleanup_failure_retains_exact_ref() {
    use herdr_threads::cli::retry;
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    f.run(&c, false).unwrap_err();
    let reference = f.journal.resolve_recovery_ref("local:2").unwrap();
    let pending = f.journal.load(&reference).unwrap();
    let path = progress_files(&f, "display-lazy-").pop().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        retry::run_retry_api_to_writer(
            &f.journal,
            &reference,
            &pending.header.scope,
            || panic!(),
            |_| Ok(CommandResult::InboxDeliveryCompleted(vec![MessageId::new(
                "lazy"
            )])),
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(f.journal.load(&reference).is_ok());
    std::fs::remove_dir(&path).unwrap();
    retry::run_retry_api_to_writer(
        &f.journal,
        &reference,
        &pending.header.scope,
        || panic!(),
        |_| {
            Ok(CommandResult::InboxDeliveryCompleted(vec![MessageId::new(
                "lazy",
            )]))
        },
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(f.journal.load(&reference).is_err());
}
#[test]
fn lazy_human_inbox_completes_without_ordinary_ack() {
    let f = Fixture::new(true);
    let c = Client::new(&f, false, false);
    f.run(&c, true).unwrap();
    let calls = c.calls.lock().unwrap();
    assert!(
        !calls
            .iter()
            .any(|c| matches!(c, Command::Ack(_) | Command::AckDisplayed(_)))
    );
    let Command::CompleteInboxDelivery(request) = calls
        .iter()
        .find(|c| matches!(c, Command::CompleteInboxDelivery(_)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        request.claim.harness,
        herdr_threads::protocol::authority::Harness::Human
    );
    assert_eq!(request.claim.seat, SeatId::new("seat"));
}
#[test]
fn lazy_frozen_actor_completed_retry_preflight() {
    use herdr_threads::{
        cli::{
            actor_route::InvocationActor,
            journal::{IntentScope, OriginalActor},
            retry,
        },
        daemon::paths::{InstancePaths, RuntimeContext},
    };
    let f = Fixture::new(true);
    let state = f.root.join("state");
    let host = f.root.join("host.sock");
    let runtime = RuntimeContext::explicit(state.clone(), host.clone(), None).unwrap();
    let paths = InstancePaths::resolve_read_only(&runtime).unwrap();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let claim =
        herdr_threads::harness::bridge::caller_claim(&f.contexts.current().unwrap().unwrap())
            .unwrap();
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let frozen = SemanticMutation::freeze(
        SemanticMutation::CompleteInboxDelivery {
            messages: vec![MessageId::new("lazy")],
        },
        claim,
    )
    .unwrap();
    let reference = journal.record(scope, frozen, 1).unwrap();
    let entry = std::fs::read_dir(paths.instance_dir.join("intents"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "intent"))
        .unwrap();
    let before = std::fs::read(&entry).unwrap();
    std::fs::remove_file(paths.instance_dir.join("intents/allocator.lock")).unwrap();
    let mut output = vec![];
    let error = herdr_threads::cli::run_in_pane(
        vec![
            "ht".into(),
            "--state-dir".into(),
            state.to_string_lossy().into_owned(),
            "--host-endpoint".into(),
            host.to_string_lossy().into_owned(),
            "--human".into(),
            "retry".into(),
            reference.recovery_ref(),
        ],
        None,
        &mut output,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("person/operator retry requires immediate human namespace")
    );
    assert!(output.is_empty());
    assert_eq!(std::fs::read(entry).unwrap(), before);
    assert!(!paths.instance_dir.join("intents/allocator.lock").exists());
    assert_eq!(
        retry::preflight_original_actor(
            paths.instance_dir.join("intents"),
            &reference.recovery_ref(),
            InvocationActor::Human,
            &Default::default()
        )
        .unwrap(),
        OriginalActor::HumanOrOperator
    );
}

// Kills: inferring retry's actor from a later binding instead of frozen intent.
#[test]
fn lazy_agent_retry_uses_saved_actor_after_binding_change() {
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    f.run(&c, false).unwrap_err();
    let original = c
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|c| matches!(c, Command::CompleteInboxDelivery(_)))
        .unwrap()
        .clone();
    let mut current = f.contexts.current().unwrap().unwrap();
    current.harness = Harness::Human;
    current.binding_generation = 2;
    f.contexts.install_reattached(current).unwrap();
    let c = Client::new(&f, false, false);
    run_cooperative(
        parse_argv(["ht", "retry", "local:2"]).unwrap(),
        &f.journal,
        &f.contexts,
        None,
        Role::TopLevel,
        &c,
        &herdr_threads::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(*c.calls.lock().unwrap(), vec![original]);
}
#[test]
fn lazy_direct_human_retry_root_preflight_zero_calls() {
    let f = Fixture::new(true);
    let c = Client::new(&f, false, true);
    f.run(&c, true).unwrap_err();
    let before = std::fs::read(intent_files(&f).pop().unwrap()).unwrap();
    let c = Client::new(&f, false, false);
    let error = run_cooperative(
        parse_argv(["ht", "retry", "local:1"]).unwrap(),
        &f.journal,
        &f.contexts,
        None,
        Role::TopLevel,
        &c,
        &herdr_threads::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("person/operator retry requires immediate human namespace")
    );
    assert!(c.calls.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read(intent_files(&f).pop().unwrap()).unwrap(),
        before
    );
    run_cooperative(
        parse_argv(["ht", "human", "retry", "local:1"]).unwrap(),
        &f.journal,
        &f.contexts,
        None,
        Role::TopLevel,
        &c,
        &herdr_threads::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(!c.calls.lock().unwrap().is_empty());
}

// Kills: accepting a foreign/partial completion reply and deleting its intent.
#[test]
fn lazy_completion_wrong_ids_keeps_intent_and_proof() {
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    f.run(&c, false).unwrap_err();
    let r = f.journal.resolve_recovery_ref("local:2").unwrap();
    let p = f.journal.load(&r).unwrap();
    assert!(
        herdr_threads::cli::retry::run_retry_api_to_writer(
            &f.journal,
            &r,
            &p.header.scope,
            || panic!(),
            |_| Ok(CommandResult::InboxDeliveryCompleted(vec![MessageId::new(
                "foreign"
            )])),
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(f.journal.load(&r).is_ok());
    assert_eq!(progress_files(&f, "display-lazy-").len(), 1);
}

#[test]
fn lazy_human_failure_has_lawful_recovery_command() {
    let f = Fixture::new(true);
    let c = Client::new(&f, false, true);
    let error = f.run(&c, true).unwrap_err().to_string();
    assert!(
        error.contains("herdr-threads human retry local:1"),
        "{error}"
    );
}

#[test]
fn lazy_completion_listing_and_batch_bounds() {
    use herdr_threads::cli::journal::IntentScope;
    use herdr_threads::protocol::{pagination::PageRequest, results::IntentKind};
    let f = Fixture::new(false);
    let claim =
        herdr_threads::harness::bridge::caller_claim(&f.contexts.current().unwrap().unwrap())
            .unwrap();
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    for messages in [vec![], vec![MessageId::new("lazy"); 101]] {
        assert!(
            SemanticMutation::freeze(
                SemanticMutation::CompleteInboxDelivery { messages },
                claim.clone()
            )
            .is_err()
        );
    }
    assert!(intent_files(&f).is_empty());
    let frozen = SemanticMutation::freeze(
        SemanticMutation::CompleteInboxDelivery {
            messages: vec![MessageId::new("lazy")],
        },
        claim,
    )
    .unwrap();
    let r = f.journal.record(scope, frozen, 1).unwrap();
    let p = f
        .journal
        .page(&PageRequest {
            cursor: None,
            limit: 10,
            max_bytes: 4096,
        })
        .unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.items[0].kind, IntentKind::CompleteInboxDelivery);
    assert_eq!(p.items[0].recovery_ref.as_str(), r.recovery_ref());
    assert_eq!(
        serde_json::to_value(&p.items[0]).unwrap()["kind"],
        "complete_inbox_delivery"
    );
}

#[test]
fn lazy_mixed_ack_wrong_ids_keeps_only_failed_intent_and_proof() {
    let f = Fixture::new(false);
    let c = Client::new(&f, true, false);
    f.run(&c, false).unwrap_err();
    let r = f.journal.resolve_recovery_ref("local:1").unwrap();
    let p = f.journal.load(&r).unwrap();
    assert!(
        herdr_threads::cli::retry::run_retry_api_to_writer(
            &f.journal,
            &r,
            &p.header.scope,
            || panic!(),
            |_| Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![MessageId::new("foreign")],
                already_acknowledged: vec![]
            })),
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(f.journal.load(&r).is_ok());
    assert_eq!(progress_files(&f, "display-").len(), 1);
    assert!(progress_files(&f, "display-lazy-").is_empty());
    herdr_threads::cli::retry::run_retry_api_to_writer(
        &f.journal,
        &r,
        &p.header.scope,
        || panic!(),
        |_| {
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![],
                already_acknowledged: vec![MessageId::new("ordinary")],
            }))
        },
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(intent_files(&f).is_empty());
    assert!(progress_files(&f, "display-").is_empty());
}
#[test]
fn lazy_completion_wrong_result_kind_keeps_intent() {
    let f = Fixture::new(false);
    let c = Client::new(&f, false, true);
    f.run(&c, false).unwrap_err();
    let r = f.journal.resolve_recovery_ref("local:2").unwrap();
    let p = f.journal.load(&r).unwrap();
    assert!(
        herdr_threads::cli::retry::run_retry_api_to_writer(
            &f.journal,
            &r,
            &p.header.scope,
            || panic!(),
            |_| Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![MessageId::new("lazy")],
                already_acknowledged: vec![]
            })),
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(f.journal.load(&r).is_ok());
    assert_eq!(progress_files(&f, "display-lazy-").len(), 1);
}

#[test]
fn lazy_completion_cannot_refresh_claim_from_native_scope() {
    use herdr_threads::cli::journal::IntentScope;
    let f = Fixture::new(false);
    assert!(
        f.journal
            .record(
                IntentScope::Native {
                    instance: "instance".into(),
                    seat: SeatId::new("seat")
                },
                SemanticMutation::CompleteInboxDelivery {
                    messages: vec![MessageId::new("lazy")]
                },
                1
            )
            .is_err()
    );
    assert!(intent_files(&f).is_empty());
}
