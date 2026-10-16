//! Configuration 6 (spec §13): one node with zero Herdr hosts. The middle node
//! of the line runs store-only, keeps running on its peer links alone, and
//! relays A's message and C's ACK between two nodes that have hosts.

use crate::support::{self, P1};

#[test]
fn store_only_node_relays() {
    let cluster = support::cluster("mn-store-only", 3, &[(0, 1), (2, 1)], &[0, 2]);
    let a = cluster.seat(0, P1);
    let c = cluster.seat(2, P1);
    for node in 0..3 {
        support::wait_seats(&cluster, node, 2);
    }
    let (_, message) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &c.id,
            "--body",
            "via store-only",
            "--require-ack",
            &c.id,
        ],
    );
    support::wait_receipt(&cluster, 2, &message, &c, "pending");
    support::wait_wakes(&cluster, 2, P1, 1);
    support::ack(&cluster, &c, &message);
    support::wait_receipt(&cluster, 0, &message, &c, "acked");
    cluster.wait_converged(crate::WAIT);

    // The store-only node holds the whole exchange and owns no seat.
    assert_eq!(
        support::receipt(&cluster, 1, &message, &c).as_deref(),
        Some("acked")
    );
    let kinds = cluster.kinds(1);
    assert_eq!(kinds.get("message.sent"), Some(&1), "{kinds:?}");
    assert_eq!(
        cluster.scalar(1, "SELECT count(*) FROM seats WHERE is_local = 1", &[]),
        Some("0".into()),
        "no local seat on a node without a host"
    );
    assert_eq!(
        cluster.scalar(1, "SELECT count(*) FROM herdr_hosts", &[]),
        Some("0".into()),
        "no Herdr host was ever attached"
    );
    // Its daemon is up on the links alone.
    let health = cluster.cli(1, &["daemon", "health"]);
    assert!(
        String::from_utf8_lossy(&health.stdout).contains("boot_id: "),
        "{health:?}"
    );
}
