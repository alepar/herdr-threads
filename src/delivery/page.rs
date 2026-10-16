//! The page drain loop, moved verbatim from the `watch` CLI (delivery-policy
//! spec DP2): page `InboxBatchV2` for a seat until exhausted, feed every item
//! to an [`Assembler`] and report what came out as [`DrainEvent`]s. Each page is
//! emitted (sorted within the page) as it arrives, so a sink's `Halt` bounds the
//! store reads.

use super::assemble::{
    AssembleError, Assembled, Assembler, DeliveryInvitation, DeliveryItem, batch_order,
    invitation_order_key, item_order_key,
};
use crate::protocol::{pagination::Page, results::InboxBatchV2Item};

pub use crate::protocol::watch::{
    WATCH_BODY_LIMIT_BYTES as BODY_LIMIT_BYTES, WATCH_PAGE_MAX_BYTES as PAGE_MAX_BYTES,
    WATCH_PAGE_MAX_ITEMS as PAGE_MAX_ITEMS,
};

/// Upper bound on pages followed in one drain (a cursor loop guard).
pub const MAX_DRAIN_PAGES: usize = 4096;

/// Why a page could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    /// The daemon refused or failed the read.
    Api,
    /// The reply was not an `InboxBatchV2` page.
    Protocol,
}

/// One page of the seat's inbox per call, following the cursor of the last page.
pub trait PageSource {
    fn page(&mut self, cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError>;
}

/// What a drain reports to its sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainEvent {
    /// A message completed.
    Item(DeliveryItem),
    Invitation(DeliveryInvitation),
    /// The seat needs attention at `version`; reported once per version.
    Attention {
        version: u64,
    },
    /// Everything the last attention pointed at was settled elsewhere.
    AttentionCleared {
        version: u64,
    },
    /// The last page was read.
    Exhausted,
}

/// One entry of a page's ordered batch.
enum Batched {
    Item(DeliveryItem),
    Invitation(DeliveryInvitation),
}

/// A sink's verdict on one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Continue,
    /// End the drain now, successfully.
    Halt,
}

#[derive(Debug)]
pub enum DrainPagesError<E> {
    Page(PageError),
    /// The daemon's chunks broke the paging contract.
    Assemble(AssembleError),
    /// The pages never ended within `max_pages`.
    TooManyPages,
    Sink(E),
}

/// Page until exhausted and report one event per completed message. Invitations
/// and warnings that wake the seat collapse into one attention event per
/// `frame_version`; informational warnings raise none.
pub fn drain_pages<E>(
    source: &mut dyn PageSource,
    assembler: &mut Assembler,
    frame_version: u64,
    max_pages: usize,
    mut sink: impl FnMut(DrainEvent) -> Result<Stop, E>,
) -> Result<(), DrainPagesError<E>> {
    macro_rules! emit {
        ($event:expr) => {
            match sink($event) {
                Ok(Stop::Continue) => {}
                Ok(Stop::Halt) => return Ok(()),
                Err(error) => return Err(DrainPagesError::Sink(error)),
            }
        };
    }
    let mut cursor: Option<String> = None;
    // Chunks buffered by an earlier, failed drain do not carry over.
    assembler.partial.clear();
    let mut saw_attention = false;
    for _ in 0..max_pages {
        let page = source.page(cursor.take()).map_err(DrainPagesError::Page)?;
        // Each page goes out as it arrives, sorted within the page only (the inbox merge already
        // orders across pages), so a sink's `Halt` stops before the next page is fetched.
        // Invitations ride the same batch, keyed p0 plus their deadline.
        let mut batch: Vec<Batched> = Vec::new();
        for item in page.items {
            match assembler.push(item).map_err(DrainPagesError::Assemble)? {
                Assembled::Item(item) => batch.push(Batched::Item(item)),
                Assembled::Invitation(item) => {
                    saw_attention |= item.content.wakes;
                    batch.push(Batched::Invitation(item));
                }
                Assembled::Attention => saw_attention = true,
                Assembled::Pending | Assembled::InformationalNotice | Assembled::Duplicate => {}
            }
        }
        batch_order(&mut batch, |entry| match entry {
            Batched::Item(item) => item_order_key(item),
            Batched::Invitation(item) => invitation_order_key(item),
        });
        for entry in batch {
            match entry {
                Batched::Item(item) => emit!(DrainEvent::Item(item)),
                Batched::Invitation(item) => emit!(DrainEvent::Invitation(item)),
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => {
                if saw_attention {
                    let id = format!("attention:{frame_version}");
                    if assembler.emitted.insert(id) {
                        emit!(DrainEvent::Attention {
                            version: frame_version
                        });
                    }
                    assembler.attention_shown = true;
                } else if assembler.attention_shown {
                    // Everything the last attention line pointed at was settled
                    // elsewhere (an accepted invitation, notices a check-in
                    // offered): retract it (ht-j16.33).
                    emit!(DrainEvent::AttentionCleared {
                        version: frame_version
                    });
                    assembler.attention_shown = false;
                    assembler.emitted.retain(|id| !id.starts_with("attention:"));
                }
                emit!(DrainEvent::Exhausted);
                return Ok(());
            }
        }
    }
    Err(DrainPagesError::TooManyPages)
}

#[cfg(test)]
#[path = "../../tests/delivery/extraction.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/delivery/promotion.rs"]
mod promotion_tests;
