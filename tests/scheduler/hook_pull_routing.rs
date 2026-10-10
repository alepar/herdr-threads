//! Hook-pull wake eligibility: a hook-pull seat is woken only when its next
//! pull brings something (or when it holds ids parked for `inbox`). A fake
//! `HookPullOutlook` stands in for the offered ledger.
use super::*;
use crate::ports::{HookPullOutlook, NextPull};

struct FakeHookPull {
    next: Mutex<NextPull>,
    parked: AtomicBool,
    asked: Mutex<Vec<(String, u64, u64)>>,
}
impl FakeHookPull {
    fn new(next: NextPull, parked: bool) -> Self {
        Self {
            next: Mutex::new(next),
            parked: AtomicBool::new(parked),
            asked: Mutex::new(Vec::new()),
        }
    }
    fn set(&self, next: NextPull) {
        *self.next.lock().unwrap() = next;
    }
}
impl HookPullOutlook for FakeHookPull {
    fn next_pull(&self, seat: &SeatId, generation: u64, attention: u64, _: UtcMillis) -> NextPull {
        self.asked
            .lock()
            .unwrap()
            .push((seat.as_str().to_owned(), generation, attention));
        *self.next.lock().unwrap()
    }
    fn holds_parked(&self, _: &SeatId, _: u64) -> bool {
        self.parked.load(Ordering::SeqCst)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// One pass over a store with a due candidate, through an outlook answering `next`.
fn attempted_with(next: NextPull, parked: bool) -> (u64, Vec<&'static str>) {
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
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let outlook = FakeHookPull::new(next, parked);
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_hook_pull(&outlook);
    let attempted = scheduler.drive_wakes(&budget()).unwrap().attempted;
    let events = events.lock().unwrap().clone();
    (attempted as u64, events)
}

// Kills: a wake reserved (or a ladder step spent) for an id whose re-offer is
// not yet due, which would send the woken turn an empty pull.
#[test]
fn a_reoffer_not_yet_due_defers_the_wake_and_leaves_the_ladder() {
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
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let outlook = FakeHookPull::new(NextPull::ReofferAt(UtcMillis(60_000)), false);
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_hook_pull(&outlook);
    for _ in 0..3 {
        assert_eq!(scheduler.drive_wakes(&budget()).unwrap().attempted, 0);
    }
    assert!(
        events.lock().unwrap().is_empty(),
        "no reserve, host or complete"
    );
    // The ladder is untouched: the very next pass wakes with no spacing to wait out.
    outlook.set(NextPull::Delivers);
    assert_eq!(scheduler.drive_wakes(&budget()).unwrap().attempted, 1);
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
    assert_eq!(
        outlook.asked.lock().unwrap().last().unwrap(),
        &("seat".to_owned(), 1, 1)
    );
}

// Kills: waking a seat whose next pull would show nothing and holds no parked id.
#[test]
fn a_seat_whose_next_pull_shows_nothing_is_not_woken() {
    let (attempted, events) = attempted_with(NextPull::Nothing, false);
    assert_eq!(attempted, 0);
    assert!(events.is_empty());
}

// Kills: dropping the parked exception, which would strand ids waiting for `inbox`.
#[test]
fn a_seat_holding_parked_ids_is_still_woken() {
    let (attempted, _) = attempted_with(NextPull::Nothing, true);
    assert_eq!(attempted, 1);
}

// Kills: gating wakes on anything but a known empty or deferred pull.
#[test]
fn unknown_and_delivers_wake_as_before() {
    for next in [NextPull::Unknown, NextPull::Delivers] {
        assert_eq!(attempted_with(next, false).0, 1, "{next:?}");
    }
}
