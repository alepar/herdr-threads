//! Channel delivery with guarded host wake; receipts require context and current branch.
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
        IdleWake::Native
    }
    /// The native prompt is content-free; the extension supplies mail at the turn boundary.
    fn wake_text(&self) -> &'static str {
        crate::notification::policy::WAKE_UP
    }
}
