use super::{Scheduler, WakePort, WakeRunner};
use crate::{
    notification::dispatch::{
        DispatchError, DispatchState, NativeWakeDispatcher, ReservationCheck,
    },
    notification::policy::{DurableRetry, RetryConfig},
    ports::StorePort,
    ports::{DuePhaseProgress, DueScanProgress, DueScanRequest, RetirementProgress, WorkAdmission},
    ports::{
        EvidenceKind, ExecutionEvidence, HostCallContext, HostLifecycleSubscription,
        HostObservation, HostPort, HostSnapshot, HostUiState, IncarnationEvidence,
        LogicalAttentionFrontier, LogicalPublicationKey, NativeLaunchCapability,
        NativeLaunchOutcome, NativeLaunchRequest, NativeOccupant, NotificationPort,
        ObservationProvenance, PromptOutcome, ReservedWakeAuthority, SafeWakeTarget,
        StructuralOccupancy, WakeAttentionWitness, WakeCandidate, WakeOutcome,
        WakeRecoveryCandidate, WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation,
        WarningOfferFrontier,
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
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
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
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        self.events.lock().unwrap().push("reserve");
        if self.fail_reservation.swap(false, Ordering::SeqCst) {
            return Err(ApiError {
                code: ErrorCode::StoreCorrupt,
                detail: "injected failed commit".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            });
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
        }))
    }
    fn complete_wake(
        &self,
        _: WakeAttemptId,
        outcome: WakeOutcome,
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        assert!(matches!(
            outcome,
            WakeOutcome::Submitted | WakeOutcome::OutcomeUnknown | WakeOutcome::Cancelled
        ));
        self.events.lock().unwrap().push("complete");
        Ok(())
    }
}

struct FakeDeadlinePort {
    clock: Arc<FakeClock>,
    due_calls: AtomicU64,
}
impl crate::scheduler::deadlines::DeadlinePort for FakeDeadlinePort {
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

#[test]
fn four_connected_owned_calls_block_a_fifth_until_their_transports_exit() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = FakeWakeStore {
        clock: clock.clone(),
        events: events.clone(),
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
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        self.completions.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        Ok(())
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
    let reopened = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings {
            daemon_boot: Some(daemon_boot()),
            minimum_wake_delay_ms: 30_000,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(100_000))),
        due_calls: AtomicU64::new(0),
    };
    let host = SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let notifier = NativeWakeDispatcher::new(&host, &reopened, clock.as_ref());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &reopened,
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
            &reopened,
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
        &reopened,
        old_reservation.attempt,
        WakeOutcome::Submitted,
        &budget,
    )
    .unwrap();
    drop(scheduler);
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
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
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
        StorePort::complete_wake(self.store, attempt, outcome, budget)?;
        if self.lose_response.swap(false, Ordering::SeqCst) {
            return Err(completion_error());
        }
        Ok(())
    }
}
fn completion_error() -> ApiError {
    ApiError {
        code: ErrorCode::StoreCorrupt,
        detail: "injected completion failure".into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
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
    let notifier = NativeWakeDispatcher::new(&host, &store, clock.as_ref());
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
        let notifier = NativeWakeDispatcher::new(&host, &store, clock.as_ref());
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
        let notifier = NativeWakeDispatcher::new(&host, &store, clock.as_ref());
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
    fn subscribe_lifecycle(
        &self,
        context: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
        self.host.subscribe_lifecycle(context)
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
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        self.host.launch_native(request, context)
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
    use crate::service::workers::{FairWriter, WorkerStatus, start_wake_worker};
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
    let worker = start_wake_worker(
        store.clone(),
        writer.clone(),
        host.clone(),
        "i".into(),
        daemon_boot(),
        RetryConfig::default(),
        cancellation.clone(),
        status.clone(),
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
    fn subscribe_lifecycle(
        &self,
        _: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
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
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
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
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), clock.clone()),
        "i",
        StoreSettings {
            daemon_boot: Some(daemon_boot()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let host = SqliteTimingHost {
        context: StoreContext::new(path.clone(), clock.clone()),
        clock: clock.clone(),
        calls: Mutex::new(Vec::new()),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &store, clock.as_ref());
    let due = FakeDeadlinePort {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &dispatch,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(300_000),
        cancellation: Cancellation::default(),
    };
    clock.mono.store(30_000, Ordering::SeqCst);
    let partial = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(
        (partial.examined, partial.attempted, partial.has_more),
        (0, 0, true)
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
    let partial = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(
        (partial.examined, partial.attempted, partial.has_more),
        (0, 0, true)
    );
    assert!(host.calls.lock().unwrap().is_empty());
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
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
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 0);
    assert_eq!(scheduler.drive_wakes(&budget).unwrap().attempted, 1);
    assert_eq!(
        *host.calls.lock().unwrap(),
        vec![MonoInstant(30_000), MonoInstant(62_000)]
    );
    drop(scheduler);
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
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.0.complete_wake(attempt, outcome, budget)
    }
}

#[test]
fn cancellation_after_commit_completes_the_attempt_without_host_io() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let base = FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
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
}
impl HostPort for FakeNativeHost {
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
    fn subscribe_lifecycle(
        &self,
        _: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
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
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
}
fn fresh_observation() -> HostObservation {
    HostObservation {
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
        WakeOutcome::Unsafe
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
    assert_eq!(check.calls.load(Ordering::SeqCst), 0);

    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
    };
    let check = FakeReservationCheck {
        current: false,
        calls: AtomicU64::new(0),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
    assert_eq!(
        dispatch.attempt_wake(test_reservation(), &context).unwrap(),
        WakeOutcome::Unsafe
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
        WakeOutcome::Unsafe
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
}

#[test]
fn safe_unregistered_recovery_hint_only_submits_the_fixed_marker() {
    let clock = FakeClock(AtomicU64::new(0));
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
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
        };
        let check = FakeReservationCheck {
            current: true,
            calls: AtomicU64::new(0),
        };
        let dispatch = NativeWakeDispatcher::new(&host, &check, &clock);
        assert_eq!(
            dispatch.attempt_wake(test_reservation(), &context).unwrap(),
            WakeOutcome::Unsafe
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
        WakeOutcome::Unsafe
    );
    assert_eq!(host.submitted.load(Ordering::SeqCst), 0);
}

struct LateReadHost {
    inner: FakeNativeHost,
    clock: Arc<FakeClock>,
}
impl HostPort for LateReadHost {
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
    fn subscribe_lifecycle(
        &self,
        context: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
        self.inner.subscribe_lifecycle(context)
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
}

#[test]
fn target_read_must_finish_before_its_seven_hundred_fifty_millisecond_deadline() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let host = LateReadHost {
        inner: FakeNativeHost {
            observation: fresh_observation(),
            submitted: AtomicU64::new(0),
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
        WakeOutcome::TimedOut
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
        WakeOutcome::Unsafe
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
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), clock.clone()),
            "i",
            StoreSettings {
                daemon_boot: Some(daemon_boot()),
                ..StoreSettings::default()
            },
        )
        .unwrap();
        let dispatch = NativeWakeDispatcher::new(&cli, &store, clock.as_ref());
        let scheduler = Scheduler::new(
            "i".into(),
            &store,
            &store,
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
        ("human input", Box::new(|o| o.ui = HostUiState::HumanInput)),
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
            WakeOutcome::Unsafe,
            "{label}"
        );
        assert_eq!(host.submitted.load(Ordering::SeqCst), 0, "{label}");
        assert_eq!(check.calls.load(Ordering::SeqCst), 0, "{label}");
    }
}

/// A cooperative-basis host that records the target handed to `submit_prompt`.
struct CooperativeRecordingHost {
    observation: HostObservation,
    prompted: std::sync::Mutex<Vec<SafeWakeTarget>>,
}
impl HostPort for CooperativeRecordingHost {
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
    fn subscribe_lifecycle(
        &self,
        _: &HostCallContext,
    ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
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
    assert_eq!(
        dispatch.attempt_wake(reservation, &context).unwrap(),
        WakeOutcome::Unsafe
    );
    assert!(host.prompted.lock().unwrap().is_empty());
}
