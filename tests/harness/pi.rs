//! Pi public adapter behavior; process admission uses causal process tables.
use crate::harness::{adapter::*, context::EventKind, registry::builtins};
use serde_json::json;

fn adapter() -> &'static crate::harness::registry::Registration {
    builtins()
        .by_id(builtins().agent("pi").expect("pi is registered"))
        .unwrap()
}
fn input(event: &str, source: &str) -> HookInput {
    HookInput {
        registered_event: Some(event.into()),
        bytes: serde_json::to_vec(&json!({
            "event": event, "source": source, "session_id": "pi-session-1", "role": "top_level",
            "mode": "tui", "pid": 80, "cwd": "/project"
        }))
        .unwrap(),
    }
}
fn budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(5000),
        cancellation: Default::default(),
    }
}
#[test]
fn pi_registers_channel_only_without_hook_mail_or_composer() {
    let row = adapter();
    assert!(builtins().wakes_only_through_channel("pi"));
    assert!(row.composer_policy().is_none());
    assert!(!builtins().binding_turn_delivery("pi", None));
    assert!(!builtins().binding_turn_delivery(
        "pi",
        Some(crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE)
    ));
    assert!(
        builtins()
            .person_guard_markers()
            .any(|m| m.name == "PI_CODING_AGENT")
    );
    assert!(
        !builtins()
            .unsupported_markers()
            .iter()
            .any(|m| m.name == "PI_CODING_AGENT")
    );
}
#[test]
fn pi_lifecycle_current_and_end_decode_without_hook_receipt_authority() {
    let row = adapter();
    let admitted = row
        .admit(
            &AdmissionRequest {
                installed: InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget(),
        )
        .unwrap();
    for (source, kind) in [
        ("startup", EventKind::Startup),
        ("reload", EventKind::Startup),
        ("new", EventKind::Clear),
        ("resume", EventKind::Resume),
        ("fork", EventKind::Fork),
        ("compact", EventKind::Compact),
        ("tree", EventKind::Compact),
    ] {
        let event = row
            .decode(&admitted, &input("SessionStart", source))
            .unwrap();
        assert!(matches!(event.intent, EventIntent::Lifecycle(k) if k == kind));
        assert!(
            matches!(row.encode(&admitted, &event, &NeutralOffer { fixed_guidance: "guidance".into(), peer_data: serde_json::Value::Null, ready_argv: vec![] }).unwrap(), EncodedOutput::ObserverOnly { bytes } if bytes == b"guidance")
        );
    }
    let current = row.decode(&admitted, &input("TurnStart", "")).unwrap();
    assert!(matches!(current.intent, EventIntent::Current));
    let end = row.decode(&admitted, &input("SessionEnd", "")).unwrap();
    assert!(matches!(end.intent, EventIntent::Observer));
    assert_eq!(
        end.binding_event.unwrap().kind,
        crate::protocol::commands::BindingEventKind::SessionEnd
    );
}

struct Table(Vec<crate::ports::ProcessRecord>);
impl crate::ports::ProcessInfo for Table {
    fn process(&self, pid: u32) -> Option<crate::ports::ProcessRecord> {
        self.0.iter().find(|p| p.pid == pid).cloned()
    }
}
fn record(pid: u32, parent: u32, argv: &[&str]) -> crate::ports::ProcessRecord {
    crate::ports::ProcessRecord {
        pid,
        parent: Some(parent),
        start: u64::from(pid) * 10,
        argv: argv.iter().map(|s| s.to_string()).collect(),
    }
}
#[test]
fn pi_process_claim_must_name_the_actual_innermost_entrypoint_for_cli_and_hook() {
    use crate::harness::caller_env::{EnvInputs, derive, harness_fingerprint};
    for argv in [
        vec!["pi"],
        vec!["/opt/pi"],
        vec!["node", "/bin/pi"],
        vec![
            "bun",
            "run",
            "/x/node_modules/@earendil-works/pi-coding-agent/dist/cli.js",
        ],
        vec![
            "node",
            "/x/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js",
        ],
        vec![
            "bun",
            "/x/node_modules/@earendil-works/pi-coding-agent/dist/bun/cli.js",
        ],
    ] {
        let table = Table(vec![
            record(100, 90, &["herdr-threads"]),
            record(90, 80, &["sh"]),
            record(80, 1, &argv),
        ]);
        for (pid, role, mode, sid, valid) in [
            ("80", "top_level", "tui", "s1", true),
            ("81", "top_level", "tui", "s1", false),
            ("0", "top_level", "tui", "s1", false),
            ("x", "top_level", "tui", "s1", false),
            ("80", "child", "tui", "s1", false),
            ("80", "", "tui", "s1", false),
            ("80", "top_level", "rpc", "s1", false),
            ("80", "top_level", "tui", "", false),
        ] {
            let env = |key: &str| match key {
                "HERDR_THREADS_PI_SESSION" => Some(sid.into()),
                "HERDR_THREADS_PI_ROLE" => Some(role.into()),
                "HERDR_THREADS_PI_MODE" => Some(mode.into()),
                "HERDR_THREADS_PI_PID" => Some(pid.into()),
                _ => None,
            };
            let inputs = EnvInputs {
                env: &env,
                self_pid: 100,
                agent_id: None,
                processes: &table,
            };
            let claim = derive(&inputs);
            assert_eq!(
                claim.is_ok(),
                valid,
                "{argv:?}: pid={pid} role={role} mode={mode} sid={sid}"
            );
            assert_eq!(harness_fingerprint(&inputs, "pi").is_ok(), valid);
            if let Ok(claim) = claim {
                assert_eq!(claim.harness.as_str(), "pi");
                assert_eq!(claim.fingerprint.pid, 80);
            }
        }
    }
    let env = |key: &str| match key {
        "HERDR_THREADS_PI_SESSION" => Some("s1".into()),
        "HERDR_THREADS_PI_ROLE" => Some("top_level".into()),
        "HERDR_THREADS_PI_MODE" => Some("tui".into()),
        "HERDR_THREADS_PI_PID" => Some("80".into()),
        _ => None,
    };
    for table in [
        Table(vec![
            record(100, 80, &["herdr-threads"]),
            record(80, 1, &["node", "/project/app.js"]),
        ]),
        Table(vec![
            record(100, 90, &["herdr-threads"]),
            record(90, 80, &["pi"]),
            record(80, 1, &["pi"]),
        ]),
    ] {
        let inputs = EnvInputs {
            env: &env,
            self_pid: 100,
            agent_id: None,
            processes: &table,
        };
        assert!(derive(&inputs).is_err());
        assert!(harness_fingerprint(&inputs, "pi").is_err());
    }
}
#[test]
fn pi_launch_accepts_captured_interactive_grammar_and_redacts_refusals() {
    let policy = adapter().launch_policy().unwrap();
    for argv in [
        vec![],
        vec![
            "--model",
            "provider/model:high",
            "--provider",
            "provider",
            "--thinking",
            "high",
        ],
        vec!["--continue"],
        vec!["--resume"],
        vec!["--fork", "s1"],
        vec![
            "--session",
            "s1",
            "--session-dir",
            "/sessions",
            "--extension",
            "/ext.js",
            "--skill",
            "/skills",
        ],
    ] {
        assert!(
            policy
                .validate_native_argv(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .is_ok(),
            "{argv:?}"
        );
    }
    for argv in [
        vec!["--provider", "provider"],
        vec!["--model=x"],
        vec!["--mode", "rpc"],
        vec!["--print"],
        vec!["-p"],
        vec!["--no-extensions"],
        vec!["--api-key=credential-do-not-print"],
        vec!["-kcredential-do-not-print"],
        vec!["credential-do-not-print"],
        vec!["--thinking", "mystery"],
        vec!["--continue", "--resume"],
        vec!["--session", "s1", "--fork", "s2"],
    ] {
        let err = policy
            .validate_native_argv(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .expect_err("refused");
        assert!(!err.detail.contains("credential-do-not-print"));
    }
    assert_eq!(
        policy
            .explicit_directory_env(std::path::Path::new("/agent"))
            .unwrap()
            .get("PI_CODING_AGENT_DIR")
            .map(String::as_str),
        Some("/agent")
    );
    let template = policy
        .prepare_startup_input(
            &["--continue".into()],
            &StartupInputSpec {
                max_text_bytes: 4096,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(template.insertion_index, 1);
    assert_eq!(template.before, ["--"]);
}
#[test]
fn pi_decode_refuses_foreign_modes_roles_and_malformed_claims() {
    let row = adapter();
    let admitted = row
        .admit(
            &AdmissionRequest {
                installed: InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget(),
        )
        .unwrap();
    for (key, value) in [
        ("role", json!("child")),
        ("role", json!("unknown")),
        ("role", json!(null)),
        ("mode", json!("rpc")),
        ("pid", json!(0)),
        ("pid", json!("80")),
        ("pid", json!(-1)),
        ("session_id", json!("a\nb")),
        ("session_id", json!("")),
        ("source", json!("mystery")),
        ("event", json!("TurnStart")),
    ] {
        let mut i = input("SessionStart", "startup");
        let mut v: serde_json::Value = serde_json::from_slice(&i.bytes).unwrap();
        v[key] = value;
        i.bytes = serde_json::to_vec(&v).unwrap();
        assert!(row.decode(&admitted, &i).is_err(), "{key}");
    }
}

#[test]
fn pi_setup_owns_only_extension_and_skill_preserving_settings_and_edits() {
    let root =
        crate::test_support::scratch::temp_path(format!("ht-pi-setup-{}", uuid::Uuid::new_v4()));
    let home = root.join("home");
    let agent = root.join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(
        agent.join("settings.json"),
        b"{\"extensions\":[\"foreign\"]}",
    )
    .unwrap();
    let env = SetupEnvironment {
        home: Some(home.into()),
        cwd: root.clone(),
        executable: root.join("herdr-threads"),
        state_dir: Some(root.join("state")),
        host_endpoint: Some(root.join("host.sock")),
        declared: [("PI_CODING_AGENT_DIR".into(), agent.clone().into_os_string())].into(),
        ..Default::default()
    };
    let row = adapter();
    let scope = row
        .resolve_setup_scope(&SetupScopeRequest::Default, &env)
        .unwrap();
    assert_eq!(scope, ResolvedSetupScope::ConfigRoot(agent.clone()));
    let request = SetupRequest {
        scope: scope.clone(),
        executable: env.executable.clone(),
        environment: env.clone(),
        native_binary: None,
        options: Default::default(),
    };
    let first = row.setup(&request, &budget()).unwrap();
    assert_eq!(first.projection["action"], "installed");
    let manifest_path = crate::harness::pi::setup::manifest_file(&agent, &env).unwrap();
    let manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.as_object().unwrap().len(), 5);
    assert_eq!(manifest["version"], 1);
    assert_eq!(manifest["files"].as_array().unwrap().len(), 2);
    let extension = agent.join("extensions/herdr-threads/index.js");
    let source = std::fs::read_to_string(&extension).unwrap();
    let launch_line = source
        .lines()
        .find_map(|line| line.strip_prefix("const LAUNCH = "))
        .unwrap();
    let launch: serde_json::Value = serde_json::from_str(
        launch_line
            .split(" // herdr-threads:launch")
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        launch["argv"],
        json!([
            request.executable.to_str().unwrap(),
            "--state-dir",
            request
                .environment
                .state_dir
                .as_ref()
                .unwrap()
                .to_str()
                .unwrap(),
            "--host-endpoint",
            request
                .environment
                .host_endpoint
                .as_ref()
                .unwrap()
                .to_str()
                .unwrap()
        ])
    );
    assert_eq!(
        row.setup(&request, &budget()).unwrap().projection["action"],
        "already_installed"
    );
    assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest_bytes);
    let status = StatusRequest {
        scope: scope.clone(),
        environment: env.clone(),
        native_binary: None,
    };
    assert!(matches!(row.status(&status,&budget()),SetupStatus::Detailed(s) if s.installed));
    std::fs::write(&extension, b"// edited").unwrap();
    assert!(row.setup(&request, &budget()).is_err());
    let removed = row
        .unsetup(
            &UnsetupRequest {
                scope,
                environment: env,
            },
            &budget(),
        )
        .unwrap();
    assert_eq!(removed.residue.as_slice(), std::slice::from_ref(&extension));
    assert_eq!(std::fs::read(extension).unwrap(), b"// edited");
    assert_eq!(
        std::fs::read(agent.join("settings.json")).unwrap(),
        b"{\"extensions\":[\"foreign\"]}"
    );
    assert!(!agent.join("skills/herdr-threads/SKILL.md").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pi_hook_owner_mismatch_refuses_every_event_including_session_end() {
    let env = |key: &str| match key {
        "HERDR_THREADS_PI_SESSION" => Some("pi-session-1".into()),
        "HERDR_THREADS_PI_ROLE" => Some("top_level".into()),
        "HERDR_THREADS_PI_MODE" => Some("tui".into()),
        "HERDR_THREADS_PI_PID" => Some("80".into()),
        _ => None,
    };
    for name in ["SessionStart", "TurnStart", "SessionEnd"] {
        let i = input(name, "startup");
        let fingerprint = || {
            Ok(crate::protocol::authority::LivenessFingerprint {
                pid: 80,
                start: 800,
            })
        };
        assert!(super::witness::hook_evidence(&i, &env, fingerprint()).is_ok());
        for (field, value) in [
            ("session_id", json!("foreign")),
            ("pid", json!(81)),
            ("role", json!("child")),
            ("mode", json!("rpc")),
        ] {
            let mut v: serde_json::Value = serde_json::from_slice(&i.bytes).unwrap();
            v[field] = value;
            let mismatched = HookInput {
                bytes: serde_json::to_vec(&v).unwrap(),
                registered_event: Some(name.into()),
            };
            assert!(
                super::witness::hook_evidence(&mismatched, &env, fingerprint()).is_err(),
                "{name}/{field}"
            );
        }
        assert!(super::witness::hook_evidence(&i, &env, Err("missing process".into())).is_err());
    }
}

#[test]
fn pi_launch_keeps_startup_input_once_and_explicit_directory_avoids_shell_probe() {
    struct NoProbe;
    impl crate::harness::launch::PaneShellProbe for NoProbe {
        fn pane_shell_env(&self, _: &str) -> Option<String> {
            panic!("an explicit profile needs no shell probe")
        }
    }
    let policy = adapter().launch_policy().unwrap();
    let caller = vec!["--model".into(), "provider/model:high".into()];
    let template = policy
        .prepare_startup_input(
            &caller,
            &StartupInputSpec {
                max_text_bytes: 4096,
            },
        )
        .unwrap()
        .unwrap();
    let (argv, slot) = template
        .apply(&caller, "handoff summary; exact recovery command")
        .unwrap();
    let composed = policy.compose_argv(argv.clone(), vec![]).unwrap();
    assert_eq!(
        composed,
        vec![
            "--model",
            "provider/model:high",
            "--",
            "handoff summary; exact recovery command"
        ]
    );
    assert_eq!(slot, 3);
    assert!(
        policy
            .prepare_startup_input(
                &argv,
                &StartupInputSpec {
                    max_text_bytes: 4096
                }
            )
            .is_err()
    );
    let scope = policy
        .resolve_scope(
            &LaunchRequest {
                argv: caller,
                environment: SetupEnvironment {
                    explicit_profile_directory: Some("/explicit-agent".into()),
                    ..Default::default()
                },
                native_binary: None,
            },
            &NoProbe,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX / 2),
                cancellation: Default::default(),
            },
        )
        .unwrap();
    assert_eq!(
        scope.setup,
        ResolvedSetupScope::ConfigRoot("/explicit-agent".into())
    );
}

#[test]
fn pi_setup_preserves_unowned_extension_and_resolves_override_tilde_and_relative_roots() {
    let row = adapter();
    for (value, want) in [
        (None, "/home/user/.pi/agent"),
        (Some("/agent"), "/agent"),
        (Some("~/custom"), "/home/user/custom"),
        (Some("relative"), "/project/relative"),
    ] {
        let environment = SetupEnvironment {
            home: Some("/home/user".into()),
            cwd: "/project".into(),
            declared: value
                .map(|v| ("PI_CODING_AGENT_DIR".into(), v.into()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            row.resolve_setup_scope(&SetupScopeRequest::Default, &environment)
                .unwrap(),
            ResolvedSetupScope::ConfigRoot(want.into())
        );
    }
    let scratch =
        crate::test_support::scratch::temp_path(format!("ht-pi-foreign-{}", uuid::Uuid::new_v4()));
    let root = scratch.join("agent");
    let extension = root.join("extensions/herdr-threads/index.js");
    std::fs::create_dir_all(extension.parent().unwrap()).unwrap();
    std::fs::write(&extension, b"foreign").unwrap();
    let env = SetupEnvironment {
        home: Some(scratch.join("home").into()),
        cwd: scratch.clone(),
        executable: scratch.join("herdr-threads"),
        state_dir: Some(scratch.join("state")),
        host_endpoint: Some(scratch.join("host.sock")),
        ..Default::default()
    };
    let scope = ResolvedSetupScope::ConfigRoot(root.clone());
    assert!(
        row.setup(
            &SetupRequest {
                scope: scope.clone(),
                executable: env.executable.clone(),
                environment: env.clone(),
                native_binary: None,
                options: Default::default()
            },
            &budget()
        )
        .is_err()
    );
    let removed = row
        .unsetup(
            &UnsetupRequest {
                scope,
                environment: env,
            },
            &budget(),
        )
        .unwrap();
    assert!(removed.residue.is_empty());
    assert_eq!(std::fs::read(extension).unwrap(), b"foreign");
    assert!(!root.join("skills").exists());
    std::fs::remove_dir_all(scratch).unwrap();
}
