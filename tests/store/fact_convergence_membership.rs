//! Two- and four-store convergence of the membership facts (fact-kinds design
//! sections 6 and 7): the `already_joined` rule from either arrival order and
//! the order-independence invariant (root section 5).
//!
//! `invitation.created` is not a shipped kind yet (ht-5fr.4.10), so the
//! invitation side is the test kind `test.invite` below. It applies the
//! created-side half of the rule the spec gives `invitation.created`: a row
//! whose invitee already holds `last_joined_n > invitee_n` is inserted
//! `already_joined`, any other as `pending` with the invitee `invited`. The
//! joined-side half is the real `settle_superseded_invitations`.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use herdr_threads::protocol::ids::{SeatId, ThreadId};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::payload::{ReferencedSeats, seat_key, thread_key};
use herdr_threads::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry, with_input};
use herdr_threads::test_support::fact_exchange;

use crate::facts_membership::thread_of;
use crate::facts_seat::Node;
use crate::facts_thread::sync_all;

// ---- the stand-in invitation kind -------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InvitePayload {
    id: String,
    thread: ThreadId,
    inviter: SeatId,
    invitee: SeatId,
    invitee_n: i64,
}

impl ReferencedSeats for InvitePayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.inviter.clone()]
    }
}

struct TestInvite;

impl FactKind for TestInvite {
    const NAME: &'static str = "test.invite";
    const MAJOR: u32 = 1;
    type Payload = InvitePayload;

    fn creates(p: &InvitePayload) -> Option<EntityKey> {
        EntityKey::new(&p.id).ok()
    }

    fn needs(p: &InvitePayload) -> Vec<EntityKey> {
        [
            thread_key(&p.thread),
            seat_key(&p.inviter),
            seat_key(&p.invitee),
        ]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        p: InvitePayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            let store = herdr_threads::protocol::results::ApiError::new;
            let err = |e: rusqlite::Error| {
                store(
                    herdr_threads::protocol::results::ErrorCode::StoreCorrupt,
                    e.to_string(),
                )
            };
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM invitations WHERE id=?1)",
                    [&p.id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if exists {
                return Ok(());
            }
            let last_joined_n: i64 = tx
                .query_row(
                    "SELECT last_joined_n FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![p.thread.as_str(), p.invitee.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?
                .unwrap_or(0);
            let state = if last_joined_n > p.invitee_n {
                "already_joined"
            } else {
                "pending"
            };
            let episode: i64 = tx
                .query_row(
                    "SELECT coalesce(max(episode),0)+1 FROM invitations WHERE thread_id=?1 AND seat_id=?2",
                    params![p.thread.as_str(), p.invitee.as_str()],
                    |r| r.get(0),
                )
                .map_err(err)?;
            tx.execute(
                "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,\
                 frozen_duration_ms,deadline_at,inviter_seat_id,origin_node,invitee_n) \
                 VALUES (?1,?2,?3,?4,?5,?6,1,1000,5000,?7,?8,?9)",
                params![
                    p.id,
                    p.thread.as_str(),
                    p.invitee.as_str(),
                    episode,
                    state,
                    m.decision_seq as i64,
                    p.inviter.as_str(),
                    m.origin_node.to_string(),
                    p.invitee_n
                ],
            )
            .map_err(err)?;
            if state == "pending" {
                tx.execute(
                    "INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) \
                     VALUES (?1,?2,?3,'invited','invited') \
                     ON CONFLICT(thread_id,seat_id) DO UPDATE SET state='invited',voluntary_state='invited' \
                     WHERE memberships.state='left'",
                    params![p.thread.as_str(), p.invitee.as_str(), episode],
                )
                .map_err(err)?;
            }
            Ok(())
        })
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<TestInvite>())
}

impl Node {
    /// Authors a `test.invite` for `invitee` as `inviter`, with the ordinal
    /// this node holds for the invitee (the inviter's view).
    fn invite(&self, thread: &ThreadId, inviter: &SeatId, invitee: &SeatId, id: &str) {
        let invitee_n = self
            .membership(thread, invitee)
            .map_or(0, |(_, home_transition, _)| home_transition);
        let payload = InvitePayload {
            id: id.into(),
            thread: thread.clone(),
            inviter: inviter.clone(),
            invitee: invitee.clone(),
            invitee_n,
        };
        self.run(|tx, d| {
            herdr_threads::store::materialize::FactRoute::Journal
                .emit::<TestInvite>(tx, self.ctx(), d, &payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
        .unwrap();
    }

    fn invitation_state(&self, id: &str) -> Option<String> {
        self.query(
            &format!("SELECT state FROM invitations WHERE id='{id}'"),
            |r| r.get(0),
        )
        .into_iter()
        .next()
    }

    fn invitation_deadline(&self, id: &str) -> Option<i64> {
        self.query(
            &format!("SELECT deadline_at FROM invitations WHERE id='{id}'"),
            |r| r.get(0),
        )
        .into_iter()
        .next()
    }
}

/// A (thread owner, inviter), B (the invitee's home) and two fresh stores X
/// and Y, all holding the thread and both seats.
struct World {
    a: Node,
    b: Node,
    x: Node,
    y: Node,
    thread: ThreadId,
    inviter: SeatId,
    invitee: SeatId,
}

fn world(seed: u8) -> World {
    let reg = registry();
    let (a, b, x, y) = (
        Node::with(seed, reg),
        Node::with(seed + 1, reg),
        Node::with(seed + 2, reg),
        Node::with(seed + 3, reg),
    );
    let (thread, inviter) = thread_of(&a, "shared");
    a.joined(&thread, &inviter, 1).unwrap();
    let invitee = b.local_seat();
    b.publish(&invitee).unwrap();
    sync_all(&[&a, &b, &x, &y]);
    assert!(a.seat(&invitee).is_some() && x.seat(&invitee).is_some());
    World {
        a,
        b,
        x,
        y,
        thread,
        inviter,
        invitee,
    }
}

// ---- convergence ------------------------------------------------------------

#[test]
fn b_seat_joins_a_thread_both_agree() {
    let w = world(101);
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("joined".into(), 1, 1))
        );
        assert_eq!(
            node.membership_events(&w.thread)
                .iter()
                .filter(|k| k.contains(w.invitee.as_str()))
                .count(),
            1,
            "one join event per node"
        );
    }
    assert_eq!(w.a.membership_projection(), w.b.membership_projection());
}

#[test]
fn leave_replicates_and_closes_the_interval_on_both() {
    let w = world(103);
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    w.b.left(&w.thread, &w.invitee, 2).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("left".into(), 2, 1))
        );
        let intervals = node.intervals(&w.thread, &w.invitee);
        assert_eq!(intervals.len(), 1);
        assert!(
            intervals[0].2.is_some_and(|left| left > intervals[0].1),
            "interval closed after it opened on {:?}",
            node.id
        );
        assert_eq!(
            node.membership_events(&w.thread)
                .iter()
                .filter(|k| k.contains(w.invitee.as_str()))
                .count(),
            2
        );
    }
    assert_eq!(w.a.membership_projection(), w.b.membership_projection());
}

#[test]
fn pending_invitation_of_joining_seat_already_joined() {
    let w = world(105);
    w.a.invite(&w.thread, &w.inviter, &w.invitee, "iPending0001");
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.invitation_state("iPending0001").as_deref(),
            Some("pending")
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("invited".into(), 0, 0))
        );
    }
    let deadline = w.a.invitation_deadline("iPending0001");
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    assert_eq!(
        w.b.invitation_state("iPending0001").as_deref(),
        Some("already_joined"),
        "the join settles it on the joining node"
    );
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.invitation_state("iPending0001").as_deref(),
            Some("already_joined")
        );
        assert_eq!(
            node.invitation_deadline("iPending0001"),
            deadline,
            "deadline kept"
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("joined".into(), 1, 1))
        );
    }
    assert_eq!(w.a.membership_projection(), w.b.membership_projection());
}

/// Ingests A's and B's concurrent facts into X (A first) and Y (B first).
fn ingest_both_orders(w: &World) {
    fact_exchange::sync(&w.x.store, &w.a.store);
    fact_exchange::sync(&w.x.store, &w.b.store);
    fact_exchange::sync(&w.y.store, &w.b.store);
    fact_exchange::sync(&w.y.store, &w.a.store);
}

#[test]
fn invite_concurrent_with_join_both_orders() {
    let w = world(110);
    // Concurrent: A invites at invitee_n = 0; B joins (n = 1).
    w.a.invite(&w.thread, &w.inviter, &w.invitee, "iConcur00001");
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();

    // X sees the invitation first: pending, then settled by the join.
    fact_exchange::sync(&w.x.store, &w.a.store);
    assert_eq!(
        w.x.invitation_state("iConcur00001").as_deref(),
        Some("pending")
    );
    assert_eq!(
        w.x.membership(&w.thread, &w.invitee),
        Some(("invited".into(), 0, 0))
    );
    // Y sees the join first: the invitation is inserted already settled.
    fact_exchange::sync(&w.y.store, &w.b.store);
    assert_eq!(
        w.y.membership(&w.thread, &w.invitee),
        Some(("joined".into(), 1, 1))
    );
    ingest_both_orders(&w);

    for node in [&w.x, &w.y, &w.a, &w.b] {
        fact_exchange::sync(&node.store, &w.a.store);
        fact_exchange::sync(&node.store, &w.b.store);
    }
    for node in [&w.x, &w.y, &w.a, &w.b] {
        assert_eq!(
            node.invitation_state("iConcur00001").as_deref(),
            Some("already_joined")
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("joined".into(), 1, 1))
        );
    }
    assert_eq!(w.x.membership_projection(), w.y.membership_projection());
    assert_eq!(w.x.membership_projection(), w.a.membership_projection());
    assert_eq!(w.x.membership_projection(), w.b.membership_projection());
}

#[test]
fn invite_concurrent_with_join_then_leave_both_orders() {
    let w = world(115);
    w.a.invite(&w.thread, &w.inviter, &w.invitee, "iConcur00002");
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    w.b.left(&w.thread, &w.invitee, 2).unwrap();

    fact_exchange::sync(&w.x.store, &w.a.store);
    assert_eq!(
        w.x.invitation_state("iConcur00002").as_deref(),
        Some("pending")
    );
    fact_exchange::sync(&w.y.store, &w.b.store);
    assert_eq!(
        w.y.membership(&w.thread, &w.invitee),
        Some(("left".into(), 2, 1)),
        "Y holds the left row before the invitation arrives"
    );
    ingest_both_orders(&w);

    for node in [&w.x, &w.y] {
        assert_eq!(
            node.invitation_state("iConcur00002").as_deref(),
            Some("already_joined"),
            "a join then a leave still supersedes the invitation"
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("left".into(), 2, 1))
        );
    }
    assert_eq!(w.x.membership_projection(), w.y.membership_projection());
}

#[test]
fn later_invitation_after_the_leave_stays_pending() {
    // Control for the rule: an invitation whose inviter saw the leave
    // (invitee_n = 2 = last_joined_n + 1) is not superseded.
    let w = world(120);
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    w.b.left(&w.thread, &w.invitee, 2).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    w.a.invite(&w.thread, &w.inviter, &w.invitee, "iLater000001");
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.invitation_state("iLater000001").as_deref(),
            Some("pending")
        );
    }
    assert_eq!(w.a.membership_projection(), w.b.membership_projection());
}
