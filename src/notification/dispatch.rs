//! Process-local wake admission and elapsed retry state.

use std::{collections::HashMap, sync::Mutex};
use uuid::Uuid;

use crate::service::pacer::Backoff;

use crate::{
    notification::policy::{DurableRetry, MARKER, RetryConfig, RetryError, RetryGuard},
    ports::{
        AgentComposerState, EvidenceKind, ExecutionEvidence, HostCallContext, HostPort,
        HostUiState, IncarnationEvidence, NotificationPort, ObservationProvenance, PromptOutcome,
        RefusalCause, ReservedWakeAuthority, StructuralOccupancy, WakeOutcome, WakeReservation,
        WakeTargetBasis,
    },
    protocol::{
        ids::{SeatId, WakeAttemptId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Clock, MonoInstant},
    },
    scheduler::{SubmissionVerification, outcome_for_verification},
};

pub const MAX_ACTIVE_PROMPTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    Retry(RetryError),
    UnknownSeat,
    ActiveLimit,
    WrongBoot,
}

const TARGET_READ_MILLIS: u64 = 750;
const PROMPT_MILLIS: u64 = 2_000;

/// Implemented by the store's final bounded attempt/boot/fence check. It is
/// invoked after a fresh native read and immediately before prompt submission.
pub trait ReservationCheck: Send + Sync {
    fn is_current(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
}
pub struct NativeWakeDispatcher<'a, H: HostPort + ?Sized, C: ReservationCheck + ?Sized> {
    host: &'a H,
    check: &'a C,
    clock: &'a dyn Clock,
    /// Post-send verification per seat, taken by the scheduler drive.
    verifications: Mutex<HashMap<SeatId, SubmissionVerification>>,
}
impl<'a, H: HostPort + ?Sized, C: ReservationCheck + ?Sized> NativeWakeDispatcher<'a, H, C> {
    pub fn new(host: &'a H, check: &'a C, clock: &'a dyn Clock) -> Self {
        Self {
            host,
            check,
            clock,
            verifications: Mutex::new(HashMap::new()),
        }
    }

    /// Wave 28: read the composer after a send; a prompt still held there gets
    /// exactly one submit-key retry and is then reported, never looped. A
    /// failed read or key send leaves the prompt unchecked (`NotChecked`).
    /// The result reaches daemon.log (rate limited) through
    /// `ObservedWakePort::observe_drive`.
    fn verify_submission(
        &self,
        target: &crate::ports::SafeWakeTarget,
        context: &HostCallContext,
    ) -> SubmissionVerification {
        let deadline = context
            .budget
            .deadline
            .0
            .min(self.clock.monotonic_now().0.saturating_add(PROMPT_MILLIS));
        let verify_context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(deadline),
                cancellation: context.budget.cancellation.clone(),
            },
            expected_boot: context.expected_boot.clone(),
            expected_epoch: context.expected_epoch,
        };
        if verify_context.budget.is_exhausted(self.clock) {
            return SubmissionVerification::NotChecked;
        }
        match self.host.pane_agent_state(target, &verify_context) {
            Ok(AgentComposerState::Submitted) => SubmissionVerification::Verified,
            Ok(AgentComposerState::HoldingPrompt) => {
                if self.host.send_submit_key(target, &verify_context).is_err() {
                    return SubmissionVerification::NotChecked;
                }
                match self.host.pane_agent_state(target, &verify_context) {
                    Ok(AgentComposerState::HoldingPrompt) => SubmissionVerification::Unsubmitted,
                    _ => SubmissionVerification::Retried,
                }
            }
            _ => SubmissionVerification::NotChecked,
        }
    }
}
/// Every exit before `submit_prompt` is a refusal: nothing was sent, so the
/// scheduler restores the ladder and retries on the refusal backoff (pacer
/// D5). Errors map by class; any other pre-send error is `Unavailable`
/// (documented choice). Cancellation is not a refusal and propagates to the
/// scheduler's existing `Cancelled` mapping.
fn refusal_for_error(err: &ApiError) -> Result<WakeOutcome, ApiError> {
    let cause = match err.code {
        ErrorCode::Cancelled => return Err(err.clone()),
        ErrorCode::DeadlineExceeded => RefusalCause::TimedOut,
        ErrorCode::TargetUnsafe | ErrorCode::TargetUnresolved => RefusalCause::Unsafe,
        _ => RefusalCause::Unavailable,
    };
    Ok(WakeOutcome::Refused(cause))
}

impl<H: HostPort + ?Sized, C: ReservationCheck + ?Sized> NotificationPort
    for NativeWakeDispatcher<'_, H, C>
{
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        if context.expected_boot.as_ref() != Some(&reservation.host_boot)
            || context.expected_epoch != Some(reservation.host_epoch)
        {
            return Ok(WakeOutcome::Refused(RefusalCause::Unsafe));
        }
        if context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::Refused(RefusalCause::TimedOut));
        }
        let read_deadline = context.budget.deadline.0.min(
            self.clock
                .monotonic_now()
                .0
                .saturating_add(TARGET_READ_MILLIS),
        );
        let read_context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(read_deadline),
                cancellation: context.budget.cancellation.clone(),
            },
            expected_boot: Some(reservation.host_boot.clone()),
            expected_epoch: Some(reservation.host_epoch),
        };
        let observation = match self
            .host
            .observe_current_target(&reservation.target, &read_context)
        {
            Ok(observation) => observation,
            Err(err) => return refusal_for_error(&err),
        };
        if read_context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::Refused(RefusalCause::TimedOut));
        }
        let cooperative = matches!(
            reservation.authority,
            ReservedWakeAuthority::Cooperative { .. }
        );
        let identity_ok = if cooperative {
            // Cooperative native policy: structural identity only; the
            // adapter's own recheck decides the occupant immediately before
            // submission (recognized idle harness, never shell/blocked).
            reservation.matches_cooperative_identity(&observation)
        } else {
            reservation.matches_fresh_identity(&observation)
                && observation.provenance == ObservationProvenance::FreshCurrentTarget
                && observation.ui == HostUiState::Idle
                && observation.occupancy == StructuralOccupancy::Occupied
                && matches!(
                    observation.incarnation,
                    IncarnationEvidence::Verified {
                        evidence_kind: EvidenceKind::NativeCurrentTarget,
                        ..
                    }
                )
                && matches!(
                    observation.execution,
                    ExecutionEvidence::Verified {
                        evidence_kind: EvidenceKind::NativeCurrentTarget,
                        ..
                    }
                )
        };
        if !identity_ok {
            return Ok(WakeOutcome::Refused(RefusalCause::Unsafe));
        }
        let Some(mut target) = self.host.safe_wake_target(&reservation.seat, &observation) else {
            return Ok(WakeOutcome::Refused(RefusalCause::Unsafe));
        };
        // The host cannot know the bound harness; the reservation does
        // (TRUST-POLICY A4 wake rule). No bound harness, no prompt: a
        // pre-send refusal, so the reminder ladder does not climb (pacer D5).
        if let ReservedWakeAuthority::Cooperative { harness, .. } = &reservation.authority {
            if harness.is_none() {
                return Ok(WakeOutcome::Refused(RefusalCause::Unsafe));
            }
            target.bound_harness = harness.clone();
        }
        let basis_ok = match (&target.basis, cooperative) {
            (WakeTargetBasis::CooperativeAgent, true) => true,
            (WakeTargetBasis::VerifiedOccupant { session, execution }, false) => {
                observation.occupant.as_ref().is_some_and(|occupant| {
                    &occupant.session == session && &occupant.execution == execution
                })
            }
            _ => false,
        };
        if target.seat != reservation.seat
            || target.target != reservation.target
            || target.host_boot != reservation.host_boot
            || target.epoch != reservation.host_epoch
            || target.generation != reservation.target_generation
            || !matches!(&observation.incarnation, IncarnationEvidence::Verified { identity, .. } if identity == &target.incarnation)
            || !basis_ok
            || observation.terminal.as_ref() != Some(&target.terminal)
        {
            return Ok(WakeOutcome::Refused(RefusalCause::Unsafe));
        }
        match self.check.is_current(&reservation, &context.budget) {
            Ok(true) => {}
            Ok(false) => return Ok(WakeOutcome::Refused(RefusalCause::Unsafe)),
            Err(err) => return refusal_for_error(&err),
        }
        if context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::Refused(RefusalCause::TimedOut));
        }
        let prompt_deadline = context
            .budget
            .deadline
            .0
            .min(self.clock.monotonic_now().0.saturating_add(PROMPT_MILLIS));
        let prompt_context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(prompt_deadline),
                cancellation: context.budget.cancellation.clone(),
            },
            expected_boot: Some(reservation.host_boot),
            expected_epoch: Some(reservation.host_epoch),
        };
        match self.host.submit_prompt(&target, MARKER, &prompt_context)? {
            PromptOutcome::Submitted => {
                let verification = self.verify_submission(&target, context);
                if let Ok(mut verifications) = self.verifications.lock() {
                    verifications.insert(reservation.seat.clone(), verification);
                }
                Ok(outcome_for_verification(verification))
            }
            PromptOutcome::OutcomeUnknown => Ok(WakeOutcome::OutcomeUnknown),
        }
    }

    fn take_verification(&self, seat: &SeatId) -> Option<SubmissionVerification> {
        self.verifications.lock().ok()?.remove(seat)
    }
}

impl From<RetryError> for DispatchError {
    fn from(error: RetryError) -> Self {
        Self::Retry(error)
    }
}

struct SeatAttempt {
    guard: RetryGuard,
    /// The guard as it was before the in-flight reservation; restored (no
    /// re-anchor) when a Refused completion's fenced store restore matched.
    prior_guard: Option<RetryGuard>,
    /// Pre-send refusal backoff (pacer D5), independent of the ladder.
    refusal: Backoff,
    active: Option<(WakeAttemptId, Uuid)>,
    seen_frontier: Option<crate::ports::LogicalAttentionFrontier>,
}

/// Reconstructed before dispatch on boot. Only a committed store reservation
/// may call `reserved`; caller proof and receipt state are outside this object.
pub struct DispatchState {
    config: RetryConfig,
    boot_mono: MonoInstant,
    daemon_boot: Uuid,
    seats: HashMap<SeatId, SeatAttempt>,
    active: usize,
    refusal_seed: Option<u64>,
}

impl DispatchState {
    pub fn new(config: RetryConfig, boot_mono: MonoInstant, daemon_boot: Uuid) -> Self {
        Self {
            config,
            boot_mono,
            daemon_boot,
            seats: HashMap::new(),
            active: 0,
            refusal_seed: None,
        }
    }

    /// Deterministic refusal-backoff jitter, for tests: each new seat's
    /// Backoff is seeded from `seed` and the number of seats tracked so far.
    pub fn with_refusal_seed(mut self, seed: u64) -> Self {
        self.refusal_seed = Some(seed);
        self
    }

    pub fn restore(&mut self, seat: SeatId, durable: DurableRetry) -> Result<(), DispatchError> {
        if let Some(state) = self.seats.get(&seat) {
            if state.active.is_some() {
                return Err(DispatchError::Retry(RetryError::InFlight));
            }
            return if state.guard.durable() == durable {
                Ok(())
            } else {
                Err(DispatchError::Retry(RetryError::InvalidHistory))
            };
        }
        let refusal = match self.refusal_seed {
            Some(seed) => Backoff::with_seed(seed.wrapping_add(self.seats.len() as u64)),
            None => Backoff::new(),
        };
        self.seats.insert(
            seat,
            SeatAttempt {
                guard: RetryGuard::from_durable(self.config, durable, self.boot_mono)?,
                prior_guard: None,
                refusal,
                active: None,
                seen_frontier: None,
            },
        );
        Ok(())
    }

    pub fn can_reserve(&self, seat: &SeatId, now: MonoInstant) -> bool {
        self.active < MAX_ACTIVE_PROMPTS
            && self.seats.get(seat).is_some_and(|state| {
                state.active.is_none()
                    && state.guard.eligible(now)
                    && (state.refusal.attempts() == 0
                        || state.refusal.next_retry_at().is_none_or(|at| now.0 >= at.0))
            })
    }

    /// Records how an attempt ended for the seat's refusal backoff. A pre-send
    /// refusal advances it; Submitted resets it; every other outcome (including
    /// OutcomeUnknown from an unsent prompt) leaves it untouched. Call only for
    /// the current attempt, after `finish` returned true.
    pub fn record_outcome(
        &mut self,
        seat: &SeatId,
        outcome: crate::ports::WakeOutcome,
        now: MonoInstant,
    ) {
        let Some(state) = self.seats.get_mut(seat) else {
            return;
        };
        match outcome {
            crate::ports::WakeOutcome::Refused(_) => {
                state.refusal.on_failure(now);
            }
            crate::ports::WakeOutcome::Submitted => state.refusal.on_success(),
            _ => {}
        }
    }

    /// The seat has no actionable work any more: nothing to retry.
    pub fn clear_refusal(&mut self, seat: &SeatId) {
        if let Some(state) = self.seats.get_mut(seat) {
            state.refusal.on_success();
        }
    }

    /// Idle seats currently in refusal backoff.
    pub fn refusing_seats(&self) -> Vec<SeatId> {
        self.seats
            .iter()
            .filter(|(_, state)| state.active.is_none() && state.refusal.attempts() > 0)
            .map(|(seat, _)| seat.clone())
            .collect()
    }

    pub fn refusal_attempts(&self, seat: &SeatId) -> Option<u32> {
        self.seats.get(seat).map(|state| state.refusal.attempts())
    }

    /// Restore the seat's pre-reservation RetryGuard (no re-anchor) after a
    /// Refused completion whose fenced store restore matched. New attention
    /// that arrived in flight stays applied.
    pub fn restore_prior_guard(&mut self, seat: &SeatId) {
        if let Some(state) = self.seats.get_mut(seat)
            && state.active.is_none()
            && let Some(prior) = state.prior_guard.take()
        {
            state.guard = state.guard.restored_to(prior);
        }
    }

    /// Earliest instant a tracked, idle seat in refusal backoff can be tried
    /// again: its refusal instant, or its ladder instant when that is later.
    /// A seat that is only waiting on the ladder counts while still in the
    /// future. In-flight seats are excluded.
    pub fn next_due_at(&self, now: MonoInstant) -> Option<MonoInstant> {
        self.seats
            .values()
            .filter(|state| state.active.is_none())
            .filter_map(|state| {
                let ladder = state.guard.earliest().ok().flatten();
                match state.refusal.next_retry_at() {
                    Some(refusal) if state.refusal.attempts() > 0 => {
                        Some(MonoInstant(refusal.0.max(ladder.map_or(0, |at| at.0))))
                    }
                    _ => ladder.filter(|at| at.0 > now.0),
                }
            })
            .min_by_key(|at| at.0)
    }

    pub fn has_seat(&self, seat: &SeatId) -> bool {
        self.seats.contains_key(seat)
    }

    /// The durable store reservation has already committed. Sample `now` after
    /// commit so queue/write latency cannot consume the elapsed retry floor.
    pub fn reserved(
        &mut self,
        seat: SeatId,
        attempt: WakeAttemptId,
        boot: Uuid,
        now: MonoInstant,
    ) -> Result<DurableRetry, DispatchError> {
        if self.active >= MAX_ACTIVE_PROMPTS {
            return Err(DispatchError::ActiveLimit);
        }
        if boot != self.daemon_boot {
            return Err(DispatchError::WrongBoot);
        }
        let state = self
            .seats
            .get_mut(&seat)
            .ok_or(DispatchError::UnknownSeat)?;
        let prior = state.guard;
        let (guard, durable) = state.guard.reserve(now)?;
        state.prior_guard = Some(prior);
        state.guard = guard;
        state.active = Some((attempt, boot));
        self.active += 1;
        Ok(durable)
    }

    /// Late old-attempt/boot results cannot clear a successor's slot or guard.
    pub fn finish(
        &mut self,
        seat: &SeatId,
        attempt: &WakeAttemptId,
        boot: &Uuid,
        now: MonoInstant,
    ) -> Result<bool, DispatchError> {
        let state = self.seats.get_mut(seat).ok_or(DispatchError::UnknownSeat)?;
        if state.active.as_ref() != Some(&(attempt.clone(), *boot)) {
            return Ok(false);
        }
        state.guard = state.guard.complete(now)?;
        state.active = None;
        self.active -= 1;
        Ok(true)
    }

    pub fn new_attention(&mut self, seat: &SeatId) -> Result<(), DispatchError> {
        let state = self.seats.get_mut(seat).ok_or(DispatchError::UnknownSeat)?;
        state.guard = state.guard.new_attention()?;
        Ok(())
    }

    /// A completed logical scan may shorten a retained retry only when an
    /// immutable publication key advances. The persisted reservation frontier
    /// supplies the initial comparison after restart; subsequent discoveries
    /// compare with the greatest frontier seen by this boot, including work
    /// that arrived while an attempt was in flight.
    pub fn observe_frontier(
        &mut self,
        seat: &SeatId,
        reserved: crate::ports::LogicalAttentionFrontier,
        current: crate::ports::LogicalAttentionFrontier,
    ) -> Result<(), DispatchError> {
        let state = self.seats.get_mut(seat).ok_or(DispatchError::UnknownSeat)?;
        let prior =
            state
                .seen_frontier
                .map_or(reserved, |seen| crate::ports::LogicalAttentionFrontier {
                    invitation: seen.invitation.max(reserved.invitation),
                    addressed_receipt: seen.addressed_receipt.max(reserved.addressed_receipt),
                    actionable_warning: seen.actionable_warning.max(reserved.actionable_warning),
                });
        if state.guard.durable().ever_reserved && current.advanced_beyond(&prior) {
            state.guard = state.guard.new_attention()?;
        }
        state.seen_frontier = Some(crate::ports::LogicalAttentionFrontier {
            invitation: prior.invitation.max(current.invitation),
            addressed_receipt: prior.addressed_receipt.max(current.addressed_receipt),
            actionable_warning: prior.actionable_warning.max(current.actionable_warning),
        });
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.active
    }
}
