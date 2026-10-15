//! `fact_attribution_staging`: stage, take and the fallback observation.

use super::*;

use rusqlite::Connection;

use crate::protocol::authority::Harness;
use crate::protocol::ids::{BindingId, ExecutionId, HostBootId, NativeSessionId};
use crate::protocol::time::{MonoInstant, UtcMillis};
use crate::store::connection::DecisionInstant;

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    crate::store::schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
        .unwrap();
    db
}

fn actor() -> AccountableActor {
    AccountableActor {
        harness: Harness::Claude,
        native_session: NativeSessionId::new("sess"),
        execution: ExecutionId::new("exec"),
        host_boot: HostBootId::new("boot"),
        target_generation: 2,
        binding_generation: 7,
        observed_at_utc: UtcMillis(5),
        provenance: "cooperative_top_level",
        binding_id: BindingId::new("b-1"),
        binding_connected: true,
    }
}

fn input(local_origin: bool, provenance: &'static str) -> MaterializeInput<'static> {
    MaterializeInput {
        decision: DecisionInstant {
            utc: UtcMillis(9),
            monotonic: MonoInstant(9),
        },
        decision_seq: 1,
        origin_node: crate::test_support::test_node_seed().uuid,
        origin_ms: 1234,
        local_origin,
        provenance,
        suppress: &|_, _| Ok(false),
    }
}

fn staged_rows(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM fact_attribution_staging", [], |r| {
        r.get(0)
    })
    .unwrap()
}

// Kills: a take that leaves the row behind, returns the wrong columns, or
// falls back although a row was staged.
#[test]
fn take_returns_and_deletes_staged() {
    let db = db();
    let tx = db.unchecked_transaction().unwrap();
    let observation = actor().observation(42);
    let key = send_key("m1");
    stage(&tx, &key, &actor(), &observation).unwrap();
    assert_eq!(staged_rows(&db), 1);
    let taken = take(&tx, &input(true, "local_journal"), &key).unwrap();
    assert_eq!(
        taken,
        Attribution {
            binding_id: Some("b-1".into()),
            generation: 7,
            observation,
        }
    );
    assert_eq!(staged_rows(&db), 0, "take deletes the staged row");
    // A second take finds nothing and falls back.
    let again = take(&tx, &input(true, "local_journal"), &key).unwrap();
    assert_eq!(again.binding_id, None);
}

// Kills: a fallback that carries a binding, a nonzero generation, or the
// wrong provenance or timestamp for another node's fact.
#[test]
fn fallback_replicated_from_for_remote() {
    let db = db();
    let tx = db.unchecked_transaction().unwrap();
    let taken = take(
        &tx,
        &input(false, "replicated_from:0123456789abcdef0123456789abcdef"),
        &ack_key("m1", "s1"),
    )
    .unwrap();
    assert_eq!(taken.binding_id, None);
    assert_eq!(taken.generation, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&taken.observation).unwrap(),
        serde_json::json!({
            "provenance": "replicated_from:0123456789abcdef0123456789abcdef",
            "decided_at": 1234
        })
    );
}

// Kills: a fallback that records the undefined value `local` (or
// `replicated_from:`) for this node's own entry with nothing staged.
#[test]
fn fallback_local_journal_for_own_old_incarnation() {
    let db = db();
    let tx = db.unchecked_transaction().unwrap();
    for provenance in ["local_journal", "local"] {
        let taken = take(&tx, &input(true, provenance), &answer_key("i1")).unwrap();
        assert_eq!(taken.binding_id, None, "{provenance}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&taken.observation).unwrap(),
            serde_json::json!({"provenance": "local_journal", "decided_at": 1234}),
            "{provenance}"
        );
    }
}

// Kills: key builders that drift from the documented spellings, or a staging
// table that collides two keys.
#[test]
fn staging_keys_and_independent_rows() {
    assert_eq!(send_key("m1"), "send:m1");
    assert_eq!(ack_key("m1", "s2"), "ack:m1/s2");
    assert_eq!(answer_key("i3"), "answer:i3");
    assert_eq!(join_key("t4", "s5", 6), "join:t4/s5/6");
    let db = db();
    let tx = db.unchecked_transaction().unwrap();
    stage(&tx, &send_key("m1"), &actor(), "{}").unwrap();
    stage(&tx, &send_key("m2"), &actor(), "{}").unwrap();
    // Re-staging one key replaces it rather than failing.
    stage(&tx, &send_key("m1"), &actor(), r#"{"a":1}"#).unwrap();
    assert_eq!(staged_rows(&db), 2);
    let first = take(&tx, &input(true, "local_journal"), &send_key("m1")).unwrap();
    assert_eq!(first.observation, r#"{"a":1}"#);
    assert_eq!(staged_rows(&db), 1, "the other key is untouched");
}
