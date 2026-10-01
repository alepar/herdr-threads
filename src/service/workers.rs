//! Bounded admission for work that must not occupy the domain writer.

use crate::ports::SnapshotGenerationId;
use crate::ports::{
    GuardedInvalidationTransition, GuardedSeatTransition, HostInvalidationFence,
    HostInvalidationReason, HostObservation, HostObservationAdmission, InvalidationSeatPage,
    PublishedSnapshot, ReconciliationOutcome, SnapshotCleanupProgress, SnapshotHeader,
    SnapshotSeatPage, SnapshotStage, SnapshotStageProgress,
};
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use crate::{
    notification::{dispatch::NativeWakeDispatcher, policy::RetryConfig},
    ports::{
        DuePhaseProgress, DueScanProgress, DueScanRequest, DurableWorkAdmission, HostPort,
        RetirementProgress, StorePort, WakeCandidate, WakeOutcome, WakeRecoveryCandidate,
        WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation, WorkAdmission, WorkCandidate,
        WorkProgress,
    },
    protocol::{
        ids::RetirementJobId,
        pagination::{Page, PageRequest},
        results::RetirementStatus,
        time::Cancellation,
    },
    scheduler::{
        Scheduler, WakePort,
        deadlines::{DeadlineDriver, DeadlinePort, DriveOutcome},
    },
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[derive(Debug, Default)]
struct LaneState {
    active: usize,
    queued: usize,
}

/// A lane admits a fixed number of running and waiting calls. A full lane
/// fails promptly, leaving unrelated service lanes available.
#[derive(Debug)]
pub struct BoundedLane {
    active_limit: usize,
    queued_limit: usize,
    state: Mutex<LaneState>,
    changed: Condvar,
}

#[derive(Debug)]
pub struct LaneGuard<'a>(&'a BoundedLane);

impl BoundedLane {
    pub fn new(active_limit: usize, queued_limit: usize) -> Self {
        assert!(active_limit > 0);
        Self {
            active_limit,
            queued_limit,
            state: Mutex::new(LaneState::default()),
            changed: Condvar::new(),
        }
    }

    pub fn queued(&self) -> usize {
        self.state.lock().expect("lane lock poisoned").queued
    }

    pub fn enter(&self, budget: &CallBudget, clock: &dyn Clock) -> Result<LaneGuard<'_>, ApiError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "lane lock poisoned"))?;
        if budget.is_exhausted(clock) {
            return Err(budget_error(budget));
        }
        if state.active < self.active_limit && state.queued == 0 {
            state.active += 1;
            return Ok(LaneGuard(self));
        }
        if state.queued >= self.queued_limit {
            return Err(error(ErrorCode::StoreBusy, "lane admission full"));
        }
        state.queued += 1;
        loop {
            if budget.is_exhausted(clock) {
                state.queued -= 1;
                self.changed.notify_all();
                return Err(budget_error(budget));
            }
            if state.active < self.active_limit {
                state.queued -= 1;
                state.active += 1;
                return Ok(LaneGuard(self));
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| error(ErrorCode::StoreCorrupt, "lane lock poisoned"))?
                .0;
        }
    }
}

impl Drop for LaneGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.active -= 1;
            self.0.changed.notify_all();
        }
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

#[derive(Debug, Default)]
struct WriterState {
    active: bool,
    next_ticket: u64,
    foreground: VecDeque<u64>,
    background: VecDeque<u64>,
    foreground_since_background: u8,
}

/// One writer admission at a time. Waiting foreground calls retain FIFO order.
/// A ready background quantum follows at most eight foreground decisions;
/// foreground resumes after that single quantum.
#[derive(Debug)]
pub struct FairWriter {
    queued_limit: usize,
    state: Mutex<WriterState>,
    changed: Condvar,
}

#[derive(Debug, Clone, Copy)]
enum WriterClass {
    Foreground,
    Background,
}

#[derive(Debug)]
pub struct WriterGuard<'a> {
    lane: &'a FairWriter,
    class: WriterClass,
}

impl FairWriter {
    pub fn new(queued_limit: usize) -> Self {
        Self {
            queued_limit,
            state: Mutex::new(WriterState::default()),
            changed: Condvar::new(),
        }
    }

    pub fn waiting(&self) -> (usize, usize) {
        let state = self.state.lock().expect("writer lane lock poisoned");
        (state.foreground.len(), state.background.len())
    }

    pub fn enter_foreground(
        &self,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        self.enter(WriterClass::Foreground, budget, clock)
    }

    pub fn enter_background(
        &self,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        self.enter(WriterClass::Background, budget, clock)
    }

    fn enter(
        &self,
        class: WriterClass,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<WriterGuard<'_>, ApiError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error(ErrorCode::StoreCorrupt, "writer lane lock poisoned"))?;
        if budget.is_exhausted(clock) {
            return Err(budget_error(budget));
        }
        if state.foreground.len() + state.background.len() >= self.queued_limit {
            return Err(error(ErrorCode::StoreBusy, "writer admission full"));
        }
        let ticket = state.next_ticket;
        state.next_ticket = state
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::StoreBusy, "writer admission counter exhausted"))?;
        match class {
            WriterClass::Foreground => state.foreground.push_back(ticket),
            WriterClass::Background => state.background.push_back(ticket),
        }
        loop {
            if budget.is_exhausted(clock) {
                let queue = match class {
                    WriterClass::Foreground => &mut state.foreground,
                    WriterClass::Background => &mut state.background,
                };
                if let Some(position) = queue.iter().position(|queued| *queued == ticket) {
                    queue.remove(position);
                }
                self.changed.notify_all();
                return Err(budget_error(budget));
            }
            let ready = match class {
                WriterClass::Foreground => {
                    state.foreground.front() == Some(&ticket)
                        && (state.background.is_empty() || state.foreground_since_background < 8)
                }
                WriterClass::Background => {
                    state.background.front() == Some(&ticket)
                        && (state.foreground.is_empty() || state.foreground_since_background >= 8)
                }
            };
            if !state.active && ready {
                match class {
                    WriterClass::Foreground => {
                        state.foreground.pop_front();
                    }
                    WriterClass::Background => {
                        state.background.pop_front();
                    }
                }
                state.active = true;
                return Ok(WriterGuard { lane: self, class });
            }
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| error(ErrorCode::StoreCorrupt, "writer lane lock poisoned"))?
                .0;
        }
    }
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lane.state.lock() {
            state.active = false;
            match self.class {
                WriterClass::Foreground => {
                    state.foreground_since_background =
                        state.foreground_since_background.saturating_add(1).min(8);
                }
                WriterClass::Background => state.foreground_since_background = 0,
            }
            self.lane.changed.notify_all();
        }
    }
}

fn budget_error(budget: &CallBudget) -> ApiError {
    if budget.cancellation.is_cancelled() {
        error(ErrorCode::Cancelled, "lane admission cancelled")
    } else {
        error(
            ErrorCode::DeadlineExceeded,
            "lane admission deadline elapsed",
        )
    }
}

struct ScheduledStore {
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
}

impl DeadlinePort for ScheduledStore {
    fn clock(&self) -> &dyn Clock {
        self.store.clock()
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        budget: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.due_obligations(request, budget)
    }
    fn pending_retirement_jobs(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError> {
        self.store.pending_retirement_jobs(page, budget)
    }
    fn advance_retirement(
        &self,
        job: RetirementJobId,
        admission: WorkAdmission,
        budget: &CallBudget,
    ) -> Result<RetirementProgress, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.advance_retirement(job, admission, budget)
    }
    fn pending_work(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WorkCandidate>, ApiError> {
        self.store.pending_work(page, budget)
    }
    fn advance_work(
        &self,
        job: &str,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<WorkProgress, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.advance_work(job, admission, budget)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FailureKey {
    General,
    Invitations,
    Receipts,
    RetirementDiscovery,
    WorkDiscovery,
    Retirement(String),
    Work(String),
    ObservationCapture,
    ObservationReconciliation,
}
impl FailureKey {
    fn job_class(&self) -> Option<usize> {
        match self {
            Self::Retirement(_) => Some(0),
            Self::Work(_) => Some(1),
            _ => None,
        }
    }
}

/// The wake-lane store operation a callback failure belongs to. Health sees
/// only this phase and the error code, never the seat or attempt identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakePhase {
    Admission,
    Discovery,
    Reservation,
    Completion,
    RecoveryDiscovery,
    Recovery,
}
impl WakePhase {
    fn label(self) -> &'static str {
        match self {
            Self::Admission => "wake admission",
            Self::Discovery => "wake discovery",
            Self::Reservation => "wake reservation",
            Self::Completion => "unsettled durable wake completion",
            Self::RecoveryDiscovery => "wake recovery discovery",
            Self::Recovery => "wake recovery",
        }
    }
}

/// A Health-safe worker failure: a fixed class and a typed code or reason,
/// never store, host or callback detail, and never a seat, job or attempt
/// identity. The exact diagnostic stays private: durable on the retirement or
/// work row (exact seat inspect) and in the in-memory diagnostic tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedactedFailure {
    /// The deadline drive call itself failed (ingress or due call).
    DeadlineDrive(ErrorCode),
    /// A due scan phase failed inside an otherwise successful due call.
    DueScan(crate::ports::DuePhase, ErrorCode),
    RetirementDiscovery(ErrorCode),
    RetirementCleanup(ErrorCode),
    WorkDiscovery(ErrorCode),
    WorkJob(ErrorCode),
    /// A wake store callback failed. `multiple_unsettled` marks distinct
    /// unresolved attempts that one later success cannot settle.
    WakeCallback {
        phase: WakePhase,
        code: ErrorCode,
        multiple_unsettled: bool,
    },
    /// The wake driver failed without any observed store callback failure.
    WakeDriver(ErrorCode),
    /// A host observation attempt failed before a durable outcome.
    ObservationCapture(ErrorCode),
    /// A host observation was durably invalidated (fail-closed).
    ObservationInvalidated(HostInvalidationReason),
    ObservationReconciliation(ErrorCode),
}
impl RedactedFailure {
    pub fn summary(&self) -> String {
        match self {
            Self::DeadlineDrive(code) => format!("deadline drive failed: {code:?}"),
            Self::DueScan(crate::ports::DuePhase::Invitations, code) => {
                format!("invitation due scan failed: {code:?}")
            }
            Self::DueScan(crate::ports::DuePhase::Receipts, code) => {
                format!("receipt due scan failed: {code:?}")
            }
            Self::RetirementDiscovery(code) => format!("retirement discovery failed: {code:?}"),
            Self::RetirementCleanup(code) => format!("retirement cleanup failed: {code:?}"),
            Self::WorkDiscovery(code) => format!("work discovery failed: {code:?}"),
            Self::WorkJob(code) => format!("work job failed: {code:?}"),
            Self::WakeCallback {
                phase,
                code,
                multiple_unsettled,
            } => {
                let mut summary = format!("{} failed: {code:?}", phase.label());
                if *multiple_unsettled {
                    summary.push_str(" (multiple attempts unsettled)");
                }
                summary
            }
            Self::WakeDriver(code) => format!("unverified wake driver recovery: {code:?}"),
            Self::ObservationCapture(code) => format!("host observation capture failed: {code:?}"),
            Self::ObservationInvalidated(reason) => {
                format!("host observation invalidated: {reason:?}")
            }
            Self::ObservationReconciliation(code) => {
                format!("host observation reconciliation failed: {code:?}")
            }
        }
    }
}

/// A lane's Health status: the typed redacted current failure, and whether
/// failure tracking overflowed. It carries no free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedWorkerHealth {
    pub failure: Option<RedactedFailure>,
    pub tracking_gap: bool,
}
impl RedactedWorkerHealth {
    pub fn summary(&self) -> String {
        match (&self.failure, self.tracking_gap) {
            (Some(failure), true) => {
                format!("{}; unresolved failure tracking gap", failure.summary())
            }
            (Some(failure), false) => failure.summary(),
            (None, _) => "unresolved failure tracking gap".into(),
        }
    }
}

/// Bounded private diagnostic text. It never reaches Health.
fn diagnostic(text: impl Into<String>) -> String {
    text.into().chars().take(512).collect()
}
fn error_diagnostic(context: &str, error: &ApiError) -> String {
    diagnostic(format!("{context}: {:?}: {}", error.code, error.detail))
}

#[derive(Default)]
struct RecoverySweep {
    active: bool,
    valid: bool,
}
struct WakeFailure {
    phase: WakePhase,
    /// Private correlation only: the seat or attempt the failure belongs to.
    identity: Option<String>,
    code: ErrorCode,
    multiple_unsettled: bool,
}
/// The single failure-state owner of one worker lane. Every current failure
/// is typed and redacted; exact detail is kept only in `history`.
#[derive(Default)]
struct FailureState {
    current: Vec<(FailureKey, RedactedFailure)>,
    history: VecDeque<String>,
    wake: Vec<WakeFailure>,
    gaps: [bool; 2],
    sweeps: [RecoverySweep; 2],
}
impl FailureState {
    fn remember(&mut self, detail: String) {
        if self.history.len() == 8 {
            self.history.pop_front();
        }
        self.history.push_back(detail);
    }
    /// Record (with its private diagnostic) or clear the failure for `key`.
    fn update(&mut self, key: FailureKey, error: Option<(RedactedFailure, String)>) {
        if let Some((failure, detail)) = error {
            self.remember(detail);
            if let Some((_, current)) = self.current.iter_mut().find(|(known, _)| known == &key) {
                *current = failure;
            } else if key.job_class().is_some()
                && self
                    .current
                    .iter()
                    .filter(|(key, _)| key.job_class().is_some())
                    .count()
                    == 64
            {
                let class = key.job_class().expect("job key");
                self.gaps[class] = true;
                self.sweeps[class].valid = false;
            } else {
                self.current.push((key, failure));
            }
        } else {
            self.current.retain(|(known, _)| known != &key);
        }
    }
    fn sweep(
        &mut self,
        class: usize,
        discovery: Option<&crate::scheduler::deadlines::DiscoveryProgress>,
        completed: bool,
        error: bool,
        new_gap: bool,
    ) {
        if error {
            self.sweeps[class].valid = false;
        }
        let Some(discovery) = discovery else {
            return;
        };
        if discovery.fresh_start {
            self.sweeps[class] = RecoverySweep {
                active: true,
                valid: !new_gap && !error,
            };
        }
        if discovery.returned_job && !completed {
            self.sweeps[class].valid = false;
        }
        if discovery.reached_end {
            if self.sweeps[class].active && self.sweeps[class].valid {
                self.gaps[class] = false;
            }
            self.sweeps[class].active = false;
        }
    }
}

/// Retirement and work failures reach Health only as a typed redacted class.
/// Only retained quantum failures are durable on the retirement or work row;
/// a top-level `Err` (budget, transaction begin, job load, discovery) is
/// persisted nowhere. So the lane's private diagnostic tail always keeps the
/// exact bounded detail, and Health never does.
fn job_failure(class: usize, job: bool, error: &ApiError) -> (RedactedFailure, String) {
    let code = error.code.clone();
    let (failure, context) = match (class, job) {
        (0, true) => (RedactedFailure::RetirementCleanup(code), "retirement job"),
        (0, false) => (
            RedactedFailure::RetirementDiscovery(code),
            "retirement discovery",
        ),
        (_, true) => (RedactedFailure::WorkJob(code), "work job"),
        (_, false) => (RedactedFailure::WorkDiscovery(code), "work discovery"),
    };
    (failure, error_diagnostic(context, error))
}

/// See `WorkerStatus::lane_guard`.
pub struct LaneLiveness {
    status: Arc<WorkerStatus>,
    cancellation: Cancellation,
}
impl Drop for LaneLiveness {
    fn drop(&mut self) {
        if thread::panicking() || !self.cancellation.is_cancelled() {
            self.status.lane_dead.store(true, Ordering::SeqCst);
        }
    }
}

/// One lane's current unresolved operations (typed, redacted) and a separate,
/// fixed-size private diagnostic tail.
#[derive(Default)]
pub struct WorkerStatus {
    state: Mutex<FailureState>,
    /// Completion time of this lane's last error-free pass.
    last_tick: Mutex<Option<crate::protocol::time::UtcMillis>>,
    /// Set when the lane's thread ended while its owner had not asked it to
    /// stop (panic or early return); never cleared.
    lane_dead: AtomicBool,
}
impl WorkerStatus {
    /// Guard held for the lane thread's whole body: if the thread unwinds or
    /// returns without a requested stop, the lane is recorded dead so Health
    /// stops reporting it Ready.
    pub fn lane_guard(self: &Arc<Self>, cancellation: &Cancellation) -> LaneLiveness {
        LaneLiveness {
            status: Arc::clone(self),
            cancellation: cancellation.clone(),
        }
    }
    /// True once the lane thread ended without a requested stop.
    pub fn lane_dead(&self) -> bool {
        self.lane_dead.load(Ordering::SeqCst)
    }

    /// Completion time of the last error-free pass; None until one completed.
    pub fn last_tick(&self) -> Option<crate::protocol::time::UtcMillis> {
        self.last_tick.lock().ok().and_then(|tick| *tick)
    }
    /// Record that a pass completed without error at `at`.
    pub fn record_tick(&self, at: crate::protocol::time::UtcMillis) {
        if let Ok(mut tick) = self.last_tick.lock() {
            *tick = Some(at);
        }
    }
    /// The typed redacted status production Health consumes.
    pub fn health(&self) -> Option<RedactedWorkerHealth> {
        let state = self.state.lock().ok()?;
        let failure = state
            .current
            .first()
            .map(|(_, failure)| failure.clone())
            .or_else(|| {
                state
                    .wake
                    .first()
                    .map(|failure| RedactedFailure::WakeCallback {
                        phase: failure.phase,
                        code: failure.code.clone(),
                        multiple_unsettled: failure.multiple_unsettled,
                    })
            });
        let tracking_gap = state.gaps.iter().any(|gap| *gap);
        (failure.is_some() || tracking_gap).then_some(RedactedWorkerHealth {
            failure,
            tracking_gap,
        })
    }
    /// The Health text of `health()`: fixed classes and codes only.
    pub fn last_error(&self) -> Option<String> {
        self.health().map(|health| health.summary())
    }
    /// The latest exact private diagnostic. Never rendered into Health.
    pub fn last_diagnostic(&self) -> Option<String> {
        self.state.lock().ok()?.history.back().cloned()
    }
    fn observe_wake_callback(
        &self,
        phase: WakePhase,
        identity: Option<String>,
        error: Option<&ApiError>,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(error) = error {
            state.remember(diagnostic(format!(
                "{} {identity:?}: {:?}: {}",
                phase.label(),
                error.code,
                error.detail
            )));
            if let Some(failure) = state.wake.iter_mut().find(|failure| failure.phase == phase) {
                // Distinct unresolved completions cannot be represented as one
                // correlated attempt. Keep a finite unsettled class, never
                // clear it from another attempt's success.
                if failure.identity != identity {
                    failure.multiple_unsettled = true;
                }
                failure.code = error.code.clone();
            } else {
                state.wake.push(WakeFailure {
                    phase,
                    identity,
                    code: error.code.clone(),
                    multiple_unsettled: false,
                });
            }
        } else {
            state.wake.retain(|failure| {
                failure.phase != phase || failure.identity != identity || failure.multiple_unsettled
            });
        }
    }
    fn record(&self, key: FailureKey, error: Option<(RedactedFailure, String)>) {
        if let Ok(mut state) = self.state.lock() {
            if error.is_some() && key == FailureKey::General {
                for sweep in &mut state.sweeps {
                    sweep.valid = false;
                }
            }
            state.update(key, error);
        }
    }
    fn record_drive_error(&self, error: &ApiError) {
        self.record(
            FailureKey::General,
            Some((
                RedactedFailure::DeadlineDrive(error.code.clone()),
                error_diagnostic("deadline drive", error),
            )),
        );
    }
    /// Preserve completed callback facts even if the enclosing drive fails.
    pub fn drive_deadlines<P: DeadlinePort + ?Sized>(
        &self,
        driver: &mut DeadlineDriver<'_, P>,
        budget: &CallBudget,
    ) -> Result<DriveOutcome, ApiError> {
        let mut partial_recorded = false;
        let result = driver.drive_with_partial(budget, |partial, error| {
            // Publish the terminal failure first so Health cannot briefly
            // become Ready when the prefix recovers an earlier failed job.
            // A prefix preceding a due Err has no completed due progress,
            // so observing it cannot clear this General failure.
            self.record_drive_error(error);
            self.observe(partial);
            partial_recorded = true;
        });
        if !partial_recorded {
            self.observe_drive(&result);
        }
        // Health's `last_scheduler_tick_at`: every tick (not a skipped call)
        // that finished without error, whether or not any deadline was due.
        if result.as_ref().is_ok_and(|outcome| outcome.ticked) {
            self.record_tick(driver.clock().utc_now());
        }
        result
    }
    pub fn observe_drive(&self, result: &Result<DriveOutcome, ApiError>) {
        match result {
            Ok(outcome) => self.observe(outcome),
            Err(error) => self.record_drive_error(error),
        }
    }
    /// Skipped/default/incomplete outcomes provide no recovery authority.
    pub fn observe(&self, outcome: &DriveOutcome) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(progress) = &outcome.due_progress {
            for (key, due_phase, phase) in [
                (
                    FailureKey::Invitations,
                    crate::ports::DuePhase::Invitations,
                    &progress.invitations,
                ),
                (
                    FailureKey::Receipts,
                    crate::ports::DuePhase::Receipts,
                    &progress.receipts,
                ),
            ] {
                match phase {
                    DuePhaseProgress::Failed(code, detail) => state.update(
                        key,
                        Some((
                            RedactedFailure::DueScan(due_phase, code.clone()),
                            diagnostic(format!("{due_phase:?} due scan: {}", detail.as_str())),
                        )),
                    ),
                    DuePhaseProgress::Complete => state.update(key, None),
                    DuePhaseProgress::More | DuePhaseProgress::Skipped => {}
                }
            }
        }
        let old_gaps = state.gaps;
        for (class, discovery_key, job, error, complete, progressed, discovery) in [
            (
                0,
                FailureKey::RetirementDiscovery,
                outcome
                    .retirement_job
                    .as_ref()
                    .map(|id| id.as_str().to_owned()),
                outcome.retirement_error.as_ref(),
                outcome.retirement_complete,
                outcome.retirement_progressed,
                outcome.retirement_discovery.as_ref(),
            ),
            (
                1,
                FailureKey::WorkDiscovery,
                outcome.work_job.clone(),
                outcome.work_error.as_ref(),
                outcome.work_complete,
                outcome.work_progressed,
                outcome.work_discovery.as_ref(),
            ),
        ] {
            if discovery.is_some() {
                state.update(discovery_key.clone(), None);
            }
            if let Some(job) = job {
                let key = if class == 0 {
                    FailureKey::Retirement(job)
                } else {
                    FailureKey::Work(job)
                };
                if let Some(error) = error {
                    state.update(key, Some(job_failure(class, true, error)));
                } else if complete || progressed {
                    // Committed progress on this job is recovery; the store
                    // has already cleared the job's retained diagnostic.
                    state.update(key, None);
                }
            } else if let Some(error) = error {
                state.update(discovery_key, Some(job_failure(class, false, error)));
            }
            let new_gap = !old_gaps[class] && state.gaps[class];
            state.sweep(class, discovery, complete, error.is_some(), new_gap);
        }
        if let Some(progress) = &outcome.due_progress
            && matches!(progress.invitations, DuePhaseProgress::Complete)
            && matches!(progress.receipts, DuePhaseProgress::Complete)
        {
            // DeadlineDriver returns Err only at ingress or from the due
            // call. A completed due retry recovers that call independently
            // of unrelated retirement/work progress; their failures have
            // separate keys and completion requirements.
            state.update(FailureKey::General, None);
        }
    }
    /// The observation worker's capture result. A durable publication (or a
    /// newer durable decision superseding this attempt) clears the capture
    /// failure; an invalidation records its typed reason; `Ok(None)` (not
    /// due) changes nothing.
    pub fn observe_capture(
        &self,
        result: &Result<Option<crate::identity::reconcile::ObservationOutcome>, ApiError>,
    ) {
        use crate::identity::reconcile::ObservationOutcome;
        let failure = match result {
            Ok(None) => return,
            Ok(Some(ObservationOutcome::Published(_) | ObservationOutcome::Superseded)) => None,
            Ok(Some(ObservationOutcome::Invalidated { reason, cause, .. })) => Some((
                RedactedFailure::ObservationInvalidated(*reason),
                diagnostic(format!("capture {reason:?}: {cause:?}")),
            )),
            Err(error) => Some((
                RedactedFailure::ObservationCapture(error.code.clone()),
                error_diagnostic("capture", error),
            )),
        };
        self.record(FailureKey::ObservationCapture, failure);
    }
    /// The observation worker's reconciliation page result. A committed page
    /// clears the reconciliation failure.
    pub fn observe_reconciliation<T>(&self, result: &Result<T, ApiError>) {
        self.record(
            FailureKey::ObservationReconciliation,
            result.as_ref().err().map(|error| {
                (
                    RedactedFailure::ObservationReconciliation(error.code.clone()),
                    error_diagnostic("observation reconciliation", error),
                )
            }),
        );
    }
}

/// Each driver call uses bounded store quanta. The writer guard is released
/// between due, retirement and work transactions, so foreground admissions
/// can take their fair turn. The worker starts only inside the elected factory.
pub fn start_deadline_worker(
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("herdr-deadlines".into())
        .spawn(move || {
            let _guard = status.lane_guard(&cancellation);
            let port = ScheduledStore { store, writer };
            let mut driver = DeadlineDriver::new(&port);
            while !cancellation.is_cancelled() {
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        port.store.clock().monotonic_now().0.saturating_add(500),
                    ),
                    cancellation: cancellation.clone(),
                };
                let _ = status.drive_deadlines(&mut driver, &budget);
                for _ in 0..5 {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        })
}

impl WakePort for ScheduledStore {
    fn clock(&self) -> &dyn Clock {
        self.store.clock()
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        self.store.wake_candidates(page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.reserve_wake(candidate, budget)
    }
    fn complete_wake(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: WakeOutcome,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.complete_wake(attempt, outcome, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.store.wake_recovery_candidates(page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.recover_wake_reservation(request, budget)
    }
}

/// Observe actual store callbacks, including no-reservation decisions. A
/// scheduler Ok result alone cannot establish that any operation was retried.
pub struct ObservedWakePort<'a, P: WakePort + ?Sized> {
    port: &'a P,
    status: &'a WorkerStatus,
    callback_failed: std::sync::atomic::AtomicBool,
}
impl<'a, P: WakePort + ?Sized> ObservedWakePort<'a, P> {
    pub fn new(port: &'a P, status: &'a WorkerStatus) -> Self {
        Self {
            port,
            status,
            callback_failed: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn begin_drive(&self, budget: &CallBudget) {
        self.callback_failed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if !budget.is_exhausted(self.port.clock()) {
            self.status
                .observe_wake_callback(WakePhase::Admission, None, None);
        }
    }
    pub fn observe_drive(&self, result: &Result<crate::scheduler::WakeDriveOutcome, ApiError>) {
        if let Err(error) = result
            && !self
                .callback_failed
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            if matches!(
                error.code,
                ErrorCode::DeadlineExceeded | ErrorCode::Cancelled
            ) {
                self.status
                    .observe_wake_callback(WakePhase::Admission, None, Some(error));
            } else {
                // No callback can verify this recovery: the failure
                // stays current for this lane.
                self.status.record(
                    FailureKey::General,
                    Some((
                        RedactedFailure::WakeDriver(error.code.clone()),
                        error_diagnostic("unverified wake driver recovery", error),
                    )),
                );
            }
        }
    }
    fn observed<T>(
        &self,
        phase: WakePhase,
        identity: Option<String>,
        result: Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        if result.is_err() {
            self.callback_failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.status
            .observe_wake_callback(phase, identity, result.as_ref().err());
        result
    }
}
impl<P: WakePort + ?Sized> WakePort for ObservedWakePort<'_, P> {
    fn clock(&self) -> &dyn Clock {
        self.port.clock()
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        self.observed(
            WakePhase::Discovery,
            None,
            self.port.wake_candidates(page, budget),
        )
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.observed(
            WakePhase::Reservation,
            Some(candidate.seat.as_str().into()),
            self.port.reserve_wake(candidate, budget),
        )
    }
    fn complete_wake(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: WakeOutcome,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.observed(
            WakePhase::Completion,
            Some(attempt.as_str().into()),
            self.port.complete_wake(attempt, outcome, budget),
        )
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.observed(
            WakePhase::RecoveryDiscovery,
            None,
            self.port.wake_recovery_candidates(page, budget),
        )
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        self.observed(
            WakePhase::Recovery,
            Some(request.attempt.as_str().into()),
            self.port.recover_wake_reservation(request, budget),
        )
    }
}

/// The wake driver owns every physical attempt until the host call exits. It
/// runs separately from local deadlines and retains no writer turn during I/O.
// Allowed: worker start-up wiring: each argument is a distinct dependency.
#[allow(clippy::too_many_arguments)]
pub fn start_wake_worker(
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    host: Arc<dyn HostPort>,
    instance: String,
    boot: uuid::Uuid,
    retry: RetryConfig,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("herdr-wakes".into())
        .spawn(move || {
            let _guard = status.lane_guard(&cancellation);
            let port = ScheduledStore { store, writer };
            let notifier =
                NativeWakeDispatcher::new(host.as_ref(), port.store.as_ref(), port.store.clock());
            let observed = ObservedWakePort::new(&port, &status);
            let scheduler = Scheduler::new(instance, &port, &observed, &notifier, retry, boot);
            while !cancellation.is_cancelled() {
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        port.store.clock().monotonic_now().0.saturating_add(5_000),
                    ),
                    cancellation: cancellation.clone(),
                };
                observed.begin_drive(&budget);
                observed.observe_drive(&scheduler.drive_wakes(&budget));
                for _ in 0..5 {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        })
}

impl crate::identity::reconcile::observation_store::ObservationStore for ScheduledStore {
    fn clock(&self) -> &dyn Clock {
        self.store.clock()
    }
    fn begin_host_observation(
        &self,
        instance: &str,
        budget: &CallBudget,
    ) -> Result<HostObservationAdmission, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.begin_host_observation(instance, budget)
    }
    fn invalidate_host_observation(
        &self,
        admission: &HostObservationAdmission,
        reason: HostInvalidationReason,
        budget: &CallBudget,
    ) -> Result<Option<HostInvalidationFence>, ApiError> {
        // The host may return after its call was cancelled. Cleanup starts a
        // fresh finite service budget only after physical host completion.
        let _ = budget;
        let maintenance = CallBudget {
            deadline: crate::protocol::time::MonoInstant(
                self.store.clock().monotonic_now().0.saturating_add(2_000),
            ),
            cancellation: Cancellation::default(),
        };
        let _turn = self
            .writer
            .enter_background(&maintenance, self.store.clock())?;
        self.store
            .invalidate_host_observation(admission, reason, &maintenance)
    }
    fn mark_unresolved_from_invalidation(
        &self,
        transition: GuardedInvalidationTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store
            .mark_unresolved_from_invalidation(transition, budget)
    }
    fn saved_seats_page_for_invalidation(
        &self,
        fence: &HostInvalidationFence,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<InvalidationSeatPage, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.saved_seats_page_for_invalidation(
            fence,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )
    }
    fn begin_snapshot_stage(
        &self,
        header: SnapshotHeader,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.begin_snapshot_stage(header, budget)
    }
    fn stage_snapshot_targets(
        &self,
        stage: &SnapshotGenerationId,
        offset: u64,
        targets: &[HostObservation],
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotStageProgress, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store
            .stage_snapshot_targets(stage, offset, targets, admission, budget)
    }
    fn seal_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.seal_snapshot_stage(stage, budget)
    }
    fn publish_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<PublishedSnapshot, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.publish_snapshot_stage(stage, budget)
    }
    fn discard_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotCleanupProgress, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.discard_snapshot_stage(stage, admission, budget)
    }
    fn saved_seats_page(
        &self,
        published: &SnapshotGenerationId,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<SnapshotSeatPage, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store
            .saved_seats_page(published, after_ordinal, high_water_ordinal, limit, budget)
    }
    fn apply_reconciliation_transition(
        &self,
        transition: GuardedSeatTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store
            .apply_reconciliation_transition(transition, budget)
    }
    fn record_reconciliation_pass(
        &self,
        published: &crate::ports::PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.record_reconciliation_pass(published, budget)
    }
}

/// End-of-page decision of a saved-seat pass: whether the pass is complete and
/// whether any page so far had a refused transition. A pass with a refusal never
/// records the reconciliation marker (TRUST-POLICY C2).
fn pass_complete(
    refused_so_far: bool,
    page: &crate::identity::reconcile::ReconcilePageProgress,
) -> (bool, bool) {
    (
        page.next_after_ordinal.is_none(),
        refused_so_far || page.transitions_refused > 0,
    )
}

/// The elected owner joins this thread before releasing its lease. A cancelled
/// host call must physically return before this worker can finish.
pub fn start_observation_worker(
    identity: Arc<crate::identity::repair::OrdinaryIdentity>,
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
    host_evidence: Arc<crate::service::host_evidence::HostEvidenceStatus>,
) -> std::io::Result<thread::JoinHandle<()>> {
    spawn_observation_loop(
        identity,
        ScheduledStore { store, writer },
        cancellation,
        status,
        host_evidence,
    )
}

/// The observation worker loop over its scheduled store port. Every feeder
/// of the lane's `WorkerStatus` (capture and reconciliation) is called here;
/// the production entry point above only supplies the fair-writer port.
fn spawn_observation_loop<S>(
    identity: Arc<crate::identity::repair::OrdinaryIdentity>,
    port: S,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
    host_evidence: Arc<crate::service::host_evidence::HostEvidenceStatus>,
) -> std::io::Result<thread::JoinHandle<()>>
where
    S: crate::identity::reconcile::observation_store::ObservationStore + 'static,
{
    thread::Builder::new()
        .name("herdr-observations".into())
        .spawn(move || {
            let _guard = status.lane_guard(&cancellation);
            let mut continuation = None;
            while !cancellation.is_cancelled() {
                let now = port.clock().monotonic_now().0;
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(now.saturating_add(5_000)),
                    cancellation: cancellation.clone(),
                };
                if let Some((outcome, after, high, refused)) = continuation.take() {
                    let page = identity.reconcile_page(&port, &outcome, after, high, &budget);
                    status.observe_reconciliation(&page);
                    if let Ok(page) = page {
                        let (done, refused) = pass_complete(refused, &page);
                        if let (false, Some(next)) = (done, page.next_after_ordinal) {
                            continuation =
                                Some((outcome, next, Some(page.high_water_ordinal), refused));
                        } else if let crate::identity::reconcile::ObservationOutcome::Published(
                            published,
                        ) = &outcome
                        {
                            if !refused {
                                // An Err leaves the marker behind; the next
                                // pass retries.
                                let _ = port.record_reconciliation_pass(published, &budget);
                            }
                            host_evidence.record_reconciled(port.clock().utc_now());
                        }
                    }
                } else {
                    let maintenance = CallBudget {
                        deadline: crate::protocol::time::MonoInstant(now.saturating_add(7_000)),
                        cancellation: Cancellation::default(),
                    };
                    let captured = identity.capture_if_due(&port, &budget, &maintenance);
                    status.observe_capture(&captured);
                    match captured {
                        Ok(Some(outcome)) => {
                            match &outcome {
                                crate::identity::reconcile::ObservationOutcome::Published(_) => {
                                    host_evidence.record_published()
                                }
                                crate::identity::reconcile::ObservationOutcome::Invalidated {
                                    reason,
                                    cause,
                                    ..
                                } => host_evidence.record_invalidated(*reason, cause.as_ref()),
                                crate::identity::reconcile::ObservationOutcome::Superseded => {}
                            }
                            if !matches!(
                                outcome,
                                crate::identity::reconcile::ObservationOutcome::Superseded
                            ) {
                                continuation = Some((outcome, 0, None, false));
                            }
                        }
                        Ok(None) => {}
                        // No durable outcome: never leave an earlier
                        // publication reported as verified host evidence.
                        Err(error) => host_evidence.record_capture_failed(&error),
                    }
                }
                // One bounded page per turn; failed captures cannot hot-loop.
                for _ in 0..5 {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
        })
}

#[cfg(test)]
#[path = "../../tests/service/worker_health.rs"]
mod tests;
