use super::*;

use std::sync::Arc;

use crate::node::NodeIdentity;
use crate::protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::journal;
use crate::sync::facts::FactRegistry;
use crate::test_support::isolation::TestIsolation;

const SEED: [u8; 32] = crate::test_support::TEST_NODE_SEED;
const NOW: i64 = 1_700_000_000_000;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(NOW)
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

/// A store with its journal open (one self log, genesis only) and a writer
/// for seeding foreign rows.
struct Fixture {
    _iso: TestIsolation,
    ctx: StoreContext,
    writer: Connection,
    next_seq: i64,
}

impl Fixture {
    fn new() -> Self {
        let iso = TestIsolation::new("sync-status");
        let ctx = StoreContext::new(iso.state_root().join("store.db"), Arc::new(FixedClock))
            .with_journal_seed(SEED);
        ctx.seed_incarnations(1);
        let mut writer = ctx.open_writer().unwrap();
        journal::open(
            &ctx,
            &mut writer,
            Arc::new(NodeIdentity::from_seed(SEED)),
            "host",
            FactRegistry::builtins(),
        )
        .unwrap();
        Self {
            _iso: iso,
            ctx,
            writer,
            next_seq: 10_000,
        }
    }

    fn query(&self) -> impl std::ops::Deref<Target = Connection> {
        let db = self.ctx.open_query(budget()).unwrap();
        db.execute_batch("BEGIN DEFERRED").unwrap();
        db
    }

    /// A foreign log of node byte `node` with one entry per `(state, detail)`:
    /// entry `n` has hash `[n; 32]` and previous hash `[n-1; 32]`.
    /// `applied_through` is the number of leading `applied` entries.
    fn seed_log(&mut self, node: u8, incarnation: u32, entries: &[(&str, Option<&str>)]) -> i64 {
        let counter = entries.len() as i64;
        let applied = entries.iter().take_while(|(s, _)| *s == "applied").count() as i64;
        let uuid = format!("{node:02x}").repeat(16);
        self.writer
            .execute(
                "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash, \
                 contiguous_counter,head_hash,applied_through,state,first_seen_at) \
                 VALUES (?1,?2,0,zeroblob(32),zeroblob(32),?3,?4,?5,'active',0)",
                params![uuid, incarnation, counter, vec![counter as u8; 32], applied],
            )
            .unwrap();
        let log_ix = self.writer.last_insert_rowid();
        for (index, (state, detail)) in entries.iter().enumerate() {
            let n = index as i64 + 1;
            self.insert_entry(log_ix, n, state, *detail);
        }
        log_ix
    }

    fn insert_entry(&mut self, log_ix: i64, n: i64, state: &str, detail: Option<&str>) {
        let applied_seq = (state == "applied").then(|| {
            self.next_seq += 1;
            self.next_seq
        });
        self.writer
            .execute(
                "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps, \
                 needs,payload,signature,ingested_at,apply_state,applied_seq,parked_on,apply_error) \
                 VALUES (?1,?2,?3,?4,1,'test.kind',1,x'',  '',x'7b7d',zeroblob(64),0,?5,?6,?7,?8)",
                params![
                    log_ix,
                    n,
                    vec![(n - 1) as u8; 32],
                    vec![n as u8; 32],
                    state,
                    applied_seq,
                    (state == "parked_dep").then_some(detail).flatten(),
                    (state == "failed").then_some(detail).flatten(),
                ],
            )
            .unwrap();
    }
}

const MIXED: [(&str, Option<&str>); 5] = [
    ("applied", None),
    ("applied", None),
    ("parked_unknown", None),
    ("parked_dep", Some("e:thread-9")),
    ("failed", Some("boom")),
];

// Kills: counts taken from the wrong log, a head prefix off the wrong bytes, an
// applied_through that includes unapplied rows, a frozen log reported active,
// and an unbounded list.
#[test]
fn per_log_counts() {
    let mut fx = Fixture::new();
    fx.seed_log(0xaa, 1, &MIXED);
    let g = fx.seed_log(
        0xbb,
        2,
        &[("applied", None), ("applied", None), ("applied", None)],
    );
    fx.writer
        .execute(
            "UPDATE sync_logs SET state='frozen',frozen_at_counter=2 WHERE log_ix=?1",
            [g],
        )
        .unwrap();
    let db = fx.query();

    let (rows, omitted) = per_log(&db, None, 10).unwrap();
    let rows: Vec<LogRow> = rows.into_iter().map(|(_, row)| row).collect();
    assert_eq!(omitted, 0);
    assert_eq!(rows.len(), 3);
    assert!(rows[0].is_self, "own log first");
    assert_eq!(
        (rows[0].parked_unknown, rows[0].parked_dep, rows[0].failed),
        (0, 0, 0)
    );
    let a = rows.iter().find(|r| r.node == "aaaaaaaa").unwrap();
    assert_eq!(a.counter, 5);
    assert_eq!(a.head, "05050505");
    assert_eq!(a.applied_through, 2);
    assert_eq!(
        (a.parked_unknown, a.parked_dep, a.failed),
        (1, 1, 1),
        "one entry in each unapplied state"
    );
    assert_eq!(a.frozen_at, None);
    assert_eq!(a.lag, None, "lag is the coordinator's to fill");
    let b = rows.iter().find(|r| r.node == "bbbbbbbb").unwrap();
    assert_eq!(b.frozen_at, Some(2));
    assert_eq!(b.incarnation, 2);

    let (one, omitted) = per_log(&db, Some("aa".repeat(16).parse().unwrap()), 10).unwrap();
    assert_eq!((one.len(), omitted), (1, 0), "the node filter narrows");
    let (capped, omitted) = per_log(&db, None, 1).unwrap();
    assert_eq!(
        (capped.len(), omitted),
        (1, 2),
        "the cap reports what it left out"
    );
}

// Kills: a page past its bound, ingest_seq order lost across logs, a missing
// continuation, totals that count applied rows, and detail text on the wrong
// state.
#[test]
fn unapplied_page_bounded_and_ordered() {
    let mut fx = Fixture::new();
    fx.seed_log(0xaa, 1, &MIXED);
    // A second log whose entries interleave after the first log's.
    fx.seed_log(
        0xbb,
        1,
        &[("parked_unknown", None), ("failed", Some("late"))],
    );
    let db = fx.query();

    let first = unapplied_page(&db, None, 2).unwrap();
    assert_eq!(first.rows.len(), 2, "bounded by the limit");
    assert_eq!(
        first
            .rows
            .iter()
            .map(|r| (r.counter, r.state.as_str()))
            .collect::<Vec<_>>(),
        vec![(3, "parked_unknown"), (4, "parked_dep")]
    );
    assert_eq!(first.rows[1].detail.as_deref(), Some("on e:thread-9"));
    assert_eq!(first.rows[0].detail, None);
    assert_eq!(first.next_after, Some(first.rows[1].ingest_seq));
    assert_eq!(
        (first.parked_unknown, first.parked_dep, first.failed),
        (2, 1, 2),
        "totals cover every page and never the applied rows"
    );

    let second = unapplied_page(&db, first.next_after, 2).unwrap();
    assert_eq!(
        second
            .rows
            .iter()
            .map(|r| (r.log.as_str(), r.counter, r.state.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("aaaaaaaa/1", 5, "failed"),
            ("bbbbbbbb/1", 1, "parked_unknown"),
        ]
    );
    assert_eq!(second.rows[0].detail.as_deref(), Some("boom"));
    assert!(second.next_after.is_some());
    let third = unapplied_page(&db, second.next_after, 2).unwrap();
    assert_eq!(third.rows.len(), 1);
    assert_eq!(third.rows[0].detail.as_deref(), Some("late"));
    assert_eq!(third.next_after, None, "the last page has no continuation");
}

/// The seeds here carry made-up hashes, which J3 reports; the linkage and
/// invariant checks under test are read without those rows.
fn without_j3(
    found: Vec<crate::protocol::sync_admin::ViolationRow>,
) -> Vec<crate::protocol::sync_admin::ViolationRow> {
    found.into_iter().filter(|v| v.invariant != "J3").collect()
}

// Kills: a verify that reports a healthy store as broken, one that misses a
// journal_forked row at or below the freeze (I2), and one that misses an entry
// whose prev_hash does not match the stored hash before it.
#[test]
fn verify_healthy_and_seeded_violations() {
    let mut fx = Fixture::new();
    assert_eq!(
        without_j3(verify(&fx.query()).unwrap()),
        Vec::new(),
        "a fresh store"
    );
    fx.seed_log(0xaa, 1, &MIXED);
    assert_eq!(
        without_j3(verify(&fx.query()).unwrap()),
        Vec::new(),
        "a foreign log with unapplied entries is not a violation"
    );
    // The seeded hashes are made up: only J3 (rehash) reports them.
    let rehash = verify(&fx.query()).unwrap();
    assert!(
        rehash.iter().all(|v| v.invariant == "J3"),
        "made-up hashes are J3 only: {rehash:?}"
    );
    assert!(!rehash.is_empty(), "J3 names the made-up hashes");

    // I2: a forked row at counter 2 of a log frozen at 2.
    let g = fx.seed_log(
        0xbb,
        1,
        &[("applied", None), ("applied", None), ("applied", None)],
    );
    fx.writer
        .execute(
            "UPDATE sync_logs SET state='frozen',frozen_at_counter=2 WHERE log_ix=?1",
            [g],
        )
        .unwrap();
    fx.writer
        .execute(
            "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version, \
             deps,needs,payload,signature,received_at) \
             VALUES (?1,2,zeroblob(32),zeroblob(32),1,'test.kind',1,x'','',x'7b7d',zeroblob(64),0)",
            [g],
        )
        .unwrap();
    let found = without_j3(verify(&fx.query()).unwrap());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].invariant, "I2");
    assert_eq!(found[0].counter, Some(2));

    // A broken chain: entry 2 of a new log names another previous hash.
    let h = fx.seed_log(0xcc, 1, &[("applied", None)]);
    fx.writer
        .execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
             payload,signature,ingested_at,apply_state,applied_seq) \
             VALUES (?1,2,zeroblob(32),?2,1,'test.kind',1,x'','',x'7b7d',zeroblob(64),0,'applied',?3)",
            params![h, vec![2u8; 32], 99_999],
        )
        .unwrap();
    fx.writer
        .execute(
            "UPDATE sync_logs SET contiguous_counter=2,head_hash=?2,applied_through=2 WHERE log_ix=?1",
            params![h, vec![2u8; 32]],
        )
        .unwrap();
    let found = without_j3(verify(&fx.query()).unwrap());
    let chain: Vec<_> = found.iter().filter(|v| v.invariant == "J2").collect();
    assert_eq!(chain.len(), 1, "{found:?}");
    assert_eq!(chain[0].log.as_deref(), Some("cccccccc/1"));
    assert_eq!(chain[0].counter, Some(2));
}

// Kills: a node name that resolves to the wrong node, a tag that is guessed
// when ambiguous, and an unknown name that does not refuse.
#[test]
fn resolve_node_uuid_tag_unknown_and_ambiguous() {
    let fx = Fixture::new();
    let db = fx.query();
    let own: String = db
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    let own_tag: String = db
        .query_row(
            "SELECT tag FROM node_directory WHERE node_uuid=?1",
            [&own],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(resolve_node(&db, &own).unwrap().to_string(), own);
    assert_eq!(resolve_node(&db, &own_tag).unwrap().to_string(), own);
    assert_eq!(resolve_node(&db, "Zz9").unwrap_err().detail, "unknown node");
    assert!(
        resolve_node(&db, "not a node")
            .unwrap_err()
            .detail
            .contains("32-hex uuid")
    );
    // A second node with the same tag makes the tag ambiguous.
    fx.writer
        .execute(
            "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,?2,0,0)",
            params!["dd".repeat(16), own_tag],
        )
        .unwrap();
    let db = fx.query();
    assert!(
        resolve_node(&db, &own_tag)
            .unwrap_err()
            .detail
            .contains("ambiguous")
    );
    assert_eq!(
        resolve_node(&db, &own).unwrap().to_string(),
        own,
        "the uuid stays exact"
    );
}

// Kills: peer rows that drop a disabled target, a learned node shown by uuid
// when the directory knows its tag, and a reachability not taken from contacts.
#[test]
fn peers_list_enabled_disabled_and_reach() {
    let mut fx = Fixture::new();
    {
        let tx = fx.writer.transaction().unwrap();
        sync_dial_targets::add_explicit(&tx, "alice@host", Some("ht --state-dir /x"), 5).unwrap();
        sync_dial_targets::add_explicit(&tx, "bob@host", None, 6).unwrap();
        sync_dial_targets::forget(&tx, "bob@host", 7).unwrap();
        tx.commit().unwrap();
    }
    let peer = "ee".repeat(16);
    fx.writer
        .execute(
            "UPDATE sync_dial_targets SET learned_node_uuid=?1,state='linked' WHERE ssh_dest='alice@host'",
            [&peer],
        )
        .unwrap();
    fx.writer
        .execute(
            "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,'Pq7',0,0)",
            [&peer],
        )
        .unwrap();
    fx.writer
        .execute(
            "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) \
             VALUES (?1,?2,'direct',0)",
            params![peer, NOW - 1_000],
        )
        .unwrap();
    let rows = peers(&fx.query(), NOW as u64).unwrap();
    assert_eq!(rows.len(), 2, "forgotten targets stay listed");
    let alice = &rows[0];
    assert_eq!(alice.ssh_dest, "alice@host");
    assert!(alice.enabled);
    assert_eq!(alice.state, "linked");
    assert_eq!(alice.source, "explicit");
    assert_eq!(alice.node.as_deref(), Some("Pq7"));
    assert_eq!(alice.reach, "reachable");
    assert_eq!(alice.remote_command.as_deref(), Some("ht --state-dir /x"));
    let bob = &rows[1];
    assert!(!bob.enabled);
    assert_eq!((bob.node.as_deref(), bob.reach.as_str()), (None, "unknown"));

    let later = peers(&fx.query(), NOW as u64 + 1_000_000).unwrap();
    assert_eq!(later[0].reach, "unreachable", "contacts age out");
}
