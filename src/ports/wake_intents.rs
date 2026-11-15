//! Internal durable effect ledger. No queue operation is delivery/receipt evidence.
use super::*;
use crate::harness::{
    registry::AgentHarnessId,
    wake::{WakeControlTarget, WakeReplayScope},
};

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
/// Cleanup is independent of submission phase; uncertainty never releases suppression.
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
