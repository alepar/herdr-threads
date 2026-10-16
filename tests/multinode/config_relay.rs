//! Configuration 2 (spec §13): three nodes in a line A - B - C. A and C each
//! dial B and never each other, so A's messages to C's seat and C's ACKs reach
//! the other end only through B's relay.

use crate::support::{self, P1};

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn three_nodes_in_a_line_relay_through_the_middle() {
    let cluster = support::cluster("mn-relay", 3, &[(0, 1), (2, 1)], &[0, 1, 2]);
    let a = cluster.seat(0, P1);
    let mid = cluster.seat(1, P1);
    let c = cluster.seat(2, P1);
    for node in 0..3 {
        support::wait_seats(&cluster, node, 3);
    }
    // The line: A and C each have a single dial target, B's.
    for node in [0, 2] {
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM sync_dial_targets", &[]),
            Some("1".into()),
            "node {node} dials only the middle"
        );
    }
    assert_eq!(
        cluster.scalar(1, "SELECT count(*) FROM sync_dial_targets", &[]),
        Some("0".into())
    );

    let (_, message) = support::send(
        &cluster,
        &a,
        &["--seat", &c.id, "--body", "across", "--require-ack", &c.id],
    );
    support::wait_receipt(&cluster, 2, &message, &c, "pending");
    support::wait_wakes(&cluster, 2, P1, 1);
    assert!(support::inbox_ids(&cluster, &c).contains(&message));
    support::ack(&cluster, &c, &message);
    // The settlement travels C -> B -> A.
    support::wait_receipt(&cluster, 0, &message, &c, "acked");
    cluster.wait_converged(crate::WAIT);
    for node in 0..3 {
        assert_eq!(
            support::receipt(&cluster, node, &message, &c).as_deref(),
            Some("acked"),
            "node {node}"
        );
    }
    // The middle node relayed and applied; neither its seat nor A's was woken.
    assert_eq!(cluster.host(1).prompts_for(P1), 0, "middle seat {}", mid.id);
    assert_eq!(cluster.host(0).prompts_for(P1), 0);
    assert!(
        support::inbox_ids(&cluster, &mid).is_empty(),
        "the middle seat is a bystander of A's pair with C"
    );
}
