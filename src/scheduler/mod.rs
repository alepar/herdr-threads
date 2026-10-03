//! Bounded scheduler composition. Host attempts run outside the store writer.

use std::{
    collections::VecDeque,
    sync::{Mutex, mpsc},
    time::Duration,
};

use self::deadlines::{DeadlineDriver, DeadlinePort, DriveOutcome};
use crate::{
    notification::{
        dispatch::{DispatchState, MAX_ACTIVE_PROMPTS},
        policy::{AttentionSnapshot, DurableRetry, RetryConfig, poke_text},
    },
    ports::{
        HostCallContext, NoPokeCapabilities, NotificationPort, PokeAttempt, PokeCapabilitySource,
        PokeDue, PokeMode, PokePlan, PokeReceipt, PokeReservation, PriorLadder, WakeCandidate,
        WakeOutcome, WakeRecoveryCandidate, WakeRecoveryOutcome, WakeRecoveryRequest,
        WakeReservation,
    },
    protocol::{
        ids::{SeatId, ThreadId, WakeAttemptId},
        pagination::{Page, PageRequest},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};

pub mod config;
pub mod deadlines;

const MAX_ATTEMPT_MILLIS: u64 = 5_000;
const COMPLETION_BUDGET_MILLIS: u64 = 2_000;
const WAKE_PAGE_LIMIT: u16 = 16;
const WAKE_PAGE_BYTES: u32 = 16_384;
/// Admitted seats a tick attempts a soft poke for (spec §10). Applied after
/// the in-memory admission filter, so seats that cannot be poked now never
/// crowd out ones that can.
const POKE_SEAT_LIMIT: u16 = 16;
/// Due seats asked of the store per tick. The store's scan is bounded by its
/// own per-source row cap, so asking for every due seat adds no store work.
const POKE_CANDIDATE_LIMIT: u16 = u16::MAX;

/// The narrow store boundary between the wake lane and its store: production
/// implements it with `ScheduledStore` (and `ObservedWakePort`); tests with
/// explicit fakes or a `ScheduledStore` over a real `SqliteStore`.
pub trait WakePort: Send + Sync {
    fn clock(&self) -> &dyn Clock;
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError>;
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError>;
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError>;
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError>;
    /// Soft-deadline poke boundary (spec §10). The defaults have no pokes, so
    /// a port that predates them keeps its behavior.
    fn poke_candidates(&self, _limit: u16, _budget: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        Ok(Vec::new())
    }
    fn poke_for_wake(
        &self,
        _seat: &SeatId,
        _budget: &CallBudget,
    ) -> Result<Option<PokeDue>, ApiError> {
        Ok(None)
    }
    fn reserve_poke(
        &self,
        _due: &PokeDue,
        _budget: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        Ok(None)
    }
    fn complete_poke(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        _receipts: &[PokeReceipt],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        // A port without pokes can only settle the slot as an ordinary wake;
        // a poke never advanced the ladder, so there is nothing to restore.
        self.complete_wake(attempt, outcome, None, budget)
            .map(|_| ())
    }
}
#[derive(Default)]
struct WakeScanState {
    cursor: Option<String>,
    pending: VecDeque<WakeCandidate>,
    /// Seats the current scan cycle (first page to last) has listed.
    seen: std::collections::HashSet<SeatId>,
}
#[derive(Default)]
struct RecoveryScanState {
    cursor: Option<String>,
    pending: VecDeque<WakeRecoveryCandidate>,
    complete: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WakeDriveOutcome {
    pub recovered: u16,
    pub examined: u16,
    pub attempted: u16,
    pub has_more: bool,
    /// Per-drive verification results of prompts this drive actually sent, as
    /// the notifier reported them; an unsent attempt (refused, cancelled, timed
    /// out, unavailable, unsent outcome-unknown) or a notifier that does not
    /// verify adds no entry.
    pub verification: Vec<(SeatId, SubmissionVerification)>,
    /// Earliest instant a seat in refusal backoff (or waiting on its ladder)
    /// can be tried again; the deadline/wake lane sleeps until then.
    pub next_due_at: Option<MonoInstant>,
}

/// Whether a sent wake prompt was seen submitted (the dispatcher performs the
/// check and the single submit-key retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmissionVerification {
    #[default]
    NotChecked,
    Verified,
    Retried,
    Unsubmitted,
}

impl SubmissionVerification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotChecked => "not_checked",
            Self::Verified => "verified",
            Self::Retried => "retried",
            Self::Unsubmitted => "unsubmitted",
        }
    }
}

/// The decision as code (ht-p03.41): a prompt still unsent after the single
/// submit-key retry was delivered to the pane, so it maps to OutcomeUnknown
/// (keeps the advanced ladder step, never re-sent in a loop); the existing
/// OutcomeUnknown last_outcome string is stored. 'unsubmitted' lives only in
/// WakeDriveOutcome::verification and the daemon.log line.
pub fn outcome_for_verification(v: SubmissionVerification) -> WakeOutcome {
    match v {
        SubmissionVerification::Unsubmitted => WakeOutcome::OutcomeUnknown,
        _ => WakeOutcome::Submitted,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PokeDriveOutcome {
    pub examined: u16,
    pub attempted: u16,
}

/// The deadline and wake lanes have separate locks. A caller can drive due
/// work on another worker while a synchronous host call is still returning.
/// The host adapter must honor its absolute deadline and cancellation token.
pub struct Scheduler<
    'a,
    D: DeadlinePort + ?Sized,
    W: WakePort + ?Sized,
    N: NotificationPort + ?Sized,
> {
    deadlines: Mutex<DeadlineDriver<'a, D>>,
    wakes: WakeRunner<'a, W, N>,
    instance: String,
    daemon_boot: uuid::Uuid,
    recovery: Mutex<RecoveryScanState>,
    scan: Mutex<WakeScanState>,
}
impl<'a, D: DeadlinePort + ?Sized, W: WakePort + ?Sized, N: NotificationPort + ?Sized>
    Scheduler<'a, D, W, N>
{
    pub fn new(
        instance: String,
        deadline_port: &'a D,
        wake_port: &'a W,
        notifier: &'a N,
        config: RetryConfig,
        daemon_boot: uuid::Uuid,
    ) -> Self {
        Self {
            deadlines: Mutex::new(DeadlineDriver::new(deadline_port)),
            wakes: WakeRunner::new(wake_port, notifier, config, daemon_boot),
            instance,
            daemon_boot,
            recovery: Mutex::new(RecoveryScanState::default()),
            scan: Mutex::new(WakeScanState::default()),
        }
    }

    /// Supplies the harness recipes' poke capability evidence.
    pub fn with_poke_capabilities(mut self, caps: &'a dyn PokeCapabilitySource) -> Self {
        self.wakes.caps = caps;
        self
    }

    /// Spec §10: examines the seats whose receipts are past their soft point
    /// and attempts one coalesced poke each, through the wake dispatcher's
    /// limits. Evaluated lazily every tick, but a seat whose reservation was
    /// refused, or whose attempt was skipped or failed, is not re-attempted before the minimum spacing
    /// elapses; it leaves `soft_poked_at` unset, so it is re-evaluated until
    /// the hard deadline. Seats that cannot be admitted now (spec §10 limits
    /// only, never the wake retry backoff) are filtered out before the
    /// per-tick cap of `POKE_SEAT_LIMIT`.
    pub fn drive_pokes(&self, budget: &CallBudget) -> Result<PokeDriveOutcome, ApiError> {
        if budget.is_exhausted(self.wakes.store.clock()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "poke scan deadline exhausted",
            ));
        }
        let completion_error = self.wakes.retry_completions(budget)?;
        let mut outcome = PokeDriveOutcome::default();
        let candidates = self
            .wakes
            .store
            .poke_candidates(POKE_CANDIDATE_LIMIT, budget)?;
        let admitted: Vec<PokeDue> = {
            let state = self
                .wakes
                .state
                .lock()
                .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
            let now = self.wakes.store.clock().monotonic_now();
            candidates
                .into_iter()
                .filter(|due| !due.receipts.is_empty() && poke_admissible(&state, &due.seat, now))
                .take(usize::from(POKE_SEAT_LIMIT))
                .collect()
        };
        for due in admitted {
            if budget.is_exhausted(self.wakes.store.clock()) {
                break;
            }
            outcome.examined += 1;
            if self.wakes.try_poke(&due, budget)?.is_some() {
                outcome.attempted += 1;
            }
        }
        match completion_error {
            Some(err) => Err(err),
            None => Ok(outcome),
        }
    }

    pub fn drive_deadlines(&self, budget: &CallBudget) -> Result<DriveOutcome, ApiError> {
        self.deadlines
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "deadline driver lock poisoned"))?
            .drive(budget)
    }

    pub fn after_committed_fence(&self) -> Result<(), ApiError> {
        self.deadlines
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "deadline driver lock poisoned"))?
            .after_committed_fence();
        Ok(())
    }

    /// Examines at most one bounded page per call. Unvisited rows from a
    /// partially attempted page remain in memory, preserving exact position.
    pub fn drive_wakes(&self, budget: &CallBudget) -> Result<WakeDriveOutcome, ApiError> {
        if budget.is_exhausted(self.wakes.store.clock()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "wake scan deadline exhausted",
            ));
        }
        let completion_error = self.wakes.retry_completions(budget)?;
        let mut outcome = WakeDriveOutcome::default();
        if !self.drive_recovery(budget, &mut outcome)? {
            outcome.has_more = true;
            return Ok(outcome);
        }
        if budget.is_exhausted(self.wakes.store.clock()) {
            outcome.has_more = true;
            return match completion_error {
                Some(err) => Err(err),
                None => Ok(outcome),
            };
        }
        let mut scan = self
            .scan
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake scan lock poisoned"))?;
        if scan.pending.is_empty() {
            let page = self.wakes.store.wake_candidates(
                PageRequest {
                    cursor: scan.cursor.clone(),
                    limit: WAKE_PAGE_LIMIT,
                    max_bytes: WAKE_PAGE_BYTES,
                },
                budget,
            );
            let page = match page {
                Ok(page) => page,
                Err(err) => {
                    if err.code == ErrorCode::CursorStale {
                        scan.cursor = None;
                    }
                    return Err(err);
                }
            };
            if page.items.len() > usize::from(WAKE_PAGE_LIMIT)
                || (page.has_more && page.next_cursor.is_none())
            {
                return Err(error(ErrorCode::StoreCorrupt, "invalid bounded wake page"));
            }
            scan.cursor = if page.has_more {
                page.next_cursor
            } else {
                None
            };
            scan.seen
                .extend(page.items.iter().map(|candidate| candidate.seat.clone()));
            scan.pending = page.items.into();
        }
        drop(scan);
        loop {
            if budget.is_exhausted(self.wakes.store.clock()) {
                break;
            }
            let candidate = {
                self.scan
                    .lock()
                    .map_err(|_| error(ErrorCode::StoreCorrupt, "wake scan lock poisoned"))?
                    .pending
                    .pop_front()
            };
            let Some(candidate) = candidate else {
                break;
            };
            outcome.examined += 1;
            let attempt = self.wakes.try_candidate(&candidate, budget);
            // Taken after every try (ht-p03.141): the notifier stores an entry
            // only for a prompt this try actually sent, so an unsent attempt (a
            // pre-send refusal, Cancelled, TimedOut, Unavailable, an unsent
            // OutcomeUnknown) reports nothing. Draining on error too means an
            // entry a failed completion left behind can never be reported
            // against a later attempt.
            let verification = self.wakes.notifier.take_verification(&candidate.seat);
            match attempt {
                Ok(Some(_)) => {
                    outcome.attempted += 1;
                    if let Some(verification) = verification {
                        outcome
                            .verification
                            .push((candidate.seat.clone(), verification));
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    self.scan
                        .lock()
                        .map_err(|_| error(ErrorCode::StoreCorrupt, "wake scan lock poisoned"))?
                        .pending
                        .push_front(candidate);
                    return Err(err);
                }
            }
        }
        let mut scan = self
            .scan
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake scan lock poisoned"))?;
        if scan.pending.is_empty() && scan.cursor.is_none() {
            // A whole cycle ended: a seat in refusal backoff that no page
            // listed has no wake work left, so its backoff has nothing to
            // retry. Left in place its past retry time would hold the lane
            // at its minimum wait forever.
            let seen = std::mem::take(&mut scan.seen);
            self.wakes.clear_refusals_not_in(&seen)?;
        }
        outcome.has_more = !scan.pending.is_empty()
            || scan.cursor.is_some()
            || self.wakes.has_pending_completions()?;
        outcome.next_due_at = self.wakes.next_due_at()?;
        match completion_error {
            Some(err) => Err(err),
            None => Ok(outcome),
        }
    }

    /// Recovery is independent of current attention and target eligibility.
    /// A page is bounded, and a partial page retains exact position on error or
    /// budget exhaustion. Normal wake admission begins only after the sweep.
    fn drive_recovery(
        &self,
        budget: &CallBudget,
        outcome: &mut WakeDriveOutcome,
    ) -> Result<bool, ApiError> {
        let mut scan = self
            .recovery
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake recovery lock poisoned"))?;
        if scan.complete {
            return Ok(true);
        }
        if scan.pending.is_empty() {
            let page = self.wakes.store.wake_recovery_candidates(
                PageRequest {
                    cursor: scan.cursor.clone(),
                    limit: WAKE_PAGE_LIMIT,
                    max_bytes: WAKE_PAGE_BYTES,
                },
                budget,
            );
            let page = match page {
                Ok(page) => page,
                Err(err) => {
                    if err.code == ErrorCode::CursorStale {
                        scan.cursor = None;
                    }
                    return Err(err);
                }
            };
            if page.items.len() > usize::from(WAKE_PAGE_LIMIT)
                || (page.has_more && page.next_cursor.is_none())
            {
                return Err(error(
                    ErrorCode::StoreCorrupt,
                    "invalid bounded recovery page",
                ));
            }
            scan.cursor = if page.has_more {
                page.next_cursor
            } else {
                None
            };
            scan.pending = page.items.into();
        }
        while !budget.is_exhausted(self.wakes.store.clock()) {
            let Some(candidate) = scan.pending.pop_front() else {
                break;
            };
            let request = WakeRecoveryRequest {
                instance: self.instance.clone(),
                seat: candidate.seat.clone(),
                attempt: candidate.attempt.clone(),
                prior_daemon_boot: candidate.prior_daemon_boot,
                elected_boot: self.daemon_boot,
            };
            match self.wakes.store.recover_wake_reservation(request, budget) {
                Ok(WakeRecoveryOutcome::Recovered) => outcome.recovered += 1,
                Ok(WakeRecoveryOutcome::AlreadySettled | WakeRecoveryOutcome::Stale) => {}
                Err(err) => {
                    scan.pending.push_front(candidate);
                    return Err(err);
                }
            }
        }
        if scan.pending.is_empty() && scan.cursor.is_none() {
            scan.complete = true;
        }
        Ok(scan.complete)
    }
}

/// A caller may run different seats concurrently. The mutex protects only
/// admission, guard updates and completion claims; it is released before host
/// I/O and durable settlement waits.
pub struct WakeRunner<'a, S: WakePort + ?Sized, N: NotificationPort + ?Sized> {
    store: &'a S,
    notifier: &'a N,
    state: Mutex<WakeRunnerState>,
    /// Evidence-backed poke capabilities per harness (default: none declared).
    caps: &'a dyn PokeCapabilitySource,
}

#[derive(Clone)]
struct PendingCompletion {
    seat: SeatId,
    attempt: WakeAttemptId,
    outcome: WakeOutcome,
    /// Pre-reservation ladder row, restored by a Refused completion. A
    /// scheduled poke never moved the ladder and carries none.
    prior: Option<PriorLadder>,
    claimed: bool,
    /// Set when the accepted prompt carried the poke text: only then does a
    /// `Submitted` settlement mark these receipts.
    receipts: Option<Vec<PokeReceipt>>,
}

/// A poke-carrying attempt: the text and receipts, and whether it is a
/// scheduled poke or rides an ordinary wake.
struct PokeJob<'p> {
    plan: &'p PokePlan,
    mode: PokeMode,
    caps: &'p dyn PokeCapabilitySource,
}

/// The in-memory admission a soft poke needs under the state lock: no pending
/// completion for the seat, room under the active-prompt limit, and the
/// dispatcher's spec §10 poke limits (never the wake retry backoff).
fn poke_admissible(state: &WakeRunnerState, seat: &SeatId, now: MonoInstant) -> bool {
    !state
        .pending
        .iter()
        .any(|completion| &completion.seat == seat)
        && state.pending.len() + state.dispatch.active_count() < MAX_ACTIVE_PROMPTS
        && state.dispatch.can_reserve_poke(seat, now)
}

struct WakeRunnerState {
    dispatch: DispatchState,
    // Joined attempts retain a bounded ownership slot until durable settlement.
    pending: VecDeque<PendingCompletion>,
}

impl<'a, S: WakePort + ?Sized, N: NotificationPort + ?Sized> WakeRunner<'a, S, N> {
    pub fn new(
        store: &'a S,
        notifier: &'a N,
        config: RetryConfig,
        daemon_boot: uuid::Uuid,
    ) -> Self {
        let boot_mono = store.clock().monotonic_now();
        Self {
            store,
            notifier,
            caps: &NoPokeCapabilities,
            state: Mutex::new(WakeRunnerState {
                dispatch: DispatchState::new(config, boot_mono, daemon_boot),
                pending: VecDeque::new(),
            }),
        }
    }

    /// Candidate discovery is advisory. `reserve_wake` rechecks it atomically,
    /// then the notification adapter must use only committed reservation identity.
    pub fn try_candidate(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeOutcome>, ApiError> {
        if budget.is_exhausted(self.store.clock()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "wake admission deadline exhausted",
            ));
        }
        if AttentionSnapshot::from_candidate(candidate)
            .select()
            .is_none()
            || candidate.attention_witness.is_none()
            || !candidate.continuity_resolved
            || candidate.target.is_none()
        {
            // No actionable work: a refusal backoff has nothing left to retry.
            if let Ok(mut state) = self.state.lock() {
                state.dispatch.clear_refusal(&candidate.seat);
            }
            return Ok(None);
        }
        let seat = candidate.seat.clone();
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        if state
            .pending
            .iter()
            .any(|completion| completion.seat == seat)
            || state.pending.len() + state.dispatch.active_count() >= MAX_ACTIVE_PROMPTS
        {
            return Ok(None);
        }
        let retry_step = u8::try_from(candidate.retry_step)
            .map_err(|_| error(ErrorCode::StoreCorrupt, "invalid durable retry step"))?;
        let was_known = state.dispatch.has_seat(&seat);
        let restored = state.dispatch.restore(
            seat.clone(),
            DurableRetry {
                retry_step,
                minimum_delay_ms: candidate.minimum_delay_ms,
                effective_delay_ms: candidate.effective_delay_ms,
                ever_reserved: candidate.last_reservation_id.is_some(),
            },
        );
        if restored.is_ok() || was_known {
            state
                .dispatch
                .observe_frontier(
                    &seat,
                    candidate.last_reserved_frontier,
                    candidate
                        .attention_witness
                        .as_ref()
                        .expect("checked")
                        .frontier(),
                )
                .map_err(dispatch_error)?;
        }
        if let Err(cause) = restored {
            if was_known {
                return Ok(None);
            }
            return Err(dispatch_error(cause));
        }
        if !state
            .dispatch
            .can_reserve(&seat, self.store.clock().monotonic_now())
        {
            return Ok(None);
        }
        let Some(reservation) = self.store.reserve_wake(candidate, budget)? else {
            return Ok(None);
        };
        // A crash here leaves only the committed reservation behind.
        failpoint!(
            "wake.after_reservation",
            reservation.daemon_boot.to_string()
        );
        let prior = PriorLadder::from_candidate(candidate);
        let committed_at = self.store.clock().monotonic_now();
        let local_reservation = state.dispatch.reserved(
            seat.clone(),
            reservation.attempt.clone(),
            reservation.daemon_boot,
            committed_at,
        );
        drop(state);
        if let Err(cause) = local_reservation {
            self.store.complete_wake(
                reservation.attempt,
                WakeOutcome::OutcomeUnknown,
                None,
                &self.completion_budget(),
            )?;
            return Err(dispatch_error(cause));
        }
        // Spec §10: when a poke is also due for this seat, one prompt goes out
        // with the poke text. The poke is advisory: no read error blocks the
        // wake, and an ineligible poke state falls back to the ordinary marker.
        let due_poke = self
            .store
            .poke_for_wake(&seat, budget)
            .ok()
            .flatten()
            .filter(|due| !due.receipts.is_empty());
        let plan = due_poke
            .as_ref()
            .map(|due| poke_plan(&due.receipts, reservation.reserved_at_utc.0));
        let job = plan.as_ref().map(|plan| PokeJob {
            plan,
            mode: PokeMode::WithWake,
            caps: self.caps,
        });
        let attempt = self.run_attempt(&reservation, job.as_ref(), budget, committed_at);
        let outcome = attempt.outcome;
        let receipts = attempt
            .poked
            .then(|| plan.map(|plan| plan.receipts))
            .flatten();
        // A crash here follows the prompt but precedes its attempt record.
        failpoint!("wake.after_prompt", reservation.daemon_boot.to_string());
        let completed_at = self.store.clock().monotonic_now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        let current = state
            .dispatch
            .finish(
                &seat,
                &reservation.attempt,
                &reservation.daemon_boot,
                completed_at,
            )
            .map_err(dispatch_error)?;
        if current {
            state.dispatch.record_outcome(&seat, outcome, completed_at);
            let completion = PendingCompletion {
                seat,
                attempt: reservation.attempt.clone(),
                outcome,
                prior: Some(prior),
                claimed: true,
                receipts,
            };
            state.pending.push_back(completion.clone());
            drop(state);
            self.settle_claimed(completion, &self.completion_budget())?;
        }
        Ok(Some(outcome))
    }

    /// Runs one committed reservation's host attempt inside its lease and maps
    /// a host error to the outcome it records.
    fn run_attempt(
        &self,
        reservation: &WakeReservation,
        job: Option<&PokeJob<'_>>,
        budget: &CallBudget,
        committed_at: MonoInstant,
    ) -> PokeAttempt {
        let simple = |outcome| PokeAttempt {
            outcome,
            poked: false,
            diagnostic: None,
        };
        let lease_end = committed_at
            .0
            .saturating_add(MAX_ATTEMPT_MILLIS)
            .min(budget.deadline.0)
            .min(reservation.lease_until.0);
        if budget.cancellation.is_cancelled() {
            return simple(WakeOutcome::Cancelled);
        }
        if lease_end <= committed_at.0 {
            return simple(WakeOutcome::TimedOut);
        }
        let owned_cancellation = Cancellation::default();
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(lease_end),
                cancellation: owned_cancellation.clone(),
            },
            expected_boot: Some(reservation.host_boot.clone()),
            expected_epoch: Some(reservation.host_epoch),
        };
        match self.run_owned_attempt(
            reservation.clone(),
            job,
            context,
            budget,
            lease_end,
            owned_cancellation,
        ) {
            Ok(attempt) => attempt,
            Err(err) => simple(match err.code {
                ErrorCode::Cancelled => WakeOutcome::Cancelled,
                ErrorCode::DeadlineExceeded => WakeOutcome::TimedOut,
                ErrorCode::TargetUnsafe | ErrorCode::TargetUnresolved => WakeOutcome::Unsafe,
                ErrorCode::HostUnavailable | ErrorCode::UnsupportedHarness => {
                    WakeOutcome::Unavailable
                }
                _ => WakeOutcome::OutcomeUnknown,
            }),
        }
    }

    /// One seat's soft poke through the wake limits: the seat's single
    /// reservation slot (`reserve_poke`), the in-memory four-active and
    /// per-seat limits, then the same owned attempt as a wake. A skipped
    /// attempt (focused, unsafe state, undeclared capability) completes
    /// without marking anything and does not advance the wake retry guard; the
    /// seat is re-evaluated after the wake retry spacing
    /// (`DispatchState::can_reserve_poke`), not on the next tick.
    pub fn try_poke(
        &self,
        due: &PokeDue,
        budget: &CallBudget,
    ) -> Result<Option<WakeOutcome>, ApiError> {
        if budget.is_exhausted(self.store.clock()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "poke admission deadline exhausted",
            ));
        }
        if due.receipts.is_empty() {
            return Ok(None);
        }
        let seat = due.seat.clone();
        // Like a wake, admission and the durable reservation share one hold of
        // the state lock, so concurrent seats cannot all pass the active limit
        // and then collide after their reservations commit.
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        if !poke_admissible(&state, &seat, self.store.clock().monotonic_now()) {
            return Ok(None);
        }
        let Some(reserved) = self.store.reserve_poke(due, budget)? else {
            state
                .dispatch
                .poke_unreserved(&seat, self.store.clock().monotonic_now());
            return Ok(None);
        };
        let PokeReservation {
            reservation,
            receipts,
        } = reserved;
        failpoint!(
            "wake.after_reservation",
            reservation.daemon_boot.to_string()
        );
        let committed_at = self.store.clock().monotonic_now();
        let local = state.dispatch.poke_reserved(
            seat.clone(),
            reservation.attempt.clone(),
            reservation.daemon_boot,
        );
        drop(state);
        if let Err(cause) = local {
            self.store.complete_poke(
                reservation.attempt,
                WakeOutcome::OutcomeUnknown,
                &[],
                &self.completion_budget(),
            )?;
            return Err(dispatch_error(cause));
        }
        let plan = poke_plan(&receipts, reservation.reserved_at_utc.0);
        let job = PokeJob {
            plan: &plan,
            mode: PokeMode::PokeOnly,
            caps: self.caps,
        };
        let attempt = self.run_attempt(&reservation, Some(&job), budget, committed_at);
        let outcome = attempt.outcome;
        failpoint!("wake.after_prompt", reservation.daemon_boot.to_string());
        let completed_at = self.store.clock().monotonic_now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        let current = state.dispatch.poke_finished(
            &seat,
            &reservation.attempt,
            &reservation.daemon_boot,
            outcome == WakeOutcome::Submitted && attempt.poked,
            completed_at,
        );
        if current {
            let completion = PendingCompletion {
                seat,
                attempt: reservation.attempt.clone(),
                outcome,
                prior: None,
                claimed: true,
                receipts: attempt.poked.then_some(receipts),
            };
            state.pending.push_back(completion.clone());
            drop(state);
            self.settle_claimed(completion, &self.completion_budget())?;
        }
        Ok(Some(outcome))
    }

    fn has_pending_completions(&self) -> Result<bool, ApiError> {
        Ok(!self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?
            .pending
            .is_empty())
    }

    /// Retry each retained joined attempt at most once per drive. Failed records
    /// rotate behind their peers, and admission continues despite their errors.
    fn retry_completions(&self, budget: &CallBudget) -> Result<Option<ApiError>, ApiError> {
        let attempts: Vec<_> = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?
            .pending
            .iter()
            .filter(|completion| !completion.claimed)
            .map(|completion| completion.attempt.clone())
            .collect();
        let mut first_error = None;
        for attempt in attempts {
            if budget.is_exhausted(self.store.clock()) {
                break;
            }
            let completion = {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
                state
                    .pending
                    .iter_mut()
                    .find(|completion| completion.attempt == attempt && !completion.claimed)
                    .map(|completion| {
                        completion.claimed = true;
                        completion.clone()
                    })
            };
            let Some(completion) = completion else {
                continue;
            };
            let retry_budget = CallBudget {
                deadline: MonoInstant(self.completion_budget().deadline.0.min(budget.deadline.0)),
                cancellation: budget.cancellation.clone(),
            };
            if let Err(err) = self.settle_claimed(completion, &retry_budget)
                && first_error.is_none()
            {
                first_error = Some(err);
            }
        }
        Ok(first_error)
    }

    /// The retained claimed record remains counted and blocks its seat while
    /// FairWriter or SQLite waits. Only its exact attempt may acknowledge it.
    fn settle_claimed(
        &self,
        completion: PendingCompletion,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let refused_restore = match completion.outcome {
            WakeOutcome::Refused(_) => completion.prior.as_ref(),
            _ => None,
        };
        let result = match &completion.receipts {
            // A poke never advanced the ladder: nothing to restore.
            Some(receipts) => self
                .store
                .complete_poke(
                    completion.attempt.clone(),
                    completion.outcome,
                    receipts,
                    budget,
                )
                .map(|()| false),
            None => self.store.complete_wake(
                completion.attempt.clone(),
                completion.outcome,
                refused_restore,
                budget,
            ),
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        let index = state
            .pending
            .iter()
            .position(|pending| pending.attempt == completion.attempt && pending.claimed)
            .ok_or_else(|| error(ErrorCode::StoreCorrupt, "wake completion claim missing"))?;
        let mut retained = state
            .pending
            .remove(index)
            .expect("located retained completion");
        match result {
            Ok(matched) => {
                // Fence miss (matched == false): the durable step stays
                // advanced, so the in-memory guard stays advanced too.
                if matched {
                    state.dispatch.restore_prior_guard(&retained.seat);
                }
                Ok(())
            }
            Err(err) => {
                retained.claimed = false;
                state.pending.push_back(retained);
                Err(err)
            }
        }
    }

    /// Drops the refusal backoff of every seat in `listed`'s complement.
    fn clear_refusals_not_in(
        &self,
        listed: &std::collections::HashSet<SeatId>,
    ) -> Result<(), ApiError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?;
        for seat in state.dispatch.refusing_seats() {
            if !listed.contains(&seat) {
                state.dispatch.clear_refusal(&seat);
            }
        }
        Ok(())
    }

    fn next_due_at(&self) -> Result<Option<MonoInstant>, ApiError> {
        let now = self.store.clock().monotonic_now();
        Ok(self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake state lock poisoned"))?
            .dispatch
            .next_due_at(now))
    }

    /// The supervisor can cancel a connected call at the lease deadline while
    /// retaining its physical slot until the owned worker has actually exited.
    /// A non-cooperative adapter keeps this scope and slot occupied; it cannot
    /// silently become an unbounded detached replacement worker.
    fn run_owned_attempt(
        &self,
        reservation: WakeReservation,
        job: Option<&PokeJob<'_>>,
        context: HostCallContext,
        caller_budget: &CallBudget,
        lease_end: u64,
        cancellation: Cancellation,
    ) -> Result<PokeAttempt, ApiError> {
        let unknown = || PokeAttempt {
            outcome: WakeOutcome::OutcomeUnknown,
            poked: false,
            diagnostic: None,
        };
        std::thread::scope(|scope| {
            let (sender, receiver) = mpsc::sync_channel(1);
            let notifier = self.notifier;
            let worker = scope.spawn(move || {
                let result = match job {
                    None => {
                        notifier
                            .attempt_wake(reservation, &context)
                            .map(|outcome| PokeAttempt {
                                outcome,
                                poked: false,
                                diagnostic: None,
                            })
                    }
                    Some(job) => {
                        notifier.attempt_poke(reservation, job.plan, job.mode, job.caps, &context)
                    }
                };
                let _ = sender.send(result);
            });
            let mut expired = false;
            let result = loop {
                match receiver.recv_timeout(Duration::from_millis(5)) {
                    Ok(result) => break Some(result),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if caller_budget.cancellation.is_cancelled()
                            || self.store.clock().monotonic_now().0 >= lease_end
                        {
                            expired = true;
                            cancellation.cancel();
                            break receiver.recv().ok();
                        }
                    }
                }
            };
            let joined = worker.join().is_ok();
            if !joined || expired || self.store.clock().monotonic_now().0 >= lease_end {
                return Ok(unknown());
            }
            result.unwrap_or_else(|| Ok(unknown()))
        })
    }

    fn completion_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(
                self.store
                    .clock()
                    .monotonic_now()
                    .0
                    .saturating_add(COMPLETION_BUDGET_MILLIS),
            ),
            cancellation: Cancellation::default(),
        }
    }
}

/// The coalesced prompt for `receipts` (ordered by effective deadline): the
/// thread ids once each, and the smallest remaining effective time.
fn poke_plan(receipts: &[PokeReceipt], now_utc: i64) -> PokePlan {
    let mut threads: Vec<ThreadId> = Vec::new();
    for receipt in receipts {
        if !threads.contains(&receipt.thread) {
            threads.push(receipt.thread.clone());
        }
    }
    let soonest = receipts
        .iter()
        .map(|receipt| receipt.effective_deadline)
        .min()
        .unwrap_or(now_utc);
    PokePlan {
        text: poke_text(soonest.saturating_sub(now_utc), &threads),
        receipts: receipts.to_vec(),
    }
}

fn error(code: ErrorCode, detail: &'static str) -> ApiError {
    ApiError::new(code, detail)
}
fn dispatch_error(err: crate::notification::dispatch::DispatchError) -> ApiError {
    error(
        ErrorCode::StoreCorrupt,
        match err {
            crate::notification::dispatch::DispatchError::Retry(_) => {
                "invalid process wake retry state"
            }
            crate::notification::dispatch::DispatchError::UnknownSeat => {
                "wake seat was not restored"
            }
            crate::notification::dispatch::DispatchError::ActiveLimit => {
                "wake active limit exceeded after reservation"
            }
            crate::notification::dispatch::DispatchError::WrongBoot => {
                "wake reservation has wrong daemon boot"
            }
        },
    )
}

#[cfg(test)]
#[path = "../../tests/scheduler/poke_flow.rs"]
mod poke_flow;
#[cfg(test)]
#[path = "../../tests/scheduler/dispatch.rs"]
mod tests;
