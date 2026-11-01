//! The herdr-core (`hc`) host adapter and its protocol layer.
//!
//! Responsibility (HcHost / HcBoot / HcDiscovery)
//! - Owns: the one-connection-per-call line client ([`rpc::HcClient`]), the
//!   tolerant wire types ([`wire`]), boot = incarnation = `hc-core:<core.id>`
//!   ([`HcBoot`]), mapping herdr-core's `(keep, reason)` finality to closure
//!   (D10, with finished records kept in the capture), `status: null` as
//!   `Unknown`, labels from `herdr.space`/`herdr.tab` plus the `herdr-app`
//!   doc ([`labels`]), `server_process` from the `LocalEndpointWitness` peer
//!   pid, and discovery from `core.json` plus `hello`. Closure, labels and
//!   discovery land in later beads; today [`HcHost`] holds the boot, the
//!   capability answers, the observation counter and `target_root_process`.
//! - Hides: the hc protocol, the socket path rules, and which hc methods it
//!   must never call (`terminal.list`, `doc.set`, `caller`).
//! - Must not: spawn `hc`, treat agent exit as closure, keep `machine/tN`
//!   mirrors, or attach on a diagnostic scan.
//! - Where new behaviour goes: here, in `src/host/hc/`. Callers never call hc
//!   directly or read hc state files.
//! - Lifecycle: built by `HostFactory.build(Hc, locator)` on a dialled
//!   attach, lives as long as its `HostSession`, holds no long-lived
//!   connection. The observation counter is per-instance mutable state.
//! - Errors: a failed `hello` gives `CompleteUnverified` (closes nothing). A
//!   core id mismatch is a new boot (C2 holds). An RPC error before a launch
//!   record exists is `NotSubmitted`; `spawn_failed`, a timeout or a
//!   disconnect is `OutcomeUnknown` (H3).
//! - Role: Adapter (a `HostPort` impl over herdr-core's JSON-RPC).

pub mod labels;
pub mod rpc;
pub mod wire;

pub use rpc::{HcClient, HcError, HcReply, Snapshot};
pub use wire::{
    AppDoc, CoreInfo, HcEvent, HcEventKind, HcForeground, HcStatus, HcTerminal, HelloReply,
};

use crate::ports::{
    AgentComposerState, ArchivalEvidence, ClosureAuthority, ComposerObservation,
    EnumerationEvidence, HostCallContext, HostKind, HostObservation, HostPort, HostSnapshot,
    IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
    PromptOutcome, SafeWakeTarget, WakeCapability,
};
use crate::protocol::{
    ids::{HostBootId, HostTargetId, SeatId},
    results::ApiError,
    time::CallBudget,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Environment herdr-core sets for a terminal: its id and socket (read for
/// caller recognition) and its scope and session. Every child the daemon side
/// spawns has these removed (K5); the H3 gate shim removes all but them.
pub const ENV_VARS: &[&str] = &[
    HC_TERMINAL_ID_ENV,
    HC_TERMINAL_SOCKET_ENV,
    "HC_SCOPE",
    "HC_SESSION",
];
/// The terminal id herdr-core sets for a terminal's processes.
pub(crate) const HC_TERMINAL_ID_ENV: &str = "HC_TERMINAL_ID";
/// The core socket herdr-core sets for a terminal's processes.
pub(crate) const HC_TERMINAL_SOCKET_ENV: &str = "HC_TERMINAL_SOCKET";

const BOOT_PREFIX: &str = "hc-core:";
const MAX_BOOT_BYTES: usize = 128;

/// A herdr-core boot: `hc-core:<core.id>`. The socket is only a dial address;
/// a different core id at a known locator is a new boot (C2 holds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcBoot(HostBootId);

impl HcBoot {
    /// The boot a `hello` reply names; `None` when the core id is empty or the
    /// boot would be non-ASCII or longer than 128 bytes.
    pub fn from_hello(hello: &HelloReply) -> Option<Self> {
        let id = hello.core.id.as_str();
        if id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_graphic())
            || BOOT_PREFIX.len() + id.len() > MAX_BOOT_BYTES
        {
            return None;
        }
        HostBootId::parse(format!("{BOOT_PREFIX}{id}"))
            .ok()
            .map(Self)
    }

    pub fn id(&self) -> &HostBootId {
        &self.0
    }
}

/// The herdr-core `HostPort`. Capability answers: no wake path, closing,
/// no archival evidence.
pub struct HcHost {
    client: HcClient,
    observation_sequence: AtomicU64,
}

impl HcHost {
    pub fn new(client: HcClient) -> Self {
        Self {
            client,
            observation_sequence: AtomicU64::new(0),
        }
    }

    /// The next value of this host's own observation counter (starts at 1).
    pub fn next_observation_sequence(&self) -> u64 {
        self.observation_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Runs one client call to completion from synchronous code, on a private
    /// runtime in its own thread so it is safe inside an async context.
    fn run<T: Send>(
        future: impl std::future::Future<Output = Result<T, HcError>> + Send,
    ) -> Result<T, HcError> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(HcError::Io)?
                        .block_on(future)
                })
                .join()
                .unwrap_or_else(|_| Err(HcError::Decode("hc call panicked".into())))
        })
    }

    fn unavailable(what: &str) -> ApiError {
        ApiError::host_unavailable(format!("hc host: {what} is not served yet"))
    }

    fn unknown_boot() -> HostBootId {
        HostBootId::new("unverified-hc-host")
    }
}

impl HostPort for HcHost {
    fn kind(&self) -> HostKind {
        HostKind::Hc
    }

    fn wake_capability(&self) -> WakeCapability {
        WakeCapability::None
    }

    fn closure_authority(&self) -> ClosureAuthority {
        ClosureAuthority::Closing
    }

    fn archival_evidence(&self) -> ArchivalEvidence {
        ArchivalEvidence::NoEvidence
    }

    fn target_root_process(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc target_root_process"));
        }
        let reply = Self::run(self.client.terminal_get(target.as_str()))
            .map_err(|e| ApiError::host_unavailable(e.to_string()))?;
        Ok(reply.value.pid)
    }

    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }

    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Err(Self::unavailable("observe_current_target"))
    }

    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        Err(Self::unavailable("archival observation"))
    }

    fn live_topology(
        &self,
        _: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        Err(Self::unavailable("live_topology"))
    }

    /// Only the failed-`hello` outcome: nothing is enumerated and nothing is
    /// closed (`CompleteUnverified`, not `CoherentVerified`). The snapshot and
    /// its capture rule are ht-kqrt.67's.
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let boot = Self::run(self.client.hello())
            .ok()
            .and_then(|reply| HcBoot::from_hello(&reply.value))
            .map(|boot| boot.0)
            .unwrap_or_else(Self::unknown_boot);
        Ok(HostSnapshot {
            boot,
            epoch: 0,
            observation_sequence: 0,
            complete: false,
            enumeration: EnumerationEvidence::CompleteUnverified,
            incarnation: IncarnationEvidence::Unknown,
            targets: Vec::new(),
        })
    }

    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }

    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        Err(Self::unavailable("submit_prompt"))
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        Err(Self::unavailable("launch_native"))
    }

    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        Err(Self::unavailable("pane_agent_state"))
    }

    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Err(Self::unavailable("send_submit_key"))
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_host.rs"]
mod tests;
