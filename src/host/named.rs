//! The built-in named host: seats that live in a plain or tmux process started
//! with `HERDR_THREADS_SEAT_NAME=<name>`.

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

pub mod tmux;

use crate::host::observation::{HostTopology, SeatHostLabels};
use crate::ports::{
    AgentComposerState, ArchivalEvidence, BootstrapObserver, BootstrapPaneObservation,
    ClosureAuthority, ComposerObservation, ComposerTraits, EnumerationEvidence, EvidenceKind,
    ExecutionEvidence, HostCallContext, HostId, HostKind, HostObservation, HostPort, HostSnapshot,
    HostUiState, IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome,
    NativeLaunchRequest, ObservationProvenance, ProcessInfo, PromptOutcome, SafeWakeTarget,
    StructuralOccupancy, WakeCapability,
};
use crate::protocol::authority::LivenessFingerprint;
use crate::protocol::{
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, SeatName, TerminalId},
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use crate::store::named_seats::NamedSeatSource;

/// Caller env of a named seat: the seat's name, its read-only marker and the tmux client's pane
/// variables (the wake hint). The daemon and every child it spawns scrub these
/// (`host::child_env_scrub_list`).
pub const ENV_VARS: &[&str] = &[
    ENV_SEAT_NAME,
    ENV_SEAT_READONLY,
    tmux::ENV_TMUX,
    tmux::ENV_TMUX_PANE,
];

/// The variable that names a named seat (precedence rule 1 of the caller site).
pub const ENV_SEAT_NAME: &str = "HERDR_THREADS_SEAT_NAME";
/// The read-only marker of a named seat.
pub const ENV_SEAT_READONLY: &str = "HERDR_THREADS_SEAT_READONLY";

/// NamedHost: the built-in, never-dialled host of named seats.
///
/// Owns: the fixed per-node boot and incarnation (`named-host:node=<uuid>`),
/// `observe_current_target(name)` (always present, generation 1, terminal
/// `named:<name>`), enumeration through `NamedSeatSource::live_named` keyed on
/// the host id, `NonClosing`, `NoEvidence`, `seat_labels` (pane label = name),
/// `ENV_VARS`, and the M11 handoff-into refusal. In N2 it also owns the tmux
/// wake path: per-binding hint verification (server pid plus nearest
/// ancestor), `capture-pane` read, `send-keys` typing and the per-target wake
/// answer.
/// Hides: that "presence" is cooperative and comes from the store; its
/// snapshot rows are the `cooperative_named_seat` observation (TRUST-POLICY A3),
/// complete but never `CoherentVerified`. tmux is a wake path of this host, not
/// a host of its own (SA18).
/// Must not: close by absence (reconcile skips that step for `NonClosing`),
/// arm `baseline_hold_unclaimed` or recovery holds, satisfy `wait:host`, move
/// or rebind a seat when a tmux check fails (that only disables wake for the
/// binding), or report wake `Composer` for a target without a verified hint.
/// Relationships: composes `NamedSeatSource`, a narrow store read the daemon
/// injects; the dependency points host -> store-read trait, never store ->
/// host. The tmux client runs through the existing `ProcessCommand`, and the
/// composer read goes through the harness's existing `ComposerPolicy`.
/// Lifecycle: the daemon creates it at start and installs it in
/// `HostSessions`' built-in slot through `attach_builtin`; it lives for the
/// whole process.
/// Errors: a seat name that fails to parse is never observed. A method that
/// cannot serve a named seat returns `HostUnavailable` (never `Unsupported`);
/// the capability answers refuse it before it is reached.
/// Where new behaviour goes: here. Role: port implementation (`HostPort`).
pub struct NamedHost {
    boot_id: String,
    host_id: HostId,
    source: Arc<dyn NamedSeatSource>,
    clock: Arc<dyn Clock>,
    epoch: AtomicU64,
    sequence: AtomicU64,
    calls: AtomicU64,
    processes: Arc<dyn ProcessInfo>,
    /// Verified tmux bindings by target (seat name), in daemon memory only: lost on restart,
    /// refreshed by every seat-acting call, pruned when the anchor process is gone.
    tmux: Mutex<HashMap<String, Vec<LivenessFingerprint>>>,
}

/// Most verified bindings held per seat (SA16 admits several in one pane).
const MAX_TMUX_BINDINGS: usize = 16;

impl NamedHost {
    /// `host_id` is the host row the daemon attached for the reserved locator.
    pub fn new(
        node_uuid: &str,
        host_id: HostId,
        source: Arc<dyn NamedSeatSource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            boot_id: format!("named-host:node={node_uuid}"),
            host_id,
            source,
            clock,
            epoch: AtomicU64::new(1),
            sequence: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            processes: Arc::new(crate::host::process_table::HostProcessTable),
            tmux: Mutex::new(HashMap::new()),
        }
    }

    /// Verify a caller's tmux hint for `target` and, when it holds, remember the binding. A hint
    /// that does not verify records nothing and never touches the seat: that binding simply has
    /// no wake path. Returns whether the binding is verified.
    pub fn record_tmux_hint(&self, target: &HostTargetId, subject_pid: u32, hint: &str) -> bool {
        let Some(anchor) = tmux::TmuxHint::parse(hint)
            .and_then(|hint| tmux::verify(&hint, subject_pid, self.processes.as_ref()))
        else {
            return false;
        };
        let Ok(mut map) = self.tmux.lock() else {
            return false;
        };
        let bindings = map.entry(target.as_str().to_owned()).or_default();
        bindings.retain(|held| held.is_alive(self.processes.as_ref()));
        if !bindings.contains(&anchor) && bindings.len() < MAX_TMUX_BINDINGS {
            bindings.push(anchor);
        }
        bindings.contains(&anchor)
    }

    /// True while some verified binding of `target` is alive.
    fn has_live_tmux_binding(&self, target: &HostTargetId) -> bool {
        let Ok(mut map) = self.tmux.lock() else {
            return false;
        };
        let Some(bindings) = map.get_mut(target.as_str()) else {
            return false;
        };
        bindings.retain(|held| held.is_alive(self.processes.as_ref()));
        if bindings.is_empty() {
            map.remove(target.as_str());
            return false;
        }
        true
    }

    fn observation(&self, name: &SeatName) -> HostObservation {
        let now = self.clock.monotonic_now();
        let epoch = self.epoch.load(Ordering::SeqCst);
        HostObservation {
            target: HostTargetId::new(name.as_str()),
            host_boot: HostBootId::new(self.boot_id.clone()),
            epoch,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: now,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            focused: false,
            terminal: Some(TerminalId::new(format!("named:{}", name.as_str()))),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: self.boot_id.clone(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!(
                "named-{}",
                self.calls.fetch_add(1, Ordering::Relaxed)
            )),
            connection_epoch: epoch,
            observation_sequence: self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            started_at_mono: now,
            completed_at_mono: now,
        }
    }

    fn live(&self, budget: &CallBudget) -> Result<Vec<(SeatId, SeatName)>, ApiError> {
        self.source.live_named(&self.host_id, budget)
    }
}

/// The named host never runs guarded bootstrap: every observation is refused.
pub struct NoBootstrap;

impl BootstrapObserver for NoBootstrap {
    fn observe_bootstrap_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<BootstrapPaneObservation, ApiError> {
        Err(unreachable_method("pane to bootstrap"))
    }
}

/// M11: the refusal of a handoff or launch into a named seat (53 bytes plus the
/// name). Keyed on the port answer `native_launch_capability() == Unsupported`.
pub fn handoff_into_refusal(name: &SeatName) -> String {
    format!(
        "herdr-threads: named seat {} has no pane to launch into",
        name.as_str()
    )
}

fn unreachable_method(what: &str) -> ApiError {
    ApiError::new(
        ErrorCode::HostUnavailable,
        format!("the named host has no {what}"),
    )
}

impl HostPort for NamedHost {
    fn kind(&self) -> HostKind {
        HostKind::Named
    }
    fn wake_capability(&self) -> WakeCapability {
        WakeCapability::None
    }
    /// `Composer` only for a target with a live binding whose tmux hint verified; any other named
    /// seat has no wake path.
    fn target_wake_capability(&self, target: &HostTargetId) -> WakeCapability {
        if self.has_live_tmux_binding(target) {
            WakeCapability::Composer(ComposerTraits::TMUX)
        } else {
            WakeCapability::None
        }
    }
    fn note_wake_hint(&self, target: &HostTargetId, subject_pid: u32, hint: &str) {
        self.record_tmux_hint(target, subject_pid, hint);
    }
    fn archival_evidence(&self) -> ArchivalEvidence {
        ArchivalEvidence::NoEvidence
    }
    fn closure_authority(&self) -> ClosureAuthority {
        ClosureAuthority::NonClosing
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn resume_after_epoch(&self, persisted: u64) {
        self.epoch
            .fetch_max(persisted.saturating_add(1), Ordering::SeqCst);
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        let name = SeatName::parse(target.as_str())
            .map_err(|_| ApiError::new(ErrorCode::NotFound, "not a seat name"))?;
        Ok(self.observation(&name))
    }
    fn seat_labels(&self, budget: &CallBudget) -> Result<Vec<SeatHostLabels>, ApiError> {
        Ok(self
            .live(budget)?
            .into_iter()
            .map(|(_, name)| SeatHostLabels {
                terminal: format!("named:{}", name.as_str()),
                incarnation: Some(self.boot_id.clone()),
                target: HostTargetId::new(name.as_str()),
                workspace_id: "named".to_owned(),
                workspace_label: None,
                tab_id: "named".to_owned(),
                tab_label: None,
                pane_label: Some(name.as_str().to_owned()),
                session: None,
            })
            .collect())
    }
    fn live_topology(&self, _budget: &CallBudget) -> Result<HostTopology, ApiError> {
        Err(unreachable_method("topology"))
    }
    fn observe_current_target_for_archival(
        &self,
        _target: &HostTargetId,
        _context: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        Err(unreachable_method("composer-aware archival observation"))
    }
    /// A store read of every live named seat: complete, but a cooperative
    /// observation (`cooperative_named_seat`), never `CoherentVerified`, so it
    /// authorizes no absence closure.
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let targets = self
            .live(&context.budget)?
            .iter()
            .map(|(_, name)| self.observation(name))
            .collect();
        Ok(HostSnapshot {
            boot: HostBootId::new(self.boot_id.clone()),
            epoch: self.epoch.load(Ordering::SeqCst),
            observation_sequence: self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            complete: true,
            enumeration: EnumerationEvidence::CompleteUnverified,
            incarnation: IncarnationEvidence::Unknown,
            targets,
        })
    }
    fn safe_wake_target(
        &self,
        _seat: &SeatId,
        _observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _target: &SafeWakeTarget,
        _text: &str,
        _context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        Err(unreachable_method("composer to submit a prompt into"))
    }
    fn launch_native(
        &self,
        _request: NativeLaunchRequest,
        _context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        Err(unreachable_method("pane to launch into"))
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        Err(unreachable_method("composer to read"))
    }
    fn send_submit_key(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<(), ApiError> {
        Err(unreachable_method("composer to submit into"))
    }
}

#[cfg(test)]
#[path = "../../tests/host/named_host.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/host/named_tmux.rs"]
mod tmux_tests;
