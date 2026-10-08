//! Process-local wake admission and elapsed retry state.

use std::{collections::HashMap, sync::Mutex};
use uuid::Uuid;

use crate::service::pacer::Backoff;

use crate::{
    harness::recipe::PokeCapabilities,
    notification::policy::{
        DurableRetry, MARKER, PokeDecision, RetryConfig, RetryError, RetryGuard, poke_eligibility,
    },
    ports::{
        AgentComposerState, EvidenceKind, ExecutionEvidence, HostCallContext, HostPort,
        HostUiState, IncarnationEvidence, NotificationPort, ObservationProvenance, PokeAttempt,
        PokeCapabilitySource, PokeMode, PokePlan, PromptOutcome, RefusalCause,
        ReservedWakeAuthority, StructuralOccupancy, WakeOutcome, WakeReservation, WakeTargetBasis,
    },
    protocol::{
        authority::Harness,
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

    /// Advisory read-only verification. A held marker is reported without
    /// pressing Enter: the composer may also contain newly typed user input.
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
            Ok(AgentComposerState::HoldingPrompt) => SubmissionVerification::Unsubmitted,
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
impl<H: HostPort + ?Sized, C: ReservationCheck + ?Sized> NativeWakeDispatcher<'_, H, C> {
    /// One reserved attempt. `poke: None` is an ordinary wake. With a poke the
    /// same identity, fence and prompt-budget checks apply; the poke text is
    /// submitted only when `poke_eligibility` accepts a fresh observation.
    ///
    /// Every exit of a wake (including a wake that carries poke text) before
    /// `submit_prompt` is a `Refused` (pacer D5): nothing was sent, so the
    /// scheduler restores the ladder and retries on the refusal backoff. A
    /// scheduled `PokeOnly` attempt never moved the ladder, so its pre-send
    /// skips keep their plain outcomes and leave `soft_poked_at` unset.
    fn attempt(
        &self,
        reservation: WakeReservation,
        poke: Option<PokeRequest<'_>>,
        context: &HostCallContext,
    ) -> Result<PokeAttempt, ApiError> {
        // A scheduled poke decides the UI state itself (`poke_eligibility`),
        // so its identity check carries no UI exclusion.
        let poke_only = poke
            .as_ref()
            .is_some_and(|request| request.mode == PokeMode::PokeOnly);
        let outcome = |outcome| {
            Ok(PokeAttempt {
                outcome,
                poked: false,
                diagnostic: None,
            })
        };
        // A pre-send exit: `Refused(cause)` for a wake, the plain outcome
        // (`poke_skip`) for a scheduled poke.
        let refuse = |cause: RefusalCause, poke_skip: WakeOutcome| {
            outcome(if poke_only {
                poke_skip
            } else {
                WakeOutcome::Refused(cause)
            })
        };
        let refuse_error = |err: ApiError| {
            if poke_only {
                return Err(err);
            }
            refusal_for_error(&err).and_then(outcome)
        };
        if context.expected_boot.as_ref() != Some(&reservation.host_boot)
            || context.expected_epoch != Some(reservation.host_epoch)
        {
            return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe);
        }
        if context.budget.is_exhausted(self.clock) {
            return refuse(RefusalCause::TimedOut, WakeOutcome::TimedOut);
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
        // Only a poke classifies the composer; an ordinary wake never pays for
        // (or depends on) that read.
        let observed = if poke.is_some() {
            self.host
                .observe_current_target_for_poke(&reservation.target, &read_context)
        } else {
            self.host
                .observe_current_target(&reservation.target, &read_context)
        };
        let observation = match observed {
            Ok(observation) => observation,
            Err(err) => return refuse_error(err),
        };
        if read_context.budget.is_exhausted(self.clock) {
            return refuse(RefusalCause::TimedOut, WakeOutcome::TimedOut);
        }
        let cooperative = matches!(
            reservation.authority,
            ReservedWakeAuthority::Cooperative { .. }
        );
        let identity_ok = if cooperative {
            // Cooperative native policy: structural identity only; the
            // adapter's own recheck decides the occupant immediately before
            // submission (recognized idle harness, never shell/blocked).
            if poke_only {
                reservation.matches_cooperative_structure(&observation)
            } else {
                reservation.matches_cooperative_identity(&observation)
            }
        } else {
            reservation.matches_fresh_identity(&observation)
                && observation.provenance == ObservationProvenance::FreshCurrentTarget
                && (poke_only || observation.ui == HostUiState::Idle)
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
            return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe);
        }
        if observation.ui == HostUiState::HumanInput {
            return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe);
        }
        let selected = if poke.is_some() {
            self.host.safe_poke_target(&reservation.seat, &observation)
        } else {
            self.host.safe_wake_target(&reservation.seat, &observation)
        };
        let Some(mut target) = selected else {
            return refuse(RefusalCause::Unsafe, WakeOutcome::Unavailable);
        };
        // The host cannot know the bound harness; the reservation does
        // (TRUST-POLICY A4 wake rule). No bound harness, no prompt: a
        // pre-send refusal, so the reminder ladder does not climb (pacer D5).
        if let ReservedWakeAuthority::Cooperative { harness, .. } = &reservation.authority {
            if harness.is_none() {
                return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe);
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
            return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe);
        }
        // Spec §10: decided from this fresh observation, immediately before
        // the prompt. A skip sends nothing and leaves `soft_poked_at` unset;
        // the next tick re-evaluates until the hard deadline.
        let decision = poke.as_ref().map(|request| {
            let bound_native_agent = match &reservation.authority {
                ReservedWakeAuthority::Registered { .. } => true,
                ReservedWakeAuthority::Cooperative { harness, .. } => harness.is_some(),
                ReservedWakeAuthority::RecoveryHint { .. } => false,
            };
            // Cooperative native reads never name an occupant (Herdr's cached
            // agent is not proof); the adapter's own agent-kind recheck
            // against the bound harness stands in for recognition there.
            let recognized = observation
                .occupant
                .as_ref()
                .is_some_and(|occupant| occupant.is_top_level)
                || (cooperative && target.bound_harness.is_some());
            let harness = observation
                .occupant
                .as_ref()
                .map(|occupant| occupant.harness)
                .or(match target.bound_harness.as_deref() {
                    Some("codex") => Some(Harness::Codex),
                    Some("claude") => Some(Harness::Claude),
                    _ => None,
                });
            let caps = harness.map_or(PokeCapabilities::NONE, |harness| {
                request.caps.capabilities(harness)
            });
            poke_eligibility(&observation, bound_native_agent, recognized, caps)
        });
        // Attention never stashes a draft. Nonempty input was refused above;
        // an ineligible coalesced poke leaves receipts unpoked.
        let using_poke = match (&decision, poke_only) {
            (Some(PokeDecision::Submit), _) => true,
            (_, true) => return outcome(WakeOutcome::Unsafe),
            _ => false,
        };
        match self.check.is_current(&reservation, &context.budget) {
            Ok(true) => {}
            Ok(false) => return refuse(RefusalCause::Unsafe, WakeOutcome::Unsafe),
            Err(err) => return refuse_error(err),
        }
        if context.budget.is_exhausted(self.clock) {
            return refuse(RefusalCause::TimedOut, WakeOutcome::TimedOut);
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
            expected_boot: Some(reservation.host_boot.clone()),
            expected_epoch: Some(reservation.host_epoch),
        };
        let text = match (&poke, using_poke) {
            (Some(request), true) => request.plan.text.as_str(),
            _ => MARKER,
        };
        // A poke accepted for a running turn queues into it; the host's
        // recheck then allows `working` for this call only.
        let during_turn = using_poke && observation.ui == HostUiState::ActiveTurn;
        let submitted = if during_turn {
            self.host
                .submit_prompt_during_turn(&target, text, &prompt_context)
        } else {
            self.host.submit_prompt(&target, text, &prompt_context)
        };
        let diagnostic = None;
        let submitted = match submitted {
            Ok(submitted) => submitted,
            Err(err) => return refuse_error(err),
        };
        match submitted {
            // A turn-time poke is queued; otherwise verification only reads.
            PromptOutcome::Submitted if during_turn => Ok(PokeAttempt {
                outcome: WakeOutcome::Submitted,
                poked: using_poke,
                diagnostic,
            }),
            PromptOutcome::Submitted => {
                let verification = self.verify_submission(&target, context);
                if let Ok(mut verifications) = self.verifications.lock() {
                    verifications.insert(reservation.seat.clone(), verification);
                }
                let outcome = outcome_for_verification(verification);
                Ok(PokeAttempt {
                    outcome,
                    poked: using_poke && outcome == WakeOutcome::Submitted,
                    diagnostic,
                })
            }
            PromptOutcome::OutcomeUnknown => Ok(PokeAttempt {
                outcome: WakeOutcome::OutcomeUnknown,
                poked: false,
                diagnostic,
            }),
        }
    }
}

/// What a poke-carrying attempt needs beyond the reservation.
struct PokeRequest<'a> {
    plan: &'a PokePlan,
    mode: PokeMode,
    caps: &'a dyn PokeCapabilitySource,
}

impl<H: HostPort + ?Sized, C: ReservationCheck + ?Sized> NotificationPort
    for NativeWakeDispatcher<'_, H, C>
{
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.attempt(reservation, None, context)
            .map(|attempt| attempt.outcome)
    }

    fn attempt_poke(
        &self,
        reservation: WakeReservation,
        plan: &PokePlan,
        mode: PokeMode,
        caps: &dyn PokeCapabilitySource,
        context: &HostCallContext,
    ) -> Result<PokeAttempt, ApiError> {
        self.attempt(reservation, Some(PokeRequest { plan, mode, caps }), context)
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

/// A seat's soft-poke state. A poke takes the seat's single in-flight slot and
/// counts toward the shared four-active limit, but never advances the wake
/// retry guard. A skipped attempt is re-evaluated once the wake retry spacing
/// has elapsed since it ended (`last_skipped`), so a seat that stays
/// ineligible costs one observation per spacing, not one per tick.
#[derive(Default)]
struct PokeSeat {
    active: Option<(WakeAttemptId, Uuid)>,
    last_submitted: Option<MonoInstant>,
    /// When the seat's last attempt ended without a submitted poke (a skip,
    /// an unsafe or failed attempt, or a reservation the store refused). In memory only, like `last_submitted`: a
    /// restart re-evaluates every seat once.
    last_skipped: Option<MonoInstant>,
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
    /// When the seat's last wake attempt was reserved or finished, for the
    /// per-seat elapsed spacing a poke honours without the wake retry backoff.
    /// After a restart, `boot_mono` when the seat was ever reserved.
    last_attempt_at: Option<MonoInstant>,
}

/// Reconstructed before dispatch on boot. Only a committed store reservation
/// may call `reserved`; caller proof and receipt state are outside this object.
pub struct DispatchState {
    config: RetryConfig,
    boot_mono: MonoInstant,
    daemon_boot: Uuid,
    seats: HashMap<SeatId, SeatAttempt>,
    pokes: HashMap<SeatId, PokeSeat>,
    batches: HashMap<SeatId, crate::notification::policy::BatchGuard>,
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
            pokes: HashMap::new(),
            batches: HashMap::new(),
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
        let last_attempt_at = durable.ever_reserved.then_some(self.boot_mono);
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
                last_attempt_at,
            },
        );
        Ok(())
    }

    pub fn clear_batch(&mut self, seat: &SeatId) {
        self.batches.remove(seat);
    }

    pub fn batch_eligible(
        &mut self,
        seat: &SeatId,
        window: Option<(crate::protocol::time::UtcMillis, u64)>,
        utc: crate::protocol::time::UtcMillis,
        now: MonoInstant,
    ) -> bool {
        let Some((deadline, delay)) = window else {
            return true;
        };
        let guard = self.batches.entry(seat.clone()).or_insert_with(|| {
            crate::notification::policy::BatchGuard::restore(deadline, delay, utc, now)
        });
        if guard.deadline != deadline {
            *guard = crate::notification::policy::BatchGuard::restore(deadline, delay, utc, now);
        }
        guard.eligible(now)
    }

    pub fn can_reserve(&self, seat: &SeatId, now: MonoInstant) -> bool {
        self.active < MAX_ACTIVE_PROMPTS
            && self
                .pokes
                .get(seat)
                .is_none_or(|poke| poke.active.is_none())
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
        let batch = self
            .batches
            .values()
            .map(|guard| guard.mature_at)
            .filter(|at| at.0 > now.0)
            .min_by_key(|at| at.0);
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
            .chain(batch)
            .min_by_key(|at| at.0)
    }

    /// Spec §10 limits only: four active prompts, one in-flight attempt per seat
    /// (poke or wake), the configured minimum since the seat's last wake attempt,
    /// and the same minimum after the seat's last submitted or skipped poke.
    /// Never the wake retry backoff (`RetryGuard::eligible`): a seat whose wakes
    /// back off to 300 s is still poked inside its soft window (ht-2i4).
    pub fn can_reserve_poke(&self, seat: &SeatId, now: MonoInstant) -> bool {
        if self.active >= MAX_ACTIVE_PROMPTS {
            return false;
        }
        let spacing = self.config.minimum_delay_ms();
        let spaced =
            |at: Option<MonoInstant>| at.is_none_or(|at| now.0 >= at.0.saturating_add(spacing));
        let poke = self.pokes.get(seat);
        if poke.is_some_and(|poke| poke.active.is_some())
            || !spaced(poke.and_then(|poke| poke.last_submitted.max(poke.last_skipped)))
        {
            return false;
        }
        self.seats
            .get(seat)
            .is_none_or(|state| state.active.is_none() && spaced(state.last_attempt_at))
    }

    /// The durable poke reservation has already committed.
    pub fn poke_reserved(
        &mut self,
        seat: SeatId,
        attempt: WakeAttemptId,
        boot: Uuid,
    ) -> Result<(), DispatchError> {
        if self.active >= MAX_ACTIVE_PROMPTS {
            return Err(DispatchError::ActiveLimit);
        }
        if boot != self.daemon_boot {
            return Err(DispatchError::WrongBoot);
        }
        self.pokes.entry(seat).or_default().active = Some((attempt, boot));
        self.active += 1;
        Ok(())
    }

    /// The store refused the poke reservation (no current authority, the
    /// receipts no longer due, a reservation already active): back off like a
    /// skipped attempt. Never touches an active slot or the active count.
    pub fn poke_unreserved(&mut self, seat: &SeatId, now: MonoInstant) {
        let poke = self.pokes.entry(seat.clone()).or_default();
        if poke.active.is_none() {
            poke.last_skipped = Some(now);
        }
    }

    /// Late old-attempt/boot results cannot clear a successor's slot.
    pub fn poke_finished(
        &mut self,
        seat: &SeatId,
        attempt: &WakeAttemptId,
        boot: &Uuid,
        submitted: bool,
        now: MonoInstant,
    ) -> bool {
        let Some(poke) = self.pokes.get_mut(seat) else {
            return false;
        };
        if poke.active.as_ref() != Some(&(attempt.clone(), *boot)) {
            return false;
        }
        poke.active = None;
        if submitted {
            poke.last_submitted = Some(now);
            poke.last_skipped = None;
        } else {
            poke.last_skipped = Some(now);
        }
        self.active -= 1;
        true
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
        state.last_attempt_at = Some(now);
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
        state.last_attempt_at = Some(now);
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
