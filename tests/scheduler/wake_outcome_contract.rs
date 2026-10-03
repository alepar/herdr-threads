//! Seam contract ht-p03.41: unsent-after-retry wake prompts, the pane-state
//! read, and the verification field that flows through `WakeDriveOutcome`.
use super::*;
use crate::{
    ports::{AgentComposerState, RefusalCause, WakeOutcome},
    scheduler::{SubmissionVerification, WakeDriveOutcome, outcome_for_verification},
};

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    }
}

fn wake_target(host: &FakeNativeHost) -> SafeWakeTarget {
    host.safe_wake_target(&SeatId::new("seat"), &host.observation)
        .unwrap()
}

#[test]
fn unsent_after_retry_maps_to_outcome_unknown_and_reads_unsubmitted() {
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let target = wake_target(&host);
    host.script_pane_states(&target, vec![AgentComposerState::HoldingPrompt]);
    assert_eq!(
        host.pane_agent_state(&target, &context()).unwrap(),
        AgentComposerState::HoldingPrompt
    );
    let verification = SubmissionVerification::Unsubmitted;
    assert_eq!(
        outcome_for_verification(verification),
        WakeOutcome::OutcomeUnknown
    );
    assert_eq!(verification.as_str(), "unsubmitted");
}

#[test]
fn scripted_states_advance_then_repeat_the_last() {
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let target = wake_target(&host);
    let ctx = context();
    assert_eq!(
        host.pane_agent_state(&target, &ctx).unwrap(),
        AgentComposerState::Submitted,
        "unscripted target defaults to Submitted"
    );
    host.script_pane_states(
        &target,
        vec![
            AgentComposerState::HoldingPrompt,
            AgentComposerState::Submitted,
        ],
    );
    let read: Vec<_> = (0..3)
        .map(|_| host.pane_agent_state(&target, &ctx).unwrap())
        .collect();
    assert_eq!(
        read,
        vec![
            AgentComposerState::HoldingPrompt,
            AgentComposerState::Submitted,
            AgentComposerState::Submitted
        ]
    );
}

#[test]
fn verified_maps_to_submitted() {
    for v in [
        SubmissionVerification::Verified,
        SubmissionVerification::Retried,
        SubmissionVerification::NotChecked,
    ] {
        assert_eq!(outcome_for_verification(v), WakeOutcome::Submitted, "{v:?}");
    }
    assert_eq!(SubmissionVerification::Verified.as_str(), "verified");
    assert_eq!(SubmissionVerification::Retried.as_str(), "retried");
    assert_eq!(SubmissionVerification::NotChecked.as_str(), "not_checked");
    assert_eq!(
        SubmissionVerification::default(),
        SubmissionVerification::NotChecked
    );
}

/// Behaves like `NativeWakeDispatcher`'s verification map: an entry is
/// stored only when the attempt sent a prompt (`on_send` is `Some` and the
/// outcome is `Submitted`), and `take_verification` removes it.
struct VerifyingNotifier {
    outcome: WakeOutcome,
    on_send: Option<SubmissionVerification>,
    map: Mutex<std::collections::HashMap<SeatId, SubmissionVerification>>,
}
impl VerifyingNotifier {
    fn new(outcome: WakeOutcome, on_send: Option<SubmissionVerification>) -> Self {
        Self {
            outcome,
            on_send,
            map: Mutex::default(),
        }
    }
}
impl NotificationPort for VerifyingNotifier {
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        if let (WakeOutcome::Submitted, Some(v)) = (self.outcome, self.on_send) {
            self.map.lock().unwrap().insert(reservation.seat.clone(), v);
        }
        Ok(self.outcome)
    }
    fn take_verification(&self, seat: &SeatId) -> Option<SubmissionVerification> {
        self.map.lock().unwrap().remove(seat)
    }
}

fn fake_wake_store() -> FakeWakeStore {
    FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: Arc::new(Mutex::new(Vec::new())),
        fail_reservation: AtomicBool::new(false),
    }
}

fn drive_once(notifier: &dyn NotificationPort, wake: &FakeWakeStore) -> WakeDriveOutcome {
    let due = FakeDeadlinePort {
        clock: wake.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        wake,
        notifier,
        RetryConfig::default(),
        daemon_boot(),
    );
    let budget = CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    };
    scheduler.drive_wakes(&budget).unwrap()
}

#[test]
fn drive_outcome_has_no_verification_when_the_notifier_reports_none() {
    let wake = fake_wake_store();
    let notifier = FakeNotifier {
        events: wake.events.clone(),
    };
    let outcome = drive_once(&notifier, &wake);
    assert_eq!(outcome.attempted, 1);
    assert!(
        outcome.verification.is_empty(),
        "{:?}",
        outcome.verification
    );
}

#[test]
fn unsent_attempts_report_no_verification() {
    for outcome in [
        WakeOutcome::Refused(RefusalCause::Unavailable),
        WakeOutcome::Refused(RefusalCause::Unsafe),
        WakeOutcome::Refused(RefusalCause::TimedOut),
        WakeOutcome::Cancelled,
        WakeOutcome::OutcomeUnknown,
    ] {
        let wake = fake_wake_store();
        let notifier = VerifyingNotifier::new(outcome, Some(SubmissionVerification::NotChecked));
        let driven = drive_once(&notifier, &wake);
        assert_eq!(driven.attempted, 1, "{outcome:?}");
        assert!(driven.verification.is_empty(), "{outcome:?}");
    }
}

#[test]
fn sent_attempts_report_their_verification() {
    for v in [
        SubmissionVerification::Verified,
        SubmissionVerification::Retried,
        SubmissionVerification::NotChecked,
        SubmissionVerification::Unsubmitted,
    ] {
        let wake = fake_wake_store();
        let notifier = VerifyingNotifier::new(WakeOutcome::Submitted, Some(v));
        let driven = drive_once(&notifier, &wake);
        assert_eq!(driven.verification, vec![(SeatId::new("seat"), v)], "{v:?}");
    }
}

#[test]
fn stale_verification_is_drained_and_never_reported_for_an_unsent_attempt() {
    let wake = fake_wake_store();
    wake.fail_reservation.store(true, Ordering::SeqCst);
    let notifier = VerifyingNotifier::new(WakeOutcome::Refused(RefusalCause::Unavailable), None);
    notifier
        .map
        .lock()
        .unwrap()
        .insert(SeatId::new("seat"), SubmissionVerification::NotChecked);
    let due = FakeDeadlinePort {
        clock: wake.clock.clone(),
        due_calls: AtomicU64::new(0),
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
    assert!(scheduler.drive_wakes(&budget).is_err());
    assert!(
        notifier.map.lock().unwrap().is_empty(),
        "the failed try must drain the stale entry"
    );
    let second = scheduler.drive_wakes(&budget).unwrap();
    assert_eq!(second.attempted, 1);
    assert!(second.verification.is_empty(), "{:?}", second.verification);
}

#[test]
fn native_stub_reports_unknown() {
    let host = FakeNativeHost {
        observation: fresh_observation(),
        submitted: AtomicU64::new(0),
        pane_states: Default::default(),
        submit_keys: AtomicU64::new(0),
    };
    let target = wake_target(&host);
    let cli = crate::host::native::NativeCli::new(
        std::env::temp_dir().join(format!("herdr-no-socket-{}", uuid::Uuid::new_v4())),
        Arc::new(FakeClock(AtomicU64::new(0))),
    );
    assert_eq!(
        cli.pane_agent_state(&target, &context()).unwrap(),
        AgentComposerState::Unknown
    );
}
