//! Opaque entries on the peer wire (ht-j87z.4) with real stores, coordinators
//! and the apply pump: a relay that understands the capability stores an
//! opaque entry and parks its successors; a node that did not advertise the
//! capability keeps the known prefix and stays linked.
//!
//! The runtime is `test_support::sync_node::SyncNode` over duplex links, as in
//! `incarnations_scenarios`. No process is spawned.

use std::sync::Arc;
use std::time::Duration;

use herdr_threads::node::NodeIdentity;
use herdr_threads::protocol::time::{CallBudget, Cancellation, MonoInstant};
use herdr_threads::sync::LogKey;
use herdr_threads::sync::envelope::StoredEntry;
use herdr_threads::sync::journal_port::JournalPort;
use herdr_threads::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
use herdr_threads::test_support::sync_node::{SyncNode, SyncNodeOptions, wait_until};

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

/// The origin's log `[genesis, opaque 2, known 3]`, signed by the origin's
/// key, for `ingest` into a store.
struct Foreign {
    log: LogKey,
    entries: Vec<StoredEntry>,
}

fn foreign(origin_seed: u8) -> Foreign {
    let origin = Node::with(origin_seed, registry());
    let (log, mut entries) = origin_entries(&origin, origin.id).remove(0);
    entries.truncate(1);
    let signer = NodeIdentity::from_seed(seed(origin_seed));
    let opaque = synthetic_entry(
        &signer,
        &synthetic_unsigned(&signer, log.incarnation, 2, entries[0].hash()),
    );
    let three = synthetic_unsigned(&signer, log.incarnation, 3, opaque.hash()).seal(&signer);
    entries.push(StoredEntry::from(opaque));
    entries.push(StoredEntry::from(three));
    Foreign { log, entries }
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

// A source holds a log with an opaque entry. A relay that advertises the
// capability pulls it byte-exact, never applies it and parks the successor
// (`blocked_on = opaque_layout`); a node that did not advertise the capability
// gets the known prefix only, stays linked and is not sent a protocol error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opaque_successor_parks_on_relay() {
    let f = foreign(31);
    let (source, relay, capless) = (
        Node::with(32, registry()),
        Node::with(33, registry()),
        Node::with(34, registry()),
    );
    ingest(&source.store, &f.log, f.entries.clone());
    let (rs, rr) = (
        SyncNode::start(source.store.clone(), seed(32), 1),
        SyncNode::start(relay.store.clone(), seed(33), 2),
    );
    rs.link(&rr).await;

    wait_until("the relay holds the whole log", PATIENCE, || {
        contiguous(&relay, &f.log) == 3
    })
    .await;
    wait_until("the relay parks the successor", PATIENCE, || {
        blocked_on(&relay, &f.log).as_deref() == Some("opaque_layout")
    })
    .await;
    let held = relay
        .store
        .serve(&f.log, 2, Default::default(), &open_budget())
        .unwrap();
    assert_eq!(held[0], f.entries[1], "the opaque entry, byte for byte");
    assert_eq!(held[1], f.entries[2]);
    let sql = |counter| {
        format!(
            "SELECT apply_state FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter={counter}",
            f.log.node, f.log.incarnation
        )
    };
    assert_eq!(scalar(&relay, &sql(3)).as_deref(), Some("pending"));

    // A node that cannot take opaque entries gets the known prefix.
    let rc = start_capless(&capless, 34, 3);
    rr.link(&rc).await;
    wait_until("the capless node holds the genesis", PATIENCE, || {
        contiguous(&capless, &f.log) == 1
    })
    .await;
    let withheld = |rr: &SyncNode| {
        let rr = rr.handle.clone();
        async move {
            let budget = CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            };
            rr.snapshot(&budget).await.unwrap().links
        }
    };
    let mut parked = false;
    for _ in 0..400 {
        if withheld(&rr).await.iter().any(|l| l.withheld == 1) {
            parked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(parked, "the relay parked the log for the capless link");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(contiguous(&capless, &f.log), 1, "held at the cut");
    assert!(rr.ended_links().is_empty(), "{:?}", rr.ended_links());
    assert!(rc.ended_links().is_empty());
    assert_eq!(
        scalar(&capless, "SELECT count(*) || '' FROM journal_opaque").as_deref(),
        Some("0"),
        "nothing opaque reached it"
    );

    rs.stop().await;
    rr.stop().await;
    rc.stop().await;
}
