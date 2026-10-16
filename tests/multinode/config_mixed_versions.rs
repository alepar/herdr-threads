//! Configuration 5 (spec §13): mixed versions. Node A authors a fact kind the
//! other daemons do not know; the middle node relays it unchanged and parks
//! it, and later kinds from the same log (a send, then C's ACK) still apply
//! everywhere.

use crate::support::{self, P1};
use crate::{FutureKind, note};

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn unknown_kind_relays_parked_and_later_kinds_still_apply() {
    let mut cluster = support::cluster("mn-mixed", 3, &[(0, 1), (2, 1)], &[0, 2]);
    let a = cluster.seat(0, P1);
    let c = cluster.seat(2, P1);
    for node in 0..3 {
        support::wait_seats(&cluster, node, 2);
    }
    cluster.wait_converged(crate::WAIT);

    // An "upgraded" A authors a kind of the future.
    let (counter, hash) = cluster.author_kind::<FutureKind>(0, &note("from the future"));
    // A later, known kind follows it in the same log.
    let (_, message) = support::send(
        &cluster,
        &a,
        &["--seat", &c.id, "--body", "after", "--require-ack", &c.id],
    );
    support::wait_receipt(&cluster, 2, &message, &c, "pending");
    support::wait_wakes(&cluster, 2, P1, 1);
    support::ack(&cluster, &c, &message);
    support::wait_receipt(&cluster, 0, &message, &c, "acked");
    cluster.wait_converged(crate::WAIT);

    for node in [1, 2] {
        let entry = cluster
            .journal_entry(node, 0, counter)
            .unwrap_or_else(|| panic!("node {node} holds the future entry"));
        assert_eq!(entry.kind, "future.note");
        assert_eq!(entry.hash, hash.to_vec(), "node {node}: relayed unchanged");
        assert_eq!(entry.apply_state, "parked_unknown", "node {node}");
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT apply_state FROM journal WHERE kind = 'message.sent'",
                &[]
            ),
            Some("applied".into()),
            "node {node} applies the send behind the parked entry"
        );
        assert_eq!(
            support::receipt(&cluster, node, &message, &c).as_deref(),
            Some("acked"),
            "node {node}"
        );
    }
}
