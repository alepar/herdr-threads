//! The order permutation suite (root design section 5 invariant; fact-kinds
//! design section 3 audit and Testing "Order permutation"; ht-5fr.4.18): for
//! each row of the audit, one scripted set of concurrent facts over three
//! stores is ingested into two fresh stores in opposite log orders, and the
//! replicated projection of every converted table must match.
//!
//! The scripts author on A, B and C while they are partitioned from each
//! other (each pair of them has only the setup in common), so every log holds
//! facts the other two have not seen. Fresh store X then ingests A's log,
//! then B's, then C's; fresh store Y the reverse. Both run their node-local
//! workers (send projection, retirement) and must project identically. The
//! three live stores then exchange everything: the hub's journal replays into
//! an empty store in strict mode (no position carried in a payload, causal
//! order kept) and every live store, with its workers run, projects like X.

use herdr_threads::protocol::commands::DeliveryMode;
use herdr_threads::protocol::ids::{MessageId, SeatId, ThreadId};
use herdr_threads::protocol::results::CommandResult;
use herdr_threads::sync::facts::payload::{ReceiptObligation, RegisterField};
use herdr_threads::test_support::assert_no_local_delivery_for_remote;
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::journal_oracle::{
    replay_into_empty, replicated_projection, replicated_tables, strict_violations,
};

use crate::fact_convergence::run_workers;
use crate::fact_convergence_seat::begin_retirement;
use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_seat::{Node, registry};
use crate::facts_thread::sync_all;

// ---- the scaffold -----------------------------------------------------------------

/// Three live stores and two fresh ones, all command-capable.
struct Trio {
    a: Node,
    b: Node,
    c: Node,
    x: Node,
    y: Node,
}

fn trio(seed: u8) -> Trio {
    let reg = registry();
    let node = |n: u8| Node::open_commands(seed + n, u64::from(seed + n), reg);
    Trio {
        a: node(0),
        b: node(1),
        c: node(2),
        x: node(3),
        y: node(4),
    }
}

impl Trio {
    /// Brings the three live stores level (the common setup).
    fn setup_done(&self) {
        sync_all(&[&self.a, &self.b, &self.c]);
    }

    /// Everything after the concurrent authoring: the opposite-order
    /// ingestion, the worker runs and the strict comparisons.
    fn check(&self) {
        // X: A's log, then B's, then C's. Y: the reverse. One-way pulls, so
        // neither store relays the other's facts back into a source.
        for source in [&self.a, &self.b, &self.c] {
            fact_exchange::pull(&self.x.store, &source.store);
        }
        for source in [&self.c, &self.b, &self.a] {
            fact_exchange::pull(&self.y.store, &source.store);
        }
        for fresh in [&self.x, &self.y] {
            let unsettled: Vec<_> = fresh
                .journal()
                .into_iter()
                .filter(|(_, state, _)| state != "applied")
                .collect();
            assert!(unsettled.is_empty(), "{:?}: {unsettled:?}", fresh.id);
            run_workers(&fresh.store);
        }
        let specs = replicated_tables();
        let up = replicated_projection(&self.x.store, self.x.id, &specs);
        let down = replicated_projection(&self.y.store, self.y.id, &specs);
        assert_eq!(up, down, "opposite log orders, identical projection");

        // The live stores, level and with their workers run, project like the
        // fresh ones, and the hub's journal replays into an empty store (its
        // workers run too) in strict mode.
        self.setup_done();
        for live in [&self.a, &self.b, &self.c] {
            let unsettled: Vec<_> = live
                .journal()
                .into_iter()
                .filter(|(_, state, _)| state != "applied")
                .collect();
            assert!(unsettled.is_empty(), "{:?}: {unsettled:?}", live.id);
            run_workers(&live.store);
            assert_eq!(
                replicated_projection(&live.store, self.x.id, &specs),
                up,
                "{:?} agrees with the fresh stores",
                live.id
            );
        }
        let replay = replay_into_empty(&self.a.store);
        run_workers(&replay);
        assert_eq!(
            strict_violations(&self.a.store, &replay, &specs),
            Vec::<String>::new(),
            "strict mode on the hub"
        );
        assert_eq!(
            replicated_projection(&self.a.store, replay.node(), &specs),
            replicated_projection(&replay, replay.node(), &specs),
            "the hub matches its replay"
        );
        for node in [&self.a, &self.b, &self.c, &self.x, &self.y] {
            assert_no_local_delivery_for_remote(&node.raw());
        }
    }
}

/// A: a thread and its joined inviter; B: a published invitee; C: a second
/// inviter. All of it level on the three live stores.
struct Cast {
    t: Trio,
    thread: ThreadId,
    inviter: SeatId,
    invitee: SeatId,
    other_inviter: SeatId,
}

fn cast(seed: u8, enroll_invitee: bool) -> Cast {
    let t = trio(seed);
    let (thread, inviter) = thread_of(&t.a, "shared");
    t.a.joined(&thread, &inviter, 1).unwrap();
    let invitee = t.b.local_seat();
    if enroll_invitee {
        t.b.enroll(&invitee, "codex", "session-invitee").unwrap();
    } else {
        t.b.publish(&invitee).unwrap();
    }
    let other_inviter = t.c.local_seat();
    t.c.publish(&other_inviter).unwrap();
    t.setup_done();
    Cast {
        t,
        thread,
        inviter,
        invitee,
        other_inviter,
    }
}

// ---- the audit rows -----------------------------------------------------------------

#[test]
fn crossed_thread_direct() {
    let t = trio(100);
    let (sa, sb) = (t.a.local_seat(), t.b.local_seat());
    t.a.bind_commands(&sa);
    t.b.bind_commands(&sb);
    t.a.publish(&sa).unwrap();
    t.b.publish(&sb).unwrap();
    t.setup_done();

    // Both open the same pair while partitioned.
    let CommandResult::ThreadCreated(from_a) =
        t.a.cmd_ensure_direct(&sa, &sb, "op-pair-a").unwrap()
    else {
        panic!("a pair thread");
    };
    let CommandResult::ThreadCreated(from_b) =
        t.b.cmd_ensure_direct(&sb, &sa, "op-pair-b").unwrap()
    else {
        panic!("a pair thread");
    };
    assert_eq!(
        from_a, from_b,
        "the pair's thread is a function of its seats"
    );
    t.check();
    for node in [&t.x, &t.y] {
        assert_eq!(
            node.query("SELECT state FROM invitations ORDER BY id", |r| r
                .get::<_, String>(0)),
            vec!["already_joined"; 2],
            "{:?}: both invitations settle",
            node.id
        );
        for seat in [&sa, &sb] {
            assert_eq!(
                node.membership(&from_a, seat).map(|m| m.0).as_deref(),
                Some("joined")
            );
        }
        assert_eq!(
            node.count("SELECT count(*) FROM threads WHERE id LIKE 'd%'"),
            1,
            "one pair thread"
        );
    }
}

#[test]
fn invite_vs_join() {
    let c = cast(110, false);
    // Concurrent: A invites at invitee_n = 0; B joins (n = 1).
    let id = c.t.a.invite_fact(&c.thread, &c.inviter, &c.invitee);
    c.t.b.joined(&c.thread, &c.invitee, 1).unwrap();
    c.t.check();
    for node in [&c.t.x, &c.t.y] {
        assert_eq!(
            node.state_of(&id).as_deref(),
            Some("already_joined"),
            "{:?}",
            node.id
        );
        assert_eq!(
            node.membership(&c.thread, &c.invitee),
            Some(("joined".into(), 1, 1))
        );
    }
}

#[test]
fn invite_vs_join_then_leave() {
    let c = cast(120, false);
    let id = c.t.a.invite_fact(&c.thread, &c.inviter, &c.invitee);
    c.t.b.joined(&c.thread, &c.invitee, 1).unwrap();
    c.t.b.left(&c.thread, &c.invitee, 2).unwrap();
    c.t.check();
    for node in [&c.t.x, &c.t.y] {
        assert_eq!(
            node.state_of(&id).as_deref(),
            Some("already_joined"),
            "a join then a leave still supersedes the invitation"
        );
        assert_eq!(
            node.membership(&c.thread, &c.invitee),
            Some(("left".into(), 2, 1))
        );
    }
}

#[test]
fn invite_vs_seat_retired() {
    let c = cast(130, true);
    // Concurrent: A invites the seat; B retires it.
    let id = c.t.a.invite_fact(&c.thread, &c.inviter, &c.invitee);
    begin_retirement(&c.t.b, &c.invitee).unwrap();
    c.t.check();
    for node in [&c.t.x, &c.t.y] {
        assert_eq!(
            node.state_of(&id).as_deref(),
            Some("recipient_retired"),
            "{:?}",
            node.id
        );
        assert!(node.attention(&c.invitee).is_none(), "{:?}", node.id);
    }
}

#[test]
fn send_vs_seat_retired() {
    let t = trio(140);
    let (thread, author) = thread_of(&t.a, "shared");
    t.a.joined(&thread, &author, 1).unwrap();
    let recipient = t.b.local_seat();
    t.b.enroll(&recipient, "codex", "session-recipient")
        .unwrap();
    t.setup_done();
    t.b.joined(&thread, &recipient, 1).unwrap();
    t.setup_done();

    // Concurrent: A sends to the seat; B retires it.
    let message: MessageId =
        t.a.stage_prep(&thread, "ordinary", &[(&recipient, FIVE_MIN, true)], &[]);
    t.a.author_sent(&sent(
        &message,
        &thread,
        &author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(
            &recipient,
            ReceiptObligation::Ack,
            Some(SENT_AT + FIVE_MIN),
        )],
    ))
    .unwrap();
    begin_retirement(&t.b, &recipient).unwrap();
    // B's fence leaves its own attention row for the retiring seat; the
    // message must add nothing to it, and no other node wakes anyone.
    let wake_before = t.b.query(
        "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    );
    t.check();
    for node in [&t.x, &t.y, &t.a, &t.b, &t.c] {
        assert_eq!(
            node.receipt_state_of(&message, &recipient).as_deref(),
            Some("recipient_retired"),
            "{:?}",
            node.id
        );
        if node.id != t.b.id {
            assert!(node.wake_seats().is_empty(), "{:?}: nobody woken", node.id);
        }
    }
    assert_eq!(
        t.b.query(
            "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        ),
        wake_before,
        "settled history wakes nobody on the recipient's node either"
    );
}

#[test]
fn concurrent_registers() {
    let t = trio(150);
    let (sa, sb, sc) = (t.a.local_seat(), t.b.local_seat(), t.c.local_seat());
    let thread = t.a.create_thread(&sa, "start", Some("n0")).unwrap();
    t.b.create_thread(&sb, "b's", None).unwrap();
    t.c.create_thread(&sc, "c's", None).unwrap();
    t.setup_done();
    // Everything below is concurrent: no node sees another's writes.
    for (node, tag) in [(&t.a, "a"), (&t.b, "b"), (&t.c, "c")] {
        node.write_register(&thread, RegisterField::Topic, Some(&format!("topic {tag}")))
            .unwrap();
        node.write_register(&thread, RegisterField::Name, Some(&format!("name {tag}")))
            .unwrap();
    }
    t.a.write_register(&thread, RegisterField::Archived, Some("1"))
        .unwrap();
    t.b.write_register(&thread, RegisterField::Archived, Some("0"))
        .unwrap();
    t.c.write_register(&thread, RegisterField::Archived, Some("1"))
        .unwrap();
    t.c.write_register(&thread, RegisterField::Name, None)
        .unwrap();
    t.check();
    // Each register ends on the same winner everywhere.
    for field in ["topic", "name", "archived"] {
        assert_eq!(
            t.x.register(&thread, field),
            t.y.register(&thread, field),
            "{field}"
        );
        assert_eq!(t.x.register(&thread, field), t.a.register(&thread, field));
    }
}

#[test]
fn accept_vs_second_invitation() {
    let c = cast(160, false);
    let first = c.t.a.invite_fact(&c.thread, &c.inviter, &c.invitee);
    c.t.setup_done();
    // Concurrent: B accepts the first; C invites the same seat (invitee_n = 0).
    c.t.b
        .author_answered(&c.t.b.accept_payload(&c.thread, &first, &c.invitee))
        .unwrap();
    let second =
        c.t.c
            .created_payload(&c.thread, &c.other_inviter, &c.invitee);
    assert_eq!(second.invitee_n, 0);
    c.t.c.author_created(&second).unwrap();
    c.t.check();
    for node in [&c.t.x, &c.t.y] {
        assert_eq!(node.state_of(&first).as_deref(), Some("accepted"));
        assert_eq!(
            node.state_of(&second.invitation).as_deref(),
            Some("already_joined"),
            "the second invitation ends settled in either order"
        );
        assert_eq!(
            node.membership(&c.thread, &c.invitee),
            Some(("joined".into(), 1, 1))
        );
    }
}
