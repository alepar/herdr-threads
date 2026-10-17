//! Configuration 2 (spec §13): three nodes in a line A - B - C. A and C each
//! dial B and never each other, so A's messages to C's seat and C's ACKs reach
//! the other end only through B's relay.

use crate::support::{self, P1, P2};

#[test]
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

/// A pair opened from both ends through a relay with no pair seat: each end's
/// `membership.joined` reaches the other node only through B, whose log holds
/// none of them. Every node settles on one pair thread with both seats joined
/// once and no invitation, whatever order the two logs arrive in.
#[test]
fn pair_auto_join_facts_relay_through_a_node_with_no_pair_seat() {
    let cluster = support::cluster("mn-relay-pair", 3, &[(0, 1), (2, 1)], &[0, 1, 2]);
    let a = cluster.seat(0, P1);
    let _mid = cluster.seat(1, P1);
    let c = cluster.seat(2, P1);
    let _spare = cluster.seat(1, P2);
    for node in 0..3 {
        support::wait_seats(&cluster, node, 4);
    }

    // Each end must open independently. Incoming auto-join facts during C's
    // send preparation can correctly trip the audience snapshot fence.
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until("C's relay link to be established", crate::WAIT, || {
        (cluster.target_state(2).as_deref() == Some("linked")).then_some(())
    });
    cluster.partition(2, 1);
    cluster.wait_until("C's relay dial to be refused", crate::WAIT, || {
        (cluster.target_state(2).as_deref() == Some("backoff")).then_some(())
    });

    // Both ends open before either can learn about the other's creation.
    let (t_a, m_a) = support::send(&cluster, &a, &["--seat", &c.id, "--body", "from A"]);
    assert_eq!(
        cluster.scalar(2, "SELECT count(*) FROM threads WHERE id LIKE 'd%'", &[]),
        Some("0".into()),
        "C must open the pair independently, before A's creation arrives"
    );
    let (t_c, m_c) = support::send(&cluster, &c, &["--seat", &a.id, "--body", "from C"]);
    assert_eq!(t_a, t_c, "one deterministic pair thread");
    cluster.heal(2, 1);
    cluster.wait_converged(crate::WAIT);
    for node in 0..3 {
        for seat in [&a, &c] {
            cluster.wait_until(
                &format!("node {node} to hold {} joined once", seat.id),
                crate::WAIT,
                || {
                    (cluster
                        .scalar(
                            node,
                            "SELECT state||' '||last_joined_n FROM memberships \
                             WHERE thread_id = ?1 AND seat_id = ?2",
                            &[&t_a, &seat.id],
                        )
                        .as_deref()
                        == Some("joined 1"))
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
        for message in [&m_a, &m_c] {
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
    // B has no pair seat: its own log authored no join, while the joins of
    // A's and C's logs reached it by relay.
    let joined_by = |node: usize, author: usize| {
        let uuid = cluster.node_uuid(author).to_string();
        cluster.scalar(
            node,
            "SELECT count(*) FROM journal j JOIN sync_logs l ON l.log_ix = j.log_ix \
             WHERE j.kind = 'membership.joined' AND l.node_uuid = ?1",
            &[&uuid],
        )
    };
    assert_eq!(joined_by(1, 1).as_deref(), Some("0"), "B authored no join");
    for author in [0, 2] {
        assert_eq!(
            joined_by(1, author).as_deref(),
            Some("1"),
            "B relayed node {author}'s join"
        );
    }
    // The relay and the bystander seat are never woken by a p1 pair opener.
    assert_eq!(cluster.host(1).prompts_for(P1), 0);
    assert_eq!(cluster.host(1).prompts_for(P2), 0);
}
