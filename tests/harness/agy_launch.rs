//! agy launch policy (L5): the flag table against the `agy --help` 1.3.2 capture, the plugin gate
//! in a temp HOME and state dir, the startup-input template and the V2 handoff composition.
//! Included from `src/harness/agy/launch.rs`.
use super::*;
use crate::harness::{
    adapter::{
        HarnessAdapter, LaunchPolicy, LaunchRequest, SetupEnvironment, SetupRequest, SetupStatus,
        StartupInputSpec, StatusRequest,
    },
    agy::AgyAdapter,
    launch::PaneShellProbe,
};
use crate::protocol::time::MonoInstant;
use std::path::PathBuf;

const HELP: &str = include_str!("../fixtures/agy/help/agy-1.3.2.txt");
const POLICY: AgyLaunch = AgyLaunch;
struct NoProbe;
impl PaneShellProbe for NoProbe {}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Default::default(),
    }
}
fn agy_admission() -> (
    &'static crate::harness::registry::Registration,
    crate::harness::registry::AdmittedHandle,
) {
    let registry = crate::harness::registry::builtins();
    let registration = registry.by_id(registry.agent("agy").unwrap()).unwrap();
    let admitted = registration
        .admit(
            &crate::harness::adapter::AdmissionRequest {
                installed: crate::harness::adapter::InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget(),
        )
        .unwrap_or_else(|e| panic!("admit: {e}"));
    (registration, admitted)
}
fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}
fn validate(words: &[&str]) -> Result<(), ApiError> {
    POLICY.validate_native_argv(&argv(words))
}

/// The flag names the capture lists under "Usage of agy:", before the subcommands.
fn help_flags() -> Vec<String> {
    HELP.lines()
        .take_while(|line| !line.starts_with("Available subcommands"))
        .filter(|line| line.starts_with("  -"))
        .map(|line| line.split_whitespace().next().unwrap().to_owned())
        .collect()
}

#[test]
fn the_flag_table_tracks_the_help_capture_in_both_directions() {
    let listed = help_flags();
    assert!(
        listed.len() >= 20,
        "the capture lists the flags: {listed:?}"
    );
    // Every flag the capture lists is classified, accepted or refused.
    for name in &listed {
        assert!(
            accepted(name).is_some() || REFUSED.contains(&name.as_str()),
            "{name} is in the help capture but neither accepted nor refused"
        );
    }
    // Every refused flag is a real flag of the capture, and so is every accepted spelling.
    for name in REFUSED {
        assert!(
            listed.iter().any(|l| l == name),
            "{name} not in the capture"
        );
    }
    for name in [
        "--add-dir",
        "--agent",
        "--conversation",
        "--effort",
        "--mode",
        "--model",
        "--project",
        "--log-file",
        "-i",
        "--prompt-interactive",
        "-c",
        "--continue",
        "--new-project",
    ] {
        assert!(accepted(name).is_some(), "{name} is not accepted");
        assert!(
            listed.iter().any(|l| l == name),
            "{name} not in the capture"
        );
    }
}

#[test]
fn accepted_flags_in_space_and_equals_forms() {
    for (flag, value) in [
        ("--add-dir", "/work/a"),
        ("--agent", "reviewer"),
        ("--conversation", "c-123"),
        ("--effort", "xhigh"),
        ("--mode", "accept-edits"),
        ("--model", "gemini-3"),
        ("--project", "my project"),
        ("--log-file", "/tmp/agy.log"),
        ("--prompt-interactive", "start here"),
    ] {
        validate(&[flag, value]).unwrap_or_else(|e| panic!("{flag} {value}: {e:?}"));
        validate(&[&format!("{flag}={value}")]).unwrap_or_else(|e| panic!("{flag}={value}: {e:?}"));
    }
    validate(&["-i", "start here"]).unwrap();
    validate(&["-i=start here"]).unwrap();
    validate(&["-c"]).unwrap();
    validate(&["--continue"]).unwrap();
    validate(&["--new-project"]).unwrap();
    validate(&[]).unwrap();
    validate(&[
        "--model",
        "m",
        "--effort=high",
        "--mode",
        "plan",
        "-c",
        "--new-project",
    ])
    .unwrap();
}

#[test]
fn add_dir_repeats_and_other_flags_do_not() {
    validate(&["--add-dir", "/a", "--add-dir=/b", "--add-dir", "/c"]).unwrap();
    for case in [
        &["--model", "a", "--model", "b"][..],
        &["--model=a", "--model", "b"],
        &["-i", "x", "--prompt-interactive", "y"],
        &["-c", "--continue"],
        &["--new-project", "--new-project"],
    ] {
        assert!(validate(case).is_err(), "accepted a repeat: {case:?}");
    }
}

#[test]
fn effort_and_mode_are_enums() {
    for value in ["low", "medium", "high", "xhigh", "max"] {
        validate(&["--effort", value]).unwrap();
    }
    for value in ["accept-edits", "plan"] {
        validate(&["--mode", value]).unwrap();
    }
    for case in [
        &["--effort", "extreme"][..],
        &["--effort=High"],
        &["--effort="],
        &["--mode", "yolo"],
        &["--mode=accept_edits"],
    ] {
        let error = validate(case).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{case:?}");
    }
}

#[test]
fn refused_forms() {
    let cases: &[&[&str]] = &[
        &["-p"],
        &["--print"],
        &["--prompt", "hi"],
        &["--prompt=hi"],
        &["-p", "hi"],
        &["--dangerously-skip-permissions"],
        &["--sandbox"],
        &["--remote-control"],
        &["--input-format", "text"],
        &["--output-format=json"],
        &["--json-schema", "{}"],
        &["--print-timeout", "5s"],
        &["--disable-slash-commands"],
        &["-model", "x"],
        &["-model=x"],
        &["-ci"],
        &["-add-dir", "/a"],
        &["--unknown"],
        &["--unknown=1"],
        &["-z"],
        &["positional"],
        &["plugin", "list"],
        &["update"],
        &["--"],
        &["--model", "x", "--", "rest"],
        &["-"],
        &["-c", "--conversation", "id"],
        &["--continue", "--conversation=id"],
        &["--conversation", "id", "-c"],
        // A value flag without its value, or followed by another flag.
        &["--model"],
        &["--model", "--effort", "low"],
        &["--model="],
        &["--add-dir", ""],
        &[""],
        // Booleans take no value.
        &["-c=true"],
        &["--new-project=false"],
        // Line breaks never reach the pane shell.
        &["-i", "two\nlines"],
    ];
    for case in cases {
        assert!(validate(case).is_err(), "accepted {case:?}");
    }
    assert_eq!(
        validate(&["-c", "--conversation", "id"]).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn the_prompt_text_may_start_with_a_dash_after_its_flag() {
    // The V2 handoff appends `--prompt-interactive <text>`; Go takes the next argument whole.
    validate(&["--prompt-interactive", "-starts with a dash"]).unwrap();
    // Other value flags stay unambiguous.
    assert!(validate(&["--model", "-starts"]).is_err());
}

#[test]
fn configured_options_use_the_same_grammar() {
    use crate::cli::launch::LaunchRequest as CliRequest;
    let registry = crate::harness::registry::builtins();
    let harness = crate::harness::launch::bootstrap_v2_target(registry, "agy").unwrap();
    let request = |argv: &[&str]| CliRequest {
        target: crate::protocol::ids::HostTargetId::new("w9:p1"),
        harness,
        harness_binary: None,
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        name: None,
        pane_label: None,
    };
    assert_eq!(POLICY.native_options_env(), Some("HERDR_THREADS_AGY_OPTS"));
    let launch = request(&["--effort", "low"])
        .with_configured_options(Some("--model 'a b' --add-dir /x".into()))
        .unwrap();
    assert_eq!(
        launch.argv,
        ["--model", "a b", "--add-dir", "/x", "--effort", "low"]
    );
    POLICY.validate_native_argv(&launch.argv).unwrap();
    // A refused flag in the environment is refused after composition.
    for options in [
        "--print",
        "-p hi",
        "--sandbox",
        "--mode yolo",
        "stray",
        "--model",
    ] {
        let launch = request(&[])
            .with_configured_options(Some(options.into()))
            .unwrap();
        assert!(
            POLICY.validate_native_argv(&launch.argv).is_err(),
            "accepted {options}"
        );
    }
    // A duplicate between the environment and the caller is refused.
    let launch = request(&["--model", "b"])
        .with_configured_options(Some("--model a".into()))
        .unwrap();
    assert!(POLICY.validate_native_argv(&launch.argv).is_err());
}

#[test]
fn compose_argv_returns_the_caller_and_refuses_owned_arguments() {
    let caller = argv(&["--model", "m", "-c"]);
    assert_eq!(POLICY.compose_argv(caller.clone(), vec![]).unwrap(), caller);
    assert!(POLICY.compose_argv(caller.clone(), argv(&["--x"])).is_err());
    assert!(POLICY.compose_argv(argv(&["--print"]), vec![]).is_err());
}

#[test]
fn startup_input_appends_prompt_interactive_and_conflicts_with_i() {
    let spec = StartupInputSpec {
        max_text_bytes: 4096,
    };
    let caller = argv(&["--model", "m"]);
    let template = POLICY
        .prepare_startup_input(&caller, &spec)
        .unwrap()
        .unwrap();
    let (composed, slot) = template.apply(&caller, "hello there").unwrap();
    assert_eq!(
        composed,
        ["--model", "m", "--prompt-interactive", "hello there"]
    );
    assert_eq!(slot, 3);
    // The retained argv validates under the same grammar, even for dash-leading text.
    POLICY.validate_native_argv(&composed).unwrap();
    let (dashed, _) = template.apply(&caller, "-x").unwrap();
    POLICY.validate_native_argv(&dashed).unwrap();
    // Occupied by either spelling.
    for occupied in [
        &["-i", "p"][..],
        &["--prompt-interactive=p"],
        &["--prompt-interactive", "p"],
    ] {
        let error = POLICY
            .prepare_startup_input(&argv(occupied), &spec)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict, "{occupied:?}");
    }
    // A caller the grammar refuses has no template.
    assert!(
        POLICY
            .prepare_startup_input(&argv(&["--print"]), &spec)
            .is_err()
    );
}

#[test]
fn a_v2_handoff_plan_composes_and_its_retained_argv_validates() {
    let registry = crate::harness::registry::builtins();
    assert!(crate::harness::launch::bootstrap_v1_target(registry, "agy").is_none());
    let harness = crate::harness::launch::bootstrap_target(registry, "agy")
        .expect("agy is a bootstrap kind through V2");
    assert_eq!(harness.as_str(), "agy");
    let caller = argv(&["--effort=high", "--add-dir", "/w"]);
    let composed =
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), caller.clone())
            .unwrap();
    assert_eq!(composed, caller);
    let template = POLICY
        .prepare_startup_input(
            &caller,
            &StartupInputSpec {
                max_text_bytes: 16384,
            },
        )
        .unwrap()
        .unwrap();
    let (retained, _) = template.apply(&caller, "resume the work").unwrap();
    assert_eq!(
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), retained.clone())
            .unwrap(),
        retained
    );
    assert!(
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), argv(&["--print"]))
            .is_err()
    );
}

// ------------------------------------------------------------------ scope and plugin gate

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    exe: PathBuf,
    roots: AgyRoots,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ht-agy-launch-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let state = root.join("state");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let roots = AgyRoots::from_home(&home);
        Self {
            exe: root.join("bin/herdr-threads"),
            root,
            home,
            state,
            roots,
        }
    }
    fn env(&self) -> SetupEnvironment {
        SetupEnvironment {
            home: Some(self.home.clone().into()),
            executable: self.exe.clone(),
            state_dir: Some(self.state.clone()),
            host_endpoint: Some(self.root.join("host.sock")),
            cwd: self.root.join("work"),
            config_roots: [("agy".to_owned(), self.roots.config_root.clone())].into(),
            ..Default::default()
        }
    }
    fn scope(&self) -> ResolvedSetupScope {
        ResolvedSetupScope::ConfigRoot(self.roots.config_root.clone())
    }
    fn setup(&self) {
        self.setup_with_options(
            [(crate::cli::setup::WITH_PERMISSIONS.to_owned(), true)]
                .into_iter()
                .collect(),
        );
    }
    fn setup_with_options(&self, options: crate::harness::adapter::SetupOptions) {
        AgyAdapter
            .setup(
                &SetupRequest {
                    scope: self.scope(),
                    executable: self.exe.clone(),
                    environment: self.env(),
                    native_binary: None,
                    options: options.clone(),
                },
                &budget(),
            )
            .unwrap();
    }
    fn request(&self, words: &[&str]) -> LaunchRequest {
        LaunchRequest {
            argv: argv(words),
            environment: self.env(),
            native_binary: None,
        }
    }
    fn status(&self) -> LocalSetupStatus {
        let request = StatusRequest {
            scope: self.scope(),
            environment: self.env(),
            native_binary: None,
        };
        match AgyAdapter.status(&request, &budget()) {
            SetupStatus::Detailed(status) => *status,
            _ => panic!("agy status is detailed"),
        }
    }
    fn scope_for(&self, request: &LaunchRequest) -> LaunchScope {
        POLICY.resolve_scope(request, &NoProbe, &budget()).unwrap()
    }
    fn disable(&self) {
        let path = &self.roots.plugins_config;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, r#"{"plugins":{"herdr-threads":{"enabled":false}}}"#).unwrap();
    }
    /// Prepare through the registry, as `launch` does (it checks the scope and the owned hook first).
    fn prepare(
        &self,
        words: &[&str],
    ) -> Result<crate::harness::adapter::LaunchPreparation, ApiError> {
        let (registration, admitted) = agy_admission();
        let request = self.request(words);
        let scope = self.scope_for(&request);
        registration.prepare_launch(
            &request,
            &scope,
            &admitted,
            &self.status(),
            &NoProbe,
            &budget(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn resolve_scope_is_the_config_root_with_the_home_default_source() {
    let fx = Fixture::new();
    let request = fx.request(&["--model", "m"]);
    let scope = POLICY.resolve_scope(&request, &NoProbe, &budget()).unwrap();
    assert_eq!(scope.setup, fx.scope());
    assert_eq!(scope.working_directory, fx.root.join("work"));
    assert_eq!(scope.config_source, "home_default");
    // Without a registered root the HOME default is used; with neither it is a refusal.
    let mut request = fx.request(&[]);
    request.environment.config_roots.clear();
    assert_eq!(
        POLICY
            .resolve_scope(&request, &NoProbe, &budget())
            .unwrap()
            .setup,
        fx.scope()
    );
    request.environment.home = None;
    assert!(POLICY.resolve_scope(&request, &NoProbe, &budget()).is_err());
    // A refused argv has no scope.
    assert!(
        POLICY
            .resolve_scope(&fx.request(&["--print"]), &NoProbe, &budget())
            .is_err()
    );
}

#[test]
fn host_kinds_and_process_hint() {
    assert_eq!(POLICY.expected_host_kinds(), ["agy"]);
    assert!(!POLICY.requires_process_hint());
    assert!(!POLICY.uses_prelaunch_observation());
}

#[test]
fn a_missing_plugin_keeps_native_launch_and_the_setup_diagnostic() {
    let fx = Fixture::new();
    let prepared = fx.prepare(&["--model", "m"]).unwrap();
    assert_eq!(prepared.argv, argv(&["--model", "m"]));
    assert!(prepared.hook.is_none());
    // Also exercise the policy directly with the observed missing status.
    let request = fx.request(&[]);
    let scope = fx.scope_for(&request);
    let (_, admitted) = agy_admission();
    let prepared = POLICY
        .prepare_launch(
            &request,
            &scope,
            &admitted,
            &fx.status(),
            &NoProbe,
            &budget(),
        )
        .unwrap();
    assert!(prepared.argv.is_empty());
    assert!(prepared.hook.is_none());
    assert!(
        prepared.report["agy"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|text| text.contains("run herdr-threads setup agy"))),
        "{:?}",
        prepared.report["agy"]["warnings"]
    );
}

#[test]
fn a_disabled_plugin_keeps_native_launch_and_the_enable_diagnostic() {
    let fx = Fixture::new();
    fx.setup();
    fx.disable();
    let status = fx.status();
    assert_eq!(status.enabled, Some(false));
    let prepared = fx.prepare(&[]).unwrap();
    assert!(prepared.argv.is_empty());
    assert!(prepared.hook.is_none());
    assert!(
        prepared.report["agy"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|text| text.contains("enable the plugin"))),
        "{:?}",
        prepared.report["agy"]["warnings"]
    );
    let request = fx.request(&[]);
    let scope = fx.scope_for(&request);
    let (_, admitted) = agy_admission();
    let prepared = POLICY
        .prepare_launch(&request, &scope, &admitted, &status, &NoProbe, &budget())
        .unwrap();
    assert!(prepared.argv.is_empty());
    assert!(prepared.hook.is_none());
    assert!(
        prepared.report["agy"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .is_some_and(|text| text.contains("enable the plugin"))),
        "{:?}",
        prepared.report["agy"]["warnings"]
    );
}

#[test]
fn an_installed_current_enabled_plugin_prepares_the_validated_caller_argv() {
    let fx = Fixture::new();
    fx.setup();
    let words = ["--model", "m", "--add-dir=/a", "-c", "--effort", "max"];
    let prepared = fx.prepare(&words).unwrap();
    assert_eq!(prepared.argv, argv(&words));
    let status = fx.status();
    assert_eq!(prepared.hook, status.configured_hook.clone());
    assert_eq!(prepared.hook.as_ref().unwrap().scope, "user");
    assert!(
        prepared
            .hook
            .as_ref()
            .unwrap()
            .path
            .ends_with("plugins/herdr-threads/hooks.json")
    );
    assert_eq!(prepared.working_directory, fx.root.join("work"));
    assert!(prepared.environment_overrides.is_empty());
    assert_eq!(prepared.report["agy"]["allow_entry"]["state"], "owned");
    assert_eq!(prepared.report["agy"]["warnings"], serde_json::json!([]));
    // A refused argv is refused at preparation too.
    let request = fx.request(&["--print"]);
    let (registration, admitted) = agy_admission();
    let scope = LaunchScope {
        setup: fx.scope(),
        working_directory: fx.root.clone(),
        config_source: "home_default",
    };
    assert!(
        registration
            .prepare_launch(&request, &scope, &admitted, &status, &NoProbe, &budget())
            .is_err()
    );
}

#[test]
fn a_not_granted_allow_entry_is_no_warning() {
    let fx = Fixture::new();
    fx.setup_with_options(Default::default());
    let prepared = fx.prepare(&["-c"]).unwrap();
    assert_eq!(
        prepared.report["agy"]["allow_entry"]["state"],
        "not_granted"
    );
    assert!(
        prepared.report["agy"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_missing_allow_entry_is_a_warning_not_a_refusal() {
    let fx = Fixture::new();
    fx.setup();
    // agy rewrote its settings without the owned member.
    std::fs::write(&fx.roots.settings, "{\"permissions\":{\"allow\":[]}}\n").unwrap();
    let prepared = fx.prepare(&["-c"]).unwrap();
    assert_eq!(prepared.argv, argv(&["-c"]));
    assert_eq!(prepared.report["agy"]["allow_entry"]["state"], "missing");
    let warnings = prepared.report["agy"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .contains("command(herdr-threads)"),
        "{warnings:?}"
    );
}

#[test]
fn a_plugin_edited_after_the_status_is_a_conflict() {
    let fx = Fixture::new();
    fx.setup();
    let status = fx.status();
    let request = fx.request(&[]);
    let scope = fx.scope_for(&request);
    let (_, admitted) = agy_admission();
    std::fs::write(fx.roots.plugin_dir.join("hooks.json"), "{}\n").unwrap();
    let error = POLICY
        .prepare_launch(&request, &scope, &admitted, &status, &NoProbe, &budget())
        .err()
        .expect("refused");
    assert_eq!(error.code, ErrorCode::Conflict);
}

#[test]
fn the_fingerprint_follows_the_hooks_digest_and_the_enabled_bit() {
    let fx = Fixture::new();
    fx.setup();
    let request = fx.request(&["--model", "m"]);
    let scope = fx.scope_for(&request);
    let first = POLICY.configuration_fingerprint(&request, &scope).unwrap();
    assert_eq!(
        first,
        POLICY.configuration_fingerprint(&request, &scope).unwrap()
    );
    // The caller argv is not part of the configuration.
    let other = fx.request(&["-c"]);
    assert_eq!(
        first,
        POLICY.configuration_fingerprint(&other, &scope).unwrap()
    );
    // The enabled bit.
    fx.disable();
    let disabled = POLICY.configuration_fingerprint(&request, &scope).unwrap();
    assert_ne!(first, disabled);
    std::fs::remove_file(&fx.roots.plugins_config).unwrap();
    assert_eq!(
        first,
        POLICY.configuration_fingerprint(&request, &scope).unwrap()
    );
    // The hooks.json digest: a moved executable re-renders the hooks.
    let mut moved = fx.request(&[]);
    moved.environment.executable = fx.root.join("elsewhere/herdr-threads");
    AgyAdapter
        .setup(
            &SetupRequest {
                scope: fx.scope(),
                executable: moved.environment.executable.clone(),
                environment: moved.environment.clone(),
                native_binary: None,
                options: Default::default(),
            },
            &budget(),
        )
        .unwrap();
    let rehomed = POLICY.configuration_fingerprint(&moved, &scope).unwrap();
    assert_ne!(first, rehomed);
    // No plugin, no fingerprint.
    std::fs::remove_dir_all(&fx.roots.plugin_dir).unwrap();
    let error = POLICY
        .configuration_fingerprint(&request, &scope)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::MissingHook);
}

#[test]
fn launch_notes_name_the_accepted_and_refused_flags() {
    let notes = crate::harness::agy::AgyAdapter.text().launch_notes;
    for needle in [
        "HERDR_THREADS_AGY_OPTS",
        "--prompt-interactive",
        "--conversation",
        "-c/--continue",
        "--dangerously-skip-permissions",
        "--sandbox",
        "--remote-control",
    ] {
        assert!(notes.contains(needle), "launch_notes lacks {needle}");
    }
}

// Kills: treating plugin readiness as authority to refuse a safe native argv.
#[test]
fn advisory_plugin_readiness_keeps_native_launch_available_without_fabricated_hooks() {
    for condition in ["missing", "stale", "disabled", "unknown"] {
        let fx = Fixture::new();
        if condition != "missing" {
            fx.setup();
        }
        match condition {
            "stale" => std::fs::write(fx.roots.plugin_dir.join("hooks.json"), "{}\n").unwrap(),
            "disabled" => fx.disable(),
            "unknown" => std::fs::write(&fx.roots.plugins_config, "invalid json").unwrap(),
            _ => {}
        }
        let prepared = fx
            .prepare(&["--model", "m"])
            .unwrap_or_else(|e| panic!("{condition}: {e:?}"));
        assert_eq!(prepared.argv, argv(&["--model", "m"]));
        assert!(
            prepared.hook.is_none(),
            "{condition}: no owned hook is used"
        );
        let unsafe_request = fx.request(&["--print"]);
        assert!(
            POLICY
                .resolve_scope(&unsafe_request, &NoProbe, &budget())
                .is_err(),
            "native grammar still applies before preparation"
        );
    }
}
