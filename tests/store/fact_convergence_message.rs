//! Two-store convergence of `message.sent` for remote recipients (fact-kinds
//! design section 8; ht-5fr.4.16): the author stages a remote recipient, every
//! other node synthesizes the preparation from the payload, a suppressed local
//! recipient gets no lazy row, and cleanup leaves a `replicated` preparation
//! alone.
//!
//! The settlement side is the test kind `test.settle` below: it creates the
//! `receipt_settle_key` the way `receipt.settled` will, which is what catch-up
//! suppression looks for.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use herdr_threads::ports::DurableWorkAdmission;
use herdr_threads::protocol::commands::DeliveryMode;
use herdr_threads::protocol::ids::{MessageId, SeatId, ThreadId, send_preparation_id_for_message};
use herdr_threads::store::journal;
use herdr_threads::store::materialization::advance_work;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::store::messages;
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::payload::{ReceiptObligation, ReferencedSeats, receipt_settle_key};
use herdr_threads::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
use herdr_threads::test_support::{assert_no_local_delivery_for_remote, fact_exchange};

use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_seat::{Node, budget};
use crate::facts_thread::sync_all;

// ---- the stand-in settlement kind --------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SettlePayload {
    message: MessageId,
    seat: SeatId,
}

impl ReferencedSeats for SettlePayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

struct TestSettle;

impl FactKind for TestSettle {
    const NAME: &'static str = "test.settle";
    const MAJOR: u32 = 1;
    type Payload = SettlePayload;

    fn creates(p: &SettlePayload) -> Option<EntityKey> {
        receipt_settle_key(&p.message, &p.seat).ok()
    }

    fn needs(_: &SettlePayload) -> Vec<EntityKey> {
        Vec::new()
    }

    fn apply(
        _: &Transaction<'_>,
        _: &mut ApplyContext<'_>,
        _: SettlePayload,
    ) -> Result<(), FactError> {
        Ok(())
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<TestSettle>())
}

// ---- fixtures -------------------------------------------------------------------

impl Node {
    /// Authors the settlement of `message` for `seat` (this node's own).
    fn settle(&self, message: &MessageId, seat: &SeatId) {
        let payload = SettlePayload {
            message: message.clone(),
            seat: seat.clone(),
        };
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<TestSettle>(tx, self.ctx(), d, &payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
        .unwrap();
    }

    /// Runs the message's `send_attention` job to completion; a message
    /// without receipts or warnings has none (D9), so there is nothing to run.
    pub fn drive_send(&self, message: &MessageId) {
        let mut db = self.raw();
        let job = format!("work:send:{}", message.as_str());
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM work_jobs WHERE id=?1)",
                [&job],
                |r| r.get(0),
            )
            .unwrap();
        if !exists {
            return;
        }
        for _ in 0..64 {
            let progress = advance_work(
                &mut db,
                &job,
                DurableWorkAdmission { max_units: 16 },
                &budget(),
                &*self.clock,
            )
            .unwrap();
            assert!(progress.last_error.is_none(), "{:?}", progress.last_error);
            if !progress.has_more {
                return;
            }
        }
        panic!("the send job did not finish");
    }

    pub fn receipt(&self, message: &MessageId, seat: &SeatId) -> Option<(String, Option<i64>)> {
        self.query(
            &format!(
                "SELECT state,deadline_at FROM receipt_state WHERE message_id='{}' AND seat_id='{}'",
                message.as_str(),
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .into_iter()
        .next()
    }

    pub fn wake_seats(&self) -> Vec<String> {
        self.query("SELECT seat_id FROM wake_work ORDER BY seat_id", |r| {
            r.get(0)
        })
    }

    fn prep_origin(&self, message: &MessageId) -> Option<String> {
        self.query(
            &format!(
                "SELECT origin FROM send_preparations WHERE operation_key='{0}' OR id='{1}'",
                message.as_str(),
                send_preparation_id_for_message(message.as_str()).unwrap()
            ),
            |r| r.get(0),
        )
        .into_iter()
        .next()
    }

    pub fn suppressed_seats(&self, message: &MessageId) -> Option<String> {
        self.query(
            &format!(
                "SELECT suppressed_seats FROM send_manifests WHERE message_id='{}'",
                message.as_str()
            ),
            |r| r.get::<_, Option<String>>(0),
        )
        .into_iter()
        .next()
        .flatten()
    }
}

/// A (the author and thread owner), B (hosts the recipients), both with the
/// thread and B's seats joined on both.
struct World {
    a: Node,
    b: Node,
    thread: ThreadId,
    author: SeatId,
    agent: SeatId,
    human: SeatId,
}

fn world(seed: u8) -> World {
    let reg = registry();
    let (a, b) = (Node::with(seed, reg), Node::with(seed + 1, reg));
    let (thread, author) = thread_of(&a, "shared");
    a.joined(&thread, &author, 1).unwrap();
    let (agent, human) = (b.local_seat(), b.local_seat());
    b.enroll(&agent, "codex", "b-agent").unwrap();
    b.enroll(&human, "human", "b-human").unwrap();
    b.publish(&agent).unwrap();
    b.publish(&human).unwrap();
    sync_all(&[&a, &b]);
    b.joined(&thread, &agent, 1).unwrap();
    b.joined(&thread, &human, 1).unwrap();
    sync_all(&[&a, &b]);
    for node in [&a, &b] {
        assert!(
            node.membership(&thread, &agent)
                .is_some_and(|m| m.0 == "joined")
        );
    }
    World {
        a,
        b,
        thread,
        author,
        agent,
        human,
    }
}

impl World {
    /// A stages and authors an ordinary message: `agent` owes a timed
    /// receipt, `human` nothing.
    fn send_ordinary(&self) -> MessageId {
        let message = self.a.stage_prep(
            &self.thread,
            "ordinary",
            &[(&self.agent, FIVE_MIN, true)],
            &[],
        );
        let payload = sent(
            &message,
            &self.thread,
            &self.author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![
                spec(
                    &self.agent,
                    ReceiptObligation::Ack,
                    Some(SENT_AT + FIVE_MIN),
                ),
                spec(&self.human, ReceiptObligation::None, None),
            ],
        );
        self.a.author_sent(&payload).unwrap();
        message
    }

    fn check_no_remote_delivery(&self) {
        for node in [&self.a, &self.b] {
            assert_no_local_delivery_for_remote(&node.raw());
        }
    }
}

// ---- convergence ----------------------------------------------------------------

#[test]
fn send_to_remote_seat_reaches_b() {
    let w = world(130);
    let message = w.send_ordinary();
    // A keeps its sealed local preparation and has not woken the remote seat.
    assert_eq!(w.a.prep_origin(&message).as_deref(), Some("local"));
    w.a.drive_send(&message);
    assert_eq!(
        w.a.receipt(&message, &w.agent),
        Some(("pending".into(), Some(SENT_AT + FIVE_MIN))),
        "A holds the receipt row"
    );
    assert!(
        w.a.wake_seats().is_empty(),
        "no wake_work for the remote seat on A"
    );

    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(w.b.prep_origin(&message).as_deref(), Some("replicated"));
    assert_eq!(
        w.b.count(&format!(
            "SELECT count(*) FROM messages WHERE id='{}' AND kind='ordinary'",
            message.as_str()
        )),
        1,
        "B has the message"
    );
    assert_eq!(
        w.b.count("SELECT count(*) FROM prepared_unavailable_warnings"),
        0
    );
    w.b.drive_send(&message);
    assert_eq!(
        w.b.receipt(&message, &w.agent),
        Some(("pending".into(), Some(SENT_AT + FIVE_MIN))),
        "B has a pending receipt for its local seat"
    );
    // The agent's receipt and the human's p0 delivery row (D9) both admit at
    // T = 0; A wakes neither remote seat.
    let mut local = vec![w.agent.as_str().to_owned(), w.human.as_str().to_owned()];
    local.sort();
    assert_eq!(
        w.b.wake_seats(),
        local,
        "wake_work for its local seats only"
    );
    w.check_no_remote_delivery();
}

#[test]
fn human_remote_recipient_no_ack_expected() {
    let w = world(132);
    let message = w.send_ordinary();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        node.drive_send(&message);
        assert_eq!(
            node.count(&format!(
                "SELECT count(*) FROM prepared_recipients WHERE seat_id='{}'",
                w.human.as_str()
            )),
            0,
            "no prepared row for the human on {:?}",
            node.id
        );
        assert_eq!(node.receipt(&message, &w.human), None);
    }
    // D9: the human owes no receipt; it gets a delivery row at the message
    // priority (p0) on its own node only.
    assert_eq!(
        w.b.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='{}' AND priority=0",
            w.human.as_str()
        )),
        1,
        "a p0 delivery row for the human on B"
    );
    assert_eq!(
        w.a.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='{}'",
            w.human.as_str()
        )),
        0,
        "no delivery row for the remote human on A"
    );
    w.check_no_remote_delivery();
}

#[test]
fn own_old_incarnation_pull_back_wakes_nobody() {
    let w = world(134);
    // A's own old incarnation authored a message to B's agent; the
    // preparation died with the wipe, so the pulled-back copy has none.
    let message = MessageId::new(
        herdr_threads::protocol::ids::send_message_id_for_preparation(
            &herdr_threads::protocol::ids::generate_public_id(
                herdr_threads::protocol::ids::prefix::SEND_PREPARATION,
                &herdr_threads::node::tag_for(w.a.id, 0),
            ),
        )
        .unwrap(),
    );
    let payload = sent(
        &message,
        &w.thread,
        &w.author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(
            &w.agent,
            ReceiptObligation::Ack,
            Some(SENT_AT + FIVE_MIN),
        )],
    );
    w.a.run(|tx, d| journal::begin_incarnation(tx, w.a.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    w.a.author_sent_again(&payload).unwrap();
    assert_eq!(w.a.prep_origin(&message).as_deref(), Some("replicated"));
    w.a.drive_send(&message);
    assert!(w.a.wake_seats().is_empty(), "nobody is woken on A");
    assert_eq!(w.a.count("SELECT count(*) FROM lazy_recipients"), 0);
    assert!(
        w.a.query(
            &format!(
                "SELECT native_observation FROM messages WHERE id='{}'",
                message.as_str()
            ),
            |r| r.get::<_, String>(0)
        )[0]
        .contains("local_journal"),
        "attributed as own journal history"
    );
    w.check_no_remote_delivery();
}

#[test]
fn suppressed_local_recipient_no_lazy_row_listed() {
    let w = world(136);
    // B has already settled the message its agent has not yet received.
    let message = w.a.stage_prep(&w.thread, "lazy", &[], &[]);
    let payload = sent(
        &message,
        &w.thread,
        &w.author,
        DeliveryMode::Lazy,
        0,
        vec![
            spec(&w.agent, ReceiptObligation::None, None),
            spec(&w.human, ReceiptObligation::None, None),
        ],
    );
    w.b.settle(&message, &w.agent);
    w.a.author_sent(&payload).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);

    assert_eq!(
        w.b.count("SELECT count(*) FROM lazy_recipients"),
        1,
        "B's human gets a lazy row"
    );
    assert_eq!(
        w.b.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='{}'",
            w.agent.as_str()
        )),
        0,
        "the suppressed agent gets none"
    );
    assert_eq!(
        w.b.suppressed_seats(&message).as_deref(),
        Some(w.agent.as_str())
    );
    assert_eq!(w.a.suppressed_seats(&message), None, "A suppressed nobody");
    w.check_no_remote_delivery();
}

#[test]
fn suppressed_ack_recipient_keeps_receipt_but_no_wake() {
    let w = world(138);
    let message =
        w.a.stage_prep(&w.thread, "ordinary", &[(&w.agent, FIVE_MIN, true)], &[]);
    let payload = sent(
        &message,
        &w.thread,
        &w.author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        // The agent alone: a p0 delivery row for the human would wake it (D9).
        vec![spec(
            &w.agent,
            ReceiptObligation::Ack,
            Some(SENT_AT + FIVE_MIN),
        )],
    );
    w.b.settle(&message, &w.agent);
    w.a.author_sent(&payload).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    w.b.drive_send(&message);
    assert_eq!(
        w.b.suppressed_seats(&message).as_deref(),
        Some(w.agent.as_str())
    );
    assert!(
        w.b.receipt(&message, &w.agent).is_some(),
        "the receipt row is still replicated state"
    );
    assert!(w.b.wake_seats().is_empty(), "settled history wakes nobody");
    w.check_no_remote_delivery();
}

// ---- idempotence on the receiver -------------------------------------------------

#[test]
fn receiver_apply_twice_noop() {
    let w = world(140);
    let message = w.send_ordinary();
    fact_exchange::sync(&w.a.store, &w.b.store);
    w.b.drive_send(&message);
    let rows = w.b.publication_rows();
    let wake: Vec<_> = w.b.query(
        "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    );
    // A second entry carrying the same payload reaches B.
    let payload = copy_of(&w.a);
    w.a.author_sent_again(&payload).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(w.b.count_kind("message.sent"), 2);
    assert!(w.b.journal().iter().all(|(_, state, _)| state == "applied"));
    assert_eq!(w.b.publication_rows(), rows, "no second publication");
    assert_eq!(
        w.b.count("SELECT count(*) FROM send_preparations"),
        1,
        "no second preparation"
    );
    assert_eq!(
        w.b.query(
            "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        ),
        wake,
        "no second attention write"
    );
    w.check_no_remote_delivery();
}

#[test]
fn receiver_reauthored_second_incarnation_noop() {
    let w = world(142);
    let message = w.send_ordinary();
    fact_exchange::sync(&w.a.store, &w.b.store);
    w.b.drive_send(&message);
    let rows = w.b.publication_rows();
    let wake: Vec<_> = w.b.query(
        "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    );
    // A restarts as a second incarnation and re-authors the same payload.
    let payload = copy_of(&w.a);
    w.a.run(|tx, d| journal::begin_incarnation(tx, w.a.ctx(), d, 78, None, "host").map(|_| ()))
        .unwrap();
    w.a.author_sent_again(&payload).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert!(w.b.journal().iter().all(|(_, state, _)| state == "applied"));
    assert_eq!(
        w.b.publication_rows(),
        rows,
        "no second publication or event"
    );
    assert_eq!(w.b.count("SELECT count(*) FROM send_preparations"), 1);
    assert_eq!(
        w.b.query(
            "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        ),
        wake
    );
    w.check_no_remote_delivery();
}

/// The `message.sent` payload `node` journaled first.
fn copy_of(node: &Node) -> herdr_threads::sync::facts::message::MessageSentPayload {
    let bytes = node.query(
        "SELECT payload FROM journal WHERE kind='message.sent' ORDER BY ingest_seq LIMIT 1",
        |r| r.get::<_, Vec<u8>>(0),
    );
    serde_json::from_slice(&bytes[0]).unwrap()
}

// ---- cleanup and abandon ----------------------------------------------------------

#[test]
fn cleanup_and_abandon_leave_replicated_prep() {
    let w = world(144);
    let message = w.send_ordinary();
    fact_exchange::sync(&w.a.store, &w.b.store);
    let prep = send_preparation_id_for_message(message.as_str()).unwrap();
    let snapshot = |n: &Node| {
        format!(
            "{:?} {} {}",
            n.query(
                &format!("SELECT status,origin FROM send_preparations WHERE id='{prep}'"),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            ),
            n.count("SELECT count(*) FROM prepared_recipients"),
            n.count("SELECT count(*) FROM lazy_recipients"),
        )
    };
    let before = snapshot(&w.b);

    // Abandon: the preparation is published.
    assert!(!messages::abandon_send_preparation(&mut w.b.raw(), &prep).unwrap());
    assert_eq!(snapshot(&w.b), before);

    // The same with the manifest gone, so only the origin guards it.
    w.b.raw()
        .execute(
            "DELETE FROM send_manifests WHERE preparation_id=?1",
            [&prep],
        )
        .unwrap();
    assert!(!messages::abandon_send_preparation(&mut w.b.raw(), &prep).unwrap());
    assert_eq!(snapshot(&w.b), before);

    // Retention: an old unpublished-looking preparation is not discarded.
    w.b.raw()
        .execute(
            "UPDATE send_preparations SET prepared_at=1 WHERE id=?1",
            [&prep],
        )
        .unwrap();
    w.b.store.prune_retention(&budget()).unwrap();
    assert_eq!(snapshot(&w.b), before, "retention left it");

    // A cleanup job against it refuses and deletes nothing.
    w.b.raw()
        .execute(
            "UPDATE send_preparations SET status='discarded' WHERE id=?1",
            [&prep],
        )
        .unwrap();
    w.b.raw()
        .execute(
            "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('work:cleanup:x','preparation_cleanup',?1,0)",
            [&prep],
        )
        .unwrap();
    let mut db = w.b.raw();
    let progress = advance_work(
        &mut db,
        "work:cleanup:x",
        DurableWorkAdmission { max_units: 16 },
        &budget(),
        &*w.b.clock,
    )
    .unwrap();
    assert!(progress.last_error.is_some(), "the cleanup job refused");
    assert_eq!(
        w.b.count("SELECT count(*) FROM prepared_recipients"),
        before
            .split_whitespace()
            .nth(2)
            .unwrap()
            .parse::<i64>()
            .unwrap()
    );
    w.check_no_remote_delivery();
}
