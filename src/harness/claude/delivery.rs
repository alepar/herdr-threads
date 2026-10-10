//! Claude's delivery declaration: the bundled mod pushes pending items over a
//! channel and wakes an idle session itself (delivery-policy DP3).
use crate::harness::adapter::{DeliveryMechanism, DeliveryPolicy, IdleWake};
use crate::protocol::authority::COOPERATIVE_MOD_DELIVERY_PROVENANCE;

pub(super) struct ClaudeDelivery;

impl DeliveryPolicy for ClaudeDelivery {
    fn mechanism(&self) -> DeliveryMechanism {
        DeliveryMechanism::PushChannel
    }
    fn provenance(&self) -> &'static str {
        COOPERATIVE_MOD_DELIVERY_PROVENANCE
    }
    fn idle_wake(&self) -> IdleWake {
        IdleWake::Channel
    }
}
