use super::*;
use crate::{
    ports::{ModBindingRef, ModBindingView, ModChannelMode, ModSeatView},
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
    views: Mutex<HashMap<BindingId, Option<ModSeatView>>>,
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
    fn mod_binding_view(
        &self,
        _seat: &SeatId,
        binding: ModBindingRef<'_>,
        _budget: &CallBudget,
    ) -> Result<Option<ModSeatView>, ApiError> {
        let fail = self.fail_views.load(Ordering::SeqCst);
        let ModBindingRef::Id(binding) = binding else {
            panic!("the registry reads bindings by id");
        };
        let view = self.views.lock().unwrap().get(binding).cloned().flatten();
        self.run_hook();
        if fail {
            return Err(ApiError::new(ErrorCode::StoreBusy, "busy".to_owned()));
        }
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
    fixture_build(setting, None)
}
fn fixture_with_cap(cap: usize) -> Fixture {
    fixture_build(ModDeliverySetting::On, Some(cap))
}
fn fixture_build(setting: ModDeliverySetting, cap: Option<usize>) -> Fixture {
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
    let registry = match cap {
        Some(cap) => Arc::new(
            Arc::try_unwrap(registry)
                .ok()
                .expect("sole owner")
                .with_connection_cap(cap),
        ),
        None => registry,
    };
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
        self.register_mode(seat, generation, ModChannelMode::Legacy)
    }
    fn register_mode(
        &self,
        seat: &str,
        generation: u64,
        mode: ModChannelMode,
    ) -> (ModChannelId, Arc<RecordingSink>) {
        self.try_register_mode(seat, generation, mode).unwrap()
    }
    fn try_register(
        &self,
        seat: &str,
        generation: u64,
    ) -> Result<(ModChannelId, Arc<RecordingSink>), WatchRefusalReason> {
        self.try_register_mode(seat, generation, ModChannelMode::Legacy)
    }
    fn try_register_mode(
        &self,
        seat: &str,
        generation: u64,
        mode: ModChannelMode,
    ) -> Result<(ModChannelId, Arc<RecordingSink>), WatchRefusalReason> {
        let sink = Arc::new(RecordingSink::default());
        let id = self.registry.register(
            ModChannelRegistration {
                mode,
                seat: SeatId::new(seat),
                binding: bid(seat, generation),
                binding_generation: generation,
                native_session: NativeSessionId::new("native"),
                harness: Harness::Claude,
                idle_wake: IdleWake::Channel,
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
            mode: ModChannelMode::Legacy,
            seat: seat(seat_name),
            binding: bid(seat_name, generation),
            binding_generation: generation,
            native_session: NativeSessionId::new("native"),
            harness: Harness::Claude,
            idle_wake: IdleWake::Channel,
            registered_at: self.now(),
        };
        let hook_sink = sink.clone();
        *self.store.on_read.lock().unwrap() = Some(Box::new(move || {
            registry.register(registration, hook_sink).unwrap();
        }));
        sink
    }
    /// Scripts what a pass reads for `binding`.
    fn set_view(&self, binding: &BindingId, view: Option<ModSeatView>) {
        self.store
            .views
            .lock()
            .unwrap()
            .insert(binding.clone(), view);
    }
}

fn seat(name: &str) -> SeatId {
    SeatId::new(name)
}

/// The binding the fixture registers for `(seat, generation)`: one binding per pair.
fn bid(seat: &str, generation: u64) -> BindingId {
    BindingId::new(format!("{seat}-{generation}"))
}

fn view(seat_name: &str, generation: u64, fingerprint: ModFingerprint) -> ModSeatView {
    ModSeatView {
        seat: seat(seat_name),
        state: "resolved".into(),
        retired: false,
        continuity_resolved: true,
        held: false,
        binding: Some(ModBindingView {
            binding_id: bid(seat_name, generation),
            generation,
            provenance: "cooperative_top_level".into(),
            harness: "claude".into(),
            native_session: "native".into(),
            top_level: true,
            connected: true,
            ended_at: None,
            disconnect_reason: None,
        }),
        attention_version: fingerprint.attention_version,
        fingerprint: ModFingerprint {
            binding_generation: Some(generation),
            ..fingerprint
        },
        newest_pending_notice: None,
        pending_invitation_version: 0,
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
fn register_then_is_live_for_its_binding_only() {
    let f = fixture();
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
    f.register("a", 1);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 2)));
    assert!(!f.registry.is_live(&seat("b"), &bid("b", 1)));
    // The binding's channel answers for its own seat only.
    assert!(!f.registry.is_live(&seat("b"), &bid("a", 1)));
    assert_eq!(f.kicks(), 0);
    let logs = f.logs.lock().unwrap().clone();
    assert_eq!(
        logs,
        vec![
            "mod channel registered seat=a binding=a-1 generation=1 provenance=cooperative_mod_channel"
        ]
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
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
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
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
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
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
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
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(
        f.registry.status().unwrap().channels[0].state,
        ModChannelState::ReconnectGrace
    );
    f.registry.sweep(f.now());
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.kicks(), 0);
    f.advance(2_000);
    // Expired but not yet swept: already not live.
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
    f.registry.sweep(f.now());
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
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
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert!(sink.frames().is_empty());
    assert_eq!(
        f.registry.status().unwrap().channels[0].state,
        ModChannelState::Live
    );
}

#[test]
fn close_binding_changed_starts_that_bindings_rebind_grace_only() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    let (_, sibling) = f.register("a", 2);
    f.registry
        .close(&bid("a", 1), WatchCloseReason::BindingChanged, f.now());
    assert_eq!(
        sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::BindingChanged
        }]
    );
    assert!(
        sibling.frames().is_empty(),
        "the seat's other channel is untouched"
    );
    assert_eq!(f.kicks(), 0);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert!(f.registry.is_live(&seat("a"), &bid("a", 2)));
    assert!(
        !f.registry.is_live(&seat("a"), &bid("a", 3)),
        "the grace covers that binding, not every binding of the seat"
    );
    let states: Vec<ModChannelState> = f
        .registry
        .status()
        .unwrap()
        .channels
        .iter()
        .map(|c| c.state)
        .collect();
    assert_eq!(
        states,
        vec![ModChannelState::RebindGrace, ModChannelState::Live]
    );
    // A registration for the same binding ends the grace without a kick.
    f.advance(5_000);
    f.register("a", 1);
    f.advance(60_000);
    f.registry.sweep(f.now());
    assert_eq!(f.kicks(), 0);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));

    // Without one, expiry removes the entry and kicks.
    f.registry
        .close(&bid("a", 1), WatchCloseReason::BindingChanged, f.now());
    f.advance(29_000);
    f.registry.sweep(f.now());
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    f.advance(2_000);
    f.registry.sweep(f.now());
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.kicks(), 1);
    assert!(
        f.registry.is_live(&seat("a"), &bid("a", 2)),
        "the sibling is still live"
    );
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
        f.registry.close(&bid("a", 1), reason, f.now());
        assert_eq!(
            sink.frames(),
            vec![WatchFrame::Close { reason }],
            "{reason:?}"
        );
        assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)), "{reason:?}");
        assert_eq!(f.kicks(), 1, "{reason:?}");
        assert!(f.registry.status().unwrap().channels.is_empty());
        // Closing a seat with no entry is a no-op: no second kick.
        f.registry.close(&bid("a", 1), reason, f.now());
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
        f.registry
            .record_attention_push(&seat("a"), &bid("a", 1), registered);
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
    f.registry.record_ack(&seat("a"), &bid("a", 1), f.now());
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
        .record_attention_push(&seat("a"), &bid("a", 1), UtcMillis(now.0 - 500));
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
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.kicks(), 1);
    // Refused for ten minutes for that generation only.
    f.advance(599_000);
    assert_eq!(
        f.try_register("a", 1).err(),
        Some(WatchRefusalReason::Cooldown)
    );
    f.try_register("a", 2).unwrap();
    f.registry
        .close(&bid("a", 1), WatchCloseReason::Retired, f.now());
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
    f.registry
        .record_attention_push(&seat("a"), &bid("a", 1), registered);
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
    f.registry.record_ack(&seat("a"), &bid("a", 1), now);
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
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(3))));
    // First reading with something pending is the initial sweep.
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 3 }]);
    // A changed fingerprint pushes again with the view's version.
    f.advance(1_000);
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(4))));
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

/// ht-yha.58 (ht-3eic): the worker reads its clock before `pass` reads the store, so an item can commit in
/// between. The push that announces that item must be recorded no earlier than the item, or the stall rule
/// (items published at or before the last push) never counts it and a wedged mod is never handed over.
#[test]
fn a_push_for_an_item_committed_after_the_pass_clock_read_still_arms_the_stall() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    // The worker's `now`, read before the commit landed.
    let stale_now = f.now();
    f.advance(50);
    let published = f.now();
    f.store.receipts.lock().unwrap().push((published, 10));
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(3))));
    f.registry.pass(stale_now).unwrap();
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 3 }]);
    f.advance(MOD_STALL_AFTER_MS as i64 + 10);
    assert!(
        f.registry.stalled(&seat("a"), f.now()),
        "the recorded push must cover the item it announced"
    );
    let calls = f.store.stall_calls.lock().unwrap().clone();
    assert_eq!(calls, vec![(published, WATCH_BODY_LIMIT_BYTES)]);
}

#[test]
fn notify_without_fingerprint_change_pushes_nothing() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    // Nothing pending on the first reading: no frame.
    f.set_view(&bid("a", 1), Some(view("a", 1, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    assert!(sink.frames().is_empty());
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(2))));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1);
    f.registry.pass(f.now()).unwrap();
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1, "an unchanged fingerprint is silent");
    // A settlement that only lowers the count is a change too.
    f.set_view(
        &bid("a", 1),
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
    let cases: [(&str, Mutate, WatchCloseReason, usize); 7] = [
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
            "binding gone",
            |v| v.binding = None,
            WatchCloseReason::BindingChanged,
            0,
        ),
        (
            "binding ended",
            |v| v.binding.as_mut().unwrap().ended_at = Some(UtcMillis(9)),
            WatchCloseReason::BindingChanged,
            0,
        ),
        (
            "binding disconnected",
            |v| v.binding.as_mut().unwrap().connected = false,
            WatchCloseReason::BindingChanged,
            0,
        ),
        (
            "binding no longer top-level",
            |v| v.binding.as_mut().unwrap().top_level = false,
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
        f.set_view(&bid("a", 1), Some(v));
        f.registry.pass(f.now()).unwrap();
        assert_eq!(sink.frames(), vec![WatchFrame::Close { reason }], "{name}");
        assert_eq!(f.kicks(), kicks, "{name}");
    }
    // A seat that vanished from the store is unresolved.
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.set_view(&bid("a", 1), None);
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Unresolved
        }]
    );
}

#[test]
fn pass_judges_each_channel_against_its_own_binding() {
    let f = fixture();
    let (_, a) = f.register("a", 1);
    let (_, b) = f.register("a", 2);
    // Binding 1 disconnected, binding 2 fine and with mail pending.
    let mut gone = view("a", 1, pending(1));
    gone.binding.as_mut().unwrap().connected = false;
    f.set_view(&bid("a", 1), Some(gone));
    f.set_view(&bid("a", 2), Some(view("a", 2, pending(1))));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        a.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::BindingChanged
        }]
    );
    assert_eq!(b.frames(), vec![WatchFrame::Attention { version: 1 }]);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 2)));
    let mut moved = view("a", 2, pending(1));
    moved.binding.as_mut().unwrap().generation = 7;
    f.set_view(&bid("a", 2), Some(moved));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        b.frames().len(),
        1,
        "a placement refresh (generation moved) neither closes nor pushes"
    );
}

#[test]
fn pass_reports_a_store_error_and_keeps_the_channel() {
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.store.fail_views.store(true, Ordering::SeqCst);
    let error = f.registry.pass(f.now()).unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
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
fn registration_over_a_configured_cap_is_busy_but_replacement_is_not() {
    let f = fixture_with_cap(2);
    f.register("seat-0", 1);
    f.register("seat-1", 1);
    assert_eq!(
        f.try_register("seat-2", 1).err(),
        Some(WatchRefusalReason::Busy)
    );
    f.try_register("seat-0", 1).unwrap();
    assert_eq!(f.registry.status().unwrap().channels.len(), 2);
}

#[test]
fn status_reports_setting_counts_and_states() {
    let f = fixture();
    let (grace, _) = f.register("b", 2);
    f.register("a", 1);
    f.register("c", 3);
    f.registry
        .close(&bid("c", 3), WatchCloseReason::BindingChanged, f.now());
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
        let path = crate::test_support::scratch::temp_path(format!(
            "mod-domain-{}.db",
            uuid::Uuid::new_v4()
        ));
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

    impl Rig {
        /// The binding of the claim `request()` makes (session `native`).
        fn binding(&self) -> BindingId {
            self.binding_of("native")
        }

        fn binding_of(&self, session: &str) -> BindingId {
            BindingId::new(
                db(self)
                    .query_row(
                        "SELECT binding_id FROM occupant_bindings WHERE native_session=?1 AND agent_id=''",
                        [session],
                        |r| r.get::<_, String>(0),
                    )
                    .unwrap(),
            )
        }

        /// Another top-level Claude binding of seat `s` (a second session) and its id.
        fn add_binding(&self, session: &str) -> BindingId {
            db(self)
                .execute(
                    "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'p','b',1,1,'claude',?1,'00000000-0000-4000-8000-0000000000bb','cooperative_top_level',0,'term-p','inc')",
                    [session],
                )
                .unwrap();
            self.binding_of(session)
        }
    }

    fn request_for(session: &str) -> WatchRequest {
        let mut request = request();
        request.claim.native_session = NativeSessionId::new(session);
        request
    }

    fn request() -> WatchRequest {
        WatchRequest {
            presentation_v2: false,
            claim: CallerClaim {
                instance: "i".into(),
                seat: seat("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Claude,
                native_session: NativeSessionId::new("native"),
                execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
                target: HostTargetId::new("p"),
                agent_id: None,
                liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
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
    fn presentation_owner_current_watch_is_admitted_and_old_lease_is_refused() {
        use crate::protocol::{
            authority::PeerIdentity,
            commands::Command,
            delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
            results::CommandResult,
        };
        let rig = rig();
        let req = q3_request_for(&rig, "native", true);
        let (first, _) = register(&rig, &req).unwrap();
        let old = rig
            .registry
            .presentation_lease(first)
            .expect("current owner lease");
        let pull = |lease: String| {
            rig.domain
                .handle(
                    Command::DeliveryPullV2(DeliveryPullV2 {
                        claim: req.claim.clone(),
                        consumer: PresentationConsumer::Watch,
                        render:
                            crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some(lease),
                        marker_prefix: vec![],
                    }),
                    PeerIdentity::from_kernel(501),
                    &budget(),
                )
                .unwrap()
        };
        assert!(
            matches!(
                pull(old.clone()),
                CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Unchanged { .. })
            ),
            "current exact Live owner must be admitted"
        );
        let (second, _) = register(&rig, &req).unwrap();
        let new = rig.registry.presentation_lease(second).unwrap();
        assert!(
            matches!(
                pull(old),
                CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
            ),
            "replacement fences previous lease"
        );
        assert!(matches!(
            pull(new.clone()),
            CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Unchanged { .. })
        ));
        rig.registry
            .unregister(second, rig.registry.clock.utc_now());
        assert!(
            matches!(
                pull(new),
                CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
            ),
            "reconnect grace grants no presentation authority"
        );
    }

    #[test]
    fn watch_register_accepts_matching_claude_binding() {
        let rig = rig();
        let (_, version) = register(&rig, &request()).unwrap();
        assert_eq!(version, 5, "the seat's attention version at registration");
        assert!(rig.registry.is_live(&seat("s"), &rig.binding()));
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
        assert!(rig.registry.is_live(&seat("s"), &rig.binding()));
        assert!(!rig.registry.is_live(&seat("s"), &bid("s", 99)));
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
        let mut agent = request();
        agent.claim.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
        assert_eq!(
            register(&rig, &agent).unwrap_err(),
            WatchRefusalReason::NotClaude,
            "a claim carrying an agent id is a subagent's"
        );
        // The seat's binding is a codex one: the Claude claim's key names no binding.
        db(&rig)
            .execute("UPDATE occupant_bindings SET harness='codex'", [])
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
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
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
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
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
    }

    #[test]
    fn watch_register_accepts_a_plain_call_enrolled_binding() {
        let rig = rig();
        db(&rig)
            .execute(
                "UPDATE occupant_bindings SET observation_provenance='cooperative_env_session'",
                [],
            )
            .unwrap();
        register(&rig, &request()).unwrap();
        assert!(rig.registry.is_live(&seat("s"), &rig.binding()));
        let provenance: String = db(&rig)
            .query_row(
                "SELECT observation_provenance FROM occupant_bindings",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(provenance, "cooperative_env_session");
    }

    #[test]
    fn no_binding() {
        let rig = rig();
        let held = rig.binding();
        // The claim's key resolves to no top-level row (the binding is a subagent's).
        db(&rig)
            .execute(
                "UPDATE occupant_bindings SET role='subagent',agent_id='x',parent_binding=binding_id",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        // A disconnected binding holds no channel.
        db(&rig)
            .execute(
                "UPDATE occupant_bindings SET role='top_level',agent_id='',parent_binding=NULL,liveness='disconnected',disconnected_at=5,disconnect_reason='process_exit'",
                [],
            )
            .unwrap();
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        // An ended binding.
        db(&rig)
            .execute("UPDATE occupant_bindings SET ended_at=9", [])
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
        assert!(!rig.registry.is_live(&seat("s"), &held));
    }

    #[test]
    fn session_mismatch() {
        let rig = rig();
        // A session the seat holds no binding for names no binding (it is not another
        // session's channel to take).
        let mut stale = request();
        stale.claim.native_session = NativeSessionId::new("an-older-session");
        assert_eq!(
            register(&rig, &stale).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        assert!(
            !rig.registry.is_live(&seat("s"), &rig.binding()),
            "a refused registration is never live"
        );
        assert!(rig.registry.status().unwrap().channels.is_empty());
        let mut foreign = request();
        foreign.claim.instance = "other-instance".into();
        assert_eq!(
            register(&rig, &foreign).unwrap_err(),
            WatchRefusalReason::SessionMismatch
        );
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
    }

    #[test]
    fn disabled() {
        let rig = rig();
        rig.registry.set_mod_delivery(ModDeliverySetting::Off);
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Disabled
        );
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
    }

    #[test]
    fn cooldown() {
        let rig = rig();
        register(&rig, &request()).unwrap();
        // A stall handover starts the cooldown for generation 1.
        let now = SystemClock::new().utc_now();
        rig.registry
            .close(&rig.binding(), WatchCloseReason::Stalled, now);
        assert_eq!(rig.kicks.load(Ordering::SeqCst), 1);
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::Cooldown
        );
        assert!(!rig.registry.is_live(&seat("s"), &rig.binding()));
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

    fn register_with(rig: &Rig, request: &WatchRequest) -> (ModChannelId, Arc<RecordingSink>) {
        let sink = Arc::new(RecordingSink::default());
        let (id, _) = rig
            .domain
            .watch_register(request, sink.clone(), &budget())
            .unwrap();
        (id, sink)
    }

    fn q3_request_for(rig: &Rig, session: &str, presentation: bool) -> WatchRequest {
        let mut req = request_for(session);
        let (generation, execution, target): (i64, String, String) = db(rig).query_row(
            "SELECT generation,execution_id,target_id FROM occupant_bindings WHERE seat_id='s' AND native_session=?1 AND agent_id='' AND ended_at IS NULL",
            [session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        req.claim.binding_generation = u64::try_from(generation).expect("nonnegative generation");
        req.claim.execution = ExecutionId::new(execution);
        req.claim.target = HostTargetId::new(target);
        let mut value = serde_json::to_value(req).unwrap();
        if presentation {
            value["presentation_v2"] = serde_json::json!(true);
        }
        let decoded = serde_json::from_value(value);
        assert!(
            decoded.is_ok(),
            "explicit Watch mode must decode: {decoded:?}"
        );
        decoded.unwrap()
    }

    fn q3_pull(
        rig: &Rig,
        claim: &CallerClaim,
        lease: &str,
    ) -> crate::protocol::delivery::DeliveryPullReplyV2 {
        use crate::protocol::{
            authority::PeerIdentity, commands::Command, delivery::*, results::CommandResult,
        };
        match rig
            .domain
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: claim.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some(lease.into()),
                    marker_prefix: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap()
        {
            CommandResult::DeliveryPulledV2(reply) => reply,
            other => panic!("unexpected pull result {other:?}"),
        }
    }

    fn q3_confirm(
        rig: &Rig,
        claim: &CallerClaim,
        lease: &str,
    ) -> crate::protocol::delivery::DeliveryPresentationConfirmationV2 {
        use crate::protocol::{
            authority::PeerIdentity, commands::Command, delivery::*, results::CommandResult,
        };
        match rig
            .domain
            .handle(
                Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                    claim: claim.clone(),
                    consumer: PresentationConsumer::Watch,
                    token: "11111111111111111111111111111111".into(),
                    presentation_lease: Some(lease.into()),
                    emitted_controls: vec![],
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap()
        {
            CommandResult::DeliveryPresentationConfirmedV2(reply) => reply.status,
            other => panic!("unexpected confirmation result {other:?}"),
        }
    }

    fn q3_due(binding: BindingId) -> crate::delivery::ledger::OrdinaryDueSnapshot {
        crate::delivery::ledger::OrdinaryDueSnapshot {
            incarnation: [1; 16],
            generation: 1,
            binding_id: binding,
            stamps: Default::default(),
            cache: Default::default(),
        }
    }

    #[test]
    fn q3_legacy_claim_only_registration_has_no_presentation_lease() {
        let rig = rig();
        let (channel, _) = register(&rig, &request()).unwrap();
        assert!(rig.registry.binding_live(&seat("s"), &rig.binding()));
        assert_eq!(rig.domain.watch_presentation_lease(channel), None);
    }

    #[test]
    fn q3_legacy_channel_cannot_pull_with_an_issued_or_guessed_lease() {
        use crate::protocol::delivery::DeliveryPullReplyV2;
        let rig = rig();
        let req = request();
        let (channel, _) = register(&rig, &req).unwrap();
        // Before Q3 the registry issues this legacy channel a real lease. Use it
        // when present so the old admission bug, rather than a random-token miss,
        // is the decisive control.
        let lease = rig
            .domain
            .watch_presentation_lease(channel)
            .unwrap_or_else(|| "0123456789abcdef0123456789abcdef".into());
        assert!(
            matches!(
                q3_pull(&rig, &req.claim, &lease),
                DeliveryPullReplyV2::Refused { .. }
            ),
            "legacy registration cannot acquire V2 authority through a token"
        );
    }

    #[test]
    fn q3_legacy_channel_cannot_capture_owner_replay() {
        let rig = rig();
        let req = request();
        let (channel, _) = register(&rig, &req).unwrap();
        let lease = rig
            .domain
            .watch_presentation_lease(channel)
            .unwrap_or_else(|| "0123456789abcdef0123456789abcdef".into());
        assert!(
            rig.registry
                .presentation_replay_snapshot(&req.claim, &lease, &q3_due(rig.binding()))
                .is_none()
        );
    }

    #[test]
    fn q3_legacy_channel_cannot_confirm_with_an_issued_or_guessed_lease() {
        use crate::protocol::delivery::DeliveryPresentationConfirmationV2;
        let rig = rig();
        let req = request();
        let (channel, _) = register(&rig, &req).unwrap();
        let lease = rig
            .domain
            .watch_presentation_lease(channel)
            .unwrap_or_else(|| "0123456789abcdef0123456789abcdef".into());
        assert_eq!(
            q3_confirm(&rig, &req.claim, &lease),
            DeliveryPresentationConfirmationV2::StaleBinding
        );
    }

    #[test]
    fn q3_mixed_same_seat_modes_and_replacement_cannot_borrow_old_leases() {
        use crate::protocol::delivery::{DeliveryPresentationConfirmationV2, DeliveryPullReplyV2};
        let rig = rig();
        let sibling_binding = rig.add_binding("native2");
        assert_ne!(rig.binding(), sibling_binding);
        let connected: i64 = db(&rig).query_row(
            "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id='s' AND liveness='connected' AND ended_at IS NULL", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(connected, 2);
        let capable = q3_request_for(&rig, "native", true);
        let legacy = q3_request_for(&rig, "native2", false);
        let (first, _) = register(&rig, &capable).unwrap();
        let old = rig
            .domain
            .watch_presentation_lease(first)
            .expect("opted-in channel lease");
        assert!(crate::protocol::delivery::valid_presentation_token(&old));
        let sibling_capable = q3_request_for(&rig, "native2", true);
        assert_ne!(capable.claim.execution, sibling_capable.claim.execution);
        let (sibling_owner, _) = register(&rig, &sibling_capable).unwrap();
        let sibling_lease = rig.domain.watch_presentation_lease(sibling_owner).unwrap();
        assert!(
            matches!(
                q3_pull(&rig, &sibling_capable.claim, &sibling_lease),
                DeliveryPullReplyV2::Unchanged { .. }
            ),
            "the canonical sibling claim is positively admitted with its own capable lease"
        );
        let mut stale_execution = sibling_capable.claim.clone();
        stale_execution.execution = capable.claim.execution.clone();
        assert!(
            matches!(
                q3_pull(&rig, &stale_execution, &sibling_lease),
                DeliveryPullReplyV2::Refused { .. }
            ),
            "execution mismatch remains a separate negative"
        );
        let (sibling, _) = register(&rig, &legacy).unwrap();
        assert!(matches!(
            q3_pull(&rig, &sibling_capable.claim, &sibling_lease),
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert_eq!(rig.domain.watch_presentation_lease(sibling), None);
        assert!(matches!(
            q3_pull(&rig, &capable.claim, &old),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        assert!(matches!(
            q3_pull(&rig, &legacy.claim, &old),
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert!(
            rig.registry
                .presentation_replay_snapshot(&legacy.claim, &old, &q3_due(sibling_binding))
                .is_none()
        );

        // Capable -> Legacy replacement owns neither the old token nor replay.
        let legacy_owner = q3_request_for(&rig, "native", false);
        let (replacement, _) = register(&rig, &legacy_owner).unwrap();
        assert_eq!(rig.domain.watch_presentation_lease(first), None);
        assert_eq!(rig.domain.watch_presentation_lease(replacement), None);
        assert!(matches!(
            q3_pull(&rig, &capable.claim, &old),
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert_eq!(
            q3_confirm(&rig, &capable.claim, &old),
            DeliveryPresentationConfirmationV2::StaleBinding
        );
        assert!(
            rig.registry
                .presentation_replay_snapshot(&capable.claim, &old, &q3_due(rig.binding()))
                .is_none()
        );

        // Legacy -> Capable replacement gets fresh exact-owner authority.
        let (current, _) = register(&rig, &capable).unwrap();
        let new = rig.domain.watch_presentation_lease(current).unwrap();
        assert_ne!(old, new);
        assert!(matches!(
            q3_pull(&rig, &capable.claim, &old),
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert!(matches!(
            q3_pull(&rig, &capable.claim, &new),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        let mut stale_generation = capable.claim.clone();
        stale_generation.binding_generation += 1;
        assert!(matches!(
            q3_pull(&rig, &stale_generation, &new),
            DeliveryPullReplyV2::Refused { .. }
        ));
        let mut stale_session = capable.claim.clone();
        stale_session.native_session = NativeSessionId::new("native2");
        assert!(matches!(
            q3_pull(&rig, &stale_session, &new),
            DeliveryPullReplyV2::Refused { .. }
        ));
        rig.domain
            .watch_unregister(current, rig.registry.clock.utc_now());
        assert!(
            rig.registry.is_live(&seat("s"), &rig.binding()),
            "legacy grace policy remains"
        );
        assert_eq!(rig.domain.watch_presentation_lease(current), None);
        assert!(matches!(
            q3_pull(&rig, &capable.claim, &new),
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert_eq!(
            q3_confirm(&rig, &capable.claim, &new),
            DeliveryPresentationConfirmationV2::StaleBinding
        );
        assert!(
            rig.registry
                .presentation_replay_snapshot(&capable.claim, &new, &q3_due(rig.binding()))
                .is_none()
        );
    }

    /// An ordinary message `id` (sequence `seq` of thread `t`) published to seat `s`.
    fn deliver(db: &rusqlite::Connection, id: &str, seq: u32) {
        db.execute_batch(&format!(
            "INSERT OR IGNORE INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
             INSERT OR IGNORE INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined');
             INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','{id}','t',{seq},'ordinary','b',{seq},{seq});
             INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p-{id}','i','actor','{id}',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
             INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p-{id}','t','s',1,300,0);
             INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) SELECT 'p-{id}','{id}','i','t',decision_seq,{seq},{seq},0,1,0 FROM messages WHERE id='{id}';"
        ))
        .unwrap();
    }

    #[test]
    fn two_claude_bindings_hold_two_live_channels() {
        let rig = rig();
        let (a, b) = (rig.binding(), rig.add_binding("native2"));
        register(&rig, &request()).unwrap();
        register(&rig, &request_for("native2")).unwrap();
        assert!(rig.registry.binding_live(&seat("s"), &a));
        assert!(rig.registry.binding_live(&seat("s"), &b));
        assert!(rig.registry.is_live(&seat("s"), &a) && rig.registry.is_live(&seat("s"), &b));
        let status = rig.registry.status().unwrap();
        assert_eq!(status.live_channels, 2);
        assert_eq!(status.channels.len(), 2);
    }

    #[test]
    fn each_channel_receives_only_its_bindings_deliveries() {
        let rig = rig();
        let (a, b) = (rig.binding(), rig.add_binding("native2"));
        let (_, a_sink) = register_with(&rig, &request());
        let (_, b_sink) = register_with(&rig, &request_for("native2"));
        let conn = db(&rig);
        deliver(&conn, "m1", 1);
        // A has read m1, B has not. (The seat's pending receipt is seat-level attention both
        // channels see; what differs is each binding's own unread.)
        for (binding, seq) in [(&a, 1), (&b, 0)] {
            conn.execute(
                "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,'t',?2)",
                rusqlite::params![binding.as_str(), seq],
            )
            .unwrap();
        }
        let now = SystemClock::new().utc_now();
        rig.registry.pass(now).unwrap();
        assert_eq!(a_sink.frames(), vec![WatchFrame::Attention { version: 5 }]);
        assert_eq!(b_sink.frames(), vec![WatchFrame::Attention { version: 5 }]);
        // Nothing changed: neither is pushed again.
        rig.registry.pass(now).unwrap();
        assert_eq!((a_sink.frames().len(), b_sink.frames().len()), (1, 1));
        // New mail is unread for both: both channels are pushed, each once.
        deliver(&conn, "m2", 2);
        conn.execute("UPDATE wake_work SET attention_version=6", [])
            .unwrap();
        rig.registry.pass(now).unwrap();
        assert_eq!(
            a_sink.frames().last(),
            Some(&WatchFrame::Attention { version: 6 })
        );
        assert_eq!(
            b_sink.frames().last(),
            Some(&WatchFrame::Attention { version: 6 })
        );
        assert_eq!((a_sink.frames().len(), b_sink.frames().len()), (2, 2));
        // A reads on: only A's own unread moved, so only A's channel is pushed.
        conn.execute(
            "UPDATE binding_cursors SET seq=2 WHERE binding_id=?1",
            [a.as_str()],
        )
        .unwrap();
        rig.registry.pass(now).unwrap();
        assert_eq!(
            a_sink.frames().len(),
            3,
            "A's fingerprint moved (its unread cleared)"
        );
        assert_eq!(
            b_sink.frames().len(),
            2,
            "B's did not: A's cursor is not B's"
        );
    }

    #[test]
    fn closing_one_channel_leaves_the_other_live() {
        let rig = rig();
        let (a, b) = (rig.binding(), rig.add_binding("native2"));
        let (_, a_sink) = register_with(&rig, &request());
        let (_, b_sink) = register_with(&rig, &request_for("native2"));
        let now = SystemClock::new().utc_now();
        rig.registry.close(&a, WatchCloseReason::Retired, now);
        assert_eq!(
            a_sink.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::Retired
            }]
        );
        assert!(b_sink.frames().is_empty());
        assert!(!rig.registry.binding_live(&seat("s"), &a));
        assert!(rig.registry.binding_live(&seat("s"), &b));
        // The surviving channel still holds the seat's channel registration.
        rig.registry.pass(now).unwrap();
        assert!(rig.registry.binding_live(&seat("s"), &b));
    }

    #[test]
    fn re_registering_a_binding_replaces_only_its_own_channel() {
        let rig = rig();
        let _ = rig.add_binding("native2");
        let (_, first) = register_with(&rig, &request());
        let (_, sibling) = register_with(&rig, &request_for("native2"));
        let (_, second) = register_with(&rig, &request());
        assert_eq!(
            first.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::Replaced
            }]
        );
        assert!(sibling.frames().is_empty());
        assert!(second.frames().is_empty());
        assert_eq!(rig.registry.status().unwrap().channels.len(), 2);
    }

    #[test]
    fn disconnect_closes_that_bindings_channel() {
        use crate::store::bindings::{DisconnectReason, disconnect};
        let rig = rig();
        let (a, b) = (rig.binding(), rig.add_binding("native2"));
        let (_, a_sink) = register_with(&rig, &request());
        let (_, b_sink) = register_with(&rig, &request_for("native2"));
        let now = SystemClock::new().utc_now();
        rig.registry.pass(now).unwrap();
        assert!(a_sink.frames().is_empty() && b_sink.frames().is_empty());
        let conn = db(&rig);
        let version = |conn: &rusqlite::Connection| -> i64 {
            conn.query_row(
                "SELECT attention_version FROM wake_work WHERE seat_id='s'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        let before = version(&conn);
        assert!(disconnect(&conn, &a, DisconnectReason::ProcessExit, UtcMillis(5)).unwrap());
        assert_eq!(
            version(&conn),
            before + 1,
            "the mod_channel_release hook bumps the seat's attention version"
        );
        rig.registry.pass(now).unwrap();
        assert_eq!(
            a_sink.frames(),
            vec![WatchFrame::Close {
                reason: WatchCloseReason::BindingChanged
            }]
        );
        assert!(
            !b_sink
                .frames()
                .iter()
                .any(|frame| matches!(frame, WatchFrame::Close { .. })),
            "B stays connected: its channel is not closed"
        );
        assert!(rig.registry.binding_live(&seat("s"), &b));
        // A disconnected binding cannot register again until it reconnects.
        assert_eq!(
            register(&rig, &request()).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        // A repeated disconnect changes nothing and does not bump again.
        let bumped = version(&conn);
        assert!(!disconnect(&conn, &a, DisconnectReason::ProcessExit, UtcMillis(6)).unwrap());
        assert_eq!(version(&conn), bumped);
    }

    #[test]
    fn subagent_and_unknown_session_cannot_register() {
        let rig = rig();
        let mut subagent = request();
        subagent.claim.role = CallerRole::Subagent;
        let mut with_agent = request();
        with_agent.claim.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
        for refused in [subagent, with_agent] {
            assert_eq!(
                register(&rig, &refused).unwrap_err(),
                WatchRefusalReason::NotClaude
            );
        }
        assert_eq!(
            register(&rig, &request_for("never-seen")).unwrap_err(),
            WatchRefusalReason::NoBinding
        );
        assert!(rig.registry.status().unwrap().channels.is_empty());
        // The seat's own session still registers.
        register(&rig, &request()).unwrap();
    }
}

#[test]
fn worker_pushes_on_the_observer_and_closes_stopping_on_cancel() {
    use crate::service::{pacer::Pacer, workers::start_mod_channel_worker};
    let f = fixture();
    let (_, sink) = f.register("a", 1);
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(3))));
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
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(4))));
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
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)));
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
        .close(&bid("a", 1), WatchCloseReason::BindingChanged, f.now());
    assert!(
        !f.registry.seat_live(&seat("a")),
        "rebind grace does not suppress wakes or the digest"
    );
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
        .close(&bid("a", 1), WatchCloseReason::BindingChanged, f.now());
    f.advance(MOD_REBIND_GRACE_MS as i64 + 1);
    assert!(!f.registry.seat_live(&seat("a")), "expired rebind grace");
}

#[test]
fn rebind_grace_authorizes_acks_but_is_neither_seat_nor_binding_live() {
    let f = fixture();
    f.register("a", 1);
    f.registry
        .close(&bid("a", 1), WatchCloseReason::BindingChanged, f.now());
    assert!(!f.registry.seat_live(&seat("a")));
    assert!(!f.registry.binding_live(&seat("a"), &bid("a", 1)));
    assert!(
        f.registry.is_live(&seat("a"), &bid("a", 1)),
        "acks stay authorized"
    );
    f.advance(MOD_REBIND_GRACE_MS as i64 - 1);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    f.advance(2);
    assert!(!f.registry.is_live(&seat("a"), &bid("a", 1)), "expired");
}

#[test]
fn registration_landing_between_pass_snapshot_and_close_survives() {
    type Mutate = fn(&mut ModSeatView);
    let cases: [(&str, Mutate); 3] = [
        ("binding ended", |v| {
            v.binding.as_mut().unwrap().ended_at = Some(UtcMillis(9))
        }),
        ("retired", |v| v.retired = true),
        ("unresolved", |v| v.continuity_resolved = false),
    ];
    for (name, mutate) in cases {
        let f = fixture();
        let (_, old_sink) = f.register("a", 1);
        let mut v = view("a", 1, pending(1));
        mutate(&mut v);
        f.set_view(&bid("a", 1), Some(v));
        let new_sink = f.register_during_next_read("a", 1);
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
        assert!(f.registry.is_live(&seat("a"), &bid("a", 1)), "{name}");
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
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.kicks(), 0);
    f.registry
        .close(&bid("a", 1), WatchCloseReason::Retired, f.now());
    f.try_register("a", 1).expect("no cooldown was armed");
}

#[test]
fn ack_landing_during_the_stall_read_prevents_the_stall_close() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    let registry = Arc::clone(&f.registry);
    *f.store.on_read.lock().unwrap() = Some(Box::new(move || {
        registry.record_ack(&seat("a"), &bid("a", 1), now);
    }));
    f.registry.sweep(now);
    assert!(f.registry.is_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.kicks(), 0);
    f.try_register("a", 1).expect("no cooldown was armed");
}

#[test]
fn record_ack_and_attention_push_for_another_binding_are_ignored() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let (f, now) = stall_setup(over, true, Some(0), 10);
    assert!(f.registry.stalled(&seat("a"), now));
    f.registry.record_ack(&seat("a"), &bid("a", 2), now);
    assert!(
        f.registry.stalled(&seat("a"), now),
        "an ack for another binding does not reset the stall clock"
    );
    // The right binding under another seat is not this entry either.
    f.registry.record_ack(&seat("b"), &bid("a", 1), now);
    assert!(f.registry.stalled(&seat("a"), now));
    // A channel that never saw a push is not stalled; a push for another
    // binding must not arm the predicate.
    let f = fixture();
    f.register("a", 1);
    f.store.receipts.lock().unwrap().push((f.now(), 10));
    f.advance(over);
    f.registry
        .record_attention_push(&seat("a"), &bid("a", 2), f.now());
    assert!(!f.registry.stalled(&seat("a"), f.now()));
    f.registry
        .record_attention_push(&seat("a"), &bid("a", 1), f.now());
    assert!(f.registry.stalled(&seat("a"), f.now()));
}

#[test]
fn passive_invitation_drain_does_not_arm_attention_stall_history() {
    let f = fixture();
    let (_, sink) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    f.set_view(&bid("a", 1), Some(view("a", 1, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    let mut changed = view("a", 1, ModFingerprint::default());
    changed.pending_invitation_version = 42;
    f.set_view(&bid("a", 1), Some(changed));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1);
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None
    );
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1);
}

#[test]
fn q3_closed_receiver_preserves_passive_drain_without_an_immediate_kick_loop() {
    struct ClosedReceiverSink {
        sender: tokio::sync::mpsc::UnboundedSender<WatchFrame>,
        attempts: AtomicUsize,
    }
    impl ModChannelSink for ClosedReceiverSink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            self.sender.send(frame).is_ok()
        }
    }
    let f = fixture();
    let worker_kicks = Arc::new(AtomicUsize::new(0));
    let counted_worker = worker_kicks.clone();
    f.registry.set_worker_wake(Arc::new(move || {
        counted_worker.fetch_add(1, Ordering::SeqCst);
    }));
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    drop(receiver);
    let sink = Arc::new(ClosedReceiverSink {
        sender,
        attempts: AtomicUsize::new(0),
    });
    let channel = f
        .registry
        .register(
            ModChannelRegistration {
                mode: ModChannelMode::PresentationV2,
                seat: seat("a"),
                binding: bid("a", 1),
                binding_generation: 1,
                native_session: NativeSessionId::new("native"),
                harness: Harness::Claude,
                idle_wake: IdleWake::Channel,
                registered_at: f.now(),
            },
            sink.clone(),
        )
        .unwrap();
    assert!(
        f.registry.presentation_lease(channel).is_some(),
        "explicit capable fixture"
    );
    assert_eq!(
        worker_kicks.swap(0, Ordering::SeqCst),
        1,
        "registration schedules its initial pass"
    );
    f.registry.take_dirty();
    let kicks = f.kicks();
    let mut changed = view("a", 1, ModFingerprint::default());
    changed.pending_invitation_version = 42;
    f.set_view(&bid("a", 1), Some(changed));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None
    );
    assert!(
        !f.registry.take_dirty(),
        "a dead sink cannot spin the worker immediately"
    );
    assert_eq!(f.kicks(), kicks);
    assert_eq!(
        worker_kicks.load(Ordering::SeqCst),
        0,
        "closed receiver adds no worker kick"
    );
    // The same clock, scope and private version: only failed-enqueue recovery
    // can make the existing safety pass retry this exact channel's drain.
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        sink.attempts.load(Ordering::SeqCst),
        2,
        "false means closed receiver; preserve the pending drain"
    );
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None
    );
    assert!(!f.registry.take_dirty());
    assert_eq!(f.kicks(), kicks);
    assert_eq!(
        worker_kicks.load(Ordering::SeqCst),
        0,
        "closed receiver adds no worker kick"
    );
}

#[test]
fn presentation_owner_lease_is_exact_live_and_replacement_fenced() {
    let f = fixture();
    let a = seat("a");
    let (first, _) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    let old = f
        .registry
        .presentation_lease(first)
        .expect("attached channel gets presentation lease");
    assert!(crate::protocol::delivery::valid_presentation_token(&old));
    let (sibling, _) = f.register_mode("a", 2, ModChannelMode::PresentationV2);
    let sibling_lease = f.registry.presentation_lease(sibling).unwrap();
    assert!(
        !f.registry
            .presentation_owner_live(&a, &bid("a", 2), 1, "native", &old)
    );
    assert!(
        f.registry
            .presentation_owner_live(&a, &bid("a", 1), 1, "native", &old)
    );
    assert!(
        !f.registry
            .presentation_owner_live(&a, &bid("a", 1), 2, "native", &old)
    );
    assert!(
        !f.registry
            .presentation_owner_live(&a, &bid("a", 1), 1, "different", &old)
    );
    let (second, _) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    let new = f
        .registry
        .presentation_lease(second)
        .expect("replacement gets fresh lease");
    assert_ne!(old, new);
    assert!(f.registry.presentation_lease(first).is_none());
    assert!(
        !f.registry
            .presentation_owner_live(&a, &bid("a", 1), 1, "native", &old)
    );
    assert!(
        f.registry
            .presentation_owner_live(&a, &bid("a", 1), 1, "native", &new)
    );
    assert!(
        f.registry
            .presentation_owner_live(&a, &bid("a", 2), 2, "native", &sibling_lease)
    );
    f.registry.unregister(second, f.now());
    assert!(
        f.registry.is_live(&a, &bid("a", 1)),
        "legacy ACK reconnect grace remains"
    );
    assert!(
        !f.registry
            .presentation_owner_live(&a, &bid("a", 1), 1, "native", &new)
    );
    assert!(f.registry.presentation_lease(second).is_none());
}

#[test]
fn presentation_owner_permit_serializes_replacement() {
    let f = fixture();
    let (first, _) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    let old = f.registry.presentation_lease(first).expect("live lease");
    let permit = f
        .registry
        .presentation_owner(&seat("a"))
        .unwrap()
        .expect("registry owner permit");
    let owner = permit.lock().unwrap();
    let (started_rx, done_rx, worker) = {
        let registry = f.registry.clone();
        let now = f.now();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = registry.register(
                ModChannelRegistration {
                    mode: ModChannelMode::PresentationV2,
                    seat: seat("a"),
                    binding: bid("a", 1),
                    binding_generation: 1,
                    native_session: NativeSessionId::new("native"),
                    harness: Harness::Claude,
                    idle_wake: IdleWake::Channel,
                    registered_at: now,
                },
                Arc::new(RecordingSink::default()),
            );
            done_tx.send(result).unwrap();
        });
        (started_rx, done_rx, worker)
    };
    started_rx.recv().unwrap();
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err(),
        "replacement cannot cross an active allocation/confirm permit"
    );
    assert!(
        f.registry
            .presentation_owner_live(&seat("a"), &bid("a", 1), 1, "native", &old)
    );
    drop(owner);
    let second = done_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert_ne!(first, second);
    assert!(
        !f.registry
            .presentation_owner_live(&seat("a"), &bid("a", 1), 1, "native", &old)
    );
}

#[test]
fn presentation_attention_cutoff_requires_exact_live_owner() {
    let f = fixture();
    let a = seat("a");
    let (first, _) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    let old = f.registry.presentation_lease(first).unwrap();
    for (binding, generation, native, lease) in [
        (bid("a", 2), 1, "native", old.as_str()),
        (bid("a", 1), 2, "native", old.as_str()),
        (bid("a", 1), 1, "wrong", old.as_str()),
        (bid("a", 1), 1, "native", "wrong"),
    ] {
        f.registry.record_presentation_attention_push(
            &a,
            &binding,
            generation,
            native,
            lease,
            UtcMillis(100),
        );
        assert_eq!(
            f.registry.lock().entries[&bid("a", 1)].last_attention_push,
            None
        );
    }
    f.registry.record_presentation_attention_push(
        &a,
        &bid("a", 1),
        1,
        "native",
        &old,
        UtcMillis(101),
    );
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        Some(UtcMillis(101))
    );
    let (second, _) = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    let new = f.registry.presentation_lease(second).unwrap();
    f.registry.record_presentation_attention_push(
        &a,
        &bid("a", 1),
        1,
        "native",
        &old,
        UtcMillis(102),
    );
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None,
        "replacement rejects previous lease"
    );
    f.registry.unregister(second, f.now());
    f.registry.record_presentation_attention_push(
        &a,
        &bid("a", 1),
        1,
        "native",
        &new,
        UtcMillis(103),
    );
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None,
        "reconnect grace grants no presentation health evidence"
    );
    struct Inert;
    impl ModChannels for Inert {}
    Inert.record_presentation_attention_push(&a, &bid("a", 1), 1, "native", &new, UtcMillis(104));
    assert!(!Inert.stalled(&a, UtcMillis(104)));
}

#[test]
fn two_claude_bindings_hold_two_live_channels() {
    let f = fixture();
    let (first, _) = f.register("a", 1);
    let (second, _) = f.register("a", 2);
    assert_ne!(first, second);
    assert!(f.registry.binding_live(&seat("a"), &bid("a", 1)));
    assert!(f.registry.binding_live(&seat("a"), &bid("a", 2)));
    assert!(!f.registry.binding_live(&seat("a"), &bid("a", 3)));
    assert!(!f.registry.binding_live(&seat("b"), &bid("a", 1)));
    assert!(f.registry.seat_live(&seat("a")));
    let status = f.registry.status().unwrap();
    assert_eq!(status.live_channels, 2);
    assert_eq!(status.channels.len(), 2);
    assert!(status.channels.iter().all(|c| c.seat == seat("a")));
    assert_eq!(f.kicks(), 0);
}

#[test]
fn each_channel_receives_only_its_bindings_deliveries() {
    let f = fixture();
    let (_, a) = f.register("a", 1);
    let (_, b) = f.register("a", 2);
    // Binding 1 has mail; binding 2 has none.
    f.set_view(&bid("a", 1), Some(view("a", 1, pending(3))));
    f.set_view(&bid("a", 2), Some(view("a", 2, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(a.frames(), vec![WatchFrame::Attention { version: 3 }]);
    assert!(b.frames().is_empty(), "binding 2 has nothing pending");
    // Binding 2's own unread arrives: only its channel is pushed.
    let unread = ModFingerprint {
        attention_version: 3,
        unread: 1,
        max_unread_rowid: 9,
        ..ModFingerprint::default()
    };
    f.set_view(&bid("a", 2), Some(view("a", 2, unread)));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(a.frames().len(), 1, "binding 1's fingerprint did not move");
    assert_eq!(b.frames(), vec![WatchFrame::Attention { version: 3 }]);
    // The seat-level notify reaches every live channel of the seat.
    f.registry.notify(&seat("a"), 8);
    assert_eq!(
        a.frames().last(),
        Some(&WatchFrame::Attention { version: 8 })
    );
    assert_eq!(
        b.frames().last(),
        Some(&WatchFrame::Attention { version: 8 })
    );
}

#[test]
fn closing_one_channel_leaves_the_other_live() {
    for reason in [
        WatchCloseReason::Retired,
        WatchCloseReason::Unresolved,
        WatchCloseReason::Stopping,
    ] {
        let f = fixture();
        let (_, a) = f.register("a", 1);
        let (_, b) = f.register("a", 2);
        f.registry.close(&bid("a", 1), reason, f.now());
        assert_eq!(a.frames(), vec![WatchFrame::Close { reason }], "{reason:?}");
        assert!(b.frames().is_empty(), "{reason:?}");
        assert!(
            !f.registry.binding_live(&seat("a"), &bid("a", 1)),
            "{reason:?}"
        );
        assert!(
            f.registry.binding_live(&seat("a"), &bid("a", 2)),
            "{reason:?}"
        );
        assert!(f.registry.seat_live(&seat("a")), "{reason:?}");
        // The remaining channel still gets its deliveries.
        f.set_view(&bid("a", 2), Some(view("a", 2, pending(2))));
        f.registry.pass(f.now()).unwrap();
        assert_eq!(
            b.frames(),
            vec![WatchFrame::Attention { version: 2 }],
            "{reason:?}"
        );
        f.registry.close(&bid("a", 2), reason, f.now());
        assert!(!f.registry.seat_live(&seat("a")), "{reason:?}");
    }
}

#[test]
fn re_registering_a_binding_replaces_only_its_own_channel() {
    let f = fixture();
    let (first, old) = f.register("a", 1);
    let (_, sibling) = f.register("a", 2);
    let (replacement, new) = f.register("a", 1);
    assert_ne!(first, replacement);
    assert_eq!(
        old.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Replaced
        }]
    );
    assert!(
        sibling.frames().is_empty(),
        "the other binding was not replaced"
    );
    assert!(new.frames().is_empty());
    assert_eq!(f.registry.status().unwrap().channels.len(), 2);
    // The replaced connection ending afterwards disturbs neither live channel.
    f.registry.unregister(first, f.now());
    assert!(f.registry.binding_live(&seat("a"), &bid("a", 1)));
    assert_eq!(f.registry.status().unwrap().live_channels, 2);
    assert_eq!(f.kicks(), 0);
}

#[test]
fn stall_cooldown_is_per_binding() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let f = fixture();
    let registered = f.now();
    let (_, stalled_sink) = f.register("a", 1);
    let (_, healthy_sink) = f.register("a", 2);
    f.store.receipts.lock().unwrap().push((registered, 10));
    f.registry
        .record_attention_push(&seat("a"), &bid("a", 1), registered);
    f.registry
        .record_attention_push(&seat("a"), &bid("a", 2), registered);
    f.advance(over);
    // Binding 2 acked just now; binding 1 never did.
    f.registry.record_ack(&seat("a"), &bid("a", 2), f.now());
    assert!(
        f.registry
            .binding_stalled(&seat("a"), &bid("a", 1), f.now())
    );
    assert!(
        !f.registry
            .binding_stalled(&seat("a"), &bid("a", 2), f.now())
    );
    assert!(
        !f.registry.stalled(&seat("a"), f.now()),
        "the seat is stalled only when every one of its channels is"
    );
    f.registry.sweep(f.now());
    assert_eq!(
        stalled_sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Stalled
        }]
    );
    assert!(healthy_sink.frames().is_empty());
    assert!(f.registry.binding_live(&seat("a"), &bid("a", 2)));
    assert_eq!(f.kicks(), 1);
    // The cooldown is binding 1's alone.
    assert_eq!(
        f.try_register("a", 1).err(),
        Some(WatchRefusalReason::Cooldown)
    );
    f.try_register("a", 3)
        .expect("another binding of the seat registers");
    f.try_register("a", 2)
        .expect("the healthy binding re-registers");
    f.advance(MOD_STALL_COOLDOWN_MS as i64 + 1);
    f.try_register("a", 1).expect("the cooldown expired");
}

#[test]
fn seat_stalled_needs_every_channel_stalled() {
    let over = MOD_STALL_AFTER_MS as i64 + 1;
    let f = fixture();
    let registered = f.now();
    f.register("a", 1);
    f.register("a", 2);
    f.store.receipts.lock().unwrap().push((registered, 10));
    for generation in [1, 2] {
        f.registry
            .record_attention_push(&seat("a"), &bid("a", generation), registered);
    }
    f.advance(over);
    assert!(f.registry.stalled(&seat("a"), f.now()));
    f.registry.record_ack(&seat("a"), &bid("a", 2), f.now());
    assert!(!f.registry.stalled(&seat("a"), f.now()));
}

fn q3_replace_at_final_push_gap(
    active: bool,
    old_mode: ModChannelMode,
    replacement_mode: ModChannelMode,
) {
    let f = fixture();
    let (old_channel, old_sink) = f.register_mode("a", 1, old_mode);
    let mut old_view = view(
        "a",
        1,
        if active {
            pending(3)
        } else {
            ModFingerprint::default()
        },
    );
    old_view.pending_invitation_version = if active { 0 } else { 42 };
    f.set_view(&bid("a", 1), Some(old_view));
    let replacement = Arc::new(Mutex::new(None));
    let captured = replacement.clone();
    let weak_registry = Arc::downgrade(&f.registry);
    let now = f.now();
    *f.registry.before_final_push.lock().unwrap() =
        Some(Box::new(move |binding, judged_channel, is_active| {
            assert_eq!(binding, &bid("a", 1));
            assert_eq!(judged_channel, old_channel);
            assert_eq!(is_active, active);
            let registry = weak_registry.upgrade().unwrap();
            {
                // Fail immediately if the seam accidentally runs under the first
                // guard. The synchronous callback is a deterministic scheduling
                // boundary, so no racing lock acquisition or sleep is needed.
                let state = registry
                    .state
                    .try_lock()
                    .expect("first observation guard must be dropped");
                let observed = &state.entries[binding];
                assert_eq!(observed.channel, Some(old_channel));
                assert_eq!(observed.mode, old_mode);
                assert!(
                    observed.last_fingerprint.is_some(),
                    "the real observation completed before replacement"
                );
            }
            let sink = Arc::new(RecordingSink::default());
            let current = registry
                .register(
                    ModChannelRegistration {
                        mode: replacement_mode,
                        seat: seat("a"),
                        binding: binding.clone(),
                        binding_generation: 1,
                        native_session: NativeSessionId::new("native"),
                        harness: Harness::Claude,
                        idle_wake: IdleWake::Channel,
                        registered_at: now,
                    },
                    sink.clone(),
                )
                .unwrap();
            *captured.lock().unwrap() = Some((current, sink));
        }));
    // Runs the actual store read, observation, callback and final push path.
    f.registry.pass(now).unwrap();
    let (current, sink) = replacement
        .lock()
        .unwrap()
        .take()
        .expect("real gap callback ran");
    assert_ne!(old_channel, current);
    assert_eq!(
        old_sink.frames(),
        vec![WatchFrame::Close {
            reason: WatchCloseReason::Replaced
        }]
    );
    let state = f.registry.lock();
    let entry = &state.entries[&bid("a", 1)];
    assert_eq!(entry.channel, Some(current));
    assert_eq!(entry.mode, replacement_mode);
    assert!(!entry.pending_delivery_recheck);
    assert_eq!(
        (
            sink.frames(),
            entry.last_attention_push,
            entry.last_fingerprint.is_none(),
            entry.last_invitation_version
        ),
        (vec![], None, true, None),
        "a replaced channel must receive no old frame, health or observation state"
    );
}

#[test]
fn q3_replacement_after_observation_cannot_receive_the_old_active_push() {
    q3_replace_at_final_push_gap(true, ModChannelMode::Legacy, ModChannelMode::PresentationV2);
}

#[test]
fn q3_replacement_after_observation_cannot_receive_the_old_passive_push() {
    q3_replace_at_final_push_gap(
        false,
        ModChannelMode::PresentationV2,
        ModChannelMode::Legacy,
    );
}

#[test]
fn q3_capable_to_legacy_replacement_cannot_receive_the_old_active_push() {
    q3_replace_at_final_push_gap(true, ModChannelMode::PresentationV2, ModChannelMode::Legacy);
}

#[test]
fn q3_legacy_to_capable_replacement_cannot_receive_the_old_passive_push() {
    q3_replace_at_final_push_gap(
        false,
        ModChannelMode::Legacy,
        ModChannelMode::PresentationV2,
    );
}

fn q3_worker_counter(f: &Fixture) -> Arc<AtomicUsize> {
    let counter = Arc::new(AtomicUsize::new(0));
    let counted = counter.clone();
    f.registry.set_worker_wake(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    counter
}

fn q3_quiet_capable(f: &Fixture) -> (ModChannelId, Arc<RecordingSink>) {
    let owner = f.register_mode("a", 1, ModChannelMode::PresentationV2);
    f.set_view(&bid("a", 1), Some(view("a", 1, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    assert!(owner.1.frames().is_empty());
    f.registry.take_dirty();
    owner
}

#[test]
fn q3_passive_hints_coalesce_without_health_or_crossing_legacy_and_seat_scope() {
    let f = fixture();
    let worker = q3_worker_counter(&f);
    let (owner, sink) = q3_quiet_capable(&f);
    let (_, legacy) = f.register("a", 2);
    f.set_view(&bid("a", 2), Some(view("a", 2, ModFingerprint::default())));
    let (_, foreign) = f.register_mode("b", 1, ModChannelMode::PresentationV2);
    f.set_view(&bid("b", 1), Some(view("b", 1, ModFingerprint::default())));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(worker.swap(0, Ordering::SeqCst), 3);
    f.registry.take_dirty();
    let health = f.registry.lock().entries[&bid("a", 1)].last_ack_or_registration;
    let native_kicks = f.kicks();
    f.registry
        .presentation_delivery_may_have_changed(&seat("unknown"));
    assert_eq!(worker.load(Ordering::SeqCst), 0);
    f.registry
        .presentation_delivery_may_have_changed(&seat("a"));
    assert_eq!(worker.load(Ordering::SeqCst), 1);
    f.registry
        .presentation_delivery_may_have_changed(&seat("a"));
    assert_eq!(
        worker.load(Ordering::SeqCst),
        2,
        "one worker callback per explicit hint; pending work is one bit"
    );
    assert!(f.registry.take_dirty());
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 0 }]);
    assert!(legacy.frames().is_empty());
    assert!(foreign.frames().is_empty());
    {
        let state = f.registry.lock();
        let entry = &state.entries[&bid("a", 1)];
        assert_eq!(entry.channel, Some(owner));
        assert!(!entry.pending_delivery_recheck);
        assert_eq!(entry.last_attention_push, None);
        assert_eq!(entry.last_ack_or_registration, health);
        assert!(!state.entries[&bid("a", 2)].pending_delivery_recheck);
        assert!(!state.entries[&bid("b", 1)].pending_delivery_recheck);
    }
    f.registry.pass(f.now()).unwrap();
    assert_eq!(
        sink.frames().len(),
        1,
        "fixed eligibility duplicate safety pass is quiet"
    );
    assert_eq!(
        worker.load(Ordering::SeqCst),
        2,
        "consumption adds no worker callback"
    );
    assert_eq!(f.kicks(), native_kicks);
    assert!(!f.registry.take_dirty());
}

#[test]
fn q3_second_hint_during_store_work_survives_the_first_successful_pass() {
    let f = fixture();
    let worker = q3_worker_counter(&f);
    let (owner, sink) = q3_quiet_capable(&f);
    assert_eq!(worker.swap(0, Ordering::SeqCst), 1);
    f.registry
        .presentation_delivery_may_have_changed(&seat("a"));
    f.registry.take_dirty();
    let weak = Arc::downgrade(&f.registry);
    *f.store.on_read.lock().unwrap() = Some(Box::new(move || {
        let registry = weak.upgrade().unwrap();
        {
            let state = registry.lock();
            let entry = &state.entries[&bid("a", 1)];
            assert_eq!(entry.channel, Some(owner));
            assert!(
                !entry.pending_delivery_recheck,
                "the first hint was taken before this real registry store call"
            );
        }
        registry.presentation_delivery_may_have_changed(&seat("a"));
    }));
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 1);
    assert!(
        f.registry.lock().entries[&bid("a", 1)].pending_delivery_recheck,
        "success cannot clear the newer event"
    );
    assert!(f.registry.take_dirty());
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 2);
    assert!(!f.registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames().len(), 2);
    assert_eq!(worker.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None
    );
    assert_eq!(f.kicks(), 0);
}

fn q3_failed_hint_work(replacement_mode: Option<ModChannelMode>) {
    let f = fixture();
    let worker = q3_worker_counter(&f);
    let (old, old_sink) = q3_quiet_capable(&f);
    assert_eq!(worker.swap(0, Ordering::SeqCst), 1);
    f.registry
        .presentation_delivery_may_have_changed(&seat("a"));
    f.registry.take_dirty();
    let replacement = Arc::new(Mutex::new(None));
    if let Some(mode) = replacement_mode {
        let weak = Arc::downgrade(&f.registry);
        let replaced = replacement.clone();
        let now = f.now();
        *f.store.on_read.lock().unwrap() = Some(Box::new(move || {
            let registry = weak.upgrade().unwrap();
            assert!(!registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
            let sink = Arc::new(RecordingSink::default());
            let id = registry
                .register(
                    ModChannelRegistration {
                        mode,
                        seat: seat("a"),
                        binding: bid("a", 1),
                        binding_generation: 1,
                        native_session: NativeSessionId::new("native"),
                        harness: Harness::Claude,
                        idle_wake: IdleWake::Channel,
                        registered_at: now,
                    },
                    sink.clone(),
                )
                .unwrap();
            *replaced.lock().unwrap() = Some((id, sink));
        }));
    }
    f.store.fail_views.store(true, Ordering::SeqCst);
    assert_eq!(
        f.registry.pass(f.now()).unwrap_err().code,
        ErrorCode::StoreBusy
    );
    let expected_worker = if replacement_mode.is_some() { 2 } else { 1 };
    assert_eq!(
        worker.load(Ordering::SeqCst),
        expected_worker,
        "only the original hint and actual replacement schedule work"
    );
    {
        let state = f.registry.lock();
        let entry = &state.entries[&bid("a", 1)];
        assert_eq!(entry.last_attention_push, None);
        if let Some(mode) = replacement_mode {
            let saved = replacement.lock().unwrap();
            let (id, sink) = saved.as_ref().unwrap();
            assert_ne!(*id, old);
            assert_eq!(entry.channel, Some(*id));
            assert_eq!(entry.mode, mode);
            assert!(
                !entry.pending_delivery_recheck,
                "failed old work cannot transfer to a replacement"
            );
            assert!(entry.last_fingerprint.is_none());
            assert_eq!(entry.last_invitation_version, None);
            assert!(sink.frames().is_empty());
            assert_eq!(
                old_sink.frames(),
                vec![WatchFrame::Close {
                    reason: WatchCloseReason::Replaced
                }]
            );
        } else {
            assert_eq!(entry.channel, Some(old));
            assert!(
                entry.pending_delivery_recheck,
                "same exact owner retains the failed captured hint"
            );
            assert!(old_sink.frames().is_empty());
        }
    }
    f.store.fail_views.store(false, Ordering::SeqCst);
    f.registry.pass(f.now()).unwrap();
    if replacement_mode.is_none() {
        assert_eq!(
            old_sink.frames(),
            vec![WatchFrame::Attention { version: 0 }]
        );
        assert!(!f.registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
    } else {
        assert!(
            replacement
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .1
                .frames()
                .is_empty()
        );
    }
    assert_eq!(f.kicks(), 0);
}

#[test]
fn q3_failed_hint_work_restores_the_same_exact_owner() {
    q3_failed_hint_work(None);
}

#[test]
fn q3_failed_store_work_keeps_a_newer_hint_without_a_captured_hint() {
    let f = fixture();
    let worker = q3_worker_counter(&f);
    let (_, sink) = q3_quiet_capable(&f);
    assert_eq!(worker.swap(0, Ordering::SeqCst), 1);
    let weak = Arc::downgrade(&f.registry);
    *f.store.on_read.lock().unwrap() = Some(Box::new(move || {
        let registry = weak.upgrade().unwrap();
        assert!(!registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
        registry.presentation_delivery_may_have_changed(&seat("a"));
    }));
    f.store.fail_views.store(true, Ordering::SeqCst);
    assert_eq!(
        f.registry.pass(f.now()).unwrap_err().code,
        ErrorCode::StoreBusy
    );
    assert!(f.registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
    assert!(sink.frames().is_empty());
    assert_eq!(worker.load(Ordering::SeqCst), 1);
    f.store.fail_views.store(false, Ordering::SeqCst);
    f.registry.pass(f.now()).unwrap();
    assert_eq!(sink.frames(), vec![WatchFrame::Attention { version: 0 }]);
    assert!(!f.registry.lock().entries[&bid("a", 1)].pending_delivery_recheck);
    assert_eq!(
        f.registry.lock().entries[&bid("a", 1)].last_attention_push,
        None
    );
    assert_eq!(worker.load(Ordering::SeqCst), 1);
    assert_eq!(f.kicks(), 0);
}

#[test]
fn q3_failed_hint_work_does_not_transfer_to_either_replacement_mode() {
    q3_failed_hint_work(Some(ModChannelMode::PresentationV2));
    q3_failed_hint_work(Some(ModChannelMode::Legacy));
}
