use super::*;

use crate::protocol::time::UtcMillis;
use crate::store::schema;

const NODE_A: &str = "00000000000000000000000000000001";
const NODE_ZERO: &str = "00000000000000000000000000000000";
const NODE_B: &str = "00000000000000000000000000000002";

fn node(text: &str) -> NodeUuid {
    text.parse().unwrap()
}

fn thread() -> ThreadId {
    ThreadId::new("t")
}

/// A fresh store with one self log and `n` journal fixture rows
/// (`ingest_seq` 1..=n) for `src_ingest_seq` to reference.
fn db_with_journal(n: i64) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    db.execute(
        "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         SELECT 1,id,1,1,zeroblob(32),zeroblob(32),?1,zeroblob(32),0 FROM local_node",
        [n],
    )
    .unwrap();
    for counter in 1..=n {
        db.execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,ingested_at,apply_state) \
             VALUES (1,?1,zeroblob(32),zeroblob(32),10,'k',1,x'',x'00',zeroblob(64),0,'pending')",
            [counter],
        )
        .unwrap();
    }
    db
}

fn stored(db: &Connection) -> (Option<String>, i64, String, i64) {
    db.query_row(
        "SELECT value, hlc, node_uuid, src_ingest_seq FROM thread_registers WHERE thread_id='t' AND field='topic'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

/// Four writes with distinct `(hlc, node)`; the winner is hlc 7 from NODE_A.
fn writes() -> [(&'static str, u64, &'static str, i64); 4] {
    [
        ("w1", 5, NODE_B, 1),
        ("w2", 7, NODE_A, 2),
        ("w3", 7, NODE_ZERO, 3),
        ("w4", 3, NODE_B, 4),
    ]
}

fn permutations(items: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == items.len() {
        out.push(items.clone());
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permutations(items, k + 1, out);
        items.swap(k, i);
    }
}

#[test]
fn all_24_arrival_orders_converge() {
    let writes = writes();
    let mut orders = Vec::new();
    permutations(&mut vec![0, 1, 2, 3], 0, &mut orders);
    assert_eq!(orders.len(), 24);
    for order in orders {
        let db = db_with_journal(4);
        let mut wins = 0;
        for ix in &order {
            let (value, hlc, n, seq) = writes[*ix];
            let outcome = upsert(
                &db,
                &thread(),
                RegisterField::Topic,
                Some(value),
                HlcStamp::from_u64(hlc),
                node(n),
                seq,
            )
            .unwrap();
            if outcome == RegisterOutcome::Won {
                wins += 1;
            }
        }
        assert!(wins >= 1, "order {order:?}");
        // Winner: hlc 7, higher node (NODE_A beats the zero-ish w3 node).
        assert_eq!(
            stored(&db),
            (Some("w2".to_string()), 7, NODE_A.to_string(), 2),
            "order {order:?}"
        );
    }
}

#[test]
fn equal_stamp_loses() {
    let db = db_with_journal(2);
    let stamp = HlcStamp::from_u64(9);
    let first = upsert(
        &db,
        &thread(),
        RegisterField::Name,
        Some("a"),
        stamp,
        node(NODE_A),
        1,
    )
    .unwrap();
    let second = upsert(
        &db,
        &thread(),
        RegisterField::Name,
        Some("b"),
        stamp,
        node(NODE_A),
        2,
    )
    .unwrap();
    assert_eq!(first, RegisterOutcome::Won);
    assert_eq!(second, RegisterOutcome::Lost);
    let (value, seq): (Option<String>, i64) = db
        .query_row(
            "SELECT value, src_ingest_seq FROM thread_registers WHERE field='name'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((value.as_deref(), seq), (Some("a"), 1));
}

#[test]
fn same_hlc_higher_node_wins_and_fields_are_independent() {
    let db = db_with_journal(3);
    let stamp = HlcStamp::from_u64(4);
    let w =
        |field, value, n, seq| upsert(&db, &thread(), field, value, stamp, node(n), seq).unwrap();
    assert_eq!(
        w(RegisterField::Topic, Some("a"), NODE_A, 1),
        RegisterOutcome::Won
    );
    assert_eq!(
        w(RegisterField::Topic, Some("b"), NODE_B, 2),
        RegisterOutcome::Won
    );
    assert_eq!(
        w(RegisterField::Topic, Some("c"), NODE_A, 3),
        RegisterOutcome::Lost
    );
    assert_eq!(
        w(RegisterField::Archived, None, NODE_A, 3),
        RegisterOutcome::Won
    );
    assert_eq!(stored(&db).0.as_deref(), Some("b"));
    assert_eq!(
        current(&db, &thread(), RegisterField::Archived).unwrap(),
        Some((stamp, node(NODE_A)))
    );
}

#[test]
fn raw_sql_regression_refused_by_trigger() {
    let db = db_with_journal(1);
    upsert(
        &db,
        &thread(),
        RegisterField::Topic,
        Some("x"),
        HlcStamp::from_u64(5),
        node(NODE_B),
        1,
    )
    .unwrap();
    let err = db
        .execute(
            "UPDATE thread_registers SET value='y', hlc=4, node_uuid=?1",
            [NODE_B],
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("thread_registers_lww"), "{err}");
    assert_eq!(stored(&db).0.as_deref(), Some("x"));
}

#[test]
fn current_returns_winner_stamp() {
    let db = db_with_journal(2);
    assert_eq!(current(&db, &thread(), RegisterField::Topic).unwrap(), None);
    upsert(
        &db,
        &thread(),
        RegisterField::Topic,
        Some("a"),
        HlcStamp::from_u64(3),
        node(NODE_A),
        1,
    )
    .unwrap();
    upsert(
        &db,
        &thread(),
        RegisterField::Topic,
        Some("b"),
        HlcStamp::from_u64(8),
        node(NODE_B),
        2,
    )
    .unwrap();
    upsert(
        &db,
        &thread(),
        RegisterField::Topic,
        Some("c"),
        HlcStamp::from_u64(6),
        node(NODE_A),
        1,
    )
    .unwrap();
    assert_eq!(
        current(&db, &thread(), RegisterField::Topic).unwrap(),
        Some((HlcStamp::from_u64(8), node(NODE_B)))
    );
}
