//! Two- and three-store convergence of the thread facts (fact-kinds design
//! section 5): LWW registers, the `thread.direct` min-register and the
//! order-independence invariant (root section 5).

use herdr_threads::protocol::ids::ThreadId;
use herdr_threads::protocol::time::{CallBudget, Cancellation, MonoInstant, UtcMillis};
use herdr_threads::store::archival::{self, Runtime};
use herdr_threads::sync::BatchLimits;
use herdr_threads::sync::facts::payload::RegisterField;
use herdr_threads::sync::journal_port::{IngestOutcome, JournalPort};
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::journal_oracle::{
    assert_replays_equal, replicated_projection, replicated_tables, strict_violations,
};

use crate::facts_seat::{Node, registry};
use crate::facts_thread::sync_all;

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// Keys of the register events of one field, oldest first.
fn register_events(node: &Node, thread: &ThreadId, field: &str) -> Vec<String> {
    node.event_keys(thread)
        .into_iter()
        .filter(|k| k.starts_with(&format!("register:{}:{field}:", thread.as_str())))
        .collect()
}

/// A, B on one shared, open thread created by A.
fn shared_thread(seed: u8) -> (Node, Node, ThreadId) {
    let reg = registry();
    let (a, b) = (Node::with(seed, reg), Node::with(seed + 1, reg));
    let seat = a.local_seat();
    let thread = a.create_thread(&seat, "start", None).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    assert!(b.thread_row(&thread).is_some(), "B holds the thread");
    (a, b, thread)
}

#[test]
fn concurrent_topic_converges_one_event_on_loser() {
    let (a, b, thread) = shared_thread(61);
    a.write_register(&thread, RegisterField::Topic, Some("from a"))
        .unwrap();
    b.write_register(&thread, RegisterField::Topic, Some("from b"))
        .unwrap();
    let (a_stamp, b_stamp) = (
        a.stamp_of("thread.register", 0),
        b.stamp_of("thread.register", 0),
    );
    assert_ne!(a_stamp, b_stamp);
    let (winner, loser, want) = if a_stamp > b_stamp {
        (&a, &b, "from a")
    } else {
        (&b, &a, "from b")
    };
    let before = (
        register_events(winner, &thread, "topic").len(),
        register_events(loser, &thread, "topic").len(),
    );
    assert_eq!(before, (1, 1), "each applied its own write");

    fact_exchange::sync(&a.store, &b.store);

    for node in [&a, &b] {
        assert_eq!(
            node.thread_row(&thread).map(|r| r.0).as_deref(),
            Some(want),
            "the higher (hlc, node) wins on both"
        );
        let (value, hlc, node_id) = node.register(&thread, "topic").unwrap();
        assert_eq!(value.as_deref(), Some(want));
        assert_eq!((hlc, node_id), a_stamp.clone().max(b_stamp.clone()));
    }
    assert_eq!(
        register_events(winner, &thread, "topic").len(),
        1,
        "the losing write appended nothing on the winner"
    );
    let events = register_events(loser, &thread, "topic");
    assert_eq!(
        events.len(),
        2,
        "the loser gained exactly the winner's event"
    );
    // The key names the origin entry: register:<thread>:<field>:<uuid8>:<counter>.
    let winner_id = winner.id.to_string();
    assert!(
        events[1].contains(&format!(":topic:{}:", &winner_id[..8])),
        "{events:?}"
    );
}

#[test]
fn concurrent_archive_reopen_converge() {
    let (a, b, thread) = shared_thread(63);
    a.write_register(&thread, RegisterField::Archived, Some("1"))
        .unwrap();
    b.write_register(&thread, RegisterField::Archived, Some("0"))
        .unwrap();
    // `reopen` on an open thread is a no-op for the thread but the write is
    // still a register fact; `archive` set it on A only.
    let stamps = (
        a.stamp_of("thread.register", 0),
        b.stamp_of("thread.register", 0),
    );
    let want_archived = stamps.0 > stamps.1;
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        assert_eq!(
            node.thread_row(&thread).map(|r| r.2),
            Some(want_archived),
            "both nodes agree on the higher write"
        );
    }
    assert_eq!(a.thread_projection(), b.thread_projection());
    // The archive/reopen events: one per actual change of the column.
    let a_events = register_events(&a, &thread, "archived");
    let b_events = register_events(&b, &thread, "archived");
    if want_archived {
        // A's archive won: A changed once; B saw no change from its own
        // reopen (already open), then archived on A's fact.
        assert_eq!((a_events.len(), b_events.len()), (1, 1));
    } else {
        // B's reopen won over A's archive: A archived, then reopened.
        assert_eq!((a_events.len(), b_events.len()), (2, 0));
    }
}

#[test]
fn automatic_archive_replicates_as_register() {
    let (a, b, thread) = shared_thread(65);
    // The thread has no joined member, so the lane archives it after the
    // grace. Tick the lane on A at increasing instants.
    let instance = a.id.to_string();
    let conn = a.raw();
    let host = herdr_threads::test_support::test_host_evidence(0, true);
    let mut archived = false;
    for at in (0..=120_000i64).step_by(60_000) {
        let rt = Runtime {
            boot: "boot".into(),
            mono: at,
            utc: UtcMillis(1_700_000_000_000 + at),
            after_ms: 60_000,
            hosts: host.clone(),
            legacy_source: Some("covered".into()),
        };
        for _ in 0..30 {
            if !archival::advance_with(&conn, Some(a.ctx()), &instance, &rt)
                .unwrap()
                .has_more
            {
                break;
            }
        }
        archived |= a.thread_row(&thread).map(|r| r.2) == Some(true);
    }
    assert!(archived, "the lane archived the thread on A");
    let registers: Vec<_> = a
        .journal()
        .into_iter()
        .filter(|(k, _, _)| k == "thread.register")
        .collect();
    assert_eq!(registers.len(), 1, "the decision is one register fact");
    assert_eq!(registers[0].1, "applied");
    assert_eq!(
        a.register(&thread, "archived").map(|r| r.0),
        Some(Some("1".into()))
    );

    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(
        b.thread_row(&thread).map(|r| r.2),
        Some(true),
        "B sees the thread archived"
    );
    assert_eq!(register_events(&b, &thread, "archived").len(), 1);
    assert_eq!(a.thread_projection(), b.thread_projection());
    // Automatic archival names no actor: not in the fact, not in either event.
    let payload: Vec<u8> = a
        .query(
            "SELECT payload FROM journal WHERE kind='thread.register' ORDER BY ingest_seq",
            |r| r.get(0),
        )
        .remove(0);
    let payload: serde_json::Value = serde_json::from_slice(&payload).expect("payload json");
    assert!(payload.get("actor").is_none(), "{payload}");
    for node in [&a, &b] {
        let events = node.register_event_json(&thread, "archived");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["action"], "archive");
        assert!(events[0].get("actor_seat").is_none(), "{}", events[0]);
    }
}

#[test]
fn thread_direct_both_orders_earliest_creator() {
    let reg = registry();
    let (a, b) = (Node::with(67, reg), Node::with(68, reg));
    let (sa, sb) = (a.local_seat(), b.local_seat());
    // Each node publishes its seat by opening a thread of its own, then both
    // learn the other's seat.
    a.create_thread(&sa, "a's", None).unwrap();
    b.create_thread(&sb, "b's", None).unwrap();
    fact_exchange::sync(&a.store, &b.store);
    // Crossed: both open the same pair while partitioned.
    let pair_a = a.author_direct(&sa, &sb).unwrap();
    let pair_b = b.author_direct(&sa, &sb).unwrap();
    assert_eq!(pair_a, pair_b, "the pair names one thread");
    let stamps = [
        a.stamp_of("thread.direct", 0),
        b.stamp_of("thread.direct", 0),
    ];
    let earliest = stamps.iter().min().unwrap().clone();

    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        assert_eq!(
            node.direct_creator(&pair_a),
            Some(earliest.clone()),
            "the earliest (hlc, node) is the creator on both"
        );
        assert_eq!(
            node.event_keys(&pair_a)
                .iter()
                .filter(|k| k.starts_with("create:"))
                .count(),
            1,
            "one create event"
        );
    }
    assert_eq!(a.thread_projection(), b.thread_projection());

    // The same two facts into fresh stores in both orders.
    let hub = &a;
    for reverse in [false, true] {
        let fresh = Node::with(if reverse { 70 } else { 69 }, reg);
        ingest_logs(&fresh, hub, reverse);
        assert_eq!(
            fresh.direct_creator(&pair_a),
            Some(earliest.clone()),
            "fresh store, reverse = {reverse}"
        );
        let row = fresh.thread_row(&pair_a).unwrap();
        let want_ms = i64::try_from(
            herdr_threads::sync::hlc::HlcStamp::from_u64(u64::try_from(earliest.0).unwrap()).ms(),
        )
        .unwrap();
        assert_eq!(row.3, want_ms, "created_at is the earliest creator's ms");
        assert_eq!(fresh.thread_projection(), a.thread_projection());
    }
}

/// Ingests every log `hub` holds into `dst`, one log at a time in `LogKey`
/// order (`reverse` flips it), then drains the apply passes.
fn ingest_logs(dst: &Node, hub: &Node, reverse: bool) {
    let mut logs: Vec<_> = hub
        .store
        .vector(&budget())
        .expect("read the vector")
        .0
        .into_iter()
        .collect();
    logs.sort_by_key(|a| a.0);
    if reverse {
        logs.reverse();
    }
    for (log, head) in logs {
        let mut next = 1;
        while next <= head.counter {
            let batch = hub
                .store
                .serve(&log, next, BatchLimits::default(), &budget())
                .expect("serve");
            let last = batch.last().expect("a batch").counter();
            match dst.store.ingest(&log, batch, &budget()).expect("ingest") {
                IngestOutcome::Stored { .. } => {}
                other => panic!("ingest of {log:?}: {other:?}"),
            }
            next = last + 1;
        }
    }
    for _ in 0..100_000 {
        if !dst.store.apply_pending(&budget()).unwrap().has_more {
            return;
        }
    }
    panic!("never drained");
}

#[test]
fn order_permutation_identical_projection_strict() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(71, reg),
        Node::with(72, reg),
        Node::with(73, reg),
    );
    let (sa, sb, sc) = (a.local_seat(), b.local_seat(), c.local_seat());
    let thread = a.create_thread(&sa, "start", Some("n0")).unwrap();
    b.create_thread(&sb, "b's", None).unwrap();
    c.create_thread(&sc, "c's", None).unwrap();
    sync_all(&[&a, &b, &c]);
    // Everything below is concurrent: no node sees another's writes.
    a.author_direct(&sa, &sb).unwrap();
    b.author_direct(&sa, &sb).unwrap();
    for (node, tag) in [(&a, "a"), (&b, "b"), (&c, "c")] {
        node.write_register(&thread, RegisterField::Topic, Some(&format!("topic {tag}")))
            .unwrap();
        node.write_register(&thread, RegisterField::Name, Some(&format!("name {tag}")))
            .unwrap();
    }
    a.write_register(&thread, RegisterField::Archived, Some("1"))
        .unwrap();
    b.write_register(&thread, RegisterField::Archived, Some("0"))
        .unwrap();
    c.write_register(&thread, RegisterField::Archived, Some("1"))
        .unwrap();
    c.write_register(&thread, RegisterField::Name, None)
        .unwrap();
    sync_all(&[&a, &b, &c]);

    let specs = replicated_tables();
    let fresh_up = Node::with(74, reg);
    let fresh_down = Node::with(75, reg);
    ingest_logs(&fresh_up, &a, false);
    ingest_logs(&fresh_down, &a, true);
    let up = replicated_projection(&fresh_up.store, fresh_up.id, &specs);
    let down = replicated_projection(&fresh_down.store, fresh_down.id, &specs);
    assert_eq!(up, down, "opposite log orders, identical projection");
    for table in ["threads", "direct_threads", "thread_registers"] {
        assert!(
            up.iter()
                .any(|(k, rows)| k.starts_with(table) && !rows.is_empty()),
            "{table} projects rows"
        );
    }
    // The live nodes' own apply orders agree with a replay of the hub.
    let replay = assert_replays_equal(&a.store, &specs);
    assert_eq!(
        strict_violations(&a.store, &replay, &specs),
        Vec::<String>::new()
    );
    for node in [&a, &b, &c] {
        let live = replicated_projection(&node.store, replay.node(), &specs);
        let replayed = replicated_projection(&replay, replay.node(), &specs);
        assert_eq!(live, replayed, "{} agrees with the replay", node.id);
    }
    // The pair's creator and created_at are the earliest of the two creates.
    let pair = herdr_threads::protocol::ids::direct_thread_id(&sa, &sb).unwrap();
    let stamps = [
        a.stamp_of("thread.direct", 0),
        b.stamp_of("thread.direct", 0),
    ];
    for node in [&fresh_up, &fresh_down] {
        assert_eq!(node.direct_creator(&pair), stamps.iter().min().cloned());
    }
}
