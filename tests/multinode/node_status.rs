//! Node status (ht-xg88): `peer list` node rows. A peer whose coordinator
//! goes silent behind a live link (ping/pong answered by the link itself)
//! reads `stalled`, never plain `linked`; a peer that stops acknowledging our
//! log reads `stalled tx` (each a test of its own, on shortened test
//! timings); an unreachable peer keeps its last-known version
//! and positions across a restart.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions, Seat};
use std::time::{Duration, Instant};

/// Every frame a peer's coordinator sends; the link's own ping, pong and
/// credit are not among them.
const COORDINATOR_FRAMES: &[&str] = &[
    "have",
    "want",
    "unwant",
    "entries",
    "seen",
    "branch_want",
    "branch_entries",
    "branch_none",
];

fn tag_of(cluster: &Cluster, node: usize) -> String {
    cluster
        .scalar(node, "SELECT tag FROM local_node", &[])
        .expect("the node's tag")
}

/// `node`'s `peer list` row for the peer tagged `tag`.
fn node_row(cluster: &Cluster, node: usize, tag: &str) -> Option<String> {
    let (code, text, stderr) = cluster.text_as(None, node, &["peer", "list"]);
    assert_eq!(code, 0, "peer list on node {node}: {stderr}");
    let prefix = format!("node {tag} ");
    text.lines()
        .find(|line| line.starts_with(&prefix))
        .map(str::to_owned)
}

fn wait_row(
    cluster: &Cluster,
    node: usize,
    tag: &str,
    what: &str,
    ok: impl Fn(&str) -> bool,
) -> String {
    cluster.wait_until(
        &format!("node {node}'s row for {tag}: {what}"),
        crate::WAIT,
        || node_row(cluster, node, tag).filter(|row| ok(row)),
    )
}

fn send(cluster: &Cluster, from: &Seat, to: &Seat, body: &str) {
    support::send(cluster, from, &["--seat", &to.id, "--body", body]);
}

/// Samples `node`'s row for `tag` every 250 ms for `span`: every sample
/// must pass `ok`.
fn hold_for(cluster: &Cluster, node: usize, tag: &str, span: Duration, ok: impl Fn(&str) -> bool) {
    let end = Instant::now() + span;
    while Instant::now() < end {
        let row = node_row(cluster, node, tag).expect("a row while linked");
        if !ok(&row) {
            let (_, status, _) = cluster.text_as(None, node, &["sync", "status"]);
            let log = std::fs::read_to_string(cluster.state_dir(node).join("node/daemon.log"))
                .unwrap_or_default();
            let tail: Vec<&str> = log.lines().rev().take(15).collect();
            panic!("node {node} sampled {row:?}\n{status}\n{}", tail.join("\n"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A gated two-node cluster with `seen` every 300 ms and both stall
/// thresholds at 1.5 s (five missed `seen`), linked and converged, with node
/// 0 holding node 1 as `linked` at its version. Returns the cluster, the two
/// seats and node 1's tag.
fn linked_stall_cluster(label: &str) -> (Cluster, Seat, Seat, String) {
    let mut env = Vec::new();
    for node in [0, 1] {
        for (name, ms) in [
            ("HT_TEST_SEEN_EVERY_MS", "300"),
            ("HT_TEST_APP_SILENT_AFTER_MS", "1500"),
            ("HT_TEST_STALL_AFTER_MS", "1500"),
        ] {
            env.push((node, name.to_owned(), ms.to_owned()));
        }
    }
    let mut cluster = Cluster::start(
        label,
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            gated: true,
            env,
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);
    send(&cluster, &a, &b, "opener");
    cluster.wait_converged(crate::WAIT);
    let tag = tag_of(&cluster, 1);
    let version = format!(" v={} ", env!("CARGO_PKG_VERSION"));
    let row = wait_row(&cluster, 0, &tag, "linked with its version", |row| {
        row.starts_with(&format!("node {tag} linked ")) && row.contains(&version)
    });
    assert!(row.contains(" tx=") && row.contains(" rx="), "{row}");
    (cluster, a, b, tag)
}

// Kills: a wedged peer coordinator behind a live link shown as `linked`
// (liveness from ping/pong alone), app contact counted from pings, and a
// stall that never clears once the peer talks again.
#[test]
fn wedged_peer_coordinator_reads_stalled_never_linked() {
    let (cluster, a, b, tag) = linked_stall_cluster("mn-node-stall-app");
    // The peer's coordinator goes silent; the link's ping/pong still flow.
    cluster.drop_frames_from(1, COORDINATOR_FRAMES);
    send(&cluster, &a, &b, "unacknowledged 1");
    let row = wait_row(&cluster, 0, &tag, "stalled app", |row| {
        row.starts_with(&format!("node {tag} stalled app"))
    });
    assert!(row.contains(" seen="), "{row}");
    send(&cluster, &a, &b, "unacknowledged 2");
    // Past the thresholds it never reads linked while the wedge lasts.
    hold_for(&cluster, 0, &tag, Duration::from_secs(2), |row| {
        row.starts_with(&format!("node {tag} stalled "))
    });
    let row = node_row(&cluster, 0, &tag).unwrap();
    assert!(row.contains("tx"), "nothing acknowledged: {row}");
    let (_, status, _) = cluster.text_as(None, 0, &["sync", "status"]);
    let link = status
        .lines()
        .find(|line| line.starts_with(&format!("link {tag} ")))
        .expect("the link is still up");
    assert!(link.contains(" stalled=app"), "{link}");

    // The coordinator talks again; the next acknowledgement clears it.
    cluster.pass_frames_from(1);
    send(&cluster, &a, &b, "after the wedge");
    wait_row(&cluster, 0, &tag, "linked again", |row| {
        row.starts_with(&format!("node {tag} linked "))
    });
}

// Kills: a peer that stops acknowledging our log (a wedged store behind a
// live coordinator, `seen` still flowing) shown healthy or as an app stall,
// and a stall that never clears after the acknowledgement resumes.
#[test]
fn wedged_peer_store_reads_stalled_tx() {
    let (cluster, a, b, tag) = linked_stall_cluster("mn-node-stall-tx");
    cluster.drop_frames_from(1, &["have", "entries"]);
    send(&cluster, &a, &b, "unacknowledged");
    // Exactly `tx`: `seen` keeps app contact fresh.
    let tx_only = format!("node {tag} stalled tx ");
    wait_row(&cluster, 0, &tag, "stalled tx", |row| {
        row.starts_with(&tx_only)
    });
    hold_for(&cluster, 0, &tag, Duration::from_secs(2), |row| {
        row.starts_with(&tx_only)
    });
    cluster.pass_frames_from(1);
    send(&cluster, &a, &b, "after the store wedge");
    wait_row(&cluster, 0, &tag, "linked again", |row| {
        row.starts_with(&format!("node {tag} linked "))
    });
}

// Kills: an unreachable peer that vanishes from `peer list` or loses its
// last-known version and positions (in-memory only, gone at a restart), and
// one shown `linked` once unreachable.
#[test]
fn unreachable_peer_keeps_last_known_status_across_restart() {
    let mut cluster = support::cluster("mn-node-unreach", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);
    send(&cluster, &a, &b, "before the cut");
    cluster.wait_converged(crate::WAIT);
    let tag = tag_of(&cluster, 1);
    let version = format!(" v={} ", env!("CARGO_PKG_VERSION"));
    wait_row(&cluster, 0, &tag, "linked", |row| {
        row.starts_with(&format!("node {tag} linked ")) && row.contains(&version)
    });
    // The stored shadow holds the version before the cut.
    let uuid = cluster.node_uuid(1).to_string();
    cluster.wait_until("node 0's stored peer status", crate::WAIT, || {
        cluster
            .scalar(
                0,
                "SELECT version FROM sync_peer_status WHERE node_uuid = ?1 AND tx_acked IS NOT NULL",
                &[&uuid],
            )
            .filter(|v| v != "NULL")
    });

    cluster.partition(0, 1);
    cluster.wait_until("node 0's dial to be refused", crate::WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    // Past the reach threshold, after a restart of node 0 (age_contact stops
    // and starts it).
    cluster.age_contact(0, 1, Duration::from_secs(200));
    let row = wait_row(&cluster, 0, &tag, "unreachable", |row| {
        row.starts_with(&format!("node {tag} unreachable since "))
    });
    assert!(row.contains(&version), "last-known version: {row}");
    assert!(
        !row.contains("tx=-") && !row.contains("rx=-"),
        "last-known positions: {row}"
    );
    assert!(!row.contains(" seen="), "no fresh-contact ages: {row}");
    cluster.heal(0, 1);
}
