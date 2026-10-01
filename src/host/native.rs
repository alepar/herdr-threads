//! Explicit Herdr API endpoint adapter with an owned, joined socket exchange.

use super::continuity::LocalEndpointWitness;
use super::observation::{
    NativePane, NativeSnapshot, normalize_pane, normalize_snapshot, structured_host_error,
};
use crate::ports::{
    self, CorrelatedStartup, EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostCallContext,
    HostLifecycleSubscription, HostObservation, HostPort, HostSnapshot, HostUiState,
    IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
    ObservationProvenance, SafeWakeTarget, StructuralOccupancy, WakeTargetBasis,
};
use crate::protocol::{
    authority::Harness,
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, TerminalId},
    results::{ApiError, CapabilityState, ErrorCode},
    time::{CallBudget, Clock},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static CALL_ID: AtomicU64 = AtomicU64::new(1);
/// Herdr `agent.start` errors that are checked before anything is typed.
const CONFIRMED_PRESTART_REFUSALS: [&str; 2] = ["agent_pane_busy", "agent_name_taken"];
/// Interval between bounded readiness polls after a submitted start.
const START_POLL_MILLIS: u64 = 250;
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

pub struct NativeCli {
    socket: PathBuf,
    clock: Arc<dyn Clock>,
    epoch: AtomicU64,
    /// Observation order within one (boot, epoch). Never reused by this adapter.
    sequence: AtomicU64,
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
    fn guarded_start_cli(
        &self,
        request: &NativeLaunchRequest,
        context: &HostCallContext,
        preflight: &HostObservation,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        request
            .validate()
            .map_err(|detail| error(ErrorCode::InvalidRequest, detail))?;
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
            return Err(error(
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
            return Err(error(ErrorCode::TargetUnsafe, "target is known busy"));
        }
        if context.budget.cancellation.is_cancelled() {
            return Err(error(
                ErrorCode::Cancelled,
                "native start cancelled before submission",
            ));
        }
        let kind = match request.harness {
            Harness::Codex => "codex",
            Harness::Claude => "claude",
            Harness::Human => {
                return Err(error(
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
                return Err(error(
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
            return Err(refusal);
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
            std::thread::sleep(Duration::from_millis(START_POLL_MILLIS));
            let left = Duration::from_millis(remaining).saturating_sub(started.elapsed());
            let limit = left.min(Duration::from_millis(750));
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
    pub fn pane_names(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::PaneName>, ApiError> {
        let raw = self.run(&["api", "snapshot"], budget, Duration::from_secs(2))?;
        crate::host::observation::normalize_pane_names(&raw)
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
        self.dispatch(args, budget, limit, false)
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
        self.dispatch(args, budget, limit, cfg!(target_os = "macos"))
    }

    fn dispatch(
        &self,
        args: &[&str],
        budget: &CallBudget,
        limit: Duration,
        witnessed: bool,
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
            ["pane", "get", target] => ("pane.get", serde_json::json!({"pane_id":target})),
            ["api", "snapshot"] => ("session.snapshot", serde_json::json!({})),
            ["agent", "get", target] => ("agent.get", serde_json::json!({"target":target})),
            ["agent", "prompt", target, text] => (
                "agent.prompt",
                serde_json::json!({"target":target,"text":text}),
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
        if outcome.as_ref().is_err_and(|error| {
            matches!(
                error.code,
                ErrorCode::Cancelled
                    | ErrorCode::DeadlineExceeded
                    | ErrorCode::HostUnavailable
                    | ErrorCode::StaleHostObservation
            )
        }) {
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
        if budget.cancellation.is_cancelled() {
            self.epoch.fetch_add(1, Ordering::AcqRel);
            return Err(error(
                ErrorCode::Cancelled,
                "host call cancelled during parse",
            ));
        }
        if started.elapsed() >= limit || self.clock.monotonic_now() >= budget.deadline {
            self.epoch.fetch_add(1, Ordering::AcqRel);
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "host call expired during parse",
            ));
        }
        Ok(())
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
        let sequence = self.next_sequence();
        Ok(self.observation(
            pane,
            epoch,
            started,
            incarnation.as_ref(),
            ObservationProvenance::FreshCurrentTarget,
            EvidenceKind::NativeCurrentTarget,
            sequence,
        ))
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

    fn subscribe_lifecycle(
        &self,
        _context: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
        Err(error(
            ErrorCode::Unsupported,
            "installed Herdr CLI has no verified lifecycle subscription",
        ))
    }

    /// Cooperative native policy: Herdr 0.9.1 cannot prove the current
    /// native execution, so the target is structural (a fresh current-target
    /// read of the same terminal in the same verified server incarnation, no
    /// positive evidence of an empty shell, active turn, blocked UI or human
    /// input). Whether the occupant is a recognized idle harness is rechecked
    /// by `submit_prompt` immediately before submission.
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if self.safe_prompt_capability() != CapabilityState::Supported
            || observation.provenance != ObservationProvenance::FreshCurrentTarget
            || observation.generation == 0
            || matches!(observation.execution, ExecutionEvidence::Verified { .. })
            || observation.occupancy == StructuralOccupancy::EmptyShell
            || matches!(
                observation.ui,
                HostUiState::ActiveTurn | HostUiState::ApprovalOrQuestion | HostUiState::HumanInput
            )
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

    fn observe_pane_agent(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Option<ports::PaneAgentObservation>, ApiError> {
        self.check_context(context)?;
        let started = Instant::now();
        let limit = Duration::from_millis(750);
        match self.run(&["agent", "get", target.as_str()], &context.budget, limit) {
            Err(failure) if failure.code == ErrorCode::NotFound => Ok(None),
            Err(failure) => Err(failure),
            Ok(raw) => {
                let parsed = crate::host::observation::normalize_pane_agent(&raw, target.as_str())?;
                self.check_after_parse(&context.budget, started, limit)?;
                Ok(parsed)
            }
        }
    }

    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        if self.native_launch_capability() == NativeLaunchCapability::Unsupported {
            return Err(error(
                ErrorCode::Unsupported,
                "verified empty-pane preflight and native launch are unavailable",
            ));
        }
        // One more fresh witnessed read immediately before submission. Any
        // failure here is a confirmed pre-start rejection (nothing was sent),
        // so it is reported as a stale observation, never as a lost start.
        let preflight = self
            .observe_current_target(&request.target, context)
            .map_err(|failure| {
                error(
                    if failure.code == ErrorCode::NotFound {
                        ErrorCode::NotFound
                    } else {
                        ErrorCode::StaleHostObservation
                    },
                    format!("native start preflight failed: {}", failure.detail),
                )
            })?;
        self.guarded_start_cli(&request, context, &preflight)
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
    if let Some(bound) = target.bound_harness.as_deref()
        && kind != Some(bound)
    {
        return Err(format!(
            "agent kind {} differs from the bound harness {bound}",
            kind.unwrap_or("none").chars().take(32).collect::<String>()
        ));
    }
    let status = text("agent_status");
    if !status.is_some_and(|status| WAKE_READY_STATUSES.contains(&status)) {
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

fn unknown_boot() -> HostBootId {
    HostBootId::new("unverified-cli-host")
}

fn error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}

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
            request.seat = SeatId::new("seat-k3Fq9a2B");
            assert_eq!(request.agent_name(), "seat-k3fq9a2b");
            // The suffix never repeats the id the name already ends with.
            let retry = &request.agent_name_candidates()[1];
            assert!(
                retry.starts_with("seat-k3fq9a2b-") && retry.len() == 20,
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
        request.seat = SeatId::new("seat-k3Fq9a2B");
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
    fn pane_exchange() -> Exchange {
        Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "pane.get");
            answer(
                stream,
                &request,
                json!({"type":"pane_info","pane":{"pane_id":"w4:p1","terminal_id":"term_1",
                    "workspace_id":"w4","tab_id":"w4:t1","focused":false,
                    "agent_status":"idle","agent":"claude","revision":2}}),
            );
        })
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
            .observe_current_target(&HostTargetId::new("w4:p1"), &base)
            .unwrap();
        let mut target = cli
            .safe_wake_target(&SeatId::new("seat_1"), &observation)
            .expect("fresh verified terminal is a cooperative wake target");
        assert_eq!(target.basis, WakeTargetBasis::CooperativeAgent);
        assert_eq!(target.terminal.as_str(), "term_1");
        let mut context = wake_context(&observation);
        tamper(&mut target, &mut context);
        let result = cli.submit_prompt(&target, "wake marker", &context);
        worker.join().unwrap();
        fs::remove_file(socket).unwrap();
        (result, methods)
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
            let (result, methods) = cooperative_wake(vec![
                recheck_exchange(wake_agent(status, Some(kind), "term_1")),
                prompt,
            ]);
            assert_eq!(
                result.unwrap(),
                ports::PromptOutcome::Submitted,
                "{status} {kind}"
            );
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.prompt"],
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
                    prompt,
                ],
                None,
                |target, _| target.bound_harness = Some(kind.into()),
            );
            assert_eq!(result.unwrap(), ports::PromptOutcome::Submitted, "{kind}");
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get", "agent.prompt"],
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
            blocked,
        ]);
        assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe);

        let lost: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            assert_eq!(request["method"], "agent.prompt");
            drop(stream.try_clone().unwrap());
        });
        let (result, methods) = cooperative_wake(vec![
            recheck_exchange(wake_agent("done", Some("claude"), "term_1")),
            lost,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
        assert_eq!(methods.lock().unwrap().len(), 3, "never replayed");

        let other_terminal: Exchange = Box::new(|stream: &mut UnixStream, request: Value| {
            answer(
                stream,
                &request,
                json!({"type":"agent_prompted","agent":wake_agent("idle", Some("claude"), "term_9")}),
            );
        });
        let (result, _) = cooperative_wake(vec![
            recheck_exchange(wake_agent("idle", Some("claude"), "term_1")),
            other_terminal,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
    }

    /// W9-4: the TOCTOU fence right before typing into a pane. When the
    /// `agent.get` recheck is answered by a different server incarnation
    /// (process identity or boot) than the target was derived from, or the
    /// connection epoch moves while the recheck is in flight, the prompt is
    /// refused and `agent.prompt` is never sent. Kills: dropping the
    /// post-recheck `same_incarnation`/epoch check.
    #[cfg(target_os = "macos")]
    #[test]
    fn cooperative_wake_refuses_when_incarnation_or_epoch_moves_during_recheck() {
        type Tamper = Box<dyn FnOnce(&mut SafeWakeTarget, &mut HostCallContext)>;
        let restarted: Vec<(&str, Tamper)> = vec![
            (
                "server process identity",
                Box::new(|target, _| target.incarnation = "server-before-restart".into()),
            ),
            (
                "server boot",
                Box::new(|target, context| {
                    target.host_boot = HostBootId::new("boot-before-restart");
                    context.expected_boot = Some(target.host_boot.clone());
                }),
            ),
        ];
        for (label, tamper) in restarted {
            let (result, methods) = cooperative_wake_with(
                vec![recheck_exchange(wake_agent(
                    "idle",
                    Some("claude"),
                    "term_1",
                ))],
                None,
                tamper,
            );
            assert_eq!(result.unwrap_err().code, ErrorCode::TargetUnsafe, "{label}");
            assert_eq!(
                *methods.lock().unwrap(),
                ["pane.get", "agent.get"],
                "{label}: agent.prompt never sent"
            );
        }

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
            recheck_exchange(wake_agent("idle", Some("codex"), "term_1")),
            weird,
        ]);
        assert_eq!(result.unwrap(), ports::PromptOutcome::OutcomeUnknown);
        assert_eq!(
            *methods.lock().unwrap(),
            ["pane.get", "agent.get", "agent.prompt"],
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
            ("human input", Box::new(|o| o.ui = HostUiState::HumanInput)),
            ("active turn", Box::new(|o| o.ui = HostUiState::ActiveTurn)),
            ("generation 0", Box::new(|o| o.generation = 0)),
        ];
        for (label, change) in refused {
            let mut changed = observation.clone();
            change(&mut changed);
            assert!(cli.safe_wake_target(&seat, &changed).is_none(), "{label}");
        }
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
}
