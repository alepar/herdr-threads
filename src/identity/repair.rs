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
    service::workers::{BoundedLane, FairWriter},
};
use std::sync::Arc;

/// Hard upper bound, in monotonic milliseconds, of the post-failure
/// invalidation compensation. It is deliberately independent of the request
/// budget (see [`OrdinaryIdentity::compensation_budget`]).
pub const INVALIDATION_COMPENSATION_MS: u64 = 2_000;

pub struct OrdinaryIdentity {
    instance: String,
    store: Arc<dyn StorePort>,
    host: Arc<dyn HostPort>,
    clock: Arc<dyn Clock>,
    writer: Arc<FairWriter>,
    reads: BoundedLane,
    lane: std::sync::Mutex<crate::identity::reconcile::ObservationLane>,
    /// Service-lifetime cancellation owned by the elected daemon. It bounds
    /// detached compensation at shutdown; it is never a request token.
    service: Cancellation,
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
            service: Cancellation::default(),
        }
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
        if !lane.snapshot_due(self.clock.monotonic_now()) {
            return Ok(None);
        }
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        };
        crate::identity::reconcile::observe_and_publish(
            self.host.as_ref(),
            store,
            &mut lane,
            &self.instance,
            &context,
            maintenance,
        )
        .map(Some)
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
        let _lane = self.lane.lock().map_err(|_| {
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
            crate::identity::reconcile::ObservationOutcome::Invalidated { fence, .. } => {
                crate::identity::reconcile::reconcile_invalidated_page(
                    store, fence, after, high, budget,
                )
            }
            crate::identity::reconcile::ObservationOutcome::Superseded => {
                Err(error(ErrorCode::CursorStale, "capture superseded"))
            }
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
    /// decides, so no read failure refuses or permits anything.
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
        self.with_observation(&target, budget, |_, observation| {
            let guard = ContinuityTargetGuard::try_new(&self.instance, &command, observation)
                .map_err(|detail| error(ErrorCode::StaleHostObservation, detail))?;
            let diagnostic = self.agent_session_diagnostic(&target, &command, budget);
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
        let context = HostCallContext {
            budget: budget.clone(),
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
        let _read = self.reads.enter(budget, self.clock.as_ref())?;
        let mut lane = self.lane.lock().map_err(|_| {
            error(
                ErrorCode::StoreCorrupt,
                "identity observation lane poisoned",
            )
        })?;
        let ticket = lane.begin_observation()?;
        let result = self
            .observe(target, budget)
            .and_then(|(admission, observation)| decide(admission, observation));
        if result.is_err() {
            lane.mark_unavailable();
        }
        // An explicit target capture does not establish complete enumeration;
        // leave that lane dirty for its separate snapshot/reconciliation driver.
        lane.discard(ticket)?;
        result
    }

    /// The store issues an ordering ticket before I/O. Publication validates
    /// the same durable baseline/fences afterward and never releases a hold.
    fn observe(
        &self,
        target: &HostTargetId,
        budget: &CallBudget,
    ) -> Result<(HostObservationAdmission, HostObservation), ApiError> {
        let admission = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.begin_host_observation(&self.instance, budget)?
        };
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
            return Err(error(
                ErrorCode::StaleHostObservation,
                "current-target observation was superseded",
            ));
        }
        Ok((admission, observation))
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
        let budget = self.compensation_budget();
        let _turn = self.writer.enter_foreground(&budget, self.clock.as_ref())?;
        self.store
            .invalidate_host_observation(admission, reason, &budget)?;
        Ok(())
    }
}
fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}
