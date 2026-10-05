//! Internal compound lifecycle contract. These claims protect work; they are
//! never receipts or evidence that a native agent was started.
use super::{
    authority::CallerClaim,
    ids::{OperationId, SeatId, ThreadId},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffIdentity {
    pub compound: OperationId,
    /// Original frozen journal semantic SHA-256, unchanged across upgrades.
    pub digest: String,
    pub claim: CallerClaim,
    pub thread: Option<ThreadId>,
    pub recipient: SeatId,
    pub create_key: OperationId,
    pub invite_key: OperationId,
    pub send_key: OperationId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffMutation {
    pub identity: HandoffIdentity,
    pub operation: OperationId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffState {
    Live,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffResult {
    pub compound: OperationId,
    pub thread: Option<ThreadId>,
    pub state: HandoffState,
}
