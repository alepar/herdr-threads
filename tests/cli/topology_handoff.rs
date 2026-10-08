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
