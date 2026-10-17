//! Cross-node 1:1 pairs: a send to a seat on another node
//! opens the canonical pair with both seats joined and no invitation. The
//! recipient's home node joins its seat when the pair's fact arrives, so a
//! pair opened while the nodes were apart settles to one joined thread, and
//! the opener is delivered at its own priority, not as an invitation.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::{Cluster, Seat};
use std::time::Duration;

/// Two linked nodes with a seat each, both seats known to both nodes, then
/// cut apart.
fn cut_cluster(label: &str) -> (Cluster, Seat, Seat) {
    let cluster = support::cluster(label, 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 2);
    }
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until(
        "the dial target to link before the cut",
        crate::WAIT,
        || (cluster.target_state(0).as_deref() == Some("linked")).then_some(()),
    );
    cluster.partition(0, 1);
    cluster.wait_until("node 0's dial to be refused", crate::WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    (cluster, a, b)
}

/// `(state, last_joined_n)` of `seat` in `thread` as `node` holds it.
fn membership(cluster: &Cluster, node: usize, thread: &str, seat: &str) -> Option<String> {
    cluster
        .rows(
            node,
            &format!(
                "SELECT state||' '||last_joined_n FROM memberships \
                 WHERE thread_id = '{thread}' AND seat_id = '{seat}'"
            ),
        )
        .into_iter()
        .next()
        .and_then(|row| row.into_iter().next())
}

#[test]
fn crossed_pair_creates_heal_to_one_joined_thread_without_wakes() {
    let (cluster, a, b) = cut_cluster("mn-pair-crossed");
    // Each side opens the pair to the other with a default send while cut.
    let (t_a, m_a) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "from A"]);
    let (t_b, m_b) = support::send(&cluster, &b, &["--seat", &a.id, "--body", "from B"]);
    assert_eq!(t_a, t_b, "the pair thread is the same deterministic thread");
    assert!(t_a.starts_with('d'), "a direct pair thread: {t_a}");

    cluster.heal(0, 1);
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        for seat in [&a, &b] {
            cluster.wait_until(
                &format!("node {node} to hold {} joined once", seat.id),
                crate::WAIT,
                || {
                    (membership(&cluster, node, &t_a, &seat.id).as_deref() == Some("joined 1"))
                        .then_some(())
                },
            );
        }
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM threads WHERE id LIKE 'd%'", &[]),
            Some("1".into()),
            "node {node}: one pair thread"
        );
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM invitations", &[]),
            Some("0".into()),
            "node {node}: no invitation for a pair"
        );
        for message in [&m_a, &m_b] {
            assert_eq!(
                cluster.scalar(
                    node,
                    "SELECT count(*) FROM messages WHERE id = ?1",
                    &[message]
                ),
                Some("1".into()),
                "node {node} holds {message}"
            );
        }
    }
    // Both openers were p1 and both seats are at T = 0: nobody is woken.
    std::thread::sleep(Duration::from_millis(1500));
    for node in [0, 1] {
        assert_eq!(
            cluster.host(node).prompts_for(P1),
            0,
            "node {node}: {:?}",
            cluster.host(node).prompts()
        );
    }
    assert!(support::inbox_ids(&cluster, &a).contains(&m_b));
    assert!(support::inbox_ids(&cluster, &b).contains(&m_a));
}

#[test]
fn one_sided_create_while_unlinked_delivers_in_priority_order_with_one_wake() {
    let (cluster, a, b) = cut_cluster("mn-pair-one-sided");
    // A opens the pair with a p1, then sends a p0 and another p1, all while cut.
    let (thread, first) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "first"]);
    let (_, urgent) = support::send(
        &cluster,
        &a,
        &["--seat", &b.id, "--body", "urgent", "-p", "0"],
    );
    let (_, last) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "last"]);
    assert_eq!(
        cluster.scalar(1, "SELECT count(*) FROM threads", &[]),
        Some("0".into()),
        "B knows nothing of the pair while cut"
    );

    cluster.heal(0, 1);
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        cluster.wait_until(
            &format!("node {node} to hold b joined once"),
            crate::WAIT,
            || {
                (membership(&cluster, node, &thread, &b.id).as_deref() == Some("joined 1"))
                    .then_some(())
            },
        );
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM invitations", &[]),
            Some("0".into()),
            "node {node}: no invitation for a pair"
        );
    }
    // The p0 wakes the idle seat once; the page lists the p0 before the two
    // p1s, which keep their age order.
    support::wait_wakes(&cluster, 1, P1, 1);
    assert_eq!(
        support::inbox_ids(&cluster, &b),
        vec![urgent, first, last],
        "priority, then age"
    );
}
