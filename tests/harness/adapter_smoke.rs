//! Synthetic configuration and same-binary author proof. No native/model claim.
use herdr_threads::cli::hook::parse_hook_argv;
use herdr_threads::harness::registry::builtins;
use std::ffi::OsString;

fn hook(harness: &str, event: &str) -> Vec<OsString> {
    ["herdr-threads", "hook", harness, "--event", event]
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Kills a fixed two-brand hook selector before any fixture claims success.
#[test]
fn adapter_smoke_four_configurations_real_entrypoints_and_main_conservation() {
    let producer = ["herdr-threads", "hook", "hermes"].map(OsString::from);
    assert!(
        parse_hook_argv(&producer).unwrap().is_ok(),
        "the shipped Hermes producer uses hook hermes without --event"
    );
    for event in [
        "pre_llm_call",
        "post_tool_call",
        "on_session_start",
        "on_session_reset",
    ] {
        let args = parse_hook_argv(&hook("hermes", event))
            .expect("registered hook invocation")
            .unwrap_or_else(|error| panic!("registered Hermes {event} was refused: {error}"));
        assert_eq!(args.harness.as_str(), "hermes");
        assert_eq!(args.event.as_deref(), Some(event));
    }
    let registry = builtins();
    assert_eq!(
        registry
            .registrations()
            .iter()
            .map(|r| r.metadata().id)
            .collect::<Vec<_>>(),
        ["claude", "codex", "hermes", "synthetic_fourth"]
    );
    fourth_author_consumers();
    settings_conservation();
}

fn fixture_environment(
    isolation: &herdr_threads::test_support::isolation::TestIsolation,
) -> herdr_threads::harness::adapter::SetupEnvironment {
    use herdr_threads::harness::adapter::SetupEnvironment;
    let home = isolation.home();
    let root = home.join("synthetic fourth Ω");
    SetupEnvironment {
        home: Some(home.clone().into_os_string()),
        cwd: home.join("bin"),
        executable: std::path::PathBuf::from(env!("CARGO_BIN_EXE_herdr-threads")),
        state_dir: Some(isolation.path("state")),
        host_endpoint: Some(isolation.socket_path("host.sock")),
        path: Some(home.clone().into_os_string()),
        config_roots: [
            ("claude".into(), home.join("claude")),
            ("codex".into(), home.join("codex")),
        ]
        .into_iter()
        .collect(),
        declared: [("HT_SYNTHETIC_FOURTH_ROOT".into(), root.into_os_string())]
            .into_iter()
            .collect(),
        ..Default::default()
    }
}

fn fourth_author_consumers() {
    use herdr_threads::test_support::{
        isolation::TestIsolation,
        synthetic_fourth::{ID, ROOT_INPUT},
    };
    use herdr_threads::{
        cli::setup::{SetupVerb, execute_registered},
        harness::{adapter::*, discovery::Discovery, registry::OccupantHarness},
        protocol::time::{CallBudget, MonoInstant},
    };
    let isolation = TestIsolation::new("adapter-author");
    let env = fixture_environment(&isolation);
    let registry = builtins();
    let id = registry.agent(ID).unwrap();
    let registration = registry.by_id(id).unwrap();
    for alias in ["SyntheticFourth", "SyntheticFourthAlias"] {
        assert_eq!(
            registry.by_context_spelling(alias).unwrap(),
            OccupantHarness::Agent(id)
        );
    }
    for kind in ["synthetic_fourth", "synthetic_fourth_alias"] {
        assert!(std::ptr::eq(
            registry.by_host_kind(kind).unwrap(),
            registration
        ));
    }
    assert!(registry.by_host_kind("unknown-fourth-kind").is_none());
    assert!(registry.agent("human").is_err());
    let discovery = Discovery::parse(
        herdr_threads::harness::discovery::render(registry)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        discovery
            .adapters
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["claude", "codex", "hermes", ID]
    );
    let entry = &discovery.adapters[3];
    assert_eq!(entry.display_name, "Synthetic fourth (test-support only)");
    assert_eq!(entry.contracts.len(), 1);
    assert_eq!(entry.contracts[0].domain, "synthetic");
    assert_eq!(
        entry.contracts[0].id,
        registration.contracts()[0].contract_id_v2().unwrap()
    );
    assert!(entry.canary_strategy.is_none());
    assert!(registration.composer_policy().is_none());
    assert_eq!(
        herdr_threads::harness::composer::read_composer(
            OccupantHarness::Agent(id),
            "draft",
            Some(80)
        ),
        herdr_threads::harness::composer::ComposerRead::Unreadable
    );
    assert!(registration.canary_strategy().is_none());
    let budget = CallBudget {
        deadline: MonoInstant(env.clock.monotonic_now().0 + 30_000),
        cancellation: Default::default(),
    };
    let admitted = registration
        .admit(
            &AdmissionRequest {
                installed: InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
            },
            &budget,
        )
        .unwrap();
    assert_eq!(admitted.kind(), AdmissionKind::ContractDeclared);
    for role in ["top_level", "child", "unknown"] {
        let input = HookInput {
            registered_event: Some("SyntheticStart".into()),
            bytes: serde_json::to_vec(&serde_json::json!({"event": "SyntheticStart",
                "event_id": "fixture-event", "session_id": "fixture-session", "role": role}))
            .unwrap(),
        };
        let event = registration.decode(&admitted, &input).unwrap();
        assert_eq!(event.can_check_in(), role == "top_level");
        assert!(matches!(
            event.runtime,
            RuntimeAttribution::Unavailable { .. }
        ));
        let outcome = herdr_threads::cli::hook::run_admitted_hook(
            &herdr_threads::cli::hook::HookArgs {
                state_dir: env.state_dir.clone(),
                host_endpoint: env.host_endpoint.clone(),
                harness: OccupantHarness::Agent(id).into(),
                event: Some("SyntheticStart".into()),
            },
            registration,
            &admitted,
            &event,
            &herdr_threads::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            std::sync::Arc::clone(&env.clock),
            None,
        );
        assert!(
            outcome.attention.is_none(),
            "no daemon or canonical offer exists in this codec fixture"
        );
        if role == "unknown" {
            assert!(outcome.stdout.is_empty());
        } else {
            let output: serde_json::Value = serde_json::from_slice(&outcome.stdout).unwrap();
            assert!(output["synthetic_context"].as_str().is_some());
            if role == "child" {
                assert!(
                    output["synthetic_context"]
                        .as_str()
                        .unwrap()
                        .contains("forbidden to subagents")
                );
            }
        }
        let other = registry.by_id(registry.agent("claude").unwrap()).unwrap();
        assert!(other.decode(&admitted, &input).is_err());
        assert!(other.validate_event(&admitted, &event).is_err());
    }
    std::fs::create_dir_all(&env.cwd).unwrap();
    for payload in [
        serde_json::json!({"event":"SyntheticStart","event_id":"bad","session_id":"session","role":"human"}),
        serde_json::json!({"event":"Unsupported","event_id":"bad","session_id":"session","role":"top_level"}),
        serde_json::json!({"event":"SyntheticStart","event_id":"bad","session_id":"session","role":"top_level","raw_body":"private"}),
    ] {
        assert!(
            registration
                .decode(
                    &admitted,
                    &HookInput {
                        registered_event: None,
                        bytes: serde_json::to_vec(&payload).unwrap()
                    }
                )
                .is_err()
        );
    }
    assert!(
        matches!(herdr_threads::harness::registry::Registry::new(Box::leak(vec![
        herdr_threads::harness::registry::Registration::new(&herdr_threads::test_support::synthetic_fourth::ADAPTER),
        herdr_threads::harness::registry::Registration::new(&herdr_threads::test_support::synthetic_fourth::ADAPTER),
    ].into_boxed_slice())), Err(herdr_threads::harness::registry::RegistryError::DuplicateId(id)) if id == ID)
    );
    assert!(
        herdr_threads::test_support::adapter_smoke_parse_hook(
            &hook(ID, "SyntheticStart"),
            registry
        )
        .unwrap()
        .is_ok()
    );
    let binary = env.cwd.join("ht-synthetic-fourth");
    std::fs::write(&binary, b"synthetic fourth fixture\n").unwrap();
    let explicit_install = InstallEnvironment {
        clock: std::sync::Arc::clone(&env.clock),
        path: Some(env.cwd.clone().into_os_string()),
        config_root: Some(env.cwd.clone()),
        state_dir: env.state_dir.clone(),
    };
    assert!(
        matches!(registration.observe_install(&explicit_install, &budget),
        InstallObservation::ExecutableAvailable { binary: observed } if observed == binary)
    );
    let undeclared_install = InstallEnvironment {
        config_root: None,
        ..explicit_install
    };
    assert!(
        matches!(
            registration.observe_install(&undeclared_install, &budget),
            InstallObservation::Unavailable { .. }
        ),
        "PATH alone must not imply a fourth fixture root"
    );

    for name in ["claude", "codex", ID] {
        let registration = registry.by_id(registry.agent(name).unwrap()).unwrap();
        let native = if name == ID {
            binary.clone()
        } else {
            let path = env.cwd.join(name);
            std::fs::write(&path, b"#!/bin/sh\nexit 99\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        execute_registered(
            registration,
            SetupVerb::Install,
            &SetupScopeRequest::Default,
            Some(&native),
            Default::default(),
            &env,
        )
        .unwrap();
        let status = execute_registered(
            registration,
            SetupVerb::Status,
            &SetupScopeRequest::Default,
            Some(&native),
            Default::default(),
            &env,
        )
        .unwrap();
        assert_eq!(status["installed"], true, "{name}: {status}");
        assert_ne!(
            status["observed"], "observed",
            "setup cannot invent callback delivery"
        );
    }
    let scope = registration
        .resolve_setup_scope(&SetupScopeRequest::Default, &env)
        .unwrap();
    let SetupStatus::Detailed(status) = registration.status(
        &StatusRequest {
            scope: scope.clone(),
            environment: env.clone(),
            native_binary: Some(binary.clone()),
        },
        &budget,
    ) else {
        panic!("actual fourth local status unavailable")
    };
    assert!(status.installed);
    assert_eq!(status.observed, None);
    let policy = registration.launch_policy().unwrap();
    assert!(policy.validate_native_argv(&["refused".into()]).is_err());
    let request = LaunchRequest {
        argv: vec!["synthetic-prompt".into()],
        environment: env.clone(),
        native_binary: Some(binary),
    };
    let prepared = registration
        .prepare_launch(
            &request,
            &LaunchScope {
                setup: scope,
                working_directory: env.cwd.clone(),
                config_source: "explicit_synthetic_fixture",
            },
            &admitted,
            &status,
            &NoShell,
            &budget,
        )
        .unwrap();
    assert_eq!(prepared.argv, ["--synthetic-only", "synthetic-prompt"]);
    assert_eq!(prepared.hook.fingerprint, status.fingerprint.unwrap());
    let host = launch_fixture::Host(std::sync::atomic::AtomicU64::new(1));
    let hooks = launch_fixture::Hooks(prepared.hook.clone());
    let managed = herdr_threads::harness::launch::prepare_managed_with_registry(
        registry,
        &host,
        &launch_fixture::Seats,
        &hooks,
        env.clock.as_ref(),
        herdr_threads::harness::launch::ManagedLaunchRequest {
            target: herdr_threads::protocol::ids::HostTargetId::new("w1:p4"),
            harness: OccupantHarness::Agent(id),
            argv: vec!["synthetic-prompt".into()],
            shell_passes_no_daemon: false,
            name_hint: Some("synthetic author fixture".into()),
        },
        &budget,
        Some(prepared.argv.clone()),
    )
    .unwrap();
    assert_eq!(managed.request.harness.as_str(), ID);
    assert_eq!(managed.request.argv, prepared.argv);
    assert!(matches!(
        herdr_threads::harness::launch::submit_prepared(&host, env.clock.as_ref(), managed)
            .unwrap(),
        herdr_threads::ports::NativeLaunchOutcome::OutcomeUnknown
    ));

    let parsed = herdr_threads::cli::commands::parse_argv([
        "herdr-threads",
        "doctor",
        "--json",
        "--harness",
        ID,
        "--state-dir",
        env.state_dir.as_ref().unwrap().to_str().unwrap(),
        "--host-endpoint",
        env.host_endpoint.as_ref().unwrap().to_str().unwrap(),
    ])
    .unwrap();
    let mut output = Vec::new();
    let _ = herdr_threads::test_support::adapter_smoke_doctor(&parsed, registry, &env, &mut output);
    let doctor: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        doctor["doctor"]["local_harnesses"][ID]["installed"], true,
        "{doctor}"
    );
    assert_eq!(
        doctor["doctor"]["local_harnesses"][ID]["observed"],
        serde_json::Value::Null,
        "{doctor}"
    );
    let store = std::sync::Arc::new(
        herdr_threads::store::SqliteStore::new(
            herdr_threads::store::connection::StoreContext::new(
                isolation.path("health.db"),
                std::sync::Arc::clone(&env.clock),
            ),
            "synthetic-author",
            Default::default(),
        )
        .unwrap(),
    );
    let provider = herdr_threads::daemon::harness_states::HarnessStatesProvider::new(
        store,
        herdr_threads::daemon::harness_states::embedded_source(),
        std::sync::Arc::clone(&env.clock),
        Box::new(|_| None),
        None,
    )
    .with_registry(registry)
    .with_observations(Box::new(|| Ok(Default::default())));
    let health = provider.report_v2(&budget).unwrap();
    assert_eq!(
        health
            .harnesses
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["claude", "codex", "hermes", ID]
    );
    let fourth_health = &health.harnesses[ID];
    assert!(fourth_health.runtime_evidence.is_empty());
    assert_eq!(
        fourth_health.installation.state,
        herdr_threads::protocol::results::InstallationState::Unknown
    );
    assert_eq!(
        fourth_health.admission.state,
        herdr_threads::protocol::results::AdmissionState::Unknown
    );
    assert!(
        provider
            .health_lines(&budget)
            .unwrap()
            .iter()
            .all(|line| !line.contains("runtime verified"))
    );
    let asset = std::path::PathBuf::from(&env.declared[ROOT_INPUT]).join("owned-hooks.json");
    let original = std::fs::read(&asset).unwrap();
    std::fs::write(&asset, b"foreign asset").unwrap();
    assert!(
        execute_registered(
            registration,
            SetupVerb::Remove,
            &SetupScopeRequest::Default,
            None,
            Default::default(),
            &env
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&asset).unwrap(), b"foreign asset");
    std::fs::write(&asset, original).unwrap();
    execute_registered(
        registration,
        SetupVerb::Remove,
        &SetupScopeRequest::Default,
        None,
        Default::default(),
        &env,
    )
    .unwrap();
    assert!(
        !std::path::PathBuf::from(&env.declared[ROOT_INPUT])
            .join("owned-hooks.json")
            .exists()
    );
}

struct NoShell;
impl herdr_threads::harness::launch::CodexShellProbe for NoShell {
    fn resolve_codex(&self) -> Result<String, String> {
        panic!("fourth adapter must not probe a Codex shell")
    }
}

fn settings_conservation() {
    use herdr_threads::{protocol::results::HealthSettings, service::config::ServiceConfig};
    let boot = uuid::Uuid::new_v4();
    assert_eq!(
        ServiceConfig::default()
            .store_settings(boot)
            .wake_batch_delay_ms,
        0
    );
    assert_eq!(
        herdr_threads::store::StoreSettings::default().wake_batch_delay_ms,
        0
    );
    let config = ServiceConfig::new(120_000, 240_000, 45_000)
        .unwrap()
        .with_wake_batch_delay(15_000)
        .unwrap();
    assert_eq!(config.store_settings(boot).minimum_wake_delay_ms, 45_000);
    assert_eq!(config.store_settings(boot).wake_batch_delay_ms, 15_000);
    assert_eq!(
        config
            .clone()
            .with_wake_batch_delay(0)
            .unwrap()
            .health_settings()
            .wake_batch_delay_ms,
        0
    );
    let legacy: HealthSettings = serde_json::from_value(serde_json::json!({
        "invitation_default_ms": 300_000, "receipt_default_ms": 300_000, "minimum_wake_delay_ms": 30_000,
    })).unwrap();
    assert_eq!(legacy.wake_batch_delay_ms, 30_000);
    let mut current = serde_json::to_value(legacy).unwrap();
    current["wake_batch_delay_ms"] = 0.into();
    assert_eq!(
        serde_json::from_value::<HealthSettings>(current)
            .unwrap()
            .wake_batch_delay_ms,
        0
    );
}

/// Kills relaxing the parser while making snake_case event selectors reachable.
#[test]
fn hook_selector_refuses_unknown_human_and_unbounded_event_tokens() {
    for harness in ["unknown", "human", "Human", "Claude", "claude\n", "hermes/"] {
        assert!(
            parse_hook_argv(&hook(harness, "SessionStart"))
                .unwrap()
                .is_err(),
            "{harness:?}"
        );
    }
    for event in [
        "".to_owned(),
        "x".repeat(64),
        "event-name".to_owned(),
        "event.name".to_owned(),
        "event name".to_owned(),
        "event\n".to_owned(),
    ] {
        assert!(
            parse_hook_argv(&hook("claude", &event)).unwrap().is_err(),
            "{event:?}"
        );
    }
    let mut duplicate = hook("claude", "SessionStart");
    duplicate.splice(
        1..1,
        ["--state-dir", "/fixture-a", "--state-dir", "/fixture-b"].map(OsString::from),
    );
    assert!(parse_hook_argv(&duplicate).unwrap().is_err());
    let mut identical = hook("codex", "SessionStart");
    identical.splice(
        1..1,
        ["--state-dir", "/fixture-a", "--state-dir", "/fixture-a"].map(OsString::from),
    );
    assert_eq!(
        parse_hook_argv(&identical)
            .unwrap()
            .unwrap()
            .state_dir
            .unwrap(),
        std::path::Path::new("/fixture-a")
    );
}

/// Pure guarded-host ports: the real launch consumer performs both observations
/// and preserves uncertainty. No native process or model is started.
mod launch_fixture {
    use herdr_threads::{
        harness::launch::*,
        ports::*,
        protocol::{ids::*, results::ApiError, time::*},
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    pub struct Host(pub AtomicU64);
    impl HostPort for Host {
        fn native_launch_capability(&self) -> NativeLaunchCapability {
            NativeLaunchCapability::HostGuardedStart
        }
        fn observe_current_target(
            &self,
            target: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<HostObservation, ApiError> {
            let sequence = self.0.fetch_add(1, Ordering::SeqCst);
            Ok(HostObservation {
                focused: false,
                target: target.clone(),
                host_boot: HostBootId::new("synthetic-host"),
                epoch: 1,
                generation: 1,
                observed_at_utc: UtcMillis(0),
                observed_at_mono: MonoInstant(sequence),
                provenance: ObservationProvenance::FreshCurrentTarget,
                occupant: None,
                ui: HostUiState::Idle,
                terminal: Some(TerminalId::new("synthetic-terminal")),
                occupancy: StructuralOccupancy::EmptyShell,
                incarnation: IncarnationEvidence::Verified {
                    identity: "synthetic-host".into(),
                    evidence_kind: EvidenceKind::NativeCurrentTarget,
                },
                execution: ExecutionEvidence::Unknown,
                call_id: HostCallId::new(format!("synthetic-{sequence}")),
                connection_epoch: 1,
                observation_sequence: sequence,
                started_at_mono: MonoInstant(sequence),
                completed_at_mono: MonoInstant(sequence),
            })
        }
        fn observe_current_target_for_archival(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<ComposerObservation, ApiError> {
            panic!("launch cannot inspect archival")
        }
        fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
            panic!("launch cannot enumerate")
        }
        fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
            panic!("launch cannot wake")
        }
        fn submit_prompt(
            &self,
            _: &SafeWakeTarget,
            _: &str,
            _: &HostCallContext,
        ) -> Result<PromptOutcome, ApiError> {
            panic!("launch cannot prompt")
        }
        fn launch_native(
            &self,
            request: NativeLaunchRequest,
            _: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            assert_eq!(request.harness.as_str(), "synthetic_fourth");
            assert_eq!(
                self.0.load(Ordering::SeqCst),
                3,
                "two fresh observations must precede submission"
            );
            Ok(NativeLaunchOutcome::OutcomeUnknown)
        }
        fn pane_agent_state(
            &self,
            _: &SafeWakeTarget,
            _: &HostCallContext,
        ) -> Result<AgentComposerState, ApiError> {
            panic!("launch cannot read composer")
        }
        fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
            panic!("launch cannot submit key")
        }
    }
    pub struct Seats;
    impl LaunchSeatResolver for Seats {
        fn resolve_for_launch(
            &self,
            target: &HostTargetId,
            _: &HostObservation,
            _: &CallBudget,
        ) -> Result<SeatId, ApiError> {
            assert_eq!(target.as_str(), "w1:p4");
            Ok(SeatId::new("synthetic-seat"))
        }
    }
    pub struct Hooks(pub ConfiguredHook);
    impl LaunchHookInspector for Hooks {
        fn configured_hook(
            &self,
            harness: herdr_threads::protocol::authority::Harness,
            _: &CallBudget,
        ) -> Result<Option<ConfiguredHook>, ApiError> {
            assert_eq!(harness.as_str(), "synthetic_fourth");
            Ok(Some(self.0.clone()))
        }
    }
}

/// A fourth author selects callback admission with the enum alone. The actual
/// hook consumer regression lives in cli::hook; this checks the public author seam.
mod enum_only_callback {
    use herdr_threads::test_support::synthetic_fourth::ADAPTER;
    use herdr_threads::{harness::adapter::*, protocol::time::CallBudget};
    struct CallbackFourth;
    const PAYLOAD: &[u8] = br#"{"event":"SyntheticStart","event_id":"fourth-callback","session_id":"fourth-session","role":"top_level"}"#;
    impl HarnessAdapter for CallbackFourth {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            ADAPTER.metadata()
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            ADAPTER.contracts()
        }
        fn hook_admission_policy(&self) -> HookAdmissionPolicy {
            HookAdmissionPolicy::QualifiedCallback
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            panic!("callback must not use installed observation")
        }
        fn admit(&self, request: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            let input = request
                .input
                .as_ref()
                .expect("enum-only fourth must receive callback input");
            assert_eq!(input.bytes, PAYLOAD);
            assert_eq!(input.registered_event.as_deref(), Some("SyntheticStart"));
            assert!(matches!(
                request.installed,
                InstallObservation::Unsupported(_)
            ));
            assert!(request.runtime_candidate.is_none());
            AdmissionDecision::ContractDeclared {
                state: (),
                recipe: "enum-only fourth callback",
            }
        }
        fn version_ladder(&self, r: &RuntimeIdentity) -> Ladder {
            ADAPTER.version_ladder(r)
        }
        fn classify(&self, i: &HookInput) -> ContractObservation {
            ADAPTER.classify(i)
        }
        fn decode(&self, a: &(), i: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            ADAPTER.decode(a, i)
        }
        fn encode(
            &self,
            a: &(),
            e: &DecodedEvent,
            o: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            ADAPTER.encode(a, e, o)
        }
        fn attribute_runtime(&self, i: &HookInput, b: &CallBudget) -> RuntimeAttribution {
            ADAPTER.attribute_runtime(i, b)
        }
        fn setup(&self, r: &SetupRequest, b: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            ADAPTER.setup(r, b)
        }
        fn status(&self, r: &StatusRequest, b: &CallBudget) -> SetupStatus {
            ADAPTER.status(r, b)
        }
        fn unsetup(
            &self,
            r: &UnsetupRequest,
            b: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            ADAPTER.unsetup(r, b)
        }
    }
    #[test]
    fn qualified_callback_enum_alone_controls_fourth_author_admission() {
        use herdr_threads::{
            harness::registry::*,
            protocol::time::{Clock, MonoInstant},
        };
        let iso =
            herdr_threads::test_support::isolation::TestIsolation::new("fourth-enum-admission");
        static CALLBACK: CallbackFourth = CallbackFourth;
        let registry = Registry::new(Box::leak(Box::new([Registration::new(&CALLBACK)]))).unwrap();
        let id = registry.agent("synthetic_fourth").unwrap();
        let registration = registry.by_id(id).unwrap();
        // This public forwarding predicate must agree with the sole author policy.
        assert!(registration.callback_admission());
        let clock = std::sync::Arc::new(herdr_threads::app::SystemClock::new());
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 1000),
            cancellation: Default::default(),
        };
        let admitted = registration
            .admit(
                &AdmissionRequest {
                    installed: InstallObservation::Unsupported(UnsupportedOperation {
                        adapter: "synthetic_fourth",
                        operation: "callback startup identity",
                    }),
                    input: Some(HookInput {
                        bytes: PAYLOAD.to_vec(),
                        registered_event: Some("SyntheticStart".into()),
                    }),
                    runtime_candidate: None,
                },
                &budget,
            )
            .unwrap();
        for (event, role) in [
            ("SyntheticStart", "top_level"),
            ("SyntheticStart", "child"),
            ("SyntheticStart", "unknown"),
            ("SyntheticObserver", "top_level"),
        ] {
            let input=HookInput { bytes:serde_json::to_vec(&serde_json::json!({"event":event,"event_id":"fourth-callback","session_id":"fourth-session","role":role})).unwrap(),registered_event:Some(event.into()) };
            let decoded = registration.decode(&admitted, &input).unwrap();
            assert_eq!(
                decoded.can_check_in(),
                event == "SyntheticStart" && role == "top_level"
            );
            assert!(matches!(
                decoded.runtime,
                RuntimeAttribution::Unavailable { .. }
            ));
            assert!(
                builtins()
                    .by_id(builtins().agent("claude").unwrap())
                    .unwrap()
                    .decode(&admitted, &input)
                    .is_err()
            );
            let outcome = herdr_threads::cli::hook::run_admitted_hook(
                &herdr_threads::cli::hook::HookArgs {
                    state_dir: Some(iso.path("state")),
                    host_endpoint: Some(iso.path("host.sock")),
                    harness: OccupantHarness::Agent(id).into(),
                    event: Some(event.into()),
                },
                registration,
                &admitted,
                &decoded,
                &herdr_threads::cli::hook::HookEnv {
                    herdr_env: true,
                    pane: Some("w1:p1".into()),
                },
                std::time::Instant::now() + std::time::Duration::from_secs(1),
                clock.clone(),
                None,
            );
            assert!(outcome.attention.is_none());
            if role == "unknown" {
                assert!(outcome.stdout.is_empty());
            } else {
                let output: serde_json::Value = serde_json::from_slice(&outcome.stdout).unwrap();
                if event == "SyntheticObserver" {
                    assert!(output["synthetic_context"].is_null());
                }
                if role == "child" {
                    assert!(
                        output["synthetic_context"]
                            .as_str()
                            .unwrap()
                            .contains("forbidden to subagents")
                    );
                }
            }
        }
    }
}

#[test]
fn hermes_presence_controls_never_invoke_or_admit_the_executable() {
    use herdr_threads::{
        app::SystemClock,
        harness::adapter::{
            AdmissionRequest, HarnessStatus, InstallEnvironment, InstallObservation,
        },
        protocol::{
            results::{CallbackObservationState, EnablementState, HarnessState},
            time::{CallBudget, Cancellation, Clock, MonoInstant},
        },
        test_support::isolation::TestIsolation,
    };
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        sync::Arc,
    };
    let iso = TestIsolation::new("hermes-presence-controls");
    let bin = iso.home().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.join("hermes");
    let sentinel = bin.join("executed");
    std::fs::write(&path, "#!/bin/sh\n: > \"${0%/*}/executed\"\nexit 91\n").unwrap();
    let clock = Arc::new(SystemClock::new());
    let env = InstallEnvironment {
        clock: clock.clone(),
        path: Some(bin.clone().into_os_string()),
        config_root: None,
        state_dir: None,
    };
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let r = builtins()
        .by_id(builtins().agent("hermes").unwrap())
        .unwrap();
    // A regular file and an executable are different installation observations.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        r.observe_daemon(&env, &budget).status,
        HarnessStatus::NotInstalled(_)
    ));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let installed = r.observe_install(&env, &budget);
    assert!(
        matches!(&installed, InstallObservation::ExecutableAvailable { binary } if binary == &path)
    );
    assert!(
        r.admit(
            &AdmissionRequest {
                installed,
                input: None,
                runtime_candidate: None
            },
            &budget
        )
        .is_err()
    );
    let observed = r.observe_daemon(&env, &budget);
    assert!(matches!(
        observed.status,
        HarnessStatus::PresentUnqualified { .. }
    ));
    assert_eq!(observed.status.state(), HarnessState::Unsupported);
    assert_eq!(observed.identity, None);
    assert_eq!(observed.receipt_basis, None);
    assert_eq!(observed.enablement.state, EnablementState::Unknown);
    assert_eq!(
        observed.callback_observation.state,
        CallbackObservationState::Unknown
    );
    assert_eq!(observed, r.observe_daemon(&env, &budget));
    assert!(!sentinel.exists());

    let expired = CallBudget {
        deadline: clock.monotonic_now(),
        cancellation: Cancellation::default(),
    };
    let cancelled = CallBudget {
        deadline: budget.deadline,
        cancellation: Cancellation::default(),
    };
    cancelled.cancellation.cancel();
    for control in [&expired, &cancelled] {
        assert!(matches!(
            r.observe_install(&env, control),
            InstallObservation::Unavailable { .. }
        ));
        let unavailable = r.observe_daemon(&env, control);
        assert!(matches!(unavailable.status, HarnessStatus::Refused(_)));
        assert_eq!(unavailable.identity, None);
        assert_eq!(unavailable.enablement.state, EnablementState::Unknown);
        assert_eq!(
            unavailable.callback_observation.state,
            CallbackObservationState::Unknown
        );
    }
    // Actual inaccessible directory: restore its permissions before assertions
    // so even a failed control cannot strand the isolated fixture at teardown.
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o000)).unwrap();
    let inaccessible = r.observe_daemon(&env, &budget);
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(inaccessible.status, HarnessStatus::Refused(_)));
    std::fs::remove_file(&path).unwrap();
    symlink("hermes", &path).unwrap();
    assert!(matches!(
        r.observe_daemon(&env, &budget).status,
        HarnessStatus::Refused(_)
    ));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        r.observe_daemon(&env, &budget).status,
        HarnessStatus::NotInstalled(_)
    ));
    std::fs::remove_dir(&path).unwrap();
    assert!(matches!(
        r.observe_daemon(&env, &budget).status,
        HarnessStatus::NotInstalled(_)
    ));
    assert!(!sentinel.exists());
    // Bounded captured environments are refused rather than scanned indefinitely.
    for captured_path in [
        std::env::join_paths((0..257).map(|n| bin.join(format!("missing-{n}")))).unwrap(),
        OsString::from("x".repeat(65_537)),
    ] {
        let bounded = InstallEnvironment {
            clock: clock.clone(),
            path: Some(captured_path),
            config_root: None,
            state_dir: None,
        };
        assert!(matches!(
            r.observe_daemon(&bounded, &budget).status,
            HarnessStatus::Refused(_)
        ));
    }
    // Do not inherit the caller's PATH when the captured environment has none.
    let no_path = InstallEnvironment { path: None, ..env };
    assert!(matches!(
        r.observe_daemon(&no_path, &budget).status,
        HarnessStatus::NotInstalled(_)
    ));
}
