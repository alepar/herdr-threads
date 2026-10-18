//! `handoff --kind agy` through the built binary and the owned fake host of `handoff_topology_cli`:
//! the plugin installed by `setup agy` lets the V2 plan compose and start, and without it the
//! handoff still starts (readiness is advisory) and carries the setup hint.
use super::handoff_topology_cli::{BODY, Fixture, successful};

fn agy_args<'a>(_f: &'a Fixture, binary: &'a str) -> Vec<&'a str> {
    vec![
        "handoff",
        "--pane",
        "w4:p2",
        "--new-thread",
        "--topic",
        "Private smoke",
        "--goal",
        "Exact durable work",
        "--kind",
        "agy",
        "--harness-binary",
        binary,
        "--agent-arg=--model",
        "--agent-arg=gemini-3",
        "--",
        BODY,
    ]
}

#[test]
fn handoff_kind_agy_composes_through_bootstrap_v2_and_starts() {
    let f = Fixture::new();
    f.setup("agy");
    assert!(
        f.iso
            .home()
            .join(".gemini/config/plugins/herdr-threads")
            .is_dir(),
        "setup agy installs the owned plugin under the isolated HOME"
    );
    let binary = f.iso.path("agy");
    let binary = binary.to_str().unwrap();
    let out = successful(f.run(&agy_args(&f, binary), ""));
    assert_eq!(out["handoff"]["outcome"], "started");

    let starts = f.host.requests("agent.start");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["params"]["kind"], "agy");
    assert_eq!(starts[0]["params"]["pane_id"], "w4:p2");
    let argv: Vec<String> = serde_json::from_value(starts[0]["params"]["args"].clone()).unwrap();
    // Caller flags first, then the V2 startup input as the value of `--prompt-interactive`.
    assert_eq!(&argv[..3], ["--model", "gemini-3", "--prompt-interactive"]);
    assert_eq!(argv.len(), 4, "{argv:?}");
    let bootstrap = &argv[3];
    assert!(bootstrap.contains(&f.instance.to_string()));
    assert!(!bootstrap.contains(BODY), "the work body is not in argv");
    // The owned fake host runs the stand-in agy with this exact argv and asserts it echoed back.
    assert_eq!(f.host.state.lock().unwrap().helpers.len(), 1);

    f.assert_work(
        out["handoff"]["seat"].as_str().unwrap(),
        out["handoff"]["thread"].as_str().unwrap(),
    );
    assert_eq!(
        f.effect_counts(),
        (0, 1, 1, 1),
        "one start, one invitation, one message"
    );
}

/// Plugin readiness is advisory for launch (TRUST-POLICY "Integration readiness is
/// advisory for launch"; agy since 4c4988d9): without the plugin the handoff still
/// starts the selected native agy once, and says how to install the integration.
#[test]
fn handoff_kind_agy_without_the_plugin_starts_with_the_setup_hint() {
    let f = Fixture::new();
    let binary = f.iso.path("agy");
    let binary = binary.to_str().unwrap();
    let out = f.run(&agy_args(&f, binary), "");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let out = successful(out);
    assert_eq!(out["handoff"]["outcome"], "started");
    assert!(text.contains("herdr-threads setup agy"), "{text}");
    let starts = f.host.requests("agent.start");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["params"]["kind"], "agy");
    assert_eq!(
        f.effect_counts(),
        (0, 1, 1, 1),
        "one start, one invitation, one message"
    );
}

#[test]
fn handoff_agy_explicit_profile_is_unsupported_without_changing_home_or_cwd() {
    use herdr_threads::harness::adapter::{LaunchRequest, SetupEnvironment};
    use herdr_threads::harness::launch::PaneShellProbe;
    use herdr_threads::protocol::{
        results::ErrorCode,
        time::{CallBudget, MonoInstant},
    };
    struct NoShell;
    impl PaneShellProbe for NoShell {}
    let registry = herdr_threads::harness::registry::builtins();
    let policy = registry
        .by_id(registry.agent("agy").unwrap())
        .unwrap()
        .launch_policy()
        .unwrap();
    let iso = herdr_threads::test_support::isolation::TestIsolation::new("agy-profile-refusal");
    let cwd = iso.path("work");
    std::fs::create_dir_all(&cwd).unwrap();
    let environment = SetupEnvironment {
        home: Some(iso.home().into_os_string()),
        cwd: cwd.clone(),
        ..Default::default()
    };
    let home = environment.home.clone();
    let error = policy
        .explicit_directory_env(&iso.path("profile"))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(error.detail, "explicit profile directory unavailable");
    let budget = CallBudget {
        deadline: MonoInstant(environment.clock.monotonic_now().0 + 10_000),
        cancellation: Default::default(),
    };
    let request = LaunchRequest {
        argv: vec![],
        environment,
        native_binary: None,
    };
    let scope = policy.resolve_scope(&request, &NoShell, &budget).unwrap();
    assert_eq!(scope.working_directory, cwd);
    assert_eq!(request.environment.home, home);
    assert!(
        request.environment.declared.is_empty(),
        "unsupported selection must not manufacture an environment override"
    );
}
