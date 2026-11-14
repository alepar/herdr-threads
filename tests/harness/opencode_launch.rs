//! opencode launch policy (design D7): the flag table against the `opencode --help` capture, the
//! refusals, the `--prompt` startup input, the content-free first prompt (review M4) and the V2
//! handoff composition. Included from `src/harness/opencode/launch.rs`.
use super::*;
use crate::harness::adapter::{
    HarnessAdapter, LaunchPolicy, LaunchRequest, SetupEnvironment, StartupInputSpec,
};
use crate::notification::policy::WAKE_UP;
use crate::protocol::time::MonoInstant;
use std::path::PathBuf;

const HELP: &str =
    include_str!("../../docs/superpowers/runs/2026-11-14-opencode-harness/captures/cli-help.txt");
const POLICY: OpencodeLaunch = OpencodeLaunch;
struct NoProbe;
impl PaneShellProbe for NoProbe {}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Default::default(),
    }
}
fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}
fn validate(words: &[&str]) -> Result<(), ApiError> {
    POLICY.validate_native_argv(&argv(words))
}
fn spec() -> StartupInputSpec {
    StartupInputSpec {
        max_text_bytes: 4096,
    }
}

/// The TUI section of the capture (before `=== opencode run --help`).
fn tui_help() -> &'static str {
    HELP.split("=== opencode run").next().unwrap()
}
/// Every long and short flag spelling the TUI help lists under "Options:".
fn help_flags() -> Vec<String> {
    tui_help()
        .split("Options:")
        .nth(1)
        .unwrap()
        .lines()
        .flat_map(|line| line.split_whitespace().take(2))
        .filter(|word| word.starts_with('-'))
        .map(|word| word.trim_end_matches(',').to_owned())
        .collect()
}
/// The subcommand words the TUI help lists under "Commands:".
fn help_subcommands() -> Vec<String> {
    tui_help()
        .split("Commands:")
        .nth(1)
        .unwrap()
        .split("Positionals:")
        .next()
        .unwrap()
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|word| !word.starts_with('['))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_captured_flag_is_classified() {
    let listed = help_flags();
    assert!(
        listed.len() >= 20,
        "the capture lists the flags: {listed:?}"
    );
    // Flags the table neither accepts nor names: refused as uncaptured options.
    let unknown = [
        "-h",
        "--help",
        "-v",
        "--version",
        "--print-logs",
        "--log-level",
        "--mdns-domain",
        "--cors",
        "--no-replay",
        "--replay-limit",
    ];
    for name in &listed {
        let classified =
            accepted(name).is_some() || refused(name).is_some() || unknown.contains(&name.as_str());
        assert!(classified, "{name} is in the help capture but unclassified");
        if accepted(name).is_none() {
            assert!(validate(&[name]).is_err(), "accepted {name}");
        }
    }
    for name in [
        "-m",
        "--model",
        "--agent",
        "-c",
        "--continue",
        "-s",
        "--session",
        "--fork",
        "--prompt",
        "--pure",
        "--auto",
        "--port",
        "--hostname",
        "--mdns",
        "--mini",
    ] {
        assert!(
            listed.iter().any(|l| l == name),
            "{name} not in the capture"
        );
    }
}

#[test]
fn every_captured_subcommand_is_refused_as_a_positional() {
    let commands = help_subcommands();
    for word in ["run", "serve", "attach", "acp", "web", "upgrade", "session"] {
        assert!(
            commands.iter().any(|c| c == word),
            "{word} not in {commands:?}"
        );
    }
    for word in &commands {
        let error = validate(&[word]).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedHarness, "{word}");
        assert!(error.detail.contains("subcommands"), "{}", error.detail);
        // Also behind accepted flags, and a word no capture lists yet.
        assert!(validate(&["-m", "p/m", word]).is_err(), "{word}");
    }
    assert!(validate(&["some-future-subcommand"]).is_err());
    // The `[project]` positional too: the launch scope keeps the caller's directory.
    assert!(validate(&["/work/project"]).is_err());
}

#[test]
fn accepted_flags_in_space_and_equals_forms() {
    for (flag, value) in [
        ("--model", "openrouter/deepseek/deepseek-v4-pro"),
        ("--agent", "plan"),
        ("--session", "ses_abc123"),
        ("--prompt", "start here"),
    ] {
        validate(&[flag, value]).unwrap_or_else(|e| panic!("{flag} {value}: {e:?}"));
        validate(&[&format!("{flag}={value}")]).unwrap_or_else(|e| panic!("{flag}={value}: {e:?}"));
    }
    validate(&["-m", "p/m"]).unwrap();
    validate(&["-s", "ses_1"]).unwrap();
    validate(&["-c"]).unwrap();
    validate(&["--continue", "--fork"]).unwrap();
    validate(&["-s", "ses_1", "--fork"]).unwrap();
    validate(&[]).unwrap();
    validate(&["-m", "p/m", "--agent", "build", "-c", "--prompt=-dash text"]).unwrap();
}

#[test]
fn refused_flags_name_their_reason() {
    for (flag, reason) in [
        ("--pure", "plugin"),
        ("--pure=true", "plugin"),
        ("--auto", "auto-approves"),
        ("--yolo", "auto-approves"),
        ("--dangerously-skip-permissions", "auto-approves"),
        ("--port", "server"),
        ("--port=4096", "server"),
        ("--hostname", "server"),
        ("--mdns", "server"),
        ("--mini", "minimal interface"),
    ] {
        let error = validate(&[flag]).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedHarness, "{flag}");
        assert!(error.detail.contains(reason), "{flag}: {}", error.detail);
        assert!(!error.detail.contains('\n'), "one line: {}", error.detail);
        // Also after accepted flags.
        assert!(validate(&["-c", flag]).is_err(), "{flag}");
    }
}

#[test]
fn refused_forms() {
    let cases: &[&[&str]] = &[
        &["--unknown"],
        &["--unknown=1"],
        &["--no-pure"],
        &["-z"],
        &["-mp/m"],
        &["-m=p/m"],
        &["-cs"],
        &["--"],
        &["-m", "p/m", "--", "rest"],
        &["-"],
        &[""],
        // Duplicates, also across spellings.
        &["-m", "a", "--model", "b"],
        &["--prompt", "a", "--prompt=b"],
        &["-c", "--continue"],
        // A value flag without its value, or followed by another flag.
        &["--model"],
        &["--model", "--agent", "x"],
        &["--model="],
        &["--prompt", "-starts with a dash"],
        // Booleans take no value.
        &["-c=true"],
        &["--fork=false"],
        // --fork needs a session to fork; -c and -s select differently.
        &["--fork"],
        &["-c", "-s", "ses_1"],
        // Line breaks never reach the pane shell.
        &["--prompt", "two\nlines"],
    ];
    for case in cases {
        assert!(validate(case).is_err(), "accepted {case:?}");
    }
    assert_eq!(
        validate(&["-c", "-s", "ses_1"]).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn model_maps_to_dash_m_and_options_use_the_same_grammar() {
    assert_eq!(
        POLICY.apply_model(argv(&["-c"]), "p/m").unwrap(),
        ["-m", "p/m", "-c"]
    );
    // A model the caller already chose is a duplicate.
    let doubled = POLICY.apply_model(argv(&["--model", "a"]), "b").unwrap();
    assert!(POLICY.validate_native_argv(&doubled).is_err());

    use crate::cli::launch::LaunchRequest as CliRequest;
    let registry = crate::harness::registry::builtins();
    let harness = crate::harness::launch::bootstrap_v2_target(registry, "opencode").unwrap();
    let request = |argv: &[&str]| CliRequest {
        directory_context: None,
        target: crate::protocol::ids::HostTargetId::new("w9:p1"),
        harness,
        harness_binary: None,
        argv: argv.iter().map(|s| (*s).to_owned()).collect(),
        name: None,
        pane_label: None,
    };
    assert_eq!(
        POLICY.native_options_env(),
        Some("HERDR_THREADS_OPENCODE_OPTS")
    );
    let launch = request(&["-c"])
        .with_configured_options(Some("--agent 'my agent' -m p/m".into()))
        .unwrap();
    assert_eq!(launch.argv, ["--agent", "my agent", "-m", "p/m", "-c"]);
    POLICY.validate_native_argv(&launch.argv).unwrap();
    for options in ["--pure", "--auto", "serve", "--port 1"] {
        let launch = request(&[])
            .with_configured_options(Some(options.into()))
            .unwrap();
        assert!(
            POLICY.validate_native_argv(&launch.argv).is_err(),
            "accepted {options}"
        );
    }
}

#[test]
fn compose_argv_returns_the_caller_and_refuses_owned_arguments() {
    let caller = argv(&["-m", "p/m", "-c"]);
    assert_eq!(POLICY.compose_argv(caller.clone(), vec![]).unwrap(), caller);
    assert!(POLICY.compose_argv(caller, argv(&["--x"])).is_err());
    assert!(POLICY.compose_argv(argv(&["--pure"]), vec![]).is_err());
    // No first prompt here: a V2 plan freezes this composition.
    assert_eq!(
        POLICY.compose_argv(vec![], vec![]).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn startup_input_is_one_prompt_argument_and_conflicts_with_prompt() {
    let caller = argv(&["-m", "p/m"]);
    let template = POLICY
        .prepare_startup_input(&caller, &spec())
        .unwrap()
        .unwrap();
    let (composed, slot) = template.apply(&caller, "hello there").unwrap();
    assert_eq!(composed, ["-m", "p/m", "--prompt=hello there"]);
    assert_eq!(slot, 2);
    POLICY.validate_native_argv(&composed).unwrap();
    // yargs reads `--prompt -x` as a flag; the `=` form keeps dash-leading text whole.
    let (dashed, _) = template.apply(&caller, "-x and = signs").unwrap();
    assert_eq!(dashed[2], "--prompt=-x and = signs");
    POLICY.validate_native_argv(&dashed).unwrap();
    for occupied in [&["--prompt", "p"][..], &["--prompt=p"]] {
        let error = POLICY
            .prepare_startup_input(&argv(occupied), &spec())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict, "{occupied:?}");
    }
    assert!(
        POLICY
            .prepare_startup_input(&argv(&["--pure"]), &spec())
            .is_err()
    );
}

#[test]
fn a_v2_handoff_plan_composes_and_its_retained_argv_validates() {
    let registry = crate::harness::registry::builtins();
    assert!(crate::harness::launch::bootstrap_v1_target(registry, "opencode").is_none());
    let harness = crate::harness::launch::bootstrap_target(registry, "opencode")
        .expect("opencode is a bootstrap kind through V2");
    assert_eq!(harness.as_str(), "opencode");
    let caller = argv(&["--agent=build", "-s", "ses_1", "--fork"]);
    let composed =
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), caller.clone())
            .unwrap();
    assert_eq!(composed, caller);
    let template =
        crate::harness::launch::bootstrap_v2_startup_input(harness.occupant(), &caller).unwrap();
    let (retained, _) = template.apply(&caller, "resume the work").unwrap();
    assert_eq!(
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), retained.clone())
            .unwrap(),
        retained
    );
    assert!(
        crate::harness::launch::compose_bootstrap_v2_argv(harness.occupant(), argv(&["run"]))
            .is_err()
    );
}

// ------------------------------------------------------------------ scope and preparation

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "ht-opencode-launch-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("home")).unwrap();
        Self { root }
    }
    fn config_root(&self) -> PathBuf {
        self.root.join("home/.config/opencode")
    }
    fn request(&self, words: &[&str]) -> LaunchRequest {
        LaunchRequest {
            argv: argv(words),
            environment: SetupEnvironment {
                home: Some(self.root.join("home").into()),
                cwd: self.root.join("work"),
                ..Default::default()
            },
            native_binary: None,
        }
    }
    fn scope(&self, request: &LaunchRequest) -> LaunchScope {
        POLICY.resolve_scope(request, &NoProbe, &budget()).unwrap()
    }
    fn status(&self, hook: Option<crate::ports::ConfiguredHook>) -> LocalSetupStatus {
        LocalSetupStatus {
            scope: ResolvedSetupScope::ConfigRoot(self.config_root()),
            installed: hook.is_some(),
            enabled: hook.as_ref().map(|_| true),
            admitted: None,
            observed: None,
            configured_hook: hook,
            fingerprint: None,
            diagnostics: vec![],
            repairs: vec![],
            projection: serde_json::json!({}),
        }
    }
    fn prepare(&self, words: &[&str]) -> LaunchPreparation {
        let request = self.request(words);
        let scope = self.scope(&request);
        POLICY
            .prepare_launch(
                &request,
                &scope,
                &admitted(),
                &self.status(None),
                &NoProbe,
                &budget(),
            )
            .unwrap()
    }
}
fn admitted() -> AdmittedHandle {
    let registry = crate::harness::registry::builtins();
    registry
        .by_id(registry.agent("opencode").unwrap())
        .unwrap()
        .admit(
            &crate::harness::adapter::AdmissionRequest {
                installed: crate::harness::adapter::InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget(),
        )
        .unwrap_or_else(|e| panic!("admit: {e}"))
}

#[test]
fn resolve_scope_prefers_explicit_then_launcher_then_home() {
    let fx = Fixture::new();
    let mut request = fx.request(&["-c"]);
    let scope = fx.scope(&request);
    assert_eq!(
        scope.setup,
        ResolvedSetupScope::ConfigRoot(fx.config_root())
    );
    assert_eq!(scope.config_source, "home_default");
    assert_eq!(scope.working_directory, fx.root.join("work"));
    request
        .environment
        .config_roots
        .insert("opencode".into(), fx.root.join("xdg/opencode"));
    let scope = fx.scope(&request);
    assert_eq!(
        scope.setup,
        ResolvedSetupScope::ConfigRoot(fx.root.join("xdg/opencode"))
    );
    assert_eq!(scope.config_source, "launcher");
    request.environment.explicit_profile_directory = Some(fx.root.join("explicit"));
    let scope = fx.scope(&request);
    assert_eq!(
        scope.setup,
        ResolvedSetupScope::ConfigRoot(fx.root.join("explicit"))
    );
    assert_eq!(scope.config_source, "explicit_profile_directory");
    // The grammar applies before any scope, and no HOME is a missing root.
    assert!(
        POLICY
            .resolve_scope(&fx.request(&["--pure"]), &NoProbe, &budget())
            .is_err()
    );
    let mut homeless = fx.request(&[]);
    homeless.environment.home = None;
    assert_eq!(
        POLICY
            .resolve_scope(&homeless, &NoProbe, &budget())
            .unwrap_err()
            .code,
        ErrorCode::MissingHook
    );
}

#[test]
fn host_kinds_and_process_hint() {
    assert_eq!(POLICY.expected_host_kinds(), ["opencode"]);
    assert!(!POLICY.requires_process_hint());
    assert!(!POLICY.uses_prelaunch_observation());
}

// Review M4: opencode creates its session at the first prompt, so a launch with none would
// leave a seat that nothing can reach.
#[test]
fn a_launch_without_prompt_gets_the_content_free_first_prompt() {
    let fx = Fixture::new();
    for caller in [&[][..], &["-m", "p/m"], &["-c"], &["-s", "ses_1", "--fork"]] {
        let prepared = fx.prepare(caller);
        let mut expected = argv(caller);
        expected.push(format!("--prompt={WAKE_UP}"));
        assert_eq!(prepared.argv, expected, "{caller:?}");
        assert_eq!(prepared.report["opencode"]["first_prompt"], true);
        POLICY.validate_native_argv(&prepared.argv).unwrap();
        crate::ports::validate_native_argv(&prepared.argv).unwrap();
    }
    // A caller prompt or a handoff's startup input is kept as the only prompt.
    for caller in [
        &["--prompt", "do it"][..],
        &["-c", "--prompt=resume the work"],
    ] {
        let prepared = fx.prepare(caller);
        assert_eq!(prepared.argv, argv(caller));
        assert_eq!(prepared.report["opencode"]["first_prompt"], false);
    }
}

// Readiness is advisory: a missing plugin keeps the native launch and says how to repair it.
#[test]
fn a_missing_plugin_keeps_native_launch_and_the_setup_warning() {
    let fx = Fixture::new();
    let prepared = fx.prepare(&["--prompt", "p"]);
    assert!(prepared.hook.is_none());
    assert!(prepared.environment_overrides.is_empty());
    assert_eq!(prepared.working_directory, fx.root.join("work"));
    assert_eq!(
        prepared.report["opencode"]["warnings"],
        serde_json::json!(["opencode plugin unavailable; run herdr-threads setup opencode"])
    );
    // A scope that is not the status's is refused.
    let request = fx.request(&[]);
    let mut scope = fx.scope(&request);
    scope.setup = ResolvedSetupScope::ConfigRoot(fx.root.join("other"));
    assert!(
        POLICY
            .prepare_launch(
                &request,
                &scope,
                &admitted(),
                &fx.status(None),
                &NoProbe,
                &budget()
            )
            .is_err()
    );
}

#[test]
fn the_fingerprint_follows_the_plugin_bytes() {
    let fx = Fixture::new();
    let request = fx.request(&[]);
    let scope = fx.scope(&request);
    assert_eq!(
        POLICY
            .configuration_fingerprint(&request, &scope)
            .unwrap_err()
            .code,
        ErrorCode::MissingHook
    );
    let plugin = fx.config_root().join("plugins/herdr-threads.js");
    std::fs::create_dir_all(plugin.parent().unwrap()).unwrap();
    std::fs::write(&plugin, "export default {}\n").unwrap();
    let first = POLICY.configuration_fingerprint(&request, &scope).unwrap();
    assert_eq!(
        first,
        POLICY.configuration_fingerprint(&request, &scope).unwrap()
    );
    std::fs::write(&plugin, "export default { changed: true }\n").unwrap();
    assert_ne!(
        first,
        POLICY.configuration_fingerprint(&request, &scope).unwrap()
    );
}

#[test]
fn launch_notes_name_the_accepted_and_refused_flags() {
    let notes = crate::harness::opencode::OpencodeAdapter
        .text()
        .launch_notes;
    for needle in [
        "HERDR_THREADS_OPENCODE_OPTS",
        "-m/--model",
        "--prompt",
        "--pure",
        "--dangerously-skip-permissions",
        "--port/--hostname/--mdns",
        "--mini",
        "subcommand",
        WAKE_UP,
    ] {
        assert!(notes.contains(needle), "launch_notes lacks {needle}");
    }
}
