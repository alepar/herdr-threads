//! Two-store convergence of `seat.created` (fact-kinds design section 4):
//! the remote row, discovery without prior interaction, and own-seat restore
//! after a wipe.

use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::test_support::fact_exchange;

use crate::facts_seat::{Node, registry};

fn seat_created_ms(node: &Node) -> i64 {
    let hlc: i64 = node.query(
        "SELECT hlc FROM journal WHERE kind='seat.created' ORDER BY ingest_seq",
        |r| r.get(0),
    )[0];
    i64::try_from(HlcStamp::from_u64(u64::try_from(hlc).unwrap()).ms()).unwrap()
}

#[test]
fn b_holds_a_seat_as_remote_after_sync() {
    let reg = registry();
    let (a, b) = (Node::with(41, reg), Node::with(42, reg));
    let seat = a.mint_seat(0);
    a.add_local_seat(&seat);
    a.enroll(&seat, "codex", "session-1").unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let row = b.seat(&seat).expect("B holds A's seat");
    assert_eq!(row.0, "remote");
    assert!(!row.1, "is_local = 0");
    assert_eq!(row.2.as_deref(), Some("agent"));
    assert!(row.3, "published = 1");
    assert_eq!(
        row.4,
        seat_created_ms(&a),
        "created_at is the origin's sent-at"
    );
    assert_eq!(row.5, a.id.to_string(), "homed on A");
    // A's own row is the local one, now published.
    let own = a.seat(&seat).unwrap();
    assert_eq!((own.0.as_str(), own.1, own.3), ("resolved", true, true));
}

#[test]
fn seat_enrolled_on_b_listed_on_a_without_interaction() {
    let reg = registry();
    let (a, b) = (Node::with(43, reg), Node::with(44, reg));
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    // Resolved to an occupant on B; nothing ever references it in a fact.
    b.enroll(&seat, "human", "session-h").unwrap();
    assert_eq!(
        b.journal_kinds()
            .iter()
            .filter(|k| *k != "node.genesis")
            .count(),
        1
    );
    fact_exchange::sync(&a.store, &b.store);
    let row = a.seat(&seat).expect("A lists B's seat");
    assert_eq!((row.0.as_str(), row.1, row.3), ("remote", false, true));
    assert_eq!(
        row.2.as_deref(),
        Some("human"),
        "kind from the binding role"
    );
}

/// A wiped store of the same node: same identity, another incarnation.
fn wiped(seed: u8, host: bool) -> Node {
    Node::open(seed, 999, registry(), host)
}

#[test]
fn own_seat_restore_unresolved_on_latest_host() {
    let reg = registry();
    let old = Node::with(45, reg);
    let seat = old.mint_seat(0);
    old.add_local_seat(&seat);
    old.enroll(&seat, "codex", "session-1").unwrap();

    let fresh = wiped(45, false);
    assert_eq!(fresh.id, old.id, "the same node");
    let ctx = fresh.ctx();
    ctx.attach_host("h-low-id").unwrap();
    ctx.attach_host("h-high-id").unwrap();
    // The lower host_id is the latest attached, so ordering by id cannot pass.
    fresh
        .raw()
        .execute_batch(
            "UPDATE herdr_hosts SET last_attached_at=300 WHERE endpoint='h-low-id'; \
             UPDATE herdr_hosts SET last_attached_at=200 WHERE endpoint='h-high-id';",
        )
        .unwrap();
    let latest: i64 = fresh.query(
        "SELECT host_id FROM herdr_hosts WHERE endpoint='h-low-id'",
        |r| r.get(0),
    )[0];

    fact_exchange::sync(&old.store, &fresh.store);

    let rows = fresh.query(
        &format!(
            "SELECT state,unresolved_reason,is_local,published,host_id,remote_kind FROM seats WHERE id='{}'",
            seat.as_str()
        ),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        },
    );
    assert_eq!(
        rows,
        vec![(
            "unresolved".to_owned(),
            Some("other".to_owned()),
            true,
            true,
            latest,
            None
        )]
    );
    assert!(
        fresh
            .journal()
            .iter()
            .filter(|(k, _, _)| k == "seat.created")
            .all(|(_, state, _)| state == "applied")
    );
}

#[test]
fn own_seat_restore_without_host_failed() {
    let reg = registry();
    let old = Node::with(46, reg);
    let seat = old.mint_seat(0);
    old.add_local_seat(&seat);
    old.enroll(&seat, "codex", "session-1").unwrap();

    let fresh = wiped(46, false);
    assert!(
        fresh
            .query("SELECT 1 FROM herdr_hosts", |r| r.get::<_, i64>(0))
            .is_empty(),
        "no host row"
    );
    fact_exchange::sync(&old.store, &fresh.store);

    let created: Vec<_> = fresh
        .journal()
        .into_iter()
        .filter(|(k, _, _)| k == "seat.created")
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].1, "failed");
    assert!(
        created[0]
            .2
            .as_deref()
            .unwrap()
            .contains("own seat restore needs an attached host"),
        "{:?}",
        created[0].2
    );
    assert!(fresh.seat(&seat).is_none(), "no row inserted");
}
