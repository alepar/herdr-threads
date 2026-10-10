//! The pure per-id settlement decision shared by every delivery mechanism
//! (delivery-policy DP3, ported byte for byte from the store's
//! `plan_mod_ack`). The store reads the facts in SQL; this module only decides.

use crate::protocol::watch::{ModAckOutcome, ModAckReason, WATCH_BODY_LIMIT_BYTES};

/// The seat's lazy completion state of an id (`lazy_delivery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyFact {
    /// Published and addressed to the seat; display not yet completed.
    Pending,
    /// Published, addressed, and already completed as displayed.
    Completed,
    /// Not a published row addressed to the seat in this instance.
    NotAddressed,
}

/// The seat's effective receipt state of an ordinary id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptFact {
    Pending,
    Acknowledged,
    RecipientRetired,
    NotRequired,
}

/// How the id was recorded as delivered: a lazy row or an ordinary receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeFact {
    Lazy(LazyFact),
    /// `None`: the seat has no receipt for the id.
    Receipt(Option<ReceiptFact>),
}

/// Everything the decision reads about one requested id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleFacts {
    /// `None`: no such message in this instance. Otherwise
    /// `Some((is_ordinary, stored_body_len_bytes))`.
    pub message: Option<(bool, usize)>,
    pub mode: ModeFact,
}

/// What the deciding transaction does with one id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlePlan {
    /// Settle the pending receipt as displayed.
    Settle,
    /// Complete the lazy row as displayed.
    Complete,
    Done(ModAckOutcome, Option<ModAckReason>),
}

fn refuse(reason: ModAckReason) -> SettlePlan {
    SettlePlan::Done(ModAckOutcome::RefusedTerminal, Some(reason))
}

/// Per-id classification (spec D6); never errors per id. Truncation is decided
/// from the stored body length against [`WATCH_BODY_LIMIT_BYTES`].
pub fn plan(facts: SettleFacts) -> SettlePlan {
    let Some((ordinary, body_len)) = facts.message else {
        return refuse(ModAckReason::Unknown);
    };
    if !ordinary {
        return refuse(ModAckReason::NotAddressed);
    }
    let over_limit = body_len > WATCH_BODY_LIMIT_BYTES;
    match facts.mode {
        ModeFact::Lazy(LazyFact::NotAddressed) => refuse(ModAckReason::NotAddressed),
        ModeFact::Lazy(_) if over_limit => refuse(ModAckReason::Truncated),
        ModeFact::Lazy(LazyFact::Completed) => {
            SettlePlan::Done(ModAckOutcome::AlreadySettled, None)
        }
        ModeFact::Lazy(LazyFact::Pending) => SettlePlan::Complete,
        ModeFact::Receipt(
            None | Some(ReceiptFact::NotRequired | ReceiptFact::RecipientRetired),
        ) => refuse(ModAckReason::NotAddressed),
        ModeFact::Receipt(Some(_)) if over_limit => refuse(ModAckReason::Truncated),
        ModeFact::Receipt(Some(ReceiptFact::Acknowledged)) => {
            SettlePlan::Done(ModAckOutcome::AlreadySettled, None)
        }
        ModeFact::Receipt(Some(ReceiptFact::Pending)) => SettlePlan::Settle,
    }
}

#[cfg(test)]
#[path = "../../tests/delivery/settle.rs"]
mod tests;
