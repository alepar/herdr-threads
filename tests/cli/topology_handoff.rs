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
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
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
                _ => panic!("unapproved live route {command:?}"),
            };
            s.calls.push(role);
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
                        .map(|v| CommandResult::Bootstrap(Box::new(v.unwrap()))),
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
            if s.fault == Fault::NotSubmitted {
                s.fault = Fault::None;
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
            let temp = Temp::new();
            let mut identity = prepared(&request(), temp.path()).unwrap();
            identity.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
            identity.digest = identity.semantic_digest().unwrap();
            let journal =
                super::super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
            let reference = publish(&journal, &identity, 1).unwrap();
            let db = rusqlite::Connection::open(temp.path().join("canonical.db")).unwrap();
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
                    journal: temp.path().join("intents"),
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
            peer.journal.parent().unwrap().join("canonical.db"),
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
}
