//! Wake receipt (ht-wake-loop): a submitted wake is received when the session makes any daemon
//! call claiming the seat before the ladder's next retry. Without one the wake is recorded
//! `unreceived` once and not retyped until the seat is heard from again or its targets change.
use super::*;
use crate::{
    ports::{HookPullOutlook, WakeTarget},
    protocol::ids::BindingId,
    scheduler::SubmissionVerification,
};

const STEPS: [u64; 4] = [30_000, 60_000, 120_000, 300_000];

/// Keeps the durable ladder columns a real store would return after each reservation.
struct LadderStore {
    clock: Arc<FakeClock>,
    events: Arc<Mutex<Vec<String>>>,
    reserved: AtomicU64,
    targets: Mutex<Vec<&'static str>>,
    /// The invitation frontier's decision sequence: a larger one is a newer item.
    newest: AtomicU64,
}
impl LadderStore {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            clock: Arc::new(FakeClock(AtomicU64::new(1_000))),
            events,
            reserved: AtomicU64::new(0),
            targets: Mutex::new(vec!["binding_a"]),
            newest: AtomicU64::new(1),
        }
    }
    fn advance(&self, millis: u64) {
        self.clock.0.fetch_add(millis, Ordering::SeqCst);
    }
    fn candidate(&self) -> WakeCandidate {
        let reserved = self.reserved.load(Ordering::SeqCst);
        let mut candidate = due_candidate();
        candidate.attention_witness = Some(WakeAttentionWitness::from_complete(
            "34750f98bd59fcfc946da45aaabe933b".into(),
            SeatId::new("seat"),
            1,
            true,
            false,
            None,
            0,
            false,
            LogicalAttentionFrontier {
                invitation: Some(LogicalPublicationKey {
                    decision_seq: self.newest.load(Ordering::SeqCst),
                    event_offset: 0,
                }),
                ..Default::default()
            },
        ));
        candidate.targets = self
            .targets
            .lock()
            .unwrap()
            .iter()
            .map(|binding| WakeTarget {
                provenance: String::new(),
                binding: BindingId::new(*binding),
                harness: "codex".into(),
                fingerprint_pid: None,
            })
            .collect();
        if reserved > 0 {
            let step = (reserved - 1).min(3);
            candidate.retry_step = u32::try_from(step).unwrap();
            candidate.minimum_delay_ms = STEPS[0];
            candidate.effective_delay_ms = STEPS[step as usize];
            candidate.last_reservation_id = Some(WakeAttemptId::new(format!("attempt-{reserved}")));
            candidate.last_reserved_frontier =
                candidate.attention_witness.as_ref().unwrap().frontier();
        }
        candidate
    }
}
impl WakePort for LadderStore {
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
            items: vec![self.candidate()],
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
        Ok(None)
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        _: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let n = self.reserved.fetch_add(1, Ordering::SeqCst) + 1;
        self.events
            .lock()
            .unwrap()
            .push(format!("reserve attempt-{n}"));
        Ok(Some(WakeReservation {
            attempt: WakeAttemptId::new(format!("attempt-{n}")),
            seat: candidate.seat.clone(),
            attention_witness: candidate.attention_witness.clone().unwrap(),
            reasons: vec!["invitation".into()],
            retained_effective_delay_ms: 30_000,
            lease_until: MonoInstant(self.clock.monotonic_now().0 + 5_000),
            retained_minimum_delay_ms: 30_000,
            reserved_at_utc: UtcMillis(i64::try_from(n).unwrap() * 100),
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
        _: WakeOutcome,
        _: Option<&PriorLadder>,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        Ok(false)
    }
    fn mark_wake_unreceived(
        &self,
        seat: &SeatId,
        attempt: &WakeAttemptId,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.events.lock().unwrap().push(format!(
            "unreceived {} {}",
            seat.as_str(),
            attempt.as_str()
        ));
        Ok(true)
    }
}

/// `.1`: the typed wake is seen still in the composer (`Unsubmitted`, an `OutcomeUnknown`).
struct SubmittingNotifier(
    Arc<Mutex<Vec<String>>>,
    bool,
    Mutex<Option<SubmissionVerification>>,
);
impl NotificationPort for SubmittingNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.0.lock().unwrap().push("typed".into());
        let (verification, outcome) = if self.1 {
            (
                SubmissionVerification::Unsubmitted,
                WakeOutcome::OutcomeUnknown,
            )
        } else {
            (SubmissionVerification::Verified, WakeOutcome::Submitted)
        };
        *self.2.lock().unwrap() = Some(verification);
        Ok(outcome)
    }
    fn take_verification(&self, _: &SeatId) -> Option<SubmissionVerification> {
        self.2.lock().unwrap().take()
    }
}

struct UnverifiedNotifier(Arc<Mutex<Vec<String>>>);
impl NotificationPort for UnverifiedNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.0.lock().unwrap().push("typed".into());
        Ok(WakeOutcome::OutcomeUnknown)
    }
}

/// `contact`: `None` unknown, else whether the seat was heard from since the asked instant.
struct Contacts {
    contact: Mutex<Option<bool>>,
    asked: Mutex<Vec<UtcMillis>>,
}
impl HookPullOutlook for Contacts {
    fn contacted_since(&self, _: &SeatId, since: UtcMillis) -> Option<bool> {
        self.asked.lock().unwrap().push(since);
        *self.contact.lock().unwrap()
    }
}

fn budget(store: &LadderStore) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(store.clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    }
}

fn drain(events: &Mutex<Vec<String>>) -> Vec<String> {
    std::mem::take(&mut *events.lock().unwrap())
}

fn run(contact: Option<bool>, test: impl FnOnce(&dyn Fn() -> u16, &LadderStore, &Contacts)) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let notifier = SubmittingNotifier(events.clone(), false, Mutex::new(None));
    run_with(events, &notifier, contact, test);
}

fn run_with(
    events: Arc<Mutex<Vec<String>>>,
    notifier: &dyn NotificationPort,
    contact: Option<bool>,
    test: impl FnOnce(&dyn Fn() -> u16, &LadderStore, &Contacts),
) {
    let store = LadderStore::new(events);
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let contacts = Contacts {
        contact: Mutex::new(contact),
        asked: Mutex::new(Vec::new()),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_hook_pull(&contacts);
    let drive = || scheduler.drive_wakes(&budget(&store)).unwrap().attempted;
    test(&drive, &store, &contacts);
}

// Kills: retyping a wake the session never received at every ladder step, a verdict drawn
// before the retry is due, a mark repeated per pass, or a silent seat kept parked after contact.
#[test]
fn a_silent_seat_is_marked_unreceived_once_and_not_retyped_until_it_is_heard_from() {
    run(Some(false), |drive, store, contacts| {
        assert_eq!(drive(), 1);
        assert_eq!(drain(&store.events), ["reserve attempt-1", "typed"]);
        // Inside the ladder spacing nothing is judged.
        store.advance(10_000);
        assert_eq!(drive(), 0);
        assert!(drain(&store.events).is_empty());
        assert!(contacts.asked.lock().unwrap().is_empty());

        store.advance(STEPS[0]);
        assert_eq!(drive(), 0);
        assert_eq!(drain(&store.events), ["unreceived seat attempt-1"]);
        assert_eq!(*contacts.asked.lock().unwrap(), [UtcMillis(100)]);
        for _ in 0..3 {
            store.advance(STEPS[3]);
            assert_eq!(drive(), 0);
        }
        assert!(
            drain(&store.events).is_empty(),
            "marked once, never retyped"
        );

        // The session is heard from: the wake work resumes on its ladder.
        *contacts.contact.lock().unwrap() = Some(true);
        assert_eq!(drive(), 1);
        assert_eq!(drain(&store.events), ["reserve attempt-2", "typed"]);
    });
}

// Kills: parking a seat whose session answered (the ladder keeps its old cadence), or one
// whose contact is unknown (a process without the registry keeps today's behaviour).
#[test]
fn a_seat_heard_from_or_of_unknown_contact_keeps_its_retry_ladder() {
    for contact in [Some(true), None] {
        run(contact, |drive, store, _| {
            assert_eq!(drive(), 1);
            store.advance(STEPS[0] + 1);
            assert_eq!(drive(), 1, "{contact:?}");
            assert_eq!(
                drain(&store.events),
                ["reserve attempt-1", "typed", "reserve attempt-2", "typed"]
            );
        });
    }
}

// Kills: keeping a new session (a changed target set) parked on its predecessor's silence.
#[test]
fn a_changed_target_set_gets_a_fresh_wake() {
    run(Some(false), |drive, store, _| {
        assert_eq!(drive(), 1);
        store.advance(STEPS[0] + 1);
        assert_eq!(drive(), 0);
        assert_eq!(
            drain(&store.events),
            ["reserve attempt-1", "typed", "unreceived seat attempt-1"]
        );
        *store.targets.lock().unwrap() = vec!["binding_b"];
        store.advance(STEPS[1]);
        assert_eq!(drive(), 1);
        assert_eq!(drain(&store.events), ["reserve attempt-2", "typed"]);
    });
}

// Kills: withholding the wake of a newer item (a new invitation or ACK request, p0) because an
// older wake went unreceived (delivery contract rule 5).
#[test]
fn a_newer_item_gets_its_own_wake_after_an_unreceived_one() {
    run(Some(false), |drive, store, _| {
        assert_eq!(drive(), 1);
        store.advance(STEPS[0] + 1);
        assert_eq!(drive(), 0);
        assert_eq!(
            drain(&store.events),
            ["reserve attempt-1", "typed", "unreceived seat attempt-1"]
        );
        store.newest.store(2, Ordering::SeqCst);
        store.advance(STEPS[1]);
        assert_eq!(drive(), 1);
        assert_eq!(drain(&store.events), ["reserve attempt-2", "typed"]);
    });
}

// Kills: a wake typed but seen held in the composer (then cleared on agy) being retyped at every
// ladder step, and an OutcomeUnknown nobody saw typed ever being withheld.
#[test]
fn a_held_wake_awaits_its_receipt_and_an_unverified_unknown_does_not() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let held = SubmittingNotifier(events.clone(), true, Mutex::new(None));
    run_with(events, &held, Some(false), |drive, store, _| {
        assert_eq!(drive(), 1);
        store.advance(STEPS[0] + 1);
        assert_eq!(drive(), 0);
        assert_eq!(
            drain(&store.events),
            ["reserve attempt-1", "typed", "unreceived seat attempt-1"]
        );
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let unknown = UnverifiedNotifier(events.clone());
    run_with(events, &unknown, Some(false), |drive, store, _| {
        assert_eq!(drive(), 1);
        store.advance(STEPS[0] + 1);
        assert_eq!(drive(), 1);
        assert_eq!(
            drain(&store.events),
            ["reserve attempt-1", "typed", "reserve attempt-2", "typed"]
        );
    });
}
