//! Concrete Hermes bridge adapter. Cooperative startup observations are not native/model proof.
pub mod assets;
pub mod launch;
pub mod runtime;
use super::{
    adapter::*,
    contract::{self, EventClass, EventContract, HarnessContract, JsonType, field},
    evidence::{AttributionHolding, EvidenceEvent, EvidenceOrigin},
};
use crate::protocol::time::CallBudget;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub struct HermesAdapter;
pub const BRIDGE_SCHEMA: &str = "3f599002572f0efc";
static METADATA: AdapterMetadata = AdapterMetadata {
    id: "hermes",
    display_label: "Hermes",
    context_spelling: "Hermes",
    context_aliases: &[],
    executable: ExecutableLookup::Path("hermes"),
    host_kinds: &["hermes"],
    setup_scopes: &[SetupScopeKind::ConfigRoot, SetupScopeKind::Profile],
    budget: EventBudgetPolicy {
        lifecycle_ms: 5000,
        observer_ms: 1500,
    },
    runtime_sources: &[
        "build",
        "commit-build",
        "ci",
        "docker",
        "fallback",
        "git",
        "local",
        "nix",
    ],
};
const EVENTS: &[EvidenceEvent] = &[
    EvidenceEvent {
        native_event: "pre_llm_call",
        milestone: Some("qualified_turn"),
        always_send: false,
    },
    EvidenceEvent {
        native_event: "post_tool_call",
        milestone: Some("qualified_post_tool"),
        always_send: false,
    },
    EvidenceEvent {
        native_event: "on_session_start",
        milestone: None,
        always_send: true,
    },
    EvidenceEvent {
        native_event: "on_session_reset",
        milestone: None,
        always_send: true,
    },
];
const BRIDGE_FIELDS: &[contract::FieldSpec] = &[
    field("schema_version", JsonType::Number, true),
    field("callback", JsonType::String, true),
    field("platform", JsonType::StringOrNull, true),
    field("session_id", JsonType::StringOrNull, true),
    field("parent_session_id", JsonType::StringOrNull, true),
    field("task_id", JsonType::StringOrNull, true),
    field("turn_id", JsonType::StringOrNull, true),
    field("tool_call_id", JsonType::StringOrNull, true),
    field("api_request_id", JsonType::StringOrNull, true),
    field("event_id", JsonType::String, true),
    field("observation_order", JsonType::Object, true),
    field("started_at", JsonType::Number, true),
    field("deadline_at", JsonType::Number, true),
    field("reset_reason", JsonType::StringOrNull, true),
    field("runtime_identity", JsonType::Object, true),
    field("identity_unavailable_reason", JsonType::StringOrNull, true),
    field("shape", JsonType::Object, true),
    field("bridge_schema_contract_id", JsonType::String, true),
    field("startup_capture", JsonType::Object, true),
    field("timeout_observation", JsonType::Object, true),
];
const SHAPE_FIELDS: &[contract::FieldSpec] = &[
    field("shape.platform.presence", JsonType::String, true),
    field("shape.platform.type", JsonType::String, true),
    field("shape.session_id.presence", JsonType::String, true),
    field("shape.session_id.type", JsonType::String, true),
    field("shape.parent_session_id.presence", JsonType::String, true),
    field("shape.parent_session_id.type", JsonType::String, true),
    field("shape.task_id.presence", JsonType::String, true),
    field("shape.task_id.type", JsonType::String, true),
    field("shape.turn_id.presence", JsonType::String, true),
    field("shape.turn_id.type", JsonType::String, true),
    field("shape.tool_call_id.presence", JsonType::String, true),
    field("shape.tool_call_id.type", JsonType::String, true),
    field("shape.api_request_id.presence", JsonType::String, true),
    field("shape.api_request_id.type", JsonType::String, true),
];
macro_rules! events {
    ($fields:expr) => {
        &[
            EventContract {
                event: "pre_llm_call",
                class: EventClass::Other,
                fields: $fields,
            },
            EventContract {
                event: "post_tool_call",
                class: EventClass::Tool,
                fields: $fields,
            },
            EventContract {
                event: "on_session_start",
                class: EventClass::Lifecycle,
                fields: $fields,
            },
            EventContract {
                event: "on_session_reset",
                class: EventClass::Lifecycle,
                fields: $fields,
            },
        ]
    };
}
static BRIDGE: HarnessContract = HarnessContract {
    harness: "hermes",
    discriminator: "callback",
    events: events!(BRIDGE_FIELDS),
};
static NATIVE: HarnessContract = HarnessContract {
    harness: "hermes",
    discriminator: "callback",
    events: events!(SHAPE_FIELDS),
};
pub static CONTRACTS: &[ContractDescriptor] = &[
    ContractDescriptor {
        domain_id: "native_callback",
        origin: EvidenceOrigin::NativeShapeObservation,
        events: EVENTS,
        required_milestones: &["qualified_turn", "qualified_post_tool"],
        qualifications: &[
            "qualified_role_session",
            "startup_captured_identity",
            "observed_timeout",
        ],
        holding: AttributionHolding::Never,
        resumed_unavailable_reason: None,
        domain: ContractDomain::Native,
        contract: &NATIVE,
    },
    ContractDescriptor {
        domain_id: "bridge_envelope",
        origin: EvidenceOrigin::BridgeEnvelope,
        events: EVENTS,
        required_milestones: &["qualified_turn", "qualified_post_tool"],
        qualifications: &[
            "qualified_role_session",
            "startup_captured_identity",
            "observed_timeout",
            "transport_3f599002572f0efc",
        ],
        holding: AttributionHolding::Never,
        resumed_unavailable_reason: None,
        domain: ContractDomain::Bridge,
        contract: &BRIDGE,
    },
];
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Startup {
    identity_provenance: String,
    api_contract: String,
    interpreter: PathBuf,
    module_origins: BTreeMap<String, PathBuf>,
    profile: String,
    lexical_home: PathBuf,
    physical_home: PathBuf,
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Timeout {
    quality: String,
    fallback_kind: Option<String>,
    provenance: String,
    timeout_seconds: f64,
    age_seconds: f64,
    generation: u64,
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Association {
    role: String,
    provenance: String,
    session_id: String,
    turn_id: String,
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Shape {
    presence: String,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Order {
    process_nonce: uuid::Uuid,
    sequence: u64,
    observed_at_millis: i64,
    callback_budget_millis: u32,
}
impl Order {
    fn generic(&self) -> super::context::ObservationOrder {
        super::context::ObservationOrder {
            process_nonce: self.process_nonce,
            sequence: self.sequence,
            observed_at_millis: self.observed_at_millis,
            callback_budget_millis: self.callback_budget_millis,
        }
    }
}
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    callback: String,
    platform: Option<String>,
    session_id: Option<String>,
    parent_session_id: Option<String>,
    task_id: Option<String>,
    turn_id: Option<String>,
    tool_call_id: Option<String>,
    api_request_id: Option<String>,
    event_id: String,
    observation_order: Order,
    started_at: i64,
    deadline_at: i64,
    reset_reason: Option<String>,
    runtime_identity: RuntimeIdentity,
    identity_unavailable_reason: Option<String>,
    shape: BTreeMap<String, Shape>,
    bridge_schema_contract_id: String,
    startup_capture: Startup,
    timeout_observation: Timeout,
    role_association: Option<Association>,
}
fn admit_candidate(
    request: &AdmissionRequest,
    manifest: &super::manifest::Manifest,
) -> AdmissionDecision<HermesAdmission> {
    let Some(input) = &request.input else {
        return AdmissionDecision::Refused {
            diagnostic: "Hermes requires actual startup-captured callback input".into(),
        };
    };
    match parse(input) {
        Ok(envelope) => {
            if request
                .runtime_candidate
                .as_ref()
                .is_some_and(|r| r != &envelope.runtime_identity)
            {
                return AdmissionDecision::Refused {
                    diagnostic: "runtime candidate differs from startup identity".into(),
                };
            }
            if CONTRACTS.iter().any(|d| {
                manifest
                    .runtime_row("hermes", &envelope.runtime_identity, d)
                    .is_some_and(|r| r.status == super::manifest::RuntimeStatus::KnownBroken)
            }) {
                return AdmissionDecision::Refused {
                    diagnostic: "known-broken exact Hermes domain".into(),
                };
            }
            AdmissionDecision::Optimistic {
                state: HermesAdmission {
                    envelope,
                    bytes: input.bytes.clone(),
                },
                recipe: "initialized-hermes-bridge-schema1",
                diagnostic: "startup-captured API candidate; native/model acceptance unmeasured"
                    .into(),
            }
        }
        Err(_) => AdmissionDecision::Refused {
            diagnostic: "Hermes startup callback qualification unavailable".into(),
        },
    }
}
/// Private input-bound handle; diagnostic profile observations cannot construct it.
pub struct HermesAdmission {
    envelope: Envelope,
    bytes: Vec<u8>,
}
fn text(s: &str, max: usize, empty: bool) -> bool {
    (empty || !s.is_empty()) && s.len() <= max && !s.chars().any(char::is_control)
}
fn absolute(p: &Path) -> bool {
    p.is_absolute() && p.to_str().is_some_and(|s| text(s, 4096, false))
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
fn fields(v: &serde_json::Value, keys: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == keys.len() && keys.iter().all(|k| m.contains_key(*k)))
}
fn parse(input: &HookInput) -> Result<Envelope, DecodeFailure> {
    let invalid = || DecodeFailure::Invalid("invalid Hermes bridge envelope".into());
    if input.bytes.len() > 65536 {
        return Err(invalid());
    }
    let value: serde_json::Value = serde_json::from_slice(&input.bytes).map_err(|_| invalid())?;
    if !fields(
        &value,
        &[
            "schema_version",
            "callback",
            "platform",
            "session_id",
            "parent_session_id",
            "task_id",
            "turn_id",
            "tool_call_id",
            "api_request_id",
            "event_id",
            "observation_order",
            "started_at",
            "deadline_at",
            "reset_reason",
            "runtime_identity",
            "identity_unavailable_reason",
            "shape",
            "bridge_schema_contract_id",
            "startup_capture",
            "timeout_observation",
            "role_association",
        ],
    ) || !fields(
        &value["runtime_identity"],
        &[
            "key",
            "release_version",
            "source",
            "base_version",
            "derived_version",
            "commit",
            "dirty",
            "distance",
        ],
    ) || !fields(
        &value["startup_capture"],
        &[
            "identity_provenance",
            "api_contract",
            "interpreter",
            "module_origins",
            "profile",
            "lexical_home",
            "physical_home",
        ],
    ) || !fields(
        &value["timeout_observation"],
        &[
            "quality",
            "fallback_kind",
            "provenance",
            "timeout_seconds",
            "age_seconds",
            "generation",
        ],
    ) || !fields(
        &value["observation_order"],
        &[
            "process_nonce",
            "sequence",
            "observed_at_millis",
            "callback_budget_millis",
        ],
    ) {
        return Err(invalid());
    }
    let e: Envelope = serde_json::from_slice(&input.bytes).map_err(|_| invalid())?;
    if e.schema_version != 1
        || e.bridge_schema_contract_id != BRIDGE_SCHEMA
        || e.identity_unavailable_reason.is_some()
        || input
            .registered_event
            .as_ref()
            .is_some_and(|name| name != &e.callback)
        || !EVENTS.iter().any(|event| event.native_event == e.callback)
        || !text(&e.event_id, 256, false)
        || e.session_id.as_ref().is_none_or(|s| !text(s, 256, false))
        || e.platform
            .as_deref()
            .is_some_and(|p| !p.is_empty() && p != "cli")
    {
        return Err(invalid());
    }
    for s in [
        &e.platform,
        &e.parent_session_id,
        &e.task_id,
        &e.turn_id,
        &e.tool_call_id,
        &e.api_request_id,
    ]
    .into_iter()
    .flatten()
    {
        if !text(s, 256, true) {
            return Err(invalid());
        }
    }
    let startup = &e.startup_capture;
    let r = &e.runtime_identity;
    if r.release_version.is_some()
        || !METADATA.runtime_sources.contains(&r.source.as_str())
        || r.base_version.is_none()
        || r.derived_version.is_none()
        || r.dirty.is_none()
        || (r.source == "git" && (r.commit.is_none() || r.distance.is_none()))
        || startup.identity_provenance != "startup_captured_identity"
        || startup.api_contract != "initialized_plugin_context_schema1"
        || !text(&startup.profile, 256, false)
        || startup.profile == "custom"
        || [
            &startup.interpreter,
            &startup.lexical_home,
            &startup.physical_home,
        ]
        .into_iter()
        .any(|p| !absolute(p))
    {
        return Err(invalid());
    }
    let bootstrap = startup
        .module_origins
        .get("hermes_bootstrap")
        .ok_or_else(invalid)?;
    let root = bootstrap.parent().ok_or_else(invalid)?;
    let names = [
        "hermes_bootstrap",
        "hermes_constants",
        "hermes_cli.profiles",
        "hermes_cli.version_info",
        "hermes_cli.config",
        "hermes_cli.plugins",
    ];
    if startup.module_origins.len() != 6
        || names.iter().any(|name| {
            startup.module_origins.get(*name).is_none_or(|path| {
                !absolute(path) || *path != root.join(format!("{}.py", name.replace('.', "/")))
            })
        })
    {
        return Err(invalid());
    }
    let timeout = &e.timeout_observation;
    if timeout.quality != "ok"
        || timeout.fallback_kind.is_some()
        || timeout.provenance != "official_effective_config_observation"
        || timeout.generation == 0
        || !timeout.timeout_seconds.is_finite()
        || !(0.0..=600.0).contains(&timeout.timeout_seconds)
        || !timeout.age_seconds.is_finite()
        || !(0.0..=5.0).contains(&timeout.age_seconds)
    {
        return Err(invalid());
    }
    let lifecycle = e.callback.starts_with("on_session_");
    let cap = if lifecycle { 4500 } else { 1200 };
    let native = if timeout.timeout_seconds == 0.0 {
        cap as f64
    } else {
        (timeout.timeout_seconds * 1000.0 - 100.0).min(cap as f64)
    };
    if (timeout.timeout_seconds > 0.0 && timeout.timeout_seconds <= 0.2)
        || e.observation_order.callback_budget_millis > cap
        || f64::from(e.observation_order.callback_budget_millis) > native
        || e.started_at != e.observation_order.observed_at_millis
        || e.started_at
            .checked_add(i64::from(e.observation_order.callback_budget_millis))
            != Some(e.deadline_at)
        || e.observation_order
            .generic()
            .validate_deadline(now())
            .is_err()
    {
        return Err(invalid());
    }
    let ids = [
        ("platform", &e.platform),
        ("session_id", &e.session_id),
        ("parent_session_id", &e.parent_session_id),
        ("task_id", &e.task_id),
        ("turn_id", &e.turn_id),
        ("tool_call_id", &e.tool_call_id),
        ("api_request_id", &e.api_request_id),
    ];
    if e.shape.len() != 7 {
        return Err(invalid());
    }
    for (name, id) in ids {
        let shape = e.shape.get(name).ok_or_else(invalid)?;
        if (shape.presence == "missing" && (shape.kind != "absent" || id.is_some()))
            || (shape.presence == "present"
                && shape.kind != if id.is_some() { "string" } else { "null" })
            || !["missing", "present"].contains(&shape.presence.as_str())
        {
            return Err(invalid());
        }
    }
    if e.callback == "pre_llm_call" {
        let parent = e.parent_session_id.as_deref().ok_or_else(invalid)?;
        let association = e.role_association.as_ref().ok_or_else(invalid)?;
        if !fields(
            &value["role_association"],
            &["role", "provenance", "session_id", "turn_id"],
        ) || association.provenance != "explicit_parent"
            || association.role != if parent.is_empty() { "top" } else { "child" }
            || Some(&association.session_id) != e.session_id.as_ref()
            || Some(&association.turn_id) != e.turn_id.as_ref()
            || !text(&association.turn_id, 256, false)
            || e.shape["parent_session_id"].presence != "present"
        {
            return Err(invalid());
        }
    } else if e.callback == "post_tool_call" {
        let association = e.role_association.as_ref().ok_or_else(invalid)?;
        if !fields(
            &value["role_association"],
            &["role", "provenance", "session_id", "turn_id"],
        ) || association.role != "top"
            || association.provenance != "qualified_pre_llm_cache"
            || Some(&association.session_id) != e.session_id.as_ref()
            || Some(&association.turn_id) != e.turn_id.as_ref()
            || !text(&association.turn_id, 256, false)
            || e.shape["parent_session_id"].presence != "missing"
        {
            return Err(invalid());
        }
    } else if e.role_association.is_some() {
        return Err(invalid());
    }
    if (e.callback == "on_session_reset" && e.reset_reason.as_deref() != Some("new_session"))
        || (e.callback != "on_session_reset" && e.reset_reason.is_some())
    {
        return Err(invalid());
    }
    Ok(e)
}
fn observation(input: &HookInput, domain: ContractDomain) -> ContractObservation {
    let descriptor = CONTRACTS.iter().find(|d| d.domain == domain).unwrap();
    let classification = contract::classify(
        descriptor.contract,
        input.registered_event.as_deref(),
        &input.bytes,
    );
    // The native shape contract uses callback only to select an event; it
    // does not declare the bridge's callback field. A bad selector cannot
    // produce a field violation in that native domain.
    let classification = match classification {
        contract::Classification::Violation { event, field }
            if !descriptor
                .contract
                .events
                .iter()
                .any(|e| e.event == event && e.fields.iter().any(|f| f.path == field)) =>
        {
            contract::Classification::Malformed(contract::Malformed::UnknownEvent)
        }
        other => other,
    };
    ContractObservation {
        domain,
        // Structure belongs to this descriptor. Runtime, role, values and
        // deadlines remain the strict parser's separate admission concerns.
        classification,
    }
}
struct HermesCanary;
impl CanaryStrategy for HermesCanary {
    fn descriptor(&self) -> CanaryDescriptor {
        CanaryDescriptor {
            kind: CanaryKind::ExactRuntime,
            candidate_kind: CandidateKind::ExactBuild,
            npm_package: None,
            model_key_env: None,
            companion: "scripts/canary/adapters/hermes.py".into(),
            artifact_schema_version: 1,
        }
    }
}
/// Captured PATH metadata only: never invoke the executable or discover a
/// profile. Check the caller's budget between each bounded candidate lookup.
fn observe_executable(
    env: &InstallEnvironment,
    budget: &CallBudget,
) -> Result<Option<std::path::PathBuf>, String> {
    use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
    let check = || -> Result<(), String> {
        if budget.cancellation.is_cancelled() {
            Err("hermes executable observation cancelled".into())
        } else if env.clock.monotonic_now() >= budget.deadline {
            Err("hermes executable observation budget expired".into())
        } else {
            Ok(())
        }
    };
    check()?;
    let Some(path) = env.path.as_deref() else {
        return Ok(None);
    };
    if path.as_bytes().len() > 65_536 {
        return Err("hermes executable observation PATH exceeds bounded lookup".into());
    }
    for (index, directory) in std::env::split_paths(path).enumerate() {
        check()?;
        if index >= 256 {
            return Err("hermes executable observation PATH exceeds bounded lookup".into());
        }
        if !directory.is_absolute() {
            continue;
        }
        let binary = directory.join("hermes");
        let metadata = std::fs::metadata(&binary);
        check()?;
        match metadata {
            Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => {
                return Ok(Some(binary));
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(_) => return Err("hermes executable observation metadata unavailable".into()),
        }
    }
    check()?;
    Ok(None)
}

impl HarnessAdapter for HermesAdapter {
    fn canary_strategy(&self) -> Option<&dyn CanaryStrategy> {
        Some(&HermesCanary)
    }

    type Admission = HermesAdmission;
    fn metadata(&self) -> &'static AdapterMetadata {
        &METADATA
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        CONTRACTS
    }
    fn hook_admission_policy(&self) -> HookAdmissionPolicy {
        HookAdmissionPolicy::QualifiedCallback
    }
    fn qualified_turn_policy(&self) -> super::context::QualifiedTurnPolicy {
        super::context::QualifiedTurnPolicy::StartupAttach
    }
    fn observe_install(&self, env: &InstallEnvironment, budget: &CallBudget) -> InstallObservation {
        match observe_executable(env, budget) {
            Ok(Some(binary)) => InstallObservation::ExecutableAvailable { binary },
            Ok(None) => InstallObservation::Unavailable {
                diagnostic: "no executable `hermes` on the daemon's PATH".into(),
            },
            Err(diagnostic) => InstallObservation::Unavailable { diagnostic },
        }
    }
    fn observe_daemon(&self, env: &InstallEnvironment, budget: &CallBudget) -> DaemonObservation {
        let status = match observe_executable(env, budget) {
            Ok(Some(_)) => HarnessStatus::PresentUnqualified {
                detail: "hermes executable present; callback qualification unavailable; runtime metadata, enablement and native behavior unobserved".into(),
            },
            Ok(None) => HarnessStatus::NotInstalled(
                "no executable `hermes` on the daemon's PATH".into(),
            ),
            Err(diagnostic) => HarnessStatus::Refused(diagnostic),
        };
        // Executable presence cannot supply qualified callback admission or
        // runtime identity. No fingerprint reuse: every pass observes again.
        DaemonObservation {
            status,
            ..Default::default()
        }
    }
    fn admit(
        &self,
        request: &AdmissionRequest,
        _: &CallBudget,
    ) -> AdmissionDecision<Self::Admission> {
        admit_candidate(request, super::manifest::embedded())
    }
    fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
        Ladder::Admitted
    }
    fn classify(&self, input: &HookInput) -> ContractObservation {
        observation(input, ContractDomain::Bridge)
    }
    fn evidence_observations(&self, input: &HookInput) -> Vec<EvidenceProjection> {
        CONTRACTS
            .iter()
            .map(|d| EvidenceProjection {
                domain: d.domain,
                origin: d.origin,
                contract_id: d.contract_id_v2().unwrap(),
                classification: observation(input, d.domain).classification,
            })
            .collect()
    }
    fn decode(
        &self,
        state: &HermesAdmission,
        input: &HookInput,
    ) -> Result<DecodedEvent, DecodeFailure> {
        let e = parse(input)?;
        if state.bytes != input.bytes || e != state.envelope {
            return Err(DecodeFailure::RegistrationMismatch);
        }
        let role = match e.role_association.as_ref().map(|a| a.role.as_str()) {
            Some("top") => EventRole::TopLevel,
            Some("child") => EventRole::Subagent,
            _ => EventRole::Unknown,
        };
        let session = e.session_id.clone().unwrap();
        let intent = if e.callback == "pre_llm_call" {
            EventIntent::QualifiedTurn(super::context::QualifiedTurn {
                session: session.clone(),
                event_key: e.event_id.clone(),
                reset: None,
                ordering: Some(e.observation_order.generic()),
            })
        } else if e.callback == "on_session_reset" {
            EventIntent::DeclaredReset(super::context::DeclaredReset {
                session: session.clone(),
                event_key: e.event_id.clone(),
                ordering: e.observation_order.generic(),
            })
        } else {
            EventIntent::Observer
        };
        let observer = e.callback != "pre_llm_call";
        Ok(DecodedEvent {
            harness: super::registry::builtins().agent("hermes").unwrap(),
            role,
            native_session: Some(session),
            event_id: e.event_id,
            intent,
            metadata: EventMetadata {
                context_source: e.callback.clone(),
                native_event: e.callback,
                shape_fields: e.shape.keys().cloned().collect(),
                callback_deadline: Some(
                    Instant::now() + Duration::from_millis((e.deadline_at - now()).max(0) as u64),
                ),
                skill_pointer: false,
                capability: super::Capability::ObservedInput,
                domain: ContractDomain::Bridge,
            },
            delivery: if observer {
                DeliveryEligibility::ObserverOnly
            } else {
                DeliveryEligibility::Context
            },
            runtime: RuntimeAttribution::Attributed(e.runtime_identity),
        })
    }
    fn encode(
        &self,
        state: &HermesAdmission,
        event: &DecodedEvent,
        offer: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        if event.event_id != state.envelope.event_id
            || event.native_session != state.envelope.session_id
            || event.metadata.native_event != state.envelope.callback
        {
            return Err(EncodeFailure::RegistrationMismatch);
        }
        let consuming = state.envelope.callback == "pre_llm_call"
            && event.role == EventRole::TopLevel
            && event.can_check_in();
        if !consuming {
            return Ok(EncodedOutput::ObserverOnly {
                bytes: br#"{"context":null,"lifecycle_ack":null}"#.to_vec(),
            });
        }
        if offer.fixed_guidance.len() > 4096 {
            return Err(EncodeFailure::Invalid(
                "Hermes context exceeds4096 bytes".into(),
            ));
        }
        let mode = match event.intent {
            EventIntent::Current | EventIntent::Lifecycle(super::context::EventKind::Tool) => {
                "current"
            }
            EventIntent::Lifecycle(super::context::EventKind::Clear) => "clear",
            EventIntent::Lifecycle(super::context::EventKind::Startup) => "startup",
            _ => {
                return Err(EncodeFailure::Invalid(
                    "qualified result kind unavailable".into(),
                ));
            }
        };
        let bytes=serde_json::to_vec(&serde_json::json!({"context":offer.fixed_guidance,"lifecycle_ack":{"event_id":event.event_id,"session_id":event.native_session,"mode":mode}})).map_err(|_|EncodeFailure::Invalid("Hermes result encoding failed".into()))?;
        if bytes.len() > 8192 {
            return Err(EncodeFailure::Invalid(
                "Hermes output exceeds8192 bytes".into(),
            ));
        }
        Ok(EncodedOutput::ContextBearing { bytes })
    }
    fn attribute_runtime(&self, input: &HookInput, _: &CallBudget) -> RuntimeAttribution {
        match parse(input) {
            Ok(e) => RuntimeAttribution::Attributed(e.runtime_identity),
            Err(_) => RuntimeAttribution::Unavailable {
                diagnostic: "startup_callback_unavailable".into(),
            },
        }
    }
    fn evidence_qualifications(
        &self,
        request: &EvidenceQualificationRequest<'_>,
        _: &CallBudget,
    ) -> Result<Vec<String>, String> {
        let e = parse(request.input).map_err(|_| "invalid callback".to_string())?;
        if &e.runtime_identity != request.runtime {
            return Err("runtime mismatch".into());
        }
        Ok(
            if e.role_association.as_ref().is_some_and(|a| a.role == "top") {
                request
                    .descriptor
                    .qualifications
                    .iter()
                    .map(|s| (*s).into())
                    .collect()
            } else {
                vec![]
            },
        )
    }
    fn setup_environment_inputs(&self) -> &'static [&'static str] {
        &["HERMES_HOME", "HERMES_PROFILE"]
    }
    fn resolve_setup_scope(
        &self,
        request: &SetupScopeRequest,
        env: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        let profile = match request {
            SetupScopeRequest::Default => "default",
            SetupScopeRequest::Profile(p) => p,
        };
        let o = inspect_profile(
            profile,
            None,
            env,
            &CallBudget {
                deadline: crate::protocol::time::MonoInstant(
                    env.clock.monotonic_now().0.saturating_add(2000),
                ),
                cancellation: Default::default(),
            },
        )?;
        Ok(ResolvedSetupScope::Profile {
            name: o.profile,
            home: o.home,
        })
    }
    fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure> {
        let o = inspect_scope(
            &request.scope,
            request.native_binary.as_deref(),
            &request.environment,
            budget,
        )?;
        let state = request
            .environment
            .state_dir
            .as_deref()
            .ok_or_else(|| SetupFailure::Invalid("state unavailable".into()))?;
        let host = request
            .environment
            .host_endpoint
            .as_deref()
            .ok_or_else(|| SetupFailure::Invalid("host unavailable".into()))?;
        assets::setup(&o, state, &request.executable, host).map_err(asset_error)?;
        let status = assets::status(&o, state).map_err(asset_error)?;
        Ok(SetupOutcome {
            actions: vec![SetupAction::InstalledOwned],
            diagnostic: "owned assets staged; native enable remains manual".into(),
            projection: asset_projection(&status),
            diagnostics: vec![],
        })
    }
    fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus {
        let run = || {
            let o = inspect_scope(
                &request.scope,
                request.native_binary.as_deref(),
                &request.environment,
                budget,
            )?;
            let state = request
                .environment
                .state_dir
                .as_deref()
                .ok_or_else(|| SetupFailure::Invalid("state unavailable".into()))?;
            assets::status(&o, state).map_err(asset_error)
        };
        match run() {
            Ok(s) => SetupStatus::Detailed(Box::new(LocalSetupStatus {
                scope: request.scope.clone(),
                installed: s.installed,
                enabled: s.configured_enabled,
                admitted: None,
                observed: None,
                configured_hook: s.launch_hook.clone(),
                fingerprint: s.launch_hook.as_ref().map(|h| h.fingerprint.clone()),
                diagnostics: vec![],
                repairs: vec![],
                projection: asset_projection(&s),
            })),
            Err(e) => SetupStatus::Failed(e),
        }
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        Some(&launch::POLICY)
    }
    fn unsetup(
        &self,
        request: &UnsetupRequest,
        _: &CallBudget,
    ) -> Result<RemovalOutcome, SetupFailure> {
        let (home, profile) = match &request.scope {
            ResolvedSetupScope::Profile { name, home } => (home, name.as_str()),
            ResolvedSetupScope::ConfigRoot(home) => (home, "default"),
        };
        let state = request
            .environment
            .state_dir
            .as_deref()
            .ok_or_else(|| SetupFailure::Invalid("state unavailable".into()))?;
        let removed = assets::unsetup(home, profile, state).map_err(asset_error)?;
        Ok(RemovalOutcome {
            actions: vec![SetupAction::RemovedOwned],
            diagnostic: "exact owned assets removed; native disable remains manual".into(),
            residue: removed.residue.iter().map(PathBuf::from).collect(),
            projection: serde_json::json!({"manual_argv":removed.manual_argv}),
            diagnostics: vec![],
        })
    }
}
fn asset_projection(s: &assets::AssetStatus) -> serde_json::Value {
    serde_json::json!({"installed":s.installed,"configured_enabled":s.configured_enabled,"repairable":s.repairable,"manual_argv":s.manual_argv,"native_acceptance":"unmet"})
}
fn asset_error(e: super::setup::SetupError) -> SetupFailure {
    SetupFailure::Invalid(format!("Hermes owned assets: {e:?}"))
}
fn launcher(env: &SetupEnvironment) -> Option<PathBuf> {
    env.path.as_ref().and_then(|path| {
        std::env::split_paths(path)
            .map(|p| p.join("hermes"))
            .find(|p| p.is_file())
    })
}
fn inspect_profile(
    profile: &str,
    native: Option<&Path>,
    env: &SetupEnvironment,
    budget: &CallBudget,
) -> Result<runtime::ProfileObservation, SetupFailure> {
    let native = native
        .map(PathBuf::from)
        .or_else(|| launcher(env))
        .ok_or_else(|| SetupFailure::Invalid("Hermes launcher unavailable".into()))?;
    let helper = inspection_helper(env)?;
    runtime::discover_selected_profile(&native, &helper, profile, env, budget)
        .map_err(|_| SetupFailure::Invalid("Hermes profile inspection unavailable".into()))
}
fn inspection_helper(env: &SetupEnvironment) -> Result<PathBuf, SetupFailure> {
    let helper = env
        .state_dir
        .as_ref()
        .ok_or_else(|| SetupFailure::Invalid("state unavailable".into()))?
        .join("setup/hermes-runtime-helper.py");
    crate::daemon::paths::ensure_private_dir(helper.parent().unwrap()).map_err(SetupFailure::Io)?;
    use std::{
        io::{Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let expected = include_bytes!("../../integrations/hermes/runtime_helper.py");
    let mut options = std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW);
    match options.open(&helper) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(expected).and_then(|_| file.sync_all()) {
                let _ = std::fs::remove_file(&helper);
                return Err(SetupFailure::Io(error));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&helper)
                .map_err(SetupFailure::Io)?;
            let metadata = file.metadata().map_err(SetupFailure::Io)?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.len() != expected.len() as u64
            {
                return Err(SetupFailure::Invalid(
                    "Hermes helper ownership conflict".into(),
                ));
            }
            let mut bytes = Vec::new();
            file.take(expected.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(SetupFailure::Io)?;
            if bytes != expected {
                return Err(SetupFailure::Invalid(
                    "Hermes helper ownership conflict".into(),
                ));
            }
        }
        Err(error) => return Err(SetupFailure::Io(error)),
    }
    Ok(helper)
}
fn inspect_scope(
    scope: &ResolvedSetupScope,
    native: Option<&Path>,
    env: &SetupEnvironment,
    budget: &CallBudget,
) -> Result<runtime::ProfileObservation, SetupFailure> {
    let (profile, home) = match scope {
        ResolvedSetupScope::Profile { name, home } => (name.as_str(), home),
        ResolvedSetupScope::ConfigRoot(home) => ("default", home),
    };
    let observed = inspect_profile(profile, native, env, budget)?;
    if &observed.home != home {
        return Err(SetupFailure::Invalid(
            "observed selected home differs from requested scope".into(),
        ));
    }
    Ok(observed)
}
#[cfg(test)]
mod adapter_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    fn fixture(callback: &str) -> HookInput {
        let mut v: serde_json::Value =
            serde_json::from_slice(include_bytes!("../../tests/fixtures/hermes/envelopes.json"))
                .unwrap();
        let at = now();
        v["started_at"] = at.into();
        v["deadline_at"] = (at + 1200).into();
        v["observation_order"]["observed_at_millis"] = at.into();
        v["callback"] = callback.into();
        if callback == "post_tool_call" {
            v["parent_session_id"] = serde_json::Value::Null;
            v["shape"]["parent_session_id"] =
                serde_json::json!({"presence":"missing","type":"absent"});
            v["role_association"]["provenance"] = "qualified_pre_llm_cache".into();
        }
        if callback.starts_with("on_session_") {
            v["role_association"] = serde_json::Value::Null;
            if callback == "on_session_reset" {
                v["reset_reason"] = "new_session".into();
            }
        }
        HookInput {
            bytes: serde_json::to_vec(&v).unwrap(),
            registered_event: None,
        }
    }
    fn timing() -> (Arc<dyn crate::protocol::time::Clock>, CallBudget) {
        let clock: Arc<dyn crate::protocol::time::Clock> = Arc::new(crate::app::SystemClock::new());
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(clock.monotonic_now().0 + 1000),
            cancellation: Default::default(),
        };
        (clock, budget)
    }
    fn registration() -> &'static super::super::registry::Registration {
        let r = super::super::registry::builtins();
        r.by_id(
            r.agent("hermes")
                .expect("concrete Hermes registry consumer is missing"),
        )
        .unwrap()
    }
    #[test]
    fn hermes_admission_actual_runtime_and_two_domain_milestones_keep_observers_nonconsuming() {
        let r = registration();
        let (clock, budget) = timing();
        let input = fixture("pre_llm_call");
        let path = RuntimeIdentity::stable_release("9.9.9", "git").unwrap();
        let handle = r
            .admit(
                &AdmissionRequest {
                    installed: InstallObservation::Available {
                        binary: "/fixture/path-install".into(),
                        identity: path,
                    },
                    input: Some(HookInput {
                        bytes: input.bytes.clone(),
                        registered_event: None,
                    }),
                    runtime_candidate: None,
                },
                &budget,
            )
            .unwrap();
        assert_eq!(handle.kind(), AdmissionKind::Optimistic);
        let event = r.decode(&handle, &input).unwrap();
        assert!(matches!(event.intent, EventIntent::QualifiedTurn(_)));
        let RuntimeAttribution::Attributed(identity) = &event.runtime else {
            panic!("actual runtime missing")
        };
        assert_eq!(
            identity.commit.as_deref(),
            Some("1234567890abcdef1234567890abcdef12345678")
        );
        assert_eq!(identity.release(), None);
        assert_eq!(r.contracts().len(), 2);
        for d in r.contracts() {
            assert_eq!(
                d.required_milestones,
                &["qualified_turn", "qualified_post_tool"]
            );
            assert!(!d.verified(&["qualified_turn"], d.qualifications));
            assert!(!d.verified(&["startup", "qualified_post_tool"], d.qualifications));
        }
        for callback in ["post_tool_call", "on_session_start", "on_session_reset"] {
            let input = fixture(callback);
            let h = r
                .admit(
                    &AdmissionRequest {
                        installed: InstallObservation::Unsupported(UnsupportedOperation {
                            adapter: "hermes",
                            operation: "PATH",
                        }),
                        input: Some(HookInput {
                            bytes: input.bytes.clone(),
                            registered_event: None,
                        }),
                        runtime_candidate: None,
                    },
                    &budget,
                )
                .unwrap();
            let event = r.decode(&h, &input).unwrap();
            assert!(!event.can_check_in());
            assert_eq!(event.metadata.native_event, callback);
            let args = crate::cli::hook::HookArgs {
                harness: super::super::registry::OccupantHarness::Agent(
                    super::super::registry::builtins().agent("hermes").unwrap(),
                )
                .into(),
                state_dir: Some("/missing/hermes-observer".into()),
                host_endpoint: None,
                event: None,
            };
            let outcome = crate::cli::hook::run_admitted_hook(
                &args,
                r,
                &h,
                &event,
                &crate::cli::hook::HookEnv {
                    herdr_env: true,
                    pane: Some("w1:p1".into()),
                },
                Instant::now() + Duration::from_secs(1),
                Arc::clone(&clock),
                None,
            );
            assert!(outcome.attention.is_none());
            assert!(outcome.diagnostic.is_none());
            if !outcome.stdout.is_empty() {
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&outcome.stdout).unwrap(),
                    serde_json::json!({"context":null,"lifecycle_ack":null})
                );
            }
        }
    }
    #[test]
    fn hermes_unknown_rotation_uses_declared_startup_policy_without_resume() {
        use super::super::context::*;
        let directory =
            std::env::temp_dir().join(format!("hermes-context-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let instance = uuid::Uuid::new_v4();
        let journal = ContextJournal::open(
            &directory,
            instance,
            "fixture-seat",
            Duration::from_millis(50),
        )
        .unwrap();
        let current = OccupantContext {
            format_version: 1,
            instance,
            seat: "fixture-seat".into(),
            target: "w1:p1".into(),
            harness: super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into(),
            binding_generation: 1,
            execution: uuid::Uuid::new_v4(),
            session: SessionReference::Native("old-session".into()),
            role: Role::TopLevel,
        };
        journal.install_reattached(current.clone()).unwrap();
        let at = now();
        let turn = QualifiedTurn {
            session: "new-session".into(),
            event_key: "new-turn".into(),
            reset: None,
            ordering: Some(ObservationOrder {
                process_nonce: uuid::Uuid::new_v4(),
                sequence: 1,
                observed_at_millis: at,
                callback_budget_millis: 1200,
            }),
        };
        let result = journal.get_or_prepare_qualified(
            current.harness,
            &current.target,
            &turn,
            at,
            |c, kind| {
                assert_eq!(kind, EventKind::Startup);
                Ok(PendingCheckIn {
                    operation_id: uuid::Uuid::new_v4(),
                    mode: kind.mode(),
                    context: c.unwrap().for_event(
                        kind,
                        uuid::Uuid::new_v4(),
                        Some(turn.session.clone()),
                    )?,
                    expected_generation: Some(1),
                    event_id: turn.event_key.clone(),
                    payload_version: 1,
                    payload: b"synthetic pending bytes".to_vec(),
                })
            },
        );
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(
            result.is_ok(),
            "explicit Hermes startup attach policy is unreachable: {result:?}"
        );
    }
    #[test]
    fn actual_generic_hook_facade_qualifies_callback_input_instead_of_path_install() {
        let r = registration();
        let input = fixture("post_tool_call");
        let (clock, _) = timing();
        let args = crate::cli::hook::HookArgs {
            harness: super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into(),
            state_dir: Some("/missing/hermes-observer".into()),
            host_endpoint: None,
            event: None,
        };
        let outcome = crate::cli::hook::run_hook(
            &args,
            &crate::cli::hook::InstalledHarness::Claude("9.9.9".into()),
            &input.bytes,
            &crate::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            Instant::now() + Duration::from_secs(1),
            clock,
            None,
        );
        assert!(
            outcome.diagnostic.is_none(),
            "callback admission still depends on unrelated PATH installation: {:?}",
            outcome.diagnostic
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&outcome.stdout).unwrap(),
            serde_json::json!({"context":null,"lifecycle_ack":null})
        );
        assert!(outcome.attention.is_none());
        assert!(r.callback_admission());
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn declared_reset_observer_records_only_hint_then_qualified_journal_consumes_once() {
        use crate::{
            daemon::paths::{InstancePaths, RuntimeContext},
            harness::context::*,
            test_support::isolation::TestIsolation,
        };
        use std::os::unix::fs::PermissionsExt;
        let iso = TestIsolation::new("hermes-reset");
        let host = iso.path("host.sock");
        let context =
            RuntimeContext::explicit(iso.state_root().into(), host.clone(), None).unwrap();
        let paths = InstancePaths::resolve_read_only(&context).unwrap();
        paths.prepare_instance_dir().unwrap();
        let instance = uuid::Uuid::new_v4();
        std::fs::write(&paths.namespace_path, instance.to_string()).unwrap();
        std::fs::set_permissions(
            &paths.namespace_path,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        crate::daemon::paths::ensure_private_dir(&paths.instance_dir.join("contexts")).unwrap();
        let directory = paths.instance_dir.join("contexts/synthetic-seat");
        crate::daemon::paths::ensure_private_dir(&directory).unwrap();
        let journal = ContextJournal::open(
            &directory,
            instance,
            "fixture-seat",
            Duration::from_millis(50),
        )
        .unwrap();
        let current = OccupantContext {
            format_version: 1,
            instance,
            seat: "fixture-seat".into(),
            target: "w1:p1".into(),
            harness: super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into(),
            binding_generation: 1,
            execution: uuid::Uuid::new_v4(),
            session: SessionReference::Native("old-session".into()),
            role: Role::TopLevel,
        };
        journal.install_reattached(current.clone()).unwrap();
        let input = fixture("on_session_reset");
        let r = registration();
        let (clock, budget) = timing();
        let h = r
            .admit(
                &AdmissionRequest {
                    installed: InstallObservation::Unsupported(UnsupportedOperation {
                        adapter: "hermes",
                        operation: "PATH",
                    }),
                    input: Some(HookInput {
                        bytes: input.bytes.clone(),
                        registered_event: None,
                    }),
                    runtime_candidate: None,
                },
                &budget,
            )
            .unwrap();
        let event = r.decode(&h, &input).unwrap();
        assert_eq!(event.role, EventRole::Unknown);
        let args = crate::cli::hook::HookArgs {
            harness: current.harness,
            state_dir: Some(iso.state_root().into()),
            host_endpoint: Some(host),
            event: None,
        };
        let outcome = crate::cli::hook::run_admitted_hook(
            &args,
            r,
            &h,
            &event,
            &crate::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            Instant::now() + Duration::from_secs(1),
            clock,
            None,
        );
        assert!(outcome.attention.is_none());
        assert!(outcome.stdout.is_empty());
        assert_eq!(journal.current().unwrap(), Some(current.clone()));
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("context.json")).unwrap())
                .unwrap();
        assert_eq!(
            saved["declared_resets"].as_array().map(Vec::len),
            Some(1),
            "reset observer never records bounded local metadata"
        );
        let mut order = match event.intent {
            EventIntent::DeclaredReset(r) => r.ordering,
            _ => panic!("reset intent missing"),
        };
        order.sequence += 1;
        order.observed_at_millis = now();
        let turn = QualifiedTurn {
            session: "fixture-session".into(),
            event_key: "after-reset".into(),
            reset: None,
            ordering: Some(order),
        };
        let prepared = journal
            .get_or_prepare_qualified(current.harness, &current.target, &turn, now(), |c, kind| {
                assert_eq!(kind, EventKind::Clear);
                Ok(PendingCheckIn {
                    operation_id: uuid::Uuid::new_v4(),
                    mode: kind.mode(),
                    context: c.unwrap().for_event(
                        kind,
                        uuid::Uuid::new_v4(),
                        Some(turn.session.clone()),
                    )?,
                    expected_generation: Some(1),
                    event_id: turn.event_key.clone(),
                    payload_version: 1,
                    payload: b"synthetic pending bytes".to_vec(),
                })
            })
            .unwrap();
        drop(journal);
        let journal = ContextJournal::open(
            &directory,
            instance,
            "fixture-seat",
            Duration::from_millis(50),
        )
        .unwrap();
        assert_eq!(
            journal
                .get_or_prepare_qualified(
                    current.harness,
                    &current.target,
                    &turn,
                    now(),
                    |_, _| panic!("exact replay lost consumed reset")
                )
                .unwrap(),
            prepared
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("context.json")).unwrap())
                .unwrap();
        assert_eq!(saved["declared_resets"][0]["consumed"], true);
        assert_eq!(
            journal.prepared_kind_for_event(&turn.event_key).unwrap(),
            EventKind::Clear
        );
        let mut response_context = prepared.context.clone();
        response_context.binding_generation += 1;
        let response = super::super::context::CheckInResponse {
            context: response_context,
            historical: false,
            output: b"completed clear output".to_vec(),
        };
        journal
            .dispatch(&turn.event_key, &mut |_: &PendingCheckIn| {
                Ok(response.clone())
            })
            .unwrap();
        assert_eq!(
            journal.prepared_kind_for_event(&turn.event_key).unwrap(),
            EventKind::Clear
        );
        assert_eq!(
            journal
                .get_or_prepare_qualified(
                    current.harness,
                    &current.target,
                    &turn,
                    now(),
                    |_, _| panic!("completed clear replay reselected")
                )
                .unwrap(),
            prepared
        );

        assert_eq!(
            saved["prepared_kinds"][0]["kind"], "Clear",
            "immutable selected result mode is lost on replay"
        );
    }
    #[test]
    fn foreign_helper_content_is_preserved_before_any_launcher_execution() {
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-helper-ownership");
        let setup = iso.path("setup");
        crate::daemon::paths::ensure_private_dir(&setup).unwrap();
        let helper = setup.join("hermes-runtime-helper.py");
        std::fs::write(&helper, b"foreign helper bytes").unwrap();
        let environment = SetupEnvironment {
            state_dir: Some(iso.state_root().into()),
            ..Default::default()
        };
        let (_, budget) = timing();
        assert!(
            inspect_profile(
                "default",
                Some(Path::new("/missing/synthetic-launcher")),
                &environment,
                &budget
            )
            .is_err()
        );
        assert_eq!(
            super::super::setup::fingerprint(&std::fs::read(helper).unwrap()),
            super::super::setup::fingerprint(b"foreign helper bytes"),
            "setup overwrote an unowned helper before inspection"
        );
    }
    #[test]
    fn setup_consumer_uses_actual_discovery_producer_and_refuses_scope_mismatch_before_assets() {
        use crate::test_support::spawn::SpawnOwned;
        use std::{os::unix::fs::PermissionsExt, process::Stdio};
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-setup-consumer");
        let mut python = iso.command("python3");
        python
            .args(["-c", "import sys;print(sys.executable)"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = python.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(output.status.success());
        let interpreter = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
            .canonicalize()
            .unwrap();
        let root = iso.path("native source");
        let package = root.join("hermes_cli");
        std::fs::create_dir_all(&package).unwrap();
        let dependencies = iso.path("selected dependencies/site-packages");
        std::fs::create_dir_all(&dependencies).unwrap();
        let home = iso.path("selected home");
        std::fs::create_dir_all(home.join("profiles/work")).unwrap();
        for (path, bytes) in [
            (
                root.join("hermes_bootstrap.py"),
                "import os,sys\nfrom pathlib import Path\n_root=Path(__file__).resolve().parent\n_pm_repair=False\n_launch_python=None\nsys.path[:]=[str(_root),os.environ['FIXTURE_DEP'],*sys.path[1:]]\nos.environ['PYTHONPATH']=os.pathsep.join(sys.path[:2])\n",
            ),
            (
                root.join("hermes_constants.py"),
                "import os\nfrom pathlib import Path\ndef get_hermes_home(): return Path(os.environ['HERMES_HOME'])\n",
            ),
            (package.join("__init__.py"), ""),
            (
                package.join("profiles.py"),
                "import os\nfrom pathlib import Path\ndef normalize_profile_name(p): return p.strip().lower()\ndef validate_profile_name(p):\n if p not in ('default','work'): raise ValueError('private')\ndef resolve_profile_env(p):\n p=normalize_profile_name(p);validate_profile_name(p)\n root=Path(os.environ['FIXTURE_HOME']);home=root if p=='default' else root/'profiles'/p\n if not home.is_dir(): raise FileNotFoundError('private')\n return str(home)\n",
            ),
            (
                package.join("version_info.py"),
                "from types import SimpleNamespace\ndef get_version_info(): return SimpleNamespace(source='git',base_version='0.21.5',derived_version='0.21.5+1.g1234567',commit='1234567890abcdef1234567890abcdef12345678',dirty=False,distance=1)\n",
            ),
            (
                package.join("config.py"),
                "class FailedConfigRead(dict): pass\ndef load_config_readonly(): return {'plugins':{'enabled':['herdr-threads'],'disabled':[]},'other':{'keep':True}}\n",
            ),
        ] {
            std::fs::write(path, bytes).unwrap();
        }
        let bootstrap = format!(
            "import sys,runpy;sys.path.insert(0,{});import hermes_bootstrap;runpy.run_module('trace',run_name='__main__',alter_sys=True)",
            serde_json::to_string(root.to_str().unwrap()).unwrap()
        );
        let launcher = iso.path("synthetic-hermes");
        let script = format!(
            "#!{}\nimport json,sys\nprint(json.dumps([{},'-I','-c',{},'--count','--no-report',sys.argv[-3],'--profile',sys.argv[-1]]))\n",
            interpreter.display(),
            serde_json::to_string(interpreter.to_str().unwrap()).unwrap(),
            serde_json::to_string(&bootstrap).unwrap()
        );
        std::fs::write(&launcher, script).unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
        let environment = SetupEnvironment {
            home: Some(iso.home().into_os_string()),
            cwd: iso.path(""),
            state_dir: Some(iso.state_root().into()),
            host_endpoint: Some(iso.path("host.sock")),
            executable: iso.path("herdr-threads"),
            declared: [
                ("HERMES_HOME".into(), home.clone().into_os_string()),
                ("FIXTURE_HOME".into(), home.clone().into_os_string()),
                ("FIXTURE_DEP".into(), dependencies.into_os_string()),
            ]
            .into(),
            ..Default::default()
        };
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(
                environment.clock.monotonic_now().0 + 2000,
            ),
            cancellation: Default::default(),
        };
        let observed = inspect_profile("work", Some(&launcher), &environment, &budget)
            .expect("actual discovery cannot reach setup consumer");
        assert_eq!(observed.home, home.join("profiles/work"));
        let mut request = SetupRequest {
            scope: ResolvedSetupScope::Profile {
                name: "work".into(),
                home: home.clone(),
            },
            executable: environment.executable.clone(),
            environment: environment.clone(),
            native_binary: Some(launcher),
            options: Default::default(),
        };
        assert!(HermesAdapter.setup(&request, &budget).is_err());
        assert!(!home.join("plugins/herdr-threads").exists());
        request.scope = ResolvedSetupScope::Profile {
            name: "work".into(),
            home: observed.home.clone(),
        };
        let installed = HermesAdapter.setup(&request, &budget).unwrap();
        assert_eq!(installed.projection["native_acceptance"], "unmet");
        assert!(
            observed
                .home
                .join("plugins/herdr-threads/__init__.py")
                .is_file()
        );
        let status = HermesAdapter.status(
            &StatusRequest {
                scope: request.scope.clone(),
                environment: environment.clone(),
                native_binary: request.native_binary.clone(),
            },
            &budget,
        );
        let SetupStatus::Detailed(status) = status else {
            panic!("actual status producer refused")
        };
        assert!(status.installed);
        assert_eq!(status.enabled, Some(true));
        assert_eq!(status.admitted, None);
        assert_eq!(status.observed, None);
        let removed = HermesAdapter
            .unsetup(
                &UnsetupRequest {
                    scope: request.scope,
                    environment,
                },
                &budget,
            )
            .unwrap();
        assert!(removed.residue.is_empty());
    }
    #[test]
    fn actual_process_entrypoint_reaches_startup_callback_admission() {
        use crate::test_support::spawn::SpawnOwned;
        use std::process::Stdio;
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-process-consumer");
        let mut command = iso.command(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "harness::hermes::adapter_tests::hermes_process_entrypoint_child",
                "--ignored",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("{\"context\":null,\"lifecycle_ack\":null}"),
            "actual process facade refused qualified callback: {stdout}; {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    #[ignore = "owned process entrypoint fixture"]
    fn hermes_process_entrypoint_child() {
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-process-child");
        let r = super::super::registry::builtins();
        let args = crate::cli::hook::HookArgs {
            harness: super::super::registry::OccupantHarness::Agent(r.agent("hermes").unwrap())
                .into(),
            state_dir: Some(iso.state_root().into()),
            host_endpoint: None,
            event: None,
        };
        let input = fixture("post_tool_call");
        assert_eq!(
            crate::cli::hook::run_process_with(
                Ok(args),
                &crate::cli::hook::HookEnv {
                    herdr_env: true,
                    pane: Some("w1:p1".into())
                },
                std::io::Cursor::new(input.bytes)
            ),
            0
        );
    }
    #[test]
    fn immutable_qualified_modes_survive_pending_completed_replay_and_refuse_missing_metadata() {
        use super::super::context::{
            CheckInResponse, ContextJournal, EventKind, OccupantContext, PendingCheckIn,
            QualifiedTurn, Role, SessionReference,
        };
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-result-kinds");
        let directory = iso.path("contexts");
        crate::daemon::paths::ensure_private_dir(&directory).unwrap();
        let instance = uuid::Uuid::new_v4();
        let journal = ContextJournal::open(
            &directory,
            instance,
            "fixture-seat",
            Duration::from_millis(50),
        )
        .unwrap();
        let r = registration();
        let mut seed = OccupantContext {
            format_version: 1,
            instance,
            seat: "fixture-seat".into(),
            target: "w1:p1".into(),
            harness: super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into(),
            binding_generation: 1,
            execution: uuid::Uuid::new_v4(),
            session: SessionReference::Native("old".into()),
            role: Role::TopLevel,
        };
        for (number, session, want) in [
            (1, "first", EventKind::Startup),
            (2, "first", EventKind::Tool),
            (3, "rotated", EventKind::Startup),
        ] {
            let turn = QualifiedTurn {
                session: session.into(),
                event_key: format!("result-{number}"),
                reset: None,
                ordering: None,
            };
            let prepared = journal
                .get_or_prepare_qualified(
                    seed.harness,
                    &seed.target,
                    &turn,
                    now(),
                    |current, kind| {
                        assert_eq!(kind, want);
                        let current = current.unwrap_or(&seed);
                        Ok(PendingCheckIn {
                            operation_id: uuid::Uuid::new_v4(),
                            mode: kind.mode(),
                            context: current.for_event(
                                kind,
                                uuid::Uuid::new_v4(),
                                Some(session.into()),
                            )?,
                            expected_generation: if kind.mode()
                                == super::super::context::CheckInMode::Lifecycle
                            {
                                Some(current.binding_generation)
                            } else {
                                None
                            },
                            event_id: turn.event_key.clone(),
                            payload_version: 1,
                            payload: b"bounded synthetic request".to_vec(),
                        })
                    },
                )
                .unwrap();
            assert_eq!(
                journal.prepared_kind_for_event(&turn.event_key).unwrap(),
                want
            );
            assert_eq!(
                journal
                    .get_or_prepare_qualified(
                        seed.harness,
                        &seed.target,
                        &turn,
                        now(),
                        |_, _| panic!("pending response-loss replay reselected mode")
                    )
                    .unwrap(),
                prepared
            );
            let mut response_context = prepared.context.clone();
            if prepared.mode == super::super::context::CheckInMode::Lifecycle {
                response_context.binding_generation += 1;
            }
            let response = CheckInResponse {
                context: response_context.clone(),
                historical: false,
                output: b"synthetic completed output".to_vec(),
            };
            journal
                .dispatch(&turn.event_key, &mut |_: &PendingCheckIn| {
                    Ok(response.clone())
                })
                .unwrap();
            assert_eq!(
                journal.prepared_kind_for_event(&turn.event_key).unwrap(),
                want
            );
            assert_eq!(
                journal
                    .get_or_prepare_qualified(
                        seed.harness,
                        &seed.target,
                        &turn,
                        now(),
                        |_, _| panic!("completed replay reselected mode")
                    )
                    .unwrap(),
                prepared
            );
            seed = response_context;
        }
        let path = directory.join("context.json");
        let bytes = std::fs::read(&path).unwrap();
        let mut saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        saved["prepared_kinds"] = serde_json::json!([]);
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(
            journal.prepared_kind_for_event("result-3").is_err(),
            "missing metadata fabricated precise mode"
        );
        std::fs::write(&path, &bytes).unwrap();
        saved = serde_json::from_slice(&bytes).unwrap();
        saved["prepared_kinds"][0]["request_digest"] = serde_json::json!("sha256:wrong");
        std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(
            journal.prepared_kind_for_event("result-3").is_err(),
            "mismatched immutable request metadata was trusted"
        );
        let input = fixture("pre_llm_call");
        let (_, budget) = timing();
        let admitted = r
            .admit(
                &AdmissionRequest {
                    installed: InstallObservation::Unsupported(UnsupportedOperation {
                        adapter: "hermes",
                        operation: "PATH",
                    }),
                    input: Some(HookInput {
                        bytes: input.bytes.clone(),
                        registered_event: None,
                    }),
                    runtime_candidate: None,
                },
                &budget,
            )
            .unwrap();
        let event = r.decode(&admitted, &input).unwrap();
        let offer = NeutralOffer {
            fixed_guidance: "bounded actual context".into(),
            peer_data: serde_json::Value::Null,
            ready_argv: vec![],
        };
        assert!(
            r.encode(&admitted, &event, &offer).is_err(),
            "unselected QualifiedTurn fabricated startup ACK"
        );
        for (kind, mode) in [
            (EventKind::Startup, "startup"),
            (EventKind::Tool, "current"),
            (EventKind::Clear, "clear"),
        ] {
            let (bytes, consumes, diagnostic) = crate::cli::hook::encode_prepared_result(
                r,
                &admitted,
                &event,
                Some(kind),
                offer.fixed_guidance.clone(),
            );
            assert!(consumes);
            assert!(diagnostic.is_none(), "{diagnostic:?}");
            let output: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(output["lifecycle_ack"]["mode"], mode);
            assert_eq!(event.metadata.native_event, "pre_llm_call");
        }
    }
    #[test]
    fn hermes_strict_envelope_refuses_privacy_unknown_and_scope_timeout_shape_errors() {
        let base: serde_json::Value =
            serde_json::from_slice(&fixture("pre_llm_call").bytes).unwrap();
        for (path, value) in [
            ("body", serde_json::json!("PRIVATE")),
            ("parent_session_id", serde_json::Value::Null),
            ("session_id", serde_json::json!("x".repeat(257))),
            (
                "bridge_schema_contract_id",
                serde_json::json!("d73f44f51c4ef9dd"),
            ),
        ] {
            let mut v = base.clone();
            v[path] = value;
            assert!(
                parse(&HookInput {
                    bytes: serde_json::to_vec(&v).unwrap(),
                    registered_event: None
                })
                .is_err(),
                "accepted {path}"
            );
        }
        for (section, key, value) in [
            (
                "startup_capture",
                "api_contract",
                serde_json::json!("unknown"),
            ),
            (
                "startup_capture",
                "interpreter",
                serde_json::json!("relative"),
            ),
            (
                "timeout_observation",
                "age_seconds",
                serde_json::json!(5.01),
            ),
            (
                "timeout_observation",
                "quality",
                serde_json::json!("failed_config_read"),
            ),
            ("role_association", "turn_id", serde_json::json!("wrong")),
        ] {
            let mut v = base.clone();
            v[section][key] = value;
            assert!(
                parse(&HookInput {
                    bytes: serde_json::to_vec(&v).unwrap(),
                    registered_event: None
                })
                .is_err()
            );
        }
        let raw = String::from_utf8(fixture("pre_llm_call").bytes)
            .unwrap()
            .replace(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
            );
        assert!(
            parse(&HookInput {
                bytes: raw.into_bytes(),
                registered_event: None
            })
            .is_err()
        );
    }
    #[test]
    fn real_v2_recorder_verifies_two_domains_only_after_actual_qualified_callbacks() {
        use crate::{
            ports::{LocalClient, StorePort},
            protocol::{
                commands::Command,
                results::{CommandResult, HarnessEvidenceV2Recorded},
            },
            test_support::counting_client::{CountingLocalClient, DaemonVintage},
        };
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-rich-recorder");
        let (clock, budget) = timing();
        let store = Arc::new(
            crate::store::SqliteStore::new(
                crate::store::connection::StoreContext::new(
                    iso.state_root().join("store.db"),
                    Arc::clone(&clock),
                ),
                "synthetic-instance",
                crate::store::StoreSettings::default(),
            )
            .unwrap(),
        );
        let recorder = Arc::new(
            crate::daemon::harness_evidence::HarnessEvidenceRecorderV2::new(
                store.clone(),
                None,
                Arc::clone(&clock),
            ),
        );
        let call_budget = budget.clone();
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| match command {
                Command::HarnessEvidenceV2(note) => {
                    recorder.record(note, &call_budget).map(|verified| {
                        CommandResult::HarnessEvidenceV2Recorded(HarnessEvidenceV2Recorded {
                            verified,
                        })
                    })
                }
                other => panic!("advisory evidence issued authority command {other:?}"),
            },
            DaemonVintage::Current,
        ));
        for (callback, want) in [
            ("on_session_start", false),
            ("pre_llm_call", false),
            ("post_tool_call", true),
        ] {
            let input = fixture(callback);
            assert_eq!(
                crate::cli::hook_evidence::run_registered(
                    registration(),
                    None,
                    &input.bytes,
                    None,
                    now() as u64,
                    (&budget, clock.as_ref()),
                    |_| Some((
                        Arc::clone(&client) as Arc<dyn LocalClient>,
                        client.capabilities()
                    ))
                ),
                crate::cli::hook_evidence::Delivery::Sent(Some(want))
            );
        }
        let rows = store.harness_evidence_v2_all("hermes", 0, &budget).unwrap();
        assert_eq!(rows.len(), 2);
        for row in rows {
            let descriptor = CONTRACTS
                .iter()
                .find(|d| d.domain_id == row.domain)
                .unwrap();
            assert!(row.verified(descriptor));
            assert!(row.identity.release().is_none());
        }
    }
    #[test]
    fn known_broken_manifest_refuses_exact_identity_in_either_domain_without_semver_floor() {
        let input = fixture("pre_llm_call");
        let envelope = parse(&input).unwrap();
        for d in CONTRACTS {
            let manifest=super::super::manifest::parse(&serde_json::to_vec(&serde_json::json!({"schema_version":2,"rows":[],"runtime_contracts":{"hermes":[{"domain":d.domain_id,"origin":d.origin,"id":d.contract_id_v2().unwrap(),"events":d.events.iter().map(|e|serde_json::json!({"event":e.native_event,"milestone":e.milestone,"always_send":e.always_send})).collect::<Vec<_>>(),"required_milestones":d.required_milestones}]},"runtime_rows":[{"harness":"hermes","identity":envelope.runtime_identity,"domain":d.domain_id,"origin":d.origin,"contract_id":d.contract_id_v2().unwrap(),"status":"known_broken","evidence_stage":"source_captured","source":"manual","required_milestones":d.required_milestones,"successful_milestones":[],"broken_event":"pre_llm_call","broken_field":if d.domain==ContractDomain::Native {"shape.parent_session_id.type"} else {"parent_session_id"},"supported_since":null,"issue_url":null,"last_seen_at":1}]})).unwrap()).unwrap();
            let request = AdmissionRequest {
                installed: InstallObservation::Unsupported(UnsupportedOperation {
                    adapter: "hermes",
                    operation: "PATH",
                }),
                input: Some(HookInput {
                    bytes: input.bytes.clone(),
                    registered_event: None,
                }),
                runtime_candidate: None,
            };
            assert!(
                matches!(
                    admit_candidate(&request, &manifest),
                    AdmissionDecision::Refused { .. }
                ),
                "known-broken exact domain admitted as candidate"
            );
            let mut other: serde_json::Value = serde_json::from_slice(&input.bytes).unwrap();
            let mut descriptor = envelope.runtime_identity.descriptor.clone();
            descriptor.base_version = Some("0.21.6".into());
            let identity = RuntimeIdentity::build(descriptor).unwrap();
            other["runtime_identity"] = serde_json::to_value(identity).unwrap();
            let request = AdmissionRequest {
                input: Some(HookInput {
                    bytes: serde_json::to_vec(&other).unwrap(),
                    registered_event: None,
                }),
                ..request
            };
            assert!(
                matches!(
                    admit_candidate(&request, &manifest),
                    AdmissionDecision::Optimistic { .. }
                ),
                "exact broken row became a guessed release floor"
            );
        }
    }
    #[test]
    fn reset_hints_are_inert_without_current_or_for_other_targets_and_expired_order_cannot_clear() {
        use super::super::context::{
            ContextJournal, DeclaredReset, EventKind, ObservationOrder, OccupantContext,
            PendingCheckIn, QualifiedTurn, Role, SessionReference,
        };
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-reset-conservative");
        let directory = iso.path("contexts");
        crate::daemon::paths::ensure_private_dir(&directory).unwrap();
        let instance = uuid::Uuid::new_v4();
        let journal = ContextJournal::open(
            &directory,
            instance,
            "fixture-seat",
            Duration::from_millis(50),
        )
        .unwrap();
        let harness: super::super::context::Harness =
            super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into();
        let current = OccupantContext {
            format_version: 1,
            instance,
            seat: "fixture-seat".into(),
            target: "w1:p1".into(),
            harness,
            binding_generation: 1,
            execution: uuid::Uuid::new_v4(),
            session: SessionReference::Native("old".into()),
            role: Role::TopLevel,
        };
        let at = now() - 600001;
        let reset = DeclaredReset {
            session: "new".into(),
            event_key: "declared-old-reset".into(),
            ordering: ObservationOrder {
                process_nonce: uuid::Uuid::new_v4(),
                sequence: 2,
                observed_at_millis: at,
                callback_budget_millis: 1200,
            },
        };
        assert!(
            !journal
                .record_declared_reset(harness, &current.target, &reset, at)
                .unwrap()
        );
        assert!(!directory.join("context.json").exists());
        journal.install_reattached(current.clone()).unwrap();
        assert!(
            !journal
                .record_declared_reset(harness, "other-target", &reset, at)
                .unwrap()
        );
        assert!(
            journal
                .record_declared_reset(harness, &current.target, &reset, at)
                .unwrap()
        );
        assert!(
            !journal
                .record_declared_reset(harness, &current.target, &reset, at)
                .unwrap()
        );
        let mut stale = reset.clone();
        stale.event_key = "stale-reset".into();
        stale.ordering.sequence = 1;
        assert!(
            !journal
                .record_declared_reset(harness, &current.target, &stale, at)
                .unwrap()
        );
        let turn = QualifiedTurn {
            session: "new".into(),
            event_key: "after-expiry".into(),
            reset: None,
            ordering: Some(ObservationOrder {
                observed_at_millis: now(),
                sequence: 3,
                ..reset.ordering.clone()
            }),
        };
        journal
            .get_or_prepare_qualified(harness, &current.target, &turn, now(), |saved, kind| {
                assert_eq!(
                    kind,
                    EventKind::Startup,
                    "expired reset hint manufactured Clear"
                );
                Ok(PendingCheckIn {
                    operation_id: uuid::Uuid::new_v4(),
                    mode: kind.mode(),
                    context: saved.unwrap().for_event(
                        kind,
                        uuid::Uuid::new_v4(),
                        Some(turn.session.clone()),
                    )?,
                    expected_generation: Some(1),
                    event_id: turn.event_key.clone(),
                    payload_version: 1,
                    payload: b"synthetic immutable request".to_vec(),
                })
            })
            .unwrap();
        let fresh = DeclaredReset {
            event_key: "later-reset".into(),
            ordering: ObservationOrder {
                observed_at_millis: now(),
                sequence: 4,
                ..reset.ordering
            },
            ..reset
        };
        assert!(
            !journal
                .record_declared_reset(harness, &current.target, &fresh, now())
                .unwrap(),
            "observer overwrote pending qualified request"
        );
        assert_eq!(journal.current().unwrap(), Some(current));
        assert_eq!(
            journal.prepared_kind_for_event(&turn.event_key).unwrap(),
            EventKind::Startup
        );
    }
    // Admit real valid Hermes fixture bytes, then deterministically model the
    // decode-time refusal caused by deadline expiry after admission. No native
    // execution or real-wall-clock race is needed to exercise the consumer.
    #[cfg(feature = "test-support")]
    struct CallbackDecodeFailure;
    #[cfg(feature = "test-support")]
    impl HarnessAdapter for CallbackDecodeFailure {
        type Admission = HermesAdmission;
        fn metadata(&self) -> &'static AdapterMetadata {
            HermesAdapter.metadata()
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            CONTRACTS
        }
        fn observe_install(&self, e: &InstallEnvironment, b: &CallBudget) -> InstallObservation {
            HermesAdapter.observe_install(e, b)
        }
        fn admit(
            &self,
            r: &AdmissionRequest,
            b: &CallBudget,
        ) -> AdmissionDecision<Self::Admission> {
            HermesAdapter.admit(r, b)
        }
        fn version_ladder(&self, r: &RuntimeIdentity) -> Ladder {
            HermesAdapter.version_ladder(r)
        }
        fn classify(&self, i: &HookInput) -> ContractObservation {
            HermesAdapter.classify(i)
        }
        fn hook_admission_policy(&self) -> HookAdmissionPolicy {
            HookAdmissionPolicy::QualifiedCallback
        }
        fn decode(
            &self,
            _: &HermesAdmission,
            _: &HookInput,
        ) -> Result<DecodedEvent, DecodeFailure> {
            Err(DecodeFailure::Native(
                super::super::context::ContextError::Conflict,
            ))
        }
        fn encode(
            &self,
            a: &HermesAdmission,
            e: &DecodedEvent,
            o: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            HermesAdapter.encode(a, e, o)
        }
        fn attribute_runtime(&self, i: &HookInput, b: &CallBudget) -> RuntimeAttribution {
            HermesAdapter.attribute_runtime(i, b)
        }
        fn resolve_setup_scope(
            &self,
            r: &SetupScopeRequest,
            e: &SetupEnvironment,
        ) -> Result<ResolvedSetupScope, SetupFailure> {
            HermesAdapter.resolve_setup_scope(r, e)
        }
        fn setup(&self, r: &SetupRequest, b: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            HermesAdapter.setup(r, b)
        }
        fn status(&self, r: &StatusRequest, b: &CallBudget) -> SetupStatus {
            HermesAdapter.status(r, b)
        }
        fn unsetup(
            &self,
            r: &UnsetupRequest,
            b: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            HermesAdapter.unsetup(r, b)
        }
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn callback_admitted_decode_failure_quietly_refuses_without_consumption() {
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-decode-refusal");
        let input = fixture("pre_llm_call");
        let (clock, _) = timing();
        let args = crate::cli::hook::HookArgs {
            harness: super::super::registry::OccupantHarness::Agent(
                super::super::registry::builtins().agent("hermes").unwrap(),
            )
            .into(),
            state_dir: Some(iso.path("state")),
            host_endpoint: Some(iso.path("host.sock")),
            event: None,
        };
        static ADAPTER: CallbackDecodeFailure = CallbackDecodeFailure;
        let registrations = Box::leak(Box::new([super::super::registry::Registration::new(
            &ADAPTER,
        )]));
        let registry = super::super::registry::Registry::new(registrations).unwrap();
        let registration = registry.by_id(registry.agent("hermes").unwrap()).unwrap();
        // Reintroducing the optional-compatibility unwrap must panic here;
        // refusal before admission would fail the decode diagnostic assertion.
        let outcome = crate::cli::hook::run_hook_registered(
            registration,
            &args,
            &crate::cli::hook::InstalledHarness::Claude("9.9.9".into()),
            &input.bytes,
            &crate::cli::hook::HookEnv {
                herdr_env: true,
                pane: Some("w1:p1".into()),
            },
            Instant::now() + Duration::from_secs(1),
            clock,
            Some(&iso.path("must-not-start-daemon")),
        );
        assert!(
            outcome.stdout.is_empty(),
            "refused callback emitted context or an ACK"
        );
        assert!(
            outcome.attention.is_none(),
            "refused callback consumed attention"
        );
        assert_eq!(
            outcome.diagnostic.as_deref(),
            Some("unsupported hook payload: Native(Conflict)")
        );
        assert!(
            !iso.path("state").exists(),
            "refused callback wrote journal/evidence or started a daemon"
        );
    }
    struct InvalidProjection(u8);
    impl HarnessAdapter for InvalidProjection {
        type Admission = HermesAdmission;
        fn metadata(&self) -> &'static AdapterMetadata {
            HermesAdapter.metadata()
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            CONTRACTS
        }
        fn observe_install(&self, e: &InstallEnvironment, b: &CallBudget) -> InstallObservation {
            HermesAdapter.observe_install(e, b)
        }
        fn admit(
            &self,
            r: &AdmissionRequest,
            b: &CallBudget,
        ) -> AdmissionDecision<Self::Admission> {
            HermesAdapter.admit(r, b)
        }
        fn version_ladder(&self, r: &RuntimeIdentity) -> Ladder {
            HermesAdapter.version_ladder(r)
        }
        fn classify(&self, i: &HookInput) -> ContractObservation {
            HermesAdapter.classify(i)
        }
        fn decode(
            &self,
            a: &HermesAdmission,
            i: &HookInput,
        ) -> Result<DecodedEvent, DecodeFailure> {
            HermesAdapter.decode(a, i)
        }
        fn encode(
            &self,
            a: &HermesAdmission,
            e: &DecodedEvent,
            o: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            HermesAdapter.encode(a, e, o)
        }
        fn attribute_runtime(&self, i: &HookInput, b: &CallBudget) -> RuntimeAttribution {
            HermesAdapter.attribute_runtime(i, b)
        }
        fn resolve_setup_scope(
            &self,
            r: &SetupScopeRequest,
            e: &SetupEnvironment,
        ) -> Result<ResolvedSetupScope, SetupFailure> {
            HermesAdapter.resolve_setup_scope(r, e)
        }
        fn setup(&self, r: &SetupRequest, b: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            HermesAdapter.setup(r, b)
        }
        fn status(&self, r: &StatusRequest, b: &CallBudget) -> SetupStatus {
            HermesAdapter.status(r, b)
        }
        fn unsetup(
            &self,
            r: &UnsetupRequest,
            b: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            HermesAdapter.unsetup(r, b)
        }
        fn evidence_observations(&self, input: &HookInput) -> Vec<EvidenceProjection> {
            let mut projected = HermesAdapter.evidence_observations(input);
            match self.0 {
                0 => projected.push(HermesAdapter.evidence_observations(input).remove(0)),
                1 => projected[0].origin = EvidenceOrigin::BridgeEnvelope,
                2 => projected[0].contract_id = "0000000000000000".into(),
                3 => {
                    projected[0].classification = contract::Classification::Ok {
                        event: "undeclared",
                    }
                }
                4 => projected.clear(),
                _ => {
                    for _ in 0..7 {
                        projected.push(HermesAdapter.evidence_observations(input).remove(0));
                    }
                }
            }
            projected
        }
    }
    #[test]
    fn invalid_duplicate_and_undeclared_projection_refuse_before_any_io() {
        let input = fixture("pre_llm_call");
        let (clock, budget) = timing();
        for invalid in 0..6 {
            let adapter = Box::leak(Box::new(InvalidProjection(invalid)));
            let registration = super::super::registry::Registration::new(adapter);
            assert_eq!(
                crate::cli::hook_evidence::run_registered(
                    &registration,
                    None,
                    &input.bytes,
                    None,
                    now() as u64,
                    (&budget, clock.as_ref()),
                    |_| panic!("invalid projection connected before declaration validation")
                ),
                crate::cli::hook_evidence::Delivery::Unsupported
            );
        }
    }
    #[test]
    fn shared_budget_exhaustion_cannot_advance_unsent_domain_or_claim_verification() {
        use crate::{
            ports::LocalClient,
            protocol::{
                commands::Command,
                results::{CommandResult, HarnessEvidenceV2Recorded},
            },
            test_support::counting_client::{CountingLocalClient, DaemonVintage},
        };
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-shared-budget");
        let (clock, budget) = timing();
        let input = fixture("pre_llm_call");
        let cancellation = budget.cancellation.clone();
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| {
                assert!(matches!(command, Command::HarnessEvidenceV2(_)));
                cancellation.cancel();
                Ok(CommandResult::HarnessEvidenceV2Recorded(
                    HarnessEvidenceV2Recorded { verified: true },
                ))
            },
            DaemonVintage::Current,
        ));
        assert_eq!(
            crate::cli::hook_evidence::run_registered(
                registration(),
                None,
                &input.bytes,
                Some(iso.state_root()),
                now() as u64,
                (&budget, clock.as_ref()),
                |_| Some((
                    Arc::clone(&client) as Arc<dyn LocalClient>,
                    client.capabilities()
                ))
            ),
            crate::cli::hook_evidence::Delivery::Sent(None)
        );
        assert_eq!(client.total_calls(), 1);
        let gates = crate::cli::hook_evidence::v2_gate_dir(iso.state_root());
        assert!(
            !gates.exists() || std::fs::read_dir(gates).unwrap().next().is_none(),
            "late reply advanced domain gate after shared budget exhausted"
        );
    }
    #[test]
    fn partial_domain_failure_retries_only_failed_domain_and_old_daemon_stays_unsupported() {
        use crate::{
            ports::LocalClient,
            protocol::{
                commands::Command,
                results::{ApiError, CommandResult, ErrorCode, HarnessEvidenceV2Recorded},
            },
            test_support::counting_client::{CountingLocalClient, DaemonVintage},
        };
        let iso = crate::test_support::isolation::TestIsolation::new("hermes-evidence-domains");
        let notes = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&notes);
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| {
                let Command::HarnessEvidenceV2(note) = command else {
                    panic!("evidence path issued authority command")
                };
                let mut notes = recorded.lock().unwrap();
                notes.push(note.clone());
                if notes.len() == 2 {
                    return Err(ApiError::new(
                        ErrorCode::InvalidRequest,
                        "synthetic second-domain transport failure",
                    ));
                }
                Ok(CommandResult::HarnessEvidenceV2Recorded(
                    HarnessEvidenceV2Recorded { verified: true },
                ))
            },
            DaemonVintage::Current,
        ));
        let (clock, budget) = timing();
        let input = fixture("pre_llm_call");
        let at = now() as u64;
        let run = |now_ms| {
            crate::cli::hook_evidence::run_registered(
                registration(),
                None,
                &input.bytes,
                Some(iso.state_root()),
                now_ms,
                (&budget, clock.as_ref()),
                |_| {
                    Some((
                        Arc::clone(&client) as Arc<dyn LocalClient>,
                        client.capabilities(),
                    ))
                },
            )
        };
        assert_eq!(run(at), crate::cli::hook_evidence::Delivery::Sent(None));
        assert_eq!(
            run(at + 1),
            crate::cli::hook_evidence::Delivery::Sent(Some(true))
        );
        assert_eq!(
            notes
                .lock()
                .unwrap()
                .iter()
                .map(|n| n.domain.as_str())
                .collect::<Vec<_>>(),
            vec!["native_callback", "bridge_envelope", "bridge_envelope"]
        );
        assert_eq!(run(at + 2), crate::cli::hook_evidence::Delivery::Suppressed);
        let old = Arc::new(CountingLocalClient::scripted(
            |_| panic!("rich evidence downgraded to old daemon"),
            DaemonVintage::Older,
        ));
        assert_eq!(
            crate::cli::hook_evidence::run_registered(
                registration(),
                None,
                &input.bytes,
                None,
                at,
                (&budget, clock.as_ref()),
                |_| Some((Arc::clone(&old) as Arc<dyn LocalClient>, old.capabilities()))
            ),
            crate::cli::hook_evidence::Delivery::Unsupported
        );
        assert_eq!(old.total_calls(), 0);
    }
    #[test]
    #[cfg(feature = "test-support")]
    fn both_actual_hermes_domain_projections_reach_one_generic_connection() {
        use crate::ports::LocalClient;
        use crate::protocol::{
            commands::Command,
            results::{CommandResult, HarnessEvidenceV2Recorded},
        };
        use crate::test_support::counting_client::{CountingLocalClient, DaemonVintage};
        let notes = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&notes);
        let client = Arc::new(CountingLocalClient::scripted(
            move |command| {
                let Command::HarnessEvidenceV2(note) = command else {
                    panic!("observer issued consuming command")
                };
                recorded.lock().unwrap().push(note.clone());
                Ok(CommandResult::HarnessEvidenceV2Recorded(
                    HarnessEvidenceV2Recorded { verified: false },
                ))
            },
            DaemonVintage::Current,
        ));
        let (clock, budget) = timing();
        let input = fixture("pre_llm_call");
        let mut connects = 0;
        let delivery = crate::cli::hook_evidence::run_registered(
            registration(),
            None,
            &input.bytes,
            None,
            now() as u64,
            (&budget, clock.as_ref()),
            |_| {
                connects += 1;
                Some((
                    Arc::clone(&client) as Arc<dyn LocalClient>,
                    client.capabilities(),
                ))
            },
        );
        assert_eq!(connects, 1);
        assert_eq!(
            delivery,
            crate::cli::hook_evidence::Delivery::Sent(Some(false))
        );
        let notes = notes.lock().unwrap();
        assert_eq!(
            notes.len(),
            2,
            "generic consumer drops one exact evidence domain"
        );
        assert_eq!(notes[0].origin, EvidenceOrigin::NativeShapeObservation);
        assert_eq!(notes[1].origin, EvidenceOrigin::BridgeEnvelope);
        assert_eq!(notes[0].qualifications.len(), 3);
        assert_eq!(notes[1].qualifications.len(), 4);
        drop(notes);
    }
}

#[cfg(test)]
mod launch_boundary_tests {
    #[test]
    fn hermes_classic_launch_captured_scope_enablement_and_permission_flags() {
        let registry = crate::harness::registry::builtins();
        let registration = registry.by_id(registry.agent("hermes").unwrap()).unwrap();
        let policy = registration
            .launch_policy()
            .expect("Hermes positive launch provider is missing");
        assert!(policy.requires_process_hint());
        assert!(
            policy
                .validate_native_argv(&[
                    "--model".into(),
                    "model Ω".into(),
                    "-q".into(),
                    "first turn".into()
                ])
                .is_ok()
        );
        for flag in [
            "--oneshot",
            "--yolo",
            "--ignore-user-config",
            "gateway",
            "--resume",
        ] {
            assert!(policy.validate_native_argv(&[flag.into()]).is_err());
        }
    }
}

#[cfg(test)]
mod companion_boundary_tests {
    #[test]
    fn hermes_canary_exact_runtime_missing_input_is_explicit() {
        let registration = crate::harness::registry::builtins()
            .by_id(
                crate::harness::registry::builtins()
                    .agent("hermes")
                    .unwrap(),
            )
            .unwrap();
        let strategy = registration
            .canary_strategy()
            .expect("Hermes exact-runtime companion is absent");
        let descriptor = strategy.descriptor();
        assert_eq!(
            descriptor.kind,
            crate::harness::adapter::CanaryKind::ExactRuntime
        );
        assert_eq!(
            descriptor.candidate_kind,
            crate::harness::adapter::CandidateKind::ExactBuild
        );
        assert!(descriptor.npm_package.is_none());
        assert!(descriptor.model_key_env.is_none());
        assert_eq!(descriptor.artifact_schema_version, 1);
        assert_eq!(descriptor.companion, "scripts/canary/adapters/hermes.py");
    }
}
