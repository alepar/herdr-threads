//! Incarnation convergence scenarios (incarnations design, Testing
//! "Convergence scenarios"; ht-5fr.6.11): restores, wipes, forks and clones
//! healed end to end by the real coordinators, `IncarnationsSink` and
//! `IncarnationsWorker` of three to four in-process nodes over duplex links.
//! No process is spawned and no daemon runs.
//!
//! The runtime is `test_support::sync_node::SyncNode`; the stores, commands
//! and fact helpers are `facts_seat::Node` and its extensions. Setup that
//! needs no network (history, backups) moves entries through
//! `fact_exchange::sync` before any runtime starts.

use std::sync::Arc;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use herdr_threads::node::NodeUuid;
use herdr_threads::protocol::commands::DeliveryMode;
use herdr_threads::protocol::ids::{MessageId, SeatId, ThreadId};
use herdr_threads::protocol::results::ApiError;
use herdr_threads::protocol::time::CallBudget;
use herdr_threads::store::SqliteStore;
use herdr_threads::store::incarnations as inc;
use herdr_threads::store::journal;
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::LogKey;
use herdr_threads::sync::coordinator::{CloneKind, CloneReport};
use herdr_threads::sync::envelope::StoredEntry;
use herdr_threads::sync::facts::payload::{ReceiptObligation, SettleHow};
use herdr_threads::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
use herdr_threads::sync::facts::{GenesisPayload, Predecessor};
use herdr_threads::sync::incarnations::HELD_LINE;
use herdr_threads::sync::journal_port::{ApplyProgress, IngestOutcome, JournalPort, LogMeta};
use herdr_threads::sync::{BatchLimits, Vector};
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::journal_oracle::{replicated_projection, replicated_tables};
use herdr_threads::test_support::sync_node::{SyncNode, wait_for, wait_until};

use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_receipt::{ingest, origin_entries, rworld, settled};
use crate::facts_seat::{Node, budget, registry};

const PATIENCE: Duration = Duration::from_secs(60);

/// A node's key seed: `Node::with(n, ..)` keys its store with `[n; 32]`.
fn uuid8(node: NodeUuid) -> String {
    node.to_string().chars().take(8).collect()
}

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn start(node: &Node, n: u8, boot: u128) -> SyncNode {
    SyncNode::start(node.store.clone(), seed(n), boot)
}

/// Every replicated table of `node`, nothing excluded.
fn projection(node: &Node) -> std::collections::BTreeMap<String, String> {
    let nobody = NodeUuid::from_bytes([0xee; 16]);
    replicated_projection(&node.store, nobody, &replicated_tables())
        .into_iter()
        .map(|(table, rows)| (table, format!("{rows:?}")))
        .collect()
}

/// Panics naming the replicated tables in which `nodes` differ and, per
/// table, the rows of the first node and of the other.
fn assert_projections_equal(nodes: &[&Node]) {
    let first = projection(nodes[0]);
    for other in &nodes[1..] {
        let theirs = projection(other);
        let differing: Vec<String> = first
            .iter()
            .filter(|(table, rows)| theirs.get(*table) != Some(*rows))
            .map(|(table, rows)| {
                format!(
                    "{table}:\n  {}: {rows}\n  {}: {:?}",
                    nodes[0].id,
                    other.id,
                    theirs.get(table)
                )
            })
            .collect();
        assert!(
            differing.is_empty(),
            "{} and {} project differently:\n{}",
            nodes[0].id,
            other.id,
            differing.join("\n")
        );
    }
}

// ---- reading a node's logs --------------------------------------------------

/// `(log, frozen_at, contiguous)` of every log `node` holds.
fn logs(node: &Node) -> Vec<(LogKey, Option<u64>, u64)> {
    node.query(
        "SELECT node_uuid,incarnation,frozen_at_counter,contiguous_counter FROM sync_logs \
         ORDER BY first_seen_at,log_ix",
        |r| {
            let node: String = r.get(0)?;
            Ok((
                LogKey {
                    node: node.parse().expect("a node uuid"),
                    incarnation: r.get(1)?,
                },
                r.get::<_, Option<i64>>(2)?.map(|c| c as u64),
                r.get::<_, i64>(3)? as u64,
            ))
        },
    )
}

fn active_log(node: &Node) -> LogKey {
    let (uuid, inc): (String, u32) = node
        .query(
            "SELECT l.node_uuid,l.incarnation FROM local_node n \
             JOIN sync_logs l ON l.log_ix=n.active_log_ix",
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .remove(0);
    LogKey {
        node: uuid.parse().expect("a node uuid"),
        incarnation: inc,
    }
}

fn contiguous(node: &Node, log: &LogKey) -> Option<u64> {
    logs(node)
        .into_iter()
        .find(|(l, _, _)| l == log)
        .map(|(_, _, c)| c)
}

fn frozen_at(node: &Node, log: &LogKey) -> Option<u64> {
    logs(node)
        .into_iter()
        .find(|(l, _, _)| l == log)
        .and_then(|(_, f, _)| f)
}

fn pending(node: &Node) -> i64 {
    node.count("SELECT count(*) FROM journal WHERE apply_state='pending'")
}

fn open_clone_evidence(node: &Node) -> i64 {
    node.count("SELECT count(*) FROM sync_clone_evidence WHERE cleared_at IS NULL")
}

fn forks(node: &Node) -> i64 {
    node.count("SELECT count(*) FROM sync_forks")
}

fn lazy_rows(node: &Node) -> i64 {
    node.count("SELECT count(*) FROM lazy_recipients")
}

fn has_message(node: &Node, message: &MessageId) -> bool {
    node.count(&format!(
        "SELECT count(*) FROM messages WHERE id='{}'",
        message.as_str()
    )) == 1
}

fn has_thread(node: &Node, thread: &ThreadId) -> bool {
    node.count(&format!(
        "SELECT count(*) FROM threads WHERE id='{}'",
        thread.as_str()
    )) == 1
}

// ---- authoring --------------------------------------------------------------

/// An ordinary message from `from` to `to` that wants an ACK.
fn send_ack(node: &Node, thread: &ThreadId, from: &SeatId, to: &SeatId) -> MessageId {
    let message = node.stage_prep(thread, "ordinary", &[(to, FIVE_MIN, true)], &[]);
    node.author_sent(&sent(
        &message,
        thread,
        from,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(to, ReceiptObligation::Ack, Some(SENT_AT + FIVE_MIN))],
    ))
    .unwrap();
    message
}

/// A lazy message from `from` to `to`.
fn send_lazy(node: &Node, thread: &ThreadId, from: &SeatId, to: &SeatId) -> MessageId {
    let message = node.stage_prep(thread, "lazy", &[], &[to]);
    node.author_sent(&sent(
        &message,
        thread,
        from,
        DeliveryMode::Lazy,
        0,
        vec![spec(to, ReceiptObligation::None, None)],
    ))
    .unwrap();
    message
}

fn settle(node: &Node, message: &MessageId, seat: &SeatId, batch: &str) {
    node.author_settled(&settled(message, seat, SettleHow::Acked, batch, true))
        .unwrap();
}

/// `count` fresh threads created by `creator`: one journal entry each (the
/// creator seat is published already), different on every node that runs
/// the same call.
fn author_threads(node: &Node, creator: &SeatId, count: usize) -> Vec<ThreadId> {
    (0..count)
        .map(|n| {
            node.create_thread(creator, &format!("t{n}"), None)
                .expect("create a thread")
        })
        .collect()
}

/// What an operator-less rotation would do: starts the next random
/// incarnation naming `predecessor` (not forked). Returns the new log.
fn rotate_by_hand(node: &Node, predecessor: u32) -> LogKey {
    node.store
        .incarnations_quantum(&budget(), |tx, ctx, d| {
            let incarnation = inc::next_incarnation(tx, ctx)?;
            journal::begin_incarnation(
                tx,
                ctx,
                d,
                incarnation,
                Some(Predecessor {
                    incarnation: predecessor,
                    frozen_at: None,
                }),
                "host",
            )
            .map(|_| ())
        })
        .expect("rotate by hand");
    active_log(node)
}

/// The predecessor `log`'s genesis names.
fn genesis_predecessor(node: &Node, log: &LogKey) -> Option<Predecessor> {
    let payload: Vec<u8> = node
        .query(
            &format!(
                "SELECT j.payload FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter=1",
                log.node, log.incarnation
            ),
            |r| r.get(0),
        )
        .remove(0);
    serde_json::from_slice::<GenesisPayload>(&payload)
        .expect("a genesis payload")
        .predecessor
}

/// Waits until every clause of `checks` holds; on timeout panics naming the
/// clauses that do not, then `report` (the nodes' state).
async fn converge(
    what: &str,
    timeout: Duration,
    checks: impl Fn() -> Vec<(&'static str, bool)>,
    report: impl Fn() -> String,
) {
    if wait_for(timeout, || checks().iter().all(|(_, ok)| *ok)).await {
        return;
    }
    let failing: Vec<&str> = checks()
        .into_iter()
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| name)
        .collect();
    panic!(
        "timed out waiting for {what}; not holding: {failing:?}\n{}",
        report()
    );
}

/// What `node` holds, for a failure message.
fn dump(label: &str, node: &Node, running: &SyncNode) -> String {
    let branches = node.query(
        "SELECT tip_counter,state,attempts,coalesce(last_error,'') FROM sync_branches",
        |r| {
            Ok(format!(
                "tip {} {} attempts {} {}",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?
            ))
        },
    );
    format!(
        "{label}: active {:?}\n logs {:?}\n forks {} branches {branches:?} reissues {} pending {} evidence {:?}\n lines {:?}",
        active_log(node),
        logs(node),
        forks(node),
        reissues(node),
        pending(node),
        status_of(node).clone_evidence,
        running.lines(),
    )
}

fn status_of(node: &Node) -> inc::IncarnationsStatus {
    let db = node.ctx().open_query(budget()).expect("a query connection");
    inc::status(&db, node.ctx()).expect("read the incarnations status")
}

fn reissues(node: &Node) -> i64 {
    node.count("SELECT count(*) FROM sync_reissues")
}

/// Holds `node`'s automatic repair the way a reported `node_collision` would,
/// before its runtime starts: owner rotation and re-issue wait for
/// [`clear_clone`]. Lets a scenario show what the other nodes do while the
/// owner is not yet repairing.
fn hold_repairs(node: &Node) {
    node.store
        .incarnations_quantum(&budget(), |tx, _, d| {
            inc::record_clone_evidence(
                tx,
                &CloneReport {
                    subject: node.id,
                    kind: CloneKind::Handshake,
                    boots: Vec::new(),
                    source: node.id,
                },
                d.utc,
            )
        })
        .expect("record the clone evidence");
}

/// The operator's `sync clone-cleared` on `node` (what `SyncAdmin` does).
fn clear_clone(node: &Node, running: &SyncNode) -> u32 {
    let cleared = node
        .store
        .incarnations_quantum(&budget(), |tx, _, d| {
            inc::clear_clone_evidence(tx, "operator:local-user:501", d.utc)
        })
        .expect("clear the clone evidence");
    running.sink.notify_clone_cleared();
    cleared
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_nodes_link_and_converge() {
    let reg = registry();
    let (a, b) = (Node::with(11, reg), Node::with(12, reg));
    let (thread, _creator) = thread_of(&a, "smoke");
    let (ra, rb) = (start(&a, 11, 1), start(&b, 12, 2));
    ra.link(&rb).await;
    wait_until("B applies A's thread", PATIENCE, || has_thread(&b, &thread)).await;
    assert_projections_equal(&[&a, &b]);
    ra.stop().await;
    rb.stop().await;
}

// ---- 1. restore without a fork ----------------------------------------------

/// A is restored to a backup; B holds A's tail. A rotates and pulls its tail
/// back with no fork. Settled history wakes nobody; the two unsettled
/// messages between A's own seats are offered once the own-history hold
/// lifts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_restore_without_fork() {
    let reg = registry();
    let (a, b) = (Node::with(21, reg), Node::with(22, reg));
    let (thread, sender) = thread_of(&a, "s1");
    let peer = a.local_seat();
    a.joined(&thread, &sender, 1).unwrap();
    a.joined(&thread, &peer, 1).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let backup = a.snapshot("backup.db");
    let old_log = active_log(&a);

    // The tail the backup lacks: one settled ACK message, one unsettled ACK
    // message and one unsettled lazy message, all between A's seats.
    let (settled_msg, open_ack) = (
        send_ack(&a, &thread, &sender, &peer),
        send_ack(&a, &thread, &sender, &peer),
    );
    let open_lazy = send_lazy(&a, &thread, &sender, &peer);
    settle(&a, &settled_msg, &peer, "op-s1");
    fact_exchange::sync(&a.store, &b.store);

    let lost = a;
    let a = Node::open_restored(21, 2100, reg, &backup, false);
    drop(lost);
    assert!(!has_message(&a, &settled_msg), "the restore lacks the tail");
    let (ra, rb) = (start(&a, 21, 3), start(&b, 22, 4));
    ra.link(&rb).await;

    wait_until("A rotates and pulls its tail back", PATIENCE, || {
        active_log(&a) != old_log
            && has_message(&a, &open_lazy)
            && pending(&a) == 0
            && pending(&b) == 0
            && ra.vector() == rb.vector()
    })
    .await;
    // Quiet the stores before reading and driving them.
    ra.stop().await;
    rb.stop().await;

    // No fork: nothing frozen, nothing recorded, the old log kept whole.
    assert!(logs(&a).iter().all(|(_, frozen, _)| frozen.is_none()));
    assert_eq!(forks(&a) + forks(&b), 0);
    assert_eq!(open_clone_evidence(&a), 0);
    assert!(contiguous(&a, &old_log) > Some(0));
    assert_eq!(contiguous(&a, &old_log), contiguous(&b, &old_log));

    // Settled history wakes nobody; open obligations are offered.
    assert_eq!(
        a.suppressed_seats(&settled_msg).as_deref(),
        Some(peer.as_str())
    );
    assert_eq!(a.suppressed_seats(&open_ack), None);
    assert_eq!(lazy_rows(&a), 1, "only the unsettled lazy message");
    assert_eq!(
        a.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE message_id='{}'",
            open_lazy.as_str()
        )),
        1
    );
    a.drive_send(&settled_msg);
    assert!(
        a.wake_seats().is_empty(),
        "no wake_work for settled history"
    );
    a.drive_send(&open_ack);
    assert_eq!(a.wake_seats(), vec![peer.as_str().to_owned()]);

    // B projects the receipts of the same messages (its sends are driven too).
    b.drive_send(&settled_msg);
    b.drive_send(&open_ack);
    assert_projections_equal(&[&a, &b]);
}

// ---- 1a. restore older than a rotation ---------------------------------------

/// A is backed up on I1, rotates to I2 and is then restored from the I1-era
/// backup. B advertises I1's tail and I2. A is not a clone: it adopts the
/// lineage, rotates naming I2, pulls I1 and I2 back and converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1a_restore_older_than_rotation() {
    let reg = registry();
    let (a, b) = (Node::with(23, reg), Node::with(24, reg));
    let (thread, sender) = thread_of(&a, "s1a");
    a.joined(&thread, &sender, 1).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let backup = a.snapshot("i1-era.db");
    let i1 = active_log(&a);

    let tail1 = author_threads(&a, &sender, 1);
    let i2 = rotate_by_hand(&a, i1.incarnation);
    let tail2 = author_threads(&a, &sender, 1);
    fact_exchange::sync(&a.store, &b.store);

    let lost = a;
    let a = Node::open_restored(23, 2300, reg, &backup, false);
    drop(lost);
    assert_eq!(active_log(&a), i1, "the restore is on I1 again");
    let (ra, rb) = (start(&a, 23, 3), start(&b, 24, 4));
    ra.link(&rb).await;

    wait_until("A adopts the lineage and converges", PATIENCE, || {
        let now = active_log(&a);
        now != i1
            && now != i2
            && has_thread(&a, &tail1[0])
            && has_thread(&a, &tail2[0])
            && pending(&a) == 0
            && pending(&b) == 0
            && ra.vector() == rb.vector()
    })
    .await;

    let i3 = active_log(&a);
    assert_eq!(
        genesis_predecessor(&a, &i3),
        Some(Predecessor {
            incarnation: i2.incarnation,
            frozen_at: None
        }),
        "the new incarnation names the newest log of the lineage"
    );
    assert_eq!(
        status_of(&a).clone_evidence,
        vec![],
        "a restore is not a clone"
    );
    assert_eq!(open_clone_evidence(&a) + open_clone_evidence(&b), 0);
    assert!(logs(&a).iter().all(|(_, frozen, _)| frozen.is_none()));
    assert_eq!(forks(&a) + forks(&b), 0);
    assert_eq!(contiguous(&a, &i1), contiguous(&b, &i1));
    assert_eq!(contiguous(&a, &i2), contiguous(&b, &i2));
    assert_projections_equal(&[&a, &b]);
    ra.stop().await;
    rb.stop().await;
}

/// The variant: A authored on its restored I1 before B linked, so the
/// restore successor rules fail. A is held as a clone suspect and `status`
/// names the single-copy remedy. After `clone-cleared` alone (no move-aside)
/// A rotates, re-issues its divergent I1 entries and converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1a_variant_authored_on_i1_clone_suspect_then_cleared() {
    let reg = registry();
    let (a, b) = (Node::with(25, reg), Node::with(26, reg));
    let (thread, sender) = thread_of(&a, "s1a-variant");
    a.joined(&thread, &sender, 1).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let backup = a.snapshot("i1-era.db");
    let i1 = active_log(&a);

    let tail1 = author_threads(&a, &sender, 1);
    let i2 = rotate_by_hand(&a, i1.incarnation);
    let tail2 = author_threads(&a, &sender, 1);
    fact_exchange::sync(&a.store, &b.store);

    let lost = a;
    let a = Node::open_restored(25, 2500, reg, &backup, false);
    // I2 reached the restored node before the fork surfaced (the worker only
    // finds the clone once the newer log is held): ship just that log.
    for (log, entries) in origin_entries(&lost, lost.id) {
        if log == i2 {
            ingest(&a.store, &log, entries);
        }
    }
    drop(lost);
    // Authored on I1 after the node moved on to I2 (more entries than the
    // original made before it rotated, so their HLCs pass I2's genesis).
    let divergent = author_threads(&a, &sender, 8);
    let (ra, rb) = (start(&a, 25, 3), start(&b, 26, 4));
    ra.link(&rb).await;

    wait_until("A records the clone evidence", PATIENCE, || {
        open_clone_evidence(&a) >= 1 && has_thread(&a, &tail2[0])
    })
    .await;
    // Held: no rotation and no re-issue while the evidence is open.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        active_log(&a),
        i1,
        "a clone suspect does not rotate\n{}",
        dump("A", &a, &ra)
    );
    assert_eq!(reissues(&a), 0);
    let status = status_of(&a);
    assert_eq!(status.own_unknown_newer, vec![i2]);
    assert_eq!(
        status.remedy.map(|r| r.single_copy),
        Some(inc::REMEDY_SINGLE_COPY)
    );

    assert!(clear_clone(&a, &ra) >= 1);
    let all = [&tail1[0], &tail2[0], &divergent[0], &divergent[7]];
    let done = wait_for(Duration::from_secs(15), || {
        let now = active_log(&a);
        now != i1
            && all.iter().all(|t| has_thread(&a, t) && has_thread(&b, t))
            && pending(&a) == 0
            && pending(&b) == 0
            && ra.head_of(&now) == rb.head_of(&now)
    })
    .await;
    assert!(done, "{}\n{}", dump("A", &a, &ra), dump("B", &b, &rb));

    let i3 = active_log(&a);
    assert_eq!(
        genesis_predecessor(&a, &i3).map(|p| p.incarnation),
        Some(i1.incarnation),
        "the rotation names the forked log"
    );
    assert!(frozen_at(&a, &i1).is_some() && frozen_at(&b, &i1).is_some());
    assert!(reissues(&a) > 0, "the divergent entries were re-issued");
    assert_projections_equal(&[&a, &b]);
    ra.stop().await;
    rb.stop().await;
}

// ---- 2. restore, then author --------------------------------------------------

/// A is restored and authors again, so its log forks against the branch C
/// (and D) hold. C freezes from ingest. D, linked to C only, learns the fork
/// from C's `ForkRef`. A rotates, pulls the lost branch from C and re-issues
/// both branches; the lost branch's unsettled message to a local seat is
/// offered, its settled one is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_restore_then_author_three_parties() {
    let reg = registry();
    let (a, c, d) = (
        Node::with(31, reg),
        Node::with(32, reg),
        Node::with(33, reg),
    );
    let (thread, from) = thread_of(&a, "s2");
    let to = a.local_seat();
    a.joined(&thread, &from, 1).unwrap();
    a.joined(&thread, &to, 1).unwrap();
    fact_exchange::sync(&a.store, &c.store);
    fact_exchange::sync(&c.store, &d.store);
    let backup = a.snapshot("backup.db");
    let i1 = active_log(&a);

    // The branch A will lose: held by C and D.
    let open_msg = send_ack(&a, &thread, &from, &to);
    let settled_msg = send_ack(&a, &thread, &from, &to);
    settle(&a, &settled_msg, &to, "op-s2");
    let lost_threads = author_threads(&a, &from, 1);
    fact_exchange::sync(&a.store, &c.store);
    fact_exchange::sync(&c.store, &d.store);

    let lost = a;
    let a = Node::open_restored(31, 3100, reg, &backup, false);
    drop(lost);
    let new_threads = author_threads(&a, &from, 3);
    // The owner is held, so the third parties act alone first.
    hold_repairs(&a);

    let (ra, rc, rd) = (start(&a, 31, 3), start(&c, 32, 4), start(&d, 33, 5));
    ra.link(&rc).await;
    rc.link(&rd).await;

    // C freezes from ingest; D, linked to C only, from C's fork reference.
    // No incarnation of A exists beyond I1 yet.
    let seen = wait_for(Duration::from_secs(20), || {
        frozen_at(&c, &i1).is_some()
            && frozen_at(&d, &i1).is_some()
            && d.count("SELECT count(*) FROM sync_forks WHERE origin='evidence'") >= 1
    })
    .await;
    assert!(
        seen,
        "{}\n{}\n{}",
        dump("A", &a, &ra),
        dump("C", &c, &rc),
        dump("D", &d, &rd)
    );
    assert_eq!(active_log(&a), i1, "the held owner has not rotated");
    for node in [&a, &c, &d] {
        assert_eq!(
            logs(node).iter().filter(|(l, _, _)| l.node == a.id).count(),
            1,
            "only I1 of A exists on {}",
            node.id
        );
    }
    clear_clone(&a, &ra);

    let everything = |n: &Node| {
        has_message(n, &open_msg)
            && has_message(n, &settled_msg)
            && lost_threads
                .iter()
                .chain(&new_threads)
                .all(|t| has_thread(n, t))
            && pending(n) == 0
    };
    converge(
        "A to rotate and every node to hold both branches",
        Duration::from_secs(20),
        || {
            let now = active_log(&a);
            vec![
                ("A rotated", now != i1),
                ("A has everything", everything(&a)),
                ("C has everything", everything(&c)),
                ("D has everything", everything(&d)),
                ("A holds its new log", ra.head_of(&now).is_some()),
                ("C's head matches", ra.head_of(&now) == rc.head_of(&now)),
                ("D's head matches", ra.head_of(&now) == rd.head_of(&now)),
            ]
        },
        || {
            format!(
                "{}\n{}\n{}",
                dump("A", &a, &ra),
                dump("C", &c, &rc),
                dump("D", &d, &rd)
            )
        },
    )
    .await;

    // The old incarnation is frozen wherever the fork was seen, and D heard of
    // it from C alone.
    for node in [&a, &c, &d] {
        assert!(frozen_at(node, &i1).is_some(), "I1 frozen on {}", node.id);
    }
    let sources: Vec<String> = d.query(
        "SELECT source_node FROM sync_forks WHERE origin='evidence'",
        |r| r.get(0),
    );
    assert!(!sources.is_empty(), "D recorded the fork's proof");
    assert!(
        sources.iter().all(|s| *s == c.id.to_string()),
        "{sources:?}"
    );
    assert_eq!(
        rd.lines()
            .iter()
            .filter(|l| l.starts_with("sync: link up"))
            .count(),
        1,
        "D has one link, to C"
    );
    assert_eq!(open_clone_evidence(&a), 0);

    // Delivery: the lost branch's open obligation is offered, its settled
    // history is not. (The stores are quiet first.)
    ra.stop().await;
    rc.stop().await;
    rd.stop().await;
    assert_eq!(
        a.suppressed_seats(&settled_msg).as_deref(),
        Some(to.as_str())
    );
    assert_eq!(a.suppressed_seats(&open_msg), None);
    a.drive_send(&settled_msg);
    assert!(a.wake_seats().is_empty(), "settled history wakes nobody");
    a.drive_send(&open_msg);
    assert_eq!(a.wake_seats(), vec![to.as_str().to_owned()]);
    for n in [&c, &d] {
        n.drive_send(&settled_msg);
        n.drive_send(&open_msg);
    }

    assert_projections_equal(&[&a, &c, &d]);
}

// ---- 3. wiped store, key intact ----------------------------------------------

/// A's store is wiped but its key survives: A starts a new random
/// incarnation (no fork) and pulls its old one back. B's log is served to A
/// before A's old log may be; the own-history hold keeps B's `message.sent`
/// unapplied until A's old log (holding the settlement) is ingested. The
/// settled message then applies with no lazy row, no `wake_work` and no wake
/// dispatch; the unsettled one in the same catch-up is offered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_wiped_store_key_intact() {
    // rworld: `a` is B here (the sender), `b` is A (hosts the seats).
    let w = rworld(60);
    let (settled_msg, open_msg) = (w.send(), w.send());
    w.deliver(&[&settled_msg, &open_msg]);
    w.b.author_settled(&settled(
        &settled_msg,
        &w.agent,
        SettleHow::Acked,
        "op-3",
        true,
    ))
    .unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    let old_log = active_log(&w.b);
    let old_head = contiguous(&w.b, &old_log);

    // The wipe: a fresh store of the same node (same key, another draw).
    let wiped = Node::open_commands(61, 6100, registry());
    assert_eq!(wiped.id, w.b.id, "the same node");
    let new_log = active_log(&wiped);
    assert_ne!(new_log, old_log, "a wiped store starts a new incarnation");

    let (ra, rb) = (start(&w.a, 60, 3), start(&wiped, 61, 4));
    rb.link(&ra).await;

    // Whenever B's message is applied on the wiped node, the settlement from
    // its old log is already known to suppress it.
    let mut violated = false;
    let done = wait_for(Duration::from_secs(20), || {
        {
            let m = &settled_msg;
            if has_message(&wiped, m) && wiped.suppressed_seats(m).is_none() {
                violated = true;
                return true;
            }
        }
        has_message(&wiped, &settled_msg)
            && has_message(&wiped, &open_msg)
            && contiguous(&wiped, &old_log) == old_head
            && pending(&wiped) == 0
    })
    .await;
    assert!(done, "{}\n{}", dump("A", &wiped, &rb), dump("B", &w.a, &ra));
    assert!(!violated, "the settled message was applied unsuppressed");
    // Quiet the stores before reading and driving them.
    ra.stop().await;
    rb.stop().await;

    // No fork: the old log was pulled back whole, nothing frozen or recorded.
    assert_eq!(contiguous(&wiped, &old_log), old_head);
    assert!(logs(&wiped).iter().all(|(_, frozen, _)| frozen.is_none()));
    assert_eq!(forks(&wiped) + forks(&w.a), 0);
    assert_eq!(open_clone_evidence(&wiped), 0);

    assert_eq!(
        wiped.settled_row(&settled_msg, &w.agent).map(|r| r.0),
        Some("acked".into())
    );
    assert_eq!(
        wiped.suppressed_seats(&settled_msg).as_deref(),
        Some(w.agent.as_str())
    );
    assert_eq!(
        lazy_rows(&wiped),
        0,
        "no lazy_recipients for settled history"
    );
    wiped.drive_send(&settled_msg);
    assert!(
        wiped.wake_seats().is_empty(),
        "no wake_work, so no wake dispatch, for the settled seat"
    );
    assert_eq!(wiped.suppressed_seats(&open_msg), None);
    wiped.drive_send(&open_msg);
    assert_eq!(
        wiped.wake_seats(),
        vec![w.agent.as_str().to_owned()],
        "the unsettled message in the same catch-up is offered"
    );
    assert_projections_equal(&[&w.a, &wiped]);
}

// ---- 4. Have-time mismatch -----------------------------------------------------

/// Two nodes hold one log at the same counter with different heads. Neither
/// wants an entry from the other, so no entry is ingested: the fork surfaces
/// from the `have` frames alone (a head differing at a counter both hold), is
/// proven by a single-entry fetch and ends frozen on both.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_have_time_mismatch_detects_fork() {
    let reg = registry();
    let (a, b) = (Node::with(71, reg), Node::with(72, reg));
    let (thread, creator) = thread_of(&a, "s4");
    a.joined(&thread, &creator, 1).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let backup = a.snapshot("backup.db");
    let i1 = active_log(&a);
    let common = contiguous(&a, &i1).unwrap();

    let branch_b = author_threads(&a, &creator, 3);
    fact_exchange::sync(&a.store, &b.store);
    let lost = a;
    let a = Node::open_restored(71, 7100, reg, &backup, false);
    drop(lost);
    let branch_a = author_threads(&a, &creator, 3);

    // Equal counters, different heads.
    let head = |n: &Node| n.store.vector(&budget()).unwrap().0[&i1];
    let (head_a, head_b) = (head(&a), head(&b));
    assert_eq!(head_a.counter, head_b.counter);
    assert_ne!(head_a.head_hash, head_b.head_hash);
    let tip = head_b.counter;
    assert_eq!(b.count("SELECT count(*) FROM journal_forked"), 0);

    let (ra, rb) = (start(&a, 71, 3), start(&b, 72, 4));
    ra.link(&rb).await;

    let all = |n: &Node| branch_a.iter().chain(&branch_b).all(|t| has_thread(n, t));
    let done = wait_for(Duration::from_secs(20), || {
        let now = active_log(&a);
        now != i1
            && all(&a)
            && all(&b)
            && pending(&a) == 0
            && pending(&b) == 0
            && ra.head_of(&now).is_some()
            && ra.head_of(&now) == rb.head_of(&now)
    })
    .await;
    assert!(done, "{}\n{}", dump("A", &a, &ra), dump("B", &b, &rb));

    // The proof is the heads themselves: at the shared counter, on both
    // nodes, with B's I1 untouched (no entry was ingested into it).
    for node in [&a, &b] {
        let proofs: Vec<(i64, i64, i64)> = node.query(
            "SELECT proof_counter,held_counter,conflicting_counter FROM sync_forks \
             WHERE origin='evidence'",
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        let tip = tip as i64;
        assert_eq!(proofs, vec![(tip, tip, tip)], "{}", node.id);
    }
    assert_eq!(rb.head_of(&i1), Some((tip, head_b.head_hash)));
    assert_eq!(b.count("SELECT count(*) FROM journal_forked"), 1);
    // B froze at the proof; the owner, who pulled the whole branch, lowered
    // its freeze to the true divergence.
    assert_eq!(frozen_at(&b, &i1), Some(tip - 1));
    assert_eq!(frozen_at(&a, &i1), Some(common));
    assert_projections_equal(&[&a, &b]);
    ra.stop().await;
    rb.stop().await;
}

// ---- 5. corrupt branch reply ---------------------------------------------------

/// A store that serves branches with a broken signature (the rest of the
/// journal port is the store's).
struct CorruptBranches {
    store: Arc<SqliteStore>,
    served: AtomicUsize,
}

impl JournalPort for CorruptBranches {
    fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError> {
        self.store.vector(budget)
    }
    fn stored_hash(
        &self,
        log: &LogKey,
        counter: u64,
        budget: &CallBudget,
    ) -> Result<Option<[u8; 32]>, ApiError> {
        self.store.stored_hash(log, counter, budget)
    }
    fn serve(
        &self,
        log: &LogKey,
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Vec<StoredEntry>, ApiError> {
        self.store.serve(log, from, limits, budget)
    }
    fn serve_branch(
        &self,
        log: &LogKey,
        tip: (u64, [u8; 32]),
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Option<Vec<StoredEntry>>, ApiError> {
        let mut served = self.store.serve_branch(log, tip, from, limits, budget)?;
        if let Some(entries) = served.as_mut().filter(|e| !e.is_empty()) {
            self.served.fetch_add(1, Ordering::SeqCst);
            for entry in entries {
                if let StoredEntry::Known(entry) = entry {
                    entry.signature[0] ^= 0xff;
                }
            }
        }
        Ok(served)
    }
    fn log_meta(&self, log: &LogKey, budget: &CallBudget) -> Result<Option<LogMeta>, ApiError> {
        self.store.log_meta(log, budget)
    }
    fn ingest(
        &self,
        log: &LogKey,
        entries: Vec<StoredEntry>,
        budget: &CallBudget,
    ) -> Result<IngestOutcome, ApiError> {
        self.store.ingest(log, entries, budget)
    }
    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
        self.store.apply_pending(budget)
    }
}

/// A restored A forks against two holders of its lost branch. The first one
/// A can ask serves a branch with a broken signature: A rejects it, avoids
/// that source for the log and completes the branch from the other holder
/// once that one is linked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_corrupt_branch_reply_penalized_branch_completes_elsewhere() {
    let reg = registry();
    let (a, b1, b2) = (
        Node::with(75, reg),
        Node::with(76, reg),
        Node::with(77, reg),
    );
    let (thread, creator) = thread_of(&a, "s5");
    a.joined(&thread, &creator, 1).unwrap();
    fact_exchange::sync(&a.store, &b1.store);
    fact_exchange::sync(&a.store, &b2.store);
    let backup = a.snapshot("backup.db");
    let i1 = active_log(&a);
    let lost_threads = author_threads(&a, &creator, 3);
    fact_exchange::sync(&a.store, &b1.store);
    fact_exchange::sync(&a.store, &b2.store);
    let lost = a;
    let a = Node::open_restored(75, 7500, reg, &backup, false);
    drop(lost);
    let new_threads = author_threads(&a, &creator, 2);

    let (bad, bad_seed, good, good_seed) = (&b1, 76, &b2, 77);
    let corrupt = Arc::new(CorruptBranches {
        store: bad.store.clone(),
        served: AtomicUsize::new(0),
    });
    let ra = start(&a, 75, 3);
    let rbad = SyncNode::start_serving(bad.store.clone(), corrupt.clone(), seed(bad_seed), 4);
    let rgood = start(good, good_seed, 5);

    // Only the corrupt holder is linked at first: A asks it for the branch,
    // rejects the reply and avoids it. Then the honest holder joins.
    ra.link(&rbad).await;
    let asked = wait_for(Duration::from_secs(20), || {
        corrupt.served.load(Ordering::SeqCst) >= 1 && ra.sink.is_penalized(bad.id, i1)
    })
    .await;
    assert!(asked, "{}", dump("A", &a, &ra));
    assert_eq!(
        a.count("SELECT count(*) FROM sync_branches WHERE state='complete'"),
        0,
        "nothing completed from the bad source"
    );
    ra.link(&rgood).await;

    let everything = |n: &Node| {
        lost_threads
            .iter()
            .chain(&new_threads)
            .all(|t| has_thread(n, t))
    };
    let done = wait_for(Duration::from_secs(20), || {
        let now = active_log(&a);
        now != i1
            && [&a, &b1, &b2]
                .iter()
                .all(|n| everything(n) && pending(n) == 0)
            && ra.head_of(&now).is_some()
            && ra.head_of(&now) == rbad.head_of(&now)
            && ra.head_of(&now) == rgood.head_of(&now)
    })
    .await;
    assert!(
        done,
        "{}\n{}\n{}",
        dump("A", &a, &ra),
        dump("bad", bad, &rbad),
        dump("good", good, &rgood)
    );

    assert!(
        ra.sink.is_penalized(bad.id, i1),
        "the bad source stays avoided"
    );
    assert!(!ra.sink.is_penalized(good.id, i1));
    assert!(
        ra.lines()
            .iter()
            .any(|l| l.contains("rejected") && l.contains(&uuid8(bad.id))),
        "the rejection is logged: {:?}",
        ra.lines()
    );
    assert!(frozen_at(&a, &i1).is_some());
    let branch_states: Vec<String> = a.query("SELECT state FROM sync_branches", |r| r.get(0));
    assert!(!branch_states.is_empty());
    assert!(
        branch_states.iter().all(|s| s == "complete"),
        "{branch_states:?}\n{}",
        dump("A", &a, &ra)
    );
    assert_projections_equal(&[&a, &b1, &b2]);
    ra.stop().await;
    rbad.stop().await;
    rgood.stop().await;
}

// ---- 6. unknown kind -------------------------------------------------------------

/// A fact kind only some nodes know; its handler has no effect of its own
/// (the apply state of its journal entries is what the scenario reads).
#[derive(Serialize, Deserialize)]
struct Note {
    id: String,
}

struct ScnNote;

impl FactKind for ScnNote {
    const NAME: &'static str = "scn.note";
    const MAJOR: u32 = 1;
    type Payload = Note;
    fn creates(p: &Note) -> Option<EntityKey> {
        EntityKey::new(format!("note:{}", p.id)).ok()
    }
    fn needs(_: &Note) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Note) -> Result<(), FactError> {
        Ok(())
    }
}

/// The apply states of the `scn.note` entries of `node`'s `journal`, oldest
/// first.
fn note_states(node: &Node) -> Vec<String> {
    node.query(
        "SELECT apply_state FROM journal WHERE kind='scn.note' ORDER BY ingest_seq",
        |r| r.get(0),
    )
}

/// `(kind_version, hlc, creates, needs, payload)` of every `scn.note` entry
/// in `table` (`journal` or `journal_forked`).
#[allow(clippy::type_complexity)]
fn note_entries(node: &Node, table: &str) -> Vec<(i64, i64, Option<String>, String, Vec<u8>)> {
    node.query(
        &format!(
            "SELECT kind_version,hlc,creates,needs,payload FROM {table} \
             WHERE kind='scn.note' ORDER BY counter"
        ),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
}

/// A is restored by a binary that does not know `scn.note`, and the lost
/// branch holds one. A re-issues the entry unchanged, without understanding
/// it; a node that knows the kind applies it exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_unknown_kind_reissued_unchanged_applies_where_known() {
    let knows = FactRegistry::builtins_plus(|b| b.with::<ScnNote>());
    let (a, b, c) = (
        Node::with(85, knows),
        Node::with(86, knows),
        Node::with(87, knows),
    );
    let (thread, creator) = thread_of(&a, "s6");
    a.joined(&thread, &creator, 1).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let backup = a.snapshot("backup.db");
    let i1 = active_log(&a);

    // The branch A will lose: a note and a thread, held by B.
    a.run(|tx, d| {
        journal::author::<ScnNote>(tx, a.ctx(), d, &Note { id: "n1".into() }).map(|_| ())
    })
    .unwrap();
    let lost_threads = author_threads(&a, &creator, 1);
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(note_states(&b), vec!["applied"]);
    let original = note_entries(&b, "journal");
    assert_eq!(original.len(), 1);

    // The restored A runs a binary without the kind and authors elsewhere.
    let lost = a;
    let a = Node::open_restored(85, 8500, FactRegistry::builtins(), &backup, false);
    drop(lost);
    let new_threads = author_threads(&a, &creator, 2);

    let (ra, rb, rc) = (start(&a, 85, 3), start(&b, 86, 4), start(&c, 87, 5));
    ra.link(&rb).await;
    ra.link(&rc).await;

    let everything = |n: &Node| {
        lost_threads
            .iter()
            .chain(&new_threads)
            .all(|t| has_thread(n, t))
    };
    let done = wait_for(Duration::from_secs(20), || {
        let now = active_log(&a);
        now != i1
            && [&a, &b, &c].iter().all(|n| everything(n))
            && note_states(&c) == vec!["applied"]
            && pending(&a) == 0
            && pending(&c) == 0
            && ra.head_of(&now).is_some()
            && ra.head_of(&now) == rb.head_of(&now)
            && ra.head_of(&now) == rc.head_of(&now)
    })
    .await;
    assert!(
        done,
        "{}\n{}\n{}",
        dump("A", &a, &ra),
        dump("B", &b, &rb),
        dump("C", &c, &rc)
    );

    // Unchanged: kind version, HLC, creates, needs and payload bytes equal the
    // original on every node that holds the re-issued copy.
    assert_eq!(
        note_entries(&a, "journal_forked"),
        original,
        "A pulled the original back"
    );
    for node in [&a, &c] {
        assert_eq!(note_entries(node, "journal"), original, "{}", node.id);
    }
    assert_eq!(
        note_entries(&b, "journal").len(),
        2,
        "B holds the original and the re-issued copy"
    );
    assert!(
        note_entries(&b, "journal")
            .iter()
            .all(|e| *e == original[0])
    );
    // A never understood it (its copy waits for a binary that does); C, which
    // knows the kind, applied it. B had applied the original, so the
    // re-issued copy changes nothing: it is not applied a second time.
    let on_a = note_states(&a);
    assert_eq!(on_a.len(), 1);
    assert_ne!(on_a[0], "applied", "A does not know the kind");
    assert_ne!(on_a[0], "pending");
    assert_eq!(note_states(&c), vec!["applied"]);
    let on_b = note_states(&b);
    assert_eq!(on_b.len(), 2, "{on_b:?}");
    assert!(
        on_b.iter().all(|s| s != "pending" && s != "failed"),
        "{on_b:?}"
    );
    assert_projections_equal(&[&a, &b, &c]);
    ra.stop().await;
    rb.stop().await;
    rc.stop().await;
}

// ---- 7. three branches ------------------------------------------------------------

/// One log, three branches. B holds the shared segment plus its own tail
/// (diverging from the owner at `p1 + 1`); C holds a branch that left the
/// common chain earlier (at `p0 + 1`). The owner first learns B's branch and
/// freezes at `p1`; when C's branch arrives its freeze lowers to the minimum
/// divergence `p0`, which exposes the shared segment's rows to re-issue.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_three_branches_freeze_at_min_divergence() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(91, reg),
        Node::with(92, reg),
        Node::with(93, reg),
    );
    let (thread, creator) = thread_of(&a, "s7");
    a.joined(&thread, &creator, 1).unwrap();
    let i1 = active_log(&a);
    let p0 = contiguous(&a, &i1).unwrap();
    let from_p0 = a.snapshot("p0.db");

    let shared = author_threads(&a, &creator, 2);
    fact_exchange::sync(&a.store, &b.store);
    let p1 = contiguous(&a, &i1).unwrap();
    assert!(p1 > p0);
    let from_p1 = a.snapshot("p1.db");
    let b_tail = author_threads(&a, &creator, 2);
    fact_exchange::sync(&a.store, &b.store);

    // C's branch left the common chain at p0.
    let a_c = Node::open_restored(91, 9100, reg, &from_p0, false);
    let c_branch = author_threads(&a_c, &creator, 2);
    fact_exchange::sync(&a_c.store, &c.store);
    drop(a_c);

    // The owner's own branch left it at p1.
    let owner = Node::open_restored(91, 9101, reg, &from_p1, false);
    let own = author_threads(&owner, &creator, 2);
    drop(a);

    let (ro, rb, rc) = (start(&owner, 91, 3), start(&b, 92, 4), start(&c, 93, 5));
    ro.link(&rb).await;
    let first = wait_for(Duration::from_secs(20), || {
        active_log(&owner) != i1
            && frozen_at(&owner, &i1) == Some(p1)
            && b_tail.iter().all(|t| has_thread(&owner, t))
    })
    .await;
    assert!(
        first,
        "{}\n{}",
        dump("owner", &owner, &ro),
        dump("B", &b, &rb)
    );

    ro.link(&rc).await;
    let everyone = [&owner, &b, &c];
    let all = |n: &Node| {
        shared
            .iter()
            .chain(&b_tail)
            .chain(&c_branch)
            .chain(&own)
            .all(|t| has_thread(n, t))
    };
    let done = wait_for(Duration::from_secs(30), || {
        let now = active_log(&owner);
        frozen_at(&owner, &i1) == Some(p0)
            && everyone.iter().all(|n| all(n) && pending(n) == 0)
            && ro.head_of(&now).is_some()
            && ro.head_of(&now) == rb.head_of(&now)
            && ro.head_of(&now) == rc.head_of(&now)
    })
    .await;
    assert!(
        done,
        "{}\n{}\n{}",
        dump("owner", &owner, &ro),
        dump("B", &b, &rb),
        dump("C", &c, &rc)
    );

    assert_eq!(frozen_at(&owner, &i1), Some(p0), "the minimum divergence");
    for n in [&b, &c] {
        let at = frozen_at(n, &i1).unwrap_or_else(|| panic!("I1 frozen on {}", n.id));
        assert!(at >= p0, "a third party never freezes below the divergence");
    }
    assert_eq!(open_clone_evidence(&owner), 0);
    assert_projections_equal(&[&owner, &b, &c]);
    ro.stop().await;
    rb.stop().await;
    rc.stop().await;
}

// ---- 8. clone ------------------------------------------------------------------------

/// A's node directory is copied to A' (one key, two live daemons), and each
/// copy authors its own facts. C links A' first, then A: C closes the newer
/// link with `node_collision` and reports it, and the closed copy (A) records
/// clone evidence. When D, who holds A''s branch, shows A the fork, A neither
/// rotates nor re-issues. After A''s directory is moved aside (A' stops, and
/// would restart as a new node) and `clone-cleared` runs on A, A rotates,
/// re-issues A''s divergent facts and every projection converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_clone_detected_held_then_moved_aside_and_cleared() {
    let reg = registry();
    let (a, c, d) = (
        Node::with(95, reg),
        Node::with(96, reg),
        Node::with(97, reg),
    );
    let (thread, creator) = thread_of(&a, "s8");
    a.joined(&thread, &creator, 1).unwrap();
    let i1 = active_log(&a);
    let copy_of_a = a.snapshot("copy.db");
    let a_clone = Node::open_restored(95, 9500, reg, &copy_of_a, false);
    assert_eq!(active_log(&a_clone), i1, "a copy is the same node");

    // Each copy authors its own facts; D holds the clone's.
    let a_threads = author_threads(&a, &creator, 2);
    let clone_threads = author_threads(&a_clone, &creator, 2);
    fact_exchange::sync(&a_clone.store, &d.store);

    // C links the clone first.
    let (r_clone, rc, rd) = (
        start(&a_clone, 95, 11),
        start(&c, 96, 12),
        start(&d, 97, 13),
    );
    r_clone.link(&rc).await;
    converge(
        "C to hold the clone's branch",
        Duration::from_secs(20),
        || {
            vec![
                (
                    "C has the clone's threads",
                    clone_threads.iter().all(|t| has_thread(&c, t)),
                ),
                ("C applied them", pending(&c) == 0),
            ]
        },
        || dump("C", &c, &rc),
    )
    .await;

    // Then the original, with another boot: C closes this newer link.
    let ra = start(&a, 95, 21);
    ra.link(&rc).await;
    // Observe the close before the combined convergence check, as an earlier
    // poll can when the clone evidence has not yet reached both stores. The
    // report must remain visible to later polls and timeout diagnostics.
    wait_until(
        "A to observe the collision close",
        Duration::from_secs(30),
        || {
            ra.ended_links()
                .iter()
                .any(|why| why.contains("NodeCollision"))
        },
    )
    .await;
    let c_reports = || {
        c.count(&format!(
            "SELECT count(*) FROM sync_clone_evidence WHERE subject='{}' AND cleared_at IS NULL",
            a.id
        ))
    };
    converge(
        "C to close the newer link and report the clone",
        Duration::from_secs(30),
        || {
            vec![
                (
                    "the link ended node_collision",
                    ra.ended_links()
                        .iter()
                        .any(|why| why.contains("NodeCollision")),
                ),
                ("A recorded evidence", open_clone_evidence(&a) >= 1),
                ("C recorded evidence", c_reports() >= 1),
            ]
        },
        || {
            format!(
                "{}\n{}\nended {:?}",
                dump("A", &a, &ra),
                dump("C", &c, &rc),
                ra.ended_links()
            )
        },
    )
    .await;

    // D shows A the fork. A is a clone suspect: nothing rotates or re-issues.
    rd.link(&ra).await;
    converge(
        "A to hold the repair",
        Duration::from_secs(20),
        || vec![("the held line", ra.lines().iter().any(|l| l == HELD_LINE))],
        || dump("A", &a, &ra),
    )
    .await;
    assert_eq!(active_log(&a), i1, "the held copy does not rotate");
    assert_eq!(reissues(&a), 0);
    assert!(status_of(&a).remedy.is_some());

    // A''s directory is moved aside: it stops (and would restart as a new node
    // with a new key). The operator clears the evidence on A.
    r_clone.stop().await;
    drop(a_clone);
    assert!(clear_clone(&a, &ra) >= 1);
    rc.link(&ra).await;

    let everyone = [&a, &c, &d];
    let all = |n: &Node| {
        a_threads
            .iter()
            .chain(&clone_threads)
            .all(|t| has_thread(n, t))
    };
    converge(
        "A to rotate, re-issue and converge",
        Duration::from_secs(30),
        || {
            let now = active_log(&a);
            vec![
                ("A rotated", now != i1),
                (
                    "every node has the threads",
                    everyone.iter().all(|n| all(n)),
                ),
                ("nothing pending", everyone.iter().all(|n| pending(n) == 0)),
                ("A holds its new log", ra.head_of(&now).is_some()),
                ("C's head matches", ra.head_of(&now) == rc.head_of(&now)),
                ("D's head matches", ra.head_of(&now) == rd.head_of(&now)),
            ]
        },
        || {
            format!(
                "{}\n{}\n{}",
                dump("A", &a, &ra),
                dump("C", &c, &rc),
                dump("D", &d, &rd)
            )
        },
    )
    .await;
    assert!(reissues(&a) > 0, "A' divergent facts were re-issued");
    assert!(frozen_at(&a, &i1).is_some());
    assert_eq!(open_clone_evidence(&a), 0);
    assert_projections_equal(&[&a, &c, &d]);
    ra.stop().await;
    rc.stop().await;
    rd.stop().await;
}
