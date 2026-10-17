//! Configuration "dial each other at start" (ht-5fr.5.29): both nodes hold a
//! dial target for the other, so two links race at every start (discovery is
//! on by default, this is the normal topology). A link the peer promotes with a
//! `want` or a delta `have` must ask for the peer's vector; otherwise one side
//! wants nothing until the 90 s dead timer. A 20 s bound per round catches that
//! stall without flaking on a slow machine.

use crate::{FutureKind, held, note};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};
use std::time::Duration;

#[test]
fn both_nodes_dial_each_other_and_pull_within_seconds() {
    let mut cluster = Cluster::start(
        "mn-dial-each",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1), (1, 0)],
            ..Default::default()
        },
    );
    cluster.wait_converged(Duration::from_secs(20));
    for round in 1..=5 {
        cluster.stop_node(0);
        cluster.stop_node(1);
        // Both down while authoring, so neither restarts around the write.
        let (c0, _) = cluster.author_kind::<FutureKind>(0, &note(&format!("round {round} a")));
        let (c1, _) = cluster.author_kind::<FutureKind>(1, &note(&format!("round {round} b")));
        cluster.start_node(0);
        cluster.start_node(1);
        cluster.wait_until(
            &format!("round {round}: each node to pull the other's new entry"),
            Duration::from_secs(20),
            || {
                (held(&cluster, 0, 1) >= c1 as i64 && held(&cluster, 1, 0) >= c0 as i64)
                    .then_some(())
            },
        );
    }
}
