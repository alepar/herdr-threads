use super::{Scheduler, WakePort, WakeRunner};
use crate::{
    notification::dispatch::{
        DispatchError, DispatchState, NativeWakeDispatcher, ReservationCheck,
    },
    notification::policy::{DurableRetry, RetryConfig},
    ports::StorePort,
    ports::{DuePhaseProgress, DueScanProgress, DueScanRequest, RetirementProgress, WorkAdmission},
    ports::{
        EvidenceKind, ExecutionEvidence, HostCallContext, HostObservation, HostPort, HostSnapshot,
        HostUiState, IncarnationEvidence, LogicalAttentionFrontier, LogicalPublicationKey,
        NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, NativeOccupant,
        NotificationPort, ObservationProvenance, PriorLadder, PromptOutcome, RefusalCause,
        ReservedWakeAuthority, SafeWakeTarget, StructuralOccupancy, WakeAttentionWitness,
        WakeCandidate, WakeOutcome, WakeRecoveryCandidate, WakeRecoveryOutcome,
        WakeRecoveryRequest, WakeReservation, WarningOfferFrontier,
    },
    protocol::{
        authority::Harness,
        ids::{
            ExecutionId, HostBootId, HostCallId, HostTargetId, NativeSessionId, RetirementJobId,
            SeatId, TerminalId, WakeAttemptId,
        },
        pagination::{Consistency, Page, PageRequest, StopReason},
        results::{ApiError, ErrorCode, RetirementStatus},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::{fair_writer::FairWriter, workers::ScheduledStore},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
// Child module so the contract tests reuse this file's private fixtures.
#[path = "wake_outcome_contract.rs"]
mod wake_outcome_contract;
#[path = "wake_outcome_seam.rs"]
mod wake_outcome_seam;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
};

fn daemon_boot() -> uuid::Uuid {
    uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap()
}
fn old_daemon_boot() -> uuid::Uuid {
    uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap()
}
fn empty_recovery_page() -> Page<WakeRecoveryCandidate> {
    Page {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

fn fresh() -> DurableRetry {
    DurableRetry {
        retry_step: 0,
        minimum_delay_ms: 0,
        effective_delay_ms: 0,
        ever_reserved: false,
    }
}

#[test]
fn four_global_slots_and_one_per_seat_release_only_matching_attempt() {
    let boot = daemon_boot();
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), boot);
    for n in 0..5 {
        state
            .restore(SeatId::new(format!("seat-{n}")), fresh())
            .unwrap();
    }
    for n in 0..4 {
        state
            .reserved(
                SeatId::new(format!("seat-{n}")),
                WakeAttemptId::new(format!("attempt-{n}")),
                boot,
                MonoInstant(0),
            )
            .unwrap();
    }
    assert_eq!(state.active_count(), 4);
    assert!(!state.can_reserve(&SeatId::new("seat-4"), MonoInstant(0)));
    assert!(!state.can_reserve(&SeatId::new("seat-0"), MonoInstant(0)));
    assert!(
        !state
            .finish(
                &SeatId::new("seat-0"),
                &WakeAttemptId::new("attempt-0"),
                &old_daemon_boot(),
                MonoInstant(2_000)
            )
            .unwrap()
    );
    assert_eq!(state.active_count(), 4);
    assert!(
        state
            .finish(
                &SeatId::new("seat-0"),
                &WakeAttemptId::new("attempt-0"),
                &boot,
                MonoInstant(2_000)
            )
            .unwrap()
    );
    assert_eq!(state.active_count(), 3);
    assert!(state.can_reserve(&SeatId::new("seat-4"), MonoInstant(2_000)));
    assert!(!state.can_reserve(&SeatId::new("seat-0"), MonoInstant(31_999)));
    assert!(state.can_reserve(&SeatId::new("seat-0"), MonoInstant(32_000)));
}

#[test]
fn repeated_candidate_discovery_retains_completion_anchor_and_attention_floor() {
    let seat = SeatId::new("seat");
    let boot = daemon_boot();
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), boot);
    let attempt = WakeAttemptId::new("attempt");
    state.restore(seat.clone(), fresh()).unwrap();
    let durable = state
        .reserved(seat.clone(), attempt.clone(), boot, MonoInstant(0))
        .unwrap();
    assert!(
        state
            .finish(&seat, &attempt, &boot, MonoInstant(2_000))
            .unwrap()
    );
    state.restore(seat.clone(), durable).unwrap();
    state.new_attention(&seat).unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(31_999)));
    assert!(state.can_reserve(&seat, MonoInstant(32_000)));
}

#[test]
fn logical_frontier_advances_once_during_inflight_attempt_and_survives_completion() {
    let seat = SeatId::new("seat");
    let boot = daemon_boot();
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), boot);
    state.restore(seat.clone(), fresh()).unwrap();
    let initial = LogicalAttentionFrontier {
        invitation: Some(LogicalPublicationKey {
            decision_seq: 7,
            event_offset: 0,
        }),
        ..Default::default()
    };
    state
        .observe_frontier(&seat, Default::default(), initial)
        .unwrap();
    let attempt = WakeAttemptId::new("attempt");
    state
        .reserved(seat.clone(), attempt.clone(), boot, MonoInstant(0))
        .unwrap();
    let newer = LogicalAttentionFrontier {
        invitation: Some(LogicalPublicationKey {
            decision_seq: 8,
            event_offset: 0,
        }),
        ..initial
    };
    state.observe_frontier(&seat, initial, newer).unwrap();
    state.observe_frontier(&seat, initial, newer).unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(60_000)));
    state
        .finish(&seat, &attempt, &boot, MonoInstant(2_000))
        .unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(31_999)));
    assert!(state.can_reserve(&seat, MonoInstant(32_000)));
}

#[test]
fn newer_persisted_reservation_baseline_cannot_be_mistaken_for_new_attention() {
    let seat = SeatId::new("seat");
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), daemon_boot());
    state
        .restore(
            seat.clone(),
            DurableRetry {
                retry_step: 1,
                minimum_delay_ms: 30_000,
                effective_delay_ms: 60_000,
                ever_reserved: true,
            },
        )
        .unwrap();
    let first = LogicalAttentionFrontier {
        invitation: Some(LogicalPublicationKey {
            decision_seq: 1,
            event_offset: 0,
        }),
        ..Default::default()
    };
    let next = LogicalAttentionFrontier {
        invitation: Some(LogicalPublicationKey {
            decision_seq: 2,
            event_offset: 0,
        }),
        ..first
    };
    state.observe_frontier(&seat, first, first).unwrap();
    state.observe_frontier(&seat, next, next).unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(30_000)));
    assert!(state.can_reserve(&seat, MonoInstant(60_000)));
}

#[test]
fn reservation_from_an_old_daemon_boot_never_occupies_a_slot() {
    let seat = SeatId::new("seat");
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), daemon_boot());
    state.restore(seat.clone(), fresh()).unwrap();
    assert_eq!(
        state.reserved(
            seat.clone(),
            WakeAttemptId::new("old-attempt"),
            old_daemon_boot(),
            MonoInstant(0)
        ),
        Err(DispatchError::WrongBoot)
    );
    assert_eq!(state.active_count(), 0);
    assert!(state.can_reserve(&seat, MonoInstant(0)));
}

#[test]
fn restart_guard_uses_full_retained_delay_without_wall_clock_credit() {
    let seat = SeatId::new("seat");
    let history = DurableRetry {
        retry_step: 1,
        minimum_delay_ms: 30_000,
        effective_delay_ms: 60_000,
        ever_reserved: true,
    };
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(100_000), daemon_boot());
    state.restore(seat.clone(), history).unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(159_999)));
    assert!(state.can_reserve(&seat, MonoInstant(160_000)));
    state.new_attention(&seat).unwrap();
    assert!(!state.can_reserve(&seat, MonoInstant(129_999)));
    assert!(state.can_reserve(&seat, MonoInstant(130_000)));
}

struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

struct FakeWakeStore {
    clock: Arc<FakeClock>,
    events: Arc<Mutex<Vec<&'static str>>>,
    fail_reservation: AtomicBool,
    batch: Option<(UtcMillis, u64)>,
}
impl WakePort for FakeWakeStore {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn wake_recovery_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        Ok(empty_recovery_page())
    }
    fn recover_wake_reservation(
        &self,
        _: WakeRecoveryRequest,
        _: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        unreachable!()
    }
    fn wake_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        Ok(Page {
            items: vec![due_candidate()],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        })
    }
    fn wake_batch_window(
        &self,
        _: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<(UtcMillis, u64)>, ApiError> {
        Ok(self.batch)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.events.lock().unwrap().push("reserve");
        if self.fail_reservation.swap(false, Ordering::SeqCst) {
            return Err(ApiError::store_corrupt("injected failed commit"));
        }
        let Some(witness) = candidate.attention_witness.as_ref() else {
            return Ok(None);
        };
        if !witness.valid_for(candidate, "i", 1, 0, false) {
            return Ok(None);
        }
        Ok(Some(WakeReservation {
            attempt: WakeAttemptId::new("attempt"),
            seat: candidate.seat.clone(),
            attention_witness: witness.clone(),
            reasons: vec!["invitation".into()],
            retained_effective_delay_ms: 30_000,
            lease_until: MonoInstant(if self.batch.is_some() {
                self.clock.monotonic_now().0 + 5_000
            } else {
                5_000
            }),
            retained_minimum_delay_ms: 30_000,
            reserved_at_utc: UtcMillis(0),
            daemon_boot: daemon_boot(),
            host_boot: HostBootId::new("boot"),
            host_epoch: 1,
            target: HostTargetId::new("target"),
            target_generation: 1,
            authority: ReservedWakeAuthority::RecoveryHint {
                execution: ExecutionId::new("execution"),
            },
        }))
    }
    fn complete_wake(
        &self,
        _: WakeAttemptId,
        outcome: WakeOutcome,
        _: Option<&PriorLadder>,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        assert!(matches!(
            outcome,
            WakeOutcome::Submitted
                | WakeOutcome::OutcomeUnknown
                | WakeOutcome::Cancelled
                | WakeOutcome::Refused(_)
        ));
        self.events.lock().unwrap().push("complete");
        Ok(false)
    }
}

/// The store's final attempt fence, as `ScheduledStore` supplies it to the wake
/// worker, over a store this test also wraps with a failure-injecting port.
struct StoreFence<'a>(&'a SqliteStore);
impl ReservationCheck for StoreFence<'_> {
    fn is_current(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        StorePort::validate_wake_reservation(self.0, reservation, budget)
    }
}

struct FakeDeadlinePort {
    clock: Arc<FakeClock>,
    due_calls: AtomicU64,
}
impl crate::scheduler::deadlines::DeadlinePort for FakeDeadlinePort {
    crate::no_durable_work!();
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        _: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        self.due_calls.fetch_add(1, Ordering::SeqCst);
        Ok(DueScanProgress {
            state: request.state,
            examined_candidates: 0,
            warnings_added: 0,
            invitations: DuePhaseProgress::Complete,
            receipts: DuePhaseProgress::Complete,
            has_more: false,
        })
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
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
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

struct WaitingNotifier {
    clock: Arc<FakeClock>,
    entered: Arc<(Mutex<bool>, Condvar)>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

struct CancellationOnlyNotifier {
    entered: Arc<(Mutex<bool>, Condvar)>,
    active: AtomicU64,
}
impl NotificationPort for CancellationOnlyNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.active.fetch_add(1, Ordering::SeqCst);
        let (lock, cv) = &*self.entered;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        while !context.budget.cancellation.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(WakeOutcome::OutcomeUnknown)
    }
}

#[test]
fn lease_expiry_cancels_and_joins_owned_transport_before_slot_release() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events: Arc::new(Mutex::new(Vec::new())),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let entered = Arc::new((Mutex::new(false), Condvar::new()));
    let notifier = CancellationOnlyNotifier {
        entered: entered.clone(),
        active: AtomicU64::new(0),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let external_cancel = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: external_cancel.clone(),
    };
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let runner_ref = &runner;
        let budget_ref = &budget;
        let worker = scope.spawn(move || {
            let result = runner_ref.try_candidate(&due_candidate(), budget_ref);
            tx.send(result).unwrap();
        });
        let (lock, cv) = &*entered;
        let mut started = lock.lock().unwrap();
        while !*started {
            started = cv.wait(started).unwrap();
        }
        drop(started);
        assert_eq!(notifier.active.load(Ordering::SeqCst), 1);
        clock.0.store(5_000, Ordering::SeqCst);
        let automatic = rx.recv_timeout(std::time::Duration::from_millis(200));
        if automatic.is_err() {
            // Failure cleanup keeps the scoped worker joinable; the assertion
            // below still records that lease expiry alone did not cancel it.
            external_cancel.cancel();
            let _ = rx.recv_timeout(std::time::Duration::from_secs(2));
        }
        worker.join().unwrap();
        assert!(
            automatic.is_ok(),
            "lease expiry did not cancel owned transport"
        );
        assert_eq!(notifier.active.load(Ordering::SeqCst), 0);
        assert_eq!(runner.state.lock().unwrap().dispatch.active_count(), 0);
    });
}

/// Delivers the prompt, then spends past the attempt lease in post-send
/// verification (a slow pane read), as `NativeWakeDispatcher` can.
struct SlowVerificationNotifier {
    clock: Arc<FakeClock>,
}
impl NotificationPort for SlowVerificationNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.clock.0.store(5_000, Ordering::SeqCst);
        Ok(WakeOutcome::Submitted)
    }
}

#[test]
fn delivered_prompt_stays_submitted_when_verification_outlives_the_lease() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events: Arc::new(Mutex::new(Vec::new())),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = SlowVerificationNotifier {
        clock: clock.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        Some(WakeOutcome::Submitted)
    );
}

#[test]
fn four_connected_owned_calls_block_a_fifth_until_their_transports_exit() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = CancellationOnlyNotifier {
        entered: Arc::new((Mutex::new(false), Condvar::new())),
        active: AtomicU64::new(0),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let external_cancel = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: external_cancel.clone(),
    };
    let candidate = |index: usize| {
        let mut candidate = due_candidate();
        candidate.seat = SeatId::new(format!("seat-{index}"));
        candidate.attention_witness = Some(WakeAttentionWitness::from_complete(
            "i".into(),
            candidate.seat.clone(),
            1,
            true,
            false,
            None,
            0,
            false,
            Default::default(),
        ));
        candidate
    };
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut workers = Vec::new();
        for index in 0..4 {
            let sender = sender.clone();
            let candidate = candidate(index);
            let runner_ref = &runner;
            let budget_ref = &budget;
            workers.push(scope.spawn(move || {
                sender
                    .send(runner_ref.try_candidate(&candidate, budget_ref))
                    .unwrap();
            }));
        }
        let mut reached_four = false;
        for _ in 0..100 {
            if notifier.active.load(Ordering::SeqCst) == 4 {
                reached_four = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !reached_four {
            external_cancel.cancel();
        }
        assert!(reached_four, "four owned transports did not enter");
        assert_eq!(runner.state.lock().unwrap().dispatch.active_count(), 4);
        assert_eq!(runner.try_candidate(&candidate(4), &budget).unwrap(), None);
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| **event == "reserve")
                .count(),
            4
        );
        clock.0.store(5_000, Ordering::SeqCst);
        for _ in 0..4 {
            if receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_err()
            {
                external_cancel.cancel();
                panic!("owned transport did not exit after lease expiry");
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(notifier.active.load(Ordering::SeqCst), 0);
        assert_eq!(runner.state.lock().unwrap().dispatch.active_count(), 0);
    });
}
impl NotificationPort for WaitingNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        let (lock, cv) = &*self.entered;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        let (lock, cv) = &*self.release;
        let mut released = lock.lock().unwrap();
        while !*released && !context.budget.is_exhausted(self.clock.as_ref()) {
            released = cv
                .wait_timeout(released, std::time::Duration::from_millis(10))
                .unwrap()
                .0;
        }
        Ok(WakeOutcome::OutcomeUnknown)
    }
}

#[test]
fn scheduler_drives_due_work_while_native_prompt_waits() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let wake = FakeWakeStore {
        clock: clock.clone(),
        events,
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let due = FakeDeadlinePort {
        clock: clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let entered = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let notifier = WaitingNotifier {
        clock: clock.clone(),
        entered: entered.clone(),
        release: release.clone(),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &wake,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    std::thread::scope(|scope| {
        let wake_thread = scope.spawn(|| scheduler.drive_wakes(&budget));
        let (lock, cv) = &*entered;
        let mut started = lock.lock().unwrap();
        while !*started {
            started = cv.wait(started).unwrap();
        }
        drop(started);
        assert_eq!(
            scheduler
                .drive_deadlines(&budget)
                .unwrap()
                .due_examined_candidates,
            0
        );
        assert_eq!(due.due_calls.load(Ordering::SeqCst), 1);
        assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
        assert_eq!(
            wake.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| **event == "reserve")
                .count(),
            1
        );
        clock.0.store(5_000, Ordering::SeqCst);
        let (lock, cv) = &*release;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        assert_eq!(wake_thread.join().unwrap().unwrap().attempted, 1);
    });
}
struct FakeNotifier {
    events: Arc<Mutex<Vec<&'static str>>>,
}
impl NotificationPort for FakeNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        assert_eq!(context.budget.deadline, MonoInstant(5_000));
        self.events.lock().unwrap().push("host");
        Ok(WakeOutcome::Submitted)
    }
}
fn due_candidate() -> WakeCandidate {
    WakeCandidate {
        seat: SeatId::new("seat"),
        attention_witness: Some(test_witness(true, false)),
        effectively_retired: false,
        continuity_resolved: true,
        binding_generation: Some(1),
        binding_execution: Some(ExecutionId::new("execution")),
        target: Some(HostTargetId::new("target")),
        reason_bits: 0,
        has_pending_invitation: true,
        has_pending_receipt: false,
        actionable_warning_generation: None,
        actionable_warning_seq: None,
        warning_offer: None,
        attention_version: 1,
        checkpoint_version: 0,
        retry_step: 0,
        reservation_id: None,
        reservation_boot: None,
        last_reservation_id: None,
        last_reservation_boot: None,
        last_reserved_frontier: Default::default(),
        minimum_delay_ms: 0,
        effective_delay_ms: 0,
        last_outcome: None,
        last_reserved_at_utc: None,
    }
}

fn test_witness(pending_invitation: bool, pending_receipt: bool) -> WakeAttentionWitness {
    WakeAttentionWitness::from_complete(
        "i".into(),
        SeatId::new("seat"),
        1,
        pending_invitation,
        pending_receipt,
        None,
        0,
        false,
        Default::default(),
    )
}

#[test]
fn runner_commits_reservation_before_host_and_completes_without_receipt_mutation() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        None
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
}

#[test]
fn incomplete_attention_never_reaches_reservation() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let mut candidate = due_candidate();
    candidate.attention_witness = None;
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn stale_attention_witness_is_rejected_by_reserving_store() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let mut candidate = due_candidate();
    candidate.attention_witness = Some(WakeAttentionWitness::from_complete(
        "i".into(),
        candidate.seat.clone(),
        0,
        true,
        false,
        None,
        0,
        false,
        Default::default(),
    ));
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
    assert_eq!(*events.lock().unwrap(), vec!["reserve"]);
}

struct JumpClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl Clock for JumpClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}
struct SpacedWakeStore {
    clock: Arc<JumpClock>,
    reservations: AtomicU64,
    completions: AtomicU64,
}
impl WakePort for SpacedWakeStore {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn wake_recovery_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        Ok(empty_recovery_page())
    }
    fn recover_wake_reservation(
        &self,
        _: WakeRecoveryRequest,
        _: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        unreachable!()
    }
    fn wake_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        unreachable!()
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let n = self.reservations.fetch_add(1, Ordering::SeqCst);
        let mut reservation = test_reservation();
        reservation.seat = candidate.seat.clone();
        reservation.attention_witness = candidate.attention_witness.clone().unwrap();
        reservation.attempt = WakeAttemptId::new(format!("attempt-{n}"));
        reservation.lease_until = MonoInstant(self.clock.monotonic_now().0 + 5_000);
        Ok(Some(reservation))
    }
    fn complete_wake(
        &self,
        _: WakeAttemptId,
        _: WakeOutcome,
        _: Option<&PriorLadder>,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.completions.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}
struct SpacedNotifier {
    clock: Arc<JumpClock>,
    calls: Mutex<Vec<MonoInstant>>,
}

/// Models a persisted reservation left by the prior daemon boot. The new
/// scheduler must recover it durably before it can reserve a successor.
struct StrandedOldBootStore {
    clock: Arc<JumpClock>,
    old_active: AtomicBool,
    reserve_calls: AtomicU64,
}
impl WakePort for StrandedOldBootStore {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn wake_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        let mut candidate = due_candidate();
        candidate.last_reservation_id = Some(WakeAttemptId::new("old-attempt"));
        candidate.last_reservation_boot = Some(HostBootId::new("old-boot"));
        candidate.minimum_delay_ms = 30_000;
        candidate.effective_delay_ms = 60_000;
        candidate.retry_step = 1;
        Ok(Page {
            items: vec![candidate],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        })
    }
    fn wake_recovery_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        let mut page = empty_recovery_page();
        if self.old_active.load(Ordering::SeqCst) {
            page.items.push(WakeRecoveryCandidate {
                seat: SeatId::new("seat"),
                attempt: WakeAttemptId::new("old-attempt"),
                prior_daemon_boot: old_daemon_boot(),
            });
        }
        Ok(page)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        _: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        assert_eq!(request.instance, "i");
        assert_eq!(request.elected_boot, daemon_boot());
        assert_eq!(request.prior_daemon_boot, old_daemon_boot());
        assert_eq!(request.attempt, WakeAttemptId::new("old-attempt"));
        self.old_active.store(false, Ordering::SeqCst);
        Ok(WakeRecoveryOutcome::Recovered)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.reserve_calls.fetch_add(1, Ordering::SeqCst);
        if self.old_active.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut reservation = test_reservation();
        reservation.seat = candidate.seat.clone();
        reservation.attention_witness = candidate.attention_witness.clone().unwrap();
        reservation.lease_until = MonoInstant(self.clock.monotonic_now().0 + 5_000);
        Ok(Some(reservation))
    }
    fn complete_wake(
        &self,
        _: WakeAttemptId,
        _: WakeOutcome,
        _: Option<&PriorLadder>,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        Ok(false)
    }
}

#[test]
fn elected_boot_recovers_stranded_attempt_then_waits_full_monotonic_delay() {
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(100_000),
        utc: AtomicI64::new(3_600_000),
    });
    let store = StrandedOldBootStore {
        clock: clock.clone(),
        old_active: AtomicBool::new(true),
        reserve_calls: AtomicU64::new(0),
    };
    let notifier = SpacedNotifier {
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let deadline_clock = Arc::new(FakeClock(AtomicU64::new(100_000)));
    let due = FakeDeadlinePort {
        clock: deadline_clock,
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    clock.mono.store(159_999, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().recovered, 1);
    assert_eq!(store.reserve_calls.load(Ordering::SeqCst), 0);
    clock.mono.store(160_000, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(*notifier.calls.lock().unwrap(), vec![MonoInstant(160_000)]);
}

#[test]
fn sqlite_reopen_recovers_old_boot_before_real_fake_host_attempt() {
    let path = std::env::temp_dir().join(format!(
        "herdr-scheduler-recovery-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let context = StoreContext::new(path.clone(), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat','i','resolved','native','target',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','target','host',1,1,0,'fresh','occupied','idle','execution',1,'term-'||'target','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('seat',1,1,'target','host',1,'codex','session','execution','fresh',0,0,'term-'||'target','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread','seat','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES ('invite','thread','seat',1,'pending',0,100,100,1);\
    ").unwrap();
    drop(db);
    let old_store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(old_daemon_boot()),
            minimum_wake_delay_ms: 60_000,
            wake_batch_delay_ms: 0,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let old_budget = CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    };
    let old_candidate = StorePort::wake_candidates(&old_store, PageRequest::default(), &old_budget)
        .unwrap()
        .items
        .remove(0);
    let old_reservation = StorePort::reserve_wake(&old_store, &old_candidate, &old_budget)
        .unwrap()
        .unwrap();
    drop(old_store);
    // Recovery must clear old uncertainty even where normal candidate
    // discovery returns no work or target policy excludes the seat.
    let recovery_context = StoreContext::new(path.clone(), clock.clone());
    let db = recovery_context.open_writer().unwrap();
    for (seat, state) in [
        ("zero", "resolved"),
        ("unsafe", "unresolved"),
        ("retired", "retired"),
    ] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,target_generation,created_at,retired_at) VALUES (?1,'i',?2,'native',1,1,0,CASE WHEN ?2='retired' THEN 0 ELSE NULL END)", rusqlite::params![seat,state]).unwrap();
        db.execute("INSERT INTO wake_work(seat_id,reason_bits,retry_step,reservation_id,reservation_boot,last_reservation_id,last_reservation_boot,minimum_delay_ms,effective_delay_ms) VALUES (?1,0,0,?1,?2,?1,?2,60000,60000)", rusqlite::params![seat,old_daemon_boot().to_string()]).unwrap();
    }
    drop(db);

    clock.mono.store(100_000, Ordering::SeqCst);
    clock.utc.store(3_600_000, Ordering::SeqCst);
    let reopened = Arc::new(
        SqliteStore::new(
            StoreContext::new(path.clone(), clock.clone()),
            "i",
            StoreSettings {
                daemon_boot: Some(daemon_boot()),
                minimum_wake_delay_ms: 30_000,
                wake_batch_delay_ms: 0,
                ..StoreSettings::default()
            },
        )
        .unwrap(),
    );
    let ports = ScheduledStore::new(reopened.clone(), Arc::new(FairWriter::new(32)));
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(100_000))),
        due_calls: AtomicU64::new(0),
    };
    let host = SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let notifier = NativeWakeDispatcher::new(&host, &ports, clock.as_ref());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &ports,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    clock.mono.store(159_999, Ordering::SeqCst);
    let first = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!((first.recovered, first.attempted), (4, 0));
    assert!(host.calls.lock().unwrap().is_empty());
    assert_eq!(
        StorePort::recover_wake_reservation(
            reopened.as_ref(),
            WakeRecoveryRequest {
                instance: "i".into(),
                seat: SeatId::new("seat"),
                attempt: old_reservation.attempt.clone(),
                prior_daemon_boot: old_daemon_boot(),
                elected_boot: daemon_boot(),
            },
            &budget,
        )
        .unwrap(),
        WakeRecoveryOutcome::AlreadySettled
    );
    let db = recovery_context.open_writer().unwrap();
    let recovered: i64 = db.query_row("SELECT COUNT(*) FROM wake_work WHERE reservation_id IS NULL AND last_outcome='outcome_unknown' AND minimum_delay_ms=60000 AND effective_delay_ms=60000", [], |r| r.get(0)).unwrap();
    assert_eq!(recovered, 4);
    drop(db);
    clock.mono.store(160_000, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(*host.calls.lock().unwrap(), vec![MonoInstant(160_000)]);
    StorePort::complete_wake(
        reopened.as_ref(),
        old_reservation.attempt,
        WakeOutcome::Submitted,
        None,
        &budget,
    )
    .unwrap();
    drop(scheduler);
    drop(ports);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

/// Only the external host is fake: discovery, completed-attention witnesses,
/// reservation, final fence and completion all cross the SQLite facade.
struct SqliteTimingHost {
    context: StoreContext,
    clock: Arc<JumpClock>,
    calls: Mutex<Vec<MonoInstant>>,
}

/// Fail only the completion boundary; all durable operations still use SQLite.
struct FailingCompletionStore<'a> {
    store: &'a SqliteStore,
    failures: AtomicU64,
    clock: &'a JumpClock,
    exhaust_budget: AtomicBool,
    lose_response: AtomicBool,
    completions: Mutex<Vec<(WakeAttemptId, WakeOutcome)>>,
}
impl WakePort for FailingCompletionStore<'_> {
    fn clock(&self) -> &dyn Clock {
        StorePort::clock(self.store)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        StorePort::wake_candidates(self.store, page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        StorePort::reserve_wake(self.store, candidate, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        StorePort::wake_recovery_candidates(self.store, page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        StorePort::recover_wake_reservation(self.store, request, budget)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.completions
            .lock()
            .unwrap()
            .push((attempt.clone(), outcome));
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            if self.exhaust_budget.load(Ordering::SeqCst) {
                self.clock.mono.store(budget.deadline.0, Ordering::SeqCst);
            }
            return Err(completion_error());
        }
        let matched =
            StorePort::complete_wake(self.store, attempt, outcome, refused_restore, budget)?;
        if self.lose_response.swap(false, Ordering::SeqCst) {
            return Err(completion_error());
        }
        Ok(matched)
    }
}
fn completion_error() -> ApiError {
    ApiError::store_corrupt("injected completion failure")
}

fn completion_fixture() -> (
    std::path::PathBuf,
    Arc<JumpClock>,
    StoreContext,
    SqliteStore,
) {
    let path = std::env::temp_dir().join(format!("herdr-completion-{}.db", uuid::Uuid::new_v4()));
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let context = StoreContext::new(path.clone(), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat','i','resolved','native','target',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','target','host',1,1,0,'fresh','occupied','idle','execution',1,'term-'||'target','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('seat',1,1,'target','host',1,'codex','session','execution','fresh',0,0,'term-'||'target','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread','seat','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES ('invite','thread','seat',1,'pending',0,100,100,1);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('message','i','thread',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('message','thread','seat','pending',100);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings {
            daemon_boot: Some(daemon_boot()),
            wake_batch_delay_ms: 0,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (path, clock, context, store)
}

#[test]
fn sqlite_same_boot_retries_joined_completion_without_another_prompt_or_ack() {
    // Dropping the correlated completion record loses this same-boot settlement.
    let (path, clock, context, store) = completion_fixture();
    let failing = FailingCompletionStore {
        store: &store,
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        failures: AtomicU64::new(1),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let host = SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(vec![]),
    };
    let fence = StoreFence(&store);
    let notifier = NativeWakeDispatcher::new(&host, &fence, clock.as_ref());
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        scheduler.drive_wakes(&budget).unwrap_err().code,
        ErrorCode::StoreCorrupt
    );
    let db = context.open_writer().unwrap();
    let attempt: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='seat'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT last_outcome FROM wake_work WHERE seat_id='seat'",
            [],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );
    clock.mono.store(40_000, Ordering::SeqCst);
    scheduler.drive_wakes(&budget).unwrap();
    let settled: (Option<String>, String, Option<String>) = db.query_row("SELECT reservation_id,last_reservation_id,last_outcome FROM wake_work WHERE seat_id='seat'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(settled, (None, attempt.clone(), Some("submitted".into())));
    assert_eq!(
        *failing.completions.lock().unwrap(),
        vec![
            (WakeAttemptId::new(attempt.clone()), WakeOutcome::Submitted),
            (WakeAttemptId::new(attempt), WakeOutcome::Submitted)
        ]
    );
    assert_eq!(*host.calls.lock().unwrap(), vec![MonoInstant(0)]);
    assert_eq!(
        db.query_row(
            "SELECT state FROM receipts WHERE message_id='message'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    drop(db);
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_joined_completion_survives_attention_retirement_and_response_loss() {
    // Candidate eligibility must not gate an already joined attempt's settlement.
    for (mutation, expected, response_loss) in [
        (
            "DELETE FROM invitations; DELETE FROM receipts;",
            "submitted",
            false,
        ),
        (
            "UPDATE seats SET state='retired',retired_at=0;",
            "unsafe",
            false,
        ),
        (
            "UPDATE seats SET generation=2; UPDATE host_instances SET host_epoch=2;",
            "unsafe",
            false,
        ),
        ("", "submitted", true),
    ] {
        let (path, clock, context, store) = completion_fixture();
        let failing = FailingCompletionStore {
            store: &store,
            clock: clock.as_ref(),
            exhaust_budget: AtomicBool::new(false),
            failures: AtomicU64::new(u64::from(!response_loss)),
            lose_response: AtomicBool::new(response_loss),
            completions: Mutex::new(vec![]),
        };
        let host = SqliteTimingHost {
            context: StoreContext::new(path.clone(), clock.clone()),
            clock: clock.clone(),
            calls: Mutex::new(vec![]),
        };
        let fence = StoreFence(&store);
        let notifier = NativeWakeDispatcher::new(&host, &fence, clock.as_ref());
        let due = FakeDeadlinePort {
            clock: Arc::new(FakeClock(AtomicU64::new(0))),
            due_calls: AtomicU64::new(0),
        };
        let scheduler = Scheduler::new(
            "i".into(),
            &due,
            &failing,
            &notifier,
            RetryConfig::default(),
            daemon_boot(),
        );
        let budget = CallBudget {
            deadline: MonoInstant(300_000),
            cancellation: Cancellation::default(),
        };
        assert!(scheduler.drive_wakes(&budget).is_err());
        let db = context.open_writer().unwrap();
        db.execute_batch(mutation).unwrap();
        // Ensure there is no retained candidate to carry the retry implicitly.
        scheduler.scan.lock().unwrap().pending.clear();
        clock.utc.store(900, Ordering::SeqCst);
        scheduler.drive_wakes(&budget).unwrap();
        let terminal: (Option<String>, String, i64) = db.query_row("SELECT reservation_id,last_outcome,completed_at_utc FROM wake_work WHERE seat_id='seat'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(
            terminal,
            (None, expected.into(), if response_loss { 0 } else { 900 })
        );
        let calls = failing.completions.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        assert_eq!(*host.calls.lock().unwrap(), vec![MonoInstant(0)]);
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM receipts WHERE state='acked'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(calls);
        drop(db);
        drop(scheduler);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn sqlite_late_joined_completion_cannot_clear_new_attempt_or_foreign_boot() {
    // Removing the store's attempt/boot fences would clear a successor here.
    for mutation in [
        "UPDATE wake_work SET reservation_id='successor',last_reservation_id='successor';",
        "UPDATE wake_work SET reservation_boot='00000000-0000-4000-8000-000000000002';",
    ] {
        let (path, clock, context, store) = completion_fixture();
        let failing = FailingCompletionStore {
            store: &store,
            clock: clock.as_ref(),
            exhaust_budget: AtomicBool::new(false),
            failures: AtomicU64::new(1),
            lose_response: AtomicBool::new(false),
            completions: Mutex::new(vec![]),
        };
        let host = SqliteTimingHost {
            context: StoreContext::new(path.clone(), clock.clone()),
            clock: clock.clone(),
            calls: Mutex::new(vec![]),
        };
        let fence = StoreFence(&store);
        let notifier = NativeWakeDispatcher::new(&host, &fence, clock.as_ref());
        let due = FakeDeadlinePort {
            clock: Arc::new(FakeClock(AtomicU64::new(0))),
            due_calls: AtomicU64::new(0),
        };
        let scheduler = Scheduler::new(
            "i".into(),
            &due,
            &failing,
            &notifier,
            RetryConfig::default(),
            daemon_boot(),
        );
        let budget = CallBudget {
            deadline: MonoInstant(300_000),
            cancellation: Cancellation::default(),
        };
        assert!(scheduler.drive_wakes(&budget).is_err());
        let db = context.open_writer().unwrap();
        db.execute_batch(mutation).unwrap();
        let before: (Option<String>, Option<String>) = db
            .query_row(
                "SELECT reservation_id,reservation_boot FROM wake_work",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        scheduler.drive_wakes(&budget).unwrap();
        let after: (Option<String>, Option<String>, Option<String>) = db
            .query_row(
                "SELECT reservation_id,reservation_boot,last_outcome FROM wake_work",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(after, (before.0, before.1, None));
        assert_eq!(*host.calls.lock().unwrap(), vec![MonoInstant(0)]);
        assert_eq!(failing.completions.lock().unwrap().len(), 2);
        drop(db);
        drop(scheduler);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

struct CompletionNotifier(Mutex<Vec<WakeReservation>>);
impl NotificationPort for CompletionNotifier {
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.0.lock().unwrap().push(reservation);
        Ok(WakeOutcome::Submitted)
    }
}

struct SqliteLockingNotifier {
    context: StoreContext,
    lock: Mutex<Option<rusqlite::Connection>>,
    calls: AtomicU64,
}
impl NotificationPort for SqliteLockingNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let db = self.context.open_writer().unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        *self.lock.lock().unwrap() = Some(db);
        Ok(WakeOutcome::Submitted)
    }
}
impl SqliteLockingNotifier {
    fn release(&self) {
        if let Some(db) = self.lock.lock().unwrap().take() {
            db.execute_batch("ROLLBACK").unwrap();
        }
    }
}

fn wait_for_completion_calls(store: &FailingCompletionStore<'_>, count: usize) {
    let started = std::time::Instant::now();
    while store.completions.lock().unwrap().len() < count {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "completion did not enter the real store boundary"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

struct RollbackOnDrop<'a>(&'a rusqlite::Connection);
impl Drop for RollbackOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("ROLLBACK");
    }
}
struct ReleaseNotifierLockOnDrop<'a>(&'a SqliteLockingNotifier);
impl Drop for ReleaseNotifierLockOnDrop<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct ContendedTimingHost {
    host: SqliteTimingHost,
    lock: Mutex<Option<rusqlite::Connection>>,
}
impl ContendedTimingHost {
    fn release(&self) {
        if let Some(db) = self.lock.lock().unwrap().take() {
            db.execute_batch("ROLLBACK").unwrap();
        }
    }
}
impl HostPort for ContendedTimingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.host.native_launch_capability()
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.host.observe_current_target(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.host.enumerate_targets(context)
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.host.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        let result = self.host.submit_prompt(target, text, context)?;
        let db = self.host.context.open_writer().unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        *self.lock.lock().unwrap() = Some(db);
        Ok(result)
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.host.launch_native(request, context)
    }
    fn send_submit_key(
        &self,
        _: &crate::ports::SafeWakeTarget,
        _: &crate::ports::HostCallContext,
    ) -> Result<(), crate::protocol::results::ApiError> {
        Ok(())
    }
}
struct WakeWorkerCleanup {
    cancellation: Cancellation,
    host: Arc<ContendedTimingHost>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for WakeWorkerCleanup {
    fn drop(&mut self) {
        self.host.release();
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn wait_for_condition(mut ready: impl FnMut() -> bool, detail: &str) {
    let started = std::time::Instant::now();
    while !ready() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{detail}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn sqlite_actual_wake_worker_retries_completion_and_yields_to_foreground() {
    // The actual ScheduledStore background lane must release its turn on a
    // bounded SQLite error, preserve Health's exact failure, and retry it.
    use crate::service::fair_writer::FairWriter;
    use crate::service::workers::{WorkerStatus, start_wake_worker};
    let (path, clock, context, store) = completion_fixture();
    let db = context.open_writer().unwrap();
    let store = Arc::new(store);
    let writer = Arc::new(FairWriter::new(16));
    let status = Arc::new(WorkerStatus::default());
    let host = Arc::new(ContendedTimingHost {
        host: SqliteTimingHost {
            context: StoreContext::new(path.clone(), clock.clone()),
            clock: clock.clone(),
            calls: Mutex::new(vec![]),
        },
        lock: Mutex::new(None),
    });
    let cancellation = Cancellation::default();
    let pacer = Arc::new(crate::service::pacer::Pacer::new(
        "wake",
        clock.clone(),
        cancellation.clone(),
    ));
    let worker = start_wake_worker(
        store.clone(),
        writer.clone(),
        host.clone(),
        "i".into(),
        daemon_boot(),
        RetryConfig::default(),
        pacer.clone(),
        cancellation.clone(),
        status.clone(),
        Arc::new(crate::ports::NoPokeCapabilities),
        Arc::new(crate::service::host_reachability::HostReachability::default()),
    )
    .unwrap();
    let cleanup = WakeWorkerCleanup {
        cancellation,
        host: host.clone(),
        worker: Some(worker),
    };
    wait_for_condition(
        || host.lock.lock().unwrap().is_some(),
        "native host did not enter external writer contention",
    );
    // Allow the owned host worker to join and enter its completion turn before
    // queuing foreground work; the queue assertion below verifies contention.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let original: String = db
        .query_row("SELECT reservation_id FROM wake_work", [], |r| r.get(0))
        .unwrap();
    let foreground_budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let writer_ref = writer.as_ref();
        let clock_ref = clock.as_ref();
        let foreground_budget_ref = &foreground_budget;
        let foreground = scope.spawn(move || {
            let _ = sender.send(writer_ref.enter_foreground(foreground_budget_ref, clock_ref));
        });
        wait_for_condition(
            || writer.waiting().0 == 1,
            "foreground did not wait behind the real completion turn",
        );
        clock.mono.store(4_000, Ordering::SeqCst);
        let turn = receiver
            .recv_timeout(std::time::Duration::from_millis(250))
            .expect("foreground waited past one bounded background completion")
            .unwrap();
        foreground.join().unwrap();
        wait_for_condition(
            || status.last_error().is_some(),
            "completion failure was not reported by actual worker callbacks",
        );
        assert!(status.last_error().unwrap().contains("DeadlineExceeded"));
        let retained: (String, Option<String>) = db
            .query_row(
                "SELECT reservation_id,last_outcome FROM wake_work",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(retained, (original.clone(), None));
        assert_eq!(*host.host.calls.lock().unwrap(), vec![MonoInstant(0)]);
        host.release();
        drop(turn);
    });
    // The failed completion was a store-callback error, so the lane does not
    // back off; it retries the retained completion at its next safety tick.
    clock.mono.store(60_000, Ordering::SeqCst);
    pacer.clock_advanced();
    wait_for_condition(
        || {
            db.query_row(
                "SELECT reservation_id IS NULL AND last_outcome='submitted' FROM wake_work",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
        },
        "actual worker did not settle its retained completion",
    );
    wait_for_condition(
        || status.last_error().is_none(),
        "matching retry did not clear actual worker failure",
    );
    assert_eq!(*host.host.calls.lock().unwrap(), vec![MonoInstant(0)]);
    assert_eq!(
        db.query_row("SELECT last_reservation_id FROM wake_work", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    drop(cleanup);
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_initial_settlement_releases_runner_state_during_external_writer_wait() {
    // Holding runner state through BEGIN IMMEDIATE blocks unrelated admission.
    let (path, clock, context, store) = completion_fixture();
    let failing = FailingCompletionStore {
        store: &store,
        failures: AtomicU64::new(0),
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let notifier = SqliteLockingNotifier {
        context: StoreContext::new(path.clone(), clock.clone()),
        lock: Mutex::new(None),
        calls: AtomicU64::new(0),
    };
    let runner = WakeRunner::new(&failing, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget)
        .unwrap()
        .items
        .remove(0);
    std::thread::scope(|scope| {
        let _release = ReleaseNotifierLockOnDrop(&notifier);
        let worker = scope.spawn(|| runner.try_candidate(&candidate, &budget));
        wait_for_completion_calls(&failing, 1);
        // Readability of the actual runner mutex is checked while SQLite is locked.
        let unlocked = runner.state.try_lock().is_ok();
        if unlocked {
            let state = runner.state.lock().unwrap();
            assert_eq!(state.pending.len(), 1);
            assert!(state.pending[0].claimed);
            assert_eq!(state.dispatch.active_count(), 0);
            drop(state);
            assert_eq!(runner.retry_completions(&budget).unwrap(), None);
            assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
            assert_eq!(failing.completions.lock().unwrap().len(), 1);
        }
        notifier.release();
        assert_eq!(
            worker.join().unwrap().unwrap(),
            Some(WakeOutcome::Submitted)
        );
        assert!(
            unlocked,
            "initial durable completion held runner state during SQLite wait"
        );
    });
    assert_eq!(notifier.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        context
            .open_writer()
            .unwrap()
            .query_row("SELECT last_outcome FROM wake_work", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "submitted"
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_retry_settlement_releases_runner_state_during_external_writer_wait() {
    // A retry must remain owned while allowing concurrent drives to see its claim.
    let (path, clock, context, store) = completion_fixture();
    let failing = FailingCompletionStore {
        store: &store,
        failures: AtomicU64::new(1),
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let notifier = CompletionNotifier(Mutex::new(vec![]));
    let runner = WakeRunner::new(&failing, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget)
        .unwrap()
        .items
        .remove(0);
    assert!(runner.try_candidate(&candidate, &budget).is_err());
    let db = context.open_writer().unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    std::thread::scope(|scope| {
        let _rollback = RollbackOnDrop(&db);
        let worker = scope.spawn(|| runner.retry_completions(&budget));
        wait_for_completion_calls(&failing, 2);
        let unlocked = runner.state.try_lock().is_ok();
        if unlocked {
            let state = runner.state.lock().unwrap();
            assert_eq!(state.pending.len(), 1);
            assert!(state.pending[0].claimed);
            assert_eq!(state.dispatch.active_count(), 0);
            drop(state);
            assert_eq!(runner.retry_completions(&budget).unwrap(), None);
            assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
            assert_eq!(failing.completions.lock().unwrap().len(), 2);
        }
        db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), None);
        assert!(
            unlocked,
            "retry durable completion held runner state during SQLite wait"
        );
    });
    assert_eq!(notifier.0.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row("SELECT last_outcome FROM wake_work", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "submitted"
    );
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_claimed_settlement_counts_toward_four_slots_during_writer_wait() {
    // Moving a claimed completion out of counted state would admit a fifth seat.
    let (path, clock, context, store) = completion_fixture();
    for n in 1..=4 {
        add_completion_peer(&context, n);
    }
    let failing = FailingCompletionStore {
        store: &store,
        failures: AtomicU64::new(4),
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let notifier = CompletionNotifier(Mutex::new(vec![]));
    let runner = WakeRunner::new(&failing, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    let candidates = StorePort::wake_candidates(&store, PageRequest::default(), &budget)
        .unwrap()
        .items;
    assert_eq!(candidates.len(), 5);
    for candidate in &candidates[..4] {
        assert!(runner.try_candidate(candidate, &budget).is_err());
    }
    let db = context.open_writer().unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    std::thread::scope(|scope| {
        let _rollback = RollbackOnDrop(&db);
        let worker = scope.spawn(|| runner.retry_completions(&budget));
        wait_for_completion_calls(&failing, 5);
        let state = runner
            .state
            .try_lock()
            .expect("claimed settlement must release runner state");
        assert_eq!(state.pending.len() + state.dispatch.active_count(), 4);
        assert_eq!(
            state
                .pending
                .iter()
                .filter(|pending| pending.claimed)
                .count(),
            1
        );
        drop(state);
        assert_eq!(runner.try_candidate(&candidates[4], &budget).unwrap(), None);
        assert_eq!(notifier.0.lock().unwrap().len(), 4);
        db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), None);
    });
    assert!(!runner.has_pending_completions().unwrap());
    assert_eq!(
        runner.try_candidate(&candidates[4], &budget).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(notifier.0.lock().unwrap().len(), 5);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM wake_work WHERE last_outcome='submitted' AND reservation_id IS NULL", [], |r| r.get::<_,i64>(0)).unwrap(), 5);
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_contended_writer_cancellation_and_expiry_return_completion_claims() {
    // A claimed record must be returned while another completion still owns
    // the common writer; cancellation must not wait for that writer to unlock.
    for cancel in [true, false] {
        let (path, clock, context, store) = completion_fixture();
        add_completion_peer(&context, 1);
        let failing = FailingCompletionStore {
            store: &store,
            failures: AtomicU64::new(2),
            clock: clock.as_ref(),
            exhaust_budget: AtomicBool::new(false),
            lose_response: AtomicBool::new(false),
            completions: Mutex::new(vec![]),
        };
        let notifier = CompletionNotifier(Mutex::new(vec![]));
        let runner = WakeRunner::new(&failing, &notifier, RetryConfig::default(), daemon_boot());
        let budget = CallBudget {
            deadline: MonoInstant(300_000),
            cancellation: Cancellation::default(),
        };
        let candidates = StorePort::wake_candidates(&store, PageRequest::default(), &budget)
            .unwrap()
            .items;
        for candidate in &candidates {
            assert!(runner.try_candidate(candidate, &budget).is_err());
        }
        let db = context.open_writer().unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        std::thread::scope(|scope| {
            let _rollback = RollbackOnDrop(&db);
            let first = scope.spawn(|| runner.retry_completions(&budget));
            wait_for_completion_calls(&failing, 3);
            let short = CallBudget {
                deadline: MonoInstant(1),
                cancellation: Cancellation::default(),
            };
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let runner_ref = &runner;
            let short_for_worker = CallBudget {
                deadline: short.deadline,
                cancellation: short.cancellation.clone(),
            };
            let second = scope.spawn(move || {
                let result = runner_ref.retry_completions(&short_for_worker);
                sender.send(result).unwrap();
            });
            wait_for_completion_calls(&failing, 4);
            assert!(
                receiver
                    .recv_timeout(std::time::Duration::from_millis(20))
                    .is_err()
            );
            if cancel {
                short.cancellation.cancel();
            } else {
                clock.mono.store(1, Ordering::SeqCst);
            }
            let early = receiver.recv_timeout(std::time::Duration::from_millis(250));
            let (retained, claimed) = {
                let state = runner.state.lock().unwrap();
                (
                    state.pending.len(),
                    state
                        .pending
                        .iter()
                        .filter(|pending| pending.claimed)
                        .count(),
                )
            };
            let unsettled: i64 = db.query_row("SELECT COUNT(*) FROM wake_work WHERE reservation_id IS NOT NULL AND last_outcome IS NULL", [], |r| r.get(0)).unwrap();
            // Release before asserting so a failing regression still joins workers.
            db.execute_batch("ROLLBACK").unwrap();
            first.join().unwrap().unwrap();
            second.join().unwrap();
            let result = early
                .expect("live completion budget waited for common writer unlock")
                .unwrap()
                .unwrap();
            assert_eq!(
                result.code,
                if cancel {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::DeadlineExceeded
                }
            );
            assert_eq!((retained, claimed, unsettled), (2, 1, 2));
        });
        runner.retry_completions(&budget).unwrap();
        assert!(!runner.has_pending_completions().unwrap());
        assert_eq!(notifier.0.lock().unwrap().len(), 2);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM wake_work WHERE reservation_id IS NULL AND last_outcome='submitted'", [], |r| r.get::<_,i64>(0)).unwrap(), 2);
        drop(db);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn sqlite_external_writer_budget_errors_keep_exact_completion_retryable() {
    // Live budget expiry during BEGIN must neither decide a late mutation nor
    // strand the scheduler claim after the real store returns its error.
    for cancel in [true, false] {
        let (path, clock, context, store) = completion_fixture();
        let failing = FailingCompletionStore {
            store: &store,
            failures: AtomicU64::new(1),
            clock: clock.as_ref(),
            exhaust_budget: AtomicBool::new(false),
            lose_response: AtomicBool::new(false),
            completions: Mutex::new(vec![]),
        };
        let notifier = CompletionNotifier(Mutex::new(vec![]));
        let runner = WakeRunner::new(&failing, &notifier, RetryConfig::default(), daemon_boot());
        let budget = CallBudget {
            deadline: MonoInstant(300_000),
            cancellation: Cancellation::default(),
        };
        let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget)
            .unwrap()
            .items
            .remove(0);
        assert!(runner.try_candidate(&candidate, &budget).is_err());
        let original = failing.completions.lock().unwrap()[0].clone();
        let db = context.open_writer().unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        std::thread::scope(|scope| {
            let _rollback = RollbackOnDrop(&db);
            let short = CallBudget {
                deadline: MonoInstant(1),
                cancellation: Cancellation::default(),
            };
            let short_for_worker = CallBudget {
                deadline: short.deadline,
                cancellation: short.cancellation.clone(),
            };
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let runner_ref = &runner;
            let worker = scope.spawn(move || {
                sender
                    .send(runner_ref.retry_completions(&short_for_worker))
                    .unwrap();
            });
            wait_for_completion_calls(&failing, 2);
            assert_eq!(runner.retry_completions(&budget).unwrap(), None);
            assert_eq!(failing.completions.lock().unwrap().len(), 2);
            assert!(
                receiver
                    .recv_timeout(std::time::Duration::from_millis(20))
                    .is_err()
            );
            if cancel {
                short.cancellation.cancel();
            } else {
                clock.mono.store(1, Ordering::SeqCst);
            }
            let early = receiver.recv_timeout(std::time::Duration::from_millis(250));
            let claimed = runner.state.lock().unwrap().pending[0].claimed;
            let durable: (String, Option<String>) = db
                .query_row(
                    "SELECT reservation_id,last_outcome FROM wake_work",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            db.execute_batch("ROLLBACK").unwrap();
            worker.join().unwrap();
            let err = early
                .expect("live completion budget waited for SQLite unlock")
                .unwrap()
                .unwrap();
            assert_eq!(
                err.code,
                if cancel {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::DeadlineExceeded
                }
            );
            assert!(!claimed, "completion error stranded its claimed capacity");
            assert_eq!(durable, (original.0.as_str().into(), None));
        });
        assert!(
            db.query_row(
                "SELECT reservation_id IS NOT NULL FROM wake_work",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert_eq!(runner.retry_completions(&budget).unwrap(), None);
        assert_eq!(
            *failing.completions.lock().unwrap(),
            vec![original.clone(), original.clone(), original]
        );
        assert_eq!(notifier.0.lock().unwrap().len(), 1);
        assert!(!runner.has_pending_completions().unwrap());
        assert_eq!(
            db.query_row("SELECT last_outcome FROM wake_work", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "submitted"
        );
        assert_eq!(
            db.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "pending"
        );
        drop(db);
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
fn add_completion_peer(context: &StoreContext, n: usize) {
    let db = context.open_writer().unwrap();
    let seat = format!("peer-{n}");
    let target = format!("target-{n}");
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,1,0)", rusqlite::params![seat,target]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'host',1,1,0,'fresh','occupied','idle',?2,1,'term-'||?1,'inc','coherent_enumeration',1)", rusqlite::params![target,seat]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'host',1,'codex',?1,?1,'fresh',0,0,'term-'||?2,'inc')", rusqlite::params![seat,target]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread',?1,'invited')",
        [&seat],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES (?1,'thread',?1,1,'pending',0,100,100,1)", [&seat]).unwrap();
}

#[test]
fn sqlite_failed_completion_allows_peer_progress_and_preserves_join_guard() {
    // An error in the retry lane must not abort admission for healthy seats.
    let (path, clock, context, store) = completion_fixture();
    let failing = FailingCompletionStore {
        store: &store,
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        failures: AtomicU64::new(1),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let notifier = CompletionNotifier(Mutex::new(vec![]));
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    assert!(scheduler.drive_wakes(&budget).is_err());
    add_completion_peer(&context, 1);
    scheduler.scan.lock().unwrap().pending.clear();
    failing.failures.store(1, Ordering::SeqCst);
    assert!(scheduler.drive_wakes(&budget).is_err());
    assert_eq!(notifier.0.lock().unwrap().len(), 2);
    let db = context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT last_outcome FROM wake_work WHERE seat_id='peer-1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "submitted"
    );
    assert!(
        db.query_row(
            "SELECT reservation_id IS NOT NULL FROM wake_work WHERE seat_id='seat'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    let cancelled = CallBudget {
        deadline: MonoInstant(100),
        cancellation: Cancellation::default(),
    };
    cancelled.cancellation.cancel();
    let calls_before = failing.completions.lock().unwrap().len();
    assert!(scheduler.drive_wakes(&cancelled).is_err());
    assert_eq!(failing.completions.lock().unwrap().len(), calls_before);
    clock.mono.store(29_999, Ordering::SeqCst);
    scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(notifier.0.lock().unwrap().len(), 2);
    // Settlement latency must not reset the 30-second guard anchored at join.
    clock.mono.store(30_000, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 2);
    assert_eq!(notifier.0.lock().unwrap().len(), 4);
    drop(db);
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sqlite_repeated_completion_failures_keep_bounded_capacity_and_retry_work() {
    // Without retaining bounded ownership, each drive would accumulate more attempts.
    let (path, clock, context, store) = completion_fixture();
    for n in 1..=5 {
        add_completion_peer(&context, n);
    }
    let failing = FailingCompletionStore {
        store: &store,
        clock: clock.as_ref(),
        exhaust_budget: AtomicBool::new(false),
        failures: AtomicU64::new(100),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    };
    let notifier = CompletionNotifier(Mutex::new(vec![]));
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    for _ in 0..8 {
        let before = failing.completions.lock().unwrap().len();
        assert!(scheduler.drive_wakes(&budget).is_err());
        assert!(failing.completions.lock().unwrap().len() - before <= 5);
    }
    assert_eq!(notifier.0.lock().unwrap().len(), 4);
    let db = context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM wake_work WHERE reservation_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    let before = failing.completions.lock().unwrap().len();
    scheduler.drive_wakes(&budget).unwrap_err();
    assert_eq!(failing.completions.lock().unwrap().len() - before, 4);
    failing.exhaust_budget.store(true, Ordering::SeqCst);
    let short_budget = CallBudget {
        deadline: MonoInstant(1),
        cancellation: Cancellation::default(),
    };
    let before = failing.completions.lock().unwrap().len();
    assert!(scheduler.drive_wakes(&short_budget).is_err());
    assert_eq!(clock.monotonic_now(), MonoInstant(1));
    assert_eq!(failing.completions.lock().unwrap().len() - before, 1);
    assert_eq!(notifier.0.lock().unwrap().len(), 4);
    failing.exhaust_budget.store(false, Ordering::SeqCst);
    failing.failures.store(0, Ordering::SeqCst);
    scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(notifier.0.lock().unwrap().len(), 6);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM wake_work WHERE last_outcome='submitted' AND reservation_id IS NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 6);
    drop(db);
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}
impl HostPort for SqliteTimingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        let now = self.clock.monotonic_now();
        assert_eq!(target.as_str(), "target");
        assert_eq!(context.budget.deadline, MonoInstant(now.0 + 750));
        // A fresh read can open a writer transaction: reservation commit has
        // finished and the scheduler holds no database writer during host I/O.
        let db = self.context.open_writer().unwrap();
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        let reserved: bool = db
            .query_row(
                "SELECT reservation_id IS NOT NULL FROM wake_work WHERE seat_id='seat'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(reserved);
        db.execute_batch("ROLLBACK").unwrap();
        let mut observation = fresh_observation();
        observation.host_boot = HostBootId::new("host");
        observation.observed_at_mono = now;
        observation.started_at_mono = now;
        observation.completed_at_mono = now;
        Ok(observation)
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        FakeNativeHost {
            observation: observation.clone(),
            submitted: AtomicU64::new(0),
            pane_states: Default::default(),
            submit_keys: AtomicU64::new(0),
        }
        .safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        let now = self.clock.monotonic_now();
        assert_eq!(text, crate::notification::policy::MARKER);
        assert_eq!(context.budget.deadline, MonoInstant(now.0 + 2_000));
        self.calls.lock().unwrap().push(now);
        self.clock.mono.store(now.0 + 2_000, Ordering::SeqCst);
        Ok(PromptOutcome::Submitted)
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(
        &self,
        _: &crate::ports::SafeWakeTarget,
        _: &crate::ports::HostCallContext,
    ) -> Result<(), crate::protocol::results::ApiError> {
        Ok(())
    }
}

#[test]
fn sqlite_completed_frontier_shortens_retry_only_after_new_logical_publication() {
    let path = std::env::temp_dir().join(format!(
        "herdr-scheduler-frontier-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let context = StoreContext::new(path.clone(), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,101);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat','i','resolved','native','target',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','target','host',1,1,0,'fresh','occupied','idle','execution',1,'term-'||'target','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('seat',1,1,'target','host',1,'codex','session','execution','fresh',0,0,'term-'||'target','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread','seat','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES ('invite','thread','seat',1,'pending',0,100,100,1);\
    ").unwrap();
    for n in 1..=101 {
        let message = format!("m{n}");
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,'i','thread',?2,'ordinary','body',?2,0)", rusqlite::params![message,n]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'thread','seat',?2,100)", rusqlite::params![message,if n==101 {"pending"} else {"acked"}]).unwrap();
    }
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_invitation_seq,last_invitation_offset,last_receipt_seq,last_receipt_offset) VALUES ('seat',3,1,30000,60000,'prior',?1,1,1,101,0)", [daemon_boot().to_string()]).unwrap();
    drop(db);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(path.clone(), clock.clone()),
            "i",
            StoreSettings {
                daemon_boot: Some(daemon_boot()),
                wake_batch_delay_ms: 0,
                ..StoreSettings::default()
            },
        )
        .unwrap(),
    );
    let ports = ScheduledStore::new(store.clone(), Arc::new(FairWriter::new(32)));
    let host = SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &ports, clock.as_ref());
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &ports,
        &dispatch,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    clock.mono.store(30_000, Ordering::SeqCst);
    // The pending probe finds the one pending receipt without walking the 100
    // settled ones: the seat is examined in the first page and held by its delay.
    let first = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(
        (first.examined, first.attempted, first.has_more),
        (1, 0, false)
    );
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    // A projection/version update is not a new logical obligation.
    let db = context.open_writer().unwrap();
    db.execute(
        "UPDATE wake_work SET attention_version=attention_version+1",
        [],
    )
    .unwrap();
    drop(db);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    assert!(host.calls.lock().unwrap().is_empty());

    // This receipt is published through a manifest, without physical receipt
    // projection or any attention-version change.
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        UPDATE host_instances SET decision_seq=102 WHERE id='i';\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p','i','actor','o',zeroblob(32),'thread',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p','thread','seat',1,100,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('unprojected','i','thread',102,'ordinary','body',0,102);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p','unprojected','i','thread',102,0,102,0,1,0);\
    ").unwrap();
    drop(db);
    let published = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!((published.examined, published.attempted), (1, 1));
    assert_eq!(*host.calls.lock().unwrap(), vec![MonoInstant(30_000)]);
    let db = context.open_writer().unwrap();
    let retained: (i64,i64,i64,i64,i64) = db.query_row("SELECT last_invitation_seq,last_receipt_seq,retry_step,effective_delay_ms,attention_version FROM wake_work WHERE seat_id='seat'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(retained, (1, 102, 2, 120_000, 1));
    let pending: String = db
        .query_row(
            "SELECT state FROM receipts WHERE message_id='m101'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, "pending");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE message_id='unprojected'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("\
        UPDATE host_instances SET decision_seq=103 WHERE id='i';\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread2','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('thread2','seat','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,deadline_at,frozen_duration_ms,created_decision_seq) VALUES ('invite2','thread2','seat',1,'pending',0,100,100,103);\
    ").unwrap();
    drop(db);
    clock.utc.store(3_600_000, Ordering::SeqCst);
    clock.mono.store(61_999, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    clock.utc.store(-3_600_000, Ordering::SeqCst);
    clock.mono.store(62_000, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![MonoInstant(30_000), MonoInstant(62_000)]
    );
    drop(scheduler);
    drop(ports);
    drop(store);
    std::fs::remove_file(path).unwrap();
}
impl NotificationPort for SpacedNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        let now = self.clock.monotonic_now();
        self.calls.lock().unwrap().push(now);
        if now == MonoInstant(0) {
            self.clock.mono.store(2_000, Ordering::SeqCst);
        }
        Ok(WakeOutcome::OutcomeUnknown)
    }
}
#[test]
fn actual_second_host_call_waits_thirty_seconds_after_completion_despite_wall_jumps_and_new_work() {
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let store = SpacedWakeStore {
        clock: clock.clone(),
        reservations: AtomicU64::new(0),
        completions: AtomicU64::new(0),
    };
    let notifier = SpacedNotifier {
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        Some(WakeOutcome::OutcomeUnknown)
    );
    let mut new_work = due_candidate();
    new_work.last_reservation_id = Some(WakeAttemptId::new("attempt-0"));
    new_work.last_reservation_boot = Some(HostBootId::new("boot"));
    new_work.minimum_delay_ms = 30_000;
    new_work.effective_delay_ms = 30_000;
    new_work.has_pending_receipt = true;
    new_work.attention_witness = Some(test_witness(true, true));
    clock.utc.store(3_600_000, Ordering::SeqCst);
    clock.mono.store(31_999, Ordering::SeqCst);
    assert_eq!(runner.try_candidate(&new_work, &budget).unwrap(), None);
    clock.utc.store(-3_600_000, Ordering::SeqCst);
    clock.mono.store(32_000, Ordering::SeqCst);
    assert_eq!(
        runner.try_candidate(&new_work, &budget).unwrap(),
        Some(WakeOutcome::OutcomeUnknown)
    );
    assert_eq!(
        *notifier.calls.lock().unwrap(),
        vec![MonoInstant(0), MonoInstant(32_000)]
    );
    assert_eq!(store.reservations.load(Ordering::SeqCst), 2);
    assert_eq!(store.completions.load(Ordering::SeqCst), 2);
}

#[test]
fn later_retry_shortens_only_for_a_new_same_kind_logical_obligation() {
    let clock = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let store = SpacedWakeStore {
        clock: clock.clone(),
        reservations: AtomicU64::new(0),
        completions: AtomicU64::new(0),
    };
    let notifier = SpacedNotifier {
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let mut pending = due_candidate();
    pending.last_reservation_id = Some(WakeAttemptId::new("prior-attempt"));
    pending.last_reservation_boot = Some(HostBootId::new("boot"));
    pending.retry_step = 1;
    pending.minimum_delay_ms = 30_000;
    pending.effective_delay_ms = 60_000;
    pending.last_reserved_frontier = LogicalAttentionFrontier {
        invitation: Some(LogicalPublicationKey {
            decision_seq: 1,
            event_offset: 0,
        }),
        ..Default::default()
    };
    pending.attention_witness.as_mut().unwrap().frontier = pending.last_reserved_frontier;
    let budget = CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(runner.try_candidate(&pending, &budget).unwrap(), None);
    clock.mono.store(29_999, Ordering::SeqCst);
    assert_eq!(runner.try_candidate(&pending, &budget).unwrap(), None);
    let mut projection_only = pending.clone();
    projection_only.attention_version += 1;
    clock.mono.store(30_000, Ordering::SeqCst);
    assert_eq!(
        runner.try_candidate(&projection_only, &budget).unwrap(),
        None
    );
    // A second invitation has published, with the same reason kind and no
    // projection or attention-version change.
    let mut second_invitation = projection_only;
    second_invitation
        .attention_witness
        .as_mut()
        .unwrap()
        .decision_seq = 2;
    second_invitation
        .attention_witness
        .as_mut()
        .unwrap()
        .frontier
        .invitation = Some(LogicalPublicationKey {
        decision_seq: 2,
        event_offset: 0,
    });
    assert_eq!(
        runner.try_candidate(&second_invitation, &budget).unwrap(),
        Some(WakeOutcome::OutcomeUnknown)
    );
    assert_eq!(*notifier.calls.lock().unwrap(), vec![MonoInstant(30_000)]);
}

#[test]
fn failed_reservation_never_calls_host_and_remains_discoverable() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(true),
    };
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        scheduler.drive_wakes(&budget).unwrap_err().code,
        ErrorCode::StoreCorrupt
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve"]);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(
        *events.lock().unwrap(),
        vec!["reserve", "reserve", "host", "complete"]
    );
}

struct CancelAfterReserve<'a>(&'a FakeWakeStore);
impl WakePort for CancelAfterReserve<'_> {
    fn clock(&self) -> &dyn Clock {
        self.0.clock()
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.0.wake_recovery_candidates(page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        self.0.recover_wake_reservation(request, budget)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        self.0.wake_candidates(page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let result = self.0.reserve_wake(candidate, budget)?;
        budget.cancellation.cancel();
        Ok(result)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.0
            .complete_wake(attempt, outcome, refused_restore, budget)
    }
}

#[test]
fn cancellation_after_commit_completes_the_attempt_without_host_io() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let base = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let store = CancelAfterReserve(&base);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        Some(WakeOutcome::Cancelled)
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "complete"]);
}

#[test]
fn settled_and_already_offered_work_never_reserves_or_prompts() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    let mut candidate = due_candidate();
    candidate.has_pending_invitation = false;
    candidate.has_pending_receipt = false;
    assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
    candidate.actionable_warning_generation = Some(1);
    candidate.actionable_warning_seq = Some(9);
    candidate.warning_offer = Some(WarningOfferFrontier {
        generation: 1,
        execution: ExecutionId::new("execution"),
        offered_through_seq: 9,
    });
    assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
    assert!(events.lock().unwrap().is_empty());
}

struct FakeReservationCheck {
    current: bool,
    calls: AtomicU64,
}
impl ReservationCheck for FakeReservationCheck {
    fn is_current(&self, _: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.current)
    }
}
struct EpisodeReservationCheck {
    episode: u64,
}
impl ReservationCheck for EpisodeReservationCheck {
    fn is_current(&self, reservation: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        Ok(reservation
            .attention_witness
            .valid_at("i", &reservation.seat, 1, self.episode, false))
    }
}
struct FakeNativeHost {
    observation: HostObservation,
    submitted: AtomicU64,
    /// Submit-key retries sent.
    submit_keys: AtomicU64,
    /// Scripted composer states per target; the last entry repeats, and an
    /// unscripted target reads `Submitted`.
    pane_states: Mutex<
        std::collections::HashMap<
            HostTargetId,
            std::collections::VecDeque<crate::ports::AgentComposerState>,
        >,
    >,
}
impl FakeNativeHost {
    fn script_pane_states(
        &self,
        target: &SafeWakeTarget,
        states: Vec<crate::ports::AgentComposerState>,
    ) {
        self.pane_states
            .lock()
            .unwrap()
            .insert(target.target.clone(), states.into());
    }
}
impl HostPort for FakeNativeHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        assert_eq!(context.budget.deadline, MonoInstant(750));
        Ok(self.observation.clone())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: TerminalId::new("terminal"),
            incarnation: "incarnation".into(),
            basis: crate::ports::WakeTargetBasis::VerifiedOccupant {
                session: NativeSessionId::new("session"),
                execution: ExecutionId::new("execution"),
            },
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        assert_eq!(text, crate::notification::policy::MARKER);
        assert_eq!(context.budget.deadline, MonoInstant(2_000));
        self.submitted.fetch_add(1, Ordering::SeqCst);
        Ok(PromptOutcome::Submitted)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        let mut scripted = self.pane_states.lock().unwrap();
        let Some(queue) = scripted.get_mut(&target.target) else {
            return Ok(crate::ports::AgentComposerState::Submitted);
        };
        Ok(if queue.len() > 1 {
            queue.pop_front().unwrap()
        } else {
            queue
                .front()
                .copied()
                .unwrap_or(crate::ports::AgentComposerState::Submitted)
        })
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        self.submit_keys.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn fresh_observation() -> HostObservation {
    HostObservation {
        focused: false,
        target: HostTargetId::new("target"),
        host_boot: HostBootId::new("boot"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(0),
        observed_at_mono: MonoInstant(0),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: Some(NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("session"),
            execution: ExecutionId::new("execution"),
            is_top_level: true,
        }),
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("terminal")),
        occupancy: StructuralOccupancy::Occupied,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Verified {
            execution: ExecutionId::new("execution"),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(0),
        completed_at_mono: MonoInstant(0),
    }
}
fn test_reservation() -> WakeReservation {
    WakeReservation {
        attempt: WakeAttemptId::new("attempt"),
        seat: SeatId::new("seat"),
        attention_witness: test_witness(true, false),
        reasons: vec!["invitation".into()],
        retained_effective_delay_ms: 30_000,
        lease_until: MonoInstant(5_000),
        retained_minimum_delay_ms: 30_000,
        reserved_at_utc: UtcMillis(0),
        daemon_boot: daemon_boot(),
        host_boot: HostBootId::new("boot"),
        host_epoch: 1,
        target: HostTargetId::new("target"),
        target_generation: 1,
        authority: ReservedWakeAuthority::RecoveryHint {
            execution: ExecutionId::new("execution"),
        },
    }
}

#[test]
fn native_dispatch_rejects_changed_target_and_retirement_before_prompt() {
    let clock = FakeClock(AtomicU64::new(0));
    let mut observation = fresh_observation();
    observation.generation = 2;
    let host = FakeNativeHost {
        observation,
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
    assert_eq!(check.calls.load(Ordering::SeqCst), 0);

    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: false,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
    assert_eq!(check.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn closed_attention_episode_rejects_prompt_at_final_fence() {
    let clock = FakeClock(AtomicU64::new(0));
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = EpisodeReservationCheck { episode: 1 };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
}

#[test]
fn safe_unregistered_recovery_hint_only_submits_the_fixed_marker() {
    let clock = FakeClock(AtomicU64::new(0));
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Submitted
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 1);
    assert_eq!(check.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn blocked_or_human_input_target_is_never_prompted() {
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    for ui in [
        HostUiState::ApprovalOrQuestion,
        HostUiState::HumanInput,
        HostUiState::Unknown,
        HostUiState::ActiveTurn,
    ] {
        let mut observation = fresh_observation();
        observation.ui = ui;
        let host = FakeNativeHost {
            observation,
            submitted: AtomicU64::new(0),
            pane_states: Default::default(),
            submit_keys: AtomicU64::new(0),
        };
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
        assert_eq!(
            dispatch.attempt_wake(test_reservation(), &context).unwrap(),
            WakeOutcome::Refused(RefusalCause::Unsafe)
        );
        assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn adapter_safe_target_must_preserve_fresh_incarnation() {
    let clock = FakeClock(AtomicU64::new(0));
    let mut observation = fresh_observation();
    observation.incarnation = IncarnationEvidence::Verified {
        identity: "successor-incarnation".into(),
        evidence_kind: EvidenceKind::NativeCurrentTarget,
    };
    let host = FakeNativeHost {
        observation,
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
}

struct LateReadHost {
    inner: FakeNativeHost,
    clock: Arc<FakeClock>,
}
impl HostPort for LateReadHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.inner.native_launch_capability()
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        let observation = self.inner.observe_current_target(target, context)?;
        self.clock.0.store(750, Ordering::SeqCst);
        Ok(observation)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(context)
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.inner.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.inner.submit_prompt(target, text, context)
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.send_submit_key(target, context)
    }
}

#[test]
fn target_read_must_finish_before_its_seven_hundred_fifty_millisecond_deadline() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let host = LateReadHost {
        inner: FakeNativeHost {
            observation: fresh_observation(),
            submitted: AtomicU64::new(0),
            pane_states: Default::default(),
            submit_keys: AtomicU64::new(0),
        },
        clock: clock.clone(),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, clock.as_ref());
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::TimedOut)
    );
    assert_eq!(host.inner.submitted.load(Ordering::SeqCst), 0);
    assert_eq!(check.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cached_current_target_response_does_not_authorize_a_prompt() {
    let clock = FakeClock(AtomicU64::new(0));
    let mut observation = fresh_observation();
    observation.provenance = ObservationProvenance::UncharacterizedCache;
    let host = FakeNativeHost {
        observation,
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
}

/// A fake Herdr 0.9.1 API endpoint for the production NativeCli: one
/// ping+operation exchange per connection, answering `pane.get`,
/// `agent.get` (with the current agent status, or `agent_not_found` for a
/// shell) and `agent.prompt`, recording every operation and prompt text.
#[cfg(target_os = "macos")]
struct FakeHerdr {
    socket: std::path::PathBuf,
    status: Arc<Mutex<&'static str>>,
    methods: Arc<Mutex<Vec<String>>>,
    prompts: Arc<Mutex<Vec<String>>>,
}
#[cfg(target_os = "macos")]
impl FakeHerdr {
    fn start(status: &'static str) -> Self {
        use std::io::{BufRead, BufReader, Write};
        let socket = std::env::temp_dir().join(format!("ht-wake-{}", uuid::Uuid::new_v4()));
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let status = Arc::new(Mutex::new(status));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let (shared_status, shared_methods, shared_prompts) =
            (status.clone(), methods.clone(), prompts.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default().to_owned();
                let current = *shared_status.lock().unwrap();
                let agent = serde_json::json!({"agent":"claude","agent_status":current,
                    "pane_id":"w4:p1","terminal_id":"term_1","tab_id":"w4:t1",
                    "workspace_id":"w4","focused":false,"revision":2});
                let response = match method.as_str() {
                    "ping" => serde_json::json!({"type":"pong","version":"0.9.1","protocol":22}),
                    "pane.get" => serde_json::json!({"type":"pane_info","pane":{"pane_id":"w4:p1",
                        "terminal_id":"term_1","workspace_id":"w4","tab_id":"w4:t1",
                        "focused":false,"revision":2,
                        "agent_status":if current == "shell" { "unknown" } else { current }}}),
                    "agent.get" if current == "shell" => {
                        shared_methods.lock().unwrap().push(method);
                        let _ = writeln!(
                            stream,
                            "{}",
                            serde_json::json!({"id":request["id"],
                            "error":{"code":"agent_not_found","message":"agent target w4:p1 not found"}})
                        );
                        continue;
                    }
                    "agent.get" => serde_json::json!({"type":"agent_info","agent":agent}),
                    "agent.prompt" => {
                        shared_prompts.lock().unwrap().push(
                            request["params"]["text"]
                                .as_str()
                                .unwrap_or_default()
                                .into(),
                        );
                        serde_json::json!({"type":"agent_prompted","agent":agent})
                    }
                    _ => continue,
                };
                if method != "ping" {
                    shared_methods.lock().unwrap().push(method);
                }
                let _ = writeln!(
                    stream,
                    "{}",
                    serde_json::json!({"id":request["id"],"result":response})
                );
            }
        });
        Self {
            socket,
            status,
            methods,
            prompts,
        }
    }
}

/// Scheduler -> SQLite store -> NativeWakeDispatcher -> production NativeCli
/// -> Herdr API, under the cooperative native policy. A registered
/// cooperative seat (self-reported execution, older host epoch, same
/// terminal and verified incarnation) misses a receipt deadline: the due scan
/// writes one durable warning and the wake lane submits exactly one
/// coalesced prompt, only after the fresh agent recheck. The minimum spacing
/// then holds the seat, and a working or shell target is never prompted.
#[cfg(target_os = "macos")]
#[test]
fn cooperative_overdue_warning_wakes_idle_native_agent_exactly_once() {
    for (status, expect_prompt) in [
        ("idle", true),
        ("done", true),
        ("working", false),
        ("blocked", false),
        ("shell", false),
    ] {
        let herdr = FakeHerdr::start(status);
        let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::new());
        let cli = crate::host::native::NativeCli::new(herdr.socket.clone(), clock.clone());
        let learn = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
                cancellation: Cancellation::default(),
            },
            expected_boot: None,
            expected_epoch: None,
        };
        let observed = cli
            .observe_current_target(&HostTargetId::new("w4:p1"), &learn)
            .unwrap();
        let IncarnationEvidence::Verified { identity, .. } = &observed.incarnation else {
            panic!("the kernel peer witness verifies the fake server");
        };
        herdr.methods.lock().unwrap().clear();
        let path =
            std::env::temp_dir().join(format!("herdr-coop-wake-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,?1,1,10)", [observed.host_boot.as_str()]).unwrap();
        db.execute_batch("\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('a','i','resolved','native','w4:p9',1,1,0);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat','i','resolved','native','w4:p1',1,1,0);\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2);\
            INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','a','joined',0);\
            INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','seat','joined',0);\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('m','i','t',1,'ordinary','a','handoff',0,5);\
            INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m','t','seat','pending',500,0,500);\
        ").unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','w4:p1',?1,1,1,0,'fresh','term_1',?2,'native_current_target',1)", rusqlite::params![observed.host_boot.as_str(), identity]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('seat',1,1,'w4:p1',?1,0,'claude','session','self-reported','cooperative_top_level',0,0,'term_1',?2)", rusqlite::params![observed.host_boot.as_str(), identity]).unwrap();
        drop(db);
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), clock.clone()),
                "i",
                StoreSettings {
                    daemon_boot: Some(daemon_boot()),
                    wake_batch_delay_ms: 0,
                    ..StoreSettings::default()
                },
            )
            .unwrap(),
        );
        let ports = ScheduledStore::new(store.clone(), Arc::new(FairWriter::new(32)));
        let dispatch = NativeWakeDispatcher::new(&cli, &ports, clock.as_ref());
        let scheduler = Scheduler::new(
            "i".into(),
            &ports,
            &ports,
            &dispatch,
            RetryConfig::default(),
            daemon_boot(),
        );
        let budget = || CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        };
        let deadlines = scheduler.drive_deadlines(&budget()).unwrap();
        let db = context.open_writer().unwrap();
        let warnings: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE kind='warn' AND source_message_id='m'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(warnings, 1, "{status}: one durable warning ({deadlines:?})");
        let mut attempted = 0;
        for _ in 0..3 {
            attempted += scheduler.drive_wakes(&budget()).unwrap().attempted;
        }
        let prompts = herdr.prompts.lock().unwrap().clone();
        let methods = herdr.methods.lock().unwrap().clone();
        let (outcome, covered): (Option<String>, bool) = db
            .query_row("SELECT w.last_outcome,w.last_warning_seq>=m.decision_seq FROM wake_work w, messages m WHERE w.seat_id='seat' AND m.kind='warn' AND m.source_message_id='m'", [], |r| Ok((r.get(0)?, r.get::<_, Option<bool>>(1)?.unwrap_or(false))))
            .unwrap();
        assert_eq!(attempted, 1, "{status}: spacing admits one attempt");
        if expect_prompt {
            assert_eq!(prompts, [crate::notification::policy::MARKER], "{status}");
            assert_eq!(
                methods,
                ["pane.get", "agent.get", "agent.prompt"],
                "{status}"
            );
            assert_eq!(outcome.as_deref(), Some("submitted"), "{status}");
        } else {
            assert!(prompts.is_empty(), "{status} must never be prompted");
            assert_eq!(methods, ["pane.get", "agent.get"], "{status}");
            assert_eq!(outcome.as_deref(), Some("unsafe"), "{status}");
        }
        assert!(
            covered,
            "{status}: the one attempt covers the warning (coalesced)"
        );
        // Prompt success is transport only: the receipt stays pending.
        let receipt: String = db
            .query_row("SELECT state FROM receipts WHERE message_id='m'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(receipt, "pending", "{status}");
        *herdr.status.lock().unwrap() = "idle";
        drop(db);
        drop(scheduler);
        drop(ports);
        drop(store);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(&herdr.socket);
    }
}

/// The cooperative dispatcher branch rechecks structure only and never
/// crosses bases: blocked/active UI, an empty shell, a replaced terminal, a
/// different incarnation or a verified execution refuse before the adapter,
/// and a verified-occupant target never satisfies a cooperative reservation.
#[test]
fn cooperative_reservation_rechecks_structure_and_never_crosses_bases() {
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    let mut reservation = test_reservation();
    reservation.authority = ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation: None,
        harness: Some("claude".into()),
    };
    let cooperative = || {
        let mut observation = fresh_observation();
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        observation
    };
    type Change = Box<dyn Fn(&mut HostObservation)>;
    let changes: Vec<(&str, Change)> = vec![
        (
            "approval",
            Box::new(|o| o.ui = HostUiState::ApprovalOrQuestion),
        ),
        ("active turn", Box::new(|o| o.ui = HostUiState::ActiveTurn)),
        (
            "empty shell",
            Box::new(|o| o.occupancy = StructuralOccupancy::EmptyShell),
        ),
        (
            "terminal",
            Box::new(|o| o.terminal = Some(TerminalId::new("other"))),
        ),
        (
            "cache",
            Box::new(|o| o.provenance = ObservationProvenance::UncharacterizedCache),
        ),
        (
            "incarnation",
            Box::new(|o| {
                o.incarnation = IncarnationEvidence::Verified {
                    identity: "other".into(),
                    evidence_kind: EvidenceKind::NativeCurrentTarget,
                }
            }),
        ),
        ("verified execution", Box::new(|o| *o = fresh_observation())),
        ("unchanged, verified basis", Box::new(|_| {})),
    ];
    for (label, change) in changes {
        let mut observation = cooperative();
        change(&mut observation);
        let host = FakeNativeHost {
            observation,
            submitted: AtomicU64::new(0),
            pane_states: Default::default(),
            submit_keys: AtomicU64::new(0),
        };
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
        assert_eq!(
            dispatch
                .attempt_wake(reservation.clone(), &context)
                .unwrap(),
            WakeOutcome::Refused(RefusalCause::Unsafe),
            "{label}"
        );
        assert_eq!(host.submitted.load(Ordering::SeqCst), 0, "{label}");
        assert_eq!(check.calls.load(Ordering::SeqCst), 0, "{label}");
    }
}

/// Drives one wake through Scheduler -> NativeWakeDispatcher -> scripted host
/// and returns (verification, submit-key sends, prompt sends, reserve count,
/// second-drive attempts).
fn drive_scripted_wake(
    states: Vec<crate::ports::AgentComposerState>,
) -> (
    Vec<(SeatId, crate::scheduler::SubmissionVerification)>,
    u64,
    u64,
    usize,
    u16,
) {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let wake = FakeWakeStore {
        clock: clock.clone(),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let due = FakeDeadlinePort {
        clock: clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        submit_keys: AtomicU64::new(0),
        pane_states: Default::default(),
    };
    host.script_pane_states(
        &host
            .safe_wake_target(&SeatId::new("seat"), &host.observation)
            .unwrap(),
        states,
    );
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, clock.as_ref());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &wake,
        &dispatch,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    let first = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(first.attempted, 1);
    let second = scheduler.drive_wakes(&budget).unwrap();
    let reserves = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| **event == "reserve")
        .count();
    (
        first.verification,
        host.submit_keys.load(Ordering::SeqCst),
        host.submitted.load(Ordering::SeqCst),
        reserves,
        second.attempted,
    )
}

#[test]
fn unsent_prompt_gets_exactly_one_submit_key_retry_then_verified() {
    use crate::ports::AgentComposerState::{HoldingPrompt, Submitted};
    let (verification, keys, prompts, _, _) = drive_scripted_wake(vec![HoldingPrompt, Submitted]);
    assert_eq!(
        verification,
        vec![(
            SeatId::new("seat"),
            crate::scheduler::SubmissionVerification::Retried
        )]
    );
    assert_eq!((keys, prompts), (1, 1));
    assert_eq!(
        crate::scheduler::outcome_for_verification(verification[0].1),
        WakeOutcome::Submitted
    );
}

#[test]
fn already_submitted_prompt_gets_no_retry() {
    use crate::ports::AgentComposerState::Submitted;
    let (verification, keys, prompts, _, _) = drive_scripted_wake(vec![Submitted]);
    assert_eq!(
        verification,
        vec![(
            SeatId::new("seat"),
            crate::scheduler::SubmissionVerification::Verified
        )]
    );
    assert_eq!((keys, prompts), (0, 1));
}

#[test]
fn still_unsent_after_retry_is_reported_not_looped() {
    use crate::ports::AgentComposerState::HoldingPrompt;
    let (verification, keys, prompts, reserves, second_attempts) =
        drive_scripted_wake(vec![HoldingPrompt, HoldingPrompt]);
    assert_eq!(
        verification,
        vec![(
            SeatId::new("seat"),
            crate::scheduler::SubmissionVerification::Unsubmitted
        )]
    );
    // One retry only; the second drive pass neither reserves nor re-sends.
    assert_eq!((keys, prompts, reserves, second_attempts), (1, 1, 1, 0));
    assert_eq!(
        crate::scheduler::outcome_for_verification(verification[0].1),
        WakeOutcome::OutcomeUnknown
    );
}

#[test]
fn unsent_after_retry_completes_the_attempt_as_outcome_unknown() {
    let clock = FakeClock(AtomicU64::new(0));
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        submit_keys: AtomicU64::new(0),
        pane_states: Default::default(),
    };
    host.script_pane_states(
        &host
            .safe_wake_target(&SeatId::new("seat"), &host.observation)
            .unwrap(),
        vec![crate::ports::AgentComposerState::HoldingPrompt],
    );
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(2_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::OutcomeUnknown
    );
    assert_eq!(host.submit_keys.load(Ordering::SeqCst), 1);
    assert_eq!(
        dispatch.take_verification(&SeatId::new("seat")),
        Some(crate::scheduler::SubmissionVerification::Unsubmitted)
    );
    assert_eq!(dispatch.take_verification(&SeatId::new("seat")), None);
}

// ---- ht-p03.9.3: pre-send refusals retry on backoff, ladder untouched ----

/// Plays a fixed script of outcomes and records when each attempt ran. `hook`
/// runs inside the attempt, after the reservation committed and before the
/// outcome is returned.
struct ScriptedNotifier {
    clock: Arc<JumpClock>,
    script: Mutex<std::collections::VecDeque<Result<WakeOutcome, ApiError>>>,
    calls: Mutex<Vec<MonoInstant>>,
    hook: Mutex<Option<Box<dyn Fn() + Send>>>,
}
impl ScriptedNotifier {
    fn new(clock: &Arc<JumpClock>, script: Vec<Result<WakeOutcome, ApiError>>) -> Self {
        Self {
            clock: clock.clone(),
            script: Mutex::new(script.into()),
            calls: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
        }
    }
    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}
impl NotificationPort for ScriptedNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.calls.lock().unwrap().push(self.clock.monotonic_now());
        if let Some(hook) = self.hook.lock().unwrap().as_ref() {
            hook();
        }
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(WakeOutcome::Submitted))
    }
}

/// `completion_fixture` with a prior ladder row (step 1, 60 s).
fn refusal_fixture() -> (
    std::path::PathBuf,
    Arc<JumpClock>,
    StoreContext,
    SqliteStore,
) {
    let (path, clock, context, store) = completion_fixture();
    let db = context.open_writer().unwrap();
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot) VALUES ('seat',3,1,30000,60000,'prior',?1)", [daemon_boot().to_string()]).unwrap();
    drop(db);
    (path, clock, context, store)
}
/// The runner anchors its boot guard when the scheduler is built, so the
/// clock moves past the prior step's 60 s only after that.
fn start_due(clock: &JumpClock) {
    clock.mono.store(60_000, Ordering::SeqCst);
}
type RefusalRow = (Option<String>, i64, i64, Option<String>, Option<String>);
fn refusal_row(context: &StoreContext) -> RefusalRow {
    let db = context.open_writer().unwrap();
    db.query_row("SELECT reservation_id,retry_step,effective_delay_ms,last_reservation_id,last_outcome FROM wake_work WHERE seat_id='seat'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap()
}
fn refusal_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000_000),
        cancellation: Cancellation::default(),
    }
}
fn failing_store<'a>(store: &'a SqliteStore, clock: &'a JumpClock) -> FailingCompletionStore<'a> {
    FailingCompletionStore {
        store,
        clock,
        exhaust_budget: AtomicBool::new(false),
        failures: AtomicU64::new(0),
        lose_response: AtomicBool::new(false),
        completions: Mutex::new(vec![]),
    }
}
fn refusal_due_port() -> FakeDeadlinePort {
    FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    }
}
fn refusal_attempts(scheduler_state: &std::sync::MutexGuard<'_, super::WakeRunnerState>) -> u32 {
    scheduler_state
        .dispatch
        .refusal_attempts(&SeatId::new("seat"))
        .unwrap()
}

#[test]
fn refusals_back_off_without_climbing_the_ladder() {
    // Kills: refusals that climb 30 s -> 300 s (step advances), retries that
    // ignore the 100 ms x 2^n schedule, and a Submitted that fails to advance
    // the ladder exactly one step after the refusals.
    let (path, clock, context, store) = refusal_fixture();
    let failing = failing_store(&store, clock.as_ref());
    let notifier = ScriptedNotifier::new(
        &clock,
        vec![Ok(WakeOutcome::Refused(RefusalCause::Unavailable)); 4],
    );
    let due = refusal_due_port();
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    start_due(&clock);
    let budget = refusal_budget();
    let mut gaps = Vec::new();
    for _ in 0..4 {
        let attempted_at = clock.monotonic_now();
        let drove = scheduler.drive_wakes(&budget).unwrap();
        assert_eq!(drove.attempted, 1, "{drove:?}");
        let due_at = drove.next_due_at.expect("a refusal reports its retry");
        gaps.push(due_at.0 - attempted_at.0);
        // Refused: step, delay and last reservation are the prior row's.
        assert_eq!(
            refusal_row(&context),
            (
                None,
                1,
                60_000,
                Some("prior".into()),
                Some("unavailable".into())
            )
        );
        // Not retried one millisecond early, retried at the reported instant.
        let before = notifier.calls();
        clock.mono.store(due_at.0 - 1, Ordering::SeqCst);
        assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
        assert_eq!(notifier.calls(), before);
        clock.mono.store(due_at.0, Ordering::SeqCst);
    }
    for (n, gap) in gaps.iter().enumerate() {
        let nominal = 100u64 << n;
        assert!(
            *gap * 5 >= nominal * 4 && *gap * 5 <= nominal * 6,
            "refusal {n}: gap {gap} ms vs nominal {nominal} ms"
        );
    }
    // The fifth attempt is accepted: exactly one ladder step.
    let drove = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(drove.attempted, 1);
    let row = refusal_row(&context);
    assert_eq!(
        (row.0.clone(), row.1, row.2, row.4),
        (None, 2, 120_000, Some("submitted".into()))
    );
    assert_ne!(row.3.as_deref(), Some("prior"), "a new reservation id");
    assert_eq!(notifier.calls(), 5);
    {
        let state = scheduler.wakes.state.lock().unwrap();
        assert_eq!(refusal_attempts(&state), 0, "Submitted resets the backoff");
    }
    // Ladder after Submitted: the next attempt waits the advanced 120 s step.
    let submitted_at = clock.monotonic_now().0;
    assert!(drove.next_due_at.unwrap().0 > submitted_at);
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

/// A FakeWakeStore whose fenced restore always matches when one is supplied.
struct MatchedRestore<'a>(&'a FakeWakeStore, Mutex<Vec<bool>>);
impl WakePort for MatchedRestore<'_> {
    fn clock(&self) -> &dyn Clock {
        self.0.clock()
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.0.wake_recovery_candidates(page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        self.0.recover_wake_reservation(request, budget)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        self.0.wake_candidates(page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.0.reserve_wake(candidate, budget)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.0
            .complete_wake(attempt, outcome, refused_restore, budget)?;
        self.1.lock().unwrap().push(refused_restore.is_some());
        Ok(refused_restore.is_some())
    }
}

/// A wake port whose candidate listing can be emptied, as when the seat's
/// attention was settled elsewhere between two passes.
struct Unlisting<'a, P: WakePort>(&'a P, AtomicBool);
impl<P: WakePort> WakePort for Unlisting<'_, P> {
    fn clock(&self) -> &dyn Clock {
        self.0.clock()
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        let mut listed = self.0.wake_candidates(page, budget)?;
        if self.1.load(Ordering::SeqCst) {
            listed.items.clear();
        }
        Ok(listed)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.0.reserve_wake(candidate, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.0.wake_recovery_candidates(page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        self.0.recover_wake_reservation(request, budget)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.0
            .complete_wake(attempt, outcome, refused_restore, budget)
    }
}

#[test]
fn refusal_backoff_of_a_seat_that_left_the_listing_stops_reporting_a_due_time() {
    // Kills: a refusal backoff that outlives the seat's wake work. Its retry
    // time stays in the past, `next_due_at` keeps reporting it, and the wake
    // lane re-passes at its 100 ms minimum wait for as long as the daemon runs.
    let (path, clock, _context, store) = refusal_fixture();
    let failing = failing_store(&store, clock.as_ref());
    let unlisting = Unlisting(&failing, AtomicBool::new(false));
    let notifier = ScriptedNotifier::new(
        &clock,
        vec![Ok(WakeOutcome::Refused(RefusalCause::Unavailable))],
    );
    let due = refusal_due_port();
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &unlisting,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    start_due(&clock);
    let budget = refusal_budget();
    let refused = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(refused.attempted, 1, "{refused:?}");
    let retry_at = refused.next_due_at.expect("a refusal reports its retry");
    // The seat leaves the listing and its retry time passes.
    unlisting.1.store(true, Ordering::SeqCst);
    clock.mono.store(retry_at.0 + 1_000, Ordering::SeqCst);
    let idle = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(idle.attempted, 0, "{idle:?}");
    assert_eq!(
        idle.next_due_at, None,
        "a seat nothing lists has no retry to wait for"
    );
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn refused_warning_wake_is_retried_not_offered() {
    // Kills: treating a refused warning wake as an offer (the work would drop
    // out of selection and never be retried).
    let mut candidate = due_candidate();
    candidate.has_pending_invitation = false;
    candidate.actionable_warning_generation = Some(1);
    candidate.actionable_warning_seq = Some(5);
    candidate.attention_witness = Some(WakeAttentionWitness::from_complete(
        "i".into(),
        SeatId::new("seat"),
        1,
        false,
        false,
        Some(5),
        0,
        false,
        Default::default(),
    ));
    assert!(candidate.has_actionable_work());
    let events = Arc::new(Mutex::new(Vec::new()));
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let base = FakeWakeStore {
        clock: clock.clone(),
        events,
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let store = MatchedRestore(&base, Mutex::new(vec![]));
    let jump = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let notifier = ScriptedNotifier::new(
        &jump,
        vec![
            Ok(WakeOutcome::Refused(RefusalCause::Unsafe)),
            Ok(WakeOutcome::Submitted),
        ],
    );
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = refusal_budget();
    assert_eq!(
        runner.try_candidate(&candidate, &budget).unwrap(),
        Some(WakeOutcome::Refused(RefusalCause::Unsafe))
    );
    // A refusal is not an offer: the selection still wants this warning.
    assert!(!candidate.warning_offered_for_current_occupant());
    assert!(candidate.has_actionable_work());
    // Backing off: no immediate retry, then one at the reported instant.
    assert_eq!(runner.try_candidate(&candidate, &budget).unwrap(), None);
    let due = runner
        .next_due_at()
        .unwrap()
        .expect("refusal retry instant");
    assert!((80..=120).contains(&due.0), "100 ms +-20 %: {due:?}");
    clock.0.store(due.0, Ordering::SeqCst);
    assert_eq!(
        runner.try_candidate(&candidate, &budget).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(notifier.calls(), 2);
    assert_eq!(*store.1.lock().unwrap(), vec![true, false]);
}

#[test]
fn submit_prompt_error_keeps_todays_mapping() {
    // Kills: routing a submit_prompt error through the refusal path (the send
    // may have happened, so the ladder must stay advanced and the refusal
    // backoff untouched).
    struct SubmitErrHost(SqliteTimingHost);
    impl HostPort for SubmitErrHost {
        fn observe_current_target_for_archival(
            &self,
            _: &crate::protocol::ids::HostTargetId,
            _: &crate::ports::HostCallContext,
        ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
            Err(crate::protocol::results::ApiError::unsupported(
                "test adapter has no composer-aware archival observation",
            ))
        }
        fn send_submit_key(
            &self,
            target: &SafeWakeTarget,
            ctx: &HostCallContext,
        ) -> Result<(), ApiError> {
            self.0.send_submit_key(target, ctx)
        }
        fn native_launch_capability(&self) -> NativeLaunchCapability {
            self.0.native_launch_capability()
        }
        fn observe_current_target(
            &self,
            target: &HostTargetId,
            context: &HostCallContext,
        ) -> Result<HostObservation, ApiError> {
            self.0.observe_current_target(target, context)
        }
        fn enumerate_targets(&self, c: &HostCallContext) -> Result<HostSnapshot, ApiError> {
            self.0.enumerate_targets(c)
        }
        fn safe_wake_target(
            &self,
            seat: &SeatId,
            observation: &HostObservation,
        ) -> Option<SafeWakeTarget> {
            self.0.safe_wake_target(seat, observation)
        }
        fn submit_prompt(
            &self,
            _: &SafeWakeTarget,
            _: &str,
            _: &HostCallContext,
        ) -> Result<PromptOutcome, ApiError> {
            Err(ApiError::new(ErrorCode::HostUnavailable, "pane went away"))
        }
        fn pane_agent_state(
            &self,
            target: &SafeWakeTarget,
            context: &HostCallContext,
        ) -> Result<crate::ports::AgentComposerState, ApiError> {
            self.0.pane_agent_state(target, context)
        }
        fn launch_native(
            &self,
            request: NativeLaunchRequest,
            context: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            self.0.launch_native(request, context)
        }
    }
    let (path, clock, context, store) = refusal_fixture();
    let failing = failing_store(&store, clock.as_ref());
    let host = SubmitErrHost(SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(vec![]),
    });
    let fence = StoreFence(&store);
    let notifier = NativeWakeDispatcher::new(&host, &fence, clock.as_ref());
    let due = refusal_due_port();
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    start_due(&clock);
    assert_eq!(
        scheduler.drive_wakes(&refusal_budget()).unwrap().attempted,
        1
    );
    let row = refusal_row(&context);
    assert_eq!(
        (row.0, row.1, row.2, row.4),
        (None, 2, 120_000, Some("unavailable".into())),
        "the existing Unavailable mapping, ladder advanced"
    );
    assert_eq!(
        failing.completions.lock().unwrap()[0].1,
        WakeOutcome::Unavailable
    );
    {
        let state = scheduler.wakes.state.lock().unwrap();
        assert_eq!(refusal_attempts(&state), 0);
    }
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn crash_between_reserve_and_complete_keeps_the_advanced_step() {
    // Kills: a restore that runs at reservation or recovery time. A crash
    // after reserve leaves the advanced step; abandoned recovery settles the
    // attempt as outcome_unknown without moving it back.
    let (path, clock, context, fixture_store) = refusal_fixture();
    drop(fixture_store);
    // Failpoints are process-global and keyed by boot: use a boot no other
    // test shares.
    let crash_boot = uuid::Uuid::new_v4();
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings {
            daemon_boot: Some(crash_boot),
            wake_batch_delay_ms: 0,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let failing = failing_store(&store, clock.as_ref());
    let notifier = ScriptedNotifier::new(&clock, vec![]);
    let due = refusal_due_port();
    {
        let scheduler = Scheduler::new(
            "i".into(),
            &due,
            &failing,
            &notifier,
            RetryConfig::default(),
            crash_boot,
        );
        start_due(&clock);
        let _fp = crate::test_support::failpoints::Failpoint::error(
            "wake.after_reservation",
            crash_boot.to_string(),
            ErrorCode::Cancelled,
        );
        assert!(scheduler.drive_wakes(&refusal_budget()).is_err());
        assert_eq!(_fp.fired(), 1);
    }
    assert_eq!(notifier.calls(), 0);
    let row = refusal_row(&context);
    assert!(row.0.is_some(), "reservation committed before the crash");
    assert_eq!((row.1, row.2), (2, 120_000));
    drop(store);
    // New daemon boot: recovery settles the abandoned attempt, step unchanged.
    clock.mono.store(100_000, Ordering::SeqCst);
    let recovery_boot = uuid::Uuid::new_v4();
    let reopened = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings {
            daemon_boot: Some(recovery_boot),
            wake_batch_delay_ms: 0,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let failing = failing_store(&reopened, clock.as_ref());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        recovery_boot,
    );
    assert_eq!(
        scheduler.drive_wakes(&refusal_budget()).unwrap().recovered,
        1
    );
    let row = refusal_row(&context);
    assert_eq!(
        (row.0, row.1, row.2, row.4),
        (None, 2, 120_000, Some("outcome_unknown".into()))
    );
    drop(scheduler);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn next_due_at_reports_the_earliest_refusal_retry() {
    // Kills: a next_due_at that is None, ignores the refusal instant, or
    // reports a later seat's instant.
    let events = Arc::new(Mutex::new(Vec::new()));
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events,
        batch: None,
        fail_reservation: AtomicBool::new(false),
    };
    let store = MatchedRestore(&store, Mutex::new(vec![]));
    let jump = Arc::new(JumpClock {
        mono: AtomicU64::new(0),
        utc: AtomicI64::new(0),
    });
    let notifier = ScriptedNotifier::new(
        &jump,
        vec![
            Ok(WakeOutcome::Refused(RefusalCause::Unavailable)),
            Ok(WakeOutcome::Refused(RefusalCause::Unavailable)),
        ],
    );
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = refusal_budget();
    assert_eq!(runner.next_due_at().unwrap(), None, "nothing tracked yet");
    runner.try_candidate(&due_candidate(), &budget).unwrap();
    let first = runner.next_due_at().unwrap().expect("first refusal");
    assert!((80..=120).contains(&first.0), "{first:?}");
    // Second refusal at the first retry instant: 200 ms +-20 % later.
    clock.0.store(first.0, Ordering::SeqCst);
    runner.try_candidate(&due_candidate(), &budget).unwrap();
    let second = runner.next_due_at().unwrap().expect("second refusal");
    let gap = second.0 - first.0;
    assert!((160..=240).contains(&gap), "{gap}");
}

#[test]
fn fence_miss_restores_nothing_and_keeps_memory_consistent() {
    // Design roast r1 (ht-p03.56). A host invalidation clears reservation_id
    // while the attempt is in flight; the Refused completion's fenced restore
    // matches 0 rows. Kills: restoring the in-memory RetryGuard anyway (memory
    // would be eligible while the durable step is advanced) and an error on
    // the 0-row restore.
    let (path, clock, context, store) = refusal_fixture();
    let failing = failing_store(&store, clock.as_ref());
    let notifier = ScriptedNotifier::new(
        &clock,
        vec![
            Ok(WakeOutcome::Refused(RefusalCause::Unavailable)),
            Ok(WakeOutcome::Submitted),
        ],
    );
    let invalidating = StoreContext::new(path.clone(), clock.clone());
    *notifier.hook.lock().unwrap() = Some(Box::new(move || {
        // The mark-unresolved path's wake_work write.
        invalidating
            .open_writer()
            .unwrap()
            .execute(
                "UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,binding_generation=NULL WHERE seat_id='seat'",
                [],
            )
            .unwrap();
    }));
    let due = refusal_due_port();
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    start_due(&clock);
    let budget = refusal_budget();
    let drove = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(drove.attempted, 1);
    // No error, and the durable step stays advanced by exactly one.
    let row = refusal_row(&context);
    assert_eq!((row.0.clone(), row.1, row.2), (None, 2, 120_000));
    assert_ne!(
        row.3.as_deref(),
        Some("prior"),
        "last reservation not restored"
    );
    {
        let state = scheduler.wakes.state.lock().unwrap();
        assert_eq!(refusal_attempts(&state), 1, "the refusal backoff advanced");
    }
    // The refusal instant passes, but the advanced ladder (120 s from the
    // completion at 60 s) still blocks: memory matches the durable step.
    let refusal_due = drove.next_due_at.unwrap();
    assert_eq!(
        refusal_due.0, 180_000,
        "ladder, not the 100 ms refusal, is due"
    );
    clock.mono.store(refusal_due.0 - 1, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    assert_eq!(notifier.calls(), 1);
    clock.mono.store(refusal_due.0, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(notifier.calls(), 2);
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn outcome_unknown_from_an_unsent_prompt_keeps_the_step_and_ignores_the_refusal_backoff() {
    // ht-p03.41 contract: Unsubmitted maps to OutcomeUnknown, which stores the
    // existing outcome_unknown string, keeps the advanced step, and neither
    // resets nor advances the seat's refusal backoff.
    assert_eq!(
        crate::scheduler::outcome_for_verification(
            crate::scheduler::SubmissionVerification::Unsubmitted
        ),
        WakeOutcome::OutcomeUnknown
    );
    let (path, clock, context, store) = refusal_fixture();
    let failing = failing_store(&store, clock.as_ref());
    let notifier = ScriptedNotifier::new(
        &clock,
        vec![
            Ok(WakeOutcome::Refused(RefusalCause::Unavailable)),
            Ok(WakeOutcome::OutcomeUnknown),
        ],
    );
    let due = refusal_due_port();
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &failing,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    start_due(&clock);
    let budget = refusal_budget();
    let refused = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(refusal_row(&context).1, 1);
    clock
        .mono
        .store(refused.next_due_at.unwrap().0, Ordering::SeqCst);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    let row = refusal_row(&context);
    assert_eq!(
        (row.0, row.1, row.2, row.4),
        (None, 2, 120_000, Some("outcome_unknown".into()))
    );
    {
        let state = scheduler.wakes.state.lock().unwrap();
        assert_eq!(refusal_attempts(&state), 1, "neither reset nor advanced");
    }
    drop(scheduler);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

struct ErrObserveHost {
    inner: FakeNativeHost,
    error: ApiError,
}
impl HostPort for ErrObserveHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        ctx: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.send_submit_key(target, ctx)
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Err(self.error.clone())
    }
    fn enumerate_targets(&self, c: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(c)
    }
    fn safe_wake_target(&self, seat: &SeatId, o: &HostObservation) -> Option<SafeWakeTarget> {
        self.inner.safe_wake_target(seat, o)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.inner.submit_prompt(target, text, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        self.inner.pane_agent_state(target, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, context)
    }
}
struct ErrReservationCheck(ApiError);
impl ReservationCheck for ErrReservationCheck {
    fn is_current(&self, _: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        Err(self.0.clone())
    }
}

#[test]
fn pre_send_errors_are_refusals_by_class_and_never_reach_submit() {
    // Kills: `?` on observe_current_target / is_current (the error would
    // reach the scheduler as a ladder-climbing completion) and a class map
    // that sends a deadline or unsafe error to the wrong cause.
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    for (code, expected) in [
        (ErrorCode::HostUnavailable, Some(RefusalCause::Unavailable)),
        (ErrorCode::DeadlineExceeded, Some(RefusalCause::TimedOut)),
        (ErrorCode::TargetUnsafe, Some(RefusalCause::Unsafe)),
        (ErrorCode::StoreCorrupt, Some(RefusalCause::Unavailable)),
        (ErrorCode::Cancelled, None),
    ] {
        let error = ApiError::new(code.clone(), "injected");
        let host = ErrObserveHost {
            inner: FakeNativeHost {
                observation: fresh_observation(),
                submitted: AtomicU64::new(0),
                submit_keys: AtomicU64::new(0),
                pane_states: Default::default(),
            },
            error: error.clone(),
        };
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
        let observed = dispatch.attempt_wake(test_reservation(), &context);
        let fenced_host = FakeNativeHost {
            observation: fresh_observation(),
            submitted: AtomicU64::new(0),
            submit_keys: AtomicU64::new(0),
            pane_states: Default::default(),
        };
        let err_check = ErrReservationCheck(error.clone());
        let fenced = NativeWakeDispatcher::new(&fenced_host, &err_check, &clock)
            .attempt_wake(test_reservation(), &context);
        for (label, got) in [("observe", observed), ("is_current", fenced)] {
            match expected {
                Some(cause) => assert_eq!(
                    got.unwrap_or_else(|e| panic!("{label} {code:?} propagated: {e:?}")),
                    WakeOutcome::Refused(cause),
                    "{label} {code:?}"
                ),
                None => assert_eq!(
                    got.unwrap_err().code,
                    code,
                    "{label} propagates cancellation"
                ),
            }
        }
        assert_eq!(host.inner.submitted.load(Ordering::SeqCst), 0);
        assert_eq!(fenced_host.submitted.load(Ordering::SeqCst), 0);
    }
}

/// A cooperative-basis host that records the target handed to `submit_prompt`.
struct CooperativeRecordingHost {
    observation: HostObservation,
    prompted: std::sync::Mutex<Vec<SafeWakeTarget>>,
}
impl HostPort for CooperativeRecordingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::HostGuardedStart
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation.clone())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: crate::ports::WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.prompted.lock().unwrap().push(target.clone());
        Ok(PromptOutcome::Submitted)
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        unreachable!("a submitted prompt needs no submit key")
    }
}

/// Kills: a dispatcher that leaves `bound_harness` unset (the adapter would
/// then accept any recognized agent kind) or sets it from the wrong source.
#[test]
fn dispatcher_passes_bound_harness_to_prompt_target() {
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    for bound in [Some("claude"), Some("codex")] {
        let mut reservation = test_reservation();
        reservation.authority = ReservedWakeAuthority::Cooperative {
            terminal: TerminalId::new("terminal"),
            incarnation: "incarnation".into(),
            binding_generation: None,
            harness: bound.map(str::to_string),
        };
        let mut observation = fresh_observation();
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        let host = CooperativeRecordingHost {
            observation,
            prompted: std::sync::Mutex::new(vec![]),
        };
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
        assert_eq!(
            dispatch.attempt_wake(reservation, &context).unwrap(),
            WakeOutcome::Submitted,
            "{bound:?}"
        );
        let prompted = host.prompted.lock().unwrap();
        assert_eq!(prompted.len(), 1, "{bound:?}");
        assert_eq!(
            prompted[0].bound_harness.as_deref(),
            bound,
            "the prompt target carries the reservation's harness"
        );
    }
}

/// Kills: a cooperative reservation without a bound harness reaching the
/// prompt (TRUST-POLICY A4: no open binding, no wake).
#[test]
fn cooperative_reservation_without_harness_is_not_prompted() {
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    let mut reservation = test_reservation();
    reservation.authority = ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation: None,
        harness: None,
    };
    let mut observation = fresh_observation();
    observation.occupant = None;
    observation.ui = HostUiState::Unknown;
    observation.occupancy = StructuralOccupancy::Unknown;
    observation.execution = ExecutionEvidence::Unknown;
    let host = CooperativeRecordingHost {
        observation,
        prompted: std::sync::Mutex::new(vec![]),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    // A pre-send refusal (pacer D5): the reminder ladder does not climb.
    assert_eq!(
        dispatch.attempt_wake(reservation, &context).unwrap(),
        WakeOutcome::Refused(RefusalCause::Unsafe)
    );
    assert!(host.prompted.lock().unwrap().is_empty());
}

/// A cooperative host that answers the plain observation with an unclassified
/// UI (`Unknown`, what Herdr's status alone gives) and the poke observation
/// with the composer-classified `Idle`, counting each call.
struct ComposerReadHost {
    inner: CooperativeRecordingHost,
    plain_reads: AtomicU64,
    poke_reads: AtomicU64,
}
impl ComposerReadHost {
    fn new() -> Self {
        let mut observation = fresh_observation();
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        Self {
            inner: CooperativeRecordingHost {
                observation,
                prompted: std::sync::Mutex::new(vec![]),
            },
            plain_reads: AtomicU64::new(0),
            poke_reads: AtomicU64::new(0),
        }
    }
}
impl HostPort for ComposerReadHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.inner.native_launch_capability()
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.plain_reads.fetch_add(1, Ordering::SeqCst);
        self.inner.observe_current_target(target, context)
    }
    fn observe_current_target_for_poke(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.poke_reads.fetch_add(1, Ordering::SeqCst);
        let mut observation = self.inner.observe_current_target(target, context)?;
        observation.ui = HostUiState::Idle;
        Ok(observation)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(context)
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.inner.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.inner.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        self.inner.pane_agent_state(target, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.send_submit_key(target, context)
    }
}

/// The root cause of ht-1ip.55: an ordinary wake observed the target through
/// the composer-classifying read, so a Herdr that cannot answer the composer
/// read (UI `Unknown`) cost the wake an extra host round trip and widened its
/// stale-witness window (TRUST-POLICY A4: an ordinary wake never depends on
/// composer classification). Kills: an ordinary wake that calls the poke
/// observation (counted), or one refused for an `Unknown` UI. The same host
/// still serves a poke its composer classification, so a poke that fell back
/// to the plain observation would skip (`Unsafe`) instead of submitting.
#[test]
fn ordinary_wake_never_reads_the_composer_but_a_poke_does() {
    let clock = FakeClock(AtomicU64::new(0));
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let cooperative = |reservation: WakeReservation| WakeReservation {
        authority: ReservedWakeAuthority::Cooperative {
            terminal: TerminalId::new("terminal"),
            incarnation: "incarnation".into(),
            binding_generation: None,
            harness: Some("codex".into()),
        },
        ..reservation
    };

    let host = ComposerReadHost::new();
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    assert_eq!(
        dispatch
            .attempt_wake(cooperative(test_reservation()), &context)
            .unwrap(),
        WakeOutcome::Submitted,
        "an unclassified UI never refuses an ordinary wake"
    );
    assert_eq!(host.plain_reads.load(Ordering::SeqCst), 1);
    assert_eq!(host.poke_reads.load(Ordering::SeqCst), 0);

    let host = ComposerReadHost::new();
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatch
        .attempt_poke(
            cooperative(poke_reservation("seat", "a")),
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &STASH,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Submitted);
    assert!(attempt.poked);
    assert_eq!(host.plain_reads.load(Ordering::SeqCst), 0);
    assert_eq!(host.poke_reads.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// Soft-deadline pokes (spec §10): fake host and store matrix.
// ---------------------------------------------------------------------------
use crate::{
    harness::recipe::{NativeSupport, PokeCapabilities},
    ports::{
        ComposerStash, PokeCapabilitySource, PokeDue, PokeMode, PokePlan, PokeReceipt,
        PokeReservation, PokeSource,
    },
    protocol::ids::{MessageId, ThreadId},
};
use std::collections::HashMap;

fn poke_receipt(message: &str, thread: &str, effective_deadline: i64) -> PokeReceipt {
    PokeReceipt {
        message: MessageId::new(message),
        seat: SeatId::new("seat"),
        thread: ThreadId::new(thread),
        source: PokeSource::ReceiptState,
        effective_deadline,
    }
}
fn poke_due(seat: &str, receipts: Vec<PokeReceipt>) -> PokeDue {
    PokeDue {
        seat: SeatId::new(seat),
        receipts: receipts
            .into_iter()
            .map(|mut r| {
                r.seat = SeatId::new(seat);
                r
            })
            .collect(),
    }
}
fn poke_reservation(seat: &str, attempt: &str) -> WakeReservation {
    WakeReservation {
        attempt: WakeAttemptId::new(attempt),
        seat: SeatId::new(seat),
        attention_witness: test_witness(false, true),
        reasons: vec!["soft_deadline".into()],
        // Outlives the retry spacings the poke tests step the clock over.
        lease_until: MonoInstant(10_000_000),
        authority: ReservedWakeAuthority::Registered {
            binding_generation: 1,
            execution: ExecutionId::new("execution"),
        },
        ..test_reservation()
    }
}

/// A store holding due pokes. `held` is the seats' single reservation slot,
/// shared by wake and poke reservations exactly like `wake_work`.
struct PokeStore {
    clock: Arc<FakeClock>,
    due: Mutex<Vec<PokeDue>>,
    marked: Mutex<Vec<String>>,
    log: Mutex<Vec<String>>,
    held: Mutex<HashMap<String, SeatId>>,
    wake: Mutex<Vec<WakeCandidate>>,
    seq: AtomicU64,
    /// The largest `limit` `poke_candidates` was asked for.
    asked_limit: AtomicU64,
}
impl PokeStore {
    fn new(clock: Arc<FakeClock>, due: Vec<PokeDue>) -> Self {
        Self {
            clock,
            due: Mutex::new(due),
            marked: Mutex::new(Vec::new()),
            log: Mutex::new(Vec::new()),
            held: Mutex::new(HashMap::new()),
            wake: Mutex::new(Vec::new()),
            seq: AtomicU64::new(0),
            asked_limit: AtomicU64::new(0),
        }
    }
    /// Due pokes minus the receipts a submitted attempt marked.
    fn remaining(&self) -> Vec<PokeDue> {
        let marked = self.marked.lock().unwrap();
        self.due
            .lock()
            .unwrap()
            .iter()
            .filter_map(|due| {
                let receipts: Vec<_> = due
                    .receipts
                    .iter()
                    .filter(|r| !marked.iter().any(|m| m == r.message.as_str()))
                    .cloned()
                    .collect();
                (!receipts.is_empty()).then(|| PokeDue {
                    seat: due.seat.clone(),
                    receipts,
                })
            })
            .collect()
    }
    fn next_attempt(&self, seat: &SeatId) -> Option<WakeAttemptId> {
        let mut held = self.held.lock().unwrap();
        if held.values().any(|s| s == seat) {
            return None;
        }
        let attempt = format!("attempt-{}", self.seq.fetch_add(1, Ordering::SeqCst));
        held.insert(attempt.clone(), seat.clone());
        Some(WakeAttemptId::new(attempt))
    }
    fn release(&self, attempt: &WakeAttemptId) {
        self.held.lock().unwrap().remove(attempt.as_str());
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}
impl WakePort for PokeStore {
    fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }
    fn wake_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        Ok(Page {
            items: self.wake.lock().unwrap().clone(),
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        })
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let Some(attempt) = self.next_attempt(&candidate.seat) else {
            return Ok(None);
        };
        self.log.lock().unwrap().push("reserve_wake".into());
        Ok(Some(WakeReservation {
            attention_witness: candidate.attention_witness.clone().unwrap(),
            ..poke_reservation(candidate.seat.as_str(), attempt.as_str())
        }))
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        _: Option<&crate::ports::PriorLadder>,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.release(&attempt);
        self.log
            .lock()
            .unwrap()
            .push(format!("complete_wake:{outcome:?}"));
        Ok(false)
    }
    fn wake_recovery_candidates(
        &self,
        _: PageRequest,
        _: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        Ok(empty_recovery_page())
    }
    fn recover_wake_reservation(
        &self,
        _: WakeRecoveryRequest,
        _: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        unreachable!()
    }
    fn poke_candidates(&self, limit: u16, _: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        self.asked_limit
            .fetch_max(u64::from(limit), Ordering::SeqCst);
        // Like the real store: soonest effective deadline first, then `limit`.
        let mut due = self.remaining();
        due.sort_by_key(|due| {
            (
                due.receipts.iter().map(|r| r.effective_deadline).min(),
                due.seat.clone(),
            )
        });
        due.truncate(usize::from(limit));
        Ok(due)
    }
    fn poke_for_wake(&self, seat: &SeatId, _: &CallBudget) -> Result<Option<PokeDue>, ApiError> {
        Ok(self.remaining().into_iter().find(|due| &due.seat == seat))
    }
    fn reserve_poke(
        &self,
        due: &PokeDue,
        _: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        let Some(attempt) = self.next_attempt(&due.seat) else {
            return Ok(None);
        };
        self.log.lock().unwrap().push("reserve_poke".into());
        Ok(Some(PokeReservation {
            reservation: poke_reservation(due.seat.as_str(), attempt.as_str()),
            receipts: due.receipts.clone(),
        }))
    }
    fn complete_poke(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        receipts: &[PokeReceipt],
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        self.release(&attempt);
        if outcome == WakeOutcome::Submitted {
            self.marked
                .lock()
                .unwrap()
                .extend(receipts.iter().map(|r| r.message.as_str().to_owned()));
        }
        self.log
            .lock()
            .unwrap()
            .push(format!("complete_poke:{outcome:?}:{}", receipts.len()));
        Ok(())
    }
}

/// A native host whose observation, prompt result and focus the test drives.
/// It leaves the composer-stash hook at its inert default.
struct PokeHost {
    observation: Mutex<HostObservation>,
    prompts: Mutex<Vec<String>>,
    submit: Mutex<Result<PromptOutcome, ErrorCode>>,
    gate: Option<(Mutex<bool>, Condvar)>,
    entered: AtomicU64,
}
impl PokeHost {
    fn new(ui: HostUiState, focused: bool) -> Self {
        let mut observation = fresh_observation();
        observation.ui = ui;
        observation.focused = focused;
        Self {
            observation: Mutex::new(observation),
            prompts: Mutex::new(Vec::new()),
            submit: Mutex::new(Ok(PromptOutcome::Submitted)),
            gate: None,
            entered: AtomicU64::new(0),
        }
    }
    fn set(&self, ui: HostUiState, focused: bool) {
        let mut observation = self.observation.lock().unwrap();
        observation.ui = ui;
        observation.focused = focused;
    }
    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}
impl HostPort for PokeHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation.lock().unwrap().clone())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        FakeNativeHost {
            observation: observation.clone(),
            submitted: AtomicU64::new(0),
            submit_keys: AtomicU64::new(0),
            pane_states: Default::default(),
        }
        .safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        if let Some((open, condvar)) = &self.gate {
            let mut open = open.lock().unwrap();
            while !*open {
                open = condvar.wait(open).unwrap();
            }
        }
        self.prompts.lock().unwrap().push(text.to_owned());
        match &*self.submit.lock().unwrap() {
            Ok(outcome) => Ok(*outcome),
            Err(code) => Err(ApiError::new(code.clone(), "injected")),
        }
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
}

/// `PokeHost` with a stash-capable adapter: records stash and restore calls.
struct StashingHost {
    inner: PokeHost,
    stash: ComposerStash,
    restore_fails: bool,
    stash_calls: AtomicU64,
    restored: Mutex<Vec<String>>,
}
impl StashingHost {
    fn new(inner: PokeHost, stash: ComposerStash) -> Self {
        Self {
            inner,
            stash,
            restore_fails: false,
            stash_calls: AtomicU64::new(0),
            restored: Mutex::new(Vec::new()),
        }
    }
}
impl HostPort for StashingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.inner.native_launch_capability()
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.inner.observe_current_target(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        self.inner.pane_agent_state(target, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.send_submit_key(target, context)
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.inner.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.inner.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, context)
    }
    fn stash_composer(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        self.stash_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.stash.clone())
    }
    fn restore_composer(
        &self,
        _: &SafeWakeTarget,
        saved: &str,
        _: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.restored.lock().unwrap().push(saved.to_owned());
        if self.restore_fails {
            return Err(ApiError::new(ErrorCode::HostUnavailable, "restore refused"));
        }
        Ok(())
    }
}

/// Capability evidence the test declares for every harness.
struct Declared(PokeCapabilities);
impl PokeCapabilitySource for Declared {
    fn capabilities(&self, _: Harness) -> PokeCapabilities {
        self.0
    }
}
const STASH: Declared = Declared(PokeCapabilities {
    composer_stash: NativeSupport::Supported,
    poke_during_turn: NativeSupport::Unsupported,
});
const DURING_TURN: Declared = Declared(PokeCapabilities {
    composer_stash: NativeSupport::Unsupported,
    poke_during_turn: NativeSupport::Supported,
});

fn poke_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// A budget that outlives the retry spacings tests step the clock over.
fn long_poke_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000_000),
        cancellation: Cancellation::default(),
    }
}

/// Steps the fake clock past one wake retry spacing.
fn pass_retry_spacing(store: &PokeStore) {
    store.clock.0.fetch_add(
        RetryConfig::default().minimum_delay_ms(),
        std::sync::atomic::Ordering::SeqCst,
    );
}

/// One scheduler over `store` and `host`, kept across ticks so the in-memory
/// wake limits persist between them.
fn with_scheduler<H: HostPort, R>(
    store: &PokeStore,
    host: &H,
    caps: &dyn PokeCapabilitySource,
    f: impl FnOnce(
        &Scheduler<
            '_,
            FakeDeadlinePort,
            PokeStore,
            NativeWakeDispatcher<'_, H, FakeReservationCheck>,
        >,
    ) -> R,
) -> R {
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatcher = NativeWakeDispatcher::new(host, &check, store.clock.as_ref());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        store,
        &dispatcher,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_poke_capabilities(caps);
    f(&scheduler)
}

fn one_seat_due() -> Vec<PokeDue> {
    vec![poke_due(
        "seat",
        vec![
            poke_receipt("m1", "t1", 40_000),
            poke_receipt("m2", "t2", 50_000),
            poke_receipt("m3", "t1", 60_000),
        ],
    )]
}
const POKE_TEXT: &str = "herdr-threads: receipt due in 40s on t1, t2; run herdr-threads inbox";

#[test]
fn idle_unfocused_target_gets_one_coalesced_poke() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    let host = PokeHost::new(HostUiState::Idle, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            let outcome = scheduler.drive_pokes(&poke_budget()).unwrap();
            assert_eq!((outcome.examined, outcome.attempted), (1, 1));
            assert_eq!(host.prompts(), [POKE_TEXT]);
            // soft_poked_at is set for every coalesced receipt, and only those.
            assert_eq!(*store.marked.lock().unwrap(), ["m1", "m2", "m3"]);
            // The next tick finds nothing due: one poke per soft point.
            let outcome = scheduler.drive_pokes(&poke_budget()).unwrap();
            assert_eq!((outcome.examined, outcome.attempted), (0, 0));
            assert_eq!(host.prompts().len(), 1);
        },
    );
    assert_eq!(
        store.log(),
        ["reserve_poke", "complete_poke:Submitted:3"],
        "one reservation, settled once with the receipts it covered"
    );
}

#[test]
fn focused_or_unsafe_states_skip_and_retry_after_the_spacing() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    let host = PokeHost::new(HostUiState::Idle, true);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            // Focused, then each state the recipe does not allow: nothing is sent,
            // nothing is marked, and the seat is re-evaluated once the wake
            // retry spacing has passed (a skip does not advance the wake guard).
            for (ui, focused) in [
                (HostUiState::Idle, true),
                (HostUiState::ActiveTurn, false),
                (HostUiState::HumanInput, false),
                (HostUiState::ApprovalOrQuestion, false),
                (HostUiState::Unknown, false),
            ] {
                host.set(ui, focused);
                let outcome = scheduler.drive_pokes(&long_poke_budget()).unwrap();
                assert_eq!(
                    outcome.attempted, 1,
                    "{ui:?} focused={focused} re-evaluated"
                );
                assert!(host.prompts().is_empty(), "{ui:?} focused={focused}");
                assert!(store.marked.lock().unwrap().is_empty());
                // Before the spacing passes the seat is not attempted again.
                let outcome = scheduler.drive_pokes(&long_poke_budget()).unwrap();
                assert_eq!(outcome.attempted, 0, "{ui:?} backs off");
                pass_retry_spacing(&store);
            }
            // Focus leaves: the next evaluation sends.
            host.set(HostUiState::Idle, false);
            scheduler.drive_pokes(&long_poke_budget()).unwrap();
            assert_eq!(host.prompts(), [POKE_TEXT]);
            assert_eq!(store.marked.lock().unwrap().len(), 3);
        },
    );
    let log = store.log();
    assert_eq!(
        log.iter().filter(|l| *l == "complete_wake:Unsafe").count(),
        5,
        "every skip releases the slot unmarked: {log:?}"
    );
}

#[test]
fn failed_submission_leaves_soft_poked_at_unset() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    let host = PokeHost::new(HostUiState::Idle, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            for failure in [
                Err(ErrorCode::TargetUnsafe),
                Err(ErrorCode::HostUnavailable),
                Ok(PromptOutcome::OutcomeUnknown),
            ] {
                *host.submit.lock().unwrap() = failure;
                scheduler.drive_pokes(&long_poke_budget()).unwrap();
                assert!(
                    store.marked.lock().unwrap().is_empty(),
                    "a prompt the host did not accept is never marked"
                );
                pass_retry_spacing(&store);
            }
            *host.submit.lock().unwrap() = Ok(PromptOutcome::Submitted);
            // The receipts were never lost: the next evaluation pokes them.
            scheduler.drive_pokes(&long_poke_budget()).unwrap();
            assert_eq!(store.marked.lock().unwrap().len(), 3);
        },
    );
}

#[test]
fn human_input_with_stash_capability_uses_inert_hook_and_skips() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    // The default HostPort hook reports Unsupported: the poke is skipped.
    let host = StashingHost::new(
        PokeHost::new(HostUiState::HumanInput, false),
        ComposerStash::Unsupported,
    );
    with_scheduler(&store, &host, &STASH, |scheduler| {
        scheduler.drive_pokes(&poke_budget()).unwrap();
    });
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 1);
    assert!(host.inner.prompts().is_empty());
    assert!(store.marked.lock().unwrap().is_empty());

    // The same state through the unmodified default hook (no override at all).
    let plain = PokeHost::new(HostUiState::HumanInput, false);
    with_scheduler(&store, &plain, &STASH, |scheduler| {
        scheduler.drive_pokes(&poke_budget()).unwrap();
    });
    assert!(plain.prompts().is_empty());
    assert!(store.marked.lock().unwrap().is_empty());

    // Without a declared capability the hook is not even consulted.
    let host = StashingHost::new(
        PokeHost::new(HostUiState::HumanInput, false),
        ComposerStash::Saved("typed".into()),
    );
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            scheduler.drive_pokes(&poke_budget()).unwrap();
        },
    );
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 0);
    assert!(host.inner.prompts().is_empty());
}

fn dispatch_context() -> HostCallContext {
    HostCallContext {
        budget: poke_budget(),
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    }
}
fn poke_plan_for_tests() -> PokePlan {
    PokePlan {
        text: POKE_TEXT.into(),
        receipts: vec![poke_receipt("m1", "t1", 40_000)],
    }
}

#[test]
fn dispatcher_matrix_decides_submit_stash_or_skip() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let states = [
        HostUiState::Idle,
        HostUiState::ActiveTurn,
        HostUiState::HumanInput,
        HostUiState::ApprovalOrQuestion,
        HostUiState::Unknown,
    ];
    let mut cases = 0;
    for ui in states {
        for focused in [false, true] {
            for during_turn in [NativeSupport::Unsupported, NativeSupport::Supported] {
                for stash in [NativeSupport::Unsupported, NativeSupport::Supported] {
                    let caps = Declared(PokeCapabilities {
                        composer_stash: stash,
                        poke_during_turn: during_turn,
                    });
                    let host = StashingHost::new(
                        PokeHost::new(ui, focused),
                        ComposerStash::Saved("typed".into()),
                    );
                    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
                    let attempt = dispatcher
                        .attempt_poke(
                            poke_reservation("seat", "a"),
                            &poke_plan_for_tests(),
                            PokeMode::PokeOnly,
                            &caps,
                            &dispatch_context(),
                        )
                        .unwrap();
                    let supported = |s| s == NativeSupport::Supported;
                    let (submits, stashes) = match (ui, focused) {
                        (_, true) => (false, false),
                        (HostUiState::Idle, _) => (true, false),
                        (HostUiState::ActiveTurn, _) => (supported(during_turn), false),
                        (HostUiState::HumanInput, _) => (supported(stash), supported(stash)),
                        _ => (false, false),
                    };
                    let label =
                        format!("{ui:?} focused={focused} turn={during_turn:?} stash={stash:?}");
                    assert_eq!(host.inner.prompts().len(), usize::from(submits), "{label}");
                    assert_eq!(
                        host.stash_calls.load(Ordering::SeqCst),
                        u64::from(stashes),
                        "{label}"
                    );
                    assert_eq!(
                        *host.restored.lock().unwrap(),
                        if stashes {
                            vec!["typed".to_owned()]
                        } else {
                            vec![]
                        },
                        "{label}"
                    );
                    assert_eq!(attempt.poked, submits, "{label}");
                    assert_eq!(
                        attempt.outcome,
                        if submits {
                            WakeOutcome::Submitted
                        } else {
                            WakeOutcome::Unsafe
                        },
                        "{label}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 40);
}

#[test]
fn stash_restore_failure_is_kept_as_a_diagnostic_with_the_typed_text() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let mut host = StashingHost::new(
        PokeHost::new(HostUiState::HumanInput, false),
        ComposerStash::Saved("half-typed words".into()),
    );
    host.restore_fails = true;
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            poke_reservation("seat", "a"),
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &STASH,
            &dispatch_context(),
        )
        .unwrap();
    // The prompt was accepted; the failed restore does not undo it.
    assert_eq!(attempt.outcome, WakeOutcome::Submitted);
    assert!(attempt.poked);
    let diagnostic = attempt.diagnostic.expect("restore failure kept");
    assert!(diagnostic.contains("half-typed words"), "{diagnostic}");
    // A failed stash skips before any prompt.
    let host = StashingHost::new(
        PokeHost::new(HostUiState::HumanInput, false),
        ComposerStash::Failed("composer busy".into()),
    );
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            poke_reservation("seat", "a"),
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &STASH,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Unsafe);
    assert!(host.inner.prompts().is_empty());
}

#[test]
fn unbound_target_is_not_poked() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    // A recovery-hint seat (no registered binding) is not a bound native agent.
    let host = PokeHost::new(HostUiState::Idle, false);
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let hint = WakeReservation {
        authority: ReservedWakeAuthority::RecoveryHint {
            execution: ExecutionId::new("execution"),
        },
        ..poke_reservation("seat", "a")
    };
    let attempt = dispatcher
        .attempt_poke(
            hint,
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &crate::ports::NoPokeCapabilities,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Unsafe);
    assert!(host.prompts().is_empty());
}

fn wake_and_poke_store(clock: Arc<FakeClock>) -> PokeStore {
    let store = PokeStore::new(clock, one_seat_due());
    *store.wake.lock().unwrap() = vec![due_candidate()];
    store
}

#[test]
fn poke_and_wake_due_send_one_prompt_with_poke_text() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = wake_and_poke_store(clock);
    let host = PokeHost::new(HostUiState::Idle, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            assert_eq!(scheduler.drive_wakes(&poke_budget()).unwrap().attempted, 1);
            // Pokes run right after the wake in the same tick: nothing is left.
            assert_eq!(scheduler.drive_pokes(&poke_budget()).unwrap().attempted, 0);
        },
    );
    assert_eq!(
        host.prompts(),
        [POKE_TEXT],
        "one prompt, with the poke text"
    );
    assert_eq!(*store.marked.lock().unwrap(), ["m1", "m2", "m3"]);
    assert_eq!(
        store.log(),
        ["reserve_wake", "complete_poke:Submitted:3"],
        "the wake's own settlement carried the poke's receipts"
    );
}

#[test]
fn wake_without_an_eligible_poke_keeps_the_plain_marker_and_marks_nothing() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = wake_and_poke_store(clock);
    // A focused pane still gets the ordinary wake marker (as before), but the
    // poke is not eligible, so no receipt is marked.
    let host = PokeHost::new(HostUiState::Idle, true);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            assert_eq!(scheduler.drive_wakes(&poke_budget()).unwrap().attempted, 1);
        },
    );
    assert_eq!(host.prompts(), [crate::notification::policy::MARKER]);
    assert!(store.marked.lock().unwrap().is_empty());
    assert_eq!(store.log(), ["reserve_wake", "complete_wake:Submitted"]);
}

#[test]
fn dispatch_state_applies_the_wake_limits_to_pokes() {
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), daemon_boot());
    let seat = |n: usize| SeatId::new(format!("seat-{n}"));
    // An unknown seat has no wake guard: a poke may take the slot.
    assert!(state.can_reserve_poke(&seat(0), MonoInstant(0)));
    state
        .poke_reserved(seat(0), WakeAttemptId::new("p0"), daemon_boot())
        .unwrap();
    // One in flight per seat: neither a second poke nor a wake may start.
    assert!(!state.can_reserve_poke(&seat(0), MonoInstant(0)));
    state.restore(seat(0), fresh()).unwrap();
    assert!(!state.can_reserve(&seat(0), MonoInstant(0)));
    // A foreign boot is refused.
    assert_eq!(
        state.poke_reserved(seat(1), WakeAttemptId::new("p1"), old_daemon_boot()),
        Err(DispatchError::WrongBoot)
    );
    // Four active prompts overall, pokes and wakes alike.
    for n in 1..4 {
        state
            .poke_reserved(seat(n), WakeAttemptId::new(format!("p{n}")), daemon_boot())
            .unwrap();
    }
    assert_eq!(state.active_count(), 4);
    assert!(!state.can_reserve_poke(&seat(9), MonoInstant(0)));
    assert_eq!(
        state.poke_reserved(seat(9), WakeAttemptId::new("p9"), daemon_boot()),
        Err(DispatchError::ActiveLimit)
    );
    // A late result for another attempt cannot free the slot.
    assert!(!state.poke_finished(
        &seat(0),
        &WakeAttemptId::new("other"),
        &daemon_boot(),
        true,
        MonoInstant(5)
    ));
    assert_eq!(state.active_count(), 4);
    // A skipped attempt frees the slot but holds the seat for the retry
    // spacing (see `skipped_poke_waits_the_retry_spacing`); an accepted
    // prompt holds it for the configured minimum between pokes.
    assert!(state.poke_finished(
        &seat(1),
        &WakeAttemptId::new("p1"),
        &daemon_boot(),
        false,
        MonoInstant(100)
    ));
    assert_eq!(state.active_count(), 3);
    assert!(state.poke_finished(
        &seat(0),
        &WakeAttemptId::new("p0"),
        &daemon_boot(),
        true,
        MonoInstant(100)
    ));
    assert!(!state.can_reserve_poke(&seat(0), MonoInstant(100 + 29_999)));
    assert!(state.can_reserve_poke(&seat(0), MonoInstant(100 + 30_000)));
}

/// Kills: re-attempting a skipped seat on the next tick (one reservation and
/// one host read per ~100 ms wake loop per ineligible seat), a backoff that
/// never expires, one off by a millisecond at the spacing, and a submitted
/// poke that leaves the skip backoff in place.
#[test]
fn skipped_poke_waits_the_retry_spacing() {
    let config = RetryConfig::default();
    let spacing = config.minimum_delay_ms();
    assert!(spacing > 1, "a spacing of at most 1 ms cannot be probed");
    let mut state = DispatchState::new(config, MonoInstant(0), daemon_boot());
    let seat = SeatId::new("seat");
    let reserve = |state: &mut DispatchState, n: u32| {
        let attempt = WakeAttemptId::new(format!("p{n}"));
        state
            .poke_reserved(seat.clone(), attempt.clone(), daemon_boot())
            .unwrap();
        attempt
    };
    let t = 1_000;
    let attempt = reserve(&mut state, 0);
    assert!(state.poke_finished(&seat, &attempt, &daemon_boot(), false, MonoInstant(t)));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(t)));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(t + 100)));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(t + spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(t + spacing)));
    // A second skip restarts the spacing from its own end.
    let later = t + spacing;
    let attempt = reserve(&mut state, 1);
    assert!(state.poke_finished(&seat, &attempt, &daemon_boot(), false, MonoInstant(later)));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(later + spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(later + spacing)));
    // A submitted poke clears the skip: only the submit spacing applies, and
    // it is measured from the submit, not from the earlier skip.
    let now = later + spacing;
    let attempt = reserve(&mut state, 2);
    assert!(state.poke_finished(&seat, &attempt, &daemon_boot(), true, MonoInstant(now)));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(now + spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(now + spacing)));
    // A late result for a past attempt does not start a backoff.
    assert!(!state.poke_finished(
        &seat,
        &attempt,
        &daemon_boot(),
        false,
        MonoInstant(now + spacing)
    ));
    assert!(state.can_reserve_poke(&seat, MonoInstant(now + spacing)));
}

/// Kills: a poke gated by the wake guard's retry backoff (`eligible`), a poke
/// that ignores the restart spacing, and one that ignores the spacing after a
/// wake attempt.
#[test]
fn poke_ignores_the_wake_retry_backoff() {
    let config = RetryConfig::default();
    let spacing = config.minimum_delay_ms();
    let mut state = DispatchState::new(config, MonoInstant(0), daemon_boot());
    let seat = SeatId::new("seat");
    state
        .restore(
            seat.clone(),
            DurableRetry {
                retry_step: 3,
                minimum_delay_ms: spacing,
                effective_delay_ms: 300_000,
                ever_reserved: true,
            },
        )
        .unwrap();
    assert_eq!(spacing, 30_000);
    assert!(!state.can_reserve_poke(&seat, MonoInstant(spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(spacing)));
    assert!(
        !state.can_reserve(&seat, MonoInstant(spacing)),
        "300 s wake backoff"
    );

    // A wake attempt restarts the elapsed spacing, not the backoff.
    let t = 400_000;
    let attempt = WakeAttemptId::new("w1");
    state
        .reserved(seat.clone(), attempt.clone(), daemon_boot(), MonoInstant(t))
        .unwrap();
    assert!(
        !state.can_reserve_poke(&seat, MonoInstant(t + 5_000)),
        "in flight"
    );
    assert!(
        state
            .finish(&seat, &attempt, &daemon_boot(), MonoInstant(t + 1_000))
            .unwrap()
    );
    assert!(!state.can_reserve_poke(&seat, MonoInstant(t + 1_000 + spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(t + 1_000 + spacing)));
    assert!(!state.can_reserve(&seat, MonoInstant(t + 1_000 + spacing)));
}

/// Kills: `can_reserve_poke` requiring `guard.eligible` (ht-2i4): a seat whose
/// wakes sit at the 300 s step never got a poke inside its soft window.
#[test]
fn seat_in_wake_backoff_past_its_soft_point_is_poked() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    let mut candidate = due_candidate();
    candidate.retry_step = 3;
    candidate.minimum_delay_ms = 30_000;
    candidate.effective_delay_ms = 300_000;
    candidate.last_reservation_id = Some(WakeAttemptId::new("prior"));
    candidate.last_reservation_boot = Some(HostBootId::new("old-boot"));
    *store.wake.lock().unwrap() = vec![candidate];
    let host = PokeHost::new(HostUiState::Idle, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            // The wake loop restores the seat's guard: backed off until 300 s.
            assert_eq!(
                scheduler
                    .drive_wakes(&long_poke_budget())
                    .unwrap()
                    .attempted,
                0
            );
            store.clock.0.store(30_000, Ordering::SeqCst);
            assert_eq!(
                scheduler
                    .drive_wakes(&long_poke_budget())
                    .unwrap()
                    .attempted,
                0
            );
            let outcome = scheduler.drive_pokes(&long_poke_budget()).unwrap();
            assert_eq!((outcome.examined, outcome.attempted), (1, 1));
            assert_eq!(host.prompts(), [POKE_TEXT]);
            assert_eq!(*store.marked.lock().unwrap(), ["m1", "m2", "m3"]);
            // The wake is still refused by its own backoff.
            assert_eq!(
                scheduler
                    .drive_wakes(&long_poke_budget())
                    .unwrap()
                    .attempted,
                0
            );
        },
    );
    assert_eq!(store.log(), ["reserve_poke", "complete_poke:Submitted:3"]);
}

/// Kills: cutting the due list to `POKE_SEAT_LIMIT` before the in-memory
/// admission, so 16 seats inside their skip spacing hide a 17th eligible one.
#[test]
fn ineligible_seats_do_not_crowd_out_an_eligible_one() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let seat_due = |n: usize| {
        poke_due(
            &format!("seat-{n:02}"),
            vec![poke_receipt(&format!("m{n}"), "t1", 40_000 + n as i64)],
        )
    };
    let store = PokeStore::new(clock, (0..16).map(seat_due).collect());
    let host = PokeHost::new(HostUiState::Idle, true);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            // All 16 are skipped (focused) and held by the skip spacing.
            let outcome = scheduler.drive_pokes(&poke_budget()).unwrap();
            assert_eq!((outcome.examined, outcome.attempted), (16, 16));
            assert!(host.prompts().is_empty());
            // A 17th seat, later in deadline order, becomes due and eligible.
            store.due.lock().unwrap().push(seat_due(16));
            host.set(HostUiState::Idle, false);
            let outcome = scheduler.drive_pokes(&poke_budget()).unwrap();
            assert_eq!((outcome.examined, outcome.attempted), (1, 1));
            assert_eq!(host.prompts().len(), 1);
            assert_eq!(*store.marked.lock().unwrap(), ["m16"]);
        },
    );
    assert!(
        store.asked_limit.load(Ordering::SeqCst) > 16,
        "the store was asked for more than the per-tick cap"
    );
}

#[test]
fn a_seat_with_an_active_wake_reservation_gets_no_poke() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    // The seat's slot is held by a wake attempt (store-side `wake_work`).
    store
        .held
        .lock()
        .unwrap()
        .insert("wake-attempt".into(), SeatId::new("seat"));
    let host = PokeHost::new(HostUiState::Idle, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            assert_eq!(scheduler.drive_pokes(&poke_budget()).unwrap().attempted, 0);
        },
    );
    assert!(host.prompts().is_empty());
    assert!(store.log().is_empty());
}

#[test]
fn five_due_seats_send_at_most_four_active_prompts() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let due = (0..5)
        .map(|n| {
            poke_due(
                &format!("seat-{n}"),
                vec![poke_receipt(&format!("m{n}"), "t1", 40_000)],
            )
        })
        .collect::<Vec<_>>();
    let store = PokeStore::new(clock.clone(), due.clone());
    let mut host = PokeHost::new(HostUiState::Idle, false);
    host.gate = Some((Mutex::new(false), Condvar::new()));
    let due_port = FakeDeadlinePort {
        clock: clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let dispatcher = NativeWakeDispatcher::new(&host, &check, clock.as_ref());
    let runner = WakeRunner::new(&store, &dispatcher, RetryConfig::default(), daemon_boot());
    let _ = &due_port;
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = due
            .iter()
            .map(|due| {
                let runner = &runner;
                scope.spawn(move || runner.try_poke(due, &poke_budget()))
            })
            .collect();
        // Four attempts reach the host while the fifth is refused admission.
        wait_for_condition(
            || host.entered.load(Ordering::SeqCst) == 4,
            "four pokes in flight",
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(
            host.entered.load(Ordering::SeqCst),
            4,
            "a fifth never reaches the host while four are active"
        );
        let (open, condvar) = host.gate.as_ref().unwrap();
        *open.lock().unwrap() = true;
        condvar.notify_all();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results.iter().filter(|r| r.is_some()).count(),
        4,
        "{results:?}"
    );
    assert_eq!(host.prompts().len(), 4);
    assert_eq!(store.marked.lock().unwrap().len(), 4);
}

#[test]
fn active_turn_is_poked_only_where_the_recipe_declares_it() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = PokeStore::new(clock, one_seat_due());
    let host = PokeHost::new(HostUiState::ActiveTurn, false);
    with_scheduler(
        &store,
        &host,
        &crate::ports::NoPokeCapabilities,
        |scheduler| {
            scheduler.drive_pokes(&poke_budget()).unwrap();
        },
    );
    assert!(host.prompts().is_empty(), "undeclared: skipped");
    with_scheduler(&store, &host, &DURING_TURN, |scheduler| {
        scheduler.drive_pokes(&poke_budget()).unwrap();
    });
    assert_eq!(host.prompts(), [POKE_TEXT], "declared poke_during_turn");
    assert_eq!(store.marked.lock().unwrap().len(), 3);
}

#[test]
fn unreserved_poke_waits_the_retry_spacing() {
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), daemon_boot());
    let seat = SeatId::new("seat-0");
    let spacing = RetryConfig::default().minimum_delay_ms();
    let t = 1_000;
    state.poke_unreserved(&seat, MonoInstant(t));
    assert!(!state.can_reserve_poke(&seat, MonoInstant(t + spacing - 1)));
    assert!(state.can_reserve_poke(&seat, MonoInstant(t + spacing)));
    assert_eq!(state.active_count(), 0);

    // While a poke is active the slot and the count are left alone.
    let other = SeatId::new("seat-1");
    state
        .poke_reserved(other.clone(), WakeAttemptId::new("p1"), daemon_boot())
        .unwrap();
    state.poke_unreserved(&other, MonoInstant(t));
    assert_eq!(state.active_count(), 1);
    assert!(state.poke_finished(
        &other,
        &WakeAttemptId::new("p1"),
        &daemon_boot(),
        true,
        MonoInstant(t + 10)
    ));
    assert_eq!(state.active_count(), 0);
}

// ---------------------------------------------------------------------------
// Composer content never refuses an ordinary wake (TRUST-POLICY A4, ht-1ip.46).
// ---------------------------------------------------------------------------

/// A cooperative-basis host over typed composer input that records prompt
/// texts and stash/restore calls.
struct CooperativeStashingHost {
    observation: HostObservation,
    stash: ComposerStash,
    prompts: Mutex<Vec<String>>,
    stash_calls: AtomicU64,
    restored: Mutex<Vec<String>>,
}
impl CooperativeStashingHost {
    fn over_typed_input(stash: ComposerStash) -> Self {
        let mut observation = fresh_observation();
        observation.occupant = None;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        observation.focused = false;
        observation.ui = HostUiState::HumanInput;
        Self {
            observation,
            stash,
            prompts: Mutex::new(Vec::new()),
            stash_calls: AtomicU64::new(0),
            restored: Mutex::new(Vec::new()),
        }
    }
}
impl HostPort for CooperativeStashingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::HostGuardedStart
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation.clone())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: crate::ports::WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.prompts.lock().unwrap().push(text.to_owned());
        Ok(PromptOutcome::Submitted)
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn stash_composer(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        self.stash_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.stash.clone())
    }
    fn restore_composer(
        &self,
        _: &SafeWakeTarget,
        saved: &str,
        _: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.restored.lock().unwrap().push(saved.to_owned());
        Ok(())
    }
}

fn cooperative_reservation() -> WakeReservation {
    WakeReservation {
        authority: ReservedWakeAuthority::Cooperative {
            terminal: TerminalId::new("terminal"),
            incarnation: "incarnation".into(),
            binding_generation: None,
            harness: Some("claude".into()),
        },
        ..test_reservation()
    }
}

/// Records the real dispatcher's optional-interface calls without changing the
/// existing cooperative host defaults or inventing native occupant evidence.
struct RegisteredPokeHost {
    inner: CooperativeStashingHost,
    during_turn: Mutex<Vec<(SafeWakeTarget, String, HostCallContext)>>,
    submissions: Mutex<Vec<(SafeWakeTarget, String, HostCallContext)>>,
    submit_keys: AtomicU64,
    reads: Mutex<Vec<HostCallContext>>,
    target_override: Option<SafeWakeTarget>,
    read_clock: Option<(Arc<FakeClock>, u64)>,
    submission_error: Option<ApiError>,
    submission_unknown: bool,
}
impl RegisteredPokeHost {
    fn new(ui: HostUiState) -> Self {
        let mut inner =
            CooperativeStashingHost::over_typed_input(ComposerStash::Saved("typed".into()));
        inner.observation.ui = ui;
        Self {
            inner,
            during_turn: Mutex::new(Vec::new()),
            submissions: Mutex::new(Vec::new()),
            submit_keys: AtomicU64::new(0),
            reads: Mutex::new(Vec::new()),
            target_override: None,
            read_clock: None,
            submission_error: None,
            submission_unknown: false,
        }
    }
}
impl HostPort for RegisteredPokeHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.inner.native_launch_capability()
    }
    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, ApiError> {
        self.inner
            .observe_current_target_for_archival(target, context)
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.reads.lock().unwrap().push(context.clone());
        if let Some((clock, at)) = &self.read_clock {
            clock.0.store(*at, Ordering::SeqCst);
        }
        self.inner.observe_current_target(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        self.inner.enumerate_targets(context)
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.target_override
            .clone()
            .or_else(|| self.inner.safe_wake_target(seat, observation))
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        if let Some(error) = &self.submission_error {
            return Err(error.clone());
        }
        let result = self.inner.submit_prompt(target, text, context)?;
        self.submissions
            .lock()
            .unwrap()
            .push((target.clone(), text.into(), context.clone()));
        Ok(if self.submission_unknown {
            PromptOutcome::OutcomeUnknown
        } else {
            result
        })
    }
    fn submit_prompt_during_turn(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.during_turn
            .lock()
            .unwrap()
            .push((target.clone(), text.into(), context.clone()));
        self.submit_prompt(target, text, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        self.inner.pane_agent_state(target, context)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        self.submit_keys.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.inner.launch_native(request, context)
    }
    fn stash_composer(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        self.inner.stash_composer(target, context)
    }
    fn restore_composer(
        &self,
        target: &SafeWakeTarget,
        saved: &str,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        self.inner.restore_composer(target, saved, context)
    }
}

struct RegisteredPokeCaps {
    selected: Harness,
    calls: Mutex<Vec<Harness>>,
    declared: PokeCapabilities,
}
impl PokeCapabilitySource for RegisteredPokeCaps {
    fn capabilities(&self, harness: Harness) -> PokeCapabilities {
        self.calls.lock().unwrap().push(harness);
        if harness == self.selected {
            self.declared
        } else {
            PokeCapabilities::NONE
        }
    }
}

/// Kills: dropping a registered cooperative bound identity before capability
/// selection. This injected seam is not proof of any shipped native capability.
#[test]
fn cooperative_registered_poke_identity_reaches_capability_source() {
    let ids = vec![
        "hermes",
        #[cfg(feature = "test-support")]
        "synthetic_fourth",
        "codex",
        "claude",
    ];
    let mut cases = Vec::new();
    for bound in ids {
        let selected = Harness::Agent(crate::harness::registry::builtins().agent(bound).unwrap());
        let caps = RegisteredPokeCaps {
            selected,
            calls: Mutex::new(Vec::new()),
            declared: DURING_TURN.0,
        };
        let host = RegisteredPokeHost::new(HostUiState::ActiveTurn);
        let clock = FakeClock(AtomicU64::new(0));
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
        let mut reservation = cooperative_reservation();
        if let ReservedWakeAuthority::Cooperative { harness, .. } = &mut reservation.authority {
            *harness = Some(bound.into());
        }
        let attempt = dispatcher
            .attempt_poke(
                reservation,
                &poke_plan_for_tests(),
                PokeMode::PokeOnly,
                &caps,
                &dispatch_context(),
            )
            .unwrap();
        let calls = caps.calls.lock().unwrap().clone();
        let submitted = host.during_turn.lock().unwrap().clone();
        eprintln!(
            "registered-poke case={bound} source={calls:?} outcome={:?} poked={} during_turn={}",
            attempt.outcome,
            attempt.poked,
            submitted.len()
        );
        assert!(host.inner.observation.occupant.is_none());
        assert_eq!(
            host.inner.observation.occupancy,
            StructuralOccupancy::Unknown
        );
        assert!(matches!(
            host.inner.observation.execution,
            ExecutionEvidence::Unknown
        ));
        assert_eq!(host.inner.stash_calls.load(Ordering::SeqCst), 0);
        assert!(host.inner.restored.lock().unwrap().is_empty());
        assert_eq!(host.submit_keys.load(Ordering::SeqCst), 0);
        cases.push((
            bound,
            selected,
            calls,
            submitted,
            attempt,
            host.inner.prompts.lock().unwrap().clone(),
        ));
    }
    for (bound, selected, calls, submitted, attempt, prompts) in cases {
        assert_eq!(
            calls,
            [selected],
            "{bound}: exact registered identity reaches source once"
        );
        assert_eq!(attempt.outcome, WakeOutcome::Submitted, "{bound}");
        assert!(attempt.poked, "{bound}");
        assert_eq!(prompts, [POKE_TEXT], "{bound}");
        assert_eq!(submitted.len(), 1, "{bound}: during-turn submission");
        assert_eq!(submitted[0].0.bound_harness.as_deref(), Some(bound));
        assert_eq!(submitted[0].1, POKE_TEXT);
        assert_eq!(submitted[0].2.budget.deadline, MonoInstant(2_000));
        assert_eq!(submitted[0].2.expected_boot, Some(HostBootId::new("boot")));
        assert_eq!(submitted[0].2.expected_epoch, Some(1));
    }
}

fn registered_poke_reservation(bound: Option<&str>) -> WakeReservation {
    let mut reservation = cooperative_reservation();
    if let ReservedWakeAuthority::Cooperative { harness, .. } = &mut reservation.authority {
        *harness = bound.map(str::to_owned);
    }
    reservation
}

fn registered_poke_caps(bound: &str, declared: PokeCapabilities) -> RegisteredPokeCaps {
    RegisteredPokeCaps {
        selected: Harness::Agent(crate::harness::registry::builtins().agent(bound).unwrap()),
        calls: Mutex::new(Vec::new()),
        declared,
    }
}

fn assert_no_registered_poke_effects(host: &RegisteredPokeHost) {
    assert!(host.inner.prompts.lock().unwrap().is_empty());
    assert!(host.during_turn.lock().unwrap().is_empty());
    assert_eq!(host.inner.stash_calls.load(Ordering::SeqCst), 0);
    assert!(host.inner.restored.lock().unwrap().is_empty());
    assert_eq!(host.submit_keys.load(Ordering::SeqCst), 0);
}

/// Kills: registration or missing optional composer granting rich behavior,
/// and a poke-carrying ordinary wake stashing a person's draft.
#[test]
fn cooperative_registered_poke_none_stays_conservative() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    for registration in crate::harness::registry::builtins().registrations() {
        let bound = registration.metadata().id;
        for (ui, mode, declared, outcome, poked, prompt, stash) in [
            (
                HostUiState::ActiveTurn,
                PokeMode::PokeOnly,
                PokeCapabilities::NONE,
                WakeOutcome::Unsafe,
                false,
                None,
                false,
            ),
            (
                HostUiState::HumanInput,
                PokeMode::PokeOnly,
                PokeCapabilities::NONE,
                WakeOutcome::Unsafe,
                false,
                None,
                false,
            ),
            (
                HostUiState::Idle,
                PokeMode::PokeOnly,
                PokeCapabilities::NONE,
                WakeOutcome::Submitted,
                true,
                Some(POKE_TEXT),
                false,
            ),
            (
                HostUiState::HumanInput,
                PokeMode::WithWake,
                PokeCapabilities::NONE,
                WakeOutcome::Submitted,
                false,
                Some(crate::notification::policy::MARKER),
                false,
            ),
            (
                HostUiState::HumanInput,
                PokeMode::WithWake,
                STASH.0,
                WakeOutcome::Submitted,
                false,
                Some(crate::notification::policy::MARKER),
                false,
            ),
            (
                HostUiState::HumanInput,
                PokeMode::PokeOnly,
                STASH.0,
                WakeOutcome::Submitted,
                true,
                Some(POKE_TEXT),
                true,
            ),
        ] {
            let host = RegisteredPokeHost::new(ui);
            let caps = registered_poke_caps(bound, declared);
            let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
            let attempt = dispatcher
                .attempt_poke(
                    registered_poke_reservation(Some(bound)),
                    &poke_plan_for_tests(),
                    mode,
                    &caps,
                    &dispatch_context(),
                )
                .unwrap();
            assert_eq!(
                (attempt.outcome, attempt.poked),
                (outcome, poked),
                "{bound} {ui:?} {mode:?}"
            );
            assert_eq!(*caps.calls.lock().unwrap(), [caps.selected]);
            assert_eq!(
                *host.inner.prompts.lock().unwrap(),
                prompt.into_iter().map(str::to_owned).collect::<Vec<_>>()
            );
            assert_eq!(
                host.inner.stash_calls.load(Ordering::SeqCst),
                u64::from(stash)
            );
            assert_eq!(
                *host.inner.restored.lock().unwrap(),
                if stash {
                    vec!["typed".to_owned()]
                } else {
                    vec![]
                }
            );
            assert_eq!(host.submit_keys.load(Ordering::SeqCst), 0);
            for (target, text, _) in host.submissions.lock().unwrap().iter() {
                assert_eq!(target.bound_harness.as_deref(), Some(bound));
                assert_eq!(Some(text.as_str()), prompt);
            }
            assert!(host.inner.observation.occupant.is_none());
            eprintln!(
                "none-control case={bound} ui={ui:?} mode={mode:?} declared={declared:?} outcome={outcome:?} poked={poked} stash={stash}"
            );
        }
    }
}

/// Kills: string presence, Human or a child being promoted to recognized
/// agent; also kills bound fallback overriding the verified occupant identity.
#[test]
fn cooperative_poke_invalid_identity_never_calls_capability_source() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    for (label, bound, occupant) in [
        ("unknown", Some("unregistered"), None),
        ("human", Some("human"), None),
        ("Human", Some("Human"), None),
        ("context spelling", Some("Hermes"), None),
        ("absent", None, None),
        (
            "explicit Human",
            Some("hermes"),
            Some((Harness::Human, true)),
        ),
        ("child", Some("hermes"), Some((Harness::Codex, false))),
    ] {
        let mut host = RegisteredPokeHost::new(HostUiState::Idle);
        host.inner.observation.occupant = occupant.map(|(harness, is_top_level)| NativeOccupant {
            harness,
            is_top_level,
            session: NativeSessionId::new("session"),
            execution: ExecutionId::new("execution"),
        });
        let caps = registered_poke_caps("hermes", DURING_TURN.0);
        let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
        let attempt = dispatcher
            .attempt_poke(
                registered_poke_reservation(bound),
                &poke_plan_for_tests(),
                PokeMode::PokeOnly,
                &caps,
                &dispatch_context(),
            )
            .unwrap();
        assert_eq!(attempt.outcome, WakeOutcome::Unsafe, "{label}");
        assert!(!attempt.poked);
        assert!(caps.calls.lock().unwrap().is_empty(), "{label}");
        assert_no_registered_poke_effects(&host);
        eprintln!("invalid-identity case={label} source=0 prompts=0");
    }
    // Registered-basis evidence uses the observed occupant, even when the
    // injected target carries a different bound hint.
    let mut host = RegisteredPokeHost::new(HostUiState::ActiveTurn);
    host.inner.observation = fresh_observation();
    host.inner.observation.ui = HostUiState::ActiveTurn;
    let mut target = host
        .inner
        .safe_wake_target(&SeatId::new("seat"), &host.inner.observation)
        .unwrap();
    target.basis = crate::ports::WakeTargetBasis::VerifiedOccupant {
        session: NativeSessionId::new("session"),
        execution: ExecutionId::new("execution"),
    };
    target.bound_harness = Some("hermes".into());
    host.target_override = Some(target);
    let reservation = WakeReservation {
        authority: ReservedWakeAuthority::Registered {
            binding_generation: 1,
            execution: ExecutionId::new("execution"),
        },
        ..test_reservation()
    };
    let caps = registered_poke_caps("codex", DURING_TURN.0);
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            reservation,
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &caps,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(*caps.calls.lock().unwrap(), [Harness::Codex]);
    assert_eq!(
        (attempt.outcome, attempt.poked),
        (WakeOutcome::Submitted, true)
    );
    assert_eq!(*host.inner.prompts.lock().unwrap(), [POKE_TEXT]);
}

struct RegisteredPokeCheck {
    result: Result<bool, ApiError>,
    clock: Option<(Arc<FakeClock>, u64)>,
    budgets: Mutex<Vec<CallBudget>>,
}
impl ReservationCheck for RegisteredPokeCheck {
    fn is_current(&self, _: &WakeReservation, budget: &CallBudget) -> Result<bool, ApiError> {
        self.budgets.lock().unwrap().push(budget.clone());
        if let Some((clock, at)) = &self.clock {
            clock.0.store(*at, Ordering::SeqCst);
        }
        self.result.clone()
    }
}

/// Kills: identity selection bypassing structural, UI, current-reservation or
/// original deadline/cancellation fences, and failed sends marking a poke.
#[test]
fn cooperative_registered_poke_preserves_fences_and_budget() {
    type Change = Box<dyn Fn(&mut RegisteredPokeHost)>;
    let mut changes: Vec<(&str, Change)> = vec![
        (
            "target",
            Box::new(|h| h.inner.observation.target = HostTargetId::new("other")),
        ),
        (
            "boot",
            Box::new(|h| h.inner.observation.host_boot = HostBootId::new("other")),
        ),
        ("epoch", Box::new(|h| h.inner.observation.epoch = 2)),
        (
            "generation",
            Box::new(|h| h.inner.observation.generation = 2),
        ),
        (
            "terminal",
            Box::new(|h| h.inner.observation.terminal = Some(TerminalId::new("other"))),
        ),
        (
            "stale",
            Box::new(|h| {
                h.inner.observation.provenance = ObservationProvenance::UncharacterizedCache
            }),
        ),
        (
            "incarnation",
            Box::new(|h| {
                h.inner.observation.incarnation = IncarnationEvidence::Verified {
                    identity: "other".into(),
                    evidence_kind: EvidenceKind::NativeCurrentTarget,
                }
            }),
        ),
        (
            "empty shell",
            Box::new(|h| h.inner.observation.occupancy = StructuralOccupancy::EmptyShell),
        ),
        (
            "basis",
            Box::new(|h| {
                let mut target = h
                    .inner
                    .safe_wake_target(&SeatId::new("seat"), &h.inner.observation)
                    .unwrap();
                target.basis = crate::ports::WakeTargetBasis::VerifiedOccupant {
                    session: NativeSessionId::new("session"),
                    execution: ExecutionId::new("execution"),
                };
                h.target_override = Some(target);
            }),
        ),
    ];
    for field in [
        "seat",
        "target",
        "boot",
        "epoch",
        "generation",
        "terminal",
        "incarnation",
    ] {
        changes.push((
            field,
            Box::new(move |host| {
                let mut target = host
                    .inner
                    .safe_wake_target(&SeatId::new("seat"), &host.inner.observation)
                    .unwrap();
                match field {
                    "seat" => target.seat = SeatId::new("other"),
                    "target" => target.target = HostTargetId::new("other"),
                    "boot" => target.host_boot = HostBootId::new("other"),
                    "epoch" => target.epoch = 2,
                    "generation" => target.generation = 2,
                    "terminal" => target.terminal = TerminalId::new("other"),
                    "incarnation" => target.incarnation = "other".into(),
                    _ => unreachable!(),
                }
                host.target_override = Some(target);
            }),
        ));
    }
    for (label, change) in changes {
        for mode in [PokeMode::PokeOnly, PokeMode::WithWake] {
            let clock = FakeClock(AtomicU64::new(0));
            let check = FakeReservationCheck {
                current: true,
                calls: AtomicU64::new(0),
            };
            let mut host = RegisteredPokeHost::new(HostUiState::Idle);
            change(&mut host);
            let caps = registered_poke_caps("hermes", DURING_TURN.0);
            let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
            let attempt = dispatcher
                .attempt_poke(
                    registered_poke_reservation(Some("hermes")),
                    &poke_plan_for_tests(),
                    mode,
                    &caps,
                    &dispatch_context(),
                )
                .unwrap();
            assert_eq!(
                attempt.outcome,
                if mode == PokeMode::PokeOnly {
                    WakeOutcome::Unsafe
                } else {
                    WakeOutcome::Refused(RefusalCause::Unsafe)
                },
                "{label} {mode:?}"
            );
            assert!(!attempt.poked);
            assert!(caps.calls.lock().unwrap().is_empty());
            assert_eq!(check.calls.load(Ordering::SeqCst), 0);
            assert_no_registered_poke_effects(&host);
            eprintln!("fence case={label} mode={mode:?} source=0 prompts=0");
        }
    }
    for (ui, focused) in [
        (HostUiState::Idle, true),
        (HostUiState::Unknown, false),
        (HostUiState::ApprovalOrQuestion, false),
    ] {
        let clock = FakeClock(AtomicU64::new(0));
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let mut host = RegisteredPokeHost::new(ui);
        host.inner.observation.focused = focused;
        let caps = registered_poke_caps("hermes", DURING_TURN.0);
        let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
        let attempt = dispatcher
            .attempt_poke(
                registered_poke_reservation(Some("hermes")),
                &poke_plan_for_tests(),
                PokeMode::PokeOnly,
                &caps,
                &dispatch_context(),
            )
            .unwrap();
        assert_eq!(
            (attempt.outcome, attempt.poked),
            (WakeOutcome::Unsafe, false)
        );
        assert_eq!(*caps.calls.lock().unwrap(), [caps.selected]);
        assert_no_registered_poke_effects(&host);
        eprintln!("ui-fence ui={ui:?} focused={focused} source=1 prompts=0");
    }
    for mode in [PokeMode::PokeOnly, PokeMode::WithWake] {
        for case in [
            "exhausted",
            "cancelled",
            "read deadline",
            "current false",
            "current error",
            "prompt deadline",
            "decreasing",
            "unknown send",
            "rejected send",
        ] {
            let clock = Arc::new(FakeClock(AtomicU64::new(0)));
            let mut host = RegisteredPokeHost::new(HostUiState::Idle);
            let mut context = dispatch_context();
            let mut check = RegisteredPokeCheck {
                result: Ok(true),
                clock: None,
                budgets: Mutex::new(Vec::new()),
            };
            match case {
                "exhausted" => context.budget.deadline = MonoInstant(0),
                "cancelled" => context.budget.cancellation.cancel(),
                "read deadline" => host.read_clock = Some((clock.clone(), 750)),
                "current false" => check.result = Ok(false),
                "current error" => {
                    check.result = Err(ApiError::new(
                        ErrorCode::HostUnavailable,
                        "check unavailable",
                    ))
                }
                "prompt deadline" => check.clock = Some((clock.clone(), 10_000)),
                "decreasing" => {
                    context.budget.deadline = MonoInstant(600);
                    host.read_clock = Some((clock.clone(), 200));
                }
                "unknown send" => host.submission_unknown = true,
                "rejected send" => {
                    host.submission_error =
                        Some(ApiError::new(ErrorCode::TargetUnsafe, "prompt rejected"))
                }
                _ => unreachable!(),
            }
            let caps = registered_poke_caps("hermes", DURING_TURN.0);
            let dispatcher = NativeWakeDispatcher::new(&host, &check, clock.as_ref());
            let result = dispatcher.attempt_poke(
                registered_poke_reservation(Some("hermes")),
                &poke_plan_for_tests(),
                mode,
                &caps,
                &context,
            );
            if case == "current error" && mode == PokeMode::PokeOnly || case == "rejected send" {
                assert_eq!(
                    result.unwrap_err().code,
                    if case == "rejected send" {
                        ErrorCode::TargetUnsafe
                    } else {
                        ErrorCode::HostUnavailable
                    }
                );
            } else {
                let attempt = result.unwrap();
                let plain = match case {
                    "exhausted" | "cancelled" | "read deadline" | "prompt deadline" => {
                        WakeOutcome::TimedOut
                    }
                    "current false" => WakeOutcome::Unsafe,
                    "current error" => WakeOutcome::Refused(RefusalCause::Unavailable),
                    "decreasing" => WakeOutcome::Submitted,
                    "unknown send" => WakeOutcome::OutcomeUnknown,
                    _ => unreachable!(),
                };
                let expected = if mode == PokeMode::WithWake && plain == WakeOutcome::TimedOut {
                    WakeOutcome::Refused(RefusalCause::TimedOut)
                } else if mode == PokeMode::WithWake && plain == WakeOutcome::Unsafe {
                    WakeOutcome::Refused(RefusalCause::Unsafe)
                } else {
                    plain
                };
                assert_eq!(attempt.outcome, expected, "{case} {mode:?}");
                assert_eq!(attempt.poked, case == "decreasing");
            }
            let before_source = matches!(case, "exhausted" | "cancelled" | "read deadline");
            assert_eq!(
                caps.calls.lock().unwrap().len(),
                usize::from(!before_source)
            );
            if !matches!(case, "decreasing" | "unknown send") {
                assert_no_registered_poke_effects(&host);
            }
            for read in host.reads.lock().unwrap().iter() {
                assert!(read.budget.deadline.0 <= context.budget.deadline.0);
            }
            for budget in check.budgets.lock().unwrap().iter() {
                assert_eq!(budget.deadline, context.budget.deadline);
            }
            for (_, _, prompt_context) in host.submissions.lock().unwrap().iter() {
                assert!(prompt_context.budget.deadline.0 <= context.budget.deadline.0);
                if case == "decreasing" {
                    assert_eq!(prompt_context.budget.deadline, MonoInstant(600));
                    assert_eq!(
                        host.reads.lock().unwrap()[0].budget.deadline,
                        MonoInstant(600)
                    );
                }
            }
            context.budget.cancellation.cancel();
            for read in host.reads.lock().unwrap().iter() {
                assert!(read.budget.cancellation.is_cancelled());
            }
            for budget in check.budgets.lock().unwrap().iter() {
                assert!(budget.cancellation.is_cancelled());
            }
            for (_, _, prompt_context) in host.submissions.lock().unwrap().iter() {
                assert!(prompt_context.budget.cancellation.is_cancelled());
            }
            eprintln!(
                "budget-fence case={case} mode={mode:?} source={} prompts={}",
                caps.calls.lock().unwrap().len(),
                host.inner.prompts.lock().unwrap().len()
            );
        }
    }
}

/// Kills: composer content refusing an ordinary wake, and a skipped poke
/// that is not confined to the poke.
#[test]
fn typed_draft_skips_the_poke_but_not_the_ordinary_wake() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let stash_failed = || {
        CooperativeStashingHost::over_typed_input(ComposerStash::Failed(
            crate::harness::composer::CLAUDE_NOT_KNOWN_EMPTY.into(),
        ))
    };
    let host = stash_failed();
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    assert_eq!(
        dispatcher
            .attempt_wake(cooperative_reservation(), &dispatch_context())
            .unwrap(),
        WakeOutcome::Submitted
    );
    assert_eq!(
        *host.prompts.lock().unwrap(),
        [crate::notification::policy::MARKER]
    );
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 0);

    // The poke over the same draft: its stash is refused, nothing is sent.
    let host = stash_failed();
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            cooperative_reservation(),
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &STASH,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Unsafe);
    assert!(!attempt.poked);
    assert!(host.prompts.lock().unwrap().is_empty());
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 1);

    // Without a declared stash the hook is not consulted: still skipped.
    let host = stash_failed();
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            cooperative_reservation(),
            &poke_plan_for_tests(),
            PokeMode::PokeOnly,
            &crate::ports::NoPokeCapabilities,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Unsafe);
    assert!(host.prompts.lock().unwrap().is_empty());
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 0);
}

/// Kills: a wake that carries a poke typing the poke text into a draft by way
/// of a stash.
#[test]
fn with_wake_over_typed_input_sends_the_plain_marker_without_stash() {
    let clock = FakeClock(AtomicU64::new(0));
    let check = FakeReservationCheck {
        current: true,
        calls: AtomicU64::new(0),
    };
    let host = CooperativeStashingHost::over_typed_input(ComposerStash::Saved("typed".into()));
    let dispatcher = NativeWakeDispatcher::new(&host, &check, &clock);
    let attempt = dispatcher
        .attempt_poke(
            cooperative_reservation(),
            &poke_plan_for_tests(),
            PokeMode::WithWake,
            &STASH,
            &dispatch_context(),
        )
        .unwrap();
    assert_eq!(attempt.outcome, WakeOutcome::Submitted);
    assert!(!attempt.poked);
    assert_eq!(
        *host.prompts.lock().unwrap(),
        [crate::notification::policy::MARKER]
    );
    assert_eq!(host.stash_calls.load(Ordering::SeqCst), 0);
    assert!(host.restored.lock().unwrap().is_empty());
}

/// Kills: a skipped poke advancing the wake retry step or its spacing.
#[test]
fn skipped_poke_leaves_the_wake_guard_unchanged() {
    let boot = daemon_boot();
    let seat = SeatId::new("seat");
    let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), boot);
    state.restore(seat.clone(), fresh()).unwrap();
    let attempt = WakeAttemptId::new("wake");
    let durable = state
        .reserved(seat.clone(), attempt.clone(), boot, MonoInstant(0))
        .unwrap();
    assert!(
        state
            .finish(&seat, &attempt, &boot, MonoInstant(2_000))
            .unwrap()
    );
    let eligible = |state: &DispatchState| {
        (
            state.can_reserve(&seat, MonoInstant(31_999)),
            state.can_reserve(&seat, MonoInstant(32_000)),
        )
    };
    assert_eq!(eligible(&state), (false, true));
    let poke = WakeAttemptId::new("poke");
    state
        .poke_reserved(seat.clone(), poke.clone(), boot)
        .unwrap();
    assert!(state.poke_finished(&seat, &poke, &boot, false, MonoInstant(40_000)));
    assert_eq!(eligible(&state), (false, true));
    // The durable retry is the one the wake left: restoring it is accepted
    // only when the guard still holds exactly that.
    state.restore(seat.clone(), durable).unwrap();
}

#[test]
fn batching_defers_reservation_and_host_without_postponing_later_arrivals() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events: events.clone(),
        fail_reservation: AtomicBool::new(false),
        batch: Some((UtcMillis(30_000), 30_000)),
    };
    let notifier = CompletionNotifier(Mutex::new(Vec::new()));
    let runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    let budget = CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        None
    );
    clock.0.store(29_999, Ordering::SeqCst);
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        None
    );
    assert!(events.lock().unwrap().is_empty());
    assert!(notifier.0.lock().unwrap().is_empty());
    clock.0.store(30_000, Ordering::SeqCst);
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(notifier.0.lock().unwrap().len(), 1);
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "complete"]);
}

struct DurableCleanupStore {
    inner: FakeWakeStore,
    windows: Vec<SeatId>,
    checks: Mutex<Vec<SeatId>>,
    fail_once: AtomicBool,
}
impl WakePort for DurableCleanupStore {
    fn clock(&self) -> &dyn Clock {
        self.inner.clock()
    }
    fn wake_batch_seats(
        &self,
        after: Option<&SeatId>,
        limit: u16,
        _: &CallBudget,
    ) -> Result<Vec<SeatId>, ApiError> {
        Ok(self
            .windows
            .iter()
            .filter(|seat| after.is_none_or(|after| *seat > after))
            .take(usize::from(limit))
            .cloned()
            .collect())
    }
    fn clear_wake_batch_if_empty(&self, seat: &SeatId, _: &CallBudget) -> Result<bool, ApiError> {
        self.checks.lock().unwrap().push(seat.clone());
        if self.fail_once.swap(false, Ordering::SeqCst) {
            return Err(ApiError::deadline_exceeded("injected cleanup cancellation"));
        }
        Ok(false)
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        self.inner.wake_candidates(page, budget)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.inner.reserve_wake(candidate, budget)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        prior: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.inner.complete_wake(attempt, outcome, prior, budget)
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        self.inner.wake_recovery_candidates(page, budget)
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        self.inner.recover_wake_reservation(request, budget)
    }
}

#[test]
fn durable_batch_cleanup_pages_untracked_windows_and_requeues_failed_check() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let store = DurableCleanupStore {
        inner: FakeWakeStore {
            clock: clock.clone(),
            events: Arc::new(Mutex::new(Vec::new())),
            fail_reservation: AtomicBool::new(false),
            batch: None,
        },
        windows: (0..20)
            .map(|n| SeatId::new(format!("seat-{n:02}")))
            .collect(),
        checks: Mutex::new(Vec::new()),
        fail_once: AtomicBool::new(true),
    };
    let due = FakeDeadlinePort {
        clock,
        due_calls: AtomicU64::new(0),
    };
    let notifier = CompletionNotifier(Mutex::new(Vec::new()));
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Cancellation::default(),
    };
    assert!(scheduler.drive_batch_cleanup(&budget).is_err());
    assert!(scheduler.drive_batch_cleanup(&budget).unwrap());
    assert_eq!(store.checks.lock().unwrap().len(), 17);
    {
        let checks = store.checks.lock().unwrap();
        assert_eq!(checks[0], checks[1]);
    }
    assert!(scheduler.drive_batch_cleanup(&budget).unwrap());
    assert_eq!(store.checks.lock().unwrap().len(), 21);
    assert!(scheduler.drive_batch_cleanup(&budget).unwrap()); // end-of-sweep resets cursor
    assert_eq!(store.checks.lock().unwrap().len(), 21);
    assert!(scheduler.drive_batch_cleanup(&budget).unwrap()); // starts next continuous sweep
    assert_eq!(store.checks.lock().unwrap().len(), 37);
    assert!(notifier.0.lock().unwrap().is_empty());
}
