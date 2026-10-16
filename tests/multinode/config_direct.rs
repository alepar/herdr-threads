//! Configuration 1 (spec §13): two nodes linked directly; a seat on each; a
//! direct send with `--require-ack` from A to B's seat, delivered and woken on
//! B, ACKed on B, settled on both.

use crate::support::{self, P1};

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn two_nodes_direct_send_and_ack() {
    let cluster = support::cluster("mn-direct", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);

    let (thread, message) = support::send(
        &cluster,
        &a,
        &["--seat", &b.id, "--body", "ping", "--require-ack", &b.id],
    );
    // B holds the message and its pending receipt; its idle seat was woken once.
    support::wait_receipt(&cluster, 1, &message, &b, "pending");
    support::wait_wakes(&cluster, 1, P1, 1);
    assert!(
        support::inbox_ids(&cluster, &b).contains(&message),
        "B's seat is offered the message"
    );
    assert_eq!(
        cluster.host(0).prompts_for(P1),
        0,
        "the sender is not woken"
    );

    support::ack(&cluster, &b, &message);
    support::wait_receipt(&cluster, 0, &message, &b, "acked");
    support::wait_receipt(&cluster, 1, &message, &b, "acked");
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        let kinds = cluster.kinds(node);
        assert_eq!(
            kinds.get("message.sent"),
            Some(&1),
            "node {node}: {kinds:?}"
        );
        assert!(
            kinds.contains_key("receipt.settled"),
            "node {node}: {kinds:?}"
        );
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM threads WHERE id = ?1",
                &[&thread]
            ),
            Some("1".into()),
            "node {node} holds the pair thread"
        );
    }
    drop(cluster);
}
