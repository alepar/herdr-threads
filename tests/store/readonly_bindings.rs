//! Read-only bindings (D3, N3-readonly): a binding opened with `HERDR_THREADS_SEAT_READONLY=1`
//! reads, advances its cursors and sends like any other, but its display never settles a
//! receipt-bearing row; a sibling writable binding of the same seat still does.
use super::*;
use crate::store::effective::{self, EffectiveReceiptState};

fn state(fx: &Fx, id: &MessageId) -> EffectiveReceiptState {
    effective::effective_receipt(&fx.conn, id.as_str(), "b")
        .unwrap()
        .expect("receipt")
        .state
}

fn mark_readonly(fx: &Fx, claim: &CallerClaim) {
    let binding = binding_of(fx, claim);
    assert!(!bindings::is_readonly(&fx.conn, &binding).unwrap());
    bindings::set_readonly(&fx.conn, &binding).unwrap();
    assert!(bindings::is_readonly(&fx.conn, &binding).unwrap());
}

#[test]
fn readonly_display_never_acks() {
    let mut fx = fx("ro-display");
    // The sender binding is read-only too: sending is unaffected.
    mark_readonly(&fx, &claim_a());
    mark_readonly(&fx, &claim_b2());
    let m1 = send(&mut fx, "one");
    let m2 = send(&mut fx, "two");
    let both = [&m1, &m2];

    // The read-only binding reads its rows (still ACK candidates for a writable viewer) ...
    assert_eq!(
        page(&fx, Some(&claim_b2())),
        vec![msg(&m1, true), msg(&m2, true)]
    );
    // ... its display ACK settles nothing and reports nothing acknowledged ...
    ack_displayed(&fx, &claim_b2(), "ro-ack", &both);
    assert_eq!(state(&fx, &m1), EffectiveReceiptState::Pending);
    assert_eq!(state(&fx, &m2), EffectiveReceiptState::Pending);
    // ... and it still advances its own cursor. The obligations are the seat's, so they stay
    // listed as pending until a writable binding discharges them.
    advance(&fx, &claim_b2(), "ro-adv", &both);
    assert_cursor(&fx, &claim_b2(), &m2, None);
    assert_eq!(
        page(&fx, Some(&claim_b2())),
        vec![msg(&m1, true), msg(&m2, true)]
    );

    // The writable sibling still sees the obligations pending and its display settles them.
    assert_eq!(
        page(&fx, Some(&claim_b1())),
        vec![msg(&m1, true), msg(&m2, true)]
    );
    ack_displayed(&fx, &claim_b1(), "rw-ack", &both);
    assert_eq!(state(&fx, &m1), EffectiveReceiptState::Acknowledged);
    assert_eq!(state(&fx, &m2), EffectiveReceiptState::Acknowledged);
}

#[test]
fn pane_site_without_variable_unchanged() {
    let mut fx = fx("ro-unchanged");
    let m = send(&mut fx, "one");
    assert!(!bindings::is_readonly(&fx.conn, &binding_of(&fx, &claim_b1())).unwrap());
    ack_displayed(&fx, &claim_b1(), "rw-ack", &[&m]);
    assert_eq!(state(&fx, &m), EffectiveReceiptState::Acknowledged);
}

#[test]
fn readonly_flag_only_rises_and_is_per_binding() {
    let fx = fx("ro-flag");
    mark_readonly(&fx, &claim_b2());
    // The sibling binding of the same seat is untouched.
    assert!(!bindings::is_readonly(&fx.conn, &binding_of(&fx, &claim_b1())).unwrap());
    // Raising it again is a no-op.
    bindings::set_readonly(&fx.conn, &binding_of(&fx, &claim_b2())).unwrap();
    assert!(bindings::is_readonly(&fx.conn, &binding_of(&fx, &claim_b2())).unwrap());
}
