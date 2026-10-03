//! Bounded scheduling of store-owned due and retirement decisions.

use std::collections::VecDeque;

use crate::ports::{
    DuePhase, DuePhaseProgress, DueScanProgress, DueScanRequest, DueScanState,
    DurableWorkAdmission, RetirementProgress, WorkAdmission, WorkCandidate, WorkProgress,
};
use crate::protocol::{
    ids::RetirementJobId,
    pagination::{Page, PageRequest},
    results::{ApiError, ErrorCode, RetirementStatus},
    time::{CallBudget, Clock, MonoInstant},
};

pub const DUE_BATCH_LIMIT: u16 = 100;
/// The deadline lane's safety tick: a pass with nothing kicking it still runs
/// this often, so time-driven expiries fire at most this late. Commits that
/// create work kick the lane at once (`after_committed_change`).
pub const TICK_MILLIS: u64 = 5_000;
const RETIREMENT_RETRY_MILLIS: u64 = 5_000;
const DUE_PHASE_RETRY_MILLIS: u64 = 5_000;
// Retry bookkeeping is bounded independently of the durable work queue.
// Eviction can shorten a failed job's delay; discovery still advances.
const MAX_WORK_RETRY_ENTRIES: usize = 64;

/// Narrow view of the store used by the deadline driver. Production implements
/// it with `ScheduledStore`; tests with explicit fakes or a `ScheduledStore`
/// over a real `SqliteStore`.
pub trait DeadlinePort {
    fn clock(&self) -> &dyn Clock;
    fn due_obligations(
        &self,
        request: DueScanRequest,
        budget: &CallBudget,
    ) -> Result<DueScanProgress, ApiError>;
    /// Enumerate every retirement under a frozen high water, including terminal
    /// records. A complete fresh sweep may establish recovery coverage only
    /// when every returned job is affirmatively completed without error.
    fn pending_retirement_jobs(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError>;
    fn advance_retirement(
        &self,
        job: RetirementJobId,
        admission: WorkAdmission,
        budget: &CallBudget,
    ) -> Result<RetirementProgress, ApiError>;
    /// Visit every ordinal under a frozen high water and return all pending or
    /// failed jobs. Failures remain enumerable until successful completion;
    /// completion cannot accompany last_error. These properties are required
    /// for bounded recovery coverage, including for qualified test doubles.
    fn pending_work(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WorkCandidate>, ApiError>;
    fn advance_work(
        &self,
        job: &str,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<WorkProgress, ApiError>;
}
/// Affirmative discovery coverage, independent of job admission/progress.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DiscoveryProgress {
    pub fresh_start: bool,
    pub reached_end: bool,
    pub returned_job: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DriveOutcome {
    pub due_examined_candidates: u16,
    pub due_warnings_added: u16,
    pub due_continuation: bool,
    pub due_progress: Option<DueScanProgress>,
    pub retirement_discovery: Option<DiscoveryProgress>,
    pub retirement_job: Option<RetirementJobId>,
    pub retirement_complete: bool,
    /// The retirement quantum committed at least one unit and retained no
    /// error. Committed progress clears the job's failure state.
    pub retirement_progressed: bool,
    pub retirement_error: Option<ApiError>,
    pub work_discovery: Option<DiscoveryProgress>,
    pub work_complete: bool,
    /// The work quantum committed at least one unit and retained no error.
    /// Committed progress clears the job's failure state.
    pub work_progressed: bool,
    pub work_job: Option<String>,
    pub work_error: Option<ApiError>,
    /// This call was a scheduler tick (it passed the tick gate), not a skip.
    pub ticked: bool,
}

impl DriveOutcome {
    /// A retirement or work job committed progress and reported more pending,
    /// so the lane re-runs at once instead of waiting for its safety tick.
    pub fn progressed_with_more(&self) -> bool {
        (self.retirement_progressed && !self.retirement_complete)
            || (self.work_progressed && !self.work_complete)
    }
}

/// Holds bounded phase state, discovery cursors, and retry guards. Each call
/// admits at most one 100-candidate due scan and one quantum per background class.
pub struct DeadlineDriver<'a, P: DeadlinePort + ?Sized> {
    port: &'a P,
    next_tick: MonoInstant,
    tick_millis: u64,
    due_continuation: bool,
    due_state: DueScanState,
    invitation_retry_at: MonoInstant,
    receipt_retry_at: MonoInstant,
    retirement_cursor: Option<String>,
    retirement_retry_at: MonoInstant,
    work_cursor: Option<String>,
    work_discovery_retry_at: MonoInstant,
    work_retry_at: VecDeque<(String, MonoInstant)>,
    first_class: u8,
}
impl<'a, P: DeadlinePort + ?Sized> DeadlineDriver<'a, P> {
    pub fn new(port: &'a P) -> Self {
        Self {
            port,
            next_tick: MonoInstant(0),
            tick_millis: TICK_MILLIS,
            due_continuation: false,
            due_state: DueScanState::default(),
            invitation_retry_at: MonoInstant(0),
            receipt_retry_at: MonoInstant(0),
            retirement_cursor: None,
            retirement_retry_at: MonoInstant(0),
            work_cursor: None,
            work_discovery_retry_at: MonoInstant(0),
            work_retry_at: VecDeque::new(),
            first_class: 0,
        }
    }

    /// Overrides the tick gate's length (default [`TICK_MILLIS`]); lets a test
    /// exercise the job-local retry gates at a finer tick than the safety tick.
    pub fn with_tick_millis(mut self, millis: u64) -> Self {
        self.tick_millis = millis;
        self
    }

    /// Call on boot, after a committed change or fence, and on the monotonic safety tick.
    /// The store samples its own UTC decision time inside each write transaction.
    pub fn drive(&mut self, budget: &CallBudget) -> Result<DriveOutcome, ApiError> {
        self.drive_with_partial(budget, |_, _| {})
    }

    /// Report completed callbacks before a later due error discards the turn's
    /// accumulated outcome. The borrowed prefix is delivered at most once;
    /// the terminal Result and all scheduling state retain drive semantics.
    pub fn drive_with_partial(
        &mut self,
        budget: &CallBudget,
        on_partial: impl FnOnce(&DriveOutcome, &ApiError),
    ) -> Result<DriveOutcome, ApiError> {
        let now = self.port.clock().monotonic_now();
        if budget.is_exhausted(self.port.clock()) {
            return Err(ApiError::deadline_exceeded(
                "deadline driver budget exhausted",
            ));
        }
        if now < self.next_tick && !self.due_continuation {
            return Ok(DriveOutcome::default());
        }
        self.next_tick = MonoInstant(now.0.saturating_add(self.tick_millis));
        let mut outcome = DriveOutcome {
            ticked: true,
            ..DriveOutcome::default()
        };
        let first_class = self.first_class;
        self.first_class = (first_class + 1) % 3;
        for offset in 0..3 {
            match (first_class + offset) % 3 {
                0 => self.drive_retirement(budget, &mut outcome),
                1 => {
                    if let Err(error) = self.drive_due(now, budget, &mut outcome) {
                        on_partial(&outcome, &error);
                        return Err(error);
                    }
                }
                2 => self.drive_work(budget, &mut outcome),
                _ => unreachable!(),
            }
        }
        Ok(outcome)
    }

    fn drive_due(
        &mut self,
        now: MonoInstant,
        budget: &CallBudget,
        outcome: &mut DriveOutcome,
    ) -> Result<(), ApiError> {
        let run_invitations = now >= self.invitation_retry_at;
        let run_receipts = now >= self.receipt_retry_at;
        if run_invitations || run_receipts {
            let request = DueScanRequest {
                state: self.due_state.clone(),
                max_candidates: DUE_BATCH_LIMIT,
                run_invitations,
                run_receipts,
            };
            self.due_state.next_phase = match self.due_state.next_phase {
                DuePhase::Invitations => DuePhase::Receipts,
                DuePhase::Receipts => DuePhase::Invitations,
            };
            let progress = self.port.due_obligations(request, budget)?;
            self.due_state = progress.state.clone();
            if matches!(progress.invitations, DuePhaseProgress::Failed(..)) {
                self.invitation_retry_at =
                    MonoInstant(now.0.saturating_add(DUE_PHASE_RETRY_MILLIS));
            }
            if matches!(progress.receipts, DuePhaseProgress::Failed(..)) {
                self.receipt_retry_at = MonoInstant(now.0.saturating_add(DUE_PHASE_RETRY_MILLIS));
            }
            outcome.due_examined_candidates = progress.examined_candidates;
            outcome.due_warnings_added = progress.warnings_added;
            self.due_continuation = progress.has_more
                && (matches!(progress.invitations, DuePhaseProgress::More)
                    || matches!(progress.receipts, DuePhaseProgress::More));
            outcome.due_continuation = self.due_continuation;
            outcome.due_progress = Some(progress);
        } else {
            self.due_continuation = false;
        }
        Ok(())
    }

    /// The port's clock (Health's scheduler tick time).
    pub fn clock(&self) -> &dyn Clock {
        self.port.clock()
    }

    /// Invoke only after the store reports a successful commit.
    pub fn after_committed_change(&mut self) {
        self.next_tick = MonoInstant(0);
    }

    pub fn after_committed_fence(&mut self) {
        self.after_committed_change();
    }

    fn drive_retirement(&mut self, budget: &CallBudget, outcome: &mut DriveOutcome) {
        if self.port.clock().monotonic_now() < self.retirement_retry_at {
            return;
        }
        let fresh_start = self.retirement_cursor.is_none();
        let page = match self.port.pending_retirement_jobs(
            PageRequest {
                cursor: self.retirement_cursor.clone(),
                limit: 1,
                max_bytes: 16_384,
            },
            budget,
        ) {
            Ok(page) => page,
            Err(error) => {
                if error.code == ErrorCode::CursorStale {
                    self.retirement_cursor = None;
                }
                self.record_retirement_error(error, outcome);
                return;
            }
        };
        // The next discovery continues from the captured high-water page. On the
        // final page we wrap to the beginning, including jobs added meanwhile.
        outcome.retirement_discovery = Some(DiscoveryProgress {
            fresh_start,
            reached_end: !page.has_more,
            returned_job: !page.items.is_empty(),
        });
        self.retirement_cursor = page.next_cursor;
        let Some(job) = page.items.into_iter().next() else {
            return;
        };
        outcome.retirement_job = Some(job.job.clone());
        match self
            .port
            .advance_retirement(job.job.clone(), WorkAdmission::Background, budget)
        {
            Ok(progress) => {
                // A retained quantum failure is durable; back off and surface it
                // like any other retirement error. Completion never carries one.
                outcome.retirement_complete = progress.complete && progress.last_error.is_none();
                outcome.retirement_progressed =
                    progress.processed_this_turn > 0 && progress.last_error.is_none();
                if let Some(detail) = progress.last_error {
                    self.record_retirement_error(
                        ApiError::store_busy(detail.as_str().to_owned()),
                        outcome,
                    );
                }
            }
            Err(error) => self.record_retirement_error(error, outcome),
        }
    }

    fn record_retirement_error(&mut self, mut error: ApiError, outcome: &mut DriveOutcome) {
        self.retirement_retry_at = MonoInstant(
            self.port
                .clock()
                .monotonic_now()
                .0
                .saturating_add(RETIREMENT_RETRY_MILLIS),
        );
        error.detail.truncate(error.detail.floor_char_boundary(512));
        error.restart_argv = None;
        outcome.retirement_error = Some(error);
    }

    fn drive_work(&mut self, budget: &CallBudget, outcome: &mut DriveOutcome) {
        if self.port.clock().monotonic_now() < self.work_discovery_retry_at {
            return;
        }
        let fresh_start = self.work_cursor.is_none();
        let page = match self.port.pending_work(
            PageRequest {
                cursor: self.work_cursor.clone(),
                limit: 1,
                max_bytes: 16_384,
            },
            budget,
        ) {
            Ok(page) => page,
            Err(error) => {
                if error.code == ErrorCode::CursorStale {
                    self.work_cursor = None;
                }
                self.work_discovery_retry_at = MonoInstant(
                    self.port
                        .clock()
                        .monotonic_now()
                        .0
                        .saturating_add(RETIREMENT_RETRY_MILLIS),
                );
                self.record_work_error(error, outcome);
                return;
            }
        };
        outcome.work_discovery = Some(DiscoveryProgress {
            fresh_start,
            reached_end: !page.has_more,
            returned_job: !page.items.is_empty(),
        });
        self.work_cursor = page.next_cursor;
        let Some(job) = page.items.into_iter().next() else {
            return;
        };
        if let Some(position) = self.work_retry_at.iter().position(|(id, _)| id == &job.id) {
            if self.port.clock().monotonic_now() < self.work_retry_at[position].1 {
                return;
            }
            self.work_retry_at.remove(position);
        }
        outcome.work_job = Some(job.id.clone());
        let admission = DurableWorkAdmission::new(16).expect("fixed maximum");
        match self.port.advance_work(&job.id, admission, budget) {
            Ok(progress) => {
                outcome.work_complete = !progress.has_more && progress.last_error.is_none();
                outcome.work_progressed =
                    progress.processed_this_turn > 0 && progress.last_error.is_none();
                if let Some(detail) = progress.last_error {
                    self.record_failed_work_job(job.id, ApiError::store_busy(detail), outcome);
                }
            }
            Err(error) => self.record_failed_work_job(job.id, error, outcome),
        }
    }

    fn record_failed_work_job(&mut self, job: String, error: ApiError, outcome: &mut DriveOutcome) {
        if self.work_retry_at.len() == MAX_WORK_RETRY_ENTRIES {
            self.work_retry_at.pop_front();
        }
        self.work_retry_at.push_back((
            job,
            MonoInstant(
                self.port
                    .clock()
                    .monotonic_now()
                    .0
                    .saturating_add(RETIREMENT_RETRY_MILLIS),
            ),
        ));
        self.record_work_error(error, outcome);
    }

    fn record_work_error(&mut self, mut error: ApiError, outcome: &mut DriveOutcome) {
        error.detail.truncate(error.detail.floor_char_boundary(512));
        error.restart_argv = None;
        outcome.work_error = Some(error);
    }
}

#[cfg(test)]
#[path = "../../tests/scheduler/deadlines.rs"]
mod tests;
