//! One contract suite for the two `JournalPort` implementations: every case
//! runs against `SqliteStore` and against `MemoryJournal` and must give the
//! same answer (journal-core design, section 12; Testing "JournalPort
//! contract"). `Headroom` needs a disk, so it runs on `SqliteStore` only.

use std::sync::Arc;

use herdr_threads::node::{NodeIdentity, NodeUuid};
use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use herdr_threads::store::connection::StoreContext;
use herdr_threads::store::{SqliteStore, StoreSettings, journal};
use herdr_threads::sync::chain::{ChainStep, HeldLog, check_next};
use herdr_threads::sync::envelope::{
    Deps, Envelope, EnvelopeError, OpaqueEntry, StoredEntry, UnsignedEnvelope,
};
use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::sync::journal_port::{ForkEvidence, IngestOutcome, JournalPort, LogMeta};
use herdr_threads::sync::memory_journal::MemoryJournal;
use herdr_threads::sync::{BatchLimits, LogHead, LogKey, LogState};
use herdr_threads::test_support::opaque_entry::{
    synthetic_entry, synthetic_parts, synthetic_unsigned,
};
use herdr_threads::test_support::{TEST_NODE_SEED, isolation::TestIsolation};

// ---- the two subjects -------------------------------------------------------

/// What the cases need beyond the port: the node's own identity and log, and
/// a way to close a foreign log (the port has no freeze; `journal::freeze`
/// is called by the sync layer's fork handling).
trait Subject {
    fn port(&self) -> &dyn JournalPort;
    fn self_log(&self) -> LogKey;
    fn freeze(&self, log: &LogKey, at: u64);
    /// Records forked-branch entries (the port has no write for them; the
    /// sync layer's fork handling calls `journal::store_forked`).
    fn store_forked(&self, log: &LogKey, entries: &[StoredEntry]);
    /// Lowers the most forked entries `serve_branch` walks.
    fn set_walk_bound(&self, steps: u64);
}

fn own_identity() -> NodeIdentity {
    NodeIdentity::from_seed(TEST_NODE_SEED)
}

struct FixedClock;
impl Clock for FixedClock {
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

struct SqliteSubject {
    // Declared first: the store closes its connections before the directory goes.
    store: SqliteStore,
    _dir: TestIsolation,
}

fn sqlite_subject() -> SqliteSubject {
    let dir = TestIsolation::new("journal-port-contract");
    let context = StoreContext::new(dir.path("store.db"), Arc::new(FixedClock));
    // The fixture context opens the journal of `TEST_NODE_SEED` with its own
    // genesis at counter 1.
    let store = SqliteStore::new(
        context,
        herdr_threads::test_support::test_node(),
        StoreSettings::default(),
    )
    .unwrap();
    SqliteSubject { store, _dir: dir }
}

impl Subject for SqliteSubject {
    fn port(&self) -> &dyn JournalPort {
        &self.store
    }

    fn self_log(&self) -> LogKey {
        let node = own_identity().uuid();
        *self
            .store
            .vector(&budget())
            .unwrap()
            .0
            .keys()
            .find(|key| key.node == node)
            .expect("the fixture store opened its own log")
    }

    fn freeze(&self, log: &LogKey, at: u64) {
        let context = self.store.context();
        let mut conn = context.open_writer().unwrap();
        context
            .execute_decision(
                &mut conn,
                |_| Ok(()),
                |tx, _, ()| journal::freeze(tx, log, at),
            )
            .unwrap();
    }

    fn store_forked(&self, log: &LogKey, entries: &[StoredEntry]) {
        let context = self.store.context();
        let mut conn = context.open_writer().unwrap();
        context
            .execute_decision(
                &mut conn,
                |_| Ok(()),
                |tx, _, ()| journal::store_forked_entries(tx, log, entries, None),
            )
            .unwrap();
    }

    fn set_walk_bound(&self, steps: u64) {
        self.store.context().set_branch_walk_bound(steps);
    }
}

struct MemorySubject(MemoryJournal);

fn memory_subject() -> MemorySubject {
    let memory = MemoryJournal::new(own_identity());
    memory.bootstrap();
    MemorySubject(memory)
}

impl Subject for MemorySubject {
    fn port(&self) -> &dyn JournalPort {
        &self.0
    }

    fn self_log(&self) -> LogKey {
        self.0.self_log()
    }

    fn freeze(&self, log: &LogKey, at: u64) {
        self.0.freeze(log, at).unwrap();
    }

    fn store_forked(&self, log: &LogKey, entries: &[StoredEntry]) {
        self.0.store_forked(log, entries).unwrap();
    }

    fn set_walk_bound(&self, steps: u64) {
        self.0.set_branch_walk_bound(steps);
    }
}

// ---- entries ----------------------------------------------------------------

fn remote() -> NodeIdentity {
    NodeIdentity::from_seed([7; 32])
}

fn remote_log(id: &NodeIdentity) -> LogKey {
    LogKey {
        node: id.uuid(),
        incarnation: 3,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn seal(
    id: &NodeIdentity,
    signer: &NodeIdentity,
    log: LogKey,
    counter: u64,
    prev: [u8; 32],
    payload: String,
) -> Envelope {
    let genesis = counter == 1;
    let payload = if genesis {
        // A genesis the receiving node can apply, not just store.
        format!(
            r#"{{"pubkey":"{}","tag":"{}","tag_k":{},"incarnation":{},"host_label":"remote"}}"#,
            hex(&id.public_key()),
            id.tag(),
            id.tag_k(),
            log.incarnation
        )
    } else {
        payload
    };
    UnsignedEnvelope {
        log,
        counter,
        prev_hash: prev,
        // Well before the test clock, so nothing is skewed.
        hlc: HlcStamp::from_parts(1_600_000_000_000 + counter, 0),
        kind: if genesis {
            "node.genesis".into()
        } else {
            "test.note".into()
        },
        kind_version: 1,
        deps: Deps::empty(),
        creates: None,
        needs: vec![],
        payload: payload.into_bytes(),
    }
    .seal(signer)
}

/// Entries `1..=n` of `id`'s log (a genesis, then small notes).
fn chain(id: &NodeIdentity, n: u64) -> Vec<Envelope> {
    chain_with(id, n, |c| format!(r#"{{"n":{c}}}"#))
}

fn chain_with(id: &NodeIdentity, n: u64, payload: impl Fn(u64) -> String) -> Vec<Envelope> {
    let log = remote_log(id);
    let mut out: Vec<Envelope> = Vec::new();
    for counter in 1..=n {
        let prev = out.last().map_or([0; 32], |e| e.hash);
        out.push(seal(id, id, log, counter, prev, payload(counter)));
    }
    out
}

/// Known entries as the port takes them.
fn wrap(entries: &[Envelope]) -> Vec<StoredEntry> {
    entries.iter().cloned().map(StoredEntry::from).collect()
}

/// A served batch of known entries; an opaque one is a test failure.
fn known(batch: Vec<StoredEntry>) -> Vec<Envelope> {
    batch
        .into_iter()
        .map(|entry| entry.as_known().cloned().expect("a known entry"))
        .collect()
}

fn ingest(s: &dyn Subject, log: &LogKey, entries: &[Envelope]) -> IngestOutcome {
    ingest_entries(s, log, wrap(entries))
}

fn ingest_entries(s: &dyn Subject, log: &LogKey, entries: Vec<StoredEntry>) -> IngestOutcome {
    s.port().ingest(log, entries, &budget()).unwrap()
}

fn head(s: &dyn Subject, log: &LogKey) -> Option<LogHead> {
    s.port().vector(&budget()).unwrap().0.get(log).copied()
}

fn stored_head(counter: u64, entries: &[Envelope]) -> LogHead {
    LogHead {
        counter,
        head_hash: entries[counter as usize - 1].hash,
        state: LogState::Active,
        frozen_at: None,
    }
}

/// An opaque (`HTJRNL99`) entry of `id`'s log at `counter` after `prev`; `tag`
/// varies the payload so two at one position differ.
fn opaque_at(id: &NodeIdentity, counter: u64, prev: [u8; 32], tag: &str) -> StoredEntry {
    let mut unsigned = synthetic_unsigned(id, remote_log(id).incarnation, counter, prev);
    unsigned.payload = tag.as_bytes().to_vec();
    StoredEntry::from(synthetic_entry(id, &unsigned))
}

// ---- cases ------------------------------------------------------------------

/// The cases, as functions of the subject; `contract!` runs each on both.
mod cases {
    use super::*;

    pub fn vector_after_author_and_ingest(s: &dyn Subject) {
        let own = s.self_log();
        let before = s.port().vector(&budget()).unwrap();
        assert_eq!(before.0.len(), 1, "only the own log: {before:?}");
        let genesis = known(
            s.port()
                .serve(&own, 1, BatchLimits::default(), &budget())
                .unwrap(),
        );
        assert_eq!(genesis.len(), 1);
        assert_eq!(before.0[&own], stored_head(1, &genesis));

        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        assert_eq!(head(s, &log), None, "unknown log is absent");
        let outcome = ingest(s, &log, &entries);
        assert_eq!(
            outcome,
            IngestOutcome::Stored {
                stored: 3,
                duplicates: 0,
                head: stored_head(3, &entries)
            }
        );
        let after = s.port().vector(&budget()).unwrap();
        assert_eq!(after.0.len(), 2);
        assert_eq!(after.0[&log], stored_head(3, &entries));
        assert_eq!(after.0[&own], before.0[&own], "own head unchanged");
    }

    pub fn serve_limits(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        // 300 small entries: the 256-entry limit bites first.
        let entries = chain(&id, 300);
        for batch in entries.chunks(100) {
            assert!(matches!(
                ingest(s, &log, batch),
                IngestOutcome::Stored { .. }
            ));
        }
        let limits = BatchLimits::default();
        let page = s.port().serve(&log, 1, limits, &budget()).unwrap();
        assert_eq!(page.len(), 256);
        assert_eq!(page.first().map(StoredEntry::counter), Some(1));
        assert_eq!(page.last().map(StoredEntry::counter), Some(256));
        let rest = s.port().serve(&log, 257, limits, &budget()).unwrap();
        assert_eq!(rest.len(), 44);
        assert_eq!(rest[0], StoredEntry::from(entries[256].clone()));
        assert_eq!(page[255], StoredEntry::from(entries[255].clone()));
        // `from` 0 reads as 1; beyond the head and an unknown log are empty.
        assert_eq!(
            s.port().serve(&log, 0, limits, &budget()).unwrap()[0],
            StoredEntry::from(entries[0].clone())
        );
        assert!(
            s.port()
                .serve(&log, 301, limits, &budget())
                .unwrap()
                .is_empty()
        );
        let unknown = LogKey {
            node: NodeUuid::from_bytes([9; 16]),
            incarnation: 1,
        };
        assert!(
            s.port()
                .serve(&unknown, 1, limits, &budget())
                .unwrap()
                .is_empty()
        );
        // A smaller entry limit.
        let three = BatchLimits {
            entries: 3,
            bytes: limits.bytes,
        };
        assert_eq!(s.port().serve(&log, 10, three, &budget()).unwrap().len(), 3);

        // 256 KiB: ~100 KB entries, so the byte limit stops after two.
        let id2 = NodeIdentity::from_seed([8; 32]);
        let log2 = remote_log(&id2);
        let fat = chain_with(&id2, 4, |c| {
            format!(r#"{{"n":{c},"pad":"{}"}}"#, "x".repeat(100_000))
        });
        assert!(matches!(
            ingest(s, &log2, &fat),
            IngestOutcome::Stored { stored: 4, .. }
        ));
        let by_bytes = s.port().serve(&log2, 2, limits, &budget()).unwrap();
        assert_eq!(
            by_bytes.len(),
            2,
            "two ~100 KB entries fit in 256 KiB, three do not"
        );
        let size: usize = by_bytes.iter().map(StoredEntry::size).sum();
        assert!(size <= limits.bytes);
        assert!(size + fat[3].canonical_bytes().len() > limits.bytes);

        // An entry bigger than the byte limit is still served alone.
        let tiny = BatchLimits {
            entries: 256,
            bytes: 1_000,
        };
        let oversized = s.port().serve(&log2, 2, tiny, &budget()).unwrap();
        assert_eq!(oversized, wrap(&fat[1..2]));
        assert!(fat[1].canonical_bytes().len() > tiny.bytes);
        let zero = BatchLimits {
            entries: 0,
            bytes: 0,
        };
        assert_eq!(s.port().serve(&log2, 3, zero, &budget()).unwrap().len(), 1);
    }

    pub fn ingest_stored(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 4);
        assert_eq!(
            ingest(s, &log, &entries[..2]),
            IngestOutcome::Stored {
                stored: 2,
                duplicates: 0,
                head: stored_head(2, &entries)
            }
        );
        assert_eq!(
            ingest(s, &log, &entries[2..]),
            IngestOutcome::Stored {
                stored: 2,
                duplicates: 0,
                head: stored_head(4, &entries)
            }
        );
        assert_eq!(
            ingest(s, &log, &[]),
            IngestOutcome::Stored {
                stored: 0,
                duplicates: 0,
                head: stored_head(4, &entries)
            }
        );
        assert_eq!(
            s.port()
                .serve(&log, 1, BatchLimits::default(), &budget())
                .unwrap(),
            wrap(&entries)
        );
    }

    pub fn ingest_duplicates(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        ingest(s, &log, &entries[..2]);
        assert_eq!(
            ingest(s, &log, &entries[..2]),
            IngestOutcome::Stored {
                stored: 0,
                duplicates: 2,
                head: stored_head(2, &entries)
            }
        );
        // Duplicates mixed with new entries.
        assert_eq!(
            ingest(s, &log, &entries),
            IngestOutcome::Stored {
                stored: 1,
                duplicates: 2,
                head: stored_head(3, &entries)
            }
        );
    }

    pub fn ingest_gap(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 5);
        // An unknown log must start at 1.
        assert_eq!(
            ingest(s, &log, &entries[1..3]),
            IngestOutcome::Gap {
                stored: 0,
                expected: 1
            }
        );
        assert_eq!(head(s, &log), None);
        // The valid prefix is kept, then the gap stops the batch.
        let skipping = [entries[0].clone(), entries[1].clone(), entries[3].clone()];
        assert_eq!(
            ingest(s, &log, &skipping),
            IngestOutcome::Gap {
                stored: 2,
                expected: 3
            }
        );
        assert_eq!(head(s, &log), Some(stored_head(2, &entries)));
    }

    pub fn ingest_fork_conflict(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        ingest(s, &log, &entries);
        // Same position, different signed content.
        let rival = seal(&id, &id, log, 2, entries[0].hash, r#"{"n":"rival"}"#.into());
        assert_ne!(rival.hash, entries[1].hash);
        assert_eq!(
            ingest(s, &log, std::slice::from_ref(&rival)),
            IngestOutcome::Fork {
                stored: 0,
                evidence: ForkEvidence::Conflict {
                    held: Box::new(StoredEntry::from(entries[1].clone())),
                    conflicting: Box::new(StoredEntry::from(rival)),
                }
            }
        );
        // The next counter with the wrong prev_hash, after a valid entry.
        let next = seal(&id, &id, log, 4, entries[2].hash, r#"{"n":4}"#.into());
        let wrong_prev = seal(&id, &id, log, 5, entries[0].hash, r#"{"n":5}"#.into());
        assert_eq!(
            ingest(s, &log, &[next.clone(), wrong_prev.clone()]),
            IngestOutcome::Fork {
                stored: 1,
                evidence: ForkEvidence::Conflict {
                    held: Box::new(StoredEntry::from(next.clone())),
                    conflicting: Box::new(StoredEntry::from(wrong_prev)),
                }
            }
        );
        assert_eq!(
            head(s, &log).unwrap().counter,
            4,
            "the prefix before the fork is kept"
        );
    }

    pub fn ingest_fork_ahead_of_self(s: &dyn Subject) {
        let own = s.self_log();
        let genesis = known(
            s.port()
                .serve(&own, 1, BatchLimits::default(), &budget())
                .unwrap(),
        );
        let id = own_identity();
        let claim = seal(&id, &id, own, 2, genesis[0].hash, r#"{"n":2}"#.into());
        assert_eq!(
            ingest(s, &own, std::slice::from_ref(&claim)),
            IngestOutcome::Fork {
                stored: 0,
                evidence: ForkEvidence::AheadOfSelf {
                    log: own,
                    my_head: stored_head(1, &genesis),
                    entry: Box::new(StoredEntry::from(claim)),
                }
            }
        );
        // An own entry the node already holds is a duplicate, not evidence.
        assert_eq!(
            ingest(s, &own, &genesis),
            IngestOutcome::Stored {
                stored: 0,
                duplicates: 1,
                head: stored_head(1, &genesis)
            }
        );
        assert_eq!(head(s, &own), Some(stored_head(1, &genesis)));
    }

    pub fn ingest_self_log_forged_is_corrupt(s: &dyn Subject) {
        let own = s.self_log();
        let genesis = known(
            s.port()
                .serve(&own, 1, BatchLimits::default(), &budget())
                .unwrap(),
        );
        let id = own_identity();
        let forged = seal(&id, &remote(), own, 2, genesis[0].hash, r#"{"n":2}"#.into());
        assert_eq!(
            ingest(s, &own, &[forged]),
            IngestOutcome::Corrupt {
                log: own,
                counter: 2,
                reason: EnvelopeError::BadSignature
            }
        );
        assert_eq!(head(s, &own), Some(stored_head(1, &genesis)));
    }

    pub fn ingest_frozen(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 5);
        ingest(s, &log, &entries[..3]);
        s.freeze(&log, 4);
        let frozen = head(s, &log).unwrap();
        assert_eq!(frozen.state, LogState::Frozen);
        assert_eq!(frozen.frozen_at, Some(4));
        // Up to the freeze counter the chain still grows; past it, nothing.
        assert_eq!(
            ingest(s, &log, &entries[3..]),
            IngestOutcome::Frozen {
                stored: 1,
                frozen_at: 4
            }
        );
        let after = head(s, &log).unwrap();
        assert_eq!((after.counter, after.state), (4, LogState::Frozen));
        assert_eq!(
            ingest(s, &log, &entries[4..]),
            IngestOutcome::Frozen {
                stored: 0,
                frozen_at: 4
            }
        );
        // Duplicates of held entries are still duplicates.
        assert!(matches!(
            ingest(s, &log, &entries[..2]),
            IngestOutcome::Stored {
                stored: 0,
                duplicates: 2,
                ..
            }
        ));
    }

    pub fn ingest_corrupt_hash_mismatch(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 4);
        ingest(s, &log, &entries[..2]);
        let mut tampered = entries[3].clone();
        tampered.payload = br#"{"n":"changed"}"#.to_vec();
        // An earlier good entry in the batch is rolled back too.
        assert_eq!(
            ingest(s, &log, &[entries[2].clone(), tampered]),
            IngestOutcome::Corrupt {
                log,
                counter: 4,
                reason: EnvelopeError::HashMismatch
            }
        );
        assert_eq!(
            head(s, &log),
            Some(stored_head(2, &entries)),
            "nothing stored"
        );

        // A tampered genesis of an unknown log stores nothing and creates no log.
        let id2 = NodeIdentity::from_seed([8; 32]);
        let log2 = remote_log(&id2);
        let mut genesis = chain(&id2, 1).remove(0);
        genesis.payload.push(b' ');
        assert!(matches!(
            ingest(s, &log2, &[genesis]),
            IngestOutcome::Corrupt {
                counter: 1,
                reason: EnvelopeError::HashMismatch,
                ..
            }
        ));
        assert_eq!(head(s, &log2), None);
    }

    pub fn ingest_corrupt_bad_signature(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        ingest(s, &log, &entries[..2]);
        let forger = NodeIdentity::from_seed([66; 32]);
        // Valid hash, signed by someone else: forged, never a fork.
        let forged_next = seal(&id, &forger, log, 3, entries[1].hash, r#"{"n":3}"#.into());
        assert_eq!(
            ingest(s, &log, &[forged_next]),
            IngestOutcome::Corrupt {
                log,
                counter: 3,
                reason: EnvelopeError::BadSignature
            }
        );
        let forged_rival = seal(&id, &forger, log, 2, entries[0].hash, r#"{"n":"x"}"#.into());
        assert_eq!(
            ingest(s, &log, &[forged_rival]),
            IngestOutcome::Corrupt {
                log,
                counter: 2,
                reason: EnvelopeError::BadSignature
            }
        );
        // A genesis signed by another key than it announces.
        let id2 = NodeIdentity::from_seed([8; 32]);
        let log2 = remote_log(&id2);
        let forged_genesis = seal(&id2, &forger, log2, 1, [0; 32], String::new());
        assert_eq!(
            ingest(s, &log2, &[forged_genesis]),
            IngestOutcome::Corrupt {
                log: log2,
                counter: 1,
                reason: EnvelopeError::BadSignature
            }
        );
        assert_eq!(head(s, &log), Some(stored_head(2, &entries)));
        assert_eq!(head(s, &log2), None);
    }

    pub fn ingest_batch_bounds(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 257);
        // 257 entries are over the batch bound; 256 are not.
        assert!(matches!(
            ingest(s, &log, &entries),
            IngestOutcome::Corrupt {
                counter: 0,
                reason: EnvelopeError::Malformed { field: "batch" },
                ..
            }
        ));
        assert_eq!(head(s, &log), None);
        assert!(matches!(
            ingest(s, &log, &entries[..256]),
            IngestOutcome::Stored { stored: 256, .. }
        ));
        // An entry of another log is refused.
        let id2 = NodeIdentity::from_seed([8; 32]);
        let stray = chain(&id2, 1);
        assert!(matches!(
            ingest(s, &log, &stray),
            IngestOutcome::Corrupt {
                counter: 1,
                reason: EnvelopeError::Malformed { field: "log" },
                ..
            }
        ));
    }

    pub fn stored_hash(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        let hash = |log: &LogKey, counter| s.port().stored_hash(log, counter, &budget()).unwrap();
        assert_eq!(hash(&log, 1), None, "unknown log");
        ingest(s, &log, &entries);
        for entry in &entries {
            assert_eq!(hash(&log, entry.counter), Some(entry.hash));
        }
        assert_eq!(hash(&log, 0), None);
        assert_eq!(hash(&log, 4), None);
        let own = s.self_log();
        let genesis = known(
            s.port()
                .serve(&own, 1, BatchLimits::default(), &budget())
                .unwrap(),
        );
        assert_eq!(hash(&own, 1), Some(genesis[0].hash));
    }

    pub fn apply_pending_marks_processed(s: &dyn Subject) {
        // The own genesis was applied when the node opened.
        let idle = s.port().apply_pending(&budget()).unwrap();
        assert_eq!((idle.applied, idle.has_more), (0, false), "{idle:?}");
        let id = remote();
        let log = remote_log(&id);
        ingest(s, &log, &chain(&id, 1));
        let progress = s.port().apply_pending(&budget()).unwrap();
        assert_eq!(
            (progress.applied, progress.failed, progress.has_more),
            (1, 0, false),
            "{progress:?}"
        );
        assert_eq!(s.port().apply_pending(&budget()).unwrap().applied, 0);
    }

    // ---- opaque entries: the port relays them and they join the chain ------

    pub fn contract_opaque_append_and_serve_round_trip(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 2);
        let opaque = opaque_at(&id, 3, entries[1].hash, "three");
        let mut batch = wrap(&entries);
        batch.push(opaque.clone());
        let held = LogHead {
            counter: 3,
            head_hash: opaque.hash(),
            state: LogState::Active,
            frozen_at: None,
        };
        assert_eq!(
            ingest_entries(s, &log, batch.clone()),
            IngestOutcome::Stored {
                stored: 3,
                duplicates: 0,
                head: held
            }
        );
        let served = s
            .port()
            .serve(&log, 1, BatchLimits::default(), &budget())
            .unwrap();
        assert_eq!(served, batch, "known and opaque, in order");
        let relayed = served[2].as_opaque().expect("the third entry is opaque");
        let sent = opaque.as_opaque().unwrap();
        assert_eq!(relayed.raw(), sent.raw(), "raw bytes are relayed unchanged");
        assert_eq!(relayed.hash(), sent.hash());
        assert_eq!(relayed.signature(), sent.signature());
        // The same entry again is a duplicate, not a fork.
        assert_eq!(
            ingest_entries(s, &log, vec![opaque]),
            IngestOutcome::Stored {
                stored: 0,
                duplicates: 1,
                head: held
            }
        );
    }

    pub fn contract_opaque_conflict_is_fork(s: &dyn Subject) {
        // A known entry held, an opaque rival at its position.
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        ingest(s, &log, &entries);
        let rival = opaque_at(&id, 3, entries[1].hash, "rival");
        assert_eq!(
            ingest_entries(s, &log, vec![rival.clone()]),
            IngestOutcome::Fork {
                stored: 0,
                evidence: ForkEvidence::Conflict {
                    held: Box::new(StoredEntry::from(entries[2].clone())),
                    conflicting: Box::new(rival),
                }
            }
        );

        // An opaque entry held, a known rival at its position.
        let id2 = NodeIdentity::from_seed([8; 32]);
        let log2 = remote_log(&id2);
        let entries2 = chain(&id2, 2);
        let held = opaque_at(&id2, 3, entries2[1].hash, "held");
        let mut batch = wrap(&entries2);
        batch.push(held.clone());
        ingest_entries(s, &log2, batch);
        let rival = seal(
            &id2,
            &id2,
            log2,
            3,
            entries2[1].hash,
            r#"{"n":"rival"}"#.into(),
        );
        assert_eq!(
            ingest(s, &log2, std::slice::from_ref(&rival)),
            IngestOutcome::Fork {
                stored: 0,
                evidence: ForkEvidence::Conflict {
                    held: Box::new(held),
                    conflicting: Box::new(StoredEntry::from(rival)),
                }
            }
        );
    }

    pub fn contract_opaque_counter_one_is_corrupt(s: &dyn Subject) {
        // An unknown log cannot start with an opaque entry: the genesis is
        // always a known layout, so counter 2 is a gap that wants counter 1.
        let id = remote();
        let log = remote_log(&id);
        let early = opaque_at(&id, 2, [1; 32], "two");
        assert_eq!(
            ingest_entries(s, &log, vec![early]),
            IngestOutcome::Gap {
                stored: 0,
                expected: 1
            }
        );
        assert_eq!(head(s, &log), None, "the log was not created");

        // `OpaqueEntry` itself refuses counter 1, so no opaque genesis attempt
        // reaches a port, held log or not.
        let mut genesis = synthetic_unsigned(&id, log.incarnation, 2, [0; 32]);
        genesis.counter = 1;
        let (header, raw) = synthetic_parts(&id, &genesis);
        assert!(OpaqueEntry::new(header, raw).is_err());
        let entries = chain(&id, 2);
        ingest(s, &log, &entries);
        assert_eq!(head(s, &log), Some(stored_head(2, &entries)));
    }

    pub fn contract_opaque_counts_toward_vector_head(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 2);
        let opaque = opaque_at(&id, 3, entries[1].hash, "three");
        let mut batch = wrap(&entries);
        batch.push(opaque.clone());
        ingest_entries(s, &log, batch);
        assert_eq!(
            head(s, &log),
            Some(LogHead {
                counter: 3,
                head_hash: opaque.hash(),
                state: LogState::Active,
                frozen_at: None,
            })
        );
        assert_eq!(
            s.port().stored_hash(&log, 3, &budget()).unwrap(),
            Some(opaque.hash())
        );
        assert_eq!(
            meta(s, &log).map(|m| (m.contiguous, m.head)),
            Some((3, opaque.hash()))
        );
    }

    pub fn contract_opaque_prev_continuity(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 2);
        let opaque = opaque_at(&id, 3, entries[1].hash, "three");
        let mut batch = wrap(&entries);
        batch.push(opaque.clone());
        ingest_entries(s, &log, batch);

        // A validly signed 4 that does not chain on the opaque 3 is a fork
        // against it.
        let wrong = seal(&id, &id, log, 4, entries[1].hash, r#"{"n":4}"#.into());
        assert_eq!(
            ingest(s, &log, std::slice::from_ref(&wrong)),
            IngestOutcome::Fork {
                stored: 0,
                evidence: ForkEvidence::Conflict {
                    held: Box::new(opaque.clone()),
                    conflicting: Box::new(StoredEntry::from(wrong)),
                }
            }
        );
        // One that names the opaque entry's hash appends.
        let next = seal(&id, &id, log, 4, opaque.hash(), r#"{"n":4}"#.into());
        assert_eq!(
            ingest(s, &log, std::slice::from_ref(&next)),
            IngestOutcome::Stored {
                stored: 1,
                duplicates: 0,
                head: LogHead {
                    counter: 4,
                    head_hash: next.hash,
                    state: LogState::Active,
                    frozen_at: None,
                }
            }
        );
    }

    pub fn contract_serve_branch_with_opaque_branch_row(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let main = chain(&id, 4);
        ingest(s, &log, &main);
        // An opaque rival at counter 3, recorded as a fork branch row; the
        // log freezes after counter 2.
        let rival = opaque_at(&id, 3, main[1].hash, "rival");
        s.store_forked(&log, std::slice::from_ref(&rival));
        s.freeze(&log, 2);

        let tip = (3, rival.hash());
        let served = s
            .port()
            .serve_branch(&log, tip, 1, BatchLimits::default(), &budget())
            .unwrap();
        let mut expected = wrap(&main[..2]);
        expected.push(rival.clone());
        assert_eq!(served, Some(expected), "the branch ends in the opaque row");
        // The main branch is unaffected.
        assert_eq!(
            counters_of(
                s.port()
                    .serve_branch(&log, tip_of(&main, 4), 1, BatchLimits::default(), &budget())
                    .unwrap()
            ),
            Some(vec![1, 2, 3, 4])
        );
        // Past the freeze nothing more is taken.
        let past = seal(&id, &id, log, 5, main[3].hash, r#"{"n":5}"#.into());
        assert_eq!(
            ingest(s, &log, std::slice::from_ref(&past)),
            IngestOutcome::Frozen {
                stored: 0,
                frozen_at: 2
            }
        );
    }

    // ---- serve_branch and log_meta ----------------------------------------

    /// A chain whose entries from `fork_at` on differ from `chain(id, n)`.
    fn branch(id: &NodeIdentity, n: u64, fork_at: u64) -> Vec<Envelope> {
        chain_with(id, n, |c| {
            if c >= fork_at {
                format!(r#"{{"branch":{c}}}"#)
            } else {
                format!(r#"{{"n":{c}}}"#)
            }
        })
    }

    fn tip_of(entries: &[Envelope], counter: u64) -> (u64, [u8; 32]) {
        (counter, entries[counter as usize - 1].hash)
    }

    fn served(
        s: &dyn Subject,
        log: &LogKey,
        tip: (u64, [u8; 32]),
        from: u64,
        limits: BatchLimits,
    ) -> Option<Vec<Envelope>> {
        s.port()
            .serve_branch(log, tip, from, limits, &budget())
            .unwrap()
            .map(known)
    }

    fn counters_of(batch: Option<Vec<StoredEntry>>) -> Option<Vec<u64>> {
        batch.map(|b| b.iter().map(StoredEntry::counter).collect())
    }

    fn counters(batch: Option<Vec<Envelope>>) -> Option<Vec<u64>> {
        batch.map(|b| b.iter().map(|e| e.counter).collect())
    }

    pub fn serve_branch_journal_range(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 10);
        ingest(s, &log, &entries);
        let all = BatchLimits::default();
        assert_eq!(
            served(s, &log, tip_of(&entries, 10), 1, all),
            Some(entries.clone())
        );
        assert_eq!(
            served(s, &log, tip_of(&entries, 10), 4, all),
            Some(entries[3..].to_vec())
        );
        // The branch ends at the tip, not at the head.
        assert_eq!(
            served(s, &log, tip_of(&entries, 6), 4, all),
            Some(entries[3..6].to_vec())
        );
        // Counter 0 reads as 1.
        assert_eq!(
            counters(served(s, &log, tip_of(&entries, 3), 0, all)),
            Some(vec![1, 2, 3])
        );
        // A known branch with nothing in range is empty, not `None`.
        assert_eq!(served(s, &log, tip_of(&entries, 10), 11, all), Some(vec![]));
    }

    pub fn serve_branch_forked_walk_into_journal(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let main = chain(&id, 5);
        ingest(s, &log, &main);
        // The other branch shares counters 1 and 2 and differs from 3 to 6.
        let other = branch(&id, 6, 3);
        assert_eq!(other[..2], main[..2]);
        s.store_forked(&log, &wrap(&other[2..]));
        let all = BatchLimits::default();
        let tip = tip_of(&other, 6);
        assert_eq!(served(s, &log, tip, 1, all), Some(other.clone()));
        assert_eq!(
            served(s, &log, tip, 3, all),
            Some(other[2..].to_vec()),
            "from at the first forked entry"
        );
        assert_eq!(
            served(s, &log, tip, 5, all),
            Some(other[4..].to_vec()),
            "from inside the forked part"
        );
        assert_eq!(
            served(s, &log, tip, 2, all),
            Some(other[1..].to_vec()),
            "from the last shared entry"
        );
        assert_eq!(served(s, &log, tip, 7, all), Some(vec![]));
        // A tip inside the forked part ends there.
        assert_eq!(
            served(s, &log, tip_of(&other, 4), 1, all),
            Some(other[..4].to_vec())
        );
        // The main branch still serves from `journal`.
        assert_eq!(
            served(s, &log, tip_of(&main, 5), 1, all),
            Some(main.clone())
        );
    }

    pub fn serve_branch_unknown_tip_none(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let main = chain(&id, 5);
        let other = branch(&id, 6, 3);
        let all = BatchLimits::default();
        assert_eq!(
            served(s, &log, tip_of(&main, 5), 1, all),
            None,
            "unknown log"
        );
        ingest(s, &log, &main);
        assert_eq!(
            served(s, &log, tip_of(&other, 6), 1, all),
            None,
            "a forked tip nobody stored"
        );
        assert_eq!(
            served(s, &log, (5, other[4].hash), 1, all),
            None,
            "a hash that is on no branch"
        );
        assert_eq!(
            served(s, &log, (9, main[4].hash), 1, all),
            None,
            "a counter beyond the head"
        );
        // A forked tip whose predecessor was never stored: a broken chain.
        s.store_forked(&log, &wrap(&other[4..]));
        assert_eq!(served(s, &log, tip_of(&other, 6), 1, all), None);
        assert_eq!(
            counters(served(s, &log, tip_of(&other, 6), 5, all)),
            Some(vec![5, 6]),
            "the walk stops at `from` before it needs the missing entry"
        );
    }

    pub fn serve_branch_over_walk_bound_none(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        ingest(s, &log, &chain(&id, 5));
        let other = branch(&id, 7, 3);
        s.store_forked(&log, &wrap(&other[2..]));
        let all = BatchLimits::default();
        let tip = tip_of(&other, 7);
        // Five forked entries (3..=7).
        s.set_walk_bound(4);
        assert_eq!(served(s, &log, tip, 1, all), None, "over the bound");
        assert_eq!(
            counters(served(s, &log, tip, 4, all)),
            Some(vec![4, 5, 6, 7]),
            "a walk of four steps fits"
        );
        s.set_walk_bound(5);
        assert_eq!(
            served(s, &log, tip, 1, all),
            Some(other.clone()),
            "exactly at the bound"
        );
    }

    pub fn serve_branch_limits_honoured(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let main = chain(&id, 300);
        for batch in main.chunks(100) {
            ingest(s, &log, batch);
        }
        let two = BatchLimits {
            entries: 2,
            bytes: BatchLimits::BYTES,
        };
        assert_eq!(
            counters(served(s, &log, tip_of(&main, 300), 10, two)),
            Some(vec![10, 11])
        );
        let default = served(s, &log, tip_of(&main, 300), 1, BatchLimits::default()).unwrap();
        assert_eq!(default.len(), BatchLimits::ENTRIES, "the 256-entry limit");
        let one_entry = main[1].canonical_bytes().len();
        let tight = BatchLimits {
            entries: 256,
            bytes: one_entry * 2 + one_entry / 2,
        };
        assert_eq!(
            counters(served(s, &log, tip_of(&main, 300), 2, tight)),
            Some(vec![2, 3])
        );
        let tiny = BatchLimits {
            entries: 256,
            bytes: 1,
        };
        assert_eq!(
            counters(served(s, &log, tip_of(&main, 300), 2, tiny)),
            Some(vec![2]),
            "at least one entry, even an oversized one"
        );
        // The same limits cut the forked walk, across the join.
        let other = branch(&id, 303, 299);
        s.store_forked(&log, &wrap(&other[298..]));
        let tip = tip_of(&other, 303);
        assert_eq!(
            counters(served(s, &log, tip, 297, BatchLimits::default())),
            Some(vec![297, 298, 299, 300, 301, 302, 303])
        );
        let four = BatchLimits {
            entries: 4,
            bytes: BatchLimits::BYTES,
        };
        assert_eq!(
            counters(served(s, &log, tip, 297, four)),
            Some(vec![297, 298, 299, 300])
        );
        assert_eq!(
            served(s, &log, tip, 297, four).unwrap().last().unwrap(),
            &other[299],
            "past the join the entries are the forked ones"
        );
        assert_eq!(
            counters(served(s, &log, tip, 299, four)),
            Some(vec![299, 300, 301, 302]),
        );
    }

    fn meta(s: &dyn Subject, log: &LogKey) -> Option<LogMeta> {
        s.port().log_meta(log, &budget()).unwrap()
    }

    pub fn log_meta_own_active(s: &dyn Subject) {
        let own = s.self_log();
        let genesis = known(
            s.port()
                .serve(&own, 1, BatchLimits::default(), &budget())
                .unwrap(),
        );
        assert_eq!(
            meta(s, &own),
            Some(LogMeta {
                pubkey: own_identity().public_key(),
                own: true,
                active: true,
                state: LogState::Active,
                frozen_at: None,
                contiguous: 1,
                head: genesis[0].hash,
            })
        );
    }

    pub fn log_meta_own_old_incarnation(s: &dyn Subject) {
        let id = own_identity();
        let active = s.self_log();
        let old = LogKey {
            node: id.uuid(),
            incarnation: active.incarnation + 1,
        };
        let mut entries: Vec<Envelope> = Vec::new();
        for counter in 1..=2 {
            let prev = entries.last().map_or([0; 32], |e| e.hash);
            entries.push(seal(&id, &id, old, counter, prev, r#"{"n":2}"#.into()));
        }
        assert!(matches!(
            ingest(s, &old, &entries),
            IngestOutcome::Stored { stored: 2, .. }
        ));
        assert_eq!(
            meta(s, &old),
            Some(LogMeta {
                pubkey: id.public_key(),
                own: true,
                active: false,
                state: LogState::Active,
                frozen_at: None,
                contiguous: 2,
                head: entries[1].hash,
            })
        );
    }

    pub fn log_meta_foreign(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 3);
        ingest(s, &log, &entries);
        assert_eq!(
            meta(s, &log),
            Some(LogMeta {
                pubkey: id.public_key(),
                own: false,
                active: false,
                state: LogState::Active,
                frozen_at: None,
                contiguous: 3,
                head: entries[2].hash,
            })
        );
    }

    pub fn log_meta_frozen(s: &dyn Subject) {
        let id = remote();
        let log = remote_log(&id);
        let entries = chain(&id, 4);
        ingest(s, &log, &entries);
        s.freeze(&log, 2);
        assert_eq!(
            meta(s, &log),
            Some(LogMeta {
                pubkey: id.public_key(),
                own: false,
                active: false,
                state: LogState::Frozen,
                frozen_at: Some(2),
                contiguous: 4,
                head: entries[3].hash,
            })
        );
    }

    pub fn log_meta_unknown_none(s: &dyn Subject) {
        let log = remote_log(&remote());
        assert_eq!(meta(s, &log), None);
        let own = s.self_log();
        let unknown_incarnation = LogKey {
            incarnation: own.incarnation + 7,
            ..own
        };
        assert_eq!(meta(s, &unknown_incarnation), None);
    }

    pub fn cancelled_call_is_an_error(s: &dyn Subject) {
        let cancelled = budget();
        cancelled.cancellation.cancel();
        let log = s.self_log();
        assert!(s.port().vector(&cancelled).is_err());
        assert!(s.port().stored_hash(&log, 1, &cancelled).is_err());
        assert!(
            s.port()
                .serve(&log, 1, BatchLimits::default(), &cancelled)
                .is_err()
        );
        assert!(s.port().ingest(&log, vec![], &cancelled).is_err());
        assert!(s.port().apply_pending(&cancelled).is_err());
        assert!(
            s.port()
                .serve_branch(&log, (1, [0; 32]), 1, BatchLimits::default(), &cancelled)
                .is_err()
        );
        assert!(s.port().log_meta(&log, &cancelled).is_err());
    }
}

macro_rules! contract {
    ($($case:ident),+ $(,)?) => {
        mod on_sqlite {
            $(#[test] fn $case() { super::cases::$case(&super::sqlite_subject()); })+
        }
        mod on_memory {
            $(#[test] fn $case() { super::cases::$case(&super::memory_subject()); })+
        }
    };
}

contract!(
    vector_after_author_and_ingest,
    serve_limits,
    ingest_stored,
    ingest_duplicates,
    ingest_gap,
    ingest_fork_conflict,
    ingest_fork_ahead_of_self,
    ingest_self_log_forged_is_corrupt,
    ingest_frozen,
    ingest_corrupt_hash_mismatch,
    ingest_corrupt_bad_signature,
    ingest_batch_bounds,
    stored_hash,
    apply_pending_marks_processed,
    cancelled_call_is_an_error,
    serve_branch_journal_range,
    serve_branch_forked_walk_into_journal,
    serve_branch_unknown_tip_none,
    serve_branch_over_walk_bound_none,
    serve_branch_limits_honoured,
    log_meta_own_active,
    log_meta_own_old_incarnation,
    log_meta_foreign,
    log_meta_frozen,
    log_meta_unknown_none,
    contract_opaque_append_and_serve_round_trip,
    contract_opaque_conflict_is_fork,
    contract_opaque_counter_one_is_corrupt,
    contract_opaque_counts_toward_vector_head,
    contract_opaque_prev_continuity,
    contract_serve_branch_with_opaque_branch_row,
);

// ---- one-implementation cases ------------------------------------------------

/// `Headroom` is a disk condition: `MemoryJournal` has no disk and never
/// returns it, so this case runs on `SqliteStore` only.
#[test]
fn ingest_headroom_sqlite_only() {
    let s = sqlite_subject();
    let id = remote();
    let log = remote_log(&id);
    let entries = chain(&id, 2);
    s.store.context().set_ingest_headroom(u64::MAX);
    assert_eq!(ingest(&s, &log, &entries), IngestOutcome::Headroom);
    assert_eq!(head(&s, &log), None, "nothing stored");
    s.store.context().set_ingest_headroom(0);
    assert!(matches!(
        ingest(&s, &log, &entries),
        IngestOutcome::Stored { stored: 2, .. }
    ));
}

/// Entries sealed by `author_raw` are what a real node accepts.
#[test]
fn author_raw_entries_ingest_into_sqlite_as_stored() {
    let author = MemoryJournal::new(NodeIdentity::from_seed([21; 32]));
    let first = author.author_raw("test.note", r#"{"n":2}"#).unwrap();
    let second = author.author_raw("test.note", r#"{"n":3}"#).unwrap();
    let log = author.self_log();
    assert_eq!(
        (first.counter, second.counter),
        (2, 3),
        "genesis came first"
    );
    assert_eq!(second.prev_hash, first.hash);
    let all = known(
        author
            .serve(&log, 1, BatchLimits::default(), &budget())
            .unwrap(),
    );
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].kind, "node.genesis");
    all[2].verify(&author_pubkey(&author)).unwrap();

    let sqlite = sqlite_subject();
    match ingest(&sqlite, &log, &all) {
        IngestOutcome::Stored {
            stored: 3,
            duplicates: 0,
            head,
        } => {
            assert_eq!((head.counter, head.head_hash), (3, second.hash));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sqlite.port().apply_pending(&budget()).unwrap().applied, 1);

    // And a second MemoryJournal takes them the same way.
    let memory = memory_subject();
    assert!(matches!(
        ingest(&memory, &log, &all),
        IngestOutcome::Stored { stored: 3, .. }
    ));
}

fn author_pubkey(journal: &MemoryJournal) -> [u8; 32] {
    let genesis = journal
        .serve(&journal.self_log(), 1, BatchLimits::default(), &budget())
        .unwrap()
        .remove(0);
    genesis.genesis_pubkey().unwrap()
}

/// `MemoryJournal` verifies: it refuses what `check_next` refuses, so a link
/// test cannot pass on data a real node rejects.
#[test]
fn memory_journal_never_skips_verification() {
    let memory = memory_subject();
    let id = remote();
    let log = remote_log(&id);
    let entries = chain(&id, 2);
    let forger = NodeIdentity::from_seed([66; 32]);
    let forged = seal(&id, &forger, log, 2, entries[0].hash, r#"{"n":2}"#.into());
    ingest(&memory, &log, &entries[..1]);
    let held = HeldLog {
        pubkey: id.public_key(),
        contiguous: 1,
        head_hash: entries[0].hash,
        state: LogState::Active,
        frozen_at: None,
        is_active_self: false,
        stored_hash: None,
    };
    assert_eq!(
        check_next(Some(&held), &StoredEntry::from(forged.clone())),
        ChainStep::Corrupt(EnvelopeError::BadSignature)
    );
    assert!(matches!(
        ingest(&memory, &log, &[forged]),
        IngestOutcome::Corrupt {
            reason: EnvelopeError::BadSignature,
            ..
        }
    ));
    assert_eq!(head(&memory, &log).unwrap().counter, 1);
}

#[test]
fn memory_journal_author_raw_refuses_bad_shapes_and_self_freeze() {
    let memory = MemoryJournal::new(own_identity());
    assert!(memory.author_raw("not a kind", "{}").is_err());
    assert!(memory.author_raw("node.genesis", "{}").is_err());
    assert!(memory.author_raw("test.note", "not json").is_err());
    let log = memory.self_log();
    let entry = memory.author_raw("test.note", "{}").unwrap();
    assert_eq!(entry.counter, 2);
    assert!(memory.freeze(&log, 1).is_err(), "the own log never freezes");
    let unknown = remote_log(&remote());
    assert!(memory.freeze(&unknown, 1).is_err());
}
