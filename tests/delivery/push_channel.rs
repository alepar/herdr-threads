//! The Claude push-channel re-fit (delivery-policy DP3, DP7, DP10): the neutral
//! `NO_PUSH_CHANNEL` constants keep the frozen wire spelling, the `watch
//! --harness` list is registry-derived and still `["claude"]`, and an idle
//! wake is suppressed only for a channel whose policy says `Channel`.
use super::*;
use crate::ports::{ModSeatView, ModStoreReads};
use crate::{
    harness::{
        adapter::{DeliveryMechanism, IdleWake},
        registry::builtins,
    },
    protocol::{
        ids::NativeSessionId,
        results::ApiError,
        time::MonoInstant,
        watch::{WatchRefusalReason, WatchStatusReason},
    },
};
use std::sync::atomic::AtomicI64;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}
struct NoReads;
impl ModStoreReads for NoReads {
    fn mod_binding_view(
        &self,
        _seat: &SeatId,
        _binding: crate::ports::ModBindingRef<'_>,
        _budget: &CallBudget,
    ) -> Result<Option<ModSeatView>, ApiError> {
        Ok(None)
    }
    fn mod_stall_oldest(
        &self,
        _seat: &SeatId,
        _at_or_before: UtcMillis,
        _body_limit: usize,
        _budget: &CallBudget,
    ) -> Result<Option<UtcMillis>, ApiError> {
        Ok(None)
    }
}
struct NullSink;
impl ModChannelSink for NullSink {
    fn push(&self, _frame: WatchFrame) -> bool {
        true
    }
}

fn registry(clock: &Arc<TestClock>) -> ModChannelRegistry {
    ModChannelRegistry::new(
        clock.clone(),
        Arc::new(NoReads),
        "instance".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    )
}
/// One binding per seat in these tests.
fn binding_of(seat: &str) -> crate::protocol::ids::BindingId {
    crate::protocol::ids::BindingId::new(format!("binding-{seat}"))
}
fn register(registry: &ModChannelRegistry, seat: &str, idle_wake: IdleWake) {
    registry
        .register(
            ModChannelRegistration {
                seat: SeatId::new(seat),
                binding: binding_of(seat),
                binding_generation: 1,
                native_session: NativeSessionId::new("native"),
                harness: Harness::Claude,
                idle_wake,
                registered_at: UtcMillis(1_000),
            },
            Arc::new(NullSink),
        )
        .unwrap();
}

#[test]
fn no_push_channel_keeps_the_frozen_not_claude_wire_spelling() {
    assert_eq!(
        serde_json::to_string(&WatchRefusalReason::NO_PUSH_CHANNEL).unwrap(),
        "\"not_claude\""
    );
    assert_eq!(
        serde_json::to_string(&WatchStatusReason::NO_PUSH_CHANNEL).unwrap(),
        "\"not_claude\""
    );
    // A refusal still maps onto the same status reason.
    assert_eq!(
        WatchStatusReason::from(WatchRefusalReason::NO_PUSH_CHANNEL),
        WatchStatusReason::NO_PUSH_CHANNEL
    );
}

#[test]
fn watch_harness_possible_values_are_exactly_claude() {
    let command = crate::cli::commands::command_for_registry(builtins());
    let watch = command.find_subcommand("watch").expect("watch subcommand");
    let arg = watch
        .get_arguments()
        .find(|arg| arg.get_long() == Some("harness"))
        .expect("--harness");
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .map(|value| value.get_name().to_owned())
        .collect();
    assert_eq!(values, ["claude"]);
}

#[test]
fn only_claude_declares_the_push_channel_policy() {
    let policy = builtins().delivery_policy("claude").expect("claude policy");
    assert_eq!(policy.mechanism(), DeliveryMechanism::PushChannel);
    assert_eq!(
        policy.provenance(),
        crate::protocol::authority::COOPERATIVE_MOD_DELIVERY_PROVENANCE
    );
    assert_eq!(policy.provenance(), "cooperative_mod_delivery");
    assert_eq!(policy.idle_wake(), IdleWake::Channel);
    assert!(builtins().delivery_policy("codex").is_none());
    assert!(builtins().delivery_policy("hermes").is_none());
}

#[test]
fn wake_suppressed_follows_the_registered_idle_wake() {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let registry = registry(&clock);
    let native = SeatId::new("native-seat");
    let channel = SeatId::new("channel-seat");
    let absent = SeatId::new("absent-seat");
    register(&registry, "native-seat", IdleWake::Native);
    register(&registry, "channel-seat", IdleWake::Channel);
    // Both channels are live, but only a `Channel` policy suppresses native wake.
    assert!(registry.seat_live(&native));
    assert!(!registry.wake_suppressed(&native));
    assert!(registry.seat_live(&channel));
    assert!(registry.wake_suppressed(&channel));
    assert!(!registry.wake_suppressed(&absent));
}

#[test]
fn wake_suppressed_ends_when_the_channel_grace_expires() {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let registry = registry(&clock);
    let seat = SeatId::new("channel-seat");
    register(&registry, "channel-seat", IdleWake::Channel);
    let channel = registry.status().unwrap().channels.len();
    assert_eq!(channel, 1);
    registry.close(
        &binding_of("channel-seat"),
        WatchCloseReason::Retired,
        UtcMillis(1_000),
    );
    assert!(!registry.wake_suppressed(&seat));
}
