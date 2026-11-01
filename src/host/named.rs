//! The built-in named host: seats that live in a plain or tmux process started
//! with `HERDR_THREADS_SEAT_NAME=<name>`.

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

pub mod tmux;

use crate::harness::adapter::ComposerPolicy;
use crate::harness::composer::{ComposerRead, read_composer_in};
use crate::host::observation::{HostTopology, SeatHostLabels};
use crate::ports::{
    AgentComposerState, ArchivalEvidence, BootstrapObserver, BootstrapPaneObservation,
    ClosureAuthority, ComposerObservation, ComposerTraits, EnumerationEvidence, EvidenceKind,
    ExecutionEvidence, HostCallContext, HostId, HostKind, HostObservation, HostPort, HostSnapshot,
    HostUiState, IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome,
    NativeLaunchRequest, ObservationProvenance, ProcessInfo, PromptOutcome, SafeWakeTarget,
    StructuralOccupancy, WakeCapability, WakeTargetBasis,
};
use crate::protocol::authority::LivenessFingerprint;
use crate::protocol::{
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, SeatName, TerminalId},
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use crate::service::turn_state::{TurnKey, TurnState, TurnStateBook};
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
    tmux: Mutex<HashMap<String, Vec<TmuxBinding>>>,
    /// The daemon's per-binding hook turn state; unset (or a binding it has not heard from) never
    /// reads as idle.
    turns: OnceLock<Arc<TurnStateBook>>,
}

/// One verified tmux binding of a seat.
struct TmuxBinding {
    /// The long-lived process the verification anchored on; the binding lives while it does.
    anchor: LivenessFingerprint,
    hint: tmux::TmuxHint,
    /// The pids from the caller up to the pane's own process: a binding's fingerprint names one.
    walk: Vec<u32>,
}

/// The wake text a typed composer is read back against, and the settle wait before that read.
const SETTLE: Duration = Duration::from_millis(300);
/// `C-u` presses `clear_own_wake_text` makes at most (an Enter taken as a newline leaves two rows).
const CLEAR_PRESSES: usize = 2;

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
            turns: OnceLock::new(),
        }
    }

    /// Install the daemon's turn state (once). Without it every binding reads `Unknown`.
    pub fn set_turn_state(&self, turns: Arc<TurnStateBook>) {
        let _ = self.turns.set(turns);
    }

    /// Verify a caller's tmux hint for `target` and, when it holds, remember the binding. A hint
    /// that does not verify records nothing and never touches the seat: that binding simply has
    /// no wake path. Returns whether the binding is verified.
    pub fn record_tmux_hint(&self, target: &HostTargetId, subject_pid: u32, hint: &str) -> bool {
        let Some((hint, anchor, walk)) = tmux::TmuxHint::parse(hint).and_then(|hint| {
            tmux::verify_walk(&hint, subject_pid, self.processes.as_ref())
                .map(|(anchor, walk)| (hint, anchor, walk))
        }) else {
            return false;
        };
        let Ok(mut map) = self.tmux.lock() else {
            return false;
        };
        let bindings = map.entry(target.as_str().to_owned()).or_default();
        bindings.retain(|held| held.anchor.is_alive(self.processes.as_ref()));
        if let Some(held) = bindings.iter_mut().find(|held| held.anchor == anchor) {
            held.hint = hint;
            held.walk = walk;
            return true;
        }
        if bindings.len() >= MAX_TMUX_BINDINGS {
            return false;
        }
        bindings.push(TmuxBinding { anchor, hint, walk });
        true
    }

    /// The live verified bindings of `target`, pruning the dead ones.
    fn live_bindings<T>(
        &self,
        target: &HostTargetId,
        read: impl FnOnce(&[TmuxBinding]) -> T,
    ) -> Option<T> {
        let mut map = self.tmux.lock().ok()?;
        let bindings = map.get_mut(target.as_str())?;
        bindings.retain(|held| held.anchor.is_alive(self.processes.as_ref()));
        if bindings.is_empty() {
            map.remove(target.as_str());
            return None;
        }
        Some(read(bindings))
    }

    /// True while some verified binding of `target` is alive.
    fn has_live_tmux_binding(&self, target: &HostTargetId) -> bool {
        self.live_bindings(target, |_| ()).is_some()
    }

    /// The wake-armed named bindings: verified tmux hint, live fingerprint. Dead ones are pruned
    /// here, so the daemon's idle exit sees the count fall to zero once the last process is gone.
    fn wake_armed(&self) -> usize {
        let Ok(mut map) = self.tmux.lock() else {
            return 0;
        };
        map.retain(|_, bindings| {
            bindings.retain(|held| held.anchor.is_alive(self.processes.as_ref()));
            !bindings.is_empty()
        });
        map.values().map(Vec::len).sum()
    }

    /// The composer policy of the bound harness; `None` means no tmux wake for it.
    fn wake_policy(
        target: &SafeWakeTarget,
    ) -> Option<(
        crate::harness::registry::AgentHarnessId,
        &'static dyn ComposerPolicy,
    )> {
        let registry = crate::harness::registry::builtins();
        let id = registry.agent(target.bound_harness.as_deref()?).ok()?;
        Some((id, registry.by_id(id).ok()?.composer_policy()?))
    }

    /// Every check before a wake is typed, per pane (rule 4): the context fence, one verified pane
    /// for the seat, a pane that delivers keys, every live top-level binding idle (a stale busy is
    /// cleared by the screen rule), a recognised unblocked screen and an empty composer. Each
    /// refusal is `TargetUnsafe`: the attempt is retried on the ladder, never `no_wake`.
    fn wake_gate(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
        what: &str,
    ) -> Result<(tmux::TmuxHint, u16), ApiError> {
        let refuse = |detail: &str| {
            ApiError::new(
                ErrorCode::TargetUnsafe,
                format!("{what} refused before submission: {detail}"),
            )
        };
        if target.basis != WakeTargetBasis::CooperativeAgent {
            return Err(refuse("native execution is unverified"));
        }
        if context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || self.epoch.load(Ordering::SeqCst) != target.epoch
        {
            return Err(refuse("host context changed"));
        }
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::new(ErrorCode::Cancelled, "wake prompt cancelled"));
        }
        let Some((harness, policy)) = Self::wake_policy(target) else {
            return Err(refuse("no composer policy for the bound harness"));
        };
        let hints: Vec<tmux::TmuxHint> = self
            .live_bindings(&target.target, |bindings| {
                let mut hints: Vec<tmux::TmuxHint> = Vec::new();
                for held in bindings {
                    if !hints.contains(&held.hint) {
                        hints.push(held.hint.clone());
                    }
                }
                hints
            })
            .unwrap_or_default();
        let [hint] = hints.as_slice() else {
            return Err(refuse(if hints.is_empty() {
                "no verified tmux binding"
            } else {
                "bindings in several panes"
            }));
        };
        let Some(facts) = tmux::pane_facts(hint).filter(|f| f.server_pid == hint.server_pid) else {
            return Err(refuse("pane unreadable"));
        };
        if !facts.delivers_keys() {
            return Err(refuse(
                "pane is in a mode, has input off or is synchronized",
            ));
        }
        // The pane is idle only when every live top-level binding of the seat is.
        let generations = self
            .source
            .top_level_generations(&target.seat, &context.budget)?;
        let keys: Vec<TurnKey> = generations
            .into_iter()
            .map(|generation| TurnKey {
                seat: target.seat.clone(),
                generation,
            })
            .collect();
        let Some(turns) = self.turns.get().filter(|_| !keys.is_empty()) else {
            return Err(refuse("no turn state for the pane's bindings"));
        };
        let mut screen = None;
        if keys.iter().any(|key| turns.state(key) == TurnState::Busy) {
            let seen = tmux::capture(hint).ok_or_else(|| refuse("pane unreadable"))?;
            let running = policy.screen_turn_running(&seen);
            let now = self.clock.monotonic_now().0;
            for key in &keys {
                turns.observe_screen(key, running, now);
            }
            screen = Some(seen);
        }
        if !keys.iter().all(|key| turns.state(key) == TurnState::Idle) {
            return Err(refuse("pane busy or a binding's turn state is unknown"));
        }
        let screen = match screen {
            Some(screen) => screen,
            None => tmux::capture(hint).ok_or_else(|| refuse("pane unreadable"))?,
        };
        if policy.screen_blocked(&screen) != Some(false) {
            return Err(refuse("screen blocked or not recognised"));
        }
        match read_composer_in(
            crate::harness::registry::builtins(),
            crate::harness::registry::OccupantHarness::Agent(harness),
            &screen,
            Some(facts.width),
        ) {
            ComposerRead::Empty => Ok((hint.clone(), facts.width)),
            ComposerRead::Unreadable => Err(refuse("composer is unreadable")),
            _ => Err(refuse("composer is nonempty")),
        }
    }

    /// The wake: gate, ask `still_owed`, type the text literally, press Enter.
    fn type_wake(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
        still_owed: Option<&dyn Fn() -> Result<bool, ApiError>>,
    ) -> Result<Option<PromptOutcome>, ApiError> {
        let (hint, _) = self.wake_gate(target, context, "wake prompt")?;
        if let Some(still_owed) = still_owed
            && !still_owed()?
        {
            return Ok(None);
        }
        if context.budget.is_exhausted(self.clock.as_ref()) {
            return Err(ApiError::new(
                ErrorCode::DeadlineExceeded,
                "insufficient wake prompt budget",
            ));
        }
        if !tmux::type_text(&hint, text) {
            // Possibly typed: the composer read-back decides what to clear.
            return Ok(Some(PromptOutcome::OutcomeUnknown));
        }
        Ok(Some(if tmux::press(&hint, "Enter") {
            PromptOutcome::Submitted
        } else {
            PromptOutcome::OutcomeUnknown
        }))
    }

    /// The composer of the seat's pane now, through the bound harness's policy.
    fn read_pane_composer(&self, target: &SafeWakeTarget) -> Option<ComposerRead> {
        let (harness, _) = Self::wake_policy(target)?;
        let hint = self.live_bindings(&target.target, |bindings| {
            bindings.first().map(|held| held.hint.clone())
        })??;
        let screen = tmux::capture(&hint)?;
        let width = tmux::pane_facts(&hint).map_or(80, |facts| facts.width);
        Some(read_composer_in(
            crate::harness::registry::builtins(),
            crate::harness::registry::OccupantHarness::Agent(harness),
            &screen,
            Some(width),
        ))
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

/// The named host's answer to "can a target be created here": no, a named seat has no pane.
/// [`HostPort::native_launch_capability`] returns it and the handoff refusal reads it, so the CLI
/// asks no named-host instance.
pub const NATIVE_LAUNCH: NativeLaunchCapability = NativeLaunchCapability::Unsupported;

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
    /// The host starts a wake lane (pacer, reachability, Health) because a verified tmux binding
    /// can be typed into; a target without one answers `None` per target.
    fn wake_capability(&self) -> WakeCapability {
        WakeCapability::Composer(ComposerTraits::TMUX)
    }
    fn wake_armed_bindings(&self) -> usize {
        self.wake_armed()
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
        NATIVE_LAUNCH
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
                // The synthetic workspace and tab render `-` (SA17 fix (d)).
                workspace_id: "-".to_owned(),
                workspace_label: None,
                tab_id: "-".to_owned(),
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
    /// A cooperative target for a seat with a live verified tmux binding. The composer, turn
    /// state and pane-mode checks run immediately before typing (`wake_gate`).
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if !self.has_live_tmux_binding(&observation.target) {
            return None;
        }
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.type_wake(target, text, context, None)
            .map(|outcome| outcome.unwrap_or(PromptOutcome::OutcomeUnknown))
    }
    fn submit_prompt_if_owed(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
        still_owed: &dyn Fn() -> Result<bool, ApiError>,
    ) -> Result<Option<PromptOutcome>, ApiError> {
        self.type_wake(target, text, context, Some(still_owed))
    }
    /// The pids of the verified bindings' walks, which a binding's fingerprint names.
    fn foreground_pids(
        &self,
        target: &HostTargetId,
        _context: &HostCallContext,
    ) -> Result<Vec<u32>, ApiError> {
        Ok(self
            .live_bindings(target, |bindings| {
                let mut pids: Vec<u32> = bindings
                    .iter()
                    .flat_map(|held| held.walk.iter().copied())
                    .collect();
                pids.sort_unstable();
                pids.dedup();
                pids
            })
            .unwrap_or_default())
    }
    fn launch_native(
        &self,
        _request: NativeLaunchRequest,
        _context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        Err(unreachable_method("pane to launch into"))
    }
    /// The read-back after a wake: the composer empty again means submitted, text in it means the
    /// wake (or a draft) is held.
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        if context.budget.cancellation.wait_blocking(SETTLE) {
            return Ok(AgentComposerState::Unknown);
        }
        Ok(match self.read_pane_composer(target) {
            Some(ComposerRead::Empty) => AgentComposerState::Submitted,
            Some(ComposerRead::Text(_) | ComposerRead::Unsafe { .. }) => {
                AgentComposerState::HoldingPrompt
            }
            _ => AgentComposerState::Unknown,
        })
    }
    fn send_submit_key(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<(), ApiError> {
        Err(unreachable_method("composer to submit into"))
    }
    /// One `C-u` per press, only when the bound harness opts in and a fresh read holds exactly the
    /// wake text: any other content (a draft, the wake plus typed input) is left alone.
    fn clear_own_wake_text(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<bool, ApiError> {
        let Some((_, policy)) = Self::wake_policy(target).filter(|(_, policy)| {
            policy.clears_unsubmitted_wake() && policy.clear_key() == "ctrl+u"
        }) else {
            return Ok(false);
        };
        let _ = policy;
        if target.basis != WakeTargetBasis::CooperativeAgent
            || context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || self.epoch.load(Ordering::SeqCst) != target.epoch
        {
            return Ok(false);
        }
        let Some(hint) = self
            .live_bindings(&target.target, |bindings| {
                bindings.first().map(|held| held.hint.clone())
            })
            .flatten()
        else {
            return Ok(false);
        };
        for press in 0..=CLEAR_PRESSES {
            match self.read_pane_composer(target) {
                Some(ComposerRead::Empty) => return Ok(press > 0),
                Some(ComposerRead::Text(held)) if held == text && press < CLEAR_PRESSES => {
                    if !tmux::press(&hint, "C-u") {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "../../tests/host/named_host.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/host/named_tmux.rs"]
pub(crate) mod tmux_tests;
