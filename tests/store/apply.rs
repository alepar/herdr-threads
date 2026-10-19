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
use crate::sync::envelope::{Envelope, EnvelopeError, UnsignedEnvelope};
use crate::sync::facts::{FactRegistry, WaitFor};
use crate::sync::journal_port::IngestOutcome;
use crate::sync::{BatchLimits, Vector};

/// A paused clock: the monotonic reading moves only through `set`.
struct FixedClock(std::sync::atomic::AtomicU64);
impl FixedClock {
    fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(1))
    }
    fn set(&self, ms: u64) {
        self.0.store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(std::sync::atomic::Ordering::SeqCst))
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
            "wait_host" => {
                let hosts: i64 = tx
                    .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
                    .map_err(|e| FactError::from_api(store_error(e)))?;
                if hosts == 0 {
                    Err(FactError::Wait {
                        on: WaitFor::Host,
                        error: ApiError::new(ErrorCode::HostUnavailable, "needs a host"),
                    })
                } else {
                    Ok(())
                }
            }
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

// ---- delivery kinds ---------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Msg {
    id: String,
}

/// A message-like kind: always records what it saw in `test_seen`, and writes
/// a lazy row and a wake row only when `delivery_suppressed` is false.
struct MsgKind;
impl FactKind for MsgKind {
    const NAME: &'static str = "test.msg";
    const MAJOR: u32 = 1;
    type Payload = Msg;
    fn creates(p: &Msg) -> Option<EntityKey> {
        key(format!("m:{}", p.id))
    }
    fn needs(_: &Msg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Msg) -> Result<(), FactError> {
        let fail = |e: rusqlite::Error| FactError::from_api(store_error(e));
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS test_seen(id TEXT, suppressed INTEGER);\
             CREATE TABLE IF NOT EXISTS test_lazy(id TEXT);\
             CREATE TABLE IF NOT EXISTS test_wake(id TEXT);",
        )
        .map_err(fail)?;
        let settle = key(format!("s:{}", p.id)).expect("key");
        let suppressed = cx
            .delivery_suppressed(tx, &settle)
            .map_err(FactError::from_api)?;
        tx.execute(
            "INSERT INTO test_seen(id,suppressed) VALUES (?1,?2)",
            params![p.id, suppressed],
        )
        .map_err(fail)?;
        if !suppressed {
            tx.execute("INSERT INTO test_lazy(id) VALUES (?1)", [&p.id])
                .map_err(fail)?;
            tx.execute("INSERT INTO test_wake(id) VALUES (?1)", [&p.id])
                .map_err(fail)?;
        }
        Ok(())
    }
}

/// The settlement of `Msg { id }`.
struct SettleKind;
impl FactKind for SettleKind {
    const NAME: &'static str = "test.settle";
    const MAJOR: u32 = 1;
    type Payload = Msg;
    fn creates(p: &Msg) -> Option<EntityKey> {
        key(format!("s:{}", p.id))
    }
    fn needs(_: &Msg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Msg) -> Result<(), FactError> {
        bump(tx)
    }
}

fn msg(id: &str) -> Msg {
    Msg { id: id.into() }
}

fn delivery() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| {
        b.with::<PutKind>()
            .with::<MsgKind>()
            .with::<SettleKind>()
            .with::<HistKind>()
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
    clock: Arc<FixedClock>,
}

impl Node {
    fn new(seed: u8, registry: &'static FactRegistry) -> Self {
        Self::with_incarnations(seed, u64::from(seed), registry)
    }

    /// A node with identity `seed` whose incarnation draws start from
    /// `incarnation_seed`: the same identity with another value is the same
    /// node after a wipe (a new incarnation).
    fn with_incarnations(seed: u8, incarnation_seed: u64, registry: &'static FactRegistry) -> Self {
        let path = crate::test_support::scratch::temp_path(format!(
            "ht-apply-{}.db",
            uuid::Uuid::new_v4()
        ));
        let clock = Arc::new(FixedClock::new());
        let (ctx, conn, report) =
            Self::open_at(&path, seed, incarnation_seed, registry, Arc::clone(&clock));
        Self {
            path,
            seed,
            ctx,
            conn,
            log: report.self_log,
            clock,
        }
    }

    fn open_at(
        path: &Path,
        seed: u8,
        incarnation_seed: u64,
        registry: &'static FactRegistry,
        clock: Arc<FixedClock>,
    ) -> (StoreContext, Connection, OpenReport) {
        let identity = Arc::new(NodeIdentity::from_seed([seed; 32]));
        let ctx = StoreContext::new(path.to_path_buf(), clock)
            .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
        ctx.seed_incarnations(incarnation_seed);
        let mut conn = ctx.open_writer().unwrap();
        let report = jr::open(&ctx, &mut conn, identity, "host", registry).unwrap();
        (ctx, conn, report)
    }

    /// Restarts the node with another registry (an upgrade).
    fn reopen(&mut self, registry: &'static FactRegistry) -> OpenReport {
        let (ctx, conn, report) = Self::open_at(
            &self.path,
            self.seed,
            u64::from(self.seed),
            registry,
            Arc::clone(&self.clock),
        );
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
        self.ship_range(from, log, 1, upto);
    }

    /// Ingests `from`'s entries of `log` with counters `first..=upto`.
    fn ship_range(&mut self, from: &Node, log: LogKey, first: u64, upto: u64) {
        let entries: Vec<_> = jr::serve(&from.conn, &log, first, BatchLimits::default())
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

/// A node holding a remote `wait_host` entry and an entry that needs it.
fn host_wait_fixture(seeds: (u8, u8)) -> (Node, Node) {
    let mut a = Node::new(seeds.0, base());
    a.author::<RemoteKind>(&remote("w", "wait_host")); // A@2 creates e:w
    a.author::<PutKind>(&put_needing("y", "e:w")); // A@3
    let mut c = Node::new(seeds.1, base());
    c.ship(&a, a.log, 3);
    (a, c)
}

fn attach_fake_host(c: &Node) {
    c.conn
        .execute(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at,last_attached_at) \
             SELECT id,'/h.sock',0,0 FROM local_node",
            [],
        )
        .unwrap();
}

fn parked_on(c: &Node, log: &LogKey, counter: u64) -> Option<String> {
    c.conn
        .query_row(
            "SELECT j.parked_on FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=?3",
            params![log.node.to_string(), log.incarnation, counter as i64],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn host_wait_parks_then_applies_once_a_host_row_exists() {
    let (a, mut c) = host_wait_fixture((60, 61));
    let progress = c.pass();
    assert_eq!(progress.parked, 2, "{progress:?}");
    let (state, _, error) = c.row(&a.log, 2);
    assert_eq!(state, "parked_dep");
    assert!(error.unwrap().contains("needs a host"));
    assert_eq!(parked_on(&c, &a.log, 2).as_deref(), Some("wait:host"));
    assert_eq!(c.state(&a.log, 3), "parked_dep");
    assert_eq!(parked_on(&c, &a.log, 3).as_deref(), Some("e:w"));

    // No host: nothing changes and the pass does not loop.
    let progress = c.pass();
    assert!(!progress.has_more, "{progress:?}");
    assert_eq!(progress.applied, 0, "{progress:?}");
    assert_eq!(c.state(&a.log, 2), "parked_dep");

    attach_fake_host(&c);
    let progress = c.pass();
    assert_eq!(progress.applied, 2, "{progress:?}");
    assert_eq!(c.state(&a.log, 2), "applied");
    assert_eq!(c.state(&a.log, 3), "applied");
}

#[test]
fn startup_replay_takes_host_waiting_entries_when_a_host_exists() {
    let (a, mut c) = host_wait_fixture((62, 63));
    c.pass();
    assert_eq!(c.state(&a.log, 2), "parked_dep");

    // No host row: startup leaves the waiting entry alone.
    let progress = c.replay(ReplayScope::Startup);
    assert_eq!(progress.applied, 0, "{progress:?}");
    assert_eq!(c.state(&a.log, 2), "parked_dep");

    attach_fake_host(&c);
    let progress = c.replay(ReplayScope::Startup);
    assert_eq!(progress.applied, 2, "{progress:?}");
    assert_eq!(c.state(&a.log, 2), "applied");
    assert_eq!(c.state(&a.log, 3), "applied");
}

#[test]
fn wait_parked_on_matches_the_ready_predicate() {
    assert_eq!(WaitFor::Host.parked_on(), "wait:host");
    assert!(PARKED_READY.contains(WaitFor::Host.parked_on()));
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

// ---- follow-up tag and depth ------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Tag {
    id: String,
}

/// A follow-up whose handler raises the tag counter.
struct TagChildKind;
impl FactKind for TagChildKind {
    const NAME: &'static str = "test.tagchild";
    const MAJOR: u32 = 1;
    type Payload = Tag;
    fn creates(p: &Tag) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(_: &Tag) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, cx: &mut ApplyContext<'_>, _: Tag) -> Result<(), FactError> {
        cx.set_self_tag_k(7);
        Ok(())
    }
}

/// Always queues the tag-raising child, on any origin.
struct TagParentKind;
impl FactKind for TagParentKind {
    const NAME: &'static str = "test.tagparent";
    const MAJOR: u32 = 1;
    type Payload = Tag;
    fn creates(p: &Tag) -> Option<EntityKey> {
        key(format!("e:{}", p.id))
    }
    fn needs(_: &Tag) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Tag) -> Result<(), FactError> {
        cx.follow_up::<TagChildKind>(&Tag {
            id: format!("c-{}", p.id),
        })
        .map_err(|e| FactError::Rejected(ApiError::new(ErrorCode::InvalidRequest, e.to_string())))
    }
}

#[derive(Serialize, Deserialize)]
struct Link {
    id: String,
    depth: u32,
}

/// Queues itself (a fresh `creates` each time) once it is a follow-up or
/// arrived from another node: an unbounded chain. A depth-0 entry authored
/// locally queues nothing.
struct LinkKind;
impl FactKind for LinkKind {
    const NAME: &'static str = "test.link";
    const MAJOR: u32 = 1;
    type Payload = Link;
    fn creates(p: &Link) -> Option<EntityKey> {
        key(format!("e:{}-{}", p.id, p.depth))
    }
    fn needs(_: &Link) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Link) -> Result<(), FactError> {
        bump(tx)?;
        if !cx.is_local_origin() || p.depth > 0 {
            cx.follow_up::<LinkKind>(&Link {
                id: p.id,
                depth: p.depth + 1,
            })
            .map_err(|e| {
                FactError::Rejected(ApiError::new(ErrorCode::InvalidRequest, e.to_string()))
            })?;
        }
        Ok(())
    }
}

fn chain() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| {
        b.with::<TagParentKind>()
            .with::<TagChildKind>()
            .with::<LinkKind>()
    })
}

#[test]
fn follow_up_self_tag_reaches_apply_progress() {
    let mut a = Node::new(80, chain());
    a.author::<TagParentKind>(&Tag { id: "p".into() }); // A@2
    let mut b = Node::new(81, chain());
    b.ship(&a, a.log, 2);
    let progress = b.pass();
    assert_eq!(progress.self_tag_k, Some(7), "{progress:?}");

    // Authored by a command, the follow-up's raise reaches the caller.
    let mut c = Node::new(82, chain());
    let authored = c.author::<TagParentKind>(&Tag { id: "q".into() });
    assert_eq!(authored.self_tag_k, Some(7));
}

#[test]
fn unbounded_follow_up_chain_fails_its_trigger() {
    let mut a = Node::new(83, chain());
    a.author::<LinkKind>(&Link {
        id: "l".into(),
        depth: 0,
    }); // A@2, queues nothing locally
    let mut b = Node::new(84, chain());
    let revision = b.revision();
    b.ship(&a, a.log, 2);
    let progress = b.pass();
    assert_eq!((progress.applied, progress.failed), (1, 1), "{progress:?}");
    let (state, seq, error) = b.row(&a.log, 2);
    assert_eq!((state.as_str(), seq), ("failed", None));
    let error = error.unwrap();
    assert!(error.contains("follow-up chain deeper than 4"), "{error}");
    assert_eq!(b.log_row(&b.log), (1, 1, None), "no follow-up row survives");
    assert_eq!(b.revision(), revision, "the whole chain rolled back");

    // Authored by a command, the same chain fails the command.
    let mut c = Node::new(85, chain());
    let before = c.log_row(&c.log);
    let ctx = &c.ctx;
    let error = ctx
        .execute_decision(
            &mut c.conn,
            |_| Ok(()),
            |tx, d, ()| {
                jr::author::<LinkKind>(
                    tx,
                    ctx,
                    d,
                    &Link {
                        id: "m".into(),
                        depth: 1,
                    },
                )
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(
        error.detail.contains("follow-up chain deeper than 4"),
        "{}",
        error.detail
    );
    assert_eq!(c.log_row(&c.log), before, "nothing committed");
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
    let b_path =
        crate::test_support::scratch::temp_path(format!("ht-apply-{}.db", uuid::Uuid::new_v4()));
    let identity = Arc::new(NodeIdentity::from_seed([47; 32]));
    let ctx = StoreContext::new(b_path.clone(), Arc::new(FixedClock::new()))
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
    let entries = entries
        .into_iter()
        .map(crate::sync::envelope::StoredEntry::from)
        .collect();
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

// ---- ApplyContext services --------------------------------------------------

impl Node {
    fn sync_hold(&self) -> Arc<crate::store::connection::OwnHistoryHold> {
        Arc::clone(self.ctx.sync().unwrap().own_history_hold())
    }

    fn hold(&self, source: HoldSource, on: bool, ms: u64) {
        self.clock.set(ms);
        self.sync_hold().set(source, on, MonoInstant(ms));
    }

    /// Runs `f` with an ApplyContext whose origin is `origin`.
    fn with_cx<T>(
        &mut self,
        origin: LogKey,
        f: impl FnOnce(&Transaction<'_>, &ApplyContext<'_>) -> T,
    ) -> T {
        let ctx = &self.ctx;
        let local = ctx.sync().unwrap().identity.uuid();
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| {
                let cx = ApplyContext::new(origin, 1, 1, HlcStamp::from_u64(0), d, 1, local);
                Ok(f(tx, &cx))
            },
        )
        .unwrap()
    }

    fn suppressed(&mut self, origin: LogKey, settle: &str) -> bool {
        let k = key(settle.into()).unwrap();
        self.with_cx(origin, |tx, cx| cx.delivery_suppressed(tx, &k).unwrap())
    }

    fn seen(&self, id: &str) -> Option<bool> {
        self.conn
            .query_row("SELECT suppressed FROM test_seen WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .ok()
    }

    fn delivered(&self, id: &str) -> (i64, i64) {
        let n = |table: &str| {
            self.conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE id=?1"),
                    [id],
                    |r| r.get(0),
                )
                .unwrap_or(0)
        };
        (n("test_lazy"), n("test_wake"))
    }
}

use crate::store::connection::{HoldSource, OWN_HISTORY_HOLD_MAX};

#[test]
fn provenance_replicated_and_local_incl_own_old_incarnation() {
    let mut a = Node::new(60, delivery());
    let b = Node::new(61, delivery());
    let a_old = Node::with_incarnations(60, 999, delivery());
    assert_ne!(a_old.log, a.log, "another incarnation of the same node");

    let replicated = a.with_cx(b.log, |_, cx| cx.provenance());
    assert_eq!(replicated, Provenance::ReplicatedFrom(b.log.node));
    assert_eq!(
        replicated.as_str(),
        format!("replicated_from:{}", b.log.node)
    );
    assert_eq!(replicated.as_str().len(), "replicated_from:".len() + 32);
    let own = a.log;
    assert_eq!(a.with_cx(own, |_, cx| cx.provenance()), Provenance::Local);
    assert_eq!(
        a.with_cx(a_old.log, |_, cx| cx.provenance()),
        Provenance::Local,
        "an own old incarnation is Local"
    );
    assert_eq!(Provenance::Local.as_str(), "local_journal");
}

#[test]
fn is_local_seat_by_home() {
    let mut a = Node::new(62, delivery());
    let own = a.log;
    a.conn
        .execute_batch(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at) \
             VALUES ((SELECT id FROM local_node),'local',0);\
             INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
             VALUES('s-local',(SELECT id FROM local_node),'unresolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
             INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
             VALUES('s-remote',(SELECT id FROM local_node),'remote','native',1,0,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,'agent',1);",
        )
        .unwrap();
    let check = |a: &mut Node, id: &str| {
        let seat = crate::protocol::ids::SeatId::new(id);
        a.with_cx(own, |tx, cx| cx.is_local_seat(tx, &seat).unwrap())
    };
    assert!(check(&mut a, "s-local"));
    assert!(!check(&mut a, "s-remote"));
    assert!(!check(&mut a, "s-unknown"));
}

#[test]
fn delivery_suppressed_journal_and_forked_any_state() {
    let mut x1 = Node::new(63, delivery());
    x1.author::<PutKind>(&put("k"));
    let mut a = Node::new(64, delivery());
    a.author::<PutKind>(&put("foreign"));
    let mut x2 = Node::with_incarnations(63, 500, delivery());
    x2.author::<PutKind>(&put("live"));
    let own = x2.log;

    // x1's entry is in an own old incarnation's log, still pending.
    x2.ship(&x1, x1.log, 2);
    a.ship(&x1, x1.log, 2);
    assert_eq!(x2.state(&x1.log, 2), "pending");
    assert!(x2.suppressed(own, "e:k"), "own old log, pending");
    assert!(x2.suppressed(own, "e:live"), "active self log, applied");
    assert!(!x2.suppressed(own, "e:none"), "no row");

    // A foreign log's row never suppresses.
    x2.ship(&a, a.log, 2);
    assert!(!x2.suppressed(own, "e:foreign"));

    // A forked-branch row in a log of this node does.
    let log_ix: i64 = x2
        .conn
        .query_row("SELECT active_log_ix FROM local_node", [], |r| r.get(0))
        .unwrap();
    x2.conn
        .execute(
            "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,payload,signature,received_at,raw) \
             VALUES (?1,9,zeroblob(32),zeroblob(32),1,'test.put',1,x'',?2,x'',zeroblob(64),0,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
            params![log_ix, "e:forked"],
        )
        .unwrap();
    assert!(x2.suppressed(own, "e:forked"), "journal_forked row");
    // The same key in a foreign log's fork would not count.
    let a_ix: i64 = x2
        .conn
        .query_row(
            "SELECT log_ix FROM sync_logs WHERE node_uuid=?1",
            [a.log.node.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    x2.conn
        .execute(
            "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,payload,signature,received_at,raw) \
             VALUES (?1,9,zeroblob(32),zeroblob(32),1,'test.put',1,x'',?2,x'',zeroblob(64),0,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
            params![a_ix, "e:foreign-fork"],
        )
        .unwrap();
    assert!(!x2.suppressed(own, "e:foreign-fork"));
}

#[test]
fn delivery_suppressed_false_for_unsettled_own_old_origin() {
    let mut x1 = Node::new(65, delivery());
    x1.author::<MsgKind>(&msg("u"));
    let mut x2 = Node::with_incarnations(65, 501, delivery());
    x2.ship(&x1, x1.log, 2);
    // The message itself (creates m:u) sits in the own old log; its settle
    // key s:u has no row anywhere.
    assert!(!x2.suppressed(x1.log, "s:u"));
    assert!(x2.suppressed(x1.log, "m:u"), "the key it creates does");
}

// ---- own-history hold -------------------------------------------------------

#[test]
fn hold_keeps_foreign_and_own_old_heads_pending_active_self_applies() {
    let mut a = Node::new(66, delivery());
    a.author::<PutKind>(&put("a1"));
    let mut x1 = Node::new(67, delivery());
    x1.author::<PutKind>(&put("x1"));
    let mut x2 = Node::with_incarnations(67, 502, delivery());
    x2.ship(&a, a.log, 2);
    x2.ship(&x1, x1.log, 2);
    x2.hold(HoldSource::Links, true, 1_000);

    let before = x2.conn.total_changes();
    let progress = x2.pass();
    assert_eq!(progress.applied, 0, "{progress:?}");
    assert_eq!(x2.state(&a.log, 1), "pending");
    assert_eq!(x2.state(&x1.log, 1), "pending");
    assert_eq!(x2.log_row(&a.log).2, None, "no blocked_on write");
    assert_eq!(x2.log_row(&x1.log).2, None);
    assert_eq!(x2.conn.total_changes(), before, "nothing written");

    // The active self log is unaffected: authoring applies inline.
    let authored = x2.author::<PutKind>(&put("mine"));
    assert_eq!(x2.state(&x2.log, authored.counter), "applied");
    assert!(x2.sync_hold().active(MonoInstant(1_000)));
}

#[test]
fn hold_start_not_reset_while_flag_set_rearms_after_clear() {
    let x = Node::new(68, delivery());
    let hold = x.sync_hold();
    let at = |ms| MonoInstant(ms);
    hold.set(HoldSource::Links, true, at(1_000));
    hold.set(HoldSource::Branches, true, at(200_000)); // does not restart
    hold.set(HoldSource::Links, false, at(250_000)); // Branches still set
    assert!(hold.active(at(1_000 + OWN_HISTORY_HOLD_MAX - 1)));
    assert!(
        !hold.active(at(1_000 + OWN_HISTORY_HOLD_MAX)),
        "the start stayed at 1000"
    );
    hold.set(HoldSource::Branches, false, at(310_000));
    assert!(!hold.active(at(310_000)), "both clear");

    // A new hold re-arms from its own first set.
    hold.set(HoldSource::Links, true, at(400_000));
    assert!(hold.active(at(400_000 + OWN_HISTORY_HOLD_MAX - 1)));
    assert!(!hold.active(at(400_000 + OWN_HISTORY_HOLD_MAX)));
}

#[test]
fn clearing_flag_applies_next_pass() {
    let mut a = Node::new(69, delivery());
    a.author::<PutKind>(&put("a1"));
    let mut x2 = Node::with_incarnations(70, 503, delivery());
    x2.ship(&a, a.log, 2);
    x2.hold(HoldSource::Branches, true, 10);
    assert_eq!(x2.pass().applied, 0);
    let _ = x2.ctx.sync().unwrap().take_apply_hint();
    let kicks = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counted = Arc::clone(&kicks);
    x2.sync_hold().set_release_kick(Arc::new(move || {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }));

    x2.hold(HoldSource::Branches, false, 20);
    assert!(
        x2.ctx.sync().unwrap().take_apply_hint(),
        "the last flag cleared sets the hint"
    );
    assert_eq!(kicks.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(x2.pass().applied, 2);
    assert_eq!(x2.state(&a.log, 2), "applied");
}

#[test]
fn hold_expires_after_5_min_with_one_log_line() {
    let mut a = Node::new(71, delivery());
    a.author::<PutKind>(&put("a1"));
    let mut x2 = Node::with_incarnations(72, 504, delivery());
    x2.ship(&a, a.log, 2);
    x2.hold(HoldSource::Links, true, 1_000);
    let _ = x2.ctx.sync().unwrap().take_apply_hint();

    x2.clock.set(1_000 + OWN_HISTORY_HOLD_MAX - 1);
    assert_eq!(x2.pass().applied, 0, "one ms short of the cap");
    assert_eq!(x2.sync_hold().expiry_logs(), 0);

    x2.clock.set(1_000 + OWN_HISTORY_HOLD_MAX);
    assert_eq!(x2.pass().applied, 2, "the cap lifts the filter");
    assert_eq!(x2.sync_hold().expiry_logs(), 1);
    assert!(x2.ctx.sync().unwrap().take_apply_hint(), "expiry sets it");
    x2.pass();
    x2.pass();
    assert_eq!(x2.sync_hold().expiry_logs(), 1, "logged once");
}

#[test]
fn no_flag_never_filters() {
    let mut a = Node::new(73, delivery());
    a.author::<PutKind>(&put("a1"));
    let mut x2 = Node::with_incarnations(74, 505, delivery());
    x2.ship(&a, a.log, 2);
    x2.clock.set(10 * OWN_HISTORY_HOLD_MAX);
    assert!(
        !x2.sync_hold()
            .active(MonoInstant(10 * OWN_HISTORY_HOLD_MAX))
    );
    assert_eq!(x2.pass().applied, 2);
    assert_eq!(x2.sync_hold().expiry_logs(), 0);
}

// ---- catch-up suppression scenarios ------------------------------------------

#[test]
fn wipe_then_pull_settled_message_not_redelivered() {
    let mut a = Node::new(75, delivery());
    a.author::<MsgKind>(&msg("m1"));
    let mut b1 = Node::new(76, delivery());
    b1.ship(&a, a.log, 2);
    b1.pass();
    assert_eq!(b1.delivered("m1"), (1, 1), "delivered once before the wipe");
    b1.author::<SettleKind>(&msg("m1")); // settle key authored by B

    // B is wiped: same seed, new incarnation. A's log comes first.
    let mut b2 = Node::with_incarnations(76, 506, delivery());
    assert_eq!(b2.log.node, b1.log.node);
    assert_ne!(b2.log, b1.log);
    b2.hold(HoldSource::Links, true, 100);
    b2.ship(&a, a.log, 2);
    b2.ship(&b1, b1.log, 3);
    assert_eq!(b2.pass().applied, 0);
    assert_eq!(b2.state(&a.log, 2), "pending", "waits for the hold");

    b2.hold(HoldSource::Links, false, 200);
    b2.pass();
    assert_eq!(b2.state(&a.log, 2), "applied");
    assert_eq!(b2.seen("m1"), Some(true), "delivery_suppressed was true");
    assert_eq!(b2.delivered("m1"), (0, 0), "no lazy row, no wake_work");
}

#[test]
fn own_old_unsettled_message_delivered_after_hold() {
    let mut b1 = Node::new(77, delivery());
    b1.author::<MsgKind>(&msg("open"));
    let mut b2 = Node::with_incarnations(77, 507, delivery());
    b2.hold(HoldSource::Branches, true, 100);
    b2.ship(&b1, b1.log, 2);
    assert_eq!(b2.pass().applied, 0);
    assert_eq!(b2.seen("open"), None, "held");

    b2.hold(HoldSource::Branches, false, 200);
    b2.pass();
    assert_eq!(b2.state(&b1.log, 2), "applied");
    assert_eq!(b2.seen("open"), Some(false));
    assert_eq!(b2.delivered("open"), (1, 1), "lazy row and wake_work");
}

#[test]
fn own_old_settled_message_not_delivered_even_with_later_settlement_batch() {
    let mut b1 = Node::new(78, delivery());
    b1.author::<MsgKind>(&msg("open")); // 2
    b1.author::<MsgKind>(&msg("done")); // 3
    b1.author::<SettleKind>(&msg("done")); // 4
    let mut b2 = Node::with_incarnations(78, 508, delivery());
    b2.hold(HoldSource::Branches, true, 100);
    b2.ship(&b1, b1.log, 3); // both messages, no settlement yet
    b2.ship_range(&b1, b1.log, 4, 4); // the settlement in a later batch
    assert_eq!(b2.pass().applied, 0);

    b2.hold(HoldSource::Branches, false, 200);
    b2.pass();
    assert_eq!(b2.state(&b1.log, 4), "applied");
    assert_eq!(b2.seen("done"), Some(true));
    assert_eq!(b2.delivered("done"), (0, 0));
    assert_eq!(b2.seen("open"), Some(false));
    assert_eq!(b2.delivered("open"), (1, 1), "the unsettled one is offered");
}

// ---- re-issue and own history -------------------------------------------------

/// Records what the apply context said about the entry's history and what
/// `attribution::take` fell back to, into `test_hist`.
struct HistKind;
impl FactKind for HistKind {
    const NAME: &'static str = "test.hist";
    const MAJOR: u32 = 1;
    type Payload = Msg;
    fn creates(_: &Msg) -> Option<EntityKey> {
        None
    }
    fn needs(_: &Msg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Msg) -> Result<(), FactError> {
        let fail = |e: rusqlite::Error| FactError::from_api(store_error(e));
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS test_hist(id TEXT, own INTEGER, provenance TEXT, observation TEXT);",
        )
        .map_err(fail)?;
        let taken = crate::sync::facts::with_input(cx, |m| {
            crate::store::attribution::take(tx, m, "send:hist")
        })?;
        tx.execute(
            "INSERT INTO test_hist(id,own,provenance,observation) VALUES (?1,?2,?3,?4)",
            params![
                p.id,
                cx.is_own_history(),
                cx.provenance().as_str(),
                taken.observation
            ],
        )
        .map_err(fail)?;
        Ok(())
    }
}

impl Node {
    fn reissue(&mut self, source: &crate::sync::envelope::Envelope) -> Authored {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| jr::reissue(tx, ctx, d, source),
        )
        .unwrap()
    }

    fn entry(&self, log: &LogKey, counter: u64) -> crate::sync::envelope::Envelope {
        jr::serve(&self.conn, log, counter, BatchLimits::default())
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    /// A `journal_forked` row creating `creates` in `log` (a log of this node).
    fn forked_settlement(&self, log: &LogKey, creates: &str) {
        self.conn
            .execute(
                "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,payload,signature,received_at,raw) \
                 SELECT log_ix,9,zeroblob(32),zeroblob(32),1,'test.settle',1,x'',?3,x'',zeroblob(64),0,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000' \
                 FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
                params![log.node.to_string(), log.incarnation, creates],
            )
            .unwrap();
    }

    /// `(own_history, provenance, observation provenance)` the `HistKind` saw.
    fn hist(&self, id: &str) -> (bool, String, String) {
        let (own, prov, obs): (bool, String, String) = self
            .conn
            .query_row(
                "SELECT own,provenance,observation FROM test_hist WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let obs: serde_json::Value = serde_json::from_str(&obs).unwrap();
        (own, prov, obs["provenance"].as_str().unwrap().to_owned())
    }
}

// Kills: a reissue-mode suppression that ignores `journal_forked` (the
// settlement of a lost branch usually sits there).
#[test]
fn reissue_settlement_only_in_journal_forked_suppresses() {
    let mut x1 = Node::new(80, delivery());
    x1.author::<MsgKind>(&msg("r1"));
    let mut x2 = Node::with_incarnations(80, 510, delivery());
    let own = x2.log;
    x2.forked_settlement(&own, "s:r1");
    let source = x1.entry(&x1.log, 2);
    let authored = x2.reissue(&source);
    assert_eq!(x2.state(&x2.log, authored.counter), "applied");
    assert_eq!(x2.seen("r1"), Some(true), "settled in journal_forked only");
    assert_eq!(x2.delivered("r1"), (0, 0));
}

// Kills: a reissue that suppresses every re-issued fact (an open obligation
// from a lost branch would never be offered).
#[test]
fn reissue_unsettled_message_like_not_suppressed() {
    let mut x1 = Node::new(81, delivery());
    x1.author::<MsgKind>(&msg("r2"));
    let mut x2 = Node::with_incarnations(81, 511, delivery());
    let source = x1.entry(&x1.log, 2);
    x2.reissue(&source);
    assert_eq!(x2.seen("r2"), Some(false));
    assert_eq!(x2.delivered("r2"), (1, 1), "offered normally");
}

// Kills: an is_own_history that is true for a foreign or live-self entry, or
// false for a re-issue; and a provenance that stops being Local under Reissue.
#[test]
fn is_own_history_true_for_reissue_and_own_old() {
    let mut a = Node::new(82, delivery());
    a.author::<HistKind>(&msg("foreign"));
    let mut x1 = Node::new(83, delivery());
    x1.author::<HistKind>(&msg("re"));
    let mut x2 = Node::with_incarnations(83, 512, delivery());
    x2.author::<HistKind>(&msg("live"));
    x2.ship(&a, a.log, 2);
    x2.pass();
    let source = x1.entry(&x1.log, 2);
    x2.reissue(&source);

    let foreign_prov = format!("replicated_from:{}", a.log.node);
    assert_eq!(
        x2.hist("foreign"),
        (false, foreign_prov.clone(), foreign_prov)
    );
    let local = "local_journal".to_owned();
    let live = x2.hist("live");
    assert!(!live.0, "an entry of the live self log is not history");
    assert_eq!(live.1, local);
    assert_eq!(x2.hist("re"), (true, local.clone(), local), "Reissue mode");
}

#[test]
fn own_old_incarnation_is_own_history_in_normal_mode() {
    let mut x1 = Node::new(84, delivery());
    x1.author::<HistKind>(&msg("old"));
    let mut x2 = Node::with_incarnations(84, 513, delivery());
    x2.ship(&x1, x1.log, 2);
    x2.pass();
    assert_eq!(x2.state(&x1.log, 2), "applied");
    let local = "local_journal".to_owned();
    assert_eq!(x2.hist("old"), (true, local.clone(), local));
}

// Kills: a take that records `replicated_from` or `local` for a re-issued or
// own-old entry with nothing staged, or that ignores a staged row in Reissue.
#[test]
fn take_returns_local_journal_for_own_history() {
    let mut x1 = Node::new(85, delivery());
    x1.author::<HistKind>(&msg("old"));
    let mut x2 = Node::with_incarnations(85, 514, delivery());
    let source = x1.entry(&x1.log, 2);
    x2.reissue(&source);
    let (own, prov, obs) = x2.hist("old");
    assert!(own);
    assert_eq!(
        (prov.as_str(), obs.as_str()),
        ("local_journal", "local_journal")
    );
}

// Kills: a suppression that reads only `journal` (a pulled branch's
// settlement lives in `journal_forked`) or only reissue mode.
#[test]
fn fork_then_pull_forked_settlement_suppresses_foreign_normal_mode() {
    let mut p = Node::new(86, delivery());
    p.author::<MsgKind>(&msg("f1"));
    p.author::<MsgKind>(&msg("f2"));
    let mut x1 = Node::new(87, delivery());
    let mut x2 = Node::with_incarnations(87, 515, delivery());
    x2.ship(&x1, x1.log, 1);
    x2.freeze(&x1.log, 1);
    x2.forked_settlement(&x1.log, "s:f1");
    x2.ship(&p, p.log, 3);
    x2.pass();
    assert_eq!(x2.state(&p.log, 2), "applied");
    assert_eq!(x2.state(&p.log, 3), "applied");
    assert_eq!(x2.seen("f1"), Some(true), "settled by the lost branch");
    assert_eq!(x2.delivered("f1"), (0, 0), "a peer's message wakes nobody");
    assert_eq!(x2.seen("f2"), Some(false), "key absent: not suppressed");
    assert_eq!(x2.delivered("f2"), (1, 1));
    let _ = &mut x1;
}

// ---- payload and key agreement ----------------------------------------------

/// A sealed `test.put` entry at `counter` of `a`'s log, signed by `a`'s own
/// identity, whose envelope keys are given by hand.
fn forged_put(
    a: &Node,
    counter: u64,
    payload: &str,
    creates: Option<&str>,
    needs: &[&str],
) -> Envelope {
    let held = jr::serve(&a.conn, &a.log, counter - 1, BatchLimits::default())
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    UnsignedEnvelope {
        log: a.log,
        counter,
        prev_hash: held.hash,
        hlc: HlcStamp::from_u64(held.hlc.as_u64() + 1),
        kind: PutKind::NAME.into(),
        kind_version: PutKind::MAJOR,
        deps: Deps::empty(),
        creates: creates.map(|k| EntityKey::new(k).unwrap()),
        needs: needs.iter().map(|k| EntityKey::new(*k).unwrap()).collect(),
        payload: payload.as_bytes().to_vec(),
    }
    .seal(&NodeIdentity::from_seed([a.seed; 32]))
}

#[test]
fn entry_keys_must_match_the_payload() {
    let a = Node::new(60, base());
    // The payload creates e:x; the signed envelope claims e:forged.
    let lying = forged_put(&a, 2, r#"{"id":"x"}"#, Some("e:forged"), &[]);
    // A dependent that needs the forged key, plus an honest superset of needs.
    let dependent = forged_put_after(&a, &lying, 3, r#"{"id":"y"}"#, Some("e:y"), &["e:forged"]);
    let mut c = Node::new(61, base());
    c.ship(&a, a.log, 1);
    let outcome = jr::ingest(
        &c.ctx,
        &mut c.conn,
        &a.log,
        vec![lying, dependent],
        &budget(),
    )
    .unwrap();
    assert!(
        matches!(outcome, IngestOutcome::Stored { stored: 2, .. }),
        "{outcome:?}"
    );
    let revision = c.revision();
    c.pass();
    let (state, seq, error) = c.row(&a.log, 2);
    assert_eq!((state.as_str(), seq), ("failed", None));
    assert!(
        error.unwrap().contains("fact keys differ from its payload"),
        "the keys error is recorded"
    );
    assert_eq!(c.revision(), revision, "the handler never ran");
    assert_eq!(
        c.state(&a.log, 3),
        "parked_dep",
        "a dependent of the forged key stays parked"
    );
}

fn forged_put_after(
    a: &Node,
    prev: &Envelope,
    counter: u64,
    payload: &str,
    creates: Option<&str>,
    needs: &[&str],
) -> Envelope {
    UnsignedEnvelope {
        log: a.log,
        counter,
        prev_hash: prev.hash,
        hlc: HlcStamp::from_u64(prev.hlc.as_u64() + 1),
        kind: PutKind::NAME.into(),
        kind_version: PutKind::MAJOR,
        deps: Deps::empty(),
        creates: creates.map(|k| EntityKey::new(k).unwrap()),
        needs: needs.iter().map(|k| EntityKey::new(*k).unwrap()).collect(),
        payload: payload.as_bytes().to_vec(),
    }
    .seal(&NodeIdentity::from_seed([a.seed; 32]))
}

#[test]
fn entry_keys_superset_of_needs_applies() {
    let a = Node::new(62, base());
    let honest = forged_put(&a, 2, r#"{"id":"x"}"#, Some("e:x"), &[]);
    let mut c = Node::new(63, base());
    c.ship(&a, a.log, 1);
    jr::ingest(&c.ctx, &mut c.conn, &a.log, vec![honest], &budget()).unwrap();
    c.pass();
    assert_eq!(c.state(&a.log, 2), "applied");
}

#[test]
fn duplicate_key_entry_is_refused_at_ingest() {
    let a = Node::new(64, base());
    let dup = forged_put(&a, 2, r#"{"id":"x","id":"y"}"#, Some("e:x"), &[]);
    let mut c = Node::new(65, base());
    c.ship(&a, a.log, 1);
    let outcome = jr::ingest(&c.ctx, &mut c.conn, &a.log, vec![dup], &budget()).unwrap();
    assert!(
        matches!(
            outcome,
            IngestOutcome::Corrupt {
                counter: 2,
                reason: EnvelopeError::Malformed { field: "payload" },
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(
        c.count("SELECT COUNT(*) FROM journal WHERE counter=2"),
        0,
        "nothing stored"
    );
}

// ---- opaque chain positions (ht-j87z.11) -------------------------------------

/// A log that stops at an opaque position is passed over: nothing is applied
/// for it, `blocked_on` records `opaque_layout` (ht-j87z.10), and another
/// log's ready head still applies in the same pass.
#[test]
fn opaque_next_position_is_skipped_with_opaque_layout_recorded() {
    let mut a = Node::new(11, base());
    a.author::<PutKind>(&put("a1"));
    let mut b = Node::new(12, base());
    b.author::<PutKind>(&put("b1"));
    let mut c = Node::new(13, base());
    c.ship(&a, a.log, 1);
    c.pass();
    assert_eq!(c.log_row(&a.log), (1, 1, None));

    // P (A's log): opaque at 2, a pending known entry at 3, head at 3.
    let node = a.log.node.to_string();
    let hash = |n: u8| [n; 32].to_vec();
    c.conn
        .execute(
            "INSERT INTO journal(log_ix,counter,hash,prev_hash,signature,raw,needs,ingested_at,apply_state) \
             SELECT log_ix,2,?2,?3,zeroblob(64),zeroblob(68),NULL,0,'parked_layout' FROM sync_logs WHERE node_uuid=?1",
            params![node, hash(2), hash(1)],
        )
        .unwrap();
    c.conn
        .execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,ingested_at,apply_state,raw) \
             SELECT log_ix,3,?2,?3,5,'test.put',1,x'0000',x'00',zeroblob(64),0,'pending',?4 \
             FROM sync_logs WHERE node_uuid=?1",
            params![
                node,
                hash(2),
                hash(3),
                crate::test_support::opaque_entry::known_raw_stub()
            ],
        )
        .unwrap();
    c.conn
        .execute(
            "UPDATE sync_logs SET contiguous_counter=3,head_hash=?2 WHERE node_uuid=?1",
            params![node, hash(3)],
        )
        .unwrap();

    // Q (B's log): a ready head.
    c.ship(&b, b.log, 1);
    let progress = c.pass();
    assert_eq!(progress.applied, 1, "only Q's genesis: {progress:?}");
    assert_eq!(c.state(&b.log, 1), "applied");
    assert_eq!(
        c.log_row(&a.log),
        (1, 3, Some("opaque_layout".into())),
        "P stops at the opaque entry"
    );
    assert_eq!(c.state(&a.log, 3), "pending");

    // No news: a second pass changes nothing.
    let before = c.conn.total_changes();
    c.pass();
    assert_eq!(c.conn.total_changes(), before);
}

/// Apply classifies a chain head by projection presence: an empty-projection
/// head (`parked_layout`) stops its log with `opaque_layout` and is not
/// selected, and the same row after its projection is filled (the promotion
/// shape: direct UPDATE from all-NULL, `apply_state` to `pending`) is ready and
/// applies. The row's raw is a real HTJRNL01 entry in both states, so a
/// classification by raw magic would not see the difference.
#[test]
fn promoted_head_is_ready_and_empty_head_is_marked() {
    let mut a = Node::new(31, base());
    a.author::<PutKind>(&put("a1"));
    a.author::<PutKind>(&put("a2"));
    let mut donor = Node::new(32, base());
    donor.ship(&a, a.log, 2);
    type Cols = (
        i64,
        String,
        i64,
        Vec<u8>,
        Option<String>,
        String,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    );
    let (hlc, kind, version, deps, creates, needs, payload, raw, signature, hash, prev): Cols =
        donor
            .conn
            .query_row(
                "SELECT j.hlc,j.kind,j.kind_version,j.deps,j.creates,j.needs,j.payload,j.raw, \
                        j.signature,j.hash,j.prev_hash FROM journal j \
                 JOIN sync_logs l ON l.log_ix=j.log_ix WHERE l.node_uuid=?1 AND j.counter=2",
                params![a.log.node.to_string()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                    ))
                },
            )
            .unwrap();

    let mut c = Node::new(33, base());
    c.ship(&a, a.log, 1);
    c.pass();
    assert_eq!(c.log_row(&a.log), (1, 1, None));
    c.conn
        .execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,signature,raw,needs,ingested_at,apply_state) \
             SELECT log_ix,2,?2,?3,?4,?5,NULL,0,'parked_layout' FROM sync_logs WHERE node_uuid=?1",
            params![a.log.node.to_string(), prev, hash, signature, raw],
        )
        .unwrap();
    c.conn
        .execute(
            "UPDATE sync_logs SET contiguous_counter=2,head_hash=?2 WHERE node_uuid=?1",
            params![a.log.node.to_string(), hash],
        )
        .unwrap();

    let stalled = c.pass();
    assert_eq!(stalled.applied, 0, "an empty-projection head is not ready");
    assert_eq!(
        c.log_row(&a.log),
        (1, 2, Some("opaque_layout".into())),
        "the log is marked blocked"
    );
    assert_eq!(c.state(&a.log, 2), "parked_layout");

    c.conn
        .execute(
            "UPDATE journal SET hlc=?2,kind=?3,kind_version=?4,deps=?5,creates=?6,needs=?7,payload=?8, \
             apply_state='pending' WHERE counter=2 AND log_ix=(SELECT log_ix FROM sync_logs WHERE node_uuid=?1)",
            params![
                a.log.node.to_string(),
                hlc,
                kind,
                version,
                deps,
                creates,
                needs,
                payload
            ],
        )
        .unwrap();
    let released = c.pass();
    assert_eq!(released.applied, 1, "{released:?}");
    assert_eq!(c.state(&a.log, 2), "applied");
    assert_eq!(c.log_row(&a.log).0, 2, "applied_through advanced");
}

// ---- opaque stall, cross-log stall and release (ht-j87z.10) ---------------------

/// The cross-log scenario: log P (node A) holds known 1 (applied), an opaque
/// entry 2 that is really a `test.put` and a known 3; log Q (node B) has a
/// genesis and an entry 2 that depends on `P@3`.
struct Stalled {
    /// The receiving node, with P up to 1 applied and the rest not yet shipped.
    c: Node,
    p: LogKey,
    q: LogKey,
    opaque: crate::sync::envelope::OpaqueEntry,
    p3: Envelope,
    q2: Envelope,
    b_genesis: Vec<Envelope>,
}

fn stalled() -> Stalled {
    use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
    let a = Node::new(21, base());
    let b = Node::new(22, base());
    let identity = NodeIdentity::from_seed([21; 32]);
    let hash1 = jr::serve(&a.conn, &a.log, 1, BatchLimits::default()).unwrap()[0].hash;
    let mut unsigned = synthetic_unsigned(&identity, a.log.incarnation, 2, hash1);
    unsigned.kind = PutKind::NAME.into();
    unsigned.creates = Some(EntityKey::new("e:a2").unwrap());
    unsigned.payload = br#"{"id":"a2"}"#.to_vec();
    let opaque = synthetic_entry(&identity, &unsigned);
    let p3 = UnsignedEnvelope {
        log: a.log,
        counter: 3,
        prev_hash: opaque.hash(),
        hlc: HlcStamp::from_parts(1_700_000_000_000, 2),
        kind: PutKind::NAME.into(),
        kind_version: PutKind::MAJOR,
        deps: Deps::empty(),
        creates: Some(EntityKey::new("e:a3").unwrap()),
        needs: vec![],
        payload: br#"{"id":"a3"}"#.to_vec(),
    }
    .seal(&identity);
    let b_genesis = jr::serve(&b.conn, &b.log, 1, BatchLimits::default()).unwrap();
    let q2 = UnsignedEnvelope {
        log: b.log,
        counter: 2,
        prev_hash: b_genesis[0].hash,
        hlc: HlcStamp::from_parts(1_700_000_000_000, 3),
        kind: PutKind::NAME.into(),
        kind_version: PutKind::MAJOR,
        deps: Deps::new(vec![(a.log, 3)], &b.log).unwrap(),
        creates: Some(EntityKey::new("e:b2").unwrap()),
        needs: vec![],
        payload: br#"{"id":"b2"}"#.to_vec(),
    }
    .seal(&NodeIdentity::from_seed([22; 32]));
    let mut c = Node::new(23, base());
    c.ship(&a, a.log, 1);
    c.pass();
    Stalled {
        c,
        p: a.log,
        q: b.log,
        opaque,
        p3,
        q2,
        b_genesis,
    }
}

impl Stalled {
    fn ingest_p_tail(&mut self) {
        let entries = vec![self.opaque.clone().into(), self.p3.clone().into()];
        let outcome =
            jr::ingest_entries(&self.c.ctx, &mut self.c.conn, &self.p, entries, &budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { stored: 2, .. }),
            "{outcome:?}"
        );
    }

    fn ingest_q(&mut self) {
        let mut entries = self.b_genesis.clone();
        entries.push(self.q2.clone());
        let outcome =
            jr::ingest(&self.c.ctx, &mut self.c.conn, &self.q, entries, &budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { stored: 2, .. }),
            "{outcome:?}"
        );
    }
}

#[test]
fn opaque_head_stops_log_with_opaque_layout() {
    let mut s = stalled();
    s.ingest_p_tail();
    let progress = s.c.pass();
    assert_eq!(progress.applied, 0, "nothing applies for P: {progress:?}");
    assert_eq!(progress.parked, 0, "{progress:?}");
    assert_eq!(
        s.c.log_row(&s.p),
        (1, 3, Some(OPAQUE_LAYOUT.to_owned())),
        "applied_through unchanged, stall recorded"
    );
    assert_eq!(s.c.state(&s.p, 3), "pending", "not parked_unknown");
    assert_eq!(
        s.c.count("SELECT COUNT(*) FROM journal WHERE apply_state='parked_unknown'"),
        0
    );
    // No news: the stall is written once.
    let before = s.c.conn.total_changes();
    s.c.pass();
    assert_eq!(s.c.conn.total_changes(), before);
}

#[test]
fn dependent_log_blocks_on_deps_string() {
    let mut s = stalled();
    s.ingest_p_tail();
    s.ingest_q();
    s.c.pass();
    assert_eq!(s.c.state(&s.q, 1), "applied");
    assert_eq!(s.c.state(&s.q, 2), "pending");
    assert_eq!(
        s.c.log_row(&s.q),
        (1, 2, Some(blocked_text(&s.p, 3))),
        "the ordinary deps text, not a parked state"
    );
    assert_eq!(
        s.c.log_row(&s.p).2.as_deref(),
        Some(OPAQUE_LAYOUT),
        "the stalled log keeps its own text"
    );
    assert_eq!(
        s.c.count("SELECT COUNT(*) FROM journal WHERE apply_state='parked_dep'"),
        0
    );
}
