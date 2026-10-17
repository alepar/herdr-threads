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
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct FakeModChannels {
    /// `status()` lists these seats, whatever their binding generation.
    any: Mutex<HashMap<SeatId, u64>>,
    stalled: Mutex<HashSet<SeatId>>,
}
impl FakeModChannels {
    fn go_live(&self, seat: &str, generation: u64) {
        self.any
            .lock()
            .unwrap()
            .insert(SeatId::new(seat), generation);
    }
    fn remove(&self, seat: &str) {
        self.any.lock().unwrap().remove(&SeatId::new(seat));
        self.stalled.lock().unwrap().remove(&SeatId::new(seat));
    }
    fn stall(&self, seat: &str) {
        self.stalled.lock().unwrap().insert(SeatId::new(seat));
    }
}
impl ModChannels for FakeModChannels {
    fn stalled(&self, seat: &SeatId, _: UtcMillis) -> bool {
        self.stalled.lock().unwrap().contains(seat)
    }
    fn status(&self) -> Option<ModChannelStatus> {
        let channels: Vec<ModChannelEntry> = self
            .any
            .lock()
            .unwrap()
            .iter()
            .map(|(seat, generation)| ModChannelEntry {
                seat: seat.clone(),
                harness: "claude".into(),
                binding_generation: *generation,
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

// Kills: gating on the candidate's binding generation, so the /clear window
// (the check-in committed generation 1 -> the candidate; the registry still
// holds the previous channel at another generation until the worker pass)
// lets a native prompt through (F1).
#[test]
fn wake_suppressed_when_the_seat_has_a_channel_of_another_generation() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
    channels.go_live("seat", 2);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    assert_eq!(
        runner.try_candidate(&due_candidate(), &budget()).unwrap(),
        None
    );
    assert!(events.lock().unwrap().is_empty());
}

// Kills: gating on any channel at all, so another seat's channel would
// suppress this seat's native wake.
#[test]
fn wake_reserved_when_only_another_seat_has_a_channel() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = store(&events);
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let channels = FakeModChannels::default();
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
    /// The due poke's targets (empty: the source carries none).
    targets: Vec<crate::ports::WakeTarget>,
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
            targets: self.targets.clone(),
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
        targets: Vec::new(),
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
        targets: Vec::new(),
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

fn target(id: &str, pid: u32) -> crate::ports::WakeTarget {
    crate::ports::WakeTarget {
        provenance: String::new(),
        binding: crate::protocol::ids::BindingId::new(id),
        harness: "claude".into(),
        fingerprint_pid: Some(pid),
    }
}

/// Mod channels keyed by binding (what ht-54r.12's registry answers): only the listed bindings
/// have a live channel; `seat_live` is deliberately false so only the per-binding answer counts.
#[derive(Default)]
struct BindingChannels {
    live: Mutex<HashSet<crate::protocol::ids::BindingId>>,
    stalled: Mutex<bool>,
}
impl BindingChannels {
    fn live(ids: &[&str]) -> Self {
        let channels = Self::default();
        for id in ids {
            channels
                .live
                .lock()
                .unwrap()
                .insert(crate::protocol::ids::BindingId::new(*id));
        }
        channels
    }
}
impl ModChannels for BindingChannels {
    fn binding_live(&self, _: &SeatId, binding: &crate::protocol::ids::BindingId) -> bool {
        self.live.lock().unwrap().contains(binding)
    }
    fn stalled(&self, _: &SeatId, _: UtcMillis) -> bool {
        *self.stalled.lock().unwrap()
    }
}

/// A wake store whose reservations are cooperative and carry the candidate's targets, like the
/// SQLite store's.
struct CooperativeStore {
    inner: FakeWakeStore,
    /// Set when a reservation commits (a channel "registering" between pre-check and attempt).
    on_reserve: Option<Arc<AtomicBool>>,
}
fn cooperative(inner: FakeWakeStore) -> CooperativeStore {
    CooperativeStore {
        inner,
        on_reserve: None,
    }
}
impl WakePort for CooperativeStore {
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
        if let Some(flag) = &self.on_reserve {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(self.inner.reserve_wake(candidate, budget)?.map(|mut r| {
            r.authority = ReservedWakeAuthority::Cooperative {
                targets: candidate.targets.clone(),
                terminal: crate::protocol::ids::TerminalId::new("terminal"),
                incarnation: "incarnation".into(),
                binding_generation: None,
                harness: Some("claude".into()),
            };
            r
        }))
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
}

/// Records the targets of each reservation the host is asked to type to.
#[derive(Default)]
struct TargetNotifier {
    typed: Mutex<Vec<Vec<crate::protocol::ids::BindingId>>>,
}
impl NotificationPort for TargetNotifier {
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        _: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError> {
        self.typed.lock().unwrap().push(
            reservation
                .authority
                .targets()
                .iter()
                .map(|t| t.binding.clone())
                .collect(),
        );
        Ok(WakeOutcome::Submitted)
    }
}

fn candidate_with(targets: Vec<crate::ports::WakeTarget>) -> WakeCandidate {
    WakeCandidate {
        targets,
        ..due_candidate()
    }
}

// Kills: gating on the seat instead of the bindings, so a binding without a channel is never
// woken because its sibling has one; and typing to the binding its channel already woke.
#[test]
fn binding_without_a_channel_is_typed_to_and_one_with_a_channel_is_not() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = cooperative(store(&events));
    let notifier = TargetNotifier::default();
    let channels = BindingChannels::live(&["a"]);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    let candidate = candidate_with(vec![target("a", 10), target("b", 20)]);
    assert_eq!(
        runner.try_candidate(&candidate, &budget()).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(
        *notifier.typed.lock().unwrap(),
        vec![vec![crate::protocol::ids::BindingId::new("b")]],
        "only the channel-less binding is left for the composer"
    );
}

// Kills: both connected Claude bindings being woken through their channels and the composer
// still typing to one of them.
#[test]
fn two_connected_claude_bindings_both_served_by_channels_are_not_typed_to() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = cooperative(store(&events));
    let notifier = TargetNotifier::default();
    let channels = BindingChannels::live(&["a", "b"]);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    let candidate = candidate_with(vec![target("a", 10), target("b", 20)]);
    assert_eq!(runner.try_candidate(&candidate, &budget()).unwrap(), None);
    assert!(events.lock().unwrap().is_empty(), "nothing reserved");
    assert!(notifier.typed.lock().unwrap().is_empty());
}

// Kills: a stalled channel still counting as a wake of its binding.
#[test]
fn stalled_channel_binding_falls_back_to_the_composer() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let store = cooperative(store(&events));
    let notifier = TargetNotifier::default();
    let channels = BindingChannels::live(&["a", "b"]);
    *channels.stalled.lock().unwrap() = true;
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    let candidate = candidate_with(vec![target("a", 10), target("b", 20)]);
    assert_eq!(
        runner.try_candidate(&candidate, &budget()).unwrap(),
        Some(WakeOutcome::Submitted)
    );
    assert_eq!(notifier.typed.lock().unwrap()[0].len(), 2);
}

// Kills: a candidate whose targets are all covered by the time the attempt runs (a channel
// registered between the pre-check and the reservation) still being typed to.
#[test]
fn channel_registered_after_the_precheck_leaves_nothing_to_type() {
    struct LateChannels(BindingChannels, Arc<AtomicBool>);
    impl ModChannels for LateChannels {
        fn binding_live(&self, seat: &SeatId, binding: &crate::protocol::ids::BindingId) -> bool {
            self.1.load(Ordering::SeqCst) && self.0.binding_live(seat, binding)
        }
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let armed = Arc::new(AtomicBool::new(false));
    let store = CooperativeStore {
        inner: store(&events),
        on_reserve: Some(armed.clone()),
    };
    let notifier = TargetNotifier::default();
    let channels = LateChannels(BindingChannels::live(&["a", "b"]), armed);
    let mut runner = WakeRunner::new(&store, &notifier, RetryConfig::default(), daemon_boot());
    runner.mod_channels = &channels;
    let candidate = candidate_with(vec![target("a", 10), target("b", 20)]);
    assert_eq!(
        runner.try_candidate(&candidate, &budget()).unwrap(),
        Some(WakeOutcome::Refused(RefusalCause::Unsafe)),
        "a pre-send refusal: the ladder does not climb"
    );
    assert!(notifier.typed.lock().unwrap().is_empty());
}

// Kills: a poke filtered by the seat's channels instead of its target bindings'.
#[test]
fn poke_follows_the_same_per_binding_rule() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let wake = PokeStore {
        inner: store(&events),
        reserves: Default::default(),
        targets: vec![target("a", 10), target("b", 20)],
    };
    let notifier = FakeNotifier {
        events: events.clone(),
    };
    let mut runner = WakeRunner::new(&wake, &notifier, RetryConfig::default(), daemon_boot());
    let due = wake.poke_candidates(1, &budget()).unwrap().remove(0);
    let one = BindingChannels::live(&["a"]);
    runner.mod_channels = &one;
    runner.try_poke(&due, &budget()).unwrap();
    assert_eq!(
        wake.reserves.load(Ordering::SeqCst),
        1,
        "binding b has no channel"
    );
    let both = BindingChannels::live(&["a", "b"]);
    runner.mod_channels = &both;
    assert_eq!(runner.try_poke(&due, &budget()).unwrap(), None);
    assert_eq!(wake.reserves.load(Ordering::SeqCst), 1, "all covered");
}
