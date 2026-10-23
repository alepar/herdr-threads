//! The adapter hook seam (core-sweep D1/D6): registry validation, the neutral defaults, the
//! Claude/Codex/Hermes values (pinned against the core text they feed) and the
//! allow-member-only settings engine.
use crate::cli::setup::{SetupEnv, foreground};
use crate::harness::{
    adapter::*,
    claude, codex,
    context::Harness,
    contract,
    registry::{Registration, Registry, RegistryError, builtins},
    setup::{
        AllowRuleOwnership, InstallPhase, NativeObservation, OwnedAllowSpec, SettingsKind,
        SettingsKindSpec, SetupError, inspect_user_settings, install_user_settings,
        read_settings_manifest, remove_user_settings,
    },
};
use crate::protocol::time::CallBudget;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- registry validation

/// A registry-validation probe: only the hooks the registry reads are real.
struct Probe {
    metadata: &'static AdapterMetadata,
    text: &'static AdapterText,
    kind: Option<&'static SettingsKindSpec>,
}
impl HarnessAdapter for Probe {
    type Admission = ();
    fn metadata(&self) -> &'static AdapterMetadata {
        self.metadata
    }
    fn text(&self) -> &'static AdapterText {
        self.text
    }
    fn settings_kind(&self) -> Option<&'static SettingsKindSpec> {
        self.kind
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        &[]
    }
    fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
        unreachable!()
    }
    fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
        unreachable!()
    }
    fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
        unreachable!()
    }
    fn classify(&self, _: &HookInput) -> ContractObservation {
        unreachable!()
    }
    fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
        unreachable!()
    }
    fn encode(
        &self,
        _: &(),
        _: &DecodedEvent,
        _: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        unreachable!()
    }
    fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
        unreachable!()
    }
    fn setup(&self, _: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
        unreachable!()
    }
    fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
        unreachable!()
    }
    fn unsetup(&self, _: &UnsetupRequest, _: &CallBudget) -> Result<RemovalOutcome, SetupFailure> {
        unreachable!()
    }
}

fn probe(
    id: &'static str,
    markers: Vec<AgentEnvMarker>,
    text: AdapterText,
    kind: Option<&'static SettingsKindSpec>,
) -> Registration {
    let metadata = Box::leak(Box::new(AdapterMetadata {
        id,
        display_label: id,
        context_spelling: Box::leak(format!("Ctx{id}").into_boxed_str()),
        context_aliases: &[],
        executable: ExecutableLookup::Unsupported,
        host_kinds: &[],
        setup_scopes: &[SetupScopeKind::ConfigRoot],
        budget: EventBudgetPolicy {
            lifecycle_ms: 1000,
            observer_ms: 1000,
        },
        runtime_sources: &[],
        agent_env_markers: Box::leak(markers.into_boxed_slice()),
    }));
    let adapter = Box::leak(Box::new(Probe {
        metadata,
        text: Box::leak(Box::new(text)),
        kind,
    }));
    Registration::new(adapter)
}

fn validate(registrations: Vec<Registration>) -> Result<Registry, RegistryError> {
    Registry::new(Box::leak(registrations.into_boxed_slice()))
}

fn marker(name: &'static str, person_guard: bool) -> AgentEnvMarker {
    AgentEnvMarker { name, person_guard }
}

#[test]
fn adapter_seam_registry_rejects_ill_formed_env_markers() {
    let sixty_five: &'static str = Box::leak("A".repeat(65).into_boxed_str());
    let sixty_four: &'static str = Box::leak("A".repeat(64).into_boxed_str());
    for bad in [
        "bad-name",
        "",
        sixty_five,
        "lower",
        "1LEADING",
        "HAS SPACE",
        "_X",
    ] {
        let result = validate(vec![probe(
            "probe",
            vec![marker(bad, false)],
            AdapterText::NONE,
            None,
        )]);
        assert_eq!(
            result.err(),
            Some(RegistryError::InvalidEnvMarker(bad.into())),
            "{bad:?}"
        );
    }
    // The 64-byte boundary and the digit/underscore tail are well formed.
    validate(vec![probe(
        "probe",
        vec![marker(sixty_four, false), marker("A_1", true)],
        AdapterText::NONE,
        None,
    )])
    .expect("well-formed markers register");
}

#[test]
fn adapter_seam_registry_rejects_duplicate_env_markers() {
    let across = validate(vec![
        probe(
            "one",
            vec![marker("SHARED_MARK", false)],
            AdapterText::NONE,
            None,
        ),
        probe(
            "two",
            vec![marker("SHARED_MARK", true)],
            AdapterText::NONE,
            None,
        ),
    ]);
    assert_eq!(
        across.err(),
        Some(RegistryError::DuplicateEnvMarker("SHARED_MARK".into()))
    );
    let within = validate(vec![probe(
        "one",
        vec![marker("TWICE", false), marker("TWICE", false)],
        AdapterText::NONE,
        None,
    )]);
    assert_eq!(
        within.err(),
        Some(RegistryError::DuplicateEnvMarker("TWICE".into()))
    );
}

fn no_groups(_: &str) -> Vec<(&'static str, Value)> {
    Vec::new()
}

static MISMATCHED: SettingsKindSpec = SettingsKindSpec {
    harness: "someone_else",
    file_name: "settings.json",
    hook_groups: no_groups,
    owned_allow: None,
    adoption_note: "",
    legacy_upgrade_warning: "",
    permission_rules: crate::harness::setup::no_permission_rules,
};
static MATCHING: SettingsKindSpec = SettingsKindSpec {
    harness: "probe",
    file_name: "settings.json",
    hook_groups: no_groups,
    owned_allow: None,
    adoption_note: "",
    legacy_upgrade_warning: "",
    permission_rules: crate::harness::setup::no_permission_rules,
};

#[test]
fn adapter_seam_registry_refuses_a_settings_kind_of_another_harness() {
    let refused = validate(vec![probe(
        "probe",
        vec![],
        AdapterText::NONE,
        Some(&MISMATCHED),
    )]);
    assert_eq!(
        refused.err(),
        Some(RegistryError::InvalidSettingsKind("probe".into()))
    );
    let registry = validate(vec![probe(
        "probe",
        vec![],
        AdapterText::NONE,
        Some(&MATCHING),
    )])
    .expect("a kind of the declaring adapter registers");
    assert_eq!(
        registry.registrations()[0]
            .settings_kind()
            .map(|k| k.harness),
        Some("probe")
    );
}

#[test]
fn adapter_seam_registry_bounds_adapter_text() {
    type Field = fn(&'static str) -> AdapterText;
    let fields: [Field; 7] = [
        |s| AdapterText {
            setup_scope: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            setup_foreground: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            setup_notes: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            launch_notes: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            config_root_hint: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            trust_reminder_label: s,
            ..AdapterText::NONE
        },
        |s| AdapterText {
            version_history_note: s,
            ..AdapterText::NONE
        },
    ];
    let at_limit: &'static str = Box::leak("x".repeat(4096).into_boxed_str());
    let over: &'static str = Box::leak("x".repeat(4097).into_boxed_str());
    for field in fields {
        let ok = |text: &'static str| validate(vec![probe("probe", vec![], field(text), None)]);
        ok(at_limit).expect("4096 bytes is allowed");
        ok("two\nlines\n").expect("newline is allowed");
        for (bad, why) in [
            (over, "4097 bytes"),
            ("tab\there", "tab"),
            ("carriage\rreturn", "carriage return"),
            ("bell\u{7}", "bell"),
        ] {
            assert_eq!(
                ok(bad).err(),
                Some(RegistryError::InvalidAdapterText("probe".into())),
                "{why}"
            );
        }
    }
}

// ---------------------------------------------------------------- registry helpers

fn names<'a>(markers: impl Iterator<Item = &'a AgentEnvMarker>) -> Vec<&'static str> {
    markers.map(|m| m.name).collect()
}

#[test]
fn adapter_seam_registry_lists_markers_in_registry_order_and_splits_guards() {
    let registry = builtins();
    assert_eq!(
        names(registry.env_markers()),
        [
            "CLAUDECODE",
            "CODEX_THREAD_ID",
            "CODEX_SANDBOX",
            "CODEX_SANDBOX_NETWORK_DISABLED",
            "ANTIGRAVITY_AGENT"
        ]
    );
    let guards: Vec<_> = names(registry.person_guard_markers());
    assert_eq!(
        guards,
        [
            "CLAUDECODE",
            "CODEX_SANDBOX",
            "CODEX_SANDBOX_NETWORK_DISABLED",
            "ANTIGRAVITY_AGENT"
        ]
    );
    // CODEX_THREAD_ID is output evidence only; widening A4 to it would change behaviour.
    assert!(!guards.contains(&"CODEX_THREAD_ID"));
}

// ---------------------------------------------------------------- values

fn registration(id: &str) -> &'static Registration {
    let registry = builtins();
    registry
        .by_id(registry.agent(id).expect("builtin"))
        .expect("registration")
}

/// Every non-empty fragment reaches the help or source text it feeds, verbatim.
#[test]
fn adapter_seam_text_fragments_reach_the_assembled_help() {
    let registry = builtins();
    let launch_help = crate::cli::help::launch_help_in(registry);
    let setup_help = crate::cli::help::setup_help_in(registry);
    let setup_source = include_str!("../../src/cli/setup.rs");
    for id in ["claude", "codex", "hermes"] {
        let text = registration(id).text();
        for (name, fragment, haystack) in [
            ("setup_scope", text.setup_scope, setup_help.as_str()),
            (
                "setup_foreground",
                text.setup_foreground,
                setup_help.as_str(),
            ),
            ("setup_notes", text.setup_notes, setup_help.as_str()),
            ("launch_notes", text.launch_notes, launch_help.as_str()),
            (
                "config_root_hint",
                text.config_root_hint,
                launch_help.as_str(),
            ),
            (
                "trust_reminder_label",
                text.trust_reminder_label,
                setup_source,
            ),
        ] {
            assert!(
                haystack.contains(fragment),
                "{id}.{name} {fragment:?} is not in the text it feeds"
            );
        }
    }
    // The fragments each adapter owns, and only those.
    let claude = registration("claude").text();
    assert!(claude.setup_scope.starts_with("  claude  "));
    assert!(!claude.setup_foreground.is_empty() && !claude.setup_notes.is_empty());
    assert!(claude.launch_notes.is_empty() && claude.trust_reminder_label.is_empty());
    let codex = registration("codex").text();
    assert!(codex.setup_scope.starts_with("  codex   "));
    assert!(!codex.setup_foreground.is_empty() && !codex.launch_notes.is_empty());
    assert_eq!(codex.trust_reminder_label, "codex hook trust");
    assert_eq!(codex.config_root_hint, "CODEX_HOME");
    assert_eq!(claude.config_root_hint, "CLAUDE_CONFIG_DIR");
    let hermes = registration("hermes").text();
    assert!(hermes.setup_scope.starts_with("  hermes  ") && !hermes.launch_notes.is_empty());
    assert!(hermes.setup_foreground.is_empty() && hermes.setup_notes.is_empty());
    assert!(hermes.version_history_note.is_empty() && hermes.trust_reminder_label.is_empty());
}

// ---------------------------------------------------------------- extension proof

fn env_of(
    set: &'static [(&'static str, &'static str)],
) -> impl Fn(&str) -> Option<std::ffi::OsString> {
    move |name| {
        set.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| std::ffi::OsString::from(value))
    }
}

/// One test-only adapter in a test registry reaches the markers and the assembled help with no
/// core edit: its guard marker refuses by presence, its output marker needs a value and never
/// refuses, and its fragments appear after the built-in ones' position in registry order.
#[test]
fn adapter_seam_test_adapter_reaches_markers_and_help_without_core_edits() {
    use crate::cli::{agent_env_marker_in, help, output::harness_marked_in};
    let registry = validate(vec![
        probe(
            "probe",
            vec![
                marker("HT_TEST_MARKER", true),
                marker("HT_TEST_OUTPUT", false),
            ],
            AdapterText {
                setup_scope: "  probe   $PROBE_HOME/hooks (default ~/.probe/hooks)",
                setup_foreground: "Probe: turn off its daemon.",
                setup_notes: "Probe notes: confirm trust.",
                launch_notes: "Probe launches report `probe`.",
                config_root_hint: "PROBE_HOME",
                ..AdapterText::NONE
            },
            None,
        ),
        probe(
            "second",
            vec![],
            AdapterText {
                setup_scope: "  second  $SECOND_HOME/hooks",
                setup_foreground: "Second: nothing to do.",
                ..AdapterText::NONE
            },
            None,
        ),
    ])
    .unwrap();

    assert!(harness_marked_in(
        &registry,
        env_of(&[("HT_TEST_MARKER", "1")])
    ));
    assert!(harness_marked_in(
        &registry,
        env_of(&[("HT_TEST_OUTPUT", "1")])
    ));
    assert!(!harness_marked_in(
        &registry,
        env_of(&[("HT_TEST_MARKER", "")])
    ));
    // A built-in marker means nothing to a registry that does not declare it.
    assert!(!harness_marked_in(
        &registry,
        env_of(&[("CLAUDECODE", "1")])
    ));

    assert_eq!(
        agent_env_marker_in(&registry, [("HT_TEST_MARKER", "")]),
        Some("HT_TEST_MARKER".to_owned())
    );
    assert_eq!(
        agent_env_marker_in(&registry, [("HT_TEST_OUTPUT", "1")]),
        None
    );
    assert_eq!(agent_env_marker_in(&registry, [("CLAUDECODE", "1")]), None);

    let me = help::me_init_help_in(&registry);
    assert!(
        me.contains("agent markers (HT_TEST_MARKER) or a registered agent harness"),
        "{me}"
    );
    assert!(
        !me.contains("HT_TEST_OUTPUT") && !me.contains("CLAUDECODE"),
        "{me}"
    );

    let setup = help::setup_help_in(&registry);
    let at = |needle: &str| {
        setup
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} missing"))
    };
    assert!(at("  probe   $PROBE_HOME/hooks") < at("  second  $SECOND_HOME/hooks"));
    assert!(at("Probe: turn off its daemon. Second: nothing to do.") > at("Foreground agents"));
    assert!(at("Probe notes: confirm trust.") > at("Second: nothing to do."));
    assert!(at("Probe notes: confirm trust.") < at("Herdr instance:"));
    let launch = help::launch_help_in(&registry);
    assert!(
        launch.contains("settings or hook file of the PROBE_HOME the agent"),
        "{launch}"
    );
    assert!(
        launch.contains("registry.\n\nProbe launches report `probe`.\n\nExit status:"),
        "{launch}"
    );
}

/// The built-in help carries each marker class exactly: guards are named, the output-only
/// `CODEX_THREAD_ID` is not.
#[test]
fn adapter_seam_builtin_me_init_help_names_the_guard_markers_only() {
    let me = crate::cli::help::me_init_help();
    assert!(
        me.contains(
            "agent markers (CLAUDECODE, CODEX_SANDBOX, CODEX_SANDBOX_NETWORK_DISABLED, ANTIGRAVITY_AGENT) or a registered agent harness reported by Herdr are present"
        ),
        "{me}"
    );
    assert!(!me.contains("CODEX_THREAD_ID"));
}

/// The foreground hook reassembles exactly the report core builds today: same keys, same values,
/// same order, for every outcome of the read.
#[test]
fn adapter_seam_foreground_advice_reproduces_the_core_report() {
    let dir = scratch("foreground");
    let env = |home: &Path| SetupEnv {
        home: None,
        declared_environment: Default::default(),
        executable: PathBuf::from("/x/herdr-threads"),
        state_dir: None,
        cwd: PathBuf::from("/"),
        path: None,
        config_roots: [
            ("codex".into(), home.join("codex")),
            ("claude".into(), home.join("claude")),
        ]
        .into(),
        host_endpoint: None,
        instance_source: Value::Null,
    };
    // The assembly D5 gives core: skeleton, fixed fields, then the read.
    let assemble = |advice: &ForegroundAdvice, root: &Path, explanation: &Value| {
        let path = root.join(advice.settings_file);
        let mut report = json!({
            "status": "required", "scope": "user", "execution": "unknown",
            "settings_file": path.display().to_string(),
            "explanation": explanation, "advice": advice.advice, "modified_by_setup": false,
        });
        for (key, value) in advice.fixed_fields {
            report[*key] = json!(value);
        }
        let result = match std::fs::read(&path) {
            Err(_) => {
                report["user_config"] = json!("missing");
                Ok(())
            }
            Ok(bytes) => (advice.inspect)(&bytes, &mut report).map(|()| {
                report["user_config"] = json!("read");
            }),
        };
        if let Err(error) = result {
            report["status"] = json!("unknown");
            report["inspection_error"] = json!(error);
        }
        // Then the adapter's optional persistent-environment reading (home None: no files).
        if let Some(inspect) = advice.inspect_environment {
            inspect(&Default::default(), &mut report);
        }
        report
    };
    let cases: [(&str, Harness, &str, &[&str]); 2] = [
        (
            "claude",
            Harness::Claude,
            "claude",
            &[
                r#"{"disableAgentView":true}"#,
                r#"{"env":{"CLAUDE_CODE_DISABLE_AGENT_VIEW":"1"},"x":1}"#,
                r#"{"env":{"CLAUDE_CODE_DISABLE_AGENT_VIEW":"off"}}"#,
                r#"{"disableAgentView":"yes"}"#,
                r#"{"env":{"CLAUDE_CODE_DISABLE_AGENT_VIEW":1}}"#,
                "[]",
                "not json",
            ],
        ),
        (
            "codex",
            Harness::Codex,
            "codex",
            &[
                "[features]\ndaemon_auto_start = false\n",
                "[features]\ndaemon_auto_start = \"x\"\n",
                "other = 1\n",
                "= not toml",
            ],
        ),
    ];
    for (id, harness, sub, samples) in cases {
        let advice = registration(id).foreground_advice().expect("advice");
        let root = dir.join(sub);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join(advice.settings_file);
        let mut variants: Vec<Option<&str>> = vec![None];
        variants.extend(samples.iter().copied().map(Some));
        for sample in variants {
            let _ = std::fs::remove_file(&file);
            if let Some(text) = sample {
                std::fs::write(&file, text).unwrap();
            }
            let core = foreground::inspect(harness, &env(&dir));
            let ours = assemble(advice, &root, &core["explanation"]);
            assert_eq!(
                serde_json::to_string(&ours).unwrap(),
                serde_json::to_string(&core).unwrap(),
                "{id} {sample:?}"
            );
        }
    }
    assert!(registration("hermes").foreground_advice().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn adapter_seam_default_config_root_follows_the_declared_variable_then_home() {
    let environment = |var: &str, value: Option<&str>, home: Option<&str>| SetupEnvironment {
        home: home.map(Into::into),
        declared: value
            .into_iter()
            .map(|value| (var.to_owned(), value.into()))
            .collect(),
        ..SetupEnvironment::default()
    };
    for (id, var, dir) in [
        ("claude", "CLAUDE_CONFIG_DIR", ".claude"),
        ("codex", "CODEX_HOME", ".codex"),
    ] {
        let registration = registration(id);
        assert_eq!(
            registration.default_config_root(&environment(var, Some("/cfg"), Some("/h"))),
            Some(PathBuf::from("/cfg")),
            "{id} set"
        );
        for empty in [None, Some("")] {
            assert_eq!(
                registration.default_config_root(&environment(var, empty, Some("/h"))),
                Some(PathBuf::from("/h").join(dir)),
                "{id} unset/empty falls back to home"
            );
        }
        assert_eq!(
            registration.default_config_root(&environment(var, None, None)),
            None,
            "{id} with no home"
        );
    }
    assert_eq!(
        claude::setup::claude_config_dir_from(Some("/c".into()), Some("/h".into())),
        Some(PathBuf::from("/c"))
    );
    let hermes = registration("hermes");
    assert_eq!(
        hermes.default_config_root(&environment("HERMES_HOME", Some("/x"), Some("/h"))),
        None
    );
}

#[test]
fn adapter_seam_version_syntax_normalizes_like_core() {
    let normalize = |syntax: &VersionSyntax, raw: &str| {
        let trimmed = raw.trim_matches(|c: char| c.is_ascii_whitespace());
        let core = trimmed.strip_suffix(syntax.suffix).unwrap_or(trimmed);
        let core = core.strip_prefix(syntax.prefix).unwrap_or(core);
        crate::harness::recipe::Version::parse(core).map(|v| v.to_string())
    };
    for id in ["claude", "codex"] {
        let syntax = registration(id).version_syntax().expect("syntax");
        assert_eq!(syntax.examples, contract::normalize_examples(id), "{id}");
        for (raw, normalized) in syntax.examples {
            assert_eq!(normalize(syntax, raw).as_deref(), Some(*normalized));
        }
        for raw in [
            "2.1.287 (Claude Code)",
            "codex-cli 0.158.0",
            "  codex-cli 0.158.0\n",
            "2.1.287",
            "0.158.0",
            "codex-cli 0.158.0-beta.1",
            "2.1.287 (Claude Code) extra",
            "",
            "garbage",
        ] {
            assert_eq!(
                normalize(syntax, raw),
                contract::normalize_version(id, raw),
                "{id} {raw:?}"
            );
        }
    }
    assert_eq!(
        normalize(
            registration("claude").version_syntax().unwrap(),
            "2.1.287 (Claude Code)"
        )
        .as_deref(),
        Some("2.1.287")
    );
    assert_eq!(
        normalize(
            registration("codex").version_syntax().unwrap(),
            "codex-cli 0.158.0"
        )
        .as_deref(),
        Some("0.158.0")
    );
    assert!(registration("hermes").version_syntax().is_none());
}

#[test]
fn adapter_seam_transcript_version_specs_read_what_attribution_reads() {
    let claude_spec = registration("claude").transcript_version().expect("claude");
    assert_eq!(claude_spec.read, TranscriptRead::NewestEntry);
    assert!(claude_spec.current_runtime);
    assert_eq!(
        (claude_spec.extract)(&json!({"version":"2.1.287"})),
        Some("2.1.287")
    );
    assert_eq!((claude_spec.extract)(&json!({"version":3})), None);
    assert_eq!((claude_spec.extract)(&json!({"other":"2.1.287"})), None);

    let codex_spec = registration("codex").transcript_version().expect("codex");
    assert_eq!(codex_spec.read, TranscriptRead::HeadLine);
    assert!(!codex_spec.current_runtime);
    let meta = json!({"type":"session_meta","payload":{"cli_version":"0.158.0"}});
    assert_eq!((codex_spec.extract)(&meta), Some("0.158.0"));
    let other = json!({"type":"event_msg","payload":{"cli_version":"0.158.0"}});
    assert_eq!((codex_spec.extract)(&other), None);
    assert_eq!(
        (codex_spec.extract)(&json!({"type":"session_meta","payload":{}})),
        None
    );
    assert!(registration("hermes").transcript_version().is_none());
}

#[test]
fn adapter_seam_settings_kinds_name_their_adapter_and_declare_today_s_groups() {
    let claude_kind = registration("claude").settings_kind().expect("claude");
    assert_eq!(
        (claude_kind.harness, claude_kind.file_name),
        ("claude", "settings.json")
    );
    let allow = claude_kind
        .owned_allow
        .expect("claude owns an allow member");
    assert_eq!(allow.rule, claude::HERDR_THREADS_ALLOW_RULE);
    assert_eq!(allow.retired, [claude::CALLER_CONTEXT_ALLOW_RULE]);
    assert!(!allow.native_rewrites);
    assert_eq!(
        (claude_kind.hook_groups)("cmd"),
        claude::declared_hook_groups("cmd")
    );
    assert_eq!(
        (claude_kind.hook_groups)("cmd")
            .iter()
            .map(|(event, _)| *event)
            .collect::<Vec<_>>(),
        [
            "SessionStart",
            "PreToolUse",
            "SubagentStart",
            "SubagentStop"
        ]
    );

    let codex_kind = registration("codex").settings_kind().expect("codex");
    assert_eq!(
        (codex_kind.harness, codex_kind.file_name),
        ("codex", "hooks.json")
    );
    assert!(codex_kind.owned_allow.is_none());
    assert_eq!(
        (codex_kind.hook_groups)("cmd")
            .iter()
            .map(|(event, _)| *event)
            .collect::<Vec<_>>(),
        [
            "SessionStart",
            "SubagentStart",
            "PreToolUse",
            "UserPromptSubmit",
            "Stop",
            "SubagentStop",
            "SessionEnd"
        ]
    );
    assert_eq!(codex_kind.legacy_upgrade_warning, codex::RETRUST_WARNING);
    assert!(codex_kind.adoption_note.contains("recorded trust"));
    assert!(registration("hermes").settings_kind().is_none());
    assert_ne!(
        claude::settings_kind(),
        codex::settings_kind(),
        "kinds compare by (harness, file name)"
    );
    assert_eq!(
        claude::settings_kind(),
        SettingsKind::new(&claude::SETTINGS_KIND)
    );
}

#[test]
fn adapter_seam_report_hooks_return_todays_values() {
    let claude = registration("claude");
    let lines = claude.setup_report_lines(&json!({
        "prompt_suggestions": {"action": "disabled"},
        "mod": "installed",
    }));
    assert_eq!(
        lines,
        [
            "prompt suggestions: disabled (`promptSuggestionEnabled: false`)".to_owned(),
            "mod: installed".to_owned(),
        ]
    );
    assert!(claude.setup_report_lines(&json!({})).is_empty());
    assert!(
        registration("codex")
            .setup_report_lines(&json!({"mod": "x"}))
            .is_empty()
    );

    assert_eq!(
        registration("codex").doctor_summary_claims(),
        ["Codex hook review required"]
    );
    assert!(claude.doctor_summary_claims().is_empty());

    // setup_status_extras is a no-op unless the projection carries the mod object.
    let request = StatusRequest {
        scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("/nowhere")),
        environment: SetupEnvironment::default(),
        native_binary: None,
    };
    let mut projection = json!({"installed": true});
    claude.setup_status_extras(&request, &mut projection);
    assert_eq!(projection, json!({"installed": true}));
}

#[test]
fn adapter_seam_hermes_supplies_only_text() {
    let hermes = registration("hermes");
    assert_eq!(hermes.metadata().agent_env_markers.len(), 0);
    assert!(hermes.settings_kind().is_none() && hermes.foreground_advice().is_none());
    assert!(hermes.version_syntax().is_none() && hermes.transcript_version().is_none());
    assert!(hermes.doctor_summary_claims().is_empty());
    assert_eq!(
        hermes.doctor_text(&json!({"x": 1}), DoctorTextForm::Debug),
        ""
    );
    assert_eq!(
        hermes.doctor_text(&json!({"x": 1}), DoctorTextForm::Summary),
        ""
    );
    assert!(hermes.setup_report_lines(&json!({"mod": "x"})).is_empty());
    assert!(!hermes.text().setup_scope.is_empty());
    #[cfg(feature = "test-support")]
    {
        let fourth = registration("synthetic_fourth");
        assert_eq!(fourth.text().fields(), AdapterText::NONE.fields());
        assert!(fourth.settings_kind().is_none() && fourth.metadata().agent_env_markers.is_empty());
    }
}

// ---------------------------------------------------------------- allow-member-only engine

const MEMBER: &str = "Bash(agy-test *)";

fn member_spec(native_rewrites: bool) -> &'static SettingsKindSpec {
    Box::leak(Box::new(SettingsKindSpec {
        harness: "synthetic",
        file_name: "settings.json",
        hook_groups: no_groups,
        owned_allow: Some(OwnedAllowSpec {
            rule: MEMBER,
            retired: &[],
            native_rewrites,
        }),
        adoption_note: "",
        legacy_upgrade_warning: "",
        permission_rules: crate::harness::setup::no_permission_rules,
    }))
}

fn scratch(label: &str) -> PathBuf {
    let dir = crate::test_support::scratch::temp_path(format!(
        "ht-seam-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Member {
    dir: PathBuf,
    kind: SettingsKind,
    config: PathBuf,
    manifest: PathBuf,
}
impl Member {
    fn new(native_rewrites: bool, base: &str) -> Self {
        let dir = scratch("member");
        let config = dir.join("settings.json");
        let manifest = dir.join("state").join("manifest.json");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&config, base).unwrap();
        Self {
            dir,
            kind: SettingsKind::new(member_spec(native_rewrites)),
            config,
            manifest,
        }
    }
    fn install(&self, base: &str) -> Result<crate::harness::setup::OwnershipManifest, SetupError> {
        install_user_settings(
            self.kind,
            &self.config,
            &self.manifest,
            &["/x/herdr-threads".to_owned()],
            base.as_bytes(),
        )
    }
    fn inspect(&self) -> crate::harness::setup::HookInspection {
        inspect_user_settings(
            self.kind,
            &self.config,
            &self.manifest,
            NativeObservation::Unknown,
        )
        .unwrap()
    }
    fn remove(&self) -> Result<(), SetupError> {
        remove_user_settings(self.kind, &self.config, &self.manifest)
    }
    fn file(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.config).unwrap()).unwrap()
    }
    fn rewrite(&self, value: Value) {
        std::fs::write(&self.config, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}
impl Drop for Member {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn adapter_seam_member_only_pre_existing_member_is_recorded_and_never_removed() {
    let base = format!(r#"{{"permissions":{{"allow":["{MEMBER}"]}},"keep":1}}"#);
    let member = Member::new(true, &base);
    let manifest = member.install(&base).unwrap();
    assert!(manifest.owned.is_empty() && manifest.phase == InstallPhase::Installed);
    let permission = manifest.permission.expect("recorded");
    assert!(permission.pre_existing && permission.rule == MEMBER);
    assert_eq!(
        std::fs::read(&member.config).unwrap(),
        base.as_bytes(),
        "file untouched"
    );
    let inspection = member.inspect();
    assert!(inspection.installed && inspection.adopted.is_none());
    let allow = inspection.allow_rule.expect("allow rule reported");
    assert_eq!(allow.ownership, AllowRuleOwnership::PreExisting);
    assert!(allow.present && allow.rule == MEMBER);
    member.remove().unwrap();
    assert_eq!(std::fs::read(&member.config).unwrap(), base.as_bytes());
    assert!(!member.manifest.exists());
}

#[test]
fn adapter_seam_member_only_creates_permissions_and_prunes_them_on_unsetup() {
    let base = r#"{"other":1}"#;
    let member = Member::new(true, base);
    let manifest = member.install(base).unwrap();
    assert!(!manifest.permission.as_ref().unwrap().pre_existing);
    assert_eq!(
        member.file(),
        json!({"other":1,"permissions":{"allow":[MEMBER]}})
    );
    let inspection = member.inspect();
    assert!(inspection.installed);
    assert_eq!(
        inspection.allow_rule.unwrap().ownership,
        AllowRuleOwnership::Owned
    );
    assert!(inspection.configured_hook.is_some());
    // Re-running setup is a no-op.
    let before = std::fs::read(&member.config).unwrap();
    member.install(base).unwrap();
    assert_eq!(std::fs::read(&member.config).unwrap(), before);
    member.remove().unwrap();
    assert_eq!(
        member.file(),
        json!({"other":1}),
        "permissions pruned: the base lacked it"
    );
    assert!(!member.manifest.exists());
}

#[test]
fn adapter_seam_member_only_prunes_only_containers_the_base_lacked() {
    // permissions present with no allow: allow is created and pruned, permissions stays.
    let base = r#"{"permissions":{"deny":["Bash(rm *)"]}}"#;
    let member = Member::new(true, base);
    member.install(base).unwrap();
    assert_eq!(
        member.file(),
        json!({"permissions":{"deny":["Bash(rm *)"],"allow":[MEMBER]}})
    );
    member.remove().unwrap();
    assert_eq!(
        member.file(),
        json!({"permissions":{"deny":["Bash(rm *)"]}})
    );
    // an empty allow array the user had stays.
    let base = r#"{"permissions":{"allow":[]}}"#;
    let member = Member::new(true, base);
    member.install(base).unwrap();
    assert_eq!(member.file(), json!({"permissions":{"allow":[MEMBER]}}));
    member.remove().unwrap();
    assert_eq!(member.file(), json!({"permissions":{"allow":[]}}));
    // a user's other allow entries survive and only the owned member goes.
    let base = r#"{"permissions":{"allow":["Read(*)"]}}"#;
    let member = Member::new(true, base);
    member.install(base).unwrap();
    assert_eq!(
        member.file(),
        json!({"permissions":{"allow":["Read(*)",MEMBER]}})
    );
    member.remove().unwrap();
    assert_eq!(member.file(), json!({"permissions":{"allow":["Read(*)"]}}));
}

#[test]
fn adapter_seam_member_only_native_rewrite_keeping_the_member_stays_installed() {
    let base = r#"{"other":1,"more":2}"#;
    let member = Member::new(true, base);
    member.install(base).unwrap();
    // The harness rewrites the file, dropping an unrelated key and reordering.
    member.rewrite(json!({"permissions":{"allow":[MEMBER]},"more":2}));
    let inspection = member.inspect();
    assert!(
        inspection.installed,
        "installed is not gated on the whole-file fingerprint"
    );
    assert!(inspection.allow_rule.unwrap().present);
    let rewritten = std::fs::read(&member.config).unwrap();
    member.install(base).unwrap();
    assert_eq!(
        std::fs::read(&member.config).unwrap(),
        rewritten,
        "setup leaves it alone"
    );
}

#[test]
fn adapter_seam_member_only_rewrite_removing_the_member_is_missing_then_readded() {
    let base = r#"{"other":1}"#;
    let member = Member::new(true, base);
    member.install(base).unwrap();
    member.rewrite(json!({"other":2}));
    let inspection = member.inspect();
    assert!(!inspection.installed);
    let allow = inspection.allow_rule.expect("reported");
    assert!(!allow.present, "status reports the member missing");
    // Setup re-adds and records it.
    let manifest = member.install(base).unwrap();
    assert!(!manifest.permission.as_ref().unwrap().pre_existing);
    assert_eq!(
        member.file(),
        json!({"other":2,"permissions":{"allow":[MEMBER]}})
    );
    assert!(member.inspect().installed);
    let recorded = read_settings_manifest(&member.manifest).unwrap().unwrap();
    assert_eq!(recorded.permission, manifest.permission);
    // The re-recorded base is the file without the member: unsetup restores that.
    member.remove().unwrap();
    assert_eq!(member.file(), json!({"other":2}));
}

#[test]
fn adapter_seam_member_only_user_member_removed_by_rewrite_becomes_owned_when_readded() {
    let base = format!(r#"{{"permissions":{{"allow":["{MEMBER}"]}}}}"#);
    let member = Member::new(true, &base);
    assert!(
        member
            .install(&base)
            .unwrap()
            .permission
            .unwrap()
            .pre_existing
    );
    member.rewrite(json!({}));
    assert!(!member.inspect().installed);
    let manifest = member.install(&base).unwrap();
    assert!(
        !manifest.permission.unwrap().pre_existing,
        "this setup added it, so unsetup removes it"
    );
    member.remove().unwrap();
    assert_eq!(member.file(), json!({}));
}

#[test]
fn adapter_seam_member_only_without_native_rewrites_refuses_a_missing_member() {
    let base = r#"{"other":1}"#;
    let member = Member::new(false, base);
    member.install(base).unwrap();
    member.rewrite(json!({"other":1}));
    assert_eq!(member.install(base).err(), Some(SetupError::Conflict));
    assert_eq!(
        member.file(),
        json!({"other":1}),
        "nothing written on refusal"
    );
    assert!(!member.inspect().installed);
    // Removal of the missing member still cleans up the manifest.
    member.remove().unwrap();
    assert!(!member.manifest.exists());
}

#[test]
fn adapter_seam_member_only_refuses_a_foreign_or_edited_baseline() {
    let base = r#"{"other":1}"#;
    let member = Member::new(true, base);
    // The file changed since the baseline was read.
    assert_eq!(
        member.install(r#"{"other":2}"#).err(),
        Some(SetupError::Conflict)
    );
    assert_eq!(member.file(), json!({"other":1}));
    // A file name outside the kind is refused outright.
    let other = member.dir.join("hooks.json");
    std::fs::write(&other, base).unwrap();
    assert_eq!(
        install_user_settings(
            member.kind,
            &other,
            &member.manifest,
            &["x".into()],
            base.as_bytes()
        )
        .err(),
        Some(SetupError::Invalid)
    );
    // A non-object file is not composable.
    std::fs::write(&member.config, "[]").unwrap();
    assert_eq!(member.install("[]").err(), Some(SetupError::Invalid));
}
