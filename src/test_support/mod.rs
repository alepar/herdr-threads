//! Test-only synchronization and fault fixtures; absent from ordinary builds.
pub mod attention_oracle;
pub mod counting_client;
pub mod failpoints;
pub mod history;
#[cfg(feature = "test-support")]
pub mod isolated_herdr;
pub mod isolation;
pub mod owner_watch;
#[cfg(feature = "test-support")]
pub mod search_barrier;
#[cfg(feature = "test-support")]
pub mod server_completion;
pub mod spawn;

/// Explicit authenticated-peer fixture for direct handler controls.
/// Production peers are constructed only from the socket kernel credential.
pub fn peer_identity(uid: u32) -> crate::protocol::authority::PeerIdentity {
    crate::protocol::authority::PeerIdentity::from_kernel(uid)
}

/// The `Unsupported` rejection a fixture returns for a port route it does not serve.
pub fn unserved(detail: &str) -> crate::protocol::results::ApiError {
    crate::protocol::results::ApiError::unsupported(detail)
}

/// Delegates to the actual generic doctor consumer with an owned environment.
#[cfg(feature = "test-support")]
pub fn adapter_smoke_doctor(
    parsed: &crate::cli::commands::ParsedCli,
    registry: &crate::harness::registry::Registry,
    environment: &crate::harness::adapter::SetupEnvironment,
    writer: &mut Vec<u8>,
) -> Result<(), crate::cli::RunError> {
    crate::cli::doctor::run_registered(parsed, registry, Some(environment), writer)
}

/// Thin parser delegate for injected registration collision/selector fixtures.
#[cfg(feature = "test-support")]
pub fn adapter_smoke_parse_hook(
    args: &[std::ffi::OsString],
    registry: &crate::harness::registry::Registry,
) -> Option<Result<crate::cli::hook::HookArgs, String>> {
    crate::cli::hook::parse_hook_argv_registered(args, registry)
}

/// Synthetic author fixture. It is compiled only with test-support and never
/// supplies native/runtime qualification or searches a user's configuration.
#[cfg(feature = "test-support")]
pub mod synthetic_fourth {
    use crate::harness::{adapter::*, context::EventKind, contract::*, evidence::*};
    use crate::protocol::time::CallBudget;
    use serde::Deserialize;
    use serde_json::json;
    use std::path::{Path, PathBuf};

    pub const ID: &str = "synthetic_fourth";
    pub const ROOT_INPUT: &str = "HT_SYNTHETIC_FOURTH_ROOT";
    pub static ADAPTER: Fourth = Fourth;
    pub struct Fourth;
    static METADATA: AdapterMetadata = AdapterMetadata {
        id: ID,
        display_label: "Synthetic fourth (test-support only)",
        context_spelling: "SyntheticFourth",
        context_aliases: &["SyntheticFourthAlias"],
        executable: ExecutableLookup::Path("ht-synthetic-fourth"),
        host_kinds: &["synthetic_fourth", "synthetic_fourth_alias"],
        setup_scopes: &[SetupScopeKind::ConfigRoot],
        budget: EventBudgetPolicy {
            lifecycle_ms: 4500,
            observer_ms: 1200,
        },
        runtime_sources: &["synthetic"],
    };
    const FIELDS: &[FieldSpec] = &[
        field("session_id", JsonType::String, true),
        field("event_id", JsonType::String, true),
        field("role", JsonType::String, true),
    ];
    static CONTRACT: HarnessContract = HarnessContract {
        harness: ID,
        discriminator: "event",
        events: &[
            EventContract {
                event: "SyntheticStart",
                class: EventClass::Lifecycle,
                fields: FIELDS,
            },
            EventContract {
                event: "SyntheticTool",
                class: EventClass::Tool,
                fields: FIELDS,
            },
            EventContract {
                event: "SyntheticObserver",
                class: EventClass::Other,
                fields: FIELDS,
            },
        ],
    };
    static DOMAINS: &[ContractDescriptor] = &[ContractDescriptor {
        domain_id: "synthetic",
        origin: EvidenceOrigin::NativePayload,
        events: &[
            EvidenceEvent {
                native_event: "SyntheticStart",
                milestone: Some("lifecycle"),
                always_send: true,
            },
            EvidenceEvent {
                native_event: "SyntheticTool",
                milestone: Some("tool"),
                always_send: false,
            },
            EvidenceEvent {
                native_event: "SyntheticObserver",
                milestone: None,
                always_send: false,
            },
        ],
        required_milestones: &["lifecycle", "tool"],
        qualifications: &[],
        holding: AttributionHolding::Never,
        resumed_unavailable_reason: None,
        domain: ContractDomain::Native,
        contract: &CONTRACT,
    }];
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Payload {
        event: String,
        session_id: String,
        event_id: String,
        role: String,
    }
    fn text(value: &str) -> bool {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    }
    fn root(environment: &SetupEnvironment) -> Result<PathBuf, SetupFailure> {
        let root = environment
            .declared
            .get(ROOT_INPUT)
            .map(PathBuf::from)
            .ok_or_else(|| {
                SetupFailure::Invalid("synthetic fourth: explicit fixture root unavailable".into())
            })?;
        if !root.is_absolute()
            || environment
                .home
                .as_ref()
                .is_none_or(|home| !root.starts_with(Path::new(home)))
        {
            return Err(SetupFailure::Invalid(
                "synthetic fourth: root must be inside declared fixture HOME".into(),
            ));
        }
        Ok(root)
    }
    fn scoped(
        scope: &ResolvedSetupScope,
        environment: &SetupEnvironment,
    ) -> Result<PathBuf, SetupFailure> {
        let root = root(environment)?;
        if *scope != ResolvedSetupScope::ConfigRoot(root.clone()) {
            return Err(SetupFailure::Invalid(
                "synthetic fourth: scope mismatch".into(),
            ));
        }
        Ok(root)
    }
    fn owned(environment: &SetupEnvironment) -> Result<Vec<u8>, SetupFailure> {
        serde_json::to_vec(
            &json!({"synthetic": true, "executable": environment.executable,
            "state": environment.state_dir, "host": environment.host_endpoint}),
        )
        .map_err(|error| SetupFailure::Invalid(error.to_string()))
    }
    fn local_status(request: &StatusRequest) -> Result<LocalSetupStatus, SetupFailure> {
        let root = scoped(&request.scope, &request.environment)?;
        let bytes = owned(&request.environment)?;
        let path = root.join("owned-hooks.json");
        let installed = match std::fs::read(&path) {
            Ok(current) if current == bytes => true,
            Ok(_) => {
                return Err(SetupFailure::Invalid(
                    "synthetic fourth: foreign or modified asset".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(SetupFailure::Io(error)),
        };
        let fingerprint = crate::harness::setup::fingerprint(&bytes);
        Ok(LocalSetupStatus {
            scope: request.scope.clone(),
            installed,
            enabled: Some(installed),
            admitted: None,
            observed: None,
            configured_hook: installed.then(|| crate::ports::ConfiguredHook {
                scope: "synthetic fixture".into(),
                path: path.display().to_string(),
                fingerprint: fingerprint.clone(),
            }),
            fingerprint: installed.then_some(fingerprint),
            diagnostics: vec![],
            repairs: vec![],
            projection: json!({"harness": ID, "synthetic": true, "installed": installed,
                "enabled": installed, "observed": "unknown", "runtime": null}),
        })
    }
    impl HarnessAdapter for Fourth {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            &METADATA
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            DOMAINS
        }
        fn hook_admission_policy(&self) -> HookAdmissionPolicy {
            HookAdmissionPolicy::RegisteredContract
        }
        fn setup_environment_inputs(&self) -> &'static [&'static str] {
            &[ROOT_INPUT]
        }
        fn resolve_setup_scope(
            &self,
            request: &SetupScopeRequest,
            environment: &SetupEnvironment,
        ) -> Result<ResolvedSetupScope, SetupFailure> {
            if *request != SetupScopeRequest::Default {
                return Err(SetupFailure::Unsupported(UnsupportedOperation {
                    adapter: ID,
                    operation: "named profiles",
                }));
            }
            root(environment).map(ResolvedSetupScope::ConfigRoot)
        }
        fn observe_daemon(
            &self,
            env: &InstallEnvironment,
            budget: &CallBudget,
        ) -> DaemonObservation {
            // Without an explicit fixture directory the stand-in is absent, not
            // refused: it must not degrade daemon health in test-support builds.
            if env.config_root.is_none() {
                return DaemonObservation {
                    status: HarnessStatus::NotInstalled(
                        "synthetic fourth: no explicit fixture binary directory".into(),
                    ),
                    ..Default::default()
                };
            }
            observe_daemon_default(self, env, budget)
        }
        fn observe_install(&self, env: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            // No generic PATH/default-config guess. Tests explicitly pass a
            // fixture binary directory; the stand-in is never executed here.
            let Some(root) = env.config_root.as_ref() else {
                return InstallObservation::Unavailable {
                    diagnostic: "synthetic fourth: explicit fixture binary directory unavailable"
                        .into(),
                };
            };
            let binary = root.join("ht-synthetic-fourth");
            if root.is_absolute()
                && std::fs::read(&binary).ok().as_deref() == Some(b"synthetic fourth fixture\n")
            {
                InstallObservation::ExecutableAvailable { binary }
            } else {
                InstallObservation::Unavailable {
                    diagnostic: "synthetic fourth: fixture binary unavailable".into(),
                }
            }
        }
        fn admit(&self, request: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            if matches!(
                request.installed,
                InstallObservation::NotRequested | InstallObservation::ExecutableAvailable { .. }
            ) {
                AdmissionDecision::ContractDeclared {
                    state: (),
                    recipe: "synthetic declared fixture contract",
                }
            } else {
                AdmissionDecision::Refused {
                    diagnostic: "synthetic fourth: explicit fixture observation required".into(),
                }
            }
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            Ladder::Admitted
        }
        fn classify(&self, input: &HookInput) -> ContractObservation {
            ContractObservation {
                domain: ContractDomain::Native,
                classification: classify(
                    &CONTRACT,
                    input.registered_event.as_deref(),
                    &input.bytes,
                ),
            }
        }
        fn decode(&self, _: &(), input: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            if input.bytes.len() > MAX_PAYLOAD {
                return Err(DecodeFailure::Invalid("synthetic payload too large".into()));
            }
            let payload: Payload = serde_json::from_slice(&input.bytes)
                .map_err(|_| DecodeFailure::Invalid("synthetic payload shape".into()))?;
            if !text(&payload.session_id)
                || !text(&payload.event_id)
                || input
                    .registered_event
                    .as_deref()
                    .is_some_and(|event| event != payload.event)
            {
                return Err(DecodeFailure::Invalid(
                    "synthetic event/session mismatch".into(),
                ));
            }
            let role = match payload.role.as_str() {
                "top_level" => EventRole::TopLevel,
                "child" => EventRole::Subagent,
                "unknown" => EventRole::Unknown,
                _ => return Err(DecodeFailure::Invalid("synthetic role".into())),
            };
            let intent = match payload.event.as_str() {
                "SyntheticStart" => EventIntent::Lifecycle(EventKind::Startup),
                "SyntheticTool" => EventIntent::Current,
                "SyntheticObserver" => EventIntent::Observer,
                _ => return Err(DecodeFailure::Invalid("synthetic event".into())),
            };
            let observer = matches!(intent, EventIntent::Observer);
            Ok(DecodedEvent {
                harness: crate::harness::registry::builtins()
                    .agent(ID)
                    .map_err(|e| DecodeFailure::Invalid(e.to_string()))?,
                role,
                native_session: Some(payload.session_id),
                event_id: payload.event_id,
                intent,
                metadata: EventMetadata {
                    skill_pointer: false,
                    callback_deadline: None,
                    context_source: payload.event.clone(),
                    capability: crate::harness::Capability::ContractValidatedInput,
                    domain: ContractDomain::Native,
                    native_event: payload.event,
                    shape_fields: vec![],
                },
                delivery: if observer {
                    DeliveryEligibility::ObserverOnly
                } else {
                    DeliveryEligibility::Context
                },
                runtime: RuntimeAttribution::Unavailable {
                    diagnostic: "synthetic_runtime_unavailable".into(),
                },
            })
        }
        fn encode(
            &self,
            _: &(),
            event: &DecodedEvent,
            offer: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            let context = (!matches!(event.role, EventRole::Unknown)
                && !matches!(event.intent, EventIntent::Observer)
                && !offer.fixed_guidance.is_empty())
            .then_some(&offer.fixed_guidance);
            let bytes = serde_json::to_vec(&json!({"synthetic_context": context}))
                .map_err(|e| EncodeFailure::Invalid(e.to_string()))?;
            Ok(if matches!(event.intent, EventIntent::Observer) {
                EncodedOutput::ObserverOnly { bytes }
            } else {
                EncodedOutput::ContextBearing { bytes }
            })
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            RuntimeAttribution::Unavailable {
                diagnostic: "synthetic_runtime_unavailable".into(),
            }
        }
        fn setup(
            &self,
            request: &SetupRequest,
            _: &CallBudget,
        ) -> Result<SetupOutcome, SetupFailure> {
            let root = scoped(&request.scope, &request.environment)?;
            if request.native_binary.as_ref().is_none_or(|binary| {
                binary
                    .file_name()
                    .is_none_or(|name| name != "ht-synthetic-fourth")
            }) {
                return Err(SetupFailure::Invalid(
                    "synthetic fourth: explicit fixture binary required".into(),
                ));
            }
            let status = local_status(&StatusRequest {
                scope: request.scope.clone(),
                environment: request.environment.clone(),
                native_binary: request.native_binary.clone(),
            })?;
            std::fs::create_dir_all(root.clone()).map_err(SetupFailure::Io)?;
            std::fs::write(root.join("owned-hooks.json"), owned(&request.environment)?)
                .map_err(SetupFailure::Io)?;
            Ok(SetupOutcome {
                actions: vec![if status.installed {
                    SetupAction::Unchanged
                } else {
                    SetupAction::InstalledOwned
                }],
                diagnostic: String::new(),
                diagnostics: vec![],
                projection: json!({"harness": ID, "synthetic": true, "action": if status.installed { "unchanged" } else { "installed" }, "runtime": null}),
            })
        }
        fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
            match local_status(request) {
                Ok(status) => SetupStatus::Detailed(Box::new(status)),
                Err(error) => SetupStatus::Failed(error),
            }
        }
        fn unsetup(
            &self,
            request: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            let root = scoped(&request.scope, &request.environment)?;
            let status = local_status(&StatusRequest {
                scope: request.scope.clone(),
                environment: request.environment.clone(),
                native_binary: None,
            })?;
            if status.installed {
                std::fs::remove_file(root.join("owned-hooks.json")).map_err(SetupFailure::Io)?;
            }
            Ok(RemovalOutcome {
                actions: vec![if status.installed {
                    SetupAction::RemovedOwned
                } else {
                    SetupAction::Unchanged
                }],
                diagnostic: String::new(),
                residue: vec![],
                diagnostics: vec![],
                projection: json!({"harness": ID, "synthetic": true, "action": if status.installed { "removed" } else { "not_installed" }}),
            })
        }
        fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
            Some(self)
        }
    }
    impl LaunchPolicy for Fourth {
        fn resolve_scope(
            &self,
            request: &LaunchRequest,
            _: &dyn crate::harness::launch::CodexShellProbe,
            _: &CallBudget,
        ) -> Result<LaunchScope, crate::protocol::results::ApiError> {
            let root = root(&request.environment).map_err(|error| {
                crate::protocol::results::ApiError::invalid_request(format!("{error:?}"))
            })?;
            Ok(LaunchScope {
                setup: ResolvedSetupScope::ConfigRoot(root),
                working_directory: request.environment.cwd.clone(),
                config_source: "explicit_synthetic_fixture",
            })
        }
        fn validate_native_argv(
            &self,
            argv: &[String],
        ) -> Result<(), crate::protocol::results::ApiError> {
            if argv.iter().any(|word| word != "synthetic-prompt") {
                Err(crate::protocol::results::ApiError::invalid_request(
                    "synthetic fourth: unsupported argv",
                ))
            } else {
                Ok(())
            }
        }
        fn compose_argv(
            &self,
            caller: Vec<String>,
            owned: Vec<String>,
        ) -> Result<Vec<String>, crate::protocol::results::ApiError> {
            self.validate_native_argv(&caller)?;
            Ok([vec!["--synthetic-only".into()], owned, caller].concat())
        }
        fn prepare_launch(
            &self,
            request: &LaunchRequest,
            scope: &LaunchScope,
            _: &crate::harness::registry::AdmittedHandle,
            status: &LocalSetupStatus,
            _: &dyn crate::harness::launch::CodexShellProbe,
            _: &CallBudget,
        ) -> Result<LaunchPreparation, crate::protocol::results::ApiError> {
            Ok(LaunchPreparation {
                argv: self.compose_argv(request.argv.clone(), vec![])?,
                hook: crate::harness::launch::owned_launch_hook(status)?,
                working_directory: scope.working_directory.clone(),
                environment_overrides: Default::default(),
                report: json!({"synthetic": true, "runtime": null}),
            })
        }
        fn configuration_fingerprint(
            &self,
            request: &LaunchRequest,
            scope: &LaunchScope,
        ) -> Result<String, crate::protocol::results::ApiError> {
            let root = scoped(&scope.setup, &request.environment).map_err(|error| {
                crate::protocol::results::ApiError::invalid_request(format!("{error:?}"))
            })?;
            std::fs::read(root.join("owned-hooks.json"))
                .map(|bytes| crate::harness::setup::fingerprint(&bytes))
                .map_err(|_| {
                    crate::protocol::results::ApiError::invalid_request(
                        "synthetic fourth: missing owned hooks",
                    )
                })
        }
        fn expected_host_kinds(&self) -> &'static [&'static str] {
            METADATA.host_kinds
        }
    }
}

/// Explicit `LocalService` bodies for the routes a fixture does not serve,
/// named one per route: `unserved_local_service_routes!(service_control, ...)`.
/// `handle_with_output` serves the default output through `handle`.
#[macro_export]
macro_rules! unserved_local_service_routes {
    () => {};
    (service_control $(, $rest:ident)* $(,)?) => {
        fn service_control(
            &self,
            _: $crate::protocol::commands::Command,
            _: $crate::protocol::authority::PeerIdentity,
            _: &str,
            _: &str,
            _: &$crate::service::live_gate::LiveServiceGate,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service recovery control is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (audit_service_disconnect $(, $rest:ident)* $(,)?) => {
        fn audit_service_disconnect(
            &self,
            _: &str,
            _: u64,
            _: $crate::protocol::authority::PeerIdentity,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<(), $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service recovery audit is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (service_operation $(, $rest:ident)* $(,)?) => {
        fn service_operation(
            &self,
            _: $crate::protocol::service::ServiceOperation,
            _: &$crate::ports::ServiceConnectionAuthority,
            _: &dyn $crate::ports::ServiceAuthorityGate,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::service::ServiceResult, $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service operation route is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (handle_with_output $(, $rest:ident)* $(,)?) => {
        fn handle_with_output(
            &self,
            command: $crate::protocol::commands::Command,
            peer: $crate::protocol::authority::PeerIdentity,
            budget: &$crate::protocol::time::CallBudget,
            output: &$crate::protocol::output::OutputSpec,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            if *output != $crate::protocol::output::OutputSpec::default() {
                return Err($crate::test_support::unserved(
                    "selected output is unavailable in this service",
                ));
            }
            self.handle(command, peer, budget)
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
}

/// Explicit `LocalClient::call_with_output` for a fixture that serves only the
/// default output through `call`.
#[macro_export]
macro_rules! default_output_local_client {
    () => {
        fn call_with_output(
            &self,
            command: $crate::protocol::commands::Command,
            output: &$crate::protocol::output::OutputSpec,
            budget: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            if *output != $crate::protocol::output::OutputSpec::default() {
                return Err($crate::protocol::results::ApiError::invalid_request(
                    "selected output unsupported by this client",
                ));
            }
            self.call(command, budget)
        }
    };
}

/// Explicit `DeadlinePort` durable-work methods for a fixture with no work jobs.
#[macro_export]
macro_rules! no_durable_work {
    () => {
        fn pending_work(
            &self,
            _: $crate::protocol::pagination::PageRequest,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<
            $crate::protocol::pagination::Page<$crate::ports::WorkCandidate>,
            $crate::protocol::results::ApiError,
        > {
            Ok($crate::protocol::pagination::Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: $crate::protocol::pagination::StopReason::Complete,
                consistency: $crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            _: &str,
            _: $crate::ports::DurableWorkAdmission,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::ports::WorkProgress, $crate::protocol::results::ApiError> {
            unreachable!("advance_work requires a discovered work job")
        }
    };
}

/// Construct sealed bootstrap attachment evidence from a qualified test current
/// observation and explicit same-response scope. Absent in ordinary builds.
pub fn bootstrap_attachment_guard(
    request: &crate::protocol::commands::ResolveSeat,
    observation: crate::ports::HostObservation,
    workspace: crate::protocol::ids::HostTargetId,
    tab: crate::protocol::ids::HostTargetId,
    admission: &crate::ports::HostObservationAdmission,
    witness: crate::host::continuity::LocalEndpointWitness,
) -> Result<crate::ports::BootstrapAttachmentGuard, &'static str> {
    let pane =
        crate::ports::BootstrapPaneObservation::try_new(observation, workspace, tab, witness)?;
    crate::ports::BootstrapAttachmentGuard::try_new(request, pane, admission)
}
#[cfg(feature = "test-support")]
pub mod archival_composer_fixture;
