//! Integration sweep (ht-5fr.15): flows of the goal that no per-seam
//! configuration covers. An agent on A creates a thread and invites seats on B
//! that no fact referenced before; each invitation (priority 0) wakes its
//! invitee, B accepts, later sends wake or not per the delivery contract and
//! ACKs; A sees the settlement. A three-node partition with writes on both
//! sides, a pair opened from the cut side included, heals into identical
//! replicated projections.

use crate::support::{self, P1, P2, P3};
use herdr_threads::test_support::journal_oracle::{TableSpec, replicated_tables};
use herdr_threads::test_support::multinode::{Cluster, Seat};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

type Projection = BTreeMap<String, BTreeSet<Vec<String>>>;

fn label(spec: &TableSpec) -> String {
    match spec.filter {
        Some(filter) => format!("{} where {filter}", spec.name),
        None => spec.name.to_owned(),
    }
}

/// The replicated columns of every replicated table as `node` holds them, as
/// sets of rows (the order rows were applied in does not show).
fn projection(cluster: &Cluster, node: usize) -> Projection {
    let mut out = Projection::new();
    for spec in replicated_tables() {
        let columns: Vec<&str> = spec
            .key_columns
            .iter()
            .chain(spec.replicated_columns)
            .copied()
            .collect();
        let filter = spec.filter.map_or(String::new(), |f| format!(" WHERE {f}"));
        let sql = format!("SELECT {} FROM {}{filter}", columns.join(","), spec.name);
        out.insert(label(&spec), cluster.rows(node, &sql).into_iter().collect());
    }
    out
}

/// Waits until the vectors agree and every node's projection equals node 0's;
/// returns that projection.
fn wait_identical_projections(cluster: &Cluster, nodes: usize) -> Projection {
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until("the replicated projections to agree", crate::WAIT, || {
        let first = projection(cluster, 0);
        (1..nodes)
            .all(|n| projection(cluster, n) == first)
            .then_some(first)
    })
}

/// Sends into `thread` as `from` and returns the message id.
fn send_in(cluster: &Cluster, from: &Seat, thread: &str, args: &[&str]) -> String {
    let mut argv = vec!["send", thread];
    argv.extend_from_slice(args);
    let data = cluster.ok_as(Some(from), from.node, &argv);
    data.as_str().expect("a message id").to_owned()
}

/// `a` creates a thread and invites `invitee` (a seat on node 1 that no fact
/// referenced before); waits for the pending invitation on B and the
/// invitee's one wake, accepts it as the invitee and waits until both nodes
/// hold the membership joined.
fn open_thread(cluster: &Cluster, a: &Seat, invitee: &Seat, topic: &str) -> String {
    let thread = cluster.ok_as(Some(a), 0, &["thread", "create", "--topic", topic]);
    let thread = thread.as_str().expect("a thread id").to_owned();
    cluster.ok_as(Some(a), 0, &["invite", &thread, "--seat", &invitee.id]);
    cluster.wait_until("the invitation to reach B", crate::WAIT, || {
        let state = cluster.scalar(
            1,
            "SELECT state FROM invitations WHERE thread_id = ?1 AND seat_id = ?2",
            &[&thread, &invitee.id],
        );
        (state.as_deref() == Some("pending")).then_some(())
    });
    // An invitation is p0: it wakes the idle invitee once, before the accept
    // settles it (an accepted invitation is no longer pending to wake for).
    support::wait_wakes(cluster, 1, &invitee.pane, 1);
    cluster.ok_as(Some(invitee), 1, &["accept", &thread]);
    for node in [0, 1] {
        cluster.wait_until("the membership to be joined", crate::WAIT, || {
            let state = cluster.scalar(
                node,
                "SELECT state FROM memberships WHERE thread_id = ?1 AND seat_id = ?2",
                &[&thread, &invitee.id],
            );
            (state.as_deref() == Some("joined")).then_some(())
        });
    }
    thread
}

#[test]
fn created_thread_invites_unseen_seats_and_delivery_follows_the_contract() {
    let cluster = support::cluster("mn-sweep-invite", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    // One recipient and one thread per case: a seat is woken at most once per 30 s.
    let nudged = cluster.seat(1, P1);
    let quiet = cluster.seat(1, P2);
    let acker = cluster.seat(1, P3);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 4);
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM threads", &[]),
            Some("0".into()),
            "nothing references B's seats yet"
        );
    }

    let t_nudge = open_thread(&cluster, &a, &nudged, "nudge");
    let t_quiet = open_thread(&cluster, &a, &quiet, "quiet");
    let t_ack = open_thread(&cluster, &a, &acker, "ack");
    // Each invitation woke its invitee exactly once, and the accept woke nobody.
    for pane in [P1, P2, P3] {
        assert_eq!(cluster.host(1).prompts_for(pane), 1, "{pane}");
    }

    // A `-p 0` reaches the seat that was just woken for its invitation. The
    // wake matrix (`config_delivery`) judges priority against threshold on
    // fresh seats; a second wake of one seat is spaced out by the wake
    // backoff, so only delivery is judged here.
    let n = send_in(&cluster, &a, &t_nudge, &["--body", "urgent", "-p", "0"]);
    // Send completion is local to A; B's inbox needs the applied message.
    cluster.wait_until("the p0 message to reach B", crate::WAIT, || {
        cluster.scalar(1, "SELECT id FROM messages WHERE id = ?1", &[&n])
    });
    assert!(support::inbox_ids(&cluster, &nudged).contains(&n));

    // A p1 waits for the seat's next turn.
    let l = send_in(&cluster, &a, &t_quiet, &["--body", "quiet"]);
    cluster.wait_until("the p1 message to reach B", crate::WAIT, || {
        cluster.scalar(1, "SELECT id FROM messages WHERE id = ?1", &[&l])
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        cluster.host(1).prompts_for(P2),
        1,
        "a p1 send wakes nothing beyond the invitation: {:?}",
        cluster.host(1).prompts()
    );
    assert!(support::inbox_ids(&cluster, &quiet).contains(&l));

    // `--require-ack` is p0; the ACK settles on A.
    let r = send_in(
        &cluster,
        &a,
        &t_ack,
        &["--body", "ack me", "--require-ack", &acker.id],
    );
    support::wait_receipt(&cluster, 0, &r, &acker, "pending");
    // A's receipt exists at send commit; the ACK on B needs its replica too.
    support::wait_receipt(&cluster, 1, &r, &acker, "pending");
    support::ack(&cluster, &acker, &r);
    support::wait_receipt(&cluster, 0, &r, &acker, "acked");
    assert_eq!(
        cluster.host(0).prompts_for(P1),
        0,
        "the sender is never woken: {:?}",
        cluster.host(0).prompts()
    );
    wait_identical_projections(&cluster, 2);
}

#[test]
fn three_node_partition_with_writes_on_both_sides_heals_to_identical_projections() {
    // A - B - C: C is cut off from B while A and C keep writing.
    let cluster = support::cluster("mn-sweep-heal", 3, &[(0, 1), (2, 1)], &[0, 2]);
    let a = cluster.seat(0, P1);
    let c = cluster.seat(2, P1);
    for node in 0..3 {
        support::wait_seats(&cluster, node, 2);
    }
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until("C's dial to link before the cut", crate::WAIT, || {
        (cluster.target_state(2).as_deref() == Some("linked")).then_some(())
    });

    cluster.partition(2, 1);
    cluster.wait_until("C's dial to be refused", crate::WAIT, || {
        (cluster.target_state(2).as_deref() == Some("backoff")).then_some(())
    });
    let thread = cluster.ok_as(Some(&a), 0, &["thread", "create", "--topic", "cut"]);
    let thread = thread.as_str().expect("a thread id").to_owned();
    cluster.ok_as(Some(&a), 0, &["invite", &thread, "--seat", &c.id]);
    let m_a = send_in(&cluster, &a, &thread, &["--body", "A while cut"]);
    let (pair, m_c) = support::send(
        &cluster,
        &c,
        &["--seat", &a.id, "--body", "C while cut", "-p", "0"],
    );
    assert!(
        cluster.converged().is_none(),
        "the vectors differ while cut"
    );
    assert_ne!(
        projection(&cluster, 0),
        projection(&cluster, 2),
        "the sides hold different replicated state while cut"
    );

    cluster.heal(2, 1);
    let healed = wait_identical_projections(&cluster, 3);
    assert!(
        healed.values().any(|rows| !rows.is_empty()),
        "the compared projection is not empty"
    );
    for node in 0..3 {
        // The pair C opened while cut auto-joined both seats, with no invitation.
        for seat in [&a, &c] {
            assert_eq!(
                cluster
                    .scalar(
                        node,
                        "SELECT state FROM memberships WHERE thread_id = ?1 AND seat_id = ?2",
                        &[&pair, &seat.id],
                    )
                    .as_deref(),
                Some("joined"),
                "node {node}: {} in the pair",
                seat.id
            );
        }
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM invitations WHERE thread_id = ?1",
                &[&pair]
            ),
            Some("0".into()),
            "node {node}: no invitation for the pair"
        );
        assert_eq!(
            cluster
                .scalar(node, "SELECT priority FROM messages WHERE id = ?1", &[&m_c])
                .as_deref(),
            Some("0"),
            "node {node}: C's priority travels"
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
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM threads WHERE id IN (?1, ?2)",
                &[&thread, &pair]
            ),
            Some("2".into()),
            "node {node} holds both threads"
        );
        let out = cluster.cli(node, &["sync", "verify"]);
        assert!(out.status.success(), "node {node}: {out:?}");
    }
}
