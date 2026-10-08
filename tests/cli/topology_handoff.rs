use super::*;
use crate::protocol::handoff::topology_contract_tests::claim;
use crate::{
    host::observation::{HostTopology, TopologyPane, TopologySpace, TopologyTab},
    ports::LocalClient,
    protocol::{
        commands::Command,
        ids::{HostTargetId, ThreadId},
        results::{ApiError, CommandResult},
        time::CallBudget,
    },
};
struct ReadOnly;
impl LocalClient for ReadOnly {
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        match command {
            Command::ResolveThread(q)
                if q.selector == "chosen"
                    && q.caller.as_ref().map(|s| s.as_str()) == Some("sender")
                    && q.caller_target.as_ref().map(|s| s.as_str()) == Some("w1:p1") =>
            {
                Ok(CommandResult::ThreadResolved(ThreadId::new(
                    "canonical-thread",
                )))
            }
            Command::ResolveThread(_) => Err(ApiError::invalid_request("ambiguous thread")),
            _ => panic!("preparation must never mutate: {command:?}"),
        }
    }
}
fn request() -> Request {
    Request {
        new_tab: Some("same label".into()),
        existing: false,
        selector: PaneSelector::default(),
        seat: None,
        cwd: None,
        thread: Some("chosen".into()),
        thread_name: None,
        topic: None,
        goal: None,
        body: "literal '$HOME' body".into(),
        kind: Some("codex".into()),
        binary: Some("/bin/quoted path".into()),
        name: Some("peer".into()),
        argv: vec!["--model".into(), "saved value".into()],
    }
}
fn topology() -> HostTopology {
    HostTopology {
        spaces: vec![
            TopologySpace {
                id: "w1".into(),
                label: Some("work".into()),
            },
            TopologySpace {
                id: "w2".into(),
                label: Some("other".into()),
            },
        ],
        tabs: vec![TopologyTab {
            id: "w1:t1".into(),
            space: "w1".into(),
            label: Some("same label".into()),
        }],
        panes: vec![TopologyPane {
            target: HostTargetId::new("w1:p1"),
            space: "w1".into(),
            tab: "w1:t1".into(),
            label: None,
            agent_names: vec![],
        }],
    }
}
fn prepared(
    r: &Request,
    root: &std::path::Path,
) -> Result<crate::protocol::handoff::BootstrapIdentity, RunError> {
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "i".into(),
        state_dir: root.join("state dir"),
        host_endpoint: root.join("host.sock"),
    };
    prepare(
        r,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: root,
            options: Some("--config 'quoted $HOME' $(literal)".into()),
            client: &ReadOnly,
            clock: &crate::app::SystemClock::new(),
        },
    )
}
#[test]
fn publication_freezes_route_argv_caller_and_channel() {
    let temp = Temp::new();
    let identity = prepared(&request(), temp.path()).expect("preparation must succeed");
    assert_eq!(identity.payload.workspace.as_str(), "w1");
    assert_eq!(identity.payload.cwd, temp.path().canonicalize().unwrap());
    assert_eq!(
        identity.payload.launch.argv,
        vec![
            "--config",
            "quoted $HOME",
            "$(literal)",
            "--model",
            "saved value"
        ]
    );
    assert_eq!(
        identity.payload.launch.binary.as_deref(),
        Some("/bin/quoted path")
    );
    assert_eq!(
        identity.payload.handoff.channel.thread().unwrap().as_str(),
        "canonical-thread"
    );
    assert_eq!(identity.payload.handoff.body, "literal '$HOME' body");
    assert!(!identity.payload.focus);
    assert!(identity.payload.env.is_empty());
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let reference = publish(&journal, &identity, 1).expect("publication must succeed");
    let loaded = journal.load(&reference).unwrap();
    assert_eq!(loaded.header.scope, identity.scope);
    assert_eq!(loaded.header.semantic_digest, identity.digest);
    assert_eq!(loaded.semantic.frozen_claim(), Some(&identity.claim));
    assert_ne!(reference.operation, identity.compound);
    let super::super::journal::SemanticMutation::Frozen { mutation, .. } = loaded.semantic else {
        panic!("missing frozen semantic")
    };
    let super::super::journal::SemanticMutation::HandoffBootstrap(plan) = *mutation else {
        panic!("missing bootstrap")
    };
    assert_eq!(plan.payload, identity.payload);
    // Names and options can change after publication; reread is only saved bytes.
    let changed = prepared(
        &Request {
            thread: Some("now ambiguous".into()),
            ..request()
        },
        temp.path(),
    );
    assert!(changed.is_err());
    assert_eq!(
        journal.load(&reference).unwrap().header.semantic_digest,
        identity.digest
    );
}
#[test]
fn publication_validation_has_zero_journal_effects() {
    let temp = Temp::new();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    for cwd in [
        temp.path().join("absent"),
        std::path::PathBuf::from("relative"),
    ] {
        assert!(
            prepared(
                &Request {
                    cwd: Some(cwd),
                    ..request()
                },
                temp.path()
            )
            .is_err()
        );
    }
    assert!(
        !std::fs::read_dir(journal.root()).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|s| s == "intent"))
    );
}

struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ht-qhz4-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn publication_rejects_tampered_identity_before_allocation() {
    let temp = Temp::new();
    let identity = prepared(&request(), temp.path()).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    for mutate in [
        |i: &mut crate::protocol::handoff::BootstrapIdentity| i.payload.label = "changed".into(),
        |i: &mut crate::protocol::handoff::BootstrapIdentity| {
            i.payload.launch.argv.push("changed".into())
        },
        |i: &mut crate::protocol::handoff::BootstrapIdentity| {
            i.claim.execution = crate::protocol::ids::ExecutionId::new("changed")
        },
        |i: &mut crate::protocol::handoff::BootstrapIdentity| {
            i.payload.handoff.namespace.host_endpoint = "/changed.sock".into()
        },
        |i: &mut crate::protocol::handoff::BootstrapIdentity| {
            i.scope = super::super::journal::IntentScope::Cooperative {
                instance: "foreign".into(),
                seat: i.claim.seat.clone(),
            }
        },
    ] {
        let mut tampered = identity.clone();
        mutate(&mut tampered);
        assert!(publish(&journal, &tampered, 1).is_err());
    }
    assert!(journal.resolve_recovery_ref("local:1").is_err());
    let reference = publish(&journal, &identity, 1).unwrap();
    let intent = std::fs::read_dir(journal.root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "intent"))
        .unwrap();
    let bytes = std::fs::read_to_string(&intent).unwrap();
    assert!(bytes.contains("same label"));
    std::fs::write(intent, bytes.replace("same label", "evil label")).unwrap();
    assert!(
        journal.load(&reference).is_err(),
        "changed bytes must not pass original digest"
    );
}
#[test]
fn publication_selected_workspace_and_new_channel_defaults() {
    let temp = Temp::new();
    let mut r = request();
    r.thread = None;
    r.selector.space = Some("other".into());
    let identity = prepared(&r, temp.path()).unwrap();
    assert_eq!(identity.payload.workspace.as_str(), "w2");
    assert_eq!(
        identity.payload.handoff.channel,
        crate::protocol::handoff::HandoffChannel::New {
            name: None,
            topic: "Handoff to same label".into(),
            goal: "Handoff to same label".into()
        }
    );
    let namespace = identity.payload.handoff.namespace.clone();
    let mut host = topology();
    host.spaces.push(TopologySpace {
        id: "w3".into(),
        label: Some("other".into()),
    });
    assert!(
        prepare(
            &r,
            Preparation {
                claim: &claim(),
                namespace: &namespace,
                topology: &host,
                invocation_cwd: temp.path(),
                options: None,
                client: &ReadOnly,
                clock: &crate::app::SystemClock::new()
            }
        )
        .is_err()
    );
    r.selector.space = Some("w2".into());
    assert_eq!(
        prepare(
            &r,
            Preparation {
                claim: &claim(),
                namespace: &namespace,
                topology: &host,
                invocation_cwd: temp.path(),
                options: None,
                client: &ReadOnly,
                clock: &crate::app::SystemClock::new()
            }
        )
        .unwrap()
        .payload
        .workspace
        .as_str(),
        "w2"
    );
    host.panes.clear();
    r.selector.space = None;
    assert!(
        prepare(
            &r,
            Preparation {
                claim: &claim(),
                namespace: &namespace,
                topology: &host,
                invocation_cwd: temp.path(),
                options: None,
                client: &ReadOnly,
                clock: &crate::app::SystemClock::new()
            }
        )
        .is_err()
    );
}
#[test]
fn publication_public_route_is_fenced_before_namespace_access() {
    let temp = Temp::new();
    let state = temp.path().join("must not exist");
    let argv = vec![
        "herdr-threads".into(),
        "--state-dir".into(),
        state.display().to_string(),
        "--host-endpoint".into(),
        temp.path().join("absent.sock").display().to_string(),
        "handoff".into(),
        "--new-tab".into(),
        "peer".into(),
        "--new-thread".into(),
        "--kind".into(),
        "codex".into(),
        "--".into(),
        "work".into(),
    ];
    let mut output = vec![];
    let result = super::super::run_in_pane(argv, None, &mut output);
    assert!(matches!(
        result,
        Err(RunError::Api(ApiError {
            code: crate::protocol::results::ErrorCode::Unsupported,
            ..
        }))
    ));
    assert!(!state.exists());
    assert!(output.is_empty());
}

#[test]
fn publication_explicit_cwd_claude_options_and_invalid_inputs() {
    let temp = Temp::new();
    let work = temp.path().join("quoted worktree");
    std::fs::create_dir(&work).unwrap();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "i".into(),
        state_dir: temp.path().join("state"),
        host_endpoint: temp.path().join("host.sock"),
    };
    let r = Request {
        cwd: Some(work.clone()),
        kind: Some("claude".into()),
        name: Some("Peer Name".into()),
        ..request()
    };
    let identity = prepare(
        &r,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: temp.path(),
            options: Some("--model 'claude option'".into()),
            client: &ReadOnly,
            clock: &crate::app::SystemClock::new(),
        },
    )
    .unwrap();
    assert_eq!(identity.payload.cwd, work.canonicalize().unwrap());
    assert_eq!(
        identity.payload.launch.harness,
        crate::protocol::authority::Harness::Claude
    );
    assert_eq!(identity.payload.launch.name.as_deref(), Some("peer-name"));
    assert_eq!(
        identity.payload.launch.argv,
        vec!["--model", "claude option", "--model", "saved value"]
    );
    assert!(
        prepare(
            &r,
            Preparation {
                claim: &claim(),
                namespace: &namespace,
                topology: &topology(),
                invocation_cwd: temp.path(),
                options: Some("unterminated '".into()),
                client: &ReadOnly,
                clock: &crate::app::SystemClock::new()
            }
        )
        .is_err()
    );
    let file = temp.path().join("file");
    std::fs::write(&file, "file").unwrap();
    for r in [
        Request {
            cwd: Some(file),
            ..request()
        },
        Request {
            body: " ".into(),
            ..request()
        },
        Request {
            new_tab: Some(" ".into()),
            ..request()
        },
        Request {
            kind: None,
            ..request()
        },
        Request {
            kind: Some("human".into()),
            ..request()
        },
        Request {
            topic: Some("conflict".into()),
            ..request()
        },
    ] {
        assert!(prepared(&r, temp.path()).is_err());
    }
}

#[test]
fn publication_refuses_cwd_removed_after_preparation() {
    let temp = Temp::new();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let identity = prepared(
        &Request {
            cwd: Some(work.clone()),
            ..request()
        },
        temp.path(),
    )
    .unwrap();
    std::fs::remove_dir(&work).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    assert!(
        publish(&journal, &identity, 1).is_err(),
        "missing cwd must refuse before publication"
    );
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}

// Real canonical attempt writers and a private on-disk journal; native outcomes
// are deterministic typed boundary doubles, without a model or Herdr process.
mod live {
    use super::*;
    use crate::{
        ports::{CreateTabOutcome, CreateTabPort, CreateTabRequest, HostCallContext},
        protocol::{handoff::*, ids::*, results::ErrorCode, time::UtcMillis},
        store::{schema, topology_handoff as canonical},
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        ReserveReply,
        HostReply,
        NotSubmitted,
        NotSubmittedRecordUnavailable,
        Check,
        RecordReply,
        SavePossible,
        SaveCreated,
        MissingCapability,
        OldCapability,
        WrongCapability,
        BadCorrelation,
        ResolveUnsupported,
        AttachReply,
        MovedResolveTab,
        MovedAttachTab,
        CompleteBefore,
        CompleteReply,
        StatusUnavailable,
        TerminalSave,
    }
    struct State {
        db: rusqlite::Connection,
        calls: Vec<&'static str>,
        native_calls: usize,
        fault: Fault,
    }
    struct Peer {
        state: Mutex<State>,
        identity: BootstrapIdentity,
        journal: std::path::PathBuf,
        database_path: std::path::PathBuf,
        reference: super::super::super::journal::IntentRef,
        native_marker: Option<std::path::PathBuf>,
        block_native: bool,
    }
    impl LocalClient for Peer {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
        fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
            let mut s = self.state.lock().unwrap();
            let role = match &command {
                Command::Capabilities => "capabilities",
                Command::BeginBootstrap(_) => "begin",
                Command::BootstrapStatus(_) => "status",
                Command::ReserveBootstrapAttempt(_) => "reserve",
                Command::CheckBootstrapSubmission(_) => "check",
                Command::RecordBootstrapCreated(_) => "record",
                Command::RecordBootstrapNotSubmitted(_) => "not_submitted",
                Command::ResolveBootstrapSeat(_) => "resolve",
                Command::AttachBootstrapHandoff(_) => "attach",
                Command::BeginHandoff(_) => "child_begin",
                Command::CompleteLinkedBootstrap(_) => "linked_complete",
                Command::CreateThread(_) => "create_child",
                Command::Invite(_) => "invite_child",
                Command::SendMessage(_) => "send_child",
                _ => panic!("unapproved live route {command:?}"),
            };
            s.calls.push(role);
            if role == "status" && s.fault == Fault::StatusUnavailable {
                return Err(ApiError::host_unavailable("status unavailable"));
            }
            if matches!(role, "create_child" | "invite_child" | "send_child") {
                drop(s);
                return self.durable_child(command, budget);
            }
            if role == "linked_complete" && s.fault == Fault::CompleteBefore {
                s.fault = Fault::None;
                return Err(ApiError::host_unavailable(
                    "crash before wrapper deciding transaction",
                ));
            }
            if role == "capabilities" {
                if s.fault == Fault::OldCapability {
                    return Err(ApiError::unsupported("older daemon"));
                }
                if s.fault == Fault::WrongCapability {
                    return Ok(CommandResult::SeatResolved(SeatId::new("wrong-reply")));
                }
                return Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: if s.fault == Fault::MissingCapability {
                            vec![]
                        } else {
                            vec![
                                crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1
                                    .into(),
                            ]
                        },
                    },
                ));
            }
            if role == "not_submitted" && s.fault == Fault::NotSubmittedRecordUnavailable {
                s.fault = Fault::None;
                return Err(ApiError::host_unavailable(
                    "no-effect record not submitted to writer",
                ));
            }
            if role == "resolve" || role == "attach" {
                let fault = s.fault;
                drop(s);
                if fault == Fault::ResolveUnsupported {
                    return Err(ApiError::unsupported(
                        "injected unavailable guarded handler",
                    ));
                }
                let tab = if (role == "resolve" && fault == Fault::MovedResolveTab)
                    || (role == "attach" && fault == Fault::MovedAttachTab)
                {
                    "w1:t9"
                } else {
                    "w1:t2"
                };
                let (ctx, guard, budget) = scoped_peer_guard(self, tab);
                let mut s = self.state.lock().unwrap();
                let ns = &self.identity.payload.handoff.namespace;
                match command {
                    Command::ResolveBootstrapSeat(r) => {
                        assert_eq!(r.identity, self.identity);
                        assert_eq!(
                            r.expected_attempt,
                            canonical::current(&s.db, ns, &r.identity)
                                .unwrap()
                                .unwrap()
                                .attempt
                        );
                        crate::store::seats::resolve_bootstrap_seat(
                            &ctx, &mut s.db, ns, &r, &guard, &budget,
                        )
                        .map(CommandResult::SeatResolved)
                    }
                    Command::AttachBootstrapHandoff(r) => {
                        let tx = s
                            .db
                            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                            .unwrap();
                        let result = canonical::attach_pending(&tx, ns, &r, &guard);
                        tx.commit().unwrap();
                        if result.is_ok() && fault == Fault::AttachReply {
                            s.fault = Fault::None;
                            return Err(ApiError::unknown_outcome("committed attach reply lost"));
                        }
                        result.map(|v| CommandResult::Bootstrap(Box::new(v)))
                    }
                    _ => unreachable!(),
                }
            } else {
                let ns = &self.identity.payload.handoff.namespace;
                let tx =
                    s.db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                        .unwrap();
                let result = match command {
                    Command::BeginBootstrap(r) => {
                        assert_eq!(r.identity, self.identity);
                        assert_eq!(r.operation, self.identity.payload.handoff.keys.begin);
                        canonical::begin_pending(&tx, ns, &r.identity, UtcMillis(1))
                            .map(|v| CommandResult::Bootstrap(Box::new(v)))
                    }
                    Command::BootstrapStatus(r) => canonical::current(&tx, ns, &r.identity)
                        .and_then(|v| {
                            v.map(|v| CommandResult::Bootstrap(Box::new(v)))
                                .ok_or_else(|| {
                                    ApiError::new(ErrorCode::NotFound, "bootstrap absent")
                                })
                        }),
                    Command::BeginHandoff(r) => {
                        let parent = canonical::current(&tx, ns, &self.identity)?.unwrap();
                        assert_eq!(&r.identity, &parent.attachment.unwrap().handoff);
                        assert_ne!(r.identity.compound, self.reference.operation);
                        assert_ne!(r.identity.digest, self.identity.digest);
                        assert_eq!(r.operation, self.identity.payload.handoff.keys.begin);
                        crate::store::handoff::begin_linked_pending(
                            &tx,
                            ns,
                            &r.identity,
                            UtcMillis(1),
                        )
                        .map(CommandResult::Handoff)
                    }
                    Command::CompleteLinkedBootstrap(r) => {
                        assert_eq!(r.operation, self.identity.payload.linked_complete_key);
                        assert_eq!(
                            r.legacy_completion.operation,
                            self.identity.payload.handoff.keys.complete
                        );
                        canonical::complete_linked_pending(&tx, ns, &r, UtcMillis(2))
                            .map(|done| CommandResult::LinkedBootstrapCompleted(Box::new(done)))
                    }
                    Command::ReserveBootstrapAttempt(r) => {
                        canonical::attempts::reserve_attempt(&tx, ns, &r)
                            .map(|v| CommandResult::BootstrapReserved(Box::new(v)))
                    }
                    Command::CheckBootstrapSubmission(r) => {
                        canonical::attempts::check_submission(&tx, ns, &r)
                            .map(CommandResult::BootstrapSubmissionChecked)
                    }
                    Command::RecordBootstrapCreated(r) => {
                        canonical::attempts::record_created(&tx, ns, &r)
                            .map(|v| CommandResult::Bootstrap(Box::new(v)))
                    }
                    Command::RecordBootstrapNotSubmitted(r) => {
                        canonical::attempts::record_not_submitted(&tx, ns, &r)
                            .map(|v| CommandResult::Bootstrap(Box::new(v)))
                    }
                    _ => unreachable!(),
                };
                tx.commit().unwrap();
                if role == "linked_complete" && s.fault == Fault::TerminalSave {
                    std::fs::create_dir(self.journal.join(format!(
                        "bootstrap-{:020}-{}.terminal",
                        self.reference.ordinal,
                        self.reference.operation.as_str()
                    )))
                    .unwrap();
                    s.fault = Fault::None;
                }
                if role == "linked_complete" && s.fault == Fault::CompleteReply {
                    s.fault = Fault::None;
                    return Err(ApiError::unknown_outcome("committed wrapper reply lost"));
                }
                if role == "reserve" && s.fault == Fault::Check {
                    s.db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-000000000002' WHERE seat_id='sender'", []).unwrap();
                }
                if role == "reserve" && s.fault == Fault::SavePossible {
                    std::fs::create_dir(self.progress()).unwrap();
                }
                if (role == "reserve" && s.fault == Fault::ReserveReply)
                    || (role == "record" && s.fault == Fault::RecordReply)
                {
                    s.fault = Fault::None;
                    return Err(ApiError::unknown_outcome("committed reply lost"));
                }
                result
            }
        }
    }
    impl Peer {
        fn durable_child(
            &self,
            command: Command,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            use crate::ports::StorePort;
            let context = crate::store::connection::StoreContext::new(
                self.database_path.clone(),
                Arc::new(crate::app::SystemClock::new()),
            );
            let store = crate::store::SqliteStore::new(
                crate::store::connection::StoreContext::new(
                    self.database_path.clone(),
                    Arc::new(crate::app::SystemClock::new()),
                ),
                "i",
                Default::default(),
            )?;
            let mutation = crate::protocol::commands::PermitMutation::try_from(command).unwrap();
            let claim = match &mutation {
                crate::protocol::commands::PermitMutation::CreateThread(r) => {
                    assert_eq!(r.operation, self.identity.payload.handoff.keys.create);
                    &r.claim
                }
                crate::protocol::commands::PermitMutation::Invite(r) => {
                    assert_eq!(r.operation, self.identity.payload.handoff.keys.invite);
                    &r.claim
                }
                crate::protocol::commands::PermitMutation::SendMessage(r) => {
                    assert_eq!(r.operation, self.identity.payload.handoff.keys.send);
                    assert_eq!(r.body, self.identity.payload.handoff.body);
                    &r.claim
                }
                _ => unreachable!(),
            };
            assert_eq!(claim, &self.identity.claim);
            if let crate::protocol::commands::PermitMutation::SendMessage(send) = &mutation {
                loop {
                    match store.prepare_send_step(
                        send,
                        crate::ports::DurableWorkAdmission::new(16).unwrap(),
                        budget,
                    )? {
                        crate::ports::SendPreparationProgress::Ready { .. } => break,
                        crate::ports::SendPreparationProgress::More { .. } => {}
                        crate::ports::SendPreparationProgress::Committed(result) => {
                            return Ok(result);
                        }
                    }
                }
            }
            let permit = store.issue_cooperative_permit(
                crate::store::cooperative_permit_request(&mutation)?,
                budget,
            )?;
            if let crate::protocol::commands::PermitMutation::CreateThread(create) = &mutation {
                crate::store::control::create_thread_in_namespace(
                    &context,
                    &mut context.open_writer()?,
                    budget,
                    create,
                    permit,
                    &self.identity.payload.handoff.namespace,
                )
            } else {
                store.mutate(mutation, permit, budget)
            }
        }
        fn progress(&self) -> std::path::PathBuf {
            self.journal.join(format!(
                "handoff-{}.progress",
                self.reference.operation.as_str()
            ))
        }
        fn status(&self) -> BootstrapResult {
            canonical::current(
                &self.state.lock().unwrap().db,
                &self.identity.payload.handoff.namespace,
                &self.identity,
            )
            .unwrap()
            .unwrap()
        }
    }
    impl CreateTabPort for Peer {
        fn create_tab(&self, r: &CreateTabRequest, _: &HostCallContext) -> CreateTabOutcome {
            let mut s = self.state.lock().unwrap();
            s.calls.push("native");
            s.native_calls += 1;
            let progress: serde_json::Value =
                serde_json::from_slice(&std::fs::read(self.progress()).unwrap()).unwrap();
            assert_eq!(
                progress["possible_creation"], true,
                "durable possible-creation must precede native write"
            );
            assert_eq!(progress["request"], serde_json::to_value(r).unwrap());
            let current = canonical::current(
                &s.db,
                &self.identity.payload.handoff.namespace,
                &self.identity,
            )
            .unwrap()
            .unwrap();
            assert_eq!(current.state, BootstrapState::PossibleCreation);
            assert_eq!(
                s.calls[s.calls.len() - 2],
                "check",
                "fresh canonical check must immediately precede native call"
            );
            assert_eq!(r.workspace, self.identity.payload.workspace);
            assert_eq!(r.cwd, self.identity.payload.cwd);
            assert_eq!(r.label, self.identity.payload.label);
            assert!(!r.focus);
            assert!(r.env.is_empty());
            if let Some(marker) = &self.native_marker {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(marker)
                    .unwrap();
                writeln!(file, "{}", r.correlation.as_str()).unwrap();
                file.sync_all().unwrap();
            }
            if self.block_native {
                use std::io::{Read, Write};
                println!("QHZ9_NATIVE_READY");
                std::io::stdout().flush().unwrap();
                std::io::stdin().read_exact(&mut [0]).unwrap();
            }
            if s.fault == Fault::HostReply {
                return CreateTabOutcome::OutcomeUnknown(ApiError::unknown_outcome(
                    "host reply lost",
                ));
            }
            if matches!(
                s.fault,
                Fault::NotSubmitted | Fault::NotSubmittedRecordUnavailable
            ) {
                if s.fault == Fault::NotSubmitted {
                    s.fault = Fault::None;
                }
                return CreateTabOutcome::NotSubmitted(ApiError::host_unavailable(
                    "proven zero bytes",
                ));
            }
            if s.fault == Fault::SaveCreated {
                std::fs::remove_file(self.progress()).unwrap();
                std::fs::create_dir(self.progress()).unwrap();
            }
            let mut created = crate::protocol::handoff::topology_contract_tests::created();
            created.correlation = r.correlation.clone();
            created.witness = r.expected_witness.clone();
            if s.fault == Fault::BadCorrelation {
                created.correlation = HostCallId::new("different-call");
            }
            CreateTabOutcome::Created(Box::new(created))
        }
    }
    struct Fixture {
        _temp: Temp,
        journal: super::super::super::journal::Journal,
        peer: Arc<Peer>,
        clock: crate::app::SystemClock,
        context: HostCallContext,
        witness: crate::host::continuity::LocalEndpointWitness,
    }
    impl Fixture {
        fn new(fault: Fault) -> Self {
            Self::with_request(fault, request(), None)
        }
        fn downstream(fault: Fault) -> Self {
            Self::with_request(
                fault,
                request(),
                Some(vec![
                    "--config".into(),
                    "quoted $HOME".into(),
                    "--model".into(),
                    "saved value".into(),
                ]),
            )
        }
        fn with_request(fault: Fault, request: Request, argv: Option<Vec<String>>) -> Self {
            Self::with_layout(fault, request, argv, false, false)
        }
        fn aligned(fault: Fault, argv: Option<Vec<String>>) -> Self {
            Self::with_layout(
                fault,
                request(),
                argv.or_else(|| Some(vec!["--model".into(), "saved value".into()])),
                true,
                false,
            )
        }
        fn with_layout(
            fault: Fault,
            request: Request,
            argv: Option<Vec<String>>,
            aligned: bool,
            maximal: bool,
        ) -> Self {
            let temp = Temp::new();
            let mut identity = prepared(&request, temp.path()).unwrap();
            if let Some(argv) = argv {
                identity.payload.launch.argv = argv;
            }
            identity.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
            identity.digest = identity.semantic_digest().unwrap();
            let (journal_root, database_path) = if aligned {
                std::fs::create_dir_all(&identity.payload.handoff.namespace.state_dir).unwrap();
                let runtime = crate::daemon::paths::RuntimeContext::explicit(
                    identity.payload.handoff.namespace.state_dir.clone(),
                    identity.payload.handoff.namespace.host_endpoint.clone(),
                    None,
                )
                .unwrap();
                let paths =
                    crate::daemon::paths::InstancePaths::resolve_read_only(&runtime).unwrap();
                std::fs::create_dir_all(&paths.instance_dir).unwrap();
                identity.payload.handoff.namespace.state_dir = runtime.state_dir.clone();
                identity.payload.handoff.namespace.host_endpoint = runtime.host_endpoint.clone();
                identity.digest = identity.semantic_digest().unwrap();
                (paths.instance_dir.join("intents"), paths.database_path)
            } else {
                (
                    temp.path().join("intents"),
                    temp.path().join("canonical.db"),
                )
            };
            if maximal {
                identity.payload.launch.argv.push("--config=".into());
                let mut created = crate::protocol::handoff::topology_contract_tests::created();
                created.witness.endpoint = identity.payload.handoff.namespace.host_endpoint.clone();
                created.correlation =
                    crate::protocol::ids::HostCallId::new("00000000-0000-4000-8000-000000000001");
                let sample = BootstrapProgress {
                    version: 1,
                    identity: identity.clone(),
                    attempt: BootstrapAttempt::first(),
                    possible_creation: true,
                    request: Some(crate::ports::CreateTabRequest {
                        correlation: created.correlation.clone(),
                        workspace: identity.payload.workspace.clone(),
                        cwd: identity.payload.cwd.clone(),
                        label: identity.payload.label.clone(),
                        focus: false,
                        env: identity.payload.env.clone(),
                        expected_witness: created.witness.clone(),
                    }),
                    creation: Some(created),
                    not_submitted: false,
                };
                let length = serde_json::to_vec(&sample).unwrap().len();
                assert!(length < MAX_BOOTSTRAP_PROGRESS);
                let room = MAX_BOOTSTRAP_PROGRESS - length;
                let last = identity.payload.launch.argv.last_mut().unwrap();
                last.push_str(&"\u{1}".repeat(room / 6));
                last.push_str(&"x".repeat(room % 6));
                assert!(last.len() <= 4096);
                identity.digest = identity.semantic_digest().unwrap();
                assert!(
                    canonical::encode_identity(&identity.payload.handoff.namespace, &identity)
                        .unwrap()
                        .len()
                        < canonical::MAX_IDENTITY_BYTES
                );
            }
            let journal = super::super::super::journal::Journal::open(&journal_root).unwrap();
            let reference = publish(&journal, &identity, 1).unwrap();
            let db = rusqlite::Connection::open(&database_path).unwrap();
            schema::initialize(&db, || UtcMillis(0)).unwrap();
            let created = crate::protocol::handoff::topology_contract_tests::created();
            let boot = &created.host_incarnation;
            db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,observation_sequence,observation_admission_sequence,observation_decided_sequence,lifecycle_revision,recovery_boot,recovery_epoch) VALUES('i',0,?1,1,1,1,1,1,?1,1)",[boot.as_str()]).unwrap();
            db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) VALUES('g','i',?1,1,1,'structural-incarnation',0,0,'published',0,0,0,0)",[boot.as_str()]).unwrap();
            db.execute_batch("UPDATE host_instances SET active_snapshot_id='g',recovery_baseline_generation_id='g'; INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('sender','i','resolved','native','w1:p1',1,0,0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('canonical-thread','i','topic','goal',0,0); INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('canonical-thread','sender','joined',0)").unwrap();
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('sender',1,'w1:p1',?1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0)",[boot.as_str()]).unwrap();
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','w1:p1',?1,1,0,2,'fresh',0,'caller-terminal','structural-incarnation','native_current_target',1)",[boot.as_str()]).unwrap();
            let clock = crate::app::SystemClock::new();
            let context = HostCallContext {
                budget: super::super::super::cooperative_budget(&clock),
                expected_boot: Some(boot.clone()),
                expected_epoch: Some(1),
            };
            let mut witness = created.witness;
            witness.endpoint = identity.payload.handoff.namespace.host_endpoint.clone();
            Self {
                journal,
                peer: Arc::new(Peer {
                    state: Mutex::new(State {
                        db,
                        calls: vec![],
                        native_calls: 0,
                        fault,
                    }),
                    identity,
                    journal: journal_root,
                    database_path,
                    reference,
                    native_marker: None,
                    block_native: false,
                }),
                _temp: temp,
                clock,
                context,
                witness,
            }
        }
        fn run(&self) -> Result<BootstrapResult, RunError> {
            resume_to_attachment(
                &self.journal,
                &self.peer.reference,
                &self.peer.identity.payload.handoff.namespace,
                self.peer.as_ref(),
                self.peer.as_ref(),
                &self.clock,
                BootstrapSubmissionInputs {
                    witness: &self.witness,
                    context: &self.context,
                },
            )
        }
    }
    #[derive(Default)]
    struct DownstreamLauncher {
        starts: usize,
        unknown: bool,
        crash_after_gate: bool,
        crash_after_start: bool,
        invalid_report: bool,
        oversized_report: bool,
        report_padding: usize,
        max_report: bool,
        report_save_loss: Option<std::path::PathBuf>,
        not_submitted: bool,
        change_binding_before_gate: Option<std::path::PathBuf>,
    }
    impl super::super::super::handoff::HandoffLauncher for DownstreamLauncher {
        fn preflight(
            &mut self,
            _: &super::super::super::launch::LaunchRequest,
        ) -> Result<SeatId, RunError> {
            panic!("attachment must not allocate or preflight another recipient")
        }
        fn launch(
            &mut self,
            request: &super::super::super::launch::LaunchRequest,
            seat: &SeatId,
            gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
        ) -> Result<super::super::super::launch::LaunchReport, RunError> {
            let argv = crate::harness::launch::compose_native_argv(
                crate::protocol::authority::Harness::Codex,
                request.argv.clone(),
                vec![],
            )?;
            if let Some(path) = &self.change_binding_before_gate {
                rusqlite::Connection::open(path).unwrap().execute("UPDATE occupant_bindings SET execution_id='changed-before-launch' WHERE seat_id='sender'",[]).unwrap();
            }
            gate(true)?;
            if self.not_submitted {
                gate(false)?;
                self.not_submitted = false;
                return Err(
                    ApiError::host_unavailable("adapter proven no native submission").into(),
                );
            }
            assert!(
                !self.crash_after_gate,
                "crash after possible-start persistence"
            );
            self.starts += 1;
            assert!(
                !self.crash_after_start,
                "crash after native start before report save"
            );
            if let Some(root) = &self.report_save_loss {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o500)).unwrap();
            }
            let mut report = serde_json::json!({"outcome":if self.unknown {"outcome_unknown"} else {"started"},"pane":request.target,"seat":seat,"harness":"codex","argv":argv});
            if self.invalid_report {
                report["seat"] = serde_json::json!("different-seat");
            }
            if self.max_report {
                report["padding"] = serde_json::json!("");
                let room = 1024 * 1024 - serde_json::to_vec(&report).unwrap().len();
                report["padding"] = serde_json::json!(format!(
                    "{}{}",
                    "\u{1}".repeat(room / 6),
                    "x".repeat(room % 6)
                ));
                assert_eq!(serde_json::to_vec(&report).unwrap().len(), 1024 * 1024);
            }
            if self.report_padding > 0 {
                report["padding"] = serde_json::json!("x".repeat(self.report_padding));
            }
            if self.oversized_report {
                report["padding"] = serde_json::json!("x".repeat(1024 * 1024));
            }
            Ok(super::super::super::launch::LaunchReport {
                report,
                exit: if self.unknown { 5 } else { 0 },
            })
        }
    }

    fn downstream<W: std::io::Write>(
        f: &Fixture,
        launcher: &mut DownstreamLauncher,
        writer: &mut W,
    ) -> Result<BootstrapResult, RunError> {
        super::super::super::retry::run_bootstrap_retry_to_writer(
            &f.journal,
            &f.peer.reference,
            super::super::super::actor_route::InvocationActor::Agent,
            &f.peer.identity.payload.handoff.namespace,
            f.peer.as_ref(),
            f.peer.as_ref(),
            launcher,
            &f.clock,
            BootstrapSubmissionInputs {
                witness: &f.witness,
                context: &f.context,
            },
            &crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                ..Default::default()
            },
            writer,
        )
    }
    fn composition_source(f: &Fixture) -> crate::archival_legacy::Source {
        let ns = &f.peer.identity.payload.handoff.namespace;
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            ns.state_dir.clone(),
            ns.host_endpoint.clone(),
            None,
        )
        .unwrap();
        let paths = crate::daemon::paths::InstancePaths::resolve_read_only(&runtime).unwrap();
        assert_eq!(paths.database_path, f.peer.database_path);
        assert_eq!(paths.instance_dir.join("intents"), f.journal.root());
        crate::archival_legacy::Source::new(
            &paths,
            "i".into(),
            crate::protocol::output::ContinuationContext {
                state_dir: Some(ns.state_dir.to_string_lossy().into()),
                host: Some(ns.host_endpoint.to_string_lossy().into()),
            },
        )
    }
    fn composition_scan(f: &Fixture) -> crate::archival_legacy::Scan {
        let mut source = composition_source(f);
        let mut result = crate::archival_legacy::Scan::default();
        for _ in 0..100 {
            let page = source.scan(|| false).unwrap();
            result.hints.extend(page.hints);
            if !page.pending {
                result.coverage = page.coverage;
                return result;
            }
        }
        panic!("bounded source did not finish");
    }
    fn composition_import(
        f: &Fixture,
        scan: &crate::archival_legacy::Scan,
    ) -> Result<(), ApiError> {
        let store = crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(
                f.peer.database_path.clone(),
                Arc::new(crate::app::SystemClock::new()),
            ),
            "i",
            Default::default(),
        )
        .unwrap();
        composition_pass(&store, scan)
    }
    fn composition_pass(
        store: &crate::store::SqliteStore,
        scan: &crate::archival_legacy::Scan,
    ) -> Result<(), ApiError> {
        use crate::ports::StorePort;
        let rt = crate::store::archival::Runtime {
            boot: "composition".into(),
            mono: 0,
            utc: UtcMillis(0),
            after_ms: 60000,
            host_generation: 0,
            coherent: false,
            valid_until_mono: None,
            legacy_source: scan.coverage.clone(),
        };
        let budget = super::super::super::cooperative_budget(store.clock());
        store.archival_pass(&rt, &scan.hints, &budget).map(|_| ())
    }
    #[test]
    fn composition_canonical_created_actual_request_none_establishes_coverage() {
        let f = Fixture::aligned(Fault::ResolveUnsupported, None);
        assert!(f.run().is_err());
        std::fs::remove_file(f.peer.progress()).unwrap();
        assert!(f.run().is_err());
        let saved = load_bootstrap_progress(&f.journal, &f.peer.reference, &f.peer.identity)
            .unwrap()
            .unwrap();
        assert!(saved.request.is_none());
        assert!(saved.creation.is_some());
        let before = f.peer.status();
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "actual canonical-created requestNone must compose"
        );
        assert_eq!(scan.hints.len(), 1);
        composition_import(&f, &scan).unwrap();
        assert_eq!(f.peer.status(), before);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn composition_no_effect_request_none_vetoes_without_advancing_attempt() {
        let f = Fixture::aligned(Fault::HostReply, None);
        unknown(f.run());
        let before = f.peer.status();
        let mut saved = load_bootstrap_progress(&f.journal, &f.peer.reference, &f.peer.identity)
            .unwrap()
            .unwrap();
        saved.request = None;
        saved.not_submitted = true;
        save_bootstrap_progress(&f.journal, &f.peer.reference, &saved).unwrap();
        assert!(f.run().is_err());
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_none(),
            "impossible no-effect requestNone must veto"
        );
        composition_import(&f, &scan).unwrap();
        assert_eq!(f.peer.status(), before);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn composition_actual_retained_child_establishes_coverage_without_authority() {
        let f = Fixture::aligned(Fault::None, None);
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert!(child_progress_path(&f.journal, &f.peer.reference).exists());
        let before = f.peer.status();
        let child = &before.attachment.as_ref().unwrap().handoff;
        assert_ne!(child.compound, f.peer.reference.operation);
        assert_ne!(child.compound, f.peer.identity.compound);
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "actual retained child must be validated"
        );
        composition_import(&f, &scan).unwrap();
        assert_eq!(f.peer.status(), before);
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_actual_large_retained_terminal_and_survivors_establish_coverage() {
        let argv = vec![format!("--config={}", "\"".repeat(3800)); 16];
        let f = Fixture::aligned(Fault::None, Some(argv));
        let original = f
            .journal
            .snapshot_bootstrap_origin(&f.peer.reference)
            .unwrap();
        assert!(original.len() > 65536);
        let mut launcher = DownstreamLauncher {
            report_padding: 200000,
            ..Default::default()
        };
        let mut output = FailingOutput {
            bytes: vec![],
            write: true,
        };
        assert!(downstream(&f, &mut launcher, &mut output).is_err());
        let terminal = std::fs::read(terminal_path(&f.journal, &f.peer.reference)).unwrap();
        let child = std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap();
        assert!(terminal.len() > 262144);
        assert!(child.len() > 262144);
        let before = f.peer.status();
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "legal large complete bundle must compose"
        );
        composition_import(&f, &scan).unwrap();
        assert_eq!(f.peer.status(), before);
        assert_eq!(
            f.journal
                .snapshot_bootstrap_origin(&f.peer.reference)
                .unwrap(),
            original
        );
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_retained_completion_requires_full_canonical_equality() {
        let f = Fixture::aligned(Fault::None, None);
        downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]).unwrap();
        let path = terminal_path(&f.journal, &f.peer.reference);
        let mut terminal = read_terminal(&f.journal, &f.peer.reference)
            .unwrap()
            .unwrap();
        terminal.completed.retained.report["extra"] =
            serde_json::json!("different local retained report");
        terminal.completed.retained.report_digest = format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(
                serde_json::to_vec(&terminal.completed.retained.report).unwrap()
            )
        );
        std::fs::write(path, terminal_bytes(&terminal).unwrap()).unwrap();
        let before = f.peer.status();
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "locally valid report must reach deciding comparison"
        );
        assert!(
            composition_import(&f, &scan).is_err(),
            "full canonical retained report mismatch must refuse"
        );
        assert_eq!(f.peer.status(), before);
    }
    #[test]
    fn composition_retained_child_requires_exact_canonical_attachment() {
        let f = Fixture::aligned(Fault::None, None);
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let path = child_progress_path(&f.journal, &f.peer.reference);
        let mut child: ChildProgress =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        child.attachment.created.correlation =
            crate::protocol::ids::HostCallId::new("different-canonical-correlation");
        super::super::super::handoff::save_progress_at(&f.journal, &path, &child).unwrap();
        std::fs::remove_file(f.peer.progress()).unwrap();
        let before = f.peer.status();
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "locally coherent attachment reaches deciding comparison"
        );
        assert!(
            composition_import(&f, &scan).is_err(),
            "local child cannot replace canonical attachment"
        );
        assert_eq!(f.peer.status(), before);
    }
    struct CompositionHost;
    impl crate::ports::HostPort for CompositionHost {
        fn native_launch_capability(&self) -> crate::ports::NativeLaunchCapability {
            crate::ports::NativeLaunchCapability::Unsupported
        }
        fn observe_current_target(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<crate::ports::HostObservation, ApiError> {
            panic!("incoherent archival never observes")
        }
        fn observe_current_target_for_archival(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<crate::ports::ComposerObservation, ApiError> {
            panic!("incoherent archival never observes")
        }
        fn enumerate_targets(
            &self,
            _: &HostCallContext,
        ) -> Result<crate::ports::HostSnapshot, ApiError> {
            panic!("archival never enumerates")
        }
        fn safe_wake_target(
            &self,
            _: &SeatId,
            _: &crate::ports::HostObservation,
        ) -> Option<crate::ports::SafeWakeTarget> {
            None
        }
        fn submit_prompt(
            &self,
            _: &crate::ports::SafeWakeTarget,
            _: &str,
            _: &HostCallContext,
        ) -> Result<crate::ports::PromptOutcome, ApiError> {
            panic!("archival never submits")
        }
        fn launch_native(
            &self,
            _: crate::ports::NativeLaunchRequest,
            _: &HostCallContext,
        ) -> Result<crate::ports::NativeLaunchOutcome, ApiError> {
            panic!("archival never launches")
        }
        fn pane_agent_state(
            &self,
            _: &crate::ports::SafeWakeTarget,
            _: &HostCallContext,
        ) -> Result<crate::ports::AgentComposerState, ApiError> {
            panic!("archival never reads agent")
        }
        fn send_submit_key(
            &self,
            _: &crate::ports::SafeWakeTarget,
            _: &HostCallContext,
        ) -> Result<(), ApiError> {
            panic!("archival never types")
        }
    }
    fn composition_worker(f: &Fixture) -> crate::service::archival::ArchivalWorker {
        crate::service::archival::ArchivalWorker {
            store: Arc::new(
                crate::store::SqliteStore::new(
                    crate::store::connection::StoreContext::new(
                        f.peer.database_path.clone(),
                        Arc::new(crate::app::SystemClock::new()),
                    ),
                    "i",
                    Default::default(),
                )
                .unwrap(),
            ),
            host: Arc::new(CompositionHost),
            writer: Arc::new(crate::service::fair_writer::FairWriter::new(32)),
            reachability: Arc::new(crate::service::host_reachability::HostReachability::default()),
            source: composition_source(f),
            boot: "composition-worker".into(),
            after_ms: 60000,
            cancellation: Default::default(),
        }
    }
    fn composition_drain(worker: &mut crate::service::archival::ArchivalWorker) {
        for _ in 0..200 {
            if !worker.run_page().unwrap() {
                return;
            }
        }
        panic!("worker traversal failed to terminate");
    }
    fn composition_veto(f: &Fixture) -> bool {
        f.peer
            .state
            .lock()
            .unwrap()
            .db
            .query_row(
                "SELECT bootstrap_veto FROM archival_instances WHERE instance_id='i'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn composition_actual_large_bundle_worker_admission_loss_stays_sticky_then_fresh_recovers() {
        let f = Fixture::aligned(
            Fault::None,
            Some(vec![format!("--config={}", "\"".repeat(3800)); 16]),
        );
        let mut launcher = DownstreamLauncher {
            report_padding: 200000,
            ..Default::default()
        };
        assert!(
            downstream(
                &f,
                &mut launcher,
                &mut FailingOutput {
                    bytes: vec![],
                    write: true
                }
            )
            .is_err()
        );
        let before = f.peer.status();
        let mut worker = composition_worker(&f);
        worker.writer = Arc::new(crate::service::fair_writer::FairWriter::new(0));
        assert_eq!(worker.run_page().unwrap_err().code, ErrorCode::StoreBusy);
        worker.writer = Arc::new(crate::service::fair_writer::FairWriter::new(32));
        composition_drain(&mut worker);
        assert!(
            composition_veto(&f),
            "consumed bundle must retain traversal veto at EOF"
        );
        composition_drain(&mut worker);
        assert!(
            !composition_veto(&f),
            "fresh admitted traversal must recover"
        );
        assert_eq!(f.peer.status(), before);
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_actual_retained_bundle_cap_and_corruption_controls() {
        for control in [
            "child_version",
            "child_unknown",
            "terminal_version",
            "terminal_unknown",
            "origin_contradiction",
            "child_contradiction",
            "child_orphan",
            "child_wrong_local",
            "origin_overflow",
            "submission_overflow",
            "child_overflow",
            "terminal_overflow",
            "embedded_origin_overflow",
        ] {
            let f = Fixture::aligned(Fault::None, None);
            assert!(
                downstream(
                    &f,
                    &mut DownstreamLauncher::default(),
                    &mut FailingOutput {
                        bytes: vec![],
                        write: true
                    }
                )
                .is_err()
            );
            let original = f.journal.root().join(format!(
                "{:020}-{}.intent",
                f.peer.reference.ordinal,
                f.peer.reference.operation.as_str()
            ));
            let child = child_progress_path(&f.journal, &f.peer.reference);
            let terminal = terminal_path(&f.journal, &f.peer.reference);
            match control {
                "child_version" | "child_unknown" | "child_contradiction" => {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&child).unwrap()).unwrap();
                    if control == "child_version" {
                        value["version"] = serde_json::json!(2);
                    } else if control == "child_unknown" {
                        value["unknown"] = serde_json::json!(true);
                    } else {
                        value["progress"]["launch"]["extra"] = serde_json::json!("contradiction");
                    }
                    std::fs::write(&child, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "terminal_version" | "terminal_unknown" | "embedded_origin_overflow" => {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&terminal).unwrap()).unwrap();
                    if control == "terminal_version" {
                        value["version"] = serde_json::json!(2);
                    } else if control == "terminal_unknown" {
                        value["unknown"] = serde_json::json!(true);
                    } else {
                        value["original"] =
                            serde_json::json!("x".repeat(
                                super::super::super::journal::MAX_BOOTSTRAP_ORIGIN_BYTES + 1
                            ));
                    }
                    std::fs::write(&terminal, serde_json::to_vec(&value).unwrap()).unwrap();
                }
                "origin_contradiction" => {
                    let mut data = std::fs::read(&original).unwrap();
                    data.push(b' ');
                    std::fs::write(&original, data).unwrap();
                }
                "child_orphan" => {
                    std::fs::remove_file(&terminal).unwrap();
                    std::fs::remove_file(&original).unwrap();
                }
                "child_wrong_local" => {
                    std::fs::rename(
                        &child,
                        f.journal
                            .root()
                            .join(format!("bootstrap-child-{}.progress", uuid::Uuid::new_v4())),
                    )
                    .unwrap();
                }
                "origin_overflow" => std::fs::write(
                    &original,
                    vec![b'x'; super::super::super::journal::MAX_BOOTSTRAP_ORIGIN_BYTES + 1],
                )
                .unwrap(),
                "submission_overflow" => {
                    std::fs::write(f.peer.progress(), vec![b'x'; MAX_BOOTSTRAP_PROGRESS + 1])
                        .unwrap()
                }
                "child_overflow" => {
                    std::fs::write(&child, vec![b'x'; MAX_LINKED_LOCAL_BYTES + 1]).unwrap()
                }
                "terminal_overflow" => {
                    std::fs::write(&terminal, vec![b'x'; MAX_BOOTSTRAP_TERMINAL_BYTES + 1]).unwrap()
                }
                _ => unreachable!(),
            }
            let before = f.peer.status();
            let scan = composition_scan(&f);
            assert!(
                scan.coverage.is_none(),
                "control {control} must veto complete coverage"
            );
            assert_eq!(f.peer.status(), before);
        }
    }
    #[test]
    fn composition_child_lookup_many_names_and_modern_header_progress() {
        let f = Fixture::aligned(Fault::None, None);
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        for n in 0..700 {
            std::fs::write(f.journal.root().join(format!(".intent-noise-{n}")), []).unwrap();
        }
        let mut source = composition_source(&f);
        let mut pages = 0;
        let mut count = 0;
        let mut reads = 0;
        let mut maximum_capacity = 0;
        loop {
            let scan = source.scan(|| false).unwrap();
            let statistics = source.last_reads;
            assert!(statistics[0] <= 3, "one original/submission/child bundle");
            assert_eq!(statistics[2], statistics[1] + statistics[0]);
            reads += statistics[0];
            maximum_capacity = maximum_capacity.max(statistics[2]);
            pages += 1;
            count += scan
                .hints
                .iter()
                .filter(|h| matches!(h, crate::archival_legacy::Hint::Bootstrap { .. }))
                .count();
            if !scan.pending {
                assert!(scan.coverage.is_some());
                break;
            }
            assert!(scan.coverage.is_none());
            assert!(pages < 100);
        }
        assert!(
            pages > 40,
            "both directory and bounded lookup must make incremental progress"
        );
        assert_eq!(
            count, 2,
            "parent and independently validated child each yield exact hint"
        );
        assert_eq!(reads, 6, "metadata lookup never rereads child wire bytes");
        eprintln!(
            "COMPOSITION_LOOKUP unrelated_names=700 pages={pages} hints={count} full_reads={reads} max_page_buffer_capacity={maximum_capacity}"
        );
        let original = f.journal.root().join(format!(
            "{:020}-{}.intent",
            f.peer.reference.ordinal,
            f.peer.reference.operation.as_str()
        ));
        let mut data = std::fs::read(&original).unwrap();
        data.splice(0..0, vec![b' '; 5000]);
        std::fs::write(&original, data).unwrap();
        assert!(
            composition_scan(&f).coverage.is_some(),
            "typed Bootstrap decoder has no independent4096 header cap"
        );
    }
    #[test]
    fn composition_cancel_between_reads_vetoes_consumed_large_bundle() {
        let f = Fixture::aligned(
            Fault::None,
            Some(vec![format!("--config={}", "\"".repeat(3800)); 16]),
        );
        assert!(
            downstream(
                &f,
                &mut DownstreamLauncher::default(),
                &mut FailingOutput {
                    bytes: vec![],
                    write: true
                }
            )
            .is_err()
        );
        let before = f.peer.status();
        let mut source = composition_source(&f);
        let calls = std::cell::Cell::new(0);
        let error = source
            .scan(|| {
                calls.set(calls.get() + 1);
                calls.get() >= 6
            })
            .err()
            .expect("cancel during consumed bundle");
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(f.peer.status(), before);
    }
    #[cfg(target_os = "macos")]
    fn composition_heap_statistics() -> [usize; 3] {
        #[repr(C)]
        #[derive(Default)]
        struct Statistics {
            blocks: u32,
            in_use: usize,
            maximum_touched: usize,
            allocated: usize,
        }
        unsafe extern "C" {
            fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut Statistics);
        }
        let mut stats = Statistics::default();
        // SDK malloc.h: NULL sums all zones; max_size_in_use is touched-memory
        // high water, separate from the sampled current live allocation count.
        unsafe {
            malloc_zone_statistics(std::ptr::null_mut(), &mut stats);
        }
        [stats.in_use, stats.maximum_touched, stats.allocated]
    }
    #[cfg(target_os = "macos")]
    struct CompositionSampler {
        stop: Arc<std::sync::atomic::AtomicBool>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    #[cfg(target_os = "macos")]
    impl CompositionSampler {
        fn new(baseline: usize) -> Self {
            use std::sync::atomic::Ordering;
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let peak = Arc::new(std::sync::atomic::AtomicUsize::new(baseline));
            let thread = {
                let stop = stop.clone();
                let peak = peak.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        peak.fetch_max(composition_heap_statistics()[0], Ordering::Relaxed);
                        std::thread::sleep(std::time::Duration::from_micros(50));
                    }
                })
            };
            Self {
                stop,
                peak,
                thread: Some(thread),
            }
        }
    }
    #[cfg(target_os = "macos")]
    impl Drop for CompositionSampler {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn composition_maximum_legal_escaped_serializer_memory_and_budgets() {
        use crate::ports::StorePort;
        use std::sync::atomic::Ordering;
        let f = Fixture::with_layout(
            Fault::None,
            request(),
            Some(vec![format!("--config={}", "\u{1}".repeat(1250)); 16]),
            true,
            true,
        );
        let mut launcher = DownstreamLauncher {
            max_report: true,
            ..Default::default()
        };
        assert!(
            downstream(
                &f,
                &mut launcher,
                &mut FailingOutput {
                    bytes: vec![],
                    write: true
                }
            )
            .is_err()
        );
        let before = f.peer.status();
        assert_eq!(
            before.state,
            BootstrapState::Completed,
            "maximum report must actually canonically complete"
        );
        let child = std::fs::metadata(child_progress_path(&f.journal, &f.peer.reference))
            .unwrap()
            .len();
        let terminal = std::fs::metadata(terminal_path(&f.journal, &f.peer.reference))
            .unwrap()
            .len();
        let original = f
            .journal
            .snapshot_bootstrap_origin(&f.peer.reference)
            .unwrap()
            .len();
        let submission = std::fs::metadata(f.peer.progress()).unwrap().len();
        assert_eq!(submission as usize, MAX_BOOTSTRAP_PROGRESS);
        let identity = canonical::encode_identity(
            &f.peer.identity.payload.handoff.namespace,
            &f.peer.identity,
        )
        .unwrap()
        .len();
        let report = serde_json::to_vec(&before.completed.as_ref().unwrap().retained.report)
            .unwrap()
            .len();
        let baseline = composition_heap_statistics();
        let sampler = CompositionSampler::new(baseline[0]);
        let store = Arc::new(
            crate::store::SqliteStore::new(
                crate::store::connection::StoreContext::new(
                    f.peer.database_path.clone(),
                    Arc::new(crate::app::SystemClock::new()),
                ),
                "i",
                Default::default(),
            )
            .unwrap(),
        );
        let writer = Arc::new(crate::service::fair_writer::FairWriter::new(32));
        let mut source = composition_source(&f);
        let started = std::time::Instant::now();
        let mut pages = 0;
        let mut reads = 0;
        let mut maximum_capacity = 0;
        let mut maximum_scan = std::time::Duration::ZERO;
        let mut maximum_deciding = std::time::Duration::ZERO;
        let mut maximum_foreground = std::time::Duration::ZERO;
        loop {
            let page_start = std::time::Instant::now();
            let scan = source.scan(|| false).unwrap();
            maximum_scan = maximum_scan.max(page_start.elapsed());
            let statistics = source.last_reads;
            reads += statistics[0];
            maximum_capacity = maximum_capacity.max(statistics[2]);
            assert!(statistics[0] <= 4, "one oversized bundle per page");
            assert!(statistics[1] <= 5_390_340);
            assert_eq!(
                statistics[2],
                statistics[1] + statistics[0],
                "fixed metadata+1 buffer allocations"
            );
            let budget = super::super::super::cooperative_budget(store.clock());
            let turn = writer.enter_background(&budget, store.clock()).unwrap();
            let (ready, waiting) = std::sync::mpsc::sync_channel(0);
            let contender = {
                let writer = writer.clone();
                let store = store.clone();
                std::thread::spawn(move || {
                    let budget = super::super::super::cooperative_budget(store.clock());
                    let start = std::time::Instant::now();
                    ready.send(()).unwrap();
                    let _turn = writer.enter_foreground(&budget, store.clock()).unwrap();
                    start.elapsed()
                })
            };
            waiting.recv().unwrap();
            let deciding_start = std::time::Instant::now();
            composition_pass(&store, &scan).unwrap();
            maximum_deciding = maximum_deciding.max(deciding_start.elapsed());
            drop(turn);
            maximum_foreground = maximum_foreground.max(contender.join().unwrap());
            pages += 1;
            if !scan.pending {
                assert!(scan.coverage.is_some());
                break;
            }
            assert!(pages < 100);
        }
        let sampled_peak = sampler.peak.load(Ordering::Relaxed);
        drop(sampler);
        let after = composition_heap_statistics();
        eprintln!(
            "COMPOSITION_MAX identity={identity} report={report} original={original} submission={submission} child={child} terminal={terminal} pages={pages} reads={reads} max_page_buffer_capacity={maximum_capacity} scan_max_ms={} deciding_max_ms={} foreground_wait_max_ms={} total_ms={} baseline_live={} sampled_peak_live={} all_zone_touched_highwater={} allocated_after={}",
            maximum_scan.as_millis(),
            maximum_deciding.as_millis(),
            maximum_foreground.as_millis(),
            started.elapsed().as_millis(),
            baseline[0],
            sampled_peak,
            after[1],
            after[2]
        );
        assert_eq!(f.peer.status(), before);
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_actual_terminal_after_cleanup_matches_human_original_and_worker() {
        let f = Fixture::aligned(Fault::None, None);
        let pending = f.journal.load(&f.peer.reference).unwrap();
        downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]).unwrap();
        assert!(f.journal.load(&f.peer.reference).is_err());
        let retained = super::super::super::retry::load_original_for_actor(
            &f.journal,
            &f.peer.reference.recovery_ref(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&retained.header).unwrap(),
            serde_json::to_value(&pending.header).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&retained.semantic).unwrap(),
            serde_json::to_value(&pending.semantic).unwrap()
        );
        assert_eq!(
            super::super::super::journal::classify_original_actor(
                &retained.header.scope,
                &retained.semantic
            )
            .unwrap(),
            super::super::super::journal::OriginalActor::Agent
        );
        let before = f.peer.status();
        let scan = composition_scan(&f);
        assert!(scan.coverage.is_some());
        assert_eq!(scan.hints.len(), 1);
        let mut worker = composition_worker(&f);
        composition_drain(&mut worker);
        assert!(!composition_veto(&f));
        assert_eq!(f.peer.status(), before);
    }
    #[test]
    fn composition_absent_parent_and_foreign_database_cannot_be_authorized_locally() {
        let f = Fixture::aligned(Fault::None, None);
        let scan = composition_scan(&f);
        assert!(scan.coverage.is_some());
        composition_import(&f, &scan).unwrap();
        assert!(composition_veto(&f));
        assert_eq!(
            f.peer
                .state
                .lock()
                .unwrap()
                .db
                .query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let other = Fixture::aligned(Fault::None, None);
        assert!(
            composition_import(&other, &scan).is_err(),
            "database_list must reject another aligned path"
        );
        assert_eq!(other.peer.state.lock().unwrap().native_calls, 0);
    }
    #[test]
    fn composition_actual_new_thread_child_requires_canonical_created_thread() {
        let mut request = request();
        request.thread = None;
        let f = Fixture::with_layout(
            Fault::None,
            request,
            Some(vec!["--model".into(), "saved value".into()]),
            true,
            false,
        );
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let path = child_progress_path(&f.journal, &f.peer.reference);
        let mut child: ChildProgress =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(child.attachment.handoff.thread.is_none());
        assert!(child.progress.thread.is_some());
        let scan = composition_scan(&f);
        assert!(scan.coverage.is_some());
        composition_import(&f, &scan).unwrap();
        child.progress.thread = Some(ThreadId::new("wrong-new-thread"));
        super::super::super::handoff::save_progress_at(&f.journal, &path, &child).unwrap();
        let scan = composition_scan(&f);
        assert!(scan.coverage.is_some());
        assert!(composition_import(&f, &scan).is_err());
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_terminal_surviving_submission_must_match_completed_attempt_and_creation() {
        for control in ["creation", "attempt"] {
            let f = Fixture::aligned(Fault::None, None);
            assert!(
                downstream(
                    &f,
                    &mut DownstreamLauncher::default(),
                    &mut FailingOutput {
                        bytes: vec![],
                        write: true
                    }
                )
                .is_err()
            );
            let mut progress =
                load_bootstrap_progress(&f.journal, &f.peer.reference, &f.peer.identity)
                    .unwrap()
                    .unwrap();
            if control == "creation" {
                progress.request = None;
                progress.creation.as_mut().unwrap().correlation =
                    crate::protocol::ids::HostCallId::new("contradictory-retained-creation");
            } else {
                progress.attempt = BootstrapAttempt::new(2).unwrap();
            }
            save_bootstrap_progress(&f.journal, &f.peer.reference, &progress).unwrap();
            let scan = composition_scan(&f);
            assert!(
                scan.coverage.is_none(),
                "terminal surviving submission {control} contradiction must veto"
            );
        }
    }
    #[test]
    fn downstream_atomic_completion_runs_exact_child_then_flushes_success() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher::default();
        let mut output = vec![];
        let result = downstream(&f, &mut launcher, &mut output).unwrap();
        assert_eq!(
            result.state,
            BootstrapState::Completed,
            "attachment alone does not complete downstream"
        );
        assert_eq!(launcher.starts, 1);
        let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["handoff"]["outcome"], "started");
        assert!(f.journal.load(&f.peer.reference).is_err());
    }
    #[test]
    fn downstream_original_agent_after_intent_removed_replays_history() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher::default();
        let mut first = vec![];
        downstream(&f, &mut launcher, &mut first).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        let mut replay = vec![];
        let result = downstream(&f, &mut launcher, &mut replay);
        assert!(
            result.is_ok(),
            "retained Agent original must survive intent cleanup: {result:?}"
        );
        assert_eq!(replay, first);
        assert_eq!(launcher.starts, 1);
        assert!(
            f.peer
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|role| *role == "status")
        );
    }
    #[test]
    fn downstream_original_human_after_intent_removed_refuses_root_before_effects() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher::default();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        let path = terminal_path(&f.journal, &f.peer.reference);
        let mut terminal: BootstrapTerminal =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // A structurally valid retained Human original is an origin selection
        // fixture. The root classifier must refuse BEFORE a canonical query.
        let pending = super::super::super::journal::Journal::decode_bootstrap_origin(
            &f.peer.reference,
            terminal.original.as_bytes(),
        )
        .unwrap();
        let mut identity = terminal.completed.identity.clone();
        identity.claim.harness = crate::protocol::authority::Harness::Human;
        identity.digest = identity.semantic_digest().unwrap();
        let plan = downstream_plan(
            &identity,
            &terminal.completed.attachment.created,
            &terminal.completed.attachment.resolved_seat,
        )
        .unwrap();
        terminal.completed.identity = identity.clone();
        terminal.completed.attachment.handoff = downstream_identity(&identity, &plan).unwrap();
        let semantic = super::super::super::journal::SemanticMutation::freeze(
            super::super::super::journal::SemanticMutation::HandoffBootstrap(Box::new(
                super::super::super::journal::BootstrapPlan {
                    version: 1,
                    payload: identity.payload.clone(),
                },
            )),
            identity.claim.clone(),
        )
        .unwrap();
        let mut header = pending.header;
        header.semantic_digest = identity.digest;
        terminal.original = format!(
            "{}\n{}",
            serde_json::to_string(&header).unwrap(),
            serde_json::to_string(&semantic).unwrap()
        );
        std::fs::write(path, serde_json::to_vec(&terminal).unwrap()).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        let before = std::fs::read_dir(f.journal.root()).unwrap().count();
        let mut output = vec![];
        let result = downstream(&f, &mut launcher, &mut output);
        assert!(
            format!("{result:?}")
                .contains("person/operator retry requires immediate human namespace"),
            "retained Human root gate missing: {result:?}"
        );
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        assert!(output.is_empty());
        assert_eq!(std::fs::read_dir(f.journal.root()).unwrap().count(), before);
        assert_eq!(launcher.starts, 1);
    }
    fn assert_downstream_once(f: &Fixture, launcher: &DownstreamLauncher) {
        let state = f.peer.state.lock().unwrap();
        assert_eq!(state.native_calls, 1);
        assert_eq!(launcher.starts, 1);
        assert_eq!(
            state
                .db
                .query_row(
                    "SELECT count(*) FROM messages WHERE actor_seat_id='sender' AND kind='ordinary'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            state
                .db
                .query_row("SELECT count(*) FROM invitations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn downstream_new_thread_create_links_parent_child_in_same_real_transaction() {
        let mut request = request();
        request.thread = None;
        request.thread_name = Some("new-channel".into());
        request.topic = Some("new topic".into());
        request.goal = Some("new goal".into());
        let f = Fixture::with_request(Fault::CompleteBefore, request, Some(vec![]));
        let mut launcher = DownstreamLauncher::default();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let saved_parent = f.peer.status();
        let state = f.peer.state.lock().unwrap();
        let child = crate::store::handoff::current(
            &state.db,
            &saved_parent.attachment.as_ref().unwrap().handoff,
        )
        .unwrap()
        .unwrap();
        assert_eq!(child.state, HandoffState::Live);
        assert!(child.thread.is_some());
        assert_eq!(
            state
                .db
                .query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            child.thread.unwrap().as_str()
        );
        assert_eq!(
            state
                .db
                .query_row(
                    "SELECT count(*) FROM threads WHERE name='new-channel'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        drop(state);
        assert_eq!(
            downstream(&f, &mut launcher, &mut vec![]).unwrap().state,
            BootstrapState::Completed
        );
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_wrapper_before_call_failure_retries_cached_real_report_only() {
        let f = Fixture::downstream(Fault::CompleteBefore);
        let mut launcher = DownstreamLauncher::default();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let parent = f.peer.status();
        assert_eq!(parent.state, BootstrapState::Attached);
        assert_eq!(
            crate::store::handoff::current(
                &f.peer.state.lock().unwrap().db,
                &parent.attachment.unwrap().handoff
            )
            .unwrap()
            .unwrap()
            .state,
            HandoffState::Live
        );
        let parent_progress: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.peer.progress()).unwrap()).unwrap();
        assert_eq!(parent_progress["possible_creation"], true);
        assert!(parent_progress.get("possible_start").is_none());
        let child: ChildProgress = serde_json::from_slice(
            &std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap(),
        )
        .unwrap();
        assert!(child.progress.possible_start);
        assert_eq!(child.progress.launch.unwrap()["outcome"], "started");
        f.peer.state.lock().unwrap().calls.clear();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert!(!f.peer.state.lock().unwrap().calls.iter().any(|r| matches!(
            *r,
            "invite_child" | "send_child" | "create_child" | "native" | "resolve" | "attach"
        )));
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_wrapper_reply_loss_recovers_atomic_canonical_report() {
        let f = Fixture::downstream(Fault::CompleteReply);
        let mut launcher = DownstreamLauncher::default();
        let done = downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_eq!(done.state, BootstrapState::Completed);
        assert_eq!(
            done.completed.unwrap().legacy_result.state,
            HandoffState::Completed
        );
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_crash_before_or_after_native_start_never_relaunches() {
        for after_start in [false, true] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher {
                crash_after_gate: !after_start,
                crash_after_start: after_start,
                ..Default::default()
            };
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| downstream(
                    &f,
                    &mut launcher,
                    &mut vec![]
                )))
                .is_err()
            );
            launcher.crash_after_gate = false;
            launcher.crash_after_start = false;
            let starts = launcher.starts;
            unknown(downstream(&f, &mut launcher, &mut vec![]));
            assert_eq!(launcher.starts, starts);
            assert_eq!(f.peer.status().state, BootstrapState::Attached);
            assert!(
                !f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .contains(&"linked_complete")
            );
        }
    }
    #[test]
    fn downstream_unknown_launch_never_becomes_success_or_clears_fence() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        unknown(downstream(&f, &mut launcher, &mut vec![]));
        launcher.unknown = false;
        unknown(downstream(&f, &mut launcher, &mut vec![]));
        assert_downstream_once(&f, &launcher);
        assert_eq!(f.peer.status().state, BootstrapState::Attached);
        assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
    }
    #[test]
    fn downstream_adapter_proven_not_submitted_can_retry_start_without_restage() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher {
            not_submitted: true,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let child: ChildProgress = serde_json::from_slice(
            &std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap(),
        )
        .unwrap();
        assert!(!child.progress.possible_start);
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_launch_report_save_loss_retains_possible_start_after_reopen() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher {
            report_save_loss: Some(f.journal.root().into()),
            ..Default::default()
        };
        let failed = downstream(&f, &mut launcher, &mut vec![]);
        std::fs::set_permissions(f.journal.root(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(failed, Err(RunError::Io(_))), "{failed:?}");
        let saved: ChildProgress = serde_json::from_slice(
            &std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap(),
        )
        .unwrap();
        assert!(saved.progress.possible_start);
        assert!(saved.progress.launch.is_none());
        launcher.report_save_loss = None;
        unknown(downstream(&f, &mut launcher, &mut vec![]));
        assert_downstream_once(&f, &launcher);
    }
    struct FailingOutput {
        bytes: Vec<u8>,
        write: bool,
    }
    impl std::io::Write for FailingOutput {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.write {
                return Err(std::io::Error::other("injected output write loss"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("injected output flush loss"))
        }
    }
    #[test]
    fn downstream_failed_output_reopen_archive_depart_binding_changes_are_history_only() {
        for write in [false, true] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher::default();
            let mut failed = FailingOutput {
                bytes: vec![],
                write,
            };
            assert!(downstream(&f, &mut launcher, &mut failed).is_err());
            assert_eq!(f.peer.status().state, BootstrapState::Completed);
            let original = f
                .journal
                .snapshot_bootstrap_origin(&f.peer.reference)
                .unwrap();
            let mut state = f.peer.state.lock().unwrap();
            state.db.execute_batch("UPDATE threads SET archived=1; DELETE FROM memberships WHERE seat_id='sender'; UPDATE occupant_bindings SET execution_id='changed',generation=generation+1 WHERE seat_id='sender'; UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id='sender'").unwrap();
            state.db = rusqlite::Connection::open(f._temp.path().join("canonical.db")).unwrap();
            state.calls.clear();
            drop(state);
            let journal = super::super::super::journal::Journal::open(f.journal.root()).unwrap();
            let mut output = vec![];
            super::super::super::retry::run_bootstrap_retry_to_writer(
                &journal,
                &f.peer.reference,
                super::super::super::actor_route::InvocationActor::Agent,
                &f.peer.identity.payload.handoff.namespace,
                f.peer.as_ref(),
                f.peer.as_ref(),
                &mut launcher,
                &f.clock,
                BootstrapSubmissionInputs {
                    witness: &f.witness,
                    context: &f.context,
                },
                &crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                },
                &mut output,
            )
            .unwrap();
            if !write {
                assert_eq!(output, failed.bytes);
            }
            assert_eq!(
                read_terminal(&journal, &f.peer.reference)
                    .unwrap()
                    .unwrap()
                    .original
                    .as_bytes(),
                original
            );
            assert!(
                f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .iter()
                    .all(|r| *r == "status")
            );
            assert_downstream_once(&f, &launcher);
        }
    }
    #[test]
    fn downstream_terminal_save_loss_recovers_canonical_success_without_effects() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher::default();
        let path = terminal_path(&f.journal, &f.peer.reference);
        std::fs::create_dir(&path).unwrap();
        // A malformed existing retained origin must refuse before any effect.
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        std::fs::remove_dir(&path).unwrap();
        // Native/semantic work completes, then a terminal-local write obstacle
        // is installed at wrapper commit, outside the local origin preflight.
        f.peer.state.lock().unwrap().fault = Fault::TerminalSave;
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert_eq!(f.peer.status().state, BootstrapState::Completed);
        std::fs::remove_dir(&path).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert!(
            f.peer
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|r| *r == "status")
        );
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_invalid_or_oversized_success_report_refuses_before_wrapper_commit() {
        for oversized in [false, true] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher {
                invalid_report: !oversized,
                oversized_report: oversized,
                ..Default::default()
            };
            assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
            assert_eq!(f.peer.status().state, BootstrapState::Attached);
            assert!(
                !f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .contains(&"linked_complete")
            );
            assert_eq!(launcher.starts, 1);
        }
    }
    #[test]
    fn downstream_canonical_accepted_escaped_origin_exceeding_delivery_bound_replays() {
        let argv = vec![format!("--config={}", "\"".repeat(3800)); 16];
        let f = Fixture::with_request(Fault::None, request(), Some(argv));
        let original = f
            .journal
            .snapshot_bootstrap_origin(&f.peer.reference)
            .unwrap();
        assert!(original.len() > 65536);
        assert!(original.len() < super::super::super::journal::MAX_BOOTSTRAP_ORIGIN_BYTES);
        canonical::encode_identity(&f.peer.identity.payload.handoff.namespace, &f.peer.identity)
            .unwrap();
        let mut launcher = DownstreamLauncher::default();
        let mut first = vec![];
        downstream(&f, &mut launcher, &mut first).unwrap();
        let mut second = vec![];
        downstream(&f, &mut launcher, &mut second).unwrap();
        assert_eq!(first, second);
        assert_downstream_once(&f, &launcher);
        assert_eq!(
            read_terminal(&f.journal, &f.peer.reference)
                .unwrap()
                .unwrap()
                .original
                .as_bytes(),
            original
        );
    }
    #[test]
    fn downstream_original_binding_change_at_native_gate_stops_start() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher {
            change_binding_before_gate: Some(f._temp.path().join("canonical.db")),
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert_eq!(
            launcher.starts, 0,
            "fresh canonical original actor guard must precede native start"
        );
        assert_eq!(f.peer.status().state, BootstrapState::Attached);
    }
    struct CleanupLoss {
        root: std::path::PathBuf,
        reference: super::super::super::journal::IntentRef,
        mode: &'static str,
    }
    impl std::io::Write for CleanupLoss {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            use std::os::unix::fs::PermissionsExt;
            if self.mode == "child" {
                let path = self.root.join(format!(
                    "bootstrap-child-{}.progress",
                    self.reference.operation.as_str()
                ));
                std::fs::remove_file(&path)?;
                std::fs::create_dir(path)?;
            } else {
                for path in [
                    self.root.join(format!(
                        "bootstrap-child-{}.progress",
                        self.reference.operation.as_str()
                    )),
                    self.root.join(format!(
                        "handoff-{}.progress",
                        self.reference.operation.as_str()
                    )),
                ] {
                    std::fs::remove_file(path)?;
                }
                std::fs::set_permissions(
                    &self.root,
                    std::fs::Permissions::from_mode(if self.mode == "intent" {
                        0o500
                    } else {
                        0o300
                    }),
                )?;
            }
            Ok(())
        }
    }
    #[test]
    fn downstream_child_intent_and_directory_sync_cleanup_loss_replays_only_history() {
        use std::os::unix::fs::PermissionsExt;
        for mode in ["child", "intent", "sync"] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher::default();
            let mut writer = CleanupLoss {
                root: f.journal.root().into(),
                reference: f.peer.reference.clone(),
                mode,
            };
            let failed = downstream(&f, &mut launcher, &mut writer);
            std::fs::set_permissions(f.journal.root(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            assert!(matches!(failed, Err(RunError::Io(_))), "{mode}: {failed:?}");
            if mode == "child" {
                std::fs::remove_dir(child_progress_path(&f.journal, &f.peer.reference)).unwrap();
            }
            f.peer.state.lock().unwrap().calls.clear();
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            assert!(
                f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .iter()
                    .all(|r| *r == "status")
            );
            assert_downstream_once(&f, &launcher);
        }
    }
    #[test]
    fn downstream_retained_origin_damage_or_ambiguity_refuses_before_client_or_output() {
        for damage in [
            "missing",
            "malformed",
            "oversized",
            "symlink",
            "unknown-top",
            "unknown-nested",
            "missing-optional",
            "origin-unknown",
            "origin-oversized",
            "report-digest",
            "argv",
            "cross-kind",
            "cross-operation",
            "intent-kind",
            "duplicate-intent",
            "conflicting-intent",
        ] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher::default();
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            let path = terminal_path(&f.journal, &f.peer.reference);
            let bytes = std::fs::read(&path).unwrap();
            let mut saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let original = saved["original"].as_str().unwrap().to_owned();
            let intent = f.journal.root().join(format!(
                "{:020}-{}.intent",
                f.peer.reference.ordinal,
                f.peer.reference.operation.as_str()
            ));
            match damage {
                "missing" => std::fs::remove_file(&path).unwrap(),
                "malformed" => std::fs::write(&path, b"{broken").unwrap(),
                "oversized" => {
                    std::fs::write(&path, vec![b' '; MAX_BOOTSTRAP_TERMINAL_BYTES + 1]).unwrap()
                }
                "symlink" => {
                    std::fs::remove_file(&path).unwrap();
                    std::os::unix::fs::symlink(f._temp.path().join("canonical.db"), &path).unwrap();
                }
                "unknown-top" => saved["unknown"] = serde_json::json!(true),
                "unknown-nested" => {
                    saved["completed"]["identity"]["scope"]["unknown"] = serde_json::json!(true)
                }
                "missing-optional" => {
                    saved["completed"]["attachment"]["handoff"]
                        .as_object_mut()
                        .unwrap()
                        .remove("thread");
                }
                "origin-unknown" => {
                    let (header, body) = original.split_once('\n').unwrap();
                    let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
                    header["unknown"] = serde_json::json!(true);
                    saved["original"] = serde_json::json!(format!("{}\n{body}", header));
                }
                "origin-oversized" => {
                    saved["original"] = serde_json::json!(format!(
                        "{original}{}",
                        " ".repeat(super::super::super::journal::MAX_BOOTSTRAP_ORIGIN_BYTES + 1)
                    ))
                }
                "report-digest" => {
                    saved["completed"]["retained"]["report_digest"] =
                        serde_json::json!("0".repeat(64))
                }
                "argv" => {
                    saved["completed"]["retained"]["report"]["argv"] =
                        serde_json::json!(["wrong-argv"]);
                    use sha2::{Digest, Sha256};
                    saved["completed"]["retained"]["report_digest"] = serde_json::json!(format!(
                        "{:x}",
                        Sha256::digest(
                            serde_json::to_vec(&saved["completed"]["retained"]["report"]).unwrap()
                        )
                    ));
                }
                "cross-kind" => {
                    std::fs::write(
                        f.journal.root().join(format!(
                            "delivery-{:020}-{}.terminal",
                            f.peer.reference.ordinal,
                            f.peer.reference.operation.as_str()
                        )),
                        &bytes,
                    )
                    .unwrap();
                }
                "cross-operation" => {
                    std::fs::write(
                        f.journal.root().join(format!(
                            "bootstrap-{:020}-{}.terminal",
                            f.peer.reference.ordinal,
                            uuid::Uuid::new_v4()
                        )),
                        &bytes,
                    )
                    .unwrap();
                }
                "intent-kind" => {
                    let (header, _) = original.split_once('\n').unwrap();
                    let mut header: super::super::super::journal::IntentHeader =
                        serde_json::from_str(header).unwrap();
                    let mutation = super::super::super::journal::SemanticMutation::freeze(
                        super::super::super::journal::SemanticMutation::SetTopic {
                            thread: ThreadId::new("canonical-thread"),
                            topic: "other".into(),
                        },
                        f.peer.identity.claim.clone(),
                    )
                    .unwrap();
                    use sha2::{Digest, Sha256};
                    header.kind = mutation.kind();
                    header.thread = mutation.thread().cloned();
                    header.semantic_digest = format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&mutation).unwrap())
                    );
                    std::fs::write(
                        &intent,
                        format!(
                            "{}\n{}",
                            serde_json::to_string(&header).unwrap(),
                            serde_json::to_string(&mutation).unwrap()
                        ),
                    )
                    .unwrap();
                }
                "duplicate-intent" => {
                    std::fs::write(&intent, &original).unwrap();
                    std::fs::write(
                        f.journal.root().join(format!(
                            "{:020}-{}.intent",
                            f.peer.reference.ordinal,
                            uuid::Uuid::new_v4()
                        )),
                        &original,
                    )
                    .unwrap();
                }
                "conflicting-intent" => {
                    let (header, body) = original.split_once('\n').unwrap();
                    let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
                    header["created_at_millis"] = serde_json::json!(999);
                    std::fs::write(&intent, format!("{}\n{body}", header)).unwrap();
                }
                _ => unreachable!(),
            }
            if matches!(
                damage,
                "unknown-top"
                    | "unknown-nested"
                    | "missing-optional"
                    | "origin-unknown"
                    | "origin-oversized"
                    | "report-digest"
                    | "argv"
            ) {
                std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
            }
            f.peer.state.lock().unwrap().calls.clear();
            let mut output = vec![];
            assert!(
                downstream(&f, &mut launcher, &mut output).is_err(),
                "{damage} accepted"
            );
            assert!(
                f.peer.state.lock().unwrap().calls.is_empty(),
                "{damage} reached client"
            );
            assert!(output.is_empty());
            assert_eq!(launcher.starts, 1);
        }
    }
    #[test]
    fn downstream_terminal_namespace_mismatch_refuses_even_after_cleanup() {
        for field in ["instance", "state", "host", "state-spelling"] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher::default();
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            let mut namespace = f.peer.identity.payload.handoff.namespace.clone();
            match field {
                "instance" => namespace.instance = "different".into(),
                "state" => namespace.state_dir = f._temp.path().join("different-state"),
                "host" => namespace.host_endpoint = f._temp.path().join("different-host"),
                "state-spelling" => {
                    namespace.state_dir =
                        std::path::PathBuf::from(format!("{}//", namespace.state_dir.display()))
                }
                _ => unreachable!(),
            }
            f.peer.state.lock().unwrap().calls.clear();
            let mut output = vec![];
            assert!(
                super::super::super::retry::run_bootstrap_retry_to_writer(
                    &f.journal,
                    &f.peer.reference,
                    super::super::super::actor_route::InvocationActor::Agent,
                    &namespace,
                    f.peer.as_ref(),
                    f.peer.as_ref(),
                    &mut launcher,
                    &f.clock,
                    BootstrapSubmissionInputs {
                        witness: &f.witness,
                        context: &f.context
                    },
                    &Default::default(),
                    &mut output
                )
                .is_err()
            );
            assert!(f.peer.state.lock().unwrap().calls.is_empty());
            assert!(output.is_empty());
        }
    }
    #[test]
    fn downstream_retained_terminal_requires_exact_canonical_completed_report() {
        for damage in [
            "missing-report",
            "report-corrupt",
            "report-mismatch",
            "nonterminal",
            "unavailable",
        ] {
            let f = Fixture::downstream(Fault::None);
            let mut launcher = DownstreamLauncher::default();
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            let mut state = f.peer.state.lock().unwrap();
            // Isolated database corruption fixtures deliberately bypass the
            // immutable SQL triggers; no production migration is changed.
            if damage != "unavailable" {
                state.db.execute_batch("DROP TRIGGER bootstrap_report_immutable; DROP TRIGGER bootstrap_report_retained; DROP TRIGGER bootstrap_terminal").unwrap();
            }
            match damage {
                "missing-report" => {
                    state
                        .db
                        .execute("DELETE FROM bootstrap_reports", [])
                        .unwrap();
                }
                "report-corrupt" => {
                    state
                        .db
                        .execute(
                            "UPDATE bootstrap_reports SET completed_json=?1",
                            [b"{broken".as_slice()],
                        )
                        .unwrap();
                }
                "report-mismatch" => {
                    let bytes: Vec<u8> = state
                        .db
                        .query_row("SELECT completed_json FROM bootstrap_reports", [], |r| {
                            r.get(0)
                        })
                        .unwrap();
                    let mut done: CompletedBootstrapResult =
                        serde_json::from_slice(&bytes).unwrap();
                    done.retained.report["unexpected-detail"] =
                        serde_json::json!("valid-different-history");
                    use sha2::{Digest, Sha256};
                    done.retained.report_digest = format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&done.retained.report).unwrap())
                    );
                    state
                        .db
                        .execute(
                            "UPDATE bootstrap_reports SET completed_json=?1",
                            [serde_json::to_vec(&done).unwrap()],
                        )
                        .unwrap();
                }
                "nonterminal" => {
                    state
                        .db
                        .execute(
                            "UPDATE bootstrap_handoffs SET state='attached',terminal_at=NULL",
                            [],
                        )
                        .unwrap();
                }
                "unavailable" => state.fault = Fault::StatusUnavailable,
                _ => unreachable!(),
            }
            state.calls.clear();
            drop(state);
            let mut output = vec![];
            assert!(
                downstream(&f, &mut launcher, &mut output).is_err(),
                "{damage} accepted"
            );
            assert!(
                f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .iter()
                    .all(|r| *r == "status")
            );
            assert!(output.is_empty());
            assert_eq!(launcher.starts, 1);
        }
    }
    #[test]
    fn downstream_partial_terminal_with_live_original_refuses_before_any_effect() {
        let f = Fixture::downstream(Fault::CompleteBefore);
        let mut launcher = DownstreamLauncher::default();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let path = terminal_path(&f.journal, &f.peer.reference);
        std::fs::write(&path, b"{partial").unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_child_progress_corruption_or_oversize_never_clears_launch_fence() {
        for damage in ["unknown", "identity", "oversized"] {
            let f = Fixture::downstream(Fault::CompleteBefore);
            let mut launcher = DownstreamLauncher::default();
            assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
            let path = child_progress_path(&f.journal, &f.peer.reference);
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            if damage == "unknown" {
                saved["progress"]["unknown"] = serde_json::json!(true);
            } else {
                saved["identity"]["digest"] = serde_json::json!("0".repeat(64));
            }
            std::fs::write(
                &path,
                if damage == "oversized" {
                    vec![b' '; MAX_LINKED_LOCAL_BYTES + 1]
                } else {
                    serde_json::to_vec(&saved).unwrap()
                },
            )
            .unwrap();
            f.peer.state.lock().unwrap().calls.clear();
            assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
            assert_eq!(launcher.starts, 1);
            assert!(
                !f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .contains(&"linked_complete")
            );
        }
    }
    #[test]
    fn downstream_retained_identity_alternate_path_spelling_is_corruption() {
        let f = Fixture::downstream(Fault::None);
        let mut launcher = DownstreamLauncher::default();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        let path = terminal_path(&f.journal, &f.peer.reference);
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let exact =
            saved["completed"]["identity"]["payload"]["handoff"]["namespace"]["host_endpoint"]
                .as_str()
                .unwrap()
                .to_owned();
        saved["completed"]["identity"]["payload"]["handoff"]["namespace"]["host_endpoint"] =
            serde_json::json!(format!("{exact}//"));
        std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        assert!(
            downstream(&f, &mut launcher, &mut vec![]).is_err(),
            "retained frozen path spelling changed without changing original digest"
        );
        assert!(
            f.peer.state.lock().unwrap().calls.is_empty(),
            "corrupt retained identity reached canonical client"
        );
    }
    #[test]
    fn downstream_delivery_terminal_with_live_bootstrap_origin_refuses_before_client() {
        let f = Fixture::downstream(Fault::CompleteBefore);
        let mut launcher = DownstreamLauncher::default();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        let foreign = f.journal.root().join(format!(
            "delivery-{:020}-{}.terminal",
            f.peer.reference.ordinal,
            f.peer.reference.operation.as_str()
        ));
        std::fs::write(foreign, b"{partial foreign kind").unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        let mut output = vec![];
        let result = downstream(&f, &mut launcher, &mut output);
        assert!(
            result.is_err(),
            "live bootstrap ignored another kind's retained original"
        );
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        assert!(output.is_empty());
    }
    fn unknown(result: Result<BootstrapResult, RunError>) {
        assert!(
            matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: ErrorCode::UnknownOutcome,
                    ..
                }))
            ),
            "expected creation uncertainty, got {result:?}"
        );
    }
    #[test]
    fn live_created_attaches_exact_recipient_without_downstream_effects() {
        let f = Fixture::new(Fault::None);
        let result = f.run();
        assert_eq!(
            f.peer.state.lock().unwrap().native_calls,
            1,
            "coordinator must submit exactly once"
        );
        let attached = result.expect("internal coordinator must reach exact attachment");
        assert_eq!(attached.state, BootstrapState::Attached);
        let a = attached.attachment.as_ref().unwrap();
        assert_eq!(a.created, attached.creation.clone().unwrap());
        assert_eq!(a.handoff.compound, f.peer.identity.payload.handoff_key);
        assert_eq!(a.handoff.claim, f.peer.identity.claim);
        assert_eq!(counts(&f), (1, 1, 1));
        assert_eq!(
            f.peer.state.lock().unwrap().calls,
            [
                "capabilities",
                "begin",
                "reserve",
                "check",
                "native",
                "record",
                "resolve",
                "attach"
            ]
        );
        let _ = f.run();
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn live_lost_reservation_reply_never_submits_on_retry() {
        let f = Fixture::new(Fault::ReserveReply);
        unknown(f.run());
        unknown(f.run());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
        assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
    }
    #[test]
    fn live_lost_host_reply_never_creates_twice() {
        let f = Fixture::new(Fault::HostReply);
        unknown(f.run());
        unknown(f.run());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        assert!(f.peer.status().creation.is_none());
    }
    #[test]
    fn live_only_typed_zero_submission_allocates_distinct_next_attempt() {
        let f = Fixture::new(Fault::NotSubmitted);
        let result = f
            .run()
            .expect("typed zero submission must close first attempt");
        assert_eq!(result.attempt.get(), 2);
        assert_eq!(result.state, BootstrapState::Prepared);
        let _ = f.run();
        assert_eq!(f.peer.status().attempt.get(), 2);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 2);
        let s = f.peer.state.lock().unwrap();
        let keys: Vec<String> =
            s.db.prepare("SELECT reserve_key FROM bootstrap_attempts ORDER BY attempt")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
        assert_ne!(keys[0], keys[1]);
    }
    #[test]
    fn live_fresh_authority_refusal_fences_submission() {
        let f = Fixture::new(Fault::Check);
        assert!(f.run().is_err());
        assert!(
            f.peer.state.lock().unwrap().calls.contains(&"check"),
            "fresh canonical check was never reached"
        );
        assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
        f.peer.state.lock().unwrap().fault = Fault::None;
        unknown(f.run());
    }
    #[test]
    fn live_record_reply_loss_recovers_canonical_creation_without_resubmit() {
        let f = Fixture::new(Fault::RecordReply);
        let _ = f.run();
        assert_eq!(
            f.peer.state.lock().unwrap().native_calls,
            1,
            "creation must occur before lost record reply recovery"
        );
        assert_eq!(f.peer.status().state, BootstrapState::Attached);
        let _ = f.run();
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn live_possible_save_failure_never_reaches_native() {
        let f = Fixture::new(Fault::SavePossible);
        assert!(f.run().is_err());
        assert!(
            f.peer.state.lock().unwrap().calls.contains(&"reserve"),
            "reservation must precede save failure"
        );
        assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
    }
    #[test]
    fn live_created_save_failure_is_unknown_without_record_or_resubmit() {
        let f = Fixture::new(Fault::SaveCreated);
        unknown(f.run());
        assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        std::fs::remove_dir(f.peer.progress()).unwrap();
        unknown(f.run());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn live_canonical_evidence_recovers_missing_progress_across_two_retries() {
        let f = Fixture::new(Fault::None);
        let _ = f.run();
        std::fs::remove_file(f.peer.progress()).unwrap();
        for _ in 0..2 {
            let result = f.run();
            assert_eq!(result.unwrap().state, BootstrapState::Attached);
        }
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn live_possible_save_failure_reports_exact_uncertainty() {
        let f = Fixture::new(Fault::SavePossible);
        unknown(f.run());
        std::fs::remove_dir(f.peer.progress()).unwrap();
        unknown(f.run());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
    }
    #[test]
    fn live_owned_child() {
        let Ok(path) = std::env::var("HT_QHZ9_CHILD_INPUT") else {
            return;
        };
        let input: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let root = std::path::PathBuf::from(input["root"].as_str().unwrap());
        let identity: BootstrapIdentity =
            serde_json::from_value(input["identity"].clone()).unwrap();
        let reference = serde_json::from_value(input["reference"].clone()).unwrap();
        let journal_root = std::path::PathBuf::from(input["journal"].as_str().unwrap());
        let journal = super::super::super::journal::Journal::open(journal_root.clone()).unwrap();
        let db = rusqlite::Connection::open(root.join("canonical.db")).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
        let peer = Peer {
            state: Mutex::new(State {
                db,
                calls: vec![],
                native_calls: 0,
                fault: Fault::HostReply,
            }),
            identity,
            journal: journal_root,
            database_path: root.join("canonical.db"),
            reference,
            native_marker: Some(root.join("native-calls")),
            block_native: input["block"].as_bool().unwrap(),
        };
        let clock = crate::app::SystemClock::new();
        let context = HostCallContext {
            budget: super::super::super::cooperative_budget(&clock),
            expected_boot: Some(
                crate::protocol::handoff::topology_contract_tests::created().host_incarnation,
            ),
            expected_epoch: Some(1),
        };
        let mut witness = crate::protocol::handoff::topology_contract_tests::created().witness;
        witness.endpoint = peer
            .identity
            .payload
            .handoff
            .namespace
            .host_endpoint
            .clone();
        let result = resume_to_attachment(
            &journal,
            &peer.reference,
            &peer.identity.payload.handoff.namespace,
            &peer,
            &peer,
            &clock,
            BootstrapSubmissionInputs {
                witness: &witness,
                context: &context,
            },
        );
        if input["expect_lock"].as_bool().unwrap() {
            assert!(
                matches!(result, Err(RunError::Io(_))),
                "expected operation lock refusal, got {result:?}"
            );
            assert_eq!(peer.state.lock().unwrap().calls, ["capabilities"]);
        } else {
            unknown(result);
        }
    }
    fn child(
        f: &Fixture,
        journal: &std::path::Path,
        block: bool,
        expect_lock: bool,
        suffix: &str,
    ) -> crate::test_support::spawn::OwnedChild {
        use crate::test_support::spawn::SpawnOwned;
        let input = f._temp.path().join(format!("child-{suffix}.json"));
        std::fs::write(&input,serde_json::to_vec(&serde_json::json!({"root":f._temp.path(),"identity":f.peer.identity,"reference":f.peer.reference,"journal":journal,"block":block,"expect_lock":expect_lock})).unwrap()).unwrap();
        let home = f._temp.path().join(format!("home-{suffix}"));
        std::fs::create_dir_all(home.join("codex")).unwrap();
        std::fs::create_dir_all(home.join("claude")).unwrap();
        let mut command = crate::test_support::spawn::command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cli::topology_handoff::tests::live::live_owned_child",
                "--nocapture",
            ])
            .env("HT_QHZ9_CHILD_INPUT", input)
            .env("HOME", &home)
            .env("CODEX_HOME", home.join("codex"))
            .env("CLAUDE_CONFIG_DIR", home.join("claude"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.spawn_owned().unwrap()
    }
    fn duplicate_processes(copied: bool, crash: bool) {
        use std::io::{BufRead, Write};
        let f = Fixture::new(Fault::None);
        let mut first = child(&f, f.journal.root(), true, false, "first");
        let mut reader = std::io::BufReader::new(first.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).unwrap() > 0,
                "owned child ended before native boundary"
            );
            if line.contains("QHZ9_NATIVE_READY") {
                break;
            }
        }
        let second_root = if copied {
            let root = f._temp.path().join("copied-intents");
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            for entry in std::fs::read_dir(f.journal.root()).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() {
                    std::fs::copy(&path, root.join(path.file_name().unwrap())).unwrap();
                }
            }
            root
        } else {
            f.journal.root().to_path_buf()
        };
        let second = child(&f, &second_root, false, !copied, "second")
            .wait_with_output()
            .unwrap();
        assert!(
            second.status.success(),
            "duplicate child failed: {}",
            String::from_utf8_lossy(&second.stderr)
        );
        if crash {
            assert!(first.stop().is_some());
        } else {
            first.stdin.as_mut().unwrap().write_all(&[1]).unwrap();
            let output = first.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "original child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let retry = child(&f, &second_root, false, false, "retry")
            .wait_with_output()
            .unwrap();
        assert!(
            retry.status.success(),
            "post-child retry failed: {}",
            String::from_utf8_lossy(&retry.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(f._temp.path().join("native-calls"))
                .unwrap()
                .lines()
                .count(),
            1,
            "a second native submission escaped local/canonical exclusion"
        );
        assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
    }
    #[test]
    fn live_owned_process_duplicate_shared_journal_lock_no_second_submission() {
        duplicate_processes(false, false);
    }
    #[test]
    fn live_owned_process_copied_journal_canonical_uniqueness() {
        duplicate_processes(true, false);
    }
    #[test]
    fn live_owned_process_crash_during_write_no_second_submission() {
        duplicate_processes(true, true);
    }
    #[test]
    fn live_missing_guard_capability_refuses_before_any_mutation() {
        for fault in [
            Fault::MissingCapability,
            Fault::OldCapability,
            Fault::WrongCapability,
        ] {
            let f = Fixture::new(fault);
            let result = f.run();
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: ErrorCode::Unsupported,
                    ..
                }))
            ));
            let s = f.peer.state.lock().unwrap();
            assert_eq!(s.native_calls, 0, "capability absence must fence creation");
            assert_eq!(
                s.db.query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(s.calls, ["capabilities"]);
            assert!(!f.peer.progress().exists());
        }
    }
    #[test]
    fn live_attachment_reply_loss_recovers_identical_child_without_resubmit() {
        let f = Fixture::new(Fault::AttachReply);
        let first = f
            .run()
            .expect("canonical attachment must survive reply loss");
        let second = f.run().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.state, BootstrapState::Attached);
        assert_eq!(counts(&f), (1, 1, 1));
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    #[test]
    fn live_moved_scope_at_resolve_or_attach_retains_exact_creation() {
        for fault in [Fault::MovedResolveTab, Fault::MovedAttachTab] {
            let f = Fixture::new(fault);
            assert!(f.run().is_err());
            assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(f.peer.status().state, BootstrapState::Created);
            assert!(f.peer.status().attachment.is_none());
            assert_eq!(
                counts(&f),
                if fault == Fault::MovedResolveTab {
                    (0, 0, 0)
                } else {
                    (1, 1, 1)
                }
            );
            f.peer.state.lock().unwrap().fault = Fault::None;
            assert_eq!(f.run().unwrap().state, BootstrapState::Attached);
            assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        }
    }
    fn actor_run(
        f: &Fixture,
        reference: &super::super::super::journal::IntentRef,
    ) -> Result<BootstrapResult, RunError> {
        super::super::super::retry::run_bootstrap_retry(
            &f.journal,
            reference,
            super::super::super::actor_route::InvocationActor::Agent,
            &f.peer.identity.payload.handoff.namespace,
            f.peer.as_ref(),
            f.peer.as_ref(),
            &f.clock,
            BootstrapSubmissionInputs {
                witness: &f.witness,
                context: &f.context,
            },
        )
    }
    #[test]
    fn live_actor_wrapper_human_origin_refuses_before_client_lock_or_progress() {
        let f = Fixture::new(Fault::MissingCapability);
        let mut identity = f.peer.identity.clone();
        identity.claim.harness = crate::protocol::authority::Harness::Human;
        identity.digest = identity.semantic_digest().unwrap();
        let reference = publish(&f.journal, &identity, 2).unwrap();
        let result = actor_run(&f, &reference);
        assert!(
            format!("{result:?}")
                .contains("person/operator retry requires immediate human namespace"),
            "{result:?}"
        );
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        assert!(!f.peer.progress().exists());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
    }
    #[test]
    fn live_actor_wrapper_invalid_missing_ambiguous_origins_refuse_readonly() {
        for damage in ["malformed", "missing", "ambiguous"] {
            let f = Fixture::new(Fault::MissingCapability);
            let path = std::fs::read_dir(f.journal.root())
                .unwrap()
                .map(Result::unwrap)
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "intent"))
                .unwrap();
            match damage {
                "malformed" => std::fs::write(&path, b"{broken").unwrap(),
                "missing" => std::fs::remove_file(&path).unwrap(),
                "ambiguous" => {
                    let second = f.journal.root().join(format!(
                        "{:020}-{}.intent",
                        f.peer.reference.ordinal,
                        uuid::Uuid::new_v4()
                    ));
                    std::fs::copy(&path, second).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(actor_run(&f, &f.peer.reference).is_err());
            assert!(
                f.peer.state.lock().unwrap().calls.is_empty(),
                "{damage} origin reached client"
            );
            assert!(!f.peer.progress().exists());
        }
    }
    #[test]
    fn live_actor_wrapper_agent_original_reaches_guarded_attachment() {
        let f = Fixture::new(Fault::None);
        assert_eq!(
            actor_run(&f, &f.peer.reference).unwrap().state,
            BootstrapState::Attached
        );
    }
    fn scoped_peer_current(
        peer: &Peer,
    ) -> (
        crate::store::connection::StoreContext,
        crate::ports::HostObservation,
        crate::ports::HostObservationAdmission,
        CallBudget,
    ) {
        use crate::ports::*;
        let context = crate::store::connection::StoreContext::new(
            peer.database_path.clone(),
            Arc::new(crate::app::SystemClock::new()),
        );
        let budget = super::super::super::cooperative_budget(context.clock());
        let mut s = peer.state.lock().unwrap();
        let admission =
            crate::store::seats::begin_host_observation(&context, &mut s.db, "i", &budget).unwrap();
        let observed_at_mono = context.clock().monotonic_now();
        let observation = HostObservation {
            focused: false,
            target: HostTargetId::new("w1:p2"),
            host_boot: HostBootId::new("herdr-server:pid=42:start=1.000002:uid=501"),
            epoch: 1,
            generation: 1,
            observed_at_utc: context.clock().utc_now(),
            observed_at_mono,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            terminal: Some(TerminalId::new("terminal")),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: "structural-incarnation".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(uuid::Uuid::new_v4().to_string()),
            connection_epoch: 1,
            observation_sequence: admission.sequence() + 2,
            started_at_mono: observed_at_mono,
            completed_at_mono: observed_at_mono,
        };
        assert!(
            crate::store::seats::publish_current_target_observation(
                &context,
                &mut s.db,
                &admission,
                &observation,
                &budget
            )
            .unwrap()
        );
        (context, observation, admission, budget)
    }
    fn scoped_peer_guard(
        peer: &Peer,
        tab: &str,
    ) -> (
        crate::store::connection::StoreContext,
        crate::ports::BootstrapAttachmentGuard,
        CallBudget,
    ) {
        use crate::ports::*;
        let (context, observation, admission, budget) = scoped_peer_current(peer);
        let guard = BootstrapAttachmentGuard::try_new(
            &crate::protocol::commands::ResolveSeat {
                target: observation.target.clone(),
                operation: peer.identity.payload.resolve_key.clone(),
            },
            BootstrapPaneObservation::try_new(
                observation,
                HostTargetId::new("w1"),
                HostTargetId::new(tab),
            )
            .unwrap(),
            &admission,
        )
        .unwrap();
        (context, guard, budget)
    }
    fn scoped_current(
        f: &Fixture,
    ) -> (
        crate::store::connection::StoreContext,
        crate::ports::HostObservation,
        crate::ports::HostObservationAdmission,
        CallBudget,
    ) {
        scoped_peer_current(&f.peer)
    }
    fn scoped_guard(
        f: &Fixture,
        tab: &str,
    ) -> (
        crate::store::connection::StoreContext,
        crate::ports::BootstrapAttachmentGuard,
        CallBudget,
    ) {
        scoped_peer_guard(&f.peer, tab)
    }
    fn resolve_request(f: &Fixture) -> ResolveBootstrapSeat {
        ResolveBootstrapSeat {
            identity: f.peer.identity.clone(),
            expected_attempt: BootstrapAttempt::first(),
            operation: f.peer.identity.payload.resolve_key.clone(),
        }
    }
    fn counts(f: &Fixture) -> (i64, i64, i64) {
        let s = f.peer.state.lock().unwrap();
        s.db.query_row("SELECT (SELECT count(*) FROM seats WHERE target_id='w1:p2'),(SELECT count(*) FROM allocation_decisions WHERE target_id='w1:p2'),(SELECT count(*) FROM operations WHERE operation_key=?1)",[f.peer.identity.payload.resolve_key.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
    fn resolve(
        f: &Fixture,
        request: &ResolveBootstrapSeat,
        context: &crate::store::connection::StoreContext,
        guard: &crate::ports::BootstrapAttachmentGuard,
        budget: &CallBudget,
    ) -> Result<SeatId, ApiError> {
        crate::store::seats::resolve_bootstrap_seat(
            context,
            &mut f.peer.state.lock().unwrap().db,
            &f.peer.identity.payload.handoff.namespace,
            request,
            guard,
            budget,
        )
    }
    #[test]
    fn live_preallocation_moved_tab_refuses_with_zero_seats_and_operations() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t9");
        let result = resolve(&f, &resolve_request(&f), &ctx, &guard, &budget);
        assert!(result.is_err(), "moved tab allocated a seat: {result:?}");
        assert_eq!(counts(&f), (0, 0, 0));
        assert_eq!(f.peer.status().state, BootstrapState::Created);
    }
    #[test]
    fn live_preallocation_same_response_allocates_once_and_keeps_old_digest() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        let recipient = resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).unwrap();
        assert_eq!(counts(&f), (1, 1, 1));
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        assert_eq!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).unwrap(),
            recipient
        );
        assert_eq!(counts(&f), (1, 1, 1));
        let s = f.peer.state.lock().unwrap();
        let digest: Vec<u8> =
            s.db.query_row(
                "SELECT digest FROM operations WHERE operation_key=?1",
                [f.peer.identity.payload.resolve_key.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            digest,
            schema::canonical_digest(&("resolve_seat", "i", HostTargetId::new("w1:p2"))).unwrap()
        );
    }
    #[test]
    fn live_preallocation_stale_attempt_refuses_before_allocation() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        let mut r = resolve_request(&f);
        r.expected_attempt = BootstrapAttempt::new(2).unwrap();
        assert!(
            resolve(&f, &r, &ctx, &guard, &budget).is_err(),
            "stale attempt was allowed to resolve"
        );
        assert_eq!(counts(&f), (0, 0, 0));
    }
    fn preexisting_owner(f: &Fixture) -> SeatId {
        use crate::ports::{
            OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome,
        };
        let (ctx, observation, admission, budget) = scoped_current(f);
        let request = crate::protocol::commands::ResolveSeat {
            target: HostTargetId::new("w1:p2"),
            operation: OperationId::new("preexisting-fixture"),
        };
        let guard = OrdinaryResolutionGuard::try_new(&request, observation, &admission).unwrap();
        let OrdinaryResolutionOutcome::Resolved(seat) = crate::store::seats::resolve_seat(
            &ctx,
            &mut f.peer.state.lock().unwrap().db,
            "i",
            request,
            OrdinaryResolutionAttempt::Observed(guard),
            &budget,
        )
        .unwrap() else {
            panic!("ordinary allocation missing")
        };
        seat
    }
    #[test]
    fn live_preallocation_existing_owner_positive_uses_no_allocation_bump() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let owner = preexisting_owner(&f);
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        assert_eq!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).unwrap(),
            owner
        );
        assert_eq!(counts(&f), (1, 1, 1));
    }
    #[test]
    fn live_preallocation_lifecycle_only_change_refuses_existing_owner_fresh_key() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let _ = preexisting_owner(&f);
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        f.peer
            .state
            .lock()
            .unwrap()
            .db
            .execute(
                "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'",
                [],
            )
            .unwrap();
        assert!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err(),
            "owner bypassed lifecycle invalidation"
        );
        assert_eq!(counts(&f), (1, 1, 0));
    }
    #[test]
    fn live_preallocation_lifecycle_only_change_refuses_exact_replay_and_preserves_row() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        let _ = resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).unwrap();
        let before: (Vec<u8>, String) = f
            .peer
            .state
            .lock()
            .unwrap()
            .db
            .query_row(
                "SELECT digest,result_json FROM operations WHERE operation_key=?1",
                [f.peer.identity.payload.resolve_key.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        f.peer
            .state
            .lock()
            .unwrap()
            .db
            .execute(
                "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'",
                [],
            )
            .unwrap();
        assert!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err(),
            "historical result bypassed lifecycle invalidation"
        );
        let after: (Vec<u8>, String) = f
            .peer
            .state
            .lock()
            .unwrap()
            .db
            .query_row(
                "SELECT digest,result_json FROM operations WHERE operation_key=?1",
                [f.peer.identity.payload.resolve_key.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(counts(&f), (1, 1, 1));
    }
    #[test]
    fn live_preallocation_own_extra_lifecycle_bump_rolls_back_allocation() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        f.peer.state.lock().unwrap().db.execute_batch("CREATE TEMP TRIGGER extra_lifecycle AFTER INSERT ON allocation_decisions BEGIN UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'; END;").unwrap();
        assert!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err(),
            "unexpected own-transaction lifecycle bump committed"
        );
        assert_eq!(counts(&f), (0, 0, 0));
    }
    #[test]
    fn live_preallocation_a2_namespace_claim_and_race_controls_zero_effect() {
        for change in [
            "membership",
            "archive",
            "binding",
            "namespace",
            "claim",
            "key",
            "hold",
            "admission",
            "incarnation",
            "owner",
            "unresolved",
            "retired",
        ] {
            let f = Fixture::new(Fault::ResolveUnsupported);
            let _ = f.run();
            if change == "owner" {
                let _ = preexisting_owner(&f);
            }
            let before = counts(&f);
            let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
            let mut request = resolve_request(&f);
            let mut namespace = f.peer.identity.payload.handoff.namespace.clone();
            let mut state = f.peer.state.lock().unwrap();
            match change {
                "membership" => {
                    state
                        .db
                        .execute("DELETE FROM memberships WHERE seat_id='sender'", [])
                        .unwrap();
                }
                "archive" => {
                    state
                        .db
                        .execute(
                            "UPDATE threads SET archived=1 WHERE id='canonical-thread'",
                            [],
                        )
                        .unwrap();
                }
                "binding" => {
                    state.db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-000000000002' WHERE seat_id='sender'", []).unwrap();
                }
                "namespace" => namespace.host_endpoint = f._temp.path().join("different.sock"),
                "claim" => {
                    request.identity.claim.execution =
                        ExecutionId::new("00000000-0000-4000-8000-000000000002");
                    request.identity.digest = request.identity.semantic_digest().unwrap();
                }
                "key" => request.operation = OperationId::new("wrong-resolve"),
                "hold" => {
                    state.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','w1:p2',?1,1,'test hold')", ["herdr-server:pid=42:start=1.000002:uid=501"]).unwrap();
                }
                "admission" => {
                    crate::store::seats::begin_host_observation(&ctx, &mut state.db, "i", &budget)
                        .unwrap();
                }
                "incarnation" => {
                    state.db.execute("UPDATE observed_targets SET incarnation='different-incarnation' WHERE target_id='w1:p2'", []).unwrap();
                }
                "unresolved" => {
                    state.db.execute("UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id='sender'", []).unwrap();
                }
                "retired" => {
                    state.db.execute("UPDATE seats SET state='retired',retired_at=1,retired_seq=1 WHERE id='sender'", []).unwrap();
                }
                "owner" => {
                    state.db.execute("UPDATE seats SET structural_terminal_id='different-terminal' WHERE target_id='w1:p2'", []).unwrap();
                }
                _ => unreachable!(),
            }
            let result = crate::store::seats::resolve_bootstrap_seat(
                &ctx,
                &mut state.db,
                &namespace,
                &request,
                &guard,
                &budget,
            );
            drop(state);
            assert!(result.is_err(), "{change} guard accepted: {result:?}");
            assert_eq!(counts(&f), before, "{change} guard mutated seats/ledger");
        }
    }
    #[test]
    fn live_preallocation_same_response_wrong_workspace_or_terminal_refuses() {
        use crate::ports::{BootstrapAttachmentGuard, BootstrapPaneObservation};
        for change in ["workspace", "terminal"] {
            let f = Fixture::new(Fault::ResolveUnsupported);
            let _ = f.run();
            let (ctx, mut observation, admission, budget) = scoped_current(&f);
            if change == "terminal" {
                observation.terminal = Some(TerminalId::new("different-terminal"));
            }
            let response = BootstrapPaneObservation::try_new(
                observation,
                HostTargetId::new(if change == "workspace" { "w9" } else { "w1" }),
                HostTargetId::new("w1:t2"),
            );
            if change == "workspace" {
                assert!(response.is_err());
                assert_eq!(counts(&f), (0, 0, 0));
                continue;
            }
            let response = response.unwrap();
            let guard = BootstrapAttachmentGuard::try_new(
                &crate::protocol::commands::ResolveSeat {
                    target: HostTargetId::new("w1:p2"),
                    operation: f.peer.identity.payload.resolve_key.clone(),
                },
                response,
                &admission,
            )
            .unwrap();
            assert!(resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err());
            assert_eq!(counts(&f), (0, 0, 0));
        }
    }
    #[test]
    fn live_preallocation_operation_insert_failure_rolls_back_allocation() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        f.peer.state.lock().unwrap().db.execute_batch("CREATE TEMP TRIGGER reject_resolution BEFORE INSERT ON operations WHEN NEW.actor_scope='service-allocation:i' BEGIN SELECT RAISE(ABORT,'injected late ledger failure'); END;").unwrap();
        assert!(resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err());
        assert_eq!(counts(&f), (0, 0, 0));
    }
    #[test]
    fn live_preallocation_budget_entry_and_final_presentation_zero_effect() {
        use crate::protocol::time::{Cancellation, Clock, MonoInstant};
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct EdgeClock {
            calls: AtomicUsize,
            trip: usize,
            cancel: Option<Cancellation>,
        }
        impl Clock for EdgeClock {
            fn utc_now(&self) -> UtcMillis {
                UtcMillis(1)
            }
            fn monotonic_now(&self) -> MonoInstant {
                if self.calls.fetch_add(1, Ordering::SeqCst) + 1 >= self.trip {
                    if let Some(cancel) = &self.cancel {
                        cancel.cancel();
                        MonoInstant(1)
                    } else {
                        MonoInstant(10)
                    }
                } else {
                    MonoInstant(1)
                }
            }
        }
        for (trip, cancelled) in [(1, false), (5, false), (1, true), (4, true)] {
            let f = Fixture::new(Fault::ResolveUnsupported);
            let _ = f.run();
            let (_, guard, _) = scoped_guard(&f, "w1:t2");
            let cancel = Cancellation::default();
            if trip == 1 && cancelled {
                cancel.cancel();
            }
            let ctx = crate::store::connection::StoreContext::new(
                f._temp.path().join("canonical.db"),
                Arc::new(EdgeClock {
                    calls: AtomicUsize::new(0),
                    trip,
                    cancel: cancelled.then(|| cancel.clone()),
                }),
            );
            let budget = CallBudget {
                deadline: MonoInstant(10),
                cancellation: cancel,
            };
            let result = resolve(&f, &resolve_request(&f), &ctx, &guard, &budget);
            assert!(
                result.is_err(),
                "budget ({trip},{cancelled}) accepted {result:?}"
            );
            assert_eq!(counts(&f), (0, 0, 0));
        }
    }
    #[test]
    fn live_preallocation_integer_ceiling_allocating_and_owner_phases() {
        for existing in [false, true] {
            let f = Fixture::new(Fault::ResolveUnsupported);
            let _ = f.run();
            let owner = existing.then(|| preexisting_owner(&f));
            f.peer
                .state
                .lock()
                .unwrap()
                .db
                .execute(
                    "UPDATE host_instances SET lifecycle_revision=?1 WHERE id='i'",
                    [i64::MAX - 1],
                )
                .unwrap();
            let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
            let before = counts(&f);
            let result = resolve(&f, &resolve_request(&f), &ctx, &guard, &budget);
            if let Some(owner) = owner {
                assert_eq!(result.unwrap(), owner);
                assert_eq!(counts(&f), (1, 1, 1));
            } else {
                assert!(result.is_err());
                assert_eq!(counts(&f), before);
            }
        }
    }
    #[test]
    fn live_progress_nested_unknown_fields_refuse_without_new_effects() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let mut progress: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.peer.progress()).unwrap()).unwrap();
        progress["request"]["unknown-option"] = serde_json::json!("must refuse");
        std::fs::write(f.peer.progress(), serde_json::to_vec(&progress).unwrap()).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        assert!(f.run().is_err());
        assert_eq!(f.peer.state.lock().unwrap().calls, ["capabilities"]);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        assert_eq!(counts(&f), (0, 0, 0));
    }
    #[test]
    fn live_attached_plan_exact_frozen_options_and_digest_tampering_refusal() {
        let f = Fixture::new(Fault::None);
        let status = f.run().unwrap();
        let mut attachment = status.attachment.unwrap();
        let plan = attached_handoff_plan(&f.peer.identity, &attachment).unwrap();
        assert_eq!(
            plan.request.launch.argv,
            f.peer.identity.payload.launch.argv
        );
        assert_eq!(plan.request.launch.target, attachment.created.root_pane);
        assert_eq!(
            plan.context.state_dir.as_deref(),
            f.peer.identity.payload.handoff.namespace.state_dir.to_str()
        );
        assert_eq!(plan.request.body, f.peer.identity.payload.handoff.body);
        attachment.handoff.digest = "0".repeat(64);
        assert!(attached_handoff_plan(&f.peer.identity, &attachment).is_err());
    }
    #[test]
    fn live_uncorrelated_creation_stays_unknown_without_record_or_retry() {
        let f = Fixture::new(Fault::BadCorrelation);
        unknown(f.run());
        unknown(f.run());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        assert!(!f.peer.state.lock().unwrap().calls.contains(&"record"));
        assert!(f.peer.status().creation.is_none());
    }
    #[test]
    fn live_preallocation_replay_rechecks_original_authority_preserving_ledger() {
        for change in ["archive", "binding"] {
            let f = Fixture::new(Fault::ResolveUnsupported);
            let _ = f.run();
            let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).unwrap();
            let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
            let mut s = f.peer.state.lock().unwrap();
            let before: (Vec<u8>, String) =
                s.db.query_row(
                    "SELECT digest,result_json FROM operations WHERE operation_key=?1",
                    [f.peer.identity.payload.resolve_key.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            if change == "archive" {
                s.db.execute(
                    "UPDATE threads SET archived=1 WHERE id='canonical-thread'",
                    [],
                )
                .unwrap();
            } else {
                s.db.execute("UPDATE occupant_bindings SET execution_id='00000000-0000-4000-8000-000000000002' WHERE seat_id='sender'",[]).unwrap();
            }
            let result = crate::store::seats::resolve_bootstrap_seat(
                &ctx,
                &mut s.db,
                &f.peer.identity.payload.handoff.namespace,
                &resolve_request(&f),
                &guard,
                &budget,
            );
            assert!(result.is_err());
            let after: (Vec<u8>, String) =
                s.db.query_row(
                    "SELECT digest,result_json FROM operations WHERE operation_key=?1",
                    [f.peer.identity.payload.resolve_key.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(before, after);
            drop(s);
            assert_eq!(counts(&f), (1, 1, 1));
        }
    }
    #[test]
    fn live_preallocation_actual_next_attempt_rejects_previous_attempt() {
        let f = Fixture::new(Fault::NotSubmitted);
        assert_eq!(f.run().unwrap().attempt.get(), 2);
        f.peer.state.lock().unwrap().fault = Fault::ResolveUnsupported;
        let _ = f.run();
        assert_eq!(f.peer.status().attempt.get(), 2);
        assert_eq!(f.peer.status().state, BootstrapState::Created);
        let (ctx, guard, budget) = scoped_guard(&f, "w1:t2");
        assert!(resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err());
        assert_eq!(counts(&f), (0, 0, 0));
    }
    #[test]
    fn live_fix1_no_effect_without_saved_request_refuses_uncertain_attempt_readonly() {
        let f = Fixture::new(Fault::HostReply);
        unknown(f.run());
        let before = f.peer.status();
        assert_eq!(before.state, BootstrapState::PossibleCreation);
        assert_eq!(before.attempt.get(), 1);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        let mut progress: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.peer.progress()).unwrap()).unwrap();
        assert!(progress["request"].is_object());
        progress["request"] = serde_json::Value::Null;
        progress["not_submitted"] = serde_json::Value::Bool(true);
        std::fs::write(f.peer.progress(), serde_json::to_vec(&progress).unwrap()).unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        let result = f.run();
        assert_eq!(
            f.peer.status(),
            before,
            "malformed local phase advanced the canonical uncertain attempt: {result:?}"
        );
        assert!(matches!(
            result,
            Err(RunError::Api(ApiError {
                code: ErrorCode::InvalidRequest,
                ..
            }))
        ));
        let s = f.peer.state.lock().unwrap();
        assert_eq!(
            s.calls,
            ["capabilities"],
            "malformed phase must refuse before Begin and no-effect recording"
        );
        assert_eq!(s.native_calls, 1);
    }
    #[test]
    fn live_fix1_genuine_typed_zero_saved_request_replays_no_effect_record() {
        let f = Fixture::new(Fault::NotSubmittedRecordUnavailable);
        assert!(f.run().is_err());
        let before = f.peer.status();
        assert_eq!(before.state, BootstrapState::PossibleCreation);
        assert_eq!(before.attempt.get(), 1);
        let progress: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.peer.progress()).unwrap()).unwrap();
        assert_eq!(progress["not_submitted"], true);
        assert!(
            progress["request"].is_object(),
            "typed outcome must retain its actual saved request"
        );
        f.peer.state.lock().unwrap().calls.clear();
        let result = f.run().unwrap();
        assert_eq!(result.state, BootstrapState::Prepared);
        assert_eq!(result.attempt.get(), 2);
        let s = f.peer.state.lock().unwrap();
        assert_eq!(s.calls, ["capabilities", "begin", "not_submitted"]);
        assert_eq!(s.native_calls, 1);
    }
    #[test]
    fn live_fix1_canonical_created_without_local_request_still_attaches() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        assert!(f.run().is_err());
        assert_eq!(f.peer.status().state, BootstrapState::Created);
        let mut progress: serde_json::Value =
            serde_json::from_slice(&std::fs::read(f.peer.progress()).unwrap()).unwrap();
        progress["request"] = serde_json::Value::Null;
        assert!(progress["creation"].is_object());
        assert_eq!(progress["not_submitted"], false);
        std::fs::write(f.peer.progress(), serde_json::to_vec(&progress).unwrap()).unwrap();
        let mut s = f.peer.state.lock().unwrap();
        s.fault = Fault::None;
        s.calls.clear();
        drop(s);
        let result = f.run().unwrap();
        assert_eq!(result.state, BootstrapState::Attached);
        let s = f.peer.state.lock().unwrap();
        assert_eq!(s.calls, ["capabilities", "begin", "resolve", "attach"]);
        assert_eq!(s.native_calls, 1);
        drop(s);
        assert_eq!(result.creation, f.peer.status().creation);
    }
}
