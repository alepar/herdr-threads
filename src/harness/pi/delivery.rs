//! Channel-only delivery and wake; the native extension observes context and current branch.
use crate::harness::adapter::{DeliveryMechanism, DeliveryPolicy, IdleWake};
use crate::protocol::authority::COOPERATIVE_MOD_DELIVERY_PROVENANCE;

pub(super) struct PiDelivery;

impl DeliveryPolicy for PiDelivery {
    fn mechanism(&self) -> DeliveryMechanism {
        DeliveryMechanism::PushChannel
    }
    fn provenance(&self) -> &'static str {
        COOPERATIVE_MOD_DELIVERY_PROVENANCE
    }
    fn idle_wake(&self) -> IdleWake {
        IdleWake::Channel
    }
    /// The plugin's wake prompt is content-free: the append before it carries the items.
    fn wake_text(&self) -> &'static str {
        crate::notification::policy::WAKE_UP
    }
}
