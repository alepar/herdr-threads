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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    Codex,
    Claude,
    /// A person typing in their own Herdr pane (`herdr-threads me init`).
    /// Its bindings and decisions carry `operator_human` provenance; it is
    /// never an agent claim and never launched, hooked or prompted.
    Human,
}

impl Harness {
    /// The durable `occupant_bindings.harness` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Human => "human",
        }
    }
    /// Provenance recorded for this occupant's cooperative registration and
    /// accountable decisions. A person is recorded as `operator_human`, never
    /// as an agent's `cooperative_top_level` claim.
    pub fn cooperative_provenance(self) -> &'static str {
        match self {
            Self::Human => OPERATOR_HUMAN_PROVENANCE,
            Self::Codex | Self::Claude => COOPERATIVE_TOP_LEVEL_PROVENANCE,
        }
    }
}

/// An agent's cooperative top-level claim (not native attestation).
pub const COOPERATIVE_TOP_LEVEL_PROVENANCE: &str = "cooperative_top_level";
/// A person acting from their own pane identity (`herdr-threads me init`).
pub const OPERATOR_HUMAN_PROVENANCE: &str = "operator_human";

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

/// Proven after a current-target host observation. This type has no wire decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCaller {
    pub(crate) seat: SeatId,
    pub(crate) harness: Harness,
    pub(crate) native_session: NativeSessionId,
    pub(crate) execution: ExecutionId,
    pub(crate) host_boot: HostBootId,
    pub(crate) target_generation: u64,
    /// Durable seat/binding generation, independent of host target generation.
    pub(crate) binding_generation: u64,
    pub(crate) observed_at_utc: UtcMillis,
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

#[derive(Debug)]
enum PermitActor {
    Native(VerifiedCaller),
    Cooperative {
        claim: CallerClaim,
        mapping_revision: u64,
        invalidation_revision: u64,
    },
}

/// One transaction decision may consume this exact request/payload grant once.
#[derive(Debug)]
pub struct MutationPermit {
    actor: PermitActor,
    request: OperationId,
    obligation: ObligationRef,
    payload_hash: [u8; 32],
    observed_at: MonoInstant,
    host_epoch: u64,
    consumed: bool,
    cooperative_budget: Option<CallBudget>,
}
impl MutationPermit {
    /// Stable durable-seat scope for looking up a previously committed operation.
    /// Reading this identity does not consume the permit or authorize new work.
    pub(crate) fn seat_for_replay_scope(&self) -> &SeatId {
        match &self.actor {
            PermitActor::Native(v) => &v.seat,
            PermitActor::Cooperative { claim, .. } => &claim.seat,
        }
    }
    #[allow(dead_code)] // Caller verifier issues this in the runtime task.
    pub(crate) fn new(
        actor: VerifiedCaller,
        request: OperationId,
        obligation: ObligationRef,
        payload_hash: [u8; 32],
        observed_at: MonoInstant,
        host_epoch: u64,
    ) -> Self {
        Self {
            actor: PermitActor::Native(actor),
            request,
            obligation,
            payload_hash,
            observed_at,
            host_epoch,
            consumed: false,
            cooperative_budget: None,
        }
    }
    /// Issued only after service-local durable context validation. No wire decoder.
    pub(crate) fn cooperative(
        claim: CallerClaim,
        request: OperationId,
        obligation: ObligationRef,
        payload_hash: [u8; 32],
        observed_at: MonoInstant,
        mapping_revision: u64,
        invalidation_revision: u64,
    ) -> Self {
        Self {
            actor: PermitActor::Cooperative {
                claim,
                mapping_revision,
                invalidation_revision,
            },
            request,
            obligation,
            payload_hash,
            observed_at,
            host_epoch: 0,
            consumed: false,
            cooperative_budget: None,
        }
    }
    pub(crate) fn with_cooperative_budget(mut self, budget: CallBudget) -> Self {
        self.cooperative_budget = Some(budget);
        self
    }
    pub(crate) fn cooperative_metadata(&self) -> Option<(CallerClaim, CallBudget)> {
        self.cooperative_claim()
            .cloned()
            .zip(self.cooperative_budget.clone())
    }
    pub(crate) fn cooperative_claim(&self) -> Option<&CallerClaim> {
        match &self.actor {
            PermitActor::Cooperative { claim, .. } => Some(claim),
            _ => None,
        }
    }
    pub fn consume_cooperative(
        &mut self,
        fence: &CooperativeDecisionFence,
        request: &OperationId,
        obligation: &ObligationRef,
        payload_hash: &[u8; 32],
    ) -> Result<&CallerClaim, &'static str> {
        let PermitActor::Cooperative {
            claim,
            mapping_revision,
            invalidation_revision,
        } = &self.actor
        else {
            return Err("native permit cannot authorize cooperative work");
        };
        if self.consumed {
            return Err("permit already consumed");
        }
        if fence.known_invalidated {
            return Err("known mapping invalidation");
        }
        if claim.role != CallerRole::TopLevel
            || claim.instance != fence.instance
            || claim.seat != fence.seat
            || claim.target != fence.target
            || claim.binding_generation != fence.binding_generation
            || *mapping_revision != fence.mapping_revision
            || *invalidation_revision != fence.invalidation_revision
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
        Ok(claim)
    }
    pub fn consume(
        &mut self,
        fence: &DecisionFence,
        request: &OperationId,
        obligation: &ObligationRef,
        payload_hash: &[u8; 32],
    ) -> Result<&VerifiedCaller, &'static str> {
        let PermitActor::Native(actor) = &self.actor else {
            return Err("cooperative permit is not native evidence");
        };
        if self.consumed {
            return Err("permit already consumed");
        }
        if fence.known_invalidated {
            return Err("known host invalidation");
        }
        if fence.host_epoch != self.host_epoch
            || fence.host_boot != actor.host_boot
            || fence.target_generation != actor.target_generation
            || fence.binding_generation != actor.binding_generation
        {
            return Err("host target changed");
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
        Ok(actor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptRegistration {
    pub seat: SeatId,
    pub host_boot: HostBootId,
    pub target_generation: u64,
    pub binding_generation: u64,
    pub native_session: NativeSessionId,
    pub execution: ExecutionId,
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn actor() -> VerifiedCaller {
        VerifiedCaller {
            seat: SeatId::new("s1"),
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n1"),
            execution: ExecutionId::new("e1"),
            host_boot: HostBootId::new("b1"),
            target_generation: 7,
            binding_generation: 2,
            observed_at_utc: UtcMillis(1),
        }
    }
    #[test]
    fn permit_checks_current_target_fence_payload_and_single_use() {
        let obligation = ObligationRef::Invitation(InvitationId::new("i1"));
        let mut permit = MutationPermit::new(
            actor(),
            OperationId::new("o1"),
            obligation.clone(),
            [4; 32],
            MonoInstant(100),
            3,
        );
        assert_eq!(permit.seat_for_replay_scope().as_str(), "s1");
        let mut fence = DecisionFence {
            now: MonoInstant(200),
            host_boot: HostBootId::new("b2"),
            host_epoch: 3,
            target_generation: 7,
            binding_generation: 2,
            known_invalidated: false,
        };
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
        fence.host_boot = HostBootId::new("b1");
        fence.target_generation = 8;
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
        fence.target_generation = 7;
        fence.binding_generation = 8;
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
        fence.binding_generation = 2;
        fence.known_invalidated = true;
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
        fence.known_invalidated = false;
        assert!(
            permit
                .consume(&fence, &OperationId::new("o2"), &obligation, &[4; 32])
                .is_err()
        );
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[5; 32])
                .is_err()
        );
        fence.now = MonoInstant(351);
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
        fence.now = MonoInstant(250);
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_ok()
        );
        assert!(
            permit
                .consume(&fence, &OperationId::new("o1"), &obligation, &[4; 32])
                .is_err()
        );
    }
    #[test]
    fn cooperative_permit_is_payload_bound_expires_and_cannot_be_consumed_as_native() {
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
            0,
            3,
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
