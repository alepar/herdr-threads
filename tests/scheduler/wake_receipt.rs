//! Wake receipt (ht-wake-loop): a submitted wake is received when the session makes any daemon
//! call claiming the seat before the ladder's next retry. Without one the wake is recorded
//! `unreceived` once and not retyped until the seat is heard from again or its targets change.
use super::*;
use crate::{
    ports::{HookPullOutlook, WakeTarget},
    protocol::ids::{BindingId, NativeSessionId},
    scheduler::SubmissionVerification,
};

/// The reservation instant the ladder store reports (`reserved_at_utc`), whatever the fake clock says.
const RESERVED_AT_UTC: i64 = 100;
const STEPS: [u64; 4] = [30_000, 60_000, 120_000, 300_000];

/// Keeps the durable ladder columns a real store would return after each reservation.
struct LadderStore {
    clock: Arc<FakeClock>,
    events: Arc<Mutex<Vec<String>>>,
    reserved: AtomicU64,
    targets: Mutex<Vec<&'static str>>,
    /// The invitation frontier's decision sequence: a larger one is a newer item.
    newest: AtomicU64,
    /// The seat's `programmatic_wake` as the candidate's attention witness reports it.
    programmatic: AtomicBool,
    /// A warning-only liveness fixture; ordinary tests keep their invitation source.
    warning_only: bool,
}
impl LadderStore {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            clock: Arc::new(FakeClock(AtomicU64::new(1_000))),
            events,
            reserved: AtomicU64::new(0),
            targets: Mutex::new(vec!["binding_a"]),
            newest: AtomicU64::new(1),
            programmatic: AtomicBool::new(false),
            warning_only: false,
        }
    }
    fn advance(&self, millis: u64) {
        self.clock.0.fetch_add(millis, Ordering::SeqCst);
    }
    fn candidate(&self) -> WakeCandidate {
        let reserved = self.reserved.load(Ordering::SeqCst);
        let mut candidate = due_candidate();
        candidate.attention_witness = Some(
            WakeAttentionWitness::from_complete(
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
            )
            .with_programmatic_wake(self.programmatic.load(Ordering::SeqCst)),
        );
        if self.warning_only {
            candidate.has_pending_invitation = false;
            candidate.actionable_warning_generation = Some(9);
            candidate.actionable_warning_seq = Some(9);
            candidate.attention_witness = Some(WakeAttentionWitness::from_complete(
                "34750f98bd59fcfc946da45aaabe933b".into(),
                SeatId::new("seat"),
                9,
                false,
                false,
                Some(9),
                0,
                false,
                LogicalAttentionFrontier {
                    actionable_warning: Some(LogicalPublicationKey {
                        decision_seq: 9,
                        event_offset: 0,
                    }),
                    ..Default::default()
                },
            ));
        }
        candidate.targets = self
            .targets
            .lock()
            .unwrap()
            .iter()
            .map(|binding| WakeTarget {
                provenance: String::new(),
                binding: BindingId::new(*binding),
                harness: if self.warning_only { "pi" } else { "codex" }.into(),
                fingerprint_pid: None,
                dormant: false,
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
            reasons: vec![
                if self.warning_only {
                    "actionable_warning"
                } else {
                    "invitation"
                }
                .into(),
            ],
            retained_effective_delay_ms: 30_000,
            lease_until: MonoInstant(self.clock.monotonic_now().0 + 5_000),
            retained_minimum_delay_ms: 30_000,
            reserved_at_utc: if self.warning_only {
                self.clock.utc_now()
            } else {
                UtcMillis(i64::try_from(n).unwrap() * 100)
            },
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

// Kills: a `programmatic_wake` flip that leaves the seat parked on its unreceived native wake
// (ht-z41e.4): the flip changes how the seat may be woken, so the next due attempt is admitted
// and reaches the transport choice, where the store reads the new flag (selection tests).
#[test]
fn programmatic_wake_flip_lifts_unreceived_and_the_next_attempt_queues() {
    for flip_to in [true, false] {
        run(Some(false), |drive, store, _| {
            store.programmatic.store(!flip_to, Ordering::SeqCst);
            assert_eq!(drive(), 1);
            store.advance(STEPS[0] + 1);
            assert_eq!(drive(), 0);
            assert_eq!(
                drain(&store.events),
                ["reserve attempt-1", "typed", "unreceived seat attempt-1"]
            );
            // Same flag: still parked.
            store.advance(STEPS[1]);
            assert_eq!(drive(), 0);
            assert!(drain(&store.events).is_empty());
            store.programmatic.store(flip_to, Ordering::SeqCst);
            assert_eq!(drive(), 1, "flip to {flip_to}");
            assert_eq!(drain(&store.events), ["reserve attempt-2", "typed"]);
        });
    }
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

/// A notifier reporting `transport`; a `Queue` one hands over `root` as the queued target.
struct QueueNotifier {
    outcome: WakeOutcome,
    transport: crate::ports::WakeTransport,
    root: Option<&'static str>,
    typed: Arc<Mutex<Vec<String>>>,
}
impl QueueNotifier {
    fn queued(outcome: WakeOutcome, root: &'static str, typed: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            outcome,
            transport: crate::ports::WakeTransport::Queue,
            root: Some(root),
            typed,
        }
    }
}
impl NotificationPort for QueueNotifier {
    fn attempt_wake(
        &self,
        _: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.typed.lock().unwrap().push("typed".into());
        Ok(self.outcome)
    }
    fn take_transport(&self, _: &WakeAttemptId) -> crate::ports::WakeTransport {
        self.transport
    }
    fn take_queue_root(&self, _: &WakeAttemptId) -> Option<NativeSessionId> {
        self.root.map(NativeSessionId::new)
    }
}

/// The real per-seat and per-(seat, session) contact book as the scheduler's outlook.
struct Book(crate::service::delivery_pull::DeliveryPullState);
impl HookPullOutlook for Book {
    fn contacted_since(&self, seat: &SeatId, since: UtcMillis) -> Option<bool> {
        self.0.contacted_since(seat, since)
    }
    fn contacted_since_session(
        &self,
        seat: &SeatId,
        session: &NativeSessionId,
        since: UtcMillis,
    ) -> Option<bool> {
        self.0.contacted_since_session(seat, session, since)
    }
}

/// Reserves one wake through `notifier`, advances to the first retry and lets `contact` record a
/// call (seat, session) after the reservation; returns what the retry then did.
fn queued_retry(
    notifier: QueueNotifier,
    contact: &[&'static str],
    test: impl FnOnce(&dyn Fn() -> u16, &LadderStore, &Mutex<Vec<String>>),
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = LadderStore::new(events.clone());
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let book = Book(Default::default());
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_hook_pull(&book);
    let drive = || scheduler.drive_wakes(&budget(&store)).unwrap().attempted;
    assert_eq!(drive(), 1);
    assert_eq!(drain(&events), ["reserve attempt-1"]);
    store.advance(10);
    for session in contact {
        book.0.record_contact(
            &SeatId::new("seat"),
            &NativeSessionId::new(*session),
            UtcMillis(RESERVED_AT_UTC + 50),
        );
    }
    store.advance(STEPS[0]);
    test(&drive, &store, &events);
}

// Kills: a queued wake settled, or its `unreceived` record lifted, by a `/side` session of the
// same seat (race R5: side contact is not root arrival), and a retype loop it would re-arm.
#[test]
fn a_queued_root_wake_with_only_side_contact_is_unreceived_and_not_retyped() {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let notifier = QueueNotifier::queued(WakeOutcome::Submitted, "root", typed.clone());
    queued_retry(notifier, &["side"], |drive, store, events| {
        assert_eq!(drive(), 0, "side contact does not settle the queued wake");
        assert_eq!(drain(events), ["unreceived seat attempt-1"]);
        for _ in 0..3 {
            store.advance(STEPS[3]);
            assert_eq!(drive(), 0);
        }
        assert!(drain(events).is_empty(), "marked once, never retyped");
        assert_eq!(typed.lock().unwrap().len(), 1, "one wake typed in all");
    });
}

// Kills: a queued root wake never awaited (the old behaviour) and one that no contact from its
// own session can settle. Any binding carrying the root's native session UUID counts, so a root
// that re-registered under a new binding id between reservation and contact still settles it.
#[test]
fn root_session_contact_settles_a_queued_wake_whatever_binding_made_it() {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let notifier = QueueNotifier::queued(WakeOutcome::Submitted, "root", typed);
    queued_retry(notifier, &["side", "root"], |drive, _, events| {
        assert_eq!(drive(), 1, "root contact settles; the ladder continues");
        assert_eq!(drain(events), ["reserve attempt-2"]);
    });
}

// Kills: judging a native wake on one session only: any binding of the seat receives it.
#[test]
fn a_native_wake_settles_on_any_binding_contact() {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let notifier = QueueNotifier {
        outcome: WakeOutcome::Submitted,
        transport: crate::ports::WakeTransport::Composer,
        root: None,
        typed,
    };
    queued_retry(notifier, &["side"], |drive, _, events| {
        assert_eq!(drive(), 1);
        assert_eq!(drain(events), ["reserve attempt-2"]);
    });
}

// Kills: a queue-selected attempt that fell back to native (stale authority, or `programmatic_wake`
// flipped off before arm; `programmatic_wake_flipped_off_before_arm_refuses_no_effect_and_falls_back_native`
// pins the dispatch, with no root left) keeping root scope: its receipt is seat-scoped, and a
// silent seat is marked `unreceived`.
#[test]
fn a_queue_selected_attempt_that_fell_back_to_native_is_seat_scoped() {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let notifier = QueueNotifier {
        outcome: WakeOutcome::Submitted,
        transport: crate::ports::WakeTransport::Composer,
        root: None,
        typed: typed.clone(),
    };
    queued_retry(notifier, &[], |drive, _, events| {
        assert_eq!(drive(), 0);
        assert_eq!(drain(events), ["unreceived seat attempt-1"]);
    });
}

/// A queue attempt of `outcome` handing `root` (none: the dispatcher left no root), no contact at all.
fn silent_queue(
    outcome: WakeOutcome,
    root: Option<&'static str>,
    transport: crate::ports::WakeTransport,
) -> (u16, Vec<String>, usize) {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let notifier = QueueNotifier {
        outcome,
        transport,
        root,
        typed: typed.clone(),
    };
    let mut seen = None;
    queued_retry(notifier, &[], |drive, _, events| {
        let attempted = drive();
        seen = Some((attempted, drain(events)));
    });
    let (attempted, events) = seen.unwrap();
    let typed = typed.lock().unwrap().len();
    (attempted, events, typed)
}

// Kills: awaiting a queue attempt whose root is unknown (`|| root_session.is_some()` dropped, or
// every Queue attempt awaited): nothing can judge it, so it would sit `unreceived` forever. The
// control hands a root and is judged.
#[test]
fn a_queue_attempt_with_an_unknown_root_is_not_awaited_and_the_ladder_retries() {
    use crate::ports::WakeTransport::Queue;
    let (attempted, events, typed) = silent_queue(WakeOutcome::Submitted, None, Queue);
    assert_eq!(attempted, 1, "an unjudgeable attempt retries");
    assert_eq!(events, ["reserve attempt-2"]);
    assert_eq!(typed, 2);
    let (attempted, events, typed) = silent_queue(WakeOutcome::Submitted, Some("root"), Queue);
    assert_eq!(
        attempted, 0,
        "control: the same attempt with its root is awaited"
    );
    assert_eq!(events, ["unreceived seat attempt-1"]);
    assert_eq!(typed, 1);
}

// Kills: arming a queued OutcomeUnknown attempt (`armed: true` for it): its landing is unproven,
// so the silence is not judged and the ladder retries. The control is the same root submitted.
#[test]
fn a_queued_outcome_unknown_attempt_with_a_root_is_not_judged_silent() {
    use crate::ports::WakeTransport::Queue;
    let (attempted, events, typed) = silent_queue(WakeOutcome::OutcomeUnknown, Some("root"), Queue);
    assert_eq!(attempted, 1, "not armed: the retry proceeds");
    assert_eq!(events, ["reserve attempt-2"]);
    assert_eq!(typed, 2);
    let (attempted, events, _) = silent_queue(WakeOutcome::Submitted, Some("root"), Queue);
    assert_eq!(
        attempted, 0,
        "control: Submitted with the same root is judged"
    );
    assert_eq!(events, ["unreceived seat attempt-1"]);
}

// Kills: reading the queue root whatever the transport (`take_queue_root` called for a Composer
// attempt): a queue-selected attempt that fell back to native reports Composer, and a notifier
// that still holds a root must not make its receipt root-scoped. Side-only contact settles it; the
// control is the same contact and root on a Queue attempt, which is not settled.
#[test]
fn a_native_fallback_ignores_a_lingering_queue_root_and_stays_seat_scoped() {
    let typed = Arc::new(Mutex::new(Vec::new()));
    let fallback = QueueNotifier {
        outcome: WakeOutcome::Submitted,
        transport: crate::ports::WakeTransport::Composer,
        root: Some("root"),
        typed,
    };
    queued_retry(fallback, &["side"], |drive, _, events| {
        assert_eq!(drive(), 1, "any binding of the seat settles a native wake");
        assert_eq!(drain(events), ["reserve attempt-2"]);
    });
    let typed = Arc::new(Mutex::new(Vec::new()));
    let queued = QueueNotifier::queued(WakeOutcome::Submitted, "root", typed);
    queued_retry(queued, &["side"], |drive, _, events| {
        assert_eq!(drive(), 0, "control: the root's own contact is needed");
        assert_eq!(drain(events), ["unreceived seat attempt-1"]);
    });
}

/// Actual contact tracking with a deliberately empty ordinary pull outlook. This exercises the
/// actionable-warning exception rather than making a fake pending ordinary message force retry.
struct WarningContacts(crate::service::delivery_pull::DeliveryPullState);
impl HookPullOutlook for WarningContacts {
    fn next_pull(
        &self,
        _: &SeatId,
        _: &[BindingId],
        _: u64,
        _: u64,
        _: UtcMillis,
    ) -> crate::ports::NextPull {
        crate::ports::NextPull::Nothing
    }
    fn contacted_since(&self, seat: &SeatId, since: UtcMillis) -> Option<bool> {
        self.0.contacted_since(seat, since)
    }
}

// Causal control: the same publication frontier retries only when the prior wake was heard.
// Canonical old-page confirmation/successor allocation is covered by the paired real-store test;
// this test models their late availability after a completed turn, not native callback timing.
#[test]
fn late_notice_successor_retries_same_warning_frontier_after_contact() {
    for heard in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let notifier = SubmittingNotifier(events.clone(), false, Mutex::new(None));
        let mut store = LadderStore::new(events.clone());
        store.warning_only = true;
        let contacts = WarningContacts(Default::default());
        let due = FakeDeadlinePort {
            clock: store.clock.clone(),
            due_calls: AtomicU64::new(0),
        };
        let channels = native_notice_channel(store.clock.clone());
        use crate::ports::ModChannels;
        assert!(channels.binding_live(&SeatId::new("seat"), &BindingId::new("binding_a")));
        assert!(
            !channels.binding_wake_suppressed(&SeatId::new("seat"), &BindingId::new("binding_a"))
        );
        let scheduler = Scheduler::new(
            "i".into(),
            &due,
            &store,
            &notifier,
            RetryConfig::default(),
            daemon_boot(),
        )
        .with_hook_pull(&contacts)
        .with_mod_channels(&channels);
        let frontier = store.candidate().attention_witness.unwrap().frontier();
        assert!(
            frontier.invitation.is_none()
                && frontier.addressed_receipt.is_none()
                && frontier.lazy.is_none()
        );
        assert_eq!(frontier.actionable_warning.unwrap().decision_seq, 9);
        assert_eq!(scheduler.drive_wakes(&budget(&store)).unwrap().attempted, 1);
        assert_eq!(drain(&events), ["reserve attempt-1", "typed"]);
        // The scheduler attempt has completed. Native after-settle successor timing is
        // independently exercised by the SDK fixture, not fabricated by a boolean here.
        store.advance(1);
        assert_eq!(
            contacts.contacted_since(&SeatId::new("seat"), store.clock.utc_now()),
            Some(false)
        );
        if heard {
            contacts.0.record_contact(
                &SeatId::new("seat"),
                &NativeSessionId::new("session"),
                store.clock.utc_now(),
            );
            assert_eq!(
                contacts.contacted_since(&SeatId::new("seat"), store.clock.utc_now()),
                Some(true)
            );
        }
        store.advance(STEPS[0] - 2);
        assert_eq!(scheduler.drive_wakes(&budget(&store)).unwrap().attempted, 0);
        assert!(drain(&events).is_empty());
        assert_eq!(
            store.candidate().attention_witness.unwrap().frontier(),
            frontier
        );
        store.advance(1);
        assert_eq!(
            scheduler.drive_wakes(&budget(&store)).unwrap().attempted,
            u16::from(heard),
            "same warning requires a second automatic wake after contact"
        );
        assert_eq!(
            store.candidate().attention_witness.unwrap().frontier(),
            frontier
        );
        assert_eq!(
            drain(&events),
            if heard {
                vec!["reserve attempt-2".to_owned(), "typed".to_owned()]
            } else {
                vec!["unreceived seat attempt-1".to_owned()]
            }
        );
    }
}

fn native_notice_channel(
    clock: Arc<dyn Clock>,
) -> crate::service::mod_channels::ModChannelRegistry {
    use crate::ports::{ModChannelSink, ModChannels, ModStoreReads};
    struct NoReads;
    impl ModStoreReads for NoReads {
        fn mod_binding_view(
            &self,
            _: &SeatId,
            _: crate::ports::ModBindingRef<'_>,
            _: &CallBudget,
        ) -> Result<Option<crate::ports::ModSeatView>, ApiError> {
            Ok(None)
        }
        fn mod_stall_oldest(
            &self,
            _: &SeatId,
            _: UtcMillis,
            _: usize,
            _: &CallBudget,
        ) -> Result<Option<UtcMillis>, ApiError> {
            Ok(None)
        }
    }
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: crate::protocol::watch::WatchFrame) -> bool {
            true
        }
    }
    let registry = crate::harness::registry::builtins();
    let id = registry.agent("pi").unwrap();
    let idle_wake = registry
        .by_id(id)
        .unwrap()
        .delivery_policy()
        .unwrap()
        .idle_wake();
    assert_eq!(idle_wake, crate::harness::adapter::IdleWake::Native);
    let registered_at = clock.utc_now();
    let channels = crate::service::mod_channels::ModChannelRegistry::new(
        clock,
        Arc::new(NoReads),
        "i".into(),
        Arc::new(|| {}),
        crate::protocol::watch::ModDeliverySetting::On,
    );
    channels
        .register(
            crate::ports::ModChannelRegistration {
                mode: crate::ports::ModChannelMode::PresentationV2,
                seat: SeatId::new("seat"),
                binding: BindingId::new("binding_a"),
                binding_generation: 1,
                native_session: crate::protocol::ids::NativeSessionId::new("native"),
                harness: Harness::Agent(id),
                idle_wake,
                readonly: false,
                registered_at,
            },
            Arc::new(Sink),
        )
        .unwrap();
    channels
}
