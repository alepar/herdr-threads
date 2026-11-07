//! Ordinary structural seat resolution; native authority remains separate.
use crate::{
    ports::{
        BootstrapAttachmentGuard, BootstrapObserver, ContinuityRequest, ContinuityTargetGuard,
        HostCallContext, HostId, HostInvalidationReason, HostObservation, HostObservationAdmission,
        HostPort, OperatorRequest, OperatorTargetGuard, OrdinaryResolutionAttempt,
        OrdinaryResolutionGuard, OrdinaryResolutionOutcome, ProcessInfo, ResolvedTargetCheck,
        ShellContinuityCandidate, ShellContinuityGuard, ShellContinuityRequest, StorePort,
    },
    protocol::{
        authority::OperatorActor,
        commands::{ContinuityCheckIn, OperatorCommand, ResolveSeat},
        ids::{HostBootId, HostTargetId, SeatId},
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{fair_writer::FairWriter, pacer::Pacer, workers::BoundedLane},
};
use std::sync::Arc;
use std::time::Duration;

/// Hard upper bound, in monotonic milliseconds, of the post-failure
/// invalidation compensation. It is deliberately independent of the request
/// budget (see [`OrdinaryIdentity::compensation_budget`]).
pub const INVALIDATION_COMPENSATION_MS: u64 = 2_000;

/// Attempts of one explicit current-target read whose publication another
/// decision superseded (see [`OrdinaryIdentity::observe`]).
pub const SUPERSEDED_READ_ATTEMPTS: u32 = 3;

/// Longest an explicit target read waits for the lane capture it needs (no
/// published snapshot yet, or the host moved to a new boot or epoch).
const CAPTURE_WAIT_CAP: std::time::Duration = std::time::Duration::from_secs(5);

/// What an explicit current-target read produced.
enum Observed<R> {
    Read(Box<(HostObservationAdmission, R)>),
    /// The published snapshot is missing or behind the host's boot or epoch:
    /// only the observation lane's capture can move it. Carries the refusal
    /// the caller gets if waiting for that capture does not help.
    NeedsCapture(ApiError),
}

/// One explicit current-target read attempt.
enum ReadAttempt<R> {
    Published(Box<(HostObservationAdmission, R)>),
    /// Another decision moved the admission's fences; nothing was written.
    Superseded(HostObservationAdmission),
    /// Refused because no snapshot is published or the host's boot or epoch
    /// moved past the published one (refused at publication, or by the
    /// adapter's own context check).
    NeedsCapture(ApiError),
    /// The host was invalidated, or its boot or epoch changed, since the
    /// previous attempt.
    HostMoved,
}

/// Own time budget of the C1 `agent.get` diagnostic read.
const DIAGNOSTIC_READ_MILLIS: u64 = 750;
/// Own time budget of kept-claim bookkeeping and of one pass deciding kept
/// claims (ht-dtx0). The pass runs on the observation lane, so it stays short.
const KEPT_CONTINUITY_BUDGET_MILLIS: u64 = 3_000;

/// OrdinaryIdentity: structural seat resolution and observation for ONE Herdr
/// server.
///
/// Owns: the explicit current-target reads, the lane captures and the seat
/// decisions (resolve, operator repair, C1 continuity, reconciliation pages) of
/// the server named by its `host_id`. Every observation it admits, publishes or
/// reconciles carries that `host_id`, so a snapshot of this server can resolve,
/// unresolve or retire only seats whose `host_id` is this server's, and a target
/// id (unique only within one server) is never matched against another server's
/// seats.
/// Does not own: which servers exist or are attached (`HostSessions`), the
/// cooperative caller path (`cooperative_claim` names its own host), delivery,
/// wake or archival scope, or native caller authority.
/// Role: built once per attached server (interim: the one observation worker
/// passes the attached session's `host_id`). New per-host queries take a
/// [`HostId`] (from here, from a seat row's `host_id`, or from a claim) and add
/// `host_id = ?` plus `LOCAL_SEAT` to every seat predicate; they never read the
/// node-wide host.
pub struct OrdinaryIdentity {
    instance: String,
    host_id: HostId,
    store: Arc<dyn StorePort>,
    host: Arc<dyn HostPort>,
    clock: Arc<dyn Clock>,
    writer: Arc<FairWriter>,
    reads: BoundedLane,
    lane: std::sync::Mutex<crate::identity::reconcile::ObservationLane>,
    /// Signalled (with `lane`) whenever a lane capture completes.
    lane_captured: std::sync::Condvar,
    /// Service-lifetime cancellation owned by the elected daemon. It bounds
    /// detached compensation at shutdown; it is never a request token.
    service: Cancellation,
    /// The observation lane's Pacer: its backoff gates retries after a failed
    /// capture. Unset (tests, in-process services) means no gate.
    observation_pacer: Option<Arc<Pacer>>,
    /// C1 process continuity candidates the facts or a definitive C1 refusal ruled out
    /// (ht-hsyg), by binding, seat generation and fingerprint, with when (monotonic ms): not read
    /// again for [`PROCESS_DECLINE_TTL_MS`], so a transient answer does not stick for the whole
    /// generation. In-process only (a daemon restart forgets them); bounded, expired entries
    /// dropped first, cleared when still full.
    process_declined: std::sync::Mutex<std::collections::HashMap<ProcessDeclineKey, u64>>,
    /// The last seat C1 process continuity walked this round; `None` starts over.
    process_cursor: std::sync::Mutex<Option<SeatId>>,
    /// C1 shell continuity candidates the facts or a definitive refusal ruled out (ht-ma6i.1), by
    /// seat, generation and fingerprint, with when: same cap, TTL and eviction as
    /// `process_declined`.
    shell_declined: std::sync::Mutex<std::collections::HashMap<ShellDeclineKey, u64>>,
    /// The last seat the fingerprint recording walked; `None` starts over.
    shell_record_cursor: std::sync::Mutex<Option<SeatId>>,
    /// The last seat C1 shell continuity walked; `None` starts over.
    shell_decide_cursor: std::sync::Mutex<Option<SeatId>>,
    /// The kernel facts shell continuity reads.
    processes: Arc<dyn ProcessInfo>,
    /// The lane's configured cadence in milliseconds.
    observation_cadence_millis: u64,
}

use crate::identity::process_continuity::DeclineKey as ProcessDeclineKey;
use crate::identity::shell_continuity::DeclineKey as ShellDeclineKey;
/// Most C1 process continuity declines remembered before the set is cleared.
const PROCESS_DECLINED_CAP: usize = 1024;
/// How long a C1 process continuity decline stands before the candidate is read again.
const PROCESS_DECLINE_TTL_MS: u64 = 10 * 60 * 1000;

impl OrdinaryIdentity {
    pub fn new(
        instance: String,
        host_id: HostId,
        store: Arc<dyn StorePort>,
        host: Arc<dyn HostPort>,
        clock: Arc<dyn Clock>,
        writer: Arc<FairWriter>,
    ) -> Self {
        Self {
            instance,
            host_id,
            store,
            host,
            clock,
            writer,
            reads: BoundedLane::new(1, 8),
            lane: std::sync::Mutex::new(crate::identity::reconcile::ObservationLane::default()),
            lane_captured: std::sync::Condvar::new(),
            service: Cancellation::default(),
            observation_pacer: None,
            process_declined: std::sync::Mutex::new(std::collections::HashMap::new()),
            process_cursor: std::sync::Mutex::new(None),
            shell_declined: std::sync::Mutex::new(std::collections::HashMap::new()),
            shell_record_cursor: std::sync::Mutex::new(None),
            shell_decide_cursor: std::sync::Mutex::new(None),
            processes: Arc::new(crate::host::process_table::HostProcessTable),
            observation_cadence_millis: crate::identity::reconcile::DEFAULT_SNAPSHOT_CADENCE_MILLIS,
        }
    }

    /// The lane's configured capture cadence.
    pub fn with_observation_cadence(mut self, cadence: Duration) -> Self {
        self.observation_cadence_millis = u64::try_from(cadence.as_millis()).unwrap_or(u64::MAX);
        self
    }

    /// The Herdr server this identity acts on.
    pub fn host_id(&self) -> HostId {
        self.host_id
    }

    /// The port of the Herdr server this identity acts on.
    pub fn host_port(&self) -> &Arc<dyn HostPort> {
        &self.host
    }

    /// Gate background retries on the observation lane's Pacer backoff.
    pub fn with_observation_pacer(mut self, pacer: Arc<Pacer>) -> Self {
        self.observation_pacer = Some(pacer);
        self
    }

    /// The process table shell continuity reads (a fixture's, in a test).
    pub fn with_process_info(mut self, processes: Arc<dyn ProcessInfo>) -> Self {
        self.processes = processes;
        self
    }

    /// Bind detached compensation to the owner's shutdown token. Without it
    /// (tests, in-process services) compensation is bounded only by
    /// [`INVALIDATION_COMPENSATION_MS`].
    pub fn with_service_cancellation(mut self, service: Cancellation) -> Self {
        self.service = service;
        self
    }

    /// Background captures use the same bounded read lane as explicit targets.
    pub fn capture_if_due(
        &self,
        store: &(impl crate::identity::reconcile::observation_store::ObservationStore + ?Sized),
        budget: &CallBudget,
        maintenance: &CallBudget,
    ) -> Result<Option<crate::identity::reconcile::ObservationOutcome>, ApiError> {
        let _read = self.reads.enter(budget, self.clock.as_ref())?;
        let mut lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        let retry_at = self
            .observation_pacer
            .as_ref()
            .and_then(|pacer| pacer.next_retry_at());
        if !lane.snapshot_due_gated(
            self.clock.monotonic_now(),
            retry_at,
            self.observation_cadence_millis,
        ) {
            return Ok(None);
        }
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        };
        // A store-only host's capture is its own rows: unchanged, there is nothing to publish, and
        // the pass would write five transactions to say so. Only a finished pass with no lane
        // failure outstanding may be skipped; anything else runs a real capture, which alone
        // reports recovery. A failed read falls through to the capture.
        lane.clear_staged_live();
        // A failure the lane never saw (the read gate) counts too.
        let failing = self
            .observation_pacer
            .as_ref()
            .is_some_and(|pacer| pacer.attempts() > 0);
        if let Ok(Some(live)) = self.host.store_enumeration(&context) {
            if !failing
                && !lane.explicit_capture_requested()
                && lane.published_live() == Some(live.as_slice())
            {
                lane.skip_unchanged_capture();
                return Ok(None);
            }
            lane.stage_live(live);
        }
        lane.begin_pass();
        let outcome = crate::identity::reconcile::observe_and_publish(
            self.host.as_ref(),
            store,
            &mut lane,
            &self.instance,
            self.host_id,
            &context,
            maintenance,
        );
        lane.note_capture_completed();
        drop(lane);
        self.lane_captured.notify_all();
        outcome.map(Some)
    }

    /// Record that `published`'s saved-seat pass ended with no refusal that
    /// holds the marker back (TRUST-POLICY C2). Taken under the observation lane lock, like
    /// a reconciliation page: the marker write can lift the baseline hold and
    /// bump the lifecycle revision, which would otherwise supersede an explicit
    /// target observation (operator repair, resolve) admitted concurrently.
    pub fn record_reconciliation_pass(
        &self,
        store: &(impl crate::identity::reconcile::observation_store::ObservationStore + ?Sized),
        published: &crate::ports::PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let mut lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        let recorded = store.record_reconciliation_pass(published, budget)?;
        if recorded {
            lane.promote_live();
        }
        Ok(recorded)
    }

    pub fn reconcile_page(
        &self,
        store: &(impl crate::identity::reconcile::observation_store::ObservationStore + ?Sized),
        outcome: &crate::identity::reconcile::ObservationOutcome,
        after: u64,
        high: Option<u64>,
        budget: &CallBudget,
    ) -> Result<crate::identity::reconcile::ReconcilePageProgress, ApiError> {
        let _read = self.reads.enter(budget, self.clock.as_ref())?;
        let mut lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        match outcome {
            crate::identity::reconcile::ObservationOutcome::Published(publication) => {
                crate::identity::reconcile::reconcile_published_page_for(
                    store,
                    publication,
                    after,
                    high,
                    budget,
                    self.host.closure_authority(),
                )
            }
            crate::identity::reconcile::ObservationOutcome::Invalidated {
                fence, reason, ..
            } => {
                let page = crate::identity::reconcile::reconcile_invalidated_page(
                    store, fence, after, high, budget,
                );
                lane.note_invalidation_page(*reason, &page);
                page
            }
            crate::identity::reconcile::ObservationOutcome::Superseded => {
                Err(error(ErrorCode::CursorStale, "capture superseded"))
            }
            crate::identity::reconcile::ObservationOutcome::Frozen { .. } => Err(error(
                ErrorCode::CursorStale,
                "a frozen capture has no pages",
            )),
            crate::identity::reconcile::ObservationOutcome::InvalidationRepeated { .. } => Err(
                error(ErrorCode::CursorStale, "repeated invalidation has no pages"),
            ),
        }
    }

    pub fn resolve(&self, request: ResolveSeat, budget: &CallBudget) -> Result<SeatId, ApiError> {
        let replay = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.resolve_seat(
                request.clone(),
                OrdinaryResolutionAttempt::ReplayOnly,
                budget,
            )?
        };
        if let OrdinaryResolutionOutcome::Resolved(seat) = replay {
            return Ok(seat);
        }
        self.with_observation(&request.target, budget, |admission, observation| {
            let guard = OrdinaryResolutionGuard::try_new(&request, observation, &admission)
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            match self.store.resolve_seat(
                request.clone(),
                OrdinaryResolutionAttempt::Observed(guard),
                budget,
            )? {
                OrdinaryResolutionOutcome::Resolved(seat) => Ok(seat),
                OrdinaryResolutionOutcome::NeedsObservation => Err(error(
                    ErrorCode::StoreCorrupt,
                    "observed resolution returned no deciding result",
                )),
            }
        })
    }

    /// Seat-acting resolve under a name on the built-in named host: the store allocates the live
    /// seat of that name when none exists. No host observation: a named seat's presence is
    /// cooperative (TRUST-POLICY A3), so there is no pane to read.
    pub fn resolve_named(
        &self,
        request: ResolveSeat,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let result = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store
                .resolve_named_seat(request, self.host_id, budget)?
        };
        if let CommandResult::NamedSeatResolved { seat, .. } = &result {
            self.note_named_seat(seat);
        }
        Ok(result)
    }

    /// A seat the lane has not published is waited on by its own hook: capture now, not at the next
    /// tick. After the writer turn, and never blocking: a capture holding the lane may wait for the
    /// writer, and the next tick compares the live set anyway.
    fn note_named_seat(&self, seat: &SeatId) {
        let Ok(mut lane) = self.lane.try_lock() else {
            return;
        };
        if lane
            .published_live()
            .is_some_and(|live| live.contains(seat))
        {
            return;
        }
        lane.mark_event_dirty();
        drop(lane);
        if let Some(pacer) = &self.observation_pacer {
            pacer.kick();
        }
    }

    /// Historical replay precedes host work and grants no current authority.
    pub fn operator(
        &self,
        command: OperatorCommand,
        actor: OperatorActor,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let replay = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store
                .replay_operator(command.clone(), actor.clone(), budget)?
        };
        if let Some(result) = replay {
            return Ok(result);
        }
        let target = match &command {
            OperatorCommand::OrphanInvite(request) => {
                let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
                return self.store.mutate_operator(
                    OperatorRequest::OrphanInvite(request.clone()),
                    actor,
                    budget,
                );
            }
            OperatorCommand::Retire(request) => {
                let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
                return self.store.mutate_operator(
                    OperatorRequest::Retire(request.clone()),
                    actor,
                    budget,
                );
            }
            OperatorCommand::Rebind(request) => request.target.clone(),
            OperatorCommand::Replace(request) => request.target.clone(),
            OperatorCommand::FreshSeat(request) => request.target.clone(),
        };
        self.with_observation(&target, budget, |_, observation| {
            let guard =
                OperatorTargetGuard::try_new(&self.instance, self.host_id, &command, observation)
                    .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
            let request = match command {
                OperatorCommand::Rebind(c) => OperatorRequest::Rebind(c, guard),
                OperatorCommand::FreshSeat(c) => OperatorRequest::FreshSeat(c, guard),
                OperatorCommand::Replace(c) => OperatorRequest::Replace(c, guard),
                OperatorCommand::OrphanInvite(_) | OperatorCommand::Retire(_) => unreachable!(),
            };
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.mutate_operator(request, actor, budget)
        })
    }

    /// TRUST-POLICY C1: reattach the unresolved seat a resumed session belongs
    /// to. Historical replay precedes host work and grants no current
    /// authority. The Herdr `agent_session` read is a bounded diagnostic whose
    /// outcome (match, mismatch, absent, read error) is recorded and never
    /// decides, so no read failure refuses or permits anything. The read runs
    /// before the fresh target observation, so its latency is outside the
    /// guard's freshness window; its failures map to `read_error` and never
    /// fence (the adapter does not bump the connection epoch for it).
    pub fn continuity(
        &self,
        command: ContinuityCheckIn,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let replay = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.replay_continuity(command.clone(), budget)?
        };
        if let Some(result) = replay {
            return Ok(result);
        }
        let (result, not_ready) =
            self.decide_continuity_now(command.clone(), budget, false, claim_evidence(&command));
        // ht-dtx0: a claim refused only because the host evidence is not ready
        // (reconciliation lag, a failed host read) is kept for the observation
        // worker to decide once the epoch has reconciled; a decided claim
        // retires any kept copy. Best effort on the daemon's own short budget
        // (the caller's may be spent) and only with writer admission; the reply
        // is the same either way.
        let budget = self.maintenance_budget();
        if let Ok(_turn) = self.writer.enter_foreground(&budget, self.clock.as_ref()) {
            let _ = if not_ready {
                self.store
                    .keep_continuity_claim(self.host_id, &command, &budget)
                    .map(|_| ())
            } else {
                self.store
                    .forget_continuity_claim(self.host_id, &command, None, &budget)
            };
        }
        result
    }

    /// ht-dtx0: decide this host's kept continuity claims, after a completed
    /// reconciliation pass, on the observation lane: each target read is quiet
    /// (no explicit capture, no wait for one), so a claim that stays not ready
    /// costs two bounded reads per pass (the C1 diagnostic and the target) at
    /// the lane's own cadence. Each is the ordinary
    /// C1 decision under its own operation key (a session's later
    /// resubmission replays the result). A claim whose process is gone, or that
    /// was decided either way, is dropped; one whose evidence is still not
    /// ready is kept, unrefreshed, until it expires. A drop removes only the
    /// row as loaded, never a newer claim of the same session.
    pub fn decide_kept_continuity(&self) -> Result<(), ApiError> {
        let budget = self.maintenance_budget();
        let claims = {
            let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
            self.store.kept_continuity_claims(self.host_id, &budget)?
        };
        let processes = crate::host::process_table::HostProcessTable;
        for claim in claims {
            if budget.is_exhausted(self.clock.as_ref()) {
                break;
            }
            let command = &claim.command;
            let alive = command
                .liveness
                .is_some_and(|fingerprint| fingerprint.is_alive(&processes));
            let settled = !alive || {
                let replayed = {
                    let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
                    self.store.replay_continuity(command.clone(), &budget)
                };
                replay_settles(&replayed).unwrap_or_else(|| {
                    !self
                        .decide_continuity_now(
                            command.clone(),
                            &budget,
                            true,
                            claim_evidence(command),
                        )
                        .1
                })
            };
            if settled {
                let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
                self.store
                    .forget_continuity_claim(self.host_id, command, Some(&claim), &budget)?;
            }
        }
        Ok(())
    }

    /// TRUST-POLICY C1 process continuity (ht-hsyg): after a completed reconciliation pass, on
    /// the observation lane (after the kept claims, which are the sessions' own), reattach each
    /// unresolved seat whose top-level harness binding's own process Herdr's pane at the seat's
    /// saved target still hosts (see [`crate::identity::process_continuity`]). The saved target
    /// only selects the pane to ask: one quiet `pane process-info` read per seat with a live,
    /// undeclined candidate. Seats are walked a page at a time from a cursor, so every one is
    /// reached however many others are declined, and each match is decided at once, before the
    /// next seat is read. A match is not decided while another seat saved at the same target has
    /// a live process. The decision is the ordinary C1 transaction, pinned to the judged seat and
    /// generation, under a daemon operation key per (seat, binding, seat generation), recorded
    /// `structural_process_continuity`. A candidate the facts rule out, or that a definitive C1
    /// refusal answers, is remembered for its seat generation and fingerprint; a gone process, an
    /// unread fact or a not-ready refusal is retried next pass.
    pub fn decide_process_continuity(&self) -> Result<(), ApiError> {
        use crate::identity::process_continuity::{
            decline_key, judge_seat, page_groups, same_boot,
        };
        let budget = self.maintenance_budget();
        let after = self
            .process_cursor
            .lock()
            .ok()
            .and_then(|cursor| cursor.clone());
        let rows = {
            let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
            self.store.process_continuity_candidates(
                self.host_id,
                crate::ports::ProcessContinuityPage {
                    after,
                    target: None,
                },
                &budget,
            )?
        };
        let (groups, last_page) = page_groups(rows.candidates, rows.full);
        let processes = crate::host::process_table::HostProcessTable;
        let now = self.clock.monotonic_now().0;
        let declined = |key: &ProcessDeclineKey| decline_stands(&self.process_declined, key, now);
        let mut walked_all = true;
        for group in &groups {
            if budget.is_exhausted(self.clock.as_ref()) {
                walked_all = false;
                break;
            }
            // A previous boot's candidate (or one of an unknown boot) costs no host read.
            let worth_reading = group.iter().any(|candidate| {
                !declined(&decline_key(candidate))
                    && same_boot(&processes, candidate.liveness_boot.as_deref())
                    && candidate.liveness.is_alive(&processes)
            });
            if worth_reading {
                let shell = self
                    .host
                    .pane_shell_pid(&group[0].target, &self.quiet_read_context(&budget))
                    .ok()
                    .flatten();
                let judged = judge_seat(group, &processes, shell, &declined);
                for key in judged.declines {
                    self.decline_process_candidate(key);
                }
                if let (Some(candidate), Some(shell)) = (judged.matched, shell) {
                    self.decide_process_match(candidate, &processes, shell, &budget)?;
                }
            }
            if let Ok(mut cursor) = self.process_cursor.lock() {
                *cursor = Some(group[0].seat.clone());
            }
        }
        if walked_all
            && last_page
            && let Ok(mut cursor) = self.process_cursor.lock()
        {
            *cursor = None;
        }
        Ok(())
    }

    /// One judged match: refuse to choose while another seat at the target has a live process,
    /// else the C1 transaction pinned to the judged seat and generation.
    fn decide_process_match(
        &self,
        candidate: &crate::ports::ProcessContinuityCandidate,
        processes: &dyn crate::ports::ProcessInfo,
        shell: u32,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let at_target = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.process_continuity_candidates(
                self.host_id,
                crate::ports::ProcessContinuityPage {
                    after: None,
                    target: Some(candidate.target.clone()),
                },
                budget,
            )?
        };
        if crate::identity::process_continuity::has_rival_in_pane(
            &at_target.candidates,
            &candidate.seat,
            processes,
            shell,
        ) {
            return Ok(());
        }
        let Ok(operation) = crate::protocol::ids::OperationId::parse(format!(
            "process-continuity:{}:{}:{}",
            candidate.seat.as_str(),
            candidate.binding_id,
            candidate.seat_generation
        )) else {
            return Ok(());
        };
        let command = ContinuityCheckIn {
            target: candidate.target.clone(),
            host: candidate.host.clone(),
            harness: candidate.harness,
            native_session: candidate.native_session.clone(),
            source: ContinuityCheckIn::SAME_SESSION_SOURCE.to_owned(),
            operation,
            execution: candidate.execution.clone(),
            liveness: Some(candidate.liveness),
            continuity_key: None,
        };
        let (result, not_ready) = self.decide_continuity_now(
            command,
            budget,
            true,
            crate::ports::ContinuityEvidence::Process {
                seat: candidate.seat.clone(),
                seat_generation: candidate.seat_generation,
            },
        );
        if result.is_err() && !not_ready {
            self.decline_process_candidate(crate::identity::process_continuity::decline_key(
                candidate,
            ));
        }
        Ok(())
    }

    /// TRUST-POLICY C1 shell continuity (ht-ma6i.1): after a completed reconciliation pass, record
    /// the pane shell of up to [`crate::ports::SHELL_FINGERPRINT_READS_PER_PASS`] resolved seats
    /// with no fingerprint for their current terminal. A terminal's shell does not change while
    /// its id stands, so a current row is never read again and steady passes make no host call.
    /// An unreadable shell records nothing and the cursor still moves past it, so it cannot starve
    /// the seats behind it; the cursor wraps at the end of the listing. Each read is fenced to the
    /// listing publication's boot and epoch (refused before I/O on another epoch, after it when the
    /// epoch moved or another boot answered) and the write re-checks that publication, so a Herdr
    /// restart before the write records nothing (an unreadable shell; the next pass lists it anew).
    pub fn record_shell_fingerprints(&self) -> Result<(), ApiError> {
        let budget = self.maintenance_budget();
        let after = lock_cursor(&self.shell_record_cursor);
        let gaps = {
            let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
            self.store
                .shell_fingerprint_gaps(self.host_id, after, &budget)?
        };
        let mut walked_all = gaps.len() < crate::ports::SHELL_FINGERPRINT_READS_PER_PASS;
        for gap in &gaps {
            if budget.is_exhausted(self.clock.as_ref()) {
                walked_all = false;
                break;
            }
            let fingerprint = self
                .host
                .pane_shell_pid(
                    &gap.target,
                    &self.fenced_quiet_read_context(&budget, &gap.host_boot, gap.host_epoch),
                )
                .ok()
                .flatten()
                .and_then(|pid| crate::identity::shell_continuity::read(&*self.processes, pid));
            if let Some(fingerprint) = fingerprint {
                let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
                self.store
                    .record_shell_fingerprint(gap, &fingerprint, &budget)?;
            }
            set_cursor(&self.shell_record_cursor, Some(gap.seat.clone()));
        }
        if walked_all {
            set_cursor(&self.shell_record_cursor, None);
        }
        Ok(())
    }

    /// TRUST-POLICY C1 shell continuity (ht-ma6i.1): after C1 process continuity, resolve each
    /// unresolved seat whose recorded fingerprint is still the shell of the pane at its saved
    /// target. A seat with a live undeclined process continuity candidate is left to that path,
    /// which also reopens a binding. A rival fingerprint costs no host read. `NotContinuous` and a
    /// definitive refusal are remembered for the fingerprint; `Gone`, `Unknown`, an unread pane, a
    /// rival process and a not-ready refusal are retried next pass.
    pub fn decide_shell_continuity(&self) -> Result<(), ApiError> {
        use crate::identity::shell_continuity::decline_key;
        let budget = self.maintenance_budget();
        let after = lock_cursor(&self.shell_decide_cursor);
        let candidates = {
            let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
            self.store
                .shell_continuity_candidates(self.host_id, after, &budget)?
        };
        let now = self.clock.monotonic_now().0;
        let mut walked_all = candidates.len() < crate::ports::SHELL_CONTINUITY_CANDIDATES_PER_PASS;
        for candidate in &candidates {
            if budget.is_exhausted(self.clock.as_ref()) {
                walked_all = false;
                break;
            }
            if !decline_stands(&self.shell_declined, &decline_key(candidate), now) {
                self.judge_shell_candidate(candidate, now, &budget)?;
            }
            set_cursor(&self.shell_decide_cursor, Some(candidate.seat.clone()));
        }
        if walked_all {
            set_cursor(&self.shell_decide_cursor, None);
        }
        Ok(())
    }

    /// One candidate not declined: what it costs a host read to judge, and the decision.
    fn judge_shell_candidate(
        &self,
        candidate: &ShellContinuityCandidate,
        now: u64,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        use crate::identity::{process_continuity, shell_continuity};
        let (at_target, rival_fingerprint) = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            let at_target = self.store.process_continuity_candidates(
                self.host_id,
                crate::ports::ProcessContinuityPage {
                    after: None,
                    target: Some(candidate.target.clone()),
                },
                budget,
            )?;
            let rival =
                self.store
                    .shell_rival_exists(&candidate.seat, &candidate.fingerprint, budget)?;
            (at_target.candidates, rival)
        };
        let left_to_process_path = at_target.iter().any(|row| {
            row.seat == candidate.seat
                && process_continuity::same_boot(&*self.processes, row.liveness_boot.as_deref())
                && row.liveness.is_alive(&*self.processes)
                && !decline_stands(
                    &self.process_declined,
                    &process_continuity::decline_key(row),
                    now,
                )
        });
        if left_to_process_path || rival_fingerprint {
            return Ok(());
        }
        let Some(shell) = self
            .host
            .pane_shell_pid(&candidate.target, &self.quiet_read_context(budget))
            .ok()
            .flatten()
        else {
            return Ok(());
        };
        match shell_continuity::judge(&*self.processes, &candidate.fingerprint, shell) {
            process_continuity::Verdict::Continuous => {}
            process_continuity::Verdict::NotContinuous => {
                self.decline_shell_candidate(candidate);
                return Ok(());
            }
            process_continuity::Verdict::Gone | process_continuity::Verdict::Unknown => {
                return Ok(());
            }
        }
        if process_continuity::has_rival_in_pane(
            &at_target,
            &candidate.seat,
            &*self.processes,
            shell,
        ) {
            return Ok(());
        }
        let result = self.with_read_inner(
            &candidate.target,
            budget,
            |target, context| self.host.observe_current_target(target, context),
            |observation| observation,
            |_, observation| {
                let guard = ShellContinuityGuard::try_new(
                    &self.instance,
                    self.host_id,
                    &candidate.target,
                    observation,
                )
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
                let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
                self.store.decide_shell_continuity(
                    ShellContinuityRequest {
                        seat: candidate.seat.clone(),
                        seat_generation: candidate.seat_generation,
                        fingerprint: candidate.fingerprint.clone(),
                        guard,
                    },
                    budget,
                )
            },
            true,
        );
        if result.is_err() && !continuity_not_ready(&result) {
            self.decline_shell_candidate(candidate);
        }
        Ok(())
    }

    fn decline_shell_candidate(&self, candidate: &ShellContinuityCandidate) {
        remember_decline(
            &self.shell_declined,
            crate::identity::shell_continuity::decline_key(candidate),
            self.clock.monotonic_now().0,
        );
    }

    /// A quiet host read's context fenced to one host boot and epoch: the host refuses the read
    /// once its context moved past them.
    fn fenced_quiet_read_context(
        &self,
        budget: &CallBudget,
        boot: &HostBootId,
        epoch: u64,
    ) -> HostCallContext {
        HostCallContext {
            expected_boot: Some(boot.clone()),
            expected_epoch: Some(epoch),
            ..self.quiet_read_context(budget)
        }
    }

    /// A quiet host read's context: its own short budget within `budget`, no fences.
    fn quiet_read_context(&self, budget: &CallBudget) -> HostCallContext {
        HostCallContext {
            budget: CallBudget {
                deadline: budget.deadline.min(MonoInstant(
                    self.clock
                        .monotonic_now()
                        .0
                        .saturating_add(DIAGNOSTIC_READ_MILLIS),
                )),
                cancellation: budget.cancellation.clone(),
            },
            expected_boot: None,
            expected_epoch: None,
        }
    }

    fn decline_process_candidate(&self, key: ProcessDeclineKey) {
        remember_decline(&self.process_declined, key, self.clock.monotonic_now().0);
    }

    /// The daemon's own short budget for kept-claim bookkeeping and decisions,
    /// bounded by the service's lifetime rather than a request's.
    fn maintenance_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(
                self.clock
                    .monotonic_now()
                    .0
                    .saturating_add(KEPT_CONTINUITY_BUDGET_MILLIS),
            ),
            cancellation: self.service.clone(),
        }
    }

    /// One C1 decision against a fresh target read, and whether a refusal
    /// only says the evidence is not ready yet (the claim is kept): every
    /// refusal the client would retry, as `cli::retry::is_continuity_refusal`
    /// classifies it, so daemon and hook agree. Reconciliation lag, a failed,
    /// superseded or raced host read, writer admission and budget are not
    /// ready; no match, several matches, an owned or unusable target and an
    /// invalid claim are definitive. `quiet` reads without requesting a lane
    /// capture or waiting for one (the observation lane's own decisions).
    fn decide_continuity_now(
        &self,
        command: ContinuityCheckIn,
        budget: &CallBudget,
        quiet: bool,
        evidence: crate::ports::ContinuityEvidence,
    ) -> (Result<CommandResult, ApiError>, bool) {
        let target = command.target.clone();
        let diagnostic = self.agent_session_diagnostic(&target, &command, budget);
        let result = self.with_read_inner(
            &target,
            budget,
            |target, context| self.host.observe_current_target(target, context),
            |observation| observation,
            |_, observation| {
                let guard = ContinuityTargetGuard::try_new(
                    &self.instance,
                    self.host_id,
                    &command,
                    observation,
                )
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
                let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
                self.store.decide_continuity(
                    ContinuityRequest {
                        command,
                        guard,
                        diagnostic,
                        evidence,
                    },
                    budget,
                )
            },
            quiet,
        );
        let not_ready = continuity_not_ready(&result);
        (result, not_ready)
    }

    /// Compare Herdr's reported agent session for the pane with the claimed native
    /// session id. Diagnostic only (C1): the value is stored for seat inspect.
    fn agent_session_diagnostic(
        &self,
        target: &HostTargetId,
        command: &ContinuityCheckIn,
        budget: &CallBudget,
    ) -> &'static str {
        // Its own bounded budget (at most 750 ms) so a slow read cannot
        // consume the whole request budget.
        let bounded = CallBudget {
            deadline: budget.deadline.min(MonoInstant(
                self.clock
                    .monotonic_now()
                    .0
                    .saturating_add(DIAGNOSTIC_READ_MILLIS),
            )),
            cancellation: budget.cancellation.clone(),
        };
        let context = HostCallContext {
            budget: bounded,
            expected_boot: None,
            expected_epoch: None,
        };
        match self.host.observe_pane_agent(target, &context) {
            Ok(Some(agent)) => match agent.agent_session {
                Some(session) if session == command.native_session.as_str() => "match",
                Some(_) => "mismatch",
                None => "absent",
            },
            Ok(None) => "absent",
            Err(_) => "read_error",
        }
    }

    /// This is a new, non-replayed currentness decision. A historical resolve
    /// result cannot satisfy launch eligibility or select a replacement seat.
    pub fn check_current_target(
        &self,
        expected_seat: SeatId,
        request: ResolveSeat,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.with_observation(&request.target, budget, |admission, observation| {
            let guard = OrdinaryResolutionGuard::try_new(&request, observation, &admission)
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.check_resolved_target(
                ResolvedTargetCheck {
                    expected_seat,
                    guard,
                },
                budget,
            )
        })
    }

    /// A required native scoped read goes through the ordinary canonical lane.
    /// The same response is published before its deciding transaction; the
    /// callback receives no allocation or caller authority from this guard.
    pub fn with_bootstrap_observation<T>(
        &self,
        observer: &dyn BootstrapObserver,
        request: &ResolveSeat,
        budget: &CallBudget,
        decide: impl FnOnce(BootstrapAttachmentGuard) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        self.with_read(
            &request.target,
            budget,
            |target, context| observer.observe_bootstrap_target(target, context),
            |pane| pane.observation(),
            |admission, pane| {
                let guard = BootstrapAttachmentGuard::try_new(request, pane, &admission)
                    .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
                let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
                decide(guard)
            },
        )
    }

    fn with_observation<T>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        decide: impl FnOnce(HostObservationAdmission, HostObservation) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        self.with_read(
            target,
            budget,
            |target, context| self.host.observe_current_target(target, context),
            |observation| observation,
            decide,
        )
    }

    fn with_read<T, R>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        read: impl Fn(&HostTargetId, &HostCallContext) -> Result<R, ApiError>,
        observation_of: impl Fn(&R) -> &HostObservation,
        decide: impl FnOnce(HostObservationAdmission, R) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        self.with_read_inner(target, budget, read, observation_of, decide, false)
    }

    /// [`Self::with_read`]; `quiet` neither waits for nor requests a lane
    /// capture (a decision made on the observation lane itself, ht-dtx0).
    fn with_read_inner<T, R>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        read: impl Fn(&HostTargetId, &HostCallContext) -> Result<R, ApiError>,
        observation_of: impl Fn(&R) -> &HostObservation,
        decide: impl FnOnce(HostObservationAdmission, R) -> Result<T, ApiError>,
        quiet: bool,
    ) -> Result<T, ApiError> {
        let mut decide = Some(decide);
        let mut waited = false;
        loop {
            let (result, captures) = {
                let _read = self.reads.enter(budget, self.clock.as_ref())?;
                let mut lane = self.lane.lock().map_err(|_| {
                    error(
                        ErrorCode::StoreCorrupt,
                        "identity observation lane poisoned",
                    )
                })?;
                let ticket = lane.begin_observation()?;
                let result = match self.observe(target, budget, &read, &observation_of) {
                    Ok(Observed::Read(read)) => {
                        let (admission, observation) = *read;
                        let decide = decide.take().expect("one deciding read per request");
                        Some(decide(admission, observation))
                    }
                    // First contact after a daemon start or a host restart:
                    // wait (once) for the lane capture the read needs, with
                    // the read slot and lane lock released.
                    Ok(Observed::NeedsCapture(_))
                        if !waited
                            && !quiet
                            && self.observation_pacer.is_some()
                            && !budget.is_exhausted(self.clock.as_ref()) =>
                    {
                        None
                    }
                    Ok(Observed::NeedsCapture(refusal)) => Some(Err(refusal)),
                    Err(error) => Some(Err(error)),
                };
                if !matches!(result, Some(Ok(_))) {
                    lane.mark_unavailable();
                }
                // An explicit target capture does not establish complete
                // enumeration; leave that lane due for its separate
                // snapshot/reconciliation driver and wake it (a kick never
                // shortens an outstanding backoff).
                lane.discard(ticket)?;
                // Ask the lane for its own snapshot now, even inside a backoff
                // wait (ht-p03.104): after a host restart this is the first
                // contact, and the published boot only moves once the lane
                // captures. A quiet read is the lane's own and asks nothing.
                if !quiet {
                    lane.request_explicit_capture();
                    if let Some(pacer) = &self.observation_pacer {
                        pacer.kick_explicit();
                    }
                }
                (result, lane.completed_captures())
            };
            if let Some(result) = result {
                return result;
            }
            waited = true;
            self.wait_for_capture(captures, budget)?;
        }
    }

    /// Block until the lane completes a capture after `seen`, the request
    /// budget runs out, or [`CAPTURE_WAIT_CAP`] passes.
    fn wait_for_capture(&self, seen: u64, budget: &CallBudget) -> Result<(), ApiError> {
        let remaining = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        let wait = std::time::Duration::from_millis(remaining).min(CAPTURE_WAIT_CAP);
        let lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        let _ = self
            .lane_captured
            .wait_timeout_while(lane, wait, |lane| {
                lane.completed_captures() == seen && !budget.cancellation.is_cancelled()
            })
            .map_err(|_| {
                error(
                    ErrorCode::StoreCorrupt,
                    "identity observation lane poisoned",
                )
            })?;
        Ok(())
    }

    /// One explicit current-target read, admitted again (up to
    /// [`SUPERSEDED_READ_ATTEMPTS`], inside the request budget) when its
    /// publication was superseded. Supersession is not a failure of the read:
    /// any concurrent seat decision (another agent's check-in bumps the
    /// instance's lifecycle revision) or publication moves the fences the
    /// admission captured. The superseded attempt wrote nothing and needs no
    /// invalidation, and each retry is a whole new admission, read and
    /// publication against the current canonical view (TRUST-POLICY A2), so
    /// the caller is refused only when the view keeps moving or the budget
    /// runs out. A host invalidation, boot or epoch change since the previous
    /// attempt is not ordinary contention: the request is refused as before,
    /// without another read.
    fn observe<R>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        read: &impl Fn(&HostTargetId, &HostCallContext) -> Result<R, ApiError>,
        observation_of: &impl Fn(&R) -> &HostObservation,
    ) -> Result<Observed<R>, ApiError> {
        let mut previous = None;
        let mut attempt = 1;
        loop {
            match self.observe_once(target, budget, previous.as_ref(), read, observation_of)? {
                ReadAttempt::Published(read) => return Ok(Observed::Read(read)),
                ReadAttempt::NeedsCapture(refusal) => return Ok(Observed::NeedsCapture(refusal)),
                ReadAttempt::Superseded(admission)
                    if attempt < SUPERSEDED_READ_ATTEMPTS
                        && !budget.is_exhausted(self.clock.as_ref()) =>
                {
                    previous = Some(admission);
                    attempt += 1;
                }
                ReadAttempt::Superseded(_) | ReadAttempt::HostMoved => {
                    return Err(error(
                        ErrorCode::StaleHostObservation,
                        "current-target observation was superseded",
                    ));
                }
            }
        }
    }

    /// The store issues an ordering ticket before I/O. Publication validates
    /// the same durable baseline/fences afterward and never releases a hold.
    fn observe_once<R>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        previous: Option<&HostObservationAdmission>,
        read: &impl Fn(&HostTargetId, &HostCallContext) -> Result<R, ApiError>,
        observation_of: &impl Fn(&R) -> &HostObservation,
    ) -> Result<ReadAttempt<R>, ApiError> {
        let admission = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store
                .begin_host_observation(&self.instance, self.host_id, budget)?
        };
        if let Some(previous) = previous
            && (admission.invalidation_revision != previous.invalidation_revision
                || admission.expected_boot != previous.expected_boot
                || admission.expected_epoch != previous.expected_epoch)
        {
            return Ok(ReadAttempt::HostMoved);
        }
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: admission.expected_boot.clone(),
            expected_epoch: Some(admission.expected_epoch),
        };
        let response = match read(target, &context) {
            Ok(observation) => observation,
            // The server answered that this target does not exist. That is
            // not host unavailability and must not unresolve other seats;
            // absence of a known terminal is decided by coherent snapshots.
            Err(error) if error.code == ErrorCode::NotFound => return Err(error),
            // The adapter refused the published boot/epoch as behind the host
            // (a restart): invalidate as before, and let the request wait for
            // the lane capture that moves it.
            Err(error) if error.code == ErrorCode::StaleHostObservation => {
                self.invalidate(&admission, HostInvalidationReason::HostUnavailable)?;
                return Ok(ReadAttempt::NeedsCapture(error));
            }
            Err(error) => {
                self.invalidate(&admission, HostInvalidationReason::HostUnavailable)?;
                return Err(error);
            }
        };
        if budget.is_exhausted(self.clock.as_ref()) {
            self.invalidate(&admission, HostInvalidationReason::HostUnavailable)?;
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "current-target preparation exhausted its budget",
            ));
        }
        let observation = observation_of(&response);
        if observation.target != *target || observation.verified_structural_proof().is_none() {
            self.invalidate(&admission, HostInvalidationReason::CoherenceLost)?;
            return Err(error(
                ErrorCode::StaleHostObservation,
                "current target lacks verified structural evidence",
            ));
        }
        let publication = (|| {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store
                .publish_current_target_observation(&admission, observation, budget)
        })();
        let published = match publication {
            Ok(published) => published,
            Err(error) => {
                self.invalidate(&admission, HostInvalidationReason::PublicationFailed)?;
                return Err(error);
            }
        };
        if !published {
            let host_moved = admission.expected_active.is_none()
                || admission.expected_boot.as_ref() != Some(&observation.host_boot)
                || admission.expected_epoch != observation.epoch;
            return Ok(if host_moved {
                ReadAttempt::NeedsCapture(error(
                    ErrorCode::StaleHostObservation,
                    "current-target observation was superseded",
                ))
            } else {
                ReadAttempt::Superseded(admission)
            });
        }
        Ok(ReadAttempt::Published(Box::new((admission, response))))
    }

    /// Budget for compensating a failed/partial/unpublished target read.
    ///
    /// Ownership decision (seat-identity design, "Ordered observation and
    /// reconciliation" and "Authorization flow" step 4): once a host read has
    /// been admitted and then failed, that failure is a *known invalidation*
    /// which later decisions must reject against. It is not uncommitted
    /// request work. The most common failure is the request budget itself
    /// expiring or being cancelled while the host is held (the `is_exhausted`
    /// branch in `observe` exists for exactly that), so a budget derived from
    /// the request would refuse the compensation precisely when it is needed
    /// and leave the previously published target looking available until the
    /// next reconciliation. Hence the compensation deliberately outlives the
    /// request's cancellation and deadline.
    ///
    /// It stays bounded and owned: a fresh monotonic deadline of
    /// [`INVALIDATION_COMPENSATION_MS`] from its start (covering writer queue,
    /// SQLite lock and the one scalar write) and the daemon's service
    /// shutdown token, so shutdown never waits on it (daemon design: host and
    /// cleanup waits cannot hold shutdown). Dropping it at shutdown is safe:
    /// restart starts a new boot/epoch, discards permits and reconciles before
    /// host-dependent mutations. The store rejects stale admissions without
    /// touching a newer successor, so a late compensation cannot regress state.
    fn compensation_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(
                self.clock
                    .monotonic_now()
                    .0
                    .saturating_add(INVALIDATION_COMPENSATION_MS),
            ),
            cancellation: self.service.clone(),
        }
    }

    fn invalidate(
        &self,
        admission: &HostObservationAdmission,
        reason: HostInvalidationReason,
    ) -> Result<(), ApiError> {
        // Unavailability is not evidence: the published view stays frozen and
        // the failed read is simply refused (TRUST-POLICY C4).
        if reason.is_unavailability() {
            return Ok(());
        }
        let budget = self.compensation_budget();
        let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
        self.store
            .invalidate_host_observation(admission, reason, &budget)?;
        Ok(())
    }
}

fn lock_cursor(cursor: &std::sync::Mutex<Option<SeatId>>) -> Option<SeatId> {
    cursor.lock().ok().and_then(|cursor| cursor.clone())
}

fn set_cursor(cursor: &std::sync::Mutex<Option<SeatId>>, seat: Option<SeatId>) {
    if let Ok(mut cursor) = cursor.lock() {
        *cursor = seat;
    }
}

/// Whether `key`'s decline is within its TTL.
fn decline_stands<K: Eq + std::hash::Hash>(
    declined: &std::sync::Mutex<std::collections::HashMap<K, u64>>,
    key: &K,
    now: u64,
) -> bool {
    declined.lock().is_ok_and(|declined| {
        declined
            .get(key)
            .is_some_and(|at| now.saturating_sub(*at) < PROCESS_DECLINE_TTL_MS)
    })
}

/// Remember a decline, in a bounded set: expired entries go first, and it is cleared when still full.
fn remember_decline<K: Eq + std::hash::Hash>(
    declined: &std::sync::Mutex<std::collections::HashMap<K, u64>>,
    key: K,
    now: u64,
) {
    if let Ok(mut declined) = declined.lock() {
        if declined.len() >= PROCESS_DECLINED_CAP {
            declined.retain(|_, at| now.saturating_sub(*at) < PROCESS_DECLINE_TTL_MS);
            if declined.len() >= PROCESS_DECLINED_CAP {
                declined.clear();
            }
        }
        declined.insert(key, now);
    }
}

fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

/// What a session's own claim rests on, at intake and when a kept claim is decided.
fn claim_evidence(command: &ContinuityCheckIn) -> crate::ports::ContinuityEvidence {
    if command.continuity_key.is_some() {
        crate::ports::ContinuityEvidence::Key
    } else {
        crate::ports::ContinuityEvidence::Cooperative
    }
}

/// ht-dtx0: whether a C1 decision's refusal only says the evidence is not
/// ready, so the claim is kept: a refusal the hook would retry
/// (`cli::retry::is_continuity_refusal` is false). A decision is never.
fn continuity_not_ready(result: &Result<CommandResult, ApiError>) -> bool {
    result
        .as_ref()
        .err()
        .is_some_and(|refusal| !crate::cli::retry::is_continuity_refusal(&refusal.code))
}

/// ht-dtx0: what a kept claim's replay says. `Some(true)`: settled (a recorded
/// decision, or a definitive refusal such as a payload mismatch); `Some(false)`:
/// a transient read error, keep it for the next pass; `None`: nothing recorded,
/// decide it now.
fn replay_settles(replayed: &Result<Option<CommandResult>, ApiError>) -> Option<bool> {
    match replayed {
        Ok(Some(_)) => Some(true),
        Ok(None) => None,
        Err(refusal) => Some(crate::cli::retry::is_continuity_refusal(&refusal.code)),
    }
}

#[cfg(test)]
#[path = "../../tests/identity/kept_continuity_rules.rs"]
mod kept_continuity_rules;
