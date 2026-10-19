//! Promotion of a relayed opaque entry after an upgrade, end to end (ht-76u6.6).
//!
//! Source -> relay -> capless third node, as in `opaque_relay`: the author's log
//! `[genesis, HTJRNL99 entry 2, known 3]` and a dependent log whose entry 2
//! depends on the author's entry 2. The relay stores the HTJRNL99 entry with an
//! empty projection and stalls the log (`blocked_on = opaque_layout`). The test
//! then plays the upgrade (`understand_synthetic`, the test hook that registers
//! a parser for the layout) and drives an apply pass on the relay: the entry's
//! projection is filled from its stored bytes, raw/hash/signature stay as
//! received, the successors and the dependent log apply, both stalls clear, and
//! the entry is still served opaque, so a capability peer linked afterwards
//! gets it as an opaque item byte for byte while the node that never advertised
//! the capability still never receives counter 2. The capability nodes' heads
//! converge.
//!
//! The pre-migration round trip (a schema-4 store migrated to the single-table
//! journal serves every entry byte for byte) is
//! `tests/store/journal.rs::served_known_frames_byte_identical_across_migration`;
//! it opens the migrated file at the connection level and checks every entry
//! through `serve_entries`, which is the layer the relay serves from, so this
//! file does not repeat it.
//!
//! The layout registry is process-wide state, hence this binary of its own
//! (`required-features = ["test-support"]`). Every node and link started here
//! is stopped before a test returns; no process is spawned.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use herdr_threads::node::{NodeIdentity, NodeUuid};
use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use herdr_threads::store::connection::StoreContext;
use herdr_threads::store::journal;
use herdr_threads::store::{SqliteStore, StoreSettings};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::envelope::{Deps, OpaqueEntry, StoredEntry, UnsignedEnvelope};
use herdr_threads::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::sync::journal_port::{IngestOutcome, JournalPort};
use herdr_threads::sync::{BatchLimits, LogKey};
use herdr_threads::test_support::isolation::TestIsolation;
use herdr_threads::test_support::opaque_entry::{
    synthetic_entry, synthetic_unsigned, understand_synthetic,
};
use herdr_threads::test_support::sync_node::{SyncNode, SyncNodeOptions, wait_for, wait_until};

const PATIENCE: Duration = Duration::from_secs(60);

/// The layout registry is process-wide: tests of this binary that register it
/// run one at a time under `cargo test` (nextest gives each its own process).
static LAYOUT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The kind the synthetic entries carry (`test.opaque` v1, payload `{"n":N}`);
/// registered so that an applied entry materializes as a no-op.
struct OpaqueKind;

#[derive(serde::Serialize, serde::Deserialize)]
struct N {
    n: u64,
}

impl FactKind for OpaqueKind {
    const NAME: &'static str = "test.opaque";
    const MAJOR: u32 = 1;
    type Payload = N;
    fn creates(_: &N) -> Option<EntityKey> {
        None
    }
    fn needs(_: &N) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(
        _: &rusqlite::Transaction<'_>,
        _: &mut ApplyContext<'_>,
        _: N,
    ) -> Result<(), FactError> {
        Ok(())
    }
}

fn registry() -> &'static FactRegistry {
    static REGISTRY: std::sync::OnceLock<&'static FactRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| FactRegistry::builtins_plus(|b| b.with::<OpaqueKind>()))
}

fn seed(n: u8) -> [u8; 32] {
    [n; 32]
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

struct FixedClock(AtomicU64);

impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

/// One journal-open store of node `seed` (as `facts_seat::Node::with`).
struct Node {
    store: Arc<SqliteStore>,
    id: NodeUuid,
    _isolation: TestIsolation,
}

impl Node {
    fn with(seed_byte: u8) -> Self {
        let isolation = TestIsolation::new("opaque-promotion");
        let clock = Arc::new(FixedClock(AtomicU64::new(1)));
        let ctx =
            StoreContext::new(isolation.path("store.db"), clock).with_journal_seed([seed_byte; 32]);
        ctx.seed_incarnations(u64::from(seed_byte));
        let identity = Arc::new(NodeIdentity::from_seed([seed_byte; 32]));
        let id = identity.uuid();
        let mut writer = ctx.open_writer().expect("open the writer");
        journal::open(&ctx, &mut writer, identity, "opaque-promotion", registry())
            .expect("open journal");
        drop(writer);
        ctx.attach_host("test-host").expect("attach a host");
        let store = Arc::new(
            SqliteStore::new(ctx, id.to_string(), StoreSettings::default())
                .expect("open the store"),
        );
        Self {
            store,
            id,
            _isolation: isolation,
        }
    }

    fn query<T>(
        &self,
        sql: &str,
        map: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let db = self
            .store
            .context()
            .open_query(budget())
            .expect("open a query connection");
        let mut stmt = db.prepare(sql).expect("prepare");
        stmt.query_map([], map)
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    fn scalar(&self, sql: &str) -> Option<String> {
        self.query(sql, |r| r.get::<_, String>(0))
            .into_iter()
            .next()
    }

    fn log_col(&self, log: &LogKey, col: &str) -> Option<i64> {
        self.query(
            &format!(
                "SELECT {col} FROM sync_logs WHERE node_uuid='{}' AND incarnation={}",
                log.node, log.incarnation
            ),
            |r| r.get::<_, i64>(0),
        )
        .first()
        .copied()
    }

    fn blocked_on(&self, log: &LogKey) -> Option<String> {
        self.query(
            &format!(
                "SELECT blocked_on FROM sync_logs WHERE node_uuid='{}' AND incarnation={}",
                log.node, log.incarnation
            ),
            |r| r.get::<_, Option<String>>(0),
        )
        .into_iter()
        .next()
        .flatten()
    }

    fn contiguous(&self, log: &LogKey) -> u64 {
        self.log_col(log, "contiguous_counter")
            .map_or(0, |c| u64::try_from(c).unwrap_or(0))
    }

    fn apply_state(&self, log: &LogKey, counter: u64) -> Option<String> {
        self.scalar(&format!(
            "SELECT apply_state FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter={counter}",
            log.node, log.incarnation
        ))
    }

    /// `(raw, hash, signature)` of the row at `counter`, whatever its projection.
    fn bytes(&self, log: &LogKey, counter: u64) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
        self.query(
            &format!(
                "SELECT j.raw,j.hash,j.signature FROM journal j \
                 JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter={counter}",
                log.node, log.incarnation
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .into_iter()
        .next()
    }

    /// The projection `kind` of the row at `counter`: `None` while it is empty.
    fn kind(&self, log: &LogKey, counter: u64) -> Option<String> {
        self.query(
            &format!(
                "SELECT j.kind FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter={counter}",
                log.node, log.incarnation
            ),
            |r| r.get::<_, Option<String>>(0),
        )
        .into_iter()
        .next()
        .flatten()
    }

    /// Rows of `log` at `counter` or beyond.
    fn rows_from(&self, log: &LogKey, counter: u64) -> String {
        self.scalar(&format!(
            "SELECT count(*) || '' FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid='{}' AND l.incarnation={} AND j.counter >= {counter}",
            log.node, log.incarnation
        ))
        .unwrap()
    }

    fn drain(&self) {
        for _ in 0..10_000 {
            if !self.store.apply_pending(&budget()).unwrap().has_more {
                return;
            }
        }
        panic!("a store never drained its apply passes");
    }
}

fn ingest(to: &SqliteStore, log: &LogKey, entries: Vec<StoredEntry>) {
    match to.ingest(log, entries, &budget()).unwrap() {
        IngestOutcome::Stored { .. } => {}
        other => panic!("ingest of {log:?}: {other:?}"),
    }
}

/// The node's own first log and its genesis entry.
fn genesis_of(node: &Node) -> (LogKey, StoredEntry) {
    let (log, _) = node
        .store
        .vector(&budget())
        .unwrap()
        .0
        .into_iter()
        .find(|(log, _)| log.node == node.id)
        .expect("the node authored a log");
    let genesis = node
        .store
        .serve(&log, 1, BatchLimits::default(), &budget())
        .unwrap()
        .remove(0);
    (log, genesis)
}

/// The origin log `[genesis, opaque 2, known 3]` and a dependent log
/// `[genesis, entry 2 depending on origin@2]`.
struct Fixture {
    log: LogKey,
    entries: Vec<StoredEntry>,
    opaque: OpaqueEntry,
    q: LogKey,
    q_entries: Vec<StoredEntry>,
}

fn fixture(origin_seed: u8, dependent_seed: u8) -> Fixture {
    let origin = Node::with(origin_seed);
    let (log, genesis) = genesis_of(&origin);
    let signer = NodeIdentity::from_seed(seed(origin_seed));
    let opaque = synthetic_entry(
        &signer,
        &synthetic_unsigned(&signer, log.incarnation, 2, genesis.hash()),
    );
    let three = synthetic_unsigned(&signer, log.incarnation, 3, opaque.hash()).seal(&signer);
    let entries = vec![
        genesis,
        StoredEntry::from(opaque.clone()),
        StoredEntry::from(three),
    ];

    let dep = Node::with(dependent_seed);
    let (q, q_genesis) = genesis_of(&dep);
    let two = UnsignedEnvelope {
        log: q,
        counter: 2,
        prev_hash: q_genesis.hash(),
        hlc: HlcStamp::from_parts(1_700_000_000_000, 5),
        kind: "test.opaque".into(),
        kind_version: 1,
        deps: Deps::new(vec![(log, 2)], &q).unwrap(),
        creates: None,
        needs: vec![],
        payload: br#"{"n":2}"#.to_vec(),
    }
    .seal(&NodeIdentity::from_seed(seed(dependent_seed)));
    Fixture {
        log,
        entries,
        opaque,
        q,
        q_entries: vec![q_genesis, StoredEntry::from(two)],
    }
}

fn start_capless(node: &Node, n: u8, boot: u128) -> SyncNode {
    let journal: Arc<dyn JournalPort> = node.store.clone();
    SyncNode::start_with(
        node.store.clone(),
        journal,
        seed(n),
        boot,
        SyncNodeOptions {
            advertise_opaque: false,
        },
    )
}

fn assert_original_bytes(node: &Node, f: &Fixture, who: &str) {
    let row = node
        .bytes(&f.log, 2)
        .unwrap_or_else(|| panic!("{who}: no row at counter 2"));
    assert_eq!(row.0, f.opaque.raw(), "{who}: raw");
    assert_eq!(row.1, f.opaque.hash(), "{who}: hash");
    assert_eq!(row.2, f.opaque.signature(), "{who}: signature");
}

fn assert_served_opaque(node: &Node, f: &Fixture, who: &str) {
    let held = node
        .store
        .serve(&f.log, 2, BatchLimits::default(), &budget())
        .unwrap();
    assert!(
        matches!(held[0], StoredEntry::Opaque(_)),
        "{who}: entry 2 is served opaque, not as a known envelope: {:?}",
        held[0]
    );
    assert_eq!(held[0], f.entries[1], "{who}: entry 2 byte for byte");
    assert_eq!(held[1], f.entries[2], "{who}: the known successor");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upgrade_promotes_relayed_entry_and_successors_apply() {
    let _serial = LAYOUT.lock().await;
    let f = fixture(61, 65);
    let (a, r, c, d) = (
        Node::with(62),
        Node::with(63),
        Node::with(64),
        Node::with(66),
    );
    ingest(&a.store, &f.log, f.entries.clone());
    ingest(&a.store, &f.q, f.q_entries.clone());
    let (na, nr, nc) = (
        SyncNode::start(a.store.clone(), seed(62), 1),
        SyncNode::start(r.store.clone(), seed(63), 2),
        start_capless(&c, 64, 3),
    );
    na.link(&nr).await;
    nr.link(&nc).await;

    // Before the upgrade: stored bytes-first, stalled, withheld from the capless node.
    wait_until("the relay holds both logs", PATIENCE, || {
        r.contiguous(&f.log) == 3 && r.contiguous(&f.q) == 2
    })
    .await;
    wait_until("the relay stalls", PATIENCE, || {
        r.blocked_on(&f.log).as_deref() == Some("opaque_layout") && r.blocked_on(&f.q).is_some()
    })
    .await;
    assert_eq!(
        r.kind(&f.log, 2),
        None,
        "empty projection before the upgrade"
    );
    assert_eq!(r.apply_state(&f.log, 2).as_deref(), Some("parked_layout"));
    assert_original_bytes(&r, &f, "relay before");
    let before = r.bytes(&f.log, 2).unwrap();
    let stalled_q = r.blocked_on(&f.q);
    assert!(
        stalled_q.as_deref().is_some_and(|t| t.starts_with("deps ")),
        "the dependent waits on the opaque entry: {stalled_q:?}"
    );
    assert_eq!(r.log_col(&f.log, "applied_through"), Some(1));
    assert_eq!(r.log_col(&f.q, "applied_through"), Some(1));

    // The upgrade: the layout becomes understood, an apply pass runs on the relay.
    let _understood = understand_synthetic();
    r.drain();

    assert_eq!(
        r.kind(&f.log, 2).as_deref(),
        Some("test.opaque"),
        "the projection is filled"
    );
    assert_eq!(
        r.bytes(&f.log, 2).unwrap(),
        before,
        "raw, hash and signature are untouched by promotion"
    );
    for (log, counter) in [(&f.log, 2), (&f.log, 3), (&f.q, 2)] {
        assert_eq!(
            r.apply_state(log, counter).as_deref(),
            Some("applied"),
            "{log:?}@{counter}"
        );
    }
    assert_eq!(r.log_col(&f.log, "applied_through"), Some(3));
    assert_eq!(r.log_col(&f.q, "applied_through"), Some(2));
    assert_eq!(r.blocked_on(&f.log), None, "the opaque stall cleared");
    assert_eq!(r.blocked_on(&f.q), None, "the dependent stall cleared");
    assert_served_opaque(&r, &f, "relay after promotion");

    // A capability peer linked after the promotion receives the entry as an
    // opaque item, byte for byte; the capless node still never gets counter 2.
    let nd = SyncNode::start(d.store.clone(), seed(66), 4);
    nr.link(&nd).await;
    wait_until("the late capability peer holds both logs", PATIENCE, || {
        d.contiguous(&f.log) == 3 && d.contiguous(&f.q) == 2
    })
    .await;
    assert_original_bytes(&d, &f, "late peer");
    assert_served_opaque(&d, &f, "late peer");

    // Heads converge across the capability nodes, and every holder applies.
    for log in [f.log, f.q] {
        wait_until("heads converge", PATIENCE, || {
            let heads = [na.head_of(&log), nr.head_of(&log), nd.head_of(&log)];
            heads[0].is_some() && heads.iter().all(|h| *h == heads[0])
        })
        .await;
    }
    assert_eq!(
        nd.head_of(&f.log),
        Some((3, f.entries[2].hash())),
        "the converged head is the known successor"
    );
    wait_until("the source and the late peer apply too", PATIENCE, || {
        [&a, &d].iter().all(|n| {
            n.log_col(&f.log, "applied_through") == Some(3)
                && n.log_col(&f.q, "applied_through") == Some(2)
                && n.blocked_on(&f.log).is_none()
                && n.blocked_on(&f.q).is_none()
        })
    })
    .await;

    let moved = wait_for(Duration::from_secs(2), || c.contiguous(&f.log) > 1).await;
    assert!(!moved, "the capless node never passes the cut");
    assert_eq!(
        c.rows_from(&f.log, 2),
        "0",
        "it holds nothing from counter 2"
    );
    assert!(nr.ended_links().is_empty(), "{:?}", nr.ended_links());
    assert!(nc.ended_links().is_empty(), "{:?}", nc.ended_links());

    na.stop().await;
    nr.stop().await;
    nc.stop().await;
    nd.stop().await;
}

// A promoted entry that is relayed, then a restart of the relay: the stored
// state survives and nothing is undone by open (open runs promotion again).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn promoted_state_holds_after_relay_restart() {
    let _serial = LAYOUT.lock().await;
    let f = fixture(71, 75);
    let (a, r) = (Node::with(72), Node::with(73));
    ingest(&a.store, &f.log, f.entries.clone());
    ingest(&a.store, &f.q, f.q_entries.clone());
    let (na, nr) = (
        SyncNode::start(a.store.clone(), seed(72), 1),
        SyncNode::start(r.store.clone(), seed(73), 2),
    );
    na.link(&nr).await;
    wait_until("the relay stalls", PATIENCE, || {
        r.contiguous(&f.log) == 3
            && r.contiguous(&f.q) == 2
            && r.blocked_on(&f.log).as_deref() == Some("opaque_layout")
    })
    .await;

    let _understood = understand_synthetic();
    r.drain();
    assert_eq!(r.apply_state(&f.log, 2).as_deref(), Some("applied"));

    nr.stop().await;
    let nr = SyncNode::start(r.store.clone(), seed(73), 3);
    na.link(&nr).await;
    let moved = wait_for(Duration::from_secs(2), || {
        r.apply_state(&f.log, 2).as_deref() != Some("applied")
    })
    .await;
    assert!(!moved, "the promotion is not undone by a restart");
    assert_eq!(r.blocked_on(&f.log), None);
    assert_served_opaque(&r, &f, "relay after restart");

    na.stop().await;
    nr.stop().await;
}
