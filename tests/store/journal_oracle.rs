//! Self-tests of the journal replay oracle: honest kinds replay to an equal
//! projection and pass strict mode; a kind that lets a local position into
//! its payload, or whose rows order differently on a replaying node, is caught.

use super::*;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::protocol::ids::ThreadId;
use crate::store::connection::store_error;
use crate::store::thread_registers::{self, RegisterField};
use crate::sync::facts::{ApplyContext, FactError, FactKind};

// The journal-core tables are the only ones a store accepts, so the test
// kinds keep their rows in `thread_registers`: `topic` rows are replicated
// values, `archived` rows carry a local position in `value`.
const TOPICS: TableSpec = TableSpec {
    name: "thread_registers",
    filter: Some("field='topic'"),
    key_columns: &["thread_id"],
    replicated_columns: &["value", "hlc", "node_uuid"],
    origin_column: Some("node_uuid"),
    position_columns: &[],
    src_column: Some("src_ingest_seq"),
};

const ITEMS: TableSpec = TableSpec {
    name: "thread_registers",
    filter: Some("field='archived'"),
    key_columns: &["thread_id"],
    replicated_columns: &["hlc", "node_uuid"],
    origin_column: Some("node_uuid"),
    position_columns: &["value"],
    src_column: Some("src_ingest_seq"),
};

const ITEMS_LABEL: &str = "thread_registers where field='archived'";

fn specs() -> Vec<TableSpec> {
    vec![CORE_TABLES[0], TOPICS, ITEMS]
}

fn db(error: rusqlite::Error) -> FactError {
    FactError::from_api(store_error(error))
}

fn put_item(
    tx: &Transaction<'_>,
    cx: &ApplyContext<'_>,
    id: &str,
    pos: i64,
) -> Result<(), FactError> {
    thread_registers::upsert(
        tx,
        &ThreadId::parse(id).expect("a thread id"),
        RegisterField::Archived,
        Some(&pos.to_string()),
        cx.hlc,
        cx.origin.node,
        cx.ingest_seq,
    )
    .map(|_| ())
    .map_err(db)
}

fn key(text: String) -> Option<EntityKey> {
    EntityKey::new(text).ok()
}

#[derive(Serialize, Deserialize)]
struct Item {
    id: String,
    #[serde(default)]
    need: Option<String>,
}

fn item(id: &str) -> Item {
    Item {
        id: id.into(),
        need: None,
    }
}

/// Honest: the position is the engine's own `decision_seq`.
struct ItemKind;
impl FactKind for ItemKind {
    const NAME: &'static str = "oracle.item";
    const MAJOR: u32 = 1;
    type Payload = Item;
    fn creates(p: &Item) -> Option<EntityKey> {
        key(format!("item:{}", p.id))
    }
    fn needs(p: &Item) -> Vec<EntityKey> {
        p.need
            .iter()
            .filter_map(|n| key(format!("item:{n}")))
            .collect()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Item) -> Result<(), FactError> {
        put_item(tx, cx, &p.id, cx.decision_seq)
    }
}

#[derive(Serialize, Deserialize)]
struct Reg {
    thread: String,
    value: String,
}

struct RegKind;
impl FactKind for RegKind {
    const NAME: &'static str = "oracle.reg";
    const MAJOR: u32 = 1;
    type Payload = Reg;
    fn creates(_: &Reg) -> Option<EntityKey> {
        None
    }
    fn needs(_: &Reg) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Reg) -> Result<(), FactError> {
        let thread = ThreadId::parse(p.thread).expect("a thread id");
        thread_registers::upsert(
            tx,
            &thread,
            RegisterField::Topic,
            Some(&p.value),
            cx.hlc,
            cx.origin.node,
            cx.ingest_seq,
        )
        .map(|_| ())
        .map_err(db)
    }
}

#[derive(Serialize, Deserialize)]
struct Leaky {
    id: String,
    pos: i64,
}

/// Dishonest: the authoring node's position travels in the payload.
struct LeakyKind;
impl FactKind for LeakyKind {
    const NAME: &'static str = "oracle.leaky";
    const MAJOR: u32 = 1;
    type Payload = Leaky;
    fn creates(p: &Leaky) -> Option<EntityKey> {
        key(format!("item:{}", p.id))
    }
    fn needs(_: &Leaky) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Leaky) -> Result<(), FactError> {
        put_item(tx, cx, &p.id, p.pos)
    }
}

/// Dishonest: orders its rows one way on the authoring node and the opposite
/// way anywhere else, so replay inverts a causally ordered pair.
struct FlipKind;
impl FactKind for FlipKind {
    const NAME: &'static str = "oracle.flip";
    const MAJOR: u32 = 1;
    type Payload = Item;
    fn creates(p: &Item) -> Option<EntityKey> {
        key(format!("item:{}", p.id))
    }
    fn needs(_: &Item) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(tx: &Transaction<'_>, cx: &mut ApplyContext<'_>, p: Item) -> Result<(), FactError> {
        let pos = if cx.is_local_origin() {
            5_000_000 + cx.decision_seq
        } else {
            5_000_000 - cx.decision_seq
        };
        put_item(tx, cx, &p.id, pos)
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| {
        b.with::<ItemKind>()
            .with::<RegKind>()
            .with::<LeakyKind>()
            .with::<FlipKind>()
    })
}

fn author<K: FactKind>(store: &SqliteStore, payload: &K::Payload) {
    let ctx = store.context();
    let mut conn = ctx.open_writer().unwrap();
    ctx.execute_decision(
        &mut conn,
        |_| Ok(()),
        |tx, d, ()| journal::author::<K>(tx, ctx, d, payload),
    )
    .unwrap();
}

fn log_of(store: &OracleStore) -> LogKey {
    *store
        .vector(&budget())
        .unwrap()
        .0
        .keys()
        .find(|k| k.node == store.node())
        .unwrap()
}

fn ship(from: &SqliteStore, to: &SqliteStore, log: LogKey, upto: u64) {
    let entries: Vec<_> = from
        .serve(&log, 1, BatchLimits::default(), &budget())
        .unwrap()
        .into_iter()
        .filter(|e| e.counter() <= upto)
        .collect();
    let outcome = to.ingest(&log, entries, &budget()).unwrap();
    assert!(
        matches!(outcome, IngestOutcome::Stored { .. }),
        "{outcome:?}"
    );
}

fn freeze(store: &SqliteStore, log: LogKey, at: u64) {
    let ctx = store.context();
    let mut conn = ctx.open_writer().unwrap();
    ctx.execute_decision(
        &mut conn,
        |_| Ok(()),
        |tx, _, ()| journal::freeze(tx, &log, at),
    )
    .unwrap();
}

/// A source node with local facts and two peers' facts, one of which is
/// ingested before the fact it needs is applied (parked, then applied).
fn source() -> OracleStore {
    let x = OracleStore::open([21; 32], registry());
    author::<ItemKind>(&x, &item("x1")); // X@2 creates item:x1
    let d = OracleStore::open([22; 32], registry());
    ship(&x, &d, log_of(&x), 2);
    d.apply_pending(&budget()).unwrap();
    author::<ItemKind>(
        &d,
        &Item {
            id: "d1".into(),
            need: Some("x1".into()),
        },
    ); // D@2 deps X@2, needs item:x1

    let src = OracleStore::open([1; 32], registry());
    author::<ItemKind>(&src, &item("a1"));
    author::<RegKind>(
        &src,
        &Reg {
            thread: "t1".into(),
            value: "topic one".into(),
        },
    );
    ship(&x, &src, log_of(&x), 1);
    ship(&d, &src, log_of(&d), 2);
    ship(&x, &src, log_of(&x), 2);
    freeze(&src, log_of(&x), 1); // D@2 is ready before its creator applied
    let progress = src.apply_pending(&budget()).unwrap();
    assert_eq!(
        progress.replayed, 1,
        "D@2 parked, then re-attempted: {progress:?}"
    );
    src
}

fn count(store: &SqliteStore, sql: &str) -> i64 {
    query(store)
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

// Kills: a replay that drops, duplicates or alters a replicated row, or a
// projection that cannot see the kinds' tables.
#[test]
fn honest_kinds_replay_to_equal_projection() {
    let src = source();
    let replay = assert_replays_equal(&src, &specs());
    let projection = replicated_projection(&replay, replay.node(), &specs());
    assert_eq!(projection[ITEMS_LABEL].len(), 3, "a1, x1, d1");
    assert_eq!(projection["thread_registers where field='topic'"].len(), 1);
    assert_eq!(projection["node_directory"].len(), 3, "src, X and D");
    assert_eq!(
        count(
            &replay,
            "SELECT count(*) FROM journal WHERE apply_state<>'applied'"
        ),
        0,
        "the replay applied everything"
    );
}

// Kills: a projection that compares the replaying node's own genesis or self
// directory row, or one that filters nothing.
#[test]
fn replaying_node_genesis_not_compared() {
    let src = source();
    let replay = replay_into_empty(&src);
    let own = replay.node().to_string();
    assert_eq!(
        count(
            &replay,
            &format!("SELECT count(*) FROM node_directory WHERE node_uuid='{own}'")
        ),
        1,
        "the replay has its own directory row"
    );
    assert_eq!(
        count(
            &src,
            &format!("SELECT count(*) FROM node_directory WHERE node_uuid='{own}'")
        ),
        0,
        "the source has never heard of it"
    );
    assert_eq!(
        replicated_projection(&src, replay.node(), &specs()),
        replicated_projection(&replay, replay.node(), &specs())
    );
    // Without the exclusion the two projections differ by exactly that row.
    let unrelated = NodeIdentity::from_seed([99; 32]).uuid();
    let with_own = replicated_projection(&replay, unrelated, &specs());
    let without = replicated_projection(&src, unrelated, &specs());
    assert_eq!(
        with_own["node_directory"]
            .difference(&without["node_directory"])
            .count(),
        1
    );
}

// Kills: strict mode flagging honest positions, and a replay store whose
// counters do not start at the offset.
#[test]
fn strict_mode_passes_on_honest_kinds() {
    let src = source();
    let replay = assert_replays_equal(&src, &specs());
    assert_eq!(
        strict_violations(&src, &replay, &specs()),
        Vec::<String>::new()
    );
    let lowest: i64 = query(&replay)
        .unwrap()
        .query_row(
            "SELECT min(CAST(value AS INTEGER)) FROM thread_registers WHERE field='archived'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        lowest >= REPLAY_POSITION_OFFSET,
        "replay positions are its own: {lowest}"
    );
}

// Kills: strict mode missing a position that a payload carried from the
// authoring node (FactKind review rule 1).
#[test]
fn strict_mode_detects_position_in_payload() {
    let src = source();
    let seq: i64 = query(&src)
        .unwrap()
        .query_row("SELECT decision_seq FROM local_node", [], |r| r.get(0))
        .unwrap();
    author::<LeakyKind>(
        &src,
        &Leaky {
            id: "l1".into(),
            pos: seq,
        },
    );
    let replay = assert_replays_equal(&src, &specs()); // not a replicated column
    let violations = strict_violations(&src, &replay, &specs());
    // The leaked position is below the replay's own range; it also breaks
    // the relative order against the rows applied after it.
    assert!(
        violations.iter().any(
            |v| v.contains("[l1].value = ") && v.contains("a payload carried a local position")
        ),
        "{violations:?}"
    );
}

// Kills: strict mode ignoring the relative order of causally ordered rows.
#[test]
fn strict_mode_detects_inverted_order() {
    let src = source();
    author::<FlipKind>(&src, &item("f1"));
    author::<FlipKind>(&src, &item("f2")); // same log, after f1
    let replay = assert_replays_equal(&src, &specs());
    let violations = strict_violations(&src, &replay, &specs());
    assert!(
        violations
            .iter()
            .any(|v| v.contains("[f1].value precedes") && v.contains("[f2].value")),
        "{violations:?}"
    );
}

/// Inserts the same replicated rows into `store`, with `skew` shifting every
/// local position, attribution and delivery-side column.
fn seed_converted(store: &OracleStore, skew: i64, body: &str) {
    seed_converted_with(store, skew, body, 0, "ordinary");
}

/// [`seed_converted`] with the message's delivery priority and mode chosen.
fn seed_converted_with(store: &OracleStore, skew: i64, body: &str, priority: i64, mode: &str) {
    let conn = store.context().open_writer().unwrap();
    let me: String = conn
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    // Burn ordinals so every AUTOINCREMENT position differs between stores.
    for n in 0..skew {
        conn.execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
             VALUES(?1,?2,'burn','',1,1,0)",
            rusqlite::params![format!("burn{n}"), me],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,name,created_at,updated_at,journaled, \
         topic_revision,next_sequence) VALUES('t1',?1,'topic','goal','nm',10,?2,1,?3,?4)",
        rusqlite::params![me, 100 + skew, skew, 1 + skew],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local, \
         remote_kind,published) VALUES('s-r',?1,'remote','native',?2,?3,?4,0,'agent',1)",
        rusqlite::params![me, skew, 5 + skew, "ab".repeat(16)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,episode,state,joined_at,home_transition) \
         VALUES('t1','s-r',?1,'joined',?2,0)",
        rusqlite::params![1 + skew, 7 + skew],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,decision_seq, \
         body,decision_at,origin_node,native_observation,priority,delivery_mode) \
         VALUES('m1',?1,'t1',?2,'ordinary','s-r',?3,?6,42,?4,?5,?7,?8)",
        rusqlite::params![
            me,
            1 + skew,
            1 + skew,
            "ab".repeat(16),
            format!("obs{skew}"),
            body,
            priority,
            mode
        ],
    )
    .unwrap();
}

fn converted_projection(store: &OracleStore) -> BTreeMap<String, BTreeSet<Row>> {
    let stranger = NodeIdentity::from_seed([99; 32]).uuid();
    replicated_projection(store, stranger, CONVERTED_TABLES)
}

/// The rows of one table in a projection, whatever filter labels it.
fn table_rows<'a>(
    projection: &'a BTreeMap<String, BTreeSet<Row>>,
    table: &str,
) -> &'a BTreeSet<Row> {
    let (_, rows) = projection
        .iter()
        .find(|(label, _)| label.as_str() == table || label.starts_with(&format!("{table} where")))
        .unwrap_or_else(|| panic!("{table} is not projected"));
    rows
}

// Kills: a projection that ignores the message's delivery priority.
#[test]
fn changed_message_priority_projects_differently() {
    let a = OracleStore::open([41; 32], registry());
    let b = OracleStore::open([42; 32], registry());
    seed_converted_with(&a, 0, "hello", 0, "ordinary");
    seed_converted_with(&b, 0, "hello", 3, "ordinary");
    let (pa, pb) = (converted_projection(&a), converted_projection(&b));
    assert_ne!(table_rows(&pa, "messages"), table_rows(&pb, "messages"));
}

// Kills: a projection that ignores the message's delivery mode.
#[test]
fn changed_message_delivery_mode_projects_differently() {
    let a = OracleStore::open([41; 32], registry());
    let b = OracleStore::open([42; 32], registry());
    seed_converted_with(&a, 0, "hello", 1, "ordinary");
    seed_converted_with(&b, 0, "hello", 1, "lazy");
    let (pa, pb) = (converted_projection(&a), converted_projection(&b));
    assert_ne!(table_rows(&pa, "messages"), table_rows(&pb, "messages"));
}

// Kills: a projection that includes the node-local nudge threshold, which
// differs between nodes by design (reconcile 1.1).
#[test]
fn nudge_threshold_is_not_projected() {
    let a = OracleStore::open([41; 32], registry());
    let b = OracleStore::open([42; 32], registry());
    seed_converted(&a, 0, "hello");
    seed_converted(&b, 0, "hello");
    let before = converted_projection(&b);
    b.context()
        .open_writer()
        .unwrap()
        .execute("UPDATE seats SET nudge_threshold=5 WHERE id='s-r'", [])
        .unwrap();
    assert_eq!(before, converted_projection(&b), "threshold is node-local");
    assert_eq!(converted_projection(&a), converted_projection(&b));
}

// Kills: a projection that includes a local position, an attribution column or
// a local-only thread, or one that cannot see the converted tables at all.
#[test]
fn equal_replicated_rows_with_different_positions_project_equal() {
    let a = OracleStore::open([41; 32], registry());
    let b = OracleStore::open([42; 32], registry());
    seed_converted(&a, 0, "hello");
    seed_converted(&b, 3, "hello");
    let (pa, pb) = (converted_projection(&a), converted_projection(&b));
    assert_eq!(pa, pb);
    for table in ["threads", "seats", "memberships", "messages"] {
        assert_eq!(
            table_rows(&pa, table).len(),
            1,
            "{table} projects its row (and not the local-only ones)"
        );
    }
    let seat = table_rows(&pa, "seats").iter().next().unwrap();
    assert!(seat.contains(&("kind", "agent".to_owned())), "{seat:?}");
    assert!(seat.contains(&("retired", "0".to_owned())), "{seat:?}");
}

// Kills: a projection that drops a replicated column.
#[test]
fn changed_replicated_column_projects_differently() {
    let a = OracleStore::open([41; 32], registry());
    seed_converted(&a, 0, "hello");
    for (sql, table) in [
        ("UPDATE threads SET topic='other'", "threads"),
        ("UPDATE memberships SET state='left'", "memberships"),
        ("UPDATE seats SET state='retired', retired_at=9", "seats"),
    ] {
        let b = OracleStore::open([42; 32], registry());
        seed_converted(&b, 0, "hello");
        b.context().open_writer().unwrap().execute(sql, []).unwrap();
        let (pa, pb) = (converted_projection(&a), converted_projection(&b));
        assert_ne!(table_rows(&pa, table), table_rows(&pb, table), "{sql}");
    }
}

// Kills: a projection that ignores the message body.
#[test]
fn changed_message_body_projects_differently() {
    let a = OracleStore::open([41; 32], registry());
    let b = OracleStore::open([42; 32], registry());
    seed_converted(&a, 0, "hello");
    seed_converted(&b, 0, "bye");
    let (pa, pb) = (converted_projection(&a), converted_projection(&b));
    assert_ne!(table_rows(&pa, "messages"), table_rows(&pb, "messages"));
}
