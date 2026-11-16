//! Durable current native activity, independent of delivery and wake effects.
use crate::{
    harness::{adapter::TurnEdge, wake::WakeScope},
    protocol::ids::NativeTurnId,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTurnPhase {
    Started,
    /// The current native ID had a matched Start followed by End in this exact scope.
    Ended,
    EndFirst,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTurnState {
    pub scope: WakeScope,
    pub id: NativeTurnId,
    pub phase: NativeTurnPhase,
    pub ordinal: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTurnUpdate {
    /// A validated edge for the process-local book; None for refused or stale observations.
    pub edge: Option<TurnEdge>,
    /// A fresh accepted observation should refresh process-local activity;
    /// this does not imply a durable queue-qualified state change.
    pub changed: bool,
}
