use super::*;
use crate::{
    ports::{ModBindingView, ModSeatView},
    protocol::{
        ids::NativeSessionId,
        results::ErrorCode,
        time::MonoInstant,
        watch::{MOD_STALL_AFTER_MS, WATCH_BODY_LIMIT_BYTES},
    },
};
use std::sync::atomic::{AtomicI64, AtomicUsize};

struct FakeClock(AtomicI64);
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}

#[derive(Default)]
struct RecordingSink {
    frames: Mutex<Vec<WatchFrame>>,
    gone: AtomicBool,
}
impl ModChannelSink for RecordingSink {
    fn push(&self, frame: WatchFrame) -> bool {
        if self.gone.load(Ordering::SeqCst) {
            return false;
        }
        self.frames.lock().unwrap().push(frame);
        true
    }
}
impl RecordingSink {
    fn frames(&self) -> Vec<WatchFrame> {
        self.frames.lock().unwrap().clone()
    }
}

/// Serves scripted views and one scripted stall answer; records the arguments
/// of the stall read.
#[derive(Default)]
struct FakeStore {
    views: Mutex<HashMap<SeatId, Option<ModSeatView>>>,
    /// `(published_at, body_len)` of the seat's pending ordinary receipts.
    receipts: Mutex<Vec<(UtcMillis, usize)>>,
    stall_calls: Mutex<Vec<(UtcMillis, usize)>>,
    fail_views: AtomicBool,
    /// Runs once inside the next store read, before it returns: a commit or
    /// registration landing between a judgement's snapshot and its close.
    on_read: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl FakeStore {
    fn run_hook(&self) {
        let hook = self.on_read.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
}
impl ModStoreReads for FakeStore {
    fn mod_seat_view(
        &self,
        seat: &SeatId,
        _budget: &CallBudget,
    ) -> Result<Option<ModSeatView>, ApiError> {
        if self.fail_views.load(Ordering::SeqCst) {
            return Err(ApiError::new(ErrorCode::StoreBusy, "busy".to_owned()));
        }
        let view = self.views.lock().unwrap().get(seat).cloned().flatten();
        self.run_hook();
        Ok(view)
    }
    fn mod_stall_oldest(
        &self,
        _seat: &SeatId,
        at_or_before: UtcMillis,
        body_limit: usize,
        _budget: &CallBudget,
    ) -> Result<Option<UtcMillis>, ApiError> {
        self.stall_calls
            .lock()
            .unwrap()
            .push((at_or_before, body_limit));
        let oldest = self
            .receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|(at, len)| *at <= at_or_before && *len <= body_limit)
            .map(|(at, _)| *at)
            .min();
        self.run_hook();
        Ok(oldest)
    }
}

struct Fixture {
    clock: Arc<FakeClock>,
    store: Arc<FakeStore>,
    kicks: Arc<AtomicUsize>,
    registry: Arc<ModChannelRegistry>,
    logs: Arc<Mutex<Vec<String>>>,
}
fn fixture_with(setting: ModDeliverySetting) -> Fixture {
    let clock = Arc::new(FakeClock(AtomicI64::new(1_000_000)));
    let store = Arc::new(FakeStore::default());
    let kicks = Arc::new(AtomicUsize::new(0));
    let logs = Arc::new(Mutex::new(Vec::new()));
    let kick_count = Arc::clone(&kicks);
    let log_lines = Arc::clone(&logs);
    let registry = Arc::new(
        ModChannelRegistry::new(
            clock.clone(),
            store.clone(),
            "instance".into(),
            Arc::new(move || {
                kick_count.fetch_add(1, Ordering::SeqCst);
            }),
            setting,
        )
        .with_log(Arc::new(move |line: &str| {
            log_lines.lock().unwrap().push(line.to_owned())
        })),
    );
    Fixture {
        clock,
        store,
        kicks,
        registry,
        logs,
    }
}
fn fixture() -> Fixture {
    fixture_with(ModDeliverySetting::On)
}
impl Fixture {
    fn now(&self) -> UtcMillis {
        self.clock.utc_now()
    }
    fn advance(&self, millis: i64) {
        self.clock.0.fetch_add(millis, Ordering::SeqCst);
    }
    fn kicks(&self) -> usize {
        self.kicks.load(Ordering::SeqCst)
    }
    fn register(&self, seat: &str, generation: u64) -> (ModChannelId, Arc<RecordingSink>) {
        self.try_register(seat, generation).unwrap()
    }
    fn try_register(
        &self,
        seat: &str,
        generation: u64,
    ) -> Result<(ModChannelId, Arc<RecordingSink>), WatchRefusalReason> {
        let sink = Arc::new(RecordingSink::default());
        let id = self.registry.register(
            ModChannelRegistration {
                seat: SeatId::new(seat),
                binding_generation: generation,
                native_session: NativeSessionId::new("native"),
                harness: Harness::Claude,
                registered_at: self.now(),
            },
            sink.clone(),
        )?;
        Ok((id, sink))
    }
    /// Installs the hook that registers `seat` at `generation` during the
    /// next store read (the sink it uses is returned).
    fn register_during_next_read(&self, seat_name: &str, generation: u64) -> Arc<RecordingSink> {
        let sink = Arc::new(RecordingSink::default());
        let registry = Arc::clone(&self.registry);
        let registration = ModChannelRegistration {
            seat: seat(seat_name),
            binding_generation: generation,
            native_session: NativeSessionId::new("native"),
            harness: Harness::Claude,
            registered_at: self.now(),
        };
        let hook_sink = sink.clone();
        *self.store.on_read.lock().unwrap() = Some(Box::new(move || {
            registry.register(registration, hook_sink).unwrap();
        }));
        sink
    }
    fn set_view(&self, seat: &str, view: Option<ModSeatView>) {
        self.store
            .views
            .lock()
            .unwrap()
            .insert(SeatId::new(seat), view);
    }
}

fn seat(name: &str) -> SeatId {
    SeatId::new(name)
}

fn view(seat_name: &str, generation: u64, fingerprint: ModFingerprint) -> ModSeatView {
    ModSeatView {
        seat: seat(seat_name),
        state: "resolved".into(),
        retired: false,
        continuity_resolved: true,
        held: false,
        binding: Some(ModBindingView {
            generation,
            provenance: "cooperative_top_level".into(),
            harness: "claude".into(),
            native_session: "native".into(),
        }),
        attention_version: fingerprint.attention_version,
        fingerprint: ModFingerprint {
            binding_generation: Some(generation),
            ..fingerprint
        },
    }
}
fn pending(version: u64) -> ModFingerprint {
    ModFingerprint {
        attention_version: version,
        pending_receipts: 1,
        max_pending_ordinal: version as i64,
        ..ModFingerprint::default()
    }
}

#[test]
fn register_then_is_live_for_its_generation_only() {
    let f = fixture();
    assert!(!f.registry.is_live(&seat("a"), 1));
    f.register("a", 1);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert!(!f.registry.is_live(&seat("a"), 2));
    assert!(!f.registry.is_live(&seat("b"), 1));
    assert_eq!(f.kicks(), 0);
    let logs = f.logs.lock().unwrap().clone();
    assert_eq!(
        logs,
        vec!["mod channel registered seat=a generation=1 provenance=cooperative_mod_channel"]
    );
}

#[test]
fn second_registration_same_seat_replaces_old_with_close_replaced_and_no_kick() {
    let f = fixture();
    let (first, old_sink) = f.register("a", 1);
    let (second, new_sink) = f.register("a", 1);
    assert_ne!(first, second);
    assert_eq!(
        old_sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Replaced
        }]
    );
    assert!(new_sink.frames().is_empty());
    // The old connection ending afterwards must not disturb the new channel.
    f.registry.unregister(first, f.now());
    assert!(f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.registry.status().unwrap().live_channels, 1);
    assert_eq!(f.kicks(), 0);
}

#[test]
fn register_refuses_disabled_when_setting_off() {
    let f = fixture_with(ModDeliverySetting::Off);
    assert_eq!(
        f.try_register("a", 1).err(),
        Some(WatchRefusalReason::Disabled)
    );
    assert!(!f.registry.is_live(&seat("a"), 1));
}

#[test]
fn set_mod_delivery_off_closes_every_channel_with_disabled() {
    let f = fixture();
    let (_, a) = f.register("a", 1);
    let (_, b) = f.register("b", 4);
    f.registry.set_mod_delivery(ModDeliverySetting::Off);
    for sink in [&a, &b] {
        assert_eq!(
            sink.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::Disabled
            }]
        );
    }
    assert!(!f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 2);
    let status = f.registry.status().unwrap();
    assert_eq!(status.mod_delivery, ModDeliverySetting::Off);
    assert!(status.channels.is_empty());
    assert_eq!(
        f.try_register("a", 1).err(),
        Some(WatchRefusalReason::Disabled)
    );
}

#[test]
fn unregister_keeps_entry_live_in_reconnect_grace_then_expiry_removes_and_kicks() {
    let f = fixture();
    let (id, _) = f.register("a", 1);
    f.registry.unregister(id, f.now());
    f.advance(29_000);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert_eq!(
        f.registry.status().unwrap().channels[0].state,
        ModChannelState::ReconnectGrace
    );
    f.registry.sweep(f.now());
    assert!(f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 0);
    f.advance(2_000);
    // Expired but not yet swept: already not live.
    assert!(!f.registry.is_live(&seat("a"), 1));
    f.registry.sweep(f.now());
    assert!(!f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 1);
    assert!(f.registry.status().unwrap().channels.is_empty());
}

#[test]
fn same_generation_reregistration_within_grace_cancels_kick() {
    let f = fixture();
    let (id, _) = f.register("a", 1);
    f.registry.unregister(id, f.now());
    f.advance(10_000);
    let (_, sink) = f.register("a", 1);
    f.advance(60_000);
    f.registry.sweep(f.now());
    assert_eq!(f.kicks(), 0);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert!(sink.frames().is_empty());
    assert_eq!(
        f.registry.status().unwrap().channels[0].state,
        ModChannelState::Live
    );
}

#[test]
fn close_binding_changed_starts_rebind_grace_live_for_every_generation() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.registry
        .close(&seat("a"), WatchCloseReason::BindingChanged, f.now());
    assert_eq!(
        sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::BindingChanged
        }]
    );
    assert_eq!(f.kicks(), 0);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert!(f.registry.is_live(&seat("a"), 2));
    assert_eq!(
        f.registry.status().unwrap().channels[0].state,
        ModChannelState::RebindGrace
    );
    // A registration for the new generation ends the grace without a kick.
    f.advance(5_000);
    f.register("a", 2);
    f.advance(60_000);
    f.registry.sweep(f.now());
    assert_eq!(f.kicks(), 0);
    assert!(f.registry.is_live(&seat("a"), 2));
    assert!(!f.registry.is_live(&seat("a"), 1));

    // Without one, expiry removes the entry and kicks.
    f.registry
        .close(&seat("a"), WatchCloseReason::BindingChanged, f.now());
    f.advance(29_000);
    f.registry.sweep(f.now());
    assert!(f.registry.is_live(&seat("a"), 9));
    f.advance(2_000);
    f.registry.sweep(f.now());
    assert!(!f.registry.is_live(&seat("a"), 2));
    assert_eq!(f.kicks(), 1);
}

#[test]
fn close_retired_unresolved_disabled_stopping_remove_at_once_and_kick() {
    for reason in [
        WatchCloseReason::Retired,
        WatchCloseReason::Unresolved,
        WatchCloseReason::Disabled,
        WatchCloseReason::Stopping,
    ] {
        let f = fixture();
        let (_, sink) = f.register("a", 1);
        f.registry.close(&seat("a"), reason, f.now());
        assert_eq!(
            sink.frames(),
            vec![WatchFrame::Close { reason }],
            "{reason:?}"
        );
        assert!(!f.registry.is_live(&seat("a"), 1), "{reason:?}");
        assert_eq!(f.kicks(), 1, "{reason:?}");
        assert!(f.registry.status().unwrap().channels.is_empty());
        // Closing a seat with no entry is a no-op: no second kick.
        f.registry.close(&seat("a"), reason, f.now());
        assert_eq!(f.kicks(), 1, "{reason:?}");
    }
}

/// Registers `a`, pushes one Attention at `push_after`, publishes a receipt
/// at `published_after` (relative to registration), then lets `idle` pass.
fn stall_setup(
    ack_age: i64,
    pushed: bool,
    receipt_age: Option<i64>,
    body_len: usize,
) -> (Fixture, UtcMillis) {
    let f = fixture();
    let registered = f.now();
    f.register("a", 1);
    if let Some(age) = receipt_age {
        f.store
            .receipts
            .lock()
            .unwrap()
            .push((UtcMillis(registered.0 - age), body_len));
    }
    if pushed {
        f.registry.record_attention_push(&seat("a"), 1, registered);
    }
    f.advance(ack_age);
    let now = f.now();
    (f, now)
}

#[test]
fn stalled_requires_old_ack_old_receipt_published_before_last_push() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    // All three conditions hold.
    let (f, now) = stall_setup(over, true, Some(0), 10);
    assert!(f.registry.stalled(&seat("a"), now));
    // Boundary: exactly 10 minutes is not yet stalled.
    let (f, now) = stall_setup(MOD_STALL_AFTER_MS as i64, true, Some(0), 10);
    assert!(!f.registry.stalled(&seat("a"), now));
    // Recent ack (registration) -> not stalled.
    let (f, now) = stall_setup(over, true, Some(0), 10);
    f.registry.record_ack(&seat("a"), 1, f.now());
    assert!(!f.registry.stalled(&seat("a"), now));
    // No Attention ever pushed -> not stalled.
    let (f, now) = stall_setup(over, false, Some(0), 10);
    assert!(!f.registry.stalled(&seat("a"), now));
    // No pending receipt published before the push -> not stalled.
    let (f, now) = stall_setup(over, true, None, 10);
    assert!(!f.registry.stalled(&seat("a"), now));
    // The only receipt was published after the last push -> not stalled.
    let (f, now) = stall_setup(over, true, Some(-5_000), 10);
    assert!(!f.registry.stalled(&seat("a"), now));
    // The receipt is recent (published 1 s ago, push after it): old ack alone
    // is not enough.
    let f = fixture();
    f.register("a", 1);
    f.advance(over);
    let now = f.now();
    f.registry
        .record_attention_push(&seat("a"), 1, UtcMillis(now.0 - 500));
    f.store
        .receipts
        .lock()
        .unwrap()
        .push((UtcMillis(now.0 - 1_000), 10));
    assert!(!f.registry.stalled(&seat("a"), now));
    // An unknown seat or a channel in rebind grace is never stalled.
    assert!(!f.registry.stalled(&seat("nobody"), now));
}

#[test]
fn truncated_receipts_never_count_toward_stall() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), WATCH_BODY_LIMIT_BYTES + 1);
    assert!(!f.registry.stalled(&seat("a"), now));
    let calls = f.store.stall_calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].1, WATCH_BODY_LIMIT_BYTES,
        "the registry passes the D4 limit"
    );
    // The same receipt at exactly the limit does count.
    let (f, now) = stall_setup(over, true, Some(0), WATCH_BODY_LIMIT_BYTES);
    assert!(f.registry.stalled(&seat("a"), now));
}

#[test]
fn stall_close_starts_cooldown_refusing_same_generation_for_ten_minutes() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    // A sweep detects the stall and closes with Stalled.
    f.registry.sweep(now);
    assert!(!f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 1);
    // Refused for ten minutes for that generation only.
    f.advance(599_000);
    assert_eq!(
        f.try_register("a", 1).err(),
        Some(WatchRefusalReason::Cooldown)
    );
    f.try_register("a", 2).unwrap();
    f.registry
        .close(&seat("a"), WatchCloseReason::Retired, f.now());
    f.advance(2_000);
    f.try_register("a", 1).unwrap();
}

#[test]
fn stall_close_pushes_close_stalled_to_the_channel() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let f = fixture();
    let registered = f.now();
    let (_, sink) = f.register("a", 1);
    f.store.receipts.lock().unwrap().push((registered, 10));
    f.registry.record_attention_push(&seat("a"), 1, registered);
    f.advance(over);
    f.registry.sweep(f.now());
    assert_eq!(
        sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Stalled
        }]
    );
}

#[test]
fn record_ack_resets_stall_clock() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    assert!(f.registry.stalled(&seat("a"), now));
    f.registry.record_ack(&seat("a"), 1, now);
    assert!(!f.registry.stalled(&seat("a"), now));
    f.advance(MOD_STALL_AFTER_MS as i64 - 1);
    assert!(!f.registry.stalled(&seat("a"), f.now()));
    f.advance(2);
    assert!(f.registry.stalled(&seat("a"), f.now()));
}

#[test]
fn notify_pushes_attention_and_records_push_time() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.set_view("a", Some(view("a", 1, pending(3))));
    // First reading with something pending is the initial sweep.
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 3 }]);
    // A changed fingerprint pushes again with the view's version.
    f.advance(1_000);
    f.set_view("a", Some(view("a", 1, pending(4))));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        sink.frames(),
        vec![
            WatchFrame::Attention { version: 3 },
            WatchFrame::Attention { version: 4 }
        ]
    );
    // The push time feeds the stall predicate: a receipt published before the
    // second push is eligible, one published after it is not.
    f.advance(MOD_STALL_AFTER_MS as i64 + 10);
    // Published before the second push (at 1_001_000) and now over ten
    // minutes old.
    f.store
        .receipts
        .lock()
        .unwrap()
        .push((UtcMillis(1_000_500), 10));
    assert!(f.registry.stalled(&seat("a"), f.now()));
    let calls = f.store.stall_calls.lock().unwrap().clone();
    assert_eq!(calls, vec![(UtcMillis(1_001_000), WATCH_BODY_LIMIT_BYTES)]);
}

#[test]
fn notify_without_fingerprint_change_pushes_nothing() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    // Nothing pending on the first reading: no frame.
    f.set_view("a", Some(view("a", 1, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    assert!(sink.frames().is_empty());
    f.set_view("a", Some(view("a", 1, pending(2))));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1);
    f.registry.pass(f.now()).unwrap();
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1, "an unchanged fingerprint is silent");
    // A settlement that only lowers the count is a change too.
    f.set_view(
        "a",
        Some(view(
            "a",
            1,
            ModFingerprint {
                attention_version: 2,
                ..ModFingerprint::default()
            },
        )),
    );
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 2);
}

#[test]
fn pass_closes_channels_whose_seat_or_binding_no_longer_holds() {
    type Mutate = fn(&mut ModSeatView);
    let cases: [(&str, Mutate, WatchCloseReason, usize); 5] = [
        (
            "retired",
            |v| v.retired = true,
            WatchCloseReason::Retired,
            1,
        ),
        (
            "unresolved",
            |v| v.continuity_resolved = false,
            WatchCloseReason::Unresolved,
            1,
        ),
        (
            "binding ended",
            |v| v.binding = None,
            WatchCloseReason::BindingChanged,
            0,
        ),
        (
            "generation moved",
            |v| v.binding.as_mut().unwrap().generation = 2,
            WatchCloseReason::BindingChanged,
            0,
        ),
        (
            "native session replaced",
            |v| v.binding.as_mut().unwrap().native_session = "other".into(),
            WatchCloseReason::BindingChanged,
            0,
        ),
    ];
    for (name, mutate, reason, kicks) in cases {
        let f = fixture();
        let (_, sink) = f.register("a", 1);
        let mut v = view("a", 1, pending(1));
        mutate(&mut v);
        f.set_view("a", Some(v));
        f.registry.pass(f.now()).unwrap();
        assert_eq!(sink.frames(), vec![WatchFrame::Close { reason }], "{name}");
        assert_eq!(f.kicks(), kicks, "{name}");
    }
    // A seat that vanished from the store is unresolved.
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.set_view("a", None);
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Unresolved
        }]
    );
}

#[test]
fn pass_reports_a_store_error_and_keeps_the_channel() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.store.fail_views.store(true, Ordering::SeqCst);
    let error = f.registry.pass(f.now()).unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert!(sink.frames().is_empty());
}

#[test]
fn registration_over_the_connection_bound_is_busy_but_replacement_is_not() {
    let f = fixture();
    for index in 0..MAX_WATCH_CONNECTIONS {
        f.register(&format!("seat-{index}"), 1);
    }
    assert_eq!(
        f.try_register("one-too-many", 1).err(),
        Some(WatchRefusalReason::Busy)
    );
    // A seat that already has an entry replaces it even at the bound.
    f.try_register("seat-0", 1).unwrap();
    assert_eq!(
        f.registry.status().unwrap().channels.len(),
        MAX_WATCH_CONNECTIONS
    );
}

#[test]
fn status_reports_setting_counts_and_states() {
    let f = fixture();
    let (grace, _) = f.register("b", 2);
    f.register("a", 1);
    f.register("c", 3);
    f.registry
        .close(&seat("c"), WatchCloseReason::BindingChanged, f.now());
    f.registry.unregister(grace, f.now());
    let status = f.registry.status().unwrap();
    assert_eq!(status.mod_delivery, ModDeliverySetting::On);
    assert_eq!(status.live_channels, 1);
    let rows: Vec<(String, u64, ModChannelState)> = status
        .channels
        .iter()
        .map(|c| (c.seat.as_str().to_owned(), c.binding_generation, c.state))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("a".into(), 1, ModChannelState::Live),
            ("b".into(), 2, ModChannelState::ReconnectGrace),
            ("c".into(), 3, ModChannelState::RebindGrace),
        ]
    );
    assert!(status.channels.iter().all(|c| c.harness == "claude"));
    assert_eq!(status.channels[0].connected_since, UtcMillis(1_000_000));
}

#[test]
fn observer_marks_dirty_and_wakes_the_worker() {
    let f = fixture();
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    f.registry.set_worker_wake(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    assert!(!f.registry.take_dirty());
    (f.registry.observer())();
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    assert!(f.registry.take_dirty());
    assert!(!f.registry.take_dirty(), "dirty is consumed once");
    // A registration needs its initial sweep.
    f.register("a", 1);
    assert_eq!(woken.load(Ordering::SeqCst), 2);
    assert!(f.registry.take_dirty());
}

/// The A2 registration decision (`DomainService::watch_register`) against a
/// real temp store and the real registry.
mod domain {
    use super::*;
    use crate::{
        app::SystemClock,
        ports::{LocalService, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole},
            ids::{ExecutionId, HostTargetId},
            watch::WatchRequest,
        },
        service::dispatch::DomainService,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };

    struct Rig {
        path: std::path::PathBuf,
        registry: Arc<ModChannelRegistry>,
        domain: DomainService,
        kicks: Arc<AtomicUsize>,
    }
    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn rig() -> Rig {
        let path = std::env::temp_dir().join(format!("mod-domain-{}.db", uuid::Uuid::new_v4()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let sqlite = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), clock.clone()),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch(
                "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1);\
                 INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,1,0);\
                 INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,1,0,'fresh','term-p','inc','coherent_enumeration',1);\
                 INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'p','b',1,1,'claude','native','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,'term-p','inc');\
                 INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('s',1,5);",
            )
            .unwrap();
        }
        let kicks = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&kicks);
        let registry = Arc::new(ModChannelRegistry::new(
            clock.clone(),
            sqlite.clone(),
            "i".into(),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            ModDeliverySetting::On,
        ));
        let store: Arc<dyn StorePort> = sqlite;
        let domain = DomainService::new("i".into(), store, clock)
            .with_mod_channels(registry.clone() as Arc<dyn ModChannels>);
        Rig {
            path,
            registry,
            domain,
            kicks,
        }
    }

    fn db(rig: &Rig) -> rusqlite::Connection {
        rusqlite::Connection::open(&rig.path).unwrap()
    }

    fn request() -> WatchRequest {
        WatchRequest {
            claim: CallerClaim {
                instance: "i".into(),
                seat: seat("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Claude,
                native_session: NativeSessionId::new("native"),
                execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
                target: HostTargetId::new("p"),
            },
        }
    }

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        }
    }

    fn register(
        rig: &Rig,
        request: &WatchRequest,
    ) -> Result<(ModChannelId, u64), WatchRefusalReason> {
        rig.domain
            .watch_register(request, Arc::new(RecordingSink::default()), &budget())
    }

    #[test]
    fn watch_register_accepts_matching_claude_binding() {
        let rig = rig();
        let (_, version) = register(&rig, &request()).unwrap();
        assert_eq!(version, 5, "the seat's attention version at registration");
        assert!(rig.registry.is_live(&seat("s"), 1));
        let status = rig.registry.status().unwrap();
        assert_eq!(status.live_channels, 1);
        assert_eq!(status.channels[0].harness, "claude");
        assert_eq!(status.channels[0].binding_generation, 1);
    }

    #[test]
    fn registration_uses_the_bindings_generation_not_the_claims() {
        let rig = rig();
        let mut claimed = request();
        claimed.claim.binding_generation = 99;
        register(&rig, &claimed).unwrap();
        assert!(rig.registry.is_live(&seat("s"), 1));
        assert!(!rig.registry.is_live(&seat("s"), 99));
    }

    #[test]
    fn second_registration_replaces_the_first() {
        let rig = rig();
        let first = Arc::new(RecordingSink::default());
        rig.domain
            .watch_register(&request(), first.clone(), &budget())
            .unwrap();
        register(&rig, &request()).unwrap();
        assert_eq!(
            first.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::Replaced
            }]
        );
        assert_eq!(rig.registry.status().unwrap().channels.len(), 1);
    }

    #[test]
    fn not_claude() {
        // A claim that is not a Claude top-level agent.
        let rig = rig();
        let mut codex = request();
        codex.claim.harness = Harness::Codex;
        assert_eq!(
            register(&rig, &codex).unwrap_err(),
            WatchRefusalReason::NotClaude
        );
        let mut subagent = request();
        subagent.claim.role = CallerRole::Subagent;
        assert_eq!(
            register(&rig, &subagent).unwrap_err(),
            WatchRefusalReason::NotClaude
        );
        // A codex binding for the seat.
        db(&rig)
            .execute("UPDATE occupant_bindings SET harness='codex'", [])
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NotClaude
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn unresolved() {
        let rig = rig();
        db(&rig)
            .execute("UPDATE seats SET state='unresolved'", [])
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Unresolved
        );
        db(&rig)
            .execute(
                "UPDATE seats SET state='retired',retired_at=1,retired_seq=1",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Unresolved
        );
        let mut unknown = request();
        unknown.claim.seat = seat("nobody");
        assert_eq!(
            register(&rig, &unknown).unwrap_err(),
            WatchRefusalReason::Unresolved
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn held() {
        let rig = rig();
        db(&rig)
            .execute(
                "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','p','b',1,'hold')",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Held
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn no_binding() {
        let rig = rig();
        // Provenance other than cooperative_top_level.
        db(&rig)
            .execute(
                "UPDATE occupant_bindings SET observation_provenance='operator_human'",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        // An ended binding.
        db(&rig)
            .execute(
                "UPDATE occupant_bindings SET observation_provenance='cooperative_top_level',ended_at=9",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        // No binding at all.
        db(&rig)
            .execute("DELETE FROM occupant_bindings", [])
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn session_mismatch() {
        let rig = rig();
        let mut stale = request();
        stale.claim.native_session = NativeSessionId::new("an-older-session");
        assert_eq!(
            register(&rig, &stale).unwrap_err(),
            WatchRefusalReason::SessionMismatch
        );
        assert!(
            !rig.registry.is_live(&seat("s"), 1),
            "a refused registration is never live"
        );
        assert!(rig.registry.status().unwrap().channels.is_empty());
        let mut foreign = request();
        foreign.claim.instance = "other-instance".into();
        assert_eq!(
            register(&rig, &foreign).unwrap_err(),
            WatchRefusalReason::SessionMismatch
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn disabled() {
        let rig = rig();
        rig.registry.set_mod_delivery(ModDeliverySetting::Off);
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Disabled
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn cooldown() {
        let rig = rig();
        register(&rig, &request()).unwrap();
        // A stall handover starts the cooldown for generation 1.
        let now = SystemClock::new().utc_now();
        rig.registry
            .close(&seat("s"), WatchCloseReason::Stalled, now);
        assert_eq!(rig.kicks.load(Ordering::SeqCst), 1);
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Cooldown
        );
        assert!(!rig.registry.is_live(&seat("s"), 1));
    }

    #[test]
    fn a_store_failure_is_retryable_busy() {
        let rig = rig();
        let cancelled = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: {
                let c = Cancellation::default();
                c.cancel();
                c
            },
        };
        assert_eq!(
            rig.domain
                .watch_register(&request(), Arc::new(RecordingSink::default()), &cancelled)
                .unwrap_err(),
            WatchRefusalReason::Busy
        );
    }

    #[test]
    fn the_worker_pass_pushes_attention_for_a_real_send_and_closes_on_rebind() {
        let rig = rig();
        let sink = Arc::new(RecordingSink::default());
        rig.domain
            .watch_register(&request(), sink.clone(), &budget())
            .unwrap();
        let now = SystemClock::new().utc_now();
        // Nothing pending: the initial sweep pushes nothing.
        rig.registry.pass(now).unwrap();
        assert!(sink.frames().is_empty());
        // An ordinary message addressed to the seat.
        db(&rig)
            .execute_batch(
                "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
                 INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t',1,'ordinary','hello',0,1);\
                 INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t','s','pending',100);\
                 UPDATE wake_work SET attention_version=6 WHERE seat_id='s';",
            )
            .unwrap();
        rig.registry.pass(now).unwrap();
        assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 6 }]);
        // The binding ends: the channel closes into the rebind grace.
        db(&rig)
            .execute("UPDATE occupant_bindings SET ended_at=9", [])
            .unwrap();
        rig.registry.pass(now).unwrap();
        assert_eq!(
            sink.frames().last(),
            Some(&WatchFrame::Close {
                reason: WatchCloseReason::BindingChanged
            })
        );
        assert_eq!(
            rig.registry.status().unwrap().channels[0].state,
            ModChannelState::RebindGrace
        );
    }
}

#[test]
fn worker_pushes_on_the_observer_and_closes_stopping_on_cancel() {
    use crate::service::{pacer::Pacer, workers::start_mod_channel_worker};
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.set_view("a", Some(view("a", 1, pending(3))));
    let cancellation = Cancellation::default();
    let clock: Arc<dyn Clock> = f.clock.clone();
    let pacer = Arc::new(Pacer::new(
        "mod-channels",
        clock.clone(),
        cancellation.clone(),
    ));
    let worker = start_mod_channel_worker(
        Arc::clone(&f.registry),
        pacer,
        clock,
        cancellation.clone(),
        Arc::new(|_| {}),
    )
    .unwrap();
    // The registration already marked the registry dirty and woke the worker.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while sink.frames().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 3 }]);
    // A commit observed later pushes the next version.
    f.set_view("a", Some(view("a", 1, pending(4))));
    (f.registry.observer())();
    while sink.frames().len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(sink.frames()[1], WatchFrame::Attention { version: 4 });
    cancellation.cancel();
    worker.join().unwrap();
    assert_eq!(
        sink.frames().last(),
        Some(&WatchFrame::Close {
            reason: WatchCloseReason::Stopping
        })
    );
    assert!(!f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 1, "stopping removes the entry and kicks");
}

#[test]
fn seat_live_counts_every_unexpired_entry_whatever_its_generation() {
    let f = fixture();
    let (_, _sink) = f.register("a", 1);
    assert!(f.registry.seat_live(&seat("a")));
    assert!(!f.registry.seat_live(&seat("b")));
    f.registry.unregister(ModChannelId(1), f.now());
    assert!(f.registry.seat_live(&seat("a")), "reconnect grace");
    f.registry
        .close(&seat("a"), WatchCloseReason::BindingChanged, f.now());
    assert!(f.registry.seat_live(&seat("a")), "rebind grace");
    f.register("a", 2);
    assert!(f.registry.seat_live(&seat("a")), "live at generation 2");
}

#[test]
fn seat_live_ignores_an_expired_grace_before_the_sweep() {
    let f = fixture();
    let (id, _sink) = f.register("a", 1);
    f.registry.unregister(id, f.now());
    f.advance(MOD_RECONNECT_GRACE_MS as i64 + 1);
    assert!(!f.registry.seat_live(&seat("a")), "expired reconnect grace");
    let f = fixture();
    f.register("a", 1);
    f.registry
        .close(&seat("a"), WatchCloseReason::BindingChanged, f.now());
    f.advance(MOD_REBIND_GRACE_MS as i64 + 1);
    assert!(!f.registry.seat_live(&seat("a")), "expired rebind grace");
}

#[test]
fn registration_landing_between_pass_snapshot_and_close_survives() {
    type Mutate = fn(&mut ModSeatView);
    let cases: [(&str, Mutate); 3] = [
        ("binding moved", |v| {
            v.binding.as_mut().unwrap().generation = 2
        }),
        ("retired", |v| v.retired = true),
        ("unresolved", |v| v.continuity_resolved = false),
    ];
    for (name, mutate) in cases {
        let f = fixture();
        let (_, old_sink) = f.register("a", 1);
        let mut v = view("a", 2, pending(1));
        mutate(&mut v);
        f.set_view("a", Some(v));
        let new_sink = f.register_during_next_read("a", 2);
        f.registry.pass(f.now()).unwrap();
        assert!(
            new_sink.frames().is_empty(),
            "{name}: no Close for the new channel"
        );
        assert_eq!(
            old_sink.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::Replaced
            }],
            "{name}"
        );
        assert!(f.registry.is_live(&seat("a"), 2), "{name}");
        assert_eq!(
            f.registry.status().unwrap().channels[0].state,
            ModChannelState::Live,
            "{name}"
        );
        assert_eq!(f.kicks(), 0, "{name}");
    }
}

#[test]
fn registration_landing_during_the_stall_read_is_not_closed_and_arms_no_cooldown() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    let new_sink = f.register_during_next_read("a", 1);
    f.registry.sweep(now);
    assert!(new_sink.frames().is_empty(), "no Close{{stalled}}");
    assert!(f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 0);
    f.registry
        .close(&seat("a"), WatchCloseReason::Retired, f.now());
    f.try_register("a", 1).expect("no cooldown was armed");
}

#[test]
fn ack_landing_during_the_stall_read_prevents_the_stall_close() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    let registry = Arc::clone(&f.registry);
    *f.store.on_read.lock().unwrap() = Some(Box::new(move || {
        registry.record_ack(&seat("a"), 1, now);
    }));
    f.registry.sweep(now);
    assert!(f.registry.is_live(&seat("a"), 1));
    assert_eq!(f.kicks(), 0);
    f.try_register("a", 1).expect("no cooldown was armed");
}

#[test]
fn record_ack_and_attention_push_for_another_generation_are_ignored() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    assert!(f.registry.stalled(&seat("a"), now));
    f.registry.record_ack(&seat("a"), 2, now);
    assert!(
        f.registry.stalled(&seat("a"), now),
        "an ack for another generation does not reset the stall clock"
    );
    // A channel that never saw a push is not stalled; a push for another
    // generation must not arm the predicate.
    let f = fixture();
    f.register("a", 1);
    f.store.receipts.lock().unwrap().push((f.now(), 10));
    f.advance(over);
    f.registry.record_attention_push(&seat("a"), 2, f.now());
    assert!(!f.registry.stalled(&seat("a"), f.now()));
    f.registry.record_attention_push(&seat("a"), 1, f.now());
    assert!(f.registry.stalled(&seat("a"), f.now()));
}
