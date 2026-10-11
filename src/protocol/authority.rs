use crate::protocol::{ids::*, time::*};
use serde::{Deserialize, Serialize};

/// Honest cooperative context. A dishonest child can declare `top_level`;
/// these fields do not provide native execution attestation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallerClaim {
    pub instance: String,
    pub seat: SeatId,
    pub binding_generation: u64,
    pub role: CallerRole,
    pub harness: Harness,
    pub native_session: NativeSessionId,
    pub execution: ExecutionId,
    pub target: HostTargetId,
    /// Subagent agent id (Claude `--agent-id`, Codex `CODEX_THREAD_ID` when it differs from
    /// `CODEX_SESSION_ID`). Absent for a top-level caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// The caller's liveness fingerprint (decision 9). Required of every agent claim that creates
    /// or connects a binding; human claims carry none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liveness: Option<LivenessFingerprint>,
}

/// The stable detail of `ErrorCode::SubagentReadOnly`.
pub const SUBAGENT_READ_ONLY_DETAIL: &str = "subagent bindings are read-only: they may read, summarize and advance their own cursor, never ACK, send, accept, reject, leave, invite or check in for the seat";

impl CallerClaim {
    /// A subagent claim (ht-54r.20): the declared role or any agent id. Cooperative recognition,
    /// not verification (TRUST-POLICY).
    pub fn is_subagent(&self) -> bool {
        self.role == CallerRole::Subagent || self.agent_id.is_some()
    }

    /// Wire-level checks of the binding fields (ids are validated by their types).
    pub fn validate_binding_fields(&self) -> Result<(), &'static str> {
        if self.liveness.is_some_and(|f| f.pid == 0) {
            return Err("invalid liveness fingerprint pid");
        }
        Ok(())
    }
}

/// Liveness fingerprint (spec §1, decision 9): the harness process's pid and its OS start
/// stamp. `start` is opaque: it is only ever compared for equality with a value produced by the
/// same `ProcessInfo` implementation on the same host (macOS: microseconds since the Unix epoch
/// from `kp_proc.p_starttime`; Linux: `/proc/<pid>/stat` field 22, clock ticks since boot).
/// A pid whose current start stamp differs is a different process (pid reuse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LivenessFingerprint {
    pub pid: u32,
    pub start: u64,
}
impl LivenessFingerprint {
    pub fn of(record: &crate::ports::ProcessRecord) -> Self {
        Self {
            pid: record.pid,
            start: record.start,
        }
    }
    /// A fixed fingerprint for tests that need an agent claim to enroll (the daemon requires one
    /// on every claim that creates or connects a binding). It names no real process.
    #[cfg(any(test, feature = "test-support"))]
    pub const TEST: Self = Self { pid: 1, start: 1 };
    /// The fingerprint of this very process, for tests whose daemon runs the liveness worker: a
    /// binding carrying [`Self::TEST`] is judged an exited process and disconnected there.
    #[cfg(any(test, feature = "test-support"))]
    pub fn current_process() -> Self {
        use crate::ports::ProcessInfo;
        let record = crate::host::process_table::HostProcessTable
            .process(std::process::id())
            .expect("this process is in the process table");
        Self::of(&record)
    }
    /// True iff the fingerprinted process still exists with the same start stamp.
    pub fn is_alive(&self, info: &dyn crate::ports::ProcessInfo) -> bool {
        info.process(self.pid)
            .is_some_and(|p| p.start == self.start)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerRole {
    TopLevel,
    Subagent,
}

pub use crate::harness::registry::OccupantHarness as Harness;

/// An agent's cooperative top-level claim (not native attestation).
pub const COOPERATIVE_TOP_LEVEL_PROVENANCE: &str = "cooperative_top_level";
/// TRUST-POLICY A3: the pane's top-level Claude session, through the bundled
/// mod's `watch` child, opened a delivery channel for the current binding
/// generation. Channel registrations only; never on bindings or receipts.
pub const COOPERATIVE_MOD_CHANNEL_PROVENANCE: &str = "cooperative_mod_channel";
/// TRUST-POLICY A3: receipt action observation (and lazy completion claim)
/// for an item the mod reported delivered (spec D6). Not proof of reading.
pub const COOPERATIVE_MOD_DELIVERY_PROVENANCE: &str = "cooperative_mod_delivery";

/// TRUST-POLICY A3: a plain cooperative call enrolled its session's binding (and a Codex or
/// Hermes subagent attached to its parent) from the harness environment, without a hook.
pub const COOPERATIVE_ENV_SESSION_PROVENANCE: &str = "cooperative_env_session";
/// TRUST-POLICY A3: a Claude subagent's binding, attached from its hook-carried agent id.
pub const COOPERATIVE_HOOK_AGENT_PROVENANCE: &str = "cooperative_hook_agent";
/// TRUST-POLICY A3; defined, behaviour lands in later beads (ht-54r).
pub const LIVENESS_PROCESS_EXIT_PROVENANCE: &str = "liveness_process_exit";
/// TRUST-POLICY A3; defined, behaviour lands in later beads (ht-54r).
pub const LIVENESS_END_EVENT_PROVENANCE: &str = "liveness_end_event";

/// Whether Herdr actually reports a registered native host kind. Registry
/// metadata alone is never evidence of native recognition or continuity.
pub fn is_harness_agent_kind(kind: &str) -> bool {
    crate::harness::registry::builtins()
        .by_host_kind(kind)
        .is_some()
}
/// A person acting from their own pane identity (`herdr-threads human me init`).
pub const OPERATOR_HUMAN_PROVENANCE: &str = "operator_human";
/// C4: bindings a structural reconfirmation carries to a new host epoch;
/// native `verified_current_target` bindings re-register instead. The planner
/// predicate and the applier both read this one set.
pub const CARRIED_BINDING_PROVENANCES: [&str; 4] = [
    COOPERATIVE_TOP_LEVEL_PROVENANCE,
    OPERATOR_HUMAN_PROVENANCE,
    COOPERATIVE_ENV_SESSION_PROVENANCE,
    COOPERATIVE_HOOK_AGENT_PROVENANCE,
];
/// TRUST-POLICY A3/C1: a seat was reattached because a resumed harness
/// session id matched an unresolved seat's last binding. Seat rebinds only
/// (the `allocation_decisions.kind` of the reattachment); never on receipts
/// and never the provenance of a binding (the binding the reattached seat
/// opens stays `cooperative_top_level`).
pub const COOPERATIVE_CONTINUITY_PROVENANCE: &str = "cooperative_continuity";
/// TRUST-POLICY A3: Herdr's guarded `agent.start` in this pane was observed
/// starting the harness and the agent has not checked in. Bindings only (an
/// unregistered occupant binding the daemon opens after a correlated launch,
/// on a seat with no open binding); never on receipts; it authorizes nothing
/// but a wake prompt to the bound harness. Its session and execution are
/// placeholders ([`MANAGED_LAUNCH_PLACEHOLDER_PREFIX`]) that no caller claim
/// can match, and the first lifecycle check-in replaces it.
pub const MANAGED_LAUNCH_PROVENANCE: &str = "managed_launch";
/// Prefix of a `managed_launch` binding's placeholder native session and
/// execution: never a canonical UUID, so no cooperative claim matches it.
pub const MANAGED_LAUNCH_PLACEHOLDER_PREFIX: &str = "launch:";
/// TRUST-POLICY A4: the open-binding provenances that are an agent's, which a
/// person's `me init` never replaces without `--operator` and a second
/// `launch` respects.
pub const AGENT_BINDING_PROVENANCES: [&str; 4] = [
    COOPERATIVE_TOP_LEVEL_PROVENANCE,
    MANAGED_LAUNCH_PROVENANCE,
    COOPERATIVE_ENV_SESSION_PROVENANCE,
    COOPERATIVE_HOOK_AGENT_PROVENANCE,
];

/// Obtained from the local socket kernel credential, never from JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerIdentity {
    effective_uid: u32,
}
impl PeerIdentity {
    pub(crate) fn from_kernel(effective_uid: u32) -> Self {
        Self { effective_uid }
    }
    pub fn effective_uid(&self) -> u32 {
        self.effective_uid
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorActor {
    effective_uid: u32,
}
impl OperatorActor {
    pub(crate) fn effective_uid(&self) -> u32 {
        self.effective_uid
    }
    pub(crate) fn from_peer(peer: PeerIdentity, owner_uid: u32) -> Option<Self> {
        (peer.effective_uid == owner_uid).then_some(Self {
            effective_uid: owner_uid,
        })
    }
    pub fn audit_label(&self) -> String {
        format!("operator:local-user:{}", self.effective_uid)
    }
    /// Canonical durable operation scope, isolated by daemon instance.
    pub fn operation_scope(&self, instance: &str) -> String {
        format!("operator:{instance}:local-user:{}", self.effective_uid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObligationRef {
    /// Issuer request marker only: freeze the current addressed invitation before issuing.
    AcceptCurrent(ThreadId),
    Invitation(InvitationId),
    Receipt {
        message: MessageId,
        seat: SeatId,
    },
    CheckIn(SeatId),
    Control(ThreadId),
}

pub const MAX_PERMIT_MILLIS: u64 = 250;

/// Current host state sampled at the transaction decision. A known invalidation
/// forbids committing even if a preceding observation was fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionFence {
    pub now: MonoInstant,
    pub host_boot: HostBootId,
    pub host_epoch: u64,
    pub target_generation: u64,
    pub binding_generation: u64,
    pub known_invalidated: bool,
}

/// Local durable mapping fence, independent of native execution attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooperativeDecisionFence {
    pub now: MonoInstant,
    pub instance: String,
    pub seat: SeatId,
    pub target: HostTargetId,
    pub mapping_revision: u64,
    pub binding_generation: u64,
    pub invalidation_revision: u64,
    pub known_invalidated: bool,
}

/// Private runtime requirements are never accepted from a wire decoder.
#[derive(Debug, Clone)]
pub(crate) enum HandoffRequirement {
    Delivery(Box<crate::protocol::handoff::DeliveryMutation>),
    BootstrapChild {
        namespace: crate::protocol::handoff::HandoffNamespace,
        command: Box<crate::protocol::commands::PermitMutation>,
    },
}
/// One transaction decision may consume this exact request/payload grant once.
#[derive(Debug)]
pub struct MutationPermit {
    claim: CallerClaim,
    mapping_revision: u64,
    invalidation_revision: u64,
    request: OperationId,
    obligation: ObligationRef,
    payload_hash: [u8; 32],
    observed_at: MonoInstant,
    consumed: bool,
    cooperative_budget: CallBudget,
    handoff_requirement: Option<HandoffRequirement>,
}
impl MutationPermit {
    /// Stable durable-seat scope for looking up a previously committed operation.
    /// Reading this identity does not consume the permit or authorize new work.
    pub(crate) fn seat_for_replay_scope(&self) -> &SeatId {
        &self.claim.seat
    }
    /// Issued only after service-local durable context validation. No wire decoder.
    pub(crate) fn cooperative(
        claim: CallerClaim,
        request: OperationId,
        obligation: ObligationRef,
        payload_hash: [u8; 32],
        observed_at: MonoInstant,
        (mapping_revision, invalidation_revision): (u64, u64),
        budget: CallBudget,
    ) -> Self {
        Self {
            claim,
            mapping_revision,
            invalidation_revision,
            request,
            obligation,
            payload_hash,
            observed_at,
            consumed: false,
            cooperative_budget: budget,
            handoff_requirement: None,
        }
    }
    /// Selected store attaches only a validated explicit frozen delivery envelope.
    pub(crate) fn with_delivery(
        mut self,
        request: &crate::protocol::handoff::DeliveryMutation,
    ) -> Self {
        self.handoff_requirement = Some(HandoffRequirement::Delivery(Box::new(request.clone())));
        self
    }
    pub(crate) fn delivery_requirement(
        &self,
    ) -> Option<&crate::protocol::handoff::DeliveryMutation> {
        match self.handoff_requirement.as_ref() {
            Some(HandoffRequirement::Delivery(request)) => Some(request),
            _ => None,
        }
    }
    pub(crate) fn with_bootstrap_child(
        mut self,
        namespace: &crate::protocol::handoff::HandoffNamespace,
        command: &crate::protocol::commands::PermitMutation,
    ) -> Self {
        self.handoff_requirement = Some(HandoffRequirement::BootstrapChild {
            namespace: namespace.clone(),
            command: Box::new(command.clone()),
        });
        self
    }
    pub(crate) fn handoff_requirement(&self) -> Option<&HandoffRequirement> {
        self.handoff_requirement.as_ref()
    }
    pub(crate) fn cooperative_metadata(
        &self,
    ) -> (CallerClaim, CallBudget, Option<HandoffRequirement>) {
        (
            self.claim.clone(),
            self.cooperative_budget.clone(),
            self.handoff_requirement.clone(),
        )
    }
    pub(crate) fn claim(&self) -> &CallerClaim {
        &self.claim
    }
    pub fn consume_cooperative(
        &mut self,
        fence: &CooperativeDecisionFence,
        request: &OperationId,
        obligation: &ObligationRef,
        payload_hash: &[u8; 32],
    ) -> Result<&CallerClaim, &'static str> {
        if self.consumed {
            return Err("permit already consumed");
        }
        if fence.known_invalidated {
            return Err("known mapping invalidation");
        }
        let claim = &self.claim;
        if claim.role != CallerRole::TopLevel
            || claim.instance != fence.instance
            || claim.seat != fence.seat
            || claim.target != fence.target
            || claim.binding_generation != fence.binding_generation
            || self.mapping_revision != fence.mapping_revision
            || self.invalidation_revision != fence.invalidation_revision
        {
            return Err("cooperative context changed");
        }
        if fence.now.0 < self.observed_at.0 || fence.now.0 - self.observed_at.0 > MAX_PERMIT_MILLIS
        {
            return Err("permit expired");
        }
        if request != &self.request
            || obligation != &self.obligation
            || payload_hash != &self.payload_hash
        {
            return Err("permit payload mismatch");
        }
        self.consumed = true;
        Ok(&self.claim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operational_ids_require_registered_agents() {
        let registry = crate::harness::registry::builtins();
        assert_eq!(registry.agent("codex").unwrap().as_str(), "codex");
        for id in ["human", "Human", "unknown", "Codex"] {
            assert!(registry.agent(id).is_err());
        }
    }
    #[test]
    fn occupant_wire_and_context_spellings_are_legacy_exact() {
        use crate::harness::{
            context,
            registry::{OccupantHarness, builtins},
        };
        for (wire, local, occupant) in [
            ("codex", "Codex", OccupantHarness::Codex),
            ("claude", "Claude", OccupantHarness::Claude),
            ("human", "Human", OccupantHarness::Human),
        ] {
            assert_eq!(
                serde_json::to_string(&occupant).unwrap(),
                format!("\"{wire}\"")
            );
            assert_eq!(
                serde_json::to_string(&context::Harness::from(occupant)).unwrap(),
                format!("\"{local}\"")
            );
            assert_eq!(
                serde_json::from_str::<OccupantHarness>(&format!("\"{wire}\"")).unwrap(),
                occupant
            );
            assert_eq!(
                OccupantHarness::from(
                    serde_json::from_str::<context::Harness>(&format!("\"{local}\"")).unwrap()
                ),
                occupant
            );
            if wire != local {
                assert!(serde_json::from_str::<OccupantHarness>(&format!("\"{local}\"")).is_err());
                assert!(serde_json::from_str::<context::Harness>(&format!("\"{wire}\"")).is_err());
            }
        }
        for id in ["human", "Human", "unregistered"] {
            assert!(builtins().agent(id).is_err());
        }
        assert!(serde_json::from_str::<OccupantHarness>("\"unregistered\"").is_err());
        assert!(serde_json::from_str::<context::Harness>("\"Unregistered\"").is_err());
    }
    #[test]
    fn identity_bridge_is_lossless_for_registered_agents() {
        use crate::harness::{
            context,
            registry::{OccupantHarness, builtins},
        };
        for registration in builtins().registrations() {
            let occupant =
                OccupantHarness::Agent(builtins().agent(registration.metadata().id).unwrap());
            assert_eq!(
                OccupantHarness::from(context::Harness::from(occupant)),
                occupant
            );
        }
    }
    #[test]
    fn human_occupant_is_recorded_as_operator_human_never_an_agent_claim() {
        assert_eq!(Harness::Human.as_str(), "human");
        assert_eq!(Harness::Human.cooperative_provenance(), "operator_human");
        for agent in [Harness::Codex, Harness::Claude] {
            assert_eq!(agent.cooperative_provenance(), "cooperative_top_level");
        }
        assert_eq!(serde_json::to_value(Harness::Human).unwrap(), "human");
        assert_eq!(
            serde_json::from_value::<Harness>(serde_json::json!("human")).unwrap(),
            Harness::Human
        );
    }
    #[test]
    fn cooperative_permit_is_payload_bound_expires_and_single_use() {
        let claim = CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("plugin_context:n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: None,
        };
        let obligation = ObligationRef::CheckIn(claim.seat.clone());
        let mut permit = MutationPermit::cooperative(
            claim.clone(),
            OperationId::new("op"),
            obligation.clone(),
            [7; 32],
            MonoInstant(100),
            (0, 3),
            CallBudget {
                deadline: MonoInstant(1000),
                cancellation: Default::default(),
            },
        );
        let mut fence = CooperativeDecisionFence {
            now: MonoInstant(350),
            instance: "i".into(),
            seat: claim.seat.clone(),
            target: claim.target.clone(),
            mapping_revision: 0,
            binding_generation: 0,
            invalidation_revision: 3,
            known_invalidated: false,
        };
        assert!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[8; 32])
                .is_err()
        );
        fence.mapping_revision = 1;
        assert!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[7; 32])
                .is_err()
        );
        fence.mapping_revision = 0;
        fence.now = MonoInstant(351);
        assert!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[7; 32])
                .is_err()
        );
        fence.now = MonoInstant(350);
        fence.known_invalidated = true;
        assert!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[7; 32])
                .is_err()
        );
        fence.known_invalidated = false;
        assert_eq!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[7; 32])
                .unwrap(),
            &claim
        );
        assert!(
            permit
                .consume_cooperative(&fence, &OperationId::new("op"), &obligation, &[7; 32])
                .is_err()
        );
    }

    fn base_claim() -> CallerClaim {
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: None,
        }
    }

    #[test]
    fn claim_without_binding_fields_serializes_exactly_as_before() {
        let claim = base_claim();
        let value = serde_json::to_value(&claim).unwrap();
        assert!(value.get("agent_id").is_none());
        assert!(value.get("liveness").is_none());
        // A pre-V29 (wire 6 shape) claim still decodes.
        let back: CallerClaim = serde_json::from_value(value).unwrap();
        assert_eq!(back, claim);
        let full = CallerClaim {
            agent_id: Some(AgentId::new("agent-7")),
            liveness: Some(LivenessFingerprint {
                pid: 42,
                start: 1_700_000_000_000_000,
            }),
            role: CallerRole::Subagent,
            ..claim.clone()
        };
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["agent_id"], "agent-7");
        assert_eq!(
            json["liveness"],
            serde_json::json!({"pid": 42, "start": 1_700_000_000_000_000u64})
        );
        assert_eq!(serde_json::from_value::<CallerClaim>(json).unwrap(), full);
        let mut bad = serde_json::to_value(&claim).unwrap();
        bad["liveness"] = serde_json::json!({"pid": 1, "start": 2, "extra": 3});
        assert!(serde_json::from_value::<CallerClaim>(bad).is_err());
    }

    #[test]
    fn binding_fields_validation_rejects_pid_zero() {
        let mut claim = base_claim();
        assert!(claim.validate_binding_fields().is_ok());
        claim.liveness = Some(LivenessFingerprint { pid: 0, start: 1 });
        assert!(claim.validate_binding_fields().is_err());
        claim.liveness = Some(LivenessFingerprint { pid: 1, start: 0 });
        assert!(claim.validate_binding_fields().is_ok());
    }

    #[test]
    fn fingerprint_is_alive_only_for_the_same_pid_and_start() {
        struct Table(Vec<crate::ports::ProcessRecord>);
        impl crate::ports::ProcessInfo for Table {
            fn process(&self, pid: u32) -> Option<crate::ports::ProcessRecord> {
                self.0.iter().find(|p| p.pid == pid).cloned()
            }
        }
        let rec = crate::ports::ProcessRecord {
            pid: 10,
            parent: Some(1),
            start: 500,
            argv: vec!["claude".into()],
        };
        let fp = LivenessFingerprint::of(&rec);
        assert!(fp.is_alive(&Table(vec![rec.clone()])));
        assert!(!fp.is_alive(&Table(vec![]))); // exited
        assert!(!fp.is_alive(&Table(vec![crate::ports::ProcessRecord {
            start: 501,
            ..rec
        }]))); // pid reuse
    }

    #[test]
    fn session_binding_provenances_are_defined_in_trust_policy() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/TRUST-POLICY.md"))
            .unwrap();
        for value in [
            COOPERATIVE_ENV_SESSION_PROVENANCE,
            COOPERATIVE_HOOK_AGENT_PROVENANCE,
            LIVENESS_PROCESS_EXIT_PROVENANCE,
            LIVENESS_END_EVENT_PROVENANCE,
        ] {
            assert!(
                text.contains(&format!("| `{value}` |")),
                "{value} missing from the A3 table"
            );
        }
        assert_eq!(
            COOPERATIVE_ENV_SESSION_PROVENANCE,
            "cooperative_env_session"
        );
        assert_eq!(COOPERATIVE_HOOK_AGENT_PROVENANCE, "cooperative_hook_agent");
        assert_eq!(LIVENESS_PROCESS_EXIT_PROVENANCE, "liveness_process_exit");
        assert_eq!(LIVENESS_END_EVENT_PROVENANCE, "liveness_end_event");
    }
}
