//! Three-node relay of an opaque entry and the stall it leaves (ht-j87z.5).
//!
//! An author's log `[genesis, HTJRNL99 entry 2, known 3]` and a dependent log
//! whose entry 2 depends on the author's entry 2 travel source -> relay -> third
//! node. Promotion of a stored opaque entry is deferred to ht-evlm, so the stall
//! (`blocked_on = opaque_layout`, dependents on the `deps` string) is permanent
//! in this release, also across a restart.
//!
//! The runtime is `test_support::sync_node::SyncNode` over duplex links, as in
//! `opaque_wire`. No process is spawned and no process-wide state is kept,
//! which is why the module lives in `combined`.

use std::sync::Arc;
use std::time::Duration;

use herdr_threads::node::NodeIdentity;
use herdr_threads::protocol::time::{CallBudget, Cancellation, MonoInstant};
use herdr_threads::sync::LogKey;
use herdr_threads::sync::envelope::{Deps, StoredEntry, UnsignedEnvelope};
use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::sync::journal_port::JournalPort;
use herdr_threads::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
use herdr_threads::test_support::sync_node::{SyncNode, SyncNodeOptions, wait_for, wait_until};

use crate::facts_receipt::{ingest, origin_entries};
use crate::facts_seat::{Node, registry};

const PATIENCE: Duration = Duration::from_secs(60);

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn open_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// The contiguous counter of `log` on `node`, 0 when the log is not held.
fn contiguous(node: &Node, log: &LogKey) -> u64 {
    let sql = format!(
        "SELECT contiguous_counter FROM sync_logs WHERE node_uuid='{}' AND incarnation={}",
        log.node, log.incarnation
    );
    node.query(&sql, |r| r.get::<_, i64>(0))
        .first()
        .map_or(0, |c| u64::try_from(*c).unwrap_or(0))
}

fn applied_through(node: &Node, log: &LogKey) -> i64 {
    let sql = format!(
        "SELECT applied_through FROM sync_logs WHERE node_uuid='{}' AND incarnation={}",
        log.node, log.incarnation
    );
    node.query(&sql, |r| r.get::<_, i64>(0))
        .first()
        .copied()
        .unwrap_or(-1)
}

fn blocked_on(node: &Node, log: &LogKey) -> Option<String> {
    let sql = format!(
        "SELECT blocked_on FROM sync_logs WHERE node_uuid='{}' AND incarnation={}",
        log.node, log.incarnation
    );
    node.query(&sql, |r| r.get::<_, Option<String>>(0))
        .into_iter()
        .next()
        .flatten()
}

fn scalar(node: &Node, sql: &str) -> Option<String> {
    node.query(sql, |r| r.get::<_, String>(0))
        .into_iter()
        .next()
}

/// `apply_state` of the journal row at `counter` of `log`.
fn apply_state(node: &Node, log: &LogKey, counter: u64) -> Option<String> {
    scalar(
        node,
        &format!(
            "SELECT apply_state FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter={counter}",
            log.node, log.incarnation
        ),
    )
}

/// `(raw, hash, signature)` of the main-chain opaque row at `counter`.
fn opaque_row(node: &Node, log: &LogKey, counter: u64) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let sql = format!(
        "SELECT o.raw,o.hash,o.signature FROM journal o \
         JOIN sync_logs l ON l.log_ix=o.log_ix \
         WHERE l.node_uuid='{}' AND l.incarnation={} AND o.counter={counter} AND o.kind IS NULL",
        log.node, log.incarnation
    );
    node.query(&sql, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .into_iter()
        .next()
}

fn start_capless(node: &Node, n: u8, boot: u128) -> SyncNode {
    let journal: Arc<dyn JournalPort> = node.store.clone();
    SyncNode::start_with(
        node.store.clone(),
        journal,
        seed(n),
        boot,
        SyncNodeOptions {
            advertise_opaque: false,
        },
    )
}

/// The origin log `[genesis, opaque 2, known 3]` and a dependent log
/// `[genesis, entry 2 depending on origin@2]`.
struct Fixture {
    log: LogKey,
    entries: Vec<StoredEntry>,
    opaque: herdr_threads::sync::envelope::OpaqueEntry,
    q: LogKey,
    q_entries: Vec<StoredEntry>,
    deps_text: String,
}

fn relay_fixture(origin_seed: u8, dependent_seed: u8) -> Fixture {
    let origin = Node::with(origin_seed, registry());
    let (log, mut entries) = origin_entries(&origin, origin.id).remove(0);
    entries.truncate(1);
    let signer = NodeIdentity::from_seed(seed(origin_seed));
    let opaque = synthetic_entry(
        &signer,
        &synthetic_unsigned(&signer, log.incarnation, 2, entries[0].hash()),
    );
    let three = synthetic_unsigned(&signer, log.incarnation, 3, opaque.hash()).seal(&signer);
    entries.push(StoredEntry::from(opaque.clone()));
    entries.push(StoredEntry::from(three));

    let dep = Node::with(dependent_seed, registry());
    let (q, mut q_entries) = origin_entries(&dep, dep.id).remove(0);
    q_entries.truncate(1);
    let two = UnsignedEnvelope {
        log: q,
        counter: 2,
        prev_hash: q_entries[0].hash(),
        hlc: HlcStamp::from_parts(1_700_000_000_000, 5),
        kind: "test.opaque".into(),
        kind_version: 1,
        deps: Deps::new(vec![(log, 2)], &q).unwrap(),
        creates: None,
        needs: vec![],
        payload: br#"{"n":2}"#.to_vec(),
    }
    .seal(&NodeIdentity::from_seed(seed(dependent_seed)));
    q_entries.push(StoredEntry::from(two));
    let deps_text = format!("deps {}/{}@2", &log.node.to_string()[..8], log.incarnation);
    Fixture {
        log,
        entries,
        opaque,
        q,
        q_entries,
        deps_text,
    }
}

/// The stall a node holding the relayed log must show.
fn assert_stalled(node: &Node, f: &Fixture, who: &str) {
    assert_eq!(
        blocked_on(node, &f.log).as_deref(),
        Some("opaque_layout"),
        "{who}: origin log"
    );
    assert_eq!(
        blocked_on(node, &f.q).as_deref(),
        Some(f.deps_text.as_str()),
        "{who}: dependent log"
    );
    assert_eq!(applied_through(node, &f.log), 1, "{who}");
    assert_eq!(applied_through(node, &f.q), 1, "{who}");
    assert_eq!(
        apply_state(node, &f.log, 3).as_deref(),
        Some("pending"),
        "{who}"
    );
    assert_eq!(
        apply_state(node, &f.q, 2).as_deref(),
        Some("pending"),
        "{who}"
    );
    assert_eq!(
        scalar(
            node,
            "SELECT count(*) || '' FROM journal WHERE apply_state IN ('parked_unknown','parked_dep')"
        )
        .as_deref(),
        Some("0"),
        "{who}: nothing took the parked paths"
    );
}

fn assert_opaque_bytes(node: &Node, f: &Fixture, who: &str) {
    let row = opaque_row(node, &f.log, 2).unwrap_or_else(|| panic!("{who}: no opaque row"));
    assert_eq!(row.0, f.opaque.raw(), "{who}: raw");
    assert_eq!(row.1, f.opaque.hash(), "{who}: hash");
    assert_eq!(row.2, f.opaque.signature(), "{who}: signature");
}

// The source holds the opaque entry; the third node never links it. The relay
// forwards the entry byte for byte, both keep it unapplied, and the stall
// (opaque_layout, deps string) holds on both, also after a restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_forwards_htjrnl99_to_third_node_byte_exactly() {
    let f = relay_fixture(41, 45);
    let (a, r, c) = (
        Node::with(42, registry()),
        Node::with(43, registry()),
        Node::with(44, registry()),
    );
    ingest(&a.store, &f.log, f.entries.clone());
    ingest(&a.store, &f.q, f.q_entries.clone());
    let (na, nr, nc) = (
        SyncNode::start(a.store.clone(), seed(42), 1),
        SyncNode::start(r.store.clone(), seed(43), 2),
        SyncNode::start(c.store.clone(), seed(44), 3),
    );
    na.link(&nr).await;
    nr.link(&nc).await;

    wait_until("the third node holds both logs", PATIENCE, || {
        contiguous(&c, &f.log) == 3 && contiguous(&c, &f.q) == 2
    })
    .await;
    for (node, who) in [(&r, "relay"), (&c, "third")] {
        let held = node
            .store
            .serve(&f.log, 2, Default::default(), &open_budget())
            .unwrap();
        assert_eq!(held[0], f.entries[1], "{who}: the opaque entry");
        assert_eq!(held[1], f.entries[2], "{who}: the known successor");
        assert_opaque_bytes(node, &f, who);
    }
    assert_eq!(nc.head_of(&f.log), Some((3, f.entries[2].hash())));

    for (node, who) in [(&r, "relay"), (&c, "third")] {
        wait_until("the stall shows", PATIENCE, || {
            blocked_on(node, &f.log).as_deref() == Some("opaque_layout")
                && blocked_on(node, &f.q).as_deref() == Some(f.deps_text.as_str())
        })
        .await;
        assert_stalled(node, &f, who);
    }

    // Nothing promotes it: still stalled after a settle window.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_stalled(&r, &f, "relay after settle");
    assert_stalled(&c, &f, "third after settle");

    // Restart the third node on its store, no layout hook.
    nc.stop().await;
    let nc = SyncNode::start(c.store.clone(), seed(44), 4);
    nr.link(&nc).await;
    let moved = wait_for(Duration::from_secs(5), || {
        nc.head_of(&f.log) != Some((3, f.entries[2].hash()))
    })
    .await;
    assert!(!moved, "the head holds over the new link");
    assert_stalled(&c, &f, "third after restart");
    assert_opaque_bytes(&c, &f, "third after restart");

    na.stop().await;
    nr.stop().await;
    nc.stop().await;
}

// A third node that did not advertise the capability gets the known prefix
// only: its have-head stays behind the withheld counter, the session stays
// healthy, other logs still cross and the dependent waits on its dependency.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capless_third_node_is_withheld_and_stays_healthy() {
    let f = relay_fixture(51, 55);
    let (a, r, c) = (
        Node::with(52, registry()),
        Node::with(53, registry()),
        Node::with(54, registry()),
    );
    ingest(&a.store, &f.log, f.entries.clone());
    ingest(&a.store, &f.q, f.q_entries.clone());
    let a_log = origin_entries(&a, a.id).remove(0).0;
    let (na, nr, nc) = (
        SyncNode::start(a.store.clone(), seed(52), 1),
        SyncNode::start(r.store.clone(), seed(53), 2),
        start_capless(&c, 54, 3),
    );
    na.link(&nr).await;
    nr.link(&nc).await;

    wait_until("the relay holds the whole log", PATIENCE, || {
        contiguous(&r, &f.log) == 3
    })
    .await;
    let advanced = wait_for(Duration::from_secs(5), || contiguous(&c, &f.log) > 1).await;
    assert!(!advanced, "the capless node never passes the cut");
    assert_eq!(contiguous(&c, &f.log), 1);
    assert_eq!(
        nc.head_of(&f.log),
        Some((1, f.entries[0].hash())),
        "its have-head stays behind the withheld counter"
    );

    let mut withheld = false;
    for _ in 0..400 {
        let links = nr.handle.snapshot(&open_budget()).await.unwrap().links;
        if links.iter().any(|l| l.withheld_logs >= 1) {
            withheld = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(withheld, "the relay counts the parked log");

    assert!(nr.ended_links().is_empty(), "{:?}", nr.ended_links());
    assert!(nc.ended_links().is_empty(), "{:?}", nc.ended_links());
    for (node, who) in [(&nr, "relay"), (&nc, "third")] {
        assert!(
            !node.lines().iter().any(|l| l.contains("protocol_error")),
            "{who}: {:?}",
            node.lines()
        );
    }
    assert_eq!(
        scalar(&c, "SELECT ((SELECT count(*) FROM journal WHERE kind IS NULL) + (SELECT count(*) FROM journal_forked WHERE kind IS NULL)) || ''").as_deref(),
        Some("0"),
        "nothing opaque reached it"
    );

    // The session is healthy for other logs, and the dependent waits.
    let a_head = na.head_of(&a_log);
    assert!(a_head.is_some());
    wait_until(
        "the source's own log reaches the third node",
        PATIENCE,
        || nc.head_of(&a_log) == a_head,
    )
    .await;
    wait_until("the dependent log crosses", PATIENCE, || {
        contiguous(&c, &f.q) == 2
    })
    .await;
    wait_until("the dependent waits on its dependency", PATIENCE, || {
        blocked_on(&c, &f.q).as_deref() == Some(f.deps_text.as_str())
    })
    .await;

    na.stop().await;
    nr.stop().await;
    nc.stop().await;
}
