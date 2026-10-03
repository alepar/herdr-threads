//! Ordinary structural seat resolution; native authority remains separate.
use crate::{
    ports::{
        ContinuityRequest, ContinuityTargetGuard, HostCallContext, HostInvalidationReason,
        HostObservation, HostObservationAdmission, HostPort, OperatorRequest, OperatorTargetGuard,
        OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome,
        ResolvedTargetCheck, StorePort,
    },
    protocol::{
        authority::OperatorActor,
        commands::{ContinuityCheckIn, OperatorCommand, ResolveSeat},
        ids::{HostTargetId, SeatId},
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{fair_writer::FairWriter, pacer::Pacer, workers::BoundedLane},
};
use std::sync::Arc;

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
enum Observed {
    Read(Box<(HostObservationAdmission, HostObservation)>),
    /// The published snapshot is missing or behind the host's boot or epoch:
    /// only the observation lane's capture can move it. Carries the refusal
    /// the caller gets if waiting for that capture does not help.
    NeedsCapture(ApiError),
}

/// One explicit current-target read attempt.
enum ReadAttempt {
    Published(Box<(HostObservationAdmission, HostObservation)>),
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

pub struct OrdinaryIdentity {
    instance: String,
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
}
impl OrdinaryIdentity {
    pub fn new(
        instance: String,
        store: Arc<dyn StorePort>,
        host: Arc<dyn HostPort>,
        clock: Arc<dyn Clock>,
        writer: Arc<FairWriter>,
    ) -> Self {
        Self {
            instance,
            store,
            host,
            clock,
            writer,
            reads: BoundedLane::new(1, 8),
            lane: std::sync::Mutex::new(crate::identity::reconcile::ObservationLane::default()),
            lane_captured: std::sync::Condvar::new(),
            service: Cancellation::default(),
            observation_pacer: None,
        }
    }

    /// Gate background retries on the observation lane's Pacer backoff.
    pub fn with_observation_pacer(mut self, pacer: Arc<Pacer>) -> Self {
        self.observation_pacer = Some(pacer);
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
        if !lane.snapshot_due_gated(self.clock.monotonic_now(), retry_at) {
            return Ok(None);
        }
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        };
        let outcome = crate::identity::reconcile::observe_and_publish(
            self.host.as_ref(),
            store,
            &mut lane,
            &self.instance,
            &context,
            maintenance,
        );
        lane.note_capture_completed();
        drop(lane);
        self.lane_captured.notify_all();
        outcome.map(Some)
    }

    /// Record that `published`'s saved-seat pass ended with no refused
    /// transition (TRUST-POLICY C2). Taken under the observation lane lock, like
    /// a reconciliation page: the marker write can lift the baseline hold and
    /// bump the lifecycle revision, which would otherwise supersede an explicit
    /// target observation (operator repair, resolve) admitted concurrently.
    pub fn record_reconciliation_pass(
        &self,
        store: &(impl crate::identity::reconcile::observation_store::ObservationStore + ?Sized),
        published: &crate::ports::PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let _lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        store.record_reconciliation_pass(published, budget)
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
                crate::identity::reconcile::reconcile_published_page(
                    store,
                    publication,
                    after,
                    high,
                    budget,
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
            let guard = OperatorTargetGuard::try_new(&self.instance, &command, observation)
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
        let target = command.target.clone();
        let diagnostic = self.agent_session_diagnostic(&target, &command, budget);
        self.with_observation(&target, budget, |_, observation| {
            let guard = ContinuityTargetGuard::try_new(&self.instance, &command, observation)
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.decide_continuity(
                ContinuityRequest {
                    command,
                    guard,
                    diagnostic,
                },
                budget,
            )
        })
    }

    /// Compare Herdr's reported agent session for the pane with the resumed
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

    fn with_observation<T>(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        decide: impl FnOnce(HostObservationAdmission, HostObservation) -> Result<T, ApiError>,
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
                let result = match self.observe(target, budget) {
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
                // captures.
                lane.request_explicit_capture();
                if let Some(pacer) = &self.observation_pacer {
                    pacer.kick_explicit();
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
    fn observe(&self, target: &HostTargetId, budget: &CallBudget) -> Result<Observed, ApiError> {
        let mut previous = None;
        let mut attempt = 1;
        loop {
            match self.observe_once(target, budget, previous.as_ref())? {
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
    fn observe_once(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
        previous: Option<&HostObservationAdmission>,
    ) -> Result<ReadAttempt, ApiError> {
        let admission = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.begin_host_observation(&self.instance, budget)?
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
        let observation = match self.host.observe_current_target(target, &context) {
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
                .publish_current_target_observation(&admission, &observation, budget)
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
        Ok(ReadAttempt::Published(Box::new((admission, observation))))
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
fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}
