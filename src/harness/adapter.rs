//! In-memory author contract. Observations grant no seat or receipt authority.
use crate::protocol::time::CallBudget;
use std::ffi::OsString;

/// Registered static representation of one JSON string; never caller-supplied
/// native bytes, a callback, or an authority/admission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationCodec {
    JsonStringEnvelope {
        prefix: &'static str,
        suffix: &'static str,
    },
}

pub trait HarnessAdapter: Send + Sync + 'static {
    type Admission: Send + Sync + 'static;
    /// Pure static presentation descriptor. Unsupported selectors grant no V2
    /// Hook representation; callers may retain historical delivery instead.
    fn presentation_codec(&self, _selector: u8) -> Option<PresentationCodec> {
        None
    }
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
    /// Selects the sole source of hook admission. Callback input is supplied
    /// only for startup-qualified adapters; the policy itself grants no authority.
    fn hook_admission_policy(&self) -> HookAdmissionPolicy {
        HookAdmissionPolicy::InstalledObservation
    }
    fn qualified_turn_policy(&self) -> super::context::QualifiedTurnPolicy {
        super::context::QualifiedTurnPolicy::Strict
    }
    /// Which decoded events may enroll a seat. Grants no authority by itself.
    fn enrollment_policy(&self) -> EnrollmentPolicy {
        EnrollmentPolicy::Lifecycle
    }
    /// How a CLI call made inside a session of this harness names its session from the process
    /// environment and ancestry (session-keyed bindings, spec §1). `None`: the generic caller
    /// derivation does not know this harness's shape.
    fn caller_env(&self) -> Option<CallerEnvShape> {
        None
    }
    /// How the daemon and the client treat this harness's subagent and child sessions.
    fn binding_policy(&self) -> BindingPolicy {
        BindingPolicy::default()
    }
    /// The hook events this harness's installed integration must register for herdr-threads to
    /// work; a missing one marks the install stale.
    fn required_hook_events(&self) -> Vec<&'static str> {
        Vec::new()
    }
    /// A daemon-free rewrite of this harness's hook stdout, applied to every outcome. `None`:
    /// leave `existing_stdout` as it is.
    fn rewrite_hook_output(&self, _input: &[u8], _existing_stdout: &[u8]) -> Option<Vec<u8>> {
        None
    }
    fn evidence_observations(&self, input: &HookInput) -> Vec<EvidenceProjection> {
        let observed = self.classify(input);
        self.contracts()
            .iter()
            .find(|d| d.domain == observed.domain)
            .map(|d| {
                vec![EvidenceProjection {
                    domain: d.domain,
                    origin: d.origin,
                    contract_id: d.contract_id_v2().unwrap_or_default(),
                    classification: observed.classification,
                }]
            })
            .unwrap_or_default()
    }
    fn nonholding_unavailable_reasons(&self, _: &ContractDescriptor) -> &'static [&'static str] {
        &[]
    }
    fn contracts(&self) -> &'static [ContractDescriptor];
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation;
    /// None means observation reuse is unsafe. Implementers include every observed
    /// input (profile/config/assets as applicable), not merely the executable.
    fn observation_fingerprint(&self, _: &InstallEnvironment) -> Option<String> {
        None
    }
    fn observe_daemon(&self, env: &InstallEnvironment, budget: &CallBudget) -> DaemonObservation {
        observe_daemon_default(self, env, budget)
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
    /// Cooperative facts about this exact input/runtime/domain, never native attestation.
    /// The default supplies no qualification, including for unavailable runtime identity.
    fn evidence_qualifications(
        &self,
        _: &EvidenceQualificationRequest<'_>,
        _: &CallBudget,
    ) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
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
    /// Resolve one local operation with the caller's captured inputs/deadline.
    fn resolve_setup_scope_for(
        &self,
        request: &SetupScopeResolutionRequest<'_>,
        budget: &CallBudget,
    ) -> Result<SetupScopeResolution, SetupFailure> {
        check_setup_budget(request.environment, budget)?;
        let scope = self.resolve_setup_scope(request.selector, request.environment)?;
        check_setup_budget(request.environment, budget)?;
        Ok(SetupScopeResolution {
            scope,
            removal_generation: None,
        })
    }
    /// Legacy adapters cannot silently consume an owned-generation receipt.
    fn unsetup_resolved(
        &self,
        request: &UnsetupRequest,
        resolution: &SetupScopeResolution,
        budget: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        check_setup_budget(&request.environment, budget)?;
        if request.scope != resolution.scope || resolution.removal_generation.is_some() {
            return Err(SetupFailure::Invalid(
                "unsupported or mismatched removal resolution".into(),
            ));
        }
        self.unsetup(request, budget)
    }
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
    fn installer_policy(&self) -> Option<&dyn InstallerPolicy> {
        None
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        None
    }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        None
    }
    /// Native runtime artifacts about the event's session, read by the runner after the pure
    /// decode and folded with [`LocalEvidence::refine`]. Default: none.
    fn local_evidence(&self, _: &LocalEvidenceRequest<'_>) -> LocalEvidence {
        LocalEvidence::default()
    }
    /// A step the runner runs for a top-level `Observer` event in place of the neutral output.
    fn hook_step(&self) -> Option<&dyn HookStep> {
        None
    }
    /// `None`: hooks deliver and the native wake applies (delivery-policy DP1).
    fn delivery_policy(&self) -> Option<&dyn DeliveryPolicy> {
        None
    }
    /// Native wake text selected from canonical evidence for the reached binding.
    /// Missing evidence keeps the policy's conservative default. This selects text only,
    /// never delivery eligibility or receipt authority.
    fn binding_wake_text(&self, _provenance: Option<&str>) -> &'static str {
        self.delivery_policy()
            .map_or(crate::notification::policy::MARKER, |p| p.wake_text())
    }
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> {
        None
    }
    /// Static text fragments the neutral core splices into help and reports.
    fn text(&self) -> &'static AdapterText {
        &AdapterText::NONE
    }
    /// The config root when the environment names none explicitly: the declared
    /// environment variable when non-empty, else a directory under `home`.
    fn default_config_root(&self, _environment: &SetupEnvironment) -> Option<std::path::PathBuf> {
        None
    }
    /// The user settings file this adapter's installation edits through the shared JSON engine.
    fn settings_kind(&self) -> Option<&'static super::setup::SettingsKindSpec> {
        None
    }
    fn foreground_advice(&self) -> Option<&'static ForegroundAdvice> {
        None
    }
    /// `setup-status` only (never doctor/launch status): adapter-local enrichments.
    fn setup_status_extras(&self, _: &StatusRequest, _: &mut serde_json::Value) {}
    /// Extra `setup` text lines rendered as `<id>: <line>` after the outcome line.
    fn setup_report_lines(&self, _: &serde_json::Value) -> Vec<String> {
        Vec::new()
    }
    /// Text for this adapter's `report.hooks.<id>` object; empty renders nothing.
    fn doctor_text(&self, _: &serde_json::Value, _: DoctorTextForm) -> String {
        String::new()
    }
    /// Prefixes of report-level limitations the Summary text already printed.
    fn doctor_summary_claims(&self) -> &'static [&'static str] {
        &[]
    }
    fn version_syntax(&self) -> Option<&'static VersionSyntax> {
        None
    }
    fn transcript_version(&self) -> Option<&'static TranscriptVersionSpec> {
        None
    }
}

/// A harness's caller shape for environment attribution (spec §1, decision 15): the process the
/// harness runs as and the variable its children inherit naming the native session. A caller
/// under such a process whose variable is unset is refused (no session id), never attributed to
/// the pane. Recorded as `cooperative_env_session` (TRUST-POLICY A3); never verified.
#[derive(Debug, Clone, Copy)]
pub struct CallerEnvShape {
    /// argv[0] basenames of the harness's own process.
    pub process_names: &'static [&'static str],
    /// Environment variable carrying the native session id in the harness's descendants.
    pub session_var: &'static str,
    /// Client-local evidence of the session's role, given its session id and the caller's
    /// environment (cooperative, never authority: the daemon decides the claim against A2).
    /// `None`: every session is top-level.
    pub role: Option<CallerRoleWitness>,
    /// Other process shapes of the harness beyond the argv[0] names (argv contents, interpreter
    /// launchers).
    pub process_match: Option<fn(&crate::ports::ProcessRecord) -> bool>,
    /// A variable in which the harness exports its own pid to its descendants: a process with
    /// that pid is this harness, and its record is preferred for the fingerprint while it exists.
    pub pid_var: Option<&'static str>,
    /// A process of this harness that has no pane (a managed daemon): a caller under it is
    /// refused `managed_daemon`.
    pub paneless: Option<PanelessProcess>,
}

/// A process of a harness that has no pane, and the refusal text for a caller under it.
#[derive(Debug, Clone, Copy)]
pub struct PanelessProcess {
    pub matches: fn(&crate::ports::ProcessRecord) -> bool,
    /// The refusal message, given the pid of the paneless process.
    pub refusal: fn(u32) -> String,
}

/// Adapter-declared facts about how a harness's sessions bind, used by the client and the daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindingPolicy {
    /// The harness's own hook passes a subagent's id to the CLI as `--agent-id`: the client
    /// honours `--agent-id` only for such a harness, and its subagent bindings are recorded
    /// `cooperative_hook_agent` (else `cooperative_env_session`).
    pub hook_agent_id: bool,
    /// Child sessions are registered by their parent as subagent bindings keyed
    /// `(seat, harness, child, agent_id = child)`; a top-level-shaped claim naming the child
    /// session resolves to that binding.
    pub registered_child_sessions: bool,
}

/// `CallerEnvShape::role`: the session id and the caller's environment lookup.
pub type CallerRoleWitness = fn(&str, &dyn Fn(&str) -> Option<String>) -> CallerSession;

/// What a harness's local evidence says about the session a CLI call names (spec §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerSession {
    TopLevel,
    /// A child session of the top-level session `parent`: the call is a subagent call whose agent
    /// id is the child's own session id.
    Subagent {
        parent: String,
    },
    /// A subagent of the named session whose agent id is `agent`.
    Marked {
        agent: String,
    },
    /// A subagent marker in `var` is present but unparseable: refused, never read as top-level.
    MalformedMarker {
        var: &'static str,
    },
    /// The evidence cannot tell: refused, never read as top-level (fails toward child).
    Unattributable(String),
}

/// One environment variable that marks an agent process of this harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEnvMarker {
    /// Exact env var name, `[A-Z][A-Z0-9_]{0,63}`. A non-empty value marks an agent process
    /// (output form).
    pub name: &'static str,
    /// Also TRUST-POLICY A4 person-pane refusal evidence (presence suffices).
    pub person_guard: bool,
}

/// Static text fragments of one adapter. Adding a field later touches no adapter: every
/// adapter spells `..AdapterText::NONE`.
#[derive(Debug, Clone, Copy)]
pub struct AdapterText {
    /// Verbatim block under the setup help "Scope:" heading (its own indentation).
    pub setup_scope: &'static str,
    /// Sentence(s) in the setup help "Foreground agents" paragraph.
    pub setup_foreground: &'static str,
    /// Own paragraph(s) after it (Claude prompt suggestions).
    pub setup_notes: &'static str,
    /// Paragraph(s) appended to the launch help.
    pub launch_notes: &'static str,
    /// For example `CLAUDE_CONFIG_DIR`; used in launch and setup messages.
    pub config_root_hint: &'static str,
    /// For example `codex hook trust`; empty renders `<id> setup trust`.
    pub trust_reminder_label: &'static str,
    /// The per-version note of the harness-state report; empty renders none.
    pub version_history_note: &'static str,
}
impl AdapterText {
    pub const NONE: Self = Self {
        setup_scope: "",
        setup_foreground: "",
        setup_notes: "",
        launch_notes: "",
        config_root_hint: "",
        trust_reminder_label: "",
        version_history_note: "",
    };
    /// Every field, for registry validation.
    pub fn fields(&self) -> [&'static str; 7] {
        [
            self.setup_scope,
            self.setup_foreground,
            self.setup_notes,
            self.launch_notes,
            self.config_root_hint,
            self.trust_reminder_label,
            self.version_history_note,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorTextForm {
    Debug,
    Summary,
}

/// Read-only foreground-agent advice of one harness (`setup`/`setup-status` reports).
pub struct ForegroundAdvice {
    /// Relative to the adapter's config root.
    pub settings_file: &'static str,
    pub advice: &'static str,
    /// For example `("persistent_opt_out", "...")`.
    pub fixed_fields: &'static [(&'static str, &'static str)],
    pub inspect: fn(&[u8], &mut serde_json::Value) -> Result<(), String>,
    /// Optional read-only inspection of a persistent foreground choice outside the settings
    /// file (for example an owned shell-startup block), run after the settings read. It may set
    /// `status` to `configured`; it never proves execution and grants no authority.
    pub inspect_environment: Option<fn(&SetupEnvironment, &mut serde_json::Value)>,
}

/// How `--version` output of one harness is normalized to `X.Y.Z`.
pub struct VersionSyntax {
    /// Stripped when present: Codex `codex-cli `.
    pub prefix: &'static str,
    /// Stripped when present: Claude ` (Claude Code)`.
    pub suffix: &'static str,
    /// `(raw, normalized)` examples; the contract-id listing renders them.
    pub examples: &'static [(&'static str, &'static str)],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptRead {
    NewestEntry,
    HeadLine,
}
/// Where a transcript names the runtime version of its harness.
pub struct TranscriptVersionSpec {
    pub read: TranscriptRead,
    pub extract: fn(&serde_json::Value) -> Option<&str>,
    /// `false`: the transcript names only its creator (Codex): payload attribution is
    /// `CreatorOnly`, a known resume `ResumedCreator`. `true`: resume is `ResumeBeforeFirstEntry`.
    pub current_runtime: bool,
}

/// How a harness receives pending items (delivery-policy DP1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMechanism {
    PushChannel,
    HookPull,
}
/// What wakes an idle seat of this harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleWake {
    Native,
    Channel,
}
/// Delivery declaration of one adapter; `Registry::new` validates the
/// `(mechanism, provenance, idle_wake)` triple against the allowed pairings.
pub trait DeliveryPolicy: Send + Sync + 'static {
    fn mechanism(&self) -> DeliveryMechanism;
    /// An A3 provenance constant.
    fn provenance(&self) -> &'static str;
    fn idle_wake(&self) -> IdleWake;
    /// The prompt a native idle wake types. Default: the marker that names `inbox`, for a harness
    /// whose turn start injects nothing. A policy whose turn-start hook or channel injects the
    /// pending items returns [`crate::notification::policy::WAKE_UP`].
    fn wake_text(&self) -> &'static str {
        crate::notification::policy::MARKER
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
    /// Environment variables that mark an agent process of this harness.
    pub agent_env_markers: &'static [AgentEnvMarker],
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAdmissionPolicy {
    InstalledObservation,
    RegisteredContract,
    QualifiedCallback,
}
pub enum InstallObservation {
    /// Executable availability alone, without invoking it or identifying its runtime.
    ExecutableAvailable {
        binary: std::path::PathBuf,
    },
    /// Hook-only selection under an explicit registered-contract policy.
    NotRequested,
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
    /// The admitting caller's clock reading (a hook passes its injected clock's), against
    /// which an adapter measures a callback window the input carries. `None`: the caller
    /// has no run clock, and the adapter reads the host's.
    pub admitted_at: Option<AdmissionInstant>,
}
/// One reading of the admitting caller's clock, both domains at the same present: an
/// adapter validates a callback window in UTC and anchors what remains of it on `instant`.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionInstant {
    pub utc: crate::protocol::time::UtcMillis,
    pub instant: std::time::Instant,
}
pub enum AdmissionDecision<A> {
    ContractDeclared {
        state: A,
        recipe: &'static str,
    },
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
    ContractDeclared,
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
pub struct EvidenceQualificationRequest<'a> {
    pub input: &'a HookInput,
    pub runtime: &'a RuntimeIdentity,
    pub descriptor: &'a ContractDescriptor,
}
pub struct EvidenceProjection {
    pub domain: ContractDomain,
    pub origin: super::evidence::EvidenceOrigin,
    pub contract_id: String,
    pub classification: crate::harness::contract::Classification,
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
#[derive(Clone)]
pub enum EventIntent {
    Lifecycle(crate::harness::context::EventKind),
    Current,
    QualifiedTurn(crate::harness::context::QualifiedTurn),
    /// A witnessed top-level session start: Hermes' CLI `on_session_start` with the
    /// bridge's enrollment witness. It enrolls the pane through the ordinary resolver
    /// without a check-in; the first qualified turn checks in.
    Enroll,
    Observer,
    DeclaredReset(super::context::DeclaredReset),
}
/// What an adapter lets enroll a seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentPolicy {
    /// Only original admitted lifecycle intent enrolls.
    Lifecycle,
    /// Lifecycle, plus a witnessed session start and a witnessed turn on an unowned pane.
    WitnessedStartAndUnownedTurn,
}
/// Why a decoded event may enroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentKind {
    Lifecycle,
    SessionStart,
    /// Eligible for the Unowned fallback; the seat check belongs to the caller.
    UnownedTurn,
}
/// Who vouched that the event came from an interactive top-level session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentWitness {
    CliPlatform,
}
#[derive(Clone)]
pub struct EventMetadata {
    pub enrollment_witness: Option<EnrollmentWitness>,
    pub skill_pointer: bool,
    /// An adapter callback can shorten the core end-to-end limit: the native callback's
    /// deadline on the admitting clock's monotonic domain, anchored at admission, so a later
    /// wall-clock step never grows it.
    pub callback_deadline: Option<std::time::Instant>,
    pub context_source: String,
    pub capability: super::Capability,
    pub domain: ContractDomain,
    pub native_event: String,
    pub shape_fields: Vec<String>,
    /// The subagent id a native event carried (see `LifecycleEvent::agent_id`).
    pub agent_id: Option<String>,
}
#[derive(Clone)]
pub enum DeliveryEligibility {
    Context,
    ObserverOnly,
    Ineligible,
}
#[derive(Clone)]
pub enum RuntimeAttribution {
    Attributed(RuntimeIdentity),
    Unavailable { diagnostic: String },
}
/// A decoded harness start/stop/end event that enrolls or disconnects a binding (spec §2 Enrollment,
/// §3 end events). Carried next to the event's ordinary intent: a Claude or Codex `SubagentStart`
/// still renders its child guidance after the binding is enrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingEventDecode {
    pub kind: crate::protocol::commands::BindingEventKind,
    /// The subagent's agent id (Claude and Codex `agent_id`); `None` for a session end and for a
    /// Hermes delegate, which is known by its child session.
    pub agent_id: Option<String>,
    /// Hermes only: the delegate child's own session id.
    pub child_session: Option<String>,
    /// The session the claim names when it is not the event's own native session: a Hermes
    /// delegate event is about the child session, its claim is the parent's.
    pub claim_session: Option<String>,
}
impl BindingEventDecode {
    /// The event a native Claude or Codex hook payload (`hook_event_name`, `agent_id`) carries,
    /// if it is a start/stop/end event.
    pub fn from_native_payload(payload: &serde_json::Value) -> Option<Self> {
        use crate::protocol::commands::BindingEventKind as Kind;
        let kind = match payload.get("hook_event_name")?.as_str()? {
            "SubagentStart" => Kind::SubagentStart,
            "SubagentStop" => Kind::SubagentStop,
            "SessionEnd" => Kind::SessionEnd,
            _ => return None,
        };
        let agent_id = payload
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        // A subagent event names its agent; a session end is the root's (an `agent_id` on one is
        // a malformed payload and is not trusted to disconnect anything).
        (agent_id.is_some() == (kind != Kind::SessionEnd)).then_some(Self {
            kind,
            agent_id,
            child_session: None,
            claim_session: None,
        })
    }
}
#[derive(Clone)]
pub struct DecodedEvent {
    pub harness: crate::harness::registry::AgentHarnessId,
    pub role: EventRole,
    pub native_session: Option<String>,
    pub event_id: String,
    pub intent: EventIntent,
    pub metadata: EventMetadata,
    pub delivery: DeliveryEligibility,
    pub runtime: RuntimeAttribution,
    /// The binding enrollment or disconnect this event reports, if any.
    pub binding_event: Option<BindingEventDecode>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Witness {
    Present,
    Absent,
    Unchecked,
    Unreadable,
}
/// Native runtime artifacts about one session's role, mapped by its adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleWitness {
    /// The runtime's own top-level marker for this session exists.
    pub top_level_marker: Witness,
    /// A runtime child listing names this session under a candidate parent session.
    pub child_listing: Witness,
}
impl RoleWitness {
    /// Fails toward child: top-level only on a present marker and an examined, absent listing.
    pub fn decide(&self) -> EventRole {
        if self.top_level_marker == Witness::Present && self.child_listing == Witness::Absent {
            EventRole::TopLevel
        } else {
            EventRole::Subagent
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalEvidence {
    pub role: Option<RoleWitness>,
    /// Adapter reading of a turn-start that fires no session-start: Startup | Clear | Resume only.
    pub lifecycle: Option<crate::harness::context::EventKind>,
}
impl LocalEvidence {
    /// Pure fold applied by the runner. It never raises `Subagent`, and `Unknown` becomes
    /// `TopLevel` only on positive witnesses.
    pub fn refine(&self, event: &mut DecodedEvent) {
        use crate::harness::context::EventKind;
        if let Some(witness) = &self.role
            && event.role != EventRole::Subagent
        {
            event.role = witness.decide();
        }
        if let Some(kind) = self.lifecycle
            && matches!(
                kind,
                EventKind::Startup | EventKind::Clear | EventKind::Resume
            )
            && event.role == EventRole::TopLevel
            && matches!(
                event.intent,
                EventIntent::Observer | EventIntent::Lifecycle(_)
            )
        {
            event.intent = EventIntent::Lifecycle(kind);
            event.delivery = DeliveryEligibility::Context;
            // Preserve the native lifecycle source. A resume retains C1's
            // resolved-pane probe; other events claim same-session recovery
            // only when no resolved mapping exists.
            event.metadata.context_source = match kind {
                EventKind::Clear => "clear",
                EventKind::Resume => crate::protocol::commands::ContinuityCheckIn::RESUME_SOURCE,
                _ => "startup",
            }
            .into();
        }
    }
}
pub struct LocalEvidenceRequest<'a> {
    pub input: &'a HookInput,
    /// The hook process's HOME (the harness's own environment).
    pub home: Option<&'a std::ffi::OsStr>,
    /// `<node_dir>/adapter-state/<id>/` (read-only here).
    pub state_dir: Option<&'a std::path::Path>,
    /// Lazy client-local hint (A2: selects what to ask, never authorizes): the native session of
    /// the pane's current context for this harness, or None.
    pub pane_session: &'a dyn Fn() -> Option<String>,
    pub deadline: std::time::Instant,
}
pub trait HookStep: Send + Sync {
    /// Complete native stdout for one top-level `Observer` event; empty means none.
    /// Fails open: every internal error returns empty bytes. Must finish by `context.deadline`.
    fn run(&self, context: &HookStepContext<'_>) -> Vec<u8>;
    /// Prepare stdout plus adapter-owned completion, without claiming it was emitted.
    fn run_with_completion(&self, context: &HookStepContext<'_>) -> HookStepOutput {
        HookStepOutput {
            stdout: self.run(context),
            completion: None,
            diagnostic: None,
        }
    }
    /// Invoked only after all prepared stdout was written and flushed. The daemon
    /// remains pinned to the original instance and boot; failure leaves work replayable.
    fn complete(
        &self,
        _payload: &[u8],
        _daemon: &dyn HookStepDaemon,
        _state_dir: &std::path::Path,
        _deadline: std::time::Instant,
    ) {
    }
}
/// Private adapter protocol bytes, bounded before any output is emitted.
#[derive(Debug, PartialEq, Eq)]
pub struct HookStepCompletionPayload(pub(crate) Vec<u8>);
impl HookStepCompletionPayload {
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        (bytes.len() <= 65_536).then_some(Self(bytes))
    }
}
pub struct HookStepOutput {
    pub stdout: Vec<u8>,
    pub completion: Option<HookStepCompletionPayload>,
    /// Bounded adapter failure detail for stderr, never model context.
    pub diagnostic: Option<String>,
}
pub struct HookStepContext<'a> {
    pub input: &'a HookInput,
    pub decoded: &'a DecodedEvent,
    /// `bridge::caller_claim` of the pane context when its harness and native session equal the
    /// event's (client-local hint; the daemon re-decides each request against A2).
    pub claim: Option<crate::protocol::authority::CallerClaim>,
    /// A running daemon of this instance, or None. Never starts one.
    pub daemon: Option<&'a dyn HookStepDaemon>,
    /// `<node_dir>/adapter-state/<id>/`, created 0700 by the runner on first use.
    pub state_dir: Option<std::path::PathBuf>,
    /// `cli::hook::cli_prefix` argv for commands printed to the model.
    pub command_prefix: Vec<String>,
    pub deadline: std::time::Instant,
}
pub trait HookStepDaemon {
    fn supports_capability(&self, _name: &str) -> bool {
        false
    }
    fn call(
        &self,
        command: crate::protocol::commands::Command,
    ) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError>;
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
    /// Explicit canonical launch selection, distinct from inherited config hints.
    /// Adapters resolve this captured directory without a later pane-shell override.
    pub explicit_profile_directory: Option<std::path::PathBuf>,
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
            explicit_profile_directory: None,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupScopeOperation {
    Install,
    Status,
    Remove,
}
#[derive(Debug)]
pub struct SetupScopeResolutionRequest<'a> {
    pub operation: SetupScopeOperation,
    pub selector: &'a SetupScopeRequest,
    pub native_binary: Option<&'a std::path::Path>,
    pub environment: &'a SetupEnvironment,
}
/// Local ownership generation only; conveys no native/runtime qualification.
#[derive(Debug, Clone)]
pub struct SetupScopeResolution {
    pub scope: ResolvedSetupScope,
    pub removal_generation: Option<String>,
}
pub(crate) fn check_setup_budget(
    environment: &SetupEnvironment,
    budget: &CallBudget,
) -> Result<(), SetupFailure> {
    if budget.cancellation.is_cancelled() || budget.deadline_passed(environment.clock.as_ref()) {
        return Err(SetupFailure::Invalid(
            "local operation deadline elapsed or request cancelled".into(),
        ));
    }
    Ok(())
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
    pub hook: Option<crate::ports::ConfiguredHook>,
    pub working_directory: std::path::PathBuf,
    pub environment_overrides: std::collections::BTreeMap<String, OsString>,
    pub report: serde_json::Value,
}
/// Opaque startup text data; composition declares grammar without inspecting text.
#[derive(Debug, Clone, Copy)]
pub struct StartupInputSpec {
    pub max_text_bytes: usize,
}

/// Exactly one text argument inserted among unchanged caller tokens.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupInputTemplate {
    pub insertion_index: usize,
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub prefix: String,
    pub suffix: String,
    pub max_arg_bytes: usize,
}
impl StartupInputTemplate {
    pub fn positional(insertion_index: usize) -> Self {
        Self {
            insertion_index,
            before: vec![],
            after: vec![],
            prefix: String::new(),
            suffix: String::new(),
            max_arg_bytes: crate::ports::NativeLaunchRequest::MAX_ARG_BYTES,
        }
    }
    pub fn validate(&self, caller_len: usize) -> Result<(), crate::protocol::results::ApiError> {
        use crate::protocol::results::{ApiError, ErrorCode};
        let invalid = || ApiError::new(ErrorCode::InvalidRequest, "invalid startup input template");
        if self.insertion_index > caller_len
            || self.max_arg_bytes == 0
            || self.max_arg_bytes > crate::ports::NativeLaunchRequest::MAX_ARG_BYTES
            || self
                .before
                .len()
                .checked_add(self.after.len())
                .is_none_or(|count| count > 8)
            || self.prefix.len() > 128
            || self.suffix.len() > 128
            || self
                .prefix
                .chars()
                .chain(self.suffix.chars())
                .any(char::is_control)
        {
            return Err(invalid());
        }
        let mut bytes = 0usize;
        for token in self.before.iter().chain(&self.after) {
            if token.is_empty() || token.len() > 4096 || token.chars().any(char::is_control) {
                return Err(invalid());
            }
            bytes = bytes.checked_add(token.len()).ok_or_else(invalid)?;
        }
        if bytes > 16384 {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn apply(
        &self,
        caller: &[String],
        text: &str,
    ) -> Result<(Vec<String>, usize), crate::protocol::results::ApiError> {
        use crate::protocol::results::{ApiError, ErrorCode};
        self.validate(caller.len())?;
        let bytes = self
            .prefix
            .len()
            .checked_add(text.len())
            .and_then(|n| n.checked_add(self.suffix.len()));
        if bytes.is_none_or(|n| n > self.max_arg_bytes) {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "startup input exceeds complete native argument limit",
            ));
        }
        let mut argv = caller[..self.insertion_index].to_vec();
        argv.extend(self.before.iter().cloned());
        let slot = argv.len();
        argv.push(format!("{}{}{}", self.prefix, text, self.suffix));
        argv.extend(self.after.iter().cloned());
        argv.extend_from_slice(&caller[self.insertion_index..]);
        crate::ports::validate_native_argv(&argv)
            .map_err(|detail| ApiError::new(ErrorCode::InvalidRequest, detail))?;
        Ok((argv, slot))
    }
}

pub trait LaunchPolicy: Send + Sync {
    /// Adapter-owned projection for an explicit canonical profile directory.
    /// Consumers validate the bounded projection with `LaunchDirectoryContext`.
    fn explicit_directory_env(
        &self,
        _: &std::path::Path,
    ) -> Result<std::collections::BTreeMap<String, String>, crate::protocol::results::ApiError>
    {
        Err(crate::protocol::results::ApiError::unsupported(
            "explicit profile directory unavailable",
        ))
    }
    /// Optional adapter-owned model selection: the caller argv with the harness's own
    /// model option for `model` applied. The model passes through unchanged, never checked
    /// against a known list or version; absence is an honest refusal.
    fn apply_model(
        &self,
        _: Vec<String>,
        _: &str,
    ) -> Result<Vec<String>, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "selected harness has no model selection",
        ))
    }
    /// Optional pure original-caller grammar. Absence is honest unsupported handoff.
    fn prepare_startup_input(
        &self,
        _: &[String],
        _: &StartupInputSpec,
    ) -> Result<Option<StartupInputTemplate>, crate::protocol::results::ApiError> {
        Ok(None)
    }
    /// Optional adapter-owned configured arguments; absent providers inherit no key.
    /// Declarations must be ASCII, at most 128 bytes, and match
    /// HERDR_THREADS_[A-Z0-9_]+_OPTS with a nonempty middle. Consumers validate
    /// before environment lookup or recovery serialization; no key is inferred.
    fn native_options_env(&self) -> Option<&'static str> {
        None
    }
    /// Cooperative host recognition input; never caller or runtime authority.
    fn requires_process_hint(&self) -> bool {
        false
    }
    /// Separately observed launch-only API/profile facts, never callback admission.
    fn uses_prelaunch_observation(&self) -> bool {
        false
    }
    fn observe_prelaunch(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
        _: &CallBudget,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "prelaunch observation unavailable",
        ))
    }
    fn prepare_prelaunch(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
        _: &(dyn std::any::Any + Send + Sync),
        _: &LocalSetupStatus,
        _: &dyn super::launch::PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchPreparation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "prelaunch preparation unavailable",
        ))
    }
    fn recheck_prelaunch(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
        _: &(dyn std::any::Any + Send + Sync),
        _: &CallBudget,
    ) -> Result<String, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "prelaunch recheck unavailable",
        ))
    }
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        probe: &dyn super::launch::PaneShellProbe,
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
    ) -> Result<Vec<String>, crate::protocol::results::ApiError>;
    fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &super::registry::AdmittedHandle,
        status: &LocalSetupStatus,
        probe: &dyn super::launch::PaneShellProbe,
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
    /// The enrollment this event earns under `policy`, if any.
    pub fn enrollment(&self, policy: EnrollmentPolicy) -> Option<EnrollmentKind> {
        use super::context::EventKind;
        let top = matches!(self.role, EventRole::TopLevel);
        // A qualified turn's normalized Startup/Clear is not lifecycle evidence.
        if matches!(
            self.intent,
            EventIntent::Lifecycle(
                EventKind::Fork | EventKind::Startup | EventKind::Clear | EventKind::Resume
            )
        ) && top
            && matches!(self.delivery, DeliveryEligibility::Context)
            && self.can_check_in()
        {
            return Some(EnrollmentKind::Lifecycle);
        }
        if policy != EnrollmentPolicy::WitnessedStartAndUnownedTurn
            || !top
            || self.metadata.enrollment_witness != Some(EnrollmentWitness::CliPlatform)
        {
            return None;
        }
        match self.intent {
            EventIntent::Enroll => Some(EnrollmentKind::SessionStart),
            EventIntent::QualifiedTurn(_)
                if matches!(self.delivery, DeliveryEligibility::Context) =>
            {
                Some(EnrollmentKind::UnownedTurn)
            }
            _ => None,
        }
    }
}

/// The default daemon observation: install availability, then admission.
pub fn observe_daemon_default<A: HarnessAdapter + ?Sized>(
    adapter: &A,
    env: &InstallEnvironment,
    budget: &CallBudget,
) -> DaemonObservation {
    let installed = adapter.observe_install(env, budget);
    let identity = match &installed {
        InstallObservation::Available { identity, .. } => Some(identity.clone()),
        _ => None,
    };
    let status = match &installed {
        InstallObservation::Unavailable { diagnostic } => {
            HarnessStatus::Refused(diagnostic.clone())
        }
        InstallObservation::Unsupported(operation) => HarnessStatus::Refused(operation.to_string()),
        _ => match adapter.admit(
            &AdmissionRequest {
                installed,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            budget,
        ) {
            AdmissionDecision::ContractDeclared { recipe, .. } => HarnessStatus::ContractDeclared {
                detail: format!(
                    "{recipe}; contract_declared; runtime metadata unavailable; rich optional capabilities unavailable"
                ),
            },
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
        let root = crate::test_support::scratch::temp_path(format!(
            "local-status-{}",
            uuid::Uuid::new_v4()
        ));
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
                enrollment_witness: None,
                context_source: "PreToolUse".into(),
                callback_deadline: None,
                skill_pointer: false,
                capability: crate::harness::Capability::ObservedInput,
                domain: ContractDomain::Native,
                native_event: "Tool".into(),
                shape_fields: vec![],
                agent_id: None,
            },
            delivery: DeliveryEligibility::Context,
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "unavailable".into(),
            },
            binding_event: None,
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
    fn enrollment_policy_maps_decoded_events_to_kinds() {
        use super::super::context::{EventKind, QualifiedTurn};
        use EnrollmentKind as K;
        use EnrollmentPolicy::{Lifecycle as L, WitnessedStartAndUnownedTurn as W};
        let cli = Some(EnrollmentWitness::CliPlatform);
        let mut event = DecodedEvent {
            harness: super::super::registry::builtins().agent("codex").unwrap(),
            role: EventRole::TopLevel,
            native_session: Some("s".into()),
            event_id: "event".into(),
            intent: EventIntent::Current,
            metadata: EventMetadata {
                enrollment_witness: None,
                context_source: "SessionStart".into(),
                callback_deadline: None,
                skill_pointer: false,
                capability: crate::harness::Capability::ObservedInput,
                domain: ContractDomain::Native,
                native_event: "SessionStart".into(),
                shape_fields: vec![],
                agent_id: None,
            },
            delivery: DeliveryEligibility::Context,
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "unavailable".into(),
            },
            binding_event: None,
        };
        let turn = || {
            EventIntent::QualifiedTurn(QualifiedTurn {
                lineage_parent: None,
                session: "s".into(),
                event_key: "event".into(),
                reset: None,
                ordering: None,
            })
        };
        for policy in [L, W] {
            for kind in [
                EventKind::Fork,
                EventKind::Startup,
                EventKind::Clear,
                EventKind::Resume,
            ] {
                event.role = EventRole::TopLevel;
                event.intent = EventIntent::Lifecycle(kind);
                assert_eq!(event.enrollment(policy), Some(K::Lifecycle));
                for role in [EventRole::Subagent, EventRole::Unknown] {
                    event.role = role;
                    assert_eq!(event.enrollment(policy), None);
                }
            }
            event.role = EventRole::TopLevel;
            for kind in [EventKind::Compact, EventKind::Restart] {
                event.intent = EventIntent::Lifecycle(kind);
                assert_eq!(event.enrollment(policy), None);
            }
            event.intent = EventIntent::Lifecycle(EventKind::Startup);
            event.delivery = DeliveryEligibility::ObserverOnly;
            assert_eq!(event.enrollment(policy), None);
            event.delivery = DeliveryEligibility::Context;
            for intent in [
                EventIntent::Current,
                EventIntent::Observer,
                EventIntent::Enroll,
            ] {
                event.intent = intent;
                assert_eq!(event.enrollment(policy), None);
            }
        }
        // Under Lifecycle a witnessed turn or start is not admitted.
        event.metadata.enrollment_witness = cli;
        event.intent = turn();
        assert_eq!(event.enrollment(L), None);
        event.intent = EventIntent::Enroll;
        assert_eq!(event.enrollment(L), None);
        // Under the witnessed policy.
        assert_eq!(event.enrollment(W), Some(K::SessionStart));
        event.intent = turn();
        assert_eq!(event.enrollment(W), Some(K::UnownedTurn));
        event.delivery = DeliveryEligibility::ObserverOnly;
        assert_eq!(event.enrollment(W), None);
        event.delivery = DeliveryEligibility::Context;
        for role in [EventRole::Subagent, EventRole::Unknown] {
            event.role = role;
            event.intent = EventIntent::Enroll;
            assert_eq!(event.enrollment(W), None);
            event.intent = turn();
            assert_eq!(event.enrollment(W), None);
        }
        event.role = EventRole::TopLevel;
        event.intent = EventIntent::DeclaredReset(super::super::context::DeclaredReset {
            session: "s".into(),
            event_key: "event".into(),
            ordering: super::super::context::ObservationOrder {
                process_nonce: uuid::Uuid::nil(),
                sequence: 0,
                observed_at_millis: 0,
                callback_budget_millis: 0,
            },
        });
        assert_eq!(event.enrollment(W), None);
        event.intent = EventIntent::Observer;
        assert_eq!(event.enrollment(W), None);
        event.metadata.enrollment_witness = None;
        event.intent = EventIntent::Enroll;
        assert_eq!(event.enrollment(W), None);
        event.intent = turn();
        assert_eq!(event.enrollment(W), None);
        // Enroll is neither a check-in nor a context event.
        event.intent = EventIntent::Enroll;
        assert!(!event.can_check_in());
        assert!(event.context_event().is_none());
        // Only Hermes overrides the default.
        let builtins = super::super::registry::builtins();
        let hermes = builtins.by_id(builtins.agent("hermes").unwrap()).unwrap();
        for registration in builtins.registrations() {
            let expected = if std::ptr::eq(registration, hermes) {
                EnrollmentPolicy::WitnessedStartAndUnownedTurn
            } else {
                EnrollmentPolicy::Lifecycle
            };
            assert_eq!(registration.enrollment_policy(), expected);
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
                admitted_at: None,
            };
            assert!(registration.admit(&request, &budget).is_err());
            let status = registration.status(
                &StatusRequest {
                    scope: ResolvedSetupScope::ConfigRoot(PathBuf::from("unused")),
                    environment: SetupEnvironment::default(),
                    native_binary: None,
                },
                &budget,
            );
            if matches!(registration.metadata().id, "hermes" | "synthetic_fourth") {
                assert!(matches!(status, SetupStatus::Failed(_)));
            } else {
                assert!(matches!(status, SetupStatus::Detailed(_)));
            }
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
            match registration.metadata().id {
                "claude" | "codex" => {
                    assert_eq!(
                        registration.composer_policy().unwrap().capabilities(None),
                        crate::harness::recipe::PokeCapabilities::NONE
                    );
                    assert!(registration.canary_strategy().is_some());
                }
                "hermes" => {
                    assert!(registration.composer_policy().is_none());
                    assert!(registration.canary_strategy().is_some());
                }
                "agy" => {
                    assert_eq!(
                        registration.composer_policy().unwrap().capabilities(None),
                        crate::harness::recipe::PokeCapabilities::NONE
                    );
                    assert!(registration.canary_strategy().is_none());
                }
                "synthetic_fourth" => {
                    assert!(registration.composer_policy().is_none());
                    assert!(registration.canary_strategy().is_none());
                }
                other => panic!("unexpected builtin {other}"),
            }
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
            assert_eq!(
                observation.domain,
                if registration.metadata().id == "hermes" {
                    ContractDomain::Bridge
                } else {
                    ContractDomain::Native
                }
            );
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
        } else if event.source == "SubagentStop" {
            "SubagentStop"
        } else if event.source == "SessionEnd" {
            "SessionEnd"
        } else {
            "SessionStart"
        };
        // A subagent's or session's end is a bookkeeping event: it never checks in and renders
        // nothing.
        let stop = matches!(native_event, "SubagentStop" | "SessionEnd");
        Self {
            harness,
            role: match event.role {
                super::context::Role::TopLevel => EventRole::TopLevel,
                super::context::Role::Subagent => EventRole::Subagent,
            },
            native_session: event.native_session,
            event_id: event.event_id,
            intent: if stop {
                EventIntent::Observer
            } else if event.kind == super::context::EventKind::Tool {
                EventIntent::Current
            } else {
                EventIntent::Lifecycle(event.kind)
            },
            metadata: EventMetadata {
                enrollment_witness: None,
                context_source: event.source,
                callback_deadline: None,
                skill_pointer: native_event == "SessionStart",
                capability: event.capability,
                domain: ContractDomain::Native,
                native_event: native_event.into(),
                shape_fields: vec![],
                agent_id: event.agent_id,
            },
            delivery: if stop {
                DeliveryEligibility::Ineligible
            } else {
                DeliveryEligibility::Context
            },
            runtime: RuntimeAttribution::Unavailable {
                diagnostic: "runtime attribution is separate".into(),
            },
            binding_event: None,
        }
    }
    /// [`Self::from_native`] plus the binding enrollment or disconnect the raw hook payload reports.
    pub fn from_native_payload(event: super::LifecycleEvent, payload: &[u8]) -> Self {
        let mut decoded = Self::from_native(event);
        decoded.binding_event = serde_json::from_slice(payload)
            .ok()
            .and_then(|value| BindingEventDecode::from_native_payload(&value));
        decoded
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
                EventIntent::Enroll | EventIntent::Observer | EventIntent::DeclaredReset(_) => {
                    return None;
                }
            },
            native_session: self.native_session.clone(),
            role: match self.role {
                EventRole::TopLevel => super::context::Role::TopLevel,
                EventRole::Subagent => super::context::Role::Subagent,
                EventRole::Unknown => return None,
            },
            event_id: self.event_id.clone(),
            capability: self.metadata.capability,
            agent_id: self.metadata.agent_id.clone(),
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
    /// Executable present; callback/runtime qualification is unavailable.
    /// Grants no operational contract, enablement, runtime or receipt basis.
    PresentUnqualified { detail: String },
    /// Present but its `--version` could not be observed or recognized, which
    /// blocks the hook: a limitation that degrades Health.
    Refused(String),
    /// Present, observed, and its version is below the recipe floor or inside
    /// a known-broken range. Whether that matters is the version verdict's
    /// (`harness::state`, rendered from evidence and the manifest), so Health
    /// shows nothing for it here; doctor shows the detected-version line.
    VersionRefused(String),
    /// Registered core input contract; no runtime recipe admission.
    ContractDeclared { detail: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerHookState {
    Missing,
    Owned,
}
/// The installer's view of a permission component in one config root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallerPermissionState {
    Missing,
    Granted,
    /// Reached through a symbolic link to a grant herdr-threads owns, unedited, at another
    /// root; the detail names it. Nothing is written through the link.
    Shared(String),
    /// Reached through a symbolic link to a file not installed at its real location; the
    /// detail says to set up the root that holds it first. Nothing is written through the link.
    SharedMissing(String),
}
#[derive(Debug, Clone)]
pub struct InstallerSkillDestination {
    pub root: std::path::PathBuf,
    pub file: std::path::PathBuf,
}
pub trait InstallerPolicy: Send + Sync {
    fn inspect_hooks(
        &self,
        request: &StatusRequest,
        budget: &CallBudget,
    ) -> Result<InstallerHookState, SetupFailure>;
    fn skill_destination(&self, scope: &ResolvedSetupScope) -> Option<InstallerSkillDestination>;
    /// The state of the harness's herdr-threads permission grant; `None` when the adapter has
    /// no permission component (its integration needs no grant).
    fn permission_state(
        &self,
        _request: &StatusRequest,
    ) -> Result<Option<InstallerPermissionState>, SetupFailure> {
        Ok(None)
    }
    /// The backend of the adapter's permission component, if it has one: its manifest tag and lock name.
    fn permission_backend(&self) -> Option<crate::harness::permissions::PermissionBackend> {
        None
    }
}
