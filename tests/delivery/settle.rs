//! `delivery::settle::plan` reproduces every decision of the store's former
//! `plan_mod_ack` (delivery-policy DP3).
use super::*;

fn ordinary(body_len: usize, mode: ModeFact) -> SettleFacts {
    SettleFacts {
        message: Some((true, body_len)),
        mode,
    }
}

fn refused(reason: ModAckReason) -> SettlePlan {
    SettlePlan::Done(ModAckOutcome::RefusedTerminal, Some(reason))
}

#[test]
fn unknown_id_is_refused_unknown() {
    let facts = SettleFacts {
        message: None,
        mode: ModeFact::Receipt(Some(ReceiptFact::Pending)),
    };
    assert_eq!(plan(facts), refused(ModAckReason::Unknown));
}

#[test]
fn non_ordinary_message_is_not_addressed() {
    let facts = SettleFacts {
        message: Some((false, 3)),
        mode: ModeFact::Receipt(Some(ReceiptFact::Pending)),
    };
    assert_eq!(plan(facts), refused(ModAckReason::NotAddressed));
}

#[test]
fn ordinary_pending_receipt_settles() {
    let facts = ordinary(10, ModeFact::Receipt(Some(ReceiptFact::Pending)));
    assert_eq!(plan(facts), SettlePlan::Settle);
}

#[test]
fn body_exactly_at_the_limit_is_not_truncated() {
    let facts = ordinary(
        WATCH_BODY_LIMIT_BYTES,
        ModeFact::Receipt(Some(ReceiptFact::Pending)),
    );
    assert_eq!(plan(facts), SettlePlan::Settle);
}

#[test]
fn truncated_body_is_refused_terminal_for_receipt_and_lazy() {
    let over = WATCH_BODY_LIMIT_BYTES + 1;
    for mode in [
        ModeFact::Receipt(Some(ReceiptFact::Pending)),
        ModeFact::Receipt(Some(ReceiptFact::Acknowledged)),
        ModeFact::Lazy(LazyFact::Pending),
        ModeFact::Lazy(LazyFact::Completed),
    ] {
        assert_eq!(
            plan(ordinary(over, mode)),
            refused(ModAckReason::Truncated),
            "{mode:?}"
        );
    }
}

#[test]
fn unaddressed_wins_over_truncation() {
    let over = WATCH_BODY_LIMIT_BYTES + 1;
    for mode in [
        ModeFact::Receipt(None),
        ModeFact::Receipt(Some(ReceiptFact::NotRequired)),
        ModeFact::Receipt(Some(ReceiptFact::RecipientRetired)),
        ModeFact::Lazy(LazyFact::NotAddressed),
    ] {
        assert_eq!(
            plan(ordinary(over, mode)),
            refused(ModAckReason::NotAddressed),
            "{mode:?}"
        );
    }
}

#[test]
fn already_settled_receipt_and_lazy_row() {
    let done = SettlePlan::Done(ModAckOutcome::AlreadySettled, None);
    assert_eq!(
        plan(ordinary(
            1,
            ModeFact::Receipt(Some(ReceiptFact::Acknowledged))
        )),
        done
    );
    assert_eq!(plan(ordinary(1, ModeFact::Lazy(LazyFact::Completed))), done);
}

#[test]
fn pending_lazy_row_completes() {
    assert_eq!(
        plan(ordinary(1, ModeFact::Lazy(LazyFact::Pending))),
        SettlePlan::Complete
    );
}

// Kills: choosing the settle path from the message rather than from the (message, seat) row
// actually present. One mixed-ACK message gives its receipt seat `Settle` and its delivery-row
// seat `Complete`; the store-level scenario is `mixed_ack_display_and_completion`.
#[test]
fn a_mixed_ack_message_settles_by_the_row_each_seat_holds() {
    let receipt_seat = ordinary(10, ModeFact::Receipt(Some(ReceiptFact::Pending)));
    let row_seat = ordinary(10, ModeFact::Lazy(LazyFact::Pending));
    assert_eq!(plan(receipt_seat), SettlePlan::Settle);
    assert_eq!(plan(row_seat), SettlePlan::Complete);
    let other_seat = ordinary(10, ModeFact::Receipt(None));
    assert_eq!(plan(other_seat), refused(ModAckReason::NotAddressed));
}
