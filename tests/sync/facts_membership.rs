//! `membership.joined` and `membership.left` (fact-kinds design section 7;
//! ht-5fr.4.9): payloads, keys, the ordinal rule, idempotence, retirement,
//! origin authority and the fork residual. The two-node convergence cases are
//! in `fact_convergence_membership`.
//!
//! The helpers at the top (authoring a membership fact on a [`Node`], reading
//! its rows) are shared with that file.

use herdr_threads::protocol::ids::{SeatId, ThreadId};
use herdr_threads::protocol::results::ApiError;
use herdr_threads::store::control::{self, Transition};
use herdr_threads::store::journal;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::membership::{MembershipJoined, MembershipLeft, MembershipPayload};
use herdr_threads::sync::facts::payload::ReferencedSeats;
use herdr_threads::sync::facts::{FactKind, FactRegistry};
use herdr_threads::test_support::fact_exchange;

use crate::facts_seat::{Node, registry};
use crate::facts_thread::sync_all;

// ---- shared fixtures --------------------------------------------------------

/// The `at` of the transition with ordinal `n` in these tests.
pub fn at_of(n: u64) -> i64 {
    1_000 + i64::try_from(n).expect("a small ordinal")
}

impl Node {
    /// Authors a membership fact `K` on the journal route, the way the
    /// commands do (the write guard armed by [`Node::run`]).
    pub fn author_membership<K: FactKind<Payload = MembershipPayload>>(
        &self,
        thread: &ThreadId,
        seat: &SeatId,
        n: u64,
    ) -> Result<(), ApiError> {
        let payload = MembershipPayload {
            thread: thread.clone(),
            seat: seat.clone(),
            n,
            at: at_of(n),
            actor: None,
        };
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<K>(tx, self.ctx(), d, &payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    pub fn joined(&self, thread: &ThreadId, seat: &SeatId, n: u64) -> Result<(), ApiError> {
        self.author_membership::<MembershipJoined>(thread, seat, n)
    }

    pub fn left(&self, thread: &ThreadId, seat: &SeatId, n: u64) -> Result<(), ApiError> {
        self.author_membership::<MembershipLeft>(thread, seat, n)
    }

    /// `(state, home_transition, last_joined_n)` of a membership row.
    pub fn membership(&self, thread: &ThreadId, seat: &SeatId) -> Option<(String, i64, i64)> {
        self.query(
            &format!(
                "SELECT state,home_transition,last_joined_n FROM memberships \
                 WHERE thread_id='{}' AND seat_id='{}'",
                thread.as_str(),
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .into_iter()
        .next()
    }

    /// `(joined_at, left_at)` of a membership row.
    pub fn membership_times(
        &self,
        thread: &ThreadId,
        seat: &SeatId,
    ) -> Option<(Option<i64>, Option<i64>)> {
        self.query(
            &format!(
                "SELECT joined_at,left_at FROM memberships WHERE thread_id='{}' AND seat_id='{}'",
                thread.as_str(),
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .into_iter()
        .next()
    }

    /// `(episode, joined_seq, left_seq)` of the seat's intervals, oldest first.
    pub fn intervals(&self, thread: &ThreadId, seat: &SeatId) -> Vec<(i64, i64, Option<i64>)> {
        self.query(
            &format!(
                "SELECT episode,joined_seq,left_seq FROM membership_intervals \
                 WHERE thread_id='{}' AND seat_id='{}' ORDER BY ordinal",
                thread.as_str(),
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    /// The `join:` and `leave:` event keys of a thread, oldest first.
    pub fn membership_events(&self, thread: &ThreadId) -> Vec<String> {
        self.event_keys(thread)
            .into_iter()
            .filter(|k| k.starts_with("join:") || k.starts_with("leave:"))
            .collect()
    }

    /// The replicated membership and invitation rows as one comparable string.
    pub fn membership_projection(&self) -> String {
        let specs: Vec<_> = herdr_threads::test_support::journal_oracle::replicated_tables()
            .into_iter()
            .filter(|t| matches!(t.name, "memberships" | "invitations"))
            .collect();
        format!(
            "{:?}",
            herdr_threads::test_support::journal_oracle::replicated_projection(
                &self.store,
                herdr_threads::node::NodeUuid::from_bytes([0xee; 16]),
                &specs
            )
        )
    }
}

/// A thread created by a fresh local seat of `node`, and that creator.
pub fn thread_of(node: &Node, topic: &str) -> (ThreadId, SeatId) {
    let creator = node.local_seat();
    let thread = node.create_thread(&creator, topic, None).unwrap();
    (thread, creator)
}

// ---- payloads, keys, registration ------------------------------------------

#[test]
fn payload_round_trips_creates_needs() {
    let payload = MembershipPayload {
        thread: ThreadId::new("tAbc12345678"),
        seat: SeatId::new("sAbc12345678"),
        n: 3,
        actor: None,
        at: 1_700_000_000_000,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        r#"{"thread":"tAbc12345678","seat":"sAbc12345678","n":3,"at":1700000000000}"#
    );
    assert_eq!(
        serde_json::from_str::<MembershipPayload>(&json).unwrap(),
        payload
    );
    // An additive field is ignored.
    let newer: MembershipPayload = serde_json::from_str(
        r#"{"thread":"tAbc12345678","seat":"sAbc12345678","n":3,"at":1,"future":{"x":1}}"#,
    )
    .unwrap();
    assert_eq!(newer.n, 3);
    assert!(
        serde_json::from_str::<MembershipPayload>(
            r#"{"thread":"tAbc12345678","seat":"sAbc12345678","at":1}"#
        )
        .is_err(),
        "the ordinal is required"
    );

    assert_eq!(MembershipJoined::NAME, "membership.joined");
    assert_eq!(MembershipLeft::NAME, "membership.left");
    for needs in [
        MembershipJoined::needs(&payload),
        MembershipLeft::needs(&payload),
    ] {
        assert_eq!(
            needs,
            vec![
                EntityKey::new("tAbc12345678").unwrap(),
                EntityKey::new("sAbc12345678").unwrap()
            ]
        );
    }
    assert_eq!(MembershipJoined::creates(&payload), None);
    assert_eq!(MembershipLeft::creates(&payload), None);
    assert_eq!(payload.referenced_seats(), vec![payload.seat.clone()]);

    let builtins = FactRegistry::builtins();
    for kind in ["membership.joined", "membership.left"] {
        assert!(builtins.lookup(kind, 1).is_some(), "{kind} registered");
    }
}

// ---- the ordinal rule --------------------------------------------------------

#[test]
fn join_leave_then_reissued_join_n1_noop() {
    let node = Node::with(81, registry());
    let (thread, _creator) = thread_of(&node, "t");
    let seat = node.local_seat();
    node.joined(&thread, &seat, 1).unwrap();
    assert_eq!(
        node.membership(&thread, &seat),
        Some(("joined".into(), 1, 1))
    );
    assert_eq!(
        node.membership_times(&thread, &seat),
        Some((Some(at_of(1)), None))
    );
    node.left(&thread, &seat, 2).unwrap();
    assert_eq!(node.membership(&thread, &seat), Some(("left".into(), 2, 1)));
    assert_eq!(
        node.membership_times(&thread, &seat),
        Some((Some(at_of(1)), Some(at_of(2))))
    );
    let intervals = node.intervals(&thread, &seat);
    assert_eq!(intervals.len(), 1);
    assert!(
        intervals[0].2.is_some_and(|left| left > intervals[0].1),
        "the interval closed after it opened: {intervals:?}"
    );
    let events = node.membership_events(&thread);
    let (join_key, leave_key) = (
        format!("join:{}:{}:1", thread.as_str(), seat.as_str()),
        format!("leave:{}:{}:2", thread.as_str(), seat.as_str()),
    );
    assert!(
        events.contains(&join_key) && events.contains(&leave_key),
        "{events:?}"
    );
    let before = (
        node.membership_projection(),
        events.clone(),
        intervals.clone(),
    );

    // A re-issued join with n = 1 must not resurrect the seat.
    node.joined(&thread, &seat, 1).unwrap();
    assert_eq!(
        (
            node.membership_projection(),
            node.membership_events(&thread),
            node.intervals(&thread, &seat)
        ),
        before,
        "no row, interval or event changed"
    );
    assert_eq!(
        node.count_kind("membership.joined"),
        2,
        "the copy is journaled"
    );
    assert!(
        node.journal()
            .iter()
            .all(|(_, state, _)| state == "applied"),
        "and applied"
    );
}

#[test]
fn apply_twice_and_reauthored_noop() {
    let node = Node::with(82, registry());
    let (thread, _creator) = thread_of(&node, "t");
    let seat = node.local_seat();
    node.joined(&thread, &seat, 1).unwrap();
    let before = (
        node.membership_projection(),
        node.membership_events(&thread),
        node.intervals(&thread, &seat),
    );
    assert_eq!(before.1.len(), 1);
    assert_eq!(before.2.len(), 1);
    let payload = MembershipPayload {
        thread: thread.clone(),
        seat: seat.clone(),
        actor: None,
        n: 1,
        at: at_of(1),
    };
    // The same payload again in the same log.
    node.run(|tx, d| journal::author::<MembershipJoined>(tx, node.ctx(), d, &payload).map(|_| ()))
        .unwrap();
    // And a re-authored copy under a second incarnation of this node.
    node.run(|tx, d| journal::begin_incarnation(tx, node.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    node.run(|tx, d| journal::author::<MembershipJoined>(tx, node.ctx(), d, &payload).map(|_| ()))
        .unwrap();
    assert_eq!(node.count_kind("membership.joined"), 3);
    assert!(
        node.journal()
            .iter()
            .filter(|(k, _, _)| k == "membership.joined")
            .all(|(_, state, _)| state == "applied")
    );
    assert_eq!(
        (
            node.membership_projection(),
            node.membership_events(&thread),
            node.intervals(&thread, &seat)
        ),
        before,
        "no second interval or event"
    );
}

#[test]
fn creator_first_join_is_implied_by_the_create_event() {
    let node = Node::with(80, registry());
    let (thread, creator) = thread_of(&node, "t");
    let create_seq: i64 = node.query(
        &format!(
            "SELECT decision_seq FROM messages WHERE event_key='create:{}'",
            thread.as_str()
        ),
        |r| r.get(0),
    )[0];
    node.joined(&thread, &creator, 1).unwrap();
    assert!(
        node.membership_events(&thread).is_empty(),
        "no second event for the creator's own first join"
    );
    assert_eq!(
        node.intervals(&thread, &creator),
        vec![(1, create_seq, None)],
        "the creator joins at the create event's position, so it sees it"
    );
    // A later join of the creator (after a leave) and any other seat's first
    // join are ordinary joins with their own event.
    node.left(&thread, &creator, 2).unwrap();
    node.joined(&thread, &creator, 3).unwrap();
    let other = node.local_seat();
    node.joined(&thread, &other, 1).unwrap();
    let events = node.membership_events(&thread);
    assert!(
        events.contains(&format!("join:{}:{}:3", thread.as_str(), creator.as_str()))
            && events.contains(&format!("join:{}:{}:1", thread.as_str(), other.as_str())),
        "{events:?}"
    );
    assert!(
        !events.contains(&format!("join:{}:{}:1", thread.as_str(), creator.as_str())),
        "{events:?}"
    );
}

#[test]
fn left_without_joined_row_is_recorded_not_resurrected() {
    let node = Node::with(83, registry());
    let (thread, _creator) = thread_of(&node, "t");
    let seat = node.local_seat();
    // A leave whose join this store never applied keeps its ordinal.
    node.left(&thread, &seat, 2).unwrap();
    assert_eq!(node.membership(&thread, &seat), Some(("left".into(), 2, 0)));
    assert!(node.intervals(&thread, &seat).is_empty());
    assert!(node.membership_events(&thread).is_empty(), "no leave event");
    // The older join then loses.
    node.joined(&thread, &seat, 1).unwrap();
    assert_eq!(node.membership(&thread, &seat), Some(("left".into(), 2, 0)));
}

#[test]
fn zero_ordinal_is_refused() {
    let node = Node::with(84, registry());
    let (thread, _creator) = thread_of(&node, "t");
    let seat = node.local_seat();
    for result in [node.joined(&thread, &seat, 0), node.left(&thread, &seat, 0)] {
        assert!(result.is_err(), "ordinal 0 is rejected at authoring");
    }
    assert!(node.membership(&thread, &seat).is_none());
}

// ---- retirement --------------------------------------------------------------

#[test]
fn joined_on_retired_row_records_home_transition_only() {
    let node = Node::with(85, registry());
    let (thread, _creator) = thread_of(&node, "t");
    let seat = node.local_seat();
    node.joined(&thread, &seat, 1).unwrap();
    node.raw()
        .execute(
            "UPDATE memberships SET state='retired',voluntary_state='retired',retired_at=5 \
             WHERE thread_id=?1 AND seat_id=?2",
            [thread.as_str(), seat.as_str()],
        )
        .unwrap();
    let events = node.membership_events(&thread);
    let intervals = node.intervals(&thread, &seat);
    node.joined(&thread, &seat, 2).unwrap();
    assert_eq!(
        node.membership(&thread, &seat),
        Some(("retired".into(), 2, 1)),
        "retirement is terminal; the ordinal moved, last_joined_n did not"
    );
    assert_eq!(node.membership_events(&thread), events, "no join event");
    assert_eq!(node.intervals(&thread, &seat), intervals, "no new interval");
    // A leave on a retired row records the ordinal only too.
    node.left(&thread, &seat, 3).unwrap();
    assert_eq!(
        node.membership(&thread, &seat),
        Some(("retired".into(), 3, 1))
    );
    assert_eq!(node.membership_events(&thread), events, "no leave event");
}

// ---- origin authority --------------------------------------------------------

/// A, B and C with B's seat published everywhere and a thread of A's.
fn three_nodes(seed: u8) -> (Node, Node, Node, SeatId, ThreadId) {
    let reg = registry();
    let (a, b, c) = (
        Node::with(seed, reg),
        Node::with(seed + 1, reg),
        Node::with(seed + 2, reg),
    );
    let (thread, _creator) = thread_of(&a, "a's");
    let seat = b.local_seat();
    b.publish(&seat).unwrap();
    sync_all(&[&a, &b, &c]);
    (a, b, c, seat, thread)
}

#[test]
fn foreign_origin_joined_failed() {
    let (a, b, c, b_seat, thread) = three_nodes(86);
    assert!(a.seat(&b_seat).is_some() && c.seat(&b_seat).is_some());
    // C authors a join for B's seat; C's own apply passes (local origin), every
    // other node rejects it.
    c.joined(&thread, &b_seat, 1).unwrap();
    sync_all(&[&a, &b, &c]);
    for node in [&a, &b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "membership.joined" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        let error = failed[0].2.as_deref().unwrap();
        assert!(
            error.contains("is not the authorized author of membership.joined for")
                && error.contains(b_seat.as_str()),
            "{error}"
        );
        assert!(node.membership(&thread, &b_seat).is_none(), "no row");
        assert!(node.membership_events(&thread).is_empty(), "no event");
    }
    // The authorized author's own fact passes everywhere.
    b.joined(&thread, &b_seat, 1).unwrap();
    sync_all(&[&a, &b, &c]);
    for node in [&a, &b] {
        assert_eq!(
            node.membership(&thread, &b_seat),
            Some(("joined".into(), 1, 1))
        );
    }
}

// ---- the node-local route ----------------------------------------------------

#[test]
fn managed_thread_membership_is_local_only_and_publishes_nothing() {
    let node = Node::with(87, registry());
    let seat = node.local_seat();
    let thread = node.mint_thread();
    node.run(|tx, d| {
        let input = herdr_threads::store::materialize::local_only(tx, d)?;
        control::materialize_thread_created(
            tx,
            &input,
            &control::NewThread {
                thread: &thread,
                topic: "managed",
                goal: "goal",
                name: None,
                creator: None,
                managed_owner: None,
                journaled: false,
            },
        )
    })
    .unwrap();
    let before = node.journal_kinds();
    let payload = MembershipPayload {
        thread: thread.clone(),
        actor: None,
        seat: seat.clone(),
        n: 1,
        at: at_of(1),
    };
    assert_eq!(
        node.run(|tx, d| FactRoute::for_thread(tx, &thread).map(|route| (route, d)))
            .map(|(route, _)| route),
        Ok(FactRoute::LocalOnly)
    );
    node.run(|tx, d| {
        FactRoute::LocalOnly
            .emit::<MembershipJoined>(tx, node.ctx(), d, &payload, None, |tx, m, p| {
                control::materialize_membership(tx, m, Transition::Joined, p)
            })
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(
        node.membership(&thread, &seat),
        Some(("joined".into(), 1, 1))
    );
    assert_eq!(node.journal_kinds(), before, "nothing journaled");
    assert!(
        !node.seat(&seat).unwrap().3,
        "a managed membership does not publish the seat"
    );
}

// ---- the fork residual -------------------------------------------------------

/// Two branches of B's log (two incarnations of one node) each author a
/// `membership.joined` with the same `n`. A store that saw only one branch
/// keeps it; B's next transition (`n + 1`) makes the stores agree again
/// (root section 12, "forked-log same-key facts": accepted, asserted here as
/// documented behaviour).
#[test]
fn same_n_two_branches_first_applied_then_next_transition_agrees() {
    let reg = registry();
    let a = Node::with(88, reg);
    // Two incarnations of node B (same identity, different logs).
    let b1 = Node::open(89, 1, reg, true);
    let b2 = Node::open(89, 2, reg, true);
    assert_eq!(b1.id, b2.id);
    let (c, d) = (Node::with(90, reg), Node::with(91, reg));
    let (thread, _creator) = thread_of(&a, "a's");
    let seat = b1.mint_seat(0);
    b1.add_local_seat(&seat);
    b2.add_local_seat(&seat);
    b1.publish(&seat).unwrap();
    b2.publish(&seat).unwrap();
    for other in [&b1, &b2, &c, &d] {
        fact_exchange::sync(&a.store, &other.store);
    }

    // Branch 1: joined(1). Branch 2: joined(1), left(2).
    b1.joined(&thread, &seat, 1).unwrap();
    b2.joined(&thread, &seat, 1).unwrap();
    b2.left(&thread, &seat, 2).unwrap();
    fact_exchange::sync(&c.store, &b1.store);
    fact_exchange::sync(&d.store, &b2.store);
    assert_eq!(c.membership(&thread, &seat), Some(("joined".into(), 1, 1)));
    assert_eq!(d.membership(&thread, &seat), Some(("left".into(), 2, 1)));
    assert_ne!(
        c.membership_projection(),
        d.membership_projection(),
        "each store keeps the branch it applied: the documented residual"
    );

    // B's next transition on branch 1 carries n = 2.
    b1.left(&thread, &seat, 2).unwrap();
    fact_exchange::sync(&c.store, &b1.store);
    fact_exchange::sync(&d.store, &b1.store);
    for node in [&c, &d] {
        assert_eq!(node.membership(&thread, &seat), Some(("left".into(), 2, 1)));
    }
    assert_eq!(c.membership_projection(), d.membership_projection());
}

// ---- the pair creator's actor ------------------------------------------------

#[test]
fn membership_payload_actor_round_trips_and_is_optional() {
    let base = r#"{"thread":"tAbc12345678","seat":"sAbc12345678","n":1,"at":5}"#;
    let old: MembershipPayload = serde_json::from_str(base).unwrap();
    assert_eq!(old.actor, None, "a payload without actor decodes to None");
    assert!(
        !serde_json::to_string(&old).unwrap().contains("actor"),
        "None serializes with no actor key"
    );
    let with = MembershipPayload {
        actor: Some(SeatId::new("sCreator0001")),
        ..old
    };
    let json = serde_json::to_string(&with).unwrap();
    assert!(json.contains(r#""actor":"sCreator0001""#), "{json}");
    assert_eq!(
        serde_json::from_str::<MembershipPayload>(&json).unwrap(),
        with
    );
    assert_eq!(
        with.referenced_seats(),
        vec![with.seat.clone()],
        "the actor is not a referenced seat"
    );
}

#[test]
fn join_event_drops_an_actor_outside_the_pair() {
    let node = Node::with(83, registry());
    let (s1, s2, outsider) = (node.local_seat(), node.local_seat(), node.local_seat());
    let pair = node.author_direct(&s1, &s2).unwrap();
    node.joined(&pair, &s1, 1).unwrap();
    let payload = MembershipPayload {
        thread: pair.clone(),
        seat: s2.clone(),
        n: 1,
        at: at_of(1),
        actor: Some(outsider),
    };
    node.run(|tx, d| journal::author::<MembershipJoined>(tx, node.ctx(), d, &payload).map(|_| ()))
        .unwrap();
    assert!(
        node.journal()
            .iter()
            .filter(|(k, _, _)| k == "membership.joined")
            .all(|(_, state, _)| state == "applied")
    );
    assert_eq!(node.membership(&pair, &s2), Some(("joined".into(), 1, 1)));
    // The fact applied and its join event exists (an actor is never the
    // creating join), but names no actor_seat for a seat outside the pair.
    let events = node.query(
        &format!(
            "SELECT event_json FROM messages WHERE thread_id='{}' AND event_key='join:{}:{}:1'",
            pair.as_str(),
            pair.as_str(),
            s2.as_str()
        ),
        |r| r.get::<_, String>(0),
    );
    assert_eq!(events.len(), 1, "the peer's join event exists");
    let event: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
    assert_eq!(event["seat"], s2.as_str());
    assert!(event.get("actor_seat").is_none(), "{event}");
}
