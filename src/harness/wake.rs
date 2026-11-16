//! Programmatic idle-wake control; never message delivery or receipt evidence.
use crate::{
    ports::ProcessInfo,
    protocol::{
        authority::LivenessFingerprint,
        ids::{BindingId, ExecutionId, NativeSessionId},
        results::ApiError,
        time::{CallBudget, Clock},
    },
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Canonical connected root scope for observation/add; a saved intent scope for control.
/// File observations support loadedness; they are not a runtime-manager query.
pub struct WakeScope {
    pub binding: BindingId,
    pub native_session: NativeSessionId,
    pub execution: ExecutionId,
    pub generation: u64,
    pub process: LivenessFingerprint,
    pub kernel_boot: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeControlTarget {
    pub scope: WakeScope,
    /// Adapter-owned file identities only; bounded before decoding. No argv/env dump.
    pub descriptor: Vec<u8>,
}
/// The bounded control invocation represented by a durable helper slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeHelperPurpose {
    Add,
    List,
    Delete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeHelperWitness {
    pub process: LivenessFingerprint,
    pub kernel_boot: String,
    pub pgid: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeHelperQuiescence {
    BootChanged,
    LeaderEndedAndGroupAbsent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeHelperPresence {
    Live,
    Quiescent(WakeHelperQuiescence),
    Unknown,
}
/// Core commits the exact witness before the adapter initializes its child.
pub trait WakeHelperObserver: Send + Sync {
    fn started(&self, witness: &WakeHelperWitness) -> Result<(), ApiError>;
    fn completed(&self, cleanup_confirmed: bool) -> Result<(), ApiError>;
}
pub struct WakeControlContext<'a> {
    pub clock: &'a dyn Clock,
    pub budget: &'a CallBudget,
    pub processes: &'a dyn ProcessInfo,
    /// Owned adapter state directory, never the receiver's configuration home.
    pub state_dir: &'a Path,
    pub helper_observer: Option<&'a dyn WakeHelperObserver>,
}
/// Failure evidence is separate from uncertainty about helper cleanup.
#[derive(Debug)]
pub struct ControlFailure {
    pub error: ApiError,
    pub cleanup_error: Option<ApiError>,
}
impl From<ApiError> for ControlFailure {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            cleanup_error: None,
        }
    }
}
impl From<ApiError> for Box<ControlFailure> {
    fn from(error: ApiError) -> Self {
        Box::new(ControlFailure::from(error))
    }
}
#[derive(Debug)]
pub enum WakeAddOutcome {
    Accepted {
        id: String,
        /// Failure to confirm helper cleanup never erases known acceptance.
        cleanup_error: Option<ApiError>,
    },
    RefusedBeforeEffect(ControlFailure),
    Unknown(ControlFailure),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedWakeEntries {
    pub ids: Vec<String>,
    pub complete: bool,
}
/// A conservative replay cohort, never storage identity or seat continuity evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeReplayScope(String);
impl WakeReplayScope {
    pub fn parse(value: String) -> Result<Self, ApiError> {
        if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ApiError::new(
                crate::protocol::results::ErrorCode::InvalidRequest,
                "invalid wake replay scope",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
pub trait WakeControl: Send + Sync {
    /// Reap only exact owned child handles, without waiting or performing queue I/O.
    fn reap_owned_helpers(&self) {}
    /// Includes locked or uncertain retained ownership; never grants wake authority.
    fn has_owned_helpers(&self) -> bool {
        false
    }
    /// Pure cohort derivation before physical capture, granting no wake authority.
    fn replay_scope_for_session(
        &self,
        _session: &NativeSessionId,
    ) -> Result<WakeReplayScope, ApiError> {
        Err(ApiError::new(
            crate::protocol::results::ErrorCode::InvalidRequest,
            "wake replay scope unavailable",
        ))
    }

    /// Observe saved ownership without signaling or inferring absence from unreadability.
    fn probe_helper(
        &self,
        _witness: &WakeHelperWitness,
        _context: &WakeControlContext<'_>,
    ) -> Result<WakeHelperPresence, ApiError> {
        Ok(WakeHelperPresence::Unknown)
    }
    /// Pure stable suppression key; performs no I/O and grants no wake authority.
    fn replay_scope(&self, _target: &WakeControlTarget) -> Result<WakeReplayScope, ApiError> {
        Err(ApiError::new(
            crate::protocol::results::ErrorCode::InvalidRequest,
            "wake replay scope unavailable",
        ))
    }
    /// The caller supplies a canonical connected top-level scope, never a dormant target.
    fn observe(
        &self,
        scope: &WakeScope,
        context: &WakeControlContext<'_>,
    ) -> Result<Option<WakeControlTarget>, ApiError>;
    /// Revalidates after helper readiness, then invokes authorization before add bytes.
    /// The callback must honor the supplied call budget and current canonical/host guards.
    fn add(
        &self,
        target: &WakeControlTarget,
        correlation: &str,
        authorize: &mut dyn FnMut() -> Result<bool, ApiError>,
        context: &WakeControlContext<'_>,
    ) -> WakeAddOutcome;
    /// Reconcile an authorized saved intent, including after receiver exit or unload.
    fn list(
        &self,
        target: &WakeControlTarget,
        correlation: &str,
        context: &WakeControlContext<'_>,
    ) -> Result<OwnedWakeEntries, Box<ControlFailure>>;
    /// The caller authorizes an exact owned saved ID. True means a pending row was deleted,
    /// not that no turn can run. The old receiver need not still exist.
    fn delete(
        &self,
        target: &WakeControlTarget,
        id: &str,
        context: &WakeControlContext<'_>,
    ) -> Result<bool, Box<ControlFailure>>;
}
