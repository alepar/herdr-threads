use super::*;
use crate::ports::DuePhaseCursor;
use crate::protocol::ids::SeatId;
use crate::protocol::time::{Cancellation, MonoInstant, UtcMillis};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

fn due_progress(
    request: DueScanRequest,
    examined: u16,
    warnings: u16,
    more: bool,
) -> DueScanProgress {
    let mut state = request.state;
    state.next_phase = match state.next_phase {
        DuePhase::Invitations => DuePhase::Receipts,
        DuePhase::Receipts => DuePhase::Invitations,
    };
    DueScanProgress {
        state,
        examined_candidates: examined,
        warnings_added: warnings,
        invitations: if more {
            DuePhaseProgress::More
        } else {
            DuePhaseProgress::Complete
        },
        receipts: DuePhaseProgress::Complete,
        has_more: more,
    }
}

#[derive(Default)]
struct FakeClock {
    utc: AtomicI64,
    mono: AtomicU64,
}
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
struct FakePort {
    clock: FakeClock,
    deadline: AtomicI64,
    due_calls: AtomicU64,
    warnings: AtomicU64,
}
impl DeadlinePort for FakePort {
    crate::no_durable_work!();
    fn clock(&self) -> &dyn Clock {
        &self.clock
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        _budget: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        assert_eq!(request.max_candidates, 100);
        self.due_calls.fetch_add(1, Ordering::SeqCst);
        if self.clock.utc_now().0 >= self.deadline.load(Ordering::SeqCst) {
            self.warnings.fetch_add(1, Ordering::SeqCst);
            Ok(due_progress(request, 1, 1, false))
        } else {
            Ok(due_progress(request, 0, 0, false))
        }
    }
    fn pending_retirement_jobs(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError> {
        Ok(Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        })
    }
    fn advance_retirement(
        &self,
        _: RetirementJobId,
        _: WorkAdmission,
        _: &CallBudget,
    ) -> Result<RetirementProgress, ApiError> {
        unreachable!()
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn tick_uses_store_decision_clock_at_exact_boundary_after_wall_jumps() {
    let port = FakePort::default();
    port.deadline.store(300_000, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 0);
    port.clock.utc.store(299_999, Ordering::SeqCst);
    port.clock.mono.store(1_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 0);
    port.clock.utc.store(300_000, Ordering::SeqCst);
    port.clock.mono.store(2_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 1);
    port.clock.utc.store(-3_600_000, Ordering::SeqCst);
    port.clock.mono.store(3_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 0);
    port.clock.utc.store(3_900_000, Ordering::SeqCst);
    port.clock.mono.store(4_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 1);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 5);
}

#[test]
fn default_and_override_durations_reject_zero_and_overflow() {
    use crate::scheduler::config::{DeadlineDuration, SchedulerTiming};
    let timing = SchedulerTiming::default();
    assert_eq!(timing.invitation.as_millis(), 300_000);
    assert_eq!(timing.receipt.as_millis(), 300_000);
    let frozen = timing.receipt_for_operation(Some(420_000)).unwrap();
    assert_eq!(frozen.as_millis(), 420_000);
    assert_eq!(timing.receipt.as_millis(), 300_000);
    assert!(DeadlineDuration::seconds(0).is_err());
    assert!(DeadlineDuration::seconds(u64::MAX).is_err());
    assert!(DeadlineDuration::millis(0).is_err());
    assert!(DeadlineDuration::millis(u64::MAX).is_err());
    assert_eq!(
        SchedulerTiming::new(400_000, 600_000)
            .unwrap()
            .invitation
            .as_millis(),
        400_000
    );
    assert!(
        frozen
            .deadline_after(UtcMillis(i64::MAX - 419_999))
            .is_err()
    );
    assert_eq!(
        frozen.deadline_after(UtcMillis(1)).unwrap(),
        UtcMillis(420_001)
    );
}

struct RetirementPort {
    clock: FakeClock,
    jobs: Mutex<Vec<(RetirementJobId, u8)>>,
    fail_once: Mutex<Option<RetirementJobId>>,
    due_calls: AtomicU64,
    advance_calls: AtomicU64,
    discover_fail_once: AtomicU64,
    stale_cursor_once: AtomicU64,
    fail_b_persistently: AtomicU64,
    retained_error_b: AtomicU64,
}
impl RetirementPort {
    fn new() -> Self {
        Self {
            clock: FakeClock::default(),
            jobs: Mutex::new(vec![
                (RetirementJobId::new("a"), 0),
                (RetirementJobId::new("b"), 0),
                (RetirementJobId::new("c"), 0),
            ]),
            fail_once: Mutex::new(Some(RetirementJobId::new("b"))),
            due_calls: AtomicU64::new(0),
            advance_calls: AtomicU64::new(0),
            discover_fail_once: AtomicU64::new(0),
            stale_cursor_once: AtomicU64::new(0),
            fail_b_persistently: AtomicU64::new(0),
            retained_error_b: AtomicU64::new(0),
        }
    }
}
impl DeadlinePort for RetirementPort {
    crate::no_durable_work!();
    fn clock(&self) -> &dyn Clock {
        &self.clock
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        _: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        assert_eq!(request.max_candidates, 100);
        self.due_calls.fetch_add(1, Ordering::SeqCst);
        Ok(due_progress(request, 0, 0, false))
    }
    fn pending_retirement_jobs(
        &self,
        page: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError> {
        if page.cursor.is_some() && self.stale_cursor_once.swap(0, Ordering::SeqCst) == 1 {
            return Err(ApiError::cursor_stale("changed scope"));
        }
        if self.discover_fail_once.swap(0, Ordering::SeqCst) == 1 {
            return Err(ApiError::store_busy("discovery failed"));
        }
        assert_eq!(page.limit, 1);
        let after = page
            .cursor
            .as_deref()
            .map(|s| s.parse::<usize>().unwrap())
            .unwrap_or(0);
        let jobs = self.jobs.lock().unwrap();
        let next = (after..jobs.len()).find(|&i| jobs[i].1 < 2);
        let items = next
            .map(|i| RetirementStatus {
                job: jobs[i].0.clone(),
                seat: SeatId::new(format!("s{i}")),
                effective_retired: true,
                retired_at: UtcMillis(5),
                cleanup_state: crate::protocol::results::CleanupState::Pending,
                warning_history_complete: false,
                phase: "warnings".into(),
                processed_units: jobs[i].1 as u64,
                remaining_estimate: Some(2 - jobs[i].1 as u64),
                last_error: None,
            })
            .into_iter()
            .collect();
        let next_cursor = next.and_then(|i| {
            ((i + 1)..jobs.len())
                .find(|&j| jobs[j].1 < 2)
                .map(|_| (i + 1).to_string())
        });
        Ok(Page {
            items,
            next_cursor,
            next_argv: None,
            high_water_ordinal: 3,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        })
    }
    fn advance_retirement(
        &self,
        job: RetirementJobId,
        admission: WorkAdmission,
        _: &CallBudget,
    ) -> Result<RetirementProgress, ApiError> {
        assert_eq!(admission, WorkAdmission::Background);
        self.advance_calls.fetch_add(1, Ordering::SeqCst);
        if job == RetirementJobId::new("b") && self.fail_b_persistently.load(Ordering::SeqCst) == 1
        {
            return Err(ApiError::store_busy("persistent failure"));
        }
        if job == RetirementJobId::new("b") && self.retained_error_b.load(Ordering::SeqCst) == 1 {
            // An idle quantum (no unit ran) reports the retained failure of an
            // earlier failed quantum; committed progress would have cleared it.
            return Ok(RetirementProgress {
                job,
                processed_this_turn: 0,
                processed_total: 0,
                complete: false,
                warning_history_complete: false,
                last_error: Some(
                    crate::protocol::results::BoundedError::parse("SQLite: retained failure")
                        .unwrap(),
                ),
            });
        }
        let mut fail = self.fail_once.lock().unwrap();
        if fail.as_ref() == Some(&job) {
            *fail = None;
            return Err(ApiError::store_busy("temporary"));
        }
        let mut jobs = self.jobs.lock().unwrap();
        let state = &mut jobs.iter_mut().find(|(id, _)| id == &job).unwrap().1;
        *state += 1;
        Ok(RetirementProgress {
            job,
            processed_this_turn: 1,
            processed_total: *state as u64,
            complete: *state == 2,
            warning_history_complete: *state == 2,
            last_error: None,
        })
    }
}

#[test]
fn persistent_retirement_failure_allows_healthy_jobs_and_due_work_under_foreground_load() {
    let port = RetirementPort::new();
    port.fail_b_persistently.store(1, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let mut observed_errors = 0;
    for tick in 0..40 {
        // Model a foreground turn consuming 100 ms before each bounded
        // background opportunity. Actual writer admission belongs to service.
        let now = tick * 1_000 + 100;
        port.clock.mono.store(now, Ordering::SeqCst);
        let turn_budget = CallBudget {
            deadline: MonoInstant(now + 5),
            cancellation: Cancellation::default(),
        };
        if driver
            .drive(&turn_budget)
            .unwrap()
            .retirement_error
            .is_some()
        {
            observed_errors += 1;
        }
    }
    let jobs = port.jobs.lock().unwrap();
    assert_eq!(jobs[0].1, 2);
    assert_eq!(jobs[1].1, 0);
    assert_eq!(jobs[2].1, 2);
    assert!(observed_errors >= 2);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 40);
    assert!(port.advance_calls.load(Ordering::SeqCst) <= 12); // five-second retry bound
}

#[test]
fn default_tick_is_the_five_second_safety_tick_and_a_committed_change_bypasses_it() {
    assert_eq!(TICK_MILLIS, 5_000);
    let port = FakePort::default();
    let mut driver = DeadlineDriver::new(&port);
    assert!(driver.drive(&budget()).unwrap().ticked);
    port.clock.mono.store(4_999, Ordering::SeqCst);
    assert!(
        !driver.drive(&budget()).unwrap().ticked,
        "a call inside the 5 s gate is skipped"
    );
    driver.after_committed_change();
    assert!(
        driver.drive(&budget()).unwrap().ticked,
        "a committed change reopens the gate at once"
    );
    // The reopened pass re-armed the gate from 4_999.
    port.clock.mono.store(9_998, Ordering::SeqCst);
    assert!(!driver.drive(&budget()).unwrap().ticked);
    port.clock.mono.store(9_999, Ordering::SeqCst);
    assert!(driver.drive(&budget()).unwrap().ticked);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 3);
}

#[test]
fn progressed_with_more_needs_unfinished_progress() {
    let mut outcome = DriveOutcome::default();
    assert!(!outcome.progressed_with_more());
    outcome.work_progressed = true;
    outcome.work_complete = true;
    assert!(
        !outcome.progressed_with_more(),
        "finished work needs no rerun"
    );
    outcome.work_complete = false;
    assert!(outcome.progressed_with_more());
    outcome.work_progressed = false;
    outcome.retirement_progressed = true;
    assert!(outcome.progressed_with_more());
    outcome.retirement_complete = true;
    assert!(!outcome.progressed_with_more());
}

#[test]
fn retirement_jobs_rotate_retry_and_resume_without_blocking_due_scans() {
    let port = RetirementPort::new();
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let first = driver.drive(&budget()).unwrap();
    assert_eq!(first.retirement_job, Some(RetirementJobId::new("a")));
    port.clock.mono.store(1_000, Ordering::SeqCst);
    let second = driver.drive(&budget()).unwrap();
    assert_eq!(second.retirement_job, Some(RetirementJobId::new("b")));
    assert_eq!(second.retirement_error.unwrap().code, ErrorCode::StoreBusy);
    port.clock.mono.store(2_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().retirement_job, None);
    drop(driver); // daemon restart; store retains partial progress and failed job
    let mut restarted = DeadlineDriver::new(&port).with_tick_millis(1_000);
    for turn in 3..20 {
        port.clock.mono.store(turn * 1_000, Ordering::SeqCst);
        restarted.drive(&budget()).unwrap();
    }
    assert!(
        port.jobs
            .lock()
            .unwrap()
            .iter()
            .all(|(_, processed)| *processed == 2)
    );
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 20);
    assert_eq!(port.advance_calls.load(Ordering::SeqCst), 7);
}

#[test]
fn discovery_failure_and_backoff_do_not_block_due_ticks() {
    let port = RetirementPort::new();
    port.discover_fail_once.store(1, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let first = driver.drive(&budget()).unwrap();
    assert_eq!(first.retirement_error.unwrap().code, ErrorCode::StoreBusy);
    assert_eq!(first.due_warnings_added, 0);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 1);
    for tick in 1..5 {
        port.clock.mono.store(tick * 1_000, Ordering::SeqCst);
        driver.drive(&budget()).unwrap();
    }
    assert_eq!(port.advance_calls.load(Ordering::SeqCst), 0);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 5);
    port.clock.mono.store(5_000, Ordering::SeqCst);
    driver.drive(&budget()).unwrap();
    assert_eq!(port.advance_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn full_due_batch_continues_without_waiting_for_next_tick() {
    struct FullBatch(FakeClock, AtomicU64);
    impl DeadlinePort for FullBatch {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            assert_eq!(request.max_candidates, 100);
            let n = self.1.fetch_add(1, Ordering::SeqCst);
            Ok(due_progress(
                request,
                if n == 0 { 100 } else { 1 },
                1,
                n == 0,
            ))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
    }
    let port = FullBatch(FakeClock::default(), AtomicU64::new(0));
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    assert!(driver.drive(&budget()).unwrap().due_continuation);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 1);
    assert_eq!(port.1.load(Ordering::SeqCst), 2);
}

#[test]
fn zero_warning_progress_retains_exact_due_cursor_and_continues() {
    struct CursorPort(FakeClock, AtomicU64);
    impl DeadlinePort for CursorPort {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            assert_eq!(request.max_candidates, 100);
            let call = self.1.fetch_add(1, Ordering::SeqCst);
            if call > 0 {
                assert_eq!(
                    request.state.invitations.as_ref().unwrap().after_ordinal,
                    call * 100
                );
            }
            let mut progress = due_progress(request, 100, 0, call < 2);
            progress.invitations = if call < 2 {
                DuePhaseProgress::More
            } else {
                DuePhaseProgress::Complete
            };
            progress.state.invitations = if call < 2 {
                Some(DuePhaseCursor {
                    high_water_ordinal: 300,
                    after_deadline: Some(UtcMillis(300_000)),
                    after_ordinal: (call + 1) * 100,
                    receipt_sparse: None,
                })
            } else {
                None
            };
            Ok(progress)
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
    }
    let port = CursorPort(FakeClock::default(), AtomicU64::new(0));
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    for expected_more in [true, true, false] {
        let outcome = driver.drive(&budget()).unwrap();
        assert_eq!(outcome.due_examined_candidates, 100);
        assert_eq!(outcome.due_warnings_added, 0);
        assert_eq!(outcome.due_continuation, expected_more);
    }
    assert_eq!(port.1.load(Ordering::SeqCst), 3);
}

#[test]
fn failed_invitation_phase_backs_off_while_receipts_keep_scanning() {
    struct PhasePort {
        clock: FakeClock,
        requests: Mutex<Vec<DueScanRequest>>,
    }
    impl DeadlinePort for PhasePort {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.clock
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            let mut requests = self.requests.lock().unwrap();
            let call = requests.len();
            requests.push(request.clone());
            let mut progress = due_progress(request, 1, 0, true);
            progress.invitations = if call == 0 {
                DuePhaseProgress::Failed(
                    ErrorCode::StoreBusy,
                    crate::protocol::results::BoundedError::parse("blocked").unwrap(),
                )
            } else if call == 1 {
                DuePhaseProgress::Skipped
            } else {
                DuePhaseProgress::Complete
            };
            progress.receipts = DuePhaseProgress::More;
            progress.state.receipts = Some(DuePhaseCursor {
                high_water_ordinal: 20,
                after_deadline: Some(UtcMillis(10)),
                after_ordinal: (call + 1) as u64,
                receipt_sparse: None,
            });
            Ok(progress)
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
    }
    let port = PhasePort {
        clock: FakeClock::default(),
        requests: Mutex::new(vec![]),
    };
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    driver.drive(&budget()).unwrap();
    driver.drive(&budget()).unwrap();
    port.clock.mono.store(5_000, Ordering::SeqCst);
    driver.drive(&budget()).unwrap();
    let requests = port.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.run_invitations, r.run_receipts))
            .collect::<Vec<_>>(),
        vec![(true, true), (false, true), (true, true)]
    );
    assert_eq!(
        requests[1].state.receipts.as_ref().unwrap().after_ordinal,
        1
    );
    assert_eq!(
        requests[2].state.receipts.as_ref().unwrap().after_ordinal,
        2
    );
}

#[test]
fn durable_work_jobs_get_one_bounded_quantum_and_rotate_with_due_and_retirement() {
    use crate::ports::{DurableWorkAdmission, WorkCandidate, WorkKind, WorkProgress};
    struct WorkPort(FakeClock, AtomicU64, AtomicU64);
    impl DeadlinePort for WorkPort {
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
        fn pending_work(
            &self,
            page: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            assert_eq!(page.limit, 1);
            let index = page
                .cursor
                .as_deref()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap();
            Ok(Page {
                items: vec![WorkCandidate {
                    id: format!("w{index}"),
                    kind: WorkKind::WarningAttribution,
                    position: 0,
                    high_water: 1,
                    has_more: false,
                }],
                next_cursor: Some(((index + 1) % 2).to_string()),
                next_argv: None,
                high_water_ordinal: 2,
                scope_revision: None,
                has_more: true,
                stop_reason: crate::protocol::pagination::StopReason::Rows,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            job: &str,
            admission: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            assert_eq!(admission.max_units, 16);
            assert!(job == "w0" || job == "w1");
            self.2.fetch_add(1, Ordering::SeqCst);
            Ok(WorkProgress {
                completed_units: 1,
                processed_this_turn: 1,
                has_more: false,
                next_position: 1,
                last_error: None,
            })
        }
    }
    let port = WorkPort(FakeClock::default(), AtomicU64::new(0), AtomicU64::new(0));
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let mut jobs = Vec::new();
    for tick in 0..4 {
        port.0.mono.store(tick * 1_000, Ordering::SeqCst);
        jobs.push(driver.drive(&budget()).unwrap().work_job);
    }
    assert_eq!(
        jobs,
        vec![
            Some("w0".into()),
            Some("w1".into()),
            Some("w0".into()),
            Some("w1".into())
        ]
    );
    assert_eq!(port.1.load(Ordering::SeqCst), 4);
    assert_eq!(port.2.load(Ordering::SeqCst), 4);
}

#[test]
fn failing_work_job_does_not_pause_healthy_materialization() {
    use crate::ports::{DurableWorkAdmission, WorkCandidate, WorkKind, WorkProgress};
    struct MixedWork(FakeClock, AtomicU64);
    impl DeadlinePort for MixedWork {
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
        fn pending_work(
            &self,
            page: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            let index = page
                .cursor
                .as_deref()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap();
            let kind = WorkKind::WarningAttribution;
            Ok(Page {
                items: vec![WorkCandidate {
                    id: format!("w{index}"),
                    kind,
                    position: 0,
                    high_water: 1,
                    has_more: false,
                }],
                next_cursor: Some(((index + 1) % 2).to_string()),
                next_argv: None,
                high_water_ordinal: 2,
                scope_revision: None,
                has_more: true,
                stop_reason: crate::protocol::pagination::StopReason::Rows,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            job: &str,
            _: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            if job == "w0" {
                return Err(ApiError::store_busy("stuck"));
            }
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(WorkProgress {
                completed_units: 1,
                processed_this_turn: 1,
                has_more: false,
                next_position: 1,
                last_error: None,
            })
        }
    }
    let port = MixedWork(FakeClock::default(), AtomicU64::new(0));
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    for tick in 0..6 {
        port.0.mono.store(tick * 1_000, Ordering::SeqCst);
        driver.drive(&budget()).unwrap();
    }
    assert!(port.1.load(Ordering::SeqCst) >= 2);
}

#[test]
fn work_retry_cache_is_bounded_and_keeps_discovery_moving() {
    use crate::ports::{DurableWorkAdmission, WorkCandidate, WorkKind, WorkProgress};
    struct ManyFailures(FakeClock, AtomicU64, AtomicU64);
    impl DeadlinePort for ManyFailures {
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
        fn pending_work(
            &self,
            page: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            let index = page
                .cursor
                .as_deref()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap();
            Ok(Page {
                items: vec![WorkCandidate {
                    id: format!("w{index}"),
                    kind: WorkKind::WarningAttribution,
                    position: 0,
                    high_water: 1,
                    has_more: false,
                }],
                next_cursor: Some(((index + 1) % 66).to_string()),
                next_argv: None,
                high_water_ordinal: 66,
                scope_revision: None,
                has_more: true,
                stop_reason: crate::protocol::pagination::StopReason::Rows,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            job: &str,
            _: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            if job == "w65" {
                self.2.fetch_add(1, Ordering::SeqCst);
                return Ok(WorkProgress {
                    completed_units: 1,
                    processed_this_turn: 1,
                    has_more: false,
                    next_position: 1,
                    last_error: None,
                });
            }
            self.1.fetch_add(1, Ordering::SeqCst);
            Err(ApiError::store_busy("failed"))
        }
    }
    let port = ManyFailures(FakeClock::default(), AtomicU64::new(0), AtomicU64::new(0));
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    for _ in 0..67 {
        driver.after_committed_change();
        driver.drive(&budget()).unwrap();
    }
    assert_eq!(port.2.load(Ordering::SeqCst), 1); // healthy job was reached
    assert_eq!(port.1.load(Ordering::SeqCst), 66); // oldest failure was evicted and retried
    assert_eq!(driver.work_retry_at.len(), MAX_WORK_RETRY_ENTRIES);
}

#[test]
fn committed_work_prefix_error_is_visible_and_retries_at_exact_cooldown() {
    use crate::ports::{DurableWorkAdmission, WorkCandidate, WorkKind, WorkProgress};
    struct PrefixWork {
        clock: FakeClock,
        prefix: u64,
        position: Mutex<u64>,
        failed_job_positions: Mutex<Vec<u64>>,
        healthy_calls: AtomicU64,
        due_calls: AtomicU64,
    }
    impl DeadlinePort for PrefixWork {
        fn clock(&self) -> &dyn Clock {
            &self.clock
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            self.due_calls.fetch_add(1, Ordering::SeqCst);
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
        fn pending_work(
            &self,
            page: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            let sibling = page.cursor.as_deref() == Some("sibling");
            let id = if sibling { "healthy" } else { "failed" };
            let position = if sibling {
                0
            } else {
                *self.position.lock().unwrap()
            };
            Ok(Page {
                items: vec![WorkCandidate {
                    id: id.into(),
                    kind: WorkKind::WarningAttribution,
                    position,
                    high_water: 5,
                    has_more: true,
                }],
                next_cursor: Some(if sibling { "failed" } else { "sibling" }.into()),
                next_argv: None,
                high_water_ordinal: 2,
                scope_revision: None,
                has_more: true,
                stop_reason: crate::protocol::pagination::StopReason::Rows,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            job: &str,
            admission: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            assert_eq!(admission.max_units, 16);
            if job == "healthy" {
                self.healthy_calls.fetch_add(1, Ordering::SeqCst);
                return Ok(WorkProgress {
                    completed_units: 1,
                    processed_this_turn: 1,
                    has_more: false,
                    next_position: 1,
                    last_error: None,
                });
            }
            let mut position = self.position.lock().unwrap();
            let mut visits = self.failed_job_positions.lock().unwrap();
            let first_visit = visits.is_empty();
            visits.push(*position);
            if first_visit {
                *position = self.prefix;
                return Ok(WorkProgress {
                    completed_units: self.prefix,
                    processed_this_turn: 1,
                    has_more: true,
                    next_position: *position,
                    last_error: Some("x".repeat(1_024)),
                });
            }
            *position = 5;
            Ok(WorkProgress {
                completed_units: 5,
                processed_this_turn: 5,
                has_more: false,
                next_position: 5,
                last_error: None,
            })
        }
    }
    for prefix in [0, 2] {
        let port = PrefixWork {
            clock: FakeClock::default(),
            prefix,
            position: Mutex::new(0),
            failed_job_positions: Mutex::new(vec![]),
            healthy_calls: AtomicU64::new(0),
            due_calls: AtomicU64::new(0),
        };
        let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
        let first = driver.drive(&budget()).unwrap();
        assert_eq!(first.work_job.as_deref(), Some("failed"));
        let error = first
            .work_error
            .expect("successful result reports failed quantum");
        assert_eq!(error.code, ErrorCode::StoreBusy);
        assert_eq!(error.detail.len(), 512);
        assert!(error.restart_argv.is_none());
        driver.after_committed_change();
        driver.drive(&budget()).unwrap(); // healthy sibling
        port.clock.mono.store(4_999, Ordering::SeqCst);
        driver.after_committed_change();
        assert_eq!(driver.drive(&budget()).unwrap().work_job, None); // failed job still cooling
        driver.after_committed_change();
        driver.drive(&budget()).unwrap(); // healthy sibling still progresses
        port.clock.mono.store(5_000, Ordering::SeqCst);
        driver.after_committed_change();
        let recovered = driver.drive(&budget()).unwrap();
        assert_eq!(recovered.work_job.as_deref(), Some("failed"));
        assert!(recovered.work_error.is_none());
        assert_eq!(*port.failed_job_positions.lock().unwrap(), vec![0, prefix]);
        assert_eq!(port.healthy_calls.load(Ordering::SeqCst), 2);
        assert_eq!(port.due_calls.load(Ordering::SeqCst), 5);
    }
}

#[test]
fn stale_retirement_cursor_restarts_discovery_after_backoff() {
    let port = RetirementPort::new();
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    driver.drive(&budget()).unwrap();
    port.stale_cursor_once.store(1, Ordering::SeqCst);
    port.clock.mono.store(1_000, Ordering::SeqCst);
    assert_eq!(
        driver
            .drive(&budget())
            .unwrap()
            .retirement_error
            .unwrap()
            .code,
        ErrorCode::CursorStale
    );
    port.clock.mono.store(6_000, Ordering::SeqCst);
    assert_eq!(
        driver.drive(&budget()).unwrap().retirement_job,
        Some(RetirementJobId::new("a"))
    );
}

#[test]
fn committed_change_requests_immediate_scan_between_timer_ticks() {
    let port = FakePort::default();
    port.deadline.store(1, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    driver.drive(&budget()).unwrap();
    port.clock.utc.store(1, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 0);
    driver.after_committed_change();
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 1);
    assert_eq!(port.due_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn due_gets_first_turn_after_retirement_exhausts_shared_budget() {
    struct BudgetPort(FakeClock, AtomicU64, AtomicU64, AtomicU64);
    impl DeadlinePort for BudgetPort {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.0
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            budget: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            self.1.fetch_add(1, Ordering::SeqCst);
            if budget.is_exhausted(&self.0) {
                return Err(ApiError::deadline_exceeded("spent"));
            }
            self.3.fetch_add(1, Ordering::SeqCst);
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            self.2.fetch_add(1, Ordering::SeqCst);
            self.0.mono.fetch_add(10, Ordering::SeqCst);
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
    }
    let port = BudgetPort(
        FakeClock::default(),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    );
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let short = CallBudget {
        deadline: MonoInstant(5),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        driver.drive(&short).unwrap_err().code,
        ErrorCode::DeadlineExceeded
    );
    port.0.mono.store(1_000, Ordering::SeqCst);
    let second = CallBudget {
        deadline: MonoInstant(1_005),
        cancellation: Cancellation::default(),
    };
    driver.drive(&second).unwrap();
    assert_eq!(port.1.load(Ordering::SeqCst), 2);
    assert_eq!(port.2.load(Ordering::SeqCst), 2);
    assert_eq!(port.3.load(Ordering::SeqCst), 1);
}

#[test]
fn long_suspend_scans_on_resume_without_a_utc_or_restart_guard() {
    let port = FakePort::default();
    port.deadline.store(300_000, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    driver.drive(&budget()).unwrap();
    // A monotonic clock that includes suspend; a clock that excludes it is
    // covered by the next ordinary one-second tick.
    port.clock.mono.store(86_400_000, Ordering::SeqCst);
    port.clock.utc.store(86_400_000, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().due_warnings_added, 1);
    drop(driver);
    let mut restarted = DeadlineDriver::new(&port).with_tick_millis(1_000);
    assert_eq!(restarted.drive(&budget()).unwrap().due_warnings_added, 1);
}

#[test]
fn pre_due_retirement_is_driven_before_post_suspend_due_scan() {
    struct CutoverPort {
        clock: FakeClock,
        retired_at: AtomicI64,
        cleanup_complete: AtomicU64,
        due_warnings: AtomicU64,
    }
    impl DeadlinePort for CutoverPort {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.clock
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            // A previously committed terminal fence excludes this obligation.
            if self.clock.utc_now().0 >= 300_000 && self.retired_at.load(Ordering::SeqCst) == 0 {
                self.due_warnings.fetch_add(1, Ordering::SeqCst);
                return Ok(due_progress(request, 1, 1, false));
            }
            Ok(due_progress(request, 0, 0, false))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            let items = if self.cleanup_complete.load(Ordering::SeqCst) == 0 {
                vec![RetirementStatus {
                    job: RetirementJobId::new("fence"),
                    seat: SeatId::new("seat"),
                    effective_retired: true,
                    retired_at: UtcMillis(100_000),
                    cleanup_state: crate::protocol::results::CleanupState::Pending,
                    warning_history_complete: false,
                    phase: "warnings".into(),
                    processed_units: 0,
                    remaining_estimate: Some(1),
                    last_error: None,
                }]
            } else {
                vec![]
            };
            Ok(Page {
                items,
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 1,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            job: RetirementJobId,
            admission: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            assert_eq!(admission, WorkAdmission::Background);
            self.cleanup_complete.store(1, Ordering::SeqCst);
            Ok(RetirementProgress {
                job,
                processed_this_turn: 1,
                processed_total: 1,
                complete: true,
                warning_history_complete: true,
                last_error: None,
            })
        }
    }
    let port = CutoverPort {
        clock: FakeClock::default(),
        retired_at: AtomicI64::new(100_000),
        cleanup_complete: AtomicU64::new(0),
        due_warnings: AtomicU64::new(0),
    };
    port.clock.utc.store(86_400_000, Ordering::SeqCst);
    port.clock.mono.store(86_400_000, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let outcome = driver.drive(&budget()).unwrap();
    assert_eq!(outcome.retirement_job, Some(RetirementJobId::new("fence")));
    assert!(outcome.retirement_complete);
    assert_eq!(port.cleanup_complete.load(Ordering::SeqCst), 1);
    assert_eq!(port.due_warnings.load(Ordering::SeqCst), 0);
}

#[test]
fn logical_decision_uses_store_time_across_pauses() {
    struct PausePort {
        clock: FakeClock,
        pause_before: bool,
    }
    impl DeadlinePort for PausePort {
        crate::no_durable_work!();
        fn clock(&self) -> &dyn Clock {
            &self.clock
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            if self.pause_before {
                self.clock.utc.store(300_000, Ordering::SeqCst);
            }
            let decision_utc = self.clock.utc_now().0;
            if !self.pause_before {
                self.clock.utc.store(300_000, Ordering::SeqCst);
            }
            Ok(due_progress(
                request,
                u16::from(decision_utc >= 300_000),
                u16::from(decision_utc >= 300_000),
                false,
            ))
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Ok(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: crate::protocol::pagination::StopReason::Complete,
                consistency: crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            unreachable!()
        }
    }
    for (pause_before, expected) in [(true, 1), (false, 0)] {
        let port = PausePort {
            clock: FakeClock::default(),
            pause_before,
        };
        port.clock.utc.store(299_999, Ordering::SeqCst);
        let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
        assert_eq!(
            driver.drive(&budget()).unwrap().due_warnings_added,
            expected
        );
        assert_eq!(port.clock.utc_now(), UtcMillis(300_000));
    }
}

#[test]
fn retained_retirement_quantum_error_is_reported_and_backed_off() {
    let port = RetirementPort::new();
    *port.fail_once.lock().unwrap() = None;
    port.retained_error_b.store(1, Ordering::SeqCst);
    let mut driver = DeadlineDriver::new(&port).with_tick_millis(1_000);
    let first = driver.drive(&budget()).unwrap();
    assert_eq!(first.retirement_job, Some(RetirementJobId::new("a")));
    assert!(first.retirement_error.is_none());
    port.clock.mono.store(1_000, Ordering::SeqCst);
    let second = driver.drive(&budget()).unwrap();
    assert_eq!(second.retirement_job, Some(RetirementJobId::new("b")));
    assert!(!second.retirement_complete);
    let error = second.retirement_error.unwrap();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert_eq!(error.detail, "SQLite: retained failure");
    let calls = port.advance_calls.load(Ordering::SeqCst);
    port.clock.mono.store(1_001, Ordering::SeqCst);
    assert_eq!(driver.drive(&budget()).unwrap().retirement_job, None);
    assert_eq!(port.advance_calls.load(Ordering::SeqCst), calls);
}
