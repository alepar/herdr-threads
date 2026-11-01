//! N7 (ht-kqrt.33): a named seat crosses nodes by `seat.created.display_name` alone. The
//! other node materializes its name, lists it, and addresses it as `name@node`.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};

// Kills: a `display_name` that is not published or not materialized (the name is then unknown on
// the other node), a `name@node` selector that only reads pane paths, and a directory that shows a
// remote named seat as nameless.
#[test]
fn reviewer_at_b_addresses_and_displays() {
    let mut cluster = Cluster::start(
        "mn-named-seats",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    let a = cluster.seat(0, P1);
    let tag_b: String = cluster
        .scalar(1, "SELECT tag FROM local_node", &[])
        .expect("node B's tag");

    // The seat is named before its first enrollment publishes it.
    let reviewer = cluster.named_seat(1, P1, "reviewer").id;
    cluster.wait_converged(crate::WAIT);
    support::wait_seats(&cluster, 0, 2);

    // A materialized the name from the fact, not from anything B said afterwards.
    assert_eq!(
        cluster
            .scalar(0, "SELECT name FROM seats WHERE id=?1", &[&reviewer])
            .as_deref(),
        Some("reviewer")
    );

    // `reviewer@B` addresses the seat.
    let (code, text, stderr) = cluster.text_as(
        Some(&a),
        0,
        &[
            "send",
            "--seat",
            &format!("reviewer@{tag_b}"),
            "--body",
            "ping",
        ],
    );
    assert_eq!(code, 0, "{stderr}{text}");
    assert!(
        stderr.contains(&format!("reviewer@{tag_b}")) && stderr.contains(&reviewer),
        "{stderr}"
    );
    let delivered = cluster.wait_until("the message to reach reviewer on B", crate::WAIT, || {
        cluster
            .scalar(1, "SELECT count(*) FROM messages WHERE body='ping'", &[])
            .filter(|n| n != "0")
    });
    assert_eq!(delivered, "1");

    // An unknown name on B is a refusal that sends nothing, not a guess.
    let (code, _, stderr) = cluster.text_as(
        Some(&a),
        0,
        &["send", "--seat", &format!("nobody@{tag_b}"), "--body", "x"],
    );
    assert_ne!(code, 0, "{stderr}");
    assert_eq!(
        cluster
            .scalar(1, "SELECT count(*) FROM messages WHERE body='x'", &[])
            .as_deref(),
        Some("0")
    );

    // The directory shows the name once, as the pane part, with `-` workspace and tab.
    let (code, text, stderr) = cluster.text_as(None, 0, &["seat", "list"]);
    assert_eq!(code, 0, "{stderr}");
    let row = text
        .lines()
        .find(|l| l.starts_with(&format!("{reviewer} ")))
        .unwrap_or_else(|| panic!("{text}"));
    assert!(row.contains("reviewer"), "{row}");
    assert_eq!(row.matches("reviewer").count(), 1, "{row}");
}
