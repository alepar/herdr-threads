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

/// Whether Herdr actually reports a registered native host kind. Registry
/// metadata alone is never evidence of native recognition or continuity.
pub fn is_harness_agent_kind(kind: &str) -> bool {
    crate::harness::registry::builtins()
        .by_host_kind(kind)
        .is_some()
}
/// A person acting from their own pane identity (`herdr-threads me init`).
pub const OPERATOR_HUMAN_PROVENANCE: &str = "operator_human";
/// C4: bindings a structural reconfirmation carries to a new host epoch;
/// native `verified_current_target` bindings re-register instead. The planner
/// predicate and the applier both read this one set.
pub const CARRIED_BINDING_PROVENANCES: [&str; 2] =
    [COOPERATIVE_TOP_LEVEL_PROVENANCE, OPERATOR_HUMAN_PROVENANCE];
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
pub const AGENT_BINDING_PROVENANCES: [&str; 2] =
    [COOPERATIVE_TOP_LEVEL_PROVENANCE, MANAGED_LAUNCH_PROVENANCE];

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
}
