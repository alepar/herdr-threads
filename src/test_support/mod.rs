//! Test-only synchronization and fault fixtures; absent from ordinary builds.
pub mod attention_oracle;
pub mod counting_client;
pub mod failpoints;
pub mod harness_shim;
pub mod history;
#[cfg(feature = "test-support")]
pub mod isolated_herdr;
pub mod isolation;
#[cfg(feature = "test-support")]
pub mod journal_oracle;
pub mod owner_watch;
#[cfg(feature = "test-support")]
pub mod search_barrier;
#[cfg(feature = "test-support")]
pub mod server_completion;
pub mod spawn;

use crate::{
    node::{NodeUuid, tag_for},
    store::{
        connection::StoreContext,
        schema::{NodeSeed, UNNAMED_HOST_ENDPOINT},
    },
};
use rusqlite::Connection;
use std::sync::{Arc, OnceLock};

/// The ed25519 seed of the node every fixture store is created with.
pub const TEST_NODE_SEED: [u8; 32] = [1; 32];

/// The node a test store belongs to: the uuid of [`TEST_NODE_SEED`]'s key,
/// its tag at `k = 0`. A `StoreContext` built in a test build is created with
/// it, so a fixture's `instance` string is [`test_node`].
pub fn test_node_seed() -> NodeSeed {
    let key = ed25519_dalek::SigningKey::from_bytes(&TEST_NODE_SEED);
    let uuid = NodeUuid::from_public_key(&key.verifying_key().to_bytes());
    NodeSeed {
        uuid,
        tag: tag_for(uuid, 0),
        tag_k: 0,
    }
}

/// The seed of an arbitrary node, for a fixture whose store belongs to a node
/// other than the fixed test node (a daemon fixture that owns its own key).
pub fn node_seed_for(uuid: impl std::fmt::Display) -> NodeSeed {
    let uuid: NodeUuid = uuid.to_string().parse().expect("a 32-hex node uuid");
    NodeSeed {
        uuid,
        tag: tag_for(uuid, 0),
        tag_k: 0,
    }
}

/// [`test_node_seed`]'s uuid in its 32-hex form: the `instance` string and the
/// `instance_id` value of fixture rows.
pub fn test_node() -> String {
    test_node_seed().uuid.to_string()
}

/// A fixed canonical host locator for fixture claims: never touches the filesystem. It is the
/// `host_endpoint` the handoff fixtures' namespaces carry, so a fixture claim and its namespace
/// agree.
pub fn test_host_locator() -> crate::protocol::ids::HostLocator {
    crate::protocol::ids::HostLocator::parse("/host.sock").expect("canonical locator")
}

/// The endpoint of the host row a fixture attaches; it is also the endpoint a
/// daemon records for a host composed without a socket path, so a fixture row
/// is the row such a daemon attaches.
pub const TEST_HOST_ENDPOINT: &str = UNNAMED_HOST_ENDPOINT;

fn upsert_test_host(conn: &Connection) -> i64 {
    crate::store::connection::upsert_host(conn, TEST_HOST_ENDPOINT, 0)
        .expect("attach the test host row")
}

/// The id of the fixture host row: the first `herdr_hosts` row a fresh store
/// gets, which every fixture that attaches [`TEST_HOST_ENDPOINT`] first uses.
/// Observation entry points take it as their explicit host.
pub fn test_host_id() -> crate::ports::HostId {
    crate::ports::HostId::from_row(1)
}

/// Attaches the fixture Herdr host to a store: upserts its `herdr_hosts` row
/// through `conn`, makes it `ctx`'s primary host and binds `conn` to it.
/// Idempotent. Returns the host id.
pub fn attach_test_host(ctx: &StoreContext, conn: &Connection) -> i64 {
    let host_id = upsert_test_host(conn);
    ctx.set_primary_host(host_id);
    ctx.bind_connection(conn).expect("bind the primary host");
    host_id
}

/// [`attach_test_host`] for a bare connection that has no `StoreContext`
/// (`schema::initialize` on `:memory:`): the connection alone is bound.
pub fn attach_test_host_to_connection(conn: &Connection) -> i64 {
    let host_id = upsert_test_host(conn);
    let primary = Arc::new(OnceLock::new());
    let _ = primary.set(host_id);
    crate::store::connection::register_primary_host(conn, primary).expect("bind the primary host");
    host_id
}

/// Opens a raw connection to an existing store file the way a fixture reads or
/// repairs it, bound to the fixture host (see [`attach_test_host`]) so store
/// functions that read Herdr-scope columns run on it. The host row is not
/// created here: a store opened through `StoreContext::with_test_host` has it.
pub fn open_test_connection(path: &std::path::Path) -> Connection {
    let conn = Connection::open(path).expect("open the store file");
    bind_test_host(&conn);
    conn
}

/// Binds a connection whose database already holds the fixture host row (an
/// opened store file, a deserialized image) to that host, so store functions
/// that read Herdr-scope columns run on it.
pub fn bind_test_host(conn: &Connection) {
    let host_id: i64 = conn
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE endpoint=?1",
            [TEST_HOST_ENDPOINT],
            |row| row.get(0),
        )
        .unwrap_or(1);
    let primary = Arc::new(OnceLock::new());
    let _ = primary.set(host_id);
    crate::store::connection::register_primary_host(conn, primary).expect("bind the primary host");
}

impl StoreContext {
    /// Test hook: attaches the fixture Herdr host through a short-lived writer
    /// (see [`attach_test_host`]).
    pub fn attach_test_host(&self) -> i64 {
        let conn = self
            .open_writer()
            .expect("open a writer to attach the host");
        attach_test_host(self, &conn)
    }
}

/// A short real temp root for fixtures that bind Unix sockets (macOS caps
/// socket paths at 104 bytes and its default TMPDIR is long). macOS uses the
/// canonical `/private/tmp`, so paths match what the kernel reports back.
pub const SHORT_TMP: &str = if cfg!(target_os = "macos") {
    "/private/tmp"
} else {
    "/tmp"
};

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
        agent_env_markers: &[],
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
    /// Declares the hook-pull delivery policy so daemon and routing tests have a
    /// non-Claude pull harness: `(HookPull, cooperative_hook_delivery, Native)`.
    struct FourthDelivery;
    impl DeliveryPolicy for FourthDelivery {
        fn mechanism(&self) -> DeliveryMechanism {
            DeliveryMechanism::HookPull
        }
        fn provenance(&self) -> &'static str {
            crate::protocol::authority::COOPERATIVE_HOOK_DELIVERY_PROVENANCE
        }
        fn idle_wake(&self) -> IdleWake {
            IdleWake::Native
        }
    }
    static DELIVERY: FourthDelivery = FourthDelivery;
    impl HarnessAdapter for Fourth {
        fn presentation_codec(
            &self,
            selector: u8,
        ) -> Option<crate::harness::adapter::PresentationCodec> {
            (selector == 0).then_some(
                crate::harness::adapter::PresentationCodec::JsonStringEnvelope {
                    prefix: "",
                    suffix: "",
                },
            )
        }
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            &METADATA
        }
        fn delivery_policy(&self) -> Option<&dyn DeliveryPolicy> {
            Some(&DELIVERY)
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
                    enrollment_witness: None,
                    skill_pointer: false,
                    callback_deadline: None,
                    context_source: payload.event.clone(),
                    capability: crate::harness::Capability::ContractValidatedInput,
                    domain: ContractDomain::Native,
                    native_event: payload.event,
                    shape_fields: vec![],
                    agent_id: None,
                },
                delivery: if observer {
                    DeliveryEligibility::ObserverOnly
                } else {
                    DeliveryEligibility::Context
                },
                runtime: RuntimeAttribution::Unavailable {
                    diagnostic: "synthetic_runtime_unavailable".into(),
                },
                binding_event: None,
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
            _: &dyn crate::harness::launch::PaneShellProbe,
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
            // The fixture's one native prompt word, or the generated handoff bootstrap
            // prompt a bootstrap plan appends to the caller arguments.
            if argv.iter().any(|word| {
                word != "synthetic-prompt"
                    && !word.starts_with("Expected handoff command routing (JSON data): ")
            }) {
                Err(crate::protocol::results::ApiError::invalid_request(
                    "synthetic fourth: unsupported argv",
                ))
            } else {
                Ok(())
            }
        }
        fn prepare_startup_input(
            &self,
            caller: &[String],
            _: &crate::harness::adapter::StartupInputSpec,
        ) -> Result<
            Option<crate::harness::adapter::StartupInputTemplate>,
            crate::protocol::results::ApiError,
        > {
            self.validate_native_argv(caller)?;
            Ok(Some(
                crate::harness::adapter::StartupInputTemplate::positional(caller.len()),
            ))
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
            _: &dyn crate::harness::launch::PaneShellProbe,
            _: &CallBudget,
        ) -> Result<LaunchPreparation, crate::protocol::results::ApiError> {
            Ok(LaunchPreparation {
                argv: self.compose_argv(request.argv.clone(), vec![])?,
                hook: Some(crate::harness::launch::owned_launch_hook(status)?),
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

/// Write the `node.json` hint for `node_uuid` (tag derived at `tag_k` 0): what
/// the elected daemon publishes from its loaded key, for fixtures that make a
/// node visible to the CLI without running a daemon.
pub fn write_node_hint(paths: &crate::daemon::paths::NodePaths, node_uuid: crate::node::NodeUuid) {
    paths.prepare_node_dir().expect("node dir");
    crate::node::json::write_if_differs(
        &paths.node_json_path,
        &crate::node::NodeJson::new(node_uuid, crate::node::tag_for(node_uuid, 0), 0, 0),
    )
    .expect("node hint");
}

/// Load (or create) the node key under `paths` and write its `node.json`
/// hint, as the elected daemon does before it publishes anything. Returns the
/// node's uuid; a daemon started later on the same state directory loads the
/// same key.
pub fn seed_node_identity(
    paths: &crate::daemon::paths::NodePaths,
) -> std::io::Result<crate::node::NodeUuid> {
    paths.prepare_node_dir()?;
    let identity = crate::node::NodeIdentity::load_or_create(paths, false)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    crate::node::json::write_if_differs(
        &paths.node_json_path,
        &crate::node::NodeJson::new(identity.uuid(), identity.tag(), identity.tag_k(), 0),
    )?;
    Ok(identity.uuid())
}

/// [`NodePaths::resolve`] for a state directory whose node key and
/// `node.json` hint are already in place, as when the elected daemon starts.
pub fn resolve_seeded(state_dir: &std::path::Path) -> crate::daemon::paths::NodePaths {
    let paths = crate::daemon::paths::NodePaths::resolve(state_dir).expect("node paths");
    seed_node_identity(&paths).expect("seed node identity");
    paths
}

/// A detached `daemon run --state-dir <state>` child of the real binary,
/// started the way the CLI's own ensure starts it, but owned by the test: the
/// 5-argument form is store-only (no Herdr server attached unless the store
/// remembers a live one), the 7-argument form adds `--host-endpoint <hint>`.
/// stdout is discarded; pre-election stderr (a start failure's reason) goes to
/// `stderr_to`. The child leads its own process group and is stopped when the
/// returned handle drops (`spawn::OwnedChild`); `isolation` supplies the
/// scrubbed, test-owned environment (HOME and XDG under its root).
pub fn spawn_node_daemon(
    isolation: &isolation::TestIsolation,
    binary: impl AsRef<std::ffi::OsStr>,
    state_dir: &std::path::Path,
    host_hint: Option<&std::path::Path>,
    stderr_to: &std::path::Path,
) -> std::io::Result<spawn::OwnedChild> {
    use spawn::SpawnOwned;
    let mut command = isolation.command(binary);
    command
        .args(["daemon", "run", "--state-dir"])
        .arg(state_dir);
    if let Some(hint) = host_hint {
        command.arg("--host-endpoint").arg(hint);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(stderr_to)?)
        .spawn_owned()
}
