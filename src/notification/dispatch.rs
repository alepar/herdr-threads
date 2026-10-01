//! Process-local wake admission and elapsed retry state.

use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    notification::policy::{DurableRetry, MARKER, RetryConfig, RetryError, RetryGuard},
    ports::{
        EvidenceKind, ExecutionEvidence, HostCallContext, HostPort, HostUiState,
        IncarnationEvidence, NotificationPort, ObservationProvenance, PromptOutcome,
        ReservedWakeAuthority, StorePort, StructuralOccupancy, WakeOutcome, WakeReservation,
        WakeTargetBasis,
    },
    protocol::{
        ids::{SeatId, WakeAttemptId},
        results::ApiError,
        time::{CallBudget, Clock, MonoInstant},
    },
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
impl<T: StorePort + ?Sized> ReservationCheck for T {
    fn is_current(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        StorePort::validate_wake_reservation(self, reservation, budget)
    }
}

pub struct NativeWakeDispatcher<'a, H: HostPort + ?Sized, C: ReservationCheck + ?Sized> {
    host: &'a H,
    check: &'a C,
    clock: &'a dyn Clock,
}
impl<'a, H: HostPort + ?Sized, C: ReservationCheck + ?Sized> NativeWakeDispatcher<'a, H, C> {
    pub fn new(host: &'a H, check: &'a C, clock: &'a dyn Clock) -> Self {
        Self { host, check, clock }
    }
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
            return Ok(WakeOutcome::Unsafe);
        }
        if context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::TimedOut);
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
        let observation = self
            .host
            .observe_current_target(&reservation.target, &read_context)?;
        if read_context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::TimedOut);
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
            return Ok(WakeOutcome::Unsafe);
        }
        let Some(mut target) = self.host.safe_wake_target(&reservation.seat, &observation) else {
            return Ok(WakeOutcome::Unavailable);
        };
        // The host cannot know the bound harness; the reservation does
        // (TRUST-POLICY A4 wake rule).
        if let ReservedWakeAuthority::Cooperative { harness, .. } = &reservation.authority {
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
            return Ok(WakeOutcome::Unsafe);
        }
        if !self.check.is_current(&reservation, &context.budget)? {
            return Ok(WakeOutcome::Unsafe);
        }
        if context.budget.is_exhausted(self.clock) {
            return Ok(WakeOutcome::TimedOut);
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
            PromptOutcome::Submitted => Ok(WakeOutcome::Submitted),
            PromptOutcome::OutcomeUnknown => Ok(WakeOutcome::OutcomeUnknown),
        }
    }
}

impl From<RetryError> for DispatchError {
    fn from(error: RetryError) -> Self {
        Self::Retry(error)
    }
}

struct SeatAttempt {
    guard: RetryGuard,
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
}

impl DispatchState {
    pub fn new(config: RetryConfig, boot_mono: MonoInstant, daemon_boot: Uuid) -> Self {
        Self {
            config,
            boot_mono,
            daemon_boot,
            seats: HashMap::new(),
            active: 0,
        }
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
        self.seats.insert(
            seat,
            SeatAttempt {
                guard: RetryGuard::from_durable(self.config, durable, self.boot_mono)?,
                active: None,
                seen_frontier: None,
            },
        );
        Ok(())
    }

    pub fn can_reserve(&self, seat: &SeatId, now: MonoInstant) -> bool {
        self.active < MAX_ACTIVE_PROMPTS
            && self
                .seats
                .get(seat)
                .is_some_and(|state| state.active.is_none() && state.guard.eligible(now))
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
        let (guard, durable) = state.guard.reserve(now)?;
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
