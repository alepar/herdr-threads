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
            Command::Capabilities => Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: crate::protocol::capabilities::ADVERTISED
                        .iter()
                        .copied()
                        .chain([
                            crate::protocol::capabilities::HANDOFF_LAUNCH_DIRECTORIES_V1,
                            crate::protocol::capabilities::INVITATION_CONFIRMATION_V1,
                            crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1,
                        ])
                        .map(str::to_owned)
                        .collect(),
                },
            )),
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
        summary: None,
        policy: None,
        automatic: false,
        new_tab: Some("same label".into()),
        existing: false,
        selector: PaneSelector::default(),
        seat: None,
        cwd: None,
        profile_dir: None,
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

// Missing normalization would retain the symlink (and follow a later retarget on retry).
#[test]
fn parser_launch_directories_canonicalize_explicit_paths() {
    let temp = Temp::new();
    let target = temp.path().join("profile ' \" space");
    std::fs::create_dir(&target).unwrap();
    let link = temp.path().join("selected");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let argv: Vec<std::ffi::OsString> = vec![
        "ht".into(),
        "handoff".into(),
        "--new-tab".into(),
        "peer".into(),
        "--kind".into(),
        "codex".into(),
        "--profile-dir".into(),
        link.clone().into_os_string(),
        "--cwd".into(),
        link.into_os_string(),
        "--agent-arg=--profile".into(),
        "--agent-arg=native-name".into(),
        "--".into(),
        "work".into(),
    ];
    let parsed = crate::cli::commands::parse_argv(argv)
        .expect("new-tab accepts an explicit profile directory");
    let crate::cli::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("new-tab route");
    };
    assert_eq!(request.cwd, Some(target.canonicalize().unwrap()));
    assert_eq!(request.profile_dir, Some(target.canonicalize().unwrap()));
    assert_eq!(request.argv, ["--profile", "native-name"]);
}

// A parser success would allow invalid explicit cwd to reach recipient resolution.
#[test]
fn parser_launch_directories_reject_invalid_paths_before_dispatch() {
    use std::os::unix::ffi::OsStringExt;
    let temp = Temp::new();
    let file = temp.path().join("file");
    std::fs::write(&file, "x").unwrap();
    let controls = temp.path().join("has\nnewline");
    std::fs::create_dir(&controls).unwrap();
    let control_link = temp.path().join("looks-valid");
    std::os::unix::fs::symlink(&controls, &control_link).unwrap();
    let non_utf8 = temp.path().join(std::ffi::OsString::from_vec(vec![255]));
    for path in [
        file,
        temp.path().join("missing"),
        "relative".into(),
        controls,
        control_link,
        non_utf8,
        format!("/{}", "x".repeat(4096)).into(),
    ] {
        for option in ["--cwd", "--profile-dir"] {
            let argv: Vec<std::ffi::OsString> = vec![
                "ht".into(),
                "handoff".into(),
                "--new-tab".into(),
                "peer".into(),
                "--kind".into(),
                "codex".into(),
                option.into(),
                path.clone().into_os_string(),
                "--".into(),
                "work".into(),
            ];
            assert!(crate::cli::commands::parse_argv(argv).is_err());
        }
    }
}

#[test]
fn parser_launch_directories_refuse_panes_and_existing_with_new_tab_guidance() {
    for option in ["--cwd", "--profile-dir"] {
        for mode in [
            vec!["--pane", "missing-pane", "--kind", "codex"],
            vec!["--existing", "--seat", "missing-seat"],
        ] {
            let mut argv = vec!["ht", "handoff"];
            argv.extend(mode);
            argv.extend([option, "/missing", "--", "work"]);
            let error = crate::cli::commands::parse_argv(argv).unwrap_err();
            assert!(error.detail.contains("--new-tab"), "{error:?}");
        }
        assert!(
            crate::cli::commands::parse_argv([
                "ht", "launch", "--pane", "peer", "--kind", "codex", option, "/missing",
            ])
            .is_err()
        );
    }
    for mode in [
        vec!["--new-tab", "peer", "--kind", "codex"],
        vec!["--pane", "peer", "--kind", "codex"],
        vec!["--existing", "--seat", "peer"],
    ] {
        let mut argv = vec!["ht", "handoff"];
        argv.extend(mode);
        argv.extend(["--", "work"]);
        let parsed = crate::cli::commands::parse_argv(argv).unwrap();
        if let crate::cli::commands::CliAction::TopologyHandoff(request) = parsed.action {
            assert!(request.cwd.is_none());
            assert!(request.profile_dir.is_none());
        }
    }
    assert!(
        crate::cli::commands::parse_argv(["ht", "launch", "--pane", "peer", "--kind", "codex",])
            .is_ok()
    );
}

#[test]
fn profile_directory_missing_capability_and_existing_selection_have_zero_effects() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counter(AtomicUsize);
    impl LocalClient for Counter {
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            assert!(
                matches!(command, Command::Capabilities),
                "unexpected recipient lookup or effect"
            );
            Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: vec![],
                },
            ))
        }
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
    }
    let temp = Temp::new();
    let client = Counter(AtomicUsize::new(0));
    let caller = claim();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: caller.instance.clone(),
        state_dir: temp.path().into(),
        host_endpoint: "/host.sock".into(),
    };
    let topology = topology();
    let clock = crate::app::SystemClock::new();
    for automatic in [false, true] {
        client.0.store(0, Ordering::Relaxed);
        let mut r = request();
        r.automatic = automatic;
        if automatic {
            r.thread = None;
        }
        r.profile_dir = Some(temp.path().canonicalize().unwrap());
        let inputs = Preparation {
            claim: &caller,
            namespace: &namespace,
            topology: &topology,
            invocation_cwd: temp.path(),
            options: None,
            client: &client,
            clock: &clock,
        };
        let result = if automatic {
            prepare_automatic(&r, inputs).map(|_| ())
        } else {
            prepare(&r, inputs).map(|_| ())
        };
        assert!(matches!(
            result,
            Err(RunError::Api(ApiError {
                code: crate::protocol::results::ErrorCode::Unsupported,
                ..
            }))
        ));
        assert_eq!(client.0.load(Ordering::Relaxed), 1);

        r.new_tab = None;
        r.existing = true;
        r.kind = None;
        r.binary = None;
        r.name = None;
        r.argv.clear();
        r.seat = Some("recipient".into());
        if automatic {
            r.thread = None;
        }
        let inputs = super::super::handoff_delivery::Preparation {
            claim: &caller,
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &clock,
        };
        let result = if automatic {
            super::super::handoff_delivery::prepare_automatic(&r, inputs).map(|_| ())
        } else {
            super::super::handoff_delivery::prepare(&r, inputs).map(|_| ())
        };
        assert!(result.is_err());
        assert_eq!(client.0.load(Ordering::Relaxed), 1);
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
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: root.join("state dir"),
        host_endpoint: "/host.sock".into(),
    };
    prepare(
        r,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: root,
            // Only the V1 harnesses take the configured native option words here.
            options: matches!(r.kind.as_deref(), Some("claude" | "codex"))
                .then(|| "--config 'quoted $HOME' --config '$(literal)'".into()),
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
            "--config",
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
            automatic: false,
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
fn profile_directory_publication_freezes_both_routes_and_preflight() {
    let temp = Temp::new();
    let profile = temp.path().join("profile ' selected");
    std::fs::create_dir(&profile).unwrap();
    let profile = profile.canonicalize().unwrap();
    let mut r = request();
    r.profile_dir = Some(profile.clone());
    let explicit = prepared(&r, temp.path()).unwrap();
    let context = explicit.payload.directory_context.as_ref().unwrap();
    assert_eq!(context.profile_dir, profile);
    assert_eq!(
        context.env.get("CODEX_HOME"),
        profile.to_str().map(str::to_owned).as_ref()
    );
    assert_eq!(explicit.payload.env, context.env);
    assert_eq!(
        preflight_launch_request(&explicit)
            .unwrap()
            .directory_context
            .as_ref(),
        Some(context)
    );
    r.automatic = true;
    r.thread = None;
    let namespace = explicit.payload.handoff.namespace.clone();
    let automatic = prepare_automatic(
        &r,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: temp.path(),
            options: None,
            client: &ReadOnly,
            clock: &crate::app::SystemClock::new(),
        },
    )
    .unwrap();
    assert_eq!(
        automatic.plan.payload.directory_context.as_ref(),
        Some(context)
    );
    assert_eq!(automatic.plan.payload.env, context.env);
    assert_eq!(
        preflight_automatic_launch_request(&automatic)
            .unwrap()
            .directory_context
            .as_ref(),
        Some(context)
    );
}

#[test]
fn profile_directory_disappeared_selection_refuses_publication() {
    let temp = Temp::new();
    let profile = temp.path().join("selected");
    std::fs::create_dir(&profile).unwrap();
    let mut r = request();
    r.profile_dir = Some(profile.canonicalize().unwrap());
    let identity = prepared(&r, temp.path()).unwrap();
    std::fs::remove_dir(&profile).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    assert!(publish(&journal, &identity, 1).is_err());
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}
// Shape validation admits any identifier-shaped key whose value is the profile
// directory; only the adapter's re-derived projection refuses a substituted key.
#[test]
fn profile_directory_submission_rederives_adapter_projection() {
    use crate::harness::registry::{OccupantHarness, builtins};
    let temp = Temp::new();
    let profile = temp.path().canonicalize().unwrap();
    let projection = |harness: OccupantHarness| {
        crate::harness::launch::launch_registration(builtins(), harness)
            .unwrap()
            .launch_policy()
            .unwrap()
            .explicit_directory_env(&profile)
            .unwrap()
    };
    let context = |harness| crate::protocol::handoff::LaunchDirectoryContext {
        profile_dir: profile.clone(),
        cwd: profile.clone(),
        env: projection(harness),
    };
    for harness in [OccupantHarness::Codex, OccupantHarness::Claude] {
        let genuine = context(harness);
        assert_eq!(genuine.validate(), Ok(()));
        check_bootstrap_directories(harness, &profile, Some(&genuine)).unwrap();
        let mut substituted = genuine.clone();
        substituted.env = [(
            "LD_PRELOAD".to_owned(),
            profile.to_str().unwrap().to_owned(),
        )]
        .into();
        assert_eq!(substituted.validate(), Ok(()));
        let error = check_bootstrap_directories(harness, &profile, Some(&substituted)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("retained profile projection differs from adapter"),
            "{error}"
        );
        let mut swapped = genuine.clone();
        swapped.env = projection(if harness == OccupantHarness::Codex {
            OccupantHarness::Claude
        } else {
            OccupantHarness::Codex
        });
        assert!(check_bootstrap_directories(harness, &profile, Some(&swapped)).is_err());
    }
    check_bootstrap_directories(OccupantHarness::Codex, &profile, None).unwrap();
    assert!(
        check_bootstrap_directories(
            OccupantHarness::Human,
            &profile,
            Some(&context(OccupantHarness::Codex))
        )
        .is_err()
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
                    automatic: false,
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
        let path =
            crate::test_support::scratch::temp_path(format!("ht-qhz4-{}", uuid::Uuid::new_v4()));
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
                host: crate::test_support::test_host_locator(),
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
fn publication_public_route_refuses_unavailable_daemon_before_publication() {
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
    let result = super::super::run_in_pane(argv.clone(), None, &mut output);
    assert!(
        matches!(
            result,
            Err(RunError::Api(ApiError {
                code: crate::protocol::results::ErrorCode::HostUnavailable,
                ..
            }))
        ),
        "{result:?}"
    );
    assert!(!state.exists());
    assert!(output.is_empty());
    let mut argv = argv;
    argv.splice(
        1..1,
        [
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
        ]
        .map(str::to_owned),
    );
    let result = super::super::run_in_pane(argv, None, &mut output);
    assert!(
        matches!(
            result,
            Err(RunError::Api(ApiError {
                code: crate::protocol::results::ErrorCode::HostUnavailable,
                ..
            }))
        ),
        "{result:?}"
    );
    assert!(!state.exists());
    assert!(output.is_empty());
}

#[test]
fn publication_explicit_cwd_claude_options_and_invalid_inputs() {
    let temp = Temp::new();
    let work = temp.path().join("quoted worktree");
    std::fs::create_dir(&work).unwrap();
    // Request carries the parser's already-canonical explicit directory.
    let work = work.canonicalize().unwrap();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: temp.path().join("state"),
        host_endpoint: "/host.sock".into(),
    };
    let r = Request {
        automatic: false,
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
            automatic: false,
            cwd: Some(file),
            ..request()
        },
        Request {
            body: "".into(),
            ..request()
        },
        Request {
            policy: None,
            new_tab: Some(" ".into()),
            ..request()
        },
        Request {
            automatic: false,
            kind: None,
            ..request()
        },
        Request {
            automatic: false,
            kind: Some("human".into()),
            ..request()
        },
        Request {
            automatic: false,
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
            automatic: false,
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
        RemoveProfileOnCheck,
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
        verify_wire: bool,
        /// When set, the durable-child store decides on this held clock
        /// instead of the real one (see [`HeldStoreClock`]).
        store_clock: Option<Arc<HeldStoreClock>>,
    }
    /// The durable-child store clock for a fixture whose subject is not
    /// timing. The durable child path issues a 250 ms cooperative permit and
    /// consumes it in the deciding transaction, both sampled on the store
    /// context's monotonic clock. This clock follows real time, except that
    /// it is held from just before one child's permit issuance until that
    /// child's mutation returns, so host load cannot widen the
    /// issuance-to-decision gap: the gap is exactly `permit_window` for the
    /// child named by `window_role` (every child when `None`) and zero for the
    /// others. After a hold, real time resumes without running backwards.
    /// The expiry itself stays exercised (see `held_store_clock_permit_window_*`).
    ///
    /// Hang mode: while held, the writer budget and the busy handler see no
    /// time pass, so a writer deadlock inside that one child mutation would
    /// hang until nextest's timeout instead of failing `DeadlineExceeded`.
    /// Everywhere else (setup, the send preparation steps, other calls) they
    /// run on real time.
    struct HeldStoreClock {
        real: crate::app::SystemClock,
        offset: std::sync::atomic::AtomicU64,
        held: Mutex<Option<u64>>,
        permit_window: std::sync::atomic::AtomicU64,
        window_role: Mutex<Option<&'static str>>,
    }
    impl HeldStoreClock {
        fn new() -> Self {
            Self {
                real: crate::app::SystemClock::new(),
                offset: std::sync::atomic::AtomicU64::new(0),
                held: Mutex::new(None),
                permit_window: std::sync::atomic::AtomicU64::new(0),
                window_role: Mutex::new(None),
            }
        }
        fn running(&self) -> u64 {
            crate::protocol::time::Clock::monotonic_now(&self.real).0
                + self.offset.load(std::sync::atomic::Ordering::SeqCst)
        }
        /// Freezes time just before a child's permit issuance.
        fn hold(&self) -> HeldStoreClockHold<'_> {
            let mut held = self.held.lock().unwrap();
            // Not reentrant: a nested hold would move the frozen instant.
            assert!(held.is_none(), "store clock hold is not reentrant");
            *held = Some(self.running());
            drop(held);
            HeldStoreClockHold(self)
        }
        /// Moves the frozen instant by the window when `role` is targeted.
        fn pass_permit_window(&self, role: &str) {
            let target = *self.window_role.lock().unwrap();
            if target.is_none_or(|target| target == role) {
                let millis = self.permit_window.load(std::sync::atomic::Ordering::SeqCst);
                if let Some(held) = self.held.lock().unwrap().as_mut() {
                    *held += millis;
                }
            }
        }
    }
    /// Resumes real time when the held child mutation returns, including
    /// on an early error return.
    struct HeldStoreClockHold<'a>(&'a HeldStoreClock);
    impl Drop for HeldStoreClockHold<'_> {
        fn drop(&mut self) {
            let clock = self.0;
            if let Some(held) = clock.held.lock().unwrap().take() {
                let real = crate::protocol::time::Clock::monotonic_now(&clock.real).0;
                clock.offset.fetch_max(
                    held.saturating_sub(real),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
        }
    }
    impl crate::protocol::time::Clock for HeldStoreClock {
        fn utc_now(&self) -> UtcMillis {
            crate::protocol::time::Clock::utc_now(&self.real)
        }
        fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
            let held = *self.held.lock().unwrap();
            crate::protocol::time::MonoInstant(held.unwrap_or_else(|| self.running()))
        }
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
            if self.verify_wire {
                let wire = crate::protocol::wire::WireRequest {
                    version: crate::protocol::wire::PROTOCOL_VERSION,
                    request_id: "maximum-body".into(),
                    expected_instance: "00000000000040008000000000000001".into(),
                    expected_boot: None,
                    output: None,
                    command: command.clone(),
                };
                let bytes = serde_json::to_vec(&wire).unwrap();
                assert!(bytes.len() + 4 <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES);
                crate::protocol::wire::WireRequest::decode(&bytes).unwrap();
            }
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
            if role == "check" && s.fault == Fault::RemoveProfileOnCheck {
                std::fs::remove_dir(
                    &self
                        .identity
                        .payload
                        .directory_context
                        .as_ref()
                        .unwrap()
                        .profile_dir,
                )
                .unwrap();
                s.fault = Fault::None;
            }
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
                            crate::protocol::capabilities::ADVERTISED
                                .iter()
                                .copied()
                                .chain([
                                    crate::protocol::capabilities::HANDOFF_LAUNCH_DIRECTORIES_V1,
                                    crate::protocol::capabilities::INVITATION_CONFIRMATION_V1,
                                    crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1,
                                ])
                                .map(str::to_owned)
                                .collect()
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
                    s.db.execute("UPDATE occupant_bindings SET native_session='changed-session' WHERE seat_id='sender'", []).unwrap();
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
            let store_clock = || -> Arc<dyn crate::protocol::time::Clock> {
                match &self.store_clock {
                    Some(held) => held.clone(),
                    None => Arc::new(crate::app::SystemClock::new()),
                }
            };
            let store = crate::store::SqliteStore::new(
                crate::store::connection::StoreContext::new(
                    self.database_path.clone(),
                    store_clock(),
                )
                .with_test_host(),
                "34750f98bd59fcfc946da45aaabe933b",
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
                    match crate::ports::BootstrapStorePort::bootstrap_prepare_send_step(
                        &store,
                        &self.identity.payload.handoff.namespace,
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
            let role = match &mutation {
                crate::protocol::commands::PermitMutation::CreateThread(_) => "create_child",
                crate::protocol::commands::PermitMutation::Invite(_) => "invite_child",
                _ => "send_child",
            };
            let _hold = self.store_clock.as_deref().map(HeldStoreClock::hold);
            let permit = store.issue_cooperative_permit(
                crate::store::cooperative_permit_request(&mutation)?,
                budget,
            )?;
            if let Some(held) = &self.store_clock {
                held.pass_permit_window(role);
            }
            let permit = if matches!(
                mutation,
                crate::protocol::commands::PermitMutation::Invite(_)
                    | crate::protocol::commands::PermitMutation::SendMessage(_)
            ) {
                permit.with_bootstrap_child(&self.identity.payload.handoff.namespace, &mutation)
            } else {
                permit
            };
            if let crate::protocol::commands::PermitMutation::CreateThread(create) = &mutation {
                crate::store::control::create_thread_in_namespace(
                    store.context(),
                    &mut store.context().open_writer()?,
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
            assert_eq!(r.env, self.identity.payload.env);
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
        /// A same-user store writer's edit of the frozen plan, applied before publication.
        fn tampered(
            fault: Fault,
            request: Request,
            tamper: &dyn Fn(&mut crate::protocol::handoff::BootstrapIdentity),
        ) -> Self {
            Self::build(fault, request, None, false, false, Some(tamper))
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
            Self::build(fault, request, argv, aligned, maximal, None)
        }
        fn build(
            fault: Fault,
            request: Request,
            argv: Option<Vec<String>>,
            aligned: bool,
            maximal: bool,
            tamper: Option<&dyn Fn(&mut crate::protocol::handoff::BootstrapIdentity)>,
        ) -> Self {
            let temp = Temp::new();
            let mut identity = prepared(&request, temp.path()).unwrap();
            if let Some(tamper) = tamper {
                tamper(&mut identity);
            }
            if let Some(argv) = argv {
                identity.payload.launch.argv = argv;
                // A V2 plan freezes the adapter's composition of these very arguments.
                if identity.payload.launch.composed_argv.is_some() {
                    identity.payload.launch.composed_argv = Some(
                        crate::harness::launch::compose_bootstrap_v2_argv(
                            identity.payload.launch.harness,
                            identity.payload.launch.argv.clone(),
                        )
                        .unwrap(),
                    );
                    identity.payload.launch.startup_input = Some(
                        crate::harness::launch::bootstrap_v2_startup_input(
                            identity.payload.launch.harness,
                            &identity.payload.launch.argv,
                        )
                        .unwrap(),
                    );
                }
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
                    crate::daemon::paths::NodePaths::resolve_read_only(&runtime.state_dir).unwrap();
                std::fs::create_dir_all(&paths.node_dir).unwrap();
                identity.payload.handoff.namespace.state_dir = runtime.state_dir.clone();
                identity.payload.handoff.namespace.host_endpoint =
                    runtime.host_endpoint.clone().unwrap();
                identity.digest = identity.semantic_digest().unwrap();
                (paths.node_dir.join("intents"), paths.database_path)
            } else {
                (
                    temp.path().join("intents"),
                    temp.path().join("canonical.db"),
                )
            };
            if maximal {
                identity.payload.handoff.body = "\u{1}".repeat(65_536);
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
                        directory_context: identity.payload.directory_context.clone(),
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
            // A store writer seeds the fresh database from the per-binary
            // schema template (the same migrations, run once per binary)
            // and verifies it, instead of replaying every migration here.
            drop(
                crate::store::connection::StoreContext::new(
                    database_path.clone(),
                    Arc::new(crate::app::SystemClock::new()),
                )
                .with_test_host()
                .open_writer()
                .unwrap(),
            );
            let db = crate::test_support::open_test_connection(&database_path);
            let created = crate::protocol::handoff::topology_contract_tests::created();
            let boot = &created.host_incarnation;
            db.execute("UPDATE herdr_hosts SET host_boot=?1,host_epoch=1,observation_sequence=1,observation_admission_sequence=1,observation_decided_sequence=1,lifecycle_revision=1,recovery_boot=?1,recovery_epoch=1",[boot.as_str()]).unwrap();
            db.execute("INSERT INTO snapshot_generations(id,host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) VALUES('g',(SELECT min(host_id) FROM herdr_hosts),?1,1,1,'herdr-server:pid=42:start=1.000002:uid=501',0,0,'published',0,0,0,0)",[boot.as_str()]).unwrap();
            db.execute_batch("UPDATE herdr_hosts SET active_snapshot_id='g',recovery_baseline_generation_id='g'; INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('sender','34750f98bd59fcfc946da45aaabe933b','resolved','native','w1:p1',1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts)); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES('canonical-thread','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0); INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('canonical-thread','sender','joined',0)").unwrap();
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('sender',1,'w1:p1',?1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0)",[boot.as_str()]).unwrap();
            db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p1',?1,1,0,2,'fresh',0,'caller-terminal','herdr-server:pid=42:start=1.000002:uid=501','native_current_target',1)",[boot.as_str()]).unwrap();
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
                    verify_wire: request.body.len() == 65_536,
                    store_clock: None,
                }),
                _temp: temp,
                clock,
                context,
                witness,
            }
        }
        /// Holds the durable-child store clock (see [`HeldStoreClock`]).
        fn hold_store_clock(&mut self) -> Arc<HeldStoreClock> {
            let held = Arc::new(HeldStoreClock::new());
            Arc::get_mut(&mut self.peer)
                .expect("hold the store clock before sharing the peer")
                .store_clock = Some(held.clone());
            held
        }
        /// Captures the fixture's whole on-disk state (journal, retained
        /// files, canonical database) and the peer's call log, so a matrix
        /// can run each case from one shared completed setup instead of
        /// rebuilding the fixture and replaying the setup handoff per case.
        fn checkpoint(&self) -> Checkpoint {
            let mut state = self.peer.state.lock().unwrap();
            // Close the only connection so the WAL is folded into the file.
            state.db = rusqlite::Connection::open_in_memory().unwrap();
            let mut checkpoint = Checkpoint {
                entries: vec![],
                calls: state.calls.clone(),
                native_calls: state.native_calls,
                fault: state.fault,
            };
            fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<CheckpointEntry>) {
                use std::os::unix::fs::PermissionsExt;
                let mut names: Vec<_> = std::fs::read_dir(dir)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect();
                names.sort();
                for path in names {
                    let meta = std::fs::symlink_metadata(&path).unwrap();
                    let relative = path.strip_prefix(root).unwrap().to_owned();
                    let mode = meta.permissions().mode();
                    assert!(!meta.file_type().is_symlink(), "checkpoint holds no links");
                    if meta.is_dir() {
                        out.push(CheckpointEntry {
                            relative,
                            mode,
                            bytes: None,
                        });
                        walk(root, &path, out);
                    } else {
                        out.push(CheckpointEntry {
                            relative,
                            mode,
                            bytes: Some(std::fs::read(&path).unwrap()),
                        });
                    }
                }
            }
            walk(
                self._temp.path(),
                self._temp.path(),
                &mut checkpoint.entries,
            );
            state.db = crate::test_support::open_test_connection(&self.peer.database_path);
            checkpoint
        }
        /// Returns the fixture to exactly the checkpointed state: every file
        /// a case added, changed, removed or re-permissioned is reset.
        fn restore(&self, checkpoint: &Checkpoint) {
            use std::os::unix::fs::PermissionsExt;
            let mut state = self.peer.state.lock().unwrap();
            state.db = rusqlite::Connection::open_in_memory().unwrap();
            let root = self._temp.path();
            fn writable(dir: &std::path::Path) {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
                for entry in std::fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                        writable(&path);
                    }
                }
            }
            writable(root);
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                    std::fs::remove_dir_all(&path).unwrap();
                } else {
                    std::fs::remove_file(&path).unwrap();
                }
            }
            for entry in &checkpoint.entries {
                let path = root.join(&entry.relative);
                match &entry.bytes {
                    None => std::fs::create_dir(&path).unwrap(),
                    Some(bytes) => {
                        std::fs::write(&path, bytes).unwrap();
                        std::fs::set_permissions(
                            &path,
                            std::fs::Permissions::from_mode(entry.mode),
                        )
                        .unwrap();
                    }
                }
            }
            for entry in checkpoint
                .entries
                .iter()
                .rev()
                .filter(|e| e.bytes.is_none())
            {
                std::fs::set_permissions(
                    root.join(&entry.relative),
                    std::fs::Permissions::from_mode(entry.mode),
                )
                .unwrap();
            }
            state.db = crate::test_support::open_test_connection(&self.peer.database_path);
            state.calls = checkpoint.calls.clone();
            state.native_calls = checkpoint.native_calls;
            state.fault = checkpoint.fault;
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
    struct CheckpointEntry {
        relative: std::path::PathBuf,
        mode: u32,
        bytes: Option<Vec<u8>>,
    }
    struct Checkpoint {
        entries: Vec<CheckpointEntry>,
        calls: Vec<&'static str>,
        native_calls: usize,
        fault: Fault,
    }
    #[derive(Default)]
    struct DownstreamLauncher {
        requests: Vec<super::super::super::launch::LaunchRequest>,
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
        remove_directory_on_preflight: Option<std::path::PathBuf>,
    }
    impl super::super::super::handoff::HandoffLauncher for DownstreamLauncher {
        fn preflight_saved(
            &mut self,
            _: &super::super::super::launch::LaunchRequest,
            effective: &super::super::super::launch::LaunchRequest,
            recipient: &SeatId,
        ) -> Result<super::super::super::handoff::StartupPreflight, RunError> {
            if let Some(directory) = self.remove_directory_on_preflight.take() {
                std::fs::remove_dir(directory).unwrap();
            }
            Ok(super::super::super::handoff::StartupPreflight {
                recipient: recipient.clone(),
                native_argv: self.native_input(effective)?,
            })
        }
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
            self.requests.push(request.clone());
            let argv = crate::harness::launch::compose_native_argv(
                request.harness.into(),
                request.argv.clone(),
                vec![],
            )?;
            if let Some(path) = &self.change_binding_before_gate {
                crate::test_support::open_test_connection(path).execute("UPDATE occupant_bindings SET native_session='changed-before-launch' WHERE seat_id='sender'",[]).unwrap();
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
            let mut report = serde_json::json!({"outcome":if self.unknown {"outcome_unknown"} else {"started"},"pane":request.target,"seat":seat,"harness":request.harness.as_str(),"argv":argv});
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
        launcher: &mut dyn super::super::super::handoff::HandoffLauncher,
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

    #[test]
    fn profile_directory_frozen_retry_retains_canonical_paths_and_exact_child() {
        use std::os::unix::fs::symlink;
        let temp = Temp::new();
        let selected = temp.path().join("selected");
        let other = temp.path().join("other");
        std::fs::create_dir(&selected).unwrap();
        std::fs::create_dir(&other).unwrap();
        let link = temp.path().join("link");
        symlink(&selected, &link).unwrap();
        let mut r = request();
        r.profile_dir = Some(link.canonicalize().unwrap());
        let f = Fixture::with_request(Fault::None, r, None);
        std::fs::remove_file(&link).unwrap();
        symlink(&other, &link).unwrap();
        std::fs::remove_dir(&selected).unwrap();
        assert!(f.run().is_err());
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
        std::fs::create_dir(&selected).unwrap();
        let mut launcher = DownstreamLauncher::default();
        let completed = downstream(&f, &mut launcher, &mut Vec::new()).unwrap();
        assert_eq!(completed.state, BootstrapState::Completed);
        assert_eq!(launcher.starts, 1);
        assert_eq!(
            launcher.requests[0]
                .directory_context
                .as_ref()
                .unwrap()
                .profile_dir,
            selected.canonicalize().unwrap()
        );
        assert_eq!(
            launcher.requests[0].directory_context,
            f.peer.identity.payload.directory_context
        );
        std::fs::remove_dir(&selected).unwrap();
        downstream(&f, &mut launcher, &mut Vec::new()).unwrap();
        assert_eq!(launcher.starts, 1);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }

    #[test]
    fn profile_directory_missing_before_start_is_repairable_without_new_tab() {
        let temp = Temp::new();
        let selected = temp.path().join("selected");
        std::fs::create_dir(&selected).unwrap();
        let mut r = request();
        r.profile_dir = Some(selected.canonicalize().unwrap());
        let f = Fixture::with_request(Fault::None, r, None);
        f.run().unwrap();
        std::fs::remove_dir(&selected).unwrap();
        let mut launcher = DownstreamLauncher::default();
        assert!(downstream(&f, &mut launcher, &mut Vec::new()).is_err());
        assert_eq!(launcher.starts, 0);
        std::fs::create_dir(&selected).unwrap();
        downstream(&f, &mut launcher, &mut Vec::new()).unwrap();
        assert_eq!(launcher.starts, 1);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }

    // A frozen plan whose projection key was substituted passes shape validation
    // and publication, and is refused before any reservation or native call.
    #[test]
    fn profile_directory_substituted_projection_never_reaches_tab_creation() {
        let temp = Temp::new();
        let selected = temp.path().join("selected");
        std::fs::create_dir(&selected).unwrap();
        let mut r = request();
        r.profile_dir = Some(selected.canonicalize().unwrap());
        let f = Fixture::tampered(Fault::None, r, &|identity| {
            let context = identity.payload.directory_context.as_mut().unwrap();
            let value = context.profile_dir.to_str().unwrap().to_owned();
            context.env = [("LD_PRELOAD".to_owned(), value)].into();
            identity.payload.env = context.env.clone();
            identity.digest = identity.semantic_digest().unwrap();
        });
        let error = f.run().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("retained profile projection differs from adapter"),
            "{error}"
        );
        let state = f.peer.state.lock().unwrap();
        assert_eq!(state.native_calls, 0);
        assert!(
            !state.calls.iter().any(|call| call.contains("reserve")),
            "{:?}",
            state.calls
        );
    }

    #[test]
    fn profile_directory_removed_at_create_boundary_rearms_without_uncertainty() {
        let temp = Temp::new();
        let selected = temp.path().join("selected");
        std::fs::create_dir(&selected).unwrap();
        let mut r = request();
        r.profile_dir = Some(selected.canonicalize().unwrap());
        let f = Fixture::with_request(Fault::RemoveProfileOnCheck, r, None);
        let result = f.run().unwrap();
        assert_eq!(result.state, BootstrapState::Prepared);
        assert_eq!(result.attempt.get(), 2);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
        std::fs::create_dir(&selected).unwrap();
        assert_eq!(f.run().unwrap().state, BootstrapState::Attached);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    fn composition_source(f: &Fixture) -> crate::archival_legacy::Source {
        let ns = &f.peer.identity.payload.handoff.namespace;
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            ns.state_dir.clone(),
            ns.host_endpoint.clone(),
            None,
        )
        .unwrap();
        let paths = crate::daemon::paths::NodePaths::resolve_read_only(&runtime.state_dir).unwrap();
        assert_eq!(paths.database_path, f.peer.database_path);
        assert_eq!(paths.node_dir.join("intents"), f.journal.root());
        crate::archival_legacy::Source::new(
            &paths,
            "34750f98bd59fcfc946da45aaabe933b".into(),
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
            // A page that finds no record but follows one in the same traversal
            // carries a `Hint::Coverage` continuity marker (a no-op in
            // `archival_pass`, sharing the record hints' veto). Whether one
            // appears depends on where the 10 ms page deadline splits the
            // traversal, so the accumulated scan keeps only record hints.
            result.hints.extend(
                page.hints
                    .into_iter()
                    .filter(|hint| !matches!(hint, crate::archival_legacy::Hint::Coverage { .. })),
            );
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
            )
            .with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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
            hosts: crate::test_support::test_host_evidence(0, false),
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
        assert!(
            saved.creation.is_none(),
            "canonical evidence is not a local receipt"
        );
        assert!(saved.possible_creation);
        let before = f.peer.status();
        assert!(before.creation.is_some());
        let surviving_bytes = std::fs::read(f.peer.progress()).unwrap();
        let scan = composition_scan(&f);
        assert!(
            scan.coverage.is_some(),
            "actual canonical-created requestNone must compose"
        );
        assert_eq!(scan.hints.len(), 1);
        composition_import(&f, &scan).unwrap();
        assert_eq!(std::fs::read(f.peer.progress()).unwrap(), surviving_bytes);
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
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::UnknownOutcome,
            "downstream possible start",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
    /// Quotes in `large_escaped_argv`; each escapes to two origin bytes.
    const LARGE_ARGV_ESCAPES: usize = 16 * 200;
    /// Canonically legal argv (16 args, 61744 raw bytes, under the 65536 raw
    /// launch bound) whose bootstrap origin fits 64 KiB unescaped but exceeds
    /// it (`archival_legacy::RECORD_CAP`, the 65536 delivery bound) only once
    /// its quotes are escaped. Few escapes: each is a separate write when
    /// progress is serialized, so a quote-only argv cost seconds per save.
    fn large_escaped_argv() -> Vec<String> {
        vec![format!("--config={}{}", "x".repeat(3650), "\"".repeat(200)); 16]
    }
    /// One large retained bundle (origin over a record, terminal and child
    /// each over a legacy page) is built once; each phase restores it.
    #[test]
    fn composition_actual_large_bundle_survivors_admission_loss_and_cancel() {
        let mut f = Fixture::aligned(Fault::None, Some(large_escaped_argv()));
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let original = f
            .journal
            .snapshot_bootstrap_origin(&f.peer.reference)
            .unwrap();
        assert!(original.len() > crate::archival_legacy::RECORD_CAP);
        assert!(original.len() - LARGE_ARGV_ESCAPES < crate::archival_legacy::RECORD_CAP);
        let mut launcher = DownstreamLauncher {
            report_padding: 200000,
            ..Default::default()
        };
        let mut output = FailingOutput {
            bytes: vec![],
            write: true,
        };
        expect_io_error(
            downstream(&f, &mut launcher, &mut output),
            std::io::ErrorKind::Other,
            "injected output write loss",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        assert_eq!(launcher.starts, 1);
        let terminal = std::fs::read(terminal_path(&f.journal, &f.peer.reference)).unwrap();
        let child = std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap();
        assert!(terminal.len() > 262144);
        assert!(child.len() > 262144);
        let before = f.peer.status();
        let retained = f.checkpoint();

        // Legal large retained terminal and surviving files establish coverage.
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

        // Worker admission loss stays sticky, then a fresh traversal recovers.
        f.restore(&retained);
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
        drop(worker);

        // Cancellation between reads vetoes the consumed large bundle.
        f.restore(&retained);
        let mut source = composition_source(&f);
        let calls = std::cell::Cell::new(0);
        // A page may end on its 10 ms budget before the sixth check; keep
        // scanning so the cancellation still lands inside the bundle.
        let error = loop {
            match source.scan(|| {
                calls.set(calls.get() + 1);
                calls.get() >= 6
            }) {
                Err(error) => break error,
                Ok(page) => assert!(page.pending, "cancel during consumed bundle"),
            }
        };
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(f.peer.status(), before);
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn composition_retained_completion_requires_full_canonical_equality() {
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::UnknownOutcome,
            "downstream possible start",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        fn live_topology(
            &self,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<crate::host::observation::HostTopology, crate::protocol::results::ApiError>
        {
            Err(crate::protocol::results::ApiError::unsupported(
                "fixture host has no topology",
            ))
        }
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
                    )
                    .with_test_host(),
                    "34750f98bd59fcfc946da45aaabe933b",
                    Default::default(),
                )
                .unwrap(),
            ),
            hosts: Arc::new(crate::service::archival::StaticHosts(vec![
                crate::service::archival::ArchivalHost {
                    host_id: crate::test_support::test_host_id(),
                    port: Arc::new(CompositionHost),
                    reachability: Arc::new(
                        crate::service::host_reachability::HostReachability::default(),
                    ),
                },
            ])),
            writer: Arc::new(crate::service::fair_writer::FairWriter::new(32)),
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
                "SELECT bootstrap_veto FROM archival_instances WHERE instance_id='34750f98bd59fcfc946da45aaabe933b'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn composition_actual_retained_bundle_cap_and_corruption_controls() {
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        expect_io_error(
            downstream(
                &f,
                &mut DownstreamLauncher::default(),
                &mut FailingOutput {
                    bytes: vec![],
                    write: true,
                },
            ),
            std::io::ErrorKind::Other,
            "injected output write loss",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let retained = f.checkpoint();
        // Control: the undamaged restored bundle composes (restore first, so
        // this proves restore fidelity, not just the original state).
        f.restore(&retained);
        assert!(composition_scan(&f).coverage.is_some());
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
            f.restore(&retained);
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
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::UnknownOutcome,
            "downstream possible start",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
    #[cfg(target_os = "linux")]
    fn composition_heap_statistics() -> [usize; 3] {
        // glibc arena statistics: live bytes (small + mmapped), then the
        // touched arena size twice (glibc keeps no separate high water).
        let info = unsafe { libc::mallinfo2() };
        let touched = info.arena + info.hblkhd;
        [info.uordblks + info.hblkhd, touched, touched]
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    struct CompositionSampler {
        stop: Arc<std::sync::atomic::AtomicBool>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    impl Drop for CompositionSampler {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }
    struct CompositionContender<'a> {
        turn: Option<crate::service::fair_writer::WriterGuard<'a>>,
        thread: Option<std::thread::JoinHandle<std::time::Duration>>,
    }
    impl CompositionContender<'_> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        fn finish(mut self) -> std::time::Duration {
            self.turn.take();
            self.thread.take().unwrap().join().unwrap()
        }
    }
    impl Drop for CompositionContender<'_> {
        fn drop(&mut self) {
            // Release admission before joining, including unwind from a failed
            // deciding pass. Ignore a second panic while unwinding.
            self.turn.take();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    #[test]
    fn composition_foreground_contender_is_joined_when_deciding_pass_unwinds() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let writer = Arc::new(crate::service::fair_writer::FairWriter::new(8));
        let clock = Arc::new(crate::app::SystemClock::new());
        let budget = super::super::super::cooperative_budget(clock.as_ref());
        let turn = writer.enter_background(&budget, clock.as_ref()).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let thread = {
            let writer = writer.clone();
            let clock = clock.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let started = std::time::Instant::now();
                let budget = super::super::super::cooperative_budget(clock.as_ref());
                let _turn = writer.enter_foreground(&budget, clock.as_ref()).unwrap();
                done.store(true, Ordering::Release);
                started.elapsed()
            })
        };
        let contender = CompositionContender {
            turn: Some(turn),
            thread: Some(thread),
        };
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _owned = contender;
            panic!("owned deciding failure");
        }));
        assert!(error.is_err());
        assert!(done.load(Ordering::Acquire));
        assert_eq!(writer.waiting(), (0, 0));
        assert!(writer.enter_foreground(&budget, clock.as_ref()).is_ok());
    }
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn composition_maximum_legal_escaped_serializer_memory_and_budgets() {
        use crate::ports::StorePort;
        use std::sync::atomic::Ordering;
        let mut f = Fixture::with_layout(
            Fault::None,
            request(),
            Some(vec![format!("--config={}", "\u{1}".repeat(1250)); 16]),
            true,
            true,
        );
        // Setup only: the subject is the maximal report's canonical
        // completion and the composition pass's own budgets below, not the
        // 250 ms permit, so host load must not expire the child permits.
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            max_report: true,
            ..Default::default()
        };
        let error = downstream(
            &f,
            &mut launcher,
            &mut FailingOutput {
                bytes: vec![],
                write: true,
            },
        )
        .unwrap_err();
        assert!(
            matches!(error, RunError::Io(_)),
            "only the injected output loss may fail the maximal run: {error:?}"
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
                )
                .with_test_host(),
                "34750f98bd59fcfc946da45aaabe933b",
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
            assert!(
                statistics[1] <= 5_390_340,
                "bundle bytes={} identity={identity} report={report} original={original} submission={submission} child={child} terminal={terminal}",
                statistics[1]
            );
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
            let contender = CompositionContender {
                turn: Some(turn),
                thread: Some(contender),
            };
            waiting.recv().unwrap();
            let deciding_start = std::time::Instant::now();
            composition_pass(&store, &scan).unwrap();
            maximum_deciding = maximum_deciding.max(deciding_start.elapsed());
            maximum_foreground = maximum_foreground.max(contender.finish());
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
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::with_layout(
            Fault::None,
            request,
            Some(vec!["--model".into(), "saved value".into()]),
            true,
            false,
        );
        // Setup only: the subject is the retained child progress and its
        // composition, not the 250 ms permit, so host load must not expire
        // the create child's permit before its progress is saved.
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            unknown: true,
            ..Default::default()
        };
        let error = downstream(&f, &mut launcher, &mut vec![]).unwrap_err();
        assert!(
            matches!(&error, RunError::Api(e) if e.code == ErrorCode::UnknownOutcome && e.detail.contains("downstream possible start")),
            "only the unknown launch outcome may fail the first run: {error:?}"
        );
        assert_eq!(
            child_calls(&f),
            ["create_child", "invite_child", "send_child"]
        );
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
        let mut f = Fixture::aligned(Fault::None, None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        expect_io_error(
            downstream(
                &f,
                &mut DownstreamLauncher::default(),
                &mut FailingOutput {
                    bytes: vec![],
                    write: true,
                },
            ),
            std::io::ErrorKind::Other,
            "injected output write loss",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let retained = f.checkpoint();
        // Control: the uncontradicted restored bundle composes (restore first,
        // so this proves restore fidelity, not just the original state).
        f.restore(&retained);
        assert!(composition_scan(&f).coverage.is_some());
        for control in ["creation", "attempt"] {
            f.restore(&retained);
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
    fn maximum_instruction_file_freezes_through_bootstrap_completion_and_retry() {
        for body in ["x".repeat(65_536), "\u{1}".repeat(65_536)] {
            let source = Temp::new();
            let path = source.path().join("instructions");
            std::fs::write(&path, &body).unwrap();
            let parsed = crate::cli::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "peer",
                "--kind",
                "codex",
                "--thread",
                "chosen",
                "--deadline",
                "7",
                "--invite-deadline",
                "11",
                "--file",
                path.to_str().unwrap(),
            ])
            .unwrap();
            let crate::cli::commands::CliAction::TopologyHandoff(mut r) = parsed.action else {
                panic!("wrong handoff route")
            };
            r.binary = Some("/bin/quoted path".into());
            let mut f = Fixture::with_request(Fault::CompleteBefore, r, Some(vec![]));
            // Setup only: host load must not expire a 250 ms child permit;
            // the subject is not the permit (see HeldStoreClock).
            f.hold_store_clock();
            let original = f
                .journal
                .snapshot_bootstrap_origin(&f.peer.reference)
                .unwrap();
            std::fs::write(&path, "changed source").unwrap();
            std::fs::remove_file(&path).unwrap();
            let mut launcher = DownstreamLauncher::default();
            let mut output = vec![];
            expect_api_error(
                downstream(&f, &mut launcher, &mut output),
                ErrorCode::HostUnavailable,
                "crash before wrapper deciding transaction",
            );
            assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
            f.peer.state.lock().unwrap().db =
                crate::test_support::open_test_connection(&f.peer.database_path);
            let result = downstream(&f, &mut launcher, &mut output).unwrap();
            assert_eq!(result.state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            let state = f.peer.state.lock().unwrap();
            let saved: String = state
                .db
                .query_row("SELECT body FROM messages WHERE body=?1", [&body], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(saved, body);
            let (confirmation, duration): (String, i64) = state
                .db
                .query_row(
                    "SELECT confirmation,frozen_duration_ms FROM invitations",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!((confirmation.as_str(), duration), ("require_ack", 11000));
            let message: String = state
                .db
                .query_row("SELECT id FROM messages WHERE body=?1", [&body], |r| {
                    r.get(0)
                })
                .unwrap();
            let recipient: String = state
                .db
                .query_row("SELECT seat_id FROM invitations", [], |r| r.get(0))
                .unwrap();
            let receipt =
                crate::store::effective::effective_receipt(&state.db, &message, &recipient)
                    .unwrap()
                    .unwrap();
            assert_eq!(receipt.frozen_duration_ms, 7000);
            assert_eq!(receipt.deadline_at, Some(receipt.decision_at + 7000));
            drop(state);
            let terminal = read_terminal(&f.journal, &f.peer.reference)
                .unwrap()
                .unwrap();
            assert_eq!(terminal.original.as_bytes(), original);
            assert_eq!(terminal.completed.identity.payload.handoff.body, body);
            let argv = terminal.completed.retained.report["argv"]
                .as_array()
                .unwrap();
            assert!(argv.iter().all(|arg| !arg.as_str().unwrap().contains(&body)
                && !arg.as_str().unwrap().contains(path.to_str().unwrap())));
            let mut replay = vec![];
            downstream(&f, &mut launcher, &mut replay).unwrap();
            assert_eq!(launcher.starts, 1);
        }
    }

    #[test]
    fn downstream_atomic_completion_runs_exact_child_then_flushes_success() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::with_request(Fault::CompleteBefore, request, Some(vec![]));
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(
            child_calls(&f),
            ["create_child", "invite_child", "send_child"]
        );
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
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        let mut f = Fixture::downstream(Fault::CompleteReply);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        let done = downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_eq!(done.state, BootstrapState::Completed);
        assert_eq!(
            done.completed.unwrap().legacy_result.state,
            HandoffState::Completed
        );
        assert_downstream_once(&f, &launcher);
    }
    fn child_calls(f: &Fixture) -> Vec<&'static str> {
        f.peer
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .copied()
            .filter(|role| role.ends_with("_child"))
            .collect()
    }
    /// Asserts a run failed with exactly this API error code and a detail
    /// containing `detail`, so an earlier failure (such as an expired child
    /// permit) is reported by name instead of passing as any error.
    #[track_caller]
    fn expect_api_error<T: std::fmt::Debug>(
        result: Result<T, RunError>,
        code: ErrorCode,
        detail: &str,
    ) {
        assert!(
            matches!(&result, Err(RunError::Api(e)) if e.code == code && e.detail.contains(detail)),
            "expected {code:?} {detail:?}, got {result:?}"
        );
    }
    /// Asserts a run failed with exactly this I/O error kind and a message
    /// containing `message` (see [`expect_api_error`]).
    #[track_caller]
    fn expect_io_error<T: std::fmt::Debug>(
        result: Result<T, RunError>,
        kind: std::io::ErrorKind,
        message: &str,
    ) {
        assert!(
            matches!(&result, Err(RunError::Io(e)) if e.kind() == kind && e.to_string().contains(message)),
            "expected {kind:?} {message:?}, got {result:?}"
        );
    }
    /// Negative control for [`HeldStoreClock`]: holding the clock does not
    /// disable the permit expiry. A window past `MAX_PERMIT_MILLIS` between
    /// one child's issuance and decision refuses exactly that child, every
    /// earlier child having decided, and leaves the bootstrap Attached (the
    /// hosted-CI failure shape); the exact bound completes every child.
    #[test]
    fn held_store_clock_permit_window_past_the_bound_still_expires() {
        let children = {
            let mut f = Fixture::downstream(Fault::None);
            f.hold_store_clock();
            downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]).unwrap();
            child_calls(&f)
        };
        assert!(children.contains(&"invite_child") && children.contains(&"send_child"));
        for (index, role) in children.iter().copied().enumerate() {
            let mut f = Fixture::downstream(Fault::None);
            let held = f.hold_store_clock();
            *held.window_role.lock().unwrap() = Some(role);
            held.permit_window.store(
                crate::protocol::authority::MAX_PERMIT_MILLIS + 1,
                std::sync::atomic::Ordering::SeqCst,
            );
            let error =
                downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]).unwrap_err();
            assert!(
                matches!(&error, RunError::Api(e) if e.code == ErrorCode::CallerUnverified && e.detail == "permit expired"),
                "{role}: {error:?}"
            );
            assert_eq!(
                child_calls(&f),
                children[..=index],
                "{role} must be the refused child"
            );
            assert_eq!(f.peer.status().state, BootstrapState::Attached, "{role}");
        }
    }
    #[test]
    fn held_store_clock_permit_window_at_the_bound_completes() {
        let mut f = Fixture::downstream(Fault::None);
        let held = f.hold_store_clock();
        held.permit_window.store(
            crate::protocol::authority::MAX_PERMIT_MILLIS,
            std::sync::atomic::Ordering::SeqCst,
        );
        let mut launcher = DownstreamLauncher::default();
        let done = downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_eq!(done.state, BootstrapState::Completed);
        let children = child_calls(&f);
        assert!(children.contains(&"invite_child") && children.contains(&"send_child"));
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_crash_before_or_after_native_start_never_relaunches() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for after_start in [false, true] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher {
                crash_after_gate: !after_start,
                crash_after_start: after_start,
                ..Default::default()
            };
            let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                downstream(&f, &mut launcher, &mut vec![])
            }));
            assert!(
                crashed.is_err(),
                "only the injected launcher crash may end the first run: {crashed:?}"
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            not_submitted: true,
            ..Default::default()
        };
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "adapter proven no native submission",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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

    // Dropping execute_steps' phase/report would hide durable prestart refusal.
    #[test]
    fn final_sdd_pending_proven_not_submitted_preserves_phase_and_manual_frozen_argv() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            not_submitted: true,
            ..Default::default()
        };
        let mut bytes = vec![];
        expect_api_error(
            downstream(&f, &mut launcher, &mut bytes),
            ErrorCode::HostUnavailable,
            "adapter proven no native submission",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let frame: serde_json::Value =
            serde_json::from_slice(&bytes).expect("admitted refusal must report known progress");
        let report = &frame["bootstrap"];
        assert_eq!(report["phase"], "launch");
        assert_eq!(report["outcome"], "pending");
        assert_eq!(report["possible_start"], false);
        assert!(report["thread"].is_string());
        assert!(!report["message"].is_null());
        let manual = report["manual_launch_after_confirming_no_start_argv"]
            .as_array()
            .unwrap();
        assert!(manual.iter().any(|arg| arg == "saved value"));
        assert_eq!(f.peer.status().state, BootstrapState::Attached);
        assert!(child_progress_path(&f.journal, &f.peer.reference).exists());
        assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn final_sdd_pending_write_and_flush_loss_preserve_possible_start_and_retry() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for write in [true, false] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher {
                unknown: true,
                ..Default::default()
            };
            let mut output = FailingOutput {
                bytes: vec![],
                write,
            };
            let error = downstream(&f, &mut launcher, &mut output).unwrap_err();
            assert!(
                matches!(error, RunError::Io(_)),
                "pending reporting must exercise writer: {error:?}"
            );
            if !write {
                assert!(!output.bytes.is_empty());
            }
            assert_eq!(f.peer.status().state, BootstrapState::Attached);
            assert!(child_progress_path(&f.journal, &f.peer.reference).exists());
            assert!(
                f.journal
                    .root()
                    .join(format!("{}.intent", f.peer.reference.operation.as_str()))
                    .exists()
                    || load_original(&f.journal, &f.peer.reference).is_ok()
            );
            assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
            launcher.unknown = false;
            let mut bytes = vec![];
            unknown(downstream(&f, &mut launcher, &mut bytes));
            let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(frame["bootstrap"]["outcome"], "possible_start");
            assert_downstream_once(&f, &launcher);
        }
    }

    fn downstream_format<W: std::io::Write>(
        f: &Fixture,
        launcher: &mut dyn super::super::super::handoff::HandoffLauncher,
        json: bool,
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
                format: if json {
                    crate::protocol::output::OutputFormat::Json
                } else {
                    crate::protocol::output::OutputFormat::Text
                },
                ..Default::default()
            },
            writer,
        )
    }
    // A successful canonical zero-submission transition is still unfinished
    // handoff work: the actual writer must report the canonical next attempt.
    fn assert_prepared_next_attempt_report(f: &Fixture, json: bool, bytes: &[u8]) {
        let reference = f.peer.reference.recovery_ref();
        let identity = &f.peer.identity;
        if json {
            let frame: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| {
                panic!(
                    "missing useful pending report: {e}; stdout={}",
                    String::from_utf8_lossy(bytes)
                )
            });
            let report = &frame["bootstrap"];
            assert_eq!(report["phase"], "creation");
            assert_eq!(report["failed"], true);
            assert_eq!(report["outcome"], "pending");
            assert_eq!(report["attempt"], 2);
            assert_eq!(report["state"], "prepared");
            assert_eq!(report["status_is_last_observed"], true);
            assert_eq!(report["recovery_ref"], reference);
            assert_eq!(
                report["bootstrap_compound"],
                identity.compound.as_str(),
                "exact original compound"
            );
            assert_eq!(
                report["namespace"],
                serde_json::to_value(&identity.payload.handoff.namespace).unwrap()
            );
            for absent in [
                "creation",
                "tab",
                "pane",
                "seat",
                "child_compound",
                "thread",
                "invitation",
                "message",
                "manual_launch_after_confirming_no_start_argv",
                "conditional_human_recovery",
            ] {
                assert!(
                    report[absent].is_null(),
                    "fabricated {absent}: {}",
                    report[absent]
                );
            }
            assert_eq!(report["possible_start"], false);
            let retry: Vec<String> = serde_json::from_value(report["retry_argv"].clone()).unwrap();
            assert_eq!(retry[0], "env");
            assert_eq!(
                retry[1],
                format!("HERDR_PANE_ID={}", identity.claim.target.as_str())
            );
            assert_eq!(retry[retry.len() - 2..], ["retry".to_owned(), reference]);
            let ns = &identity.payload.handoff.namespace;
            assert!(retry.contains(&ns.state_dir.to_string_lossy().into_owned()));
            assert!(retry.contains(&ns.host_endpoint.to_string_lossy().into_owned()));
            assert!(!String::from_utf8_lossy(bytes).contains(&identity.payload.handoff.body));
        } else {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(text.contains("phase: creation"), "{text}");
            assert!(text.contains("outcome: pending"), "{text}");
            assert!(text.contains("attempt: 2"), "{text}");
            assert!(text.contains("state: prepared"), "{text}");
            assert!(text.contains("retry_argv: env HERDR_PANE_ID="), "{text}");
            assert!(text.contains(&format!("retry {reference}")), "{text}");
        }
        assert!(bytes.len() <= 1024 * 1024);
    }
    // A proven zero-submission rearm is retryable unfinished work with valid
    // arguments: never a usage error (exit 2), always the documented retryable
    // unavailable class (exit 3), with the retry remedy kept in the message.
    fn assert_retryable_rearm_error(error: &RunError) {
        let RunError::Api(api) = error else {
            panic!("rearm must be a typed API error: {error:?}");
        };
        assert_eq!(
            api.code,
            crate::protocol::results::ErrorCode::HostUnavailable,
            "{error:?}"
        );
        assert_eq!(error.exit_code(), crate::cli::exit::EXIT_UNAVAILABLE);
        assert!(
            api.detail.contains("retry the original to continue"),
            "{error:?}"
        );
        assert!(api.detail.contains("no automatic submission"), "{error:?}");
    }
    fn assert_no_downstream_or_resubmission(f: &Fixture, launcher: &DownstreamLauncher) {
        let s = f.peer.state.lock().unwrap();
        assert_eq!(s.native_calls, 1, "reporting must not submit attempt N+1");
        assert_eq!(launcher.starts, 0);
        assert!(
            !s.calls.iter().any(|c| matches!(
                *c,
                "resolve" | "attach" | "child_begin" | "create_child" | "linked_complete"
            )),
            "no downstream work after zero submission: {:?}",
            s.calls
        );
        drop(s);
        let current = f.peer.status();
        assert_eq!(current.state, BootstrapState::Prepared);
        assert_eq!(current.attempt.get(), 2);
        assert!(current.creation.is_none() && current.attachment.is_none());
    }
    #[test]
    fn pending_writer_fresh_not_submitted_reports_next_prepared() {
        let mut f = Fixture::downstream(Fault::NotSubmitted);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for json in [true, false] {
            f.restore(&fresh);
            let original = f
                .journal
                .snapshot_bootstrap_origin(&f.peer.reference)
                .unwrap();
            let mut launcher = DownstreamLauncher::default();
            let mut bytes = vec![];
            let error = downstream_format(&f, &mut launcher, json, &mut bytes).unwrap_err();
            assert_retryable_rearm_error(&error);
            assert_prepared_next_attempt_report(&f, json, &bytes);
            assert_no_downstream_or_resubmission(&f, &launcher);
            assert_eq!(
                f.journal
                    .snapshot_bootstrap_origin(&f.peer.reference)
                    .unwrap(),
                original
            );
            assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
            // A separate explicit original retry advances N+1 under its own keys.
            downstream_format(&f, &mut launcher, json, &mut vec![]).unwrap();
            assert_downstream_once_after_rearm(&f, &launcher);
        }
    }
    fn assert_downstream_once_after_rearm(f: &Fixture, launcher: &DownstreamLauncher) {
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 2);
        assert_eq!(launcher.starts, 1);
        assert_eq!(f.peer.status().state, BootstrapState::Completed);
        assert_eq!(f.peer.status().attempt.get(), 2);
    }
    #[test]
    fn pending_writer_saved_not_submitted_reports_next_prepared() {
        let mut f = Fixture::downstream(Fault::NotSubmittedRecordUnavailable);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for json in [true, false] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher::default();
            assert!(downstream_format(&f, &mut launcher, json, &mut vec![]).is_err());
            assert_eq!(f.peer.status().state, BootstrapState::PossibleCreation);
            let request = std::fs::read(f.peer.progress()).unwrap();
            f.peer.state.lock().unwrap().calls.clear();
            let mut bytes = vec![];
            let error = downstream_format(&f, &mut launcher, json, &mut bytes).unwrap_err();
            assert_retryable_rearm_error(&error);
            assert_prepared_next_attempt_report(&f, json, &bytes);
            assert_no_downstream_or_resubmission(&f, &launcher);
            assert!(
                !f.peer
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .iter()
                    .any(|c| matches!(*c, "reserve" | "check")),
                "saved typed zero submission must not reserve again"
            );
            // The genuine retained request remains historical evidence only.
            assert_eq!(std::fs::read(f.peer.progress()).unwrap(), request);
            downstream_format(&f, &mut launcher, json, &mut vec![]).unwrap();
            assert_downstream_once_after_rearm(&f, &launcher);
        }
    }
    #[test]
    fn pending_writer_rearm_report_write_and_flush_failure_stays_replayable() {
        let mut f = Fixture::downstream(Fault::NotSubmitted);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for write in [true, false] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher::default();
            let mut output = FailingOutput {
                bytes: vec![],
                write,
            };
            let error = downstream(&f, &mut launcher, &mut output).unwrap_err();
            assert!(matches!(error, RunError::Io(_)), "{error:?}");
            if !write {
                assert!(!output.bytes.is_empty(), "flush loss reached the writer");
            }
            assert_no_downstream_or_resubmission(&f, &launcher);
            assert!(load_original(&f.journal, &f.peer.reference).is_ok());
            assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            assert_downstream_once_after_rearm(&f, &launcher);
        }
    }
    #[test]
    fn pending_writer_completion_failure_labels_last_observed_attached_as_uncertain() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for json in [true, false] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher::default();
            let mut bytes = vec![];
            expect_api_error(
                downstream_format(&f, &mut launcher, json, &mut bytes),
                ErrorCode::HostUnavailable,
                "crash before wrapper deciding transaction",
            );
            assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
            if json {
                let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let report = &frame["bootstrap"];
                assert_eq!(report["phase"], "complete");
                assert_eq!(report["outcome"], "completion_pending");
                assert_eq!(report["completion_uncertain"], true);
                assert_eq!(report["state"], "attached");
                assert_eq!(report["status_is_last_observed"], true);
                assert!(report["thread"].is_string());
                assert!(report["manual_launch_after_confirming_no_start_argv"].is_null());
            } else {
                let text = String::from_utf8(bytes).unwrap();
                assert!(text.contains("phase: complete"), "{text}");
                assert!(text.contains("completion_uncertain: true"), "{text}");
            }
            assert!(!terminal_path(&f.journal, &f.peer.reference).exists());
            downstream(&f, &mut launcher, &mut vec![]).unwrap();
            assert_downstream_once(&f, &launcher);
        }
    }
    #[test]
    fn pending_writer_authority_and_namespace_refusals_stay_silent() {
        let f = Fixture::downstream(Fault::Check);
        let mut bytes = vec![];
        let error = downstream(&f, &mut DownstreamLauncher::default(), &mut bytes).unwrap_err();
        assert!(bytes.is_empty(), "authority refusal reported: {error:?}");
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 0);
        let f = Fixture::downstream(Fault::NotSubmitted);
        let mut foreign = f.peer.identity.payload.handoff.namespace.clone();
        foreign.state_dir = foreign.state_dir.join("copied");
        let mut bytes = vec![];
        let result = super::super::super::retry::run_bootstrap_retry_to_writer(
            &f.journal,
            &f.peer.reference,
            super::super::super::actor_route::InvocationActor::Agent,
            &foreign,
            f.peer.as_ref(),
            f.peer.as_ref(),
            &mut DownstreamLauncher::default(),
            &f.clock,
            BootstrapSubmissionInputs {
                witness: &f.witness,
                context: &f.context,
            },
            &crate::protocol::output::OutputSpec::default(),
            &mut bytes,
        );
        assert!(result.is_err());
        assert!(bytes.is_empty());
        let s = f.peer.state.lock().unwrap();
        assert_eq!(s.native_calls, 0);
        assert!(s.calls.is_empty(), "{:?}", s.calls);
    }

    // The emitted inspection continuation must be the public read-only
    // pending-ops command under the exact original pinned routing; the same
    // public parser the binary uses decides, for both JSON argv and text.
    fn assert_inspect_is_public_pending_ops(f: &Fixture, json: bool, bytes: &[u8]) {
        let ns = &f.peer.identity.payload.handoff.namespace;
        let state = ns.state_dir.to_string_lossy().into_owned();
        let host = ns.host_endpoint.to_string_lossy().into_owned();
        assert!(state.contains(' '), "fixture pins a state dir with spaces");
        let expected: Vec<String> = vec![
            crate::cli::hook::CLI_ARGV0.into(),
            "--state-dir".into(),
            state.clone(),
            "--host-endpoint".into(),
            host.clone(),
            "pending-ops".into(),
        ];
        let argv: Vec<String> = if json {
            let frame: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| {
                panic!("pending report: {e}; {}", String::from_utf8_lossy(bytes))
            });
            serde_json::from_value(frame["bootstrap"]["inspect_argv"].clone()).unwrap()
        } else {
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            let line = text
                .lines()
                .find_map(|l| l.strip_prefix("inspect_argv: "))
                .unwrap_or_else(|| panic!("no inspect_argv line: {text}"));
            shlex::split(line).unwrap_or_else(|| panic!("unquotable: {line}"))
        };
        let parsed = crate::cli::commands::parse_argv(&argv)
            .unwrap_or_else(|e| panic!("public parser refused inspect_argv {argv:?}: {e:?}"));
        assert!(
            matches!(
                parsed.action,
                crate::cli::commands::CliAction::PendingOps(_)
            ),
            "inspect_argv is not the read-only pending-ops action: {argv:?}"
        );
        assert_eq!(
            parsed.actor,
            crate::cli::actor_route::InvocationActor::Agent,
            "inspection needs no human namespace"
        );
        assert_eq!(
            parsed.output.context.state_dir.as_deref(),
            Some(state.as_str())
        );
        assert_eq!(parsed.output.context.host.as_deref(), Some(host.as_str()));
        assert_eq!(argv, expected);
    }
    #[test]
    fn pending_writer_inspect_argv_parses_as_public_pending_ops() {
        // Each distinct fresh fixture is built once; every scenario, in both
        // the JSON and text renderings, runs from an exact restore of it.
        let fixtures: Vec<(Fixture, Checkpoint)> = [
            Fixture::downstream(Fault::None),
            Fixture::new(Fault::HostReply),
            Fixture::downstream(Fault::NotSubmitted),
            Fixture::downstream(Fault::CompleteBefore),
        ]
        .into_iter()
        .map(|mut f| {
            // Setup only: host load must not expire a 250 ms child permit;
            // the subject is not the permit (see HeldStoreClock).
            f.hold_store_clock();
            let fresh = f.checkpoint();
            (f, fresh)
        })
        .collect();
        let scenario = |n: usize| {
            let (f, fresh) = &fixtures[n];
            f.restore(fresh);
            f
        };
        for json in [true, false] {
            // Canonical status unknown to this invocation (actual writer, real identity).
            let f = scenario(0);
            let mut bytes = vec![];
            super::super::write_pending_observed(
                &f.peer.reference,
                &f.peer.identity,
                None,
                None,
                "creation",
                &crate::protocol::output::OutputSpec {
                    format: if json {
                        crate::protocol::output::OutputFormat::Json
                    } else {
                        crate::protocol::output::OutputFormat::Text
                    },
                    ..Default::default()
                },
                &mut bytes,
            )
            .unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("status_unknown"));
            assert_inspect_is_public_pending_ops(f, json, &bytes);

            // Lost host reply: PossibleCreation without creation evidence.
            let f = scenario(1);
            let mut bytes = vec![];
            unknown(downstream_format(
                f,
                &mut DownstreamLauncher::default(),
                json,
                &mut bytes,
            ));
            assert!(String::from_utf8_lossy(&bytes).contains("creation_unknown"));
            assert_inspect_is_public_pending_ops(f, json, &bytes);
            if json {
                // The human recovery templates carry the same pinned routing.
                let frame: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let report = &frame["bootstrap"];
                let inspect: Vec<String> =
                    serde_json::from_value(report["inspect_argv"].clone()).unwrap();
                let human: Vec<String> = serde_json::from_value(
                    report["conditional_human_recovery"]["created_pane_argv"].clone(),
                )
                .unwrap();
                assert_eq!(human[1], "human");
                assert_eq!(human[2..6], inspect[1..5]);
            }

            // Typed zero submission: canonical next attempt Prepared.
            let f = scenario(2);
            let mut bytes = vec![];
            assert!(
                downstream_format(f, &mut DownstreamLauncher::default(), json, &mut bytes).is_err()
            );
            assert_prepared_next_attempt_report(f, json, &bytes);
            assert_inspect_is_public_pending_ops(f, json, &bytes);

            // Attached partial: proven launch refusal after attachment.
            let f = scenario(0);
            let mut launcher = DownstreamLauncher {
                not_submitted: true,
                ..Default::default()
            };
            let mut bytes = vec![];
            expect_api_error(
                downstream_format(f, &mut launcher, json, &mut bytes),
                ErrorCode::HostUnavailable,
                "adapter proven no native submission",
            );
            assert_eq!(child_calls(f), ["invite_child", "send_child"]);
            assert_eq!(f.peer.status().state, BootstrapState::Attached);
            assert!(String::from_utf8_lossy(&bytes).contains("outcome"));
            assert_inspect_is_public_pending_ops(f, json, &bytes);

            // Possible start.
            let f = scenario(0);
            let mut launcher = DownstreamLauncher {
                unknown: true,
                ..Default::default()
            };
            let mut bytes = vec![];
            expect_api_error(
                downstream_format(f, &mut launcher, json, &mut bytes),
                ErrorCode::UnknownOutcome,
                "downstream possible start",
            );
            assert_eq!(child_calls(f), ["invite_child", "send_child"]);
            assert!(String::from_utf8_lossy(&bytes).contains("possible_start"));
            assert_inspect_is_public_pending_ops(f, json, &bytes);

            // Completion pending.
            let f = scenario(3);
            let mut bytes = vec![];
            expect_api_error(
                downstream_format(f, &mut DownstreamLauncher::default(), json, &mut bytes),
                ErrorCode::HostUnavailable,
                "crash before wrapper deciding transaction",
            );
            assert_eq!(child_calls(f), ["invite_child", "send_child"]);
            assert!(String::from_utf8_lossy(&bytes).contains("completion_pending"));
            assert_inspect_is_public_pending_ops(f, json, &bytes);
        }
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for write in [false, true] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher::default();
            let mut failed = FailingOutput {
                bytes: vec![],
                write,
            };
            expect_io_error(
                downstream(&f, &mut launcher, &mut failed),
                std::io::ErrorKind::Other,
                if write {
                    "injected output write loss"
                } else {
                    "injected output flush loss"
                },
            );
            assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
            assert_eq!(f.peer.status().state, BootstrapState::Completed);
            let original = f
                .journal
                .snapshot_bootstrap_origin(&f.peer.reference)
                .unwrap();
            let mut state = f.peer.state.lock().unwrap();
            state.db.execute_batch("UPDATE threads SET archived=1; DELETE FROM memberships WHERE seat_id='sender'; UPDATE occupant_bindings SET execution_id='changed',generation=generation+1 WHERE seat_id='sender'; UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id='sender'").unwrap();
            state.db =
                crate::test_support::open_test_connection(&f._temp.path().join("canonical.db"));
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        expect_io_error(
            downstream(&f, &mut launcher, &mut vec![]),
            std::io::ErrorKind::AlreadyExists,
            "File exists",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for oversized in [false, true] {
            f.restore(&fresh);
            let mut launcher = DownstreamLauncher {
                invalid_report: !oversized,
                oversized_report: oversized,
                ..Default::default()
            };
            expect_api_error(
                downstream(&f, &mut launcher, &mut vec![]),
                ErrorCode::InvalidRequest,
                "invalid retained successful launch report",
            );
            assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        let mut f = Fixture::with_request(Fault::None, request(), Some(large_escaped_argv()));
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let original = f
            .journal
            .snapshot_bootstrap_origin(&f.peer.reference)
            .unwrap();
        assert!(original.len() > 65536);
        assert!(
            original.len() - LARGE_ARGV_ESCAPES < 65536,
            "escaping is what carries the origin past the delivery bound"
        );
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher {
            change_binding_before_gate: Some(f._temp.path().join("canonical.db")),
            ..Default::default()
        };
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::CallerUnverified,
            "cooperative occupant context changed",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let fresh = f.checkpoint();
        for mode in ["child", "intent", "sync"] {
            f.restore(&fresh);
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
        // One completed handoff is the shared, not-under-test setup; each
        // damage case starts from an exact restore of it.
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        assert_eq!(launcher.starts, 1);
        let completed = f.checkpoint();
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
            f.restore(&completed);
            let mut launcher = DownstreamLauncher {
                starts: 1,
                ..Default::default()
            };
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
        assert_restored_completion_replays(&f, &completed);
    }
    /// Control for checkpointed matrices: the undamaged restored setup
    /// replays its completed history, so each refusal above is the damage's.
    fn assert_restored_completion_replays(f: &Fixture, completed: &Checkpoint) {
        f.restore(completed);
        f.peer.state.lock().unwrap().calls.clear();
        let mut launcher = DownstreamLauncher {
            starts: 1,
            ..Default::default()
        };
        downstream(f, &mut launcher, &mut vec![]).unwrap();
        assert!(
            f.peer
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|r| *r == "status")
        );
        assert_eq!(launcher.starts, 1);
    }
    #[test]
    fn downstream_terminal_namespace_mismatch_refuses_even_after_cleanup() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        downstream(&f, &mut launcher, &mut vec![]).unwrap();
        let completed = f.checkpoint();
        for field in ["instance", "state", "host", "state-spelling"] {
            f.restore(&completed);
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
        assert_restored_completion_replays(&f, &completed);
    }
    #[test]
    fn downstream_retained_terminal_requires_exact_canonical_completed_report() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]).unwrap();
        let completed = f.checkpoint();
        for damage in [
            "missing-report",
            "report-corrupt",
            "report-mismatch",
            "nonterminal",
            "unavailable",
        ] {
            f.restore(&completed);
            let mut launcher = DownstreamLauncher {
                starts: 1,
                ..Default::default()
            };
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
        assert_restored_completion_replays(&f, &completed);
    }
    #[test]
    fn downstream_partial_terminal_with_live_original_refuses_before_any_effect() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let path = terminal_path(&f.journal, &f.peer.reference);
        std::fs::write(&path, b"{partial").unwrap();
        f.peer.state.lock().unwrap().calls.clear();
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert!(f.peer.state.lock().unwrap().calls.is_empty());
        assert_downstream_once(&f, &launcher);
    }
    #[test]
    fn downstream_child_progress_corruption_or_oversize_never_clears_launch_fence() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        expect_api_error(
            downstream(&f, &mut DownstreamLauncher::default(), &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let pending = f.checkpoint();
        for damage in ["unknown", "identity", "oversized"] {
            f.restore(&pending);
            let mut launcher = DownstreamLauncher {
                starts: 1,
                ..Default::default()
            };
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
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
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
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
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
    fn phase13_operator_created_keeps_original_request_and_allows_second_agent_retry() {
        let f = Fixture::new(Fault::HostReply);
        let fresh = f.checkpoint();
        for retained_request in [true, false] {
            f.restore(&fresh);
            unknown(f.run());
            let old = load_bootstrap_progress(&f.journal, &f.peer.reference, &f.peer.identity)
                .unwrap()
                .unwrap();
            let original_request = old.request.clone().unwrap();
            let mut local = old;
            if !retained_request {
                local.request = None;
            }
            save_bootstrap_progress(&f.journal, &f.peer.reference, &local).unwrap();
            let mut evidence = crate::protocol::handoff::topology_contract_tests::created();
            evidence.witness = f.witness.clone();
            evidence.witness.socket.inode += 1;
            evidence.correlation = if retained_request {
                original_request.correlation.clone()
            } else {
                HostCallId::new("operator-assertion-correlation")
            };
            let (_, observation, admission, _) = scoped_current(&f);
            let guard = crate::ports::BootstrapAttachmentGuard::try_new(
                &crate::protocol::commands::ResolveSeat {
                    host: crate::test_support::test_host_locator(),
                    target: evidence.root_pane.clone(),
                    operation: f.peer.identity.payload.resolve_key.clone(),
                },
                crate::ports::BootstrapPaneObservation::try_new(
                    observation,
                    evidence.workspace.clone(),
                    evidence.tab.clone(),
                    evidence.witness.clone(),
                )
                .unwrap(),
                &admission,
            )
            .unwrap();
            let mut recovery = RecoverBootstrap {
                inspection: Some(
                    BootstrapRecoveryInspection::from_status(&{
                        let state = f.peer.state.lock().unwrap();
                        canonical::current(
                            &state.db,
                            &f.peer.identity.payload.handoff.namespace,
                            &f.peer.identity,
                        )
                        .unwrap()
                        .unwrap()
                    })
                    .unwrap(),
                ),
                identity: f.peer.identity.clone(),
                expected_attempt: BootstrapAttempt::first(),
                operation: OperationId::new("placeholder"),
                disposition: BootstrapRecoveryDisposition::CreatedPane {
                    evidence,
                    structural_reference: HostCallId::new("earlier-operator-read"),
                },
            };
            recovery.operation = recovery.decision_operation().unwrap();
            {
                let mut state = f.peer.state.lock().unwrap();
                let tx = state.db.transaction().unwrap();
                canonical::attempts::recover(
                    &tx,
                    &f.peer.identity.payload.handoff.namespace,
                    &recovery,
                    501,
                    UtcMillis(3),
                    Some(&guard),
                )
                .unwrap();
                tx.commit().unwrap();
                state.fault = Fault::None;
            }
            assert_eq!(f.run().unwrap().state, BootstrapState::Attached);
            assert_eq!(
                f.run().unwrap().state,
                BootstrapState::Attached,
                "second retry must parse its retained original native submission proof"
            );
            let saved = load_bootstrap_progress(&f.journal, &f.peer.reference, &f.peer.identity)
                .unwrap()
                .unwrap();
            assert_eq!(saved.request, retained_request.then_some(original_request));
            assert!(
                saved.creation.is_none(),
                "operator canonical evidence cannot fabricate a local native submission receipt"
            );
            assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
        }
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
        let db = crate::test_support::open_test_connection(&root.join("canonical.db"));
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
            verify_wire: false,
            store_clock: None,
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
        let f = Fixture::new(Fault::MissingCapability);
        let fresh = f.checkpoint();
        for fault in [
            Fault::MissingCapability,
            Fault::OldCapability,
            Fault::WrongCapability,
        ] {
            f.restore(&fresh);
            f.peer.state.lock().unwrap().fault = fault;
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
        let f = Fixture::new(Fault::MovedResolveTab);
        let fresh = f.checkpoint();
        for fault in [Fault::MovedResolveTab, Fault::MovedAttachTab] {
            f.restore(&fresh);
            f.peer.state.lock().unwrap().fault = fault;
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
        let f = Fixture::new(Fault::MissingCapability);
        let fresh = f.checkpoint();
        for damage in ["malformed", "missing", "ambiguous"] {
            f.restore(&fresh);
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
        )
        .with_test_host();
        let budget = super::super::super::cooperative_budget(context.clock());
        let mut s = peer.state.lock().unwrap();
        let admission = crate::store::seats::begin_host_observation(
            &context,
            &mut s.db,
            "34750f98bd59fcfc946da45aaabe933b",
            crate::test_support::test_host_id(),
            &budget,
        )
        .unwrap();
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
                identity: "herdr-server:pid=42:start=1.000002:uid=501".into(),
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
                host: crate::test_support::test_host_locator(),
                target: observation.target.clone(),
                operation: peer.identity.payload.resolve_key.clone(),
            },
            BootstrapPaneObservation::try_new(
                observation,
                HostTargetId::new("w1"),
                HostTargetId::new(tab),
                crate::protocol::handoff::topology_contract_tests::created().witness,
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
            schema::canonical_digest(&(
                "resolve_seat",
                "34750f98bd59fcfc946da45aaabe933b",
                HostTargetId::new("w1:p2")
            ))
            .unwrap()
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
            host: crate::test_support::test_host_locator(),
            target: HostTargetId::new("w1:p2"),
            operation: OperationId::new("preexisting-fixture"),
        };
        let guard = OrdinaryResolutionGuard::try_new(&request, observation, &admission).unwrap();
        let OrdinaryResolutionOutcome::Resolved(seat) = crate::store::seats::resolve_seat(
            &ctx,
            &mut f.peer.state.lock().unwrap().db,
            "34750f98bd59fcfc946da45aaabe933b",
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
                "UPDATE herdr_hosts SET lifecycle_revision=lifecycle_revision+1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
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
                "UPDATE herdr_hosts SET lifecycle_revision=lifecycle_revision+1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
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
        f.peer.state.lock().unwrap().db.execute_batch("CREATE TEMP TRIGGER extra_lifecycle AFTER INSERT ON allocation_decisions BEGIN UPDATE herdr_hosts SET lifecycle_revision=lifecycle_revision+1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'; END;").unwrap();
        assert!(
            resolve(&f, &resolve_request(&f), &ctx, &guard, &budget).is_err(),
            "unexpected own-transaction lifecycle bump committed"
        );
        assert_eq!(counts(&f), (0, 0, 0));
    }
    #[test]
    fn live_preallocation_a2_namespace_claim_and_race_controls_zero_effect() {
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let fresh = f.checkpoint();
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
            f.restore(&fresh);
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
                    state.db.execute("UPDATE occupant_bindings SET native_session='changed-session' WHERE seat_id='sender'", []).unwrap();
                }
                "namespace" => namespace.host_endpoint = f._temp.path().join("different.sock"),
                "claim" => {
                    request.identity.claim.execution =
                        ExecutionId::new("00000000-0000-4000-8000-000000000002");
                    request.identity.digest = request.identity.semantic_digest().unwrap();
                }
                "key" => request.operation = OperationId::new("wrong-resolve"),
                "hold" => {
                    state.db.execute("INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p2',?1,1,'test hold')", ["herdr-server:pid=42:start=1.000002:uid=501"]).unwrap();
                }
                "admission" => {
                    crate::store::seats::begin_host_observation(
                        &ctx,
                        &mut state.db,
                        "34750f98bd59fcfc946da45aaabe933b",
                        crate::test_support::test_host_id(),
                        &budget,
                    )
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
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let fresh = f.checkpoint();
        for change in ["workspace", "terminal"] {
            f.restore(&fresh);
            let (ctx, mut observation, admission, budget) = scoped_current(&f);
            if change == "terminal" {
                observation.terminal = Some(TerminalId::new("different-terminal"));
            }
            let response = BootstrapPaneObservation::try_new(
                observation,
                HostTargetId::new(if change == "workspace" { "w9" } else { "w1" }),
                HostTargetId::new("w1:t2"),
                crate::protocol::handoff::topology_contract_tests::created().witness,
            );
            if change == "workspace" {
                assert!(response.is_err());
                assert_eq!(counts(&f), (0, 0, 0));
                continue;
            }
            let response = response.unwrap();
            let guard = BootstrapAttachmentGuard::try_new(
                &crate::protocol::commands::ResolveSeat {
                    host: crate::test_support::test_host_locator(),
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
        f.peer.state.lock().unwrap().db.execute_batch("CREATE TEMP TRIGGER reject_resolution BEFORE INSERT ON operations WHEN NEW.actor_scope='service-allocation:34750f98bd59fcfc946da45aaabe933b' BEGIN SELECT RAISE(ABORT,'injected late ledger failure'); END;").unwrap();
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
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let fresh = f.checkpoint();
        for (trip, cancelled) in [(1, false), (5, false), (1, true), (4, true)] {
            f.restore(&fresh);
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
            )
            .with_test_host();
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
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let fresh = f.checkpoint();
        for existing in [false, true] {
            f.restore(&fresh);
            let owner = existing.then(|| preexisting_owner(&f));
            f.peer
                .state
                .lock()
                .unwrap()
                .db
                .execute(
                    "UPDATE herdr_hosts SET lifecycle_revision=?1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
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
        let f = Fixture::new(Fault::ResolveUnsupported);
        let _ = f.run();
        let fresh = f.checkpoint();
        for change in ["archive", "binding"] {
            f.restore(&fresh);
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
                s.db.execute("UPDATE occupant_bindings SET native_session='changed-session' WHERE seat_id='sender'",[]).unwrap();
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

    #[test]
    fn current_main_linked_none_preserves_root_append_contract() {
        let mut request = request();
        request.kind = Some("claude".into());
        request.argv = vec!["--tools".into(), String::new()];
        let mut f = Fixture::with_request(
            Fault::None,
            request,
            Some(vec!["--tools".into(), String::new()]),
        );
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let attached = f.run().unwrap().attachment.unwrap();
        let plan =
            downstream_plan(&f.peer.identity, &attached.created, &attached.resolved_seat).unwrap();
        assert!(plan.startup_input.is_none());
        assert!(
            serde_json::to_value(&plan)
                .unwrap()
                .get("startup_input")
                .is_none()
        );
        assert_eq!(plan.request.launch.argv, ["--tools", ""]);
        assert_eq!(plan.request.body, "literal '$HOME' body");
        let ns = &f.peer.identity.payload.handoff.namespace;
        // Literal historical Root template from immutable2ae; never use current bootstrap/composer as expected output.
        let fallback = format!(
            "herdr-threads --state-dir '{}' --host-endpoint {} inbox",
            ns.state_dir.display(),
            ns.host_endpoint.display()
        );
        let prompt = format!(
            "Expected handoff command routing (JSON data): null Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `{fallback}`. The task for thread canonical-thread is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages."
        );
        struct LiteralLauncher {
            expected: Vec<String>,
            starts: usize,
        }
        impl super::super::super::handoff::HandoffLauncher for LiteralLauncher {
            fn preflight(
                &mut self,
                _: &super::super::super::launch::LaunchRequest,
            ) -> Result<SeatId, RunError> {
                panic!("linked attachment must not preflight a new seat")
            }
            fn launch(
                &mut self,
                request: &super::super::super::launch::LaunchRequest,
                seat: &SeatId,
                gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
            ) -> Result<super::super::super::launch::LaunchReport, RunError> {
                assert_eq!(request.harness, crate::harness::context::Harness::Claude);
                assert_eq!(
                    request.argv, self.expected,
                    "Root fixed prompt must follow original tools recipe and empty value"
                );
                gate(true)?;
                self.starts += 1;
                assert_eq!(self.starts, 1);
                Ok(super::super::super::launch::LaunchReport {
                    report: serde_json::json!({"outcome":"started","pane":request.target,"seat":seat,"harness":"claude","argv":self.expected}),
                    exit: 0,
                })
            }
        }
        let mut launcher = LiteralLauncher {
            expected: vec!["--tools".into(), String::new(), prompt],
            starts: 0,
        };
        let mut first = Vec::new();
        let result = downstream(&f, &mut launcher, &mut first).unwrap();
        assert_eq!(result.state, BootstrapState::Completed);
        assert_eq!(
            result.completed.as_ref().unwrap().retained.report["argv"],
            serde_json::json!(launcher.expected)
        );
        let frozen = f.peer.status();
        let mut second = Vec::new();
        let replay = downstream(&f, &mut launcher, &mut second).unwrap();
        assert_eq!(replay, frozen);
        assert_eq!(second, first);
        assert_eq!(launcher.starts, 1);
        assert_eq!(f.peer.state.lock().unwrap().native_calls, 1);
    }
    /// Simulated upgrade between a genuine saved successful launch and its
    /// completion/presentation: today's prompt renderer and composer change
    /// while every retained byte stays unchanged.
    fn arm_v1_drift() -> crate::harness::launch::current_drift::Armed {
        let armed = crate::harness::launch::current_drift::arm(
            Some(" Re-read the task body."),
            Some("saved value"),
        );
        assert!(
            crate::harness::launch::compose_native_argv(
                crate::protocol::authority::Harness::Codex,
                vec!["--model".into(), "saved value".into()],
                vec![],
            )
            .is_err(),
            "current composer observes the simulated drift"
        );
        armed
    }
    fn fourth_fixture(fault: Fault) -> Fixture {
        Fixture::with_request(fault, super::fourth_request(), Some(vec![]))
    }
    /// A V2 plan for a launch-capable adapter V1 refuses runs end to end through the real
    /// store: create, attach, launch with the adapter's own composition, complete.
    #[test]
    fn bootstrap_v2_synthetic_fourth_completes_end_to_end() {
        let mut f = fourth_fixture(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        assert_eq!(f.peer.identity.payload.plan_version(), 2);
        let mut launcher = DownstreamLauncher::default();
        let mut output = vec![];
        let done = downstream(&f, &mut launcher, &mut output)
            .unwrap_or_else(|e| panic!("V2 bootstrap must complete: {e:?}"));
        assert_eq!(done.state, BootstrapState::Completed);
        let report = done.completed.unwrap().retained.report;
        assert_eq!(report["harness"], "synthetic_fourth");
        let argv = report["argv"].as_array().unwrap();
        assert_eq!(argv[0], "--synthetic-only");
        assert_eq!(
            argv.len(),
            2,
            "synthetic-only marker plus the generated prompt"
        );
        assert_eq!(launcher.starts, 1);
        // Replay presents the same bytes without a second start.
        let mut replay = vec![];
        downstream(&f, &mut launcher, &mut replay).unwrap();
        assert_eq!(replay, output);
        assert_eq!(launcher.starts, 1);
    }
    /// agy runs the V2 bootstrap end to end: the retained report places the frozen prompt with
    /// `--prompt-interactive` and completion validates it against the frozen template.
    #[test]
    fn bootstrap_v2_agy_completes_end_to_end_with_prompt_interactive() {
        let mut f =
            Fixture::with_request(Fault::None, super::kind_request("agy", &[]), Some(vec![]));
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        assert_eq!(f.peer.identity.payload.plan_version(), 2);
        let mut launcher = DownstreamLauncher::default();
        let mut output = vec![];
        let done = downstream(&f, &mut launcher, &mut output)
            .unwrap_or_else(|e| panic!("agy V2 bootstrap must complete: {e:?}"));
        assert_eq!(done.state, BootstrapState::Completed);
        let report = done.completed.unwrap().retained.report;
        assert_eq!(report["harness"], "agy");
        let argv = report["argv"].as_array().unwrap();
        let flag = argv
            .iter()
            .position(|arg| arg == "--prompt-interactive")
            .unwrap_or_else(|| panic!("no --prompt-interactive in {argv:?}"));
        assert_eq!(flag + 2, argv.len(), "the prompt is the flag's value, last");
        assert!(
            argv[flag + 1]
                .as_str()
                .unwrap()
                .starts_with("Expected handoff command routing")
        );
        assert_eq!(launcher.starts, 1);
    }
    /// The V2 Root launch refuses before any native start when today's composer would launch
    /// different arguments than the adapter composition the plan froze.
    #[test]
    fn bootstrap_v2_root_launch_refuses_before_start_when_current_composition_differs() {
        let mut f = fourth_fixture(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        let _drift = crate::harness::launch::current_drift::arm_output("--drifted");
        let result = downstream(&f, &mut launcher, &mut vec![]);
        assert!(
            format!("{result:?}").contains("bootstrap plan V2"),
            "{result:?}"
        );
        assert_eq!(launcher.starts, 0, "no native start");
    }
    /// A retained V2 report is validated against the frozen composition at completion: a
    /// report whose argv the adapter would not have produced is refused.
    #[test]
    fn bootstrap_v2_completion_refuses_a_report_the_frozen_composition_did_not_produce() {
        let mut f = fourth_fixture(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut setup = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut setup, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let path = child_progress_path(&f.journal, &f.peer.reference);
        let mut saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        saved["progress"]["launch"]["argv"][0] = "--changed".into();
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        let mut launcher = DownstreamLauncher {
            starts: setup.starts,
            ..Default::default()
        };
        assert!(downstream(&f, &mut launcher, &mut vec![]).is_err());
        assert_eq!(f.peer.status().state, BootstrapState::Attached);
        assert_eq!(launcher.starts, 1, "no relaunch");
    }
    #[test]
    fn historical_v1_saved_success_completes_after_current_composition_drift() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut launcher, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let saved: ChildProgress = serde_json::from_slice(
            &std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap(),
        )
        .unwrap();
        let genuine = saved.progress.launch.clone().unwrap();
        let _drift = arm_v1_drift();
        f.peer.state.lock().unwrap().calls.clear();
        let mut first = vec![];
        let done = downstream(&f, &mut launcher, &mut first)
            .unwrap_or_else(|e| panic!("genuine V1 report refused after drift: {e:?}"));
        assert_eq!(done.state, BootstrapState::Completed);
        assert_eq!(done.completed.unwrap().retained.report, genuine);
        assert!(!f.peer.state.lock().unwrap().calls.iter().any(|r| matches!(
            *r,
            "invite_child" | "send_child" | "create_child" | "native" | "resolve" | "attach"
        )));
        assert_downstream_once(&f, &launcher);
        // Terminal-only replay after journal cleanup presents the same bytes.
        f.peer.state.lock().unwrap().calls.clear();
        let mut replay = vec![];
        downstream(&f, &mut launcher, &mut replay).unwrap();
        assert_eq!(replay, first);
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
    fn historical_v1_completed_terminal_presents_after_current_composition_drift() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        let mut first = vec![];
        downstream(&f, &mut launcher, &mut first).unwrap();
        assert!(f.journal.load(&f.peer.reference).is_err());
        let terminal = std::fs::read(terminal_path(&f.journal, &f.peer.reference)).unwrap();
        let _drift = arm_v1_drift();
        f.peer.state.lock().unwrap().calls.clear();
        let mut replay = vec![];
        downstream(&f, &mut launcher, &mut replay)
            .unwrap_or_else(|e| panic!("completed V1 terminal refused after drift: {e:?}"));
        assert_eq!(replay, first);
        assert_eq!(
            std::fs::read(terminal_path(&f.journal, &f.peer.reference)).unwrap(),
            terminal
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
    #[test]
    fn root_launch_refuses_before_start_when_current_composition_differs_from_v1() {
        let mut f = Fixture::downstream(Fault::None);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut launcher = DownstreamLauncher::default();
        let _drift = crate::harness::launch::current_drift::arm_output("--drifted");
        let mut output = vec![];
        let result = downstream(&f, &mut launcher, &mut output);
        assert!(
            format!("{result:?}").contains("a new bootstrap plan version is required"),
            "{result:?}"
        );
        assert_eq!(launcher.starts, 0, "no native start");
        let child: ChildProgress = serde_json::from_slice(
            &std::fs::read(child_progress_path(&f.journal, &f.peer.reference)).unwrap(),
        )
        .unwrap();
        assert!(!child.progress.possible_start);
        assert!(child.progress.launch.is_none());
        let report: serde_json::Value = serde_json::from_slice(&output)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output)));
        let report = report.get("bootstrap").unwrap_or(&report);
        let manual = report["manual_launch_after_confirming_no_start_argv"]
            .as_array()
            .unwrap_or_else(|| panic!("{report}"));
        let ns = &f.peer.identity.payload.handoff.namespace;
        let context = crate::protocol::output::ContinuationContext {
            state_dir: Some(ns.state_dir.to_string_lossy().into_owned()),
            host: Some(ns.host_endpoint.to_string_lossy().into_owned()),
        };
        let thread = child.progress.thread.as_ref().unwrap();
        assert_eq!(
            manual.last().unwrap(),
            &serde_json::json!(super::super::super::handoff::bootstrap_v1::prompt(
                thread.as_str(),
                &context,
                &f.peer.identity.claim.instance,
            ))
        );
        assert!(!manual.iter().any(|token| token == "--drifted"));
    }
    #[test]
    fn historical_v1_reply_loss_and_terminal_save_loss_recover_after_drift() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut setup = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut setup, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let pending = f.checkpoint();
        for fault in [Fault::CompleteReply, Fault::TerminalSave] {
            f.restore(&pending);
            let mut launcher = DownstreamLauncher {
                starts: setup.starts,
                ..Default::default()
            };
            let _drift = arm_v1_drift();
            f.peer.state.lock().unwrap().fault = fault;
            let first = downstream(&f, &mut launcher, &mut vec![]);
            if fault == Fault::TerminalSave {
                assert!(first.is_err());
                assert_eq!(f.peer.status().state, BootstrapState::Completed);
                std::fs::remove_dir(terminal_path(&f.journal, &f.peer.reference)).unwrap();
                downstream(&f, &mut launcher, &mut vec![])
                    .unwrap_or_else(|e| panic!("terminal save loss after drift: {e:?}"));
            } else {
                assert_eq!(
                    first
                        .unwrap_or_else(|e| panic!("reply loss after drift: {e:?}"))
                        .state,
                    BootstrapState::Completed
                );
            }
            let mut replay = vec![];
            downstream(&f, &mut launcher, &mut replay).unwrap();
            assert!(!replay.is_empty());
            assert_downstream_once(&f, &launcher);
        }
    }
    #[test]
    fn historical_v1_corrupted_saved_report_refuses_after_drift_without_relaunch() {
        let mut f = Fixture::downstream(Fault::CompleteBefore);
        // Setup only: host load must not expire a 250 ms child permit;
        // the subject is not the permit (see HeldStoreClock).
        f.hold_store_clock();
        let mut setup = DownstreamLauncher::default();
        expect_api_error(
            downstream(&f, &mut setup, &mut vec![]),
            ErrorCode::HostUnavailable,
            "crash before wrapper deciding transaction",
        );
        assert_eq!(child_calls(&f), ["invite_child", "send_child"]);
        let pending = f.checkpoint();
        for change in ["prompt", "options"] {
            f.restore(&pending);
            let mut launcher = DownstreamLauncher {
                starts: setup.starts,
                ..Default::default()
            };
            let path = child_progress_path(&f.journal, &f.peer.reference);
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let argv = saved["progress"]["launch"]["argv"].as_array_mut().unwrap();
            match change {
                "prompt" => *argv.last_mut().unwrap() = "invented prompt".into(),
                _ => argv[1] = "changed".into(),
            }
            std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
            let _drift = arm_v1_drift();
            let mut output = vec![];
            assert!(
                downstream(&f, &mut launcher, &mut output).is_err(),
                "{change}"
            );
            assert_eq!(f.peer.status().state, BootstrapState::Attached, "{change}");
            assert_eq!(launcher.starts, 1, "{change}");
            assert_eq!(f.peer.state.lock().unwrap().native_calls, 1, "{change}");
        }
    }

    mod automatic_live {
        use super::*;
        use crate::ports::{
            BootstrapAttachmentGuard, BootstrapPaneObservation, BootstrapStorePort,
            DurableWorkAdmission, SendPreparationProgress, StorePort,
        };

        struct NoAutomaticCapability;
        impl LocalClient for NoAutomaticCapability {
            fn call_with_output(
                &self,
                command: Command,
                _: &crate::protocol::output::OutputSpec,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                self.call(command, budget)
            }
            fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
                assert!(
                    matches!(command, Command::Capabilities),
                    "no canonical Begin before capability"
                );
                Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: vec![],
                    },
                ))
            }
        }

        #[test]
        fn parsed_new_tab_unsupported_refuses_before_topology_or_journal() {
            let mut parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "new peer",
                "--kind",
                "codex",
                "--",
                "literal work",
            ])
            .unwrap();
            let super::super::super::super::commands::CliAction::TopologyHandoff(request) =
                &parsed.action
            else {
                panic!("new-tab route")
            };
            assert!(request.automatic);
            let temp = Temp::new();
            let journal =
                super::super::super::super::journal::Journal::open(temp.path().join("intents"))
                    .unwrap();
            let result = super::super::super::super::resolve_automatic_new_selectors(
                &mut parsed,
                &NoAutomaticCapability,
                &crate::app::SystemClock::new(),
                || -> Result<HostTopology, ApiError> {
                    panic!("unsupported selector must not read topology")
                },
                || -> Result<Option<HostTargetId>, ApiError> {
                    panic!("unsupported selector must not read caller pane")
                },
            );
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: crate::protocol::results::ErrorCode::Unsupported,
                    ..
                }))
            ));
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher::default();
            let result =
                super::super::super::super::topology_runtime::try_run_automatic_new_with_ports(
                    &parsed,
                    &peer.base.peer.identity.claim,
                    &journal,
                    peer.ns(),
                    &NoAutomaticCapability,
                    &peer,
                    &mut launcher,
                    &peer.base.clock,
                    peer.base._temp.path(),
                    None,
                    &mut || -> Result<HostTopology, RunError> {
                        panic!("unsupported runtime must not read topology")
                    },
                    &mut |_| panic!("unsupported runtime must not preflight"),
                    &crate::protocol::output::OutputSpec::default(),
                    &mut Vec::new(),
                );
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: crate::protocol::results::ErrorCode::Unsupported,
                    ..
                }))
            ));
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            assert_eq!(launcher.starts, 0);
            assert!(!std::fs::read_dir(journal.root()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "intent")
            }));
        }

        struct NoCreationPolicy;
        impl LocalClient for NoCreationPolicy {
            fn call_with_output(
                &self,
                command: Command,
                _: &crate::protocol::output::OutputSpec,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                self.call(command, budget)
            }
            fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
                assert!(
                    matches!(command, Command::Capabilities),
                    "old peer was called for an effect or selection"
                );
                Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: crate::protocol::capabilities::ADVERTISED
                            .iter()
                            .filter(|name| {
                                **name != crate::protocol::capabilities::ENSURE_CREATION_POLICY_V1
                            })
                            .map(|name| (*name).to_owned())
                            .collect(),
                    },
                ))
            }
        }
        #[test]
        fn automatic_old_peer_with_all_other_caps_refuses_before_selector_or_caller_allocation() {
            let mut parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "peer",
                "--kind",
                "codex",
                "--",
                "work",
            ])
            .unwrap();
            let result = super::super::super::super::resolve_automatic_new_selectors(
                &mut parsed,
                &NoCreationPolicy,
                &crate::app::SystemClock::new(),
                || -> Result<HostTopology, ApiError> { panic!("old peer selected topology") },
                || -> Result<Option<HostTargetId>, ApiError> {
                    panic!("old peer allocated caller")
                },
            );
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: crate::protocol::results::ErrorCode::Unsupported,
                    ..
                }))
            ));
        }

        #[derive(Default, Clone, Copy)]
        struct AutomaticPermitSamples {
            armed: bool,
            utc_pending: bool,
            store_entry: Option<u64>,
            first_decision_pair: Option<u64>,
            decision_pairs: usize,
        }

        /// The AutoPeer store clock. AutoPeer fixtures are not about timing,
        /// but every automatic mutation they drive issues a 250 ms cooperative
        /// permit and consumes it in the deciding transaction, both sampled on
        /// this clock. It follows real time, except that `diagnosed_permit_call`
        /// holds it from just before each permit's issuance until that
        /// permit's mutation returns, so host load cannot widen the
        /// issuance-to-decision gap: the gap is exactly `permit_window` for the
        /// role named by `window_role` (every role when `None`) and zero
        /// otherwise. After a hold, real time resumes without running
        /// backwards. Setup, sizing, budgets and every other store call run on
        /// real time. The expiry itself stays exercised (see
        /// `automatic_permit_clock_window_*`).
        ///
        /// Hang mode: while held, the writer budget and the busy handler see
        /// no time pass, so a writer deadlock inside that one mutation would
        /// hang until nextest's timeout instead of failing `DeadlineExceeded`.
        #[derive(Default)]
        struct AutomaticPermitClock {
            real: crate::app::SystemClock,
            samples: Mutex<AutomaticPermitSamples>,
            offset: std::sync::atomic::AtomicU64,
            held: Mutex<Option<u64>>,
            permit_window: std::sync::atomic::AtomicU64,
            window_role: Mutex<Option<&'static str>>,
            /// Every role whose permit window was held, in order.
            windows: Mutex<Vec<&'static str>>,
        }
        impl AutomaticPermitClock {
            /// Real monotonic time, never held (diagnostic wall timings).
            fn real_now(&self) -> u64 {
                crate::protocol::time::Clock::monotonic_now(&self.real).0
            }
            fn running(&self) -> u64 {
                self.real_now() + self.offset.load(std::sync::atomic::Ordering::SeqCst)
            }
            /// The store-visible monotonic time: held during a permit window.
            fn now(&self) -> u64 {
                let held = *self.held.lock().unwrap();
                held.unwrap_or_else(|| self.running())
            }
            /// Freezes time just before a permit's issuance.
            fn hold(&self) -> AutomaticPermitClockHold<'_> {
                let mut held = self.held.lock().unwrap();
                // Not reentrant: a nested hold would move the frozen instant.
                assert!(
                    held.is_none(),
                    "automatic permit clock hold is not reentrant"
                );
                *held = Some(self.running());
                drop(held);
                AutomaticPermitClockHold(self)
            }
            /// Moves the frozen instant by the window when `role` is targeted;
            /// returns the window applied.
            fn pass_permit_window(&self, role: &'static str) -> u64 {
                self.windows.lock().unwrap().push(role);
                let target = *self.window_role.lock().unwrap();
                if target.is_some_and(|target| target != role) {
                    return 0;
                }
                let millis = self.permit_window.load(std::sync::atomic::Ordering::SeqCst);
                if let Some(held) = self.held.lock().unwrap().as_mut() {
                    *held += millis;
                }
                millis
            }
            fn arm(&self) {
                *self.samples.lock().unwrap() = AutomaticPermitSamples {
                    armed: true,
                    ..Default::default()
                };
            }
            fn finish(&self) -> AutomaticPermitSamples {
                let mut samples = self.samples.lock().unwrap();
                samples.armed = false;
                *samples
            }
            fn store_entry(&self) {
                let now = self.now();
                let mut samples = self.samples.lock().unwrap();
                if samples.armed && samples.store_entry.is_none() {
                    samples.store_entry = Some(now);
                }
            }
        }
        /// Resumes real time when the held mutation returns, including on an
        /// early error return.
        struct AutomaticPermitClockHold<'a>(&'a AutomaticPermitClock);
        impl Drop for AutomaticPermitClockHold<'_> {
            fn drop(&mut self) {
                let clock = self.0;
                if let Some(held) = clock.held.lock().unwrap().take() {
                    let real = clock.real_now();
                    clock.offset.fetch_max(
                        held.saturating_sub(real),
                        std::sync::atomic::Ordering::SeqCst,
                    );
                }
            }
        }
        impl crate::protocol::time::Clock for AutomaticPermitClock {
            fn utc_now(&self) -> UtcMillis {
                let now = crate::protocol::time::Clock::utc_now(&self.real);
                let mut samples = self.samples.lock().unwrap();
                if samples.armed {
                    samples.utc_pending = true;
                }
                now
            }
            fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
                let now = self.now();
                let mut samples = self.samples.lock().unwrap();
                if samples.armed && samples.utc_pending {
                    samples.utc_pending = false;
                    samples.decision_pairs += 1;
                    samples.first_decision_pair.get_or_insert(now);
                }
                crate::protocol::time::MonoInstant(now)
            }
        }

        fn bounded_permit_diagnostic(args: std::fmt::Arguments<'_>, limit: usize) -> String {
            struct Bounded {
                text: String,
                limit: usize,
            }
            impl std::fmt::Write for Bounded {
                fn write_str(&mut self, text: &str) -> std::fmt::Result {
                    let mut count = text.len().min(self.limit - self.text.len());
                    while !text.is_char_boundary(count) {
                        count -= 1;
                    }
                    self.text.push_str(&text[..count]);
                    if count == text.len() {
                        Ok(())
                    } else {
                        Err(std::fmt::Error)
                    }
                }
            }
            let mut output = Bounded {
                text: String::new(),
                limit,
            };
            let _ = std::fmt::write(&mut output, args);
            output.text
        }

        struct AutoPeer {
            base: Fixture,
            identity: AutomaticBootstrapIdentity,
            active_identity: std::sync::Mutex<Option<AutomaticBootstrapIdentity>>,
            reference: super::super::super::super::journal::IntentRef,
            store: crate::store::SqliteStore,
            permit_clock: Arc<AutomaticPermitClock>,
            native_outcome_once: std::sync::Mutex<Option<&'static str>>,
            lost_child_reply: std::sync::Mutex<Option<&'static str>>,
            lost_parent_reply: std::sync::Mutex<Option<&'static str>>,
            fail_outer_complete_once: std::sync::atomic::AtomicBool,
        }
        impl AutoPeer {
            fn new() -> Self {
                Self::with_kind("codex")
            }
            fn with_kind(kind: &'static str) -> Self {
                Self::with_body(kind, "literal work")
            }
            fn with_body(kind: &'static str, body: &str) -> Self {
                Self::with_body_policy(kind, body, false)
            }
            fn with_body_policy(kind: &'static str, body: &str, historical: bool) -> Self {
                Self::with_profile(kind, body, historical, None)
            }
            fn with_profile(
                kind: &'static str,
                body: &str,
                historical: bool,
                profile: Option<std::path::PathBuf>,
            ) -> Self {
                let base = Fixture::aligned(Fault::None, None);
                base.journal.complete(&base.peer.reference).unwrap();
                let parsed = super::super::super::super::commands::parse_argv([
                    "ht",
                    "handoff",
                    "--new-tab",
                    "new peer",
                    "--kind",
                    kind,
                    "--",
                    body,
                ])
                .unwrap();
                let super::super::super::super::commands::CliAction::TopologyHandoff(mut request) =
                    parsed.action
                else {
                    panic!("parsed new-tab route")
                };
                assert!(request.automatic);
                request.profile_dir = profile;
                let mut identity = prepare_automatic(
                    &request,
                    Preparation {
                        claim: &base.peer.identity.claim,
                        namespace: &base.peer.identity.payload.handoff.namespace,
                        topology: &topology(),
                        invocation_cwd: base._temp.path(),
                        options: None,
                        client: &ReadOnly,
                        clock: &base.clock,
                    },
                )
                .unwrap();
                if historical {
                    identity.plan.payload.handoff.policy = None;
                    identity.digest = identity.semantic_digest().unwrap();
                    identity.validate().unwrap();
                }
                let reference = publish_automatic(&base.journal, &identity, 1).unwrap();
                let permit_clock = Arc::new(AutomaticPermitClock::default());
                let context = crate::store::connection::StoreContext::new(
                    base.peer.database_path.clone(),
                    permit_clock.clone(),
                )
                .with_test_host();
                let entry_clock = permit_clock.clone();
                context.set_lane_fault(Some(Arc::new(move |_| {
                    entry_clock.store_entry();
                    None
                })));
                let store = crate::store::SqliteStore::new(
                    context,
                    "34750f98bd59fcfc946da45aaabe933b",
                    Default::default(),
                )
                .unwrap();
                Self {
                    base,
                    identity,
                    active_identity: std::sync::Mutex::new(None),
                    reference,
                    store,
                    permit_clock,
                    native_outcome_once: std::sync::Mutex::new(None),
                    lost_child_reply: std::sync::Mutex::new(None),
                    lost_parent_reply: std::sync::Mutex::new(None),
                    fail_outer_complete_once: std::sync::atomic::AtomicBool::new(false),
                }
            }
            fn ns(&self) -> &HandoffNamespace {
                &self.identity.plan.payload.handoff.namespace
            }
            fn diagnosed_permit_call(
                &self,
                role: &'static str,
                started: u64,
                build_started: u64,
                input: crate::ports::CooperativePermitRequest,
                budget: &CallBudget,
                mutate: impl FnOnce(
                    crate::protocol::authority::MutationPermit,
                ) -> Result<CommandResult, ApiError>,
            ) -> Result<CommandResult, ApiError> {
                let operation = input.operation.clone();
                // Store time is held from just before issuance until the
                // mutation returns (see [`AutomaticPermitClock`]); the
                // diagnostic reports store-visible instants plus the real
                // wall time the held window took.
                let real_started = self.permit_clock.real_now();
                let hold = self.permit_clock.hold();
                let built = self.permit_clock.now();
                let issued = self.store.issue_cooperative_permit(input, budget);
                let issue_finished = self.permit_clock.now();
                let permit = match issued {
                    Ok(permit) => permit,
                    Err(error) => {
                        eprintln!(
                            "{}",
                            bounded_permit_diagnostic(
                                format_args!(
                                    "automatic-permit role={role} operation={operation:?} stage=issuance clock=held real_window_ms={} start={started} build_start={build_started} issue_start={built} issue_end={issue_finished} code={:?}",
                                    self.permit_clock.real_now() - real_started,
                                    error.code
                                ),
                                4095,
                            )
                        );
                        return Err(error);
                    }
                };
                // This fresh store-issued grant has no body-bearing requirement yet.
                // Its existing Debug exposes the actual private observed_at field;
                // a changed/truncated Debug shape yields None, never an invented age.
                let issued_metadata = bounded_permit_diagnostic(format_args!("{permit:?}"), 2048);
                let observed_at = issued_metadata
                    .split_once("observed_at: MonoInstant(")
                    .and_then(|(_, tail)| tail.split_once(')'))
                    .and_then(|(number, _)| number.parse::<u64>().ok());
                let window_ms = self.permit_clock.pass_permit_window(role);
                let mutate_started = self.permit_clock.now();
                self.permit_clock.arm();
                let result = mutate(permit);
                let finished = self.permit_clock.now();
                let samples = self.permit_clock.finish();
                drop(hold);
                let real_window_ms = self.permit_clock.real_now() - real_started;
                if let Err(error) = &result {
                    // In these synchronous AutoPeer paths the first UTC->mono pair
                    // after arm is execute_budgeted_decision_with_constraints's
                    // actual DecisionInstant. Budget/prefix reads are mono-only;
                    // later historical presentation can add another pair. This is
                    // a source-dependent observation, not a production tracing API.
                    // The clock is held, so age_ms is the store-visible age
                    // (the injected window_ms); real_window_ms is the wall time.
                    let age = samples
                        .first_decision_pair
                        .zip(observed_at)
                        .map(|(decision, observed)| i128::from(decision) - i128::from(observed));
                    let detail: String = error.detail.chars().take(128).collect();
                    eprintln!(
                        "{}",
                        bounded_permit_diagnostic(
                            format_args!(
                                "automatic-permit role={role} operation={operation:?} stage=mutation clock=held window_ms={window_ms} real_window_ms={real_window_ms} start={started} build_start={build_started} issue_start={built} issue_end={issue_finished} mutate_start={mutate_started} store_entry={:?} first_decision_pair={:?} decision_pairs={} outcome={finished} observed_at={observed_at:?} age_ms={age:?} basis=first-UTC-mono-in-synchronous-accountable-path region=namespace-and-canonical-validation-combined code={:?} detail={detail:?} issued_metadata={issued_metadata}",
                                samples.store_entry,
                                samples.first_decision_pair,
                                samples.decision_pairs,
                                error.code,
                            ),
                            4095,
                        )
                    );
                }
                result
            }
            fn status(&self) -> AutomaticBootstrapResult {
                self.status_for(&self.identity)
            }
            fn print_supported_envelope_sizes(&self, label: &str) {
                let status = self.status();
                let attachment = status.attachment.as_ref().unwrap();
                let retained = status.completed.as_ref().unwrap();
                let request = |action| crate::protocol::wire::WireRequest {
                    version: crate::protocol::wire::PROTOCOL_VERSION,
                    request_id: "sizing".into(),
                    expected_instance: self.identity.claim.instance.clone(),
                    expected_boot: None,
                    output: Some(Default::default()),
                    command: Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                        identity: self.identity.clone(),
                        action,
                    })),
                };
                let complete = request(AutomaticBootstrapAction::Complete {
                    attachment: Box::new(attachment.clone()),
                    retained: retained.clone(),
                });
                let recover = request(AutomaticBootstrapAction::Recover {
                    attempt: status.attempt,
                    inspection: status.recovery_inspection().unwrap(),
                    disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                        reason: format!("x{}", "\u{1}".repeat(4095)),
                        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                        attached_child: Some(Box::new(attachment.child.clone())),
                    },
                });
                let response = crate::protocol::wire::WireResponse {
                    version: crate::protocol::wire::PROTOCOL_VERSION,
                    request_id: "sizing".into(),
                    instance: self.identity.claim.instance.clone(),
                    daemon_boot: "00000000-0000-4000-8000-000000000003".into(),
                    result: Ok(CommandResult::AutomaticBootstrap(Box::new(status.clone()))),
                };
                eprintln!(
                    "{label}: original={} attachment={} actual_summary={} framed_complete={} framed_recover_4096escapedreason={} framed_completed_response={}",
                    serde_json::to_vec(&self.identity).unwrap().len(),
                    serde_json::to_vec(attachment).unwrap().len(),
                    serde_json::to_vec(&retained.report).unwrap().len(),
                    serde_json::to_vec(&complete).unwrap().len() + 4,
                    serde_json::to_vec(&recover).unwrap().len() + 4,
                    serde_json::to_vec(&response).unwrap().len() + 4
                );
            }

            fn complete_child_with_lost_parent_reply(&self) -> DownstreamLauncher {
                self.fail_outer_complete_once
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                let mut launcher = DownstreamLauncher::default();
                let result = resume_automatic_to_writer(
                    &self.base.journal,
                    &self.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    self.ns(),
                    self,
                    self,
                    &mut launcher,
                    &self.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &self.base.witness,
                        context: &self.base.context,
                    },
                    &crate::protocol::output::OutputSpec::default(),
                    &mut Vec::new(),
                );
                assert!(matches!(result, Err(RunError::Api(_))));
                assert_eq!(self.status().state, BootstrapState::Attached);
                launcher
            }
            fn historical_retry(
                &self,
                namespace: &HandoffNamespace,
            ) -> Result<(bool, Vec<u8>), RunError> {
                let parsed = super::super::super::super::commands::parse_argv([
                    "ht",
                    "retry",
                    self.reference.recovery_ref().as_str(),
                ])?;
                assert_eq!(
                    super::super::super::super::retry::preflight_original_actor(
                        self.base.journal.root(),
                        &self.reference.recovery_ref(),
                        parsed.actor,
                        &parsed.output.context,
                    )?,
                    super::super::super::super::journal::OriginalActor::Agent
                );
                let mut output = Vec::new();
                let completed =
                    super::super::super::super::topology_runtime::try_completed_with_namespace(
                        &parsed,
                        &self.base.journal,
                        namespace,
                        self,
                        &self.base.clock,
                        &mut output,
                    )?;
                Ok((completed, output))
            }
            fn sender_lifecycle_check_in(&self) -> crate::protocol::authority::CallerClaim {
                let mut successor = self.identity.claim.clone();
                successor.execution =
                    crate::protocol::ids::ExecutionId::new(uuid::Uuid::new_v4().to_string());
                // A lifecycle check-in records the harness process (session-keyed bindings).
                successor.liveness = Some(crate::protocol::authority::LivenessFingerprint::TEST);
                let command = crate::protocol::commands::CheckIn {
                    mode: crate::protocol::commands::CheckInMode::Lifecycle {
                        expected_binding_generation: successor.binding_generation,
                    },
                    claim: successor,
                    operation: crate::protocol::ids::OperationId::new(
                        uuid::Uuid::new_v4().to_string(),
                    ),
                    lineage_parent: None,
                };
                let budget = super::super::super::super::cooperative_budget(&self.base.clock);
                // Held from issuance until the check-in returns (see [`AutomaticPermitClock`]).
                let _hold = self.permit_clock.hold();
                let permit = self
                    .store
                    .issue_cooperative_permit(
                        crate::store::cooperative_permit_request(
                            &crate::protocol::commands::PermitMutation::CheckIn(command.clone()),
                        )
                        .unwrap(),
                        &budget,
                    )
                    .unwrap();
                let CommandResult::CheckedIn(checked) = self
                    .store
                    .register_available(
                        crate::ports::RegisterAvailableRequest {
                            command,
                            read: crate::ports::ReadContext {
                                instance: self.ns().instance.clone(),
                                output: Default::default(),
                                operation_scope: None,
                            },
                            operator: None,
                        },
                        permit,
                        &budget,
                    )
                    .unwrap()
                else {
                    panic!("sender lifecycle check-in result")
                };
                checked.context
            }
            fn status_for(
                &self,
                identity: &AutomaticBootstrapIdentity,
            ) -> AutomaticBootstrapResult {
                let request = AutomaticBootstrapMutation {
                    identity: identity.clone(),
                    action: AutomaticBootstrapAction::Status,
                };
                let CommandResult::AutomaticBootstrap(status) = self
                    .store
                    .automatic_bootstrap_query(
                        &identity.plan.payload.handoff.namespace,
                        &request,
                        &super::super::super::super::cooperative_budget(&self.base.clock),
                    )
                    .unwrap()
                else {
                    panic!("automatic parent status")
                };
                *status
            }
            fn guard_for(&self, identity: &AutomaticBootstrapIdentity) -> BootstrapAttachmentGuard {
                let (_context, observation, admission, _budget) =
                    scoped_peer_current(&self.base.peer);
                BootstrapAttachmentGuard::try_new(
                    &crate::protocol::commands::ResolveSeat {
                        host: crate::test_support::test_host_locator(),
                        target: observation.target.clone(),
                        operation: identity.plan.payload.keys.resolve.clone(),
                    },
                    BootstrapPaneObservation::try_new(
                        observation,
                        HostTargetId::new("w1"),
                        HostTargetId::new("w1:t2"),
                        self.base.witness.clone(),
                    )
                    .unwrap(),
                    &admission,
                )
                .unwrap()
            }
            fn seed_unknown_creation(&self) {
                for action in [
                    AutomaticBootstrapAction::Begin,
                    AutomaticBootstrapAction::Reserve {
                        attempt: BootstrapAttempt::first(),
                    },
                    AutomaticBootstrapAction::RecordOutcomeUnknown {
                        attempt: BootstrapAttempt::first(),
                    },
                ] {
                    self.call(
                        Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                            identity: self.identity.clone(),
                            action,
                        })),
                        &super::super::super::super::cooperative_budget(&self.base.clock),
                    )
                    .unwrap();
                }
            }
        }
        impl LocalClient for AutoPeer {
            fn call_with_output(
                &self,
                command: Command,
                _: &crate::protocol::output::OutputSpec,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                self.call(command, budget)
            }
            fn call(
                &self,
                command: Command,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                let started = self.permit_clock.now();
                match command {
                    Command::Capabilities => Ok(CommandResult::Capabilities(
                        crate::protocol::results::CapabilityList {
                            capabilities: vec![
                                crate::protocol::capabilities::HANDOFF_LAUNCH_DIRECTORIES_V1.into(),
                                crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into(),
                                crate::protocol::capabilities::ENSURE_CREATION_POLICY_V1.into(),
                                crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.into(),
                                crate::protocol::capabilities::INVITATION_CONFIRMATION_V1.into(),
                            ],
                        },
                    )),
                    Command::AutomaticBootstrap(request) => {
                        {
                            let mut active = self.active_identity.lock().unwrap();
                            assert_eq!(
                                active.get_or_insert_with(|| request.identity.clone()),
                                &request.identity
                            );
                        }
                        let role = match &request.action {
                            AutomaticBootstrapAction::Status => "auto_status",
                            AutomaticBootstrapAction::Begin => "auto_begin",
                            AutomaticBootstrapAction::Reserve { .. } => "auto_reserve",
                            AutomaticBootstrapAction::CheckSubmission { .. } => "auto_check",
                            AutomaticBootstrapAction::RecordCreated { .. } => "auto_record",
                            AutomaticBootstrapAction::Resolve { .. } => "auto_resolve",
                            AutomaticBootstrapAction::Attach(_) => "auto_attach",
                            AutomaticBootstrapAction::CheckStart => "auto_check_start",
                            AutomaticBootstrapAction::Complete { .. } => "auto_complete",
                            AutomaticBootstrapAction::RecordOutcomeUnknown { .. } => "auto_unknown",
                            AutomaticBootstrapAction::RecordNotSubmitted { .. } => {
                                "auto_not_submitted"
                            }
                            AutomaticBootstrapAction::Recover { .. } => "auto_recover",
                        };
                        self.base.peer.state.lock().unwrap().calls.push(role);
                        if matches!(request.action, AutomaticBootstrapAction::Status) {
                            return self.store.automatic_bootstrap_query(
                                &request.identity.plan.payload.handoff.namespace,
                                &request,
                                budget,
                            );
                        }
                        if matches!(request.action, AutomaticBootstrapAction::Recover { .. }) {
                            let uid = unsafe { libc::geteuid() };
                            let actor = crate::protocol::authority::OperatorActor::from_peer(
                                crate::protocol::authority::PeerIdentity::from_kernel(uid),
                                uid,
                            )
                            .unwrap();
                            let result = self.store.automatic_bootstrap_recover(
                                &request.identity.plan.payload.handoff.namespace,
                                &request,
                                &actor,
                                if matches!(
                                    request.action,
                                    AutomaticBootstrapAction::Recover {
                                        disposition:
                                            AutomaticBootstrapRecoveryDisposition::CreatedPane { .. },
                                        ..
                                    }
                                ) {
                                    Some(self.guard_for(&request.identity))
                                } else {
                                    None
                                },
                                budget,
                            )?;
                            let mut loss = self.lost_parent_reply.lock().unwrap();
                            if loss.as_ref() == Some(&role) {
                                *loss = None;
                                return Err(ApiError::unknown_outcome(
                                    "simulated committed operator decision reply loss",
                                ));
                            }
                            return Ok(result);
                        }
                        if matches!(request.action, AutomaticBootstrapAction::CheckStart) {
                            return self.store.automatic_bootstrap_check_start(
                                &request.identity.plan.payload.handoff.namespace,
                                &request,
                                self.guard_for(&request.identity),
                                budget,
                            );
                        }
                        if matches!(request.action, AutomaticBootstrapAction::Complete { .. }) {
                            if self
                                .fail_outer_complete_once
                                .swap(false, std::sync::atomic::Ordering::SeqCst)
                            {
                                return Err(ApiError::host_unavailable(
                                    "simulated outer completion delay",
                                ));
                            }
                            let result = self.store.automatic_bootstrap_complete(
                                &request.identity.plan.payload.handoff.namespace,
                                &request,
                                budget,
                            )?;
                            let mut loss = self.lost_parent_reply.lock().unwrap();
                            if loss.as_ref() == Some(&role) {
                                *loss = None;
                                return Err(ApiError::unknown_outcome(
                                    "committed completion reply lost",
                                ));
                            }
                            return Ok(result);
                        }
                        let guard = if matches!(
                            request.action,
                            AutomaticBootstrapAction::Resolve { .. }
                                | AutomaticBootstrapAction::Attach(_)
                        ) {
                            Some(self.guard_for(&request.identity))
                        } else {
                            None
                        };
                        let build_started = self.permit_clock.now();
                        let mutation =
                            crate::protocol::commands::PermitMutation::AutomaticBootstrap(
                                request.clone(),
                            );
                        let result = self.diagnosed_permit_call(
                            role,
                            started,
                            build_started,
                            crate::store::cooperative_permit_request(&mutation)?,
                            budget,
                            |permit| {
                                self.store.automatic_bootstrap_mutate(
                                    &request.identity.plan.payload.handoff.namespace,
                                    &request,
                                    permit,
                                    guard,
                                    budget,
                                )
                            },
                        );
                        let result = result?;
                        let mut loss = self.lost_parent_reply.lock().unwrap();
                        if loss.as_ref() == Some(&role) {
                            *loss = None;
                            return Err(ApiError::unknown_outcome(
                                "simulated committed parent reply loss",
                            ));
                        }
                        Ok(result)
                    }
                    Command::AutomaticHandoffDelivery(request) => {
                        let identity = self
                            .active_identity
                            .lock()
                            .unwrap()
                            .clone()
                            .expect("active parent");
                        let status = self.status_for(&identity);
                        let attachment = status
                            .attachment
                            .as_ref()
                            .expect("guarded attachment before child");
                        assert_eq!(request.scope, attachment.child.scope);
                        assert_eq!(request.claim, attachment.child.claim);
                        assert_eq!(request.original, attachment.child.original);
                        let role = match &request.action {
                            AutomaticDeliveryAction::Status => "auto_child_status",
                            AutomaticDeliveryAction::Prepare => "auto_child_prepare",
                            AutomaticDeliveryAction::Begin { .. } => "auto_child_begin",
                            AutomaticDeliveryAction::Ensure(_) => "auto_child_ensure",
                            AutomaticDeliveryAction::Send(_) => "auto_child_send",
                            AutomaticDeliveryAction::Complete { .. } => "auto_child_complete",
                        };
                        self.base.peer.state.lock().unwrap().calls.push(role);
                        if matches!(
                            request.action,
                            AutomaticDeliveryAction::Status | AutomaticDeliveryAction::Prepare
                        ) {
                            return self.store.automatic_delivery_query(
                                self.ns(),
                                &request,
                                budget,
                            );
                        }
                        if matches!(request.action, AutomaticDeliveryAction::Send(_)) {
                            loop {
                                match self.store.automatic_delivery_prepare_send_step(
                                    self.ns(),
                                    &request,
                                    DurableWorkAdmission::new(16).unwrap(),
                                    budget,
                                )? {
                                    SendPreparationProgress::Ready { .. } => break,
                                    SendPreparationProgress::More { .. } => {}
                                    SendPreparationProgress::Committed(result) => {
                                        return Ok(result);
                                    }
                                }
                            }
                        }
                        let build_started = self.permit_clock.now();
                        let inner = crate::store::handoff::automatic::inner(&request)?;
                        let result = self.diagnosed_permit_call(
                            role,
                            started,
                            build_started,
                            crate::store::cooperative_permit_request(&inner)?,
                            budget,
                            |permit| {
                                self.store.automatic_delivery_mutate(
                                    self.ns(),
                                    &request,
                                    permit,
                                    budget,
                                )
                            },
                        );
                        let result = result?;
                        let mut loss = self.lost_child_reply.lock().unwrap();
                        if loss.as_ref() == Some(&role) {
                            *loss = None;
                            return Err(ApiError::unknown_outcome(
                                "simulated committed child reply loss",
                            ));
                        }
                        Ok(result)
                    }
                    _ => panic!(
                        "new-tab automatic route must not use legacy or unrelated command: {command:?}"
                    ),
                }
            }
        }
        enum OperatorTestClient<'a> {
            Peer(&'a AutoPeer),
            Unsupported,
        }
        impl LocalClient for OperatorTestClient<'_> {
            fn call_with_output(
                &self,
                command: Command,
                output: &crate::protocol::output::OutputSpec,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                match self {
                    Self::Peer(peer) => peer.call_with_output(command, output, budget),
                    Self::Unsupported => {
                        NoAutomaticCapability.call_with_output(command, output, budget)
                    }
                }
            }
            fn call(
                &self,
                command: Command,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                match self {
                    Self::Peer(peer) => peer.call(command, budget),
                    Self::Unsupported => NoAutomaticCapability.call(command, budget),
                }
            }
        }
        impl CreateTabPort for AutoPeer {
            fn create_tab(
                &self,
                request: &CreateTabRequest,
                _: &HostCallContext,
            ) -> CreateTabOutcome {
                let mut state = self.base.peer.state.lock().unwrap();
                state.calls.push("auto_native_create");
                state.native_calls += 1;
                let identity = self
                    .active_identity
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("active parent");
                assert_eq!(request.workspace, identity.plan.payload.workspace);
                assert_eq!(request.cwd, identity.plan.payload.cwd);
                assert_eq!(request.env, identity.plan.payload.env);
                assert_eq!(request.expected_witness, self.base.witness);
                drop(state);
                match self.native_outcome_once.lock().unwrap().take() {
                    Some("zero") => {
                        return CreateTabOutcome::NotSubmitted(ApiError::host_unavailable(
                            "typed zero submission",
                        ));
                    }
                    Some("unknown") => {
                        return CreateTabOutcome::OutcomeUnknown(ApiError::unknown_outcome(
                            "typed unknown submission",
                        ));
                    }
                    None => {}
                    _ => panic!("invalid native outcome fixture"),
                }
                let mut created = crate::protocol::handoff::topology_contract_tests::created();
                created.correlation = request.correlation.clone();
                created.witness = request.expected_witness.clone();
                CreateTabOutcome::Created(Box::new(created))
            }
        }

        fn attached_recovery_fixture(
            historical: bool,
            large: bool,
        ) -> (
            AutoPeer,
            AutomaticBootstrapMutation,
            AutomaticBootstrapResult,
        ) {
            let body = if large {
                "\0".repeat(16000)
            } else {
                "work".into()
            };
            let peer = AutoPeer::with_body_policy("codex", &body, historical);
            let attempt = BootstrapAttempt::first();
            let call = |action| {
                peer.call(
                    Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                        identity: peer.identity.clone(),
                        action,
                    })),
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
            };
            call(AutomaticBootstrapAction::Begin).unwrap();
            call(AutomaticBootstrapAction::Reserve { attempt }).unwrap();
            let mut created = crate::protocol::handoff::topology_contract_tests::created();
            created.witness = peer.base.witness.clone();
            call(AutomaticBootstrapAction::RecordCreated {
                attempt,
                evidence: created.clone(),
            })
            .unwrap();
            let CommandResult::SeatResolved(recipient) =
                call(AutomaticBootstrapAction::Resolve { attempt }).unwrap()
            else {
                panic!("resolved")
            };
            let attachment =
                AutomaticBootstrapAttachment::derive(&peer.identity, attempt, created, recipient)
                    .unwrap();
            call(AutomaticBootstrapAction::Attach(Box::new(
                attachment.clone(),
            )))
            .unwrap();
            let mut expected = peer.status();
            let inspection = expected.recovery_inspection().unwrap();
            let disposition = AutomaticBootstrapRecoveryDisposition::Cancelled {
                reason: "inspected cancellation".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                attached_child: Some(Box::new(attachment.child)),
            };
            let request = AutomaticBootstrapMutation {
                identity: peer.identity.clone(),
                action: AutomaticBootstrapAction::Recover {
                    attempt,
                    inspection: inspection.clone(),
                    disposition: disposition.clone(),
                },
            };
            request.validate().unwrap();
            let uid = unsafe { libc::geteuid() };
            expected.state = BootstrapState::Cancelled;
            expected.administrative_revision = 1;
            expected.recovery = Some(AutomaticBootstrapRecoveryResult {
                operation: request.operation().unwrap(),
                attempt,
                inspection,
                disposition,
                operator_uid: uid,
                operator_provenance: format!("operator:local-user:{uid}"),
                state: BootstrapState::Cancelled,
                administrative_revision: 1,
            });
            let bytes = serde_json::to_vec(&CommandResult::AutomaticBootstrap(Box::new(
                expected.clone(),
            )))
            .unwrap();
            if large {
                assert!(bytes.len() > 262144 && bytes.len() < 2097152);
                assert!(serde_json::to_vec(&request).unwrap().len() <= 262144);
            } else {
                assert!(bytes.len() <= 262144);
            }
            (peer, request, expected)
        }
        #[test]
        fn automatic_child_begin_expired_permit_has_no_effects_and_fresh_permit_succeeds() {
            let (peer, _, _) = attached_recovery_fixture(false, false);
            let parent_before = peer.status();
            let child = parent_before.attachment.as_ref().unwrap().child.begin();
            let inner = crate::store::handoff::automatic::inner(&child).unwrap();
            let input = || crate::store::cooperative_permit_request(&inner).unwrap();
            let counts = || {
                let state = peer.base.peer.state.lock().unwrap();
                [
                    "operations",
                    "channel_handoff_fences",
                    "automatic_handoff_companions",
                    "automatic_handoff_keys",
                    "direct_threads",
                    "messages",
                    "invitations",
                ]
                .map(|table| {
                    state
                        .db
                        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap()
                })
            };
            let before = counts();
            let budget = super::super::super::super::cooperative_budget(&peer.base.clock);
            let permit = peer
                .store
                .issue_cooperative_permit(input(), &budget)
                .unwrap();
            // Waiting from after issuance guarantees this same grant is at least
            // 251 ms old. Observe the live clock; never freeze or advance it.
            let expired_at = peer.permit_clock.now() + 251;
            loop {
                let now = peer.permit_clock.now();
                if now >= expired_at {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(expired_at - now));
            }
            let error = peer
                .store
                .automatic_delivery_mutate(peer.ns(), &child, permit, &budget)
                .unwrap_err();
            assert_eq!(
                error.code,
                crate::protocol::results::ErrorCode::CallerUnverified
            );
            assert_eq!(error.detail, "permit expired");
            assert_eq!(counts(), before);
            assert_eq!(peer.status(), parent_before);

            // The identical operation is still valid: only its expired grant
            // was refused, and no child fence or operation was left behind.
            let fresh = peer
                .store
                .issue_cooperative_permit(input(), &budget)
                .unwrap();
            let result = peer
                .store
                .automatic_delivery_mutate(peer.ns(), &child, fresh, &budget)
                .unwrap();
            assert!(matches!(
                result,
                CommandResult::Handoff(ref handoff)
                    if handoff.state == crate::protocol::handoff::HandoffState::Live
            ));
            let state = peer.base.peer.state.lock().unwrap();
            let operation = input().operation;
            assert_eq!(
                state
                    .db
                    .query_row(
                        "SELECT count(*) FROM operations WHERE actor_scope=?1 AND operation_key=?2",
                        rusqlite::params![
                            format!("seat:{}", child.claim.seat.as_str()),
                            operation.as_str()
                        ],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
            assert_eq!(
                state.db.query_row(
                    "SELECT count(*) FROM automatic_handoff_companions WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",
                    rusqlite::params![child.claim.instance, format!("seat:{}", child.claim.seat.as_str()), child.plan().payload.keys.compound.as_str()],
                    |row| row.get::<_, i64>(0),
                ).unwrap(),
                1
            );
        }
        fn recovery_counts(peer: &AutoPeer) -> (i64, i64, String, i64) {
            let state = peer.base.peer.state.lock().unwrap();
            state.db.query_row("SELECT (SELECT count(*) FROM automatic_bootstrap_recoveries),(SELECT count(*) FROM automatic_bootstrap_keys),state,administrative_revision FROM automatic_bootstrap_parents", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
        }
        fn install_oversized_historical_recovery(
            peer: &AutoPeer,
            expected: &AutomaticBootstrapResult,
        ) {
            let bytes = serde_json::to_vec(&CommandResult::AutomaticBootstrap(Box::new(
                expected.clone(),
            )))
            .unwrap();
            let recovery = expected.recovery.as_ref().unwrap();
            let state = peer.base.peer.state.lock().unwrap();
            // A coherent typed row models retained historical corruption; no malformed JSON.
            state.db.execute("UPDATE automatic_bootstrap_parents SET state='cancelled',administrative_revision=1", []).unwrap();
            state.db.execute("INSERT INTO automatic_bootstrap_recoveries(parent_id,attempt,decision_kind,operation,revision,result_json) SELECT id,1,'cancellation',?1,1,?2 FROM automatic_bootstrap_parents", rusqlite::params![recovery.operation.as_str(), bytes]).unwrap();
        }
        #[test]
        fn historical_automatic_recovery_over_limit_write_refuses_atomically() {
            let (peer, request, _) = attached_recovery_fixture(true, true);
            let before = recovery_counts(&peer);
            let status = peer.status();
            let result = peer.call(
                Command::AutomaticBootstrap(Box::new(request)),
                &super::super::super::super::cooperative_budget(&peer.base.clock),
            );
            assert!(
                result
                    .is_err_and(|e| e.code == crate::protocol::results::ErrorCode::InvalidRequest),
                "historical oversized result was retained"
            );
            assert_eq!(recovery_counts(&peer), before);
            assert_eq!(peer.status(), status);
        }
        #[test]
        fn historical_automatic_recovery_over_limit_read_refuses() {
            let (peer, request, expected) = attached_recovery_fixture(true, true);
            install_oversized_historical_recovery(&peer, &expected);
            let result = peer.call(
                Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                    identity: request.identity,
                    action: AutomaticBootstrapAction::Status,
                })),
                &super::super::super::super::cooperative_budget(&peer.base.clock),
            );
            assert!(
                result.is_err_and(|e| e.code == crate::protocol::results::ErrorCode::StoreCorrupt),
                "oversized historical recovery was readable"
            );
        }
        #[test]
        fn historical_automatic_recovery_over_limit_replay_refuses() {
            let (peer, request, expected) = attached_recovery_fixture(true, true);
            install_oversized_historical_recovery(&peer, &expected);
            let result = peer.call(
                Command::AutomaticBootstrap(Box::new(request)),
                &super::super::super::super::cooperative_budget(&peer.base.clock),
            );
            assert!(
                result.is_err_and(|e| e.code == crate::protocol::results::ErrorCode::StoreCorrupt),
                "oversized historical recovery replayed"
            );
        }
        #[test]
        fn historical_automatic_recovery_below_limit_replays_exact_bytes() {
            let (peer, request, expected) = attached_recovery_fixture(true, false);
            let budget = super::super::super::super::cooperative_budget(&peer.base.clock);
            let expected = CommandResult::AutomaticBootstrap(Box::new(expected));
            assert_eq!(
                peer.call(
                    Command::AutomaticBootstrap(Box::new(request.clone())),
                    &budget
                )
                .unwrap(),
                expected
            );
            let saved: Vec<u8> = peer
                .base
                .peer
                .state
                .lock()
                .unwrap()
                .db
                .query_row(
                    "SELECT result_json FROM automatic_bootstrap_recoveries",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(saved, serde_json::to_vec(&expected).unwrap());
            assert_eq!(
                peer.call(Command::AutomaticBootstrap(Box::new(request)), &budget)
                    .unwrap(),
                expected
            );
            let after: Vec<u8> = peer
                .base
                .peer
                .state
                .lock()
                .unwrap()
                .db
                .query_row(
                    "SELECT result_json FROM automatic_bootstrap_recoveries",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(after, saved);
        }
        #[test]
        fn modern_automatic_recovery_above_historical_limit_succeeds_and_replays() {
            let (peer, request, expected) = attached_recovery_fixture(false, true);
            let budget = super::super::super::super::cooperative_budget(&peer.base.clock);
            let expected = CommandResult::AutomaticBootstrap(Box::new(expected));
            assert_eq!(
                peer.call(
                    Command::AutomaticBootstrap(Box::new(request.clone())),
                    &budget
                )
                .unwrap(),
                expected
            );
            assert_eq!(
                peer.call(Command::AutomaticBootstrap(Box::new(request)), &budget)
                    .unwrap(),
                expected
            );
        }

        #[test]
        fn parsed_human_recovery_of_automatic_unknown_creation_is_durable() {
            let peer = AutoPeer::new();
            peer.seed_unknown_creation();
            let argv = vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                peer.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--not-created".to_owned(),
            ];
            let parsed = super::super::super::super::commands::parse_argv(argv).unwrap();
            let mut output = Vec::new();
            super::super::super::super::topology_runtime::run_automatic_operator_parsed(
                &parsed,
                &peer.base.journal,
                peer.ns(),
                &peer,
                &peer.base.clock,
                &mut output,
                |_, _| panic!("not-created assertion must not observe a pane"),
            )
            .unwrap();
            assert_eq!(peer.status().attempt.get(), 2);
            assert!(!output.is_empty());
        }

        #[test]
        fn parsed_human_created_and_cancelled_automatic_recovery_are_distinct() {
            let created = AutoPeer::new();
            created.seed_unknown_creation();
            let argv = vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                created.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--created-pane".to_owned(),
                "w1:p2".to_owned(),
            ];
            let parsed = super::super::super::super::commands::parse_argv(argv).unwrap();
            let mut output = Vec::new();
            super::super::super::super::topology_runtime::run_automatic_operator_parsed(
                &parsed,
                &created.base.journal,
                created.ns(),
                &created,
                &created.base.clock,
                &mut output,
                |target, request| {
                    let mut evidence = crate::protocol::handoff::topology_contract_tests::created();
                    assert_eq!(target, &evidence.root_pane);
                    evidence.correlation = request
                        .map(|v| v.correlation.clone())
                        .unwrap_or_else(|| HostCallId::new("operator-observed-creation"));
                    evidence.witness = created.base.witness.clone();
                    Ok((evidence, HostCallId::new("operator-structural-read")))
                },
            )
            .unwrap();
            assert_eq!(created.status().state, BootstrapState::Created);
            assert_eq!(created.base.peer.state.lock().unwrap().native_calls, 0);

            let cancelled = AutoPeer::new();
            cancelled.seed_unknown_creation();
            let argv = vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                cancelled.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--cancel".to_owned(),
                "--reason".to_owned(),
                "inspected quiescence".to_owned(),
            ];
            let parsed = super::super::super::super::commands::parse_argv(argv).unwrap();
            let mut output = Vec::new();
            super::super::super::super::topology_runtime::run_automatic_operator_parsed(
                &parsed,
                &cancelled.base.journal,
                cancelled.ns(),
                &cancelled,
                &cancelled.base.clock,
                &mut output,
                |_, _| panic!("cancellation needs no native pane observation"),
            )
            .unwrap();
            assert_eq!(cancelled.status().state, BootstrapState::Cancelled);
            assert_eq!(cancelled.base.peer.state.lock().unwrap().native_calls, 0);
        }

        #[test]
        fn parsed_human_automatic_operator_retry_replays_lost_decision_reply() {
            let peer = AutoPeer::new();
            peer.seed_unknown_creation();
            *peer.lost_parent_reply.lock().unwrap() = Some("auto_recover");
            let argv = vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                peer.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--not-created".to_owned(),
            ];
            let parsed = super::super::super::super::commands::parse_argv(argv).unwrap();
            let mut output = Vec::new();
            assert!(
                super::super::super::super::topology_runtime::run_automatic_operator_parsed(
                    &parsed,
                    &peer.base.journal,
                    peer.ns(),
                    &peer,
                    &peer.base.clock,
                    &mut output,
                    |_, _| panic!("not-created has no pane"),
                )
                .is_err()
            );
            assert_eq!(peer.status().attempt.get(), 2);
            let operator_ref = format!("local:{}", peer.reference.ordinal + 1);
            let retry = super::super::super::super::commands::parse_argv([
                "ht",
                "human",
                "retry",
                operator_ref.as_str(),
            ])
            .unwrap();
            super::super::super::super::topology_runtime::run_automatic_operator_retry_parsed(
                &retry,
                &peer.base.journal,
                peer.ns(),
                &peer,
                &peer.base.clock,
                &mut output,
            )
            .unwrap();
            assert_eq!(peer.status().attempt.get(), 2);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            assert!(
                peer.base
                    .journal
                    .resolve_recovery_ref(&operator_ref)
                    .is_err()
            );
        }

        #[test]
        fn parsed_operator_dispatch_classifies_before_connect_and_replays_frozen_decision() {
            let peer = AutoPeer::new();
            peer.seed_unknown_creation();
            let selected = crate::daemon::paths::RuntimeContext::explicit(
                peer.ns().state_dir.clone(),
                peer.ns().host_endpoint.clone(),
                None,
            )
            .unwrap();
            let argv = vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                peer.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--not-created".to_owned(),
            ];
            let parsed = super::super::super::super::commands::parse_argv(argv).unwrap();
            use super::super::super::super::topology_runtime::try_run_automatic_operator_with_ports as dispatch;
            let mut wrong_actor = parsed.clone();
            wrong_actor.actor = super::super::super::super::actor_route::InvocationActor::Agent;
            assert!(
                dispatch(
                    &wrong_actor,
                    &selected,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut Vec::new(),
                    &mut || -> Result<(HandoffNamespace, Box<dyn LocalClient>), RunError> {
                        panic!("actor check before connect")
                    },
                    &mut |_, _| panic!("no host observation"),
                )
                .is_err()
            );
            let mut wrong_host = selected.clone();
            wrong_host.host_endpoint = Some(peer.base._temp.path().join("wrong-host"));
            assert!(
                dispatch(
                    &parsed,
                    &wrong_host,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut Vec::new(),
                    &mut || -> Result<(HandoffNamespace, Box<dyn LocalClient>), RunError> {
                        panic!("origin host check before connect")
                    },
                    &mut |_, _| panic!("no host observation"),
                )
                .is_err()
            );
            let intent_count = || {
                std::fs::read_dir(peer.base.journal.root())
                    .unwrap()
                    .filter(|entry| {
                        entry
                            .as_ref()
                            .unwrap()
                            .path()
                            .extension()
                            .is_some_and(|ext| ext == "intent")
                    })
                    .count()
            };
            let before = intent_count();
            assert!(
                dispatch(
                    &parsed,
                    &selected,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut Vec::new(),
                    &mut || Ok((
                        peer.ns().clone(),
                        Box::new(OperatorTestClient::Unsupported) as Box<dyn LocalClient>
                    )),
                    &mut |_, _| panic!("unsupported before native observation"),
                )
                .is_err()
            );
            assert_eq!(intent_count(), before);
            *peer.lost_parent_reply.lock().unwrap() = Some("auto_recover");
            let mut output = Vec::new();
            assert!(
                dispatch(
                    &parsed,
                    &selected,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut output,
                    &mut || Ok((
                        peer.ns().clone(),
                        Box::new(OperatorTestClient::Peer(&peer)) as Box<dyn LocalClient + '_>
                    )),
                    &mut |_, _| panic!("not-created assertion needs no host observation"),
                )
                .is_err()
            );
            assert_eq!(peer.status().attempt.get(), 2);
            assert_eq!(intent_count(), before + 1);
            let operator_ref = format!("local:{}", peer.reference.ordinal + 1);
            let wrong_original = super::super::super::super::commands::parse_argv([
                "ht",
                "human",
                "handoff",
                "recover",
                operator_ref.as_str(),
                "--attempt",
                "1",
                "--not-created",
            ])
            .unwrap();
            assert!(
                dispatch(
                    &wrong_original,
                    &selected,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut Vec::new(),
                    &mut || -> Result<(HandoffNamespace, Box<dyn LocalClient>), RunError> {
                        panic!("operator original must refuse before connection")
                    },
                    &mut |_, _| panic!("operator original has no pane observation"),
                )
                .is_err()
            );
            let retry = super::super::super::super::commands::parse_argv([
                "ht",
                "human",
                "retry",
                operator_ref.as_str(),
            ])
            .unwrap();
            assert!(
                dispatch(
                    &retry,
                    &selected,
                    &peer.base.journal,
                    &peer.base.clock,
                    &mut output,
                    &mut || Ok((
                        peer.ns().clone(),
                        Box::new(OperatorTestClient::Peer(&peer)) as Box<dyn LocalClient + '_>
                    )),
                    &mut |_, _| panic!("frozen retry never re-observes pane"),
                )
                .unwrap()
            );
            assert_eq!(peer.status().attempt.get(), 2);
            assert_eq!(intent_count(), before);
            assert!(!output.is_empty());
        }

        #[test]
        fn parsed_new_tab_automatic_route_creates_one_pair_and_starts_once() {
            let peer = AutoPeer::new();
            let (messages_before, invitations_before) = {
                let state = peer.base.peer.state.lock().unwrap();
                (
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM messages WHERE kind='ordinary'",
                            [],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap(),
                    state
                        .db
                        .query_row("SELECT count(*) FROM invitations", [], |r| {
                            r.get::<_, i64>(0)
                        })
                        .unwrap(),
                )
            };
            let mut launcher = DownstreamLauncher::default();
            let mut output = Vec::new();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                },
                &mut output,
            )
            .unwrap_or_else(|error| {
                panic!(
                    "{error:?}; calls={:?}; status={:?}",
                    peer.base.peer.state.lock().unwrap().calls,
                    peer.status()
                )
            });
            assert_eq!(result.state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
            let status = peer.status();
            let selected = status.thread.expect("exact selected pair thread");
            let db = &peer.base.peer.state.lock().unwrap().db;
            assert_eq!(
                db.query_row(
                    "SELECT thread_id FROM direct_threads WHERE thread_id=?1",
                    [selected.as_str()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                selected.as_str()
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM messages WHERE kind='ordinary'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                messages_before + 1
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM invitations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                invitations_before,
            );
            // Item 17: the pair auto-joins the recipient; no invitation exists for it.
            assert_eq!(
                db.query_row(
                    "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    rusqlite::params![
                        selected.as_str(),
                        status.attachment.unwrap().resolved_seat.as_str()
                    ],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "joined"
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM invitations WHERE thread_id=?1",
                    [selected.as_str()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }

        #[test]
        fn modern_automatic_maximum_escaped_body_operator_decision_replays_exactly() {
            let peer = AutoPeer::with_body("codex", &"\0".repeat(65536));
            peer.seed_unknown_creation();
            *peer.lost_parent_reply.lock().unwrap() = Some("auto_recover");
            let parsed = super::super::super::super::commands::parse_argv(vec![
                "ht".to_owned(),
                "human".to_owned(),
                "handoff".to_owned(),
                "recover".to_owned(),
                peer.reference.recovery_ref(),
                "--attempt".to_owned(),
                "1".to_owned(),
                "--not-created".to_owned(),
            ])
            .unwrap();
            let first = super::super::super::super::topology_runtime::run_automatic_operator_parsed(
                &parsed,
                &peer.base.journal,
                peer.ns(),
                &peer,
                &peer.base.clock,
                &mut Vec::new(),
                |_, _| panic!("not-created has no host read"),
            );
            assert!(first.is_err());
            assert_eq!(peer.status().attempt.get(), 2);
            let operator_ref = format!("local:{}", peer.reference.ordinal + 1);
            let retry = super::super::super::super::commands::parse_argv([
                "ht",
                "human",
                "retry",
                operator_ref.as_str(),
            ])
            .unwrap();
            super::super::super::super::topology_runtime::run_automatic_operator_retry_parsed(
                &retry,
                &peer.base.journal,
                peer.ns(),
                &peer,
                &peer.base.clock,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(peer.status().attempt.get(), 2);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            assert_eq!(peer.identity.plan.payload.handoff.body, "\0".repeat(65536));
        }

        // Kills: admitting historical None bootstrap without send-time semantics,
        // before any parent Begin/Reserve or native creation/launch submission.
        struct BootstrapReceiptPeer<'a> {
            inner: &'a AutoPeer,
            receipts: bool,
            calls: std::sync::Mutex<Vec<Command>>,
        }
        impl LocalClient for BootstrapReceiptPeer<'_> {
            fn call_with_output(
                &self,
                command: Command,
                _: &crate::protocol::output::OutputSpec,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                self.call(command, budget)
            }
            fn call(
                &self,
                command: Command,
                budget: &CallBudget,
            ) -> Result<CommandResult, ApiError> {
                self.calls.lock().unwrap().push(command.clone());
                if matches!(command, Command::Capabilities) {
                    let mut capabilities =
                        vec![crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into()];
                    if self.receipts {
                        capabilities
                            .push(crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.into());
                    }
                    return Ok(CommandResult::Capabilities(
                        crate::protocol::results::CapabilityList { capabilities },
                    ));
                }
                self.inner.call(command, budget)
            }
        }
        fn receipt_bootstrap_counts(peer: &AutoPeer) -> Vec<i64> {
            let state = peer.base.peer.state.lock().unwrap();
            [
                "operations",
                "threads",
                "direct_threads",
                "invitations",
                "memberships",
                "messages",
                "send_preparations",
                "send_manifests",
                "prepared_recipients",
                "receipts",
                "receipt_state",
                "automatic_bootstrap_parents",
                "automatic_bootstrap_keys",
                "automatic_handoff_companions",
            ]
            .iter()
            .map(|table| {
                state
                    .db
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap()
            })
            .collect()
        }
        #[test]
        fn public_historical_none_new_tab_retry_requires_receipts_before_any_effect_and_preserves_origin()
         {
            use super::super::super::super::{actor_route::InvocationActor, retry};
            use sha2::{Digest, Sha256};
            // Original producer fixture parses actual --new-tab and then records the
            // historical omitted policy; no runtime-dependent reconstruction occurs.
            let peer = AutoPeer::with_body_policy("codex", "literal work", true);
            assert!(peer.identity.plan.payload.handoff.policy.is_none());
            let original = peer
                .base
                .journal
                .snapshot_bootstrap_origin(&peer.reference)
                .unwrap();
            let retained =
                retry::load_original_for_actor(&peer.base.journal, &peer.reference.recovery_ref())
                    .unwrap();
            let semantic = serde_json::to_vec(&retained.semantic).unwrap();
            assert!(
                !String::from_utf8(semantic.clone())
                    .unwrap()
                    .contains("\"policy\"")
            );
            assert_eq!(
                retained.header.semantic_digest,
                format!("{:x}", Sha256::digest(&semantic))
            );
            let counts = receipt_bootstrap_counts(&peer);
            let old = BootstrapReceiptPeer {
                inner: &peer,
                receipts: false,
                calls: std::sync::Mutex::new(vec![]),
            };
            let mut launcher = DownstreamLauncher::default();
            let mut output = Vec::new();
            let result = retry::run_automatic_bootstrap_retry_to_writer(
                &peer.base.journal,
                &peer.reference,
                InvocationActor::Agent,
                peer.ns(),
                &old,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut output,
            );
            assert!(
                matches!(result, Err(RunError::Api(ref error)) if error.code == crate::protocol::results::ErrorCode::Unsupported),
                "historical pending None bootstrap must negotiate receipts: {result:?}"
            );
            assert_eq!(receipt_bootstrap_counts(&peer), counts);
            assert_eq!(launcher.starts, 0);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            assert_eq!(
                peer.base
                    .journal
                    .snapshot_bootstrap_origin(&peer.reference)
                    .unwrap(),
                original
            );
            let after =
                retry::load_original_for_actor(&peer.base.journal, &peer.reference.recovery_ref())
                    .unwrap();
            assert_eq!(after.semantic, retained.semantic);
            assert_eq!(
                after.header.semantic_digest,
                retained.header.semantic_digest
            );
            let calls = old.calls.lock().unwrap();
            assert!(calls.iter().any(|c| matches!(c, Command::Capabilities)));
            assert!(calls.iter().all(|c| matches!(c, Command::Capabilities | Command::SeatInspect(_) | Command::Seats(_)) || matches!(c,
                Command::AutomaticBootstrap(q) if matches!(q.action, AutomaticBootstrapAction::Status)) || matches!(c,
                Command::AutomaticHandoffDelivery(q) if matches!(q.action, AutomaticDeliveryAction::Status))), "only discovery/read/status before refusal");
            drop(calls);
            let capable = BootstrapReceiptPeer {
                inner: &peer,
                receipts: true,
                calls: std::sync::Mutex::new(vec![]),
            };
            let mut first = Vec::new();
            let completed = retry::run_automatic_bootstrap_retry_to_writer(
                &peer.base.journal,
                &peer.reference,
                InvocationActor::Agent,
                peer.ns(),
                &capable,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut first,
            )
            .unwrap();
            assert_eq!(completed.state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            let attachment = completed.attachment.as_ref().unwrap();
            assert!(attachment.child.plan().payload.policy.is_none());
            let child = attachment.child.clone();
            for command in capable.calls.lock().unwrap().iter() {
                match command {
                    Command::AutomaticBootstrap(q) => assert_eq!(q.identity, peer.identity),
                    Command::AutomaticHandoffDelivery(q) => {
                        assert_eq!(q.plan(), child.plan());
                        if let AutomaticDeliveryAction::Ensure(ensure) = &q.action {
                            assert!(ensure.creation_policy.is_none());
                            assert_eq!(ensure.operation, child.plan().payload.keys.ensure);
                            assert_eq!(ensure.claim, peer.identity.claim);
                        }
                        if let AutomaticDeliveryAction::Send(send) = &q.action {
                            assert_eq!(send.operation, child.plan().payload.keys.send);
                            assert_eq!(send.body, "literal work");
                            assert_eq!(send.claim, peer.identity.claim);
                        }
                    }
                    _ => {}
                }
            }
            let terminal_path = peer.base.journal.root().join(format!(
                "automatic-bootstrap-{:020}-{}.terminal",
                peer.reference.ordinal,
                peer.reference.operation.as_str()
            ));
            let terminal: serde_json::Value =
                serde_json::from_slice(&std::fs::read(terminal_path).unwrap()).unwrap();
            assert_eq!(terminal["original"].as_str().unwrap().as_bytes(), original);
            let after =
                retry::load_original_for_actor(&peer.base.journal, &peer.reference.recovery_ref())
                    .unwrap();
            assert_eq!(after.semantic, retained.semantic);
            assert_eq!(
                after.header.semantic_digest,
                retained.header.semantic_digest
            );
            let mut parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "retry",
                &peer.reference.recovery_ref(),
            ])
            .unwrap();
            parsed.output = crate::protocol::output::OutputSpec::default();
            let before_replay = receipt_bootstrap_counts(&peer);
            old.calls.lock().unwrap().clear();
            let mut replay = Vec::new();
            assert!(
                super::super::super::super::topology_runtime::try_completed_with_namespace(
                    &parsed,
                    &peer.base.journal,
                    peer.ns(),
                    &old,
                    &peer.base.clock,
                    &mut replay
                )
                .unwrap()
            );
            assert_eq!(replay, first);
            assert_eq!(receipt_bootstrap_counts(&peer), before_replay);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
            assert!(old.calls.lock().unwrap().iter().all(|c| matches!(c, Command::Capabilities) || matches!(c,
                Command::AutomaticBootstrap(q) if matches!(q.action, AutomaticBootstrapAction::Status)) || matches!(c,
                Command::AutomaticHandoffDelivery(q) if matches!(q.action, AutomaticDeliveryAction::Status))));
        }

        #[test]
        fn modern_automatic_maximum_escaped_body_completes_and_recovers_exactly() {
            let body = "\0".repeat(65536);
            let peer = AutoPeer::with_body("codex", &body);
            let mut launcher = DownstreamLauncher::default();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            );
            assert!(
                result.is_ok(),
                "maximum escaped body failed: {:?}",
                result.err()
            );
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.status().state, BootstrapState::Completed);
            peer.print_supported_envelope_sizes("max-escaped-public");
            let state = peer.base.peer.state.lock().unwrap();
            let saved: String = state
                .db
                .query_row("SELECT body FROM messages WHERE kind='ordinary'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(saved, body);
            drop(state);
            assert!(peer.historical_retry(peer.ns()).unwrap().0);
            assert_eq!(launcher.starts, 1);
        }

        #[test]
        fn modern_automatic_canonical_completion_refuses_full_diagnostic_report() {
            let peer = AutoPeer::new();
            let launcher = peer.complete_child_with_lost_parent_reply();
            assert_eq!(launcher.starts, 1);
            let status = peer.status();
            let attachment = status.attachment.unwrap();
            let bytes = std::fs::read(peer.base.journal.root().join(format!(
                "automatic-diagnostics-{}.json",
                peer.reference.operation.as_str()
            )))
            .unwrap();
            let report = serde_json::from_slice(&bytes).unwrap();
            use sha2::{Digest, Sha256};
            let retained = LinkedBootstrapReport {
                thread: status.thread.unwrap(),
                recipient: attachment.resolved_seat.clone(),
                pane: attachment.created.root_pane.clone(),
                kind: "launch".into(),
                launch: peer.identity.plan.payload.launch.clone(),
                report,
                report_digest: format!("{:x}", Sha256::digest(&bytes)),
                terminal: attachment.created.terminal.clone(),
                host_incarnation: attachment.created.host_incarnation.clone(),
            };
            let request = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                identity: peer.identity.clone(),
                action: AutomaticBootstrapAction::Complete {
                    attachment: Box::new(attachment),
                    retained,
                },
            }));
            let reply = peer.call(
                request,
                &CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        crate::protocol::time::Clock::monotonic_now(&peer.base.clock).0 + 10000,
                    ),
                    cancellation: Default::default(),
                },
            );
            assert!(
                reply.is_err(),
                "modern canonical completion accepted unbounded full-report representation"
            );
            assert_eq!(peer.status().state, BootstrapState::Attached);
        }

        #[test]
        fn modern_automatic_completed_terminal_rejects_mismatched_selection_hint() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher::default();
            resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &Default::default(),
                &mut Vec::new(),
            )
            .unwrap();
            let path = peer.base.journal.root().join(format!(
                "automatic-bootstrap-{:020}-{}.terminal",
                peer.reference.ordinal,
                peer.reference.operation.as_str()
            ));
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert!(value["selection"].is_object());
            value["selection"]["recipient"] = "foreign-seat".into();
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            let before = serde_json::to_vec(&peer.status()).unwrap();
            assert!(
                peer.historical_retry(peer.ns()).is_err(),
                "completed presentation accepted mismatched retained selection hint"
            );
            assert_eq!(serde_json::to_vec(&peer.status()).unwrap(), before);
            assert_eq!(launcher.starts, 1);
        }

        #[test]
        fn modern_automatic_completed_missing_child_and_artifact_is_readonly_and_truthful() {
            for corrupt in [false, true] {
                let peer = AutoPeer::new();
                let launcher = peer.complete_child_with_lost_parent_reply();
                let status = peer.status();
                let attachment = status.attachment.unwrap();
                let progress_path = peer.base.journal.root().join(format!(
                    "automatic-child-{}.progress",
                    peer.reference.operation.as_str()
                ));
                let progress: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&progress_path).unwrap()).unwrap();
                let report = progress["launch"].clone();
                use sha2::{Digest, Sha256};
                let retained = LinkedBootstrapReport {
                    thread: status.thread.unwrap(),
                    recipient: attachment.resolved_seat.clone(),
                    pane: attachment.created.root_pane.clone(),
                    kind: "launch".into(),
                    launch: peer.identity.plan.payload.launch.clone(),
                    report_digest: format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&report).unwrap())
                    ),
                    report,
                    terminal: attachment.created.terminal.clone(),
                    host_incarnation: attachment.created.host_incarnation.clone(),
                };
                peer.call(
                    Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                        identity: peer.identity.clone(),
                        action: AutomaticBootstrapAction::Complete {
                            attachment: Box::new(attachment),
                            retained,
                        },
                    })),
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap();
                let before = serde_json::to_vec(&peer.status()).unwrap();
                if corrupt {
                    std::fs::write(&progress_path, b"{invalid existing child}").unwrap();
                } else {
                    std::fs::remove_file(&progress_path).unwrap();
                }
                std::fs::remove_file(peer.base.journal.root().join(format!(
                    "automatic-diagnostics-{}.json",
                    peer.reference.operation.as_str()
                )))
                .unwrap();
                struct ReadOnlyCompleted<'a>(&'a AutoPeer);
                impl LocalClient for ReadOnlyCompleted<'_> {
                    fn call_with_output(
                        &self,
                        c: Command,
                        _: &crate::protocol::output::OutputSpec,
                        b: &CallBudget,
                    ) -> Result<CommandResult, ApiError> {
                        self.call(c, b)
                    }
                    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
                        match &c {
                            Command::Capabilities => Ok(CommandResult::Capabilities(
                                crate::protocol::results::CapabilityList {
                                    capabilities: vec![
                                        crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into(),
                                    ],
                                },
                            )),
                            Command::AutomaticBootstrap(q)
                                if matches!(q.action, AutomaticBootstrapAction::Status)
                                    && q.identity == self.0.identity =>
                            {
                                self.0.call(c, b)
                            }
                            other => panic!(
                                "Completed missing-local inspection attempted effects/new admission: {other:?}"
                            ),
                        }
                    }
                }
                let retry_readonly = || {
                    let parsed = super::super::super::super::commands::parse_argv([
                        "ht",
                        "retry",
                        peer.reference.recovery_ref().as_str(),
                    ])
                    .unwrap();
                    let mut bytes = Vec::new();
                    super::super::super::super::topology_runtime::try_completed_with_namespace(
                        &parsed,
                        &peer.base.journal,
                        peer.ns(),
                        &ReadOnlyCompleted(&peer),
                        &peer.base.clock,
                        &mut bytes,
                    )
                    .map(|done| (done, bytes))
                };
                let retried = retry_readonly();
                if corrupt {
                    assert!(
                        retried.is_err(),
                        "corrupt existing child was mistaken for missing optional progress"
                    );
                    assert_eq!(serde_json::to_vec(&peer.status()).unwrap(), before);
                    assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
                    continue;
                }
                assert!(
                    retried.is_ok(),
                    "canonical Completed required missing optional child/artifact: {:?}",
                    retried.err()
                );
                let (done, bytes) = retried.unwrap();
                assert!(done);
                let text = String::from_utf8(bytes).unwrap();
                assert!(
                    text.contains("unavailable"),
                    "missing message id was not presented honestly: {text}"
                );
                assert_eq!(serde_json::to_vec(&peer.status()).unwrap(), before);
                assert_eq!(launcher.starts, 1);
                assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
                assert!(retry_readonly().unwrap().0);
            }
        }

        #[test]
        fn modern_automatic_large_diagnostics_preserve_full_local_report_and_complete_summary() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher {
                report_padding: 3 * 1024 * 1024,
                ..Default::default()
            };
            let mut output = Vec::new();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                },
                &mut output,
            );
            assert!(
                result.is_ok(),
                "large diagnostics stranded a started automatic handoff: {result:?}"
            );
            assert_eq!(launcher.starts, 1);
            let completed = peer.status().completed.unwrap();
            assert_eq!(completed.report["format"], "automatic_launch_summary_v1");
            assert!(completed.report.get("padding").is_none());
            let path = peer.base.journal.root().join(format!(
                "automatic-diagnostics-{}.json",
                peer.reference.operation.as_str()
            ));
            let bytes = std::fs::read(&path).unwrap();
            let full: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(full["padding"].as_str().unwrap().len(), 3 * 1024 * 1024);
            use sha2::{Digest, Sha256};
            assert_eq!(
                completed.report["full_report_digest"],
                format!("{:x}", Sha256::digest(&bytes))
            );
            let historical = peer.historical_retry(peer.ns()).unwrap();
            assert!(historical.0);
            assert_eq!(launcher.starts, 1);
            std::fs::remove_file(path).unwrap();
            assert!(
                peer.historical_retry(peer.ns()).unwrap().0,
                "optional diagnostics must not gate canonical Completed replay"
            );
        }

        #[test]
        fn modern_automatic_unknown_large_diagnostics_keep_bounded_truthful_progress() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher {
                unknown: true,
                report_padding: 3 * 1024 * 1024,
                ..Default::default()
            };
            let run = |launcher: &mut DownstreamLauncher| {
                resume_automatic_to_writer(
                    &peer.base.journal,
                    &peer.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    peer.ns(),
                    &peer,
                    &peer,
                    launcher,
                    &peer.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &peer.base.witness,
                        context: &peer.base.context,
                    },
                    &crate::protocol::output::OutputSpec::default(),
                    &mut Vec::new(),
                )
            };
            assert!(run(&mut launcher).is_err());
            let path = peer.base.journal.root().join(format!(
                "automatic-child-{}.progress",
                peer.reference.operation.as_str()
            ));
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(
                saved["launch"]["format"],
                "automatic_launch_noncompletion_v1"
            );
            assert_eq!(saved["launch"]["observed_started"], false);
            assert_ne!(saved["launch"]["outcome"], "started");
            assert!(run(&mut launcher).is_err());
            assert_eq!(launcher.starts, 1);
            assert_ne!(peer.status().state, BootstrapState::Completed);
        }

        #[test]
        fn modern_automatic_artifact_failure_keeps_synced_summary_and_reopened_retry_never_starts()
        {
            let peer = AutoPeer::new();
            let artifact = peer.base.journal.root().join(format!(
                "automatic-diagnostics-{}.json",
                peer.reference.operation.as_str()
            ));
            // A deterministic failure after the critical started summary has been synced.
            std::fs::create_dir(&artifact).unwrap();
            let mut launcher = DownstreamLauncher {
                report_padding: 3 * 1024 * 1024,
                ..Default::default()
            };
            let run = |journal: &super::super::super::super::journal::Journal,
                       launcher: &mut DownstreamLauncher| {
                resume_automatic_to_writer(
                    journal,
                    &peer.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    peer.ns(),
                    &peer,
                    &peer,
                    launcher,
                    &peer.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &peer.base.witness,
                        context: &peer.base.context,
                    },
                    &crate::protocol::output::OutputSpec::default(),
                    &mut Vec::new(),
                )
            };
            assert!(run(&peer.base.journal, &mut launcher).is_err());
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.status().state, BootstrapState::Attached);
            let critical = peer.base.journal.root().join(format!(
                "automatic-child-{}.progress",
                peer.reference.operation.as_str()
            ));
            let bytes = std::fs::read(critical).unwrap();
            assert!(bytes.len() < 2 * 1024 * 1024);
            let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(saved["possible_start"], true);
            assert_eq!(saved["launch"]["format"], "automatic_launch_summary_v1");
            assert!(saved["launch"].get("padding").is_none());
            std::fs::remove_dir(&artifact).unwrap();
            let reopened =
                super::super::super::super::journal::Journal::open(peer.base.journal.root())
                    .unwrap();
            assert_eq!(
                run(&reopened, &mut launcher).unwrap().state,
                BootstrapState::Completed
            );
            assert_eq!(launcher.starts, 1);
            assert!(!artifact.exists());
            assert!(peer.historical_retry(peer.ns()).unwrap().0);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
        }

        #[test]
        fn modern_automatic_committed_completion_reply_loss_reopens_without_artifact_or_relaunch() {
            let peer = AutoPeer::new();
            *peer.lost_parent_reply.lock().unwrap() = Some("auto_complete");
            let mut launcher = DownstreamLauncher::default();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            );
            assert_eq!(result.unwrap().state, BootstrapState::Completed);
            assert!(peer.lost_parent_reply.lock().unwrap().is_none());
            assert_eq!(peer.status().state, BootstrapState::Completed);
            let artifact = peer.base.journal.root().join(format!(
                "automatic-diagnostics-{}.json",
                peer.reference.operation.as_str()
            ));
            std::fs::remove_file(artifact).unwrap();
            let reopened =
                super::super::super::super::journal::Journal::open(peer.base.journal.root())
                    .unwrap();
            let replay = resume_automatic_to_writer(
                &reopened,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(replay.state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
        }

        #[test]
        fn modern_automatic_rejects_future_request_overflow_before_publication_or_topology() {
            let peer = AutoPeer::new();
            let parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "large",
                "--kind",
                "codex",
                "--",
                "work",
            ])
            .unwrap();
            let super::super::super::super::commands::CliAction::TopologyHandoff(mut request) =
                parsed.action
            else {
                panic!("route")
            };
            request.body = "\0".repeat(65536);
            request.argv = vec![format!("--model={}", "\u{1}".repeat(3792)); 4];
            let before = ordinary_and_invitation_counts(&peer);
            let intents = intent_count(&peer.base.journal);
            let result = prepare_automatic(
                &request,
                Preparation {
                    claim: &peer.identity.claim,
                    namespace: peer.ns(),
                    topology: &topology(),
                    invocation_cwd: peer.base._temp.path(),
                    options: None,
                    client: &ReadOnly,
                    clock: &peer.base.clock,
                },
            );
            assert!(
                result.is_err(),
                "initial admission accepted an origin whose future Complete request exceeds1MiB"
            );
            assert_eq!(ordinary_and_invitation_counts(&peer), before);
            assert_eq!(intent_count(&peer.base.journal), intents);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
        }

        #[test]
        fn modern_automatic_actual_admission_boundary_completes_and_next_metadata_size_has_zero_effects()
         {
            let mut peer = AutoPeer::new();
            let parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "boundary",
                "--kind",
                "codex",
                "--",
                "work",
            ])
            .unwrap();
            let super::super::super::super::commands::CliAction::TopologyHandoff(mut request) =
                parsed.action
            else {
                panic!("route")
            };
            request.body = "\0".repeat(65536);
            let mut lower = 0usize;
            let mut upper = 4089usize;
            let mut accepted = None;
            while lower + 1 < upper {
                let length = (lower + upper) / 2;
                request.argv = vec![format!("--model={}", "\u{1}".repeat(length)); 4];
                let result = prepare_automatic(
                    &request,
                    Preparation {
                        claim: &peer.identity.claim,
                        namespace: peer.ns(),
                        topology: &topology(),
                        invocation_cwd: peer.base._temp.path(),
                        options: None,
                        client: &ReadOnly,
                        clock: &peer.base.clock,
                    },
                );
                match result {
                    Ok(identity) => {
                        lower = length;
                        accepted = Some(identity)
                    }
                    Err(_) => upper = length,
                }
            }
            assert!(
                lower > 0 && upper <= 4088,
                "no real admission boundary within individually valid native metadata"
            );
            let before = ordinary_and_invitation_counts(&peer);
            let intents = intent_count(&peer.base.journal);
            request.argv = vec![format!("--model={}", "\u{1}".repeat(upper)); 4];
            assert!(
                prepare_automatic(
                    &request,
                    Preparation {
                        claim: &peer.identity.claim,
                        namespace: peer.ns(),
                        topology: &topology(),
                        invocation_cwd: peer.base._temp.path(),
                        options: None,
                        client: &ReadOnly,
                        clock: &peer.base.clock
                    }
                )
                .is_err()
            );
            assert_eq!(ordinary_and_invitation_counts(&peer), before);
            assert_eq!(intent_count(&peer.base.journal), intents);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            peer.base.journal.complete(&peer.reference).unwrap();
            peer.identity = accepted.unwrap();
            peer.reference = publish_automatic(&peer.base.journal, &peer.identity, 2).unwrap();
            let mut launcher = DownstreamLauncher::default();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            );
            assert!(
                result.is_ok(),
                "near-admission-boundary public completion failed: {:?}",
                result.err()
            );
            assert_eq!(launcher.starts, 1);
            peer.print_supported_envelope_sizes("near-admission-public");
            assert!(peer.historical_retry(peer.ns()).unwrap().0);
            assert_eq!(peer.status().state, BootstrapState::Completed);
        }

        fn run_public_automatic(peer: &AutoPeer) -> (Result<(), RunError>, DownstreamLauncher) {
            let mut launcher = DownstreamLauncher::default();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                peer,
                peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            )
            .map(|_| ());
            (result, launcher)
        }

        #[test]
        fn profile_directory_automatic_retry_preserves_create_and_child_context() {
            let temp = Temp::new();
            let selected = temp.path().canonicalize().unwrap();
            let peer = AutoPeer::with_profile("codex", "work", false, Some(selected.clone()));
            *peer.lost_child_reply.lock().unwrap() = Some("auto_child_send");
            let (first, mut launcher) = run_public_automatic(&peer);
            assert!(first.is_err());
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(result.state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
            assert_eq!(
                launcher.requests[0].directory_context,
                peer.identity.plan.payload.directory_context
            );
            assert_eq!(
                launcher.requests[0]
                    .directory_context
                    .as_ref()
                    .unwrap()
                    .profile_dir,
                selected
            );
        }

        #[test]
        fn cwd_only_automatic_removed_during_preflight_repairs_without_duplicate_effects() {
            let temp = Temp::new();
            let cwd = temp.path().join("selected-cwd");
            std::fs::create_dir(&cwd).unwrap();
            let cwd = cwd.canonicalize().unwrap();
            let mut peer = AutoPeer::new();
            peer.base.journal.complete(&peer.reference).unwrap();
            peer.identity.plan.payload.cwd = cwd.clone();
            peer.identity.digest = peer.identity.semantic_digest().unwrap();
            peer.identity.validate().unwrap();
            peer.reference = publish_automatic(&peer.base.journal, &peer.identity, 2).unwrap();
            assert!(peer.identity.plan.payload.directory_context.is_none());
            let mut launcher = DownstreamLauncher {
                remove_directory_on_preflight: Some(cwd.clone()),
                ..Default::default()
            };
            let run = |launcher: &mut DownstreamLauncher| {
                resume_automatic_to_writer(
                    &peer.base.journal,
                    &peer.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    peer.ns(),
                    &peer,
                    &peer,
                    launcher,
                    &peer.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &peer.base.witness,
                        context: &peer.base.context,
                    },
                    &crate::protocol::output::OutputSpec::default(),
                    &mut Vec::new(),
                )
            };
            let error = run(&mut launcher).unwrap_err();
            assert!(error.to_string().contains("bootstrap cwd disappeared"));
            assert_eq!(launcher.starts, 0);
            assert_eq!(peer.status().state, BootstrapState::Attached);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            let counts = ordinary_and_invitation_counts(&peer);
            assert_eq!(counts.0, 1);
            std::fs::create_dir(&cwd).unwrap();
            assert_eq!(run(&mut launcher).unwrap().state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(ordinary_and_invitation_counts(&peer), counts);
            std::fs::remove_dir(&cwd).unwrap();
            assert_eq!(run(&mut launcher).unwrap().state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            assert_eq!(ordinary_and_invitation_counts(&peer), counts);
        }

        /// Negative control for [`AutomaticPermitClock`]: holding the clock does
        /// not disable the permit expiry. A held window past `MAX_PERMIT_MILLIS`
        /// between one role's issuance and decision refuses that role's
        /// mutation through the same public automatic path, with no later
        /// permit role reached and the bootstrap left incomplete.
        #[test]
        fn automatic_permit_clock_window_past_the_bound_still_expires() {
            let roles = {
                let peer = AutoPeer::new();
                let (result, _) = run_public_automatic(&peer);
                result.unwrap();
                peer.permit_clock.windows.lock().unwrap().clone()
            };
            for role in ["auto_attach", "auto_child_send"] {
                assert!(roles.contains(&role), "{role} not held in {roles:?}");
            }
            let mut targets = roles.clone();
            targets.dedup();
            for role in targets {
                let peer = AutoPeer::new();
                *peer.permit_clock.window_role.lock().unwrap() = Some(role);
                peer.permit_clock.permit_window.store(
                    crate::protocol::authority::MAX_PERMIT_MILLIS + 1,
                    std::sync::atomic::Ordering::SeqCst,
                );
                let (result, _) = run_public_automatic(&peer);
                let error = result.unwrap_err();
                assert!(
                    matches!(&error, RunError::Api(e) if e.code == ErrorCode::CallerUnverified && e.detail == "permit expired"),
                    "{role}: {error:?}"
                );
                let held = peer.permit_clock.windows.lock().unwrap().clone();
                assert_eq!(
                    held.last(),
                    Some(&role),
                    "{role} must be the refused permit role: {held:?}"
                );
                // Before Begin commits there is no parent row to report.
                let status = peer.store.automatic_bootstrap_query(
                    peer.ns(),
                    &AutomaticBootstrapMutation {
                        identity: peer.identity.clone(),
                        action: AutomaticBootstrapAction::Status,
                    },
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                );
                assert!(
                    !matches!(&status, Ok(CommandResult::AutomaticBootstrap(s)) if s.state == BootstrapState::Completed),
                    "{role}: {status:?}"
                );
            }
        }

        /// The exact bound completes: every held permit window is
        /// `MAX_PERMIT_MILLIS` and the public automatic path still completes.
        #[test]
        fn automatic_permit_clock_window_at_the_bound_completes() {
            let peer = AutoPeer::new();
            peer.permit_clock.permit_window.store(
                crate::protocol::authority::MAX_PERMIT_MILLIS,
                std::sync::atomic::Ordering::SeqCst,
            );
            let (result, launcher) = run_public_automatic(&peer);
            result.unwrap();
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.status().state, BootstrapState::Completed);
            let roles = peer.permit_clock.windows.lock().unwrap().clone();
            for role in ["auto_attach", "auto_child_send"] {
                assert!(roles.contains(&role), "{role} not held in {roles:?}");
            }
        }

        fn ordinary_and_invitation_counts(peer: &AutoPeer) -> (i64, i64) {
            let state = peer.base.peer.state.lock().unwrap();
            (
                state
                    .db
                    .query_row(
                        "SELECT count(*) FROM messages WHERE kind='ordinary'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap(),
                state
                    .db
                    .query_row("SELECT count(*) FROM invitations", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap(),
            )
        }

        fn intent_count(journal: &super::super::super::super::journal::Journal) -> usize {
            std::fs::read_dir(journal.root())
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "intent")
                })
                .count()
        }

        /// The automatic route refuses an adapter whose composition rewrites the applied caller
        /// argv before the host preflight, journal publication, tab creation, launch or any
        /// store write: such a launch could never complete against the automatic plan.
        #[test]
        fn parsed_new_tab_automatic_route_refuses_a_rewriting_kind_before_any_effect() {
            let peer = AutoPeer::new();
            let before = ordinary_and_invitation_counts(&peer);
            let intents_before = intent_count(&peer.base.journal);
            let parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "new peer",
                "--kind",
                "hermes",
                "--",
                "literal work",
            ])
            .unwrap();
            let super::super::super::super::commands::CliAction::TopologyHandoff(request) =
                &parsed.action
            else {
                panic!("parsed route")
            };
            assert!(request.automatic && !request.existing);
            let mut launcher = DownstreamLauncher::default();
            let mut output = Vec::new();
            let result =
                super::super::super::super::topology_runtime::try_run_automatic_new_with_ports(
                    &parsed,
                    &peer.base.peer.identity.claim,
                    &peer.base.journal,
                    peer.ns(),
                    &peer,
                    &peer,
                    &mut launcher,
                    &peer.base.clock,
                    peer.base._temp.path(),
                    None,
                    &mut || Ok(topology()),
                    &mut |_| panic!("a refused kind must not preflight the host"),
                    &crate::protocol::output::OutputSpec::default(),
                    &mut output,
                );
            let Err(RunError::Api(error)) = &result else {
                panic!("refused: {result:?}")
            };
            assert_eq!(
                error.code,
                crate::protocol::results::ErrorCode::Unsupported,
                "{error:?}"
            );
            assert!(error.detail.contains("--new-thread"), "{error:?}");
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 0);
            assert_eq!(launcher.starts, 0);
            assert_eq!(intent_count(&peer.base.journal), intents_before);
            assert_eq!(ordinary_and_invitation_counts(&peer), before);
            assert!(output.is_empty());
        }

        /// The automatic counterpart of the explicit-channel agy test: agy's composition leaves
        /// the applied caller argv unchanged, so its automatic plan launches with
        /// `--prompt-interactive` placing the frozen prompt and completes.
        #[test]
        fn automatic_new_tab_agy_completes_end_to_end_with_prompt_interactive() {
            let peer = AutoPeer::with_kind("agy");
            assert_eq!(peer.identity.plan.payload.launch.harness.as_str(), "agy");
            assert!(peer.identity.plan.payload.startup_input.is_some());
            let before = ordinary_and_invitation_counts(&peer);
            let mut launcher = DownstreamLauncher::default();
            let mut output = Vec::new();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                },
                &mut output,
            )
            .unwrap_or_else(|error| panic!("agy automatic bootstrap must complete: {error:?}"));
            assert_eq!(result.state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
            assert_eq!(
                ordinary_and_invitation_counts(&peer),
                (before.0 + 1, before.1)
            );
            let selected = peer.status().thread.expect("exact selected pair thread");
            assert_eq!(
                peer.base
                    .peer
                    .state
                    .lock()
                    .unwrap()
                    .db
                    .query_row(
                        "SELECT thread_id FROM direct_threads WHERE thread_id=?1",
                        [selected.as_str()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                selected.as_str()
            );
            let report = &result.completed.as_ref().expect("completed").report;
            assert_eq!(report["harness"], "agy");
            let argv = report["argv"].as_array().unwrap();
            let flag = argv
                .iter()
                .position(|arg| arg == "--prompt-interactive")
                .unwrap_or_else(|| panic!("no --prompt-interactive in {argv:?}"));
            assert_eq!(flag + 2, argv.len(), "the prompt is the flag's value, last");
            assert!(
                argv[flag + 1]
                    .as_str()
                    .unwrap()
                    .starts_with("Expected handoff command routing")
            );
        }

        #[test]
        fn parsed_new_tab_selector_enters_public_runtime_dispatch() {
            let peer = AutoPeer::new();
            let mut parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "handoff",
                "--new-tab",
                "new peer",
                "--space",
                "work",
                "--kind",
                "codex",
                "--",
                "literal work",
            ])
            .unwrap();
            let before = {
                let state = peer.base.peer.state.lock().unwrap();
                (
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM messages WHERE kind='ordinary'",
                            [],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap(),
                    state
                        .db
                        .query_row("SELECT count(*) FROM invitations", [], |r| {
                            r.get::<_, i64>(0)
                        })
                        .unwrap(),
                )
            };
            super::super::super::super::resolve_automatic_new_selectors(
                &mut parsed,
                &peer,
                &peer.base.clock,
                || Ok(topology()),
                || -> Result<Option<HostTargetId>, ApiError> {
                    panic!("explicit workspace needs no caller pane")
                },
            )
            .unwrap();
            let super::super::super::super::commands::CliAction::TopologyHandoff(request) =
                &parsed.action
            else {
                panic!("parsed route")
            };
            assert!(request.automatic && !request.existing);
            assert_eq!(request.selector.space.as_deref(), Some("work"));
            let mut launcher = DownstreamLauncher::default();
            let mut output = Vec::new();
            assert!(
                super::super::super::super::topology_runtime::try_run_automatic_new_with_ports(
                    &parsed,
                    &peer.base.peer.identity.claim,
                    &peer.base.journal,
                    peer.ns(),
                    &peer,
                    &peer,
                    &mut launcher,
                    &peer.base.clock,
                    peer.base._temp.path(),
                    None,
                    &mut || Ok(topology()),
                    &mut |_| Ok(peer.base.witness.clone()),
                    &crate::protocol::output::OutputSpec {
                        format: crate::protocol::output::OutputFormat::Json,
                        ..Default::default()
                    },
                    &mut output,
                )
                .unwrap()
            );
            let active = peer
                .active_identity
                .lock()
                .unwrap()
                .clone()
                .expect("new parent");
            assert_ne!(active, peer.identity);
            assert_eq!(active.plan.payload.workspace.as_str(), "w1");
            assert_eq!(peer.status_for(&active).state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
            let state = peer.base.peer.state.lock().unwrap();
            assert_eq!(
                state
                    .db
                    .query_row(
                        "SELECT count(*) FROM messages WHERE kind='ordinary'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                before.0 + 1
            );
            assert_eq!(
                state
                    .db
                    .query_row("SELECT count(*) FROM invitations", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                before.1
            );
            let selected = peer.status_for(&active).thread.expect("paired thread");
            assert_eq!(
                state
                    .db
                    .query_row(
                        "SELECT count(*) FROM direct_threads WHERE thread_id=?1",
                        [selected.as_str()],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            assert!(!output.is_empty());
        }

        #[test]
        fn typed_native_zero_advances_attempt_but_unknown_stays_fenced() {
            let zero = AutoPeer::new();
            *zero.native_outcome_once.lock().unwrap() = Some("zero");
            let mut launcher = DownstreamLauncher::default();
            let output = crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                ..Default::default()
            };
            let run_zero = |launcher: &mut DownstreamLauncher| {
                resume_automatic_to_writer(
                    &zero.base.journal,
                    &zero.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    zero.ns(),
                    &zero,
                    &zero,
                    launcher,
                    &zero.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &zero.base.witness,
                        context: &zero.base.context,
                    },
                    &output,
                    &mut Vec::new(),
                )
            };
            assert!(run_zero(&mut launcher).is_err());
            assert_eq!(zero.status().attempt.get(), 2);
            assert_eq!(zero.status().state, BootstrapState::Prepared);
            assert_eq!(zero.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 0);
            assert_eq!(
                run_zero(&mut launcher).unwrap().state,
                BootstrapState::Completed
            );
            assert_eq!(zero.base.peer.state.lock().unwrap().native_calls, 2);
            assert_eq!(launcher.starts, 1);

            let unknown = AutoPeer::new();
            *unknown.native_outcome_once.lock().unwrap() = Some("unknown");
            let mut launcher = DownstreamLauncher::default();
            let mut pending = Vec::new();
            for _ in 0..2 {
                let error = resume_automatic_to_writer(
                    &unknown.base.journal,
                    &unknown.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    unknown.ns(),
                    &unknown,
                    &unknown,
                    &mut launcher,
                    &unknown.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &unknown.base.witness,
                        context: &unknown.base.context,
                    },
                    &output,
                    &mut pending,
                )
                .unwrap_err();
                assert!(matches!(
                    error,
                    RunError::Api(ApiError {
                        code: crate::protocol::results::ErrorCode::UnknownOutcome,
                        ..
                    })
                ));
            }
            assert_eq!(unknown.status().attempt.get(), 1);
            assert_eq!(unknown.status().state, BootstrapState::PossibleCreation);
            assert_eq!(unknown.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 0);
            let report: serde_json::Value =
                serde_json::from_slice(pending.split(|byte| *byte == b'\n').next().unwrap())
                    .unwrap();
            assert_eq!(report["bootstrap"]["outcome"], "creation_unknown");
        }

        #[test]
        fn completed_automatic_retry_is_historical_after_source_binding_changes() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher::default();
            let output = crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                ..Default::default()
            };
            resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &output,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(launcher.starts, 1);
            let calls = peer.base.peer.state.lock().unwrap().calls.len();
            peer.base.peer.state.lock().unwrap().db.execute(
                "UPDATE occupant_bindings SET execution_id='later-binding' WHERE seat_id='sender'", [],
            ).unwrap();
            let recovered = super::super::super::super::retry::load_original_for_actor(
                &peer.base.journal,
                &peer.reference.recovery_ref(),
            )
            .unwrap();
            assert_eq!(recovered.header.reference, peer.reference);
            let mut replay = Vec::new();
            assert!(
                try_completed_automatic(
                    &peer.base.journal,
                    &peer.reference,
                    peer.ns(),
                    &peer,
                    &peer.base.clock,
                    &output,
                    &mut replay,
                )
                .unwrap()
            );
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().calls.len(), calls + 1);
            assert!(std::str::from_utf8(&replay).unwrap().contains("completed"));
        }

        #[test]
        fn lost_automatic_ensure_or_send_reply_replays_one_exact_child() {
            for lost in ["auto_child_ensure", "auto_child_send"] {
                let peer = AutoPeer::new();
                *peer.lost_child_reply.lock().unwrap() = Some(lost);
                let mut launcher = DownstreamLauncher::default();
                let output = crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                };
                {
                    let mut run = || {
                        resume_automatic_to_writer(
                            &peer.base.journal,
                            &peer.reference,
                            super::super::super::super::actor_route::InvocationActor::Agent,
                            peer.ns(),
                            &peer,
                            &peer,
                            &mut launcher,
                            &peer.base.clock,
                            BootstrapSubmissionInputs {
                                witness: &peer.base.witness,
                                context: &peer.base.context,
                            },
                            &output,
                            &mut Vec::new(),
                        )
                    };
                    let first = run().unwrap_err();
                    assert!(matches!(first, RunError::Api(_)), "{lost}: {first:?}");
                    assert_eq!(run().unwrap().state, BootstrapState::Completed, "{lost}");
                }
                assert_eq!(
                    peer.base.peer.state.lock().unwrap().native_calls,
                    1,
                    "{lost}"
                );
                assert_eq!(launcher.starts, 1, "{lost}");
                let state = peer.base.peer.state.lock().unwrap();
                assert_eq!(
                    state
                        .db
                        .query_row("SELECT count(*) FROM invitations", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "{lost}: a pair auto-joins and records no invitation"
                );
                assert_eq!(
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM messages WHERE kind='ordinary'",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    1,
                    "{lost}"
                );
            }
        }

        #[test]
        fn bound_recipient_leave_after_lost_ensure_refuses_send() {
            let peer = AutoPeer::new();
            *peer.lost_child_reply.lock().unwrap() = Some("auto_child_ensure");
            let mut launcher = DownstreamLauncher::default();
            let output = crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                ..Default::default()
            };
            let mut first_output = Vec::new();
            assert!(
                resume_automatic_to_writer(
                    &peer.base.journal,
                    &peer.reference,
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    peer.ns(),
                    &peer,
                    &peer,
                    &mut launcher,
                    &peer.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &peer.base.witness,
                        context: &peer.base.context
                    },
                    &output,
                    &mut first_output,
                )
                .is_err()
            );
            let status = peer.status();
            let thread = status.thread.expect("Ensure selected canonical pair");
            let attachment = status.attachment.unwrap();
            // Item 17: the pair auto-joined the recipient; there is no invitation to reject.
            let (membership, invitations): (String, i64) = {
                let state = peer.base.peer.state.lock().unwrap();
                (
                    state
                        .db
                        .query_row(
                            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                            rusqlite::params![thread.as_str(), attachment.resolved_seat.as_str()],
                            |row| row.get(0),
                        )
                        .unwrap(),
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM invitations WHERE thread_id=?1",
                            rusqlite::params![thread.as_str()],
                            |row| row.get(0),
                        )
                        .unwrap(),
                )
            };
            assert_eq!((membership.as_str(), invitations), ("joined", 0));

            // The recipient is a real newly mapped seat. Give that pane its own
            // lifecycle check-in before exercising the ordinary Leave command.
            let mut recipient = peer.identity.claim.clone();
            recipient.seat = attachment.resolved_seat;
            recipient.target = attachment.created.root_pane;
            recipient.binding_generation = 1;
            // A real harness session (a `plugin_context:` placeholder is refused) and the
            // harness process a lifecycle check-in records.
            recipient.native_session =
                crate::protocol::ids::NativeSessionId::new("recipient-session");
            recipient.liveness = Some(crate::protocol::authority::LivenessFingerprint::TEST);
            recipient.execution =
                crate::protocol::ids::ExecutionId::new(uuid::Uuid::new_v4().to_string());
            let check_in = crate::protocol::commands::CheckIn {
                mode: crate::protocol::commands::CheckInMode::Lifecycle {
                    expected_binding_generation: 1,
                },
                claim: recipient,
                operation: crate::protocol::ids::OperationId::new(uuid::Uuid::new_v4().to_string()),
                lineage_parent: None,
            };
            let permit = peer
                .store
                .issue_cooperative_permit(
                    crate::store::cooperative_permit_request(
                        &crate::protocol::commands::PermitMutation::CheckIn(check_in.clone()),
                    )
                    .unwrap(),
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap();
            let crate::protocol::results::CommandResult::CheckedIn(checked) = peer
                .store
                .register_available(
                    crate::ports::RegisterAvailableRequest {
                        command: check_in,
                        read: crate::ports::ReadContext {
                            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                            output: Default::default(),
                            operation_scope: None,
                        },
                        operator: None,
                    },
                    permit,
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap()
            else {
                panic!("recipient check-in result")
            };
            let leave = crate::protocol::commands::Leave {
                thread,
                operation: crate::protocol::ids::OperationId::new(uuid::Uuid::new_v4().to_string()),
                claim: checked.context,
            };
            let mutation = crate::protocol::commands::PermitMutation::Leave(leave);
            let permit = peer
                .store
                .issue_cooperative_permit(
                    crate::store::cooperative_permit_request(&mutation).unwrap(),
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap();
            peer.store
                .mutate(
                    mutation,
                    permit,
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap();
            let mut second_output = Vec::new();
            let error = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &output,
                &mut second_output,
            )
            .unwrap_err();
            assert!(format!("{error:?}").contains("reinvite"), "{error:?}");
            assert_eq!(launcher.starts, 0);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert!(peer.base.journal.load(&peer.reference).is_ok());
            assert_eq!(
                peer.base
                    .peer
                    .state
                    .lock()
                    .unwrap()
                    .db
                    .query_row(
                        "SELECT count(*) FROM messages WHERE kind='ordinary'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0
            );
            assert!(
                std::str::from_utf8(&second_output)
                    .unwrap()
                    .contains(&peer.reference.recovery_ref())
            );
        }

        #[test]
        fn completed_child_reconciles_outer_without_relaunch_or_recreate() {
            let peer = AutoPeer::new();
            peer.fail_outer_complete_once
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let mut launcher = DownstreamLauncher::default();
            let output = crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                ..Default::default()
            };
            let first = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &output,
                &mut Vec::new(),
            )
            .unwrap_err();
            assert!(matches!(first, RunError::Api(_)));
            assert_eq!(peer.status().state, BootstrapState::Attached);
            assert_eq!(launcher.starts, 1);
            let second = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &output,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(second.state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
        }

        #[test]
        fn public_historical_retry_reconciles_completed_child_after_sender_check_in() {
            use super::super::super::super::{
                actor_route::InvocationActor, journal::OriginalActor,
            };

            let peer = AutoPeer::new();
            peer.fail_outer_complete_once
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let mut launcher = DownstreamLauncher::default();
            let output = crate::protocol::output::OutputSpec {
                format: crate::protocol::output::OutputFormat::Json,
                context: crate::protocol::output::ContinuationContext {
                    state_dir: Some(peer.ns().state_dir.to_string_lossy().into_owned()),
                    host: Some(peer.ns().host_endpoint.to_string_lossy().into_owned()),
                },
            };
            let original = peer
                .base
                .journal
                .snapshot_bootstrap_origin(&peer.reference)
                .unwrap();
            assert!(
                resume_automatic_to_writer(
                    &peer.base.journal,
                    &peer.reference,
                    InvocationActor::Agent,
                    peer.ns(),
                    &peer,
                    &peer,
                    &mut launcher,
                    &peer.base.clock,
                    BootstrapSubmissionInputs {
                        witness: &peer.base.witness,
                        context: &peer.base.context,
                    },
                    &output,
                    &mut Vec::new(),
                )
                .is_err()
            );
            let attached = peer.status();
            assert_eq!(attached.state, BootstrapState::Attached);
            let attachment = attached.attachment.as_ref().unwrap();
            let mut child_query = attachment.child.clone();
            child_query.action = AutomaticDeliveryAction::Status;
            let CommandResult::Handoff(child) = peer
                .store
                .automatic_delivery_query(
                    peer.ns(),
                    &child_query,
                    &super::super::super::super::cooperative_budget(&peer.base.clock),
                )
                .unwrap()
            else {
                panic!("canonical automatic child status")
            };
            assert_eq!(child.state, HandoffState::Completed);
            let progress_path = peer.base.journal.root().join(format!(
                "automatic-child-{}.progress",
                peer.reference.operation.as_str()
            ));
            let progress: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&progress_path).unwrap()).unwrap();
            let saved_report = progress["launch"].clone();
            assert_eq!(saved_report["outcome"], "started");
            let before = {
                let state = peer.base.peer.state.lock().unwrap();
                (
                    state.native_calls,
                    state
                        .db
                        .query_row("SELECT count(*) FROM invitations", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap(),
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM messages WHERE kind='ordinary'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap(),
                )
            };
            let successor = peer.sender_lifecycle_check_in();
            assert_ne!(successor, peer.identity.claim);
            // Session-keyed bindings: a lifecycle check-in of the same session re-registers its
            // binding in place (new execution, same generation).
            assert_eq!(
                successor.binding_generation,
                peer.identity.claim.binding_generation
            );
            assert_ne!(successor.execution, peer.identity.claim.execution);

            let recovery = peer.reference.recovery_ref();
            let mut parsed = super::super::super::super::commands::parse_argv([
                "ht",
                "retry",
                recovery.as_str(),
            ])
            .unwrap();
            parsed.output = output.clone();
            assert_eq!(
                super::super::super::super::retry::preflight_original_actor(
                    peer.base.journal.root(),
                    &peer.reference.recovery_ref(),
                    parsed.actor,
                    &parsed.output.context,
                )
                .unwrap(),
                OriginalActor::Agent
            );
            let mut replay = Vec::new();
            assert!(
                super::super::super::super::topology_runtime::try_completed_with_namespace(
                    &parsed,
                    &peer.base.journal,
                    peer.ns(),
                    &peer,
                    &peer.base.clock,
                    &mut replay,
                )
                .unwrap(),
                "historical retry must absorb completed child before live caller selection"
            );
            assert_eq!(peer.status().state, BootstrapState::Completed);
            assert_eq!(launcher.starts, 1);
            let after = {
                let state = peer.base.peer.state.lock().unwrap();
                (
                    state.native_calls,
                    state
                        .db
                        .query_row("SELECT count(*) FROM invitations", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap(),
                    state
                        .db
                        .query_row(
                            "SELECT count(*) FROM messages WHERE kind='ordinary'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap(),
                )
            };
            assert_eq!(after, before);
            let report: serde_json::Value = serde_json::from_slice(&replay).unwrap();
            assert_eq!(report["bootstrap"]["state"], "completed");
            assert_eq!(
                report["bootstrap"]["launch"],
                crate::cli::handoff::automatic_launch_presentation(&saved_report)
            );
            let terminal = peer.base.journal.root().join(format!(
                "automatic-bootstrap-{:020}-{}.terminal",
                peer.reference.ordinal,
                peer.reference.operation.as_str()
            ));
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(terminal).unwrap()).unwrap();
            assert_eq!(saved["original"].as_str().unwrap().as_bytes(), original);
            assert_eq!(saved["completed"]["completed"]["report"], saved_report);
            assert!(peer.base.journal.load(&peer.reference).is_err());
        }

        #[test]
        fn historical_retry_refuses_missing_or_contradictory_successful_launch() {
            for missing in [true, false] {
                let peer = AutoPeer::new();
                let launcher = peer.complete_child_with_lost_parent_reply();
                assert_eq!(launcher.starts, 1);
                peer.sender_lifecycle_check_in();
                let path = peer.base.journal.root().join(format!(
                    "automatic-child-{}.progress",
                    peer.reference.operation.as_str()
                ));
                let mut progress: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                if missing {
                    progress["launch"] = serde_json::Value::Null;
                } else {
                    progress["launch"]["outcome"] = serde_json::json!("possible_start");
                }
                std::fs::write(&path, serde_json::to_vec(&progress).unwrap()).unwrap();
                let before = peer.base.peer.state.lock().unwrap().native_calls;
                let result = peer.historical_retry(peer.ns());
                assert!(
                    matches!(result, Err(RunError::Api(_))),
                    "missing={missing}: {result:?}"
                );
                assert_eq!(peer.status().state, BootstrapState::Attached);
                assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, before);
                assert_eq!(launcher.starts, 1);
            }
        }

        #[test]
        fn historical_retry_refuses_wrong_namespace_and_changed_original() {
            let peer = AutoPeer::new();
            let launcher = peer.complete_child_with_lost_parent_reply();
            peer.sender_lifecycle_check_in();
            let mut other_namespace = peer.ns().clone();
            other_namespace.host_endpoint = peer.base._temp.path().join("different-host.sock");
            assert!(peer.historical_retry(&other_namespace).is_err());
            let path = peer.base.journal.root().join(format!(
                "{:020}-{}.intent",
                peer.reference.ordinal,
                peer.reference.operation.as_str()
            ));
            let original = std::fs::read_to_string(&path).unwrap();
            let (header, body) = original.split_once('\n').unwrap();
            let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
            header["semantic_digest"] = serde_json::json!("changed-original-digest");
            std::fs::write(&path, format!("{header}\n{body}")).unwrap();
            assert!(
                super::super::super::super::retry::preflight_original_actor(
                    peer.base.journal.root(),
                    &peer.reference.recovery_ref(),
                    super::super::super::super::actor_route::InvocationActor::Agent,
                    &Default::default(),
                )
                .is_err()
            );
            assert_eq!(peer.status().state, BootstrapState::Attached);
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
        }

        #[test]
        fn historical_retry_does_not_admit_live_child_with_possible_start() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher {
                unknown: true,
                ..Default::default()
            };
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec::default(),
                &mut Vec::new(),
            );
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: crate::protocol::results::ErrorCode::UnknownOutcome,
                    ..
                }))
            ));
            peer.sender_lifecycle_check_in();
            // Session-keyed bindings: the sender's lifecycle check-in re-registers its own binding
            // in place (same session, same binding generation), so the frozen caller is no longer
            // stale by generation. Either way the historical layer admits nothing: it refuses the
            // caller, or reports the bootstrap not completed and absorbs nothing.
            let retried = peer.historical_retry(peer.ns());
            assert!(
                matches!(
                    retried,
                    Err(RunError::Api(ApiError {
                        code: crate::protocol::results::ErrorCode::CallerUnverified,
                        ..
                    })) | Ok((false, _))
                ),
                "{retried:?}"
            );
            assert_eq!(peer.status().state, BootstrapState::Attached);
            assert_eq!(launcher.starts, 1);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
        }

        #[test]
        fn lost_guarded_attachment_reply_never_recreates_topology() {
            let peer = AutoPeer::new();
            *peer.lost_parent_reply.lock().unwrap() = Some("auto_attach");
            let mut launcher = DownstreamLauncher::default();
            let result = resume_automatic_to_writer(
                &peer.base.journal,
                &peer.reference,
                super::super::super::super::actor_route::InvocationActor::Agent,
                peer.ns(),
                &peer,
                &peer,
                &mut launcher,
                &peer.base.clock,
                BootstrapSubmissionInputs {
                    witness: &peer.base.witness,
                    context: &peer.base.context,
                },
                &crate::protocol::output::OutputSpec {
                    format: crate::protocol::output::OutputFormat::Json,
                    ..Default::default()
                },
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(result.state, BootstrapState::Completed);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
        }

        #[test]
        fn lost_start_outcome_stays_pending_without_second_start() {
            let peer = AutoPeer::new();
            let mut launcher = DownstreamLauncher {
                unknown: true,
                ..Default::default()
            };
            let mut pending = Vec::new();
            {
                let mut run = || {
                    resume_automatic_to_writer(
                        &peer.base.journal,
                        &peer.reference,
                        super::super::super::super::actor_route::InvocationActor::Agent,
                        peer.ns(),
                        &peer,
                        &peer,
                        &mut launcher,
                        &peer.base.clock,
                        BootstrapSubmissionInputs {
                            witness: &peer.base.witness,
                            context: &peer.base.context,
                        },
                        &crate::protocol::output::OutputSpec {
                            format: crate::protocol::output::OutputFormat::Json,
                            ..Default::default()
                        },
                        &mut pending,
                    )
                };
                assert!(matches!(
                    run(),
                    Err(RunError::Api(ApiError {
                        code: crate::protocol::results::ErrorCode::UnknownOutcome,
                        ..
                    }))
                ));
                assert!(matches!(
                    run(),
                    Err(RunError::Api(ApiError {
                        code: crate::protocol::results::ErrorCode::UnknownOutcome,
                        ..
                    }))
                ));
            }
            let report: serde_json::Value =
                serde_json::from_slice(pending.split(|byte| *byte == b'\n').next().unwrap())
                    .unwrap();
            assert_eq!(report["bootstrap"]["outcome"], "possible_start");
            assert_eq!(report["bootstrap"]["possible_start"], true);
            assert_eq!(
                report["bootstrap"]["recovery_ref"],
                peer.reference.recovery_ref()
            );
            assert!(
                report["bootstrap"]["guidance"]
                    .as_str()
                    .unwrap()
                    .contains("No topology cleanup")
            );
            assert_eq!(peer.status().state, BootstrapState::Attached);
            assert_eq!(peer.base.peer.state.lock().unwrap().native_calls, 1);
            assert_eq!(launcher.starts, 1);
        }
    }
}

#[test]
fn phase13_prepublication_capacity_uses_actual_request_envelope_bound() {
    let mut identity = crate::protocol::handoff::topology_contract_tests::identity();
    let witness = crate::protocol::handoff::topology_contract_tests::created().witness;
    super::preflight_progress_capacity(&identity, &witness).unwrap();
    identity.payload.handoff.body = "\u{1}".repeat(65_536);
    identity.payload.launch.argv = vec!["\"".repeat(4096); 16];
    let excess = serde_json::to_vec(&identity).unwrap().len()
        - crate::store::topology_handoff::MAX_IDENTITY_BYTES;
    let last = identity.payload.launch.argv.last_mut().unwrap();
    last.truncate(last.len() - excess.div_ceil(2));
    if !excess.is_multiple_of(2) {
        last.push('x');
    }
    identity.digest = identity.semantic_digest().unwrap();
    let encoded = crate::store::topology_handoff::encode_identity(
        &identity.payload.handoff.namespace,
        &identity,
    )
    .unwrap();
    assert_eq!(
        encoded.len(),
        crate::store::topology_handoff::MAX_IDENTITY_BYTES
    );
    assert!(
        super::preflight_progress_capacity(&identity, &witness).is_err(),
        "a valid canonical max identity must refuse when genuine request metadata cannot fit"
    );
}

#[test]
fn current_main_topology_native_argv_preserves_empty_tokens() {
    let temp = Temp::new();
    let mut request = request();
    request.kind = Some("claude".into());
    request.argv = vec!["--tools".into(), String::new()];
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: temp.path().join("state"),
        host_endpoint: "/host.sock".into(),
    };
    let identity = prepare(
        &request,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: temp.path(),
            options: None,
            client: &ReadOnly,
            clock: &crate::app::SystemClock::new(),
        },
    )
    .expect("legal empty Claude tools value must survive actual topology preparation");
    assert_eq!(identity.payload.launch.argv, vec!["--tools", ""]);
    assert_eq!(
        identity.payload.launch.harness,
        crate::protocol::authority::Harness::Claude
    );
}

fn fourth_request() -> Request {
    Request {
        kind: Some("synthetic_fourth".into()),
        binary: None,
        argv: vec![],
        ..request()
    }
}
/// V1 plans keep their exact bytes: no V2 field, plan version 1.
#[test]
fn v1_plan_payload_is_unchanged_by_bootstrap_v2() {
    let temp = Temp::new();
    let identity = prepared(&request(), temp.path()).unwrap();
    assert_eq!(identity.payload.plan_version(), 1);
    assert!(identity.payload.launch.composed_argv.is_none());
    let launch = serde_json::to_value(&identity.payload.launch).unwrap();
    assert!(launch.get("composed_argv").is_none(), "{launch}");
    assert!(identity.payload.launch.startup_input.is_none());
    assert!(launch.get("startup_input").is_none(), "{launch}");
    let plan = downstream_plan(
        &identity,
        &created_tab(),
        &crate::protocol::ids::SeatId::new("recipient"),
    )
    .unwrap();
    assert!(plan.bootstrap_input.is_none());
    let plan_json = serde_json::to_value(&plan).unwrap();
    assert!(plan_json.get("bootstrap_input").is_none(), "{plan_json}");
    // A pre-V2 stored payload (no such key) still reads back and validates.
    let legacy: crate::protocol::handoff::BootstrapPayload =
        serde_json::from_value(serde_json::to_value(&identity.payload).unwrap()).unwrap();
    assert_eq!(legacy, identity.payload);
    legacy.validate().unwrap();
}
/// A launch-capable adapter V1 refuses gets a V2 plan that freezes the adapter's composition,
/// round-trips through the journal, and carries plan version 2.
#[test]
fn v2_plan_freezes_adapter_composition_and_round_trips() {
    let temp = Temp::new();
    let identity = prepared(&fourth_request(), temp.path()).unwrap();
    assert_eq!(identity.payload.plan_version(), 2);
    assert_eq!(identity.payload.launch.harness.as_str(), "synthetic_fourth");
    let composed = identity.payload.launch.composed_argv.clone().unwrap();
    assert_eq!(
        composed,
        crate::harness::launch::compose_bootstrap_v2_argv(
            identity.payload.launch.harness,
            identity.payload.launch.argv.clone()
        )
        .unwrap()
    );
    assert_eq!(composed[0], "--synthetic-only");
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let reference = publish(&journal, &identity, 1).expect("V2 publication must succeed");
    let loaded = journal.load(&reference).unwrap();
    assert_eq!(loaded.header.semantic_digest, identity.digest);
    let super::super::journal::SemanticMutation::Frozen { mutation, .. } = loaded.semantic else {
        panic!("missing frozen semantic")
    };
    let super::super::journal::SemanticMutation::HandoffBootstrap(plan) = *mutation else {
        panic!("missing bootstrap")
    };
    assert_eq!(plan.version, 2);
    assert_eq!(plan.payload, identity.payload);
    let again: super::super::journal::BootstrapPlan =
        serde_json::from_slice(&serde_json::to_vec(&*plan).unwrap()).unwrap();
    assert_eq!(again, *plan);
    again.validate().unwrap();
}
/// The plan version and the payload must agree: V1 never carries a frozen composition and V2
/// never lacks one.
#[test]
fn bootstrap_plan_version_must_match_its_payload() {
    let temp = Temp::new();
    let v1 = prepared(&request(), temp.path()).unwrap().payload;
    let v2 = prepared(&fourth_request(), temp.path()).unwrap().payload;
    let plan = |version, payload: &crate::protocol::handoff::BootstrapPayload| {
        super::super::journal::BootstrapPlan {
            version,
            payload: payload.clone(),
        }
    };
    plan(1, &v1).validate().unwrap();
    plan(2, &v2).validate().unwrap();
    assert!(plan(2, &v1).validate().is_err());
    assert!(plan(1, &v2).validate().is_err());
    assert!(plan(3, &v2).validate().is_err());
}
/// V2 retained-launch validation accepts the frozen composition, refuses a changed argv, and
/// refuses a plan whose adapter composes the caller arguments differently from the freeze.
#[test]
fn v2_retained_argv_validation_accepts_frozen_and_refuses_changed() {
    let temp = Temp::new();
    let identity = prepared(&fourth_request(), temp.path()).unwrap();
    let launch = &identity.payload.launch;
    let ns = &identity.payload.handoff.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(ns.state_dir.to_string_lossy().into_owned()),
        host: Some(ns.host_endpoint.to_string_lossy().into_owned()),
    };
    let instance = &identity.claim.instance;
    let prompt = super::super::handoff::bootstrap_v1::prompt("t1", &context, instance);
    let mut caller = launch.argv.clone();
    caller.push(prompt);
    let frozen = serde_json::json!(
        crate::harness::launch::compose_bootstrap_v2_argv(launch.harness, caller.clone()).unwrap()
    );
    let check = |launch: &crate::protocol::handoff::BootstrapLaunch,
                 retained: &serde_json::Value| {
        super::super::handoff::retained_launch_argv_matches(
            launch,
            "t1",
            &context,
            instance,
            Some(retained),
        )
    };
    assert!(check(launch, &frozen).unwrap());
    let mut changed = frozen.clone();
    changed.as_array_mut().unwrap().push("--extra".into());
    assert!(!check(launch, &changed).unwrap());
    let mut other_prompt = frozen.clone();
    *other_prompt.as_array_mut().unwrap().last_mut().unwrap() = "invented prompt".into();
    assert!(!check(launch, &other_prompt).unwrap());
    let mut drifted = launch.clone();
    drifted.composed_argv.as_mut().unwrap().push("--old".into());
    let refusal = check(&drifted, &frozen).unwrap_err();
    assert!(
        refusal.detail.contains("synthetic_fourth") && refusal.detail.contains("V2"),
        "{refusal:?}"
    );
}
fn created_tab() -> crate::ports::CreatedTab {
    use crate::host::continuity::{LocalEndpointWitness, SocketIdentity};
    use crate::protocol::ids::{HostBootId, HostCallId, TerminalId};
    crate::ports::CreatedTab {
        correlation: HostCallId::new("correlation"),
        workspace: HostTargetId::new("w1"),
        tab: HostTargetId::new("w1:t2"),
        root_pane: HostTargetId::new("w1:p2"),
        terminal: TerminalId::new("terminal"),
        host_incarnation: HostBootId::new("herdr-server:pid=42:start=1.000002:uid=501"),
        witness: LocalEndpointWitness {
            schema: 1,
            platform: "macos-proc-bsdinfo-v1".into(),
            endpoint: "/host.sock".into(),
            peer_uid: 501,
            peer_pid: 42,
            start_seconds: 1,
            start_microseconds: 2,
            socket: SocketIdentity {
                device: 1,
                inode: 2,
                birth_seconds: 1,
                birth_nanoseconds: 0,
                change_seconds: 1,
                change_nanoseconds: 0,
            },
        },
    }
}
fn kind_request(kind: &str, argv: &[&str]) -> Request {
    Request {
        kind: Some(kind.into()),
        binary: None,
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        ..request()
    }
}
fn identity_context(
    identity: &crate::protocol::handoff::BootstrapIdentity,
) -> crate::protocol::output::ContinuationContext {
    let ns = &identity.payload.handoff.namespace;
    crate::protocol::output::ContinuationContext {
        state_dir: Some(ns.state_dir.to_string_lossy().into_owned()),
        host: Some(ns.host_endpoint.to_string_lossy().into_owned()),
    }
}
/// `handoff --new-tab --kind agy` places the frozen V1 prompt with `--prompt-interactive`
/// instead of the bare positional agy's flag grammar refuses.
#[test]
fn v2_new_tab_agy_places_the_prompt_with_prompt_interactive() {
    let temp = Temp::new();
    let identity = prepared(&kind_request("agy", &["--effort", "high"]), temp.path())
        .expect("agy new-tab preparation must succeed");
    assert_eq!(identity.payload.plan_version(), 2);
    let template = identity.payload.launch.startup_input.clone().unwrap();
    assert_eq!(template.before, ["--prompt-interactive"]);
    assert_eq!(template.insertion_index, 2);
    let fresh = preflight_launch_request(&identity).unwrap();
    let crate::protocol::handoff::HandoffChannel::Existing { thread } =
        &identity.payload.handoff.channel
    else {
        panic!("fixture uses an existing thread")
    };
    let prompt = super::super::handoff::bootstrap_v1::prompt(
        thread.as_str(),
        &identity_context(&identity),
        &identity.claim.instance,
    );
    assert_eq!(
        fresh.argv,
        ["--effort", "high", "--prompt-interactive", prompt.as_str()]
    );
    // The adapter accepts exactly this composition.
    crate::harness::launch::compose_bootstrap_v2_argv(
        identity.payload.launch.harness,
        fresh.argv.clone(),
    )
    .unwrap();
    // The downstream plan carries the frozen placement.
    let plan = downstream_plan(
        &identity,
        &created_tab(),
        &crate::protocol::ids::SeatId::new("recipient"),
    )
    .unwrap();
    assert_eq!(plan.bootstrap_input, Some(template));
    assert!(plan.startup_input.is_none());
    plan.validate().unwrap();
}
/// Hermes places the same frozen text with `--query`.
#[test]
fn v2_new_tab_hermes_places_the_prompt_with_query() {
    let temp = Temp::new();
    let identity = prepared(&kind_request("hermes", &["--model", "m"]), temp.path())
        .expect("hermes new-tab preparation must succeed");
    assert_eq!(identity.payload.plan_version(), 2);
    let template = identity.payload.launch.startup_input.clone().unwrap();
    assert_eq!(template.before, ["--query"]);
    let fresh = preflight_launch_request(&identity).unwrap();
    assert_eq!(&fresh.argv[..3], ["--model", "m", "--query"]);
    assert_eq!(fresh.argv.len(), 4);
    assert!(fresh.argv[3].starts_with("Expected handoff command routing"));
    let (expected, _) = template
        .apply(&identity.payload.launch.argv, &fresh.argv[3])
        .unwrap();
    assert_eq!(fresh.argv, expected);
    crate::harness::launch::compose_bootstrap_v2_argv(
        identity.payload.launch.harness,
        fresh.argv.clone(),
    )
    .unwrap();
}
/// Retained validation applies the template frozen in the plan; the V1 bare-positional form is
/// not a V2 composition.
#[test]
fn v2_retained_argv_validation_uses_the_frozen_template() {
    let temp = Temp::new();
    let identity = prepared(&kind_request("agy", &["--effort", "high"]), temp.path()).unwrap();
    let launch = &identity.payload.launch;
    let context = identity_context(&identity);
    let instance = &identity.claim.instance;
    let prompt = super::super::handoff::bootstrap_v1::prompt("t1", &context, instance);
    let placed = launch
        .startup_input
        .as_ref()
        .unwrap()
        .apply(&launch.argv, &prompt)
        .unwrap()
        .0;
    let retained = serde_json::json!(
        crate::harness::launch::compose_bootstrap_v2_argv(launch.harness, placed).unwrap()
    );
    let check = |launch: &crate::protocol::handoff::BootstrapLaunch,
                 retained: &serde_json::Value| {
        super::super::handoff::retained_launch_argv_matches(
            launch,
            "t1",
            &context,
            instance,
            Some(retained),
        )
    };
    assert!(check(launch, &retained).unwrap());
    let mut bare = launch.argv.clone();
    bare.push(prompt);
    let bare = serde_json::json!(bare);
    assert!(!matches!(check(launch, &bare), Ok(true)));
    // A V2 launch without its frozen template is refused, never matched.
    let mut stripped = launch.clone();
    stripped.startup_input = None;
    assert!(check(&stripped, &retained).is_err());
}
/// A V2 payload carries a template exactly when it carries a composition.
#[test]
fn v2_payload_requires_template_iff_composed_argv() {
    let temp = Temp::new();
    let v2 = prepared(&kind_request("agy", &[]), temp.path())
        .unwrap()
        .payload;
    let v1 = prepared(&request(), temp.path()).unwrap().payload;
    v2.validate().unwrap();
    v1.validate().unwrap();
    let mut missing = v2.clone();
    missing.launch.startup_input = None;
    assert!(missing.validate().is_err());
    let mut extra = v1.clone();
    extra.launch.startup_input = Some(crate::harness::adapter::StartupInputTemplate::positional(
        v1.launch.argv.len(),
    ));
    assert!(extra.validate().is_err());
    let plan = |version, payload: &crate::protocol::handoff::BootstrapPayload| {
        super::super::journal::BootstrapPlan {
            version,
            payload: payload.clone(),
        }
    };
    plan(2, &v2).validate().unwrap();
    assert!(plan(2, &missing).validate().is_err());
    assert!(plan(1, &extra).validate().is_err());
    // A template that no longer fits the caller arguments is refused too.
    let mut out_of_range = v2.clone();
    out_of_range
        .launch
        .startup_input
        .as_mut()
        .unwrap()
        .insertion_index = 9;
    assert!(out_of_range.validate().is_err());
}
/// Fresh preparation refuses a plan whose frozen placement differs from today's adapter.
#[test]
fn v2_startup_input_drift_is_refused_at_preparation() {
    let temp = Temp::new();
    let mut identity = prepared(&kind_request("agy", &["--effort", "high"]), temp.path()).unwrap();
    preflight_launch_argv(&identity).unwrap();
    identity
        .payload
        .launch
        .startup_input
        .as_mut()
        .unwrap()
        .before
        .push("--x".into());
    let refusal = preflight_launch_argv(&identity).unwrap_err();
    assert!(
        format!("{refusal:?}").contains("current startup input differs from bootstrap plan V2"),
        "{refusal:?}"
    );
}
/// Both prompt versions keep the 4096-byte retained report cap: a namespace whose quoted
/// pinned fallback alone exceeds it is refused before publication.
#[test]
fn generated_prompt_over_retained_report_cap_is_refused_for_v1_and_summary_prompt() {
    let temp = Temp::new();
    let mut identity = prepared(&request(), temp.path()).unwrap();
    identity.payload.handoff.namespace.state_dir =
        std::path::PathBuf::from(format!("/{}", "\u{1}".repeat(1100)));
    for prompt in [
        None,
        Some(crate::protocol::handoff::SummaryPrompt {
            summary: Some("Fix it".into()),
        }),
    ] {
        identity.payload.launch.summary_prompt = prompt.clone();
        let refusal = preflight_launch_argv(&identity).unwrap_err();
        assert!(
            format!("{refusal:?}").contains("retained report limit"),
            "{prompt:?}: {refusal:?}"
        );
    }
}
/// Fresh preparation keeps today's native admission: a present-day composer
/// refusal stops it before publication even though frozen V1 would accept.
/// The fresh Root generated input is the frozen V1 prompt.
#[test]
fn fresh_root_preparation_uses_current_admission_and_v1_generated_input() {
    let temp = Temp::new();
    let identity = prepared(&request(), temp.path()).unwrap();
    let fresh = preflight_launch_request(&identity).unwrap();
    let namespace = &identity.payload.handoff.namespace;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into_owned()),
        host: Some(namespace.host_endpoint.to_string_lossy().into_owned()),
    };
    assert_eq!(
        fresh.argv.last().unwrap(),
        &super::super::handoff::bootstrap_v1::prompt(
            "canonical-thread",
            &context,
            &identity.claim.instance
        )
    );
    let drift = crate::harness::launch::current_drift::arm(None, Some("saved value"));
    assert!(
        prepared(&request(), temp.path()).is_err(),
        "current composer refusal must stop fresh preparation"
    );
    drop(drift);
    // Today's composer accepts but launches different argv than frozen V1:
    // preparation refuses before any publication or durable work.
    let _drift = crate::harness::launch::current_drift::arm_output("--drifted");
    let refused = prepared(&request(), temp.path());
    assert!(
        format!("{refused:?}").contains("a new bootstrap plan version is required"),
        "{refused:?}"
    );
    assert!(!temp.path().join("intents").exists());
}

#[test]
fn maximum_escaped_bootstrap_survives_origin_and_request_progress() {
    let temp = Temp::new();
    let mut request = request();
    request.body = "\u{1}".repeat(65_536);
    let identity = prepared(&request, temp.path()).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let reference = publish(&journal, &identity, 1).unwrap();
    let origin = journal.snapshot_bootstrap_origin(&reference).unwrap();
    assert!(origin.len() >= 393_216);
    let mut created = crate::protocol::handoff::topology_contract_tests::created();
    created.witness.endpoint = identity.payload.handoff.namespace.host_endpoint.clone();
    preflight_progress_capacity(&identity, &created.witness).unwrap();
    let progress = BootstrapProgress {
        version: 1,
        identity: identity.clone(),
        attempt: crate::protocol::handoff::BootstrapAttempt::first(),
        possible_creation: true,
        request: Some(crate::ports::CreateTabRequest {
            correlation: created.correlation,
            workspace: identity.payload.workspace.clone(),
            cwd: identity.payload.cwd.clone(),
            label: identity.payload.label.clone(),
            focus: identity.payload.focus,
            env: identity.payload.env.clone(),
            directory_context: identity.payload.directory_context.clone(),
            expected_witness: created.witness,
        }),
        creation: None,
        not_submitted: false,
    };
    save_bootstrap_progress(&journal, &reference, &progress).unwrap();
    eprintln!(
        "maximum escaped bootstrap: origin={} progress={}",
        origin.len(),
        serde_json::to_vec(&progress).unwrap().len()
    );
    let loaded = load_bootstrap_progress(&journal, &reference, &identity)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.identity.payload.handoff.body, "\u{1}".repeat(65_536));
}

#[test]
fn automatic_bootstrap_preparation_freezes_v2_without_invite_or_create_child() {
    let temp = Temp::new();
    let mut request = request();
    request.automatic = true;
    request.thread = None;
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: temp.path().join("state dir"),
        host_endpoint: "/host.sock".into(),
    };
    let identity = prepare_automatic(
        &request,
        Preparation {
            claim: &claim(),
            namespace: &namespace,
            topology: &topology(),
            invocation_cwd: temp.path(),
            options: Some("--config 'quoted $HOME'".into()),
            client: &ReadOnly,
            clock: &crate::app::SystemClock::new(),
        },
    )
    .unwrap();
    assert_eq!(identity.plan.payload.workspace.as_str(), "w1");
    assert_eq!(identity.plan.payload.handoff.body, "literal '$HOME' body");
    assert_eq!(
        identity.plan.payload.handoff.channel,
        crate::protocol::handoff::HandoffChannel::Automatic
    );
    assert!(identity.plan.payload.startup_input.is_some());
    let json = serde_json::to_string(&identity).unwrap();
    assert!(!json.contains("invite_key"));
    assert!(!json.contains("create_key"));
    assert!(!temp.path().join("intents").exists());
}

/// Completion compares the retained native argv with the applied caller argv, so the
/// automatic plan admits only an adapter whose composition leaves those arguments unchanged;
/// the others are refused before any effect and pointed at the explicit-channel route.
#[test]
fn automatic_bootstrap_preparation_refuses_a_kind_whose_composition_rewrites_the_caller_argv() {
    let temp = Temp::new();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        state_dir: temp.path().join("state dir"),
        host_endpoint: "/host.sock".into(),
    };
    let prepare = |kind: &str, argv: &[&str]| {
        let mut request = kind_request(kind, argv);
        request.automatic = true;
        request.thread = None;
        prepare_automatic(
            &request,
            Preparation {
                claim: &claim(),
                namespace: &namespace,
                topology: &topology(),
                invocation_cwd: temp.path(),
                options: None,
                client: &ReadOnly,
                clock: &crate::app::SystemClock::new(),
            },
        )
    };
    for (kind, argv) in [
        ("hermes", &[][..]),
        ("hermes", &["--model", "m"][..]),
        ("synthetic_fourth", &[][..]),
    ] {
        let refused = prepare(kind, argv).expect_err(kind);
        let RunError::Api(error) = &refused else {
            panic!("{kind}: {refused:?}")
        };
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::Unsupported,
            "{kind}: {error:?}"
        );
        assert!(
            error.detail.contains("--new-thread") && error.detail.contains("--thread "),
            "{kind}: {error:?}"
        );
        assert!(
            !error.detail.contains(kind),
            "core error names no harness: {error:?}"
        );
    }
    for (kind, argv) in [
        ("claude", &[][..]),
        ("codex", &["--model", "saved value"][..]),
        ("agy", &[][..]),
    ] {
        prepare(kind, argv).unwrap_or_else(|e| panic!("{kind} stays accepted: {e:?}"));
    }
    assert!(!temp.path().join("intents").exists());
}

fn seam_automatic_bootstrap_identity(
    root: &std::path::Path,
) -> crate::protocol::handoff::AutomaticBootstrapIdentity {
    use crate::protocol::handoff::{
        AutomaticBootstrapIdentity, AutomaticBootstrapKeys, AutomaticBootstrapPayload,
        AutomaticBootstrapPlan, AutomaticHandoffPayload,
    };
    let legacy = prepared(&request(), root).unwrap();
    let compound = crate::protocol::ids::OperationId::new("automatic-test-parent");
    let handoff = AutomaticHandoffPayload::new(
        legacy.payload.handoff.namespace.clone(),
        crate::protocol::ids::OperationId::new("automatic-test-child"),
        legacy.payload.handoff.body.clone(),
    )
    .unwrap();
    AutomaticBootstrapIdentity::new(
        legacy.scope,
        legacy.claim,
        AutomaticBootstrapPlan {
            version: 2,
            payload: AutomaticBootstrapPayload {
                directory_context: None,
                handoff,
                keys: AutomaticBootstrapKeys::new(compound),
                workspace: legacy.payload.workspace,
                cwd: legacy.payload.cwd,
                label: legacy.payload.label,
                focus: false,
                env: Default::default(),
                launch: legacy.payload.launch,
                startup_input: None,
            },
        },
    )
    .unwrap()
}

#[test]
fn automatic_bootstrap_retry_loads_its_frozen_original_without_v1_decode() {
    let temp = Temp::new();
    let identity = seam_automatic_bootstrap_identity(temp.path());
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let reference = journal
        .record(identity.scope.clone(), identity.semantic(), 1)
        .unwrap();
    let original =
        super::super::retry::load_original_for_actor(&journal, &reference.recovery_ref()).unwrap();
    assert_eq!(original.header.reference, reference);
    assert_eq!(original.semantic, identity.semantic());
}

/// These are intentionally malformed *local* retained-family collisions. The
/// exact classifier diagnostic must win before either terminal decoder runs.
fn assert_retained_family_collision(
    journal: &super::super::journal::Journal,
    recovery: &str,
    terminal_names: &[String],
    expected: &str,
) {
    use std::collections::BTreeMap;
    for name in terminal_names {
        std::fs::write(journal.root().join(name), b"{not a terminal").unwrap();
    }
    let snapshot = || -> BTreeMap<std::ffi::OsString, Vec<u8>> {
        std::fs::read_dir(journal.root())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (entry.file_name(), std::fs::read(entry.path()).unwrap())
            })
            .collect()
    };
    let before = snapshot();
    let error = super::super::retry::load_original_for_actor(journal, recovery).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), expected);
    assert_eq!(
        snapshot(),
        before,
        "retry classifier changed original files"
    );
    // This loader accepts no client or output port, so refusal cannot send or present.
}

#[test]
fn retained_family_collision_bootstrap_same_kind_refuses_before_decoder() {
    let temp = Temp::new();
    let legacy = prepared(&request(), temp.path()).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let v1 = publish(&journal, &legacy, 1).unwrap();
    assert_retained_family_collision(
        &journal,
        &v1.recovery_ref(),
        &[format!(
            "automatic-bootstrap-{:020}-{}.terminal",
            v1.ordinal,
            v1.operation.as_str()
        )],
        "conflicting original intent and automatic bootstrap terminal",
    );

    let temp = Temp::new();
    let identity = seam_automatic_bootstrap_identity(temp.path());
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let v2 = journal
        .record(identity.scope.clone(), identity.semantic(), 1)
        .unwrap();
    assert_retained_family_collision(
        &journal,
        &v2.recovery_ref(),
        &[format!(
            "bootstrap-{:020}-{}.terminal",
            v2.ordinal,
            v2.operation.as_str()
        )],
        "conflicting original intent and bootstrap terminal",
    );
}

#[test]
fn retained_family_collision_delivery_same_kind_refuses_before_decoder() {
    use super::super::journal::{DeliveryPlan, SemanticMutation};
    use crate::protocol::handoff::{
        AutomaticDeliveryPlan, AutomaticHandoffPayload, AutomaticOriginal,
    };
    use crate::protocol::ids::{OperationId, SeatId};

    let temp = Temp::new();
    let legacy = prepared(&request(), temp.path()).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let v1_semantic = SemanticMutation::freeze(
        SemanticMutation::HandoffDelivery(Box::new(DeliveryPlan {
            version: 1,
            payload: legacy.payload.handoff.clone(),
            recipient: SeatId::new("recipient"),
        })),
        legacy.claim.clone(),
    )
    .unwrap();
    let v1 = journal
        .record(legacy.scope.clone(), v1_semantic, 1)
        .unwrap();
    assert_retained_family_collision(
        &journal,
        &v1.recovery_ref(),
        &[format!(
            "automatic-{:020}-{}.terminal",
            v1.ordinal,
            v1.operation.as_str()
        )],
        "conflicting automatic original",
    );

    let temp = Temp::new();
    let legacy = prepared(&request(), temp.path()).unwrap();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let payload = AutomaticHandoffPayload::new(
        legacy.payload.handoff.namespace.clone(),
        OperationId::new("automatic-delivery-seam"),
        legacy.payload.handoff.body.clone(),
    )
    .unwrap();
    let plan = AutomaticDeliveryPlan::new(payload, SeatId::new("recipient")).unwrap();
    let v2 = journal
        .record(
            legacy.scope,
            AutomaticOriginal::Delivery(Box::new(plan)).semantic(legacy.claim),
            1,
        )
        .unwrap();
    assert_retained_family_collision(
        &journal,
        &v2.recovery_ref(),
        &[format!(
            "delivery-{:020}-{}.terminal",
            v2.ordinal,
            v2.operation.as_str()
        )],
        "conflicting original intent and delivery terminal",
    );
}

#[test]
fn retained_family_collision_two_v2_terminals_refuses_before_decoder() {
    let temp = Temp::new();
    let journal = super::super::journal::Journal::open(temp.path().join("intents")).unwrap();
    let operation = uuid::Uuid::new_v4();
    assert_retained_family_collision(
        &journal,
        "local:1",
        &[
            format!("automatic-{:020}-{operation}.terminal", 1),
            format!("automatic-bootstrap-{:020}-{operation}.terminal", 1),
        ],
        "ambiguous original intent reference",
    );
}
