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
        policy::{AttentionSnapshot, DurableRetry, RetryConfig},
    },
    ports::{
        HostCallContext, NotificationPort, StorePort, WakeCandidate, WakeOutcome,
        WakeRecoveryCandidate, WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation,
    },
    protocol::{
        ids::{SeatId, WakeAttemptId},
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

/// The narrow store boundary keeps fake-port tests on the same reservation and
/// completion calls used by the production StorePort.
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
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
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
}
impl<T: StorePort + ?Sized> WakePort for T {
    fn clock(&self) -> &dyn Clock {
        StorePort::clock(self)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        StorePort::wake_candidates(self, page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        StorePort::reserve_wake(self, candidate, budget)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        StorePort::complete_wake(self, attempt, outcome, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        StorePort::wake_recovery_candidates(self, page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        StorePort::recover_wake_reservation(self, request, budget)
    }
}

#[derive(Default)]
struct WakeScanState {
    cursor: Option<String>,
    pending: VecDeque<WakeCandidate>,
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
            match self.wakes.try_candidate(&candidate, budget) {
                Ok(Some(_)) => outcome.attempted += 1,
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
        let scan = self
            .scan
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "wake scan lock poisoned"))?;
        outcome.has_more = !scan.pending.is_empty()
            || scan.cursor.is_some()
            || self.wakes.has_pending_completions()?;
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
}

#[derive(Clone)]
struct PendingCompletion {
    seat: SeatId,
    attempt: WakeAttemptId,
    outcome: WakeOutcome,
    claimed: bool,
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
                &self.completion_budget(),
            )?;
            return Err(dispatch_error(cause));
        }
        let lease_end = committed_at
            .0
            .saturating_add(MAX_ATTEMPT_MILLIS)
            .min(budget.deadline.0)
            .min(reservation.lease_until.0);
        let outcome = if budget.cancellation.is_cancelled() {
            WakeOutcome::Cancelled
        } else if lease_end <= committed_at.0 {
            WakeOutcome::TimedOut
        } else {
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
                context,
                budget,
                lease_end,
                owned_cancellation,
            ) {
                Ok(outcome) => outcome,
                Err(err) => match err.code {
                    ErrorCode::Cancelled => WakeOutcome::Cancelled,
                    ErrorCode::DeadlineExceeded => WakeOutcome::TimedOut,
                    ErrorCode::TargetUnsafe | ErrorCode::TargetUnresolved => WakeOutcome::Unsafe,
                    ErrorCode::HostUnavailable | ErrorCode::UnsupportedHarness => {
                        WakeOutcome::Unavailable
                    }
                    _ => WakeOutcome::OutcomeUnknown,
                },
            }
        };
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
            let completion = PendingCompletion {
                seat,
                attempt: reservation.attempt.clone(),
                outcome,
                claimed: true,
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
        let result =
            self.store
                .complete_wake(completion.attempt.clone(), completion.outcome, budget);
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
        if result.is_err() {
            retained.claimed = false;
            state.pending.push_back(retained);
        }
        result
    }

    /// The supervisor can cancel a connected call at the lease deadline while
    /// retaining its physical slot until the owned worker has actually exited.
    /// A non-cooperative adapter keeps this scope and slot occupied; it cannot
    /// silently become an unbounded detached replacement worker.
    fn run_owned_attempt(
        &self,
        reservation: WakeReservation,
        context: HostCallContext,
        caller_budget: &CallBudget,
        lease_end: u64,
        cancellation: Cancellation,
    ) -> Result<WakeOutcome, ApiError> {
        std::thread::scope(|scope| {
            let (sender, receiver) = mpsc::sync_channel(1);
            let notifier = self.notifier;
            let worker = scope.spawn(move || {
                let _ = sender.send(notifier.attempt_wake(reservation, &context));
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
                return Ok(WakeOutcome::OutcomeUnknown);
            }
            result.unwrap_or(Ok(WakeOutcome::OutcomeUnknown))
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

fn error(code: ErrorCode, detail: &'static str) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
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
#[path = "../../tests/scheduler/dispatch.rs"]
mod tests;
