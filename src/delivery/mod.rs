//! Delivery assembly shared by every consumer of the inbox: chunk reassembly,
//! body truncation, name labels, the page drain loop and the D4 text frame
//! (delivery-policy spec DP2). Nothing here names a harness.

pub mod assemble;
pub mod frame;
pub mod invitation;
pub mod ledger;
pub mod page;
pub mod presentation;
pub mod routing;
pub mod settle;

pub use assemble::{
    AssembleError, Assembled, Assembler, DeliveryItem, DeliveryKind, read_truncated_hint,
    record_truncated,
};
pub use frame::{FrameItem, HEADER, frame};
pub use page::{
    BODY_LIMIT_BYTES, DrainEvent, DrainPagesError, MAX_DRAIN_PAGES, PAGE_MAX_BYTES, PAGE_MAX_ITEMS,
    PageError, PageSource, Stop, drain_pages,
};

/// Whether the adapter of `harness_id` declares a delivery policy of `mechanism`.
/// `"human"` and unknown ids declare nothing. Core code decides delivery by this
/// lookup against the canonical binding's harness id, never by naming a harness.
pub fn declares(harness_id: &str, mechanism: crate::harness::adapter::DeliveryMechanism) -> bool {
    crate::harness::registry::builtins()
        .delivery_policy(harness_id)
        .is_some_and(|policy| policy.mechanism() == mechanism)
}
