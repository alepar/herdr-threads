//! Configuration 8 (spec §13): discovery end to end. Node A has `sync.discover`
//! on and a fake `herdr` first on PATH whose `machine list --json` names node
//! B's target; A discovers it, links and converges with no `peer add`.

use crate::support::P1;
use herdr_threads::test_support::{
    isolation::TestIsolation,
    multinode::{Cluster, ClusterOptions},
};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn discovery_links_a_node_with_no_peer_add() {
    // The fake `herdr` reads the machine's target from a file next to it, so the
    // target (node B's state dir) can be written once the cluster exists.
    let bin = TestIsolation::new("mn-fake-herdr");
    let dir = bin.path("bin");
    fs::create_dir_all(&dir).unwrap();
    let script = dir.join("herdr");
    fs::write(
        &script,
        "#!/bin/sh\n\
         if [ \"$*\" = \"machine list --json\" ]; then\n\
         printf '[{\"id\":\"1\",\"label\":\"b\",\"target\":\"%s\",\"enabled\":true}]\\n' \
         \"$(cat \"$(dirname \"$0\")/target\")\"\n\
         exit 0\n\
         fi\n\
         exit 2\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut cluster = Cluster::start(
        "mn-discovery",
        crate::BIN,
        2,
        ClusterOptions {
            discover: vec![0],
            env: vec![
                (0, "PATH".into(), path),
                // Passes run while a Herdr host is attached; the first one finds
                // none, so the cadence is short.
                (0, "HT_TEST_DISCOVERY_PERIOD_MS".into(), "200".into()),
            ],
            ..Default::default()
        },
    );
    fs::write(dir.join("target"), cluster.state_dir(1).to_str().unwrap()).unwrap();
    cluster.add_host(0, &[P1]);
    // A CLI call naming the server attaches it, which starts discovery passes.
    cluster.seat(0, P1);
    // No explicit peer was ever added: the only target is the discovered one.
    cluster.wait_until("node A to discover B's target", crate::WAIT, || {
        cluster.scalar(0, "SELECT source FROM sync_dial_targets", &[])
    });
    assert_eq!(
        cluster.scalar(0, "SELECT source FROM sync_dial_targets", &[]),
        Some("herdr_machine".into())
    );
    cluster.wait_converged(crate::WAIT);
    // And the link carries facts: B's genesis reached A.
    assert!(
        cluster
            .vector(0)
            .iter()
            .any(|l| l.0 == cluster.node_uuid(1).to_string())
    );
    cluster.stop();
}
