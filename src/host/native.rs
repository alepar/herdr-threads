//! Explicit Herdr API endpoint adapter with an owned, joined socket exchange.

use super::continuity::LocalEndpointWitness;
use super::observation::{
    NativePane, NativeSnapshot, normalize_pane, normalize_snapshot, structured_host_error,
};
use crate::harness::composer::{self, ComposerRead};
use crate::ports::{
    self, ComposerStash, CorrelatedStartup, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
    HostCallContext, HostObservation, HostPort, HostSnapshot, HostUiState, IncarnationEvidence,
    NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, ObservationProvenance,
    SafeWakeTarget, StructuralOccupancy, WakeTargetBasis,
};
use crate::protocol::{
    authority::Harness,
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, TerminalId},
    results::{ApiError, CapabilityState, ErrorCode},
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
const CONFIRMED_PRESTART_REFUSALS: [&str; 2] = ["agent_pane_busy", "agent_name_taken"];
/// Interval between bounded readiness polls after a submitted start.
const START_POLL_MILLIS: u64 = 250;
/// Polling time before a started-but-undetected agent is checked for an early
/// exit (the pane back at its shell prompt). Live Herdr 0.9.1 keeps such an
/// agent `launch_pending` with no detected `agent` forever.
const EARLY_EXIT_CHECK_MILLIS: u64 = 1_000;
/// Lines of the pane read for the early-exit check and quoted in its refusal.
const EARLY_EXIT_PANE_LINES: usize = 12;
/// Herdr agent kinds a wake prompt may reach.
const WAKE_AGENTS: [&str; 2] = ["claude", "codex"];
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
/// The key that deletes the composer's text one row or line at a time
/// (poke spike Q3).
const COMPOSER_CLEAR_KEY: &str = "ctrl+u";
/// Clears beyond one per composer line before a stash gives up.
const COMPOSER_CLEAR_SLACK: usize = 2;
/// Settle time before the composer is read back after a send, so the harness
/// has cleared its composer when the prompt was accepted.
const COMPOSER_SETTLE_MILLIS: u64 = 250;
/// A sent wake prompt still sitting in the composer is among the last lines of
/// the pane; a submitted one is pushed above the empty composer box.
const COMPOSER_TAIL_LINES: usize = 4;

const FOCUSED_EMPTY_MILLIS: u64 = 60_000;
const MAX_EMPTY_WINDOWS: usize = 1_024;

struct EmptyComposerWindow {
    target: SafeWakeTarget,
    since: u64,
    last: u64,
}

pub struct NativeCli {
    socket: PathBuf,
    clock: Arc<dyn Clock>,
    epoch: AtomicU64,
    /// Observation order within one (boot, epoch). Never reused by this adapter.
    sequence: AtomicU64,
    empty_windows: Mutex<HashMap<SeatId, EmptyComposerWindow>>,
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

impl NativeCli {
    /// Herdr's guarded start (`agent.start`): Herdr itself checks that the
    /// pane is at its interactive shell prompt before it starts the agent, and
    /// answers only after it detects the expected agent ready in the same
    /// terminal. `preflight` is this adapter's fresh witnessed read.
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
        let kind = match request.harness {
            Harness::Codex => "codex",
            Harness::Claude => "claude",
            Harness::Human => {
                return Err(before_start(
                    ErrorCode::InvalidRequest,
                    "a human occupant is never launched",
                ));
            }
        };
        // The readable name first; when Herdr refuses it as taken by another
        // live agent (checked before anything is typed), one retry with a
        // short seat suffix. Never more than these two submissions.
        let [first_name, retry_name] = request.agent_name_candidates();
        let mut name = first_name;
        let (name, parsed, remaining, started, submitted_at_mono) = loop {
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
            let refusal = match self.run(&args, &context.budget, Duration::from_millis(remaining)) {
                // Herdr checks both before it types anything into the pane
                // (live 2026-09-30: an occupied pane and a seat whose agent
                // still runs were refused with the pane unchanged).
                Err(error)
                    if error.code == ErrorCode::TargetUnsafe
                        && CONFIRMED_PRESTART_REFUSALS
                            .iter()
                            .any(|code| error.detail.starts_with(&format!("Herdr {code}:"))) =>
                {
                    error
                }
                Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                Ok(raw) => {
                    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
                        Ok(value) => value,
                        Err(_) => return Ok(NativeLaunchOutcome::OutcomeUnknown),
                    };
                    let Some(host_error) = structured_host_error(&parsed) else {
                        break (name, parsed, remaining, started, submitted_at_mono);
                    };
                    if !parsed
                        .pointer("/error/code")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|code| CONFIRMED_PRESTART_REFUSALS.contains(&code))
                    {
                        return Ok(NativeLaunchOutcome::OutcomeUnknown);
                    }
                    host_error
                }
            };
            if refusal.detail.starts_with("Herdr agent_name_taken:")
                && name != retry_name
                && !context.budget.cancellation.is_cancelled()
            {
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
            let ready = (current
                .get("interactive_ready")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                || matches!(status, Some("working" | "done")))
                // Herdr reports the detected agent label as `agent`.
                && current.get("agent").and_then(serde_json::Value::as_str) == Some(kind);
            // Herdr keeps `launch_pending` set until the agent is first
            // interactive_ready, so an agent already working on its initial
            // prompt (live demo: working, launch_pending true) is started too.
            let working = matches!(status, Some("working" | "done"))
                && current.get("agent").and_then(serde_json::Value::as_str) == Some(kind);
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
            let polled = match self.run(&["agent", "get", name.as_str()], &context.budget, limit) {
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
        let correlation = CorrelatedStartup {
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
        };
        Ok(NativeLaunchOutcome::ObservedStartup {
            correlation,
            diagnostic: observed,
        })
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
        Self {
            socket,
            clock,
            epoch: AtomicU64::new(1),
            sequence: AtomicU64::new(1),
            empty_windows: Mutex::new(HashMap::new()),
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
            cfg!(target_os = "macos"),
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
        self.dispatch(args, budget, limit, false, true)
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
        self.dispatch(args, budget, limit, false, false)
            .map(|(body, _)| body)
    }

    /// Identity reads carry the server-process witness of their own response
    /// connections where the platform supports it (macOS). Elsewhere the
    /// witness is absent and incarnation stays Unknown.
    fn run_witnessed(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
    ) -> Result<(String, Option<LocalEndpointWitness>), ApiError> {
        self.dispatch(args, budget, limit, cfg!(target_os = "macos"), true)
    }

    fn dispatch(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
        witnessed: bool,
        fenced: bool,
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
            ["api", "snapshot"] => ("session.snapshot", serde_json::json!({})),
            ["agent", "get", target] => ("agent.get", serde_json::json!({"target":target})),
            ["agent", "read", target, "--source", source] => (
                "agent.read",
                serde_json::json!({"target":target,"source":source}),
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
        let outcome = if witnessed {
            super::transport::request_witnessed(
                &self.socket,
                &id,
                method,
                params,
                self.clock.as_ref(),
                budget,
                limit,
            )
            .map(|response| (response.body, Some(response.witness)))
        } else {
            super::transport::request(
                &self.socket,
                &id,
                method,
                params,
                self.clock.as_ref(),
                budget,
                limit,
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
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        // Herdr 0.9.1 `agent.start` checks shell ownership and prompt
        // readiness itself. The same-incarnation preflight needs the kernel
        // peer witness, which exists only on macOS; elsewhere every
        // observation is unverified and launch stays unsupported.
        if cfg!(target_os = "macos") {
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

    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ports::ComposerObservation, ApiError> {
        let started = Instant::now();
        let observation = self.observe_target(target, context, true)?;
        // Composer I/O happens after pane.get's first fence. A concurrent host
        // failure or cancellation during that read must not earn idle evidence.
        self.check_epoch(observation.epoch)?;
        self.check_context(context)?;
        self.check_after_parse_unfenced(&context.budget, started, Duration::from_secs(5))?;
        Ok(ports::ComposerObservation(observation))
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
        self.submit_prompt_mode(target, text, context)
    }

    /// Retained port compatibility; attention now requires idle/done even
    /// through this entry point. Historical turn-time recipes are not admission.
    fn submit_prompt_during_turn(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<ports::PromptOutcome, ApiError> {
        self.submit_prompt_mode(target, text, context)
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
        let Some(harness) = target.bound_harness.as_deref().and_then(bound_harness) else {
            return Ok(ComposerStash::Failed("no bound harness".into()));
        };
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
        self.composer_call(
            &["pane", "send-text", target.target.as_str(), saved],
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

    /// Cooperative safe prompt needs the kernel peer witness (macOS) for its
    /// same-incarnation recheck and submission.
    fn safe_prompt_capability(&self) -> CapabilityState {
        if cfg!(target_os = "macos") {
            CapabilityState::Supported
        } else {
            CapabilityState::Unsupported
        }
    }

    fn incarnation_witness(&self) -> crate::protocol::results::CapabilityState {
        // The kernel peer PID/start-time witness exists only on macOS; other
        // platforms report Unknown incarnation on every read.
        if cfg!(target_os = "macos") {
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
) -> Result<(), String> {
    let text = |name: &str| agent.get(name).and_then(serde_json::Value::as_str);
    if text("pane_id") != Some(target.target.as_str())
        || text("terminal_id") != Some(target.terminal.as_str())
    {
        return Err("agent is not in the target terminal".into());
    }
    let kind = text("agent");
    if !kind.is_some_and(|kind| WAKE_AGENTS.contains(&kind)) {
        return Err(format!(
            "no recognized harness agent (agent {})",
            kind.unwrap_or("none").chars().take(32).collect::<String>()
        ));
    }
    match target.bound_harness.as_deref() {
        Some(bound) if kind != Some(bound) => {
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

    fn submit_prompt_mode(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<ports::PromptOutcome, ApiError> {
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
        if let Err(detail) = cooperative_wake_ready(&agent, target) {
            return Err(refuse(&detail));
        }
        let harness = target
            .bound_harness
            .as_deref()
            .and_then(bound_harness)
            .ok_or_else(|| refuse("no bound harness for composer read"))?;
        let focused = agent
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| refuse("unreadable pane focus"))?;
        if self.read_composer(&target.target, harness, context)? != ComposerRead::Empty {
            return Err(refuse("composer is nonempty or unreadable"));
        }
        self.check_epoch(target.epoch)?;
        self.check_context(context)?;
        if !self.focused_empty_ready(target, focused, previous) {
            return Err(refuse(
                "focused composer has not been observed empty for one minute",
            ));
        }
        if budget.cancellation.is_cancelled() {
            return Err(error(ErrorCode::Cancelled, "wake prompt cancelled"));
        }
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
            // has nowhere to type); an invalid request never left us.
            Err(failure)
                if matches!(
                    failure.code,
                    ErrorCode::TargetUnsafe | ErrorCode::NotFound | ErrorCode::InvalidRequest
                ) =>
            {
                return Err(refuse(&failure.detail));
            }
            Err(_) => return Ok(ports::PromptOutcome::OutcomeUnknown),
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
            Ok(ports::PromptOutcome::Submitted)
        } else {
            Ok(ports::PromptOutcome::OutcomeUnknown)
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
        let ui = if composer_ui {
            self.observed_ui(&pane, context)
        } else {
            status_ui(&pane)
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
        Ok(observation)
    }

    /// The UI state a pane shows: Herdr's agent status plus, for an idle,
    /// done or working claude/codex agent, a composer read. A failed or
    /// timed-out read never fails the observation; it yields `Unknown`.
    fn observed_ui(&self, pane: &NativePane, context: &HostCallContext) -> HostUiState {
        let Some(harness) = pane.agent.as_deref().and_then(bound_harness) else {
            return HostUiState::Unknown;
        };
        let status = pane.status.as_str();
        match status {
            "blocked" => composer::observed_ui(Some(status), None),
            "idle" | "done" | "working" => {
                let read = self.read_composer(&pane.target, harness, context).ok();
                composer::observed_ui(Some(status), read.as_ref())
            }
            _ => HostUiState::Unknown,
        }
    }

    /// One bounded `agent read --source detection`, composer-parsed. The pane
    /// width is not in Herdr's pane record; Claude's rule length stands in.
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
        Ok(composer::read_composer(harness, text, None))
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
        let attempts = text.lines().count() + COMPOSER_CLEAR_SLACK;
        for _ in 0..attempts {
            let cleared = self
                .composer_call(
                    &[
                        "pane",
                        "send-keys",
                        target.target.as_str(),
                        COMPOSER_CLEAR_KEY,
                    ],
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

/// The harness a Herdr agent kind or a seat's bound harness names, when its
/// composer is readable.
fn bound_harness(kind: &str) -> Option<Harness> {
    match kind {
        "claude" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        _ => None,
    }
}

/// The bound for one composer call: the ceiling or what the budget has left.
/// The UI state Herdr's `agent_status` alone shows: an open approval or
/// question is `blocked`; everything else is `Unknown`, as on every ordinary
/// wake before the composer reader existed.
fn status_ui(pane: &NativePane) -> HostUiState {
    if pane.agent.as_deref().and_then(bound_harness).is_some() && pane.status == "blocked" {
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
    if normalize(&composer).contains(&normalize(crate::notification::policy::MARKER)) {
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
mod tests {
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
        let socket = std::env::temp_dir().join(format!("ht-start-{}", uuid::Uuid::new_v4()));
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
            seat: SeatId::new("seat_1"),
            target: target.clone(),
            harness: Harness::Codex,
            argv: vec!["--no-daemon".into(), "--model".into(), "test model".into()],
            configured_hook: ConfiguredHook {
                scope: "project".into(),
                path: "/tmp/hook".into(),
                fingerprint: "sha256:test".into(),
            },
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
        let missing = std::env::temp_dir().join(format!("ht-absent-{}", uuid::Uuid::new_v4()));
        let cli = NativeCli::new(missing, Arc::new(TestClock(Instant::now())));
        cli.epoch.store(3, Ordering::Release);
        let failure = cli
            .pane_agent_within(&target, &pane_agent_context(), limit)
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::HostUnavailable);
        assert_epoch_untouched(&cli, 3);
        // Parse-time overrun: the reply arrived after the limit.
        let cli = NativeCli::new(
            std::env::temp_dir().join("unused"),
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
        let missing = std::env::temp_dir().join(format!("ht-absent-{}", uuid::Uuid::new_v4()));
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
    type Exchange = Box<dyn FnOnce(&mut UnixStream, Value) + Send>;
    /// Serves one ping+operation exchange per entry of `responses`.
    fn serve_sequence(responses: Vec<Exchange>) -> (PathBuf, NativeCli, thread::JoinHandle<()>) {
        let socket = std::env::temp_dir().join(format!("ht-start-{}", uuid::Uuid::new_v4()));
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
            let (request, context, observation) = launch_fixture();
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
            let (socket, cli, worker) = serve_sequence(exchanges);
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
            request.seat = SeatId::new("sk3Fq9a2B");
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
        request.seat = SeatId::new("sk3Fq9a2B");
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
            let socket = std::env::temp_dir().join(format!("ht-no-start-{}", uuid::Uuid::new_v4()));
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
        let result = cli.submit_prompt(&target, "wake marker", &context);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        (result, methods)
    }

    /// Catches an ordinary native wake bypassing the composer and submitting a draft.
    #[cfg(target_os = "macos")]
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
            let (result, methods) = cooperative_wake(vec![
                recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
                screen,
            ]);
            assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.read"]
            );
        }
    }

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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
            } else {
                assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted);
            }
        }
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

    /// Idle and done agents of a recognized harness are prompted only after
    /// a fresh recheck, and `Submitted` is transport submission only.
    #[cfg(target_os = "macos")]
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

    /// TRUST-POLICY A4 wake rule: the pane's detected agent kind must equal
    /// the seat's bound harness; a different kind (both directions) or no
    /// detected kind refuses before any prompt, a matching kind is prompted.
    #[cfg(target_os = "macos")]
    #[test]
    fn cooperative_wake_refuses_claude_bound_seat_with_codex_agent() {
        bound_harness_wake_refused("claude", Some("codex"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cooperative_wake_refuses_codex_bound_seat_with_claude_agent() {
        bound_harness_wake_refused("codex", Some("claude"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cooperative_wake_refuses_codex_bound_seat_without_detected_agent() {
        bound_harness_wake_refused("codex", None);
    }

    /// A cooperative target with no bound harness is refused, never compared
    /// with the detected agent kind as "any recognized kind".
    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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

        let log = std::env::temp_dir().join(format!("ht-restart-log-{}", uuid::Uuid::new_v4()));
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
            std::env::temp_dir().join(format!("ht-absent-{}", uuid::Uuid::new_v4())),
            Arc::new(TestClock(Instant::now())),
        );
        cli.epoch.store(3, Ordering::Release);
        let seat = SeatId::new("seat_1");
        let target = cli.safe_wake_target(&seat, &observation);
        assert_eq!(target.is_some(), cfg!(target_os = "macos"));
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
            cfg!(target_os = "macos"),
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
    #[cfg(target_os = "macos")]
    fn poke_session<T>(
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
        let (socket, cli, worker) = serve_sequence(exchanges);
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    #[test]
    fn stash_clears_until_the_composer_reads_empty() {
        let (stash, methods) = poke_session(
            vec![
                clear_exchange(),
                detection_exchange(claude_screen(&["line one"])),
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
    #[test]
    fn clear_not_verified_aborts() {
        let mut exchanges = vec![];
        for _ in 0..(1 + COMPOSER_CLEAR_SLACK) {
            exchanges.push(clear_exchange());
            exchanges.push(detection_exchange(CLAUDE_DRAFT));
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
    #[cfg(target_os = "macos")]
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

    /// Claude text may be a prompt suggestion, so the stash refuses before
    /// any key (the poke is skipped for that poke only).
    #[cfg(target_os = "macos")]
    #[test]
    fn claude_text_fails_stash_before_any_key() {
        for screen in [
            CLAUDE_DRAFT.to_owned(),
            claude_screen(&["herdr-threads pending-receipts"]),
        ] {
            let (stash, methods) =
                poke_session(vec![detection_exchange(screen)], |cli, target, context| {
                    cli.stash_composer(target, context)
                });
            assert_eq!(
                stash.unwrap(),
                ComposerStash::Failed(composer::CLAUDE_NOT_KNOWN_EMPTY.into())
            );
            assert_eq!(methods, ["pane.get", "agent.read"], "no pane.send_keys");
        }
    }

    /// Findings Q4: a retyped image placeholder comes back as literal text, so
    /// the stash refuses before sending any key. Kills: stashing past it.
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
    #[cfg(target_os = "macos")]
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
        let socket = std::env::temp_dir().join(format!("ht-start-{}", uuid::Uuid::new_v4()));
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

    #[test]
    fn handoff_native_confirmed_refusal_has_not_submitted_evidence() {
        let (request, context, observation) = launch_fixture();
        let (socket, cli, worker) =
            fixture(|stream, wire| refuse(stream, &wire, "agent_pane_busy"));
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
        if cfg!(target_os = "macos") {
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
}
