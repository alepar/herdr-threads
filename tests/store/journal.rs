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
            path: crate::test_support::scratch::temp_path(format!(
                "ht-journal-{}.db",
                uuid::Uuid::new_v4()
            )),
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

// ---- ingest, serve, vector, store_forked, freeze ---------------------------

const SEED_B: [u8; 32] = [2; 32];
const SEED_C: [u8; 32] = [3; 32];

fn identity_b() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::from_seed(SEED_B))
}

/// A clock a fixed number of ms ahead of [`FixedClock`].
struct AheadClock(i64);
impl Clock for AheadClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(FixedClock.utc_now().0 + self.0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

/// A store of node B (seed 2), opened with the given incarnation seed.
fn opened_b(store: &Store, incarnation_seed: u64) -> (StoreContext, Connection) {
    let ctx = StoreContext::new(store.path.clone(), Arc::new(FixedClock))
        .with_node_seed(crate::test_support::node_seed_for(identity_b().uuid()));
    ctx.seed_incarnations(incarnation_seed);
    let mut conn = ctx.open_writer().unwrap();
    open(&ctx, &mut conn, identity_b(), "b-host", registry()).unwrap();
    (ctx, conn)
}

/// Node A (seed 1) authors `puts` entries after its genesis; returns A's
/// log and entries `1..=puts+1`.
fn chain_of_a(store: &Store, puts: usize) -> (StoreContext, Connection, LogKey, Vec<Envelope>) {
    let (ctx, mut conn, report) = opened(store);
    for i in 0..puts {
        author_put(&ctx, &mut conn, &put(&format!("a{i}"))).unwrap();
    }
    let entries = (1..=puts as i64 + 1)
        .map(|counter| load_envelope(&conn, counter))
        .collect();
    (ctx, conn, report.self_log, entries)
}

fn ingest_into(
    ctx: &StoreContext,
    conn: &mut Connection,
    log: &LogKey,
    entries: &[Envelope],
) -> Result<IngestOutcome, ApiError> {
    ingest(ctx, conn, log, entries.to_vec(), &budget())
}

fn head(counter: u64, hash: [u8; 32]) -> LogHead {
    LogHead {
        counter,
        head_hash: hash,
        state: LogState::Active,
        frozen_at: None,
    }
}

/// A validly signed entry of `log` by `identity`, built like an authored one.
fn signed_entry(
    identity: &NodeIdentity,
    log: LogKey,
    counter: u64,
    prev_hash: [u8; 32],
    payload: &str,
) -> Envelope {
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
    .seal(identity)
}

fn journal_rows(conn: &Connection) -> i64 {
    count(conn, "SELECT COUNT(*) FROM journal")
}

#[test]
fn ingest_happy_path_stored_pending() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 3);
    let (ctx, mut conn) = opened_b(&sb, 77);
    let own_rows = journal_rows(&conn);

    let outcome = ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Stored {
            stored: 4,
            duplicates: 0,
            head: head(4, entries[3].hash),
        }
    );
    assert_eq!(journal_rows(&conn), own_rows + 4);
    let (is_self, contiguous, head_hash, applied, pubkey, state): (
        i64,
        i64,
        Vec<u8>,
        i64,
        Vec<u8>,
        String,
    ) = conn
        .query_row(
            "SELECT is_self,contiguous_counter,head_hash,applied_through,pubkey,state FROM sync_logs \
             WHERE node_uuid=?1",
            [log.node.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!((is_self, contiguous, applied), (0, 4, 0));
    assert_eq!(head_hash, entries[3].hash.to_vec());
    assert_eq!(pubkey, identity().public_key().to_vec());
    assert_eq!(state, "active");
    let states: Vec<(i64, String, i64)> = conn
        .prepare(
            "SELECT counter,apply_state,ingested_at FROM journal j JOIN sync_logs l USING(log_ix) \
             WHERE l.node_uuid=?1 ORDER BY counter",
        )
        .unwrap()
        .query_map([log.node.to_string()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let at = FixedClock.utc_now().0;
    assert_eq!(
        states,
        (1..=4)
            .map(|c| (c, "pending".to_owned(), at))
            .collect::<Vec<_>>()
    );
    // What was stored serves back byte for byte.
    let served = serve(&conn, &log, 1, BatchLimits::default()).unwrap();
    assert_eq!(served, entries);
}

#[test]
fn ingest_duplicates_skipped() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 3);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries[..3]).unwrap();
    let rows = journal_rows(&conn);
    let outcome = ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Stored {
            stored: 1,
            duplicates: 3,
            head: head(4, entries[3].hash),
        }
    );
    assert_eq!(journal_rows(&conn), rows + 1);
    // A batch of only duplicates stores nothing and keeps the head.
    let again = ingest_into(&ctx, &mut conn, &log, &entries[1..3]).unwrap();
    assert_eq!(
        again,
        IngestOutcome::Stored {
            stored: 0,
            duplicates: 2,
            head: head(4, entries[3].hash),
        }
    );
    assert_eq!(journal_rows(&conn), rows + 1);
}

#[test]
fn ingest_gap_keeps_prefix() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 4);
    let (ctx, mut conn) = opened_b(&sb, 77);

    // No row yet and the batch does not start at 1: a gap, nothing created.
    let outcome = ingest_into(&ctx, &mut conn, &log, &entries[1..3]).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Gap {
            stored: 0,
            expected: 1
        }
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_logs"), 1);

    let batch = [entries[0].clone(), entries[1].clone(), entries[3].clone()];
    let outcome = ingest_into(&ctx, &mut conn, &log, &batch).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Gap {
            stored: 2,
            expected: 3
        }
    );
    let contiguous = count(
        &conn,
        &format!(
            "SELECT contiguous_counter FROM sync_logs WHERE node_uuid='{}'",
            log.node
        ),
    );
    assert_eq!(contiguous, 2, "the prefix is committed");
    assert_eq!(stored_hash(&conn, &log, 2).unwrap(), Some(entries[1].hash));
    assert_eq!(stored_hash(&conn, &log, 3).unwrap(), None);
}

#[test]
fn ingest_flipped_byte_corrupt_nothing_stored() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, mut entries) = chain_of_a(&sa, 3);
    let (ctx, mut conn) = opened_b(&sb, 77);
    let rows = journal_rows(&conn);
    let last = entries[2].payload.len() - 1;
    entries[2].payload[last] ^= 0x01;

    let outcome = ingest_into(&ctx, &mut conn, &log, &entries[..3]).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Corrupt {
            log,
            counter: 3,
            reason: EnvelopeError::HashMismatch,
        }
    );
    assert_eq!(
        journal_rows(&conn),
        rows,
        "even the good prefix rolled back"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_logs"), 1);
}

#[test]
fn ingest_foreign_key_bad_signature() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 2);
    let (ctx, mut conn) = opened_b(&sb, 77);
    let rows = journal_rows(&conn);
    let stranger = NodeIdentity::from_seed(SEED_C);

    let forged = signed_entry(&stranger, log, 2, entries[0].hash, r#"{"id":"x"}"#);
    let outcome = ingest_into(&ctx, &mut conn, &log, &[entries[0].clone(), forged]).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Corrupt {
            log,
            counter: 2,
            reason: EnvelopeError::BadSignature,
        }
    );
    assert_eq!(journal_rows(&conn), rows);

    // A genesis announcing A's key but signed by another key is no genesis.
    let mut genesis = entries[0].clone();
    let forged_genesis = UnsignedEnvelope {
        log,
        counter: 1,
        prev_hash: [0; 32],
        hlc: genesis.hlc,
        kind: genesis.kind.clone(),
        kind_version: 1,
        deps: Deps::empty(),
        creates: None,
        needs: Vec::new(),
        payload: std::mem::take(&mut genesis.payload),
    }
    .seal(&stranger);
    let outcome = ingest_into(&ctx, &mut conn, &log, &[forged_genesis]).unwrap();
    assert!(matches!(
        outcome,
        IngestOutcome::Corrupt {
            reason: EnvelopeError::BadSignature,
            ..
        }
    ));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_logs"), 1);
}

#[test]
fn ingest_conflict_is_fork_prefix_kept() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 4);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries[..3]).unwrap();

    // Same position, other content, signed by the log's own key.
    let rival = signed_entry(&identity(), log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let outcome = ingest_into(&ctx, &mut conn, &log, &[entries[3].clone(), rival.clone()]).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Fork {
            stored: 1,
            evidence: ForkEvidence::Conflict {
                held: Box::new(entries[2].clone()),
                conflicting: Box::new(rival),
            },
        }
    );
    assert_eq!(
        stored_hash(&conn, &log, 4).unwrap(),
        Some(entries[3].hash),
        "the prefix before the fork is kept"
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 0);

    // The next entry with the wrong prev_hash, validly signed, is a fork
    // against the held head.
    let branch = signed_entry(&identity(), log, 5, entries[1].hash, r#"{"id":"branch"}"#);
    let outcome = ingest_into(&ctx, &mut conn, &log, std::slice::from_ref(&branch)).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Fork {
            stored: 0,
            evidence: ForkEvidence::Conflict {
                held: Box::new(entries[3].clone()),
                conflicting: Box::new(branch),
            },
        }
    );
}

#[test]
fn ingest_own_active_log_is_ahead_of_self() {
    let sb = Store::new();
    let (ctx, mut conn) = opened_b(&sb, 77);
    let own = load_envelope(&conn, 1);
    let rows = journal_rows(&conn);
    let ahead = signed_entry(&identity_b(), own.log, 2, own.hash, r#"{"id":"ahead"}"#);
    let outcome = ingest_into(&ctx, &mut conn, &own.log, std::slice::from_ref(&ahead)).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Fork {
            stored: 0,
            evidence: ForkEvidence::AheadOfSelf {
                log: own.log,
                my_head: head(1, own.hash),
                entry: Box::new(ahead),
            },
        }
    );
    assert_eq!(journal_rows(&conn), rows);
}

#[test]
fn ingest_own_active_log_redelivered_is_duplicate() {
    let sb = Store::new();
    let (ctx, mut conn) = opened_b(&sb, 77);
    let own = load_envelope(&conn, 1);
    let rows = journal_rows(&conn);
    let outcome = ingest_into(&ctx, &mut conn, &own.log, std::slice::from_ref(&own)).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Stored {
            stored: 0,
            duplicates: 1,
            head: head(1, own.hash),
        }
    );
    assert_eq!(journal_rows(&conn), rows);
}

#[test]
fn ingest_own_active_log_forged_is_corrupt() {
    let sb = Store::new();
    let (ctx, mut conn) = opened_b(&sb, 77);
    let own = load_envelope(&conn, 1);
    let rows = journal_rows(&conn);
    let forged = signed_entry(&identity(), own.log, 2, own.hash, r#"{"id":"forged"}"#);
    let outcome = ingest_into(&ctx, &mut conn, &own.log, std::slice::from_ref(&forged)).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Corrupt {
            log: own.log,
            counter: 2,
            reason: EnvelopeError::BadSignature,
        }
    );
    assert_eq!(journal_rows(&conn), rows);
}

#[test]
fn ingest_frozen_log_is_frozen() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 3);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries[..2]).unwrap();
    let tx = conn.transaction().unwrap();
    freeze(&tx, &log, 2).unwrap();
    tx.commit().unwrap();

    let outcome = ingest_into(&ctx, &mut conn, &log, &entries[2..]).unwrap();
    assert_eq!(
        outcome,
        IngestOutcome::Frozen {
            stored: 0,
            frozen_at: 2
        }
    );
    // Entries up to the freeze point are still accepted (here: duplicates).
    let outcome = ingest_into(&ctx, &mut conn, &log, &entries[..2]).unwrap();
    assert!(matches!(
        outcome,
        IngestOutcome::Stored {
            stored: 0,
            duplicates: 2,
            ..
        }
    ));
}

#[test]
fn ingest_headroom_max_nothing_opened() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 1);
    let (ctx, mut conn) = opened_b(&sb, 77);
    let rows = journal_rows(&conn);
    ctx.set_ingest_headroom(u64::MAX);

    // A competing writer holds the database: opening BEGIN IMMEDIATE would
    // come back StoreBusy after the busy timeout, so Ok(Headroom) proves no
    // transaction was opened.
    let blocker = sb.read();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let outcome = ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    assert_eq!(outcome, IngestOutcome::Headroom);
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(journal_rows(&conn), rows);

    ctx.set_ingest_headroom(0);
    let outcome = ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    assert!(matches!(outcome, IngestOutcome::Stored { stored: 2, .. }));
}

#[test]
fn ingest_batch_bounds_are_corrupt_malformed() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 1);
    let (ctx, mut conn) = opened_b(&sb, 77);

    let too_many = vec![entries[0].clone(); 257];
    let outcome = ingest_into(&ctx, &mut conn, &log, &too_many).unwrap();
    assert!(matches!(
        outcome,
        IngestOutcome::Corrupt {
            reason: EnvelopeError::Malformed { field: "batch" },
            ..
        }
    ));

    let other = LogKey {
        node: log.node,
        incarnation: log.incarnation + 1,
    };
    let outcome = ingest_into(&ctx, &mut conn, &other, &entries).unwrap();
    assert!(matches!(
        outcome,
        IngestOutcome::Corrupt {
            log: reported,
            counter: 1,
            reason: EnvelopeError::Malformed { field: "log" },
        } if reported == other
    ));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sync_logs"), 1);
}

#[test]
fn ingest_before_open_is_unsupported() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 1);
    let ctx = StoreContext::new(sb.path.clone(), Arc::new(FixedClock));
    let mut conn = ctx.open_writer().unwrap();
    let error = ingest_into(&ctx, &mut conn, &log, &entries).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
}

#[test]
fn ingest_own_old_incarnation_is_ordinary() {
    let (s_old, sb) = (Store::new(), Store::new());
    // The same node (seed 2) under an earlier incarnation, as a restored
    // copy of its store would hold.
    let (old_ctx, mut old_conn) = opened_b(&s_old, 11);
    author_put(&old_ctx, &mut old_conn, &put("old")).unwrap();
    let old_log = LogKey {
        node: identity_b().uuid(),
        incarnation: XorShift::fixed(11).draw_incarnation(),
    };
    let old_entries: Vec<Envelope> = (1..=2).map(|c| load_envelope(&old_conn, c)).collect();
    assert_eq!(old_entries[0].log, old_log);

    let (ctx, mut conn) = opened_b(&sb, 22);
    let active: i64 = count(&conn, "SELECT active_log_ix FROM local_node");
    let outcome = ingest_into(&ctx, &mut conn, &old_log, &old_entries).unwrap();
    assert!(
        matches!(outcome, IngestOutcome::Stored { stored: 2, .. }),
        "{outcome:?}"
    );
    let (is_self, state): (i64, String) = conn
        .query_row(
            "SELECT is_self,state FROM sync_logs WHERE incarnation=?1 AND node_uuid=?2",
            params![old_log.incarnation, old_log.node.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((is_self, state.as_str()), (1, "active"));
    assert_eq!(
        count(&conn, "SELECT active_log_ix FROM local_node"),
        active,
        "the active self log is untouched"
    );
    let pending = count(
        &conn,
        "SELECT COUNT(*) FROM journal WHERE apply_state='pending'",
    );
    assert_eq!(pending, 2);
}

#[test]
fn ingest_skewed_sets_skew_leaves_hlc() {
    let (sa, sb) = (Store::new(), Store::new());
    // A's clock runs 2 minutes ahead of B's wall.
    let actx = StoreContext::new(sa.path.clone(), Arc::new(AheadClock(120_000)));
    actx.seed_incarnations(42);
    let mut aconn = actx.open_writer().unwrap();
    let report = open_with(&actx, &mut aconn).unwrap();
    author_put(&actx, &mut aconn, &put("late")).unwrap();
    let entries: Vec<Envelope> = (1..=2).map(|c| load_envelope(&aconn, c)).collect();

    let (ctx, mut conn) = opened_b(&sb, 77);
    let before = ctx.sync().unwrap().hlc.lock().unwrap().last();
    let outcome = ingest_into(&ctx, &mut conn, &report.self_log, &entries).unwrap();
    assert!(
        matches!(outcome, IngestOutcome::Stored { stored: 2, .. }),
        "{outcome:?}"
    );
    assert_eq!(
        ctx.sync().unwrap().hlc.lock().unwrap().last(),
        before,
        "a skewed stamp is stored but not folded into the clock"
    );
    let skew = count(
        &conn,
        &format!(
            "SELECT skew_ahead_ms FROM sync_logs WHERE node_uuid='{}'",
            report.self_log.node
        ),
    );
    assert_eq!(skew, 120_000);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM journal WHERE apply_state='pending'"
        ),
        2
    );
}

#[test]
fn ingest_unskewed_folds_into_the_clock() {
    let (sa, sb) = (Store::new(), Store::new());
    // A's clock is 30 s ahead: within the 60 s cap, so it is absorbed.
    let actx = StoreContext::new(sa.path.clone(), Arc::new(AheadClock(30_000)));
    actx.seed_incarnations(42);
    let mut aconn = actx.open_writer().unwrap();
    let report = open_with(&actx, &mut aconn).unwrap();
    let genesis = load_envelope(&aconn, 1);

    let (ctx, mut conn) = opened_b(&sb, 77);
    let before = ctx.sync().unwrap().hlc.lock().unwrap().last();
    ingest_into(
        &ctx,
        &mut conn,
        &report.self_log,
        std::slice::from_ref(&genesis),
    )
    .unwrap();
    let after = ctx.sync().unwrap().hlc.lock().unwrap().last();
    assert!(after > before);
    assert_eq!(after, genesis.hlc);
    assert_eq!(count(&conn, "SELECT MAX(skew_ahead_ms) FROM sync_logs"), 0);
}

#[test]
fn serve_limits_256_entries_256k_and_oversized() {
    let sa = Store::new();
    let (ctx, mut conn, log, entries) = chain_of_a(&sa, 299);
    assert_eq!(entries.len(), 300);
    let limits = BatchLimits::default();

    let page = serve(&conn, &log, 1, limits).unwrap();
    assert_eq!(page.len(), 256);
    assert_eq!(page.first().map(|e| e.counter), Some(1));
    assert_eq!(page.last().map(|e| e.counter), Some(256));
    assert_eq!(page, entries[..256]);
    let rest = serve(&conn, &log, 257, limits).unwrap();
    assert_eq!(rest, entries[256..]);
    assert_eq!(serve(&conn, &log, 300, limits).unwrap(), entries[299..]);
    assert!(serve(&conn, &log, 301, limits).unwrap().is_empty());
    let unknown = LogKey {
        node: log.node,
        incarnation: log.incarnation + 1,
    };
    assert!(serve(&conn, &unknown, 1, limits).unwrap().is_empty());

    // Byte bound: stops before the entry that would pass it.
    let two = entries[0].canonical_bytes().len() + entries[1].canonical_bytes().len();
    let page = serve(
        &conn,
        &log,
        1,
        BatchLimits {
            entries: 256,
            bytes: two,
        },
    )
    .unwrap();
    assert_eq!(page.len(), 2);

    // An oversized entry is served alone rather than never.
    let mut big = put("big");
    big.blob = "x".repeat(300_000);
    author_put(&ctx, &mut conn, &big).unwrap();
    author_put(&ctx, &mut conn, &put("after")).unwrap();
    let alone = serve(&conn, &log, 301, limits).unwrap();
    assert_eq!(alone.len(), 1);
    assert!(alone[0].canonical_bytes().len() > limits.bytes);
    assert_eq!(alone[0].counter, 301);
    let next = serve(&conn, &log, 302, limits).unwrap();
    assert_eq!(next.first().map(|e| e.counter), Some(302));
}

#[test]
fn vector_covers_every_log() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 2);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    insert_foreign(&conn, 1, "test.put", "{}", "", None);
    let own = load_envelope(&conn, 1);

    let tx = conn.transaction().unwrap();
    freeze(&tx, &log, 2).unwrap();
    tx.commit().unwrap();

    let vector = vector(&conn).unwrap();
    assert_eq!(vector.0.len(), 3);
    assert_eq!(
        vector.0[&log],
        LogHead {
            counter: 3,
            head_hash: entries[2].hash,
            state: LogState::Frozen,
            frozen_at: Some(2),
        }
    );
    assert_eq!(vector.0[&own.log], head(1, own.hash));
    let peer = LogKey {
        node: PEER.parse().unwrap(),
        incarnation: 1,
    };
    assert_eq!(vector.0[&peer].counter, 1);
    assert_eq!(stored_hash(&conn, &log, 2).unwrap(), Some(entries[1].hash));
    assert_eq!(stored_hash(&conn, &log, 9).unwrap(), None);
    assert_eq!(stored_hash(&conn, &log, 0).unwrap(), None);
}

#[test]
fn store_forked_ignores_exact_duplicates() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 2);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    let rival = signed_entry(&identity(), log, 3, entries[1].hash, r#"{"id":"rival"}"#);
    let other = signed_entry(&identity(), log, 3, entries[1].hash, r#"{"id":"other"}"#);
    let source = identity_b().uuid();

    let tx = conn.transaction().unwrap();
    assert_eq!(
        store_forked(&tx, &log, &[rival.clone(), rival.clone()], Some(source)).unwrap(),
        1
    );
    assert_eq!(
        store_forked(&tx, &log, std::slice::from_ref(&rival), None).unwrap(),
        0
    );
    assert_eq!(store_forked(&tx, &log, &[other], None).unwrap(), 1);
    let stray = LogKey {
        node: log.node,
        incarnation: log.incarnation + 1,
    };
    assert_eq!(
        store_forked(&tx, &stray, std::slice::from_ref(&rival), None)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        store_forked(
            &tx,
            &log,
            &[Envelope {
                log: stray,
                ..rival
            }],
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    tx.commit().unwrap();

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM journal_forked"), 2);
    let kept: String = conn
        .query_row(
            "SELECT source_node FROM journal_forked ORDER BY forked_ix LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        kept,
        source.to_string(),
        "the first insert's source is kept"
    );
    assert_eq!(
        journal_rows(&conn),
        1 + 3,
        "the main chain is untouched: own genesis plus A's three"
    );
}

#[test]
fn freeze_frozen_lowered_unchanged_and_refuses_active_self() {
    let (sa, sb) = (Store::new(), Store::new());
    let (_actx, _aconn, log, entries) = chain_of_a(&sa, 4);
    let (ctx, mut conn) = opened_b(&sb, 77);
    ingest_into(&ctx, &mut conn, &log, &entries).unwrap();
    let frozen_at = |conn: &Connection| -> (String, Option<i64>) {
        conn.query_row(
            "SELECT state,frozen_at_counter FROM sync_logs WHERE node_uuid=?1",
            [log.node.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };

    let tx = conn.transaction().unwrap();
    assert_eq!(freeze(&tx, &log, 4).unwrap(), FreezeOutcome::Frozen);
    assert_eq!(frozen_at(&tx), ("frozen".into(), Some(4)));
    assert_eq!(freeze(&tx, &log, 2).unwrap(), FreezeOutcome::Lowered);
    assert_eq!(frozen_at(&tx), ("frozen".into(), Some(2)));
    assert_eq!(freeze(&tx, &log, 5).unwrap(), FreezeOutcome::Unchanged);
    assert_eq!(freeze(&tx, &log, 2).unwrap(), FreezeOutcome::Unchanged);
    assert_eq!(frozen_at(&tx), ("frozen".into(), Some(2)));

    let own = tx
        .query_row(
            "SELECT node_uuid,incarnation FROM sync_logs WHERE is_self=1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)),
        )
        .unwrap();
    let own = LogKey {
        node: own.0.parse().unwrap(),
        incarnation: own.1,
    };
    let refused = freeze(&tx, &own, 1).unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    assert_eq!(refused.detail, "cannot freeze the active self log");
    let unknown = LogKey {
        node: log.node,
        incarnation: log.incarnation + 1,
    };
    assert_eq!(
        freeze(&tx, &unknown, 1).unwrap_err().code,
        ErrorCode::NotFound
    );
    tx.commit().unwrap();
}

fn sqlite_store(ctx: StoreContext) -> crate::store::SqliteStore {
    crate::store::SqliteStore::new(ctx, "x", crate::store::StoreSettings::default()).unwrap()
}

fn genesis_facts(path: &std::path::Path) -> (String, i64, String, String, i64) {
    let conn = Connection::open(path).unwrap();
    let id: String = conn
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    let (ix, state, kind, counter): (i64, String, String, i64) = conn
        .query_row(
            "SELECT n.active_log_ix, j.apply_state, j.kind, j.counter FROM local_node n \
             JOIN journal j ON j.log_ix=n.active_log_ix",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    (id, ix, state, kind, counter)
}

// Kills: SqliteStore::new leaving the journal unopened (no genesis row, no
// active log) or opening it under a key other than the store's node.
#[test]
fn default_test_store_has_applied_genesis() {
    let store = Store::new();
    let sqlite = sqlite_store(store.ctx());
    assert!(sqlite.context().journal_open());
    let (id, ix, state, kind, counter) = genesis_facts(&store.path);
    assert_eq!(
        (state.as_str(), kind.as_str(), counter),
        ("applied", "node.genesis", 1)
    );
    assert!(ix > 0);
    assert_eq!(id, crate::test_support::test_node_seed().uuid.to_string());
}

// Kills: the seeded variant reusing the default identity or self log.
#[test]
fn seeded_variant_gives_distinct_node_and_self_log() {
    let default_store = Store::new();
    let seeded_store = Store::new();
    let _a = sqlite_store(default_store.ctx());
    let _b = sqlite_store(seeded_store.ctx().with_journal_seed([2; 32]));
    let (id_a, ..) = genesis_facts(&default_store.path);
    let (id_b, _, state, kind, counter) = genesis_facts(&seeded_store.path);
    assert_ne!(id_a, id_b);
    assert_eq!(
        id_b,
        crate::node::NodeIdentity::from_seed([2; 32])
            .uuid()
            .to_string()
    );
    assert_eq!(
        (state.as_str(), kind.as_str(), counter),
        ("applied", "node.genesis", 1)
    );
    let log_node = |p: &std::path::Path| -> String {
        Connection::open(p)
            .unwrap()
            .query_row(
                "SELECT l.node_uuid || l.incarnation FROM sync_logs l WHERE l.is_self=1",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_ne!(log_node(&default_store.path), log_node(&seeded_store.path));
}

// ---- reissue ---------------------------------------------------------------

fn peer_log() -> LogKey {
    LogKey {
        node: PEER.parse().unwrap(),
        incarnation: 1,
    }
}

/// A source envelope as a frozen own log's tail would hold it; `reissue`
/// reads only its fields, so hash and signature are placeholders.
fn source_envelope(payload: &str, creates: Option<&str>, needs: &[&str], deps: Deps) -> Envelope {
    Envelope {
        log: LogKey {
            node: identity().uuid(),
            incarnation: 777,
        },
        counter: 9,
        prev_hash: [7; 32],
        hlc: HlcStamp::from_u64(5),
        kind: "test.put".into(),
        kind_version: 1,
        deps,
        creates: creates.map(|k| EntityKey::new(k).unwrap()),
        needs: needs.iter().map(|k| EntityKey::new(*k).unwrap()).collect(),
        payload: payload.as_bytes().to_vec(),
        hash: [8; 32],
        signature: [9; 64],
    }
}

fn do_reissue(
    ctx: &StoreContext,
    conn: &mut Connection,
    source: &Envelope,
) -> Result<Authored, ApiError> {
    ctx.execute_decision(conn, |_| Ok(()), |tx, d, ()| reissue(tx, ctx, d, source))
}

// Kills: a reissue that re-encodes the payload, ticks the HLC, drops
// creates/needs or leaves the copy unsigned or unapplied.
#[test]
fn reissue_copies_fields_byte_for_byte() {
    let store = Store::new();
    let (ctx, mut conn, report) = opened(&store);
    author_put(&ctx, &mut conn, &put("c")).unwrap();
    // Odd spacing: a re-encode through serde would change these bytes.
    let source = source_envelope(
        r#"{ "id":"s1",  "need":"e:c" }"#,
        Some("e:s1"),
        &["e:c"],
        Deps::empty(),
    );
    let authored = do_reissue(&ctx, &mut conn, &source).unwrap();
    assert_eq!(authored.counter, 3);
    assert_eq!(authored.hlc, source.hlc);
    let copy = load_envelope(&conn, 3);
    assert_eq!(copy.log, report.self_log);
    assert_eq!(copy.kind, source.kind);
    assert_eq!(copy.kind_version, source.kind_version);
    assert_eq!(copy.hlc, source.hlc, "the HLC is not ticked");
    assert_eq!(copy.creates, source.creates);
    assert_eq!(copy.needs, source.needs);
    assert_eq!(copy.payload, source.payload);
    assert_eq!(copy.hash, authored.hash);
    copy.verify(&identity().public_key()).unwrap();
    copy.validate_shape().unwrap();
    let (state, applied_through, contiguous): (String, i64, i64) = conn
        .query_row(
            "SELECT j.apply_state,l.applied_through,l.contiguous_counter FROM journal j \
             JOIN sync_logs l USING(log_ix) WHERE j.ingest_seq=?1",
            [authored.ingest_seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "applied");
    assert_eq!(applied_through, contiguous);
}

// Kills: deps that are the owner's vector alone, the source's alone, a sum
// instead of a maximum, or that list the active log.
#[test]
fn reissue_deps_elementwise_max_without_active_log() {
    let store = Store::new();
    let (ctx, mut conn, report) = opened(&store);
    let seq = insert_foreign(&conn, 1, "test.put", r#"{"id":"p"}"#, "", Some("e:p"));
    apply_foreign(&ctx, &mut conn, seq, apply::ApplyMode::Head).unwrap();
    conn.execute(
        "UPDATE sync_logs SET contiguous_counter=3, applied_through=3 WHERE node_uuid=?1",
        [PEER],
    )
    .unwrap();
    let other = LogKey {
        node: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".parse().unwrap(),
        incarnation: 2,
    };
    let source_log = LogKey {
        node: identity().uuid(),
        incarnation: 777,
    };
    let mut entries = vec![(peer_log(), 2), (other, 7), (report.self_log, 99)];
    entries.sort();
    let deps = Deps::new(entries, &source_log).unwrap();
    let source = source_envelope(r#"{"id":"s2"}"#, Some("e:s2"), &[], deps);
    do_reissue(&ctx, &mut conn, &source).unwrap();
    let copy = load_envelope(&conn, 2);
    let mut expected = vec![(peer_log(), 3), (other, 7)];
    expected.sort();
    assert_eq!(copy.deps.entries(), expected.as_slice());
}

// Kills: a reissue that aborts on a handler rejection (as `author` does), one
// that leaves the handler's writes behind, or one that leaves the head
// unaccounted so the commit guard trips.
#[test]
fn reissue_rejection_recorded_not_aborted_commit_guard_holds() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let before = revision(&conn);
    let source = source_envelope(
        r#"{"id":"x","fail":"nope"}"#,
        Some("e:x"),
        &[],
        Deps::empty(),
    );
    // execute_decision runs the commit guard; Ok means it held.
    let authored = do_reissue(&ctx, &mut conn, &source).unwrap();
    let (state, error): (String, Option<String>) = conn
        .query_row(
            "SELECT apply_state,apply_error FROM journal WHERE ingest_seq=?1",
            [authored.ingest_seq],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "failed");
    assert!(error.unwrap().contains("nope"));
    assert_eq!(revision(&conn), before, "the handler's write rolled back");
    check_self_applied(&conn).unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT contiguous_counter FROM sync_logs l JOIN local_node n ON n.active_log_ix=l.log_ix"
        ),
        2
    );
}

// Kills: a reissue that applies past a missing `needs` creator, or aborts on
// the park.
#[test]
fn reissue_parked_dep_parks() {
    let store = Store::new();
    let (ctx, mut conn, _) = opened(&store);
    let source = source_envelope(
        r#"{"id":"d","need":"e:missing"}"#,
        Some("e:d"),
        &["e:missing"],
        Deps::empty(),
    );
    let authored = do_reissue(&ctx, &mut conn, &source).unwrap();
    let (state, on, error) = row_state(&conn, authored.ingest_seq);
    assert_eq!(state, "parked_dep");
    assert_eq!(on.as_deref(), Some("e:missing"));
    assert_eq!(error.as_deref(), Some("needs e:missing: creator absent"));
    check_self_applied(&conn).unwrap();
    assert_eq!(revision(&conn), 0, "the handler did not run");
}
