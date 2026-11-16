//! Internal durable effect ledger. No queue operation is delivery/receipt evidence.
use super::*;
use crate::harness::{
    registry::AgentHarnessId,
    wake::{WakeControlTarget, WakeHelperPurpose, WakeHelperWitness, WakeReplayScope},
};
use serde::{Deserialize, Serialize};

/// Actual host boundary result, before verification normalization. Generic errors
/// and cancellation are Unknown, never proof that nothing was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeFallbackOutcome {
    NoEffect,
    Submitted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeIntentPreparation {
    pub adapter: AgentHarnessId,
    pub replay_scope: WakeReplayScope,
    pub target: WakeControlTarget,
    pub correlation: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeIntentPhase {
    Prepared,
    Armed,
    Accepted,
    Unknown,
    /// Definitive no-add outcome of this invocation; queue absence is insufficient.
    NoEffect,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeIntentLogical {
    Active,
    Fulfilled {
        turn_id: crate::protocol::ids::NativeTurnId,
        ordinal: u64,
    },
    ScopeEnded,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeNativeBaseline {
    pub id: crate::protocol::ids::NativeTurnId,
    pub ordinal: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeHelperOperation {
    pub id: uuid::Uuid,
    pub purpose: WakeHelperPurpose,
    #[serde(default)]
    pub subject: Option<String>,
    pub owner_boot: uuid::Uuid,
    pub kernel_boot: String,
    pub witness: Option<WakeHelperWitness>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WakeIntentBlockers {
    pub programmatic: bool,
    pub native: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeIntentSnapshot {
    pub ordinal: u64,
    pub attempt: WakeAttemptId,
    pub origin_seat: SeatId,
    pub origin_host: HostId,
    pub origin_host_boot: HostBootId,
    pub origin_host_epoch: u64,
    pub origin_target: HostTargetId,
    pub origin_target_generation: u64,
    pub preparation: WakeIntentPreparation,
    pub frontier: LogicalAttentionFrontier,
    pub phase: WakeIntentPhase,
    pub revision: u64,
    pub owner_boot: uuid::Uuid,
    pub arm_nonce: Option<uuid::Uuid>,
    pub cleanup_quarantined: bool,
    pub diagnostic: String,
    pub owned_ids: Vec<String>,
    pub ids_complete: bool,
    pub logical: WakeIntentLogical,
    pub arm_baseline: Option<WakeNativeBaseline>,
    pub helper: Option<WakeHelperOperation>,
    /// Immutable arm-time evidence; never describes current helper liveness.
    pub add_helper: Option<WakeHelperOperation>,
    pub cleanup_list_attempted: bool,
    pub cleanup_pending_ids: Vec<String>,
    /// Native-only resource exception for an uninitialized revoked helper; never says
    /// the original Accepted/Unknown submission was cancelled or had no effect.
    pub native_no_add_exception: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareWakeIntent {
    Prepared(WakeIntentSnapshot),
    Existing(WakeIntentSnapshot),
    Refused,
}
/// Returned only to the committed Prepared->Armed winner, never to a replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeArmTicket {
    pub attempt: WakeAttemptId,
    pub revision: u64,
    pub owner_boot: uuid::Uuid,
    pub nonce: uuid::Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeIntentOutcome {
    Accepted {
        id: String,
    },
    Unknown,
    /// Definitive no-add outcome of this invocation; queue absence is insufficient.
    NoEffect,
}
#[derive(Debug, Clone, PartialEq, Eq)]
/// Cleanup is independent of submission phase; uncertainty never proves quiescence.
pub struct WakeIntentResult {
    pub outcome: WakeIntentOutcome,
    pub cleanup_confirmed: bool,
    pub diagnostic: String,
}
/// Physical ordinal page, at most 32 summaries; no descriptors or bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeIntentSummary {
    pub ordinal: u64,
    pub attempt: WakeAttemptId,
    pub origin_seat: SeatId,
    pub phase: WakeIntentPhase,
    pub revision: u64,
    pub cleanup_quarantined: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeIntentPage {
    pub items: Vec<WakeIntentSummary>,
    /// Last returned physical ordinal when another unresolved row was observed.
    pub next_after: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeQueueBinding {
    pub adapter: AgentHarnessId,
    pub scope: crate::harness::wake::WakeScope,
    pub replay_scope: WakeReplayScope,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeQueueSelection {
    pub binding: Option<WakeQueueBinding>,
    pub blockers: WakeIntentBlockers,
}
