use super::*;

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::node::NodeIdentity;
use crate::protocol::time::{CallBudget, Clock, MonoInstant, UtcMillis};
use crate::store::connection::{DecisionInstant, XorShift};
use crate::store::journal::{self, FreezeOutcome};
use crate::sync::envelope::{Deps, Envelope, UnsignedEnvelope};
use crate::sync::facts::Predecessor;
use crate::sync::facts::{FactKind, FactRegistry};
use crate::sync::hlc::HlcStamp;
use crate::sync::{EntityKey, LogKey};

const SEED_A: [u8; 32] = crate::test_support::TEST_NODE_SEED;
const SEED_B: [u8; 32] = [2; 32];

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

/// A store file removed with its WAL sidecars when the test ends.
struct Store {
    path: PathBuf,
}

impl Store {
    fn new() -> Self {
        Self {
            path: crate::test_support::scratch::temp_path(format!(
                "ht-incarnations-{}.db",
                uuid::Uuid::new_v4()
            )),
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.path.clone().into_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(name);
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Put {
    id: String,
    #[serde(default)]
    fail: Option<String>,
}

struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(_: &Put) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(
        _: &Transaction<'_>,
        _: &mut crate::sync::facts::ApplyContext<'_>,
        p: Put,
    ) -> Result<(), crate::sync::facts::FactError> {
        match p.fail {
            Some(detail) => Err(crate::sync::facts::FactError::Rejected(ApiError::new(
                ErrorCode::NotFound,
                detail,
            ))),
            None => Ok(()),
        }
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<PutKind>())
}

fn identity_a() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::from_seed(SEED_A))
}

fn identity_b() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::from_seed(SEED_B))
}

fn budget() -> CallBudget {
    use crate::protocol::time::Cancellation;
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Node A's store with `puts` entries after its genesis; returns A's log and
/// entries `1..=puts+1`.
fn chain_of_a(store: &Store, puts: usize) -> (LogKey, Vec<Envelope>) {
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock));
    ctx.seed_incarnations(42);
    let mut conn = ctx.open_writer().unwrap();
    let report = journal::open(&ctx, &mut conn, identity_a(), "a-host", registry()).unwrap();
    for i in 0..puts {
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d: DecisionInstant, ()| {
                journal::author::<PutKind>(
                    tx,
                    &ctx,
                    d,
                    &Put {
                        id: format!("a{i}"),
                        fail: None,
                    },
                )
            },
        )
        .unwrap();
    }
    let entries = (1..=puts as i64 + 1)
        .map(|counter| {
            let (n, inc): (String, u32) = conn
                .query_row(
                    "SELECT node_uuid,incarnation FROM sync_logs WHERE is_self=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            let log = LogKey {
                node: n.parse().unwrap(),
                incarnation: inc,
            };
            let mut served = journal::serve(
                &conn,
                &log,
                counter as u64,
                crate::sync::BatchLimits {
                    entries: 1,
                    bytes: 1 << 20,
                },
            )
            .unwrap();
            served.remove(0)
        })
        .collect();
    (report.self_log, entries)
}

/// Node B's opened store holding A's chain.
fn node_b_with(store: &Store, log: &LogKey, entries: &[Envelope]) -> (StoreContext, Connection) {
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock))
        .with_node_seed(crate::test_support::node_seed_for(identity_b().uuid()));
    ctx.seed_incarnations(77);
    let mut conn = ctx.open_writer().unwrap();
    journal::open(&ctx, &mut conn, identity_b(), "b-host", registry()).unwrap();
    journal::ingest(&ctx, &mut conn, log, entries.to_vec(), &budget()).unwrap();
    (ctx, conn)
}

fn signed_entry(log: LogKey, counter: u64, prev_hash: [u8; 32], payload: &str) -> Envelope {
    UnsignedEnvelope {
        log,
        counter,
        prev_hash,
        hlc: HlcStamp::from_parts(1_700_000_000_000, counter as u16),
        kind: "test.put".into(),
        kind_version: 1,
        deps: Deps::empty(),
        // What `PutKind` derives from the payload, so the entry applies.
        creates: serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| EntityKey::new(format!("e:{}", v["id"].as_str()?)).ok()),
        needs: Vec::new(),
        payload: payload.as_bytes().to_vec(),
    }
    .seal(&identity_a())
}

fn proof(log: LogKey, x: u64, held: &Envelope, conflicting: Envelope) -> Proof {
    Proof {
        log,
        x,
        held: held.clone(),
        conflicting,
    }
}

fn freeze_of(conn: &Connection, log: &LogKey) -> (String, Option<i64>) {
    conn.query_row(
        "SELECT state,frozen_at_counter FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
        params![log.node.to_string(), log.incarnation],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn abort_message<T>(result: rusqlite::Result<T>) -> String {
    match result {
        Ok(_) => panic!("expected an abort"),
        Err(e) => e.to_string(),
    }
}

/// Two nodes: A's four-entry chain held by B.
fn fixture() -> (
    Store,
    Store,
    LogKey,
    Vec<Envelope>,
    StoreContext,
    Connection,
) {
    let (sa, sb) = (Store::new(), Store::new());
    let (log, entries) = chain_of_a(&sa, 3);
    let (ctx, conn) = node_b_with(&sb, &log, &entries);
    (sa, sb, log, entries, ctx, conn)
}

#[test]
fn record_fork_freezes_at_x_minus_1() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let rival = signed_entry(log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let source = identity_b().uuid();

    let tx = conn.transaction().unwrap();
    let record = record_fork(&tx, &proof(log, 3, &entries[2], rival), Some(source)).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        record,
        ForkRecord {
            new: true,
            frozen_at: 2
        }
    );
    assert_eq!(freeze_of(&conn, &log), ("frozen".into(), Some(2)));
    let row: (String, i64, i64, i64, String) = conn
        .query_row(
            "SELECT origin,proof_counter,held_counter,frozen_at,source_node FROM sync_forks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, ("evidence".into(), 3, 3, 2, source.to_string()));
}

#[test]
fn record_fork_stores_missing_entry_in_journal_forked() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let rival = signed_entry(log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let journal_before = count(&conn, "SELECT COUNT(*) FROM journal");

    let tx = conn.transaction().unwrap();
    record_fork(&tx, &proof(log, 3, &entries[2], rival.clone()), None).unwrap();
    tx.commit().unwrap();

    let stored: Vec<u8> = conn
        .query_row("SELECT hash FROM journal_forked", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, rival.hash.to_vec(), "only the conflicting entry");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 1);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM journal"),
        journal_before,
        "the held branch stays in journal untouched"
    );
}

#[test]
fn record_fork_dedups_repeated_evidence() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let rival = signed_entry(log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let p = proof(log, 3, &entries[2], rival);

    let tx = conn.transaction().unwrap();
    assert!(record_fork(&tx, &p, None).unwrap().new);
    let again = record_fork(&tx, &p, Some(identity_b().uuid())).unwrap();
    tx.commit().unwrap();

    assert_eq!(
        again,
        ForkRecord {
            new: false,
            frozen_at: 2
        }
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 1);
}

#[test]
fn record_fork_lowers_never_raises() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let at3 = proof(
        log,
        3,
        &entries[2],
        signed_entry(log, 3, entries[1].hash, r#"{"id":"r3"}"#),
    );
    let at4 = proof(
        log,
        4,
        &entries[3],
        signed_entry(log, 4, entries[2].hash, r#"{"id":"r4"}"#),
    );
    let at2 = proof(
        log,
        2,
        &entries[1],
        signed_entry(log, 2, entries[0].hash, r#"{"id":"r2"}"#),
    );

    let tx = conn.transaction().unwrap();
    assert_eq!(record_fork(&tx, &at3, None).unwrap().frozen_at, 2);
    let higher = record_fork(&tx, &at4, None).unwrap();
    assert_eq!(
        higher,
        ForkRecord {
            new: true,
            frozen_at: 2
        },
        "a later fork is new evidence but never raises the freeze"
    );
    assert_eq!(freeze_of(&tx, &log), ("frozen".into(), Some(2)));
    let lower = record_fork(&tx, &at2, None).unwrap();
    assert_eq!(lower.frozen_at, 1);
    tx.commit().unwrap();
    assert_eq!(freeze_of(&conn, &log), ("frozen".into(), Some(1)));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_forks"), 3);
}

#[test]
fn record_fork_refuses_active_self_log_and_writes_nothing() {
    let (_sa, _sb, _log, _entries, _ctx, mut conn) = fixture();
    let (n, inc): (String, u32) = conn
        .query_row(
            "SELECT node_uuid,incarnation FROM sync_logs WHERE is_self=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let own = LogKey {
        node: n.parse().unwrap(),
        incarnation: inc,
    };
    let a = signed_entry(own, 2, [0; 32], "{}");
    let b = signed_entry(own, 2, [1; 32], "{}");
    let tx = conn.transaction().unwrap();
    let err = record_fork(&tx, &proof(own, 2, &a, b), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(err.detail, "cannot freeze the active self log");
    drop(tx);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_forks"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 0);
}

#[test]
fn raw_freeze_raise_trigger_aborts() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let tx = conn.transaction().unwrap();
    assert_eq!(
        journal::freeze(&tx, &log, 2).unwrap(),
        FreezeOutcome::Frozen
    );
    let message = abort_message(tx.execute(
        "UPDATE sync_logs SET frozen_at_counter=3 WHERE node_uuid=?1 AND incarnation=?2",
        params![log.node.to_string(), log.incarnation],
    ));
    assert!(
        message.contains("sync_logs_freeze_lowers_only"),
        "{message}"
    );
    // Lowering and an equal write pass.
    tx.execute(
        "UPDATE sync_logs SET frozen_at_counter=1 WHERE node_uuid=?1",
        [log.node.to_string()],
    )
    .unwrap();
    assert_eq!(freeze_of(&tx, &log), ("frozen".into(), Some(1)));
    drop(entries);
}

#[test]
fn i2_holds_after_record_fork() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let tx = conn.transaction().unwrap();
    for (x, held) in [(3u64, &entries[2]), (4, &entries[3]), (2, &entries[1])] {
        let prev = entries[x as usize - 2].hash;
        let rival = signed_entry(log, x, prev, &format!(r#"{{"id":"r{x}"}}"#));
        record_fork(&tx, &proof(log, x, held, rival), None).unwrap();
    }
    tx.commit().unwrap();

    // I2: every journal_forked row of a log is above that log's freeze.
    let violations = count(
        &conn,
        "SELECT COUNT(*) FROM journal_forked f JOIN sync_logs l USING(log_ix) \
         WHERE l.state='frozen' AND f.counter <= l.frozen_at_counter",
    );
    assert_eq!(violations, 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 3);
    let unfrozen = count(
        &conn,
        "SELECT COUNT(*) FROM journal_forked f JOIN sync_logs l USING(log_ix) WHERE l.state<>'frozen'",
    );
    assert_eq!(unfrozen, 0, "every log with forked rows is frozen");
}

#[test]
fn next_incarnation_skips_used_own_incarnation() {
    let (sa, _sb) = (Store::new(), ());
    let ctx = StoreContext::new(sa.path.clone(), Arc::new(FixedClock));
    ctx.seed_incarnations(42);
    let mut conn = ctx.open_writer().unwrap();
    let report = journal::open(&ctx, &mut conn, identity_a(), "a-host", registry()).unwrap();
    let mut reference = XorShift::fixed(42);
    let first = reference.draw_incarnation();
    let second = reference.draw_incarnation();
    let third = reference.draw_incarnation();
    assert_eq!(report.self_log.incarnation, first, "bootstrap drew first");
    assert_ne!(first, second);

    // Restart the fixed sequence: its first draw collides with the bootstrap log.
    ctx.seed_incarnations(42);
    let tx = conn.transaction().unwrap();
    assert_eq!(next_incarnation(&tx, &ctx).unwrap(), second);

    // A second forced collision: an own row on the next draw too.
    ctx.seed_incarnations(42);
    tx.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         SELECT id,?1,1,zeroblob(32),zeroblob(32),0,zeroblob(32),0 FROM local_node",
        [second],
    )
    .unwrap();
    // A foreign log with the third draw does not block it.
    tx.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES ('ffffffffffffffffffffffffffffffff',?1,0,zeroblob(32),zeroblob(32),0,zeroblob(32),0)",
        [third],
    )
    .unwrap();
    assert_eq!(next_incarnation(&tx, &ctx).unwrap(), third);
}

#[test]
fn append_only_and_immutable_triggers_abort_by_name() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    let rival = signed_entry(log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let tx = conn.transaction().unwrap();
    record_fork(&tx, &proof(log, 3, &entries[2], rival), None).unwrap();
    tx.execute(
        "INSERT INTO sync_reissues(source_log_ix,source_counter,source_hash,target_ingest_seq,reissued_at) \
         SELECT log_ix,3,zeroblob(32),(SELECT MIN(ingest_seq) FROM journal),0 FROM sync_logs WHERE node_uuid=?1",
        [log.node.to_string()],
    )
    .unwrap();

    let message = abort_message(tx.execute("DELETE FROM sync_forks", []));
    assert!(message.contains("sync_forks_append_only"), "{message}");
    let message = abort_message(tx.execute("DELETE FROM sync_reissues", []));
    assert!(message.contains("sync_reissues_append_only"), "{message}");
    let message = abort_message(tx.execute("UPDATE sync_reissues SET reissued_at=1", []));
    assert!(message.contains("sync_reissues_immutable"), "{message}");
    assert_eq!(count(&tx, "SELECT COUNT(*) FROM sync_forks"), 1);
    assert_eq!(count(&tx, "SELECT COUNT(*) FROM sync_reissues"), 1);
}

// ---- reissue_pass (ht-5fr.6.6) ----------------------------------------------

fn decision() -> DecisionInstant {
    DecisionInstant {
        utc: FixedClock.utc_now(),
        monotonic: FixedClock.monotonic_now(),
    }
}

/// Node A's store whose first log holds genesis and `puts` entries and has
/// been rotated away: the old log is frozen at `frozen_at`, a fresh
/// incarnation is active.
struct Owner {
    _store: Store,
    ctx: StoreContext,
    conn: Connection,
    old: LogKey,
    /// The old log's `journal` entries, counters `1..`.
    entries: Vec<Envelope>,
}

fn owner(puts: usize, frozen_at: u64) -> Owner {
    let store = Store::new();
    let (old, entries) = chain_of_a(&store, puts);
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock));
    let mut conn = ctx.open_writer().unwrap();
    journal::open(&ctx, &mut conn, identity_a(), "a-host", registry()).unwrap();
    let tx = conn.transaction().unwrap();
    journal::begin_incarnation(
        &tx,
        &ctx,
        decision(),
        900,
        Some(Predecessor {
            incarnation: old.incarnation,
            frozen_at: Some(frozen_at),
        }),
        "a-host",
    )
    .unwrap();
    journal::freeze(&tx, &old, frozen_at).unwrap();
    tx.commit().unwrap();
    Owner {
        _store: store,
        ctx,
        conn,
        old,
        entries,
    }
}

impl Owner {
    /// Stores a forked entry of the old log and returns it.
    fn fork(&mut self, counter: u64, prev: [u8; 32], payload: &str) -> Envelope {
        let entry = signed_entry(self.old, counter, prev, payload);
        let tx = self.conn.transaction().unwrap();
        journal::store_forked(&tx, &self.old, std::slice::from_ref(&entry), None).unwrap();
        tx.commit().unwrap();
        entry
    }

    fn pass(&mut self, limits: ReissueLimits) -> ReissueProgress {
        run_pass(&self.ctx, &mut self.conn, limits)
    }

    /// Source hashes in the order their copies were authored.
    fn reissued_order(&self) -> Vec<[u8; 32]> {
        reissued_order(&self.conn)
    }
}

fn run_pass(ctx: &StoreContext, conn: &mut Connection, limits: ReissueLimits) -> ReissueProgress {
    ctx.execute_decision(
        conn,
        |_| Ok(()),
        |tx, d, ()| reissue_pass(tx, ctx, d, limits).map(|r| r.done().expect("not held")),
    )
    .unwrap()
}

fn reissued_order(conn: &Connection) -> Vec<[u8; 32]> {
    let mut statement = conn
        .prepare("SELECT source_hash FROM sync_reissues ORDER BY target_ingest_seq")
        .unwrap();
    statement
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|hash| hash.unwrap().try_into().unwrap())
        .collect()
}

const ALL: ReissueLimits = ReissueLimits {
    entries: 64,
    wall_ms: 100,
};

// Kills: an order by counter alone (branches of one counter unordered), a
// per-branch drain (all of one branch before the next), or a branch whose own
// order is reversed.
#[test]
fn interleave_by_counter_keeps_branch_order() {
    let mut o = owner(3, 2); // journal tail: counters 3 and 4
    let prev2 = o.entries[1].hash;
    let x3 = o.fork(3, prev2, r#"{"id":"x3"}"#);
    let x4 = o.fork(4, x3.hash, r#"{"id":"x4"}"#);
    let y3 = o.fork(3, prev2, r#"{"id":"y3"}"#);

    let progress = o.pass(ALL);
    assert_eq!(
        progress,
        ReissueProgress {
            reissued: 5,
            waiting: 0,
            has_more: false
        }
    );
    assert_eq!(
        o.reissued_order(),
        vec![
            o.entries[2].hash,
            x3.hash,
            y3.hash,
            o.entries[3].hash,
            x4.hash
        ],
        "counter 3 (journal, then forked in arrival order), then counter 4"
    );
    // The copies keep the payload bytes in that order.
    let payloads: Vec<String> = {
        let mut st = o
            .conn
            .prepare(
                "SELECT j.payload FROM journal j JOIN local_node n ON n.active_log_ix=j.log_ix \
                 WHERE j.counter>1 ORDER BY j.counter",
            )
            .unwrap();
        st.query_map([], |r| r.get::<_, Vec<u8>>(0))
            .unwrap()
            .map(|p| String::from_utf8(p.unwrap()).unwrap())
            .collect()
    };
    assert_eq!(payloads.len(), 5);
    assert_eq!(payloads[1], r#"{"id":"x3"}"#);
    assert_eq!(payloads[4], r#"{"id":"x4"}"#);
}

// Kills: a candidate query that re-issues the frozen log's genesis (it sits
// above a freeze at 0), which would put a second node.genesis on the active
// log.
#[test]
fn node_genesis_skipped() {
    let mut o = owner(2, 0); // everything including genesis is above the freeze
    let progress = o.pass(ALL);
    assert_eq!(progress.reissued, 2, "the two puts, not the genesis");
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM sync_reissues WHERE source_counter=1"
        ),
        0
    );
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM journal j JOIN local_node n ON n.active_log_ix=j.log_ix \
             WHERE j.kind='node.genesis'"
        ),
        1,
        "only the active log's own genesis"
    );
}

// Kills: a forked row re-issued ahead of its predecessor, a pass that stops at
// the first waiting row, and a `waiting` count that is not reported.
#[test]
fn forked_row_waits_for_predecessor_counted_in_waiting() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    let z3 = signed_entry(o.old, 3, prev2, r#"{"id":"z3"}"#);
    let z4 = o.fork(4, z3.hash, r#"{"id":"z4"}"#); // z3 not stored yet

    let first = o.pass(ALL);
    assert_eq!(
        first,
        ReissueProgress {
            reissued: 2,
            waiting: 1,
            has_more: false
        },
        "the journal tail proceeds, z4 waits"
    );
    assert!(!o.reissued_order().contains(&z4.hash));

    let tx = o.conn.transaction().unwrap();
    journal::store_forked(&tx, &o.old, std::slice::from_ref(&z3), None).unwrap();
    tx.commit().unwrap();
    let second = o.pass(ALL);
    assert_eq!(
        second,
        ReissueProgress {
            reissued: 2,
            waiting: 0,
            has_more: false
        },
        "z3 then z4 in one pass: z4 became ready when z3 was re-issued"
    );
    let order = o.reissued_order();
    assert_eq!(&order[2..], &[z3.hash, z4.hash]);
}

// Kills: a candidate list that ignores `sync_reissues` (a second pass would
// author every copy again).
#[test]
fn second_pass_noop() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    o.fork(3, prev2, r#"{"id":"x3"}"#);
    assert_eq!(o.pass(ALL).reissued, 3);
    let journal_rows = count(&o.conn, "SELECT COUNT(*) FROM journal");
    let reissues = count(&o.conn, "SELECT COUNT(*) FROM sync_reissues");

    assert_eq!(o.pass(ALL), ReissueProgress::default());
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM journal"), journal_rows);
    assert_eq!(
        count(&o.conn, "SELECT COUNT(*) FROM sync_reissues"),
        reissues
    );
}

// Kills: a pass that aborts on a handler rejection (the whole quantum would
// roll back and the job never advance), or that skips the sync_reissues row
// of a failed copy (it would be re-issued forever).
#[test]
fn handler_rejection_recorded_failed_no_abort() {
    let mut o = owner(2, 2); // journal tail: counter 3
    let prev2 = o.entries[1].hash;
    let bad = o.fork(3, prev2, r#"{"id":"bad","fail":"nope"}"#);
    let good = o.fork(4, bad.hash, r#"{"id":"after"}"#);

    let progress = o.pass(ALL);
    assert_eq!(progress.reissued, 3, "rejection does not stop the pass");
    let (state, error): (String, Option<String>) = o
        .conn
        .query_row(
            "SELECT j.apply_state,j.apply_error FROM sync_reissues r \
             JOIN journal j ON j.ingest_seq=r.target_ingest_seq WHERE r.source_hash=?1",
            [bad.hash.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert!(error.unwrap().contains("nope"));
    let after: String = o
        .conn
        .query_row(
            "SELECT j.apply_state FROM sync_reissues r \
             JOIN journal j ON j.ingest_seq=r.target_ingest_seq WHERE r.source_hash=?1",
            [good.hash.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, "applied", "the branch continues past the rejection");
    assert_eq!(o.pass(ALL), ReissueProgress::default(), "failed is done");
}

// Kills: a pass whose journal writes and sync_reissues rows can diverge: a
// rolled-back quantum that leaves either, a limit that is not resumable, and a
// restart that repeats or skips a copy.
#[test]
fn crash_between_passes_resumes_consistently() {
    let mut o = owner(4, 2); // journal tail: counters 3, 4, 5
    let prev2 = o.entries[1].hash;
    let x3 = o.fork(3, prev2, r#"{"id":"x3"}"#);
    o.fork(4, x3.hash, r#"{"id":"x4"}"#);
    let expected_total = 5;

    // A quantum that never commits leaves nothing behind.
    {
        let ctx = &o.ctx;
        let tx = o.conn.transaction().unwrap();
        let progress = reissue_pass(&tx, ctx, decision(), ALL)
            .unwrap()
            .done()
            .unwrap();
        assert_eq!(progress.reissued, expected_total);
        drop(tx);
    }
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM sync_reissues"), 0);

    let first = o.pass(ReissueLimits {
        entries: 2,
        wall_ms: 100,
    });
    assert_eq!(
        first,
        ReissueProgress {
            reissued: 2,
            waiting: 0,
            has_more: true
        }
    );
    let committed = o.reissued_order();
    assert_eq!(committed.len(), 2);

    // The process dies here: reopen the same file with a fresh context.
    let Owner {
        _store,
        ctx,
        conn,
        old,
        entries,
    } = o;
    drop(conn);
    drop(ctx);
    let ctx = StoreContext::new(_store.path.clone(), Arc::new(FixedClock));
    let mut conn = ctx.open_writer().unwrap();
    journal::open(&ctx, &mut conn, identity_a(), "a-host", registry()).unwrap();
    assert_eq!(
        reissued_order(&conn),
        committed,
        "state survived the restart"
    );

    let rest = run_pass(&ctx, &mut conn, ALL);
    assert_eq!(
        rest,
        ReissueProgress {
            reissued: 3,
            waiting: 0,
            has_more: false
        }
    );
    let order = reissued_order(&conn);
    assert_eq!(&order[..2], committed.as_slice(), "no copy repeated");
    assert_eq!(order.len(), expected_total);
    let mut distinct = order.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), expected_total, "no source re-issued twice");
    assert_eq!(order[0], entries[2].hash);
    // journal and sync_reissues agree: one active-log entry per reissue row.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM journal j JOIN local_node n ON n.active_log_ix=j.log_ix WHERE j.counter>1"
        ),
        expected_total as i64
    );
    assert_eq!(old.node, identity_a().uuid());
}

// Kills: a sync_reissues row whose target is not on the active log (I3), one
// naming a source outside a frozen own log, or an active-log entry with no row.
#[test]
fn i3_holds() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    o.fork(3, prev2, r#"{"id":"x3"}"#);
    o.pass(ALL);

    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM sync_reissues"), 3);
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM sync_reissues r \
             JOIN journal t ON t.ingest_seq=r.target_ingest_seq \
             JOIN local_node n ON n.active_log_ix=t.log_ix"
        ),
        3,
        "every target entry is authored on the active log"
    );
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM sync_reissues r JOIN sync_logs s ON s.log_ix=r.source_log_ix \
             WHERE s.is_self=1 AND s.state='frozen'"
        ),
        3,
        "every source is on a frozen own log"
    );
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM journal j JOIN local_node n ON n.active_log_ix=j.log_ix \
             WHERE j.counter>1 AND j.ingest_seq NOT IN (SELECT target_ingest_seq FROM sync_reissues)"
        ),
        0
    );
}

// Kills: a wall limit that stalls the job (zero re-issued) or one that is
// ignored (everything re-issued).
#[test]
fn wall_limit_still_makes_progress() {
    let mut o = owner(3, 2);
    let progress = o.pass(ReissueLimits {
        entries: 64,
        wall_ms: 0,
    });
    assert_eq!(
        progress,
        ReissueProgress {
            reissued: 1,
            waiting: 0,
            has_more: true
        }
    );
}

// ---- rotation, clone gate, restore successor, branch pulls (ht-5fr.6.7) -----

use crate::store::apply::{ApplyLimits, apply_pass};
use crate::store::connection::HoldSource;
use crate::sync::BatchLimits;
use crate::sync::coordinator::{CloneKind, CloneReport};
use crate::sync::facts::{ApplyContext, FactError, GenesisPayload};
use crate::sync::journal_port::IngestOutcome;

const OPERATOR: &str = "operator:local-user:501";

/// A clock whose UTC reading the test sets.
struct TickClock(std::sync::atomic::AtomicI64);
impl TickClock {
    fn new(utc: i64) -> Arc<Self> {
        Arc::new(Self(std::sync::atomic::AtomicI64::new(utc)))
    }
    fn set(&self, utc: i64) {
        self.0.store(utc, std::sync::atomic::Ordering::SeqCst);
    }
}
impl Clock for TickClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

const T0: i64 = 1_700_000_000_000;

// A message-like kind: records what it saw in `test_seen` and writes a lazy row
// and a wake row only when `delivery_suppressed` is false.
#[derive(Serialize, Deserialize)]
struct Msg {
    id: String,
}

struct MsgKind;
impl FactKind for MsgKind {
    const NAME: &'static str = "test.msg";
    const MAJOR: u32 = 1;
    type Payload = Msg;
    fn creates(p: &Msg) -> Option<EntityKey> {
        EntityKey::new(format!("m:{}", p.id)).ok()
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
        let settle = EntityKey::new(format!("s:{}", p.id)).expect("key");
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
        EntityKey::new(format!("s:{}", p.id)).ok()
    }
    fn needs(_: &Msg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Msg) -> Result<(), FactError> {
        Ok(())
    }
}

fn delivery_registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<PutKind>().with::<MsgKind>().with::<SettleKind>())
}

fn msg(id: &str) -> Msg {
    Msg { id: id.into() }
}

/// A node of one identity: its store, context, writer and active log.
struct Node {
    _store: Store,
    ctx: StoreContext,
    conn: Connection,
    log: LogKey,
    clock: Arc<TickClock>,
}

impl Node {
    /// `identity`'s node at `utc`, with incarnation draws starting from
    /// `incarnation_seed`: the same identity and seed is the same node, so two
    /// of them are a store and its older backup.
    fn new(identity: Arc<NodeIdentity>, incarnation_seed: u64, utc: i64) -> Self {
        let store = Store::new();
        let clock = TickClock::new(utc);
        let ctx = StoreContext::new(store.path.clone(), Arc::clone(&clock) as Arc<dyn Clock>)
            .with_node_seed(crate::test_support::node_seed_for(identity.uuid()));
        ctx.seed_incarnations(incarnation_seed);
        let mut conn = ctx.open_writer().unwrap();
        let report = journal::open(&ctx, &mut conn, identity, "host", delivery_registry()).unwrap();
        Self {
            _store: store,
            ctx,
            conn,
            log: report.self_log,
            clock,
        }
    }

    fn author<K: FactKind>(&mut self, payload: &K::Payload) {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| journal::author::<K>(tx, ctx, d, payload),
        )
        .unwrap();
    }

    /// Starts incarnation `incarnation` naming `predecessor` (committed).
    fn rotate_to(&mut self, incarnation: u32, predecessor: Option<Predecessor>) -> LogKey {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| journal::begin_incarnation(tx, ctx, d, incarnation, predecessor, "host"),
        )
        .unwrap();
        self.log = LogKey {
            node: self.log.node,
            incarnation,
        };
        self.log
    }

    fn entries(&self, log: &LogKey, first: u64) -> Vec<Envelope> {
        journal::serve(&self.conn, log, first, BatchLimits::default()).unwrap()
    }

    fn ingest(&mut self, from: &Node, log: &LogKey, first: u64) {
        let entries = from.entries(log, first);
        let outcome = journal::ingest(&self.ctx, &mut self.conn, log, entries, &budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { .. }),
            "{outcome:?}"
        );
    }

    fn pass(&mut self) -> usize {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| apply_pass(tx, ctx, d, ApplyLimits::default()),
        )
        .unwrap()
        .applied
    }

    fn hold(&self, source: HoldSource, on: bool) {
        self.ctx
            .sync()
            .unwrap()
            .own_history_hold()
            .set(source, on, MonoInstant(0));
    }

    fn state(&self, log: &LogKey, counter: u64) -> String {
        self.conn
            .query_row(
                "SELECT j.apply_state FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=?3",
                params![log.node.to_string(), log.incarnation, counter as i64],
                |r| r.get(0),
            )
            .unwrap()
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

    /// Runs `f` in one decision transaction of this node's writer.
    fn decide<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>, &StoreContext, DecisionInstant) -> Result<T, ApiError>,
    ) -> T {
        let ctx = &self.ctx;
        ctx.execute_decision(&mut self.conn, |_| Ok(()), |tx, d, ()| f(tx, ctx, d))
            .unwrap()
    }
}

/// The active self log and its state.
fn active_log_of(conn: &Connection) -> (LogKey, String) {
    let (node, incarnation, state): (String, u32, String) = conn
        .query_row(
            "SELECT l.node_uuid,l.incarnation,l.state FROM local_node n \
             JOIN sync_logs l ON l.log_ix=n.active_log_ix",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    (
        LogKey {
            node: node.parse().unwrap(),
            incarnation,
        },
        state,
    )
}

fn genesis_payload(conn: &Connection, log: &LogKey) -> GenesisPayload {
    let payload: Vec<u8> = conn
        .query_row(
            "SELECT j.payload FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid=?1 AND l.incarnation=?2 AND j.counter=1",
            params![log.node.to_string(), log.incarnation],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_slice(&payload).unwrap()
}

/// Node A with `puts` puts after its genesis on an unrotated active log
/// (counters `1..=puts+1`).
fn active(puts: usize) -> Active {
    let store = Store::new();
    let (log, entries) = chain_of_a(&store, puts);
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock));
    ctx.seed_incarnations(1234);
    let mut conn = ctx.open_writer().unwrap();
    journal::open(&ctx, &mut conn, identity_a(), "a-host", registry()).unwrap();
    Active {
        _store: store,
        ctx,
        conn,
        log,
        entries,
    }
}

struct Active {
    _store: Store,
    ctx: StoreContext,
    conn: Connection,
    log: LogKey,
    entries: Vec<Envelope>,
}

impl Active {
    fn rotate(&mut self, proof: &Proof, source: Option<NodeUuid>) -> Repair<Rotation> {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| owner_rotate(tx, ctx, d, proof, source),
        )
        .unwrap()
    }

    fn restore(&mut self, entry: &Envelope) -> Repair<Rotation> {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| restore_rotate(tx, ctx, d, entry, None),
        )
        .unwrap()
    }
}

fn clone_report(subject: NodeUuid, kind: CloneKind) -> CloneReport {
    CloneReport {
        subject,
        kind,
        boots: Vec::new(),
        source: identity_b().uuid(),
    }
}

// Kills: a genesis that declares no freeze (or `frozen_at` off by one), a
// rotation that leaves `active_log_ix` on the forked log, a freeze before the
// rotation (I1: the active log frozen), a missing evidence row or branch row.
#[test]
fn owner_rotate_genesis_predecessor_frozen_at_x_minus_1_moves_active_freezes_old() {
    let mut a = active(3); // counters 1..=4
    let conflicting = signed_entry(a.log, 3, a.entries[1].hash, r#"{"id":"fork3"}"#);
    let p = proof(a.log, 3, &a.entries[2], conflicting.clone());
    let source = identity_b().uuid();

    let Repair::Done(Rotation::Forked {
        rotated: Some(new),
        fork,
        branch_ix,
    }) = a.rotate(&p, Some(source))
    else {
        panic!("expected a rotation");
    };
    assert_eq!(
        fork,
        ForkRecord {
            new: true,
            frozen_at: 2
        }
    );
    assert_ne!(new.incarnation, a.log.incarnation);
    assert_eq!(freeze_of(&a.conn, &a.log), ("frozen".into(), Some(2)));
    let (active_log, active_state) = active_log_of(&a.conn);
    assert_eq!(active_log, new, "active_log_ix moved to the new log");
    assert_eq!(active_state, "active", "I1: the active log is not frozen");
    assert_eq!(
        genesis_payload(&a.conn, &new).predecessor,
        Some(Predecessor {
            incarnation: a.log.incarnation,
            frozen_at: Some(2)
        })
    );
    assert_eq!(
        count(
            &a.conn,
            "SELECT COUNT(*) FROM journal_forked WHERE counter=3 AND source_node IS NOT NULL"
        ),
        1,
        "the conflicting entry is quarantined with its source"
    );
    assert_eq!(count(&a.conn, "SELECT COUNT(*) FROM sync_forks"), 1);
    let (state, next_from, tip): (String, i64, i64) = a
        .conn
        .query_row(
            "SELECT state,next_from,tip_counter FROM sync_branches WHERE branch_ix=?1",
            [branch_ix],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((state.as_str(), next_from, tip), ("pulling", 3, 3));
}

// Kills: a re-check that trusts the first classification (a restore that the
// node authored past would still rotate as a restore and freeze nothing), or
// one that skips the freeze for the forked old log.
#[test]
fn owner_rotate_recheck_turns_restore_into_fork() {
    let mut a = active(3); // head 4
    let tail = signed_entry(a.log, 5, a.entries[3].hash, r#"{"id":"tail"}"#);
    // Classified now it would be a restore; a local command authors counter 5.
    a.ctx
        .execute_decision(
            &mut a.conn,
            |_| Ok(()),
            |tx, d, ()| {
                journal::author::<PutKind>(
                    tx,
                    &a.ctx,
                    d,
                    &Put {
                        id: "local5".into(),
                        fail: None,
                    },
                )
            },
        )
        .unwrap();

    let Repair::Done(Rotation::Forked {
        rotated: Some(new),
        fork,
        ..
    }) = a.restore(&tail)
    else {
        panic!("a restore authored past is a fork");
    };
    assert_eq!(fork.frozen_at, 4);
    assert_eq!(freeze_of(&a.conn, &a.log), ("frozen".into(), Some(4)));
    assert_eq!(
        genesis_payload(&a.conn, &new).predecessor,
        Some(Predecessor {
            incarnation: a.log.incarnation,
            frozen_at: Some(4)
        })
    );
}

// Kills: a rotation of a non-active own log (it must not author a genesis), a
// missing branch row, and a re-check that accepts a proof whose held entry this
// store does not hold.
#[test]
fn owner_rotate_non_active_records_without_rotating_and_stale_proof_writes_nothing() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    let conflicting = signed_entry(o.old, 3, prev2, r#"{"id":"x3"}"#);
    let p = proof(o.old, 3, &o.entries[2], conflicting);
    let (active_before, _) = active_log_of(&o.conn);
    let out = run_owner_rotate(&mut o, &p);
    let Repair::Done(Rotation::Forked {
        rotated: None,
        fork,
        ..
    }) = out
    else {
        panic!("no rotation for a non-active own log: {out:?}");
    };
    assert!(fork.new);
    assert_eq!(active_log_of(&o.conn).0, active_before);
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM sync_branches"), 1);

    // A held entry this store never stored: stale, nothing written.
    let alien = signed_entry(o.old, 3, prev2, r#"{"id":"alien"}"#);
    let other = signed_entry(o.old, 3, prev2, r#"{"id":"other"}"#);
    let before = o.conn.total_changes();
    let stale = proof(o.old, 3, &alien, other);
    let out = run_owner_rotate(&mut o, &stale);
    assert_eq!(out, Repair::Done(Rotation::Stale));
    assert_eq!(o.conn.total_changes(), before);
}

fn run_owner_rotate(o: &mut Owner, p: &Proof) -> Repair<Rotation> {
    let ctx = &o.ctx;
    ctx.execute_decision(
        &mut o.conn,
        |_| Ok(()),
        |tx, d, ()| owner_rotate(tx, ctx, d, p, None),
    )
    .unwrap()
}

// Kills: a restore rotation that freezes the old log or leaves it as the
// active one, or one that declares a freeze.
#[test]
fn restore_rotate_old_log_not_frozen_not_active() {
    let mut a = active(3);
    let tail = signed_entry(a.log, 5, a.entries[3].hash, r#"{"id":"tail"}"#);
    let Repair::Done(Rotation::Restored {
        new_log,
        predecessor,
    }) = a.restore(&tail)
    else {
        panic!("expected a restore rotation");
    };
    assert_eq!(predecessor, a.log);
    let (active_log, _) = active_log_of(&a.conn);
    assert_eq!(active_log, new_log);
    assert_ne!(active_log, a.log, "the old log is no longer active");
    assert_eq!(
        freeze_of(&a.conn, &a.log),
        ("active".into(), None),
        "the old log is not frozen"
    );
    assert_eq!(
        genesis_payload(&a.conn, &new_log).predecessor,
        Some(Predecessor {
            incarnation: a.log.incarnation,
            frozen_at: None
        })
    );
    assert_eq!(count(&a.conn, "SELECT COUNT(*) FROM sync_forks"), 0);
}

/// A restored node and its original: the original authored `open`, `done` and
/// the settlement of `done` after the backup, so the restored store holds only
/// the genesis.
fn restored_pair() -> (Node, Node) {
    let mut orig = Node::new(identity_a(), 600, T0);
    orig.author::<MsgKind>(&msg("open")); // 2
    orig.author::<MsgKind>(&msg("done")); // 3
    orig.author::<SettleKind>(&msg("done")); // 4
    let restored = Node::new(identity_a(), 600, T0);
    assert_eq!(restored.log, orig.log, "the backup holds the same log");
    (orig, restored)
}

fn restore_from(restored: &mut Node, orig: &Node) -> Rotation {
    let entry = orig.entries(&orig.log, 2).remove(0);
    restored
        .decide(|tx, ctx, d| restore_rotate(tx, ctx, d, &entry, None))
        .done()
        .expect("not held")
}

// Kills: an old-tail ingest that is refused (the old log is not active any
// more), applied before the own-history hold lifts, delivered for a key the
// tail settled, or suppressed for an unsettled one.
#[test]
fn restored_tail_ingest_applied_after_hold_settled_only() {
    let (orig, mut restored) = restored_pair();
    let Rotation::Restored { new_log, .. } = restore_from(&mut restored, &orig) else {
        panic!("expected a restore rotation");
    };
    assert_ne!(new_log, orig.log);

    restored.hold(HoldSource::Links, true);
    restored.ingest(&orig, &orig.log, 2);
    assert_eq!(restored.pass(), 0, "the hold keeps the old tail pending");
    assert_eq!(restored.state(&orig.log, 4), "pending");

    restored.hold(HoldSource::Links, false);
    restored.pass();
    assert_eq!(restored.state(&orig.log, 4), "applied");
    assert_eq!(restored.seen("done"), Some(true), "settled by the tail");
    assert_eq!(restored.delivered("done"), (0, 0), "no lazy row, no wake");
    assert_eq!(restored.seen("open"), Some(false), "unsettled");
    assert_eq!(restored.delivered("open"), (1, 1), "lazy row and wake");
}

// Kills: a foreign entry applied while the tail that settles its key is still
// unpulled, and one applied after the hold with suppression lost.
#[test]
fn foreign_entry_before_old_tail_waits_for_hold_then_suppressed() {
    let (orig, mut restored) = restored_pair();
    restore_from(&mut restored, &orig);
    let mut peer = Node::new(identity_b(), 700, T0);
    peer.author::<MsgKind>(&msg("done")); // 2

    restored.hold(HoldSource::Links, true);
    restored.ingest(&peer, &peer.log, 1);
    assert_eq!(restored.pass(), 0);
    assert_eq!(
        restored.state(&peer.log, 2),
        "pending",
        "waits for the hold"
    );
    assert_eq!(restored.seen("done"), None);

    restored.ingest(&orig, &orig.log, 2);
    restored.hold(HoldSource::Links, false);
    restored.pass();
    assert_eq!(restored.state(&peer.log, 2), "applied");
    assert_eq!(restored.seen("done"), Some(true), "the tail settled it");
    assert_eq!(restored.delivered("done"), (0, 0));
}

// ---- clone gate ---------------------------------------------------------------

fn snapshot(conn: &Connection) -> i64 {
    conn.total_changes() as i64
}

// Kills: a gate on `record_fork` (the freeze must still happen), a gate that
// misses one of the three repairs, and a held repair that writes.
#[test]
fn clone_suspect_holds_rotation_and_reissue_but_not_record_fork() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    let tail = o.fork(3, prev2, r#"{"id":"x3"}"#);
    let me = identity_a().uuid();
    let tx = o.conn.transaction().unwrap();
    assert!(
        record_clone_evidence(&tx, &clone_report(me, CloneKind::Handshake), UtcMillis(10)).unwrap()
    );
    tx.commit().unwrap();
    assert!(clone_suspect(&o.conn).unwrap());

    let before = snapshot(&o.conn);
    let ctx = &o.ctx;
    let tx = o.conn.transaction().unwrap();
    let conflicting = signed_entry(o.old, 3, prev2, r#"{"id":"y3"}"#);
    let p = proof(o.old, 3, &o.entries[2], conflicting);
    assert_eq!(
        owner_rotate(&tx, ctx, decision(), &p, None).unwrap(),
        Repair::Held { clone: true }
    );
    assert_eq!(
        restore_rotate(&tx, ctx, decision(), &tail, None).unwrap(),
        Repair::Held { clone: true }
    );
    assert_eq!(
        reissue_pass(&tx, ctx, decision(), ALL).unwrap(),
        Repair::Held { clone: true }
    );
    tx.commit().unwrap();
    assert_eq!(snapshot(&o.conn), before, "held repairs write nothing");
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM sync_reissues"), 0);

    // Evidence, tips and pulls are not gated.
    let tx = o.conn.transaction().unwrap();
    let low = signed_entry(o.old, 2, o.entries[0].hash, r#"{"id":"z2"}"#);
    let low_proof = proof(o.old, 2, &o.entries[1], low.clone());
    let fork = record_fork(&tx, &low_proof, None).unwrap();
    assert_eq!(fork.frozen_at, 1, "record_fork still lowers the freeze");
    upsert_branch_tip(&tx, &o.old, low.counter, low.hash).unwrap();
    tx.commit().unwrap();
    assert_eq!(freeze_of(&o.conn, &o.old), ("frozen".into(), Some(1)));
}

// Kills: a gate on any open row (it must be about this node).
#[test]
fn other_node_row_does_not_gate() {
    let mut a = active(3);
    let tx = a.conn.transaction().unwrap();
    record_clone_evidence(
        &tx,
        &clone_report(identity_b().uuid(), CloneKind::ThirdParty),
        UtcMillis(10),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(!clone_suspect(&a.conn).unwrap());

    let conflicting = signed_entry(a.log, 3, a.entries[1].hash, r#"{"id":"f"}"#);
    let p = proof(a.log, 3, &a.entries[2], conflicting);
    assert!(matches!(
        a.rotate(&p, None),
        Repair::Done(Rotation::Forked {
            rotated: Some(_),
            ..
        })
    ));
}

// Kills: a clear that leaves the gate shut, one that skips the re-issue after,
// and lineage evidence that does not gate.
#[test]
fn clear_then_rotate_and_reissue() {
    let mut a = active(3);
    let me = identity_a().uuid();
    let conflicting = signed_entry(a.log, 3, a.entries[1].hash, r#"{"id":"f"}"#);
    let p = proof(a.log, 3, &a.entries[2], conflicting);
    let tx = a.conn.transaction().unwrap();
    record_log_clone_evidence(
        &tx,
        LogEvidence::LineageBranch,
        &a.log,
        "two geneses",
        UtcMillis(5),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(
        clone_suspect(&a.conn).unwrap(),
        "lineage evidence gates too"
    );
    assert_eq!(a.rotate(&p, None), Repair::Held { clone: true });
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_logs"),
        1,
        "no rotation while held"
    );

    let tx = a.conn.transaction().unwrap();
    assert_eq!(
        clear_clone_evidence(&tx, OPERATOR, UtcMillis(20)).unwrap(),
        1
    );
    tx.commit().unwrap();
    assert!(!clone_suspect(&a.conn).unwrap());
    assert!(matches!(
        a.rotate(&p, None),
        Repair::Done(Rotation::Forked {
            rotated: Some(_),
            ..
        })
    ));

    // The same for the re-issue pass.
    let mut o = owner(3, 2);
    let tx = o.conn.transaction().unwrap();
    record_clone_evidence(&tx, &clone_report(me, CloneKind::PeerClose), UtcMillis(10)).unwrap();
    tx.commit().unwrap();
    let ctx = &o.ctx;
    let held = o
        .ctx
        .execute_decision(
            &mut o.conn,
            |_| Ok(()),
            |tx, d, ()| reissue_pass(tx, ctx, d, ALL),
        )
        .unwrap();
    assert_eq!(held, Repair::Held { clone: true });
    let tx = o.conn.transaction().unwrap();
    assert_eq!(
        clear_clone_evidence(&tx, OPERATOR, UtcMillis(30)).unwrap(),
        1
    );
    tx.commit().unwrap();
    let progress = o.pass(ALL);
    assert_eq!(progress.reissued, 2, "counters 3 and 4 above the freeze");
}

// Kills: a repeat report that inserts a second open row, one that rewrites
// `detected_at`, or `last_seen_at` that stays put.
#[test]
fn repeat_report_advances_last_seen_only() {
    let mut a = active(1);
    let me = identity_a().uuid();
    let report = clone_report(me, CloneKind::Handshake);
    let tx = a.conn.transaction().unwrap();
    assert!(record_clone_evidence(&tx, &report, UtcMillis(1_000)).unwrap());
    assert!(!record_clone_evidence(&tx, &report, UtcMillis(2_000)).unwrap());
    assert!(
        !record_clone_evidence(&tx, &report, UtcMillis(1_500)).unwrap(),
        "an older repeat does not move last_seen_at back"
    );
    tx.commit().unwrap();
    let (rows, detected, seen): (i64, i64, i64) = a
        .conn
        .query_row(
            "SELECT COUNT(*),MIN(detected_at),MAX(last_seen_at) FROM sync_clone_evidence",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((rows, detected, seen), (1, 1_000, 2_000));

    // A different kind about the same node is its own row.
    let tx = a.conn.transaction().unwrap();
    assert!(
        record_clone_evidence(
            &tx,
            &clone_report(me, CloneKind::PeerClose),
            UtcMillis(3_000)
        )
        .unwrap()
    );
    tx.commit().unwrap();
    assert_eq!(
        count(&a.conn, "SELECT COUNT(*) FROM sync_clone_evidence"),
        2
    );
}

// Kills: a deletable evidence row, a mutable cleared row, a clear that can be
// overwritten, a clear without operator provenance, and a log-kind row with no
// log (or a report row with one).
#[test]
fn append_only_and_set_once_rules() {
    let mut a = active(1);
    let me = identity_a().uuid();
    let tx = a.conn.transaction().unwrap();
    record_clone_evidence(
        &tx,
        &clone_report(me, CloneKind::Handshake),
        UtcMillis(1_000),
    )
    .unwrap();
    tx.commit().unwrap();

    let message = abort_message(a.conn.execute("DELETE FROM sync_clone_evidence", []));
    assert!(
        message.contains("sync_clone_evidence_append_only"),
        "{message}"
    );
    for update in [
        "UPDATE sync_clone_evidence SET detail='x'",
        "UPDATE sync_clone_evidence SET subject=lower(hex(randomblob(16)))",
        "UPDATE sync_clone_evidence SET detected_at=1",
        "UPDATE sync_clone_evidence SET last_seen_at=1",
    ] {
        let message = abort_message(a.conn.execute(update, []));
        assert!(
            message.contains("sync_clone_evidence_set_once"),
            "{update}: {message}"
        );
    }

    let tx = a.conn.transaction().unwrap();
    let bad = clear_clone_evidence(&tx, "operator:other", UtcMillis(2_000)).unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidRequest);
    assert_eq!(
        clear_clone_evidence(&tx, OPERATOR, UtcMillis(2_000)).unwrap(),
        1
    );
    assert_eq!(
        clear_clone_evidence(&tx, "operator:local-user:7", UtcMillis(3_000)).unwrap(),
        0,
        "nothing is open, so nothing is cleared again"
    );
    tx.commit().unwrap();
    let cleared_by: String = a
        .conn
        .query_row("SELECT cleared_by FROM sync_clone_evidence", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(cleared_by, OPERATOR);
    for update in [
        "UPDATE sync_clone_evidence SET cleared_by='operator:local-user:7'",
        "UPDATE sync_clone_evidence SET cleared_at=NULL, cleared_by=NULL",
        "UPDATE sync_clone_evidence SET last_seen_at=last_seen_at+1",
    ] {
        let message = abort_message(a.conn.execute(update, []));
        assert!(
            message.contains("sync_clone_evidence_set_once"),
            "{update}: {message}"
        );
    }

    // A cleared row does not block a fresh open one for the same subject/kind.
    let tx = a.conn.transaction().unwrap();
    assert!(
        record_clone_evidence(
            &tx,
            &clone_report(me, CloneKind::Handshake),
            UtcMillis(4_000)
        )
        .unwrap()
    );
    tx.commit().unwrap();
    assert!(clone_suspect(&a.conn).unwrap());

    // kind and log_ix agree.
    let subject = me.to_string();
    assert!(
        a.conn
            .execute(
                "INSERT INTO sync_clone_evidence(subject,kind,log_ix,detail,detected_at,last_seen_at) \
                 VALUES (?1,'lineage_branch',NULL,'',0,0)",
                [&subject],
            )
            .is_err()
    );
    assert!(
        a.conn
            .execute(
                "INSERT INTO sync_clone_evidence(subject,kind,log_ix,detail,detected_at,last_seen_at) \
                 VALUES (?1,'handshake',1,'',0,0)",
                [&subject],
            )
            .is_err()
    );
}

// ---- restore successor --------------------------------------------------------

/// The original node: I1 (genesis and a put) rotated to `second`; and a store
/// restored from a backup taken before the rotation (I1 genesis only).
fn rotated_original_and_backup(second: u32) -> (Node, Node) {
    let mut orig = Node::new(identity_a(), 600, T0);
    orig.author::<PutKind>(&Put {
        id: "p".into(),
        fail: None,
    });
    let first = orig.log;
    orig.clock.set(T0 + 1_000);
    orig.rotate_to(
        second,
        Some(Predecessor {
            incarnation: first.incarnation,
            frozen_at: None,
        }),
    );
    let restored = Node::new(identity_a(), 600, T0);
    assert_eq!(restored.log, first);
    (orig, restored)
}

fn successor(node: &Node) -> Option<LogKey> {
    restore_successor(&node.conn, &node.ctx).unwrap()
}

fn log_of(node: &Node, incarnation: u32) -> LogKey {
    LogKey {
        node: node.log.node,
        incarnation,
    }
}

// Kills: no successor before its genesis is ingested, a successor that is not
// the newest of a chain, and a rotation that names P (a second genesis naming
// one log) instead of the successor.
#[test]
fn restore_successor_after_rotation_chain() {
    let (mut orig, mut restored) = rotated_original_and_backup(501);
    let i1 = restored.log;
    assert_eq!(successor(&restored), None, "I2's genesis is not here yet");

    restored.ingest(&orig, &log_of(&orig, 501), 1);
    assert_eq!(successor(&restored), Some(log_of(&orig, 501)));

    // A longer chain: the newest log is the successor.
    orig.clock.set(T0 + 2_000);
    orig.rotate_to(
        502,
        Some(Predecessor {
            incarnation: 501,
            frozen_at: None,
        }),
    );
    restored.ingest(&orig, &log_of(&orig, 502), 1);
    assert_eq!(successor(&restored), Some(log_of(&orig, 502)));

    // The restore rotation names the successor, so no log is named twice.
    let tail = orig.entries(&i1, 2).remove(0);
    let Rotation::Restored {
        new_log,
        predecessor,
    } = restore_from_entry(&mut restored, &tail)
    else {
        panic!("expected a restore rotation");
    };
    assert_eq!(predecessor, log_of(&orig, 502));
    assert_eq!(
        genesis_payload(&restored.conn, &new_log).predecessor,
        Some(Predecessor {
            incarnation: 502,
            frozen_at: None
        })
    );
    let named: Vec<Option<u32>> = {
        let mut geneses = own_geneses(&restored.conn).unwrap();
        geneses.sort_by_key(|g| g.hlc);
        geneses.iter().map(|g| g.predecessor).collect()
    };
    assert_eq!(
        named,
        vec![None, Some(i1.incarnation), Some(501), Some(502)],
        "each log is named once"
    );
}

fn restore_from_entry(restored: &mut Node, entry: &Envelope) -> Rotation {
    restored
        .decide(|tx, ctx, d| restore_rotate(tx, ctx, d, entry, None))
        .done()
        .expect("not held")
}

// Kills: a bootstrap genesis that is ignored, one with an older HLC accepted,
// and several bootstraps (ambiguous) resolved by guessing.
#[test]
fn restore_successor_bootstrap_genesis_newer_hlc() {
    let mut restored = Node::new(identity_a(), 600, T0);
    // A wipe: the same key, a new incarnation, no predecessor, later.
    let wiped = Node::new(identity_a(), 0x5000_0000_0000_0001, T0 + 10_000);
    assert_ne!(wiped.log, restored.log);
    restored.ingest(&wiped, &wiped.log, 1);
    assert_eq!(successor(&restored), Some(wiped.log));

    // A bootstrap that is not newer than P's genesis is no successor.
    let mut lone = Node::new(identity_a(), 600, T0);
    let ancient = Node::new(identity_a(), 0x6000_0000_0000_0001, T0 - 10_000);
    lone.ingest(&ancient, &ancient.log, 1);
    assert_eq!(
        successor(&lone),
        None,
        "an older bootstrap is not a successor"
    );

    // Two candidate bootstraps are ambiguous.
    let second = Node::new(identity_a(), 0x7000_0000_0000_0001, T0 + 20_000);
    restored.ingest(&second, &second.log, 1);
    assert_eq!(successor(&restored), None);
}

// Kills: rule 3 missing: a P entry authored after the node moved on.
#[test]
fn restore_successor_none_when_hlc_above() {
    let (orig, mut restored) = rotated_original_and_backup(501);
    restored.ingest(&orig, &log_of(&orig, 501), 1);
    assert_eq!(successor(&restored), Some(log_of(&orig, 501)));

    restored.clock.set(T0 + 10_000);
    restored.author::<PutKind>(&Put {
        id: "late".into(),
        fail: None,
    });
    assert_eq!(successor(&restored), None, "P authored after I2's genesis");
}

// Kills: rule 2 missing: a lineage branch resolved by picking one successor.
#[test]
fn restore_successor_none_when_two_geneses_name_predecessor() {
    let (orig, mut restored) = rotated_original_and_backup(501);
    let mut clone = Node::new(identity_a(), 600, T0);
    clone.clock.set(T0 + 1_500);
    clone.rotate_to(
        502,
        Some(Predecessor {
            incarnation: restored.log.incarnation,
            frozen_at: None,
        }),
    );
    restored.ingest(&orig, &log_of(&orig, 501), 1);
    assert_eq!(successor(&restored), Some(log_of(&orig, 501)));
    restored.ingest(&clone, &log_of(&clone, 502), 1);
    assert_eq!(successor(&restored), None, "I1 is named by two geneses");
}

// Kills: rule 4 missing: a process that rotated since it started.
#[test]
fn restore_successor_none_when_rotated_since_start() {
    let (orig, mut restored) = rotated_original_and_backup(501);
    restored.ingest(&orig, &log_of(&orig, 501), 1);
    assert_eq!(successor(&restored), Some(log_of(&orig, 501)));

    // A rotation that rolls back leaves no row but the process has rotated.
    let ctx = &restored.ctx;
    let tx = restored.conn.transaction().unwrap();
    journal::begin_incarnation(&tx, ctx, decision(), 777, None, "host").unwrap();
    drop(tx);
    assert!(ctx.sync().unwrap().rotated_since_start());
    assert_eq!(successor(&restored), None);
}

// Kills: a bootstrap that counts as a rotation, and a rotation that does not.
#[test]
fn begin_incarnation_after_open_sets_rotated_open_does_not() {
    let mut node = Node::new(identity_a(), 600, T0);
    assert!(
        !node.ctx.sync().unwrap().rotated_since_start(),
        "open's bootstrap does not set it"
    );
    node.rotate_to(
        501,
        Some(Predecessor {
            incarnation: node.log.incarnation,
            frozen_at: None,
        }),
    );
    assert!(node.ctx.sync().unwrap().rotated_since_start());
}

// ---- branch pulls -------------------------------------------------------------

/// A conflicting branch of `o.old`: counters `from..=to`, chained on `prev`.
fn branch(o: &Owner, from: u64, to: u64, prev: [u8; 32], tag: &str) -> Vec<Envelope> {
    let mut out = Vec::new();
    let mut prev = prev;
    for counter in from..=to {
        let entry = signed_entry(
            o.old,
            counter,
            prev,
            &format!(r#"{{"id":"{tag}{counter}"}}"#),
        );
        prev = entry.hash;
        out.push(entry);
    }
    out
}

fn tip_row(o: &mut Owner, tip: &Envelope) -> i64 {
    let tx = o.conn.transaction().unwrap();
    let ix = upsert_branch_tip(&tx, &o.old, tip.counter, tip.hash).unwrap();
    tx.commit().unwrap();
    ix
}

fn progress(o: &mut Owner, ix: i64, entries: &[Envelope], more: bool) -> BranchVerdict {
    let tx = o.conn.transaction().unwrap();
    let verdict = branch_progress(&tx, ix, identity_b().uuid(), entries, more).unwrap();
    tx.commit().unwrap();
    verdict
}

fn branch_row(conn: &Connection, ix: i64) -> (String, i64, i64, i64, Option<String>) {
    conn.query_row(
        "SELECT state,next_from,search_span,attempts,last_error FROM sync_branches WHERE branch_ix=?1",
        [ix],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

// Kills: a due read that ignores `next_attempt_at`, one that returns
// non-pulling rows, a re-advertise that does not reset `unavailable`, and a
// first `next_from` other than `frozen_at + 1`.
#[test]
fn due_branches_after_next_attempt_at() {
    let mut o = owner(3, 2);
    let chain = branch(&o, 3, 4, o.entries[1].hash, "b");
    let tip = chain.last().unwrap().clone();
    let ix = tip_row(&mut o, &tip);
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 3));

    let due = due_branches(&o.conn, UtcMillis(0)).unwrap();
    assert_eq!(
        due,
        vec![DueBranch {
            branch_ix: ix,
            log: o.old,
            tip: (4, tip.hash),
            next_from: 3
        }]
    );

    o.conn
        .execute("UPDATE sync_branches SET next_attempt_at=5000", [])
        .unwrap();
    assert!(due_branches(&o.conn, UtcMillis(4_999)).unwrap().is_empty());
    assert_eq!(due_branches(&o.conn, UtcMillis(5_000)).unwrap().len(), 1);

    o.conn
        .execute("UPDATE sync_branches SET state='unavailable'", [])
        .unwrap();
    assert!(due_branches(&o.conn, UtcMillis(9_999)).unwrap().is_empty());
    assert_eq!(tip_row(&mut o, &tip), ix, "the same tip is the same row");
    let (state, ..) = branch_row(&o.conn, ix);
    assert_eq!(state, "pulling", "a re-advertise returns it to pulling");
    assert_eq!(due_branches(&o.conn, UtcMillis(0)).unwrap().len(), 1);

    o.conn
        .execute("UPDATE sync_branches SET state='complete'", [])
        .unwrap();
    assert!(due_branches(&o.conn, UtcMillis(9_999)).unwrap().is_empty());
}

// Kills: common entries stored again, off-chain ones dropped, a freeze left
// above the first divergence (I2), `complete` not reached at the tip.
#[test]
fn branch_progress_skips_common_stores_off_chain_lowers_freeze() {
    // Own chain 1..=4; the owner thinks all four are common (freeze 4).
    let mut o = owner(3, 4);
    let mut reply = vec![o.entries[1].clone()]; // counter 2: common
    reply.extend(branch(&o, 3, 5, o.entries[1].hash, "b")); // 3', 4', 5'
    let tip = reply.last().unwrap().clone();
    let ix = tip_row(&mut o, &tip);
    o.conn
        .execute("UPDATE sync_branches SET next_from=2", [])
        .unwrap();

    let verdict = progress(&mut o, ix, &reply, false);
    assert_eq!(
        verdict,
        BranchVerdict::Progress {
            stored: 3,
            skipped: 1,
            complete: true,
            next_from: 6,
            frozen_at: 2,
            short: false
        }
    );
    assert_eq!(freeze_of(&o.conn, &o.old), ("frozen".into(), Some(2)));
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM journal_forked"), 3);
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM journal_forked f JOIN sync_logs l ON l.log_ix=f.log_ix \
             WHERE f.counter<=l.frozen_at_counter"
        ),
        0,
        "I2: every forked row lies above the freeze"
    );
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM journal_forked WHERE source_node=?1"
                .replace("?1", &format!("'{}'", identity_b().uuid()))
                .as_str()
        ),
        3,
        "forked rows carry their source"
    );
    let (state, next_from, _, attempts, error) = branch_row(&o.conn, ix);
    assert_eq!(
        (state.as_str(), next_from, attempts, error),
        ("complete", 6, 1, None)
    );
    // The owner's own rows above the new freeze are now re-issue candidates.
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.state='frozen' AND j.counter>l.frozen_at_counter"
        ),
        2,
        "counters 3 and 4 of the owner's own branch"
    );
}

// Kills: a pulling row for an ancestor of an already pulled tip (the proof's
// own conflicting entry) that nothing ever requests again, which would keep
// the own-history `Branches` flag set; and a sibling completed although part
// of its range lies below the reply.
#[test]
fn branch_progress_completes_sibling_rows_the_reply_covers() {
    let mut o = owner(3, 4);
    let chain = branch(&o, 3, 5, o.entries[1].hash, "b"); // 3', 4', 5'
    let (ancestor, tip) = (chain[1].clone(), chain[2].clone());
    let ancestor_ix = tip_row(&mut o, &ancestor);
    let tip_ix = tip_row(&mut o, &tip);
    assert_ne!(ancestor_ix, tip_ix);
    o.conn
        .execute(
            "UPDATE sync_branches SET next_from=3 WHERE branch_ix=?1",
            [tip_ix],
        )
        .unwrap();

    // The reply for the tip starts at 3: it covers the ancestor's range 3..=4.
    let verdict = progress(&mut o, tip_ix, &chain, false);
    assert!(matches!(
        verdict,
        BranchVerdict::Progress { complete: true, .. }
    ));
    let (state, next_from, ..) = branch_row(&o.conn, ancestor_ix);
    assert_eq!((state.as_str(), next_from), ("complete", 5));
    assert!(
        due_branches(&o.conn, UtcMillis(i64::MAX))
            .unwrap()
            .is_empty()
    );

    // A sibling that still needs entries below the reply stays pulling.
    let mut o = owner(3, 4);
    let chain = branch(&o, 3, 5, o.entries[1].hash, "b");
    let (ancestor, tip) = (chain[1].clone(), chain[2].clone());
    let ancestor_ix = tip_row(&mut o, &ancestor);
    let tip_ix = tip_row(&mut o, &tip);
    o.conn
        .execute(
            "UPDATE sync_branches SET next_from=CASE branch_ix WHEN ?1 THEN 2 ELSE 3 END",
            [ancestor_ix],
        )
        .unwrap();
    progress(&mut o, tip_ix, &chain, false);
    let (state, next_from, ..) = branch_row(&o.conn, ancestor_ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 2));
}

// Kills: a first entry that does not chain accepted as a branch, a span that
// does not double, a search that does not clamp at counter 1, and a short
// source that completes the branch.
#[test]
fn branch_progress_needs_earlier_widens_search() {
    let mut o = owner(3, 4);
    let far = signed_entry(o.old, 1_000, [9; 32], r#"{"id":"far"}"#);
    let ix = tip_row(&mut o, &far);
    o.conn
        .execute("UPDATE sync_branches SET next_from=1000", [])
        .unwrap();
    let verdict = progress(&mut o, ix, &[far], true);
    assert_eq!(
        verdict,
        BranchVerdict::NeedsEarlier {
            next_from: 744,
            search_span: 256
        }
    );

    let mid = signed_entry(o.old, 744, [8; 32], r#"{"id":"mid"}"#);
    let verdict = progress(&mut o, ix, &[mid], true);
    assert_eq!(
        verdict,
        BranchVerdict::NeedsEarlier {
            next_from: 232,
            search_span: 512
        }
    );
    let (state, next_from, span, attempts, error) = branch_row(&o.conn, ix);
    assert_eq!(
        (state.as_str(), next_from, span, attempts),
        ("pulling", 232, 512, 2)
    );
    assert!(error.is_some());

    // The search clamps at 1.
    o.conn
        .execute("UPDATE sync_branches SET next_from=100, search_span=0", [])
        .unwrap();
    let low = signed_entry(o.old, 100, [7; 32], r#"{"id":"low"}"#);
    let verdict = progress(&mut o, ix, &[low], true);
    assert_eq!(
        verdict,
        BranchVerdict::NeedsEarlier {
            next_from: 1,
            search_span: 256
        }
    );
    // Nothing was admitted by any of those replies.
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM journal_forked"), 0);

    // A source that holds less than the tip leaves the row pulling; a second
    // reply with the rest completes it.
    let mut o = owner(3, 2);
    let chain = branch(&o, 3, 6, o.entries[1].hash, "s");
    let ix = tip_row(&mut o, chain.last().unwrap());
    let verdict = progress(&mut o, ix, &chain[..2], false);
    assert!(
        matches!(
            verdict,
            BranchVerdict::Progress {
                complete: false,
                short: true,
                next_from: 5,
                ..
            }
        ),
        "{verdict:?}"
    );
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 5));
    let verdict = progress(&mut o, ix, &chain[2..], false);
    assert!(
        matches!(
            verdict,
            BranchVerdict::Progress {
                complete: true,
                short: false,
                next_from: 7,
                ..
            }
        ),
        "{verdict:?}"
    );
    let (state, ..) = branch_row(&o.conn, ix);
    assert_eq!(state, "complete");
}

// Kills: an empty reply with more=false completing a row short of its tip.
#[test]
fn empty_reply_never_completes_short_of_the_tip() {
    let mut o = owner(3, 2);
    let chain = branch(&o, 3, 5, o.entries[1].hash, "e");
    let ix = tip_row(&mut o, chain.last().unwrap());
    let verdict = progress(&mut o, ix, &[], false);
    assert!(
        matches!(
            verdict,
            BranchVerdict::Progress {
                stored: 0,
                complete: false,
                short: true,
                next_from: 3,
                ..
            }
        ),
        "{verdict:?}"
    );
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 3));
}

// Kills: a short-completed row that stays complete when its tip is advertised
// again, a re-open that drops the pulled progress, a truly complete row that
// re-opens, and a higher tip that reuses the lower tip's row.
#[test]
fn complete_short_row_reopens_on_re_advertise_and_higher_tip_gets_its_own_row() {
    let mut o = owner(3, 2);
    let chain = branch(&o, 3, 8, o.entries[1].hash, "h");
    let tip6 = chain[3].clone();
    let ix = tip_row(&mut o, &tip6);
    o.conn
        .execute(
            "UPDATE sync_branches SET state='complete', next_from=5, next_attempt_at=99",
            [],
        )
        .unwrap();
    assert_eq!(tip_row(&mut o, &tip6), ix, "same tip, same row");
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 5), "progress kept");
    assert_eq!(
        count(
            &o.conn,
            "SELECT COUNT(*) FROM sync_branches WHERE next_attempt_at IS NULL"
        ),
        1,
        "the re-open clears the attempt gate"
    );

    o.conn
        .execute("UPDATE sync_branches SET state='complete', next_from=7", [])
        .unwrap();
    assert_eq!(tip_row(&mut o, &tip6), ix);
    let (state, ..) = branch_row(&o.conn, ix);
    assert_eq!(
        state, "complete",
        "a branch pulled to its tip stays complete"
    );

    let higher = tip_row(&mut o, chain.last().unwrap());
    assert_ne!(higher, ix, "a higher tip is its own row");
    let (state, ..) = branch_row(&o.conn, higher);
    assert_eq!(state, "pulling");
}

// Kills: a tip at or below the freeze requested from a counter above it
// (judged Corrupt), and a tip already held in journal_forked pulled again.
#[test]
fn tip_below_next_from_is_pulled_from_the_tip() {
    let mut o = owner(4, 4);
    let rival = signed_entry(o.old, 3, o.entries[1].hash, r#"{"id":"r3"}"#);
    let ix = tip_row(&mut o, &rival);
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("pulling", 3));
    let verdict = progress(&mut o, ix, std::slice::from_ref(&rival), false);
    assert!(
        matches!(
            verdict,
            BranchVerdict::Progress {
                stored: 1,
                complete: true,
                frozen_at: 2,
                ..
            }
        ),
        "{verdict:?}"
    );
    assert_eq!(freeze_of(&o.conn, &o.old), ("frozen".into(), Some(2)));

    // A tip the store already holds in journal_forked is complete at once.
    let mut o = owner(4, 4);
    let held = o.fork(3, o.entries[1].hash, r#"{"id":"r3"}"#);
    let ix = tip_row(&mut o, &held);
    let (state, next_from, ..) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from), ("complete", 4));
}

// Kills: a bad signature or a broken chain admitted, a Corrupt reply that
// writes entries, and a missing attempts / last_error record.
#[test]
fn branch_progress_corrupt_on_broken_chain_or_bad_sig() {
    let mut o = owner(3, 2);
    let mut chain = branch(&o, 3, 5, o.entries[1].hash, "c");
    let ix = tip_row(&mut o, &chain[2].clone());

    // A bad signature: entry 4 is signed by another node's key.
    let forged = UnsignedEnvelope {
        log: o.old,
        counter: 4,
        prev_hash: chain[0].hash,
        hlc: HlcStamp::from_parts(1_700_000_000_000, 4),
        kind: "test.put".into(),
        kind_version: 1,
        deps: Deps::empty(),
        creates: None,
        needs: Vec::new(),
        payload: br#"{"id":"forged"}"#.to_vec(),
    }
    .seal(&identity_b());
    let bad = vec![chain[0].clone(), forged];
    let verdict = progress(&mut o, ix, &bad, true);
    assert!(
        matches!(verdict, BranchVerdict::Corrupt { .. }),
        "{verdict:?}"
    );
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM journal_forked"), 0);
    let (state, next_from, _, attempts, error) = branch_row(&o.conn, ix);
    assert_eq!((state.as_str(), next_from, attempts), ("pulling", 3, 1));
    assert!(error.is_some_and(|e| !e.is_empty()));

    // A broken chain: entry 5 does not continue entry 4.
    chain[2] = signed_entry(o.old, 5, [5; 32], r#"{"id":"gap"}"#);
    let verdict = progress(&mut o, ix, &chain, true);
    assert!(
        matches!(verdict, BranchVerdict::Corrupt { .. }),
        "{verdict:?}"
    );
    assert_eq!(count(&o.conn, "SELECT COUNT(*) FROM journal_forked"), 0);
    assert_eq!(branch_row(&o.conn, ix).3, 2);

    // A reply that skips the requested counter is corrupt too.
    let skipped = vec![chain[1].clone()];
    assert!(matches!(
        progress(&mut o, ix, &skipped, true),
        BranchVerdict::Corrupt { .. }
    ));
}

// ---- status, declared freeze and invariants (ht-5fr.6.12) ---------------------

fn status_of(node: &Node) -> IncarnationsStatus {
    status(&node.conn, &node.ctx).unwrap()
}

fn evidence_rows(conn: &Connection, kind: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sync_clone_evidence WHERE kind=?1",
        [kind],
        |r| r.get(0),
    )
    .unwrap()
}

fn record_unknown(node: &mut Node) -> u32 {
    node.decide(|tx, ctx, d| record_own_unknown_newer(tx, ctx, d.utc))
}

fn clear(node: &mut Node) -> u32 {
    node.decide(|tx, _, d| clear_clone_evidence(tx, "operator:local-user:501", d.utc))
}

// Kills: a bootstrap that draws from the rng directly (a collision with a used
// own incarnation is not skipped), and a draw of 1 or a non-random constant.
#[test]
fn bootstrap_non_1_random_via_next_incarnation_skips_collision() {
    let store = Store::new();
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock));
    ctx.seed_incarnations(42);
    let mut conn = ctx.open_writer().unwrap();
    let first = journal::open(&ctx, &mut conn, identity_a(), "a-host", registry())
        .unwrap()
        .self_log;
    let mut reference = XorShift::fixed(42);
    let (d1, d2) = (reference.draw_incarnation(), reference.draw_incarnation());
    assert_eq!(
        first.incarnation, d1,
        "the bootstrap draws the seeded value"
    );
    assert_ne!(first.incarnation, 1);
    assert_ne!(d1, d2);

    // Bootstrap again over the same file with the first draw already used.
    conn.execute("UPDATE local_node SET active_log_ix=NULL", [])
        .unwrap();
    drop(conn);
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock));
    ctx.seed_incarnations(42);
    let mut conn = ctx.open_writer().unwrap();
    let second = journal::open(&ctx, &mut conn, identity_a(), "a-host", registry())
        .unwrap()
        .self_log;
    assert_eq!(
        second.incarnation, d2,
        "the colliding first draw is skipped"
    );
}

// Kills: a freeze that is not lowered, one that is raised, a missing declared
// row, a held-check that errors on an unknown log, and one that freezes the
// active self log.
#[test]
fn adopt_declared_freeze_lowers_and_ignores_unheld() {
    let (_sa, _sb, log, _entries, _ctx, mut conn) = fixture();
    let tx = conn.transaction().unwrap();
    assert!(adopt_declared_freeze(&tx, &log, 3).unwrap());
    tx.commit().unwrap();
    assert_eq!(freeze_of(&conn, &log), ("frozen".into(), Some(3)));
    let declared: (String, i64, Option<i64>) = conn
        .query_row(
            "SELECT origin,frozen_at,proof_counter FROM sync_forks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(declared, ("declared".into(), 3, None));

    let tx = conn.transaction().unwrap();
    assert!(adopt_declared_freeze(&tx, &log, 1).unwrap());
    assert!(adopt_declared_freeze(&tx, &log, 4).unwrap());
    tx.commit().unwrap();
    assert_eq!(
        freeze_of(&conn, &log),
        ("frozen".into(), Some(1)),
        "lowered to 1, never raised to 4"
    );

    let unheld = LogKey {
        node: identity_a().uuid(),
        incarnation: 999,
    };
    let own_active = LogKey {
        node: identity_b().uuid(),
        incarnation: conn
            .query_row(
                "SELECT l.incarnation FROM local_node n JOIN sync_logs l ON l.log_ix=n.active_log_ix",
                [],
                |r| r.get(0),
            )
            .unwrap(),
    };
    let rows = count(&conn, "SELECT COUNT(*) FROM sync_forks");
    let tx = conn.transaction().unwrap();
    assert!(!adopt_declared_freeze(&tx, &unheld, 1).unwrap());
    assert!(!adopt_declared_freeze(&tx, &own_active, 1).unwrap());
    tx.commit().unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_forks"), rows);
    assert_eq!(freeze_of(&conn, &own_active).0, "active");
}

/// An owner A that authored `put`, then rotated naming the first log with
/// `frozen_at`; and a third store C holding the first log's genesis and put.
fn declared_freeze_pair(frozen_at: Option<u64>) -> (Node, Node, LogKey) {
    let mut a = Node::new(identity_a(), 600, T0);
    a.author::<PutKind>(&Put {
        id: "p".into(),
        fail: None,
    });
    let first = a.log;
    a.clock.set(T0 + 1_000);
    a.rotate_to(
        501,
        Some(Predecessor {
            incarnation: first.incarnation,
            frozen_at,
        }),
    );
    let mut c = Node::new(identity_b(), 700, T0);
    c.ingest(&a, &first, 1);
    c.ingest(&a, &log_of(&a, 501), 1);
    c.pass();
    (a, c, first)
}

// Kills: a genesis handler that ignores `frozen_at`, a freeze one off, and a
// missing declared row.
#[test]
fn genesis_frozen_at_some_freezes_predecessor_on_third_store() {
    let (_a, c, first) = declared_freeze_pair(Some(1));
    assert_eq!(freeze_of(&c.conn, &first), ("frozen".into(), Some(1)));
    let row: (String, i64) = c
        .conn
        .query_row("SELECT origin,frozen_at FROM sync_forks", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(row, ("declared".into(), 1));
}

// Kills: a handler that freezes on any predecessor, `None` included.
#[test]
fn genesis_frozen_at_none_freezes_nothing() {
    let (_a, c, first) = declared_freeze_pair(None);
    assert_eq!(freeze_of(&c.conn, &first).0, "active");
    assert_eq!(count(&c.conn, "SELECT COUNT(*) FROM sync_forks"), 0);
}

// Kills: a status that drops the origin, the evidence count, the branch rows
// or the reissued/pending tallies, or counts the genesis as pending.
#[test]
fn status_reports_frozen_log_details() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    let rival = o.fork(3, prev2, r#"{"id":"x3"}"#);
    let tx = o.conn.transaction().unwrap();
    record_fork(&tx, &proof(o.old, 3, &o.entries[2], rival.clone()), None).unwrap();
    upsert_branch_tip(&tx, &o.old, rival.counter, rival.hash).unwrap();
    tx.commit().unwrap();

    let before = status(&o.conn, &o.ctx).unwrap();
    let repair = before.logs.iter().find(|l| l.log == o.old).unwrap();
    assert_eq!(repair.frozen_at, Some(2));
    assert_eq!(repair.origin, Some(ForkOrigin::Evidence));
    assert_eq!(repair.evidence_count, 1);
    assert_eq!(repair.branches.len(), 1);
    let branch = &repair.branches[0];
    assert_eq!(
        (branch.tip_counter, branch.tip_hash, branch.state),
        (3, rival.hash, BranchState::Pulling)
    );
    assert_eq!(
        (repair.reissued, repair.pending),
        (0, 3),
        "journal 3 and 4, plus the forked row; never the genesis"
    );

    o.pass(ALL);
    let after = status(&o.conn, &o.ctx).unwrap();
    let repair = after.logs.iter().find(|l| l.log == o.old).unwrap();
    assert_eq!((repair.reissued, repair.pending), (3, 0));
    assert!(
        after.logs.iter().all(|l| l.log == o.old),
        "the active log is not a repair row"
    );
}

/// A store restored to a copy with only the first log (and a late put on it),
/// plus the original that rotated to the `later` incarnations (a predecessor
/// of 0 names the first log).
fn restored_with(later: &[(u32, Option<u32>)]) -> (Node, Node) {
    let mut orig = Node::new(identity_a(), 600, T0);
    let first = orig.log.incarnation;
    for (i, (incarnation, predecessor)) in later.iter().enumerate() {
        orig.clock.set(T0 + 1_000 * (i as i64 + 1));
        orig.rotate_to(
            *incarnation,
            predecessor.map(|p| Predecessor {
                incarnation: if p == 0 { first } else { p },
                frozen_at: None,
            }),
        );
    }
    let mut restored = Node::new(identity_a(), 600, T0);
    assert_eq!(restored.log.incarnation, first);
    for (incarnation, _) in later {
        restored.ingest(&orig, &log_of(&orig, *incarnation), 1);
    }
    // The restored node moved on after the rotation: no restore successor.
    restored.clock.set(T0 + 100_000);
    restored.author::<PutKind>(&Put {
        id: "late".into(),
        fail: None,
    });
    (orig, restored)
}

// Kills: no `own_unknown_newer`, one that lists the active log, an older log
// or a foreign node's, and one that fires for a restore successor.
#[test]
fn own_unknown_newer_listed_for_unnamed_newer_incarnation() {
    let (orig, restored) = restored_with(&[(501, None)]);
    let st = status_of(&restored);
    assert_eq!(st.own_unknown_newer, vec![log_of(&orig, 501)]);
    assert_eq!(st.restore_successor, None);
    // The original's active log is the newest: nothing newer than it.
    assert!(status_of(&orig).own_unknown_newer.is_empty());
}

// Kills: a missing "named by a genesis" check (501 is newer than the active
// log but named by 502's genesis) and a rotated predecessor listed.
#[test]
fn own_unknown_newer_empty_for_rotated_predecessor() {
    let (orig, restored) = restored_with(&[(501, Some(0)), (502, Some(501))]);
    let st = status_of(&restored);
    assert_eq!(
        st.own_unknown_newer,
        vec![log_of(&orig, 502)],
        "501 is named by 502's genesis; only the unnamed 502 is a suspect"
    );
    // A properly rotated store has nothing unknown.
    let st = status_of(&orig);
    assert!(st.own_unknown_newer.is_empty());
    assert_eq!(st.restore_successor, None);
}

// Kills: an I2 check missing, an off-by-one at the freeze, an I3 check
// missing, and a healthy store reported broken.
#[test]
fn check_invariants_reports_seeded_i2_and_i3() {
    let mut o = owner(3, 2);
    let prev2 = o.entries[1].hash;
    o.fork(3, prev2, r#"{"id":"x3"}"#);
    o.pass(ALL);
    assert_eq!(check_invariants(&o.conn).unwrap(), Vec::new());

    // I2: a forked row at the freeze counter (2) is a violation; 3 is fine.
    o.conn
        .execute(
            "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,received_at) \
             SELECT log_ix,2,zeroblob(32),randomblob(32),0,'test.put',1,X'',X'',zeroblob(64),0 \
             FROM sync_logs WHERE state='frozen'",
            [],
        )
        .unwrap();
    // I3: a reissue row whose source is the old log's genesis.
    o.conn
        .execute(
            "INSERT INTO sync_reissues(source_log_ix,source_counter,source_hash,target_ingest_seq,reissued_at) \
             SELECT j.log_ix,1,j.hash,(SELECT MAX(ingest_seq) FROM journal),0 \
             FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix WHERE l.state='frozen' AND j.counter=1",
            [],
        )
        .unwrap();
    let violations = check_invariants(&o.conn).unwrap();
    let kinds: Vec<(Invariant, u64)> = violations
        .iter()
        .map(|v| (v.invariant, v.counter))
        .collect();
    assert_eq!(kinds, vec![(Invariant::I2, 2), (Invariant::I3, 1)]);
    assert!(violations.iter().all(|v| v.log == o.old));
}

// Kills: invariants that fire on a store with no repair state.
#[test]
fn check_invariants_empty_when_healthy() {
    let (_sa, _sb, log, entries, _ctx, mut conn) = fixture();
    assert_eq!(check_invariants(&conn).unwrap(), Vec::new());
    let rival = signed_entry(log, 3, entries[1].hash, r#"{"id":"r"}"#);
    let tx = conn.transaction().unwrap();
    record_fork(&tx, &proof(log, 3, &entries[2], rival), None).unwrap();
    tx.commit().unwrap();
    assert_eq!(check_invariants(&conn).unwrap(), Vec::new());
}

// Kills: a restore successor listed as a clone, recorded as evidence, or not
// reported in `restore_successor`.
#[test]
fn restore_successor_exempt_from_own_unknown_newer() {
    let (orig, mut restored) = rotated_original_and_backup(501);
    restored.ingest(&orig, &log_of(&orig, 501), 1);
    let st = status_of(&restored);
    assert_eq!(st.restore_successor, Some(log_of(&orig, 501)));
    assert!(st.own_unknown_newer.is_empty());
    assert!(st.lineage_branches.is_empty());
    assert_eq!(record_unknown(&mut restored), 0);
    assert_eq!(
        count(&restored.conn, "SELECT COUNT(*) FROM sync_clone_evidence"),
        0
    );
    assert!(!clone_suspect(&restored.conn).unwrap());
}

// Kills: no lineage row, a duplicate per call, a re-record after clearing,
// and a gate that stays shut after clearing.
#[test]
fn two_geneses_one_predecessor_records_one_lineage_branch_never_after_clear() {
    let (orig, mut restored) = rotated_original_and_backup(501);
    let mut clone = Node::new(identity_a(), 600, T0);
    clone.clock.set(T0 + 1_500);
    let i1 = restored.log;
    clone.rotate_to(
        502,
        Some(Predecessor {
            incarnation: i1.incarnation,
            frozen_at: None,
        }),
    );
    restored.ingest(&orig, &log_of(&orig, 501), 1);
    restored.ingest(&clone, &log_of(&clone, 502), 1);

    let st = status_of(&restored);
    assert_eq!(st.lineage_branches, vec![i1]);
    assert_eq!(st.restore_successor, None);
    assert!(!clone_suspect(&restored.conn).unwrap());
    record_unknown(&mut restored);
    assert_eq!(evidence_rows(&restored.conn, "lineage_branch"), 1);
    assert!(clone_suspect(&restored.conn).unwrap());
    let named: i64 = restored
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_clone_evidence e JOIN sync_logs l ON l.log_ix=e.log_ix \
             WHERE e.kind='lineage_branch' AND l.incarnation=?1",
            [i1.incarnation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(named, 1, "the row names the predecessor");

    assert_eq!(
        record_unknown(&mut restored),
        0,
        "second call records nothing"
    );
    assert!(clear(&mut restored) >= 1);
    assert!(!clone_suspect(&restored.conn).unwrap());
    assert_eq!(
        record_unknown(&mut restored),
        0,
        "cleared is acknowledged for good"
    );
    assert_eq!(evidence_rows(&restored.conn, "lineage_branch"), 1);
}

// Kills: a status without the two remedies, remedies shown with no open
// evidence, and remedies that outlive the clear.
#[test]
fn status_carries_remedies() {
    let (_orig, mut restored) = restored_with(&[(501, None)]);
    assert_eq!(status_of(&restored).remedy, None, "nothing open yet");
    assert_eq!(record_unknown(&mut restored), 1);
    let remedy = status_of(&restored).remedy.expect("open evidence");
    assert!(remedy.two_copies.contains("move one copy's"));
    assert!(remedy.two_copies.contains("sync clone-cleared"));
    assert!(remedy.single_copy.contains("sync clone-cleared"));
    assert!(!remedy.single_copy.contains("move"));
    clear(&mut restored);
    assert_eq!(status_of(&restored).remedy, None);
}

// Kills: no evidence row for the two-incarnation fixture, a second call that
// records again, and a log re-recorded after it was cleared.
#[test]
fn record_own_unknown_newer_once_and_not_after_clear() {
    let (orig, mut restored) = restored_with(&[(501, None)]);
    assert!(!clone_suspect(&restored.conn).unwrap());
    assert_eq!(record_unknown(&mut restored), 1);
    assert_eq!(evidence_rows(&restored.conn, "own_unknown_newer"), 1);
    assert!(clone_suspect(&restored.conn).unwrap());
    assert_eq!(record_unknown(&mut restored), 0);
    assert_eq!(clear(&mut restored), 1);
    assert!(!clone_suspect(&restored.conn).unwrap());
    assert_eq!(record_unknown(&mut restored), 0, "that log is acknowledged");
    assert_eq!(evidence_rows(&restored.conn, "own_unknown_newer"), 1);
    assert_eq!(
        status_of(&restored).own_unknown_newer,
        vec![log_of(&orig, 501)],
        "still listed, never re-recorded"
    );
}

// Kills: status that omits rows, their kind, log, detail or cleared state.
#[test]
fn status_clone_evidence_lists_rows() {
    let (orig, mut restored) = restored_with(&[(501, None)]);
    record_unknown(&mut restored);
    let rows = status_of(&restored).clone_evidence;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, "own_unknown_newer");
    assert_eq!(row.log, Some(log_of(&orig, 501)));
    assert_eq!(row.subject, identity_a().uuid());
    assert!(row.detail.starts_with("genesis_hlc="), "{}", row.detail);
    assert_eq!((row.cleared_at, row.cleared_by.clone()), (None, None));

    clear(&mut restored);
    let rows = status_of(&restored).clone_evidence;
    assert_eq!(rows.len(), 1, "cleared rows stay listed");
    assert!(rows[0].cleared_at.is_some());
    assert_eq!(
        rows[0].cleared_by.as_deref(),
        Some("operator:local-user:501")
    );
}

// ---- opaque rows (ht-j87z.11) ---------------------------------------------------

fn log_ix_of(conn: &Connection, log: &LogKey) -> i64 {
    conn.query_row(
        "SELECT log_ix FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
        params![log.node.to_string(), log.incarnation],
        |r| r.get(0),
    )
    .unwrap()
}

fn insert_opaque(conn: &Connection, log_ix: i64, counter: i64, hash: [u8; 32], branch: i64) {
    conn.execute(
        "INSERT INTO journal_opaque(log_ix,counter,hash,prev_hash,signature,raw,branch) \
         VALUES (?1,?2,?3,zeroblob(32),zeroblob(64),zeroblob(68),?4)",
        params![log_ix, counter, hash.as_slice(), branch],
    )
    .unwrap();
}

// Kills: a first-entry check that reads `journal` and `journal_forked` only,
// so an opaque main-chain predecessor looks unknown and the branch is searched
// earlier forever.
#[test]
fn chains_on_known_accepts_opaque_predecessor() {
    let (_sa, _sb, log, _entries, _ctx, mut conn) = fixture();
    let log_ix = log_ix_of(&conn, &log);
    insert_opaque(&conn, log_ix, 5, [0x55; 32], 0);
    insert_opaque(&conn, log_ix, 6, [0x66; 32], 1);
    let branch = BranchRow {
        log_ix,
        log,
        pubkey: [0; 32],
        frozen_at: 2,
        tip_counter: 9,
        tip_hash: [9; 32],
        next_from: 6,
        search_span: 1,
    };
    let tx = conn.transaction().unwrap();
    let on_main = signed_entry(log, 6, [0x55; 32], r#"{"id":"x6"}"#);
    assert!(chains_on_known(&tx, &branch, &on_main).unwrap());
    // An opaque fork-branch row is a known predecessor too.
    let on_branch = signed_entry(log, 7, [0x66; 32], r#"{"id":"x7"}"#);
    assert!(chains_on_known(&tx, &branch, &on_branch).unwrap());
    let nowhere = signed_entry(log, 6, [0x77; 32], r#"{"id":"x6"}"#);
    assert!(!chains_on_known(&tx, &branch, &nowhere).unwrap());
}

// Kills: an I2 check blind to opaque fork-branch rows, and one that reads
// opaque chain positions (branch = 0) as branch rows.
#[test]
fn i2_checks_opaque_branch_rows() {
    let (_sa, _sb, log, _entries, _ctx, conn) = fixture();
    let log_ix = log_ix_of(&conn, &log);
    conn.execute(
        "UPDATE sync_logs SET state='frozen',frozen_at_counter=2 WHERE log_ix=?1",
        [log_ix],
    )
    .unwrap();
    // Above the freeze, and a chain position: neither is an I2 violation.
    insert_opaque(&conn, log_ix, 3, [0x33; 32], 1);
    insert_opaque(&conn, log_ix, 5, [0x55; 32], 0);
    assert_eq!(check_invariants(&conn).unwrap(), Vec::new());
    // At the freeze: a violation.
    insert_opaque(&conn, log_ix, 2, [0x22; 32], 1);
    let found = check_invariants(&conn).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].invariant, Invariant::I2);
    assert_eq!(found[0].counter, 2);
}
