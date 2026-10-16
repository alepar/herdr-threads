//! Configuration 7 (spec §13): one node with two Herdr hosts. A seat on each
//! host is reachable from another node: each is woken by its own host's wake
//! lane, and both ACK back.

use crate::support::{self, P1};

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn one_node_with_two_hosts_reaches_seats_on_both() {
    let mut cluster = support::cluster("mn-two-hosts", 2, &[(1, 0)], &[0, 1]);
    cluster.add_host(0, &[support::P1, support::P2]);
    let s0 = cluster.seat_on(0, 0, P1);
    let s1 = cluster.seat_on(0, 1, P1);
    let b = cluster.seat(1, P1);
    assert_ne!(s0.id, s1.id, "same pane id on two servers, two seats");
    for node in 0..2 {
        support::wait_seats(&cluster, node, 3);
    }
    assert_eq!(
        cluster.scalar(0, "SELECT count(*) FROM herdr_hosts", &[]),
        Some("2".into()),
        "node 0 attached both servers"
    );

    let (_, m0) = support::send(
        &cluster,
        &b,
        &[
            "--seat",
            &s0.id,
            "--body",
            "to host 0",
            "--require-ack",
            &s0.id,
        ],
    );
    let (_, m1) = support::send(
        &cluster,
        &b,
        &[
            "--seat",
            &s1.id,
            "--body",
            "to host 1",
            "--require-ack",
            &s1.id,
        ],
    );
    // Each server types exactly its own seat's wake marker.
    support::wait_wakes_at(&cluster, 0, 0, P1, 1);
    support::wait_wakes_at(&cluster, 0, 1, P1, 1);
    assert_eq!(
        cluster.host(1).prompts_for(P1),
        0,
        "the sender is not woken"
    );
    assert!(support::inbox_ids(&cluster, &s0).contains(&m0));
    assert!(!support::inbox_ids(&cluster, &s0).contains(&m1));
    assert!(support::inbox_ids(&cluster, &s1).contains(&m1));

    support::ack(&cluster, &s0, &m0);
    support::ack(&cluster, &s1, &m1);
    support::wait_receipt(&cluster, 1, &m0, &s0, "acked");
    support::wait_receipt(&cluster, 1, &m1, &s1, "acked");
    cluster.wait_converged(crate::WAIT);
}
