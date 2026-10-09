//! Spec D7 wake routing: a live mod channel (grace included) replaces native
//! wakes and pokes for its seat; a stalled channel lets the native ladder run;
//! deadlines are unaffected. A fake `ModChannels` stands in for the registry.
use super::*;
use crate::{
    ports::{ModChannels, PokeDue, PokeReceipt, PokeReservation, PokeSource},
    protocol::{
        ids::{MessageId, ThreadId},
        watch::{ModChannelEntry, ModChannelState, ModChannelStatus, ModDeliverySetting},
    },
};
use std::collections::HashSet;

#[derive(Default)]
struct FakeModChannels {
    /// `is_live(seat, generation)` is true for these pairs.
    live: Mutex<HashSet<(SeatId, u64)>>,
    /// `status()` lists these seats (any generation, as the poke path asks).
    any: Mutex<HashSet<SeatId>>,
    stalled: Mutex<HashSet<SeatId>>,
}
impl FakeModChannels {
    fn go_live(&self, seat: &str, generation: u64) {
        self.live
            .lock()
            .unwrap()
            .insert((SeatId::new(seat), generation));
        self.any.lock().unwrap().insert(SeatId::new(seat));
    }
    fn remove(&self, seat: &str) {
        self.live.lock().unwrap().clear();
        self.any.lock().unwrap().remove(&SeatId::new(seat));
        self.stalled.lock().unwrap().remove(&SeatId::new(seat));
    }
    fn stall(&self, seat: &str) {
        self.stalled.lock().unwrap().insert(SeatId::new(seat));
    }
}
impl ModChannels for FakeModChannels {
    fn is_live(&self, seat: &SeatId, generation: u64) -> bool {
        self.live
            .lock()
            .unwrap()
            .contains(&(seat.clone(), generation))
    }
    fn stalled(&self, seat: &SeatId, _: UtcMillis) -> bool {
        self.stalled.lock().unwrap().contains(seat)
    }
    fn status(&self) -> Option<ModChannelStatus> {
        let channels: Vec<ModChannelEntry> = self
            .any
            .lock()
            .unwrap()
            .iter()
            .map(|seat| ModChannelEntry {
                seat: seat.clone(),
                harness: "claude".into(),
                binding_generation: 1,
                connected_since: UtcMillis(0),
                state: ModChannelState::Live,
            })
            .collect();
        Some(ModChannelStatus {
            mod_delivery: ModDeliverySetting::On,
            live_channels: channels.len() as u32,
            channels,
        })
    }
}

fn store(events: &Arc<Mutex<Vec<&'static str>>>) -> FakeWakeStore {
    FakeWakeStore {
        clock: Arc::new(FakeClock(AtomicU64::new(0))),
        events: events.clone(),
        batch: None,
        fail_reservation: AtomicBool::new(false),
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

// Kills: a wake reserved (or a refusal/backoff recorded) for a seat whose
// generation-1 channel is live.
#[test]
fn wake_not_reserved_while_channel_live_and_ladder_unchanged() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_mod_channels(&channels);
    for _ in 0..3 {
        assert_eq!(scheduler.drive_wakes(&budget()).unwrap().attempted, 0);
    }
    assert!(
        events.lock().unwrap().is_empty(),
        "no reserve, host or complete"
    );
    // The ladder is untouched: once the channel is gone the very next pass
    // wakes the seat with no retry spacing to wait out.
    channels.remove("seat");
    assert_eq!(scheduler.drive_wakes(&budget()).unwrap().attempted, 1);
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
}

// Kills: gating on the seat alone, so a channel for a stale generation (or
// another seat) would suppress this seat's native wake.
#[test]
fn wake_reserved_when_channel_live_for_another_generation_only() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels
        .live
        .lock()
        .unwrap()
        .insert((SeatId::new("seat"), 2));
    channels.go_live("elsewhere", 1);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget()).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
}

// Kills: a stalled channel still suppressing the native ladder.
#[test]
fn native_ladder_runs_while_channel_stalled() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    channels.stall("seat");
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget()).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
}

// Kills: resuming only on the next process start; the same runner must wake
// the seat as soon as the channel is removed.
#[test]
fn wake_resumes_after_channel_removed() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget()).unwrap(),
        None
    );
    assert!(events.lock().unwrap().is_empty());
    channels.remove("seat");
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget()).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(*events.lock().unwrap(), vec!["reserve", "host", "complete"]);
}

/// A wake store that serves one due poke and counts `reserve_poke` calls.
struct PokeStore {
    inner: FakeWakeStore,
    reserves: std::sync::atomic::AtomicUsize,
}
impl WakePort for PokeStore {
    fn clock(&self) -> &dyn Clock {
        self.inner.clock()
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
        restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.inner.complete_wake(attempt, outcome, restore, budget)
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
    fn poke_candidates(&self, _: u16, _: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        Ok(vec![PokeDue {
            seat: SeatId::new("seat"),
            receipts: vec![PokeReceipt {
                message: MessageId::parse("m1").unwrap(),
                seat: SeatId::new("seat"),
                thread: ThreadId::new("t1"),
                source: PokeSource::Receipts,
                effective_deadline: 100_000,
            }],
        }])
    }
    fn reserve_poke(
        &self,
        _: &PokeDue,
        _: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        self.reserves.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

// Kills: a poke reserved while the channel is live; and a stalled channel
// that still blocks the poke.
#[test]
fn poke_skipped_while_live_and_runs_while_stalled() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let wake = PokeStore {
        inner: store(&events),
        reserves: Default::default(),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let due = FakeDeadlinePort {
        clock: wake.inner.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &wake,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_mod_channels(&channels);
    let outcome = scheduler.drive_pokes(&budget()).unwrap();
    assert_eq!(outcome.examined, 0, "filtered before the per-tick cap");
    assert_eq!(wake.reserves.load(Ordering::SeqCst), 0);
    channels.stall("seat");
    let outcome = scheduler.drive_pokes(&budget()).unwrap();
    assert_eq!(outcome.examined, 1);
    assert_eq!(wake.reserves.load(Ordering::SeqCst), 1);
}

// Kills: `try_poke` skipping its own re-check, so a seat that went live
// between the filter and the reservation is still poked.
#[test]
fn try_poke_rechecks_the_channel_before_reserving() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let wake = PokeStore {
        inner: store(&events),
        reserves: Default::default(),
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    let mut runner = WakeRunner::new(&wake, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    let due = wake.poke_candidates(1, &budget()).unwrap().remove(0);
    assert_eq!(runner.try_poke(&due, &budget()).unwrap(), None);
    assert_eq!(wake.reserves.load(Ordering::SeqCst), 0);
    channels.remove("seat");
    runner.try_poke(&due, &budget()).unwrap();
    assert_eq!(wake.reserves.load(Ordering::SeqCst), 1);
}

// Kills: the deadline lane being routed through the mod gate.
#[test]
fn deadlines_still_scan_while_live() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 1);
    let due = FakeDeadlinePort {
        clock: store.clock.clone(),
        due_calls: AtomicU64::new(0),
    };
    let scheduler = Scheduler::new(
        "i".into(),
        &due,
        &store,
        &notifier,
        RetryConfig::default(),
        daemon_boot(),
    )
    .with_mod_channels(&channels);
    scheduler.drive_deadlines(&budget()).unwrap();
    assert_eq!(due.due_calls.load(Ordering::SeqCst), 1);
}
