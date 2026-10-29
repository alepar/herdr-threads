//! Peer hygiene (2.8.0): `peer prune` forgets a dead node island and keeps a
//! dead node a live one references (TRUST-POLICY N16); the forgotten node
//! leaves the views and is never pulled again, while the live link carries
//! on; `peer events` records the link history.

use crate::support::{self, P1, P2};
use herdr_threads::test_support::multinode::Cluster;
use std::time::Duration;

const EIGHT_DAYS: Duration = Duration::from_secs(8 * 24 * 3_600);

fn tag_of(cluster: &Cluster, node: usize) -> String {
    cluster
        .scalar(node, "SELECT tag FROM local_node", &[])
        .expect("the node's tag")
}

fn text(cluster: &Cluster, node: usize, args: &[&str]) -> String {
    let (code, text, stderr) = cluster.text_as(None, node, args);
    assert_eq!(code, 0, "{args:?} on node {node}: {stderr}{text}");
    text
}

fn human(cluster: &Cluster, node: usize, args: &[&str]) -> String {
    let output = cluster.human(node, args);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "human {args:?} on node {node}: {}{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

fn line_with<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find(|line| line.starts_with(prefix))
}

/// The contiguous counter of `author`'s log as `node` holds it.
fn held(cluster: &Cluster, node: usize, author: usize) -> i64 {
    let uuid = cluster.node_uuid(author).to_string();
    cluster
        .scalar(
            node,
            "SELECT COALESCE(MAX(contiguous_counter), 0) FROM sync_logs WHERE node_uuid = ?1",
            &[&uuid],
        )
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

// Kills: a prune that forgets a node a live node references, one that writes
// on a dry run, a forgotten node still listed or still pulled, a prune that
// disturbs the live link, and a link history that misses ups and downs.
#[test]
fn prune_forgets_a_dead_island_and_keeps_referenced_nodes() {
    // Node 0 operates; 1 stays live; 2 is a dead island (its seats talk only
    // to each other); 3 is dead but messaged node 1's seat.
    let mut cluster =
        support::cluster("mn-peer-prune", 4, &[(0, 1), (0, 2), (0, 3)], &[0, 1, 2, 3]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let c = cluster.seat(2, P1);
    let c2 = cluster.seat(2, P2);
    let d = cluster.seat(3, P1);
    for node in 0..4 {
        support::wait_seats(&cluster, node, 5);
    }
    support::send(&cluster, &a, &["--seat", &b.id, "--body", "live"]);
    support::send(&cluster, &c, &["--seat", &c2.id, "--body", "island"]);
    support::send(&cluster, &d, &["--seat", &b.id, "--body", "referenced"]);
    cluster.wait_converged(crate::WAIT);
    let (live, island, referenced) = (
        tag_of(&cluster, 1),
        tag_of(&cluster, 2),
        tag_of(&cluster, 3),
    );

    // Nodes 2 and 3 go away for good: node 0 stops dialing them, and every
    // remaining node last heard from them eight days ago.
    cluster.stop_node(2);
    cluster.stop_node(3);
    human(&cluster, 0, &["peer", "forget", &island]);
    human(&cluster, 0, &["peer", "forget", &referenced]);
    // Node 1 is down while the contacts age, so no gossip refreshes them.
    cluster.stop_node(1);
    for node in [1, 0] {
        for about in [2, 3] {
            cluster.age_contact(node, about, EIGHT_DAYS);
        }
    }
    cluster.start_node(1);
    cluster.wait_until("node 0 linked to node 1 again", crate::WAIT, || {
        line_with(
            &text(&cluster, 0, &["peer", "list"]),
            &format!("node {live} linked "),
        )
        .map(|_| ())
    });

    // The dry run names the island only, and writes nothing.
    let dry = text(&cluster, 0, &["peer", "prune"]);
    assert!(dry.starts_with("peer_prune "), "{dry}");
    assert!(dry.lines().next().unwrap().ends_with(" dry_run"), "{dry}");
    assert!(
        line_with(&dry, &format!("node {island} prune ")).is_some(),
        "{dry}"
    );
    // Its island is the live one: named by node 0 or node 1.
    let kept = line_with(&dry, &format!("node {referenced} keep ")).expect("node 3 kept");
    let operator = tag_of(&cluster, 0);
    assert!(
        kept.ends_with(&format!(": referenced_by {live}"))
            || kept.ends_with(&format!(": referenced_by {operator}")),
        "{kept}"
    );
    let linked = line_with(&dry, &format!("node {live} keep ")).expect("node 1 kept");
    assert!(linked.ends_with(": linked"), "{linked}");
    assert!(dry.contains("prunable 1 kept 2\n"), "{dry}");
    assert!(
        line_with(&dry, "next: ").is_some_and(|next| next.contains("human peer prune --yes")),
        "{dry}"
    );
    assert_eq!(
        cluster
            .scalar(0, "SELECT COUNT(*) FROM sync_forgotten_nodes", &[])
            .as_deref(),
        Some("0"),
        "a dry run writes nothing"
    );

    let applied = human(&cluster, 0, &["peer", "prune", "--yes"]);
    assert!(
        applied.lines().next().unwrap().ends_with(" applied"),
        "{applied}"
    );
    assert!(
        line_with(&applied, &format!("node {island} pruned ")).is_some(),
        "{applied}"
    );
    let list = text(&cluster, 0, &["peer", "prune", "--list"]);
    let row = line_with(&list, &format!("node {island} forgotten ")).expect("the tombstone");
    assert!(row.contains(": operator:local-user:"), "{row}");

    // The views leave it out; node 3 and the live node stay.
    let peers = text(&cluster, 0, &["peer", "list"]);
    assert!(
        line_with(&peers, &format!("node {island} ")).is_none(),
        "{peers}"
    );
    assert!(
        line_with(&peers, &format!("node {referenced} ")).is_some(),
        "{peers}"
    );
    assert!(peers.contains("\nforgotten 1\n"), "{peers}");
    let status = text(&cluster, 0, &["sync", "status"]);
    assert!(
        line_with(&status, &format!("log {island}/")).is_none(),
        "{status}"
    );
    assert!(
        line_with(&status, &format!("log {referenced}/")).is_some(),
        "{status}"
    );
    assert!(status.contains("\nforgotten 1\n"), "{status}");

    // The live link carries on.
    let (_, message) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "after"]);
    cluster.wait_until("node 1 to get the message", crate::WAIT, || {
        support::inbox_ids(&cluster, &b)
            .contains(&message)
            .then_some(())
    });

    // A prune that raced a link-up: the live node 1's tombstone lands while
    // its link is already up (forced here by writing it directly). The next
    // local read sees a linked node forgotten and unforgets it.
    let uuid1 = cluster.node_uuid(1).to_string();
    cluster.write_sql(
        0,
        &format!(
            "INSERT INTO sync_forgotten_nodes(node_uuid,tag,host_label,forgotten_at,provenance) \
             VALUES ('{uuid1}','{live}',NULL,1,'operator:local-user:1')"
        ),
    );
    human(
        &cluster,
        0,
        &["peer", "prune", "--yes", "--host", "no-such-host"],
    );
    cluster.wait_until("node 0 to unforget the linked node 1", crate::WAIT, || {
        let events = text(&cluster, 0, &["peer", "events", "--node", &live]);
        (events
            .lines()
            .any(|line| line.contains(&format!(" {live} unforgotten ")))
            && forgotten_rows(&cluster) == 1)
            .then_some(())
    });

    // The operator undo while the island is away: a dry run names the
    // tombstone and writes nothing; `--yes` removes it and the node is shown
    // again; a second one has nothing to remove. Then it is pruned again.
    let dry = text(&cluster, 0, &["peer", "unforget", &island]);
    assert!(
        dry.starts_with(&format!("peer_unforget {island} would_restore ")),
        "{dry}"
    );
    assert!(
        line_with(&dry, "next: ")
            .is_some_and(|next| next.contains("human peer unforget ") && next.ends_with(" --yes")),
        "{dry}"
    );
    assert_eq!(forgotten_rows(&cluster), 1, "a dry run removes nothing");
    let undone = human(&cluster, 0, &["peer", "unforget", &island, "--yes"]);
    assert!(
        undone.starts_with(&format!("peer_unforget {island} restored ")),
        "{undone}"
    );
    let peers = text(&cluster, 0, &["peer", "list"]);
    assert!(
        line_with(&peers, &format!("node {island} ")).is_some(),
        "{peers}"
    );
    assert!(!peers.contains("\nforgotten "), "{peers}");
    let gone = cluster.human(0, &["peer", "unforget", &island, "--yes"]);
    assert!(!gone.status.success(), "no tombstone left to remove");
    let again = human(&cluster, 0, &["peer", "prune", "--yes"]);
    assert!(
        line_with(&again, &format!("node {island} pruned ")).is_some(),
        "{again}"
    );
    assert_eq!(forgotten_rows(&cluster), 1);

    // The island returns linked only to node 1: relayed contact proves
    // nothing, so node 0 keeps it forgotten and never pulls what it authors.
    let before = held(&cluster, 0, 2);
    cluster.start_node(2);
    cluster.link(2, 1);
    support::send(
        &cluster,
        &c,
        &["--seat", &c2.id, "--body", "after the prune"],
    );
    cluster.wait_until("node 1 to pull node 2's new entry", crate::WAIT, || {
        let authored = held(&cluster, 2, 2);
        (authored > before && held(&cluster, 1, 2) == authored).then_some(())
    });
    for _ in 0..12 {
        assert_eq!(
            held(&cluster, 0, 2),
            before,
            "a forgotten log is never pulled"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(forgotten_rows(&cluster), 1);

    // Its direct link proves it alive: node 0 drops the tombstone by itself
    // and pulls the log.
    cluster.link(2, 0);
    cluster.wait_until("node 0 to unforget node 2 on its link", crate::WAIT, || {
        let events = text(&cluster, 0, &["peer", "events", "--node", &island]);
        events
            .lines()
            .any(|line| line.contains(&format!(" {island} unforgotten ")))
            .then_some(())
    });
    assert_eq!(forgotten_rows(&cluster), 0);
    cluster.wait_until("node 0 to pull node 2's log again", crate::WAIT, || {
        (held(&cluster, 0, 2) == held(&cluster, 2, 2)).then_some(())
    });
    let peers = text(&cluster, 0, &["peer", "list"]);
    assert!(
        line_with(&peers, &format!("node {island} linked ")).is_some(),
        "{peers}"
    );

    // The history: the dialed link to node 2 went down, the inbound one came
    // up; a short page continues with `next:`.
    // The coordinator writes its events on the next 1 s tick.
    cluster.wait_until("the link history of node 2", crate::WAIT, || {
        let events = text(&cluster, 0, &["peer", "events", "--node", &island]);
        assert!(events.starts_with("peer_events\n"), "{events}");
        [" down dialed", " up inbound"]
            .iter()
            .all(|kind| {
                events
                    .lines()
                    .any(|line| line.contains(&format!(" {island}{kind}")))
            })
            .then_some(())
    });
    let page = text(
        &cluster,
        0,
        &["peer", "events", "--node", &island, "--limit", "1"],
    );
    assert_eq!(
        page.lines().filter(|l| l.starts_with("event ")).count(),
        1,
        "{page}"
    );
    let next = line_with(&page, "next: ").expect("a next line");
    assert!(
        next.contains("peer events --before ")
            && next.contains(&format!("--node {island}"))
            && next.contains("--limit 1"),
        "{next}"
    );
}

fn forgotten_rows(cluster: &Cluster) -> i64 {
    cluster
        .scalar(0, "SELECT COUNT(*) FROM sync_forgotten_nodes", &[])
        .and_then(|v| v.parse().ok())
        .unwrap_or(-1)
}
