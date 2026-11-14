//! The opencode registry row (opencode design D2, D3, D10): metadata, host kind, the agent
//! marker moved off the unsupported list, the caller shape, versionless admission, the output
//! policy, the idle-wake classification of the first `Channel` harness with no composer policy
//! (review M6) and the setup stubs left for ht-vxdp.3. Included from
//! `src/harness/opencode/mod.rs`.
use super::*;
use crate::harness::registry::{IdleWakeOn, builtins};
use crate::ports::{ComposerTraits, WakeCapability};

#[test]
fn metadata_and_registry_lookups() {
    let registry = builtins();
    let id = registry.agent("opencode").expect("registered");
    let row = registry.by_id(id).unwrap();
    let m = row.metadata();
    assert_eq!(m.id, "opencode");
    assert_eq!(m.display_label, "opencode");
    assert!(matches!(m.executable, ExecutableLookup::Path("opencode")));
    assert_eq!(m.host_kinds, ["opencode"]);
    assert_eq!(
        registry.by_host_kind("opencode").map(|r| r.metadata().id),
        Some("opencode")
    );
    assert_eq!(
        registry
            .by_context_spelling("OpenCode")
            .ok()
            .map(|h| format!("{h:?}")),
        registry
            .by_context_spelling("opencode")
            .ok()
            .map(|h| format!("{h:?}"))
    );
}

#[test]
fn the_opencode_marker_is_the_adapter_s_not_an_unsupported_harness_s() {
    let registry = builtins();
    assert!(
        !registry
            .unsupported_markers()
            .iter()
            .any(|marker| marker.name == "OPENCODE")
    );
    assert_eq!(
        METADATA.agent_env_markers,
        [AgentEnvMarker {
            name: "OPENCODE",
            person_guard: true,
        }]
    );
    assert!(
        registry
            .person_guard_markers()
            .any(|marker| marker.name == "OPENCODE")
    );
}

#[test]
fn the_caller_shape_is_the_plugin_variables_and_opencode_pid() {
    let shape = OpencodeAdapter.caller_env().expect("declared");
    assert_eq!(shape.process_names, ["opencode"]);
    assert_eq!(shape.session_var, "HERDR_THREADS_OPENCODE_SESSION");
    assert_eq!(shape.pid_var, Some("OPENCODE_PID"));
    assert!(shape.role.is_some());
    assert!(shape.paneless.is_none());
    assert!(shape.process_match.is_none());
    // One process serves one pane.
    assert!(!OpencodeAdapter.binding_policy().process_spans_seats);
    assert!(
        builtins()
            .caller_shapes()
            .any(|(id, shape)| id == "opencode" && shape.session_var == witness::SESSION_VAR)
    );
}

#[test]
fn admission_is_versionless_and_contract_declared() {
    let budget = CallBudget {
        deadline: crate::protocol::time::MonoInstant(100),
        cancellation: Default::default(),
    };
    assert_eq!(
        OpencodeAdapter.hook_admission_policy(),
        HookAdmissionPolicy::RegisteredContract
    );
    let admit = |installed| {
        OpencodeAdapter.admit(
            &AdmissionRequest {
                installed,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget,
        )
    };
    assert!(matches!(
        admit(InstallObservation::NotRequested),
        AdmissionDecision::ContractDeclared { recipe: RECIPE, .. }
    ));
    assert!(matches!(
        admit(InstallObservation::Unavailable {
            diagnostic: "x".into()
        }),
        AdmissionDecision::Refused { .. }
    ));
    for version in ["1.18.1", "999.0.0"] {
        assert_eq!(
            OpencodeAdapter.version_ladder(
                &RuntimeIdentity::stable_release(version, "native_transcript").unwrap()
            ),
            Ladder::Admitted
        );
    }
    let observed = OpencodeAdapter.observe_daemon(
        &InstallEnvironment {
            clock: std::sync::Arc::new(crate::app::SystemClock::new()),
            path: Some(std::ffi::OsString::new()),
            config_root: None,
            state_dir: None,
        },
        &budget,
    );
    assert!(matches!(observed.status, HarnessStatus::NotInstalled(_)));
    assert_eq!(
        observed.receipt_basis.as_deref(),
        Some(crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE)
    );
}

/// The command guidance rides lifecycle check-ins and the compact re-entry only.
#[test]
fn output_policy_carries_the_command_guidance() {
    let policy = OpencodeAdapter.output_policy();
    assert_eq!(policy.extra_guidance, OPENCODE_COMMAND_GUIDANCE);
    assert_eq!(OPENCODE_COMMAND_GUIDANCE.len(), 231);
    assert!(OPENCODE_COMMAND_GUIDANCE.ends_with('\n'));
    assert!(policy.empty_lifecycle);
    assert!(policy.session_start_hint);
    assert!(policy.child_requires_endpoint);
    assert!(!policy.guidance_every_turn);
}

/// Review M6: opencode is the first `Channel` harness with no composer policy. It has an idle
/// wake (its channel), but only a live channel wakes it, on every host: with the channel down
/// there is no composer to fall back on, so the classification is `None` (the sender's
/// `wake=no_wake`, design D8).
#[test]
fn opencode_wakes_only_through_a_live_channel() {
    let registry = builtins();
    assert_eq!(registry.has_idle_wake("opencode"), Some(true));
    let tui = WakeCapability::Composer(ComposerTraits::HERDR_TUI);
    for wake in [tui, WakeCapability::None] {
        assert_eq!(
            registry.idle_wake_on("opencode", wake, true),
            Some(IdleWakeOn::Channel)
        );
        assert_eq!(
            registry.idle_wake_on("opencode", wake, false),
            Some(IdleWakeOn::None)
        );
    }
}

/// ht-vxdp.3 fills these in; until then they are refused honestly and `setup` with no harness
/// skips opencode.
#[test]
fn setup_operations_are_unsupported_until_the_installer_lands() {
    use std::path::PathBuf;
    let budget = CallBudget {
        deadline: crate::protocol::time::MonoInstant(100),
        cancellation: Default::default(),
    };
    let scope = ResolvedSetupScope::ConfigRoot(PathBuf::from("unused"));
    assert!(METADATA.setup_scopes.is_empty());
    assert!(matches!(
        OpencodeAdapter.status(
            &StatusRequest {
                scope: scope.clone(),
                environment: SetupEnvironment::default(),
                native_binary: None,
            },
            &budget
        ),
        SetupStatus::Unsupported(_)
    ));
    assert!(matches!(
        OpencodeAdapter.setup(
            &SetupRequest {
                scope: scope.clone(),
                environment: SetupEnvironment::default(),
                native_binary: None,
                executable: PathBuf::from("unused"),
                options: Default::default(),
            },
            &budget
        ),
        Err(SetupFailure::Unsupported(_))
    ));
    assert!(matches!(
        OpencodeAdapter.unsetup(
            &UnsetupRequest {
                scope,
                environment: SetupEnvironment::default(),
            },
            &budget
        ),
        Err(SetupFailure::Unsupported(_))
    ));
}
