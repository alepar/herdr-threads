//! In-memory author contract. Observations grant no seat or receipt authority.
use crate::protocol::time::CallBudget;
use std::ffi::OsString;

pub trait HarnessAdapter: Send + Sync + 'static {
    type Admission: Send + Sync + 'static;
    fn metadata(&self) -> &'static AdapterMetadata;
    fn contracts(&self) -> &'static [ContractDescriptor];
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation;
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
    fn unsetup(
        &self,
        request: &UnsetupRequest,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure>;
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
    pub path: Option<std::ffi::OsString>,
    pub config_root: Option<std::path::PathBuf>,
}
pub enum InstallObservation {
    Available {
        binary: std::path::PathBuf,
        identity: RuntimeIdentity,
    },
    Unavailable {
        diagnostic: String,
    },
    Unsupported(UnsupportedOperation),
}
pub struct RuntimeIdentity {
    pub release_version: Option<String>,
    pub exact_key: Option<String>,
    pub provenance: RuntimeIdentityProvenance,
}
pub enum RuntimeIdentityProvenance {
    Unavailable,
    NativeRuntime,
    NativeTranscript,
    InstalledProbe,
}
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
pub struct Ladder {
    pub rows: Vec<LadderEntry>,
}
pub struct LadderEntry {
    pub recipe: &'static str,
    pub diagnostic: String,
}
pub struct HookInput {
    pub bytes: Vec<u8>,
    pub registered_event: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractDomain {
    Native,
    Bridge,
}
pub struct ContractDescriptor {
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
    Observer,
}
pub struct EventMetadata {
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
pub enum ResolvedSetupScope {
    ConfigRoot(std::path::PathBuf),
    Profile {
        name: String,
        home: std::path::PathBuf,
    },
}
pub struct SetupRequest {
    pub scope: ResolvedSetupScope,
    pub executable: std::path::PathBuf,
}
pub struct StatusRequest {
    pub scope: ResolvedSetupScope,
}
pub struct UnsetupRequest {
    pub scope: ResolvedSetupScope,
}
pub struct SetupOutcome {
    pub diagnostic: String,
}
pub struct RemovalOutcome {
    pub diagnostic: String,
    pub residue: Vec<std::path::PathBuf>,
}
pub enum SetupStatus {
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

pub struct LaunchRequest {
    pub argv: Vec<OsString>,
    pub scope: Option<ResolvedSetupScope>,
}
pub struct LaunchPreparation {
    pub argv: Vec<OsString>,
}
pub trait LaunchPolicy: Send + Sync {
    fn prepare(
        &self,
        request: &LaunchRequest,
        budget: &CallBudget,
    ) -> Result<LaunchPreparation, UnsupportedOperation>;
}
pub trait ComposerPolicy: Send + Sync {
    fn capabilities(&self) -> super::recipe::PokeCapabilities;
}
pub trait CanaryStrategy: Send + Sync {
    fn descriptor(&self) -> CanaryDescriptor;
}
pub struct CanaryDescriptor {
    pub id: &'static str,
    pub automatic_install_supported: bool,
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
failure!(DecodeFailure, "decode", RegistrationMismatch);
failure!(EncodeFailure, "encode", RegistrationMismatch);
failure!(SetupFailure, "setup",);

impl DecodedEvent {
    pub fn can_check_in(&self) -> bool {
        matches!(self.role, EventRole::TopLevel)
            && matches!(
                self.intent,
                EventIntent::Lifecycle(_) | EventIntent::Current
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn only_top_level_lifecycle_and_current_can_check_in() {
        let mut event = DecodedEvent {
            harness: super::super::registry::builtins().agent("codex").unwrap(),
            role: EventRole::TopLevel,
            native_session: None,
            event_id: "event".into(),
            intent: EventIntent::Current,
            metadata: EventMetadata {
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
    fn builtin_boundaries_are_inert_and_classify_native_contracts() {
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
                registration.observe_install(
                    &InstallEnvironment {
                        path: None,
                        config_root: None
                    },
                    &budget
                ),
                InstallObservation::Unsupported(_)
            ));
            assert!(matches!(
                registration.status(
                    &StatusRequest {
                        scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused"))
                    },
                    &budget
                ),
                SetupStatus::Unsupported(_)
            ));
            assert!(
                registration
                    .setup(
                        &SetupRequest {
                            scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused")),
                            executable: PathBuf::from("unused")
                        },
                        &budget
                    )
                    .is_err()
            );
            assert!(
                registration
                    .unsetup(
                        &UnsetupRequest {
                            scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused"))
                        },
                        &budget
                    )
                    .is_err()
            );
            assert!(registration.launch_policy().is_none());
            assert!(registration.composer_policy().is_none());
            assert!(registration.canary_strategy().is_none());
            assert!(
                registration
                    .version_ladder(&RuntimeIdentity {
                        release_version: Some("999.0.0".into()),
                        exact_key: None,
                        provenance: RuntimeIdentityProvenance::Unavailable
                    })
                    .rows
                    .is_empty()
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
