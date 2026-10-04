//! Validated static identities and registration-bound typed admission.
use super::adapter::*;
use crate::protocol::time::CallBudget;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{any::Any, sync::OnceLock};

/// Operational identity can only come from a validated registry lookup.
/// ```compile_fail
/// use herdr_threads::harness::registry::AgentHarnessId;
/// let forged = AgentHarnessId { id: "codex", context_spelling: "Codex" };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentHarnessId {
    id: &'static str,
    context_spelling: &'static str,
}
impl AgentHarnessId {
    pub fn as_str(self) -> &'static str {
        self.id
    }
    pub fn context_spelling(self) -> &'static str {
        self.context_spelling
    }
}
const CODEX: AgentHarnessId = AgentHarnessId {
    id: "codex",
    context_spelling: "Codex",
};
const CLAUDE: AgentHarnessId = AgentHarnessId {
    id: "claude",
    context_spelling: "Claude",
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OccupantHarness {
    Agent(AgentHarnessId),
    Human,
}
#[allow(non_upper_case_globals)]
impl OccupantHarness {
    pub const Codex: Self = Self::Agent(CODEX);
    pub const Claude: Self = Self::Agent(CLAUDE);
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent(id) => id.as_str(),
            Self::Human => "human",
        }
    }
    pub fn cooperative_provenance(self) -> &'static str {
        match self {
            Self::Agent(_) => crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE,
            Self::Human => crate::protocol::authority::OPERATOR_HUMAN_PROVENANCE,
        }
    }
}
impl Serialize for OccupantHarness {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let _ = builtins();
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for OccupantHarness {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        if id == "human" {
            Ok(Self::Human)
        } else {
            builtins()
                .agent(&id)
                .map(Self::Agent)
                .map_err(serde::de::Error::custom)
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    InvalidId(String),
    InvalidEvidenceMetadata(String),
    DuplicateId(String),
    DuplicateHostKind(String),
    InvalidContextSpelling(String),
    DuplicateContextSpelling(String),
    ReservedContextSpelling(String),
}
#[derive(Debug, PartialEq, Eq)]
pub enum LookupError {
    UnknownAgent(String),
    HumanIsNotAgent,
    UnknownContext(String),
}
#[derive(Debug)]
pub struct AdmissionFailure {
    pub diagnostic: String,
}
impl std::fmt::Display for AdmissionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "adapter admission refused: {}",
            self.diagnostic.chars().take(256).collect::<String>()
        )
    }
}
impl std::error::Error for AdmissionFailure {}
impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (problem, value) = match self {
            Self::InvalidId(value) => ("invalid ID", value),
            Self::InvalidEvidenceMetadata(value) => ("invalid evidence metadata", value),
            Self::DuplicateId(value) => ("duplicate ID", value),
            Self::DuplicateHostKind(value) => ("duplicate host kind", value),
            Self::InvalidContextSpelling(value) => ("invalid context spelling", value),
            Self::DuplicateContextSpelling(value) => ("duplicate context spelling", value),
            Self::ReservedContextSpelling(value) => ("reserved context spelling", value),
        };
        write!(
            f,
            "adapter registry: {problem}: {}",
            value.chars().take(64).collect::<String>()
        )
    }
}
impl std::error::Error for RegistryError {}
impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownAgent(id) => write!(
                f,
                "unregistered agent: {}",
                id.chars().take(64).collect::<String>()
            ),
            Self::HumanIsNotAgent => write!(f, "human is not an agent"),
            Self::UnknownContext(id) => write!(
                f,
                "unknown context harness: {}",
                id.chars().take(64).collect::<String>()
            ),
        }
    }
}
impl std::error::Error for LookupError {}

/// An opaque, non-cloneable admission bound to one exact registration.
/// ```compile_fail
/// use herdr_threads::harness::registry::AdmittedHandle;
/// let forged = AdmittedHandle { registration: todo!(), kind: todo!(), recipe: "x", diagnostic: None, state: Box::new(()) };
/// ```
pub struct AdmittedHandle {
    registration: &'static Registration,
    kind: AdmissionKind,
    recipe: &'static str,
    diagnostic: Option<String>,
    state: Box<dyn Any + Send + Sync>,
}
impl AdmittedHandle {
    pub fn metadata(&self) -> &'static AdapterMetadata {
        self.registration.metadata()
    }
    pub fn kind(&self) -> AdmissionKind {
        self.kind
    }
    pub fn recipe(&self) -> &'static str {
        self.recipe
    }
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}
struct ErasedAdmission {
    kind: AdmissionKind,
    recipe: &'static str,
    diagnostic: Option<String>,
    state: Box<dyn Any + Send + Sync>,
}
trait ErasedAdapter: Send + Sync {
    fn receipt_admission_summary(&self) -> Option<String>;
    fn output_policy(&self) -> OutputPolicy;
    fn legacy_contract_id(&self) -> Option<String>;
    fn contracts(&self) -> &'static [ContractDescriptor];
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation;
    fn observation_fingerprint(&self, env: &InstallEnvironment) -> Option<String>;
    fn observe_daemon(&self, env: &InstallEnvironment, budget: &CallBudget) -> DaemonObservation;
    fn admit(
        &self,
        request: &AdmissionRequest,
        budget: &CallBudget,
    ) -> Result<ErasedAdmission, AdmissionFailure>;
    fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder;
    fn classify(&self, input: &HookInput) -> ContractObservation;
    fn decode(
        &self,
        state: &(dyn Any + Send + Sync),
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure>;
    fn encode(
        &self,
        state: &(dyn Any + Send + Sync),
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure>;
    fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution;
    fn evidence_qualifications(
        &self,
        request: &EvidenceQualificationRequest<'_>,
        budget: &CallBudget,
    ) -> Result<Vec<String>, String>;
    fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure>;
    fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus;
    fn doctor_projection(
        &self,
        request: &StatusRequest,
        daemon: &serde_json::Value,
        budget: &CallBudget,
    ) -> Option<DoctorProjection>;

    fn unsetup(
        &self,
        request: &UnsetupRequest,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure>;
    fn setup_options(&self) -> &'static [SetupOption];
    fn setup_environment_inputs(&self) -> &'static [&'static str];
    fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        environment: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure>;
    fn settle_setup_consent(
        &self,
        environment: &SetupEnvironment,
        projection: &mut serde_json::Value,
        reader: &mut dyn std::io::BufRead,
        writer: &mut dyn std::io::Write,
    ) -> Result<(), SetupFailure>;
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy>;
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy>;
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy>;
}
struct TypedAdapter<A: HarnessAdapter>(&'static A);
impl<A: HarnessAdapter> ErasedAdapter for TypedAdapter<A> {
    fn receipt_admission_summary(&self) -> Option<String> {
        self.0.receipt_admission_summary()
    }
    fn observation_fingerprint(&self, env: &InstallEnvironment) -> Option<String> {
        self.0.observation_fingerprint(env)
    }
    fn observe_daemon(&self, env: &InstallEnvironment, budget: &CallBudget) -> DaemonObservation {
        self.0.observe_daemon(env, budget)
    }

    fn legacy_contract_id(&self) -> Option<String> {
        self.0.legacy_contract_id()
    }
    fn output_policy(&self) -> OutputPolicy {
        self.0.output_policy()
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        self.0.contracts()
    }
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation {
        self.0.observe_install(env, budget)
    }
    fn admit(
        &self,
        request: &AdmissionRequest,
        budget: &CallBudget,
    ) -> Result<ErasedAdmission, AdmissionFailure> {
        let (state, kind, recipe, diagnostic) = match self.0.admit(request, budget) {
            AdmissionDecision::Listed { state, recipe } => {
                (state, AdmissionKind::Listed, recipe, None)
            }
            AdmissionDecision::SchemaMatched { state, recipe } => {
                (state, AdmissionKind::SchemaMatched, recipe, None)
            }
            AdmissionDecision::Optimistic {
                state,
                recipe,
                diagnostic,
            } => (state, AdmissionKind::Optimistic, recipe, Some(diagnostic)),
            AdmissionDecision::Refused { diagnostic } => {
                return Err(AdmissionFailure { diagnostic });
            }
        };
        Ok(ErasedAdmission {
            state: Box::new(state),
            kind,
            recipe,
            diagnostic,
        })
    }
    fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder {
        self.0.version_ladder(identity)
    }
    fn classify(&self, input: &HookInput) -> ContractObservation {
        self.0.classify(input)
    }
    fn decode(
        &self,
        state: &(dyn Any + Send + Sync),
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure> {
        self.0.decode(
            state
                .downcast_ref::<A::Admission>()
                .ok_or(DecodeFailure::RegistrationMismatch)?,
            input,
        )
    }
    fn encode(
        &self,
        state: &(dyn Any + Send + Sync),
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        self.0.encode(
            state
                .downcast_ref::<A::Admission>()
                .ok_or(EncodeFailure::RegistrationMismatch)?,
            event,
            offer,
        )
    }
    fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution {
        self.0.attribute_runtime(input, budget)
    }
    fn evidence_qualifications(
        &self,
        request: &EvidenceQualificationRequest<'_>,
        budget: &CallBudget,
    ) -> Result<Vec<String>, String> {
        self.0.evidence_qualifications(request, budget)
    }
    fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure> {
        self.0.setup(request, budget)
    }
    fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus {
        self.0.status(request, budget)
    }
    fn doctor_projection(
        &self,
        request: &StatusRequest,
        daemon: &serde_json::Value,
        budget: &CallBudget,
    ) -> Option<DoctorProjection> {
        self.0.doctor_projection(request, daemon, budget)
    }

    fn unsetup(
        &self,
        request: &UnsetupRequest,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        self.0.unsetup(request, budget)
    }
    fn setup_options(&self) -> &'static [SetupOption] {
        self.0.setup_options()
    }
    fn setup_environment_inputs(&self) -> &'static [&'static str] {
        self.0.setup_environment_inputs()
    }
    fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        environment: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        self.0.resolve_setup_scope(request, environment)
    }
    fn settle_setup_consent(
        &self,
        environment: &SetupEnvironment,
        projection: &mut serde_json::Value,
        reader: &mut dyn std::io::BufRead,
        writer: &mut dyn std::io::Write,
    ) -> Result<(), SetupFailure> {
        self.0
            .settle_setup_consent(environment, projection, reader, writer)
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        self.0.launch_policy()
    }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        self.0.composer_policy()
    }
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> {
        self.0.canary_strategy()
    }
}
pub struct Registration {
    metadata: &'static AdapterMetadata,
    adapter: Box<dyn ErasedAdapter>,
    identity: OnceLock<AgentHarnessId>,
}
impl Registration {
    pub fn receipt_admission_summary(&self) -> Option<String> {
        self.adapter.receipt_admission_summary()
    }
    pub fn observation_fingerprint(&self, env: &InstallEnvironment) -> Option<String> {
        self.adapter.observation_fingerprint(env)
    }
    pub fn observe_daemon(
        &self,
        env: &InstallEnvironment,
        budget: &CallBudget,
    ) -> DaemonObservation {
        self.adapter.observe_daemon(env, budget)
    }

    pub fn new<A: HarnessAdapter>(adapter: &'static A) -> Self {
        Self {
            metadata: adapter.metadata(),
            adapter: Box::new(TypedAdapter(adapter)),
            identity: OnceLock::new(),
        }
    }
    pub fn metadata(&self) -> &'static AdapterMetadata {
        self.metadata
    }
    pub fn output_policy(&self) -> OutputPolicy {
        self.adapter.output_policy()
    }
    pub fn legacy_contract_id(&self) -> Option<String> {
        self.adapter.legacy_contract_id()
    }
    pub fn contracts(&self) -> &'static [ContractDescriptor] {
        self.adapter.contracts()
    }
    pub fn observe_install(
        &self,
        env: &InstallEnvironment,
        budget: &CallBudget,
    ) -> InstallObservation {
        self.adapter.observe_install(env, budget)
    }
    pub fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder {
        if identity.validate().is_err() {
            return Ladder::Admitted;
        }
        self.adapter.version_ladder(identity)
    }
    pub fn classify(&self, input: &HookInput) -> ContractObservation {
        self.adapter.classify(input)
    }
    /// Adapter-supplied bounded facts bound to this registration's exact domain/runtime.
    pub fn evidence_qualifications(
        &self,
        request: &EvidenceQualificationRequest<'_>,
        budget: &CallBudget,
    ) -> Result<Vec<String>, String> {
        let descriptor = request.descriptor;
        let id = descriptor.contract_id_v2()?;
        if request.runtime.validate().is_err()
            || !self
                .metadata
                .runtime_sources
                .contains(&request.runtime.source.as_str())
            || !self.contracts().iter().any(|known| {
                known.domain == descriptor.domain
                    && known.contract_id_v2().is_ok_and(|known_id| known_id == id)
            })
        {
            return Err("qualification request does not match registration".into());
        }
        let facts = self.adapter.evidence_qualifications(request, budget)?;
        let mut seen = std::collections::HashSet::new();
        if facts.len() > 8
            || facts.iter().any(|fact| {
                !super::evidence::valid_name(fact)
                    || !descriptor.qualifications.contains(&fact.as_str())
                    || !seen.insert(fact)
            })
        {
            return Err("invalid or undeclared evidence qualifications".into());
        }
        Ok(facts)
    }
    pub fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution {
        match self.adapter.attribute_runtime(input, budget) {
            RuntimeAttribution::Attributed(identity) => {
                if let Err(diagnostic) = identity.validate() {
                    return RuntimeAttribution::Unavailable { diagnostic };
                }
                if !self
                    .metadata
                    .runtime_sources
                    .contains(&identity.source.as_str())
                {
                    return RuntimeAttribution::Unavailable {
                        diagnostic: "undeclared runtime attribution source".into(),
                    };
                }
                RuntimeAttribution::Attributed(identity)
            }
            RuntimeAttribution::Unavailable { diagnostic } => {
                let diagnostic: String = diagnostic
                    .chars()
                    .filter(|c| !c.is_control())
                    .scan(0, |bytes, c| {
                        *bytes += c.len_utf8();
                        (*bytes <= 128).then_some(c)
                    })
                    .collect();
                RuntimeAttribution::Unavailable {
                    diagnostic: if diagnostic.is_empty() {
                        "adapter supplied no runtime attribution reason".into()
                    } else {
                        diagnostic
                    },
                }
            }
        }
    }
    /// Sticky session resume state belongs to the caller's durable gate.
    pub fn attribute_runtime_for_session(
        &self,
        input: &HookInput,
        budget: &CallBudget,
        resumed: bool,
    ) -> RuntimeAttribution {
        if resumed
            && let Some(reason) = self
                .contracts()
                .iter()
                .find_map(|descriptor| descriptor.resumed_unavailable_reason)
        {
            return RuntimeAttribution::Unavailable {
                diagnostic: reason.into(),
            };
        }
        self.attribute_runtime(input, budget)
    }
    pub fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure> {
        self.adapter.setup(request, budget)
    }
    pub fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus {
        self.adapter.status(request, budget)
    }
    pub fn doctor_projection(
        &self,
        request: &StatusRequest,
        daemon: &serde_json::Value,
        budget: &CallBudget,
    ) -> Option<DoctorProjection> {
        self.adapter.doctor_projection(request, daemon, budget)
    }

    pub fn unsetup(
        &self,
        request: &UnsetupRequest,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        self.adapter.unsetup(request, budget)
    }
    pub fn setup_options(&self) -> &'static [SetupOption] {
        self.adapter.setup_options()
    }
    pub fn setup_environment_inputs(&self) -> &'static [&'static str] {
        self.adapter.setup_environment_inputs()
    }
    pub fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        environment: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        self.adapter.resolve_setup_scope(request, environment)
    }
    pub fn settle_setup_consent(
        &self,
        environment: &SetupEnvironment,
        projection: &mut serde_json::Value,
        reader: &mut dyn std::io::BufRead,
        writer: &mut dyn std::io::Write,
    ) -> Result<(), SetupFailure> {
        self.adapter
            .settle_setup_consent(environment, projection, reader, writer)
    }
    pub fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        self.adapter.launch_policy()
    }
    /// Erased launch admission remains scoped to the registration that produced it.
    pub fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &AdmittedHandle,
        status: &LocalSetupStatus,
        probe: &dyn super::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchPreparation, crate::protocol::results::ApiError> {
        use crate::protocol::results::{ApiError, ErrorCode};
        if !std::ptr::eq(self, admitted.registration) {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "launch admission registration mismatch",
            ));
        }
        if status.scope != scope.setup {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "launch setup status belongs to a different scope",
            ));
        }
        super::launch::owned_launch_hook(status)?;
        self.launch_policy()
            .ok_or_else(|| {
                ApiError::new(
                    ErrorCode::UnsupportedHarness,
                    format!("{}: launch is unsupported", self.metadata().id),
                )
            })?
            .prepare_launch(request, scope, admitted, status, probe, budget)
    }
    pub fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        self.adapter.composer_policy()
    }
    pub fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> {
        self.adapter.canary_strategy()
    }
    pub fn admit(
        &'static self,
        request: &AdmissionRequest,
        budget: &CallBudget,
    ) -> Result<AdmittedHandle, AdmissionFailure> {
        if self.identity.get().is_none() {
            return Err(AdmissionFailure {
                diagnostic: format!(
                    "{}: registration has not been validated",
                    self.metadata().id
                ),
            });
        }
        let installed_identity = match &request.installed {
            InstallObservation::Available { identity, .. } => Some(identity),
            _ => None,
        };
        for identity in installed_identity
            .into_iter()
            .chain(request.runtime_candidate.as_ref())
        {
            identity
                .validate()
                .map_err(|diagnostic| AdmissionFailure { diagnostic })?;
            if !self
                .metadata
                .runtime_sources
                .contains(&identity.source.as_str())
            {
                return Err(AdmissionFailure {
                    diagnostic: "undeclared runtime attribution source".into(),
                });
            }
        }
        let row = self.adapter.admit(request, budget)?;
        Ok(AdmittedHandle {
            registration: self,
            kind: row.kind,
            recipe: row.recipe,
            diagnostic: row.diagnostic,
            state: row.state,
        })
    }
    pub fn decode(
        &'static self,
        admitted: &AdmittedHandle,
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure> {
        if !std::ptr::eq(self, admitted.registration) {
            return Err(DecodeFailure::RegistrationMismatch);
        }
        let event = self.adapter.decode(admitted.state.as_ref(), input)?;
        if self.identity.get() != Some(&event.harness) {
            return Err(DecodeFailure::RegistrationMismatch);
        }
        Ok(event)
    }
    /// Validate registration-bound admission and normalized event before orchestration.
    pub fn validate_event(
        &'static self,
        admitted: &AdmittedHandle,
        event: &DecodedEvent,
    ) -> Result<(), EncodeFailure> {
        if !std::ptr::eq(self, admitted.registration) || self.identity.get() != Some(&event.harness)
        {
            Err(EncodeFailure::RegistrationMismatch)
        } else {
            Ok(())
        }
    }
    pub fn encode(
        &'static self,
        admitted: &AdmittedHandle,
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        self.validate_event(admitted, event)?;
        self.adapter.encode(admitted.state.as_ref(), event, offer)
    }
}
pub struct Registry {
    registrations: &'static [Registration],
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
impl Registry {
    pub fn new(registrations: &'static [Registration]) -> Result<Self, RegistryError> {
        let mut ids = std::collections::HashSet::new();
        let mut contexts = std::collections::HashSet::new();
        let mut kinds = std::collections::HashSet::new();
        for r in registrations {
            let m = r.metadata();
            if !token(m.id)
                || !m.id.as_bytes()[0].is_ascii_lowercase()
                || m.id.bytes().any(|b| b.is_ascii_uppercase())
                || m.id == "human"
            {
                return Err(RegistryError::InvalidId(m.id.into()));
            }
            if m.runtime_sources.len() > 8
                || m.runtime_sources
                    .iter()
                    .any(|source| !super::runtime::token(source, 32))
                || m.runtime_sources
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != m.runtime_sources.len()
            {
                return Err(RegistryError::InvalidEvidenceMetadata(m.id.into()));
            }
            let mut domains = std::collections::HashSet::new();
            if r.contracts().len() > 8
                || r.contracts().iter().any(|descriptor| {
                    descriptor.validate().is_err()
                        || descriptor.contract.harness != m.id
                        || !domains.insert(descriptor.domain_id)
                })
            {
                return Err(RegistryError::InvalidEvidenceMetadata(m.id.into()));
            }
            if !ids.insert(m.id) {
                return Err(RegistryError::DuplicateId(m.id.into()));
            }
            for spelling in
                std::iter::once(m.context_spelling).chain(m.context_aliases.iter().copied())
            {
                if !token(spelling) {
                    return Err(RegistryError::InvalidContextSpelling(spelling.into()));
                }
                if matches!(spelling, "Human" | "human") {
                    return Err(RegistryError::ReservedContextSpelling(spelling.into()));
                }
                if !contexts.insert(spelling) {
                    return Err(RegistryError::DuplicateContextSpelling(spelling.into()));
                }
            }
            for kind in m.host_kinds {
                if !kinds.insert(*kind) {
                    return Err(RegistryError::DuplicateHostKind((*kind).into()));
                }
            }
        }
        // No registration receives an identity until the entire set passes validation.
        for r in registrations {
            let m = r.metadata();
            let _ = r.identity.set(AgentHarnessId {
                id: m.id,
                context_spelling: m.context_spelling,
            });
        }
        Ok(Self { registrations })
    }
    pub fn registrations(&self) -> &'static [Registration] {
        self.registrations
    }
    pub fn agent(&self, id: &str) -> Result<AgentHarnessId, LookupError> {
        if id == "human" || id == "Human" {
            return Err(LookupError::HumanIsNotAgent);
        }
        self.registrations
            .iter()
            .find(|r| r.metadata().id == id)
            .and_then(|r| r.identity.get())
            .copied()
            .ok_or_else(|| LookupError::UnknownAgent(id.into()))
    }
    pub fn by_id(&self, id: AgentHarnessId) -> Result<&'static Registration, LookupError> {
        self.registrations
            .iter()
            .find(|r| r.identity.get() == Some(&id))
            .ok_or_else(|| LookupError::UnknownAgent(id.as_str().into()))
    }
    pub fn by_context_spelling(&self, spelling: &str) -> Result<OccupantHarness, LookupError> {
        if spelling == "Human" {
            return Ok(OccupantHarness::Human);
        }
        self.registrations
            .iter()
            .find(|r| {
                let m = r.metadata();
                m.context_spelling == spelling || m.context_aliases.contains(&spelling)
            })
            .and_then(|r| r.identity.get())
            .copied()
            .map(OccupantHarness::Agent)
            .ok_or_else(|| LookupError::UnknownContext(spelling.into()))
    }
    pub fn by_host_kind(&self, kind: &str) -> Option<&'static Registration> {
        self.registrations
            .iter()
            .find(|r| r.metadata().host_kinds.contains(&kind))
    }
}
pub fn builtins() -> &'static Registry {
    static BUILTINS: OnceLock<Registry> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        let registrations = Box::leak(
            vec![
                Registration::new(&crate::harness::claude::ClaudeAdapter),
                Registration::new(&crate::harness::codex::CodexAdapter),
            ]
            .into_boxed_slice(),
        );
        let registry =
            Registry::new(registrations).expect("built-in adapter registry validation failed");
        assert_eq!(
            registry.agent("claude").expect("built-in Claude missing"),
            CLAUDE
        );
        assert_eq!(
            registry.agent("codex").expect("built-in Codex missing"),
            CODEX
        );
        registry
    })
}
#[cfg(test)]
mod tests {
    fn unsupported(adapter: &'static str, operation: &'static str) -> UnsupportedOperation {
        UnsupportedOperation { adapter, operation }
    }
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct TestAdapter {
        metadata: &'static AdapterMetadata,
        calls: AtomicUsize,
        event_id: &'static str,
        harness: OnceLock<AgentHarnessId>,
        offers: std::sync::Mutex<Vec<String>>,
    }
    impl HarnessAdapter for TestAdapter {
        type Admission = usize;
        fn metadata(&self) -> &'static AdapterMetadata {
            self.metadata
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            &[]
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            InstallObservation::Unsupported(unsupported(self.metadata.id, "install"))
        }
        fn admit(&self, request: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<usize> {
            match request
                .input
                .as_ref()
                .and_then(|input| input.registered_event.as_deref())
            {
                Some("schema") => AdmissionDecision::SchemaMatched {
                    state: 17,
                    recipe: "schema-recipe",
                },
                Some("optimistic") => AdmissionDecision::Optimistic {
                    state: 17,
                    recipe: "optimistic-recipe",
                    diagnostic: "new runtime".into(),
                },
                Some("refused") => AdmissionDecision::Refused {
                    diagnostic: "test: no recipe".into(),
                },
                _ => AdmissionDecision::Listed {
                    state: 17,
                    recipe: "test-recipe",
                },
            }
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            Ladder::Admitted
        }
        fn classify(&self, input: &HookInput) -> ContractObservation {
            ContractObservation {
                domain: ContractDomain::Native,
                classification: super::super::contract::classify(
                    &super::super::claude::CONTRACT,
                    input.registered_event.as_deref(),
                    &input.bytes,
                ),
            }
        }
        fn decode(&self, state: &usize, input: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(*state, 17);
            let mut event = event(self.event_id);
            if let Some(harness) = self.harness.get() {
                event.harness = *harness;
            }
            match input.registered_event.as_deref() {
                Some("unknown") => event.role = EventRole::Unknown,
                Some("observer") => {
                    event.intent = EventIntent::Observer;
                    event.delivery = DeliveryEligibility::ObserverOnly;
                }
                Some("child") => {
                    event.role = EventRole::Subagent;
                    event.intent =
                        EventIntent::Lifecycle(super::super::context::EventKind::Startup);
                }
                _ => {}
            }
            Ok(event)
        }
        fn encode(
            &self,
            state: &usize,
            _: &DecodedEvent,
            offer: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            self.offers
                .lock()
                .unwrap()
                .push(offer.fixed_guidance.clone());
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(EncodedOutput::ContextBearing {
                bytes: state.to_string().into_bytes(),
            })
        }
        fn attribute_runtime(&self, input: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            RuntimeAttribution::Unavailable {
                diagnostic: input.registered_event.as_deref().unwrap_or("test").into(),
            }
        }
        fn setup_environment_inputs(&self) -> &'static [&'static str] {
            &["LOCAL_INPUT"]
        }
        fn setup_options(&self) -> &'static [SetupOption] {
            &[
                SetupOption {
                    name: "toggle",
                    conflicts: &["other"],
                },
                SetupOption {
                    name: "other",
                    conflicts: &["toggle"],
                },
            ]
        }
        fn resolve_setup_scope(
            &self,
            scope: &SetupScopeRequest,
            environment: &SetupEnvironment,
        ) -> Result<ResolvedSetupScope, SetupFailure> {
            let root = environment.config_roots.get(self.metadata.id).unwrap();
            Ok(match scope {
                SetupScopeRequest::Default => ResolvedSetupScope::ConfigRoot(root.clone()),
                SetupScopeRequest::Profile(name) => ResolvedSetupScope::Profile {
                    name: name.clone(),
                    home: root.join(name),
                },
            })
        }
        fn setup(
            &self,
            request: &SetupRequest,
            _: &CallBudget,
        ) -> Result<SetupOutcome, SetupFailure> {
            let root = local_root(&request.scope);
            std::fs::create_dir_all(root).map_err(SetupFailure::Io)?;
            let bytes = request
                .environment
                .declared
                .get("LOCAL_INPUT")
                .unwrap()
                .as_encoded_bytes();
            std::fs::write(root.join("owned"), bytes).map_err(SetupFailure::Io)?;
            Ok(SetupOutcome {
                actions: vec![SetupAction::InstalledOwned],
                diagnostic: String::new(),
                diagnostics: vec![],
                projection: serde_json::json!({"action":"installed", "executable":request.executable, "native_binary":request.native_binary, "toggle":request.options.get("toggle"), "cwd_bytes":request.environment.cwd.as_os_str().as_encoded_bytes(), "path_bytes":request.environment.path.as_ref().map(|path| path.as_encoded_bytes()), "home_bytes":request.environment.home.as_ref().map(|home| home.as_encoded_bytes())}),
            })
        }
        fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
            let root = local_root(&request.scope);
            if root.is_file() {
                return SetupStatus::Failed(match std::fs::read_to_string(root) {
                    Ok(_) => {
                        SetupFailure::Api(crate::protocol::results::ApiError::invalid_request(
                            "config root is a file",
                        ))
                    }
                    Err(error) => SetupFailure::Io(error),
                });
            }
            SetupStatus::Detailed(Box::new(LocalSetupStatus {
                scope: request.scope.clone(),
                installed: root.join("owned").is_file(),
                enabled: None,
                admitted: None,
                observed: None,
                configured_hook: None,
                fingerprint: None,
                diagnostics: vec![],
                repairs: vec![],
                projection: serde_json::json!({"action":"status", "installed":root.join("owned").is_file()}),
            }))
        }
        fn unsetup(
            &self,
            request: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            std::fs::remove_file(local_root(&request.scope).join("owned"))
                .map_err(SetupFailure::Io)?;
            Ok(RemovalOutcome {
                actions: vec![SetupAction::RemovedOwned],
                diagnostic: String::new(),
                residue: vec![],
                diagnostics: vec![],
                projection: serde_json::json!({"action":"removed"}),
            })
        }
    }
    fn local_root(scope: &ResolvedSetupScope) -> &std::path::Path {
        match scope {
            ResolvedSetupScope::ConfigRoot(root) => root,
            ResolvedSetupScope::Profile { home, .. } => home,
        }
    }
    #[test]
    fn local_setup_refuses_relative_owned_executable_before_backend() {
        use crate::cli::setup::{SetupVerb, execute_registered};
        let adapter = fixture("relative", "Relative", &[], &["relative"]);
        let registry = registry(&[adapter]).unwrap();
        let registration = registry.by_id(registry.agent("relative").unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("adapter-relative-{}", uuid::Uuid::new_v4()));
        let mut snapshot = SetupEnvironment {
            executable: "relative-plugin".into(),
            ..Default::default()
        };
        snapshot
            .config_roots
            .insert("relative".into(), root.clone());
        snapshot
            .declared
            .insert("LOCAL_INPUT".into(), "snapshot".into());
        let result = execute_registered(
            registration,
            SetupVerb::Install,
            &Default::default(),
            None,
            Default::default(),
            &snapshot,
        );
        let wrote = root.exists();
        if wrote {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(result.is_err(), "relative owned executable must be refused");
        assert!(!wrote, "refusal must precede backend writes");
    }

    #[test]
    fn injected_local_adapter_dispatch_preserves_snapshot_profiles_options_and_removal() {
        use crate::cli::setup::{SetupVerb, execute_registered};
        let adapter = fixture("local", "Local", &[], &["local"]);
        let registry = registry(&[adapter]).unwrap();
        let registration = registry.by_id(registry.agent("local").unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("adapter-local-{}", uuid::Uuid::new_v4()));
        let mut live = SetupEnvironment {
            executable: root.join("plugin"),
            cwd: root.join("cwd"),
            path: Some("original-path".into()),
            ..Default::default()
        };
        live.config_roots.insert("local".into(), root.clone());
        live.declared
            .insert("LOCAL_INPUT".into(), "from snapshot".into());
        use std::os::unix::ffi::OsStringExt;
        live.cwd = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/cwd-\xff".to_vec()));
        live.path = Some(std::ffi::OsString::from_vec(b"/path-\xff".to_vec()));
        live.home = Some(std::ffi::OsString::from_vec(b"/home-\xff".to_vec()));
        let snapshot = live.clone();
        live.declared
            .insert("LOCAL_INPUT".into(), "changed later".into());
        live.executable = root.join("other-plugin");
        live.cwd = root.join("changed-cwd");
        live.path = Some("changed-path".into());
        live.home = Some("changed-home".into());
        let scope = SetupScopeRequest::Profile("work".into());
        assert_eq!(
            execute_registered(
                registration,
                SetupVerb::Status,
                &scope,
                None,
                Default::default(),
                &snapshot
            )
            .unwrap()["installed"],
            false
        );
        assert!(!root.exists());
        let invalid = [("unknown".into(), true)].into_iter().collect();
        assert!(
            execute_registered(
                registration,
                SetupVerb::Install,
                &scope,
                None,
                invalid,
                &snapshot
            )
            .is_err()
        );
        let conflict = [("toggle".into(), true), ("other".into(), true)]
            .into_iter()
            .collect();
        assert!(
            execute_registered(
                registration,
                SetupVerb::Install,
                &scope,
                None,
                conflict,
                &snapshot
            )
            .is_err()
        );
        assert!(!root.exists());
        let options = [("toggle".into(), true)].into_iter().collect();
        let native = root.join("native");
        let report = execute_registered(
            registration,
            SetupVerb::Install,
            &scope,
            Some(&native),
            options,
            &snapshot,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(root.join("work/owned")).unwrap(),
            b"from snapshot"
        );
        assert_eq!(report["executable"], serde_json::json!(root.join("plugin")));
        assert_eq!(report["native_binary"], serde_json::json!(native));
        assert_eq!(report["toggle"], true);
        assert_eq!(
            report["cwd_bytes"],
            serde_json::json!([47, 99, 119, 100, 45, 255])
        );
        assert_eq!(
            report["path_bytes"],
            serde_json::json!([47, 112, 97, 116, 104, 45, 255])
        );
        assert_eq!(
            report["home_bytes"],
            serde_json::json!([47, 104, 111, 109, 101, 45, 255])
        );
        assert_eq!(
            execute_registered(
                registration,
                SetupVerb::Status,
                &scope,
                None,
                Default::default(),
                &snapshot
            )
            .unwrap()["installed"],
            true
        );
        execute_registered(
            registration,
            SetupVerb::Remove,
            &scope,
            None,
            Default::default(),
            &snapshot,
        )
        .unwrap();
        assert!(!root.join("work/owned").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn injected_local_aggregate_keeps_order_continues_after_error_and_preserves_first_exit() {
        use crate::cli::setup::{PromptSuggestionPolicy, SetupVerb, execute_all_registered};
        let first = fixture("first", "First", &[], &["first"]);
        let second = fixture("second", "Second", &[], &["second"]);
        let third = fixture("third", "Third", &[], &["third"]);
        let registry = registry(&[first, second, third]).unwrap();
        let root = std::env::temp_dir().join(format!("adapter-aggregate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("first"), "invalid config root").unwrap();
        std::fs::write(root.join("second"), [255]).unwrap();
        let mut environment = SetupEnvironment::default();
        environment
            .config_roots
            .insert("first".into(), root.join("first"));
        environment
            .config_roots
            .insert("second".into(), root.join("second"));
        environment
            .config_roots
            .insert("third".into(), root.join("third"));
        let report = execute_all_registered(
            &registry,
            SetupVerb::Status,
            PromptSuggestionPolicy::Ask,
            &environment,
        )
        .unwrap();
        assert_eq!(report["exit_status"], 2);
        assert_eq!(report["harnesses"][0]["harness"], "first");
        assert_eq!(report["harnesses"][0]["outcome"], "failed");
        assert_eq!(report["harnesses"][1]["harness"], "second");
        assert_eq!(report["harnesses"][1]["outcome"], "failed");
        assert_eq!(report["harnesses"][1]["exit_status"], 1);
        assert_eq!(report["harnesses"][2]["harness"], "third");
        assert_eq!(report["harnesses"][2]["outcome"], "status");
        assert!(!root.join("third").exists());
        let skipped = execute_all_registered(
            &registry,
            SetupVerb::Install,
            PromptSuggestionPolicy::Ask,
            &environment,
        )
        .unwrap();
        assert_eq!(skipped["exit_status"], 0);
        assert_eq!(skipped["harnesses"][0]["outcome"], "skipped");
        assert_eq!(skipped["harnesses"][1]["outcome"], "skipped");
        assert_eq!(skipped["harnesses"][2]["outcome"], "skipped");
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fixture(
        id: &'static str,
        spelling: &'static str,
        aliases: &'static [&'static str],
        kinds: &'static [&'static str],
    ) -> &'static TestAdapter {
        fixture_with_budget(
            id,
            spelling,
            aliases,
            kinds,
            EventBudgetPolicy {
                lifecycle_ms: 5,
                observer_ms: 2,
            },
        )
    }
    fn fixture_with_budget(
        id: &'static str,
        spelling: &'static str,
        aliases: &'static [&'static str],
        kinds: &'static [&'static str],
        budget: EventBudgetPolicy,
    ) -> &'static TestAdapter {
        Box::leak(Box::new(TestAdapter {
            metadata: Box::leak(Box::new(AdapterMetadata {
                id,
                display_label: "Test",
                context_spelling: spelling,
                context_aliases: aliases,
                executable: ExecutableLookup::Unsupported,
                host_kinds: kinds,
                setup_scopes: &[SetupScopeKind::ConfigRoot, SetupScopeKind::Profile],
                runtime_sources: &[],
                budget,
            })),
            calls: AtomicUsize::new(0),
            event_id: "codex",
            harness: OnceLock::new(),
            offers: std::sync::Mutex::new(vec![]),
        }))
    }
    fn registry(adapters: &[&'static TestAdapter]) -> Result<Registry, RegistryError> {
        Registry::new(Box::leak(
            adapters
                .iter()
                .map(|a| Registration::new(*a))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        ))
    }
    fn event(id: &str) -> DecodedEvent {
        DecodedEvent {
            harness: builtins().agent(id).unwrap(),
            role: EventRole::TopLevel,
            native_session: None,
            event_id: "event".into(),
            intent: EventIntent::Current,
            metadata: EventMetadata {
                context_source: "PreToolUse".into(),
                callback_deadline: None,
                skill_pointer: false,
                capability: super::super::Capability::ObservedInput,
                domain: ContractDomain::Native,
                native_event: "event".into(),
                shape_fields: vec![],
            },
            delivery: DeliveryEligibility::Context,
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "test".into(),
            },
        }
    }
    fn request() -> AdmissionRequest {
        AdmissionRequest {
            installed: InstallObservation::Unavailable {
                diagnostic: "test".into(),
            },
            input: None,
            runtime_candidate: None,
        }
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: crate::protocol::time::MonoInstant(100),
            cancellation: Default::default(),
        }
    }
    fn input() -> HookInput {
        HookInput {
            bytes: b"{}".to_vec(),
            registered_event: None,
        }
    }
    // Removing the registration fallback exposes empty adapter reasons to callers.
    fn assert_unavailable_reason(raw: &str) {
        let registry = registry(&[fixture("reason", "Reason", &[], &[])]).unwrap();
        let result = registry.registrations()[0].attribute_runtime(
            &HookInput {
                registered_event: Some(raw.into()),
                ..input()
            },
            &budget(),
        );
        let RuntimeAttribution::Unavailable { diagnostic } = result else {
            panic!("unavailable attribution must not acquire a runtime key");
        };
        assert!(
            !diagnostic.is_empty(),
            "adapter reason {raw:?} became empty"
        );
        assert!(diagnostic.len() <= 128);
        assert!(!diagnostic.chars().any(char::is_control));
    }
    #[test]
    fn unavailable_attribution_empty_reason_has_nonempty_fallback() {
        assert_unavailable_reason("");
    }
    #[test]
    fn unavailable_attribution_control_only_reason_has_nonempty_fallback() {
        assert_unavailable_reason("\0\n\r\t\u{7f}");
    }
    fn offer() -> NeutralOffer {
        NeutralOffer {
            fixed_guidance: String::new(),
            peer_data: serde_json::Value::Null,
            ready_argv: vec![],
        }
    }
    // Catches brand-dependent dispatch or any canonical lookup for ineligible roles:
    // the supplied state/endpoint cannot serve a seat, offer, attention or check-in.
    #[test]
    fn hook_fake_adapter_decode_encode_skips_canonical_work_for_ineligible_roles() {
        let a = fixture_with_budget(
            "fakehooks",
            "FakeHooks",
            &[],
            &[],
            EventBudgetPolicy {
                lifecycle_ms: 5000,
                observer_ms: 1500,
            },
        );
        let registry = registry(&[a]).unwrap();
        a.harness.set(registry.agent("fakehooks").unwrap()).unwrap();
        let registration = &registry.registrations()[0];
        let admitted = registration.admit(&request(), &budget()).unwrap();
        let args = crate::cli::hook::HookArgs {
            harness: OccupantHarness::Agent(registry.agent("fakehooks").unwrap()).into(),
            state_dir: Some("/missing/no-hook-state".into()),
            host_endpoint: None,
            event: None,
        };
        for (role, expected_bytes, expected_calls) in [
            ("unknown", &b""[..], 1),
            ("observer", &b"17"[..], 2),
            ("child", &b"17"[..], 2),
        ] {
            a.calls.store(0, Ordering::SeqCst);
            a.offers.lock().unwrap().clear();
            let decoded = registration
                .decode(
                    &admitted,
                    &HookInput {
                        bytes: vec![],
                        registered_event: Some(role.into()),
                    },
                )
                .unwrap();
            let outcome = crate::cli::hook::run_admitted_hook(
                &args,
                registration,
                &admitted,
                &decoded,
                &crate::cli::hook::HookEnv {
                    herdr_env: true,
                    pane: Some("w1:p1".into()),
                },
                std::time::Instant::now() + std::time::Duration::from_secs(1),
                std::sync::Arc::new(crate::app::SystemClock::new()),
                None,
            );
            assert_eq!(outcome.stdout, expected_bytes, "{role}");
            assert_eq!(
                outcome.diagnostic, None,
                "{role}: canonical state must not be consulted"
            );
            assert!(
                outcome.attention.is_none(),
                "{role}: encoded bytes cannot consume"
            );
            assert_eq!(a.calls.load(Ordering::SeqCst), expected_calls, "{role}");
            if role == "child" {
                assert!(a.offers.lock().unwrap()[0].contains(crate::harness::CHILD_RESTRICTION));
            }
        }
        let mut decoded = registration.decode(&admitted, &input()).unwrap();
        decoded.metadata.callback_deadline =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
        a.calls.store(0, Ordering::SeqCst);
        let outcome = crate::cli::hook::run_admitted_hook(
            &args,
            registration,
            &admitted,
            &decoded,
            &crate::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            std::sync::Arc::new(crate::app::SystemClock::new()),
            None,
        );
        assert!(outcome.stdout.is_empty());
        assert!(outcome.diagnostic.unwrap().contains("budget expired"));
        assert_eq!(a.calls.load(Ordering::SeqCst), 0);
    }
    // Catches unvalidated handles on quiet-role paths and brand-dependent budgets.
    #[test]
    fn hook_fake_adapter_refuses_foreign_unknown_handle() {
        let a = fixture("small", "Small", &[], &[]);
        let registry = registry(&[a]).unwrap();
        let registration = &registry.registrations()[0];
        let admitted = registration.admit(&request(), &budget()).unwrap();
        assert_eq!(
            crate::cli::hook::event_budget(registration, true, None),
            std::time::Duration::from_millis(5)
        );
        assert_eq!(
            crate::cli::hook::event_budget(registration, false, None),
            std::time::Duration::from_millis(2)
        );
        assert_eq!(
            crate::cli::hook::event_budget(
                registration,
                true,
                Some(std::time::Duration::from_millis(1))
            ),
            std::time::Duration::from_millis(1)
        );
        let id = registry.agent("small").unwrap();
        let lifecycle = crate::harness::LifecycleEvent {
            harness: OccupantHarness::Agent(id).into(),
            source: "startup".into(),
            kind: super::super::context::EventKind::Startup,
            native_session: None,
            role: super::super::context::Role::TopLevel,
            event_id: "fake".into(),
            capability: super::super::Capability::ObservedInput,
        };

        let mut decoded = event("codex");
        decoded.harness = id;
        decoded.role = EventRole::Unknown;
        let args = crate::cli::hook::HookArgs {
            harness: lifecycle.harness,
            state_dir: Some("/unavailable/never-created".into()),
            host_endpoint: None,
            event: None,
        };
        let outcome = crate::cli::hook::run_admitted_hook(
            &args,
            builtins()
                .by_id(builtins().agent("claude").unwrap())
                .unwrap(),
            &admitted,
            &decoded,
            &crate::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            std::sync::Arc::new(crate::app::SystemClock::new()),
            None,
        );
        assert!(
            outcome
                .diagnostic
                .unwrap()
                .contains("registration mismatch")
        );
        assert_eq!(a.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn admission_preserves_kind_recipe_diagnostic_and_typed_state() {
        let a = fixture("codex", "Codex", &[], &[]);
        let r = registry(&[a]).unwrap();
        let registration = &r.registrations()[0];
        for (event_name, kind, recipe, diagnostic) in [
            (
                "schema",
                AdmissionKind::SchemaMatched,
                "schema-recipe",
                None,
            ),
            (
                "optimistic",
                AdmissionKind::Optimistic,
                "optimistic-recipe",
                Some("new runtime"),
            ),
        ] {
            let mut request = request();
            request.input = Some(HookInput {
                bytes: vec![],
                registered_event: Some(event_name.into()),
            });
            let h = registration.admit(&request, &budget()).unwrap();
            assert_eq!(h.kind(), kind);
            assert_eq!(h.recipe(), recipe);
            assert_eq!(h.diagnostic(), diagnostic);
            let EncodedOutput::ContextBearing { bytes } =
                registration.encode(&h, &event("codex"), &offer()).unwrap()
            else {
                panic!("wrong output")
            };
            assert_eq!(bytes, b"17");
        }
        let mut request = request();
        request.input = Some(HookInput {
            bytes: vec![],
            registered_event: Some("refused".into()),
        });
        let Err(error) = registration.admit(&request, &budget()) else {
            panic!("refused admission produced a handle")
        };
        assert_eq!(error.diagnostic, "test: no recipe");
    }
    #[test]
    fn new_identity_keeps_legacy_consumers_inert() {
        let r = registry(&[fixture("test-alpha", "Alpha", &[], &[])]).unwrap();
        let occupant = OccupantHarness::Agent(r.agent("test-alpha").unwrap());
        let context = crate::harness::context::Harness::from(occupant);
        assert_eq!(OccupantHarness::from(context), occupant);
        assert_eq!(context.as_str(), "test-alpha");
        assert_eq!(serde_json::to_string(&context).unwrap(), "\"Alpha\"");
        assert_eq!(serde_json::to_string(&occupant).unwrap(), "\"test-alpha\"");
        assert_eq!(occupant.cooperative_provenance(), "cooperative_top_level");
        assert_eq!(
            super::super::composer::read_composer(occupant, "› hello", Some(80)),
            super::super::composer::ComposerRead::Unreadable
        );
        assert_eq!(
            super::super::recipe::poke_capabilities(occupant, Some("2.1.287")),
            super::super::recipe::PokeCapabilities::NONE
        );
        assert!(
            crate::cli::hook::observe_harness_in(
                context,
                None,
                std::time::Duration::from_millis(1),
                None
            )
            .unwrap_err()
            .contains("test-alpha")
        );
        assert!(crate::cli::hook_evidence::classify_payload(context, None, b"{}").is_none());
    }
    #[test]
    fn registry_rejects_duplicate_ids_and_host_kinds() {
        assert!(matches!(
            registry(&[
                fixture("test-alpha", "Alpha", &[], &[]),
                fixture("test-alpha", "Beta", &["Other"], &[])
            ]),
            Err(RegistryError::DuplicateId(_))
        ));
        assert!(matches!(
            registry(&[
                fixture("test-alpha", "Alpha", &[], &["claude"]),
                fixture("test-beta", "Beta", &[], &["claude"])
            ]),
            Err(RegistryError::DuplicateHostKind(_))
        ));
        for id in ["", "Bad", "0bad", "a/b", "human"] {
            assert!(matches!(
                registry(&[fixture(id, "Valid", &[], &[])]),
                Err(RegistryError::InvalidId(_))
            ));
        }
        let long = Box::leak("a".repeat(65).into_boxed_str());
        assert!(matches!(
            registry(&[fixture(long, "Valid", &[], &[])]),
            Err(RegistryError::InvalidId(_))
        ));
        for spelling in [long, "Bad/Spelling", ""] {
            assert!(matches!(
                registry(&[fixture("test-alpha", spelling, &[], &[])]),
                Err(RegistryError::InvalidContextSpelling(_))
            ));
        }
        assert!(matches!(
            registry(&[fixture("test-alpha", "Valid", &["Bad/Spelling"], &[])]),
            Err(RegistryError::InvalidContextSpelling(_))
        ));
        assert!(matches!(
            registry(&[fixture("test-alpha", "", &[], &[])]),
            Err(RegistryError::InvalidContextSpelling(_))
        ));
    }
    #[test]
    fn registry_rejects_all_accepted_context_collisions() {
        for (spelling, aliases) in [
            ("Claude", &[][..]),
            ("Codex", &[][..]),
            ("Other", &["Claude"][..]),
        ] {
            let mut regs: Vec<_> = vec![
                Registration::new(&crate::harness::claude::ClaudeAdapter),
                Registration::new(&crate::harness::codex::CodexAdapter),
            ];
            regs.push(Registration::new(fixture(
                "test-alpha",
                spelling,
                aliases,
                &[],
            )));
            assert!(matches!(
                Registry::new(Box::leak(regs.into_boxed_slice())),
                Err(RegistryError::DuplicateContextSpelling(_))
            ));
        }
        assert!(matches!(
            registry(&[fixture("test-alpha", "Alpha", &["Alias", "Alias"], &[])]),
            Err(RegistryError::DuplicateContextSpelling(_))
        ));
        assert!(
            registry(&[
                fixture("test-alpha", "claude", &[], &[]),
                fixture("test-beta", "CLAUDE", &[], &[])
            ])
            .is_ok()
        );
    }
    #[test]
    fn registry_reserves_human_context_namespace() {
        for spelling in ["Human", "human"] {
            assert!(matches!(
                registry(&[fixture("test-alpha", spelling, &[], &[])]),
                Err(RegistryError::ReservedContextSpelling(_))
            ));
            let aliases = Box::leak(vec![spelling].into_boxed_slice());
            assert!(matches!(
                registry(&[fixture("test-alpha", "Alpha", aliases, &[])]),
                Err(RegistryError::ReservedContextSpelling(_))
            ));
        }
    }
    #[test]
    fn two_new_adapters_cannot_share_context_namespace() {
        assert!(matches!(
            registry(&[
                fixture("test-alpha", "Shared", &[], &[]),
                fixture("test-beta", "Beta", &["Shared"], &[])
            ]),
            Err(RegistryError::DuplicateContextSpelling(_))
        ));
        let r = registry(&[
            fixture("test-alpha", "Alpha", &["ALPHA"], &["alpha"]),
            fixture("test-beta", "Beta", &[], &["beta"]),
        ])
        .unwrap();
        let a = r.agent("test-alpha").unwrap();
        assert_eq!(
            r.by_context_spelling("ALPHA").unwrap(),
            OccupantHarness::Agent(a)
        );
        assert_eq!(r.by_id(a).unwrap().metadata().id, "test-alpha");
        assert_eq!(r.by_host_kind("beta").unwrap().metadata().id, "test-beta");
        assert!(r.by_id(builtins().agent("codex").unwrap()).is_err());
    }
    #[test]
    fn cross_registration_handle_is_rejected_before_decode() {
        let a = fixture("codex", "Codex", &[], &[]);
        let r = registry(&[a]).unwrap();
        let first = &r.registrations()[0];
        let second = Box::leak(Box::new(Registration::new(a)));
        let handle = first.admit(&request(), &budget()).unwrap();
        assert_eq!(handle.metadata().id, "codex");
        assert_eq!(handle.kind(), AdmissionKind::Listed);
        assert_eq!(handle.recipe(), "test-recipe");
        assert_eq!(handle.diagnostic(), None);

        assert!(matches!(
            second.decode(&handle, &input()),
            Err(DecodeFailure::RegistrationMismatch)
        ));
        assert!(matches!(
            second.encode(&handle, &event("codex"), &offer()),
            Err(EncodeFailure::RegistrationMismatch)
        ));
        assert_eq!(a.calls.load(Ordering::SeqCst), 0);
        assert_eq!(first.decode(&handle, &input()).unwrap().event_id, "event");
        let EncodedOutput::ContextBearing { bytes } =
            first.encode(&handle, &event("codex"), &offer()).unwrap()
        else {
            panic!("wrong output")
        };
        assert_eq!(bytes, b"17");
    }
    #[test]
    fn same_admission_type_does_not_authorize_another_registration() {
        let a = fixture("codex", "Codex", &[], &[]);
        let b = fixture("claude", "Claude", &[], &[]);
        let r = registry(&[a, b]).unwrap();
        let regs = r.registrations();
        let h = regs[0].admit(&request(), &budget()).unwrap();
        assert!(matches!(
            regs[1].decode(&h, &input()),
            Err(DecodeFailure::RegistrationMismatch)
        ));
        assert!(matches!(
            regs[1].encode(&h, &event("claude"), &offer()),
            Err(EncodeFailure::RegistrationMismatch)
        ));
        assert_eq!(b.calls.load(Ordering::SeqCst), 0);
        let h = regs[1].admit(&request(), &budget()).unwrap();
        // This fixture accidentally labels its event codex: the wrapper refuses it.
        assert!(matches!(
            regs[1].decode(&h, &input()),
            Err(DecodeFailure::RegistrationMismatch)
        ));
        assert_eq!(b.calls.load(Ordering::SeqCst), 1);
    }
}
