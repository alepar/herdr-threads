//! Bounded admission for work that must not occupy the domain writer.

use crate::daemon::logs::{LaneErrorLog, NoopLaneErrorLog};
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
use crate::service::fair_writer::FairWriter;
use crate::service::host_reachability::HostReachability;
use crate::service::kicks::{self, Lane};
use crate::service::pacer::{Pacer, Wake};
use crate::{
    notification::{dispatch::NativeWakeDispatcher, policy::RetryConfig},
    ports::{
        DuePhaseProgress, DueScanProgress, DueScanRequest, DurableWorkAdmission, HostPort, PokeDue,
        PokeReceipt, PokeReservation, RetirementProgress, StorePort, WakeCandidate, WakeOutcome,
        WakeRecoveryCandidate, WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation,
        WorkAdmission, WorkCandidate, WorkProgress,
    },
    protocol::{
        ids::{RetirementJobId, SeatId},
        pagination::{Page, PageRequest},
        results::RetirementStatus,
        time::Cancellation,
    },
    scheduler::{
        Scheduler, WakePort,
        deadlines::{DeadlineDriver, DeadlinePort, DriveOutcome, TICK_MILLIS},
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

/// The wake lane's safety tick (the deadline lane's is `TICK_MILLIS`).
const WAKE_SAFETY_TICK_MILLIS: u64 = 5_000;
/// The wake lane's wait when a seat's retry time has already passed.
const WAKE_MIN_WAIT_MILLIS: u64 = 100;

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

pub(super) fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

fn error_for_spawn(lane: Lane, error: &std::io::Error) -> ApiError {
    ApiError::service_busy(format!("lane {} thread spawn failed: {error}", lane.name()))
}

pub(super) fn budget_error(budget: &CallBudget) -> ApiError {
    if budget.cancellation.is_cancelled() {
        error(ErrorCode::Cancelled, "lane admission cancelled")
    } else {
        error(
            ErrorCode::DeadlineExceeded,
            "lane admission deadline elapsed",
        )
    }
}

pub(crate) struct ScheduledStore {
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
}

impl ScheduledStore {
    pub(crate) fn new(store: Arc<dyn StorePort>, writer: Arc<FairWriter>) -> Self {
        Self { store, writer }
    }
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
    /// A lane-reported failure (`WorkerStatus::record_failure`).
    LaneFailure,
    /// The lane's thread never started; only a restart clears it.
    LaneSpawn,
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
    BatchDiscovery,
    BatchCleanup,
    BatchWindow,
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
            Self::BatchDiscovery => "wake batch discovery",
            Self::BatchCleanup => "wake batch cleanup",
            Self::BatchWindow => "wake batch window",
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
    /// Herdr is unavailable: nothing was invalidated, state is frozen.
    ObservationFrozen(HostInvalidationReason),
    ObservationReconciliation(ErrorCode),
    /// A lane reported a failed pass through `WorkerStatus::record_failure`.
    LaneFailure(&'static str, ErrorCode),
    /// A lane's thread failed to spawn; the lane is absent.
    LaneSpawn(&'static str),
    /// The lane's status mutex is poisoned: its failure state can no longer be
    /// read, so the lane must never read as Ready.
    StatusPoisoned,
}
impl RedactedFailure {
    /// The typed code behind the failure; `None` for the variants that carry
    /// none (an invalidation reason, a spawn failure, a poisoned status).
    pub fn code(&self) -> Option<&ErrorCode> {
        match self {
            Self::DeadlineDrive(code)
            | Self::DueScan(_, code)
            | Self::RetirementDiscovery(code)
            | Self::RetirementCleanup(code)
            | Self::WorkDiscovery(code)
            | Self::WorkJob(code)
            | Self::WakeCallback { code, .. }
            | Self::WakeDriver(code)
            | Self::ObservationCapture(code)
            | Self::ObservationReconciliation(code)
            | Self::LaneFailure(_, code) => Some(code),
            Self::ObservationInvalidated(_)
            | Self::ObservationFrozen(_)
            | Self::LaneSpawn(_)
            | Self::StatusPoisoned => None,
        }
    }

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
            Self::ObservationFrozen(reason) => {
                format!(
                    "host unavailable ({reason:?}): seats and bindings frozen until Herdr answers"
                )
            }
            Self::ObservationReconciliation(code) => {
                format!("host observation reconciliation failed: {code:?}")
            }
            Self::LaneFailure(lane, code) => format!("lane {lane} failed: {code:?}"),
            Self::LaneSpawn(lane) => format!("lane {lane} failed to start"),
            Self::StatusPoisoned => "status poisoned".into(),
        }
    }
}

/// A lane's Health status: the typed redacted current failure, and whether
/// failure tracking overflowed. It carries no free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedWorkerHealth {
    pub failure: Option<RedactedFailure>,
    pub tracking_gap: bool,
    /// The attached Pacer's backoff: attempt count and whole seconds (rounded
    /// up) until the next retry. Set only while attempts > 0.
    pub retry: Option<(u32, u64)>,
}
impl RedactedWorkerHealth {
    pub fn summary(&self) -> String {
        let mut summary = match (&self.failure, self.tracking_gap) {
            (Some(failure), true) => {
                format!("{}; unresolved failure tracking gap", failure.summary())
            }
            (Some(failure), false) => failure.summary(),
            (None, _) => "unresolved failure tracking gap".into(),
        };
        if let Some((attempts, seconds)) = self.retry {
            summary.push_str(&format!(
                "; retrying (attempt {attempts}, next ≤ {seconds}s)"
            ));
        }
        summary
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
    /// Where `record_failure` reports; unset means `NoopLaneErrorLog`.
    error_log: Mutex<Option<Arc<dyn LaneErrorLog>>>,
    /// The lane's Pacer, whose backoff Health publishes as a retry suffix.
    pacer: Mutex<Option<Arc<Pacer>>>,
}
impl WorkerStatus {
    /// Attaches the lane's Pacer so Health shows its retry state.
    pub fn attach_pacer(&self, pacer: Arc<Pacer>) {
        if let Ok(mut slot) = self.pacer.lock() {
            *slot = Some(pacer);
        }
    }
    /// Whether a Pacer is attached (test support).
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_pacer(&self) -> bool {
        self.pacer.lock().is_ok_and(|slot| slot.is_some())
    }
    /// The attached Pacer's `(attempts, next_retry_at)` while attempts > 0.
    pub fn retry(&self) -> Option<(u32, crate::protocol::time::MonoInstant)> {
        let pacer = self.pacer.lock().ok()?.clone()?;
        let attempts = pacer.attempts();
        if attempts == 0 {
            return None;
        }
        Some((attempts, pacer.next_retry_at()?))
    }
    /// Installs the lane-error hook `record_failure` calls.
    pub fn set_error_log(&self, log: Arc<dyn LaneErrorLog>) {
        if let Ok(mut slot) = self.error_log.lock() {
            *slot = Some(log);
        }
    }
    fn report_to_error_log(&self, lane: Lane, error: &ApiError) {
        // Clone out so the hook never runs under the status lock.
        let log = self.error_log.lock().ok().and_then(|slot| slot.clone());
        let attempt = self.retry().map_or(0, |(attempts, _)| attempts);
        match log {
            Some(log) => log.record_with_attempt(lane, error, attempt),
            None => NoopLaneErrorLog.record(lane, error),
        }
    }
    /// Reports a wake prompt whose submission was `NotChecked` or stayed
    /// `Unsubmitted` to the lane error log (rate limited there). A no-op when
    /// no log is set; never touches Health.
    pub fn report_wake_verification(
        &self,
        seat: &crate::protocol::ids::SeatId,
        verification: crate::scheduler::SubmissionVerification,
    ) {
        // Clone out so the hook never runs under the status lock.
        let log = self.error_log.lock().ok().and_then(|slot| slot.clone());
        if let Some(log) = log {
            log.record_wake_verification(seat, verification);
        }
    }
    /// The only way a lane reports a good pass: clears the lane's reported
    /// failure and advances `last_tick`.
    pub fn record_success(&self, at: crate::protocol::time::UtcMillis) {
        self.record(FailureKey::LaneFailure, None);
        self.record_tick(at);
    }
    /// The only way a lane reports a failed pass: records the typed failure
    /// for Health (`last_tick` does not advance) and calls the lane-error hook.
    pub fn record_failure(&self, lane: Lane, error: &ApiError) {
        self.record(
            FailureKey::LaneFailure,
            Some((
                RedactedFailure::LaneFailure(lane.name(), error.code.clone()),
                error_diagnostic(lane.name(), error),
            )),
        );
        self.report_to_error_log(lane, error);
    }
    /// Records that `lane`'s thread failed to spawn so Health shows it
    /// degraded instead of the lane being silently absent.
    pub fn record_spawn_failure(&self, lane: Lane, error: &std::io::Error) {
        let api = error_for_spawn(lane, error);
        self.record(
            FailureKey::LaneSpawn,
            Some((
                RedactedFailure::LaneSpawn(lane.name()),
                error_diagnostic(lane.name(), &api),
            )),
        );
        self.report_to_error_log(lane, &api);
    }
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
    /// Record that a pass completed without error at `at`. Only a good pass
    /// reports through here (`record_success`, an error-free deadline tick).
    fn record_tick(&self, at: crate::protocol::time::UtcMillis) {
        if let Ok(mut tick) = self.last_tick.lock() {
            *tick = Some(at);
        }
    }
    /// The typed redacted status production Health consumes.
    pub fn health(&self) -> Option<RedactedWorkerHealth> {
        // A poisoned lock means the failure state is unreadable: report the
        // lane degraded, never Ready.
        let Ok(state) = self.state.lock() else {
            return Some(RedactedWorkerHealth {
                failure: Some(RedactedFailure::StatusPoisoned),
                tracking_gap: false,
                retry: None,
            });
        };
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
        let retry = self.retry().map(|(attempts, next)| {
            let now = self
                .pacer
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().map(|pacer| pacer.now().0))
                .unwrap_or(next.0);
            (attempts, next.0.saturating_sub(now).div_ceil(1000))
        });
        (failure.is_some() || tracking_gap).then_some(RedactedWorkerHealth {
            failure,
            tracking_gap,
            retry,
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
        // A pass that retained a job `work_error` is not an error-free pass.
        if result
            .as_ref()
            .is_ok_and(|outcome| outcome.ticked && outcome.work_error.is_none())
        {
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
        let mut logged = None;
        let failure = match result {
            Ok(None) => return,
            Ok(Some(ObservationOutcome::Published(_) | ObservationOutcome::Superseded)) => None,
            Ok(Some(ObservationOutcome::Frozen { reason, cause })) => {
                logged = Some(ApiError::new(
                    ErrorCode::HostUnavailable,
                    format!("capture frozen ({reason:?}): {cause:?}"),
                ));
                Some((
                    RedactedFailure::ObservationFrozen(*reason),
                    diagnostic(format!("capture {reason:?}: {cause:?}")),
                ))
            }
            Ok(Some(
                ObservationOutcome::Invalidated { reason, cause, .. }
                | ObservationOutcome::InvalidationRepeated { reason, cause },
            )) => {
                logged = Some(ApiError::new(
                    ErrorCode::HostUnavailable,
                    format!("capture invalidated ({reason:?}): {cause:?}"),
                ));
                Some((
                    RedactedFailure::ObservationInvalidated(*reason),
                    diagnostic(format!("capture {reason:?}: {cause:?}")),
                ))
            }
            Err(error) => {
                logged = Some(error.clone());
                Some((
                    RedactedFailure::ObservationCapture(error.code.clone()),
                    error_diagnostic("capture", error),
                ))
            }
        };
        self.record(FailureKey::ObservationCapture, failure);
        if let Some(error) = logged {
            self.report_to_error_log(Lane::Observation, &error);
        }
    }
    /// The observation worker's reconciliation page result. A committed page
    /// clears the reconciliation failure.
    pub fn observe_reconciliation<T>(&self, result: &Result<T, ApiError>) {
        if let Err(error) = result {
            self.report_to_error_log(Lane::Observation, error);
        }
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
    pacer: Arc<Pacer>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("herdr-deadlines".into())
        .spawn(move || {
            let _origin = kicks::enter_lane(Lane::Deadlines);
            status.attach_pacer(Arc::clone(&pacer));
            let _guard = status.lane_guard(&cancellation);
            let port = ScheduledStore::new(store, writer);
            let mut driver = DeadlineDriver::new(&port);
            let tick = Duration::from_millis(TICK_MILLIS);
            // The first pass runs at boot, before the first wait.
            let mut waited = false;
            while !cancellation.is_cancelled() {
                if waited {
                    match pacer.wait_blocking(tick) {
                        Wake::Cancelled => break,
                        // A kick means committed work; a due retry must not be
                        // skipped by the tick gate a failed pass left closed.
                        Wake::Kicked | Wake::RetryDue => driver.after_committed_change(),
                        Wake::Tick => {}
                    }
                }
                waited = true;
                loop {
                    let budget = CallBudget {
                        deadline: crate::protocol::time::MonoInstant(
                            port.store.clock().monotonic_now().0.saturating_add(500),
                        ),
                        cancellation: cancellation.clone(),
                    };
                    match status.drive_deadlines(&mut driver, &budget) {
                        Err(error) => {
                            if !cancellation.is_cancelled() {
                                pacer.on_failure();
                                status.record_failure(Lane::Deadlines, &error);
                            }
                            break;
                        }
                        Ok(outcome) => {
                            pacer.on_success();
                            // A skipped (gated) call is not a completed tick.
                            if outcome.ticked {
                                status.record_success(port.store.clock().utc_now());
                            }
                            if outcome.progressed_with_more() {
                                // Retirement and work progress do not reopen the
                                // tick gate themselves.
                                driver.after_committed_change();
                            }
                            if cancellation.is_cancelled()
                                || !(outcome.due_continuation || outcome.progressed_with_more())
                            {
                                break;
                            }
                        }
                    }
                }
            }
        })
}

/// Safety tick of the retention lane: with nothing kicking it (no table maps
/// to `Lane::Retention`), it still looks for prunable rows once a minute.
const RETENTION_TICK: Duration = Duration::from_secs(60);

/// The retention lane (nested spec D3/D4): prunes superseded snapshot
/// generations and completed work jobs on a `Pacer`. Each pass takes one
/// background writer turn and runs the store's bounded `prune_retention`;
/// `has_more` re-runs at once, so a backlog drains one batch per writer turn
/// and a request write waits for at most one batch. A failed pass backs off on
/// the Pacer (100 ms x 2^n, 30 s cap) and is reported through
/// `WorkerStatus::record_failure`, which feeds Health's retry suffix and the
/// lane error log. Cancellation ends the lane after the batch in flight.
pub fn start_retention_worker(
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    pacer: Arc<Pacer>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
) -> std::io::Result<thread::JoinHandle<()>> {
    status.attach_pacer(Arc::clone(&pacer));
    thread::Builder::new()
        .name("herdr-retention".into())
        .spawn(move || {
            // Commits made on this thread count as Retention-origin and kick no lane.
            let _origin = crate::service::kicks::enter_lane(Lane::Retention);
            let _guard = status.lane_guard(&cancellation);
            let pass = || -> Result<crate::ports::PruneProgress, ApiError> {
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        store.clock().monotonic_now().0.saturating_add(5_000),
                    ),
                    cancellation: cancellation.clone(),
                };
                let _turn = writer.enter_background(&budget, store.clock())?;
                store.prune_retention(&budget)
            };
            while !cancellation.is_cancelled() {
                let mut has_more = true;
                while has_more && !cancellation.is_cancelled() {
                    match pass() {
                        Ok(progress) => {
                            pacer.on_success();
                            status.record_success(store.clock().utc_now());
                            has_more = progress.has_more;
                        }
                        Err(_) if cancellation.is_cancelled() => break,
                        Err(error) => {
                            pacer.on_failure();
                            status.record_failure(Lane::Retention, &error);
                            has_more = false;
                        }
                    }
                }
                if cancellation.is_cancelled()
                    || pacer.wait_blocking(RETENTION_TICK) == crate::service::pacer::Wake::Cancelled
                {
                    break;
                }
            }
        })
}

/// The admission observer's safety tick: a swapped `claude` or `codex` binary
/// is noticed within a minute, and one pass costs one `--version` run per
/// harness. Never below 5 s (nested spec D2).
pub const ADMISSION_TICK: Duration = Duration::from_secs(60);
const _: () = assert!(ADMISSION_TICK.as_secs() >= 5);

/// The admission-observer lane (root spec B2, nested spec D2): re-observes the
/// installed harnesses on a `Pacer` instead of once at boot, and replaces the
/// whole `HarnessObservations` in `slot` after each good pass so Health never
/// reads a half-updated pair. The first pass runs at once, then one per
/// [`ADMISSION_TICK`]; a failed pass backs off on the Pacer (100 ms x 2^n,
/// 30 s cap) and reports through `WorkerStatus::record_failure`, which feeds
/// Health's existing retry suffix. The lane's kick set is empty by decision:
/// it runs only on its tick and its backoff. It holds no store, so it commits
/// nothing. Cancellation also kills the pass's in-flight `--version` run, so
/// shutdown's join returns promptly and leaves no harness child behind.
pub(crate) fn start_admission_observer<O>(
    observe: O,
    slot: Arc<Mutex<crate::app::HarnessObservations>>,
    pacer: Arc<Pacer>,
    clock: Arc<dyn Clock>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
) -> std::io::Result<thread::JoinHandle<()>>
where
    O: Fn(&CallBudget) -> Result<crate::app::HarnessObservations, ApiError> + Send + 'static,
{
    status.attach_pacer(Arc::clone(&pacer));
    thread::Builder::new()
        .name("herdr-admission".into())
        .spawn(move || {
            let _origin = kicks::enter_lane(Lane::AdmissionObserver);
            let _guard = status.lane_guard(&cancellation);
            while !cancellation.is_cancelled() {
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        clock.monotonic_now().0.saturating_add(30_000),
                    ),
                    cancellation: cancellation.clone(),
                };
                match observe(&budget) {
                    Ok(observed) => {
                        *slot.lock().unwrap_or_else(|e| e.into_inner()) = observed;
                        pacer.on_success();
                        status.record_success(clock.utc_now());
                    }
                    Err(_) if cancellation.is_cancelled() => break,
                    Err(error) => {
                        pacer.on_failure();
                        status.record_failure(Lane::AdmissionObserver, &error);
                    }
                }
                if pacer.wait_blocking(ADMISSION_TICK) == Wake::Cancelled {
                    break;
                }
            }
        })
}

/// The mod channel worker's safety tick: it sweeps grace, cooldown and stall
/// state once a second (spec D7).
const MOD_CHANNEL_TICK: Duration = Duration::from_secs(1);
/// With no commit observed, seats with a live channel are still re-read every
/// this many ticks (a missed notify costs at most this long).
const MOD_CHANNEL_SAFETY_PASS_TICKS: u32 = 10;

/// The mod channel worker (spec D2, D7): one std thread on its own `Pacer`
/// (not a store lane: it commits nothing). A commit touching an attention
/// source (`kicks::MOD_NOTIFY_TABLES`) kicks the Pacer through the registry's
/// observer; each wake runs [`crate::service::mod_channels::ModChannelRegistry::pass`]
/// when dirty (and every [`MOD_CHANNEL_SAFETY_PASS_TICKS`] ticks regardless),
/// then `sweep`, which expires graces and cooldowns and closes stalled
/// channels. A failed pass backs off on the Pacer and is logged (first
/// failure, then every 60th). Cancellation closes every channel with
/// `Close{stopping}` and ends the thread; the caller joins it.
pub fn start_mod_channel_worker(
    registry: Arc<crate::service::mod_channels::ModChannelRegistry>,
    pacer: Arc<Pacer>,
    clock: Arc<dyn Clock>,
    cancellation: Cancellation,
    log: Arc<dyn Fn(&str) + Send + Sync>,
) -> std::io::Result<thread::JoinHandle<()>> {
    let wake = Arc::clone(&pacer);
    registry.set_worker_wake(Arc::new(move || wake.kick()));
    thread::Builder::new()
        .name("herdr-mod-channels".into())
        .spawn(move || {
            let mut ticks_since_pass = 0u32;
            let mut consecutive_failures = 0u64;
            while !cancellation.is_cancelled() {
                let now = clock.utc_now();
                let dirty = registry.take_dirty();
                ticks_since_pass += 1;
                if dirty || ticks_since_pass >= MOD_CHANNEL_SAFETY_PASS_TICKS {
                    ticks_since_pass = 0;
                    match registry.pass(now) {
                        Ok(()) => {
                            pacer.on_success();
                            consecutive_failures = 0;
                        }
                        Err(_) if cancellation.is_cancelled() => break,
                        Err(error) => {
                            pacer.on_failure();
                            registry.mark_dirty();
                            if consecutive_failures.is_multiple_of(60) {
                                log(&format!(
                                    "mod channel pass failed: {:?}: {}",
                                    error.code, error.detail
                                ));
                            }
                            consecutive_failures += 1;
                        }
                    }
                }
                registry.sweep(clock.utc_now());
                if pacer.wait_blocking(MOD_CHANNEL_TICK) == Wake::Cancelled {
                    break;
                }
            }
            registry.close_all(
                crate::protocol::watch::WatchCloseReason::Stopping,
                clock.utc_now(),
            );
        })
}

/// Checks the admission observer's spawn `Result`: a failed spawn is recorded
/// on `status` so Health shows the lane degraded instead of silently absent.
pub fn admission_handle(
    spawned: std::io::Result<thread::JoinHandle<()>>,
    status: &WorkerStatus,
) -> Option<thread::JoinHandle<()>> {
    match spawned {
        Ok(handle) => Some(handle),
        Err(error) => {
            status.record_spawn_failure(Lane::AdmissionObserver, &error);
            None
        }
    }
}

/// The final attempt fence runs on the store directly: it holds no writer turn
/// across the prompt I/O it precedes.
impl crate::notification::dispatch::ReservationCheck for ScheduledStore {
    fn is_current(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.store.validate_wake_reservation(reservation, budget)
    }
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
    fn wake_batch_seats(
        &self,
        after: Option<&crate::protocol::ids::SeatId>,
        limit: u16,
        budget: &CallBudget,
    ) -> Result<Vec<crate::protocol::ids::SeatId>, ApiError> {
        self.store.wake_batch_seats(after, limit, budget)
    }
    fn clear_wake_batch_if_empty(
        &self,
        seat: &crate::protocol::ids::SeatId,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.clear_wake_batch_if_empty(seat, budget)
    }
    fn wake_batch_window(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<(crate::protocol::time::UtcMillis, u64)>, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.wake_batch_window(candidate, budget)
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
        refused_restore: Option<&crate::ports::PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store
            .complete_wake(attempt, outcome, refused_restore, budget)
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
    fn poke_candidates(&self, limit: u16, budget: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        self.store.poke_candidates(limit, budget)
    }
    fn poke_for_wake(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<PokeDue>, ApiError> {
        self.store.poke_for_wake(seat, budget)
    }
    fn reserve_poke(
        &self,
        due: &PokeDue,
        budget: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.reserve_poke(due, budget)
    }
    fn complete_poke(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: WakeOutcome,
        receipts: &[PokeReceipt],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let _turn = self.writer.enter_background(budget, self.store.clock())?;
        self.store.complete_poke(attempt, outcome, receipts, budget)
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
    /// Whether a store callback of the current drive failed (so the failure
    /// is already reported per phase).
    pub fn callback_failed(&self) -> bool {
        self.callback_failed
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn observe_drive(&self, result: &Result<crate::scheduler::WakeDriveOutcome, ApiError>) {
        use crate::scheduler::SubmissionVerification::{NotChecked, Unsubmitted};
        if let Ok(outcome) = result {
            for (seat, verification) in &outcome.verification {
                if matches!(verification, NotChecked | Unsubmitted) {
                    self.status.report_wake_verification(seat, *verification);
                }
            }
        }
        if let Err(error) = result {
            self.observe_driver_error("unverified wake driver recovery", error);
        }
    }
    /// Poke driver errors reach worker health like wake driver errors.
    pub fn observe_poke_drive(
        &self,
        result: &Result<crate::scheduler::PokeDriveOutcome, ApiError>,
    ) {
        if let Err(error) = result {
            self.observe_driver_error("unverified poke driver recovery", error);
        }
    }
    fn observe_driver_error(&self, context: &str, error: &ApiError) {
        if !self
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
                        error_diagnostic(context, error),
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
        if let Err(error) = &result {
            self.callback_failed
                .store(true, std::sync::atomic::Ordering::Relaxed);
            // The lane loop leaves a callback failure to this per-phase report
            // (no backoff, no `record_failure`), so it reaches the daemon log
            // here, through the same rate-limited lane log. A cancelled call
            // is a shutdown, not a failure.
            if error.code != ErrorCode::Cancelled {
                self.status.report_to_error_log(Lane::Wakes, error);
            }
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
    fn wake_batch_seats(
        &self,
        after: Option<&crate::protocol::ids::SeatId>,
        limit: u16,
        budget: &CallBudget,
    ) -> Result<Vec<crate::protocol::ids::SeatId>, ApiError> {
        self.observed(
            WakePhase::BatchDiscovery,
            None,
            self.port.wake_batch_seats(after, limit, budget),
        )
    }
    fn clear_wake_batch_if_empty(
        &self,
        seat: &crate::protocol::ids::SeatId,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.observed(
            WakePhase::BatchCleanup,
            Some(seat.as_str().to_owned()),
            self.port.clear_wake_batch_if_empty(seat, budget),
        )
    }
    fn wake_batch_window(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<(crate::protocol::time::UtcMillis, u64)>, ApiError> {
        self.observed(
            WakePhase::BatchWindow,
            Some(candidate.seat.as_str().to_owned()),
            self.port.wake_batch_window(candidate, budget),
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
        refused_restore: Option<&crate::ports::PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.observed(
            WakePhase::Completion,
            Some(attempt.as_str().into()),
            self.port
                .complete_wake(attempt, outcome, refused_restore, budget),
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
    fn poke_candidates(&self, limit: u16, budget: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        self.observed(
            WakePhase::Discovery,
            None,
            self.port.poke_candidates(limit, budget),
        )
    }
    fn poke_for_wake(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<PokeDue>, ApiError> {
        // Advisory: an ordinary wake never fails because of it.
        self.port.poke_for_wake(seat, budget)
    }
    fn reserve_poke(
        &self,
        due: &PokeDue,
        budget: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        self.observed(
            WakePhase::Reservation,
            Some(due.seat.as_str().into()),
            self.port.reserve_poke(due, budget),
        )
    }
    fn complete_poke(
        &self,
        attempt: crate::protocol::ids::WakeAttemptId,
        outcome: WakeOutcome,
        receipts: &[PokeReceipt],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.observed(
            WakePhase::Completion,
            Some(attempt.as_str().into()),
            self.port.complete_poke(attempt, outcome, receipts, budget),
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
    pacer: Arc<Pacer>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
    poke_capabilities: Arc<dyn crate::ports::PokeCapabilitySource>,
    reachability: Arc<HostReachability>,
    mod_channels: Arc<dyn crate::ports::ModChannels>,
) -> std::io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("herdr-wakes".into())
        .spawn(move || {
            let _origin = kicks::enter_lane(Lane::Wakes);
            status.attach_pacer(Arc::clone(&pacer));
            let _guard = status.lane_guard(&cancellation);
            let port = ScheduledStore::new(store, writer);
            let notifier = NativeWakeDispatcher::new(host.as_ref(), &port, port.store.clock());
            let observed = ObservedWakePort::new(&port, &status);
            let scheduler = Scheduler::new(instance, &port, &observed, &notifier, retry, boot)
                .with_poke_capabilities(poke_capabilities.as_ref())
                .with_mod_channels(mod_channels.as_ref());
            let safety_tick = Duration::from_millis(WAKE_SAFETY_TICK_MILLIS);
            // The first pass runs at boot, before the first wait.
            let mut next_due_at: Option<crate::protocol::time::MonoInstant> = None;
            let mut waited = false;
            let mut recoveries = reachability.recoveries();
            while !cancellation.is_cancelled() {
                if waited {
                    let tick = wake_wait(safety_tick, next_due_at, pacer.now());
                    if pacer.wait_blocking(tick) == Wake::Cancelled {
                        break;
                    }
                }
                waited = true;
                next_due_at = None;
                let mut wakes_failed = false;
                // Herdr down (TRUST-POLICY C4): the lane is frozen. No
                // completion retry, recovery, reservation, refusal record or
                // Herdr call; it waits for the observation lane's recovery
                // kick (or a safety tick). A frozen pass is not a failure.
                if reachability.is_down() {
                    status.record_success(port.store.clock().utc_now());
                    continue;
                }
                // Back after an outage: refusal backoff from before it says
                // nothing about the host now, so every due seat goes at once.
                let seen = reachability.recoveries();
                if seen != recoveries {
                    recoveries = seen;
                    let _ = scheduler.clear_wake_refusals();
                }
                loop {
                    // An outage seen mid-drain stops the drain too.
                    if reachability.is_down() {
                        break;
                    }
                    let budget = CallBudget {
                        deadline: crate::protocol::time::MonoInstant(
                            port.store.clock().monotonic_now().0.saturating_add(5_000),
                        ),
                        cancellation: cancellation.clone(),
                    };
                    observed.begin_drive(&budget);
                    let result = scheduler.drive_wakes(&budget);
                    observed.observe_drive(&result);
                    match result {
                        Err(error) => {
                            // A failure raised inside a store callback is already
                            // reported per phase and retried on the next wake;
                            // only an unclassified error backs the lane off.
                            if !observed.callback_failed() && !cancellation.is_cancelled() {
                                pacer.on_failure();
                                status.record_failure(Lane::Wakes, &error);
                            }
                            wakes_failed = true;
                            break;
                        }
                        Ok(outcome) => {
                            pacer.on_success();
                            status.record_success(port.store.clock().utc_now());
                            next_due_at = outcome.next_due_at;
                            if !outcome.has_more || cancellation.is_cancelled() {
                                break;
                            }
                        }
                    }
                }
                if cancellation.is_cancelled() {
                    break;
                }
                // A failed wake pass leaves its retained completion to the
                // wake path's next tick (no back-off for a store-callback
                // failure); the poke drive waits for a clean pass.
                // An outage seen mid-pass freezes the poke drive too (ht-72q).
                if wakes_failed || reachability.is_down() {
                    continue;
                }
                // Spec §10: soft-deadline pokes share the wake dispatcher and
                // its limits, once per pass (a pass runs at least every
                // safety tick); store callback failures are observed per
                // phase and driver errors are recorded in worker health.
                let poke_budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        port.store.clock().monotonic_now().0.saturating_add(5_000),
                    ),
                    cancellation: cancellation.clone(),
                };
                observed.observe_poke_drive(&scheduler.drive_pokes(&poke_budget));
            }
        })
}

/// The wake lane's wait: the safety tick, or less when a seat's retry comes
/// due sooner. A due time that has already passed waits `WAKE_MIN_WAIT_MILLIS`,
/// so stale bookkeeping can never spin the lane.
fn wake_wait(
    safety_tick: Duration,
    next_due_at: Option<crate::protocol::time::MonoInstant>,
    now: crate::protocol::time::MonoInstant,
) -> Duration {
    match next_due_at {
        Some(due) if due.0 > now.0 => safety_tick.min(Duration::from_millis(due.0 - now.0)),
        Some(_) => safety_tick.min(Duration::from_millis(WAKE_MIN_WAIT_MILLIS)),
        None => safety_tick,
    }
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
// Allowed: worker start-up wiring: each argument is a distinct dependency.
#[allow(clippy::too_many_arguments)]
pub fn start_observation_worker(
    identity: Arc<crate::identity::repair::OrdinaryIdentity>,
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
    host_evidence: Arc<crate::service::host_evidence::HostEvidenceStatus>,
    pacer: Arc<Pacer>,
    reachability: Arc<HostReachability>,
) -> std::io::Result<thread::JoinHandle<()>> {
    spawn_observation_loop(
        identity,
        ScheduledStore::new(store, writer),
        cancellation,
        status,
        host_evidence,
        pacer,
        reachability,
    )
}

/// The observation worker loop over its scheduled store port. Every feeder
/// of the lane's `WorkerStatus` (capture and reconciliation) is called here;
/// the production entry point above only supplies the fair-writer port.
/// The lane blocks in its Pacer: the 5 s cadence while healthy, the capped
/// backoff after a failed capture, and no wait while a reconciliation
/// continuation page is pending (pacer spec D4).
fn spawn_observation_loop<S>(
    identity: Arc<crate::identity::repair::OrdinaryIdentity>,
    port: S,
    cancellation: Cancellation,
    status: Arc<WorkerStatus>,
    host_evidence: Arc<crate::service::host_evidence::HostEvidenceStatus>,
    pacer: Arc<Pacer>,
    reachability: Arc<HostReachability>,
) -> std::io::Result<thread::JoinHandle<()>>
where
    S: crate::identity::reconcile::observation_store::ObservationStore + 'static,
{
    thread::Builder::new()
        .name("herdr-observations".into())
        .spawn(move || {
            // Its commits count as the observation origin, never as requests.
            let _origin = kicks::enter_lane(Lane::Observation);
            status.attach_pacer(Arc::clone(&pacer));
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
                        // Health: success-only `last_reconciliation_at` (B3).
                        host_evidence.record_reconcile_page(
                            u64::from(page.transitions_refused),
                            done && matches!(
                                outcome,
                                crate::identity::reconcile::ObservationOutcome::Published(_)
                            ),
                            port.clock().utc_now(),
                        );
                        if let (false, Some(next)) = (done, page.next_after_ordinal) {
                            continuation =
                                Some((outcome, next, Some(page.high_water_ordinal), refused));
                        } else if let crate::identity::reconcile::ObservationOutcome::Published(
                            published,
                        ) = &outcome
                            && !refused
                        {
                            // Durable reconciliation marker (TRUST-POLICY C2):
                            // only a pass with no refused transition records
                            // it. An Err leaves the marker behind; the next
                            // pass retries.
                            let _ = identity.record_reconciliation_pass(&port, published, &budget);
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
                            // Host reachability for the wake lane (ht-72q):
                            // a frozen capture is the outage; any capture
                            // Herdr answered ends it and kicks the wake lane.
                            match &outcome {
                                crate::identity::reconcile::ObservationOutcome::Frozen {
                                    ..
                                } => reachability.mark_down(),
                                crate::identity::reconcile::ObservationOutcome::Superseded => {}
                                _ => reachability.mark_up(),
                            }
                            match &outcome {
                                crate::identity::reconcile::ObservationOutcome::Published(_) => {
                                    reachability.mark_archival_published(port.clock().monotonic_now().0);
                                    host_evidence.record_published();
                                    pacer.on_success();
                                }
                                crate::identity::reconcile::ObservationOutcome::Invalidated {
                                    reason,
                                    cause,
                                    ..
                                } => {
                                    reachability.mark_archival_uncertain();
                                    host_evidence.record_invalidated(*reason, cause.as_ref());
                                    pacer.on_failure();
                                }
                                crate::identity::reconcile::ObservationOutcome::InvalidationRepeated {
                                    reason,
                                    cause,
                                }
                                | crate::identity::reconcile::ObservationOutcome::Frozen {
                                    reason,
                                    cause,
                                } => {
                                    reachability.mark_archival_uncertain();
                                    host_evidence.record_invalidated(*reason, cause.as_ref());
                                    pacer.on_failure();
                                }
                                crate::identity::reconcile::ObservationOutcome::Superseded => {}
                            }
                            // A repeated invalidation arms no continuation (its
                            // marking pass already completed), nor does a frozen
                            // capture (nothing to mark).
                            if !matches!(
                                outcome,
                                crate::identity::reconcile::ObservationOutcome::Superseded
                                    | crate::identity::reconcile::ObservationOutcome::InvalidationRepeated {
                                        ..
                                    }
                                    | crate::identity::reconcile::ObservationOutcome::Frozen { .. }
                            ) {
                                host_evidence.begin_reconcile_pass();
                                continuation = Some((outcome, 0, None, false));
                            }
                        }
                        Ok(None) => {}
                        // No durable outcome: never leave an earlier
                        // publication reported as verified host evidence.
                        Err(error) => {
                            reachability.mark_archival_uncertain();
                            host_evidence.record_capture_failed(&error);
                            pacer.on_failure();
                        }
                    }
                }
                // Re-run immediately while a continuation page is pending;
                // otherwise block in the Pacer (cadence, backoff, cancel).
                if continuation.is_none()
                    && pacer.wait_blocking(Duration::from_millis(5_000)) == Wake::Cancelled
                {
                    break;
                }
            }
        })
}

#[cfg(test)]
#[path = "../../tests/service/worker_health.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/service/retention_lane.rs"]
mod retention_lane_tests;

#[cfg(test)]
#[path = "../../tests/service/observation_lane.rs"]
mod observation_lane_tests;

#[cfg(test)]
#[path = "../../tests/service/admission_observer.rs"]
mod admission_observer_tests;
#[cfg(test)]
#[path = "../../tests/service/admission_reobserve.rs"]
mod admission_reobserve_tests;
