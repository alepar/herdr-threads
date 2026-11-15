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
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct WakeControlContext<'a> {
    pub clock: &'a dyn Clock,
    pub budget: &'a CallBudget,
    pub processes: &'a dyn ProcessInfo,
    /// Owned adapter state directory, never the receiver's configuration home.
    pub state_dir: &'a Path,
}
#[derive(Debug)]
pub enum WakeAddOutcome {
    Accepted {
        id: String,
        /// Failure to confirm helper cleanup never erases known acceptance.
        cleanup_error: Option<ApiError>,
    },
    RefusedBeforeEffect(ApiError),
    Unknown(ApiError),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedWakeEntries {
    pub ids: Vec<String>,
    pub complete: bool,
}
pub trait WakeControl: Send + Sync {
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
    ) -> Result<OwnedWakeEntries, ApiError>;
    /// The caller authorizes an exact owned saved ID. True means a pending row was deleted,
    /// not that no turn can run. The old receiver need not still exist.
    fn delete(
        &self,
        target: &WakeControlTarget,
        id: &str,
        context: &WakeControlContext<'_>,
    ) -> Result<bool, ApiError>;
}
