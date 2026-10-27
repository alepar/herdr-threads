//! ht-5fr.10: the multi-node harness (`test_support::multinode`) proves itself:
//! real daemons link through the dial shim, converge, survive a partition and
//! heal, and relay an entry of a kind no daemon knows unchanged.
//!
//! Own target: every node is a spawned daemon, and the harness drives them
//! through one shim directory per cluster (nothing process-wide), but the suite
//! runs long real-time waits that must not share a binary with others.

use herdr_threads::sync::{
    EntityKey,
    facts::{ApplyContext, FactError, FactKind},
};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
pub(crate) const WAIT: Duration = Duration::from_secs(90);

/// A kind of a "newer" software version: registered only where it is authored.
#[derive(Serialize, Deserialize)]
pub(crate) struct FutureNote {
    text: String,
}

pub(crate) struct FutureKind;

impl FactKind for FutureKind {
    const NAME: &'static str = "future.note";
    const MAJOR: u32 = 1;
    type Payload = FutureNote;
    fn creates(_: &FutureNote) -> Option<EntityKey> {
        None
    }
    fn needs(_: &FutureNote) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(
        _: &Transaction<'_>,
        _: &mut ApplyContext<'_>,
        _: FutureNote,
    ) -> Result<(), FactError> {
        Ok(())
    }
}

pub(crate) fn note(text: &str) -> FutureNote {
    FutureNote { text: text.into() }
}

/// The contiguous counter of `author`'s log as `node` holds it.
fn held(cluster: &Cluster, node: usize, author: usize) -> i64 {
    let uuid = cluster.node_uuid(author).to_string();
    cluster
        .vector(node)
        .iter()
        .find(|l| l.0 == uuid)
        .map_or(0, |l| l.2)
}

#[test]
fn two_nodes_link_and_converge() {
    let cluster = Cluster::start(
        "mn-converge",
        BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            ..Default::default()
        },
    );
    let held = cluster.wait_converged(WAIT);
    assert_eq!(held.len(), 2, "one genesis log per node: {held:?}");
    assert!(held.iter().all(|l| l.2 >= 1), "genesis held: {held:?}");
    cluster.wait_until("the dial target to link", WAIT, || {
        (cluster.target_state(0).as_deref() == Some("linked")).then_some(())
    });
    // The CLI driver reaches each node's own daemon.
    for node in 0..2 {
        let out = cluster.cli(node, &["daemon", "health"]);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("boot_id: "),
            "node {node}: {out:?}"
        );
    }
}

#[test]
fn partition_then_heal_converges() {
    let mut cluster = Cluster::start(
        "mn-partition",
        BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            ..Default::default()
        },
    );
    cluster.wait_converged(WAIT);
    // Linked before the cut, so a later `backoff` can only be a refused dial.
    cluster.wait_until("the dial target to link before the cut", WAIT, || {
        (cluster.target_state(0).as_deref() == Some("linked")).then_some(())
    });
    cluster.partition(0, 1);
    // Authored during the partition; node 0 restarts and its dials are refused.
    let (counter, _) = cluster.author_kind::<FutureKind>(0, &note("while cut"));
    // Node 0 (the dialer) tried to reach node 1 and was refused: a fixed sleep
    // proved nothing about whether a dial had happened.
    cluster.wait_until("node 0's dial to be refused while cut", WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    assert_eq!(
        held(&cluster, 0, 0),
        counter as i64,
        "node 0 holds its entry"
    );
    assert!(
        held(&cluster, 1, 0) < counter as i64,
        "node 1 must not see an entry authored behind the partition"
    );
    assert!(
        cluster.converged().is_none(),
        "the vectors differ while cut"
    );
    cluster.heal(0, 1);
    cluster.wait_converged(WAIT);
    assert_eq!(held(&cluster, 1, 0), counter as i64);
}

#[test]
fn unknown_kind_authored_and_relayed_unchanged() {
    // 0 and 2 both dial 1: an entry authored on 0 reaches 2 only through 1.
    let mut cluster = Cluster::start(
        "mn-unknown",
        BIN,
        3,
        ClusterOptions {
            links: vec![(0, 1), (2, 1)],
            ..Default::default()
        },
    );
    cluster.wait_converged(WAIT);
    let (counter, hash) = cluster.author_kind::<FutureKind>(0, &note("from the future"));
    cluster.wait_converged(WAIT);
    let authored = cluster
        .journal_entry(0, 0, counter)
        .expect("node 0 holds its entry");
    assert_eq!(authored.kind, "future.note");
    assert_eq!(
        authored.hash,
        hash.to_vec(),
        "the stored hash is the authored one"
    );
    for node in [1, 2] {
        let relayed = cluster
            .journal_entry(node, 0, counter)
            .unwrap_or_else(|| panic!("node {node} holds the entry"));
        assert_eq!(relayed.kind, "future.note");
        assert_eq!(
            relayed.hash,
            hash.to_vec(),
            "node {node}: relayed unchanged"
        );
        assert_eq!(
            relayed.apply_state, "parked_unknown",
            "node {node} cannot apply a kind it does not know"
        );
    }
}

#[path = "multinode/support.rs"]
mod support;

#[path = "multinode/config_direct.rs"]
mod config_direct;

#[path = "multinode/config_auto_ack.rs"]
mod config_auto_ack;

#[path = "multinode/config_relay.rs"]
mod config_relay;

#[path = "multinode/config_pair.rs"]
mod config_pair;

#[path = "multinode/config_invite.rs"]
mod config_invite;

#[path = "multinode/config_store_only.rs"]
mod config_store_only;

#[path = "multinode/config_partition.rs"]
mod config_partition;

#[path = "multinode/config_dial_each_other.rs"]
mod config_dial_each_other;

#[path = "multinode/config_mixed_versions.rs"]
mod config_mixed_versions;

#[path = "multinode/hermes_composer.rs"]
mod hermes_composer;
#[path = "multinode/styled_composer.rs"]
mod styled_composer;

#[path = "multinode/config_two_hosts.rs"]
mod config_two_hosts;

#[path = "multinode/config_discovery.rs"]
mod config_discovery;

#[path = "multinode/config_delivery.rs"]
mod config_delivery;

#[path = "multinode/node_status.rs"]
mod node_status;

#[path = "multinode/config_restore.rs"]
mod config_restore;

#[path = "multinode/config_wipe.rs"]
mod config_wipe;

#[path = "multinode/config_clone.rs"]
mod config_clone;

#[path = "multinode/sweep.rs"]
mod sweep;

#[path = "multinode/config_user_intent.rs"]
mod config_user_intent;
