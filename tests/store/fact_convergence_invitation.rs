//! Convergence of the invitation facts (fact-kinds design sections 5 to 7):
//! crossed direct pairs, the `already_joined` rule from every arrival order
//! and the order-independence invariant (root section 5), with the real
//! `invitation.created`, `invitation.answered`, `membership.*` and
//! `seat.retired` kinds.
//!
//! "Opposite orders" means two fresh stores that already hold the seats and
//! the thread and then ingest the same concurrent logs in opposite orders; the
//! replicated projection of the tables the facts write must then be identical.

use herdr_threads::ports::WorkAdmission;
use herdr_threads::protocol::authority::{CallerClaim, CallerRole, Harness};
use herdr_threads::protocol::ids::{
    ExecutionId, HostTargetId, NativeSessionId, RetirementJobId, SeatId, ThreadId,
};
use herdr_threads::store::control;
use herdr_threads::sync::facts::invitation::InvitationCreatedPayload;
use herdr_threads::sync::facts::payload::InviterKind;
use herdr_threads::test_support::journal_oracle::{replicated_projection, replicated_tables};
use herdr_threads::test_support::{assert_no_local_delivery_for_remote, fact_exchange};

use crate::fact_convergence_seat::begin_retirement;
use crate::facts_invitation::InvitationRow;
use crate::facts_membership::thread_of;
use crate::facts_seat::{Node, budget, registry};
use crate::facts_thread::sync_all;

// ---- helpers ----------------------------------------------------------------

/// Opens the direct pair of `caller` and `recipient` through the real
/// `resolve_direct_pair` (the writer under `ensure_direct_thread`), with the
/// write guard armed by [`Node::run`].
fn open_direct(node: &Node, caller: &SeatId, recipient: &SeatId) -> (ThreadId, bool) {
    let claim = CallerClaim {
        instance: node.id.to_string(),
        host: herdr_threads::test_support::test_host_locator(),
        seat: caller.clone(),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Human,
        native_session: NativeSessionId::new("human"),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("w1:p1"),
        agent_id: None,
        liveness: None,
    };
    node.run(|tx, d| control::resolve_direct_pair(tx, node.ctx(), d, &claim, recipient, None, None))
        .unwrap()
}

impl Node {
    /// The replicated rows of the named tables as one comparable string. Rows
    /// are projected for a bystander node, so no node's own rows drop out and
    /// two nodes' strings compare.
    pub fn projection_of(&self, tables: &[&str]) -> String {
        let specs: Vec<_> = replicated_tables()
            .into_iter()
            .filter(|t| tables.contains(&t.name))
            .collect();
        format!(
            "{:?}",
            replicated_projection(
                &self.store,
                herdr_threads::node::NodeUuid::from_bytes([0xee; 16]),
                &specs
            )
        )
    }

    /// Everything the invitation, membership and thread facts write.
    pub fn all_projection(&self) -> String {
        self.projection_of(&[
            "threads",
            "direct_threads",
            "thread_registers",
            "memberships",
            "invitations",
            "invitation_rejections",
        ])
    }

    /// Runs the retirement job of a replicated `seat.retired` to completion.
    pub fn finish_replicated_retirement(&self) {
        let ids = self.query(
            "SELECT id FROM retirements WHERE origin='replicated'",
            |r| r.get::<_, String>(0),
        );
        let mut conn = self.raw();
        for id in ids {
            for _ in 0..50 {
                let progress = control::advance_retirement(
                    self.ctx(),
                    &mut conn,
                    RetirementJobId::new(id.clone()),
                    WorkAdmission::Background,
                    &budget(),
                )
                .unwrap();
                if progress.complete {
                    break;
                }
            }
        }
    }
}

fn no_remote_delivery(nodes: &[&Node]) {
    for node in nodes {
        assert_no_local_delivery_for_remote(&node.raw());
    }
}

/// A (thread owner and inviter), B (the invitee's home), C (a second
/// inviter's home) and two fresh stores X and Y, all holding the thread and
/// the three seats.
struct World {
    a: Node,
    b: Node,
    c: Node,
    x: Node,
    y: Node,
    thread: ThreadId,
    inviter: SeatId,
    invitee: SeatId,
    other_inviter: SeatId,
}

fn world(seed: u8) -> World {
    let reg = registry();
    let (a, b, c, x, y) = (
        Node::with(seed, reg),
        Node::with(seed + 1, reg),
        Node::with(seed + 2, reg),
        Node::with(seed + 3, reg),
        Node::with(seed + 4, reg),
    );
    let (thread, inviter) = thread_of(&a, "shared");
    a.joined(&thread, &inviter, 1).unwrap();
    let invitee = b.local_seat();
    b.publish(&invitee).unwrap();
    let other_inviter = c.local_seat();
    c.publish(&other_inviter).unwrap();
    sync_all(&[&a, &b, &c, &x, &y]);
    assert!(x.seat(&invitee).is_some() && y.seat(&other_inviter).is_some());
    World {
        a,
        b,
        c,
        x,
        y,
        thread,
        inviter,
        invitee,
        other_inviter,
    }
}

/// X ingests A's, then B's, then C's logs; Y the reverse. One-way pulls, so
/// neither store relays the other's facts back into a source.
fn ingest_opposite_orders(w: &World) {
    for source in [&w.a, &w.b, &w.c] {
        fact_exchange::pull(&w.x.store, &source.store);
    }
    for source in [&w.c, &w.b, &w.a] {
        fact_exchange::pull(&w.y.store, &source.store);
    }
}

// ---- crossed direct pairs ---------------------------------------------------

#[test]
fn crossed_direct_pair_one_thread_both_already_joined_no_wake() {
    let reg = registry();
    let (a, b) = (Node::with(170, reg), Node::with(171, reg));
    let (sa, sb) = (a.local_seat(), b.local_seat());
    a.publish(&sa).unwrap();
    b.publish(&sb).unwrap();
    sync_all(&[&a, &b]);

    // Both open the pair while unsynced.
    let (ta, created_a) = open_direct(&a, &sa, &sb);
    let (tb, created_b) = open_direct(&b, &sb, &sa);
    assert!(created_a && created_b);
    assert_eq!(ta, tb, "the pair's thread ID is a function of its seats");
    assert!(ta.as_str().starts_with('d'));
    let authored = [
        a.stamp_of("thread.direct", 0),
        b.stamp_of("thread.direct", 0),
    ];
    let earliest = authored.iter().min().unwrap().clone();
    assert_eq!(
        a.membership(&ta, &sb),
        Some(("invited".into(), 0, 0)),
        "before the exchange A only invited B's seat"
    );

    sync_all(&[&a, &b]);
    for node in [&a, &b] {
        let threads: Vec<_> = node.query("SELECT id FROM threads WHERE id LIKE 'd%'", |r| {
            r.get::<_, String>(0)
        });
        assert_eq!(threads, vec![ta.as_str().to_owned()], "one thread");
        assert_eq!(
            node.direct_creator(&ta),
            Some(earliest.clone()),
            "earliest creator"
        );
        for seat in [&sa, &sb] {
            assert_eq!(
                node.membership(&ta, seat),
                Some(("joined".into(), 1, 1)),
                "both seats joined"
            );
        }
        let states = node.query("SELECT state FROM invitations ORDER BY id", |r| {
            r.get::<_, String>(0)
        });
        assert_eq!(states, vec!["already_joined"; 2], "both invitations settle");
        assert!(
            node.attention(&sa).is_none() && node.attention(&sb).is_none(),
            "the crossing wakes nobody"
        );
        assert_eq!(node.overdue_warnings(), 0);
    }
    assert_eq!(
        a.thread_row(&ta).map(|row| row.3),
        b.thread_row(&ta).map(|row| row.3),
        "the same created_at on both"
    );
    assert_eq!(a.all_projection(), b.all_projection());
    no_remote_delivery(&[&a, &b]);
}

#[test]
fn crossed_pair_orders() {
    let reg = registry();
    let (a, b, x, y) = (
        Node::with(172, reg),
        Node::with(173, reg),
        Node::with(174, reg),
        Node::with(175, reg),
    );
    let (sa, sb) = (a.local_seat(), b.local_seat());
    a.publish(&sa).unwrap();
    b.publish(&sb).unwrap();
    sync_all(&[&a, &b, &x, &y]);
    let (thread, _) = open_direct(&a, &sa, &sb);
    open_direct(&b, &sb, &sa);
    for source in [&a, &b] {
        fact_exchange::pull(&x.store, &source.store);
    }
    for source in [&b, &a] {
        fact_exchange::pull(&y.store, &source.store);
    }
    for node in [&x, &y] {
        assert_eq!(
            node.query("SELECT state FROM invitations ORDER BY id", |r| r
                .get::<_, String>(0)),
            vec!["already_joined"; 2]
        );
        assert_eq!(node.membership(&thread, &sa), Some(("joined".into(), 1, 1)));
        assert_eq!(node.membership(&thread, &sb), Some(("joined".into(), 1, 1)));
    }
    assert_eq!(x.all_projection(), y.all_projection());
    sync_all(&[&a, &b, &x, &y]);
    assert_eq!(x.all_projection(), a.all_projection());
    assert_eq!(a.all_projection(), b.all_projection());
    no_remote_delivery(&[&a, &b, &x, &y]);
}

// ---- relays -------------------------------------------------------------------

#[test]
fn three_nodes_relay_join_before_invite_both_orders() {
    let w = world(176);
    // Concurrent: A invites at invitee_n = 0; B joins (n = 1).
    let id = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();

    // X sees A's invitation first and B's join after; C relays B's join before
    // A's invitation reaches it. One-way pulls keep the sources apart.
    fact_exchange::pull(&w.x.store, &w.a.store);
    assert_eq!(
        w.x.state_of(&id).as_deref(),
        Some("pending"),
        "X holds the invitation before the join"
    );
    fact_exchange::pull(&w.x.store, &w.b.store);
    fact_exchange::pull(&w.c.store, &w.b.store);
    assert_eq!(
        w.c.membership(&w.thread, &w.invitee),
        Some(("joined".into(), 1, 1))
    );
    assert!(w.c.invitation(&id).is_none(), "not yet");
    fact_exchange::pull(&w.c.store, &w.a.store);

    for node in [&w.c, &w.x] {
        assert_eq!(node.state_of(&id).as_deref(), Some("already_joined"));
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("joined".into(), 1, 1))
        );
        assert!(node.attention(&w.invitee).is_none(), "no wake on a relay");
    }
    assert_eq!(w.c.all_projection(), w.x.all_projection());
    no_remote_delivery(&[&w.c, &w.x]);
}

#[test]
fn wiped_store_reingesting_own_answer_no_wake() {
    let reg = registry();
    let (a, b) = (Node::with(177, reg), Node::with(178, reg));
    let (thread, inviter) = thread_of(&a, "shared");
    a.joined(&thread, &inviter, 1).unwrap();
    let invitee = b.local_seat();
    b.publish(&invitee).unwrap();
    sync_all(&[&a, &b]);
    let id = a.invite_fact(&thread, &inviter, &invitee);
    fact_exchange::sync(&a.store, &b.store);
    assert!(
        b.attention(&invitee).is_some(),
        "the live invitee was woken once"
    );
    b.author_answered(&b.accept_payload(&thread, &id, &invitee))
        .unwrap();
    fact_exchange::sync(&a.store, &b.store);

    // B's store is wiped (same identity, another incarnation) and pulls its
    // history back from A: its own answer is ingested before the invitation
    // applies, so the invitation delivers nothing.
    let wiped = Node::open(178, 999, registry(), true);
    assert_eq!(wiped.id, b.id);
    fact_exchange::sync(&a.store, &wiped.store);
    assert!(
        wiped
            .journal()
            .iter()
            .all(|(_, state, _)| state == "applied"),
        "{:?}",
        wiped.journal()
    );
    assert_eq!(
        wiped.state_of(&id).as_deref(),
        Some("accepted"),
        "the answer applies too"
    );
    assert!(
        wiped.attention(&invitee).is_none(),
        "no invitation wake from a re-ingested answer"
    );
    assert!(
        wiped
            .invitation_events(&thread)
            .contains(&format!("invite:{}", id.as_str()))
    );
    assert_eq!(wiped.all_projection(), a.all_projection());
    no_remote_delivery(&[&a, &wiped]);
}

// ---- order permutations ------------------------------------------------------

#[test]
fn invite_vs_join_then_leave() {
    let w = world(180);
    let id = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    w.b.joined(&w.thread, &w.invitee, 1).unwrap();
    w.b.left(&w.thread, &w.invitee, 2).unwrap();
    ingest_opposite_orders(&w);
    for node in [&w.x, &w.y] {
        assert_eq!(
            node.state_of(&id).as_deref(),
            Some("already_joined"),
            "a join then a leave still supersedes the invitation"
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("left".into(), 2, 1))
        );
    }
    assert_eq!(w.x.all_projection(), w.y.all_projection());
    no_remote_delivery(&[&w.x, &w.y]);
}

#[test]
fn invite_vs_seat_retired_recipient_retired() {
    let reg = registry();
    let (a, b, x, y) = (
        Node::with(190, reg),
        Node::with(191, reg),
        Node::with(192, reg),
        Node::with(193, reg),
    );
    let (thread, inviter) = thread_of(&a, "shared");
    a.joined(&thread, &inviter, 1).unwrap();
    let invitee = b.mint_seat(0);
    b.add_local_seat(&invitee);
    b.enroll(&invitee, "codex", "session-1").unwrap();
    sync_all(&[&a, &b, &x, &y]);
    assert!(x.seat(&invitee).is_some());

    // Concurrent: A invites the seat; B retires it.
    let id = a.invite_fact(&thread, &inviter, &invitee);
    begin_retirement(&b, &invitee).unwrap();
    for source in [&a, &b] {
        fact_exchange::pull(&x.store, &source.store);
    }
    for source in [&b, &a] {
        fact_exchange::pull(&y.store, &source.store);
    }
    // Y saw the retirement first: settled at apply. X settles when its
    // retirement job runs (the deadline lane's work, not the apply's).
    assert_eq!(y.state_of(&id).as_deref(), Some("recipient_retired"));
    assert_eq!(x.state_of(&id).as_deref(), Some("pending"));
    x.finish_replicated_retirement();
    y.finish_replicated_retirement();
    for node in [&x, &y] {
        assert_eq!(node.state_of(&id).as_deref(), Some("recipient_retired"));
        assert!(node.attention(&invitee).is_none());
    }
    assert_eq!(
        x.projection_of(&["invitations"]),
        y.projection_of(&["invitations"])
    );
    no_remote_delivery(&[&x, &y]);
}

#[test]
fn accept_vs_second_invitation_already_joined() {
    let w = world(200);
    let first = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    sync_all(&[&w.a, &w.b, &w.c, &w.x, &w.y]);
    // Concurrent: B accepts the first; C invites the same seat (invitee_n = 0).
    w.b.author_answered(&w.b.accept_payload(&w.thread, &first, &w.invitee))
        .unwrap();
    let second = w.c.created_payload(&w.thread, &w.other_inviter, &w.invitee);
    assert_eq!(second.invitee_n, 0);
    w.c.author_created(&second).unwrap();
    ingest_opposite_orders(&w);
    for node in [&w.x, &w.y] {
        assert_eq!(node.state_of(&first).as_deref(), Some("accepted"));
        assert_eq!(
            node.state_of(&second.invitation).as_deref(),
            Some("already_joined"),
            "the second invitation ends settled in either order"
        );
        assert_eq!(
            node.membership(&w.thread, &w.invitee),
            Some(("joined".into(), 1, 1))
        );
    }
    assert_eq!(w.x.all_projection(), w.y.all_projection());
    sync_all(&[&w.a, &w.b, &w.c, &w.x, &w.y]);
    assert_eq!(w.x.all_projection(), w.a.all_projection());
    no_remote_delivery(&[&w.a, &w.b, &w.c, &w.x, &w.y]);
}

#[test]
fn reject_replicates_and_restores_the_invitee() {
    let w = world(210);
    let id = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    sync_all(&[&w.a, &w.b]);
    w.b.author_answered(&w.b.reject_payload(&id, &w.invitee))
        .unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.rejection(&id).map(|row| row.0),
            Some("not now".to_owned())
        );
        let keys = node.invitation_events(&w.thread);
        assert!(
            keys.contains(&format!("reject:{}", id.as_str())),
            "{keys:?}"
        );
    }
    assert_eq!(w.a.all_projection(), w.b.all_projection());
    no_remote_delivery(&[&w.a, &w.b]);
}

#[test]
fn invitation_row_reads_the_same_on_every_node() {
    let w = world(220);
    let id = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    sync_all(&[&w.a, &w.b, &w.x]);
    let rows: Vec<InvitationRow> = [&w.a, &w.b, &w.x]
        .iter()
        .map(|node| node.invitation(&id).unwrap())
        .collect();
    // Episodes are local positions; everything else is replicated.
    for row in &rows {
        assert_eq!(row.origin, w.a.id.to_string());
        assert_eq!(row.inviter.as_deref(), Some(w.inviter.as_str()));
        assert_eq!(row.state, "pending");
        assert_eq!(row.deadline_at, rows[0].deadline_at);
        assert_eq!(row.invitee_n, rows[0].invitee_n);
    }
    // B alone wakes for it: the invitee is its local seat.
    assert!(w.b.attention(&w.invitee).is_some());
    assert!(w.a.attention(&w.invitee).is_none() && w.x.attention(&w.invitee).is_none());
    no_remote_delivery(&[&w.a, &w.b, &w.x]);
}

// Kills: a `confirmation` that depends on the node (it is derived from the
// issuer kind alone), a service or operator invitation that owes an ACK, an
// invitee whose home node is not woken, and a deadline other than
// `created_at + frozen_duration_ms` (reconcile 1.2).
#[test]
fn confirmation_per_issuer_on_both_nodes() {
    let w = world(230);
    let svc_invitee = w.b.local_seat();
    w.b.publish(&svc_invitee).unwrap();
    let op_invitee = w.b.local_seat();
    w.b.publish(&op_invitee).unwrap();
    sync_all(&[&w.a, &w.b, &w.x]);

    let seat_id = w.a.invite_fact(&w.thread, &w.inviter, &w.invitee);
    let service = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Service,
        actor_label: Some("svc-author".into()),
        ..w.a.created_payload(&w.thread, &w.inviter, &svc_invitee)
    };
    w.a.author_created(&service).unwrap();
    let operator = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Operator,
        actor_label: Some("operator:local-user:501".into()),
        invitation: w.a.mint_invitation(),
        ..w.a.created_payload(&w.thread, &w.inviter, &op_invitee)
    };
    w.a.author_created(&operator).unwrap();
    sync_all(&[&w.a, &w.b, &w.x]);

    let expected = [
        (&seat_id, Some("require_ack"), &w.invitee),
        (&service.invitation, None, &svc_invitee),
        (&operator.invitation, None, &op_invitee),
    ];
    for node in [&w.a, &w.b] {
        for (id, confirmation, invitee) in expected {
            let row = node.invitation(id).unwrap();
            assert_eq!(row.confirmation.as_deref(), confirmation, "{id:?}");
            assert_eq!(
                row.deadline_at,
                service.created_at + service.duration_ms,
                "deadline is created_at + the frozen duration"
            );
            assert_eq!(node.attention(invitee).is_some(), node.id == w.b.id);
        }
    }
    no_remote_delivery(&[&w.a, &w.b, &w.x]);
}
