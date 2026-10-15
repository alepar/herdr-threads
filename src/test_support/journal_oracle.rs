//! The journal replay oracle: replaying a node's journal into an empty store
//! reproduces the replicated tables (journal-core design, Goal and Testing
//! "Oracle").
//!
//! What it checks is FactKind review rule 1: payloads never carry local
//! positions. A handler that copies a position (`decision_seq`, an ordinal, an
//! `ingest_seq`) from the authoring node into a payload makes the replayed row
//! differ from what the replaying node would have assigned itself; the oracle
//! finds that by replaying into a store whose own position counters start far
//! above anything the source assigned.
//!
//! Responsibility
//! - Owns: [`replay_into_empty`] (a fresh store with another identity ingests
//!   every log of a source store through `JournalPort`, then drains
//!   `apply_pending`), [`replicated_projection`] (an order-insensitive view of
//!   the replicated columns of the tables in [`TableSpec`] lists), and
//!   [`strict_violations`] (local positions compared by relative order over
//!   causally ordered origins, and kept inside the replaying store's own
//!   range).
//! - Hides: how a store is opened with a chosen identity and registry
//!   ([`OracleStore::open`]) and how causal order is rebuilt from envelopes.
//! - Must not: know any kind's semantics, write the source store, or compare
//!   local positions by raw value (the replay applies in any causal linear
//!   extension of the source's order, so only relative order over causally
//!   ordered origins is comparable).
//! - Role: test harness. ht-5fr.4 adds one [`TableSpec`] per converted table
//!   (its replicated columns, its position columns, the column linking a row
//!   to its journal entry) to the list it passes in; nothing else changes.
//!
//! `CONVERTED_TABLES` lists the converted fact-kind tables; see the
//! write guard for the other side of that table set.
//!
//! Excluded from a projection: rows originating from the replaying node (its
//! own genesis and self `node_directory` row exist only in the replay) and
//! every local-position or local-clock column (`applied_seq`, `decision_seq`,
//! `sequence`, `ingest_seq`, `src_ingest_seq`, revisions, ordinals,
//! `first_seen_at`), because a table spec simply does not list them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Deref;
use std::sync::Arc;

use rusqlite::{Connection, types::ValueRef};

use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::connection::StoreContext;
use crate::store::journal;
use crate::store::{SqliteStore, StoreSettings};
use crate::sync::facts::FactRegistry;
use crate::sync::journal_port::{IngestOutcome, JournalPort};
use crate::sync::{BatchLimits, EntityKey, LogKey};

use super::isolation::TestIsolation;

/// The first local position a replay store hands out. A position the source
/// assigned (small, from a store that has seen far fewer decisions) and that a
/// payload carried into the replay lies below this; a position the replay
/// store assigned itself lies at or above it.
pub const REPLAY_POSITION_OFFSET: i64 = 1_000_000;

/// The seed of the replay store's key; the source of an oracle run uses
/// [`super::TEST_NODE_SEED`] (`[1; 32]`).
pub const REPLAY_SEED: [u8; 32] = [2; 32];

/// How the oracle reads one replicated table.
///
/// Owns: which columns identify a row, which columns are replicated, which are
/// local positions and how a row is tied to its origin. Must not: list a
/// local-position or local-clock column among the replicated ones.
#[derive(Clone, Copy, Debug)]
pub struct TableSpec {
    pub name: &'static str,
    /// An SQL predicate selecting this spec's rows of the table, for a table
    /// shared by several kinds of row (`field='topic'`); `None` is every row.
    pub filter: Option<&'static str>,
    /// Columns that identify a row; rows of two stores are matched by them.
    pub key_columns: &'static [&'static str],
    /// Columns that are a function of the applied fact set.
    pub replicated_columns: &'static [&'static str],
    /// A node-uuid column; rows where it names the replaying node are skipped.
    pub origin_column: Option<&'static str>,
    /// Columns holding a local position (drawn from the store's decision or
    /// journal counters). Strict mode compares their relative order and
    /// checks they lie in the replaying store's own range.
    pub position_columns: &'static [&'static str],
    /// The column holding the `journal.ingest_seq` of the row's fact; strict
    /// mode needs it to find a row's origin entry.
    pub src_column: Option<&'static str>,
}

/// The tables the journal core itself replicates.
pub const CORE_TABLES: &[TableSpec] = &[
    TableSpec {
        name: "node_directory",
        filter: None,
        key_columns: &["node_uuid"],
        replicated_columns: &["tag", "tag_k", "host_label", "tag_hlc"],
        origin_column: Some("node_uuid"),
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "thread_registers",
        filter: None,
        key_columns: &["thread_id", "field"],
        replicated_columns: &["value", "hlc", "node_uuid"],
        origin_column: Some("node_uuid"),
        position_columns: &[],
        src_column: Some("src_ingest_seq"),
    },
];

/// The converted fact-kind tables' replicated columns (fact-kinds design,
/// Testing "Oracle"). Attribution columns, local positions (`ordinal`,
/// `sequence`, `decision_seq`, `episode`, revisions) and every delivery table
/// are left out. The other side of this table set is
/// `write_guard::REPLICATED_TABLES`.
///
/// Only journaled threads and what hangs off them project: a local-only
/// (managed) thread never replicates.
///
/// Column entries may be `expr AS name`; a projected row is labelled by the
/// name. Seats project only published rows, normalized local versus remote.
/// `prepared_recipients` and `receipt_state` project the rows of replicated
/// (manifested, ordinary) messages.
pub const CONVERTED_TABLES: &[TableSpec] = &[
    TableSpec {
        name: "threads",
        filter: Some("journaled=1"),
        key_columns: &["id"],
        replicated_columns: &[
            "topic",
            "goal",
            "name",
            "archived",
            "created_at",
            "journaled",
        ],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "direct_threads",
        filter: None,
        key_columns: &["thread_id"],
        replicated_columns: &["seat_a", "seat_b", "created_hlc", "created_node"],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "seats",
        filter: Some("published=1"),
        key_columns: &["id"],
        replicated_columns: &[
            "home_node",
            "coalesce(remote_kind, CASE WHEN EXISTS(SELECT 1 FROM occupant_bindings b \
             WHERE b.seat_id=seats.id AND b.harness='human') THEN 'human' ELSE 'agent' END) AS kind",
            "(state='retired') AS retired",
            "retired_at",
        ],
        origin_column: Some("home_node"),
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "memberships",
        filter: Some("thread_id IN (SELECT id FROM threads WHERE journaled=1)"),
        key_columns: &["thread_id AS thread", "seat_id AS seat"],
        replicated_columns: &["state", "home_transition", "last_joined_n"],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "invitations",
        filter: Some("thread_id IN (SELECT id FROM threads WHERE journaled=1)"),
        key_columns: &["id"],
        replicated_columns: &[
            "thread_id AS thread",
            "seat_id AS seat",
            "state",
            "deadline_at",
            "confirmation",
            "inviter_seat_id AS inviter",
            "inviter_kind",
            "invitee_n",
            "origin_node",
            "accepted_at",
        ],
        origin_column: Some("origin_node"),
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "invitation_rejections",
        filter: Some(
            "invitation_id IN (SELECT id FROM invitations WHERE thread_id IN \
             (SELECT id FROM threads WHERE journaled=1))",
        ),
        key_columns: &["invitation_id"],
        replicated_columns: &["reason"],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "messages",
        filter: Some("kind='ordinary' AND thread_id IN (SELECT id FROM threads WHERE journaled=1)"),
        key_columns: &["id"],
        replicated_columns: &[
            "thread_id AS thread",
            "actor_seat_id",
            "body",
            "decision_at",
            "origin_node",
        ],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "prepared_recipients",
        filter: Some(
            "preparation_id IN (SELECT preparation_id FROM send_manifests) AND thread_id IN \
             (SELECT id FROM threads WHERE journaled=1)",
        ),
        key_columns: &[
            "(SELECT m.message_id FROM send_manifests m WHERE \
             m.preparation_id=prepared_recipients.preparation_id) AS message",
            "seat_id AS seat",
        ],
        replicated_columns: &["frozen_duration_ms", "eligible_at_snapshot AS eligible"],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
    TableSpec {
        name: "receipt_state",
        filter: Some(
            "message_id IN (SELECT id FROM messages WHERE kind='ordinary' AND thread_id IN \
             (SELECT id FROM threads WHERE journaled=1))",
        ),
        key_columns: &["message_id AS message", "seat_id AS seat"],
        replicated_columns: &[
            "state",
            "deadline_at",
            "acked_at",
            "settle_batch",
            "retired_at",
        ],
        origin_column: None,
        position_columns: &[],
        src_column: None,
    },
];

/// [`CORE_TABLES`] and [`CONVERTED_TABLES`]: every replicated table.
pub fn replicated_tables() -> Vec<TableSpec> {
    CORE_TABLES
        .iter()
        .chain(CONVERTED_TABLES)
        .copied()
        .collect()
}

/// The name a projected column is reported under: the part after `AS`, else
/// the whole entry.
fn column_label(column: &'static str) -> &'static str {
    column.rsplit_once(" AS ").map_or(column, |(_, name)| name)
}

struct OracleClock;
impl Clock for OracleClock {
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

/// A store under a chosen key and fact registry, in a private temp directory
/// that is removed on drop.
///
/// Owns: the store and its scratch directory. Derefs to [`SqliteStore`], so it
/// is a `JournalPort`.
pub struct OracleStore {
    store: SqliteStore,
    node: NodeUuid,
    _isolation: TestIsolation,
}

impl OracleStore {
    /// Opens a store whose node is `NodeIdentity::from_seed(seed)` with the
    /// journal opened under `registry`.
    pub fn open(seed: [u8; 32], registry: &'static FactRegistry) -> Self {
        let isolation = TestIsolation::new("journal-oracle");
        let path = isolation.state_root().join("store.db");
        let ctx = StoreContext::new(path, Arc::new(OracleClock)).with_journal_seed(seed);
        ctx.seed_incarnations(u64::from(seed[0]));
        let identity = Arc::new(NodeIdentity::from_seed(seed));
        let node = identity.uuid();
        let mut writer = ctx.open_writer().expect("open the oracle store writer");
        journal::open(&ctx, &mut writer, identity, "oracle", registry)
            .expect("open the oracle store journal");
        drop(writer);
        let store = SqliteStore::new(ctx, node.to_string(), StoreSettings::default())
            .expect("open the oracle store");
        Self {
            store,
            node,
            _isolation: isolation,
        }
    }

    /// The node this store belongs to.
    pub fn node(&self) -> NodeUuid {
        self.node
    }
}

impl Deref for OracleStore {
    type Target = SqliteStore;
    fn deref(&self) -> &SqliteStore {
        &self.store
    }
}

fn registry_of(store: &SqliteStore) -> &'static FactRegistry {
    store
        .context()
        .sync()
        .expect("the source store's journal is open")
        .registry
}

fn query(store: &SqliteStore) -> Result<crate::store::connection::QueryConnection, ApiError> {
    store.context().open_query(budget())
}

/// Replays `src`'s journal into a fresh store: a new node (seed
/// [`REPLAY_SEED`], the same registry as `src`) whose local position counters
/// start at [`REPLAY_POSITION_OFFSET`], ingests every log of `src` one origin
/// log at a time (`LogKey` order, counter order) and drains `apply_pending`.
pub fn replay_into_empty(src: &SqliteStore) -> OracleStore {
    let replay = OracleStore::open(REPLAY_SEED, registry_of(src));
    {
        let writer = replay
            .context()
            .open_writer()
            .expect("open a writer on the replay store");
        writer
            .execute(
                "UPDATE local_node SET decision_seq=?1",
                [REPLAY_POSITION_OFFSET],
            )
            .expect("offset the replay decision counter");
        writer
            .execute(
                "UPDATE sqlite_sequence SET seq=?1 WHERE name='journal'",
                [REPLAY_POSITION_OFFSET],
            )
            .expect("offset the replay ingest counter");
    }
    let vector = src.vector(&budget()).expect("read the source vector");
    for (log, head) in &vector.0 {
        let mut from = 1;
        while from <= head.counter {
            let batch = src
                .serve(log, from, BatchLimits::default(), &budget())
                .expect("serve the source log");
            let Some(last) = batch.last().map(|e| e.counter) else {
                panic!("source log {log:?} ends before its head {}", head.counter);
            };
            match replay.ingest(log, batch, &budget()).expect("ingest") {
                IngestOutcome::Stored { .. } => {}
                other => panic!("replay ingest of {log:?} from {from}: {other:?}"),
            }
            from = last + 1;
        }
    }
    for _ in 0..100_000 {
        if !replay
            .apply_pending(&budget())
            .expect("apply pending")
            .has_more
        {
            return replay;
        }
    }
    panic!("the replay store never drained its apply passes");
}

/// One projected row: `(column, rendered value)` in spec order.
pub type Row = Vec<(&'static str, String)>;

fn render(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_owned(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => b.iter().map(|byte| format!("{byte:02x}")).collect(),
    }
}

fn table_exists(db: &Connection, name: &str) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )
    .expect("look the table up")
}

/// The `WHERE` clause of a spec; `?1` is the replaying node when the spec has
/// an origin column.
fn where_clause(spec: &TableSpec) -> String {
    let mut terms = Vec::new();
    if let Some(filter) = spec.filter {
        terms.push(format!("({filter})"));
    }
    if let Some(origin) = spec.origin_column {
        terms.push(format!("{origin} <> ?1"));
    }
    if terms.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", terms.join(" AND "))
    }
}

/// The key a spec's rows are reported under: the table, plus the filter when
/// the spec selects part of it.
fn spec_label(spec: &TableSpec) -> String {
    match spec.filter {
        Some(filter) => format!("{} where {filter}", spec.name),
        None => spec.name.to_owned(),
    }
}

/// The replicated columns of `specs`' tables in `store`, as sets of rows, so
/// the comparison does not depend on the order rows were applied. Rows whose
/// origin column names `replayer` are left out. A table that does not exist
/// (a test kind's table nobody wrote yet) projects as empty.
pub fn replicated_projection(
    store: &SqliteStore,
    replayer: NodeUuid,
    specs: &[TableSpec],
) -> BTreeMap<String, BTreeSet<Row>> {
    let db = query(store).expect("open a query connection");
    let mut out = BTreeMap::new();
    for spec in specs {
        let mut rows = BTreeSet::new();
        if table_exists(&db, spec.name) {
            let columns: Vec<&'static str> = spec
                .key_columns
                .iter()
                .chain(spec.replicated_columns)
                .copied()
                .collect();
            let sql = format!(
                "SELECT {} FROM {}{}",
                columns.join(","),
                spec.name,
                where_clause(spec)
            );
            let mut stmt = db.prepare(&sql).expect("prepare the projection");
            let mut cursor = if spec.origin_column.is_some() {
                stmt.query([replayer.to_string()])
            } else {
                stmt.query([])
            }
            .expect("run the projection");
            while let Some(row) = cursor.next().expect("read a projected row") {
                rows.insert(
                    columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (column_label(c), render(row.get_ref(i).expect("cell"))))
                        .collect(),
                );
            }
        }
        out.insert(spec_label(spec), rows);
    }
    out
}

type Origin = (LogKey, u64);

/// Causal order over journal entries, rebuilt from envelopes: the previous
/// counter of the same log, the `deps` triples and `needs` keys resolved to
/// the entry that `creates` them.
struct Causality {
    preds: HashMap<Origin, Vec<Origin>>,
}

impl Causality {
    fn of(store: &SqliteStore) -> Self {
        let vector = store.vector(&budget()).expect("read the vector");
        let mut entries = Vec::new();
        for (log, head) in &vector.0 {
            let mut from = 1;
            while from <= head.counter {
                let batch = store
                    .serve(log, from, BatchLimits::default(), &budget())
                    .expect("serve a log");
                from = batch.last().expect("a served entry").counter + 1;
                entries.extend(batch);
            }
        }
        let creators: HashMap<&EntityKey, Vec<Origin>> =
            entries.iter().fold(HashMap::new(), |mut map, entry| {
                if let Some(key) = &entry.creates {
                    map.entry(key).or_default().push((entry.log, entry.counter));
                }
                map
            });
        let mut preds: HashMap<Origin, Vec<Origin>> = HashMap::new();
        for entry in &entries {
            let at = (entry.log, entry.counter);
            let list = preds.entry(at).or_default();
            if entry.counter > 1 {
                list.push((entry.log, entry.counter - 1));
            }
            list.extend(entry.deps.entries().iter().copied());
            for need in &entry.needs {
                list.extend(creators.get(need).into_iter().flatten().copied());
            }
        }
        Self { preds }
    }

    /// Whether `earlier` is a causal ancestor of `later`.
    fn before(&self, earlier: Origin, later: Origin) -> bool {
        let mut seen = BTreeSet::new();
        let mut stack = vec![later];
        while let Some(at) = stack.pop() {
            for pred in self.preds.get(&at).into_iter().flatten() {
                if *pred == earlier {
                    return true;
                }
                if seen.insert(*pred) {
                    stack.push(*pred);
                }
            }
        }
        false
    }
}

/// One position-bearing item of a store: what it is, where it came from and
/// the position.
struct Positioned {
    label: String,
    origin: Origin,
    position: i64,
}

fn origin_of(db: &Connection, ingest_seq: i64) -> Option<Origin> {
    db.query_row(
        "SELECT l.node_uuid,l.incarnation,j.counter FROM journal j \
         JOIN sync_logs l ON l.log_ix=j.log_ix WHERE j.ingest_seq=?1",
        [ingest_seq],
        |r| {
            let node: String = r.get(0)?;
            let incarnation: u32 = r.get(1)?;
            let counter: i64 = r.get(2)?;
            Ok((
                LogKey {
                    node: node.parse().expect("a node uuid"),
                    incarnation,
                },
                counter as u64,
            ))
        },
    )
    .ok()
}

/// The positions of one store, keyed by a stable item key so the source's and
/// the replay's items match: `journal.applied_seq` per `(log, counter)` and
/// every position column of every spec per row key.
fn positions(
    store: &SqliteStore,
    replayer: NodeUuid,
    specs: &[TableSpec],
) -> BTreeMap<String, Positioned> {
    let db = query(store).expect("open a query connection");
    let mut out = BTreeMap::new();
    let mut stmt = db
        .prepare(
            "SELECT l.node_uuid,l.incarnation,j.counter,j.applied_seq FROM journal j \
             JOIN sync_logs l ON l.log_ix=j.log_ix WHERE j.applied_seq IS NOT NULL",
        )
        .expect("prepare journal positions");
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .expect("read journal positions");
    for row in rows {
        let (node, incarnation, counter, applied_seq) = row.expect("a journal position");
        if node == replayer.to_string() {
            continue;
        }
        let log = LogKey {
            node: node.parse().expect("a node uuid"),
            incarnation,
        };
        let label = format!("journal {node}/{incarnation}@{counter} applied_seq");
        out.insert(
            label.clone(),
            Positioned {
                label,
                origin: (log, counter as u64),
                position: applied_seq,
            },
        );
    }
    for spec in specs.iter().filter(|s| !s.position_columns.is_empty()) {
        let src = spec
            .src_column
            .unwrap_or_else(|| panic!("{}: position columns need a src_column", spec.name));
        if !table_exists(&db, spec.name) {
            continue;
        }
        let columns: Vec<&str> = spec
            .key_columns
            .iter()
            .chain(spec.position_columns)
            .chain(std::iter::once(&src))
            .copied()
            .collect();
        let sql = format!(
            "SELECT {} FROM {}{}",
            columns.join(","),
            spec.name,
            where_clause(spec)
        );
        let mut stmt = db.prepare(&sql).expect("prepare table positions");
        let mut cursor = if spec.origin_column.is_some() {
            stmt.query([replayer.to_string()])
        } else {
            stmt.query([])
        }
        .expect("run table positions");
        let keys = spec.key_columns.len();
        while let Some(row) = cursor.next().expect("read a position row") {
            let key: Vec<String> = (0..keys)
                .map(|i| render(row.get_ref(i).expect("key cell")))
                .collect();
            let ingest_seq: i64 = row.get(columns.len() - 1).expect("src cell");
            let Some(origin) = origin_of(&db, ingest_seq) else {
                continue;
            };
            for (offset, column) in spec.position_columns.iter().enumerate() {
                let cell = render(row.get_ref(keys + offset).expect("position cell"));
                let Ok(position) = cell.parse::<i64>() else {
                    continue;
                };
                let label = format!("{}[{}].{column}", spec_label(spec), key.join(","));
                out.insert(
                    label.clone(),
                    Positioned {
                        label,
                        origin,
                        position,
                    },
                );
            }
        }
    }
    out
}

/// Compares the local positions of `src` and `replay` and returns what is
/// wrong, empty when they agree.
///
/// - A position column of a replayed row lies below [`REPLAY_POSITION_OFFSET`]:
///   the value did not come from the replay store's own counters, so a payload
///   carried it (review rule 1).
/// - For every pair of items whose origin entries are causally ordered (same
///   log by counter, `deps`, `needs` to `creates`), the relative order of
///   their positions differs between the two stores.
pub fn strict_violations(
    src: &SqliteStore,
    replay: &OracleStore,
    specs: &[TableSpec],
) -> Vec<String> {
    let mut violations = Vec::new();
    let from_src = positions(src, replay.node(), specs);
    let from_replay = positions(replay, replay.node(), specs);
    for (label, item) in &from_replay {
        if !label.starts_with("journal ") && item.position < REPLAY_POSITION_OFFSET {
            violations.push(format!(
                "{label} = {} is below the replay store's own range ({REPLAY_POSITION_OFFSET}): \
                 a payload carried a local position",
                item.position
            ));
        }
    }
    let causality = Causality::of(src);
    let shared: Vec<(&Positioned, &Positioned)> = from_src
        .iter()
        .filter_map(|(label, s)| from_replay.get(label).map(|r| (s, r)))
        .collect();
    for (i, (a_src, a_replay)) in shared.iter().enumerate() {
        for (b_src, b_replay) in &shared[i + 1..] {
            let (first, second) = if causality.before(a_src.origin, b_src.origin) {
                ((a_src, a_replay), (b_src, b_replay))
            } else if causality.before(b_src.origin, a_src.origin) {
                ((b_src, b_replay), (a_src, a_replay))
            } else {
                continue;
            };
            let in_src = first.0.position.cmp(&second.0.position);
            let in_replay = first.1.position.cmp(&second.1.position);
            if in_src != in_replay {
                violations.push(format!(
                    "{} precedes {} causally; positions order {in_src:?} in the source but \
                     {in_replay:?} in the replay",
                    first.0.label, second.0.label
                ));
            }
        }
    }
    violations
}

/// Replays `src` and asserts the replicated projection is equal; returns the
/// replay store for further (strict) checks. Panics with both projections'
/// difference on a mismatch.
pub fn assert_replays_equal(src: &SqliteStore, specs: &[TableSpec]) -> OracleStore {
    let replay = replay_into_empty(src);
    let want = replicated_projection(src, replay.node(), specs);
    let got = replicated_projection(&replay, replay.node(), specs);
    for (table, rows) in &want {
        let replayed = &got[table];
        let missing: Vec<_> = rows.difference(replayed).collect();
        let extra: Vec<_> = replayed.difference(rows).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{table} differs after replay: missing {missing:?}, extra {extra:?}"
        );
    }
    replay
}

#[cfg(test)]
#[path = "../../tests/store/journal_oracle.rs"]
mod tests;
