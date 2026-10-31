//! Explicit Herdr API endpoint adapter with an owned, joined socket exchange.

use super::continuity::LocalEndpointWitness;
use super::observation::{
    NativePane, NativeSnapshot, normalize_pane, normalize_snapshot, structured_host_error,
};
use crate::harness::adapter::ComposerPolicy;
use crate::harness::composer::{self, ComposerRead};
use crate::harness::registry::{self, Registry};
use crate::ports::{
    self, ComposerStash, CorrelatedStartup, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
    HostCallContext, HostObservation, HostPort, HostSnapshot, HostUiState, IncarnationEvidence,
    NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, ObservationProvenance,
    SafeWakeTarget, StructuralOccupancy, WakeTargetBasis,
};
use crate::protocol::{
    authority::Harness,
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, TerminalId},
    results::{ApiError, CapabilityState, ErrorCode, WakeBlock},
    time::{CallBudget, Clock},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static CALL_ID: AtomicU64 = AtomicU64::new(1);
/// Herdr `agent.start` errors that are checked before anything is typed.
const CONFIRMED_PRESTART_REFUSALS: [&str; 3] = [
    "agent_pane_busy",
    "agent_name_taken",
    "agent_process_hint_unsupported",
];
/// A new pane may not yet have an interactive shell. Retry only Herdr's
/// confirmed pre-submission busy refusal, for at most two seconds and while
/// preserving the caller's minimum startup budget.
const PANE_READY_WINDOW: Duration = Duration::from_secs(2);
const PANE_READY_RETRY: Duration = Duration::from_millis(100);
/// Interval between bounded readiness polls after a submitted start.
const START_POLL_MILLIS: u64 = 250;
/// Polling time before a started-but-undetected agent is checked for an early
/// exit (the pane back at its shell prompt). Live Herdr 0.9.1 keeps such an
/// agent `launch_pending` with no detected `agent` forever.
const EARLY_EXIT_CHECK_MILLIS: u64 = 1_000;
/// Lines of the pane read for the early-exit check and quoted in its refusal.
const EARLY_EXIT_PANE_LINES: usize = 12;
/// Herdr agent statuses that mean the agent awaits input.
const WAKE_READY_STATUSES: [&str; 2] = ["idle", "done"];
/// Ceiling of the fresh agent recheck immediately before a wake prompt.
const PROMPT_RECHECK_MILLIS: u64 = 750;
/// Ceiling of the wake prompt submission itself.
const PROMPT_SUBMIT_MILLIS: u64 = 2_000;
/// Smallest budget a wake prompt submission is started with.
const MIN_PROMPT_MILLIS: u64 = 250;
/// Ceiling of one composer read or composer key/text send.
const COMPOSER_CALL_MILLIS: u64 = 750;
/// Clears beyond one per composer line before a stash gives up.
const COMPOSER_CLEAR_SLACK: usize = 2;
/// `clear_key` presses that may remove an unsubmitted wake (its row, and an empty row an
/// Enter taken as a newline left below it).
const WAKE_CLEAR_PRESSES: usize = 2;
/// Settle time before the composer is read back after a send, so the harness
/// has cleared its composer when the prompt was accepted.
const COMPOSER_SETTLE_MILLIS: u64 = 250;
/// A sent wake prompt still sitting in the composer is among the last lines of
/// the pane; a submitted one is pushed above the empty composer box.
const COMPOSER_TAIL_LINES: usize = 4;

const FOCUSED_EMPTY_MILLIS: u64 = 60_000;
const MAX_EMPTY_WINDOWS: usize = 1_024;
/// How long a wake attempt's recorded block is reported (`HostPort::wake_block`): past it the
/// block is stale (the screen may have changed with no further attempt) and reads as none.
const WAKE_BLOCK_MILLIS: u64 = 10 * 60_000;
/// Bound on recorded blocks; expired ones are pruned first, then a new seat is not recorded.
const MAX_WAKE_BLOCKS: usize = 1_024;

struct EmptyComposerWindow {
    target: SafeWakeTarget,
    since: u64,
    last: u64,
}

pub struct NativeCli {
    registry: &'static Registry,
    socket: PathBuf,
    clock: Arc<dyn Clock>,
    epoch: AtomicU64,
    /// Observation order within one (boot, epoch). Never reused by this adapter.
    sequence: AtomicU64,
    empty_windows: Mutex<HashMap<SeatId, EmptyComposerWindow>>,
    /// The block each seat's last wake attempt met, with when (monotonic ms) it was recorded
    /// (`HostPort::wake_block`).
    wake_blocks: Mutex<HashMap<SeatId, (WakeBlock, u64)>>,
    /// What the last answered `ping` reported, for Health: the admitted
    /// release, or None before any answer or after one the floor refused.
    release: Mutex<Option<super::compatibility::HostRelease>>,
}

/// Server-process incarnation bound to the actual response connections of one
/// call: kernel peer PID/UID of the ping and operation streams plus the
/// kernel process start time, rechecked after response EOF
/// (`transport::request_witnessed`). Herdr 0.9.1 exposes no boot token in its
/// API responses, so this local peer/process identity is the design's
/// "supported local peer/process identity" source. A restarted or handed-off
/// server is a different process and therefore a different incarnation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerIncarnation {
    boot: HostBootId,
    identity: String,
}
impl ServerIncarnation {
    fn from_witness(witness: &LocalEndpointWitness) -> Result<Self, ApiError> {
        let identity = format!(
            "herdr-server:pid={}:start={}.{:06}:uid={}",
            witness.peer_pid, witness.start_seconds, witness.start_microseconds, witness.peer_uid
        );
        let boot = HostBootId::parse(identity.clone()).map_err(|_| {
            error(
                ErrorCode::StaleHostObservation,
                "invalid server incarnation",
            )
        })?;
        Ok(Self { boot, identity })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptOutcome {
    /// Herdr reported that the prompt was submitted. This is not a receipt.
    Submitted,
    /// Submission may have happened before transport failure or timeout.
    Unknown,
    Rejected(ApiError),
}

impl ports::BootstrapObserver for NativeCli {
    fn observe_bootstrap_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ports::BootstrapPaneObservation, ApiError> {
        self.check_context(context)?;
        let epoch = self.epoch();
        let started = self.clock.monotonic_now();
        let (pane, witness) = self.pane_witnessed(target.as_str(), &context.budget)?;
        self.check_epoch(epoch)?;
        let witness = witness.ok_or_else(|| {
            error(
                ErrorCode::StaleHostObservation,
                "bootstrap pane lacks a qualified transport witness",
            )
        })?;
        let incarnation = ServerIncarnation::from_witness(&witness)?;
        self.check_boot(context, Some(&incarnation))?;
        let workspace = HostTargetId::parse(pane.workspace_id.clone()).map_err(|_| {
            error(
                ErrorCode::StaleHostObservation,
                "invalid same-response workspace",
            )
        })?;
        let tab = HostTargetId::parse(pane.tab_id.clone())
            .map_err(|_| error(ErrorCode::StaleHostObservation, "invalid same-response tab"))?;
        let ui = status_ui(&pane, self.registry);
        let mut observation = self.observation(
            pane,
            epoch,
            started,
            Some(&incarnation),
            ObservationProvenance::FreshCurrentTarget,
            EvidenceKind::NativeCurrentTarget,
            self.next_sequence(),
        );
        observation.ui = ui;
        ports::BootstrapPaneObservation::try_new(observation, workspace, tab, witness)
            .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))
    }
}

impl ports::CreateTabPort for NativeCli {
    fn create_tab(
        &self,
        request: &ports::CreateTabRequest,
        context: &HostCallContext,
    ) -> ports::CreateTabOutcome {
        use ports::CreateTabOutcome::{Created, NotSubmitted, OutcomeUnknown};
        if !self.socket.is_absolute()
            || self.socket != request.expected_witness.endpoint
            || !crate::protocol::handoff::frozen_absolute_path(&request.cwd)
            || request.cwd.to_str().is_none()
            || request.label.trim().is_empty()
            || request.focus
            || match &request.directory_context {
                Some(selection) => selection
                    .validate_bootstrap(&request.cwd, &request.env)
                    .is_err(),
                None => !request.env.is_empty(),
            }
        {
            return NotSubmitted(error(
                ErrorCode::InvalidRequest,
                "invalid frozen tab creation request",
            ));
        }
        let incarnation = match ServerIncarnation::from_witness(&request.expected_witness) {
            Ok(value) => value,
            Err(error) => return NotSubmitted(error),
        };
        if context
            .expected_boot
            .as_ref()
            .is_some_and(|boot| boot != &incarnation.boot)
            || context
                .expected_epoch
                .is_some_and(|epoch| epoch != self.epoch())
        {
            return NotSubmitted(error(
                ErrorCode::StaleHostObservation,
                "tab creation host fence changed",
            ));
        }
        let response = match super::transport::create_tab(
            &self.socket,
            request,
            self.clock.as_ref(),
            &context.budget,
            Duration::from_secs(30),
            &|release| self.record_release(release),
        ) {
            Ok(response) => response,
            Err(outcome) => return outcome,
        };
        let decode = || -> Option<ports::CreatedTab> {
            let value: serde_json::Value = serde_json::from_str(&response.body).ok()?;
            let result = value.get("result")?;
            if value.get("id")?.as_str()? != request.correlation.as_str()
                || result.get("type")?.as_str()? != "tab_created"
            {
                return None;
            }
            let tab = result.get("tab")?;
            let pane = result.get("root_pane")?;
            if tab.get("workspace_id")?.as_str()? != request.workspace.as_str()
                || pane.get("workspace_id")?.as_str()? != request.workspace.as_str()
                || tab.get("tab_id")? != pane.get("tab_id")?
            {
                return None;
            }
            let created = ports::CreatedTab {
                correlation: request.correlation.clone(),
                workspace: request.workspace.clone(),
                tab: HostTargetId::parse(tab.get("tab_id")?.as_str()?).ok()?,
                root_pane: HostTargetId::parse(pane.get("pane_id")?.as_str()?).ok()?,
                terminal: TerminalId::parse(pane.get("terminal_id")?.as_str()?).ok()?,
                host_incarnation: ServerIncarnation::from_witness(&response.witness)
                    .ok()?
                    .boot,
                witness: response.witness.clone(),
            };
            created.validate().ok()?;
            Some(created)
        };
        match decode() {
            Some(created) => Created(Box::new(created)),
            None => OutcomeUnknown(error(
                ErrorCode::StaleHostObservation,
                "missing or incoherent correlated tab creation evidence",
            )),
        }
    }
}

impl NativeCli {
    /// Herdr's guarded start (`agent.start`): Herdr itself checks that the
    /// pane is at its interactive shell prompt before it starts the agent, and
    /// answers as soon as the command is submitted (`launch_pending`); this
    /// adapter then observes readiness with bounded `agent.get` polls.
    /// `preflight` is this adapter's fresh witnessed read.
    #[cfg(test)]
    fn guarded_start_cli(
        &self,
        request: &NativeLaunchRequest,
        context: &HostCallContext,
        preflight: &HostObservation,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.guarded_start_with_evidence(request, context, preflight)
            .map_err(|failure| failure.error)
    }
    fn guarded_start_with_evidence(
        &self,
        request: &NativeLaunchRequest,
        context: &HostCallContext,
        preflight: &HostObservation,
    ) -> Result<NativeLaunchOutcome, ports::NativeLaunchFailure> {
        let before_start =
            |code, detail| ports::NativeLaunchFailure::not_submitted(error(code, detail));
        request
            .validate()
            .map_err(|detail| before_start(ErrorCode::InvalidRequest, detail))?;
        let current_epoch = self.epoch();
        if !preflight.is_fresh_structure()
            || preflight.target != request.target
            || preflight.terminal.as_ref() != Some(&request.expected_terminal)
            || preflight.generation != request.expected_generation
            || preflight.epoch != current_epoch
            || preflight.connection_epoch != current_epoch
            || context.expected_boot.as_ref() != Some(&preflight.host_boot)
            || context.expected_epoch != Some(current_epoch)
            || !matches!(&preflight.incarnation, IncarnationEvidence::Verified { identity, .. }
                if identity == &request.expected_incarnation)
        {
            return Err(before_start(
                ErrorCode::StaleHostObservation,
                "native start preflight fence changed or unverified",
            ));
        }
        if preflight.occupancy == StructuralOccupancy::Occupied
            || matches!(
                preflight.ui,
                HostUiState::ApprovalOrQuestion | HostUiState::HumanInput | HostUiState::ActiveTurn
            )
        {
            return Err(before_start(
                ErrorCode::TargetUnsafe,
                "target is known busy",
            ));
        }
        if context.budget.cancellation.is_cancelled() {
            return Err(before_start(
                ErrorCode::Cancelled,
                "native start cancelled before submission",
            ));
        }
        let Harness::Agent(id) = request.harness else {
            return Err(before_start(
                ErrorCode::InvalidRequest,
                "a human occupant is never launched",
            ));
        };
        let registration = self.registry.by_id(id).map_err(|_| {
            before_start(
                ErrorCode::UnsupportedHarness,
                "native start adapter is unregistered",
            )
        })?;
        let policy = registration.launch_policy().ok_or_else(|| {
            before_start(ErrorCode::UnsupportedHarness, "native start is unsupported")
        })?;
        if request.process_hint != policy.requires_process_hint() {
            return Err(before_start(
                ErrorCode::InvalidRequest,
                "native request mode differs from registered launch policy",
            ));
        }
        let kinds = policy.expected_host_kinds();
        let kind = kinds
            .iter()
            .copied()
            .find(|kind| registration.metadata().host_kinds.contains(kind))
            .ok_or_else(|| {
                before_start(
                    ErrorCode::UnsupportedHarness,
                    "native start has no registered host kind",
                )
            })?;
        // The readable name first; when Herdr refuses it as taken by another
        // live agent (checked before anything is typed), one retry with a
        // short seat suffix. Only confirmed pre-start refusals may retry.
        let [first_name, retry_name] = request.agent_name_candidates();
        let mut name = first_name;
        let readiness_started = Instant::now();
        let (name, parsed, remaining, started, submitted_at_mono) = loop {
            self.check_epoch(current_epoch)
                .map_err(ports::NativeLaunchFailure::not_submitted)?;
            if context.budget.cancellation.is_cancelled() {
                return Err(before_start(
                    ErrorCode::Cancelled,
                    "native start cancelled before submission",
                ));
            }
            let remaining = context
                .budget
                .deadline
                .0
                .saturating_sub(self.clock.monotonic_now().0)
                .min(30_000);
            if remaining <= 3_000 {
                // Rejected before submission: nothing was started.
                return Err(before_start(
                    ErrorCode::InvalidBudget,
                    "insufficient native startup budget",
                ));
            }
            let timeout = remaining.to_string();
            let mut args = vec![
                "agent",
                "start",
                name.as_str(),
                "--kind",
                kind,
                "--pane",
                request.target.as_str(),
                "--timeout",
                timeout.as_str(),
                "--",
            ];
            args.extend(request.argv.iter().map(String::as_str));
            let submitted_at_mono = self.clock.monotonic_now();
            let started = Instant::now();
            let response = if request.process_hint {
                if !self.socket.is_absolute() {
                    return Err(before_start(
                        ErrorCode::InvalidRequest,
                        "host API endpoint must be absolute",
                    ));
                }
                let validate = |_: &serde_json::Value, witness: &LocalEndpointWitness| {
                    self.check_epoch(current_epoch)?;
                    let incarnation = ServerIncarnation::from_witness(witness)?;
                    if incarnation.identity != request.expected_incarnation
                        || incarnation.boot != preflight.host_boot
                        || context.expected_boot.as_ref() != Some(&incarnation.boot)
                        || context.expected_epoch != Some(self.epoch())
                    {
                        return Err(error(
                            ErrorCode::StaleHostObservation,
                            "native start peer incarnation or epoch changed",
                        ));
                    }
                    Ok(())
                };
                let id = format!("threads-{}", CALL_ID.fetch_add(1, Ordering::Relaxed));
                let response = super::transport::request_guarded_start(
                    &self.socket,
                    &id,
                    serde_json::json!({"name":name,"kind":kind,"pane_id":request.target.as_str(),"args":request.argv,"timeout_ms":remaining,"process_hint":true}),
                    self.clock.as_ref(),
                    &context.budget,
                    Duration::from_millis(remaining),
                    &validate,
                    &|release| self.record_release(release),
                );
                if response.as_ref().is_err_and(|failure| {
                    matches!(
                        failure.error.code,
                        ErrorCode::Cancelled
                            | ErrorCode::DeadlineExceeded
                            | ErrorCode::HostUnavailable
                            | ErrorCode::StaleHostObservation
                    )
                }) {
                    self.epoch.fetch_add(1, Ordering::AcqRel);
                }
                match response {
                    Ok(response) => Ok(response.body),
                    Err(failure) if failure.submission == ports::NativeSubmission::NotSubmitted => {
                        return Err(failure);
                    }
                    Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                }
            } else {
                self.dispatch(
                    &args,
                    &context.budget,
                    Duration::from_millis(remaining),
                    false,
                    true,
                    true,
                )
                .map(|(body, _)| body)
            };
            let (refusal, name_taken, pane_busy) = match response {
                Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                Ok(raw) => {
                    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
                        Ok(value) => value,
                        Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                    };
                    let Some(host_error) = structured_host_error(&parsed) else {
                        break (name, parsed, remaining, started, submitted_at_mono);
                    };
                    let code = parsed
                        .pointer("/error/code")
                        .and_then(serde_json::Value::as_str);
                    if !code.is_some_and(|code| CONFIRMED_PRESTART_REFUSALS.contains(&code)) {
                        return Ok(NativeLaunchOutcome::OutcomeUnknown);
                    }
                    (
                        host_error,
                        code == Some("agent_name_taken"),
                        code == Some("agent_pane_busy"),
                    )
                }
            };
            if pane_busy && readiness_started.elapsed() < PANE_READY_WINDOW {
                let remaining = context
                    .budget
                    .deadline
                    .0
                    .saturating_sub(self.clock.monotonic_now().0);
                if remaining > 3_000 + PANE_READY_RETRY.as_millis() as u64
                    && !context.budget.cancellation.wait_blocking(PANE_READY_RETRY)
                    && readiness_started.elapsed() < PANE_READY_WINDOW
                    && !context.budget.is_exhausted(self.clock.as_ref())
                    && context
                        .budget
                        .deadline
                        .0
                        .saturating_sub(self.clock.monotonic_now().0)
                        > 3_000
                {
                    continue;
                }
            }
            if name_taken && name != retry_name && !context.budget.cancellation.is_cancelled() {
                name = retry_name.clone();
                continue;
            }
            return Err(ports::NativeLaunchFailure::not_submitted(refusal));
        };
        let result = match parsed.get("result") {
            Some(result) => result,
            None => return Ok(NativeLaunchOutcome::OutcomeUnknown),
        };
        let agent = match result.get("agent") {
            Some(agent) => agent,
            None => return Ok(NativeLaunchOutcome::OutcomeUnknown),
        };
        let returned_args = result.get("argv").and_then(serde_json::Value::as_array);
        let args_match = returned_args.is_some_and(|values| {
            if request.process_hint {
                let crate::harness::adapter::ExecutableLookup::Path(executable) =
                    registration.metadata().executable
                else {
                    return false;
                };
                return values.len() == request.argv.len() + 1
                    && values.first().and_then(serde_json::Value::as_str) == Some(executable)
                    && values[1..]
                        .iter()
                        .zip(&request.argv)
                        .all(|(actual, expected)| actual.as_str() == Some(expected));
            }
            values.len() >= request.argv.len()
                && values[values.len() - request.argv.len()..]
                    .iter()
                    .zip(&request.argv)
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        });
        // The start response identifies the exact agent Herdr created: our
        // adapter-selected name, in the expected pane and terminal, running
        // exactly the requested arguments after the canonical executable.
        if result.get("type").and_then(serde_json::Value::as_str) != Some("agent_started")
            || !args_match
            || !self.same_agent(agent, &name, request)
        {
            return Ok(NativeLaunchOutcome::OutcomeUnknown);
        }
        // Herdr 0.9.1's socket `agent.start` answers as soon as the command
        // is submitted (`launch_pending: true`, captured live 2026-09-30);
        // readiness is then observed on the same named agent with bounded
        // `agent.get` polls, as `herdr agent start --timeout` does.
        let mut current = agent.clone();
        loop {
            if self.epoch() != current_epoch
                || started.elapsed() >= Duration::from_millis(remaining)
                || context.budget.is_exhausted(self.clock.as_ref())
                || !self.same_agent(&current, &name, request)
            {
                return Ok(NativeLaunchOutcome::OutcomeUnknown);
            }
            let pending = current
                .get("launch_pending")
                .is_some_and(|pending| pending.as_bool() != Some(false));
            // An agent launched with an initial prompt starts working at once
            // and may not report `interactive_ready` until that first turn
            // ends (live tea-party demo: launch timed out as outcome_unknown
            // while the agent was already working). A detected agent that is
            // working, or done with that turn, has started just the same (idle
            // without interactive_ready still means not ready yet).
            let status = current
                .get("agent_status")
                .and_then(serde_json::Value::as_str);
            if request.process_hint && status == Some("blocked") {
                return Ok(NativeLaunchOutcome::OutcomeUnknown);
            }
            let ready = (current
                .get("interactive_ready")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                || matches!(status, Some("working" | "done")))
                // Herdr reports the detected agent label as `agent`.
                && current.get("agent").and_then(serde_json::Value::as_str) .is_some_and(|kind| kinds.contains(&kind) && registration.metadata().host_kinds.contains(&kind));
            // Herdr keeps `launch_pending` set until the agent is first
            // interactive_ready, so an agent already working on its initial
            // prompt (live demo: working, launch_pending true) is started too.
            let working = matches!(status, Some("working" | "done"))
                && current
                    .get("agent")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| {
                        kinds.contains(&kind) && registration.metadata().host_kinds.contains(&kind)
                    });
            if (ready && !pending) || working {
                break;
            }
            // Blocked during startup (a trust or approval dialog): started,
            // but not ready. Never answered here and never a safe retry.
            if current
                .get("agent_status")
                .and_then(serde_json::Value::as_str)
                == Some("blocked")
            {
                return Ok(NativeLaunchOutcome::OutcomeUnknown);
            }
            // Herdr never updates the agent record when the harness exits
            // straight away (a usage error such as rc 2): it stays
            // `launch_pending` with no detected `agent` (live 2026-10-01,
            // stand-in `codex` exiting 2) while the pane is back at its shell
            // prompt. Read the pane and report that definite failure instead
            // of waiting out the window as outcome_unknown.
            if current.get("agent").is_none()
                && started.elapsed() >= Duration::from_millis(EARLY_EXIT_CHECK_MILLIS)
                && let Some(tail) = self.early_exit_output(request, kind, &context.budget)
            {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    format!(
                        "{kind} exited right after the start (the pane is back at its shell \
                         prompt); nothing is running. Last pane lines:\n{tail}"
                    ),
                )
                .into());
            }
            std::thread::sleep(Duration::from_millis(START_POLL_MILLIS));
            let left = Duration::from_millis(remaining).saturating_sub(started.elapsed());
            let limit = left.min(Duration::from_millis(750));
            // Stays fenced: a failure here is the launch's `OutcomeUnknown` outcome.
            let response = if request.process_hint {
                self.run_witnessed(&["agent", "get", name.as_str()], &context.budget, limit)
                    .and_then(|(body, witness)| {
                        let incarnation = witness
                            .as_ref()
                            .map(ServerIncarnation::from_witness)
                            .transpose()?;
                        self.check_boot(context, incarnation.as_ref())?;
                        if incarnation
                            .as_ref()
                            .map(|incarnation| &incarnation.identity)
                            != Some(&request.expected_incarnation)
                        {
                            return Err(error(
                                ErrorCode::StaleHostObservation,
                                "native startup polling peer changed",
                            ));
                        }
                        Ok(body)
                    })
            } else {
                self.run(&["agent", "get", name.as_str()], &context.budget, limit)
            };
            let polled = match response {
                Ok(raw) => raw,
                Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
            };
            let polled: serde_json::Value = match serde_json::from_str(&polled) {
                Ok(value) => value,
                Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
            };
            match polled.get("result") {
                Some(result)
                    if result.get("type").and_then(serde_json::Value::as_str)
                        == Some("agent_info") =>
                {
                    current = match result.get("agent") {
                        Some(agent) => agent.clone(),
                        None => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                    };
                }
                _ => return Ok(NativeLaunchOutcome::OutcomeUnknown),
            }
        }
        let mut observed = preflight.clone();
        observed.provenance = ObservationProvenance::UncharacterizedCache;
        observed.occupancy = StructuralOccupancy::Occupied;
        observed.ui = HostUiState::Unknown;
        observed.incarnation = IncarnationEvidence::Unknown;
        observed.execution = ExecutionEvidence::Unknown;
        observed.occupant = None;
        observed.generation = 0;
        observed.observed_at_mono = self.clock.monotonic_now();
        observed.completed_at_mono = observed.observed_at_mono;
        let started_process = self.started_process(&request.target, &context.budget);
        let correlation = CorrelatedStartup {
            process_hint: request.process_hint,
            seat: request.seat.clone(),
            agent_name: name,
            harness: request.harness,
            target: request.target.clone(),
            terminal: request.expected_terminal.clone(),
            expected_generation: request.expected_generation,
            expected_incarnation: request.expected_incarnation.clone(),
            argv: request.argv.clone(),
            host_boot: preflight.host_boot.clone(),
            epoch: current_epoch,
            submitted_at_mono,
            completed_at_mono: observed.completed_at_mono,
            started_process,
        };
        Ok(NativeLaunchOutcome::ObservedStartup {
            correlation,
            diagnostic: observed,
        })
    }

    /// The process Herdr's guarded start left running in `target` (the managed-launch
    /// placeholder's liveness fingerprint): the pane's single foreground process other than its
    /// shell, from Herdr's `pane.process_info`, with its start stamp read from host process info.
    /// Advisory and unfenced. Never a tty or argv scan: none, several or an unreadable process
    /// yield `None`, and the launch then records no placeholder.
    fn started_process(
        &self,
        target: &HostTargetId,
        launch_budget: &CallBudget,
    ) -> Option<crate::protocol::authority::LivenessFingerprint> {
        use crate::ports::ProcessInfo as _;
        // The readiness wait may have used most of the launch budget; this one short read gets
        // its own (still cancelled by the launch's cancellation).
        let budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(
                self.clock.monotonic_now().0.saturating_add(2_000),
            ),
            cancellation: launch_budget.cancellation.clone(),
        };
        let raw = self
            .run_unfenced(
                &["pane", "process-info", target.as_str()],
                &budget,
                Duration::from_millis(750),
            )
            .ok()?;
        let pid = started_process_pid(&serde_json::from_str(&raw).ok()?)?;
        let record = crate::host::process_table::HostProcessTable.process(pid)?;
        Some(crate::protocol::authority::LivenessFingerprint::of(&record))
    }

    /// The pane's last lines when the started harness has already exited: its
    /// command line was echoed and the pane's last line is a shell prompt
    /// again. `None` while the harness may still be running or when the pane
    /// cannot be read. Advisory read: unfenced, never moves the epoch.
    fn early_exit_output(
        &self,
        request: &NativeLaunchRequest,
        kind: &str,
        budget: &CallBudget,
    ) -> Option<String> {
        let raw = self
            .run_unfenced(
                &["pane", "read", request.target.as_str()],
                budget,
                Duration::from_millis(750),
            )
            .ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let text = parsed.pointer("/result/read/text")?.as_str()?;
        shell_prompt_returned(text, kind)
    }

    /// The agent record names exactly the requested start: adapter-selected
    /// name, explicit pane and the expected terminal.
    fn same_agent(
        &self,
        agent: &serde_json::Value,
        name: &str,
        request: &NativeLaunchRequest,
    ) -> bool {
        agent.get("name").and_then(serde_json::Value::as_str) == Some(name)
            && agent.get("pane_id").and_then(serde_json::Value::as_str)
                == Some(request.target.as_str())
            && agent.get("terminal_id").and_then(serde_json::Value::as_str)
                == Some(request.expected_terminal.as_str())
    }

    /// `socket` is the explicit automation API endpoint, resolved by the caller.
    pub fn new(socket: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self::with_registry(socket, clock, registry::builtins())
    }

    pub fn with_registry(
        socket: PathBuf,
        clock: Arc<dyn Clock>,
        registry: &'static Registry,
    ) -> Self {
        Self {
            registry,
            socket,
            clock,
            epoch: AtomicU64::new(1),
            sequence: AtomicU64::new(1),
            empty_windows: Mutex::new(HashMap::new()),
            wake_blocks: Mutex::new(HashMap::new()),
            release: Mutex::new(None),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Daemon restart creates a new connection epoch above every epoch a
    /// previous daemon boot persisted for this host instance, so ordering
    /// against durable publications stays monotonic within one server
    /// incarnation. Never lowers the current epoch.
    pub fn resume_after_epoch(&self, persisted: u64) {
        self.epoch
            .fetch_max(persisted.saturating_add(1), Ordering::AcqRel);
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::AcqRel)
    }

    pub fn pane(&self, target: &str, budget: &CallBudget) -> Result<NativePane, ApiError> {
        self.pane_witnessed(target, budget).map(|(pane, _)| pane)
    }

    fn pane_witnessed(
        &self,
        target: &str,
        budget: &CallBudget,
    ) -> Result<(NativePane, Option<LocalEndpointWitness>), ApiError> {
        let started = Instant::now();
        let (raw, witness) =
            self.run_witnessed(&["pane", "get", target], budget, Duration::from_millis(750))?;
        let pane = normalize_pane(&raw, target)?;
        self.check_after_parse(budget, started, Duration::from_millis(750))?;
        Ok((pane, witness))
    }

    /// Pane IDs with their pane/tab labels, for resolving a name a person
    /// typed in `--pane`. Display metadata only; never identity evidence.
    /// Advisory read: unfenced, never moves the epoch.
    pub fn pane_names(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::PaneName>, ApiError> {
        let raw = self.run_unfenced(&["api", "snapshot"], budget, Duration::from_secs(2))?;
        crate::host::observation::normalize_pane_names(&raw)
    }

    /// One coherent topology for command-local target selection.
    pub fn topology(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        let raw = self.run_unfenced(&["api", "snapshot"], budget, Duration::from_secs(2))?;
        crate::host::observation::normalize_topology(&raw)
    }

    /// Resolve the invoking pane at this endpoint, never the focused pane.
    pub fn current_pane(
        &self,
        caller: &str,
        budget: &CallBudget,
    ) -> Result<HostTargetId, ApiError> {
        let raw = self.run_unfenced(
            &["pane", "current", "--current", caller],
            budget,
            Duration::from_millis(750),
        )?;
        crate::host::observation::normalize_current_pane(&raw)
    }

    /// Advisory labels for seat-list presentation, from one bounded snapshot.
    pub fn seat_labels(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::SeatHostLabels>, ApiError> {
        let raw = self.run_unfenced(&["api", "snapshot"], budget, Duration::from_secs(2))?;
        crate::host::observation::normalize_seat_labels(&raw)
    }

    /// One advisory topology snapshot, carrying its response's server incarnation.
    /// Failure never invalidates daemon continuity or client connection epochs.
    pub fn participant_labels(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::SeatHostLabels>, ApiError> {
        let (raw, witness) = self.dispatch(
            &["api", "snapshot"],
            budget,
            Duration::from_secs(2),
            super::continuity::PEER_WITNESS_SUPPORTED,
            false,
            false,
        )?;
        let incarnation = witness
            .as_ref()
            .map(ServerIncarnation::from_witness)
            .transpose()?
            .map(|server| server.identity);
        let mut labels = crate::host::observation::normalize_seat_labels(&raw)?;
        for label in &mut labels {
            label.incarnation = incarnation.clone();
        }
        Ok(labels)
    }

    pub fn snapshot(&self, budget: &CallBudget) -> Result<NativeSnapshot, ApiError> {
        self.snapshot_witnessed(budget)
            .map(|(snapshot, _)| snapshot)
    }

    fn snapshot_witnessed(
        &self,
        budget: &CallBudget,
    ) -> Result<(NativeSnapshot, Option<LocalEndpointWitness>), ApiError> {
        let started = Instant::now();
        let (raw, witness) =
            self.run_witnessed(&["api", "snapshot"], budget, Duration::from_secs(2))?;
        let snapshot = normalize_snapshot(&raw)?;
        self.check_after_parse(budget, started, Duration::from_secs(2))?;
        Ok((snapshot, witness))
    }

    pub fn prompt(&self, target: &str, text: &str, budget: &CallBudget) -> PromptOutcome {
        self.prompt_with_limit(target, text, budget, Duration::from_secs(2))
    }

    pub fn prompt_with_limit(
        &self,
        target: &str,
        text: &str,
        budget: &CallBudget,
        limit: Duration,
    ) -> PromptOutcome {
        let started = Instant::now();
        let raw = match self.run(&["agent", "prompt", target, text], budget, limit) {
            Ok(raw) => raw,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::DeadlineExceeded
                        | ErrorCode::Cancelled
                        | ErrorCode::HostUnavailable
                        | ErrorCode::StaleHostObservation
                ) =>
            {
                return PromptOutcome::Unknown;
            }
            Err(error) => return PromptOutcome::Rejected(error),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => return PromptOutcome::Unknown,
        };
        if self.check_after_parse(budget, started, limit).is_err() {
            return PromptOutcome::Unknown;
        }
        if let Some(host_error) = structured_host_error(&parsed) {
            return PromptOutcome::Rejected(host_error);
        }
        let result = parsed.get("result");
        let type_ok = result
            .and_then(|r| r.get("type"))
            .and_then(serde_json::Value::as_str)
            == Some("agent_prompted");
        let target_ok = result
            .and_then(|r| r.get("agent"))
            .and_then(|a| a.get("pane_id"))
            .and_then(serde_json::Value::as_str)
            == Some(target);
        if type_ok && target_ok {
            PromptOutcome::Submitted
        } else {
            PromptOutcome::Unknown
        }
    }

    pub fn run(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
    ) -> Result<String, ApiError> {
        self.dispatch(args, budget, limit, false, true, false)
            .map(|(body, _)| body)
    }

    /// A diagnostic/advisory read: it carries no fence and never invalidates
    /// the connection epoch, however it fails.
    fn run_unfenced(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
    ) -> Result<String, ApiError> {
        self.dispatch(args, budget, limit, false, false, false)
            .map(|(body, _)| body)
    }

    /// Identity reads carry the server-process witness of their own response
    /// connections where the platform supports it (macOS, Linux). Elsewhere the
    /// witness is absent and incarnation stays Unknown.
    fn run_witnessed(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
    ) -> Result<(String, Option<LocalEndpointWitness>), ApiError> {
        self.dispatch(
            args,
            budget,
            limit,
            super::continuity::PEER_WITNESS_SUPPORTED,
            true,
            false,
        )
    }

    /// `None` (a ping refused by the floor) clears the observation.
    fn record_release(&self, release: Option<super::compatibility::HostRelease>) {
        if let Ok(mut slot) = self.release.lock() {
            *slot = release;
        }
    }

    fn dispatch(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
        witnessed: bool,
        fenced: bool,
        preserve_start_refusal: bool,
    ) -> Result<(String, Option<LocalEndpointWitness>), ApiError> {
        if !self.socket.is_absolute() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "host API endpoint must be absolute",
            ));
        }
        if args
            .iter()
            .try_fold(0_usize, |size, arg| size.checked_add(arg.len()))
            .is_none_or(|size| size > 1024 * 1024)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "host API request exceeds byte limit",
            ));
        }
        let (method, params) = match args {
            ["pane", "current", "--current", caller] => {
                ("pane.current", serde_json::json!({"caller_pane_id":caller}))
            }
            ["pane", "get", target] => ("pane.get", serde_json::json!({"pane_id":target})),
            ["pane", "process-info", target] => {
                ("pane.process_info", serde_json::json!({"pane_id":target}))
            }
            ["api", "snapshot"] => ("session.snapshot", serde_json::json!({})),
            ["agent", "get", target] => ("agent.get", serde_json::json!({"target":target})),
            ["agent", "read", target, "--source", source] => (
                "agent.read",
                serde_json::json!({"target":target,"source":source}),
            ),
            [
                "agent",
                "read",
                target,
                "--source",
                source,
                "--format",
                format,
            ] => (
                "agent.read",
                serde_json::json!({"target":target,"source":source,"format":format}),
            ),
            ["pane", "send-keys", target, keys @ ..] if !keys.is_empty() => (
                "pane.send_keys",
                serde_json::json!({"pane_id":target,"keys":keys}),
            ),
            ["pane", "send-text", target, text] => (
                "pane.send_text",
                serde_json::json!({"pane_id":target,"text":text}),
            ),
            ["pane", "read", target] => (
                "pane.read",
                serde_json::json!({
                    "pane_id":target,"source":"recent_unwrapped",
                    "lines":EARLY_EXIT_PANE_LINES
                }),
            ),
            ["agent", "prompt", target, text] => (
                "agent.prompt",
                serde_json::json!({"target":target,"text":text}),
            ),
            ["agent", "send-keys", target, key] => (
                "agent.send_keys",
                serde_json::json!({"target":target,"keys":[key]}),
            ),
            [
                "agent",
                "start",
                name,
                "--kind",
                kind,
                "--pane",
                target,
                "--timeout",
                timeout,
                "--",
                rest @ ..,
            ] => {
                let timeout_ms = timeout
                    .parse::<u64>()
                    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid start timeout"))?;
                (
                    "agent.start",
                    serde_json::json!({
                        "name":name,"kind":kind,"pane_id":target,
                        "args":rest,"timeout_ms":timeout_ms
                    }),
                )
            }
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "unsupported host API route",
                ));
            }
        };
        let id = format!("ht-{}", CALL_ID.fetch_add(1, Ordering::Relaxed));
        // Every answered ping updates the observed release, even when the
        // operation after it fails (an operation this Herdr no longer serves
        // is exactly when Health must show the newer release).
        let observe = |release| self.record_release(release);
        let outcome = if witnessed {
            super::transport::request_witnessed_observed(
                &self.socket,
                &id,
                method,
                params,
                self.clock.as_ref(),
                budget,
                limit,
                &observe,
            )
            .map(|response| (response.body, Some(response.witness)))
        } else if method == "agent.start" && preserve_start_refusal {
            super::transport::request_unhinted_start(
                &self.socket,
                &id,
                params,
                self.clock.as_ref(),
                budget,
                limit,
                &observe,
            )
            .map(|body| (body, None))
        } else {
            super::transport::request(
                &self.socket,
                &id,
                method,
                params,
                self.clock.as_ref(),
                budget,
                limit,
                &observe,
            )
            .map(|body| (body, None))
        };
        if fenced
            && outcome.as_ref().is_err_and(|error| {
                matches!(
                    error.code,
                    ErrorCode::Cancelled
                        | ErrorCode::DeadlineExceeded
                        | ErrorCode::HostUnavailable
                        | ErrorCode::StaleHostObservation
                )
            })
        {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        outcome
    }

    fn check_after_parse(
        &self,
        budget: &CallBudget,
        started: Instant,
        limit: Duration,
    ) -> Result<(), ApiError> {
        let checked = self.check_after_parse_unfenced(budget, started, limit);
        if checked.is_err() {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
        checked
    }

    /// The parse-time checks of [`Self::check_after_parse`] without the epoch
    /// bump, for diagnostic reads that carry no fence.
    fn check_after_parse_unfenced(
        &self,
        budget: &CallBudget,
        started: Instant,
        limit: Duration,
    ) -> Result<(), ApiError> {
        if budget.cancellation.is_cancelled() {
            return Err(error(
                ErrorCode::Cancelled,
                "host call cancelled during parse",
            ));
        }
        if started.elapsed() >= limit || self.clock.monotonic_now() >= budget.deadline {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "host call expired during parse",
            ));
        }
        Ok(())
    }

    fn pane_agent_within(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
        limit: Duration,
    ) -> Result<Option<ports::PaneAgentObservation>, ApiError> {
        self.check_context(context)?;
        let started = Instant::now();
        match self.run_unfenced(&["agent", "get", target.as_str()], &context.budget, limit) {
            Err(failure) if failure.code == ErrorCode::NotFound => Ok(None),
            Err(failure) => Err(failure),
            Ok(raw) => {
                let parsed = crate::host::observation::normalize_pane_agent(&raw, target.as_str())?;
                self.check_after_parse_unfenced(&context.budget, started, limit)?;
                Ok(parsed)
            }
        }
    }
}

impl HostPort for NativeCli {
    fn seat_labels(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::SeatHostLabels>, ApiError> {
        self.participant_labels(budget)
    }

    fn live_topology(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        self.topology(budget)
    }

    fn native_launch_capability(&self) -> NativeLaunchCapability {
        // Herdr 0.9.1 `agent.start` checks shell ownership and prompt
        // readiness itself. The same-incarnation preflight needs the kernel
        // peer witness (macOS and Linux); elsewhere every observation is
        // unverified and launch stays unsupported.
        if super::continuity::PEER_WITNESS_SUPPORTED {
            NativeLaunchCapability::HostGuardedStart
        } else {
            NativeLaunchCapability::Unsupported
        }
    }

    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.observe_target(target, context, false)
    }

    fn observe_current_target_for_poke(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.observe_target(target, context, true)
    }

    fn wake_block(&self, seat: &SeatId) -> Option<WakeBlock> {
        let now = self.clock.monotonic_now().0;
        let (block, at) = *self.wake_blocks.lock().ok()?.get(seat)?;
        (now.saturating_sub(at) < WAKE_BLOCK_MILLIS).then_some(block)
    }

    /// The pane's non-shell foreground processes from Herdr's `pane.process_info` (advisory,
    /// unfenced). The composer wake matches a binding's fingerprint pid among them (a process
    /// spanning seats needs only one of them); none reported, and it does not fire. (The managed-launch placeholder keeps the stricter
    /// single-process read, `started_process_pid`.)
    fn foreground_pids(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Vec<u32>, ApiError> {
        self.check_context(context)?;
        let raw = self.run_unfenced(
            &["pane", "process-info", target.as_str()],
            &context.budget,
            Duration::from_millis(750),
        )?;
        Ok(serde_json::from_str(&raw)
            .ok()
            .map(|response: serde_json::Value| foreground_process_pids(&response))
            .unwrap_or_default())
    }

    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ports::ComposerObservation, ApiError> {
        let started = Instant::now();
        let (observation, composer) = self.observe_target_with_composer(target, context, true)?;
        // Composer I/O happens after pane.get's first fence. A concurrent host
        // failure or cancellation during that read must not earn idle evidence.
        self.check_epoch(observation.epoch)?;
        self.check_context(context)?;
        self.check_after_parse_unfenced(&context.budget, started, Duration::from_secs(5))?;
        Ok(ports::ComposerObservation(observation, composer))
    }

    /// Herdr 0.9.1 builds `session.snapshot` in one `&self` call on its
    /// single app loop (`src/app/api/session.rs` `session_snapshot`, routed
    /// from `src/app/api.rs` `Method::SessionSnapshot`), so one response is a
    /// complete internally coherent enumeration. It is authoritative only
    /// together with a verified server incarnation from the same connections.
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.check_context(context)?;
        let epoch = self.epoch();
        let started = self.clock.monotonic_now();
        let (native, witness) = self.snapshot_witnessed(&context.budget)?;
        self.check_epoch(epoch)?;
        let incarnation = witness
            .as_ref()
            .map(ServerIncarnation::from_witness)
            .transpose()?;
        self.check_boot(context, incarnation.as_ref())?;
        let sequence = self.next_sequence();
        let targets = native
            .panes
            .into_iter()
            .map(|pane| {
                self.observation(
                    pane,
                    epoch,
                    started,
                    incarnation.as_ref(),
                    ObservationProvenance::CoherentEnumeration,
                    EvidenceKind::CoherentEnumeration,
                    sequence,
                )
            })
            .collect();
        Ok(match incarnation {
            Some(incarnation) => HostSnapshot {
                boot: incarnation.boot,
                epoch,
                observation_sequence: sequence,
                complete: true,
                enumeration: EnumerationEvidence::CoherentVerified,
                incarnation: IncarnationEvidence::Verified {
                    identity: incarnation.identity,
                    evidence_kind: EvidenceKind::CoherentEnumeration,
                },
                targets,
            },
            None => HostSnapshot {
                boot: unknown_boot(),
                epoch,
                observation_sequence: 0,
                complete: true,
                enumeration: EnumerationEvidence::CompleteUnverified,
                incarnation: IncarnationEvidence::Unknown,
                targets,
            },
        })
    }

    /// Cooperative native policy: Herdr 0.9.1 cannot prove the current
    /// native execution, so the target is structural (a fresh current-target
    /// read of the same terminal in the same verified server incarnation, no
    /// positive evidence of an empty shell, active turn or blocked UI). Typed
    /// composer is classified by `submit_prompt` immediately before delivery.
    /// A nonempty/unreadable composer refuses delivery; focused empty panes
    /// must qualify through a minute of process-local observations.
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if matches!(
            observation.ui,
            HostUiState::ActiveTurn | HostUiState::ApprovalOrQuestion
        ) {
            return None;
        }
        self.cooperative_target(seat, observation)
    }

    /// A poke decides the UI state itself (`poke_eligibility`), so only an
    /// open approval or question is excluded here; the structural checks are
    /// the wake target's.
    fn safe_poke_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if observation.ui == HostUiState::ApprovalOrQuestion {
            return None;
        }
        self.cooperative_target(seat, observation)
    }

    /// Fresh witnessed `agent.get` recheck immediately before a witnessed
    /// `agent.prompt`: the same pane and terminal in the same server
    /// incarnation and connection epoch, a recognized harness agent
    /// (`claude`/`codex`) whose Herdr status is `idle` or `done`. Herdr
    /// itself rejects a prompt to a blocked agent before any input is sent.
    /// Every refusal before submission is `TargetUnsafe` (nothing was sent);
    /// a possible submission without a correlated response is
    /// `OutcomeUnknown`; `Submitted` is transport submission only.
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<ports::PromptOutcome, ApiError> {
        self.submit_prompt_mode(target, text, context, None)
            .map(|outcome| outcome.unwrap_or(ports::PromptOutcome::OutcomeUnknown))
    }

    fn submit_prompt_if_owed(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
        still_owed: &dyn Fn() -> Result<bool, ApiError>,
    ) -> Result<Option<ports::PromptOutcome>, ApiError> {
        self.submit_prompt_mode(target, text, context, Some(still_owed))
    }

    /// Retained port compatibility; attention now requires idle/done even
    /// through this entry point. Historical turn-time recipes are not admission.
    fn submit_prompt_during_turn(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<ports::PromptOutcome, ApiError> {
        if target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .and_then(|id| self.composer_policy(Harness::Agent(id)))
            .is_none()
        {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                "soft poke unsupported: no composer policy",
            ));
        }
        self.submit_prompt_mode(target, text, context, None)
            .map(|outcome| outcome.unwrap_or(ports::PromptOutcome::OutcomeUnknown))
    }

    /// Reads the composer (`agent read --source detection`), clears it with a
    /// bounded `ctrl+u` loop until a second read shows it empty, and returns
    /// the saved text; the dispatcher then submits the poke and calls
    /// [`Self::restore_composer`]. Any failure before the composer is empty
    /// aborts with nothing submitted; once a key was sent, the saved text is
    /// kept in the daemon log because the composer may already be partly
    /// cleared.
    fn stash_composer(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        self.composer_fence(target, context, "composer stash")?;
        let Some(harness) = target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .map(Harness::Agent)
        else {
            return Ok(ComposerStash::Failed("no bound harness".into()));
        };
        if self.composer_policy(harness).is_none() {
            return Ok(ComposerStash::Failed(
                "composer stash unsupported: no composer policy".into(),
            ));
        }
        let first = match self.read_composer(&target.target, harness, context) {
            Ok(read) => read,
            Err(failure) if composer_call_aborts(&failure) => return Err(failure),
            Err(failure) => {
                return Ok(ComposerStash::Failed(format!(
                    "composer read failed: {}",
                    failure.detail
                )));
            }
        };
        let text = match first {
            ComposerRead::Empty => return Ok(ComposerStash::Saved(String::new())),
            ComposerRead::Text(text) => text,
            ComposerRead::Unsafe { reason, .. } => {
                return Ok(ComposerStash::Failed(reason.into()));
            }
            ComposerRead::Unreadable => {
                return Ok(ComposerStash::Failed("composer unreadable".into()));
            }
        };
        self.clear_composer(target, harness, text, context)
    }

    /// One `clear_key` press, under the composer fence, only when the bound harness opts in
    /// (`clears_unsubmitted_wake`) and a fresh structured read is `Text` equal to `text`: any
    /// other content (a draft, the wake plus typed input, an unreadable screen) is left alone.
    fn clear_own_wake_text(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<bool, ApiError> {
        let Some(harness) = target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .map(Harness::Agent)
        else {
            return Ok(false);
        };
        let Some(policy) = self
            .composer_policy(harness)
            .filter(|policy| policy.clears_unsubmitted_wake())
        else {
            return Ok(false);
        };
        let clear_key = policy.clear_key();
        if clear_key != "ctrl+u" {
            return Ok(false);
        }
        self.composer_fence(target, context, "wake clear")?;
        // An Enter taken as a newline leaves an empty second row; a press there joins it to the
        // wake row first, so each further press is gated on a fresh read holding just the wake.
        for press in 0..=WAKE_CLEAR_PRESSES {
            match self.read_composer(&target.target, harness, context)? {
                ComposerRead::Empty => return Ok(press > 0),
                ComposerRead::Text(held) if held == text && press < WAKE_CLEAR_PRESSES => {
                    self.composer_call(
                        &["pane", "send-keys", target.target.as_str(), clear_key],
                        context,
                    )?;
                }
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Retypes the stashed text with `pane send-text` (real newlines) and
    /// never an Enter, so the person's draft is not submitted (Q4).
    fn restore_composer(
        &self,
        target: &SafeWakeTarget,
        saved: &str,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        if saved.is_empty() {
            return Ok(());
        }
        self.composer_fence(target, context, "composer restore")?;
        let policy = target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .and_then(|id| self.composer_policy(Harness::Agent(id)))
            .ok_or_else(|| {
                error(
                    ErrorCode::UnsupportedHarness,
                    "composer restore unsupported: no composer policy",
                )
            })?;
        let restored = policy.restore_text(saved);
        if restored != saved {
            return Err(error(
                ErrorCode::TargetUnsafe,
                "composer restore changed saved text",
            ));
        }
        self.composer_call(
            &["pane", "send-text", target.target.as_str(), &restored],
            context,
        )
        .map(|_| ())
    }

    /// Advisory read: unfenced, never moves the epoch. The settle wait ends
    /// early (`Unknown`) when the call is cancelled.
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        let budget = &context.budget;
        let left = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        if budget.cancellation.is_cancelled()
            || context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || left <= COMPOSER_SETTLE_MILLIS + MIN_PROMPT_MILLIS
        {
            return Ok(crate::ports::AgentComposerState::Unknown);
        }
        if budget
            .cancellation
            .wait_blocking(Duration::from_millis(COMPOSER_SETTLE_MILLIS))
        {
            return Ok(crate::ports::AgentComposerState::Unknown);
        }
        let left = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        let read = self.run_unfenced(
            &["pane", "read", target.target.as_str()],
            budget,
            Duration::from_millis(left.min(PROMPT_RECHECK_MILLIS)),
        );
        Ok(composer_state_from_read(read.as_deref().ok()))
    }

    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        if target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .and_then(|id| self.composer_policy(Harness::Agent(id)))
            .is_none()
        {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                "submit key unsupported: no composer policy",
            ));
        }
        let refuse = |detail: &str| {
            error(
                ErrorCode::TargetUnsafe,
                format!("submit key refused before sending: {detail}"),
            )
        };
        if target.basis != WakeTargetBasis::CooperativeAgent {
            return Err(refuse("native execution is unverified"));
        }
        if context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || self.epoch() != target.epoch
        {
            return Err(refuse("host context changed"));
        }
        let budget = &context.budget;
        if budget.cancellation.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "submit key cancelled"));
        }
        let left = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        if left < MIN_PROMPT_MILLIS {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "insufficient submit key budget",
            ));
        }
        let (_, witness) = self.run_witnessed(
            &["agent", "send-keys", target.target.as_str(), "enter"],
            budget,
            Duration::from_millis(left.min(PROMPT_SUBMIT_MILLIS)),
        )?;
        if !self.same_incarnation(witness.as_ref(), target) || self.epoch() != target.epoch {
            return Err(refuse("host server incarnation or epoch changed"));
        }
        Ok(())
    }

    /// A diagnostic/advisory read: it carries no fence and never invalidates
    /// the connection epoch, so a slow or failed read cannot refuse another
    /// call.
    fn observe_pane_agent(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Option<ports::PaneAgentObservation>, ApiError> {
        self.pane_agent_within(target, context, Duration::from_millis(750))
    }

    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.launch_native_with_evidence(request, context)
            .map_err(|failure| failure.error)
    }
    fn launch_native_with_evidence(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ports::NativeLaunchFailure> {
        if self.native_launch_capability() == NativeLaunchCapability::Unsupported {
            return Err(ports::NativeLaunchFailure::not_submitted(error(
                ErrorCode::Unsupported,
                "verified empty-pane preflight and native launch are unavailable",
            )));
        }
        // This final witnessed observation happens before native input. Its
        // failures carry explicit NotSubmitted evidence for compound retry.
        let preflight = self
            .observe_current_target(&request.target, context)
            .map_err(|failure| {
                ports::NativeLaunchFailure::not_submitted(error(
                    if failure.code == ErrorCode::NotFound {
                        ErrorCode::NotFound
                    } else {
                        ErrorCode::StaleHostObservation
                    },
                    format!("native start preflight failed: {}", failure.detail),
                ))
            })?;
        self.guarded_start_with_evidence(&request, context, &preflight)
    }

    fn resume_after_epoch(&self, persisted: u64) {
        NativeCli::resume_after_epoch(self, persisted);
    }

    /// Cooperative safe prompt needs the kernel peer witness (macOS, Linux) for its
    /// same-incarnation recheck and submission.
    fn safe_prompt_capability(&self) -> CapabilityState {
        if super::continuity::PEER_WITNESS_SUPPORTED {
            CapabilityState::Supported
        } else {
            CapabilityState::Unsupported
        }
    }

    fn observed_release(&self) -> Option<super::compatibility::HostRelease> {
        self.release.lock().ok().and_then(|slot| slot.clone())
    }

    fn incarnation_witness(&self) -> crate::protocol::results::CapabilityState {
        // The kernel peer PID/start-time witness exists on macOS and Linux;
        // other platforms report Unknown incarnation on every read.
        if super::continuity::PEER_WITNESS_SUPPORTED {
            crate::protocol::results::CapabilityState::Unknown
        } else {
            crate::protocol::results::CapabilityState::Unsupported
        }
    }
}

/// The Herdr agent record names the wake target's pane and terminal and is a
/// recognized harness awaiting input: `idle`, or `done` (its turn finished
/// and the output is not yet seen). `working`, `blocked` and `unknown` are
/// never prompted; neither is a shell or any other detected program.
fn cooperative_wake_ready(
    agent: &serde_json::Value,
    target: &SafeWakeTarget,
    registry: &Registry,
) -> Result<(), String> {
    let text = |name: &str| agent.get(name).and_then(serde_json::Value::as_str);
    if text("pane_id") != Some(target.target.as_str())
        || text("terminal_id") != Some(target.terminal.as_str())
    {
        return Err("agent is not in the target terminal".into());
    }
    let kind = text("agent");
    if kind.is_none_or(|kind| registry.by_host_kind(kind).is_none()) {
        return Err(format!(
            "no recognized harness agent (agent {})",
            kind.unwrap_or("none").chars().take(32).collect::<String>()
        ));
    }
    match target.bound_harness.as_deref() {
        Some(bound)
            if !registry
                .agent(bound)
                .ok()
                .and_then(|id| registry.by_id(id).ok())
                .is_some_and(|registration| {
                    kind.is_some_and(|kind| {
                        registration.metadata().host_kinds.contains(&kind)
                            && registration
                                .launch_policy()
                                .is_none_or(|policy| policy.expected_host_kinds().contains(&kind))
                    })
                }) =>
        {
            return Err(format!(
                "agent kind {} differs from the bound harness {bound}",
                kind.unwrap_or("none").chars().take(32).collect::<String>()
            ));
        }
        None if target.basis == WakeTargetBasis::CooperativeAgent => {
            return Err("no bound harness for a cooperative wake".into());
        }
        _ => {}
    }
    let status = text("agent_status");
    let ready = status.is_some_and(|status| WAKE_READY_STATUSES.contains(&status));
    if !ready {
        return Err(format!(
            "agent is not awaiting input (status {})",
            status
                .unwrap_or("none")
                .chars()
                .take(32)
                .collect::<String>()
        ));
    }
    Ok(())
}

impl NativeCli {
    /// Empty samples are process-local, bounded, and tied to the same target.
    /// Missing samples for over a minute restart qualification. These are
    /// screen observations, not a claim that no keyboard activity occurred.
    fn focused_empty_ready(
        &self,
        target: &SafeWakeTarget,
        focused: bool,
        previous: Option<EmptyComposerWindow>,
    ) -> bool {
        if !focused {
            return true;
        }
        let now = self.clock.monotonic_now().0;
        let mut identity = target.clone();
        // This counter orders reads, not the identity of the composer.
        identity.observation_sequence = 0;
        let since = previous
            .filter(|window| {
                window.target == identity
                    && now >= window.last
                    && now - window.last <= FOCUSED_EMPTY_MILLIS
            })
            .map_or(now, |window| window.since);
        if now.saturating_sub(since) >= FOCUSED_EMPTY_MILLIS {
            return true;
        }
        let Ok(mut windows) = self.empty_windows.lock() else {
            return false;
        };
        windows.retain(|_, window| now >= window.last && now - window.last <= FOCUSED_EMPTY_MILLIS);
        if windows.len() < MAX_EMPTY_WINDOWS {
            windows.insert(
                target.seat.clone(),
                EmptyComposerWindow {
                    target: identity,
                    since,
                    last: now,
                },
            );
        }
        false
    }

    /// Records the block a wake attempt met (`None`: the attempt got past the composer checks).
    fn note_wake_block(&self, seat: &SeatId, block: Option<WakeBlock>) {
        let Ok(mut blocks) = self.wake_blocks.lock() else {
            return;
        };
        let Some(block) = block else {
            blocks.remove(seat);
            return;
        };
        let now = self.clock.monotonic_now().0;
        if blocks.len() >= MAX_WAKE_BLOCKS && !blocks.contains_key(seat) {
            blocks.retain(|_, (_, at)| now.saturating_sub(*at) < WAKE_BLOCK_MILLIS);
            if blocks.len() >= MAX_WAKE_BLOCKS {
                return;
            }
        }
        blocks.insert(seat.clone(), (block, now));
    }

    /// `still_owed` (when given) is asked once the agent was rechecked idle or done and its
    /// composer read empty, immediately before `agent prompt`; `Ok(None)`: it declined and
    /// nothing was sent. Without it the result is always `Some`.
    fn submit_prompt_mode(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
        still_owed: Option<&dyn Fn() -> Result<bool, ApiError>>,
    ) -> Result<Option<ports::PromptOutcome>, ApiError> {
        // Taking the sample first makes every failed identity/status/read reset
        // the window. Only a successful focused-empty sample below retains it.
        let previous = self
            .empty_windows
            .lock()
            .map_err(|_| {
                error(
                    ErrorCode::TargetUnsafe,
                    "composer observation lock unavailable",
                )
            })?
            .remove(&target.seat);
        let refuse = |detail: &str| {
            error(
                ErrorCode::TargetUnsafe,
                format!("wake prompt refused before submission: {detail}"),
            )
        };
        if target.basis != WakeTargetBasis::CooperativeAgent {
            return Err(refuse("native execution is unverified"));
        }
        if self.safe_prompt_capability() != CapabilityState::Supported {
            return Err(refuse("no server incarnation witness on this platform"));
        }
        if context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || self.epoch() != target.epoch
        {
            return Err(refuse("host context changed"));
        }
        let budget = &context.budget;
        let remaining = |this: &Self| {
            budget
                .deadline
                .0
                .saturating_sub(this.clock.monotonic_now().0)
        };
        if budget.cancellation.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "wake prompt cancelled"));
        }
        let left = remaining(self);
        if left <= 2 * MIN_PROMPT_MILLIS {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "insufficient wake prompt budget",
            ));
        }
        let recheck_limit = Duration::from_millis(
            left.saturating_sub(MIN_PROMPT_MILLIS)
                .min(PROMPT_RECHECK_MILLIS),
        );
        let (raw, witness) = match self.run_witnessed(
            &["agent", "get", target.target.as_str()],
            budget,
            recheck_limit,
        ) {
            Ok(response) => response,
            Err(failure) => {
                return Err(match failure.code {
                    ErrorCode::Cancelled
                    | ErrorCode::DeadlineExceeded
                    | ErrorCode::HostUnavailable => failure,
                    _ => refuse(&failure.detail),
                });
            }
        };
        if !self.same_incarnation(witness.as_ref(), target) || self.epoch() != target.epoch {
            return Err(refuse("host server incarnation or epoch changed"));
        }
        let agent = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|value| value.pointer("/result/agent").cloned())
            .ok_or_else(|| refuse("unreadable agent recheck"))?;
        if let Err(detail) = cooperative_wake_ready(&agent, target, self.registry) {
            return Err(refuse(&detail));
        }
        let harness = target
            .bound_harness
            .as_deref()
            .and_then(|id| self.registry.agent(id).ok())
            .map(Harness::Agent)
            .ok_or_else(|| refuse("no bound harness for composer read"))?;
        let focused = agent
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| refuse("unreadable pane focus"))?;
        match self.read_composer(&target.target, harness, context)? {
            ComposerRead::Empty => {}
            ComposerRead::Unreadable => {
                self.note_wake_block(&target.seat, Some(WakeBlock::Unreadable));
                return Err(refuse("composer is unreadable"));
            }
            _ => {
                self.note_wake_block(&target.seat, None);
                return Err(refuse("composer is nonempty"));
            }
        }
        self.check_epoch(target.epoch)?;
        self.check_context(context)?;
        if !self.focused_empty_ready(target, focused, previous) {
            self.note_wake_block(&target.seat, Some(WakeBlock::Focused));
            return Err(refuse(
                "focused composer has not been observed empty for one minute",
            ));
        }
        self.note_wake_block(&target.seat, None);
        if budget.cancellation.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "wake prompt cancelled"));
        }
        // The agent is idle now, so a turn that served this attention has ended and its
        // completion is committed: ask whether the wake is still owed only after the recheck.
        if let Some(still_owed) = still_owed
            && !still_owed()?
        {
            return Ok(None);
        }
        // Sampled after the owed check: a slow store read must end in this pre-send
        // refusal, not in a prompt that times out with nothing typed (OutcomeUnknown).
        let left = remaining(self);
        if left < MIN_PROMPT_MILLIS {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "insufficient wake prompt budget",
            ));
        }
        let limit = Duration::from_millis(left.min(PROMPT_SUBMIT_MILLIS));
        let (raw, witness) = match self.run_witnessed(
            &["agent", "prompt", target.target.as_str(), text],
            budget,
            limit,
        ) {
            Ok(response) => response,
            // Herdr answers these before typing anything (a blocked agent is
            // rejected "before any input is sent"; a missing agent or pane
            // has nowhere to type); an invalid request never left us, and a
            // Herdr that does not serve or accept the method typed nothing.
            Err(failure)
                if matches!(
                    failure.code,
                    ErrorCode::TargetUnsafe
                        | ErrorCode::NotFound
                        | ErrorCode::InvalidRequest
                        | ErrorCode::Unsupported
                ) =>
            {
                return Err(refuse(&failure.detail));
            }
            Err(_) => return Ok(Some(ports::PromptOutcome::OutcomeUnknown)),
        };
        let prompted = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|value| value.pointer("/result/agent").cloned());
        let correlated = prompted.as_ref().is_some_and(|agent| {
            agent.get("pane_id").and_then(serde_json::Value::as_str) == Some(target.target.as_str())
                && agent.get("terminal_id").and_then(serde_json::Value::as_str)
                    == Some(target.terminal.as_str())
        });
        if correlated && self.same_incarnation(witness.as_ref(), target) {
            Ok(Some(ports::PromptOutcome::Submitted))
        } else {
            Ok(Some(ports::PromptOutcome::OutcomeUnknown))
        }
    }

    /// The structural target of a cooperative wake or poke: a fresh verified
    /// terminal in a witnessed server incarnation with no verified execution.
    /// UI exclusions are the callers' (a wake and a poke differ).
    fn cooperative_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if self.safe_prompt_capability() != CapabilityState::Supported
            || observation.provenance != ObservationProvenance::FreshCurrentTarget
            || observation.generation == 0
            || matches!(observation.execution, ExecutionEvidence::Verified { .. })
            || observation.occupancy == StructuralOccupancy::EmptyShell
        {
            return None;
        }
        let IncarnationEvidence::Verified {
            identity,
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        } = &observation.incarnation
        else {
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

    /// One fresh witnessed read. Only `composer_ui` (a soft-deadline poke)
    /// spends the composer read that classifies the UI; an ordinary wake never
    /// does, so a composer read that Herdr cannot answer cannot change a wake
    /// structural selection. Ordinary delivery still requires its own final
    /// composer read in `submit_prompt_mode`.
    fn observe_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
        composer_ui: bool,
    ) -> Result<HostObservation, ApiError> {
        self.observe_target_with_composer(target, context, composer_ui)
            .map(|(observation, _)| observation)
    }

    fn observe_target_with_composer(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
        composer_ui: bool,
    ) -> Result<(HostObservation, Option<ports::RegisteredComposerEvidence>), ApiError> {
        self.check_context(context)?;
        let epoch = self.epoch();
        let started = self.clock.monotonic_now();
        let (pane, witness) = self.pane_witnessed(target.as_str(), &context.budget)?;
        self.check_epoch(epoch)?;
        let incarnation = witness
            .as_ref()
            .map(ServerIncarnation::from_witness)
            .transpose()?;
        self.check_boot(context, incarnation.as_ref())?;
        // Herdr's `agent_status` is not a composer: an idle agent may hold
        // typed text, so a poke's UI state also needs the composer read.
        let (ui, composer) = if composer_ui {
            self.observed_ui(&pane, context)
        } else {
            (status_ui(&pane, self.registry), None)
        };
        let reset_empty = !matches!(pane.status.as_str(), "idle" | "done")
            || (composer_ui && ui != HostUiState::Idle);
        let sequence = self.next_sequence();
        let mut observation = self.observation(
            pane,
            epoch,
            started,
            incarnation.as_ref(),
            ObservationProvenance::FreshCurrentTarget,
            EvidenceKind::NativeCurrentTarget,
            sequence,
        );
        observation.ui = ui;
        if reset_empty || !observation.focused {
            self.reset_empty_windows(target);
        }
        Ok((observation, composer))
    }

    /// The UI state a pane shows: Herdr's agent status plus, for an idle,
    /// done or working claude/codex agent, a composer read. A failed or
    /// timed-out read never fails the observation; it yields `Unknown`.
    fn observed_ui(
        &self,
        pane: &NativePane,
        context: &HostCallContext,
    ) -> (HostUiState, Option<ports::RegisteredComposerEvidence>) {
        // The bounded transport supplies the string; exact immutable registry
        // membership determines which declared alias may select a parser.
        let Some(kind) = pane.agent.as_deref() else {
            return (HostUiState::Unknown, None);
        };
        let Some(parser) = self
            .registry
            .by_host_kind(kind)
            .and_then(|r| self.registry.agent(r.metadata().id).ok())
        else {
            return (HostUiState::Unknown, None);
        };
        let status = pane.status.as_str();
        match status {
            "blocked" => (composer::observed_ui(Some(status), None), None),
            "idle" | "done" | "working" => {
                if self.composer_policy(Harness::Agent(parser)).is_none() {
                    return (HostUiState::Unknown, None);
                }
                let read = self
                    .read_composer(&pane.target, Harness::Agent(parser), context)
                    .ok();
                let ui = composer::observed_ui(Some(status), read.as_ref());
                let evidence = read.as_ref().map(|read| ports::RegisteredComposerEvidence {
                    parser,
                    reported_host_kind: kind.into(),
                    classification: match read {
                        ComposerRead::Empty => ports::ComposerClassification::Empty,
                        ComposerRead::Text(_) => ports::ComposerClassification::Text,
                        ComposerRead::Unsafe { .. } => ports::ComposerClassification::Unsafe,
                        ComposerRead::Unreadable => ports::ComposerClassification::Unreadable,
                    },
                    basis: ports::ComposerEvidenceBasis::RegisteredHostKindComposerRead,
                });
                (ui, evidence)
            }
            _ => (HostUiState::Unknown, None),
        }
    }

    fn composer_policy(&self, harness: Harness) -> Option<&dyn ComposerPolicy> {
        let Harness::Agent(id) = harness else {
            return None;
        };
        self.registry.by_id(id).ok()?.composer_policy()
    }

    /// One bounded `agent read --source detection`, composer-parsed, and one
    /// styled read when the policy asks for it. The pane width is not in
    /// Herdr's pane record; Claude's rule length stands in.
    fn read_composer(
        &self,
        target: &HostTargetId,
        harness: Harness,
        context: &HostCallContext,
    ) -> Result<ComposerRead, ApiError> {
        let result = self.read_composer_text(target, harness, context);
        if !matches!(result, Ok(ComposerRead::Empty)) {
            self.reset_empty_windows(target);
        }
        result
    }

    fn reset_empty_windows(&self, target: &HostTargetId) {
        if let Ok(mut windows) = self.empty_windows.lock() {
            windows.retain(|_, window| &window.target.target != target);
        }
    }

    fn read_composer_text(
        &self,
        target: &HostTargetId,
        harness: Harness,
        context: &HostCallContext,
    ) -> Result<ComposerRead, ApiError> {
        let Some(policy) = self.composer_policy(harness) else {
            return Ok(ComposerRead::Unreadable);
        };
        let limit = composer_limit(self, context)?;
        let started = Instant::now();
        let raw = self.run_unfenced(
            &["agent", "read", target.as_str(), "--source", "detection"],
            &context.budget,
            limit,
        )?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "unreadable composer read"))?;
        let read = parsed
            .pointer("/result/read")
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "composer read has no read"))?;
        let text = |name: &str| read.get(name).and_then(serde_json::Value::as_str);
        if text("pane_id") != Some(target.as_str()) || text("source") != Some("detection") {
            return Err(error(
                ErrorCode::InvalidRequest,
                "composer read answers another pane or source",
            ));
        }
        let text = text("text")
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "composer read has no text"))?;
        self.check_after_parse_unfenced(&context.budget, started, limit)?;
        let read = composer::read_composer_in(self.registry, harness, text, None);
        if !policy.wants_styled(&read) {
            return Ok(read);
        }
        // A failed styled read keeps the detection read; an abort still aborts.
        Ok(self
            .read_styled_composer(target, policy, context)?
            .unwrap_or(read))
    }

    /// One bounded `agent read --source recent --format ansi` (detection
    /// text carries no styles), composer-parsed by the policy. `Ok(None)` when
    /// the read fails, answers another pane, source or format, or carries no
    /// parsable styles: a Herdr that ignores `format` returns plain text.
    /// Cancellation, an exhausted budget or a stale observation is an error.
    fn read_styled_composer(
        &self,
        target: &HostTargetId,
        policy: &dyn ComposerPolicy,
        context: &HostCallContext,
    ) -> Result<Option<ComposerRead>, ApiError> {
        let limit = composer_limit(self, context)?;
        let started = Instant::now();
        let raw = match self.run_unfenced(
            &[
                "agent",
                "read",
                target.as_str(),
                "--source",
                "recent",
                "--format",
                "ansi",
            ],
            &context.budget,
            limit,
        ) {
            Ok(raw) => raw,
            Err(failure) if composer_call_aborts(&failure) => return Err(failure),
            Err(_) => return Ok(None),
        };
        let screen = (|| {
            let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
            let read = parsed.pointer("/result/read")?;
            let text = |name: &str| read.get(name).and_then(serde_json::Value::as_str);
            if text("pane_id") != Some(target.as_str())
                || text("source") != Some("recent")
                || text("format") != Some("ansi")
            {
                return None;
            }
            crate::harness::sgr::parse(text("text")?)
        })();
        self.check_after_parse_unfenced(&context.budget, started, limit)?;
        Ok(screen.and_then(|screen| policy.read_styled(&screen, None)))
    }

    /// Clears the composer with a bounded `ctrl+u` loop until a read shows it
    /// empty; `text` is what the first read saved. Called by `stash_composer`
    /// only after the fence and a `Text` read.
    fn clear_composer(
        &self,
        target: &SafeWakeTarget,
        harness: Harness,
        text: String,
        context: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        let Some(policy) = self.composer_policy(harness) else {
            return Ok(ComposerStash::Failed(
                "composer stash unsupported: no composer policy".into(),
            ));
        };
        let clear_key = policy.clear_key();
        if clear_key != "ctrl+u" {
            return Ok(ComposerStash::Failed(
                "composer clear key unsupported".into(),
            ));
        }
        let attempts = text.lines().count() + COMPOSER_CLEAR_SLACK;
        for _ in 0..attempts {
            let cleared = self
                .composer_call(
                    &["pane", "send-keys", target.target.as_str(), clear_key],
                    context,
                )
                .and_then(|_| self.read_composer(&target.target, harness, context));
            match cleared {
                Ok(ComposerRead::Empty) => return Ok(ComposerStash::Saved(text)),
                Ok(_) => {}
                Err(failure) => {
                    eprintln!(
                        "herdr-threads: warning: composer clear failed ({}); typed text was {text:?}",
                        failure.detail
                    );
                    return if composer_call_aborts(&failure) {
                        Err(failure)
                    } else {
                        Ok(ComposerStash::Failed(format!(
                            "composer clear failed: {}",
                            failure.detail
                        )))
                    };
                }
            }
        }
        eprintln!(
            "herdr-threads: warning: composer not empty after clearing; typed text was {text:?}"
        );
        Ok(ComposerStash::Failed(
            "composer not empty after clearing".into(),
        ))
    }

    /// A composer key or text send, fenced like a prompt: a timeout or a
    /// stale host invalidates the connection epoch.
    fn composer_call(&self, args: &[&str], context: &HostCallContext) -> Result<String, ApiError> {
        let limit = composer_limit(self, context)?;
        self.run(args, &context.budget, limit)
    }

    /// The identity fence of a composer stash or restore, as a prompt's: the
    /// seat's cooperative terminal in the same server incarnation and epoch.
    fn composer_fence(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
        what: &str,
    ) -> Result<(), ApiError> {
        let refuse = |detail: &str| {
            error(
                ErrorCode::TargetUnsafe,
                format!("{what} refused before any input: {detail}"),
            )
        };
        if target.basis != WakeTargetBasis::CooperativeAgent {
            return Err(refuse("native execution is unverified"));
        }
        if self.safe_prompt_capability() != CapabilityState::Supported {
            return Err(refuse("no server incarnation witness on this platform"));
        }
        if context.expected_boot.as_ref() != Some(&target.host_boot)
            || context.expected_epoch != Some(target.epoch)
            || self.epoch() != target.epoch
        {
            return Err(refuse("host context changed"));
        }
        Ok(())
    }

    fn same_incarnation(
        &self,
        witness: Option<&LocalEndpointWitness>,
        target: &SafeWakeTarget,
    ) -> bool {
        witness
            .and_then(|witness| ServerIncarnation::from_witness(witness).ok())
            .is_some_and(|incarnation| {
                incarnation.identity == target.incarnation && incarnation.boot == target.host_boot
            })
    }

    /// The store passes its published `(host_boot, host_epoch)`. Without a
    /// published boot there is nothing to fence (the store's default epoch 0
    /// is not an adapter epoch). With one, the epoch is checked before I/O and
    /// the boot after the witnessed response in `check_boot`.
    fn check_context(&self, context: &HostCallContext) -> Result<(), ApiError> {
        if context.expected_boot.is_some() && context.expected_epoch != Some(self.epoch()) {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host context changed",
            ));
        }
        Ok(())
    }

    fn check_boot(
        &self,
        context: &HostCallContext,
        incarnation: Option<&ServerIncarnation>,
    ) -> Result<(), ApiError> {
        if let Some(expected) = context.expected_boot.as_ref()
            && incarnation.map(|incarnation| &incarnation.boot) != Some(expected)
        {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host server incarnation changed",
            ));
        }
        Ok(())
    }

    fn check_epoch(&self, epoch: u64) -> Result<(), ApiError> {
        if self.epoch() != epoch {
            return Err(error(
                ErrorCode::StaleHostObservation,
                "host connection epoch changed",
            ));
        }
        Ok(())
    }

    /// Structural fields only. Occupant, UI, occupancy and current execution
    /// stay Unknown: Herdr's cached agent/session metadata is not proof. The
    /// terminal ID is the structural identity; Herdr 0.9.1 has no structural
    /// generation counter (`revision` changes with content), so a verified
    /// live terminal has generation 1 and a replacement shows a new terminal.
    #[allow(clippy::too_many_arguments)]
    fn observation(
        &self,
        pane: NativePane,
        epoch: u64,
        started: crate::protocol::time::MonoInstant,
        incarnation: Option<&ServerIncarnation>,
        provenance: ObservationProvenance,
        evidence_kind: EvidenceKind,
        sequence: u64,
    ) -> HostObservation {
        let completed = self.clock.monotonic_now();
        let terminal = TerminalId::parse(pane.terminal_id).ok();
        let (host_boot, incarnation, provenance, generation, observation_sequence) =
            match incarnation {
                Some(incarnation) if terminal.is_some() => (
                    incarnation.boot.clone(),
                    IncarnationEvidence::Verified {
                        identity: incarnation.identity.clone(),
                        evidence_kind,
                    },
                    provenance,
                    1,
                    sequence,
                ),
                _ => (
                    unknown_boot(),
                    IncarnationEvidence::Unknown,
                    ObservationProvenance::UncharacterizedCache,
                    0,
                    sequence,
                ),
            };
        HostObservation {
            target: pane.target,
            host_boot,
            epoch,
            generation,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: completed,
            provenance,
            occupant: None,
            ui: HostUiState::Unknown,
            focused: pane.focused,
            terminal,
            occupancy: StructuralOccupancy::Unknown,
            incarnation,
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("cli-{}", CALL_ID.fetch_add(1, Ordering::Relaxed))),
            connection_epoch: epoch,
            observation_sequence,
            started_at_mono: started,
            completed_at_mono: completed,
        }
    }
}

/// The bound for one composer call: the ceiling or what the budget has left.
/// The UI state Herdr's `agent_status` alone shows: an open approval or
/// question is `blocked`; everything else is `Unknown`, as on every ordinary
/// wake before the composer reader existed.
fn status_ui(pane: &NativePane, registry: &Registry) -> HostUiState {
    if pane
        .agent
        .as_deref()
        .and_then(|kind| registry.by_host_kind(kind))
        .is_some()
        && pane.status == "blocked"
    {
        HostUiState::ApprovalOrQuestion
    } else {
        HostUiState::Unknown
    }
}

fn composer_limit(cli: &NativeCli, context: &HostCallContext) -> Result<Duration, ApiError> {
    if context.budget.cancellation.is_cancelled() {
        return Err(error(ErrorCode::Cancelled, "composer call cancelled"));
    }
    let left = context
        .budget
        .deadline
        .0
        .saturating_sub(cli.clock.monotonic_now().0);
    if left < MIN_PROMPT_MILLIS {
        return Err(error(
            ErrorCode::DeadlineExceeded,
            "insufficient composer call budget",
        ));
    }
    Ok(Duration::from_millis(left.min(COMPOSER_CALL_MILLIS)))
}

/// A failure that ends the whole attempt rather than skipping the stash.
fn composer_call_aborts(failure: &ApiError) -> bool {
    matches!(
        failure.code,
        ErrorCode::Cancelled | ErrorCode::DeadlineExceeded | ErrorCode::StaleHostObservation
    )
}

/// Whether pane text shows `kind` echoed at a shell prompt and a shell prompt
/// again after it (the harness ran and exited): the trailing lines then, else
/// `None`. A prompt line ends in `%`, `$`, `#` or `❯`; the echoed command
/// line (which names `kind`) does not.
/// The pid of the single foreground process of a `pane.process_info` response other than the
/// pane's shell. Several foreground processes (or none besides the shell) are not evidence of
/// which one Herdr started, so they yield `None`.
fn started_process_pid(response: &serde_json::Value) -> Option<u32> {
    let info = response.pointer("/result/process_info")?;
    let shell = info.get("shell_pid").and_then(serde_json::Value::as_u64);
    let mut pids = info
        .get("foreground_processes")?
        .as_array()?
        .iter()
        .filter_map(|process| process.get("pid").and_then(serde_json::Value::as_u64))
        .filter(|pid| Some(*pid) != shell);
    let pid = pids.next()?;
    if pids.next().is_some() {
        return None;
    }
    u32::try_from(pid).ok().filter(|pid| *pid != 0)
}

/// Every foreground process of a `pane.process_info` response other than the pane's shell: the
/// set the composer wake matches a binding's fingerprint pid against. A harness launcher and its
/// native child (npm `codex`: `node` plus the native binary) are both foreground, so the set,
/// not a single pid, is the evidence. Zero and overflowing pids are dropped; a malformed
/// response gives no pids, and the composer wake then does not fire.
fn foreground_process_pids(response: &serde_json::Value) -> Vec<u32> {
    let Some(info) = response.pointer("/result/process_info") else {
        return Vec::new();
    };
    let shell = info.get("shell_pid").and_then(serde_json::Value::as_u64);
    info.get("foreground_processes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|process| process.get("pid").and_then(serde_json::Value::as_u64))
        .filter(|pid| Some(*pid) != shell)
        .filter_map(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0)
        .collect()
}

fn shell_prompt_returned(text: &str, kind: &str) -> Option<String> {
    let ends_with_prompt = |line: &str| line.ends_with(['%', '$', '#', '❯']);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    let (last, before) = lines.split_last()?;
    if !ends_with_prompt(last)
        || !before
            .iter()
            .any(|line| line.contains(kind) && !ends_with_prompt(line))
    {
        return None;
    }
    let from = lines.len().saturating_sub(EARLY_EXIT_PANE_LINES);
    Some(lines[from..].join("\n"))
}

fn unknown_boot() -> HostBootId {
    HostBootId::new("unverified-cli-host")
}

fn error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

/// Whether the wake marker is still in the pane's composer. `None` is a
/// failed read; an unreadable or empty pane is `Unknown`.
///
/// Only the last `COMPOSER_TAIL_LINES` non-empty lines are looked at, and
/// within them the composer is anchored on its prompt line (the last line
/// starting with `>` or `›` once whitespace and box-drawing characters are
/// skipped) and the lines below it. History above the composer can hold the
/// marker (a submitted prompt stays on screen), so a marker above the prompt
/// line does not count. Without a recognizable prompt line the whole tail is
/// the composer. A TUI composer box wraps its own content, so the composer's
/// lines are joined and compared to the marker with whitespace and box
/// drawing removed.
pub(crate) fn composer_state_from_text(text: Option<&str>) -> ports::AgentComposerState {
    use ports::AgentComposerState::{HoldingPrompt, Submitted, Unknown};
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        return Unknown;
    };
    let mut tail: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(COMPOSER_TAIL_LINES)
        .collect();
    tail.reverse();
    let is_chrome = |c: char| c.is_whitespace() || ('\u{2500}'..='\u{257F}').contains(&c);
    let after_chrome = |line: &str| line.trim_start_matches(is_chrome).to_owned();
    let prompt_at = tail
        .iter()
        .rposition(|line| after_chrome(line).starts_with(['>', '›']));
    let composer: String = match prompt_at {
        Some(at) => {
            let first = after_chrome(tail[at]);
            let first = first.trim_start_matches(['>', '›']);
            std::iter::once(first)
                .chain(tail[at + 1..].iter().copied())
                .collect()
        }
        None => tail.concat(),
    };
    let normalize = |raw: &str| -> String { raw.chars().filter(|c| !is_chrome(*c)).collect() };
    let composer = normalize(&composer);
    if crate::notification::policy::WAKE_TEXTS
        .iter()
        .any(|text| composer.contains(&normalize(text)))
    {
        HoldingPrompt
    } else {
        Submitted
    }
}

/// `pane.read` response body to composer state.
fn composer_state_from_read(raw: Option<&str>) -> ports::AgentComposerState {
    let text = raw
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|parsed| {
            parsed
                .pointer("/result/read/text")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    composer_state_from_text(text.as_deref())
}

#[cfg(test)]
#[path = "../../tests/host/wake_submission.rs"]
mod wake_submission_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ports::ConfiguredHook;
    use crate::protocol::time::{Cancellation, MonoInstant, UtcMillis};
    use serde_json::{Value, json};
    use std::{
        fs,
        io::{BufRead, BufReader, Write},
        os::unix::net::{UnixListener, UnixStream},
        thread,
    };

    struct TestClock(Instant);
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.elapsed().as_millis() as u64)
        }
    }
    struct FrozenClock;
    impl Clock for FrozenClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(0)
        }
    }
    fn read(stream: &mut UnixStream) -> Value {
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    fn answer(stream: &mut UnixStream, request: &Value, result: Value) {
        writeln!(stream, "{}", json!({"id":request["id"],"result":result})).unwrap();
    }
    fn fixture<F>(response: F) -> (PathBuf, NativeCli, thread::JoinHandle<()>)
    where
        F: FnOnce(&mut UnixStream, Value) + Send + 'static,
    {
        let socket =
            crate::test_support::scratch::temp_path(format!("ht-start-{}", uuid::Uuid::new_v4()));
        // Reserve one byte for the Unix address terminator.
        assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
        let listener = UnixListener::bind(&socket).unwrap();
        let worker = thread::spawn(move || {
            let (mut ping, _) = listener.accept().unwrap();
            let request = read(&mut ping);
            assert_eq!(request["method"], "ping");
            answer(
                &mut ping,
                &request,
                json!({"type":"pong","version":"0.9.1","protocol":22}),
            );
            drop(ping);
            let (mut operation, _) = listener.accept().unwrap();
            let request = read(&mut operation);
            response(&mut operation, request);
        });
        let cli = NativeCli::new(socket.clone(), Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        (socket, cli, worker)
    }
    fn launch_fixture() -> (NativeLaunchRequest, HostCallContext, HostObservation) {
        let target = HostTargetId::new("w4:p1");
        let boot = HostBootId::new("proven-boot");
        let terminal = TerminalId::new("term_1");
        let request = NativeLaunchRequest {
            process_hint: false,
            seat: SeatId::new("seat_1"),
            target: target.clone(),
            harness: Harness::Codex,
            argv: vec!["--no-daemon".into(), "--model".into(), "test model".into()],
            configured_hook: Some(ConfiguredHook {
                scope: "project".into(),
                path: "/tmp/hook".into(),
                fingerprint: "sha256:test".into(),
            }),
            expected_terminal: terminal.clone(),
            expected_generation: 7,
            expected_incarnation: "server-1".into(),
            name_hint: Some("Mad Tea-Hatter (codex)".into()),
        };
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: Some(boot.clone()),
            expected_epoch: Some(3),
        };
        let observation = HostObservation {
            focused: false,
            target,
            host_boot: boot,
            epoch: 3,
            generation: 7,
            observed_at_utc: UtcMillis(0),
            observed_at_mono: MonoInstant(1),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            terminal: Some(terminal),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: "server-1".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("call1"),
            connection_epoch: 3,
            observation_sequence: 1,
            started_at_mono: MonoInstant(0),
            completed_at_mono: MonoInstant(1),
        };
        (request, context, observation)
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) struct HintSocketFixture {
        cli: NativeCli,
        socket: PathBuf,
        stop: Arc<std::sync::atomic::AtomicBool>,
        frames: Arc<std::sync::Mutex<Vec<Value>>>,
        received: Arc<std::sync::atomic::AtomicUsize>,
        worker: Option<thread::JoinHandle<()>>,
        listener: Arc<std::sync::Mutex<Option<UnixListener>>>,
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    impl HintSocketFixture {
        pub(crate) fn new<F>(mut response: F) -> Self
        where
            F: FnMut(&mut UnixStream, &Value) + Send + 'static,
        {
            use std::io::{BufRead, BufReader};
            let socket =
                crate::test_support::scratch::temp_path(format!("hint-{}", uuid::Uuid::new_v4()));
            // Reserve one byte for the Unix address terminator.
            assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
            let listener = UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let listener = Arc::new(std::sync::Mutex::new(Some(listener)));
            let worker_listener = listener.clone();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let frames = Arc::new(std::sync::Mutex::new(Vec::new()));
            let received = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let worker_stop = stop.clone();
            let worker_frames = frames.clone();
            let worker_received = received.clone();
            let worker = thread::spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let connection = worker_listener
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(UnixListener::accept);
                    let Some(connection) = connection else {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    let (mut stream, _) = match connection {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("owned hint fixture accept: {error}"),
                    };
                    // macOS accept inherits the listener's O_NONBLOCK flag.
                    // Keep accept stop-aware, but bound each accepted read.
                    stream.set_nonblocking(false).unwrap();
                    if stream
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .is_err()
                        || stream
                            .set_write_timeout(Some(Duration::from_millis(100)))
                            .is_err()
                    {
                        continue;
                    }
                    let mut line = String::new();
                    let read = BufReader::new(&mut stream).read_line(&mut line);
                    worker_received.fetch_add(line.len(), Ordering::AcqRel);
                    match read {
                        Ok(0) => continue,
                        Ok(_) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(error) => panic!("owned hint fixture read: {error}"),
                    }
                    let wire: Value = serde_json::from_str(&line).unwrap();
                    worker_frames.lock().unwrap().push(wire.clone());
                    response(&mut stream, &wire);
                }
            });
            let cli = NativeCli::new(socket.clone(), Arc::new(TestClock(Instant::now())));
            cli.epoch.store(3, Ordering::Release);
            Self {
                cli,
                socket,
                stop,
                frames,
                received,
                worker: Some(worker),
                listener,
            }
        }

        pub(crate) fn endpoint(&self) -> &std::path::Path {
            &self.socket
        }

        pub(crate) fn rebind(&self) {
            let mut listener = self.listener.lock().unwrap();
            fs::remove_file(&self.socket).unwrap();
            let replacement = UnixListener::bind(&self.socket).unwrap();
            replacement.set_nonblocking(true).unwrap();
            *listener = Some(replacement);
        }

        pub(crate) fn stop_listening(&self) {
            self.listener.lock().unwrap().take();
        }

        fn bind_preflight(
            &self,
            request: &mut NativeLaunchRequest,
            context: &mut HostCallContext,
            observation: &mut HostObservation,
        ) {
            let (_, witness) = self
                .cli
                .run_witnessed(
                    &["pane", "get", request.target.as_str()],
                    &context.budget,
                    Duration::from_millis(750),
                )
                .unwrap();
            let incarnation = ServerIncarnation::from_witness(&witness.unwrap()).unwrap();
            request.expected_incarnation = incarnation.identity.clone();
            context.expected_boot = Some(incarnation.boot.clone());
            observation.host_boot = incarnation.boot;
            observation.incarnation = IncarnationEvidence::Verified {
                identity: incarnation.identity,
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            };
        }

        pub(crate) fn finish(self) -> Vec<Value> {
            self.finish_with_bytes().0
        }

        pub(crate) fn finish_with_bytes(mut self) -> (Vec<Value>, usize) {
            self.close();
            (
                self.frames.lock().unwrap().clone(),
                self.received.load(Ordering::Acquire),
            )
        }

        fn close(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let joined = worker.join();
                let removed = fs::remove_file(&self.socket);
                if !thread::panicking() {
                    joined.unwrap();
                    if let Err(error) = removed {
                        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
                    }
                }
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    impl Drop for HintSocketFixture {
        fn drop(&mut self) {
            self.close();
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_old_host_receives_zero_start_frames() {
        for capabilities in [
            None,
            Some(json!({})),
            Some(json!({"agent_start_process_hint_v1":false})),
            Some(json!({"agent_start_process_hint_v1":null})),
            Some(json!({"agent_start_process_hint_v1":"true"})),
            Some(json!({"agent_start_process_hint_v1":1})),
            Some(json!({"agent_start_process_hint":true})),
        ] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(true);
            request.harness = Harness::Agent(registry.agent("hinted").unwrap());
            request.process_hint = true;
            let name = request.agent_name();
            let mut fixture = HintSocketFixture::new(move |stream, wire| {
                let result = match wire["method"].as_str().unwrap() {
                    "ping" => {
                        let mut pong = json!({"type":"pong","version":"0.9.1","protocol":22});
                        if let Some(capabilities) = &capabilities {
                            pong["capabilities"] = capabilities.clone();
                        }
                        pong
                    }
                    "pane.get" => json!({"type":"pane_info"}),
                    "agent.start" => started(name.clone()),
                    other => panic!("unexpected fixture operation: {other}"),
                };
                answer(stream, wire, result);
            });
            fixture.cli.registry = registry;
            fixture.bind_preflight(&mut request, &mut context, &mut observation);
            let result = fixture
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = fixture.finish();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                0,
                "unsupported host received a start frame"
            );
            let failure =
                result.expect_err("missing feature capability must refuse before submission");
            assert_eq!(failure.error.code, ErrorCode::Unsupported);
            assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_default_route_omits_field() {
        let (mut request, mut context, mut observation) = launch_fixture();
        let registry = crate::harness::launch::tests::process_hint_registry(false);
        request.harness = Harness::Agent(registry.agent("hinted").unwrap());
        let name = request.agent_name();
        let mut fixture = HintSocketFixture::new(move |stream, wire| {
            let result = match wire["method"].as_str().unwrap() {
                "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                "pane.get" => json!({"type":"pane_info"}),
                "agent.start" => {
                    assert!(wire["params"].get("process_hint").is_none());
                    started(name.clone())
                }
                "pane.process_info" => process_info_fixture(),
                other => panic!("unexpected fixture operation: {other}"),
            };
            answer(stream, wire, result);
        });
        fixture.cli.registry = registry;
        fixture.bind_preflight(&mut request, &mut context, &mut observation);
        let result = fixture
            .cli
            .guarded_start_with_evidence(&request, &context, &observation);
        let frames = fixture.finish();
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["method"] == "agent.start")
                .count(),
            1
        );
        let NativeLaunchOutcome::ObservedStartup { correlation, .. } = result.unwrap() else {
            panic!("default startup not observed")
        };
        assert!(!correlation.process_hint);
        assert!(correlation.matches_request(&request, &context));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_policy_mode_mismatch_is_not_submitted() {
        for required in [false, true] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(required);
            request.harness = Harness::Agent(registry.agent("hinted").unwrap());
            request.process_hint = !required;
            let name = request.agent_name();
            let mut fixture = HintSocketFixture::new(move |stream, wire| {
                let result = match wire["method"].as_str().unwrap() {
                    "ping" => {
                        json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                    }
                    "pane.get" => json!({"type":"pane_info"}),
                    "agent.start" => started(name.clone()),
                    other => panic!("unexpected fixture operation: {other}"),
                };
                answer(stream, wire, result);
            });
            fixture.cli.registry = registry;
            fixture.bind_preflight(&mut request, &mut context, &mut observation);
            let result = fixture
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = fixture.finish();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                0,
                "policy/request mode mismatch reached the host"
            );
            let failure = result.expect_err("request mode must match the selected launch policy");
            assert_eq!(failure.error.code, ErrorCode::InvalidRequest);
            assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_actual_peer_must_match_preflight_before_start_write() {
        let (mut request, mut context, mut observation) = launch_fixture();
        let registry = crate::harness::launch::tests::process_hint_registry(true);
        request.harness = Harness::Agent(registry.agent("hinted").unwrap());
        request.process_hint = true;
        let name = request.agent_name();
        let mut fixture = HintSocketFixture::new(move |stream, wire| {
            let result = match wire["method"].as_str().unwrap() {
                "ping" => {
                    json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                }
                "pane.get" => json!({"type":"pane_info"}),
                "agent.start" => started(name.clone()),
                other => panic!("unexpected fixture operation: {other}"),
            };
            answer(stream, wire, result);
        });
        fixture.cli.registry = registry;
        fixture.bind_preflight(&mut request, &mut context, &mut observation);
        // Internally consistent recorded fences still cannot authorize a
        // different actual server peer on the submission stream.
        let different = HostBootId::new("different-recorded-server");
        request.expected_incarnation = different.as_str().to_owned();
        observation.host_boot = different.clone();
        observation.incarnation = IncarnationEvidence::Verified {
            identity: different.as_str().to_owned(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        };
        context.expected_boot = Some(different);
        let result = fixture
            .cli
            .guarded_start_with_evidence(&request, &context, &observation);
        let frames = fixture.finish();
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["method"] == "agent.start")
                .count(),
            0,
            "actual peer mismatch received a start frame"
        );
        let failure =
            result.expect_err("actual peer must match the recorded preflight before writing");
        assert_eq!(failure.error.code, ErrorCode::StaleHostObservation);
        assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_native_start_preserves_argv_and_ready_working_correlation() {
        for readiness in ["ready", "working", "pending"] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(true);
            request.harness = Harness::Agent(registry.agent("hinted").unwrap());
            request.process_hint = true;
            request.argv = vec!["space arg".into(), "apostrophe's arg".into()];
            let name = request.agent_name();
            let mut fixture = HintSocketFixture::new(move |stream, wire| {
                let mut ready = started(name.clone());
                ready["argv"] = json!(["codex", "space arg", "apostrophe's arg"]);
                let result = match wire["method"].as_str().unwrap() {
                    "ping" => {
                        json!({"type":"pong","version":"0.9.3","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                    }
                    "pane.get" => json!({"type":"pane_info"}),
                    "agent.start" => {
                        assert_eq!(wire["params"]["process_hint"], true);
                        assert_eq!(
                            wire["params"]["args"],
                            json!(["space arg", "apostrophe's arg"])
                        );
                        assert_eq!(wire["params"]["name"], name);
                        assert_eq!(wire["params"]["pane_id"], "w4:p1");
                        assert_eq!(wire["params"]["kind"], "codex");
                        assert!(wire["params"]["timeout_ms"].as_u64().unwrap() <= 10_000);
                        if readiness == "working" {
                            ready["agent"]["interactive_ready"] = json!(false);
                            ready["agent"]["agent_status"] = json!("working");
                            ready["agent"]["launch_pending"] = json!(true);
                        } else if readiness == "pending" {
                            ready["agent"].as_object_mut().unwrap().remove("agent");
                            ready["agent"]["interactive_ready"] = json!(false);
                            ready["agent"]["launch_pending"] = json!(true);
                        }
                        ready
                    }
                    "agent.get" => {
                        assert_eq!(wire["params"]["target"], name);
                        json!({"type":"agent_info","agent":ready["agent"]})
                    }
                    "pane.process_info" => process_info_fixture(),
                    other => panic!("unexpected fixture operation: {other}"),
                };
                answer(stream, wire, result);
            });
            fixture.cli.registry = registry;
            fixture.bind_preflight(&mut request, &mut context, &mut observation);
            let result = fixture
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = fixture.finish();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                1
            );
            let NativeLaunchOutcome::ObservedStartup { correlation, .. } = result.unwrap() else {
                panic!("startup was not correlated: {readiness}")
            };
            assert!(
                correlation.process_hint,
                "requested mode was lost in native startup correlation"
            );
            assert!(correlation.matches_request(&request, &context));
            assert_eq!(correlation.argv, ["space arg", "apostrophe's arg"]);
            // ht-54r.4: the placeholder fingerprint is the pane's single foreground process
            // (this test process in the fixture), read through host process info.
            let started = correlation
                .started_process
                .unwrap_or_else(|| panic!("started process ({readiness})"));
            assert_eq!(started.pid, std::process::id());
            assert_ne!(started.start, 0);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn task48_default_and_required_native_start_preserve_empty_correlation() {
        let mut observations = vec![];
        for required in [false, true] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(required);
            request.harness = Harness::Agent(registry.agent("hinted").unwrap());
            request.process_hint = required;
            request.argv = vec!["before".into(), "".into(), "after".into(), "".into()];
            let name = request.agent_name();
            let argv = request.argv.clone();
            let mut fixture = HintSocketFixture::new(move |stream, wire| {
                let response = match wire["method"].as_str().unwrap() {
                    "ping" => {
                        json!({"type":"pong","version":"0.9.3","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                    }
                    "pane.get" => json!({"type":"pane_info"}),
                    "agent.start" => {
                        assert_eq!(wire["params"]["args"], json!(argv));
                        assert_eq!(
                            wire["params"]["process_hint"],
                            if required { json!(true) } else { Value::Null }
                        );
                        let mut response = started(name.clone());
                        response["argv"] = json!([vec!["codex".to_owned()], argv.clone()].concat());
                        response
                    }
                    "pane.process_info" => process_info_fixture(),
                    other => panic!("unexpected Task48 synthetic socket operation: {other}"),
                };
                answer(stream, wire, response);
            });
            fixture.cli.registry = registry;
            if required {
                fixture.bind_preflight(&mut request, &mut context, &mut observation);
            }
            let result = fixture
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = fixture.finish();
            eprintln!(
                "task48 actual NativeCli required={required}: result={result:?} methods={:?}",
                frames
                    .iter()
                    .map(|frame| frame["method"].clone())
                    .collect::<Vec<_>>()
            );
            observations.push((request, context, result, frames));
        }
        // Both real consumers are reached at BASE before either expected-success assertion.
        for (request, context, result, frames) in observations {
            let NativeLaunchOutcome::ObservedStartup { correlation, .. } =
                result.expect("actual NativeCli must admit lossless bounded empty data")
            else {
                panic!("empty data startup was not correlated");
            };
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                1
            );
            assert_eq!(correlation.argv, request.argv);
            assert!(correlation.matches_request(&request, &context));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_retry_renegotiates_and_unknown_write_is_possible() {
        for case in [
            "lost-capability",
            "kept-capability",
            "unsupported",
            "lookalike",
            "malformed",
            "eof",
            "wrong-argv",
            "wrong-executable",
            "wrong-kind",
            "wrong-name",
            "wrong-terminal",
            "blocked",
        ] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(true);
            request.harness = Harness::Agent(registry.agent("hinted").unwrap());
            request.process_hint = true;
            let names = request.agent_name_candidates();
            let expected_names = names.clone();
            let mut pings = 0;
            let mut starts = 0;
            let mut fixture = HintSocketFixture::new(move |stream, wire| {
                match wire["method"].as_str().unwrap() {
                    "ping" => {
                        pings += 1;
                        let supported = !(case == "lost-capability" && pings == 3);
                        answer(
                            stream,
                            wire,
                            json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":supported}}),
                        );
                    }
                    "pane.get" => answer(stream, wire, json!({"type":"pane_info"})),
                    "agent.start" => {
                        assert_eq!(wire["params"]["process_hint"], true);
                        assert_eq!(wire["params"]["name"], names[starts]);
                        starts += 1;
                        if matches!(case, "lost-capability" | "kept-capability") && starts == 1 {
                            writeln!(stream, "{}", json!({"id":wire["id"],"error":{"code":"agent_name_taken","message":"taken before typing"}})).unwrap();
                            return;
                        }
                        if matches!(case, "unsupported" | "lookalike") {
                            let code = if case == "unsupported" {
                                "agent_process_hint_unsupported"
                            } else {
                                "agent_process_hint_unsupported_other"
                            };
                            writeln!(stream, "{}", json!({"id":wire["id"],"error":{"code":code,"message":"agent_process_hint_unsupported"}})).unwrap();
                            return;
                        }
                        if case == "malformed" {
                            writeln!(stream, "{{invalid").unwrap();
                            return;
                        }
                        if case == "eof" {
                            return;
                        }
                        let mut result = started(names[starts - 1].clone());
                        match case {
                            "wrong-argv" => {
                                result["argv"] = json!([
                                    "unexpected",
                                    "codex",
                                    "--no-daemon",
                                    "--model",
                                    "test model"
                                ])
                            }
                            "wrong-executable" => result["argv"][0] = json!("other"),
                            "wrong-kind" => {
                                result["agent"]["agent"] = json!("claude");
                                result["agent"]["agent_status"] = json!("blocked");
                            }
                            "wrong-name" => result["agent"]["name"] = json!("other-name"),
                            "wrong-terminal" => {
                                result["agent"]["terminal_id"] = json!("other-terminal")
                            }
                            "blocked" => result["agent"]["agent_status"] = json!("blocked"),
                            _ => {}
                        }
                        answer(stream, wire, result);
                    }
                    "pane.process_info" => answer(stream, wire, process_info_fixture()),
                    other => panic!("unexpected fixture operation: {other}"),
                }
            });
            fixture.cli.registry = registry;
            fixture.bind_preflight(&mut request, &mut context, &mut observation);
            let result = fixture
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = fixture.finish();
            assert_eq!(
                frames
                    .iter()
                    .filter(|frame| frame["method"] == "agent.start")
                    .count(),
                if case == "kept-capability" { 2 } else { 1 },
                "{case} retried without confirmed name refusal"
            );
            if matches!(case, "lost-capability" | "unsupported") {
                let failure = result.expect_err(case);
                assert_eq!(failure.error.code, ErrorCode::Unsupported, "{case}");
                assert_eq!(
                    failure.submission,
                    ports::NativeSubmission::NotSubmitted,
                    "{case}"
                );
            } else if case == "kept-capability" {
                let NativeLaunchOutcome::ObservedStartup { correlation, .. } = result.unwrap()
                else {
                    panic!("confirmed retry was not correlated")
                };
                assert_eq!(correlation.agent_name, expected_names[1]);
                assert!(correlation.matches_request(&request, &context));
            } else {
                assert_eq!(
                    result.unwrap(),
                    NativeLaunchOutcome::OutcomeUnknown,
                    "{case} fabricated accepted startup"
                );
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn process_hint_invalid_native_argv_is_not_submitted_in_either_mode() {
        for required in [false, true] {
            for argv in [
                vec!["a\0b".into()],
                vec!["a\nb".into()],
                vec!["a\rb".into()],
                vec!["arg".into(); 65],
                vec!["x".repeat(NativeLaunchRequest::MAX_ARG_BYTES + 1)],
                vec![
                    "x".repeat(NativeLaunchRequest::MAX_ARG_BYTES),
                    "x".repeat(NativeLaunchRequest::MAX_ARG_BYTES),
                    "x".into(),
                ],
            ] {
                let (mut request, mut context, mut observation) = launch_fixture();
                let registry = crate::harness::launch::tests::process_hint_registry(required);
                request.harness = Harness::Agent(registry.agent("hinted").unwrap());
                request.process_hint = required;
                let mut fixture = HintSocketFixture::new(|stream, wire| {
                    let result = if wire["method"] == "ping" {
                        json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                    } else {
                        json!({"type":"pane_info"})
                    };
                    answer(stream, wire, result);
                });
                fixture.cli.registry = registry;
                fixture.bind_preflight(&mut request, &mut context, &mut observation);
                request.argv = argv;
                let result =
                    fixture
                        .cli
                        .guarded_start_with_evidence(&request, &context, &observation);
                let frames = fixture.finish();
                assert_eq!(
                    frames
                        .iter()
                        .filter(|frame| frame["method"] == "agent.start")
                        .count(),
                    0
                );
                let failure = result.unwrap_err();
                assert_eq!(failure.error.code, ErrorCode::InvalidRequest);
                assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
            }
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn task48_empty_return_mismatches_and_prewrite_internal_mode_budget_fences() {
        for required in [false, true] {
            for case in [
                "missing-argv",
                "drop-empty",
                "reorder-empty",
                "wrong-name",
                "wrong-terminal",
                "internal",
                "mode",
                "cancelled",
                "expired",
                "generation",
                "incarnation",
            ] {
                let (mut request, mut context, mut observation) = launch_fixture();
                let registry = crate::harness::launch::tests::process_hint_registry(required);
                request.harness = Harness::Agent(registry.agent("hinted").unwrap());
                request.process_hint = required;
                request.argv = vec!["before".into(), "".into(), "after".into()];
                let name = request.agent_name();
                let mut fixture = HintSocketFixture::new(move |stream, wire| {
                    let response = match wire["method"].as_str().unwrap() {
                        "ping" => {
                            json!({"type":"pong","version":"0.9.3","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}})
                        }
                        "pane.get" => json!({"type":"pane_info"}),
                        "agent.start" => {
                            let mut response = started(name.clone());
                            response["argv"] = json!(["codex", "before", "", "after"]);
                            match case {
                                "missing-argv" => {
                                    response.as_object_mut().unwrap().remove("argv");
                                }
                                "drop-empty" => {
                                    response["argv"] = json!(["codex", "before", "after"])
                                }
                                "reorder-empty" => {
                                    response["argv"] = json!(["codex", "", "before", "after"])
                                }
                                "wrong-name" => response["agent"]["name"] = json!("foreign"),
                                "wrong-terminal" => {
                                    response["agent"]["terminal_id"] = json!("foreign")
                                }
                                _ => panic!("prewrite-invalid case submitted agent.start: {case}"),
                            }
                            response
                        }
                        other => panic!("unexpected Task48 correlation operation: {other}"),
                    };
                    answer(stream, wire, response);
                });
                fixture.cli.registry = registry;
                if required {
                    fixture.bind_preflight(&mut request, &mut context, &mut observation);
                }
                let prewrite = matches!(
                    case,
                    "internal" | "mode" | "cancelled" | "expired" | "generation" | "incarnation"
                );
                match case {
                    "internal" => request
                        .configured_hook
                        .as_mut()
                        .unwrap()
                        .fingerprint
                        .clear(),
                    "mode" => request.process_hint = !required,
                    "cancelled" => context.budget.cancellation.cancel(),
                    "expired" => context.budget.deadline = MonoInstant(0),
                    "generation" => request.expected_generation += 1,
                    "incarnation" => request.expected_incarnation.push_str("-changed"),
                    _ => {}
                }
                let result =
                    fixture
                        .cli
                        .guarded_start_with_evidence(&request, &context, &observation);
                let frames = fixture.finish();
                assert!(
                    !matches!(result, Ok(NativeLaunchOutcome::ObservedStartup { .. })),
                    "{case} required={required} fabricated startup"
                );
                if prewrite {
                    assert_eq!(
                        frames
                            .iter()
                            .filter(|frame| frame["method"] == "agent.start")
                            .count(),
                        0,
                        "{case} required={required}"
                    );
                    assert_eq!(
                        result.unwrap_err().submission,
                        ports::NativeSubmission::NotSubmitted
                    );
                } else {
                    assert_eq!(
                        frames
                            .iter()
                            .filter(|frame| frame["method"] == "agent.start")
                            .count(),
                        1
                    );
                }
            }
        }
    }

    /// Herdr's `pane.process_info` for a pane whose shell runs one foreground process: this test
    /// process, so host process info can read its start stamp.
    fn process_info_fixture() -> Value {
        json!({"type":"pane_process_info","process_info":{
            "pane_id":"pane","shell_pid":1,
            "foreground_processes":[{"pid":std::process::id(),"name":"codex","argv0":"codex"}]
        }})
    }

    fn started(name: String) -> Value {
        // The live Herdr 0.9.1 `agent.start` success shape (captured
        // 2026-09-30 with a stand-in executable): `agent` names the detected
        // kind and there is no `launch_pending` field.
        json!({"type":"agent_started","argv":["codex","--no-daemon","--model","test model"],
            "agent":{"agent":"codex","agent_status":"idle","cwd":"/tmp","focused":false,
                "foreground_cwd":"/tmp","interactive_ready":true,"name":name,"pane_id":"w4:p1",
                "revision":0,"state_change_seq":504,"tab_id":"w4:t1","terminal_id":"term_1",
                "workspace_id":"w4"}})
    }
    fn pane_agent_context() -> HostCallContext {
        HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        }
    }
    /// One `agent.get` exchange answered with `reply` (its id is filled in).
    fn pane_agent_read(reply: Value) -> Result<Option<ports::PaneAgentObservation>, ApiError> {
        let (socket, cli, worker) = fixture(move |stream, wire| {
            assert_eq!(wire["method"], "agent.get");
            assert_eq!(wire["params"], json!({"target":"w4:p1"}));
            let mut reply = reply;
            reply["id"] = wire["id"].clone();
            writeln!(stream, "{reply}").unwrap();
        });
        let result = cli.observe_pane_agent(&HostTargetId::new("w4:p1"), &pane_agent_context());
        worker.join().unwrap();
        let _ = fs::remove_file(socket);
        result
    }
    fn agent_info(session: Option<&str>) -> Value {
        let mut agent = json!({"agent":"claude","agent_status":"idle","pane_id":"w4:p1",
            "terminal_id":"term_1"});
        if let Some(value) = session {
            agent["agent_session"] =
                json!({"agent":"claude","kind":"id","source":"herdr:claude","value":value});
        }
        json!({"result":{"type":"agent_info","agent":agent}})
    }
    #[test]
    fn pane_agent_maps_agent_get_with_session() {
        assert_eq!(
            pane_agent_read(agent_info(Some("sess-1"))).unwrap(),
            Some(ports::PaneAgentObservation {
                kind: Some("claude".into()),
                agent_session: Some("sess-1".into()),
            })
        );
    }
    #[test]
    fn pane_agent_maps_agent_get_without_session() {
        assert_eq!(
            pane_agent_read(agent_info(None)).unwrap(),
            Some(ports::PaneAgentObservation {
                kind: Some("claude".into()),
                agent_session: None,
            })
        );
    }
    #[test]
    fn pane_agent_not_found_is_absent() {
        let reply = json!({"error":{"code":"agent_not_found","message":"no agent"}});
        assert_eq!(pane_agent_read(reply).unwrap(), None);
    }
    #[test]
    fn pane_agent_other_error_is_read_error() {
        let reply = json!({"error":{"code":"permission_denied","message":"denied"}});
        assert_eq!(
            pane_agent_read(reply).unwrap_err().code,
            ErrorCode::Unauthorized
        );
    }
    /// After a diagnostic read, a fenced call carrying the pre-read epoch is
    /// still admitted: the read did not invalidate the connection.
    fn assert_epoch_untouched(cli: &NativeCli, before: u64) {
        assert_eq!(cli.epoch(), before);
        let context = HostCallContext {
            expected_boot: Some(HostBootId::new("proven-boot")),
            expected_epoch: Some(before),
            ..pane_agent_context()
        };
        cli.check_context(&context)
            .expect("fenced call must not see 'host context changed'");
    }
    #[test]
    fn pane_agent_failure_never_bumps_the_connection_epoch() {
        let limit = Duration::from_millis(150);
        let target = HostTargetId::new("w4:p1");
        // Timeout: the fixture accepts the read and never replies in time.
        let (socket, cli, worker) = fixture(|stream, _| {
            thread::sleep(Duration::from_millis(600));
            let _ = stream.flush();
        });
        let before = cli.epoch();
        let result = cli.pane_agent_within(&target, &pane_agent_context(), limit);
        assert!(result.is_err(), "a read past its limit must fail");
        assert_epoch_untouched(&cli, before);
        worker.join().unwrap();
        let _ = fs::remove_file(socket);
        // HostUnavailable: no listener at the socket path.
        let missing =
            crate::test_support::scratch::temp_path(format!("ht-absent-{}", uuid::Uuid::new_v4()));
        let cli = NativeCli::new(missing, Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        let failure = cli
            .pane_agent_within(&target, &pane_agent_context(), limit)
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::HostUnavailable);
        assert_epoch_untouched(&cli, 3);
        // Parse-time overrun: the reply arrived after the limit.
        let cli = NativeCli::new(
            crate::test_support::scratch::temp_path("unused"),
            Arc::new(TestClock(Instant::now())),
        );
        cli.epoch.store(3, Ordering::Release);
        let late = Instant::now() - Duration::from_millis(500);
        let failure = cli
            .check_after_parse_unfenced(&pane_agent_context().budget, late, limit)
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::DeadlineExceeded);
        assert_epoch_untouched(&cli, 3);
        // The fenced variant still bumps, so the unfenced one is the difference.
        assert!(
            cli.check_after_parse(&pane_agent_context().budget, late, limit)
                .is_err()
        );
        assert_eq!(cli.epoch(), 4);
    }
    fn absent_cli() -> NativeCli {
        let missing =
            crate::test_support::scratch::temp_path(format!("ht-absent-{}", uuid::Uuid::new_v4()));
        let cli = NativeCli::new(missing, Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        cli
    }
    fn composer_target() -> SafeWakeTarget {
        SafeWakeTarget {
            seat: SeatId::new("seat_1"),
            target: HostTargetId::new("w4:p1"),
            host_boot: HostBootId::new("proven-boot"),
            generation: 7,
            terminal: TerminalId::new("term_1"),
            incarnation: "server-1".into(),
            basis: ports::WakeTargetBasis::CooperativeAgent,
            epoch: 3,
            observation_sequence: 1,
            bound_harness: Some("claude".into()),
        }
    }
    fn composer_context(clock: &TestClock) -> HostCallContext {
        HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 700),
                cancellation: Cancellation::default(),
            },
            expected_boot: Some(HostBootId::new("proven-boot")),
            expected_epoch: Some(3),
        }
    }
    #[test]
    fn composer_read_failure_never_bumps_the_connection_epoch() {
        let target = composer_target();
        // Timeout: the read limit is about 450 ms and the fixture answers at 600 ms.
        let (socket, cli, worker) = fixture(|stream, _| {
            thread::sleep(Duration::from_millis(600));
            let _ = stream.flush();
        });
        let clock = TestClock(Instant::now());
        let state = cli
            .pane_agent_state(&target, &composer_context(&clock))
            .unwrap();
        assert_eq!(state, ports::AgentComposerState::Unknown);
        assert_epoch_untouched(&cli, 3);
        worker.join().unwrap();
        let _ = fs::remove_file(socket);
        // HostUnavailable: no listener at the socket path.
        let cli = absent_cli();
        let state = cli
            .pane_agent_state(&target, &composer_context(&clock))
            .unwrap();
        assert_eq!(state, ports::AgentComposerState::Unknown);
        assert_epoch_untouched(&cli, 3);
    }
    #[test]
    fn early_exit_read_failure_never_bumps_the_connection_epoch() {
        let (request, context, _) = launch_fixture();
        // Timeout: the fixture holds the read open until the test drops `release`.
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let (socket, cli, worker) = fixture(move |_stream, _| {
            let _ = hold.recv();
        });
        assert_eq!(
            cli.early_exit_output(&request, "codex", &context.budget),
            None
        );
        assert_epoch_untouched(&cli, 3);
        drop(release);
        worker.join().unwrap();
        let _ = fs::remove_file(socket);
        // HostUnavailable: no listener at the socket path.
        let cli = absent_cli();
        assert_eq!(
            cli.early_exit_output(&request, "codex", &context.budget),
            None
        );
        assert_epoch_untouched(&cli, 3);
    }
    #[test]
    fn current_caller_uses_explicit_inherited_id_and_returns_live_moved_target() {
        let (socket, cli, worker) = fixture(|stream, request| {
            assert_eq!(request["method"], "pane.current");
            assert_eq!(request["params"], json!({"caller_pane_id":"w1:p1"}));
            answer(
                stream,
                &request,
                json!({"type":"pane_current", "pane":{
                    "pane_id":"w2:p3", "tab_id":"w2:t1", "workspace_id":"w2", "focused":false
                }}),
            );
        });
        assert_eq!(
            cli.current_pane("w1:p1", &pane_agent_context().budget)
                .unwrap()
                .as_str(),
            "w2:p3"
        );
        assert_epoch_untouched(&cli, 3);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    #[test]
    fn pane_names_failure_never_bumps_the_connection_epoch() {
        let cli = absent_cli();
        let failure = cli.pane_names(&pane_agent_context().budget).unwrap_err();
        assert_eq!(failure.code, ErrorCode::HostUnavailable);
        assert_epoch_untouched(&cli, 3);
    }
    #[test]
    fn composer_settle_returns_unknown_when_cancelled() {
        let cli = absent_cli();
        let clock = TestClock(Instant::now());
        let context = composer_context(&clock);
        let cancel = context.budget.cancellation.clone();
        let canceller = thread::spawn(move || {
            // A wait on an unrelated, never-cancelled token is a plain 20 ms pause.
            Cancellation::default().wait_blocking(Duration::from_millis(20));
            cancel.cancel();
        });
        let started = Instant::now();
        let state = cli.pane_agent_state(&composer_target(), &context).unwrap();
        // No listener exists: a read after the settle would be a host error,
        // but the cancelled settle returns before any request is made.
        assert_eq!(state, ports::AgentComposerState::Unknown);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "settle wait ignored cancellation: {:?}",
            started.elapsed()
        );
        canceller.join().unwrap();
        assert_epoch_untouched(&cli, 3);
    }
    #[test]
    fn guarded_start_correlates_exact_direct_request_without_claiming_execution() {
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        // The pane label, fitted to Herdr's name rule, not a seat hash.
        assert_eq!(name, "mad-tea-hatter-codex");
        let (socket, cli, worker) = fixture(move |stream, wire| {
            assert_eq!(wire["method"], "agent.start");
            assert_eq!(wire["params"]["name"], name);
            assert_eq!(wire["params"]["kind"], "codex");
            assert_eq!(wire["params"]["pane_id"], "w4:p1");
            assert_eq!(
                wire["params"]["args"],
                json!(["--no-daemon", "--model", "test model"])
            );
            answer(stream, &wire, started(name));
        });
        let result = cli
            .guarded_start_cli(&request, &context, &observation)
            .unwrap();
        match result {
            NativeLaunchOutcome::ObservedStartup {
                correlation,
                diagnostic,
            } => {
                assert!(correlation.matches_request(&request, &context));
                assert_eq!(correlation.agent_name, "mad-tea-hatter-codex");
                assert_eq!(diagnostic.execution, ExecutionEvidence::Unknown);
                assert_eq!(diagnostic.incarnation, IncarnationEvidence::Unknown);
            }
            NativeLaunchOutcome::OutcomeUnknown => panic!("matching ready response lost"),
        }
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }
    #[test]
    fn process_hint_exact_unsupported_response_is_not_submitted() {
        let (request, context, observation) = launch_fixture();
        let (socket, cli, worker) = fixture(|stream, wire| {
            assert_eq!(wire["method"], "agent.start");
            writeln!(
                stream,
                "{}",
                json!({"id":wire["id"],"error":{
                    "code":"agent_process_hint_unsupported","message":"unsupported shell"
                }})
            )
            .unwrap();
        });
        let result = cli.guarded_start_with_evidence(&request, &context, &observation);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        let failure = result.expect_err("exact unsupported start is a confirmed refusal");
        assert_eq!(failure.error.code, ErrorCode::Unsupported);
        assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
    }

    #[test]
    fn process_hint_ordinary_run_keeps_structured_start_error_behavior() {
        for code in [
            "agent_pane_busy",
            "agent_name_taken",
            "agent_process_hint_unsupported",
        ] {
            let (socket, cli, worker) = fixture(move |stream, wire| {
                writeln!(
                    stream,
                    "{}",
                    json!({"id":wire["id"],"error":{"code":code,"message":"confirmed refusal"}})
                )
                .unwrap();
            });
            let budget = pane_agent_context().budget;
            let result = cli.run(
                &[
                    "agent",
                    "start",
                    "worker",
                    "--kind",
                    "codex",
                    "--pane",
                    "w4:p1",
                    "--timeout",
                    "10000",
                    "--",
                ],
                &budget,
                Duration::from_secs(1),
            );
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
            assert_eq!(
                result
                    .expect_err("ordinary run must retain mapped structured errors")
                    .code,
                if code == "agent_process_hint_unsupported" {
                    ErrorCode::Unsupported
                } else {
                    ErrorCode::TargetUnsafe
                }
            );
        }
    }
    type Exchange = Box<dyn FnOnce(&mut UnixStream, Value) + Send>;
    /// Serves one ping+operation exchange per entry of `responses`.
    fn serve_sequence(responses: Vec<Exchange>) -> (PathBuf, NativeCli, thread::JoinHandle<()>) {
        let socket =
            crate::test_support::scratch::temp_path(format!("ht-start-{}", uuid::Uuid::new_v4()));
        // Reserve one byte for the Unix address terminator.
        assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
        let listener = UnixListener::bind(&socket).unwrap();
        let worker = thread::spawn(move || {
            for response in responses {
                let (mut ping, _) = listener.accept().unwrap();
                let request = read(&mut ping);
                answer(
                    &mut ping,
                    &request,
                    json!({"type":"pong","version":"0.9.1","protocol":22}),
                );
                drop(ping);
                let (mut operation, _) = listener.accept().unwrap();
                let request = read(&mut operation);
                response(&mut operation, request);
            }
        });
        let cli = NativeCli::new(socket.clone(), Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        (socket, cli, worker)
    }

    /// Herdr 0.9.1's socket `agent.start` answers `launch_pending: true`
    /// before readiness (live capture 2026-09-30); readiness is then read
    /// from `agent.get` on the adapter-selected name. Kills: accepting the
    /// pending answer as startup, and never observing readiness (every real
    /// launch would stay OutcomeUnknown).
    #[test]
    fn guarded_start_polls_named_agent_until_interactive_ready() {
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        let pending_name = name.clone();
        let ready_name = name.clone();
        let (socket, cli, worker) = serve_sequence(vec![
            Box::new(move |stream, wire| {
                assert_eq!(wire["method"], "agent.start");
                let mut result = started(pending_name);
                result["agent"] = json!({"agent_status":"unknown","launch_pending":true,
                    "name":result["agent"]["name"],"pane_id":"w4:p1","terminal_id":"term_1",
                    "focused":false,"revision":0,"tab_id":"w4:t1","workspace_id":"w4"});
                answer(stream, &wire, result);
            }),
            Box::new(move |stream, wire| {
                assert_eq!(wire["method"], "agent.get");
                assert_eq!(wire["params"], json!({"target": ready_name}));
                let agent = started(ready_name)["agent"].clone();
                answer(stream, &wire, json!({"type":"agent_info","agent":agent}));
            }),
        ]);
        match cli
            .guarded_start_cli(&request, &context, &observation)
            .unwrap()
        {
            NativeLaunchOutcome::ObservedStartup { correlation, .. } => {
                assert!(correlation.matches_request(&request, &context));
            }
            NativeLaunchOutcome::OutcomeUnknown => panic!("readiness was not observed"),
        }
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();

        // An agent given an initial prompt is working before it ever reports
        // interactive_ready: that is a confirmed start, not an unknown outcome.
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        let (socket, cli, worker) = serve_sequence(vec![Box::new(move |stream, wire| {
            let mut result = started(name);
            result["agent"]["interactive_ready"] = json!(false);
            result["agent"]["agent_status"] = json!("working");
            result["agent"]["launch_pending"] = json!(true);
            answer(stream, &wire, result);
        })]);
        match cli
            .guarded_start_cli(&request, &context, &observation)
            .unwrap()
        {
            NativeLaunchOutcome::ObservedStartup { .. } => {}
            NativeLaunchOutcome::OutcomeUnknown => panic!("a working agent is a confirmed start"),
        }
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();

        // Blocked during startup is started-but-not-ready: unknown, no answer.
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        let (socket, cli, worker) = serve_sequence(vec![Box::new(move |stream, wire| {
            let mut result = started(name);
            result["agent"]["interactive_ready"] = json!(false);
            result["agent"]["agent_status"] = json!("blocked");
            answer(stream, &wire, result);
        })]);
        assert_eq!(
            cli.guarded_start_cli(&request, &context, &observation)
                .unwrap(),
            NativeLaunchOutcome::OutcomeUnknown
        );
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    #[test]
    fn guarded_start_mismatched_or_unready_response_is_unknown() {
        for field in [
            "name",
            "agent",
            "pane_id",
            "terminal_id",
            "interactive_ready",
            "launch_pending",
            "argv",
        ] {
            let (request, context, observation) = launch_fixture();
            let name = request.agent_name();
            let (socket, cli, worker) = fixture(move |stream, wire| {
                let mut result = started(name);
                if field == "argv" {
                    result["argv"] = json!(["codex", "--other"]);
                } else if field == "interactive_ready" {
                    result["agent"][field] = json!(false);
                } else if field == "launch_pending" {
                    result["agent"][field] = json!(true);
                } else {
                    result["agent"][field] = json!("wrong");
                }
                answer(stream, &wire, result);
            });
            assert_eq!(
                cli.guarded_start_cli(&request, &context, &observation)
                    .unwrap(),
                NativeLaunchOutcome::OutcomeUnknown
            );
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
        }
    }
    fn refuse(stream: &mut UnixStream, wire: &Value, code: &str) {
        let refusal = json!({"id":wire["id"],"error":{"code":code,"message":"fixture refusal"}});
        writeln!(stream, "{refusal}").unwrap();
    }

    #[test]
    fn guarded_start_retains_only_busy_as_confirmed_prestart_refusal() {
        for (code, busy) in [
            ("agent_pane_busy", true),
            ("agent_name_taken", true),
            ("agent_blocked", false),
            ("agent_not_ready", false),
        ] {
            let (request, mut context, observation) = launch_fixture();
            // A taken name is retried exactly once; a second refusal stands.
            let attempts = if code == "agent_name_taken" { 2 } else { 1 };
            let exchanges: Vec<Exchange> = (0..attempts)
                .map(|_| -> Exchange {
                    Box::new(move |stream: &mut UnixStream, wire: Value| {
                        assert_eq!(wire["method"], "agent.start");
                        refuse(stream, &wire, code);
                    })
                })
                .collect();
            let (socket, mut cli, worker) = serve_sequence(exchanges);
            // Exactly the minimum startup allowance plus one retry interval:
            // there is no spare caller budget for a readiness retry.
            cli.clock = Arc::new(FrozenClock);
            context.budget.deadline = MonoInstant(3_100);
            let result = cli.guarded_start_cli(&request, &context, &observation);
            if busy {
                assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
            } else {
                assert_eq!(result.unwrap(), NativeLaunchOutcome::OutcomeUnknown);
            }
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
        }
    }

    /// Herdr agent names match `[a-z][a-z0-9_-]{0,31}`.
    fn herdr_name_ok(name: &str) -> bool {
        let bytes = name.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= 32
            && bytes[0].is_ascii_lowercase()
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
    }

    #[test]
    fn launch_agent_name_is_sanitized_to_herdr_rule() {
        use crate::ports::sanitize_agent_name;
        for (raw, expected) in [
            ("mad-tea-hatter-codex", Some("mad-tea-hatter-codex")),
            ("Mad Tea-Hatter (codex)", Some("mad-tea-hatter-codex")),
            ("  42 reviewers!", Some("reviewers")),
            ("snake_case__ok", Some("snake_case__ok")),
            ("café/agent", Some("caf-agent")),
            ("a--b..c", Some("a-b-c")),
            ("trailing_-", Some("trailing")),
            ("123", None),
            ("", None),
            ("é_ü", None),
        ] {
            assert_eq!(sanitize_agent_name(raw).as_deref(), expected, "{raw:?}");
        }
        let long = sanitize_agent_name(&"x".repeat(100)).unwrap();
        assert_eq!(long.len(), 32);
        // A cut never leaves a trailing separator.
        let cut = sanitize_agent_name(&format!("{}-tail", "a".repeat(31))).unwrap();
        assert_eq!(cut, "a".repeat(31));
        for raw in ["Mad Tea-Hatter (codex)", "x".repeat(80).as_str(), "Z!9"] {
            assert!(herdr_name_ok(&sanitize_agent_name(raw).unwrap()), "{raw:?}");
        }
    }

    #[test]
    fn launch_agent_name_falls_back_to_short_seat_id() {
        let (mut request, _, _) = launch_fixture();
        for hint in [None, Some("!!!".to_owned()), Some("".to_owned())] {
            request.name_hint = hint;
            request.seat = SeatId::new("sQ7ak3Fq9a2B");
            assert_eq!(request.agent_name(), "sk3fq9a2b");
            // The suffix never repeats the id the name already ends with.
            let retry = &request.agent_name_candidates()[1];
            assert!(
                retry.starts_with("sk3fq9a2b-") && retry.len() == 16,
                "{retry}"
            );
            assert!(herdr_name_ok(retry), "{retry}");
        }
        // Legacy UUID seats stay within Herdr's 32-byte limit.
        request.name_hint = None;
        request.seat = SeatId::new("seat-bc121d12-9d30-4510-b93c-e7cd26e890f1");
        assert_eq!(request.agent_name(), "seat-bc121d12");
        // A seat id with nothing alphanumeric after its prefix uses a digest.
        request.seat = SeatId::new("seat-");
        assert!(herdr_name_ok(&request.agent_name()));
        for name in request.agent_name_candidates() {
            assert!(herdr_name_ok(&name), "{name}");
        }
    }

    #[test]
    fn launch_agent_name_retry_fits_a_full_length_name() {
        let (mut request, _, _) = launch_fixture();
        request.seat = SeatId::new("sQ7ak3Fq9a2B");
        request.name_hint = Some("a".repeat(40));
        let [first, retry] = request.agent_name_candidates();
        assert_eq!(first, "a".repeat(32));
        assert_eq!(retry, format!("{}-k3fq9a2b", "a".repeat(23)));
        assert!(herdr_name_ok(&retry));
        assert_ne!(first, retry);
    }

    /// A readable name another live agent already holds: Herdr refuses it
    /// before typing anything, launch retries once with the seat suffix and
    /// correlates the startup with exactly that second name.
    #[test]
    fn guarded_start_retries_taken_name_once_with_seat_suffix() {
        let (request, context, observation) = launch_fixture();
        let [first, retry] = request.agent_name_candidates();
        assert_eq!(first, "mad-tea-hatter-codex");
        assert_eq!(retry, "mad-tea-hatter-codex-1");
        let (taken, retried) = (first.clone(), retry.clone());
        let (socket, cli, worker) = serve_sequence(vec![
            Box::new(move |stream, wire| {
                assert_eq!(wire["method"], "agent.start");
                assert_eq!(wire["params"]["name"], taken);
                refuse(stream, &wire, "agent_name_taken");
            }),
            Box::new(move |stream, wire| {
                assert_eq!(wire["method"], "agent.start");
                assert_eq!(wire["params"]["name"], retried);
                answer(stream, &wire, started(retried));
            }),
        ]);
        match cli
            .guarded_start_cli(&request, &context, &observation)
            .unwrap()
        {
            NativeLaunchOutcome::ObservedStartup { correlation, .. } => {
                assert_eq!(correlation.agent_name, retry);
                assert!(correlation.matches_request(&request, &context));
            }
            NativeLaunchOutcome::OutcomeUnknown => panic!("retried start lost"),
        }
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();

        // The retry's response must still name the retried agent exactly:
        // an answer for the first (taken) name is not this launch.
        let (request, context, observation) = launch_fixture();
        let [first, _] = request.agent_name_candidates();
        let (socket, cli, worker) = serve_sequence(vec![
            Box::new(move |stream, wire| refuse(stream, &wire, "agent_name_taken")),
            Box::new(move |stream, wire| answer(stream, &wire, started(first))),
        ]);
        assert_eq!(
            cli.guarded_start_cli(&request, &context, &observation)
                .unwrap(),
            NativeLaunchOutcome::OutcomeUnknown
        );
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    #[test]
    fn guarded_start_rejects_unverified_changed_or_busy_preflight_without_dispatch() {
        for change in [
            "terminal",
            "generation",
            "incarnation",
            "occupied",
            "ui",
            "epoch",
        ] {
            let socket = crate::test_support::scratch::temp_path(format!(
                "ht-no-start-{}",
                uuid::Uuid::new_v4()
            ));
            let listener = UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let cli = NativeCli::new(socket.clone(), Arc::new(TestClock(Instant::now())));
            cli.epoch.store(3, Ordering::Release);
            let (request, context, mut observation) = launch_fixture();
            match change {
                "terminal" => observation.terminal = Some(TerminalId::new("other")),
                "generation" => observation.generation += 1,
                "incarnation" => observation.incarnation = IncarnationEvidence::Unknown,
                "occupied" => observation.occupancy = StructuralOccupancy::Occupied,
                "ui" => observation.ui = HostUiState::HumanInput,
                _ => observation.epoch += 1,
            }
            assert!(
                cli.guarded_start_cli(&request, &context, &observation)
                    .is_err()
            );
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
            drop(listener);
            fs::remove_file(socket).unwrap();
        }
    }

    #[test]
    fn guarded_start_cancelled_after_request_remains_unknown_and_closes_socket() {
        let (request, context, observation) = launch_fixture();
        let cancel = context.budget.cancellation.clone();
        let (socket, cli, worker) = fixture(move |stream, _| {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            cancel.cancel();
            let mut byte = [0];
            assert_eq!(std::io::Read::read(stream, &mut byte).unwrap(), 0);
        });
        assert_eq!(
            cli.guarded_start_cli(&request, &context, &observation)
                .unwrap(),
            NativeLaunchOutcome::OutcomeUnknown
        );
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    fn wake_agent(status: &str, kind: Option<&str>, terminal: &str) -> Value {
        let mut agent = json!({"agent_status":status,"pane_id":"w4:p1","terminal_id":terminal,
            "tab_id":"w4:t1","workspace_id":"w4","focused":false,"revision":2});
        if let Some(kind) = kind {
            agent["agent"] = json!(kind);
        }
        agent
    }
    /// A `pane.get` whose agent status is not one the composer read applies
    /// to, so the observation makes no further host call; the recheck that
    /// matters to these wake tests is the `agent.get` before the prompt.
    fn pane_exchange() -> Exchange {
        pane_exchange_with("unknown")
    }
    fn pane_exchange_with(status: &'static str) -> Exchange {
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.get");
            answer(
                stream,
                &request,
                json!({"type":"pane_info","pane":{"pane_id":"w4:p1","terminal_id":"term_1",
                    "workspace_id":"w4","tab_id":"w4:t1","focused":false,
                    "agent_status":status,"agent":"claude","revision":2}}),
            );
        })
    }
    #[test]
    fn observation_carries_the_panes_focus_flag() {
        for focused in [true, false] {
            let exchange: Exchange = Box::new(move |stream: &mut UnixStream, request: Value| {
                assert_eq!(request["method"], "pane.get");
                answer(
                    stream,
                    &request,
                    json!({"type":"pane_info","pane":{"pane_id":"w4:p1","terminal_id":"term_1",
                        "workspace_id":"w4","tab_id":"w4:t1","focused":focused,
                        "agent_status":"unknown","agent":"claude","revision":2}}),
                );
            });
            let (socket, cli, worker) = serve_sequence(vec![exchange]);
            let observation = cli
                .observe_current_target_for_poke(&HostTargetId::new("w4:p1"), &pane_agent_context())
                .unwrap();
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
            assert_eq!(observation.focused, focused);
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn recheck_exchange(agent: Value) -> Exchange {
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.get");
            assert_eq!(request["params"], json!({"target":"w4:p1"}));
            answer(stream, &request, json!({"type":"agent_info","agent":agent}));
        })
    }
    fn wake_context(observation: &HostObservation) -> HostCallContext {
        HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: Some(observation.host_boot.clone()),
            expected_epoch: Some(observation.epoch),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    /// Reads the target with a real witnessed `pane.get`, derives the
    /// cooperative target, then submits with the remaining exchanges.
    fn cooperative_wake(
        rest: Vec<Exchange>,
    ) -> (
        Result<ports::PromptOutcome, ApiError>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        cooperative_wake_with(rest, None, |_, _| {})
    }
    /// A slot through which an exchange reaches the adapter under test (to
    /// change its state mid-call, as a concurrent host call would).
    type CliSlot = Arc<std::sync::OnceLock<Arc<NativeCli>>>;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    /// [`cooperative_wake`] with the adapter published into `slot` before
    /// submission and `tamper` applied to the derived target and context.
    fn cooperative_wake_with(
        rest: Vec<Exchange>,
        slot: Option<CliSlot>,
        tamper: impl FnOnce(&mut SafeWakeTarget, &mut HostCallContext),
    ) -> (
        Result<ports::PromptOutcome, ApiError>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        cooperative_wake_in(registry::builtins(), rest, slot, tamper)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn cooperative_wake_in(
        registry: &'static Registry,
        rest: Vec<Exchange>,
        slot: Option<CliSlot>,
        tamper: impl FnOnce(&mut SafeWakeTarget, &mut HostCallContext),
    ) -> (
        Result<ports::PromptOutcome, ApiError>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        cooperative_wake_submit(registry, rest, slot, tamper, |cli, target, context, _| {
            cli.submit_prompt(target, "wake marker", context)
        })
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    /// [`cooperative_wake_in`] with the submission made by `submit` (given the Herdr methods
    /// called so far, in order).
    fn cooperative_wake_submit<R>(
        registry: &'static Registry,
        rest: Vec<Exchange>,
        slot: Option<CliSlot>,
        tamper: impl FnOnce(&mut SafeWakeTarget, &mut HostCallContext),
        submit: impl FnOnce(
            &NativeCli,
            &SafeWakeTarget,
            &HostCallContext,
            &Arc<std::sync::Mutex<Vec<String>>>,
        ) -> R,
    ) -> (R, Arc<std::sync::Mutex<Vec<String>>>) {
        let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut exchanges = vec![pane_exchange()];
        exchanges.extend(rest);
        let exchanges = exchanges
            .into_iter()
            .map(|exchange| {
                let methods = Arc::clone(&methods);
                Box::new(move |stream: &mut UnixStream, request: Value| {
                    methods
                        .lock()
                        .unwrap()
                        .push(request["method"].as_str().unwrap().to_owned());
                    exchange(stream, request)
                }) as Exchange
            })
            .collect();
        let (socket, cli, worker) = serve_sequence(exchanges);
        let mut cli = cli;
        cli.registry = registry;
        let cli = Arc::new(cli);
        if let Some(slot) = slot {
            assert!(slot.set(Arc::clone(&cli)).is_ok());
        }
        let base = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let observation = cli
            .observe_current_target_for_poke(&HostTargetId::new("w4:p1"), &base)
            .unwrap();
        let mut target = cli
            .safe_wake_target(&SeatId::new("seat_1"), &observation)
            .expect("fresh verified terminal is a cooperative wake target");
        assert_eq!(target.basis, WakeTargetBasis::CooperativeAgent);
        assert_eq!(target.terminal.as_str(), "term_1");
        // The store always supplies the bound harness for a cooperative wake.
        target.bound_harness = Some("claude".into());
        let mut context = wake_context(&observation);
        tamper(&mut target, &mut context);
        let result = submit(&cli, &target, &context, &methods);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        (result, methods)
    }

    /// Catches an ordinary native wake bypassing the composer and submitting a draft.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn nudge_safety_refuses_nonempty_or_unreadable_composer() {
        for text in [
            claude_screen(&["unfinished user draft"]),
            "unknown layout".into(),
        ] {
            // Answer either read or prompt so the pre-fix test fails on behavior,
            // rather than panicking inside its transport fixture.
            let screen: Exchange = Box::new(move |stream, request| {
                answer(
                    stream,
                    &request,
                    json!({"type":"pane_read","read":{
                    "pane_id":"w4:p1","source":"detection","text":text}}),
                );
            });
            // Neither is decidable from detection text: a styled read is asked
            // for, and its refusal keeps the detection read.
            let (result, methods) = cooperative_wake(vec![
                recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
                screen,
                styled_exchange(None),
            ]);
            assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read", "agent.read"]
            );
        }
    }

    fn styled_exchange(reply: Option<Value>) -> Exchange {
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.read");
            assert_eq!(
                request["params"],
                json!({"target":"w4:p1","source":"recent","format":"ansi"})
            );
            match &reply {
                Some(read) => answer(stream, &request, json!({"type":"pane_read","read":read})),
                None => refuse(stream, &request, "invalid_params"),
            }
        })
    }

    fn styled_read(text: &str) -> Value {
        json!({"pane_id":"w4:p1","source":"recent","format":"ansi","text":text,
               "revision":0,"truncated":false})
    }

    const SUGGESTION_DETECTION: &str = include_str!(
        "../../docs/evidence/ansi-composer/captures/claude-2.1.309-suggestion.read-detection.txt"
    );
    const SUGGESTION_ANSI: &str = include_str!(
        "../../docs/evidence/ansi-composer/captures/claude-2.1.309-suggestion.recent-ansi.txt"
    );

    /// Catches a dim prompt suggestion refusing the fallback wake: the styled
    /// read finds the composer empty and the wake is typed.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn nudge_safety_wakes_over_a_dim_prompt_suggestion() {
        let prompt: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            answer(
                stream,
                &request,
                json!({"type":"agent_prompted","agent":wake_agent("idle", Some("claude"), "term_1")}),
            );
        });
        let (result, methods) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            detection_exchange(SUGGESTION_DETECTION),
            styled_exchange(Some(styled_read(SUGGESTION_ANSI))),
            prompt,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted);
        assert_eq!(
            *methods.lock().unwrap(),
            [
                "pane.get",
                "agent.get",
                "agent.read",
                "agent.read",
                "agent.prompt"
            ]
        );
    }

    /// Fail safe: a styled read that is refused, answers in plain text (a
    /// Herdr that ignores `format`), shows typed text or answers another
    /// source keeps the detection read's refusal.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn nudge_safety_keeps_refusing_without_a_trustworthy_styled_read() {
        let draft = include_str!(
            "../../docs/evidence/ansi-composer/captures/claude-2.1.309-draft.recent-ansi.txt"
        );
        let mut plain = styled_read(SUGGESTION_DETECTION);
        plain.as_object_mut().unwrap().remove("format");
        let mut detection_source = styled_read(SUGGESTION_ANSI);
        detection_source["source"] = json!("detection");
        let mut other_pane = styled_read(SUGGESTION_ANSI);
        other_pane["pane_id"] = json!("w4:p2");
        // Herdr's detection source answers `format: "ansi"` with plain text.
        let plain_claiming_ansi = styled_read(SUGGESTION_DETECTION);
        for reply in [
            None,
            Some(plain),
            Some(styled_read(draft)),
            Some(detection_source),
            Some(other_pane),
            Some(plain_claiming_ansi),
            Some(styled_read("\x1b[2mno composer here\x1b[0m\r\n")),
        ] {
            let (result, methods) = cooperative_wake(vec![
                recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
                detection_exchange(SUGGESTION_DETECTION),
                styled_exchange(reply.clone()),
            ]);
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::TargetUnsafe,
                "{reply:?}"
            );
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read", "agent.read"],
                "{reply:?}"
            );
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn nudge_safety_focused_empty_composer_starts_waiting_without_prompt() {
        let mut agent = wake_agent("idle", Some("claude"), "term_1");
        agent["focused"] = json!(true);
        let screen: Exchange = Box::new(|stream, request| {
            answer(
                stream,
                &request,
                json!({"type":"pane_read","read":{
                "pane_id":"w4:p1","source":"detection","text":claude_screen(&[""])}}),
            );
        });
        let (result, methods) = cooperative_wake(vec![recheck_exchange(agent), screen]);
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get", "agent.read"]
        );
    }

    struct NudgeClock(AtomicU64);
    impl Clock for NudgeClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0.load(Ordering::SeqCst).try_into().unwrap())
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }

    /// Catches premature admission, stale-window reuse and identity leakage.
    #[test]
    fn nudge_safety_empty_window_boundaries_and_resets() {
        let clock = Arc::new(NudgeClock(AtomicU64::new(0)));
        let cli = NativeCli::new(PathBuf::from("/unused-nudge-test"), clock.clone());
        let target = composer_target();
        let sample = |at: u64, target: &SafeWakeTarget, focused: bool| {
            clock.0.store(at, Ordering::SeqCst);
            let prior = cli.empty_windows.lock().unwrap().remove(&target.seat);
            cli.focused_empty_ready(target, focused, prior)
        };
        assert!(!sample(0, &target, true));
        assert!(!sample(59_999, &target, true));
        assert!(sample(60_000, &target, true));
        assert!(
            !sample(60_001, &target, true),
            "successful wake resets the wait"
        );
        assert!(
            !sample(120_002, &target, true),
            "missing samples restart the wait"
        );
        assert!(
            !sample(120_001, &target, true),
            "clock reversal restarts the wait"
        );
        let mut moved = target.clone();
        moved.terminal = TerminalId::new("replacement");
        assert!(
            !sample(180_001, &moved, true),
            "replacement cannot inherit idle time"
        );
        assert!(
            sample(180_002, &moved, false),
            "unfocused empty pane wakes immediately"
        );
        assert!(!sample(180_003, &moved, true), "refocus starts a new wait");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn nudge_safety_focused_empty_wakes_after_one_minute() {
        let clock = Arc::new(NudgeClock(AtomicU64::new(0)));
        let mut focused = wake_agent("idle", Some("claude"), "term_1");
        focused["focused"] = json!(true);
        let mut exchanges = vec![pane_exchange()];
        for _ in 0..3 {
            exchanges.push(recheck_exchange(focused.clone()));
            exchanges.push(detection_exchange(claude_screen(&[""])));
        }
        exchanges.push(prompt_exchange("wake marker"));
        let (socket, mut cli, worker) = serve_sequence(exchanges);
        cli.clock = clock.clone();
        let observation = cli
            .observe_current_target(&HostTargetId::new("w4:p1"), &pane_agent_context())
            .unwrap();
        let mut target = cli
            .safe_wake_target(&SeatId::new("seat_1"), &observation)
            .unwrap();
        target.bound_harness = Some("claude".into());
        for at in [0, 59_999, 60_000] {
            clock.0.store(at, Ordering::SeqCst);
            let mut context = wake_context(&observation);
            context.budget.deadline = MonoInstant(at + 10_000);
            let result = cli.submit_prompt(&target, "wake marker", &context);
            if at < 60_000 {
                assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
                assert_eq!(cli.wake_block(&target.seat), Some(WakeBlock::Focused));
            } else {
                assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted);
                assert_eq!(cli.wake_block(&target.seat), None);
            }
        }
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    /// A recorded block is reported for `WAKE_BLOCK_MILLIS` only: an old attempt says nothing
    /// about the screen now.
    #[test]
    fn a_recorded_wake_block_expires() {
        let clock = Arc::new(NudgeClock(AtomicU64::new(0)));
        let cli = NativeCli::new(PathBuf::from("/unused-wake-block-test"), clock.clone());
        let seat = SeatId::new("seat_1");
        cli.note_wake_block(&seat, Some(WakeBlock::Unreadable));
        clock.0.store(WAKE_BLOCK_MILLIS - 1, Ordering::SeqCst);
        assert_eq!(cli.wake_block(&seat), Some(WakeBlock::Unreadable));
        clock.0.store(WAKE_BLOCK_MILLIS, Ordering::SeqCst);
        assert_eq!(cli.wake_block(&seat), None);
        cli.note_wake_block(&seat, Some(WakeBlock::Focused));
        assert_eq!(cli.wake_block(&seat), Some(WakeBlock::Focused));
        cli.note_wake_block(&seat, None);
        assert_eq!(cli.wake_block(&seat), None);
    }

    /// An unreadable composer is recorded as the seat's wake block; the next wake that reads an
    /// empty composer and submits clears it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unreadable_composer_records_a_wake_block_until_a_wake_submits() {
        let agent = wake_agent("idle", Some("claude"), "term_1");
        let (socket, cli, worker) = serve_sequence(vec![
            pane_exchange(),
            recheck_exchange(agent.clone()),
            detection_exchange("unknown layout"),
            styled_exchange(None),
            recheck_exchange(agent),
            detection_exchange(claude_screen(&[""])),
            prompt_exchange("wake marker"),
        ]);
        let observation = cli
            .observe_current_target(&HostTargetId::new("w4:p1"), &pane_agent_context())
            .unwrap();
        let mut target = cli
            .safe_wake_target(&SeatId::new("seat_1"), &observation)
            .unwrap();
        target.bound_harness = Some("claude".into());
        assert_eq!(cli.wake_block(&target.seat), None);
        let refused = cli.submit_prompt(&target, "wake marker", &wake_context(&observation));
        assert_eq!(refused.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(cli.wake_block(&target.seat), Some(WakeBlock::Unreadable));
        let submitted = cli.submit_prompt(&target, "wake marker", &wake_context(&observation));
        assert_eq!(submitted.unwrap(), ports::PromptOutcome::Submitted);
        assert_eq!(cli.wake_block(&target.seat), None);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    #[test]
    fn nudge_safety_intervening_observed_draft_resets_empty_window() {
        for screen in [claude_screen(&["draft"]), "unreadable".into()] {
            let clock = Arc::new(NudgeClock(AtomicU64::new(0)));
            let (socket, mut cli, worker) =
                serve_sequence(vec![pane_exchange_with("idle"), detection_exchange(screen)]);
            cli.clock = clock.clone();
            let target = composer_target();
            assert!(!cli.focused_empty_ready(&target, true, None));
            clock.0.store(30_000, Ordering::SeqCst);
            let observation = cli
                .observe_current_target_for_poke(
                    &target.target,
                    &HostCallContext {
                        budget: CallBudget {
                            deadline: MonoInstant(40_000),
                            cancellation: Cancellation::default(),
                        },
                        expected_boot: None,
                        expected_epoch: None,
                    },
                )
                .unwrap();
            assert_ne!(observation.ui, HostUiState::Idle);
            clock.0.store(60_000, Ordering::SeqCst);
            let previous = cli.empty_windows.lock().unwrap().remove(&target.seat);
            assert!(
                !cli.focused_empty_ready(&target, true, previous),
                "an observed draft restarts the minute"
            );
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
        }
    }

    #[test]
    fn nudge_safety_status_only_idle_read_preserves_empty_window() {
        let clock = Arc::new(NudgeClock(AtomicU64::new(0)));
        let pane: Exchange = Box::new(|stream, request| {
            answer(
                stream,
                &request,
                json!({"type":"pane_info","pane":{
                "pane_id":"w4:p1","terminal_id":"term_1","workspace_id":"w4","tab_id":"w4:t1",
                "focused":true,"agent":"claude","agent_status":"idle","revision":2}}),
            );
        });
        let (socket, mut cli, worker) = serve_sequence(vec![pane]);
        cli.clock = clock.clone();
        let target = composer_target();
        assert!(!cli.focused_empty_ready(&target, true, None));
        let observation = cli
            .observe_current_target(&target.target, &pane_agent_context())
            .unwrap();
        assert_eq!(
            observation.ui,
            HostUiState::Unknown,
            "plain read has no composer evidence"
        );
        clock.0.store(60_000, Ordering::SeqCst);
        let previous = cli.empty_windows.lock().unwrap().remove(&target.seat);
        assert!(
            cli.focused_empty_ready(&target, true, previous),
            "status-only idle read cannot erase valid samples"
        );
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
    }

    use crate::harness::adapter::*;
    struct FourthAdapter(bool, bool);
    impl HarnessAdapter for FourthAdapter {
        type Admission = crate::harness::claude::ClaudeContract;
        fn metadata(&self) -> &'static AdapterMetadata {
            static META: AdapterMetadata = AdapterMetadata {
                agent_env_markers: &[],
                id: "fourth",
                display_label: "Fourth",
                context_spelling: "Fourth",
                context_aliases: &[],
                executable: ExecutableLookup::Unsupported,
                host_kinds: &["fourth-native"],
                setup_scopes: &[],
                budget: EventBudgetPolicy {
                    lifecycle_ms: 5,
                    observer_ms: 2,
                },
                runtime_sources: &[],
            };
            static THIRD: AdapterMetadata = AdapterMetadata {
                agent_env_markers: &[],
                id: "third",
                display_label: "Third",
                context_spelling: "Third",
                context_aliases: &[],
                executable: ExecutableLookup::Unsupported,
                host_kinds: &["third-native"],
                setup_scopes: &[],
                budget: EventBudgetPolicy {
                    lifecycle_ms: 5,
                    observer_ms: 2,
                },
                runtime_sources: &[],
            };
            if self.0 { &THIRD } else { &META }
        }
        fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
            self.1.then_some(&FixtureComposer)
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            &[]
        }
        fn observe_install(
            &self,
            env: &InstallEnvironment,
            budget: &CallBudget,
        ) -> InstallObservation {
            crate::harness::claude::ClaudeAdapter.observe_install(env, budget)
        }
        fn admit(
            &self,
            request: &AdmissionRequest,
            budget: &CallBudget,
        ) -> AdmissionDecision<Self::Admission> {
            crate::harness::claude::ClaudeAdapter.admit(request, budget)
        }
        fn version_ladder(&self, identity: &RuntimeIdentity) -> Ladder {
            crate::harness::claude::ClaudeAdapter.version_ladder(identity)
        }
        fn classify(&self, input: &HookInput) -> ContractObservation {
            crate::harness::claude::ClaudeAdapter.classify(input)
        }
        fn decode(
            &self,
            admitted: &Self::Admission,
            input: &HookInput,
        ) -> Result<DecodedEvent, DecodeFailure> {
            crate::harness::claude::ClaudeAdapter.decode(admitted, input)
        }
        fn encode(
            &self,
            admitted: &Self::Admission,
            event: &DecodedEvent,
            offer: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            crate::harness::claude::ClaudeAdapter.encode(admitted, event, offer)
        }
        fn attribute_runtime(&self, input: &HookInput, budget: &CallBudget) -> RuntimeAttribution {
            crate::harness::claude::ClaudeAdapter.attribute_runtime(input, budget)
        }
        fn setup(
            &self,
            request: &SetupRequest,
            budget: &CallBudget,
        ) -> Result<SetupOutcome, SetupFailure> {
            crate::harness::claude::ClaudeAdapter.setup(request, budget)
        }
        fn status(&self, request: &StatusRequest, budget: &CallBudget) -> SetupStatus {
            crate::harness::claude::ClaudeAdapter.status(request, budget)
        }
        fn unsetup(
            &self,
            request: &UnsetupRequest,
            budget: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            crate::harness::claude::ClaudeAdapter.unsetup(request, budget)
        }
    }
    struct FixtureComposer;
    impl ComposerPolicy for FixtureComposer {
        fn capabilities(&self, _: Option<&str>) -> crate::harness::recipe::PokeCapabilities {
            crate::harness::recipe::PokeCapabilities::NONE
        }
        fn read(&self, text: &str, _: Option<u16>) -> ComposerRead {
            match text {
                "fixture empty" => ComposerRead::Empty,
                "fixture draft: unfinished" => ComposerRead::Text("unfinished".into()),
                _ => ComposerRead::Unreadable,
            }
        }
        fn clear_key(&self) -> &'static str {
            "ctrl+u"
        }
        fn restore_text(&self, saved: &str) -> String {
            saved.into()
        }
    }
    fn fourth_registry() -> &'static Registry {
        fourth_registry_with_composer(false)
    }
    fn fourth_registry_with_composer(composer: bool) -> &'static Registry {
        let fourth = Box::leak(Box::new(FourthAdapter(false, composer)));
        Box::leak(Box::new(
            Registry::new(Box::leak(
                vec![
                    registry::Registration::new(&crate::harness::claude::ClaudeAdapter),
                    registry::Registration::new(&crate::harness::codex::CodexAdapter),
                    registry::Registration::new(&FourthAdapter(true, false)),
                    registry::Registration::new(fourth),
                ]
                .into_boxed_slice(),
            ))
            .unwrap(),
        ))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn injected_registry_alias_reaches_actual_wake_submission_and_refuses_busy_or_mismatch() {
        for status in ["idle", "done"] {
            let prompt = Box::new(|stream: &mut UnixStream, request: Value| {
                assert_eq!(request["method"], "agent.prompt");
                assert_eq!(
                    request["params"],
                    json!({"target":"w4:p1","text":"wake marker"})
                );
                answer(
                    stream,
                    &request,
                    json!({"type":"agent_prompted","agent":wake_agent("idle", Some("fourth-native"), "term_1")}),
                );
            });
            let (result, methods) = cooperative_wake_in(
                fourth_registry_with_composer(true),
                vec![
                    recheck_exchange(wake_agent(status, Some("fourth-native"), "term_1")),
                    detection_exchange("fixture empty"),
                    prompt,
                ],
                None,
                |target, _| target.bound_harness = Some("fourth".into()),
            );
            assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted);
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read", "agent.prompt"]
            );
        }
        for (status, kind) in [
            ("working", "fourth-native"),
            ("blocked", "fourth-native"),
            ("idle", "claude"),
            ("idle", "unknown"),
        ] {
            let (result, methods) = cooperative_wake_in(
                fourth_registry(),
                vec![recheck_exchange(wake_agent(status, Some(kind), "term_1"))],
                None,
                |target, _| target.bound_harness = Some("fourth".into()),
            );
            assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
            assert_eq!(*methods.lock().unwrap(), ["pane.get", "agent.get"]);
        }
        let (result, methods) = cooperative_wake_in(
            fourth_registry(),
            vec![recheck_exchange(wake_agent(
                "idle",
                Some("fourth-native"),
                "term_1",
            ))],
            None,
            |target, _| target.bound_harness = Some("fourth".into()),
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(*methods.lock().unwrap(), ["pane.get", "agent.get"]);
    }

    // Kills a composer consumer that ignores its injected registry or switches on legacy IDs.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn injected_fourth_composer_reads_clears_and_restores_without_submitting_a_draft() {
        let (saved, methods) = poke_session_in(
            fourth_registry_with_composer(true),
            vec![
                detection_exchange("fixture draft: unfinished"),
                clear_exchange(),
                detection_exchange("fixture empty"),
                retype_exchange("unfinished", true),
            ],
            |cli, target, context| {
                let mut target = target.clone();
                target.bound_harness = Some("fourth".into());
                let ComposerStash::Saved(saved) = cli.stash_composer(&target, context).unwrap()
                else {
                    panic!("the injected composer's draft must be saved");
                };
                assert_eq!(saved, "unfinished");
                cli.restore_composer(&target, &saved, context).unwrap();
                saved
            },
        );
        assert_eq!(saved, "unfinished");
        assert_eq!(
            methods,
            [
                "pane.get",
                "agent.read",
                "pane.send_keys",
                "agent.read",
                "pane.send_text"
            ]
        );
    }

    const AGY_DRAFT: &str =
        include_str!("../../tests/fixtures/agy/composer/draft.read-detection.txt");
    const AGY_EMPTY: &str =
        include_str!("../../tests/fixtures/agy/composer/empty.read-detection.txt");

    fn agy_holding(text: &str) -> String {
        AGY_DRAFT.replace("hello draft", text)
    }

    fn clear_wake_as(
        harness: &'static str,
        rest: Vec<Exchange>,
    ) -> (Result<bool, ApiError>, Vec<String>) {
        poke_session_in(
            crate::harness::registry::builtins(),
            rest,
            |cli, target, context| {
                let mut target = target.clone();
                target.bound_harness = Some(harness.into());
                cli.clear_own_wake_text(&target, crate::notification::policy::WAKE_UP, context)
            },
        )
    }

    // ht-wake-loop: an agy wake left unsubmitted is cleared with ctrl+u, only while the composer
    // holds exactly that wake, and an Enter taken as a newline costs one more gated press.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn an_unsubmitted_agy_wake_is_cleared_only_while_the_composer_holds_exactly_it() {
        let wake = agy_holding(crate::notification::policy::WAKE_UP);
        let (cleared, methods) = clear_wake_as(
            "agy",
            vec![
                detection_exchange(wake.clone()),
                clear_exchange(),
                detection_exchange(AGY_EMPTY),
            ],
        );
        assert!(cleared.unwrap());
        assert_eq!(
            methods,
            ["pane.get", "agent.read", "pane.send_keys", "agent.read"]
        );

        let (cleared, methods) = clear_wake_as(
            "agy",
            vec![
                detection_exchange(wake.clone()),
                clear_exchange(),
                detection_exchange(wake.clone()),
                clear_exchange(),
                detection_exchange(AGY_EMPTY),
            ],
        );
        assert!(cleared.unwrap());
        assert_eq!(methods.iter().filter(|m| *m == "pane.send_keys").count(), 2);

        // Typed input after the wake, or a person's draft: no key is sent.
        for held in [
            format!("{}x", crate::notification::policy::WAKE_UP),
            "hello draft".to_owned(),
        ] {
            let (cleared, methods) =
                clear_wake_as("agy", vec![detection_exchange(agy_holding(&held))]);
            assert!(!cleared.unwrap(), "{held}");
            assert_eq!(methods, ["pane.get", "agent.read"], "{held}");
        }

        // Still the wake after every allowed press: give up without a further key.
        let (cleared, methods) = clear_wake_as(
            "agy",
            vec![
                detection_exchange(wake.clone()),
                clear_exchange(),
                detection_exchange(wake.clone()),
                clear_exchange(),
                detection_exchange(wake),
            ],
        );
        assert!(!cleared.unwrap());
        assert_eq!(methods.iter().filter(|m| *m == "pane.send_keys").count(), 2);
    }

    // A harness whose composer policy does not opt in is never read or keyed.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn an_unsubmitted_wake_is_left_alone_without_the_composer_opt_in() {
        for harness in ["claude", "codex"] {
            let (cleared, methods) = clear_wake_as(harness, vec![]);
            assert!(!cleared.unwrap(), "{harness}");
            assert_eq!(methods, ["pane.get"], "{harness}");
        }
    }

    // Catches a hard-coded host-kind list: a registered fourth adapter's
    // native alias must reach the real wake recheck, without changing the binding.
    #[test]
    fn registry_host_kind_and_absent_composer_preserve_wake_fences_and_send_no_keys() {
        let mut target = composer_target();
        target.bound_harness = Some("fourth".into());
        assert!(
            cooperative_wake_ready(
                &wake_agent("idle", Some("fourth-native"), "term_1"),
                &target,
                fourth_registry(),
            )
            .is_ok(),
            "a registered native alias must match its bound adapter ID"
        );
        let registry = fourth_registry();
        for status in ["done", "idle"] {
            assert!(
                cooperative_wake_ready(
                    &wake_agent(status, Some("fourth-native"), "term_1"),
                    &target,
                    registry
                )
                .is_ok()
            );
        }
        for (status, kind) in [
            ("working", "fourth-native"),
            ("blocked", "fourth-native"),
            ("unknown", "fourth-native"),
            ("idle", "unknown"),
            ("idle", "claude"),
        ] {
            assert!(
                cooperative_wake_ready(
                    &wake_agent(status, Some(kind), "term_1"),
                    &target,
                    registry
                )
                .is_err(),
                "{status}/{kind}"
            );
        }
        let cli = NativeCli::with_registry(
            PathBuf::from("/missing/no-keys.sock"),
            Arc::new(TestClock(Instant::now())),
            registry,
        );
        cli.epoch.store(3, Ordering::Release);
        let context = composer_context(&TestClock(Instant::now()));
        assert_eq!(
            cli.read_composer(
                &target.target,
                Harness::Agent(registry.agent("fourth").unwrap()),
                &context
            )
            .unwrap(),
            ComposerRead::Unreadable
        );
        assert_eq!(
            cli.submit_prompt_during_turn(&target, "poke", &context)
                .unwrap_err()
                .code,
            ErrorCode::UnsupportedHarness
        );
        assert_eq!(
            cli.send_submit_key(&target, &context).unwrap_err().code,
            ErrorCode::UnsupportedHarness
        );
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            assert_eq!(
                cli.stash_composer(&target, &context).unwrap(),
                ComposerStash::Failed("composer stash unsupported: no composer policy".into())
            );
            assert_eq!(
                cli.restore_composer(&target, "saved draft", &context)
                    .unwrap_err()
                    .code,
                ErrorCode::UnsupportedHarness
            );
        }
    }

    /// Idle and done agents of a recognized harness are prompted only after
    /// a fresh recheck, and `Submitted` is transport submission only.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_prompts_recognized_idle_or_done_agent_after_recheck() {
        for (status, kind) in [
            ("idle", "claude"),
            ("done", "claude"),
            ("idle", "codex"),
            ("done", "codex"),
        ] {
            let prompt: Exchange = Box::new(move |stream: &mut UnixStream, request: Value| {
                assert_eq!(request["method"], "agent.prompt");
                assert_eq!(
                    request["params"],
                    json!({"target":"w4:p1","text":"wake marker"})
                );
                answer(
                    stream,
                    &request,
                    json!({"type":"agent_prompted","agent":wake_agent("idle", Some(kind), "term_1")}),
                );
            });
            let (result, methods) = cooperative_wake_with(
                vec![
                    recheck_exchange(wake_agent(status, Some(kind), "term_1")),
                    detection_exchange(if kind == "claude" {
                        claude_screen(&[""])
                    } else {
                        CODEX_EMPTY.into()
                    }),
                    prompt,
                ],
                None,
                |target, _| target.bound_harness = Some(kind.into()),
            );
            assert_eq!(
                result.unwrap(),
                ports::PromptOutcome::Submitted,
                "{status} {kind}"
            );
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read", "agent.prompt"],
                "the recheck immediately precedes the prompt"
            );
        }
    }

    /// Kills: ht-78f6's busy-agent nudge. The wake's owed check must run after the idle recheck
    /// and composer read, right before `agent.prompt`: a turn that served the item and ended
    /// between the dispatcher's early check and the recheck must not be prompted. A declined
    /// check sends nothing; an agent still working is refused before the check is asked.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_asks_still_owed_after_the_recheck_and_before_the_prompt() {
        let prompt = || -> Exchange {
            Box::new(|stream: &mut UnixStream, request: Value| {
                assert_eq!(request["method"], "agent.prompt");
                answer(
                    stream,
                    &request,
                    json!({"type":"agent_prompted","agent":wake_agent("idle", Some("codex"), "term_1")}),
                );
            })
        };
        let gated = |status: &str, owed: bool, tail: Vec<Exchange>| {
            let mut rest = vec![recheck_exchange(wake_agent(
                status,
                Some("codex"),
                "term_1",
            ))];
            rest.extend(tail);
            cooperative_wake_submit(
                registry::builtins(),
                rest,
                None,
                |target, _| target.bound_harness = Some("codex".into()),
                move |cli, target, context, methods| {
                    cli.submit_prompt_if_owed(target, "wake marker", context, &|| {
                        methods.lock().unwrap().push("still_owed".into());
                        Ok(owed)
                    })
                },
            )
        };
        // Served meanwhile: asked after the recheck and the composer read, nothing typed.
        let (result, methods) = gated("idle", false, vec![detection_exchange(CODEX_EMPTY)]);
        assert_eq!(result.unwrap(), None);
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get", "agent.read", "still_owed"]
        );
        // Still owed: the prompt follows the check.
        let (result, methods) = gated(
            "done",
            true,
            vec![detection_exchange(CODEX_EMPTY), prompt()],
        );
        assert_eq!(result.unwrap(), Some(ports::PromptOutcome::Submitted));
        assert_eq!(
            *methods.lock().unwrap(),
            [
                "pane.get",
                "agent.get",
                "agent.read",
                "still_owed",
                "agent.prompt"
            ]
        );
        // Still working at the recheck: refused before the check is asked.
        let (result, methods) = gated("working", true, vec![]);
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(*methods.lock().unwrap(), ["pane.get", "agent.get"]);
    }

    /// TRUST-POLICY A4 wake rule: the pane's detected agent kind must equal
    /// the seat's bound harness; a different kind (both directions) or no
    /// detected kind refuses before any prompt, a matching kind is prompted.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_claude_bound_seat_with_codex_agent() {
        bound_harness_wake_refused("claude", Some("codex"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_codex_bound_seat_with_claude_agent() {
        bound_harness_wake_refused("codex", Some("claude"));
    }

    /// ht-f0xn: a Hermes target matches its pane, not its gateway pid, so this recheck is what
    /// refuses a pane whose agent is another harness.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_hermes_bound_seat_with_claude_agent() {
        bound_harness_wake_refused("hermes", Some("claude"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_codex_bound_seat_without_detected_agent() {
        bound_harness_wake_refused("codex", None);
    }

    /// A cooperative target with no bound harness is refused, never compared
    /// with the detected agent kind as "any recognized kind".
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_target_without_bound_harness() {
        let (result, methods) = cooperative_wake_with(
            vec![recheck_exchange(wake_agent(
                "idle",
                Some("claude"),
                "term_1",
            ))],
            None,
            |target, _| target.bound_harness = None,
        );
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::TargetUnsafe);
        assert!(
            error.detail.contains("no bound harness"),
            "{}",
            error.detail
        );
        assert_eq!(*methods.lock().unwrap(), ["pane.get", "agent.get"]);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn bound_harness_wake_refused(bound: &'static str, detected: Option<&str>) {
        let (result, methods) = cooperative_wake_with(
            vec![recheck_exchange(wake_agent("idle", detected, "term_1"))],
            None,
            |target, _| target.bound_harness = Some(bound.into()),
        );
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::TargetUnsafe, "{bound} {detected:?}");
        assert!(
            error.detail.contains("bound harness") || error.detail.contains("no recognized"),
            "{}",
            error.detail
        );
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get"],
            "no agent.prompt is sent"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_prompts_matching_bound_harness() {
        for kind in ["claude", "codex"] {
            let prompt: Exchange = Box::new(move |stream: &mut UnixStream, request: Value| {
                assert_eq!(request["method"], "agent.prompt");
                answer(
                    stream,
                    &request,
                    json!({"type":"agent_prompted","agent":wake_agent("idle", Some(kind), "term_1")}),
                );
            });
            let (result, methods) = cooperative_wake_with(
                vec![
                    recheck_exchange(wake_agent("idle", Some(kind), "term_1")),
                    detection_exchange(if kind == "claude" {
                        claude_screen(&[""])
                    } else {
                        CODEX_EMPTY.into()
                    }),
                    prompt,
                ],
                None,
                |target, _| target.bound_harness = Some(kind.into()),
            );
            assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted, "{kind}");
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read", "agent.prompt"],
                "{kind}"
            );
        }
    }

    /// Eligibility matrix at the recheck: working, blocked and unknown
    /// statuses, a shell (no agent), an unrecognized program and a different
    /// terminal are refused before any prompt is sent.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_ineligible_agents_before_any_prompt() {
        let cases: Vec<(&str, Exchange)> = vec![
            (
                "working",
                recheck_exchange(wake_agent("working", Some("claude"), "term_1")),
            ),
            (
                "blocked",
                recheck_exchange(wake_agent("blocked", Some("claude"), "term_1")),
            ),
            (
                "unknown",
                recheck_exchange(wake_agent("unknown", Some("codex"), "term_1")),
            ),
            (
                "no agent kind",
                recheck_exchange(wake_agent("idle", None, "term_1")),
            ),
            (
                "unrecognized",
                recheck_exchange(wake_agent("idle", Some("opencode"), "term_1")),
            ),
            (
                "replaced terminal",
                recheck_exchange(wake_agent("idle", Some("claude"), "term_2")),
            ),
            (
                "shell",
                Box::new(|stream: &mut UnixStream, request: Value| {
                    assert_eq!(request["method"], "agent.get");
                    writeln!(
                        stream,
                        "{}",
                        json!({"id":request["id"],"error":{"code":"agent_not_found","message":"agent target w4:p1 not found"}})
                    )
                    .unwrap();
                }),
            ),
        ];
        for (label, recheck) in cases {
            let (result, methods) = cooperative_wake(vec![recheck]);
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::TargetUnsafe,
                "{label} must be refused"
            );
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get"],
                "{label}"
            );
        }
    }

    /// Herdr's own pre-input refusal is a refusal; a submission without a
    /// correlated answer is honestly unknown and never retried here.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_classifies_refused_and_unknown_submissions() {
        let blocked: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            writeln!(
                stream,
                "{}",
                json!({"id":request["id"],"error":{"code":"agent_blocked","message":"agent is blocked"}})
            )
            .unwrap();
        });
        let (result, _) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            detection_exchange(claude_screen(&[""])),
            blocked,
        ]);
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);

        // A Herdr that does not serve the method typed nothing either.
        let unserved: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            writeln!(
                stream,
                "{}",
                json!({"id":request["id"],"error":{"code":"unknown_method","message":"unknown method: agent.prompt"}})
            )
            .unwrap();
        });
        let (result, _) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            detection_exchange(claude_screen(&[""])),
            unserved,
        ]);
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);

        let lost: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            drop(stream.try_clone().unwrap());
        });
        let (result, methods) = cooperative_wake(vec![
            recheck_exchange(wake_agent("done", Some("claude"), "term_1")),
            detection_exchange(claude_screen(&[""])),
            lost,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
        assert_eq!(methods.lock().unwrap().len(), 4, "never replayed");

        let other_terminal: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            answer(
                stream,
                &request,
                json!({"type":"agent_prompted","agent":wake_agent("idle", Some("claude"), "term_9")}),
            );
        });
        let (result, _) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            detection_exchange(claude_screen(&[""])),
            other_terminal,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
    }

    /// The second Herdr server of
    /// `cooperative_wake_refuses_when_incarnation_or_epoch_moves_during_recheck`.
    /// The test binary re-executes itself with `HT_FAKE_HOST_SOCKET` set; this
    /// then runs as a different process, so its kernel peer witness (pid and
    /// start time) is a different server incarnation. It logs each request
    /// method to `HT_FAKE_HOST_LOG`, answers `ping` and `agent.get` (an idle
    /// claude agent in `term_1`), refuses anything else, and exits when its
    /// stdin closes. Without the variable it is a no-op test.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn restarted_host_process() {
        let Ok(socket) = std::env::var("HT_FAKE_HOST_SOCKET") else {
            return;
        };
        let log = std::env::var("HT_FAKE_HOST_LOG").unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        println!("HT_FAKE_HOST_READY");
        std::io::stdout().flush().unwrap();
        thread::spawn(move || {
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read(&mut stream);
                let method = request["method"].as_str().unwrap().to_owned();
                let mut log = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .unwrap();
                writeln!(log, "{method}").unwrap();
                match method.as_str() {
                    "ping" => answer(
                        &mut stream,
                        &request,
                        json!({"type":"pong","version":"0.9.1","protocol":22}),
                    ),
                    "agent.get" => answer(
                        &mut stream,
                        &request,
                        json!({"type":"agent_info","agent":wake_agent("idle", Some("claude"), "term_1")}),
                    ),
                    _ => writeln!(
                        stream,
                        "{}",
                        json!({"id":request["id"],"error":{"code":"unexpected","message":"unexpected method"}})
                    )
                    .unwrap(),
                }
            }
        });
        let mut sink = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut sink).unwrap();
    }

    /// The target is derived from a real witnessed `pane.get` answered by
    /// server incarnation A (this process). Server A then goes away and a
    /// different process (incarnation B) listens on the same endpoint and
    /// answers the `agent.get` recheck. Returns the submission result and the
    /// methods server B saw.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn cooperative_wake_after_host_restart() -> (Result<ports::PromptOutcome, ApiError>, Vec<String>)
    {
        let (socket, cli, worker) = serve_sequence(vec![pane_exchange()]);
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let observation = cli
            .observe_current_target_for_poke(&HostTargetId::new("w4:p1"), &context)
            .unwrap();
        let target = cli
            .safe_wake_target(&SeatId::new("seat_1"), &observation)
            .expect("fresh verified terminal is a cooperative wake target");
        // `ServerIncarnation::from_witness` derives the process identity and the boot from the same
        // peer witness, so a restart moves both fields `same_incarnation` compares.
        assert_eq!(target.incarnation, observation.host_boot.as_str());
        let context = wake_context(&observation);
        worker.join().unwrap();
        fs::remove_file(&socket).unwrap();

        let log = crate::test_support::scratch::temp_path(format!(
            "ht-restart-log-{}",
            uuid::Uuid::new_v4()
        ));
        let mut host = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "host::native::tests::restarted_host_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("HT_FAKE_HOST_SOCKET", &socket)
            .env("HT_FAKE_HOST_LOG", &log)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(host.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "server B never listened"
            );
            if line.contains("HT_FAKE_HOST_READY") {
                break;
            }
        }
        let result = cli.submit_prompt(&target, "wake marker", &context);
        drop(host.stdin.take());
        host.wait().unwrap();
        let seen = fs::read_to_string(&log).unwrap_or_default();
        let _ = fs::remove_file(&log);
        let _ = fs::remove_file(&socket);
        (result, seen.lines().map(str::to_owned).collect())
    }

    /// W9-4: the TOCTOU fence right before typing into a pane. When the
    /// `agent.get` recheck is answered by a different server incarnation than
    /// the one the target was derived from (the Herdr server restarted
    /// between the observation and the recheck: a different process, which is
    /// a different process identity and boot), or the connection epoch moves
    /// while the recheck is in flight, the prompt is refused and
    /// `agent.prompt` is never sent. The host really changes: nothing here
    /// edits the target. Kills: dropping the post-recheck
    /// `same_incarnation`/epoch check.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_refuses_when_incarnation_or_epoch_moves_during_recheck() {
        let (result, seen) = cooperative_wake_after_host_restart();
        assert_eq!(
            result.unwrap_err().code,
            ErrorCode::TargetUnsafe,
            "a recheck answered by a restarted server must refuse"
        );
        assert_eq!(
            seen,
            ["ping", "agent.get"],
            "the restarted server saw the recheck and never an agent.prompt"
        );

        let slot: CliSlot = Arc::default();
        let bump = Arc::clone(&slot);
        let recheck = recheck_exchange(wake_agent("idle", Some("claude"), "term_1"));
        let epoch_moves: Exchange = Box::new(move |stream: &mut UnixStream, request: Value| {
            // A concurrent host call failed and advanced the connection
            // epoch while this recheck was being answered.
            bump.get().unwrap().epoch.fetch_add(1, Ordering::AcqRel);
            recheck(stream, request);
        });
        let (result, methods) = cooperative_wake_with(vec![epoch_moves], Some(slot), |_, _| {});
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get"],
            "agent.prompt never sent"
        );
    }

    /// W9-4: an `agent.prompt` error code this adapter does not recognise
    /// may have been answered after typing began, so it is OutcomeUnknown,
    /// never a refusal, and it is never replayed. Kills: mapping unknown
    /// codes to a pre-input refusal or retrying the prompt.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn cooperative_wake_unrecognised_prompt_error_is_unknown_without_replay() {
        let weird: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            writeln!(
                stream,
                "{}",
                json!({"id":request["id"],"error":{"code":"weird_code","message":"something new"}})
            )
            .unwrap();
        });
        let (result, methods) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            detection_exchange(claude_screen(&[""])),
            weird,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get", "agent.read", "agent.prompt"],
            "never replayed"
        );
    }

    /// Structural refusals need no I/O: an unfresh or unverified read, known
    /// empty shell or blocked/active UI never yields a target; a verified
    /// occupant basis, a changed host context or an exhausted budget never
    /// reaches the host.
    #[test]
    fn cooperative_wake_target_and_submission_fences_without_io() {
        let (_, _, observation) = launch_fixture();
        let cli = NativeCli::new(
            crate::test_support::scratch::temp_path(format!("ht-absent-{}", uuid::Uuid::new_v4())),
            Arc::new(TestClock(Instant::now())),
        );
        cli.epoch.store(3, Ordering::Release);
        let seat = SeatId::new("seat_1");
        let target = cli.safe_wake_target(&seat, &observation);
        assert_eq!(
            target.is_some(),
            crate::host::continuity::PEER_WITNESS_SUPPORTED
        );
        type Change = Box<dyn Fn(&mut HostObservation)>;
        let refused: Vec<(&str, Change)> = vec![
            (
                "cache",
                Box::new(|o| o.provenance = ObservationProvenance::UncharacterizedCache),
            ),
            (
                "unknown incarnation",
                Box::new(|o| o.incarnation = IncarnationEvidence::Unknown),
            ),
            (
                "enumeration evidence",
                Box::new(|o| {
                    o.incarnation = IncarnationEvidence::Verified {
                        identity: "server-1".into(),
                        evidence_kind: EvidenceKind::CoherentEnumeration,
                    }
                }),
            ),
            ("no terminal", Box::new(|o| o.terminal = None)),
            (
                "empty shell",
                Box::new(|o| o.occupancy = StructuralOccupancy::EmptyShell),
            ),
            (
                "approval",
                Box::new(|o| o.ui = HostUiState::ApprovalOrQuestion),
            ),
            ("active turn", Box::new(|o| o.ui = HostUiState::ActiveTurn)),
            ("generation 0", Box::new(|o| o.generation = 0)),
        ];
        for (label, change) in refused {
            let mut changed = observation.clone();
            change(&mut changed);
            assert!(cli.safe_wake_target(&seat, &changed).is_none(), "{label}");
        }
        let mut typed = observation.clone();
        typed.ui = HostUiState::HumanInput;
        assert_eq!(
            cli.safe_wake_target(&seat, &typed).is_some(),
            crate::host::continuity::PEER_WITNESS_SUPPORTED,
            "typed input is a safe wake target"
        );
        let Some(target) = target else { return };
        let context = wake_context(&observation);
        let mut verified = target.clone();
        verified.basis = WakeTargetBasis::VerifiedOccupant {
            session: crate::protocol::ids::NativeSessionId::new("s"),
            execution: crate::protocol::ids::ExecutionId::new("e"),
        };
        assert_eq!(
            cli.submit_prompt(&verified, "x", &context)
                .unwrap_err()
                .code,
            ErrorCode::TargetUnsafe
        );
        let mut stale = context.clone();
        stale.expected_epoch = Some(4);
        assert_eq!(
            cli.submit_prompt(&target, "x", &stale).unwrap_err().code,
            ErrorCode::TargetUnsafe
        );
        let mut short = context.clone();
        short.budget.deadline = MonoInstant(0);
        assert_eq!(
            cli.submit_prompt(&target, "x", &short).unwrap_err().code,
            ErrorCode::DeadlineExceeded
        );
        assert_eq!(cli.epoch(), 3, "no host call was attempted");
    }

    // ---- composer observation, stash and poke-during-turn ----

    const CLAUDE_EMPTY: &str =
        include_str!("../../docs/evidence/poke-spike/captures/claude-q1-empty.read-detection.txt");
    const CLAUDE_DRAFT: &str = include_str!(
        "../../docs/evidence/poke-spike/captures/claude-q1-q2-single.read-detection.txt"
    );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    const CLAUDE_IMAGE: &str = include_str!(
        "../../docs/evidence/poke-spike/captures/claude-q2-image-placeholder.read-detection.txt"
    );
    const CODEX_EMPTY: &str =
        include_str!("../../docs/evidence/poke-spike/captures/codex-q6-workers.read-detection.txt");

    /// A Claude screen whose composer holds `rows`.
    fn claude_screen(rows: &[&str]) -> String {
        let rule = "─".repeat(80);
        let mut body = String::new();
        for (index, row) in rows.iter().enumerate() {
            body.push_str(if index == 0 { "❯ " } else { "  " });
            body.push_str(row);
            body.push('\n');
        }
        format!("\n{rule}\n{body}{rule}\n  footer\n")
    }

    fn detection_exchange(text: impl Into<String>) -> Exchange {
        let text = text.into();
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.read");
            assert_eq!(
                request["params"],
                json!({"target":"w4:p1","source":"detection"})
            );
            answer(
                stream,
                &request,
                json!({"type":"pane_read","read":{"pane_id":"w4:p1","source":"detection",
                    "format":"text","text":text,"revision":0,"truncated":false}}),
            );
        })
    }
    fn failing_detection_exchange() -> Exchange {
        Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.read");
            refuse(stream, &request, "agent_not_found");
        })
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn clear_exchange() -> Exchange {
        Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.send_keys");
            assert_eq!(
                request["params"],
                json!({"pane_id":"w4:p1","keys":["ctrl+u"]})
            );
            answer(stream, &request, json!({"type":"ok"}));
        })
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn retype_exchange(text: &'static str, ok: bool) -> Exchange {
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.send_text");
            assert_eq!(request["params"], json!({"pane_id":"w4:p1","text":text}));
            if ok {
                answer(stream, &request, json!({"type":"ok"}));
            } else {
                refuse(stream, &request, "pane_not_found");
            }
        })
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn prompt_exchange(text: &'static str) -> Exchange {
        Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            assert_eq!(request["params"], json!({"target":"w4:p1","text":text}));
            answer(
                stream,
                &request,
                json!({"type":"agent_prompted","agent":wake_agent("idle", Some("claude"), "term_1")}),
            );
        })
    }

    /// Observes `w4:p1` (a `pane.get` the composer read does not follow),
    /// derives the poke target for a claude-bound seat, and hands the adapter,
    /// target and context to `act` while the scripted exchanges serve. Returns
    /// `act`'s result and the methods the host saw, in order.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn poke_session<T>(
        rest: Vec<Exchange>,
        act: impl FnOnce(&NativeCli, &SafeWakeTarget, &HostCallContext) -> T,
    ) -> (T, Vec<String>) {
        poke_session_in(registry::builtins(), rest, act)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn poke_session_in<T>(
        registry: &'static Registry,
        rest: Vec<Exchange>,
        act: impl FnOnce(&NativeCli, &SafeWakeTarget, &HostCallContext) -> T,
    ) -> (T, Vec<String>) {
        let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut exchanges = vec![pane_exchange()];
        exchanges.extend(rest);
        let exchanges = exchanges
            .into_iter()
            .map(|exchange| {
                let methods = Arc::clone(&methods);
                Box::new(move |stream: &mut UnixStream, request: Value| {
                    methods
                        .lock()
                        .unwrap()
                        .push(request["method"].as_str().unwrap().to_owned());
                    exchange(stream, request)
                }) as Exchange
            })
            .collect();
        let (socket, mut cli, worker) = serve_sequence(exchanges);
        cli.registry = registry;
        let base = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let observation = cli
            .observe_current_target_for_poke(&HostTargetId::new("w4:p1"), &base)
            .unwrap();
        let mut target = cli
            .safe_poke_target(&SeatId::new("seat_1"), &observation)
            .expect("a fresh verified terminal is a poke target");
        target.bound_harness = Some("claude".into());
        let context = wake_context(&observation);
        let result = act(&cli, &target, &context);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        let methods = methods.lock().unwrap().clone();
        (result, methods)
    }

    /// Kills: a clear that skips the verifying read, sends the poke before
    /// the composer is empty, retypes with a different text or presses Enter
    /// after retyping.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn clear_verifies_before_the_poke() {
        let ((stash, prompt, restore), methods) = poke_session(
            vec![
                clear_exchange(),
                detection_exchange(CLAUDE_EMPTY),
                recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
                detection_exchange(CLAUDE_EMPTY),
                prompt_exchange("poke text"),
                retype_exchange("hello world one", true),
            ],
            |cli, target, context| {
                let stash =
                    cli.clear_composer(target, Harness::Claude, "hello world one".into(), context);
                let prompt = cli.submit_prompt(target, "poke text", context);
                let restore = cli.restore_composer(target, "hello world one", context);
                (stash, prompt, restore)
            },
        );
        assert_eq!(
            stash.unwrap(),
            ComposerStash::Saved("hello world one".into())
        );
        assert_eq!(prompt.unwrap(), ports::PromptOutcome::Submitted);
        restore.unwrap();
        assert_eq!(
            methods,
            [
                "pane.get",
                "pane.send_keys",
                "agent.read",
                "agent.get",
                "agent.read",
                "agent.prompt",
                "pane.send_text",
            ],
            "clear, verify, then the poke, then the retype; never an Enter"
        );
    }

    /// A multi-line draft needs one clear per line; the stash keeps clearing
    /// until a read shows the composer empty. Kills: a single fixed clear.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn stash_clears_until_the_composer_reads_empty() {
        let (stash, methods) = poke_session(
            vec![
                clear_exchange(),
                detection_exchange(claude_screen(&["line one"])),
                styled_exchange(None),
                clear_exchange(),
                detection_exchange(CLAUDE_EMPTY),
            ],
            |cli, target, context| {
                cli.clear_composer(
                    target,
                    Harness::Claude,
                    "line one\nline two".into(),
                    context,
                )
            },
        );
        assert_eq!(
            stash.unwrap(),
            ComposerStash::Saved("line one\nline two".into())
        );
        assert_eq!(methods.iter().filter(|m| *m == "pane.send_keys").count(), 2);
    }

    /// Kills: a stash that continues after a failed composer read.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn read_failure_aborts_before_any_input() {
        let (stash, methods) = poke_session(
            vec![failing_detection_exchange()],
            |cli, target, context| cli.stash_composer(target, context),
        );
        assert!(matches!(stash.unwrap(), ComposerStash::Failed(_)));
        assert_eq!(methods, ["pane.get", "agent.read"]);
    }

    /// Kills: treating a composer that never empties as stashed (the poke
    /// would merge into the person's text). The cap is one clear per line
    /// plus the slack, and the typed text is not lost silently.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn clear_not_verified_aborts() {
        let mut exchanges = vec![];
        for _ in 0..(1 + COMPOSER_CLEAR_SLACK) {
            exchanges.push(clear_exchange());
            exchanges.push(detection_exchange(CLAUDE_DRAFT));
            exchanges.push(styled_exchange(None));
        }
        let (stash, methods) = poke_session(exchanges, |cli, target, context| {
            cli.clear_composer(target, Harness::Claude, "hello world one".into(), context)
        });
        assert_eq!(
            stash.unwrap(),
            ComposerStash::Failed("composer not empty after clearing".into())
        );
        assert!(!methods.iter().any(|m| m == "agent.prompt"));
        assert_eq!(
            methods.iter().filter(|m| *m == "pane.send_keys").count(),
            1 + COMPOSER_CLEAR_SLACK
        );
    }

    /// A failed clear key is a stash failure, not a skipped step.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn clear_key_failure_aborts() {
        let failing_keys: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.send_keys");
            refuse(stream, &request, "pane_not_found");
        });
        let (stash, methods) = poke_session(vec![failing_keys], |cli, target, context| {
            cli.clear_composer(target, Harness::Claude, "hello world one".into(), context)
        });
        assert!(matches!(
            stash.unwrap(),
            ComposerStash::Failed(detail) if detail.contains("clear failed")
        ));
        assert_eq!(methods, ["pane.get", "pane.send_keys"]);
    }

    /// Without a styled read, Claude text may be a prompt suggestion, so the
    /// stash refuses before any key (the poke is skipped for that poke only).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn claude_text_fails_stash_before_any_key() {
        for screen in [
            CLAUDE_DRAFT.to_owned(),
            claude_screen(&["herdr-threads pending-receipts"]),
        ] {
            // No styled read is served (a Herdr without one): detection text
            // alone cannot tell the draft from a suggestion.
            let (stash, methods) = poke_session(
                vec![detection_exchange(screen), styled_exchange(None)],
                |cli, target, context| cli.stash_composer(target, context),
            );
            assert_eq!(
                stash.unwrap(),
                ComposerStash::Failed(
                    crate::harness::claude::composer::CLAUDE_NOT_KNOWN_EMPTY.into()
                )
            );
            assert_eq!(
                methods,
                ["pane.get", "agent.read", "agent.read"],
                "no pane.send_keys"
            );
        }
    }

    /// Findings Q4: a retyped image placeholder comes back as literal text, so
    /// the stash refuses before sending any key. Kills: stashing past it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unsafe_condition_from_findings_fails_stash() {
        let (stash, methods) = poke_session(
            vec![detection_exchange(CLAUDE_IMAGE)],
            |cli, target, context| cli.stash_composer(target, context),
        );
        assert_eq!(
            stash.unwrap(),
            ComposerStash::Failed("image placeholder in the composer".into())
        );
        assert_eq!(methods, ["pane.get", "agent.read"]);
    }

    /// A failed retype is an error for the dispatcher, which logs the saved
    /// text and keeps the poke counted (see the dispatcher's own test).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn retype_failure_is_returned_for_the_dispatcher_to_log() {
        let (restore, methods) = poke_session(
            vec![retype_exchange("half-typed words", false)],
            |cli, target, context| cli.restore_composer(target, "half-typed words", context),
        );
        assert!(restore.is_err());
        assert_eq!(methods, ["pane.get", "pane.send_text"]);
    }

    /// Kills: an unfenced stash or restore (a changed host epoch must refuse
    /// before any input).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn stash_and_restore_refuse_a_changed_host_context() {
        let (results, methods) = poke_session(vec![], |cli, target, context| {
            let mut stale = context.clone();
            stale.expected_epoch = Some(context.expected_epoch.unwrap() + 1);
            (
                cli.stash_composer(target, &stale),
                cli.restore_composer(target, "text", &stale),
            )
        });
        assert_eq!(results.0.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(results.1.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(methods, ["pane.get"]);
    }

    /// Empty composer is not idle evidence: both prompt entry points refuse
    /// a working agent before composer I/O or any submitted input.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn active_turn_attention_is_refused_for_codex_and_claude() {
        for kind in ["codex", "claude"] {
            let working = || recheck_exchange(wake_agent("working", Some(kind), "term_1"));
            let ((ordinary, during_turn), methods) =
                poke_session(vec![working(), working()], |cli, target, context| {
                    let mut target = target.clone();
                    target.bound_harness = Some(kind.into());
                    (
                        cli.submit_prompt(&target, "wake marker", context),
                        cli.submit_prompt_during_turn(&target, "poke text", context),
                    )
                });
            for result in [ordinary, during_turn] {
                let refused = result.unwrap_err();
                assert_eq!(refused.code, ErrorCode::TargetUnsafe);
                assert!(refused.detail.contains("not awaiting input"), "{refused:?}");
            }
            assert_eq!(methods, ["pane.get", "agent.get", "agent.get"], "{kind}");
        }
    }

    /// Blocked UI is refused even in the during-turn mode.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn during_turn_mode_still_refuses_a_blocked_agent() {
        let (result, methods) = poke_session(
            vec![recheck_exchange(wake_agent(
                "blocked",
                Some("claude"),
                "term_1",
            ))],
            |cli, target, context| cli.submit_prompt_during_turn(target, "poke text", context),
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
        assert_eq!(methods, ["pane.get", "agent.get"]);
    }

    /// One `observe_current_target` over a pane in `status` of `kind`, with
    /// `rest` exchanges after the `pane.get`; returns the observed UI state
    /// and the methods the host saw.
    fn observe_ui(
        status: &'static str,
        kind: &'static str,
        rest: Vec<Exchange>,
    ) -> (HostUiState, Vec<String>) {
        observe_ui_as(true, status, kind, rest)
    }

    /// [`observe_ui`] for an ordinary wake (`poke: false`: the plain
    /// observation) or a poke (`poke: true`: the composer-classified one).
    fn observe_ui_as(
        poke: bool,
        status: &'static str,
        kind: &'static str,
        rest: Vec<Exchange>,
    ) -> (HostUiState, Vec<String>) {
        let methods = Arc::new(std::sync::Mutex::new(Vec::new()));
        let pane: Exchange = Box::new(move |stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.get");
            answer(
                stream,
                &request,
                json!({"type":"pane_info","pane":{"pane_id":"w4:p1","terminal_id":"term_1",
                    "workspace_id":"w4","tab_id":"w4:t1","focused":false,
                    "agent_status":status,"agent":kind,"revision":2}}),
            );
        });
        let mut exchanges = vec![pane];
        exchanges.extend(rest);
        let exchanges = exchanges
            .into_iter()
            .map(|exchange| {
                let methods = Arc::clone(&methods);
                Box::new(move |stream: &mut UnixStream, request: Value| {
                    methods
                        .lock()
                        .unwrap()
                        .push(request["method"].as_str().unwrap().to_owned());
                    exchange(stream, request)
                }) as Exchange
            })
            .collect();
        let (socket, cli, worker) = serve_sequence(exchanges);
        let target = HostTargetId::new("w4:p1");
        let observation = if poke {
            cli.observe_current_target_for_poke(&target, &pane_agent_context())
        } else {
            cli.observe_current_target(&target, &pane_agent_context())
        }
        .expect("a composer read never fails the observation");
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        let methods = methods.lock().unwrap().clone();
        (observation.ui, methods)
    }

    #[test]
    fn archival_observation_uses_composer_and_rejects_cancel_or_epoch_change() {
        for mode in ["idle", "draft", "cancel", "epoch"] {
            let context = pane_agent_context();
            let cancellation = context.budget.cancellation.clone();
            let slot: CliSlot = Arc::default();
            let bump = slot.clone();
            let detection = detection_exchange(if mode == "draft" {
                CLAUDE_DRAFT
            } else {
                CLAUDE_EMPTY
            });
            let exchange: Exchange = Box::new(move |stream, request| {
                if mode == "cancel" {
                    assert_eq!(request["method"], "agent.read");
                    cancellation.cancel();
                    return; // Client cancellation may close before a reply is written.
                }
                if mode == "epoch" {
                    bump.get().unwrap().epoch.fetch_add(1, Ordering::AcqRel);
                }
                detection(stream, request);
            });
            let (socket, cli, worker) = serve_sequence(vec![pane_exchange_with("idle"), exchange]);
            let cli = Arc::new(cli);
            assert!(slot.set(cli.clone()).is_ok());
            let result =
                cli.observe_current_target_for_archival(&HostTargetId::new("w4:p1"), &context);
            worker.join().unwrap();
            fs::remove_file(socket).unwrap();
            if let Ok(sample) = &result {
                assert!(sample.0.occupant.is_none());
                assert_eq!(sample.0.execution, ExecutionEvidence::Unknown);
                let evidence = sample
                    .1
                    .as_ref()
                    .expect("same validated composer read retains parser evidence");
                assert_eq!(
                    evidence.parser,
                    registry::builtins().agent("claude").unwrap()
                );
                assert_eq!(evidence.reported_host_kind, "claude");
                assert_eq!(
                    evidence.classification,
                    if mode == "idle" {
                        ports::ComposerClassification::Empty
                    } else {
                        ports::ComposerClassification::Unsafe
                    }
                );
            }
            match mode {
                "idle" => assert_eq!(result.unwrap().0.ui, HostUiState::Idle),
                "draft" => assert_eq!(result.unwrap().0.ui, HostUiState::HumanInput),
                "cancel" => assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled),
                _ => assert_eq!(result.unwrap_err().code, ErrorCode::StaleHostObservation),
            }
        }
    }

    /// An ordinary wake's observation is one `pane.get`: no composer read for
    /// any status, so an unreadable composer cannot change or slow a wake
    /// (TRUST-POLICY A4, ht-1ip.55). Kills: the plain observation issuing
    /// `agent.read`, and a blocked pane losing its approval classification.
    #[test]
    fn ordinary_observation_makes_no_composer_read() {
        for (status, expected) in [
            ("idle", HostUiState::Unknown),
            ("done", HostUiState::Unknown),
            ("working", HostUiState::Unknown),
            ("blocked", HostUiState::ApprovalOrQuestion),
        ] {
            let (ui, methods) = observe_ui_as(false, status, "codex", vec![]);
            assert_eq!(ui, expected, "{status}");
            assert_eq!(methods, ["pane.get"], "{status}");
        }
    }

    /// Production-shaped observations (spike captures): the idle/draft/working
    /// split. Kills: reporting Unknown for a readable pane, and an idle agent
    /// with typed text reading as Idle.
    #[test]
    fn observation_classifies_idle_draft_and_working_panes() {
        let (ui, methods) = observe_ui("idle", "claude", vec![detection_exchange(CLAUDE_EMPTY)]);
        assert_eq!(ui, HostUiState::Idle);
        assert_eq!(methods, ["pane.get", "agent.read"]);
        let (ui, _) = observe_ui("idle", "claude", vec![detection_exchange(CLAUDE_DRAFT)]);
        assert_eq!(ui, HostUiState::HumanInput);
        // Suggestion-shaped text: same classification; the wake no longer refuses it.
        let (ui, _) = observe_ui(
            "idle",
            "claude",
            vec![detection_exchange(claude_screen(&[
                "Run herdr-threads summary thread-x once and stop.",
            ]))],
        );
        assert_eq!(ui, HostUiState::HumanInput);
        let (ui, _) = observe_ui("done", "codex", vec![detection_exchange(CODEX_EMPTY)]);
        assert_eq!(ui, HostUiState::Idle);
        let (ui, _) = observe_ui("working", "claude", vec![detection_exchange(CLAUDE_EMPTY)]);
        assert_eq!(ui, HostUiState::ActiveTurn);
        let (ui, _) = observe_ui("working", "claude", vec![detection_exchange(CLAUDE_DRAFT)]);
        assert_eq!(ui, HostUiState::Unknown);
    }

    /// A failed detection read leaves the observation intact with Unknown.
    #[test]
    fn observation_survives_a_failed_composer_read_as_unknown() {
        let (ui, methods) = observe_ui("idle", "claude", vec![failing_detection_exchange()]);
        assert_eq!(ui, HostUiState::Unknown);
        assert_eq!(methods, ["pane.get", "agent.read"]);
        let (ui, _) = observe_ui(
            "idle",
            "claude",
            vec![detection_exchange("no composer on this screen\n")],
        );
        assert_eq!(ui, HostUiState::Unknown);
    }

    /// Blocked needs no composer read, and an unrecognized agent or status is
    /// never read. Kills: a detection read for every pane.
    #[test]
    fn blocked_and_unrecognized_panes_make_no_composer_read() {
        let (ui, methods) = observe_ui("blocked", "claude", vec![]);
        assert_eq!(ui, HostUiState::ApprovalOrQuestion);
        assert_eq!(methods, ["pane.get"]);
        let (ui, methods) = observe_ui("idle", "pi", vec![]);
        assert_eq!(ui, HostUiState::Unknown);
        assert_eq!(methods, ["pane.get"]);
        let (ui, methods) = observe_ui("unknown", "claude", vec![]);
        assert_eq!(ui, HostUiState::Unknown);
        assert_eq!(methods, ["pane.get"]);
    }

    /// Serves ping+operation exchanges, answering each request through
    /// `answer_for` until it returns `true` (the last exchange); every wire
    /// request is recorded.
    fn serve_until<F>(mut answer_for: F) -> (PathBuf, NativeCli, thread::JoinHandle<Vec<Value>>)
    where
        F: FnMut(&mut UnixStream, &Value) -> bool + Send + 'static,
    {
        let socket =
            crate::test_support::scratch::temp_path(format!("ht-start-{}", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&socket).unwrap();
        let worker = thread::spawn(move || {
            let mut wires = Vec::new();
            loop {
                let (mut ping, _) = listener.accept().unwrap();
                let request = read(&mut ping);
                answer(
                    &mut ping,
                    &request,
                    json!({"type":"pong","version":"0.9.1","protocol":22}),
                );
                drop(ping);
                let (mut operation, _) = listener.accept().unwrap();
                let wire = read(&mut operation);
                let last = answer_for(&mut operation, &wire);
                wires.push(wire);
                if last {
                    return wires;
                }
            }
        });
        let cli = NativeCli::new(socket.clone(), Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        (socket, cli, worker)
    }

    /// The pending `agent.start` answer Herdr 0.9.1 gives before the agent
    /// is detected (no `agent` field, `launch_pending`).
    fn pending_agent(name: &str) -> Value {
        json!({"agent_status":"unknown","launch_pending":true,"name":name,
            "pane_id":"w4:p1","terminal_id":"term_1","focused":false,"revision":0,
            "tab_id":"w4:t1","workspace_id":"w4"})
    }

    /// Legacy and process-hint starts share the same refusal retry boundary.
    /// Kills: returning the first not-yet-interactive-shell refusal to the caller.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn new_pane_readiness_retries_both_native_transports_before_submission() {
        for process_hint in [false, true] {
            let (mut request, mut context, mut observation) = launch_fixture();
            let registry = crate::harness::launch::tests::process_hint_registry(true);
            if process_hint {
                request.harness = Harness::Agent(registry.agent("hinted").unwrap());
                request.process_hint = true;
            }
            let name = request.agent_name();
            let mut starts = 0;
            let mut host = HintSocketFixture::new(move |stream, wire| {
                match wire["method"].as_str().unwrap() {
                    "ping" => answer(
                        stream,
                        wire,
                        json!({"type":"pong","version":"0.9.1","protocol":22,"capabilities":{"agent_start_process_hint_v1":true}}),
                    ),
                    "agent.start" => {
                        starts += 1;
                        assert_eq!(wire["params"]["name"], name);
                        assert_eq!(wire["params"]["pane_id"], "w4:p1");
                        if starts == 1 {
                            refuse(stream, wire, "agent_pane_busy");
                        } else {
                            answer(stream, wire, started(name.clone()));
                        }
                    }
                    "pane.get" => answer(stream, wire, json!({"type":"pane_info"})),
                    "pane.process_info" => answer(stream, wire, process_info_fixture()),
                    other => panic!("unexpected operation: {other}"),
                }
            });
            if process_hint {
                host.cli.registry = registry;
                host.bind_preflight(&mut request, &mut context, &mut observation);
            }
            let result = host
                .cli
                .guarded_start_with_evidence(&request, &context, &observation);
            let frames = host.finish();
            let outcome = result.expect("new pane must become launchable");
            assert!(matches!(
                outcome,
                NativeLaunchOutcome::ObservedStartup { .. }
            ));
            assert_eq!(
                frames
                    .iter()
                    .filter(|wire| wire["method"] == "agent.start")
                    .count(),
                2
            );
        }
    }

    /// A permanently busy target must remain a clean, bounded pre-start refusal.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn new_pane_readiness_exhaustion_remains_not_submitted() {
        let (request, context, observation) = launch_fixture();
        let host = HintSocketFixture::new(|stream, wire| match wire["method"].as_str().unwrap() {
            "ping" => answer(
                stream,
                wire,
                json!({"type":"pong","version":"0.9.1","protocol":22}),
            ),
            "agent.start" => refuse(stream, wire, "agent_pane_busy"),
            other => panic!("unexpected operation: {other}"),
        });
        let started = Instant::now();
        let failure = host
            .cli
            .guarded_start_with_evidence(&request, &context, &observation)
            .expect_err("persistent refusal must not claim a possible start");
        let elapsed = started.elapsed();
        let frames = host.finish();
        assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
        assert_eq!(failure.error.code, ErrorCode::TargetUnsafe);
        assert!(failure.error.detail.contains("agent_pane_busy"));
        assert!(
            frames
                .iter()
                .filter(|wire| wire["method"] == "agent.start")
                .count()
                > 1
        );
        assert!(
            elapsed < Duration::from_secs(4),
            "retry exceeded bounded window: {elapsed:?}"
        );
    }

    #[test]
    fn handoff_native_confirmed_refusal_has_not_submitted_evidence() {
        let (request, mut context, observation) = launch_fixture();
        let (socket, mut cli, worker) =
            fixture(|stream, wire| refuse(stream, &wire, "agent_pane_busy"));
        cli.clock = Arc::new(FrozenClock);
        context.budget.deadline = MonoInstant(3_100);
        let failure = cli
            .guarded_start_with_evidence(&request, &context, &observation)
            .unwrap_err();
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        assert_eq!(failure.submission, ports::NativeSubmission::NotSubmitted);
    }

    /// Live Herdr 0.9.1 (2026-10-01, stand-in `codex exec --no-daemon`
    /// exiting 2): the agent record stays `launch_pending` with no detected
    /// `agent` and the pane returns to its shell prompt. Kills: waiting out
    /// the whole window and returning `OutcomeUnknown` (rc 5) for a harness
    /// that already refused its arguments; and treating a pane whose last
    /// line is not a prompt (the harness still running) as an exit.
    #[test]
    fn codex_early_exit_inside_the_observation_window_is_reported() {
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        let started_at = Instant::now();
        let get_name = name.clone();
        let (socket, cli, worker) =
            serve_until(move |stream, wire| match wire["method"].as_str().unwrap() {
                "agent.start" => {
                    let mut result = started(name.clone());
                    result["agent"] = pending_agent(&name);
                    answer(stream, wire, result);
                    false
                }
                "agent.get" => {
                    assert_eq!(wire["params"], json!({"target": get_name}));
                    answer(
                        stream,
                        wire,
                        json!({"type":"agent_info","agent":pending_agent(&get_name)}),
                    );
                    false
                }
                "pane.read" => {
                    assert_eq!(wire["params"]["pane_id"], "w4:p1");
                    answer(
                        stream,
                        wire,
                        json!({"type":"pane_read","read":{"pane_id":"w4:p1","text":
                            "me@host r % codex --no-daemon --model x\nerror: unexpected \
                             argument '--no-daemon' found\nme@host r %\n"}}),
                    );
                    true
                }
                other => panic!("unexpected {other}"),
            });
        let failure = cli
            .guarded_start_with_evidence(&request, &context, &observation)
            .unwrap_err();
        assert_eq!(failure.submission, ports::NativeSubmission::Possible);
        let refusal = failure.error;
        assert_eq!(refusal.code, ErrorCode::InvalidRequest);
        assert!(
            refusal.detail.contains("codex exited"),
            "{}",
            refusal.detail
        );
        assert!(
            refusal
                .detail
                .contains("unexpected argument '--no-daemon' found"),
            "{}",
            refusal.detail
        );
        assert!(started_at.elapsed() < Duration::from_secs(10));
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();

        // A pane still showing the running harness (last line no prompt) is
        // not an exit: the poll goes on and the agent's detection ends it.
        assert_eq!(
            shell_prompt_returned("me@host r % codex\nloading models...", "codex"),
            None
        );
        assert_eq!(shell_prompt_returned("me@host r %", "codex"), None);
        assert_eq!(shell_prompt_returned("", "codex"), None);
        assert!(shell_prompt_returned("r % codex exec\nboom\nr %", "codex").is_some());
    }

    /// Live 2026-10-10 against Herdr 0.9.1: `pane.process_info` answers
    /// `{"result":{"process_info":{"shell_pid":N,"foreground_processes":[{"pid":N,"name",
    /// "argv0","argv","cmdline","cwd"}]}}}`. The managed-launch placeholder fingerprints the one
    /// foreground process that is not the shell. Kills: a fingerprint taken from the shell, from
    /// one of several candidate processes, or from a zero or oversized pid.
    #[test]
    fn started_process_is_the_single_non_shell_foreground_process() {
        let info = |shell: u64, pids: &[u64]| {
            serde_json::json!({"result":{"type":"pane_process_info","process_info":{
                "shell_pid":shell,
                "foreground_processes":pids.iter().map(|pid| serde_json::json!({
                    "pid":pid,"name":"p","argv0":"p","argv":["p"],"cmdline":"p"
                })).collect::<Vec<_>>()
            }}})
        };
        assert_eq!(started_process_pid(&info(10, &[20])), Some(20));
        assert_eq!(started_process_pid(&info(10, &[10, 20])), Some(20));
        assert_eq!(
            started_process_pid(&info(10, &[10])),
            None,
            "only the shell"
        );
        assert_eq!(started_process_pid(&info(10, &[])), None);
        assert_eq!(started_process_pid(&info(10, &[20, 21])), None, "ambiguous");
        assert_eq!(started_process_pid(&info(10, &[0])), None);
        assert_eq!(
            started_process_pid(&info(10, &[u64::from(u32::MAX) + 1])),
            None
        );
        assert_eq!(started_process_pid(&serde_json::json!({"error":{}})), None);
    }

    /// Live 2026-10-11 (ht-54r.89, evidence/ht-54r.89): `codex` installed with npm is a `node`
    /// launcher plus its native child in one foreground process group (and a short-lived `git`
    /// helper while it starts), so the pane's foreground is a set. The composer wake matches a
    /// binding's fingerprint pid against the whole set. Kills: reading the set as `None` when
    /// it has several members, keeping the shell, or keeping a zero or oversized pid.
    #[test]
    fn foreground_process_pids_are_every_non_shell_foreground_process() {
        let info = |shell: u64, pids: &[u64]| {
            serde_json::json!({"result":{"type":"pane_process_info","process_info":{
                "shell_pid":shell,
                "foreground_processes":pids.iter().map(|pid| serde_json::json!({
                    "pid":pid,"name":"p","argv0":"p","argv":["p"],"cmdline":"p"
                })).collect::<Vec<_>>()
            }}})
        };
        assert_eq!(foreground_process_pids(&info(10, &[20, 21])), vec![20, 21]);
        assert_eq!(
            foreground_process_pids(&info(10, &[10, 20, 21])),
            vec![20, 21],
            "the shell is not a candidate"
        );
        assert_eq!(foreground_process_pids(&info(10, &[20])), vec![20]);
        assert_eq!(foreground_process_pids(&info(10, &[10])), Vec::<u32>::new());
        assert_eq!(
            foreground_process_pids(&info(10, &[0, 20, u64::from(u32::MAX) + 1])),
            vec![20],
            "zero and overflowing pids are dropped"
        );
        assert_eq!(
            foreground_process_pids(&serde_json::json!({"error":{}})),
            Vec::<u32>::new()
        );
    }

    /// Live 2026-10-01 against Herdr 0.9.1 (stand-in `codex` through
    /// idle and working): `agent.list` kept the `agent.start` name; no
    /// launch-side call clears it. Kills: any later launch call (start
    /// retry, status poll, pane read) naming a different agent than the
    /// one launched.
    #[test]
    fn launch_name_survives_agent_state_updates() {
        let (request, context, observation) = launch_fixture();
        let name = request.agent_name();
        let mut polls = 0;
        let start_name = name.clone();
        let (socket, cli, worker) =
            serve_until(move |stream, wire| match wire["method"].as_str().unwrap() {
                "agent.start" => {
                    let mut result = started(start_name.clone());
                    result["agent"] = pending_agent(&start_name);
                    answer(stream, wire, result);
                    false
                }
                "agent.get" => {
                    polls += 1;
                    let agent = if polls < 2 {
                        pending_agent(&start_name)
                    } else {
                        started(start_name.clone())["agent"].clone()
                    };
                    answer(stream, wire, json!({"type":"agent_info","agent":agent}));
                    polls >= 2
                }
                other => panic!("unexpected {other}"),
            });
        assert!(matches!(
            cli.guarded_start_cli(&request, &context, &observation)
                .unwrap(),
            NativeLaunchOutcome::ObservedStartup { .. }
        ));
        let wires = worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        assert!(wires.len() >= 3);
        for wire in &wires {
            let params = &wire["params"];
            let named = params.get("name").or_else(|| params.get("target"));
            assert_eq!(
                named.and_then(Value::as_str).unwrap_or(&name),
                name,
                "{wire}"
            );
        }
    }
    #[test]
    fn participant_locations_snapshot_is_witnessed_once_and_advisory_failure_keeps_epoch() {
        let (socket, cli, worker) = serve_until(|stream, wire| {
            assert_eq!(wire["method"], "session.snapshot");
            answer(
                stream,
                wire,
                json!({"type":"session_snapshot","snapshot":{
                    "version":"0.9.1","protocol":22,"agents":[],"layouts":[],
                    "workspaces":[{"workspace_id":"w4","label":"Space"}],"tabs":[{"tab_id":"w4:t1","label":"Tab"}],
                    "panes":[{"pane_id":"w4:p1","terminal_id":"term_1","workspace_id":"w4","tab_id":"w4:t1","focused":false,"agent_status":"idle","revision":1,"label":"Pane"}]
                }}),
            );
            true
        });
        let budget = CallBudget {
            deadline: MonoInstant(10000),
            cancellation: Cancellation::default(),
        };
        let outcome = cli.participant_labels(&budget);
        let wires = worker.join().unwrap();
        fs::remove_file(&socket).unwrap();
        let labels = outcome.unwrap();
        assert_eq!(wires.len(), 1);
        assert_eq!(labels[0].terminal, "term_1");
        if crate::host::continuity::PEER_WITNESS_SUPPORTED {
            assert!(
                labels[0]
                    .incarnation
                    .as_deref()
                    .is_some_and(|identity| identity.starts_with("herdr-server:pid="))
            );
        }
        let epoch = cli.epoch();
        assert!(cli.participant_labels(&budget).is_err());
        assert_eq!(
            cli.epoch(),
            epoch,
            "advisory failure must not invalidate continuity"
        );
    }
    include!("../../tests/host/bootstrap_recovery.rs");
}
