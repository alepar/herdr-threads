use super::*;

use std::path::PathBuf;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::protocol::results::CommandResult;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::connection::XorShift;
use crate::store::schema;
use crate::sync::envelope::Envelope;

const SEED: [u8; 32] = crate::test_support::TEST_NODE_SEED;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn decision() -> DecisionInstant {
    DecisionInstant {
        utc: FixedClock.utc_now(),
        monotonic: FixedClock.monotonic_now(),
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// A store file removed with its WAL sidecars when the test ends.
struct Store {
    path: PathBuf,
}

impl Store {
    fn new() -> Self {
        Self {
            path: std::env::temp_dir().join(format!("ht-journal-{}.db", uuid::Uuid::new_v4())),
        }
    }

    fn ctx(&self) -> StoreContext {
        StoreContext::new(self.path.clone(), Arc::new(FixedClock))
    }

    fn read(&self) -> Connection {
        Connection::open(&self.path).unwrap()
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
    #[serde(default)]
    blob: String,
    #[serde(default)]
    need: Option<String>,
}

fn put(id: &str) -> Put {
    Put {
        id: id.into(),
        fail: None,
        blob: String::new(),
        need: None,
    }
}

/// Bumps a local counter, then rejects when `fail` is set, so a rollback of
/// the handler's own writes is observable.
struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(p: &Put) -> Vec<EntityKey> {
        p.need
            .iter()
            .filter_map(|n| EntityKey::new(n.clone()).ok())
            .collect()
    }
    fn apply(
        tx: &Transaction<'_>,
        _: &mut crate::sync::facts::ApplyContext<'_>,
        p: Put,
    ) -> Result<(), crate::sync::facts::FactError> {
        use crate::sync::facts::FactError;
        tx.execute(
            "UPDATE local_node SET send_eligibility_revision=send_eligibility_revision+1",
            [],
        )
        .map_err(|e| FactError::from_api(store_error(e)))?;
        match p.fail {
            Some(detail) => Err(FactError::Rejected(ApiError::new(
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

fn identity() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::from_seed(SEED))
}

fn open_with(ctx: &StoreContext, conn: &mut Connection) -> Result<OpenReport, ApiError> {
    open(ctx, conn, identity(), "test-host", registry())
}

/// A store opened with a fixed incarnation sequence.
fn opened(store: &Store) -> (StoreContext, Connection, OpenReport) {
    let ctx = store.ctx();
    ctx.seed_incarnations(42);
    let mut conn = ctx.open_writer().unwrap();
    let report = open_with(&ctx, &mut conn).unwrap();
    (ctx, conn, report)
}

fn author_put(ctx: &StoreContext, conn: &mut Connection, p: &Put) -> Result<Authored, ApiError> {
    ctx.execute_decision(
        conn,
        |_| Ok(()),
        |tx, d, ()| author::<PutKind>(tx, ctx, d, p),
    )
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn revision(conn: &Connection) -> i64 {
    count(conn, "SELECT send_eligibility_revision FROM local_node")
}

fn load_envelope(conn: &Connection, counter: i64) -> Envelope {
    conn.query_row(
        "SELECT l.node_uuid,l.incarnation,j.counter,j.prev_hash,j.hlc,j.kind,j.kind_version,j.deps, \
         j.creates,j.needs,j.payload,j.hash,j.signature FROM journal j JOIN sync_logs l USING(log_ix) \
         JOIN local_node n ON n.active_log_ix=l.log_ix WHERE j.counter=?1",
        [counter],
        |r| {
            let log = LogKey {
                node: r.get::<_, String>(0)?.parse().unwrap(),
                incarnation: r.get(1)?,
            };
            let needs: String = r.get(9)?;
            Ok(Envelope {
                log,
                counter: r.get::<_, i64>(2)? as u64,
                prev_hash: r.get::<_, Vec<u8>>(3)?.try_into().unwrap(),
                hlc: HlcStamp::from_u64(r.get::<_, i64>(4)? as u64),
                kind: r.get(5)?,
                kind_version: r.get(6)?,
                deps: Deps::decode(&r.get::<_, Vec<u8>>(7)?, &log).unwrap(),
                creates: r
                    .get::<_, Option<String>>(8)?
                    .map(|k| EntityKey::new(k).unwrap()),
                needs: needs
                    .split('\n')
                    .filter(|k| !k.is_empty())
                    .map(|k| EntityKey::new(k).unwrap())
                    .collect(),
                payload: r.get(10)?,
                hash: r.get::<_, Vec<u8>>(11)?.try_into().unwrap(),
                signature: r.get::<_, Vec<u8>>(12)?.try_into().unwrap(),
            })
        },
    )
    .unwrap()
}

#[test]
fn open_bootstraps_genesis_with_random_incarnation() {
    let store = Store::new();
    let (ctx, _conn, report) = opened(&store);
    let expected = XorShift::fixed(42).draw_incarnation();
    assert_ne!(expected, 1, "the fixed test sequence must not draw 1");
    assert!(report.bootstrapped);
    assert_eq!(report.counter, 1);
    assert_eq!(report.self_log.incarnation, expected);
    assert_eq!(report.self_log.node, identity().uuid());
    assert_eq!(
        (report.parked_unknown, report.parked_dep, report.failed),
        (0, 0, 0)
    );
    assert!(ctx.journal_open());

    let db = store.read();
    let (state, applied_seq, kind, hlc, deps): (String, Option<i64>, String, i64, Vec<u8>) = db
        .query_row(
            "SELECT apply_state,applied_seq,kind,hlc,deps FROM journal WHERE counter=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(state, "applied");
    assert!(applied_seq.is_some());
    assert_eq!(kind, "node.genesis");
    assert_eq!(deps, Deps::empty().encode());
    let (ix, through, contiguous, is_self): (i64, i64, i64, i64) = db
        .query_row(
            "SELECT log_ix,applied_through,contiguous_counter,is_self FROM sync_logs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!((through, contiguous, is_self), (1, 1, 1));
    assert_eq!(
        count(&db, "SELECT active_log_ix FROM local_node"),
        ix,
        "active_log_ix names the bootstrapped log"
    );
    let (tag_hlc, label): (Option<i64>, Option<String>) = db
        .query_row(
            "SELECT tag_hlc,host_label FROM node_directory WHERE node_uuid=(SELECT id FROM local_node)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tag_hlc, Some(hlc), "genesis filled the directory row");
    assert_eq!(label.as_deref(), Some("test-host"));

    let genesis = load_envelope(&db, 1);
    let pubkey = genesis.genesis_pubkey().unwrap();
    assert_eq!(pubkey, identity().public_key());
    genesis.verify(&pubkey).unwrap();
    genesis.validate_shape().unwrap();
    let payload: GenesisPayload = serde_json::from_slice(&genesis.payload).unwrap();
    assert_eq!(payload.incarnation, expected);
    assert_eq!(payload.predecessor, None);
    assert_eq!(payload.tag, identity().tag().to_string());
}

#[test]
fn second_open_is_noop() {
    let store = Store::new();
    let (ctx, mut conn, first) = opened(&store);
    let again = open_with(&ctx, &mut conn).unwrap();
    assert!(!again.bootstrapped);
    assert_eq!(again.self_log, first.self_log);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_logs"), 1);

    // A new context over the same store (a restart) finds the log, writes
    // nothing and keeps the incarnation.
    let restarted = store.ctx();
    let mut conn2 = restarted.open_writer().unwrap();
    let report = open_with(&restarted, &mut conn2).unwrap();
    assert!(!report.bootstrapped);
    assert_eq!(report.self_log, first.self_log);
    assert_eq!(count(&conn2, "SELECT COUNT(*) FROM journal"), 1);
}

#[test]
fn unseeded_draws_differ_from_copied_state() {
    let unseeded = XorShift::from_os();
    let mut differing = 0;
    for _ in 0..4 {
        let (mut a, mut b) = (unseeded.clone(), unseeded.clone());
        let (x, y) = (a.draw_incarnation(), b.draw_incarnation());
        assert!(x >= 1 && y >= 1);
        differing += usize::from(x != y);
    }
    assert!(differing >= 3, "re-seeding per draw: {differing}/4 differ");

    // The contrast: a fixed sequence repeats from a copy.
    let fixed = XorShift::fixed(9);
    let (mut a, mut b) = (fixed.clone(), fixed);
    assert_eq!(a.draw_incarnation(), b.draw_incarnation());
}

fn tampered_open_error(sql: &str) -> ApiError {
    let store = Store::new();
    let (_ctx, conn, _) = opened(&store);
    conn.execute(sql, []).unwrap();
    let restarted = store.ctx();
    let mut conn2 = restarted.open_writer().unwrap();
    open_with(&restarted, &mut conn2).unwrap_err()
}

#[test]
fn tampered_contiguous_counter_refused() {
    let error = tampered_open_error("UPDATE sync_logs SET contiguous_counter=2");
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert!(
        error.detail.starts_with("journal invariant: "),
        "{}",
        error.detail
    );
    assert!(
        error
            .detail
            .contains("contiguous_counter 2 != stored max 1"),
        "{}",
        error.detail
    );
}

#[test]
fn tampered_head_hash_refused() {
    let error = tampered_open_error("UPDATE sync_logs SET head_hash=zeroblob(32)");
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert!(
        error.detail.starts_with("journal invariant: "),
        "{}",
        error.detail
    );
    assert!(error.detail.contains("head_hash"), "{}", error.detail);
}

#[test]
fn frozen_active_self_log_refused_i1() {
    let error = tampered_open_error("UPDATE sync_logs SET state='frozen', frozen_at_counter=1");
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert_eq!(error.detail, "journal invariant: active self log frozen");
}

#[test]
fn genesis_predecessor_round_trips() {
    for (predecessor, text) in [
        (None, "\"predecessor\":null"),
        (
            Some(Predecessor {
                incarnation: 3,
                frozen_at: None,
            }),
            "\"predecessor\":{\"incarnation\":3,\"frozen_at\":null}",
        ),
        (
            Some(Predecessor {
                incarnation: 3,
                frozen_at: Some(9),
            }),
            "\"predecessor\":{\"incarnation\":3,\"frozen_at\":9}",
        ),
    ] {
        let payload = GenesisPayload {
            pubkey: "00".repeat(32),
            tag: "abc".into(),
            tag_k: 0,
            incarnation: 7,
            predecessor,
            host_label: "h".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(text), "{json}");
        assert_eq!(
            serde_json::from_str::<GenesisPayload>(&json).unwrap(),
            payload
        );
    }

    // Through the store: a rotation records each predecessor shape.
    let store = Store::new();
    let (ctx, mut conn, first) = opened(&store);
    for (incarnation, predecessor) in [
        (
            77,
            Predecessor {
                incarnation: first.self_log.incarnation,
                frozen_at: Some(9),
            },
        ),
        (
            78,
            Predecessor {
                incarnation: 77,
                frozen_at: None,
            },
        ),
    ] {
        let tx = conn.transaction().unwrap();
        begin_incarnation(&tx, &ctx, decision(), incarnation, Some(predecessor), "h").unwrap();
        tx.commit().unwrap();
        let stored: Vec<u8> = conn
            .query_row(
                "SELECT j.payload FROM journal j JOIN local_node n ON n.active_log_ix=j.log_ix WHERE j.counter=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let payload: GenesisPayload = serde_json::from_slice(&stored).unwrap();
        assert_eq!(payload.incarnation, incarnation);
        assert_eq!(payload.predecessor, Some(predecessor));
    }
    let tx = conn.transaction().unwrap();
    let error = begin_incarnation(&tx, &ctx, decision(), 77, None, "h").unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
}

#[test]
fn author_chains_prev_hash_and_counters() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let mut hlcs = vec![load_envelope(&conn, 1).hlc];
    for (i, id) in ["a", "b", "c"].into_iter().enumerate() {
        let authored = author_put(&ctx, &mut conn, &put(id)).unwrap();
        assert_eq!(authored.counter, i as u64 + 2, "gapless counters");
        hlcs.push(authored.hlc);
    }
    assert!(
        hlcs.windows(2).all(|w| w[0] < w[1]),
        "HLC strictly rises: {hlcs:?}"
    );
    for counter in 2..=4 {
        let entry = load_envelope(&conn, counter);
        let previous = load_envelope(&conn, counter - 1);
        assert_eq!(entry.prev_hash, previous.hash, "counter {counter} chains");
        entry.verify(&identity().public_key()).unwrap();
        entry.validate_shape().unwrap();
        assert_eq!(entry.kind, "test.put");
        assert_eq!(
            entry.creates.as_ref().map(EntityKey::as_str),
            Some(["e:a", "e:b", "e:c"][counter as usize - 2])
        );
    }
    let (contiguous, through, head): (i64, i64, Vec<u8>) = conn
        .query_row(
            "SELECT contiguous_counter,applied_through,head_hash FROM sync_logs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((contiguous, through), (4, 4));
    assert_eq!(head, load_envelope(&conn, 4).hash);
    assert_eq!(revision(&conn), 3, "each handler ran once");
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM journal WHERE apply_state='applied' AND applied_seq IS NOT NULL"
        ),
        4
    );
}

/// Inserts a log of another node (or an older own incarnation) with
/// `applied` entries counted.
fn insert_log(conn: &Connection, node: &str, incarnation: i64, is_self: i64, applied: i64) {
    conn.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,applied_through,first_seen_at) \
         VALUES (?1,?2,?3,zeroblob(32),zeroblob(32),5,zeroblob(32),?4,0)",
        params![node, incarnation, is_self, applied],
    )
    .unwrap();
}

#[test]
fn author_deps_are_applied_vector_minus_self() {
    let store = Store::new();
    let (ctx, mut conn, report) = opened(&store);
    let foreign = "ffffffffffffffffffffffffffffffff";
    let silent = "11111111111111111111111111111111";
    insert_log(&conn, foreign, 3, 0, 4);
    insert_log(&conn, silent, 1, 0, 0);
    insert_log(&conn, &identity().uuid().to_string(), 5, 1, 2);
    author_put(&ctx, &mut conn, &put("a")).unwrap();
    let entry = load_envelope(&conn, 2);
    let mut expected = vec![
        (
            LogKey {
                node: identity().uuid(),
                incarnation: 5,
            },
            2,
        ),
        (
            LogKey {
                node: foreign.parse().unwrap(),
                incarnation: 3,
            },
            4,
        ),
    ];
    expected.sort();
    assert_eq!(entry.deps.entries(), expected.as_slice());
    assert!(
        entry
            .deps
            .entries()
            .iter()
            .all(|(k, _)| *k != report.self_log),
        "the entry's own log is never listed"
    );
    assert_eq!(
        entry.deps.encode(),
        conn.query_row("SELECT deps FROM journal WHERE counter=2 AND log_ix=(SELECT active_log_ix FROM local_node)", [], |r| r.get::<_, Vec<u8>>(0)).unwrap(),
        "journal.deps stores the canonical bytes"
    );
}

#[test]
fn oversized_payload_rejected_nothing_stored() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let overhead = serde_json::to_vec(&put("a")).unwrap().len();
    let mut exact = put("a");
    exact.blob = "x".repeat(PAYLOAD_MAX - overhead);
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), PAYLOAD_MAX);
    let mut over = put("b");
    over.blob = "x".repeat(PAYLOAD_MAX - overhead + 1);

    let error = author_put(&ctx, &mut conn, &over).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.detail, "fact payload exceeds 512 KiB");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM journal"),
        1,
        "nothing stored"
    );
    assert_eq!(count(&conn, "SELECT contiguous_counter FROM sync_logs"), 1);

    let authored = author_put(&ctx, &mut conn, &exact).unwrap();
    assert_eq!(
        authored.counter, 2,
        "a payload of exactly 512 KiB is accepted"
    );
}

#[test]
fn handler_rejection_rolls_back_author() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    author_put(&ctx, &mut conn, &put("keep")).unwrap();
    let (rows, revision_before) = (
        count(&conn, "SELECT COUNT(*) FROM journal"),
        revision(&conn),
    );
    let mut bad = put("bad");
    bad.fail = Some("no such thing".into());

    let error = ctx
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| {
                let error = author::<PutKind>(tx, &ctx, d, &bad).unwrap_err();
                // The savepoint undid the handler's own write at once.
                assert_eq!(
                    tx.query_row(
                        "SELECT send_eligibility_revision FROM local_node",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    revision_before
                );
                Err::<(), _>(error)
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(error.detail, "no such thing");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM journal"),
        rows,
        "no journal row"
    );
    assert_eq!(
        count(&conn, "SELECT contiguous_counter FROM sync_logs"),
        rows
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM journal WHERE apply_state='failed'"
        ),
        0
    );
    assert_eq!(revision(&conn), revision_before);
    // The next author reuses the counter the rejected one never took.
    assert_eq!(
        author_put(&ctx, &mut conn, &put("next")).unwrap().counter,
        rows as u64 + 1
    );
}

#[test]
fn unmet_need_is_a_conflict_not_a_recorded_park() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let mut dependent = put("d");
    dependent.need = Some("e:missing".into());
    let error = author_put(&ctx, &mut conn, &dependent).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        error.detail,
        "fact test.put not applicable: needs e:missing: creator absent"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal"), 1);

    author_put(&ctx, &mut conn, &put("missing")).unwrap();
    author_put(&ctx, &mut conn, &dependent).unwrap();
}

#[test]
fn author_before_open_is_unsupported() {
    let store = Store::new();
    let ctx = store.ctx();
    let mut conn = ctx.open_writer().unwrap();
    let error = author_put(&ctx, &mut conn, &put("a")).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(error.detail, "journal not open");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal"), 0);
}

#[test]
fn idempotent_replay_leaves_one_entry() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let run = |conn: &mut Connection| {
        schema::execute_budgeted_idempotent_transaction_with_constraints(
            &ctx,
            conn,
            &budget(),
            None,
            "scope",
            "key-1",
            [7u8; 32],
            |_| Ok(()),
            |_| Ok(()),
            |tx, d| {
                author::<PutKind>(tx, &ctx, d, &put("once"))?;
                Ok(CommandResult::HookParseFailureRecorded)
            },
            |_, result| Ok(result),
        )
    };
    run(&mut conn).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal"), 2);
    run(&mut conn).unwrap();
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM journal"),
        2,
        "the retry replayed from operations and authored nothing"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM operations"), 1);
    assert_eq!(revision(&conn), 1);
}

#[test]
fn unapplied_self_entry_trips_commit_guard() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let error = ctx
        .execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| {
                // An entry appended behind Journal's back: no apply.
                tx.execute(
                    "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,ingested_at,apply_state) \
                     SELECT log_ix,2,head_hash,zeroblob(32),1,'test.put',1,x'0000',x'7b7d',zeroblob(64),?1,'pending' \
                     FROM sync_logs",
                    [d.utc.0],
                )
                .unwrap();
                tx.execute("UPDATE sync_logs SET contiguous_counter=2", []).unwrap();
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert_eq!(error.detail, "self journal entry left unapplied");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM journal"),
        1,
        "rolled back"
    );
    assert_eq!(count(&conn, "SELECT contiguous_counter FROM sync_logs"), 1);

    // The guard is quiet for a properly authored entry, and for the other base
    // writer too.
    author_put(&ctx, &mut conn, &put("fine")).unwrap();
    let budgeted = ctx
        .execute_budgeted_decision(
            &mut conn,
            &budget(),
            |_| Ok(()),
            |tx, d, ()| author::<PutKind>(tx, &ctx, d, &put("fine2")),
        )
        .unwrap();
    assert_eq!(budgeted.counter, 3);
}

// ---- apply_one on entries that arrived from elsewhere ----------------------

const PEER: &str = "ffffffffffffffffffffffffffffffff";

/// Inserts a pending foreign entry at `counter` of a one-log peer and returns
/// its `ingest_seq`.
fn insert_foreign(
    conn: &Connection,
    counter: i64,
    kind: &str,
    payload: &str,
    needs: &str,
    creates: Option<&str>,
) -> i64 {
    conn.execute(
        "INSERT OR IGNORE INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES (?1,1,0,zeroblob(32),zeroblob(32),0,zeroblob(32),0)",
        [PEER],
    )
    .unwrap();
    conn.execute(
        "UPDATE sync_logs SET contiguous_counter=?1 WHERE node_uuid=?2",
        params![counter, PEER],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,needs,payload,signature,ingested_at,apply_state) \
         SELECT log_ix,?1,zeroblob(32),zeroblob(32),5,?2,1,x'0000',?3,?4,?5,zeroblob(64),0,'pending' \
         FROM sync_logs WHERE node_uuid=?6",
        params![counter, kind, creates, needs, payload.as_bytes(), PEER],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn apply_foreign(
    ctx: &StoreContext,
    conn: &mut Connection,
    ingest_seq: i64,
    mode: apply::ApplyMode,
) -> Result<apply::ApplyOutcome, ApiError> {
    ctx.execute_decision(
        conn,
        |_| Ok(()),
        |tx, d, ()| apply::apply_one(tx, ctx, d, ingest_seq, mode),
    )
}

fn row_state(conn: &Connection, ingest_seq: i64) -> (String, Option<String>, Option<String>) {
    conn.query_row(
        "SELECT apply_state,parked_on,apply_error FROM journal WHERE ingest_seq=?1",
        [ingest_seq],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

fn peer_applied_through(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT applied_through FROM sync_logs WHERE node_uuid=?1",
        [PEER],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn apply_one_parks_unknown_kind_and_counts_it_processed() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let seq = insert_foreign(&conn, 1, "test.later", "{}", "", None);
    let outcome = apply_foreign(&ctx, &mut conn, seq, apply::ApplyMode::Head).unwrap();
    assert_eq!(outcome, apply::ApplyOutcome::ParkedUnknown);
    let (state, on, error) = row_state(&conn, seq);
    assert_eq!((state.as_str(), on), ("parked_unknown", None));
    assert_eq!(error.as_deref(), Some("unknown kind test.later v1"));
    assert_eq!(peer_applied_through(&conn), 1, "a parked head is processed");
}

#[test]
fn apply_one_parks_dep_then_replay_applies_without_moving_applied_through() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let seq = insert_foreign(
        &conn,
        1,
        "test.put",
        r#"{"id":"d","need":"e:c"}"#,
        "e:c",
        Some("e:d"),
    );
    let outcome = apply_foreign(&ctx, &mut conn, seq, apply::ApplyMode::Head).unwrap();
    assert_eq!(outcome, apply::ApplyOutcome::ParkedDep { on: "e:c".into() });
    let (state, on, error) = row_state(&conn, seq);
    assert_eq!(state, "parked_dep");
    assert_eq!(on.as_deref(), Some("e:c"));
    assert_eq!(error.as_deref(), Some("needs e:c: creator absent"));
    assert_eq!(peer_applied_through(&conn), 1);
    assert_eq!(revision(&conn), 0, "the handler did not run");

    author_put(&ctx, &mut conn, &put("c")).unwrap();
    let outcome = apply_foreign(&ctx, &mut conn, seq, apply::ApplyMode::Replay).unwrap();
    assert_eq!(outcome, apply::ApplyOutcome::Applied);
    let (state, on, error) = row_state(&conn, seq);
    assert_eq!((state.as_str(), on, error), ("applied", None, None));
    assert_eq!(revision(&conn), 2);
    assert_eq!(peer_applied_through(&conn), 1);
}

#[test]
fn apply_one_records_failed_with_truncated_message_and_rolls_back_handler() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let detail = "é".repeat(400);
    let payload = format!(r#"{{"id":"x","fail":"{detail}"}}"#);
    let seq = insert_foreign(&conn, 1, "test.put", &payload, "", Some("e:x"));
    let outcome = apply_foreign(&ctx, &mut conn, seq, apply::ApplyMode::Head).unwrap();
    assert_eq!(outcome, apply::ApplyOutcome::Failed);
    let (state, on, error) = row_state(&conn, seq);
    assert_eq!((state.as_str(), on), ("failed", None));
    let error = error.unwrap();
    assert!(error.starts_with("not_found: é"), "{error}");
    assert!(
        error.len() <= 512 && error.len() > 500,
        "{} bytes",
        error.len()
    );
    assert_eq!(revision(&conn), 0, "the handler's write was rolled back");
    assert_eq!(peer_applied_through(&conn), 1, "a failed head is processed");
}

#[test]
fn genesis_apply_validates_incarnation_and_tag() {
    let conn = Connection::open_in_memory().unwrap();
    schema::initialize(
        &conn,
        || UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    let node = identity().uuid();
    let run = |origin_inc: u32, payload: &GenesisPayload, hlc: u64| {
        let mut cx = crate::sync::facts::ApplyContext::new(
            LogKey {
                node,
                incarnation: origin_inc,
            },
            1,
            1,
            HlcStamp::from_u64(hlc),
            decision(),
            1,
            node,
        );
        let tx = conn.unchecked_transaction().unwrap();
        let result = NodeGenesis::apply(&tx, &mut cx, payload.clone());
        tx.commit().unwrap();
        result
    };
    let good = GenesisPayload {
        pubkey: hex_encode(&identity().public_key()),
        tag: crate::node::tag_for(node, 0).to_string(),
        tag_k: 0,
        incarnation: 4,
        predecessor: None,
        host_label: "lab".into(),
    };
    let rejected = |r: Result<(), crate::sync::facts::FactError>| match r {
        Err(crate::sync::facts::FactError::Rejected(e)) => e,
        other => panic!("expected Rejected, got {other:?}"),
    };
    let wrong_inc = rejected(run(5, &good, 10));
    assert_eq!(wrong_inc.code, ErrorCode::InvalidRequest);
    let mut wrong_tag = good.clone();
    wrong_tag.tag_k = 1;
    let wrong_tag = rejected(run(4, &wrong_tag, 10));
    assert!(wrong_tag.detail.contains("tag"), "{}", wrong_tag.detail);

    run(4, &good, 10).unwrap();
    let row = crate::store::node_directory::get(&conn, node)
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.tag_hlc, row.host_label.as_deref()),
        (Some(10), Some("lab"))
    );
    // An older stamp does not overwrite the directory row.
    let mut older = good.clone();
    older.host_label = "stale".into();
    run(4, &older, 9).unwrap();
    let row = crate::store::node_directory::get(&conn, node)
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.tag_hlc, row.host_label.as_deref()),
        (Some(10), Some("lab"))
    );
}
