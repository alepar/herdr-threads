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
            path: std::env::temp_dir().join(format!("ht-incarnations-{}.db", uuid::Uuid::new_v4())),
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
        _: Put,
    ) -> Result<(), crate::sync::facts::FactError> {
        Ok(())
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
        creates: None,
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
