//! Configuration 3 (spec §13): partition and heal with writes on both sides,
//! the same direct pair opened on both sides, the unreachable-node marker
//! after the reach threshold, and agreement on `sync verify` after the heal.

use crate::support::{self, P1};
use std::time::Duration;

/// The tag of `node` as its own store records it.
fn tag_of(cluster: &herdr_threads::test_support::multinode::Cluster, node: usize) -> String {
    cluster
        .scalar(node, "SELECT tag FROM local_node", &[])
        .expect("the node's tag")
}

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn partition_with_writes_on_both_sides_heals() {
    let mut cluster = support::cluster("mn-partition", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until(
        "the dial target to link before the cut",
        crate::WAIT,
        || (cluster.target_state(0).as_deref() == Some("linked")).then_some(()),
    );

    // The persisted contact book is flushed on a cadence: wait for each side to
    // hold the other's contact before the cut, as a long-linked pair would.
    for (node, other) in [(0, 1), (1, 0)] {
        let uuid = cluster.node_uuid(other).to_string();
        cluster.wait_until(
            &format!("node {node}'s contact with {other}"),
            crate::WAIT,
            || {
                cluster.scalar(
                    node,
                    "SELECT last_contact_at FROM sync_node_contact WHERE node_uuid = ?1",
                    &[&uuid],
                )
            },
        );
    }

    cluster.partition(0, 1);
    cluster.wait_until("node 0's dial to be refused", crate::WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    // Both sides open the same direct pair, then keep writing.
    let (t_a, m_a1) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "from A 1"]);
    let (t_b, m_b1) = support::send(&cluster, &b, &["--seat", &a.id, "--body", "from B 1"]);
    let (_, m_a2) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "from A 2"]);
    assert_eq!(t_a, t_b, "the pair thread is the same deterministic thread");
    assert!(
        cluster.converged().is_none(),
        "the vectors differ while cut"
    );

    // Within the reach threshold the cut-off node is still reachable: no marker.
    let (_, text, _) = cluster.text_as(Some(&a), 0, &["thread", "show", &t_a]);
    assert!(!text.contains("unreachable"), "{text}");

    // Past the reach threshold each side prints the marker for the other.
    cluster.age_contact(0, 1, Duration::from_secs(200));
    cluster.age_contact(1, 0, Duration::from_secs(200));
    for (node, seat, other) in [(0, &a, 1), (1, &b, 0)] {
        let marker = format!("node {} unreachable since ", tag_of(&cluster, other));
        for verb in ["show", "participants"] {
            let (code, text, stderr) = cluster.text_as(Some(seat), node, &["thread", verb, &t_a]);
            assert_eq!(code, 0, "thread {verb} on node {node}: {stderr}");
            assert!(
                text.contains(&marker),
                "node {node} thread {verb} lacks {marker:?}:\n{text}"
            );
        }
    }

    cluster.heal(0, 1);
    cluster.wait_converged(crate::WAIT);
    // Contact is fresh again: the marker is gone on both sides.
    for (node, seat, other) in [(0, &a, 1), (1, &b, 0)] {
        let marker = format!("node {} unreachable", tag_of(&cluster, other));
        cluster.wait_until(
            &format!("node {node}'s marker to clear"),
            crate::WAIT,
            || {
                let (_, text, _) = cluster.text_as(Some(seat), node, &["thread", "show", &t_a]);
                (!text.contains(&marker)).then_some(())
            },
        );
    }
    for node in [0, 1] {
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM threads WHERE id = ?1", &[&t_a]),
            Some("1".into()),
            "node {node}: one pair thread"
        );
        for m in [&m_a1, &m_b1, &m_a2] {
            assert_eq!(
                cluster.scalar(node, "SELECT count(*) FROM messages WHERE id = ?1", &[m]),
                Some("1".into()),
                "node {node} holds {m}"
            );
        }
        let out = cluster.cli(node, &["sync", "verify"]);
        assert!(out.status.success(), "node {node}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "verify ok\n",
            "node {node} agrees after the heal"
        );
    }
}
