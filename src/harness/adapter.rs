//! In-memory author contract. Observations grant no seat or receipt authority.
use crate::protocol::time::CallBudget;
use std::ffi::OsString;

pub trait HarnessAdapter: Send + Sync + 'static {
    type Admission: Send + Sync + 'static;
    fn metadata(&self) -> &'static AdapterMetadata;
    fn receipt_admission_summary(&self) -> Option<String> {
        None
    }
    fn output_policy(&self) -> OutputPolicy {
        OutputPolicy::default()
    }
    /// Explicit compatibility projection; rich domains do not imply a legacy scalar.
    fn legacy_contract_id(&self) -> Option<String> {
        None
    }
    fn contracts(&self) -> &'static [ContractDescriptor];
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation;
    /// None means observation reuse is unsafe. Implementers include every observed
    /// input (profile/config/assets as applicable), not merely the executable.
    fn observation_fingerprint(&self, _: &InstallEnvironment) -> Option<String> {
        None
    }
    fn observe_daemon(&self, env: &InstallEnvironment, budget: &CallBudget) -> DaemonObservation {
        let installed = self.observe_install(env, budget);
        let identity = match &installed {
            InstallObservation::Available { identity, .. } => Some(identity.clone()),
            InstallObservation::CodexWitness(version) => {
                RuntimeIdentity::stable_release(version.as_str(), "installed_probe").ok()
            }
            _ => None,
        };
        let status = match &installed {
            InstallObservation::Unavailable { diagnostic } => {
                HarnessStatus::Refused(diagnostic.clone())
            }
            InstallObservation::Unsupported(operation) => {
                HarnessStatus::Refused(operation.to_string())
            }
            _ => match self.admit(
                &AdmissionRequest {
                    installed,
                    input: None,
                    runtime_candidate: None,
                },
                budget,
            ) {
                AdmissionDecision::Listed { recipe, .. } => HarnessStatus::Cooperative {
                    detail: recipe.into(),
                    live_unverified: false,
                },
                AdmissionDecision::SchemaMatched { recipe, .. } => HarnessStatus::Cooperative {
                    detail: recipe.into(),
                    live_unverified: true,
                },
                AdmissionDecision::Optimistic { diagnostic, .. } => {
                    HarnessStatus::Optimistic(diagnostic)
                }
                AdmissionDecision::Refused { diagnostic } => HarnessStatus::Refused(diagnostic),
            },
        };
        DaemonObservation {
            status,
            identity,
            ..Default::default()
        }
    }
    fn admit(
        &self,
        request: &AdmissionRequest,
        budget: &CallBudget,
    ) -> AdmissionDecision<Self::Admission>;
    fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder;
    fn classify(&self, input: &HookInput) -> ContractObservation;
    fn decode(
        &self,
        admitted: &Self::Admission,
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure>;
    fn encode(
        &self,
        admitted: &Self::Admission,
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure>;
    fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution;
    fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure>;
    fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus;
    /// Optional frozen doctor projection and explicitly safe owned-local repair policy.
    /// Setup status alone never authorizes native enablement, trust or removal.
    fn doctor_projection(
        &self,
        _: &StatusRequest,
        _: &serde_json::Value,
        _: &CallBudget,
    ) -> Option<DoctorProjection> {
        None
    }

    fn unsetup(
        &self,
        request: &UnsetupRequest,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure>;
    fn setup_options(&self) -> &'static [SetupOption] {
        &[]
    }
    fn setup_environment_inputs(&self) -> &'static [&'static str] {
        &[]
    }
    fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        environment: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        match request {
            SetupScopeRequest::Default => environment
                .config_roots
                .get(self.metadata().id)
                .cloned()
                .map(ResolvedSetupScope::ConfigRoot)
                .ok_or_else(|| {
                    SetupFailure::Invalid(format!(
                        "{}: config root unavailable",
                        self.metadata().id
                    ))
                }),
            SetupScopeRequest::Profile(_) => Err(SetupFailure::Invalid(format!(
                "{}: named profile is unsupported",
                self.metadata().id
            ))),
        }
    }
    fn settle_setup_consent(
        &self,
        _: &SetupEnvironment,
        _: &mut serde_json::Value,
        _: &mut dyn std::io::BufRead,
        _: &mut dyn std::io::Write,
    ) -> Result<(), SetupFailure> {
        Ok(())
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        None
    }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        None
    }
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> {
        None
    }
}

pub struct AdapterMetadata {
    pub id: &'static str,
    pub display_label: &'static str,
    pub context_spelling: &'static str,
    pub context_aliases: &'static [&'static str],
    pub executable: ExecutableLookup,
    pub host_kinds: &'static [&'static str],
    pub setup_scopes: &'static [SetupScopeKind],
    pub budget: EventBudgetPolicy,
    pub runtime_sources: &'static [&'static str],
}
pub enum ExecutableLookup {
    Path(&'static str),
    Unsupported,
}
pub enum SetupScopeKind {
    ConfigRoot,
    Profile,
}
pub struct EventBudgetPolicy {
    pub lifecycle_ms: u64,
    pub observer_ms: u64,
}
pub struct InstallEnvironment {
    /// Clock whose monotonic epoch defines the supplied CallBudget.
    pub clock: std::sync::Arc<dyn crate::protocol::time::Clock>,
    pub path: Option<std::ffi::OsString>,
    pub config_root: Option<std::path::PathBuf>,
    pub state_dir: Option<std::path::PathBuf>,
}
pub enum InstallObservation {
    /// Carries the unforgeable installed binary/schema witness, never a payload claim.
    CodexWitness(super::codex::InstalledVersion),
    Available {
        binary: std::path::PathBuf,
        identity: RuntimeIdentity,
    },
    Unavailable {
        diagnostic: String,
    },
    Unsupported(UnsupportedOperation),
}
pub use super::runtime::RuntimeIdentity;
pub struct AdmissionRequest {
    pub installed: InstallObservation,
    pub input: Option<HookInput>,
    pub runtime_candidate: Option<RuntimeIdentity>,
}
pub enum AdmissionDecision<A> {
    Listed {
        state: A,
        recipe: &'static str,
    },
    SchemaMatched {
        state: A,
        recipe: &'static str,
    },
    Optimistic {
        state: A,
        recipe: &'static str,
        diagnostic: String,
    },
    Refused {
        diagnostic: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionKind {
    Listed,
    SchemaMatched,
    Optimistic,
}
pub use super::state::Ladder;
pub struct HookInput {
    pub bytes: Vec<u8>,
    pub registered_event: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractDomain {
    Native,
    Bridge,
}
#[derive(Debug, Clone, Copy)]
pub struct ContractDescriptor {
    pub domain_id: &'static str,
    pub origin: super::evidence::EvidenceOrigin,
    pub events: &'static [super::evidence::EvidenceEvent],
    pub required_milestones: &'static [&'static str],
    pub qualifications: &'static [&'static str],
    pub holding: super::evidence::AttributionHolding,
    pub resumed_unavailable_reason: Option<&'static str>,
    pub domain: ContractDomain,
    pub contract: &'static crate::harness::contract::HarnessContract,
}
pub struct ContractObservation {
    pub domain: ContractDomain,
    pub classification: crate::harness::contract::Classification,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRole {
    TopLevel,
    Subagent,
    Unknown,
}
pub enum EventIntent {
    Lifecycle(crate::harness::context::EventKind),
    Current,
    QualifiedTurn(crate::harness::context::QualifiedTurn),
    Observer,
}
pub struct EventMetadata {
    pub skill_pointer: bool,
    /// An adapter callback can shorten the core end-to-end limit.
    pub callback_deadline: Option<std::time::Instant>,
    pub context_source: String,
    pub capability: super::Capability,
    pub domain: ContractDomain,
    pub native_event: String,
    pub shape_fields: Vec<String>,
}
pub enum DeliveryEligibility {
    Context,
    ObserverOnly,
    Ineligible,
}
pub enum RuntimeAttribution {
    Attributed(RuntimeIdentity),
    Unavailable { diagnostic: String },
}
pub struct DecodedEvent {
    pub harness: crate::harness::registry::AgentHarnessId,
    pub role: EventRole,
    pub native_session: Option<String>,
    pub event_id: String,
    pub intent: EventIntent,
    pub metadata: EventMetadata,
    pub delivery: DeliveryEligibility,
    pub runtime: RuntimeAttribution,
}
pub struct NeutralOffer {
    pub fixed_guidance: String,
    pub peer_data: serde_json::Value,
    pub ready_argv: Vec<Vec<String>>,
}
pub enum EncodedOutput {
    ContextBearing { bytes: Vec<u8> },
    ObserverOnly { bytes: Vec<u8> },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSetupScope {
    ConfigRoot(std::path::PathBuf),
    Profile {
        name: String,
        home: std::path::PathBuf,
    },
}
/// Local inputs captured once; paths never pass through UTF-8 conversion.
#[derive(Clone)]
pub struct SetupEnvironment {
    /// Monotonic epoch shared by local requests and their CallBudget.
    pub clock: std::sync::Arc<dyn crate::protocol::time::Clock>,
    pub home: Option<OsString>,
    pub path: Option<OsString>,
    pub cwd: std::path::PathBuf,
    pub executable: std::path::PathBuf,
    pub state_dir: Option<std::path::PathBuf>,
    pub host_endpoint: Option<std::path::PathBuf>,
    pub instance_source: serde_json::Value,
    pub config_roots: std::collections::BTreeMap<String, std::path::PathBuf>,
    pub declared: std::collections::BTreeMap<String, OsString>,
}
impl Default for SetupEnvironment {
    fn default() -> Self {
        Self {
            clock: std::sync::Arc::new(crate::app::SystemClock::new()),
            home: None,
            path: None,
            cwd: Default::default(),
            executable: Default::default(),
            state_dir: None,
            host_endpoint: None,
            instance_source: serde_json::Value::Null,
            config_roots: Default::default(),
            declared: Default::default(),
        }
    }
}
impl std::fmt::Debug for SetupEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupEnvironment")
            .field("cwd", &self.cwd)
            .field("executable", &self.executable)
            .field("state_dir", &self.state_dir)
            .field("host_endpoint", &self.host_endpoint)
            .field("config_roots", &self.config_roots)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SetupScopeRequest {
    #[default]
    Default,
    Profile(String),
}
#[derive(Debug, Clone, Copy)]
pub struct SetupOption {
    pub name: &'static str,
    pub conflicts: &'static [&'static str],
}
pub type SetupOptions = std::collections::BTreeMap<String, bool>;
#[derive(Debug, Clone)]
pub struct SetupRequest {
    pub scope: ResolvedSetupScope,
    pub executable: std::path::PathBuf,
    pub environment: SetupEnvironment,
    pub native_binary: Option<std::path::PathBuf>,
    pub options: SetupOptions,
}
#[derive(Debug, Clone)]
pub struct StatusRequest {
    pub scope: ResolvedSetupScope,
    pub environment: SetupEnvironment,
    pub native_binary: Option<std::path::PathBuf>,
}
#[derive(Debug, Clone)]
pub struct UnsetupRequest {
    pub scope: ResolvedSetupScope,
    pub environment: SetupEnvironment,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}
#[derive(Debug, Clone)]
pub struct SetupDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub text: String,
    manual_argv: Option<Vec<OsString>>,
}
impl SetupDiagnostic {
    pub fn new(code: &str, severity: DiagnosticSeverity, text: &str) -> Self {
        Self {
            code: code.chars().take(64).collect(),
            severity,
            text: text.chars().take(1024).collect(),
            manual_argv: None,
        }
    }
    pub fn manual_argv(&self) -> Option<&[OsString]> {
        self.manual_argv.as_deref()
    }
    pub fn with_manual_argv(mut self, argv: Vec<OsString>) -> Result<Self, SetupFailure> {
        if argv.is_empty()
            || argv.len() > 32
            || argv.iter().any(|word| {
                word.is_empty()
                    || word.len() > 4096
                    || word
                        .as_encoded_bytes()
                        .iter()
                        .any(|byte| *byte < 32 || *byte == 127)
            })
        {
            return Err(SetupFailure::Invalid(
                "manual argv exceeds the local instruction bounds".into(),
            ));
        }
        self.manual_argv = Some(argv);
        Ok(self)
    }
}
#[derive(Debug, Clone)]
pub enum LocalRepair {
    InstallOwned,
    RepairOwned,
    RemoveOwned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Top-level local installation action; concrete projection retains substeps.
pub enum SetupAction {
    InstalledOwned,
    AdoptedOwned,
    RemovedOwned,
    Unchanged,
}
pub struct SetupOutcome {
    pub actions: Vec<SetupAction>,
    pub diagnostic: String,
    pub projection: serde_json::Value,
    pub diagnostics: Vec<SetupDiagnostic>,
}
pub struct RemovalOutcome {
    pub actions: Vec<SetupAction>,
    pub diagnostic: String,
    pub residue: Vec<std::path::PathBuf>,
    pub projection: serde_json::Value,
    pub diagnostics: Vec<SetupDiagnostic>,
}
pub struct DoctorProjection {
    /// Captured in the same bounded observation as hooks; avoids a second probe.
    pub status: Option<SetupStatus>,
    pub hooks: serde_json::Value,
    pub limitations: Vec<String>,
    pub manual_repairs: Vec<serde_json::Value>,
    pub safe_repairs: Vec<LocalRepair>,
    pub repair_options: SetupOptions,
}
pub struct LocalSetupStatus {
    pub scope: ResolvedSetupScope,
    pub installed: bool,
    pub enabled: Option<bool>,
    pub admitted: Option<bool>,
    pub observed: Option<bool>,
    pub configured_hook: Option<crate::ports::ConfiguredHook>,
    pub fingerprint: Option<String>,
    pub diagnostics: Vec<SetupDiagnostic>,
    pub repairs: Vec<LocalRepair>,
    pub projection: serde_json::Value,
}
pub enum SetupStatus {
    Detailed(Box<LocalSetupStatus>),
    Failed(SetupFailure),
    Unsupported(UnsupportedOperation),
    Available {
        installed: bool,
        enabled: Option<bool>,
        diagnostic: String,
    },
    Unavailable {
        diagnostic: String,
    },
}
#[derive(Debug)]
pub enum DecodeFailure {
    Native(super::context::ContextError),
    Unsupported(UnsupportedOperation),
    Invalid(String),
    RegistrationMismatch,
}
#[derive(Debug)]
pub enum EncodeFailure {
    Unsupported(UnsupportedOperation),
    Invalid(String),
    RegistrationMismatch,
}
#[derive(Debug)]
pub enum SetupFailure {
    Api(crate::protocol::results::ApiError),
    Io(std::io::Error),
    Unsupported(UnsupportedOperation),
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedOperation {
    pub adapter: &'static str,
    pub operation: &'static str,
}
impl std::fmt::Display for UnsupportedOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} is unsupported", self.adapter, self.operation)
    }
}
impl std::error::Error for UnsupportedOperation {}

/// Adapter-local launch inputs. No seat, binding or host mutation capability.
pub struct LaunchRequest {
    pub argv: Vec<String>,
    pub environment: SetupEnvironment,
    pub native_binary: Option<std::path::PathBuf>,
}
#[derive(Debug, Clone)]
pub struct LaunchScope {
    pub setup: ResolvedSetupScope,
    pub working_directory: std::path::PathBuf,
    pub config_source: &'static str,
}
pub struct LaunchPreparation {
    pub argv: Vec<String>,
    pub hook: crate::ports::ConfiguredHook,
    pub working_directory: std::path::PathBuf,
    pub environment_overrides: std::collections::BTreeMap<String, OsString>,
    pub report: serde_json::Value,
    pub wrapper_warning: Option<&'static str>,
}
pub trait LaunchPolicy: Send + Sync {
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        probe: &dyn super::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchScope, crate::protocol::results::ApiError>;
    fn validate_native_argv(
        &self,
        argv: &[String],
    ) -> Result<(), crate::protocol::results::ApiError>;
    fn compose_argv(
        &self,
        caller: Vec<String>,
        owned: Vec<String>,
        shell_passes_no_daemon: bool,
    ) -> Result<Vec<String>, crate::protocol::results::ApiError>;
    fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &super::registry::AdmittedHandle,
        status: &LocalSetupStatus,
        probe: &dyn super::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchPreparation, crate::protocol::results::ApiError>;
    fn configuration_fingerprint(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
    ) -> Result<String, crate::protocol::results::ApiError>;
    fn expected_host_kinds(&self) -> &'static [&'static str];
}
pub trait ComposerPolicy: Send + Sync {
    /// Recipe declarations for the observed installed version; evidence never adds support.
    fn capabilities(&self, installed: Option<&str>) -> super::recipe::PokeCapabilities;
    fn read(&self, detection: &str, pane_width: Option<u16>) -> super::composer::ComposerRead;
    fn clear_key(&self) -> &'static str;
    /// Retyped text only: restoring a draft must never submit it.
    fn restore_text(&self, saved: &str) -> String;
}
/// Pure local metadata. Providers must not observe installations or invoke native code.
pub trait CanaryStrategy: Send + Sync {
    fn descriptor(&self) -> CanaryDescriptor;
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanaryKind {
    NpmRelease,
    ExactRuntime,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    StableRelease,
    ExactBuild,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryDescriptor {
    pub kind: CanaryKind,
    pub candidate_kind: CandidateKind,
    #[serde(deserialize_with = "required_canary_nullable")]
    pub npm_package: Option<String>,
    #[serde(deserialize_with = "required_canary_nullable")]
    pub model_key_env: Option<String>,
    pub companion: String,
    pub artifact_schema_version: u8,
}
fn required_canary_nullable<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <Option<String> as serde::Deserialize>::deserialize(d)
}
macro_rules! failure {
    ($ty:ident, $operation:literal, $($mismatch:ident)?) => {
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    Self::Unsupported(e) => e.fmt(f),
                    Self::Invalid(message) => write!(f, "{}: invalid adapter input: {}", $operation, message.chars().take(256).collect::<String>()),
                    $(Self::$mismatch => write!(f, "{}: adapter registration mismatch", $operation),)?
                }
            }
        }
        impl std::error::Error for $ty {}
    };
}
impl std::fmt::Display for DecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(error) => write!(f, "native decode: {error:?}"),
            Self::Unsupported(error) => error.fmt(f),
            Self::Invalid(message) => write!(
                f,
                "decode: invalid adapter input: {}",
                message.chars().take(256).collect::<String>()
            ),
            Self::RegistrationMismatch => write!(f, "decode: adapter registration mismatch"),
        }
    }
}
impl std::error::Error for DecodeFailure {}

failure!(EncodeFailure, "encode", RegistrationMismatch);
impl std::fmt::Display for SetupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(e) => f.write_str(&e.detail),
            Self::Io(e) => e.fmt(f),
            Self::Invalid(e) => write!(
                f,
                "setup: invalid adapter input: {}",
                e.chars().take(256).collect::<String>()
            ),
            Self::Unsupported(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for SetupFailure {}

impl DecodedEvent {
    pub fn can_check_in(&self) -> bool {
        matches!(self.role, EventRole::TopLevel)
            && matches!(
                self.intent,
                EventIntent::Lifecycle(_) | EventIntent::Current | EventIntent::QualifiedTurn(_)
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn local_manual_diagnostics_refuse_unbounded_argv() {
        let diagnostic = SetupDiagnostic::new(
            "manual_native_enable",
            DiagnosticSeverity::Info,
            "Enable explicitly in the native CLI",
        );
        for argv in [
            vec![],
            vec!["word".into(); 33],
            vec!["x".repeat(4097).into()],
            vec!["line\nbreak".into()],
            vec!["".into()],
        ] {
            assert!(
                diagnostic.clone().with_manual_argv(argv).is_err(),
                "manual instructions must retain the shared argv bounds"
            );
        }
        use std::os::unix::ffi::OsStringExt;
        let diagnostic = diagnostic
            .with_manual_argv(vec![
                "native".into(),
                OsString::from_vec(b"/profile-\xff".to_vec()),
            ])
            .unwrap();
        assert_eq!(diagnostic.manual_argv().unwrap()[0], "native");
        assert_eq!(
            diagnostic.manual_argv().unwrap()[1].as_encoded_bytes(),
            b"/profile-\xff"
        );
    }

    #[test]
    fn local_options_reject_undeclared_disabled_flag_before_writes() {
        let registry = super::super::registry::builtins();
        let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
        let options = [("invented".into(), false)].into_iter().collect();
        assert!(
            super::super::setup::validate_local_request(
                Some(registration),
                true,
                &SetupScopeRequest::Default,
                &options
            )
            .is_err(),
            "unknown options cannot bypass metadata validation by being disabled"
        );
    }

    #[test]
    fn setup_registry_dispatch_freezes_environment_and_rejects_ambiguous_profile() {
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(100),
            cancellation: Default::default(),
        };
        let registry = super::super::registry::builtins();
        let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("local-status-{}", uuid::Uuid::new_v4()));
        let mut environment = SetupEnvironment {
            executable: root.join("plugin"),
            state_dir: Some(root.join("state")),
            cwd: root.clone(),
            host_endpoint: Some(root.join("herdr.sock")),
            ..Default::default()
        };
        environment
            .config_roots
            .insert("claude".into(), root.join("claude"));
        environment
            .declared
            .insert("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION".into(), "0".into());
        let scope = registration
            .resolve_setup_scope(&SetupScopeRequest::Default, &environment)
            .unwrap();
        let status = registration.status(
            &StatusRequest {
                scope,
                environment: environment.clone(),
                native_binary: None,
            },
            &budget,
        );
        let SetupStatus::Detailed(status) = status else {
            panic!("registered local status must dispatch to its backend")
        };
        assert!(!status.installed);
        assert_eq!(status.enabled, None);
        assert_eq!(status.observed, None);
        assert_eq!(
            status.projection["prompt_suggestions"]["env_override"],
            "CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=0"
        );
        assert!(
            !root.exists(),
            "status created config, state, or daemon directories"
        );
        let profile = SetupScopeRequest::Profile("work".into());
        assert!(
            super::super::setup::validate_local_request(None, true, &profile, &Default::default())
                .is_err()
        );
        assert!(
            super::super::setup::validate_local_request(
                Some(registration),
                true,
                &profile,
                &Default::default()
            )
            .is_err()
        );
        assert!(!root.exists(), "profile refusal must precede writes");
        let options = [
            ("disable-prompt-suggestions".into(), true),
            ("keep-prompt-suggestions".into(), true),
        ]
        .into_iter()
        .collect();
        assert!(
            super::super::setup::validate_local_request(
                Some(registration),
                true,
                &SetupScopeRequest::Default,
                &options
            )
            .is_err()
        );
        let options = [("invented".into(), true)].into_iter().collect();
        assert!(
            super::super::setup::validate_local_request(
                Some(registration),
                true,
                &SetupScopeRequest::Default,
                &options
            )
            .is_err()
        );
    }

    #[test]
    fn only_top_level_lifecycle_and_current_can_check_in() {
        let mut event = DecodedEvent {
            harness: super::super::registry::builtins().agent("codex").unwrap(),
            role: EventRole::TopLevel,
            native_session: None,
            event_id: "event".into(),
            intent: EventIntent::Current,
            metadata: EventMetadata {
                context_source: "PreToolUse".into(),
                callback_deadline: None,
                skill_pointer: false,
                capability: crate::harness::Capability::ObservedInput,
                domain: ContractDomain::Native,
                native_event: "Tool".into(),
                shape_fields: vec![],
            },
            delivery: DeliveryEligibility::Context,
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "unavailable".into(),
            },
        };
        for role in [EventRole::TopLevel, EventRole::Subagent, EventRole::Unknown] {
            event.role = role;
            event.intent = EventIntent::Lifecycle(super::super::context::EventKind::Startup);
            assert_eq!(event.can_check_in(), role == EventRole::TopLevel);
            event.intent = EventIntent::Current;
            assert_eq!(event.can_check_in(), role == EventRole::TopLevel);
            event.intent = EventIntent::Observer;
            assert!(!event.can_check_in());
        }
    }
    #[test]
    fn builtin_hook_admission_refuses_missing_install_and_classifies_without_admission() {
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(100),
            cancellation: Default::default(),
        };
        for registration in super::super::registry::builtins().registrations() {
            let request = AdmissionRequest {
                installed: InstallObservation::Unavailable {
                    diagnostic: "test".into(),
                },
                input: None,
                runtime_candidate: None,
            };
            assert!(registration.admit(&request, &budget).is_err());
            assert!(matches!(
                registration.status(
                    &StatusRequest {
                        scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused")),
                        environment: SetupEnvironment::default(),
                        native_binary: None
                    },
                    &budget
                ),
                SetupStatus::Detailed(_)
            ));
            assert!(
                registration
                    .setup(
                        &SetupRequest {
                            scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused")),
                            environment: SetupEnvironment::default(),
                            native_binary: None,
                            executable: PathBuf::from("unused"),
                            options: Default::default()
                        },
                        &budget
                    )
                    .is_err()
            );
            assert!(
                registration
                    .unsetup(
                        &UnsetupRequest {
                            scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused")),
                            environment: SetupEnvironment::default()
                        },
                        &budget
                    )
                    .is_err()
            );
            // Providers exist independently of admission: an unobserved
            // installed recipe must still declare no poke capability.
            assert_eq!(
                registration.composer_policy().unwrap().capabilities(None),
                crate::harness::recipe::PokeCapabilities::NONE,
            );
            assert!(registration.canary_strategy().is_some());
            assert_eq!(
                registration.version_ladder(
                    &RuntimeIdentity::stable_release("999.0.0", "native_transcript").unwrap()
                ),
                Ladder::Admitted
            );
            let input = HookInput {
                bytes: b"not json".to_vec(),
                registered_event: None,
            };
            let observation = registration.classify(&input);
            assert_eq!(observation.domain, ContractDomain::Native);
            assert_eq!(
                observation.classification,
                crate::harness::contract::Classification::Malformed(
                    crate::harness::contract::Malformed::NotJson
                )
            );
            assert!(matches!(
                registration.attribute_runtime(&input, &budget),
                RuntimeAttribution::Unavailable { .. }
            ));
        }
    }
}

#[derive(Default)]
pub struct OutputPolicy {
    /// Legacy native child startup stays silent without a local daemon endpoint.
    pub child_requires_endpoint: bool,
    pub extra_guidance: &'static str,
    pub empty_lifecycle: bool,
    pub session_start_hint: bool,
}
impl DecodedEvent {
    pub fn from_native(event: super::LifecycleEvent) -> Self {
        let crate::harness::registry::OccupantHarness::Agent(harness) = event.harness.occupant()
        else {
            unreachable!("native agent event")
        };
        let native_event = if event.kind == super::context::EventKind::Tool {
            "PreToolUse"
        } else if event.source == "SubagentStart" {
            "SubagentStart"
        } else {
            "SessionStart"
        };
        Self {
            harness,
            role: match event.role {
                super::context::Role::TopLevel => EventRole::TopLevel,
                super::context::Role::Subagent => EventRole::Subagent,
            },
            native_session: event.native_session,
            event_id: event.event_id,
            intent: if event.kind == super::context::EventKind::Tool {
                EventIntent::Current
            } else {
                EventIntent::Lifecycle(event.kind)
            },
            metadata: EventMetadata {
                context_source: event.source,
                callback_deadline: None,
                skill_pointer: native_event == "SessionStart",
                capability: event.capability,
                domain: ContractDomain::Native,
                native_event: native_event.into(),
                shape_fields: vec![],
            },
            delivery: DeliveryEligibility::Context,
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "runtime attribution is separate".into(),
            },
        }
    }
    pub fn context_event(&self) -> Option<super::LifecycleEvent> {
        Some(super::LifecycleEvent {
            harness: super::registry::OccupantHarness::Agent(self.harness).into(),
            source: self.metadata.context_source.clone(),
            kind: match self.intent {
                EventIntent::Lifecycle(kind) => kind,
                EventIntent::Current => super::context::EventKind::Tool,
                EventIntent::QualifiedTurn(ref turn) => {
                    if self.native_session.as_deref() != Some(&turn.session)
                        || self.event_id != turn.event_key
                    {
                        return None;
                    }
                    if turn.reset.is_some() {
                        super::context::EventKind::Clear
                    } else {
                        super::context::EventKind::Startup
                    }
                }
                EventIntent::Observer => return None,
            },
            native_session: self.native_session.clone(),
            role: match self.role {
                EventRole::TopLevel => super::context::Role::TopLevel,
                EventRole::Subagent => super::context::Role::Subagent,
                EventRole::Unknown => return None,
            },
            event_id: self.event_id.clone(),
            capability: self.metadata.capability,
        })
    }
}
pub(crate) fn encode_context(
    event: &DecodedEvent,
    offer: &NeutralOffer,
) -> Result<EncodedOutput, EncodeFailure> {
    let context = &offer.fixed_guidance;
    if context.len() > 4096 {
        return Err(EncodeFailure::Invalid("context exceeds budget".into()));
    }
    let bytes = if context.is_empty() {
        vec![]
    } else {
        serde_json::to_vec(&serde_json::json!({"hookSpecificOutput": {"hookEventName": event.metadata.native_event, "additionalContext": context}})).map_err(|error| EncodeFailure::Invalid(error.to_string()))?
    };
    if matches!(event.delivery, DeliveryEligibility::Context)
        && !matches!(event.intent, EventIntent::Observer)
        && matches!(event.role, EventRole::TopLevel)
    {
        Ok(EncodedOutput::ContextBearing { bytes })
    } else {
        Ok(EncodedOutput::ObserverOnly { bytes })
    }
}

pub(crate) fn adapter_timeout(
    env: &InstallEnvironment,
    budget: &CallBudget,
) -> std::time::Duration {
    std::time::Duration::from_millis(
        budget
            .deadline
            .0
            .saturating_sub(env.clock.monotonic_now().0),
    )
}

/// One harness as the daemon observed it on its own `PATH` (the bounded
/// boot observation). Hook installation is per harness environment
/// (`$CLAUDE_CONFIG_DIR`, `$CODEX_HOME`), so `doctor`, run in that
/// environment, checks it; the daemon does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HarnessStatus {
    /// The observation has not completed.
    #[default]
    Unknown,
    /// No executable on the daemon's `PATH`: a note, never a degradation.
    NotInstalled(String),
    /// Present but its `--version` could not be observed or recognized, which
    /// blocks the hook: a limitation that degrades Health.
    Refused(String),
    /// Present, observed, and its version is below the recipe floor or inside
    /// a known-broken range. Whether that matters is the version verdict's
    /// (`harness::state`, rendered from evidence and the manifest), so Health
    /// shows nothing for it here; doctor shows the detected-version line.
    VersionRefused(String),
    /// Admitted by a recipe whose receipts are cooperative
    /// (`cooperative_top_level`). `live_unverified` marks a schema-matched
    /// admission, which stays listed as a limitation.
    Cooperative {
        detail: String,
        live_unverified: bool,
    },
    /// Admitted by a recipe that declares native-verified receipt: a listed
    /// version whose recipe proves native receipt, never an unlisted one.
    Supported(String),
    /// Unlisted but admitted by the ladder's optimistic rows, parsed under an
    /// assumed recipe, live-unverified. The detail is the operator-facing
    /// label (`crate::harness::optimistic_label`); Health renders it as an
    /// informational note, never a limitation or a degradation.
    Optimistic(String),
}

/// Cached install facts only. Runtime rows cannot change these scope-local axes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonObservation {
    pub status: HarnessStatus,
    pub identity: Option<RuntimeIdentity>,
    pub enablement: crate::protocol::results::HealthAxis<crate::protocol::results::EnablementState>,
    pub callback_observation:
        crate::protocol::results::HealthAxis<crate::protocol::results::CallbackObservationState>,
    pub receipt_basis: Option<String>,
}
pub fn executable_observation_fingerprint(env: &InstallEnvironment, name: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let binary = crate::cli::hook::resolve_on_path(name, env.path.as_deref())?;
    let identity = super::BinaryIdentity::observe(&binary)?;
    Some(format!(
        "{:x}",
        Sha256::digest(format!("{identity:?}").as_bytes())
    ))
}
