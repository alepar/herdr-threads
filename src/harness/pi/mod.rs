//! Pi TUI adapter. The extension delivers bodies; the host guards native composer wake.
mod composer;
pub mod contract;
pub mod decode;
mod delivery;
pub mod launch;
pub mod setup;
pub(crate) mod witness;
use super::adapter::*;
use crate::protocol::time::CallBudget;

pub struct PiAdapter;
#[derive(Debug, Clone, Copy)]
pub struct PiAdmission;
pub const RECIPE: &str = "pi-extension-v1";
static METADATA: AdapterMetadata = AdapterMetadata {
    id: "pi",
    display_label: "pi",
    context_spelling: "Pi",
    context_aliases: &["pi"],
    executable: ExecutableLookup::Path("pi"),
    host_kinds: &["pi"],
    setup_scopes: &[SetupScopeKind::ConfigRoot],
    budget: EventBudgetPolicy {
        lifecycle_ms: 5000,
        observer_ms: 1500,
    },
    runtime_sources: &[],
    agent_env_markers: &[AgentEnvMarker {
        name: "PI_CODING_AGENT",
        person_guard: true,
    }],
};
static TEXT: AdapterText = AdapterText {
    launch_notes: launch::LAUNCH_NOTES,
    setup_scope: "  pi: $PI_CODING_AGENT_DIR, else ~/.pi/agent (extensions/herdr-threads/index.js, skills/herdr-threads/SKILL.md)\n",
    config_root_hint: "PI_CODING_AGENT_DIR",
    ..AdapterText::NONE
};
impl HarnessAdapter for PiAdapter {
    type Admission = PiAdmission;
    fn metadata(&self) -> &'static AdapterMetadata {
        &METADATA
    }
    fn hook_admission_policy(&self) -> HookAdmissionPolicy {
        HookAdmissionPolicy::RegisteredContract
    }
    fn caller_env(&self) -> Option<CallerEnvShape> {
        Some(CallerEnvShape {
            process_names: &[],
            session_var: witness::SESSION,
            role: Some(witness::caller_session),
            process_match: Some(witness::process_matches),
            validate_process_claim: Some(witness::validate_process),
            pid_var: None,
            paneless: None,
            session_marker_vars: &[witness::ROLE, witness::MODE, witness::PID],
        })
    }
    fn child_scrub_vars(&self) -> &'static [&'static str] {
        &[
            "PI_CODING_AGENT_DIR",
            "PI_SESSION_ID",
            "PI_SESSION_FILE",
            witness::ROLE,
            witness::MODE,
            witness::PID,
        ]
    }
    fn output_policy(&self) -> OutputPolicy {
        OutputPolicy {
            channel_owned_delivery: true,
            child_requires_endpoint: true,
            empty_lifecycle: true,
            session_start_hint: true,
            guidance_every_turn: false,
            extra_guidance: "",
        }
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        &contract::DESCRIPTORS
    }
    fn observe_install(&self, env: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
        match crate::cli::hook::resolve_on_path("pi", env.path.as_deref()) {
            Some(binary) => InstallObservation::ExecutableAvailable { binary },
            None => InstallObservation::Unavailable {
                diagnostic: "installed pi executable not found on PATH".into(),
            },
        }
    }
    fn observation_fingerprint(&self, env: &InstallEnvironment) -> Option<String> {
        executable_observation_fingerprint(env, "pi")
    }
    fn observe_daemon(&self, env: &InstallEnvironment, _: &CallBudget) -> DaemonObservation {
        DaemonObservation {
            status: if crate::cli::hook::resolve_on_path("pi", env.path.as_deref()).is_some() {
                HarnessStatus::ContractDeclared {
                    detail: "pi: contract_declared; runtime metadata unavailable".into(),
                }
            } else {
                HarnessStatus::NotInstalled("no executable `pi` on the daemon's PATH".into())
            },
            receipt_basis: Some(
                crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE.into(),
            ),
            ..Default::default()
        }
    }
    fn receipt_admission_summary(&self) -> Option<String> {
        Some("declared".into())
    }
    fn admit(&self, request: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<PiAdmission> {
        match request.installed {
            InstallObservation::NotRequested | InstallObservation::ExecutableAvailable { .. } => {
                AdmissionDecision::ContractDeclared {
                    state: PiAdmission,
                    recipe: RECIPE,
                }
            }
            _ => AdmissionDecision::Refused {
                diagnostic: "pi requires executable availability or declared hook selection".into(),
            },
        }
    }
    fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
        Ladder::Admitted
    }
    fn classify(&self, input: &HookInput) -> ContractObservation {
        contract::classify(input)
    }
    fn decode(
        &self,
        admitted: &PiAdmission,
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure> {
        decode::decode(admitted, input)
    }
    fn encode(
        &self,
        admitted: &PiAdmission,
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        decode::encode(admitted, event, offer)
    }
    fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
        RuntimeAttribution::Unavailable {
            diagnostic: "pi payload carries no runtime version".into(),
        }
    }
    fn local_evidence(
        &self,
        request: &LocalEvidenceRequest<'_>,
    ) -> Result<LocalEvidence, DecodeFailure> {
        witness::local_evidence(request.input)
    }
    fn setup_environment_inputs(&self) -> &'static [&'static str] {
        &["PI_CODING_AGENT_DIR"]
    }
    fn default_config_root(&self, env: &SetupEnvironment) -> Option<std::path::PathBuf> {
        setup::default_config_root(env)
    }
    fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        env: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        match request {
            SetupScopeRequest::Default => env
                .config_roots
                .get("pi")
                .cloned()
                .or_else(|| setup::default_config_root(env))
                .map(ResolvedSetupScope::ConfigRoot)
                .ok_or_else(|| {
                    SetupFailure::Invalid("pi: agent directory unavailable (HOME unset)".into())
                }),
            SetupScopeRequest::Profile(_) => Err(SetupFailure::Invalid(
                "pi: named profile is unsupported; use an explicit profile directory".into(),
            )),
        }
    }
    fn setup(&self, request: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
        setup::setup(request)
    }
    fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
        setup::status(request)
    }
    fn unsetup(
        &self,
        request: &UnsetupRequest,
        _: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        setup::unsetup(request)
    }
    fn setup_status_extras(&self, request: &StatusRequest, projection: &mut serde_json::Value) {
        setup::status_extras(request, projection)
    }
    fn setup_report_lines(&self, report: &serde_json::Value) -> Vec<String> {
        setup::report_lines(report)
    }
    fn doctor_projection(
        &self,
        request: &StatusRequest,
        _: &serde_json::Value,
        _: &CallBudget,
    ) -> Option<DoctorProjection> {
        setup::doctor_projection(request)
    }
    fn doctor_text(&self, hooks: &serde_json::Value, form: DoctorTextForm) -> String {
        setup::doctor_text(hooks, form)
    }
    fn text(&self) -> &'static AdapterText {
        &TEXT
    }
    fn delivery_policy(&self) -> Option<&dyn DeliveryPolicy> {
        Some(&delivery::PiDelivery)
    }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        Some(&composer::PiComposer)
    }
    fn turn_state_edge(&self, event: &DecodedEvent) -> Option<TurnEdge> {
        match event.metadata.native_event.as_str() {
            "TurnStart" => Some(TurnEdge::Busy),
            "TurnEnd" => Some(TurnEdge::Idle),
            _ => None,
        }
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        Some(&launch::PiLaunch)
    }
    fn binding_turn_delivery(&self, _: Option<&str>) -> bool {
        false
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/pi.rs"]
mod tests;
