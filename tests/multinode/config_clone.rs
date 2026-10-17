//! Configuration 12 (spec §13, root §10 clone gate): B's node directory is
//! copied to a second state dir B' (same key) and both daemons link to C. C
//! closes the newer link `node_collision`; the closed copy and C show the clone
//! evidence and the remedy; nothing rotates on the copy holding evidence. After
//! B' is moved aside (it restarts as a new node) and `sync clone-cleared` runs
//! on B, B's owner path re-issues what B' authored and the vectors converge.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::Cluster;
use std::time::Duration;

fn status(cluster: &Cluster, node: usize) -> String {
    cluster.text_as(None, node, &["sync", "status"]).1
}

fn incarnations_of_b(cluster: &Cluster, at: usize, b: usize) -> i64 {
    cluster
        .scalar(
            at,
            "SELECT count(*) FROM sync_logs WHERE node_uuid = ?1",
            &[&cluster.node_uuid(b).to_string()],
        )
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn cloned_node_directory_is_detected_and_repaired_by_the_operator() {
    let mut cluster = support::cluster("mn-clone", 2, &[(0, 1)], &[0, 1]);
    let b = cluster.seat(0, P1);
    let c = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);

    // The copy of B's directory; both copies are down. The copy links first.
    let copy = cluster.clone_node(0);
    let b_copy = b.on_node(copy);
    cluster.start_node(copy);
    cluster.wait_until("the copy to link to C", crate::WAIT, || {
        (cluster.target_state(copy).as_deref() == Some("linked")).then_some(())
    });
    let (_, from_copy) = support::send(
        &cluster,
        &b_copy,
        &["--seat", &c.id, "--body", "authored on the copy"],
    );
    cluster.wait_until("C to hold what the copy authored", crate::WAIT, || {
        cluster.scalar(1, "SELECT id FROM messages WHERE id = ?1", &[&from_copy])
    });

    // The original starts: its link is the newer one and C closes it.
    cluster.start_node(0);
    cluster.wait_until("C to record clone evidence about B", crate::WAIT, || {
        status(&cluster, 1).contains("clone_evidence").then_some(())
    });
    cluster.wait_until(
        "the closed copy to record evidence about itself",
        crate::WAIT,
        || status(&cluster, 0).contains("clone_evidence").then_some(()),
    );
    // C reports the evidence it saw (third party); the remedy is printed by the
    // copy that holds evidence about itself.
    let at_c = status(&cluster, 1);
    assert!(
        at_c.contains("clone_evidence") && !at_c.contains("remedy: "),
        "C shows the evidence, not a remedy:\n{at_c}"
    );
    let at_copy = status(&cluster, 0);
    assert_eq!(
        at_copy.matches("remedy: ").count(),
        2,
        "the closed copy prints both remedies:\n{at_copy}"
    );
    assert_eq!(
        cluster
            .scalar(0, "SELECT stopped_reason FROM sync_dial_targets", &[])
            .as_deref(),
        Some("node_collision"),
        "the closed copy stops dialing"
    );
    // Authored on the closed copy, divergent from what C holds.
    let (_, from_original) = support::send(
        &cluster,
        &b,
        &["--seat", &c.id, "--body", "authored on the original"],
    );
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        incarnations_of_b(&cluster, 0, 0),
        1,
        "no automatic rotation on the copy holding evidence"
    );
    assert_eq!(
        cluster.scalar(
            1,
            "SELECT id FROM messages WHERE id = ?1",
            &[&from_original]
        ),
        None,
        "nothing of the closed copy reached C"
    );

    // The operator moves the other copy aside (it starts as a new node) and
    // clears the evidence on B, then re-adds the target `node_collision` stopped.
    let new_node = cluster.restart_as_new_node(copy);
    assert_ne!(new_node, cluster.node_uuid(0), "the copy is a new node");
    let cleared = cluster.human(0, &["sync", "clone-cleared"]);
    assert!(cleared.status.success(), "{cleared:?}");
    let c_dest = cluster.state_dir(1).to_str().unwrap().to_owned();
    let added = cluster.human(0, &["peer", "add", &c_dest]);
    assert!(added.status.success(), "{added:?}");

    // B's owner path re-issues the facts of both branches and B and C agree.
    for message in [&from_copy, &from_original] {
        for node in [0, 1] {
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
    cluster.wait_until("B and C to agree", crate::WAIT, || {
        (cluster.vector(0) == cluster.vector(1)).then_some(())
    });
    assert!(
        incarnations_of_b(&cluster, 1, 0) >= 2,
        "B rotated once cleared: the old incarnation is frozen everywhere"
    );
    assert!(
        !status(&cluster, 0).contains("clone_evidence"),
        "the evidence is cleared:\n{}",
        status(&cluster, 0)
    );
}
