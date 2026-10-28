//! Seat directory (ht-xg88 phase 3): `seat list`, `thread participants` and
//! `seat inspect` show a remote seat's node, advisory presence, harness and
//! labels from its home node, filterable; once that node is unreachable they
//! show it unreachable, never a stale presence, with last-known labels.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};
use std::time::Duration;

fn tag_of(cluster: &Cluster, node: usize) -> String {
    cluster
        .scalar(node, "SELECT tag FROM local_node", &[])
        .expect("the node's tag")
}

/// `node`'s compact `seat list ARGS` row for `seat`.
fn seat_row(cluster: &Cluster, node: usize, args: &[&str], seat: &str) -> Option<String> {
    let mut argv = vec!["seat", "list"];
    argv.extend_from_slice(args);
    let (code, text, stderr) = cluster.text_as(None, node, &argv);
    assert_eq!(code, 0, "seat list {args:?} on node {node}: {stderr}");
    text.lines()
        .find(|line| line.starts_with(&format!("{seat} ")))
        .map(str::to_owned)
}

// Kills: a remote seat listed without its node, presence, harness or labels;
// filters that do not narrow; `--label` that misses the local seat; participants and inspect still saying `remote;
// labels unavailable` or showing a presence once the home node is
// unreachable.
#[test]
fn directory_shows_remote_seats_as_fresh_as_their_node() {
    let mut env = Vec::new();
    for node in [0, 1] {
        env.push((node, "HT_TEST_SEAT_CHECK_MS".to_owned(), "200".to_owned()));
    }
    let mut cluster = Cluster::start(
        "mn-seat-directory",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
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
    let (thread, _) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "hello"]);
    let tag = tag_of(&cluster, 1);
    let labels = ": w1 / w1:t1 / w1:p1";

    let row = cluster.wait_until("node 0 to list b with its presence", crate::WAIT, || {
        seat_row(&cluster, 0, &[], &b.id).filter(|row| {
            row.starts_with(&format!("{} {tag} idle claude ", b.id))
                || row.starts_with(&format!("{} {tag} busy claude ", b.id))
        })
    });
    assert!(row.ends_with(labels), "{row}");
    // Filters narrow to it, or away from it.
    assert!(seat_row(&cluster, 0, &["--node", &tag], &b.id).is_some());
    assert!(seat_row(&cluster, 0, &["--node", &tag], &a.id).is_none());
    assert!(
        seat_row(
            &cluster,
            0,
            &["--harness", "claude", "--label", "w1:P1"],
            &b.id
        )
        .is_some()
    );
    // `--label` matches the local seat too, by this node's own host labels
    // (ht-hldp); a label neither has matches neither.
    assert!(seat_row(&cluster, 0, &["--label", "w1:P1"], &a.id).is_some());
    assert!(seat_row(&cluster, 0, &["--label", "w1:p9"], &a.id).is_none());
    assert!(seat_row(&cluster, 0, &["--label", "w1:p9"], &b.id).is_none());
    assert!(seat_row(&cluster, 0, &["--local"], &b.id).is_none());
    assert!(seat_row(&cluster, 0, &["--local"], &a.id).is_some());
    assert!(seat_row(&cluster, 0, &["--harness", "codex"], &b.id).is_none());

    let (code, text, stderr) = cluster.text_as(Some(&a), 0, &["thread", "participants", &thread]);
    assert_eq!(code, 0, "{stderr}");
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("{} ", b.id)))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.contains(" remote idle claude last=") || line.contains(" remote busy claude last="),
        "{text}"
    );
    assert!(text.contains("w1 / w1:t1 / w1:p1 [advisory]"), "{text}");
    assert!(!text.contains("labels unavailable"), "{text}");

    let (code, text, stderr) = cluster.text_as(None, 0, &["seat", "inspect", &b.id]);
    assert_eq!(code, 0, "{stderr}");
    let line = text
        .lines()
        .find(|l| l.starts_with("directory: "))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(
        line.starts_with(&format!("directory: {tag} ")) && line.ends_with(labels),
        "{line}"
    );

    // The home node goes away past the reach threshold (node 0 restarts).
    cluster.partition(0, 1);
    cluster.wait_until("node 0's dial to be refused", crate::WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    cluster.age_contact(0, 1, Duration::from_secs(200));
    let row = seat_row(&cluster, 0, &[], &b.id).expect("b still listed");
    assert!(
        row.starts_with(&format!("{} {tag} unreachable since ", b.id)) && row.ends_with(labels),
        "{row}"
    );
    let (code, text, stderr) = cluster.text_as(Some(&a), 0, &["thread", "participants", &thread]);
    assert_eq!(code, 0, "{stderr}");
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("{} ", b.id)))
        .unwrap();
    assert!(!line.contains(" idle") && !line.contains(" busy"), "{text}");
    assert!(
        text.contains(&format!("node {tag} unreachable since ")),
        "{text}"
    );
    assert!(
        text.contains("w1 / w1:t1 / w1:p1 [advisory, last known]"),
        "{text}"
    );
    cluster.heal(0, 1);
}
