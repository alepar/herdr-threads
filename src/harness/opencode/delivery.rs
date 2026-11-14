//! opencode's delivery declaration: the herdr-threads plugin runs `watch` as a push channel,
//! appends each page to the bound session as a stored message and wakes an idle session itself
//! with an in-process prompt (opencode design D1, D4). No composer is ever typed into.
use crate::harness::adapter::{DeliveryMechanism, DeliveryPolicy, IdleWake};
use crate::protocol::authority::COOPERATIVE_MOD_DELIVERY_PROVENANCE;

pub(super) struct OpencodeDelivery;

impl DeliveryPolicy for OpencodeDelivery {
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

#[cfg(test)]
#[path = "../../../tests/harness/opencode_delivery.rs"]
mod tests;
