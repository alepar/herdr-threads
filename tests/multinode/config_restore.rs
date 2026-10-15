//! Configuration 4 (spec §13): a node restored from a store backup that then
//! authors before it learns what its peer holds forks its own log. The fork is
//! detected, the owner re-issues under a new incarnation and the union of both
//! branches converges; `sync resync --from <peer>` then rebuilds the store and
//! the vectors converge again.

use crate::support::{self, P1};

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

#[test]
fn restore_from_backup_forks_is_reissued_and_resync_converges() {
    // B dials A as well as A dialing B, so B has a path to A for the resync.
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
