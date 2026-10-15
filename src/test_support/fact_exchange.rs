//! Two-store fact exchange for convergence tests.
//!
//! Responsibility
//! - Owns: [`sync`], which moves every log between two seeded `SqliteStore`s
//!   through `JournalPort::serve` -> `ingest` -> `apply_pending` until both
//!   vectors agree, and drains both stores' apply passes.
//! - Hides: batching (`BatchLimits::default()`), the call budget, and the
//!   round loop that picks up logs a store only learns of mid-exchange.
//! - Must not: spawn a daemon or dial a peer, touch the journal tables
//!   directly, know any kind's semantics, or tolerate a protocol outcome
//!   other than `Stored` (a gap, fork or corrupt batch panics the test).
//! - Role: test harness. Every two-node convergence test of a fact kind
//!   (ht-5fr.4 leaves) authors on one or both stores and calls `sync(a, b)`;
//!   stores come from `StoreContext::with_journal_seed` or
//!   `journal_oracle::OracleStore::open` (distinct seeds, same registry).

use crate::protocol::time::{CallBudget, Cancellation, MonoInstant};
use crate::store::SqliteStore;
use crate::sync::journal_port::{IngestOutcome, JournalPort};
use crate::sync::{BatchLimits, LogKey};

/// Rounds before the exchange gives up; each round ships every missing entry
/// both ways and drains the apply passes, so a healthy pair needs two.
const MAX_ROUNDS: usize = 16;

/// Apply passes per drain before giving up.
const MAX_APPLY_PASSES: usize = 100_000;

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// `(log, highest contiguous counter, head hash)` per held log.
fn heads(store: &SqliteStore) -> Vec<(LogKey, u64, [u8; 32])> {
    store
        .vector(&budget())
        .expect("read a store vector")
        .0
        .into_iter()
        .map(|(log, head)| (log, head.counter, head.head_hash))
        .collect()
}

fn counter_of(heads: &[(LogKey, u64, [u8; 32])], log: &LogKey) -> u64 {
    heads
        .iter()
        .find(|(l, _, _)| l == log)
        .map_or(0, |(_, counter, _)| *counter)
}

/// Ships every entry `to` lacks from `from`, log by log.
fn ship_missing(from: &SqliteStore, to: &SqliteStore) {
    let have = heads(to);
    for (log, head, _) in heads(from) {
        let mut next = counter_of(&have, &log) + 1;
        while next <= head {
            let batch = from
                .serve(&log, next, BatchLimits::default(), &budget())
                .expect("serve a log");
            let Some(last) = batch.last().map(|e| e.counter) else {
                panic!("log {log:?} ends before its head {head}");
            };
            match to.ingest(&log, batch, &budget()).expect("ingest a batch") {
                IngestOutcome::Stored { .. } => {}
                other => panic!("ingest of {log:?} from {next}: {other:?}"),
            }
            next = last + 1;
        }
    }
}

fn drain(store: &SqliteStore) {
    for _ in 0..MAX_APPLY_PASSES {
        if !store
            .apply_pending(&budget())
            .expect("apply pending entries")
            .has_more
        {
            return;
        }
    }
    panic!("a store never drained its apply passes");
}

/// Brings `a` and `b` to equal vectors with everything applied.
///
/// Panics when they do not agree after [`MAX_ROUNDS`] rounds (a kind that
/// authors a follow-up on every apply would never settle) or when an ingest
/// is refused.
pub fn sync(a: &SqliteStore, b: &SqliteStore) {
    for _ in 0..MAX_ROUNDS {
        ship_missing(a, b);
        ship_missing(b, a);
        drain(a);
        drain(b);
        if heads(a) == heads(b) {
            return;
        }
    }
    panic!(
        "stores did not converge in {MAX_ROUNDS} rounds: {:?} vs {:?}",
        heads(a),
        heads(b)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use rusqlite::Transaction;
    use serde::{Deserialize, Serialize};

    use crate::protocol::ids::ThreadId;
    use crate::store::connection::store_error;
    use crate::store::journal;
    use crate::store::thread_registers::{self, RegisterField};
    use crate::sync::EntityKey;
    use crate::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
    use crate::test_support::journal_oracle::OracleStore;

    #[derive(Serialize, Deserialize)]
    struct Topic {
        thread: String,
        value: String,
    }

    struct TopicKind;
    impl FactKind for TopicKind {
        const NAME: &'static str = "exchange.topic";
        const MAJOR: u32 = 1;
        type Payload = Topic;
        fn creates(_: &Topic) -> Option<EntityKey> {
            None
        }
        fn needs(_: &Topic) -> Vec<EntityKey> {
            Vec::new()
        }
        fn apply(
            tx: &Transaction<'_>,
            cx: &mut ApplyContext<'_>,
            p: Topic,
        ) -> Result<(), FactError> {
            thread_registers::upsert(
                tx,
                &ThreadId::parse(p.thread).expect("a thread id"),
                RegisterField::Topic,
                Some(&p.value),
                cx.hlc,
                cx.origin.node,
                cx.ingest_seq,
            )
            .map(|_| ())
            .map_err(|e| FactError::from_api(store_error(e)))
        }
    }

    fn author(store: &SqliteStore, thread: &str, value: &str) {
        let ctx = store.context();
        let mut conn = ctx.open_writer().unwrap();
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| {
                journal::author::<TopicKind>(
                    tx,
                    ctx,
                    d,
                    &Topic {
                        thread: thread.into(),
                        value: value.into(),
                    },
                )
            },
        )
        .unwrap();
    }

    fn topics(store: &SqliteStore) -> Vec<(String, String)> {
        let db = store.context().open_query(budget()).unwrap();
        let mut stmt = db
            .prepare("SELECT thread_id,value FROM thread_registers WHERE field='topic' ORDER BY thread_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    // Kills: a sync that ships one direction only, skips the apply drain, or
    // returns before the vectors agree.
    #[test]
    fn sync_brings_two_seeded_stores_to_equal_vectors() {
        let registry = FactRegistry::builtins_plus(|b| b.with::<TopicKind>());
        let a = OracleStore::open([31; 32], registry);
        let b = OracleStore::open([32; 32], registry);
        author(&a, "t1", "from a");
        author(&a, "t2", "from a too");
        author(&b, "t3", "from b");
        assert_ne!(heads(&a), heads(&b), "the stores start apart");
        sync(&a, &b);
        assert_eq!(heads(&a), heads(&b));
        assert_eq!(heads(&a).len(), 2, "each store holds both logs");
        let expected = vec![
            ("t1".to_owned(), "from a".to_owned()),
            ("t2".to_owned(), "from a too".to_owned()),
            ("t3".to_owned(), "from b".to_owned()),
        ];
        assert_eq!(topics(&a), expected);
        assert_eq!(topics(&b), expected, "both stores applied every entry");
    }
}
