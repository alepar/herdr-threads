//! Configuration 4 (spec §13): a node restored from a store backup that then
//! authors before it learns what its peer holds forks its own log. The fork is
//! detected, the owner re-issues under a new incarnation and the union of both
//! branches converges; `sync resync --from <peer>` then rebuilds the store and
//! the vectors converge again.

use crate::support::{self, P1, P2, P3};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};

fn tag(cluster: &herdr_threads::test_support::multinode::Cluster, node: usize) -> String {
    cluster
        .scalar(node, "SELECT tag FROM local_node", &[])
        .expect("the node's tag")
}

/// The incarnations of `node`'s own log that `at` holds.
fn incarnations(
    cluster: &herdr_threads::test_support::multinode::Cluster,
    at: usize,
    node: usize,
) -> i64 {
    cluster
        .scalar(
            at,
            "SELECT count(*) FROM sync_logs WHERE node_uuid = ?1",
            &[&cluster.node_uuid(node).to_string()],
        )
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// A peer pump can pass the shutdown gate before a resync and win the next
/// daemon election store-only. Starting the node must attach its host even
/// when `daemon ensure` finds that winner already running.
#[test]
fn starting_node_after_store_only_peer_attaches_its_host() {
    let mut cluster = Cluster::start(
        "mn-restore-start-host",
        crate::BIN,
        1,
        ClusterOptions::default(),
    );
    cluster.add_host(0, &[P1]);
    cluster.stop_node(0);

    // Give the store-only contender the election deterministically, while
    // leaving the fixture's node stopped so start_node takes its ensure path.
    let out = cluster
        .isolation()
        .command(crate::BIN)
        .arg("--state-dir")
        .arg(cluster.state_dir(0))
        .args(["daemon", "ensure"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        cluster.scalar(0, "SELECT count(*) FROM herdr_hosts", &[]),
        Some("0".into()),
        "the election winner has no remembered host"
    );

    cluster.start_node(0);
    assert_eq!(
        cluster.scalar(0, "SELECT count(*) FROM herdr_hosts", &[]),
        Some("1".into()),
        "start_node attaches its host to an already-running store-only daemon"
    );
}

#[test]
fn restore_from_backup_forks_is_reissued_and_resync_converges() {
    // Targets both ways: B needs a target for A for `sync resync --from A`.
    let mut cluster = support::cluster("mn-restore", 2, &[(0, 1), (1, 0)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);

    // B's backup, then a fact B authors after it, which A ingests.
    let backup = cluster.backup(1);
    let (thread, lost) = support::send(
        &cluster,
        &b,
        &["--seat", &a.id, "--body", "authored after the backup"],
    );
    cluster.wait_converged(crate::WAIT);
    assert_eq!(
        cluster.scalar(0, "SELECT count(*) FROM messages WHERE id = ?1", &[&lost]),
        Some("1".into())
    );

    // B is restored (the store of the past) while cut off, and authors a
    // different fact at the same position of its log.
    cluster.partition(0, 1);
    cluster.restore(1, &backup);
    assert_eq!(
        cluster.scalar(1, "SELECT count(*) FROM messages WHERE id = ?1", &[&lost]),
        Some("0".into()),
        "the restored store lacks the fact"
    );
    let (thread_b, kept) = support::send(
        &cluster,
        &b,
        &["--seat", &a.id, "--body", "authored after the restore"],
    );
    assert_eq!(thread, thread_b, "the same pair thread");

    cluster.heal(0, 1);
    // Fork detected: B's old incarnation is frozen, B minted a new one, and the
    // union of both branches reached both nodes.
    cluster.wait_until(
        "B to hold a second incarnation everywhere",
        crate::WAIT,
        || (incarnations(&cluster, 0, 1) >= 2 && incarnations(&cluster, 1, 1) >= 2).then_some(()),
    );
    for node in [0, 1] {
        for message in [&lost, &kept] {
            cluster.wait_until(
                &format!("node {node} to hold {message}"),
                crate::WAIT,
                || {
                    (cluster.scalar(
                        node,
                        "SELECT count(*) FROM messages WHERE id = ?1",
                        &[message],
                    ) == Some("1".into()))
                    .then_some(())
                },
            );
        }
    }
    cluster.wait_converged(crate::WAIT);
    let (code, status, _) = cluster.text_as(None, 0, &["sync", "status"]);
    assert_eq!(code, 0);
    assert!(
        status
            .lines()
            .any(|l| l.starts_with("log ") && !l.contains("frozen=-")),
        "the forked log is frozen in A's status:\n{status}"
    );
    assert!(
        cluster
            .scalar(0, "SELECT count(*) FROM journal_forked", &[])
            .is_some_and(|n| n != "0"),
        "the losing branch is kept in journal_forked"
    );

    // `sync resync --from A` on the restored node rebuilds it from A.
    let before = incarnations(&cluster, 1, 1);
    let out = cluster.resync(1, &tag(&cluster, 0));
    assert!(out.status.success(), "{out:?}");
    cluster.wait_until(
        "the resync to move B to a newer incarnation",
        crate::WAIT,
        || (incarnations(&cluster, 1, 1) > before).then_some(()),
    );
    for message in [&lost, &kept] {
        cluster.wait_until(&format!("B to hold {message} again"), crate::WAIT, || {
            (cluster.scalar(1, "SELECT count(*) FROM messages WHERE id = ?1", &[message])
                == Some("1".into()))
            .then_some(())
        });
    }
    cluster.wait_converged(crate::WAIT);
}

/// Configuration 4a (spec §13, root §10 "restore older than a rotation"): B is
/// backed up, authors a tail (one message settled, one left unsettled), rotates
/// by `sync resync`, and is then restored from the pre-rotation backup. It
/// adopts its later incarnation as restore successor, with no clone evidence,
/// and pulls its lineage back: the settled message wakes nothing, the unsettled
/// one is offered once the own-history hold lifts, and the vectors converge.
#[test]
fn restore_older_than_a_rotation_adopts_the_successor() {
    let mut cluster = Cluster::start(
        "mn-restore-old",
        crate::BIN,
        2,
        ClusterOptions {
            // Targets both ways: B needs a target for A for `sync resync --from A`.
            links: vec![(0, 1), (1, 0)],
            gated: true,
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1, P2, P3]);
    cluster.add_host(1, &[P1, P2, P3]);
    let _a = cluster.seat(0, P1);
    let b1 = cluster.seat(1, P1);
    let b2 = cluster.seat(1, P2);
    let b3 = cluster.seat(1, P3);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 4);
    }
    cluster.wait_converged(crate::WAIT);
    // A message to B's second seat that is pending when the backup is taken.
    let (_, pre) = support::send(
        &cluster,
        &b1,
        &[
            "--seat",
            &b2.id,
            "--body",
            "pending at backup",
            "--require-ack",
            &b2.id,
        ],
    );
    support::wait_receipt(&cluster, 1, &pre, &b2, "pending");
    // Its wake is done before the backup, so the backup records it.
    support::wait_wakes(&cluster, 1, P2, 1);
    cluster.wait_converged(crate::WAIT);
    let backup = cluster.backup(1);

    // The tail: that message is settled, another is sent and left unsettled.
    support::ack(&cluster, &b2, &pre);
    let (_, open) = support::send(
        &cluster,
        &b1,
        &[
            "--seat",
            &b3.id,
            "--body",
            "unsettled in the tail",
            "--require-ack",
            &b3.id,
        ],
    );
    support::wait_receipt(&cluster, 1, &open, &b3, "pending");
    cluster.wait_converged(crate::WAIT);

    // B rotates: a resync from A rebuilds it under a new incarnation.
    let a_tag = cluster
        .scalar(0, "SELECT tag FROM local_node", &[])
        .expect("A's tag");
    let out = cluster.resync(1, &a_tag);
    assert!(out.status.success(), "{out:?}");
    cluster.wait_until("B to hold the tail again", crate::WAIT, || {
        (support::receipt(&cluster, 1, &open, &b3).as_deref() == Some("pending")).then_some(())
    });
    cluster.wait_converged(crate::WAIT);
    let rotations = incarnations(&cluster, 1, 1);
    assert!(rotations >= 2, "B has rotated: {rotations}");

    // The pre-rotation backup comes back.
    let wakes_before = cluster.host(1).prompts().len();
    // B's frames are held back after each handshake: it hears what its peer
    // holds of its own lineage, but cannot ask for it. That is the own-history
    // hold, for as long as the test keeps it.
    cluster.hold_frames_from(1);
    cluster.restore(1, &backup);
    cluster.wait_until(
        "A to advertise B's lineage beyond the restored store",
        crate::WAIT,
        || {
            let (_, status, _) = cluster.text_as(None, 1, &["sync", "status"]);
            status
                .lines()
                .filter(|l| l.starts_with("log ") && l.contains(" self "))
                .any(|l| {
                    l.split(' ')
                        .find_map(|f| f.strip_prefix("lag="))
                        .and_then(|n| n.parse::<u64>().ok())
                        .is_some_and(|n| n > 0)
                })
                .then_some(())
        },
    );
    // During the hold the ACK of a message the tail already settled is refused
    // as retryable, and nothing was authored.
    let (code, value, stderr) = cluster.run_as(Some(&b2), 1, &["ack", &pre]);
    assert_ne!(code, 0, "an ACK during the hold is refused: {value}");
    assert!(
        stderr.contains("own history catching up"),
        "{stderr}{value}"
    );
    assert_eq!(
        support::receipt(&cluster, 1, &pre, &b2).as_deref(),
        Some("pending"),
        "the refused ACK left the receipt pending"
    );
    cluster.release_frames_from(1);
    cluster.wait_until("B to pull its lineage back", crate::WAIT, || {
        (support::receipt(&cluster, 1, &open, &b3).is_some()
            && support::receipt(&cluster, 1, &pre, &b2).as_deref() == Some("acked"))
        .then_some(())
    });
    cluster.wait_converged(crate::WAIT);

    // A restore successor, not a clone.
    let (code, status, _) = cluster.text_as(None, 1, &["sync", "status"]);
    assert_eq!(code, 0);
    assert!(
        !status.contains("clone_evidence") && !status.contains("remedy:"),
        "no clone evidence:\n{status}"
    );
    for node in [0, 1] {
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM sync_dial_targets WHERE stopped_reason = 'node_collision'",
                &[]
            ),
            Some("0".into()),
            "node {node}: no node_collision"
        );
    }
    // Settled history wakes nobody; the unsettled message is offered.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        cluster
            .host(1)
            .prompts()
            .iter()
            .skip(wakes_before)
            .all(|(pane, _)| pane != P2),
        "the settled message wakes nothing: {:?}",
        cluster.host(1).prompts()
    );
    assert!(
        support::inbox_ids(&cluster, &b3).contains(&open),
        "the unsettled message is offered once the hold lifts"
    );
    // After the hold the same ACK goes through (here: the tail already holds it).
    let (code, value, stderr) = cluster.run_as(Some(&b2), 1, &["ack", &pre]);
    assert_eq!(code, 0, "an ACK after the hold: {stderr}{value}");
    support::ack(&cluster, &b3, &open);
    support::wait_receipt(&cluster, 0, &open, &b3, "acked");
    cluster.wait_converged(crate::WAIT);
    assert!(
        incarnations(&cluster, 1, 1) > rotations,
        "B rotated once more, naming the newest log of its lineage"
    );
}
