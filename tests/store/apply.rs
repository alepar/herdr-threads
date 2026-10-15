//! ApplyEngine passes, replay and follow-ups (journal-core design section 7).
//!
//! Nodes are stores with their own seeds; a scenario authors on one node and
//! ships the entries to another with `journal::ingest`, then drives
//! `apply_pass` / `replay` on the receiver.

use super::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::node::NodeIdentity;
use crate::protocol::time::{Cancellation, UtcMillis};
use crate::store::journal::{self as jr, Authored, OpenReport};
use crate::sync::facts::FactRegistry;
use crate::sync::journal_port::IngestOutcome;
use crate::sync::{BatchLimits, Vector};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

// ---- test kinds -------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Put {
    id: String,
    #[serde(default)]
    need: Option<String>,
}

fn put(id: &str) -> Put {
    Put {
        id: id.into(),
        need: None,
    }
}

fn put_needing(id: &str, need: &str) -> Put {
    Put {
        id: id.into(),
        need: Some(need.into()),
    }
}

fn key(text: String) -> Option<EntityKey> {
    EntityKey::new(text).ok()
}

fn bump(tx: &Transaction<'_>) -> Result<(), FactError> {
    tx.execute(
        "UPDATE local_node SET send_eligibility_revision=send_eligibility_revision+1",
        [],
    )
    .map(|_| ())
    .map_err(|e| FactError::from_api(store_error(e)))
}

fn put_needs(p: &Put) -> Vec<EntityKey> {
    p.need.iter().filter_map(|n| key(n.clone())).collect()
}

/// Bumps a local counter, so a handler's writes are observable.
struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(p: &Put) -> Vec<EntityKey> {
        put_needs(p)
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        bump(tx)
    }
}

/// `test.put` at a major the base registry does not know.
struct PutV2;
impl FactKind for PutV2 {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 2;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(p: &Put) -> Vec<EntityKey> {
        put_needs(p)
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        bump(tx)
    }
}

/// A kind only the rich registry knows.
struct FutureKind;
impl FactKind for FutureKind {
    const NAME: &'static str = "test.future";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(p: &Put) -> Vec<EntityKey> {
        put_needs(p)
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        bump(tx)
    }
}

#[derive(Serialize, Deserialize)]
struct Remote {
    id: String,
    mode: String,
    #[serde(default)]
    min: i64,
}

fn remote(id: &str, mode: &str) -> Remote {
    Remote {
        id: id.into(),
        mode: mode.into(),
        min: 0,
    }
}

/// Always applies on the node that authored it; on any other node it bumps the
/// revision and then misbehaves per `mode`.
struct RemoteKind;
impl FactKind for RemoteKind {
    const NAME: &'static str = "test.remote";
    const MAJOR: u32 = 1;
    type Payload = Remote;
    fn creates(p: &Remote) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(_: &Remote) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Remote) -> Result<(), FactError> {
        bump(tx)?;
        if cx.is_local_origin() {
            return Ok(());
        }
        match p.mode.as_str() {
            "reject" => Err(FactError::Rejected(ApiError::new(
                ErrorCode::NotFound,
                "x".repeat(2000),
            ))),
            "store" => Err(FactError::Store(ApiError::new(
                ErrorCode::StoreBusy,
                "busy",
            ))),
            "dupe" => {
                for _ in 0..2 {
                    tx.execute(
                        "INSERT INTO thread_registers(thread_id,field,value,hlc,node_uuid,src_ingest_seq) \
                         VALUES ('t','topic','x',1,?1,?2)",
                        params![cx.local_node.to_string(), cx.ingest_seq],
                    )
                    .map_err(|e| FactError::from_api(store_error(e)))?;
                }
                Ok(())
            }
            "gate" => {
                let revision: i64 = tx
                    .query_row(
                        "SELECT send_eligibility_revision FROM local_node",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| FactError::from_api(store_error(e)))?;
                if revision < p.min {
                    Err(FactError::Rejected(ApiError::new(
                        ErrorCode::NotFound,
                        "gate closed",
                    )))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Child {
    id: String,
    #[serde(default)]
    fail: bool,
}

struct ChildKind;
impl FactKind for ChildKind {
    const NAME: &'static str = "test.child";
    const MAJOR: u32 = 1;
    type Payload = Child;
    fn creates(p: &Child) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(_: &Child) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, p: Child) -> Result<(), FactError> {
        bump(tx)?;
        if p.fail {
            return Err(FactError::Rejected(ApiError::new(
                ErrorCode::NotFound,
                "child refused",
            )));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Parent {
    id: String,
    #[serde(default)]
    fail_child: bool,
}

/// On a remote origin, queues a child follow-up.
struct ParentKind;
impl FactKind for ParentKind {
    const NAME: &'static str = "test.parent";
    const MAJOR: u32 = 1;
    type Payload = Parent;
    fn creates(p: &Parent) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(_: &Parent) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Parent) -> Result<(), FactError> {
        bump(tx)?;
        if !cx.is_local_origin() {
            cx.follow_up::<ChildKind>(&Child {
                id: format!("c-{}", p.id),
                fail: p.fail_child,
            })
            .map_err(|e| {
                FactError::Rejected(ApiError::new(ErrorCode::InvalidRequest, e.to_string()))
            })?;
        }
        Ok(())
    }
}

fn base() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| {
        b.with::<PutKind>()
            .with::<RemoteKind>()
            .with::<ParentKind>()
            .with::<ChildKind>()
    })
}

fn rich() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| {
        b.with::<PutKind>()
            .with::<RemoteKind>()
            .with::<ParentKind>()
            .with::<ChildKind>()
            .with::<PutV2>()
            .with::<FutureKind>()
    })
}

// ---- nodes ------------------------------------------------------------------

/// One node: a store file, its context and writer, and its active log.
struct Node {
    path: PathBuf,
    seed: u8,
    ctx: StoreContext,
    conn: Connection,
    log: LogKey,
}

impl Node {
    fn new(seed: u8, registry: &'static FactRegistry) -> Self {
        let path = std::env::temp_dir().join(format!("ht-apply-{}.db", uuid::Uuid::new_v4()));
        let (ctx, conn, report) = Self::open_at(&path, seed, registry);
        Self {
            path,
            seed,
            ctx,
            conn,
            log: report.self_log,
        }
    }

    fn open_at(
        path: &Path,
        seed: u8,
        registry: &'static FactRegistry,
    ) -> (StoreContext, Connection, OpenReport) {
        let identity = Arc::new(NodeIdentity::from_seed([seed; 32]));
        let ctx = StoreContext::new(path.to_path_buf(), Arc::new(FixedClock))
            .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
        ctx.seed_incarnations(u64::from(seed));
        let mut conn = ctx.open_writer().unwrap();
        let report = jr::open(&ctx, &mut conn, identity, "host", registry).unwrap();
        (ctx, conn, report)
    }

    /// Restarts the node with another registry (an upgrade).
    fn reopen(&mut self, registry: &'static FactRegistry) -> OpenReport {
        let (ctx, conn, report) = Self::open_at(&self.path, self.seed, registry);
        self.ctx = ctx;
        self.conn = conn;
        report
    }

    fn author<K: FactKind>(&mut self, payload: &K::Payload) -> Authored {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| jr::author::<K>(tx, ctx, d, payload),
        )
        .unwrap()
    }

    fn try_pass(&mut self, limits: ApplyLimits) -> Result<ApplyProgress, ApiError> {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| apply_pass(tx, ctx, d, limits),
        )
    }

    fn pass(&mut self) -> ApplyProgress {
        self.try_pass(ApplyLimits::default()).unwrap()
    }

    fn replay(&mut self, scope: ReplayScope<'_>) -> ApplyProgress {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| replay(tx, ctx, d, scope, ApplyLimits::default()),
        )
        .unwrap()
    }

    /// Ingests `from`'s entries of `log` with counters `1..=upto`.
    fn ship(&mut self, from: &Node, log: LogKey, upto: u64) {
        let entries: Vec<_> = jr::serve(&from.conn, &log, 1, BatchLimits::default())
            .unwrap()
            .into_iter()
            .filter(|e| e.counter <= upto)
            .collect();
        let outcome = jr::ingest(&self.ctx, &mut self.conn, &log, entries, &budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { .. }),
            "{outcome:?}"
        );
    }

    fn freeze(&mut self, log: &LogKey, at: u64) {
        self.ctx
            .execute_decision(
                &mut self.conn,
                |_| Ok(()),
                |tx, _, ()| jr::freeze(tx, log, at),
            )
            .unwrap();
    }

    /// `(apply_state, applied_seq, apply_error)` of one entry.
    fn row(&self, log: &LogKey, counter: u64) -> (String, Option<i64>, Option<String>) {
        self.conn
            .query_row(
                "SELECT j.apply_state,j.applied_seq,j.apply_error FROM journal j \
                 JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=?3",
                params![log.node.to_string(), log.incarnation, counter as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn state(&self, log: &LogKey, counter: u64) -> String {
        self.row(log, counter).0
    }

    /// `(applied_through, contiguous_counter, blocked_on)` of a log.
    fn log_row(&self, log: &LogKey) -> (i64, i64, Option<String>) {
        self.conn
            .query_row(
                "SELECT applied_through,contiguous_counter,blocked_on FROM sync_logs \
                 WHERE node_uuid=?1 AND incarnation=?2",
                params![log.node.to_string(), log.incarnation],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn revision(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT send_eligibility_revision FROM local_node",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.conn.query_row(sql, [], |r| r.get(0)).unwrap()
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

fn blocked_text(log: &LogKey, counter: u64) -> String {
    let node = log.node.to_string();
    format!("deps {}/{}@{counter}", &node[..8], log.incarnation)
}

// ---- readiness --------------------------------------------------------------

#[test]
fn deps_wait_with_blocked_on_then_apply() {
    let mut a = Node::new(11, base());
    a.author::<PutKind>(&put("a1"));
    let mut b = Node::new(12, base());
    b.ship(&a, a.log, 2);
    b.pass();
    b.author::<PutKind>(&put("b1"));

    let mut c = Node::new(13, base());
    c.ship(&b, b.log, 2);
    let progress = c.pass();
    assert_eq!(
        progress.applied, 1,
        "only B's genesis is ready: {progress:?}"
    );
    assert_eq!(c.state(&b.log, 1), "applied");
    assert_eq!(c.state(&b.log, 2), "pending");
    assert_eq!(
        c.log_row(&b.log),
        (1, 2, Some(blocked_text(&a.log, 2))),
        "A has no row yet"
    );

    // No news: another pass writes nothing (blocked_on is written on change).
    let before = c.conn.total_changes();
    c.pass();
    assert_eq!(c.conn.total_changes(), before);

    // A arrives only up to its genesis: still blocked, same text.
    c.ship(&a, a.log, 1);
    c.pass();
    assert_eq!(c.log_row(&a.log).0, 1);
    assert_eq!(c.log_row(&b.log), (1, 2, Some(blocked_text(&a.log, 2))));

    c.ship(&a, a.log, 2);
    let progress = c.pass();
    assert_eq!(progress.applied, 2, "A@2 then B@2: {progress:?}");
    assert_eq!(c.state(&b.log, 2), "applied");
    assert_eq!(c.log_row(&b.log), (2, 2, None));
    assert_eq!(c.log_row(&a.log), (2, 2, None));
}

#[test]
fn frozen_dep_capped_at_frozen_at() {
    let mut a = Node::new(14, base());
    a.author::<PutKind>(&put("a1"));
    let mut b = Node::new(15, base());
    b.ship(&a, a.log, 2);
    b.pass();
    b.author::<PutKind>(&put("b1")); // deps A@2

    // Control: A known only up to its genesis, not frozen: B@2 waits.
    let mut open = Node::new(16, base());
    open.ship(&a, a.log, 1);
    open.ship(&b, b.log, 2);
    open.pass();
    assert_eq!(open.state(&b.log, 2), "pending");

    let mut c = Node::new(17, base());
    c.ship(&a, a.log, 1);
    c.freeze(&a.log, 1);
    c.ship(&b, b.log, 2);
    c.pass();
    assert_eq!(
        c.state(&b.log, 2),
        "applied",
        "A@2 is capped at frozen_at 1"
    );
    assert_eq!(c.log_row(&a.log).0, 1);
}

// ---- outcomes ---------------------------------------------------------------

#[test]
fn unknown_kind_and_newer_major_park_unknown_and_advance() {
    let mut a = Node::new(18, rich());
    a.author::<PutKind>(&put("p1")); // 2
    a.author::<FutureKind>(&put("f1")); // 3
    a.author::<PutV2>(&put("v2")); // 4
    a.author::<PutKind>(&put("p2")); // 5

    let mut b = Node::new(19, base());
    b.ship(&a, a.log, 5);
    let progress = b.pass();
    assert_eq!((progress.applied, progress.parked), (3, 2), "{progress:?}");
    assert_eq!(b.state(&a.log, 2), "applied");
    assert_eq!(b.state(&a.log, 3), "parked_unknown");
    assert_eq!(b.state(&a.log, 4), "parked_unknown");
    assert_eq!(
        b.state(&a.log, 5),
        "applied",
        "later entries are not wedged"
    );
    assert_eq!(b.log_row(&a.log), (5, 5, None));
    assert!(
        b.row(&a.log, 4)
            .2
            .unwrap()
            .contains("unknown kind test.put v2")
    );
}

#[test]
fn parked_dep_reattempted_same_pass() {
    let mut x = Node::new(21, base());
    x.author::<PutKind>(&put("foo")); // X@2 creates e:foo
    let mut d = Node::new(22, base());
    d.ship(&x, x.log, 2);
    d.pass();
    d.author::<PutKind>(&put_needing("use", "e:foo")); // D@2, deps X@2

    // C ingests X's genesis, then D's log, then X@2 (a later ingest_seq than
    // D@2), and freezes X at 1 so D@2 is ready before its creator applied.
    let mut c = Node::new(23, base());
    c.ship(&x, x.log, 1);
    c.ship(&d, d.log, 2);
    c.ship(&x, x.log, 2);
    c.freeze(&x.log, 1);

    let progress = c.pass();
    assert_eq!(c.state(&d.log, 2), "applied");
    assert_eq!(c.state(&x.log, 2), "applied");
    assert_eq!(progress.replayed, 1, "D@2 was parked, then re-attempted");
    assert_eq!(progress.parked, 1, "{progress:?}");
    let creator_seq = c.row(&x.log, 2).1.unwrap();
    let dependent_seq = c.row(&d.log, 2).1.unwrap();
    assert!(
        dependent_seq > creator_seq,
        "late position after its creator"
    );
    assert_eq!(c.log_row(&d.log), (2, 2, None));
}

#[test]
fn failed_creator_parks_dependents() {
    let mut a = Node::new(24, base());
    a.author::<RemoteKind>(&remote("r", "reject")); // A@2 creates e:r
    a.author::<PutKind>(&put_needing("y", "e:r")); // A@3

    let mut c = Node::new(25, base());
    c.ship(&a, a.log, 3);
    c.pass();
    assert_eq!(c.state(&a.log, 2), "failed");
    let (state, seq, error) = c.row(&a.log, 3);
    assert_eq!((state.as_str(), seq), ("parked_dep", None));
    assert!(
        error.unwrap().contains("needs e:r: creator failed"),
        "names the key and the creator's state"
    );
    let parked_on: String = c
        .conn
        .query_row(
            "SELECT j.parked_on FROM journal j JOIN sync_logs l USING(log_ix) \
             WHERE l.node_uuid=?1 AND j.counter=3",
            [a.log.node.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parked_on, "e:r");
    assert_eq!(
        c.log_row(&a.log),
        (3, 3, None),
        "parked entries still count"
    );
}

#[test]
fn rejected_is_failed_bounded_message_rows_gone() {
    let mut a = Node::new(26, base());
    a.author::<RemoteKind>(&remote("r", "reject"));
    let mut c = Node::new(27, base());
    let revision = c.revision();
    c.ship(&a, a.log, 2);
    c.pass();
    let (state, seq, error) = c.row(&a.log, 2);
    assert_eq!((state.as_str(), seq), ("failed", None));
    let error = error.unwrap();
    assert_eq!(error.len(), 512, "2000-byte detail cut to the bound");
    assert!(error.starts_with("not_found: xxx"), "{}", &error[..40]);
    assert_eq!(
        c.revision(),
        revision,
        "the handler's write before the rejection is gone"
    );
    assert_eq!(c.log_row(&a.log).0, 2);
}

#[test]
fn store_error_aborts_quantum_nothing_changed() {
    let mut a = Node::new(28, base());
    a.author::<PutKind>(&put("p0")); // 2
    a.author::<RemoteKind>(&remote("s", "store")); // 3
    let mut c = Node::new(29, base());
    c.ship(&a, a.log, 3);
    let revision = c.revision();

    let error = c.try_pass(ApplyLimits::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert_eq!(c.revision(), revision);
    assert_eq!(c.log_row(&a.log), (0, 3, None), "nothing advanced");
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal WHERE apply_state='pending'"),
        3
    );
    assert_eq!(c.state(&a.log, 2), "pending", "even the entry before it");
}

#[test]
fn constraint_violation_becomes_failed_not_wedge() {
    let mut a = Node::new(30, base());
    a.author::<RemoteKind>(&remote("d", "dupe")); // 2
    a.author::<PutKind>(&put("after")); // 3
    let mut c = Node::new(31, base());
    c.ship(&a, a.log, 3);
    c.pass();
    let (state, _, error) = c.row(&a.log, 2);
    assert_eq!(state, "failed");
    assert!(error.unwrap().starts_with("conflict:"));
    assert_eq!(c.state(&a.log, 3), "applied");
    assert_eq!(c.log_row(&a.log), (3, 3, None));
    assert_eq!(
        c.count("SELECT COUNT(*) FROM thread_registers"),
        0,
        "the first insert was rolled back with the savepoint"
    );
}

#[test]
fn applied_seq_unique_increasing() {
    let mut a = Node::new(32, base());
    for i in 0..5 {
        a.author::<PutKind>(&put(&format!("p{i}")));
    }
    let mut c = Node::new(33, base());
    c.ship(&a, a.log, 6);
    c.pass();
    let seqs: Vec<i64> = {
        let mut statement = c
            .conn
            .prepare(
                "SELECT applied_seq FROM journal WHERE apply_state='applied' ORDER BY ingest_seq",
            )
            .unwrap();
        statement
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(seqs.len(), 7, "own genesis, A's genesis and five puts");
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
}

#[test]
fn limits_report_has_more() {
    let mut a = Node::new(34, base());
    a.author::<PutKind>(&put("p1"));
    a.author::<PutKind>(&put("p2"));
    let mut c = Node::new(35, base());
    c.ship(&a, a.log, 3);
    let limits = ApplyLimits {
        entries: 1,
        wall_ms: 100,
    };
    let progress = c.try_pass(limits).unwrap();
    assert_eq!((progress.applied, progress.has_more), (1, true));
    assert_eq!(c.log_row(&a.log).0, 1);
    let progress = c.pass();
    assert_eq!((progress.applied, progress.has_more), (2, false));
}

// ---- follow-ups -------------------------------------------------------------

#[test]
fn follow_up_deps_include_trigger() {
    let mut a = Node::new(36, base());
    a.author::<ParentKind>(&Parent {
        id: "p".into(),
        fail_child: false,
    }); // A@2
    let mut b = Node::new(37, base());
    b.ship(&a, a.log, 2);
    let progress = b.pass();
    assert_eq!(progress.applied, 2, "A's genesis and the parent");
    assert_eq!(b.state(&a.log, 2), "applied");

    // B's log: genesis, then the child the parent's handler queued.
    assert_eq!(b.log_row(&b.log), (2, 2, None));
    let (kind, state): (String, String) = b
        .conn
        .query_row(
            "SELECT j.kind,j.apply_state FROM journal j JOIN sync_logs l USING(log_ix) \
             WHERE l.node_uuid=?1 AND j.counter=2",
            [b.log.node.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((kind.as_str(), state.as_str()), ("test.child", "applied"));
    let deps: Vec<u8> = b
        .conn
        .query_row(
            "SELECT j.deps FROM journal j JOIN sync_logs l USING(log_ix) \
             WHERE l.node_uuid=?1 AND j.counter=2",
            [b.log.node.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let deps = crate::sync::envelope::Deps::decode(&deps, &b.log).unwrap();
    assert!(
        deps.entries().contains(&(a.log, 2)),
        "the follow-up follows its trigger: {:?}",
        deps.entries()
    );
}

#[test]
fn failed_follow_up_marks_trigger_failed() {
    let mut a = Node::new(38, base());
    a.author::<ParentKind>(&Parent {
        id: "p".into(),
        fail_child: true,
    });
    let mut b = Node::new(39, base());
    let revision = b.revision();
    b.ship(&a, a.log, 2);
    let progress = b.pass();
    assert_eq!((progress.applied, progress.failed), (1, 1), "{progress:?}");
    let (state, seq, error) = b.row(&a.log, 2);
    assert_eq!((state.as_str(), seq), ("failed", None));
    let error = error.unwrap();
    assert!(
        error.starts_with("follow-up test.child: not_found: child refused"),
        "{error}"
    );
    assert_eq!(b.log_row(&b.log), (1, 1, None), "no child was kept");
    assert_eq!(b.revision(), revision, "the trigger's writes are gone");
    assert_eq!(b.log_row(&a.log), (2, 2, None));
}

// ---- replay -----------------------------------------------------------------

#[test]
fn startup_replay_applies_old_parked_unknown_in_order() {
    let mut a = Node::new(40, rich());
    a.author::<PutKind>(&put("p1")); // 2
    a.author::<FutureKind>(&put("f1")); // 3
    a.author::<FutureKind>(&put("f2")); // 4
    a.author::<PutKind>(&put("p2")); // 5

    let mut b = Node::new(41, base());
    b.ship(&a, a.log, 5);
    b.pass();
    assert_eq!(b.state(&a.log, 3), "parked_unknown");
    assert_eq!(b.state(&a.log, 4), "parked_unknown");
    let last_before = b.row(&a.log, 5).1.unwrap();

    // Nothing the base registry cannot handle is re-attempted at startup.
    let progress = b.replay(ReplayScope::Startup);
    assert_eq!(progress, ApplyProgress::default());

    let report = b.reopen(rich());
    assert_eq!(report.parked_unknown, 2);
    let progress = b.replay(ReplayScope::Startup);
    assert_eq!(
        (progress.applied, progress.replayed),
        (2, 2),
        "{progress:?}"
    );
    let (first, second) = (b.row(&a.log, 3), b.row(&a.log, 4));
    assert_eq!(first.0, "applied");
    assert_eq!(second.0, "applied");
    let (first, second) = (first.1.unwrap(), second.1.unwrap());
    assert!(
        last_before < first && first < second,
        "late positions in ingest order: {last_before} {first} {second}"
    );
    assert_eq!(b.log_row(&a.log), (5, 5, None), "replay never moves it");
}

#[test]
fn startup_replay_cascades_to_parked_dependents() {
    let mut a = Node::new(42, rich());
    a.author::<FutureKind>(&put("f1")); // 2 creates e:f1
    a.author::<PutKind>(&put_needing("p", "e:f1")); // 3
    let mut b = Node::new(43, base());
    b.ship(&a, a.log, 3);
    b.pass();
    assert_eq!(b.state(&a.log, 2), "parked_unknown");
    assert_eq!(b.state(&a.log, 3), "parked_dep");

    b.reopen(rich());
    let progress = b.replay(ReplayScope::Startup);
    assert_eq!(progress.applied, 2, "{progress:?}");
    assert_eq!(b.state(&a.log, 3), "applied");
}

#[test]
fn unapplied_failed_reattempts() {
    let mut a = Node::new(44, base());
    a.author::<RemoteKind>(&Remote {
        id: "g".into(),
        mode: "gate".into(),
        min: 1000,
    }); // 2
    let mut c = Node::new(45, base());
    c.ship(&a, a.log, 2);
    c.pass();
    assert_eq!(c.state(&a.log, 2), "failed");

    // Other states are left alone.
    let progress = c.replay(ReplayScope::Unapplied {
        states: &[ApplyState::ParkedUnknown, ApplyState::ParkedDep],
    });
    assert_eq!(progress, ApplyProgress::default());

    // Still gated: re-attempted, still failed.
    let progress = c.replay(ReplayScope::Unapplied {
        states: &[ApplyState::Failed],
    });
    assert_eq!((progress.failed, progress.applied), (1, 0), "{progress:?}");
    assert_eq!(c.state(&a.log, 2), "failed");

    c.conn
        .execute("UPDATE local_node SET send_eligibility_revision=1000", [])
        .unwrap();
    let progress = c.replay(ReplayScope::Unapplied {
        states: &[ApplyState::Failed],
    });
    assert_eq!(
        (progress.applied, progress.replayed),
        (1, 1),
        "{progress:?}"
    );
    let (state, seq, error) = c.row(&a.log, 2);
    assert_eq!(state, "applied");
    assert!(seq.is_some());
    assert_eq!(error, None);
    assert_eq!(c.log_row(&a.log).0, 2, "replay never moves it");
}

// ---- JournalPort ------------------------------------------------------------

#[test]
fn sqlite_store_apply_pending_is_apply_pass() {
    use crate::store::{SqliteStore, StoreSettings};
    use crate::sync::journal_port::JournalPort;

    let mut a = Node::new(46, base());
    a.author::<PutKind>(&put("p1"));
    let b_path = std::env::temp_dir().join(format!("ht-apply-{}.db", uuid::Uuid::new_v4()));
    let identity = Arc::new(NodeIdentity::from_seed([47; 32]));
    let ctx = StoreContext::new(b_path.clone(), Arc::new(FixedClock))
        .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
    let store = SqliteStore::new(
        ctx,
        crate::test_support::test_node(),
        StoreSettings::default(),
    )
    .unwrap();
    {
        let mut conn = store.context().open_writer().unwrap();
        jr::open(store.context(), &mut conn, identity, "host", base()).unwrap();
    }
    let entries = jr::serve(&a.conn, &a.log, 1, BatchLimits::default()).unwrap();
    let outcome = store.ingest(&a.log, entries, &budget()).unwrap();
    assert!(matches!(outcome, IngestOutcome::Stored { stored: 2, .. }));
    let progress = store.apply_pending(&budget()).unwrap();
    assert_eq!(progress.applied, 2, "{progress:?}");
    assert!(!progress.has_more);
    let vector: Vector = store.vector(&budget()).unwrap();
    assert_eq!(vector.0.get(&a.log).map(|h| h.counter), Some(2));
    assert!(store.stored_hash(&a.log, 2, &budget()).unwrap().is_some());
    assert_eq!(
        store
            .serve(&a.log, 2, BatchLimits::default(), &budget())
            .unwrap()
            .len(),
        1
    );
    drop(store);
    for suffix in ["", "-wal", "-shm"] {
        let mut name = b_path.clone().into_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(name);
    }
}
