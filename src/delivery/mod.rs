//! Delivery assembly shared by every consumer of the inbox: chunk reassembly,
//! body truncation, name labels, the page drain loop and the D4 text frame
//! (delivery-policy spec DP2). Nothing here names a harness.

pub mod assemble;
pub mod frame;
pub mod page;

pub use assemble::{
    AssembleError, Assembled, Assembler, DeliveryItem, DeliveryKind, read_truncated_hint,
    record_truncated,
};
pub use frame::{FrameItem, HEADER, frame};
pub use page::{
    BODY_LIMIT_BYTES, DrainEvent, DrainPagesError, MAX_DRAIN_PAGES, PAGE_MAX_BYTES, PAGE_MAX_ITEMS,
    PageError, PageSource, Stop, drain_pages,
};
