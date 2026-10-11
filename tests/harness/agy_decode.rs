//! agy contract, decode/encode, metadata, admission, paths and hook argv (L2a).
//! Included from `src/harness/agy/decode.rs`; fixtures are recorded payloads with `<root>` paths.
use super::*;
use crate::harness::agy::{
    AgyAdapter,
    paths::{AgyEvent, AgyRoots, conversation_state, hook_command},
};
use crate::harness::{
    adapter::{
        AdmissionDecision, AdmissionRequest, ContractDomain, DecodeFailure, DecodedEvent,
        DeliveryEligibility, EncodeFailure, EncodedOutput, EventIntent, EventRole, HarnessAdapter,
        HookInput, InstallObservation, NeutralOffer, RuntimeAttribution, SetupEnvironment,
    },
    context::EventKind,
    contract::{Classification, Malformed},
    registry::builtins,
};
use crate::protocol::time::{CallBudget, MonoInstant};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const FIXTURES: [(&str, &str); 8] = [
    ("session-start-top", "SessionStart"),
    ("session-start-child", "SessionStart"),
    ("session-start-after-clear", "SessionStart"),
    ("pre-invocation-inv0-fresh", "PreInvocation"),
    ("pre-invocation-inv0-resume", "PreInvocation"),
    ("pre-invocation-inv1", "PreInvocation"),
    ("stop-no-tool-call", "Stop"),
    ("stop-inject-error", "Stop"),
];

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agy/payloads")
        .join(format!("{name}.json"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
fn fixture(name: &str) -> Value {
    serde_json::from_slice(&fixture_bytes(name)).unwrap()
}
fn input(bytes: Vec<u8>, event: Option<&str>) -> HookInput {
    HookInput {
        bytes,
        registered_event: event.map(str::to_owned),
    }
}
fn input_of(value: &Value, event: &str) -> HookInput {
    input(serde_json::to_vec(value).unwrap(), Some(event))
}
fn classify(input: &HookInput) -> Classification {
    AgyAdapter.classify(input).classification
}
fn admitted() -> AgyAdmission {
    AgyAdmission {
        step: InjectionStep::SystemMessage,
    }
}
fn decode_fixture(name: &str, event: &str) -> DecodedEvent {
    AgyAdapter
        .decode(&admitted(), &input(fixture_bytes(name), Some(event)))
        .unwrap()
}
fn without(name: &str, key: &str) -> Value {
    let mut value = fixture(name);
    value.as_object_mut().unwrap().remove(key).unwrap();
    value
}
fn offer(text: &str) -> NeutralOffer {
    NeutralOffer {
        fixed_guidance: text.into(),
        peer_data: Value::Null,
        ready_argv: vec![],
    }
}
fn out(output: EncodedOutput) -> (bool, Vec<u8>) {
    match output {
        EncodedOutput::ContextBearing { bytes } => (true, bytes),
        EncodedOutput::ObserverOnly { bytes } => (false, bytes),
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(100),
        cancellation: Default::default(),
    }
}

#[test]
fn classify_accepts_every_recorded_payload_for_its_event() {
    for (name, event) in FIXTURES {
        let input = input(fixture_bytes(name), Some(event));
        assert_eq!(
            classify(&input),
            Classification::Ok {
                event: match event {
                    "SessionStart" => "SessionStart",
                    "PreInvocation" => "PreInvocation",
                    _ => "Stop",
                }
            },
            "{name}"
        );
        assert_eq!(AgyAdapter.classify(&input).domain, ContractDomain::Native);
    }
}

#[test]
fn classify_names_the_missing_or_mistyped_required_field_per_event() {
    let cases = [
        ("session-start-top", "SessionStart", "conversationId"),
        ("session-start-top", "SessionStart", "transcriptPath"),
        ("pre-invocation-inv1", "PreInvocation", "conversationId"),
        ("pre-invocation-inv1", "PreInvocation", "invocationNum"),
        ("pre-invocation-inv1", "PreInvocation", "initialNumSteps"),
        ("stop-no-tool-call", "Stop", "conversationId"),
        ("stop-no-tool-call", "Stop", "terminationReason"),
    ];
    for (name, event, key) in cases {
        let observed = classify(&input_of(&without(name, key), event));
        let Classification::Violation { event: got, field } = observed else {
            panic!("{name} without {key}: {observed:?}");
        };
        assert_eq!((got, field), (event, key));
    }
    let mut mistyped = fixture("pre-invocation-inv1");
    mistyped["invocationNum"] = json!("1");
    assert_eq!(
        classify(&input_of(&mistyped, "PreInvocation")),
        Classification::Violation {
            event: "PreInvocation",
            field: "invocationNum"
        }
    );
    // Optional fields may be absent: a Stop without error/fullyIdle/executionNum is still Ok.
    let mut sparse = without("stop-no-tool-call", "error");
    sparse.as_object_mut().unwrap().remove("fullyIdle");
    sparse.as_object_mut().unwrap().remove("executionNum");
    assert_eq!(
        classify(&input_of(&sparse, "Stop")),
        Classification::Ok { event: "Stop" }
    );
}

#[test]
fn classify_reads_a_reserved_discriminator_or_missing_registration_as_malformed() {
    let mut reserved = fixture("stop-no-tool-call");
    reserved["$event"] = json!("Stop");
    assert_eq!(
        classify(&input_of(&reserved, "Stop")),
        Classification::Malformed(Malformed::NotObject)
    );
    assert_eq!(
        classify(&input(fixture_bytes("stop-no-tool-call"), None)),
        Classification::Malformed(Malformed::UnknownEvent)
    );
    assert_eq!(
        classify(&input(
            fixture_bytes("stop-no-tool-call"),
            Some("PreToolUse")
        )),
        Classification::Malformed(Malformed::UnknownEvent)
    );
    assert_eq!(
        classify(&input(b"[1,2]".to_vec(), Some("Stop"))),
        Classification::Malformed(Malformed::NotObject)
    );
}

#[test]
fn classify_passes_non_json_and_oversize_input_through_unchanged() {
    assert_eq!(
        classify(&input(b"not json".to_vec(), Some("Stop"))),
        Classification::Malformed(Malformed::NotJson)
    );
    assert_eq!(
        classify(&input(b"not json".to_vec(), None)),
        Classification::Malformed(Malformed::NotJson)
    );
    assert_eq!(
        classify(&input(vec![b' '; 65_537], Some("Stop"))),
        Classification::Malformed(Malformed::TooLarge)
    );
}

#[test]
fn decode_table_matches_the_design_per_fixture() {
    for (name, event) in FIXTURES {
        let payload = fixture(name);
        let decoded = decode_fixture(name, event);
        assert_eq!(decoded.role, EventRole::Unknown, "{name}");
        assert_eq!(decoded.harness.as_str(), "agy");
        assert_eq!(
            decoded.native_session.as_deref(),
            payload["conversationId"].as_str(),
            "{name}"
        );
        assert_eq!(decoded.metadata.native_event, event);
        assert_eq!(decoded.metadata.context_source, event);
        assert_eq!(decoded.metadata.domain, ContractDomain::Native);
        assert!(decoded.metadata.shape_fields.is_empty());
        assert!(decoded.metadata.callback_deadline.is_none());
        assert!(matches!(
            decoded.runtime,
            RuntimeAttribution::Unavailable { .. }
        ));
        assert!(!decoded.event_id.is_empty());
        match event {
            "SessionStart" => {
                assert!(matches!(
                    decoded.intent,
                    EventIntent::Lifecycle(EventKind::Startup)
                ));
                assert!(matches!(decoded.delivery, DeliveryEligibility::Context));
                assert!(decoded.metadata.skill_pointer);
            }
            _ => {
                assert!(matches!(decoded.intent, EventIntent::Observer));
                assert!(matches!(
                    decoded.delivery,
                    DeliveryEligibility::ObserverOnly
                ));
                assert!(!decoded.metadata.skill_pointer);
            }
        }
        // Unknown never checks in: silent until the witnesses decide the role.
        assert!(!decoded.can_check_in(), "{name}");
    }
    let ids: std::collections::BTreeSet<_> = FIXTURES
        .iter()
        .map(|(name, event)| decode_fixture(name, event).event_id)
        .collect();
    assert_eq!(ids.len(), FIXTURES.len(), "event ids are fresh per decode");
}

#[test]
fn decode_refuses_missing_registration_and_invalid_conversation_ids() {
    let invalid = |input: HookInput| {
        matches!(
            AgyAdapter.decode(&admitted(), &input),
            Err(DecodeFailure::Invalid(_))
        )
    };
    assert!(invalid(input(fixture_bytes("session-start-top"), None)));
    assert!(invalid(input(
        fixture_bytes("session-start-top"),
        Some("PreToolUse")
    )));
    assert!(invalid(input(b"not json".to_vec(), Some("Stop"))));
    for bad in ["", "../etc", "a/b", "a b", "a.b", &"x".repeat(129)] {
        let mut payload = fixture("session-start-top");
        payload["conversationId"] = json!(bad);
        assert!(invalid(input_of(&payload, "SessionStart")), "{bad:?}");
    }
    let mut numeric = fixture("session-start-top");
    numeric["conversationId"] = json!(7);
    assert!(invalid(input_of(&numeric, "SessionStart")));
    assert!(invalid(input_of(
        &without("session-start-top", "conversationId"),
        "SessionStart"
    )));
    let mut longest = fixture("session-start-top");
    longest["conversationId"] = json!("a-Z_9".repeat(25) + "abc");
    assert_eq!(longest["conversationId"].as_str().unwrap().len(), 128);
    assert!(
        AgyAdapter
            .decode(&admitted(), &input_of(&longest, "SessionStart"))
            .is_ok()
    );
}

#[test]
fn encode_bytes_are_locked_for_both_injection_steps() {
    let event = decode_fixture("session-start-top", "SessionStart");
    let (_, system) = out(AgyAdapter.encode(&admitted(), &event, &offer("T")).unwrap());
    assert_eq!(
        String::from_utf8(system).unwrap(),
        r#"{"injectSteps":[{"systemMessage":{"systemMessage":"T"}}]}"#
    );
    let user = AgyAdmission {
        step: InjectionStep::UserMessage,
    };
    let (_, user) = out(AgyAdapter.encode(&user, &event, &offer("T")).unwrap());
    assert_eq!(
        String::from_utf8(user).unwrap(),
        r#"{"injectSteps":[{"userMessage":"T"}]}"#
    );
}

#[test]
fn encode_escapes_text_and_bounds_it() {
    let event = decode_fixture("session-start-top", "SessionStart");
    let (_, bytes) = out(AgyAdapter
        .encode(&admitted(), &event, &offer("a \"b\"\n\u{e9}"))
        .unwrap());
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed["injectSteps"][0]["systemMessage"]["systemMessage"],
        "a \"b\"\n\u{e9}"
    );
    let (_, empty) = out(AgyAdapter.encode(&admitted(), &event, &offer("")).unwrap());
    assert!(empty.is_empty());
    let exact = "x".repeat(4096);
    assert!(
        AgyAdapter
            .encode(&admitted(), &event, &offer(&exact))
            .is_ok()
    );
    let over = "x".repeat(4097);
    assert!(matches!(
        AgyAdapter.encode(&admitted(), &event, &offer(&over)),
        Err(EncodeFailure::Invalid(_))
    ));
}

#[test]
fn encode_classifies_context_bearing_only_for_a_top_level_non_observer_context_event() {
    let mut event = decode_fixture("session-start-top", "SessionStart");
    let kind =
        |event: &DecodedEvent| out(AgyAdapter.encode(&admitted(), event, &offer("T")).unwrap()).0;
    assert!(!kind(&event), "Unknown role is observer-only");
    event.role = EventRole::Subagent;
    assert!(!kind(&event));
    event.role = EventRole::TopLevel;
    assert!(kind(&event));
    event.delivery = DeliveryEligibility::ObserverOnly;
    assert!(!kind(&event));
    event.delivery = DeliveryEligibility::Context;
    event.intent = EventIntent::Observer;
    assert!(!kind(&event));
}

#[test]
fn encode_prefixes_a_lifecycle_that_a_turn_start_carried() {
    let mut event = decode_fixture("pre-invocation-inv0-resume", "PreInvocation");
    event.role = EventRole::TopLevel;
    event.intent = EventIntent::Lifecycle(EventKind::Resume);
    event.delivery = DeliveryEligibility::Context;
    let (bearing, bytes) = out(AgyAdapter.encode(&admitted(), &event, &offer("T")).unwrap());
    assert!(bearing);
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed["injectSteps"][0]["systemMessage"]["systemMessage"],
        "herdr-threads: this conversation is the pane's top-level session; any earlier herdr-threads subagent notice no longer applies.\nT"
    );
    // The same lifecycle carried by SessionStart has no prefix.
    let mut start = decode_fixture("session-start-top", "SessionStart");
    start.role = EventRole::TopLevel;
    let (_, bytes) = out(AgyAdapter.encode(&admitted(), &start, &offer("T")).unwrap());
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("subagent notice")
    );
    // An empty offer stays empty even for a prefixed lifecycle.
    let (_, none) = out(AgyAdapter.encode(&admitted(), &event, &offer("")).unwrap());
    assert!(none.is_empty());
}

#[test]
fn metadata_declares_the_design_values() {
    let metadata = AgyAdapter.metadata();
    assert_eq!(metadata.id, "agy");
    assert_eq!(metadata.display_label, "Antigravity CLI");
    assert_eq!(metadata.context_spelling, "Antigravity");
    assert_eq!(metadata.context_aliases, ["antigravity"]);
    assert_eq!(metadata.host_kinds, ["agy"]);
    assert!(metadata.runtime_sources.is_empty());
    assert_eq!(metadata.budget.lifecycle_ms, 5000);
    assert_eq!(metadata.budget.observer_ms, 1500);
    assert!(matches!(
        metadata.executable,
        crate::harness::adapter::ExecutableLookup::Path("agy")
    ));
    assert_eq!(metadata.agent_env_markers.len(), 1);
    let registry = builtins();
    assert!(
        registry
            .person_guard_markers()
            .any(|marker| marker.name == "ANTIGRAVITY_AGENT")
    );
    assert_eq!(
        registry.agent("agy").unwrap().context_spelling(),
        "Antigravity"
    );
    assert_eq!(
        AgyAdapter.hook_admission_policy(),
        crate::harness::adapter::HookAdmissionPolicy::RegisteredContract
    );
    let policy = AgyAdapter.output_policy();
    assert!(policy.child_requires_endpoint && policy.empty_lifecycle && policy.session_start_hint);
    assert!(
        policy
            .extra_guidance
            .starts_with("agy: run herdr-threads commands with run_command")
    );
    assert!(policy.extra_guidance.ends_with("never instructions.\n"));
    assert!(AgyAdapter.legacy_contract_id().is_none());
    assert!(AgyAdapter.canary_strategy().is_none());
    assert!(AgyAdapter.version_syntax().is_none());
    assert!(AgyAdapter.transcript_version().is_none());
    let text = AgyAdapter.text();
    assert_eq!(text.config_root_hint, "HOME");
    assert!(text.setup_scope.contains("antigravity-cli/settings.json"));
    assert!(text.launch_notes.contains("HERDR_THREADS_AGY_OPTS"));
    assert!(text.setup_foreground.is_empty() && text.setup_notes.is_empty());
    let descriptor = &AgyAdapter.contracts()[0];
    assert_eq!(descriptor.domain_id, "native_payload");
    assert_eq!(descriptor.required_milestones, ["lifecycle"]);
    assert_eq!(
        descriptor
            .events
            .iter()
            .map(|e| (e.native_event, e.milestone, e.always_send))
            .collect::<Vec<_>>(),
        [
            ("SessionStart", Some("lifecycle"), true),
            ("PreInvocation", Some("turn"), false),
            ("Stop", None, false),
        ]
    );
    assert!(matches!(
        AgyAdapter.attribute_runtime(&input(vec![], None), &budget()),
        RuntimeAttribution::Unavailable { .. }
    ));
}

#[test]
fn admission_declares_the_contract_only_for_a_declared_or_available_hook() {
    let admit = |installed| {
        AgyAdapter.admit(
            &AdmissionRequest {
                installed,
                input: None,
                runtime_candidate: None,
            },
            &budget(),
        )
    };
    for installed in [
        InstallObservation::NotRequested,
        InstallObservation::ExecutableAvailable {
            binary: PathBuf::from("/usr/bin/agy"),
        },
    ] {
        let AdmissionDecision::ContractDeclared { state, recipe } = admit(installed) else {
            panic!("expected ContractDeclared");
        };
        assert_eq!(state.step, InjectionStep::SystemMessage);
        assert_eq!(recipe, "agy-hooks-v1");
    }
    assert!(matches!(
        admit(InstallObservation::Unavailable {
            diagnostic: "none".into()
        }),
        AdmissionDecision::Refused { .. }
    ));
}

#[test]
fn paths_derive_from_home_and_hash_conversation_state() {
    let roots = AgyRoots::from_home(Path::new("/home/u"));
    assert_eq!(roots.config_root, Path::new("/home/u/.gemini"));
    assert_eq!(roots.data, Path::new("/home/u/.gemini/antigravity-cli"));
    assert_eq!(roots.config, Path::new("/home/u/.gemini/config"));
    assert_eq!(
        roots.plugin_dir,
        Path::new("/home/u/.gemini/config/plugins/herdr-threads")
    );
    assert_eq!(
        roots.settings,
        Path::new("/home/u/.gemini/antigravity-cli/settings.json")
    );
    assert_eq!(
        roots.plugins_config,
        Path::new("/home/u/.gemini/config/config.json")
    );
    let environment = SetupEnvironment {
        home: Some("/home/u".into()),
        ..Default::default()
    };
    assert_eq!(
        AgyAdapter.default_config_root(&environment).unwrap(),
        roots.config_root
    );
    assert!(
        AgyAdapter
            .default_config_root(&SetupEnvironment::default())
            .is_none()
    );

    // sha256("abc") starts ba7816bf8f01cfea414140de5dae2223.
    assert_eq!(
        conversation_state(Path::new("/s"), "abc"),
        Path::new("/s/conversations/ba7816bf8f01cfea414140de5dae2223.json")
    );
    assert_ne!(
        conversation_state(Path::new("/s"), "abc"),
        conversation_state(Path::new("/s"), "abd")
    );
    assert!(paths::valid_conversation_id(
        "534e47d6-64b3-4fd6-bd7f-446ceec4d86c"
    ));
    assert!(!paths::valid_conversation_id(""));
    assert!(!paths::valid_conversation_id("a/b"));
}

fn shell_words(command: &str) -> Vec<std::ffi::OsString> {
    // The command is `'w1' 'w2' ...` with no embedded quotes in these tests.
    command
        .split("' '")
        .map(|word| word.trim_matches('\'').into())
        .collect()
}

#[test]
fn hook_command_round_trips_through_the_registered_hook_parser() {
    let environment = SetupEnvironment {
        executable: PathBuf::from("/opt/ht/herdr-threads"),
        state_dir: Some(PathBuf::from("/var/st")),
        host_endpoint: Some(PathBuf::from("/var/host.sock")),
        ..Default::default()
    };
    for event in AgyEvent::ALL {
        let command = hook_command(&environment, event).unwrap();
        assert_eq!(
            command,
            format!(
                "'/opt/ht/herdr-threads' '--state-dir' '/var/st' '--host-endpoint' '/var/host.sock' 'hook' 'agy' '--event' '{}'",
                event.as_str()
            )
        );
        let parsed = crate::cli::hook::parse_hook_argv(&shell_words(&command))
            .expect("a hook invocation")
            .expect("valid hook argv");
        assert_eq!(parsed.harness.as_str(), "agy");
        assert_eq!(parsed.event.as_deref(), Some(event.as_str()));
        assert_eq!(parsed.state_dir.as_deref(), Some(Path::new("/var/st")));
        assert_eq!(
            parsed.host_endpoint.as_deref(),
            Some(Path::new("/var/host.sock"))
        );
        assert_eq!(AgyEvent::parse(event.as_str()), Some(event));
    }
    assert_eq!(AgyEvent::parse("PreToolUse"), None);
    let mut missing_state = environment.clone();
    missing_state.state_dir = None;
    assert!(hook_command(&missing_state, AgyEvent::Stop).is_err());
    let mut missing_host = environment;
    missing_host.host_endpoint = None;
    assert!(hook_command(&missing_host, AgyEvent::Stop).is_err());
}

#[test]
fn hook_agy_prints_nothing_on_every_fixture_without_a_pane_or_daemon() {
    use crate::cli::hook::{HookArgs, HookEnv, InstalledHarness, run_hook};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let clock: Arc<dyn crate::protocol::time::Clock> = Arc::new(crate::app::SystemClock::new());
    let harness =
        crate::harness::registry::OccupantHarness::Agent(builtins().agent("agy").unwrap()).into();
    for (name, event) in FIXTURES {
        let args = HookArgs {
            state_dir: None,
            host_endpoint: None,
            harness,
            event: Some(event.into()),
        };
        for env in [
            HookEnv {
                herdr_env: false,
                pane: None,
            },
            HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
        ] {
            let outcome = run_hook(
                &args,
                &InstalledHarness::Claude("0.0.0".into()),
                &fixture_bytes(name),
                &env,
                Instant::now() + Duration::from_secs(1),
                clock.clone(),
                None,
            );
            assert!(
                outcome.stdout.is_empty(),
                "{name} printed {:?}",
                String::from_utf8_lossy(&outcome.stdout)
            );
            assert!(outcome.attention.is_none(), "{name}");
        }
    }
}

#[test]
fn stubbed_surfaces_refuse_or_read_unreadable() {
    let composer = AgyAdapter.composer_policy().unwrap();
    assert_eq!(
        composer.capabilities(None),
        crate::harness::recipe::PokeCapabilities::NONE
    );
    assert_eq!(
        composer.read("> hello\n", Some(80)),
        crate::harness::composer::ComposerRead::Unreadable
    );
    let launch = AgyAdapter.launch_policy().unwrap();
    assert_eq!(launch.expected_host_kinds(), ["agy"]);
    // Launch is real (L5): `tests/harness/agy_launch.rs` covers the grammar and the plugin gate.
    launch.validate_native_argv(&[]).unwrap();
    assert_eq!(
        launch.compose_argv(vec![], vec![]).unwrap(),
        Vec::<String>::new()
    );
    assert!(
        launch
            .validate_native_argv(&["--print".to_owned()])
            .is_err()
    );
    // Setup, status and unsetup are real (L3): `tests/harness/agy_setup.rs` covers them.
    assert!(AgyAdapter.settings_kind().is_some());
    assert!(
        AgyAdapter
            .doctor_text(&Value::Null, crate::harness::adapter::DoctorTextForm::Debug)
            .is_empty()
    );
}

#[test]
fn registered_presentation_codecs_equal_actual_native_shapes() {
    use crate::harness::adapter::PresentationCodec;
    for (selector, step, overhead) in [
        (0, InjectionStep::SystemMessage, 54),
        (1, InjectionStep::UserMessage, 34),
    ] {
        assert_eq!(step.presentation_selector(), selector);
        let codec = AgyAdapter.presentation_codec(selector).unwrap();
        assert_eq!(codec, step.presentation_codec());
        assert_eq!(
            builtins()
                .by_id(builtins().agent("agy").unwrap())
                .unwrap()
                .presentation_codec(selector),
            Some(codec)
        );
        let PresentationCodec::JsonStringEnvelope { prefix, suffix } = codec;
        assert_eq!(prefix.len() + suffix.len(), overhead);
        for text in ["", "plain", "\0\u{1}\t\r\n\"\\/", "é🙂\u{2028}"] {
            let actual = serde_json::to_vec(&step.render(text)).unwrap();
            let mut descriptor = prefix.as_bytes().to_vec();
            descriptor.extend(serde_json::to_vec(text).unwrap());
            descriptor.extend(suffix.as_bytes());
            assert_eq!(descriptor, actual);
            assert_eq!(step.encode_presentation(text).unwrap(), actual);
        }
    }
    assert!(AgyAdapter.presentation_codec(2).is_none());
    assert!(AgyAdapter.presentation_codec(u8::MAX).is_none());
}

#[test]
fn native_presentation_encoder_caps_actual_final_bytes() {
    for step in [InjectionStep::SystemMessage, InjectionStep::UserMessage] {
        let crate::harness::adapter::PresentationCodec::JsonStringEnvelope { prefix, suffix } =
            step.presentation_codec();
        let available = 65_536 - prefix.len() - suffix.len() - 2;
        assert_eq!(
            step.encode_presentation(&"x".repeat(available))
                .unwrap()
                .len(),
            65_536
        );
        assert!(
            step.encode_presentation(&"x".repeat(available + 1))
                .is_none()
        );
        assert!(step.encode_presentation(&"\0".repeat(available)).is_none());
    }
}
