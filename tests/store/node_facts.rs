//! `node.genesis` directory rows, tag collisions, `node.retag` and the
//! post-commit identity update (journal-core design sections 9 and 11).
//!
//! Nodes are stores with their own keys; a scenario ships one node's log to
//! another with `journal::ingest` and drives `apply_pass` on the receiver.

use super::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;

use crate::protocol::ids::public_id_parts;
use crate::protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::apply::{ApplyLimits, apply_pass};
use crate::store::connection::StoreContext;
use crate::store::journal::{self as jr, OpenReport};
use crate::sync::facts::{FactRegistry, NodeGenesis};
use crate::sync::journal_port::{ApplyProgress, IngestOutcome};
use crate::sync::{BatchLimits, LogKey};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn seed_bytes(n: u32) -> [u8; 32] {
    let mut seed = [7u8; 32];
    seed[..4].copy_from_slice(&n.to_le_bytes());
    seed
}

fn uuid_of(n: u32) -> NodeUuid {
    NodeIdentity::from_seed(seed_bytes(n)).uuid()
}

/// Two seeds whose nodes carry the same tag at `k = 0`, as `(lower, higher)` by
/// uuid. Found by search: 62^3 tags make a pair appear within a few thousand
/// draws, and the search is deterministic.
fn colliding_seeds() -> (u32, u32) {
    let mut seen: std::collections::HashMap<NodeTag, u32> = std::collections::HashMap::new();
    for n in 0..200_000 {
        let tag = tag_for(uuid_of(n), 0);
        if let Some(&earlier) = seen.get(&tag) {
            return if uuid_of(earlier) < uuid_of(n) {
                (earlier, n)
            } else {
                (n, earlier)
            };
        }
        seen.insert(tag, n);
    }
    panic!("no tag collision within 200000 seeds");
}

/// A kind named `node.retag` that accepts any payload, so a test node can
/// author facts the shipped kind rejects.
struct LooseRetag;
impl FactKind for LooseRetag {
    const NAME: &'static str = "node.retag";
    const MAJOR: u32 = 1;
    type Payload = RetagPayload2;
    fn creates(_: &RetagPayload2) -> Option<EntityKey> {
        None
    }
    fn needs(_: &RetagPayload2) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(
        _: &Transaction<'_>,
        _: &mut ApplyContext<'_>,
        _: RetagPayload2,
    ) -> Result<(), FactError> {
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RetagPayload2 {
    k: u32,
    tag: String,
}

fn loose_registry() -> &'static FactRegistry {
    Box::leak(Box::new(
        FactRegistry::builder()
            .with::<NodeGenesis>()
            .with::<LooseRetag>()
            .build(),
    ))
}

/// One node: a store file, its context and writer, its key and active log.
struct Node {
    path: PathBuf,
    identity: Arc<NodeIdentity>,
    ctx: StoreContext,
    conn: Connection,
    log: LogKey,
}

impl Node {
    fn new(n: u32) -> Self {
        Self::with_registry(n, FactRegistry::builtins())
    }

    fn with_registry(n: u32, registry: &'static FactRegistry) -> Self {
        let path = std::env::temp_dir().join(format!("ht-node-facts-{}.db", uuid::Uuid::new_v4()));
        let identity = Arc::new(NodeIdentity::from_seed(seed_bytes(n)));
        let ctx = StoreContext::new(path.clone(), Arc::new(FixedClock))
            .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
        ctx.seed_incarnations(u64::from(n) + 1);
        let mut conn = ctx.open_writer().unwrap();
        let report: OpenReport =
            jr::open(&ctx, &mut conn, Arc::clone(&identity), "host", registry).unwrap();
        Self {
            path,
            identity,
            ctx,
            conn,
            log: report.self_log,
        }
    }

    fn uuid(&self) -> NodeUuid {
        self.identity.uuid()
    }

    fn author<K: FactKind>(&mut self, payload: &K::Payload) -> Result<(), ApiError> {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| jr::author::<K>(tx, ctx, d, payload),
        )
        .map(|_| ())
    }

    fn pass(&mut self) -> ApplyProgress {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| apply_pass(tx, ctx, d, ApplyLimits::default()),
        )
        .unwrap()
    }

    /// Ingests `from`'s whole active log.
    fn ship(&mut self, from: &Node) {
        let entries = jr::serve(&from.conn, &from.log, 1, BatchLimits::default()).unwrap();
        let outcome = jr::ingest(&self.ctx, &mut self.conn, &from.log, entries, &budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { .. }),
            "{outcome:?}"
        );
    }

    /// `(node_uuid, tag, tag_k)` of every directory row.
    fn directory(&self) -> Vec<(String, String, u32)> {
        let mut statement = self
            .conn
            .prepare("SELECT node_uuid,tag,tag_k FROM node_directory ORDER BY node_uuid")
            .unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// `(tag, tag_k)` of the `local_node` row.
    fn local_tag(&self) -> (String, u32) {
        self.conn
            .query_row("SELECT tag,tag_k FROM local_node", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// `(apply_state, apply_error)` of one entry of `log`.
    fn entry(&self, log: &LogKey, counter: i64) -> (String, Option<String>) {
        self.conn
            .query_row(
                "SELECT j.apply_state,j.apply_error FROM journal j \
                 JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=?3",
                rusqlite::params![log.node.to_string(), log.incarnation, counter],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn publish(&self, json: &Path) -> std::io::Result<Option<u32>> {
        crate::sync::facts::publish_committed_tag(&self.ctx, &self.identity, json, &budget())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.path.clone().into_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(name);
        }
    }
}

fn json_path() -> PathBuf {
    std::env::temp_dir().join(format!("ht-node-facts-{}.json", uuid::Uuid::new_v4()))
}

struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn genesis_fills_node_directory() {
    let (a, b) = (1, 2);
    assert_ne!(tag_for(uuid_of(a), 0), tag_for(uuid_of(b), 0));
    let (a, mut b) = (Node::new(a), Node::new(b));
    assert_eq!(
        b.directory().len(),
        1,
        "only the self row before the exchange"
    );
    b.ship(&a);
    let progress = b.pass();
    assert_eq!(progress.applied, 1, "{progress:?}");
    assert_eq!(progress.self_tag_k, None, "no collision, no retag");
    let (tag, tag_k, label, hlc): (String, u32, Option<String>, Option<i64>) = b
        .conn
        .query_row(
            "SELECT tag,tag_k,host_label,tag_hlc FROM node_directory WHERE node_uuid=?1",
            [a.uuid().to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(tag, tag_for(a.uuid(), 0).to_string());
    assert_eq!(tag_k, 0);
    assert_eq!(label.as_deref(), Some("host"));
    assert!(hlc.is_some());
    assert_eq!(b.directory().len(), 2);
    assert_eq!(
        b.count("SELECT COUNT(*) FROM journal WHERE kind='node.retag'"),
        0
    );
}

#[test]
fn higher_uuid_retags_lower_does_not() {
    let (lo_seed, hi_seed) = colliding_seeds();
    let (mut lo, mut hi) = (Node::new(lo_seed), Node::new(hi_seed));
    assert!(lo.uuid() < hi.uuid());
    let collided = tag_for(lo.uuid(), 0);
    assert_eq!(collided, tag_for(hi.uuid(), 0));

    // The higher node learns of the lower one's tag and moves.
    hi.ship(&lo);
    let progress = hi.pass();
    assert_eq!(progress.applied, 1, "{progress:?}");
    assert_eq!(
        progress.self_tag_k,
        Some(1),
        "the apply pass signals the tag change for the post-commit update"
    );
    let moved = tag_for(hi.uuid(), 1);
    assert_ne!(moved, collided);
    assert_eq!(hi.local_tag(), (moved.to_string(), 1));
    assert_eq!(
        hi.entry(&hi.log, 2),
        ("applied".to_owned(), None),
        "the retag is the higher node's counter 2"
    );
    assert_eq!(
        hi.count("SELECT COUNT(*) FROM journal WHERE kind='node.retag'"),
        1
    );

    // The lower node learns of the higher one's genesis and retag and keeps its tag.
    lo.ship(&hi);
    let progress = lo.pass();
    assert_eq!(progress.applied, 2, "{progress:?}");
    assert_eq!(progress.self_tag_k, None);
    assert_eq!(lo.local_tag(), (collided.to_string(), 0));
    assert_eq!(
        lo.count("SELECT contiguous_counter FROM sync_logs WHERE is_self=1"),
        1,
        "the lower node authored nothing"
    );
    assert_eq!(
        lo.count("SELECT COUNT(*) FROM journal WHERE kind='node.retag'"),
        1
    );

    // Both directories agree.
    assert_eq!(lo.directory(), hi.directory());
    let directory = hi.directory();
    assert!(directory.contains(&(hi.uuid().to_string(), moved.to_string(), 1)));
    assert!(directory.contains(&(lo.uuid().to_string(), collided.to_string(), 0)));

    // A second pass has nothing left to do and authors nothing.
    assert_eq!(hi.pass().applied, 0);
    assert_eq!(
        hi.count("SELECT COUNT(*) FROM journal WHERE kind='node.retag'"),
        1
    );
}

#[test]
fn fresh_mints_under_new_tag_after_retag() {
    let (lo_seed, hi_seed) = colliding_seeds();
    let (lo, mut hi) = (Node::new(lo_seed), Node::new(hi_seed));
    let before = crate::store::public_ids::fresh(&hi.conn, "s", &[]).unwrap();
    assert_eq!(
        public_id_parts("s", &before).unwrap().0,
        tag_for(hi.uuid(), 0)
    );
    hi.ship(&lo);
    hi.pass();
    let after = crate::store::public_ids::fresh(&hi.conn, "s", &[]).unwrap();
    assert_eq!(after.len(), before.len(), "no length change");
    assert_eq!(
        public_id_parts("s", &after).unwrap().0,
        tag_for(hi.uuid(), 1),
        "{after}"
    );
}

#[test]
fn set_tag_k_post_commit_not_on_rollback() {
    let (lo_seed, hi_seed) = colliding_seeds();
    let (lo, mut hi) = (Node::new(lo_seed), Node::new(hi_seed));
    let json = RemoveOnDrop(json_path());
    crate::node::json::publish_identity(&json.0, &hi.identity).unwrap();
    hi.ship(&lo);

    // The pass retags inside its transaction, then the transaction rolls back.
    let ctx = &hi.ctx;
    let failed = ctx.execute_decision(
        &mut hi.conn,
        |_| Ok(()),
        |tx, d, ()| -> Result<(), ApiError> {
            let progress = apply_pass(tx, ctx, d, ApplyLimits::default())?;
            assert_eq!(progress.self_tag_k, Some(1));
            Err(ApiError::new(ErrorCode::StoreBusy, "rolled back"))
        },
    );
    assert!(failed.is_err());
    assert_eq!(hi.local_tag().1, 0, "the retag rolled back with the pass");
    assert_eq!(hi.identity.tag_k(), 0);
    assert_eq!(hi.publish(&json.0).unwrap(), None);
    assert_eq!(hi.identity.tag_k(), 0);
    let hint = crate::node::json::read_hint(&json.0).unwrap();
    assert_eq!(hint.tag_k, 0);

    // The committed pass still leaves the identity alone: only the
    // post-commit helper raises it, and node.json follows.
    let progress = hi.pass();
    assert_eq!(progress.self_tag_k, Some(1));
    assert_eq!(hi.identity.tag_k(), 0);
    assert_eq!(hi.publish(&json.0).unwrap(), Some(1));
    assert_eq!(hi.identity.tag_k(), 1);
    let hint = crate::node::json::read_hint(&json.0).unwrap();
    assert_eq!((hint.tag, hint.tag_k), (tag_for(hi.uuid(), 1), 1));
}

#[test]
fn publish_committed_tag_noop_when_equal() {
    let node = Node::new(5);
    let json = RemoveOnDrop(json_path());
    assert_eq!(node.publish(&json.0).unwrap(), None);
    assert!(!json.0.exists(), "a no-op writes no node.json");

    crate::node::json::publish_identity(&json.0, &node.identity).unwrap();
    let before = std::fs::read(&json.0).unwrap();
    assert_eq!(node.publish(&json.0).unwrap(), None);
    assert_eq!(std::fs::read(&json.0).unwrap(), before);
    assert_eq!(node.identity.tag_k(), 0);
}

#[test]
fn publish_after_simulated_crash_updates_identity_and_node_json() {
    let node = Node::new(6);
    let json = RemoveOnDrop(json_path());
    crate::node::json::publish_identity(&json.0, &node.identity).unwrap();

    // The retag committed, the process died before the identity and node.json
    // followed: local_node is ahead of both.
    let committed = tag_for(node.uuid(), 2);
    node.conn
        .execute(
            "UPDATE local_node SET tag=?1, tag_k=2",
            [committed.to_string()],
        )
        .unwrap();
    assert_eq!(node.identity.tag_k(), 0);
    assert_eq!(crate::node::json::read_hint(&json.0).unwrap().tag_k, 0);

    assert_eq!(node.publish(&json.0).unwrap(), Some(2));
    assert_eq!(node.identity.tag_k(), 2);
    assert_eq!(node.identity.tag(), committed);
    let hint = crate::node::json::read_hint(&json.0).unwrap();
    assert_eq!((hint.tag, hint.tag_k), (committed, 2));

    assert_eq!(node.publish(&json.0).unwrap(), None, "idempotent");
}

#[test]
fn retag_with_mismatched_tag_is_failed() {
    let mut author = Node::with_registry(8, loose_registry());
    let mut receiver = Node::new(9);
    let wrong_tag = tag_for(author.uuid(), 5).to_string();
    let right_tag = tag_for(author.uuid(), 1).to_string();
    // counter 2: k=1 with another k's tag; counter 3: k=0; counter 4: valid.
    for (k, tag) in [
        (1, wrong_tag.as_str()),
        (0, right_tag.as_str()),
        (1, right_tag.as_str()),
    ] {
        author
            .author::<LooseRetag>(&RetagPayload2 {
                k,
                tag: tag.to_owned(),
            })
            .unwrap();
    }
    receiver.ship(&author);
    let progress = receiver.pass();
    assert_eq!(progress.failed, 2, "{progress:?}");
    assert_eq!(
        progress.applied, 2,
        "genesis and the valid retag: {progress:?}"
    );
    for counter in [2, 3] {
        let (state, error) = receiver.entry(&author.log, counter);
        assert_eq!(state, "failed", "counter {counter}");
        assert!(
            error.as_deref().is_some_and(|e| e.contains("node.retag")),
            "{error:?}"
        );
    }
    assert_eq!(receiver.entry(&author.log, 4).0, "applied");
    let row: (String, u32) = receiver
        .conn
        .query_row(
            "SELECT tag,tag_k FROM node_directory WHERE node_uuid=?1",
            [author.uuid().to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (right_tag, 1),
        "the failed retags left no directory write; the valid one set it"
    );
}
