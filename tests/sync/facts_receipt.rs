//! `receipt.settled` (fact-kinds design section 9; ht-5fr.4.12): the payload,
//! its keys, idempotence, the `retired` decoding, origin authority and the fork
//! residual. The two-node convergence, catch-up and hold cases are in
//! `fact_convergence_receipt`.
//!
//! The helpers at the top (a command-capable world, settling a receipt on a
//! [`Node`], shipping one origin's log into another store) are shared with
//! that file.

use herdr_threads::node::NodeUuid;
use herdr_threads::ports::CooperativePermitRequest;
use herdr_threads::ports::StorePort;
use herdr_threads::protocol::authority::{
    CallerClaim, CallerRole, Harness, LivenessFingerprint, ObligationRef,
};
use herdr_threads::protocol::commands::{Ack, DeliveryMode};
use herdr_threads::protocol::ids::{
    ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId, ThreadId,
};
use herdr_threads::protocol::results::{ApiError, CommandResult};
use herdr_threads::store::journal;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::store::{receipts, schema};
use herdr_threads::sync::envelope::Envelope;
use herdr_threads::sync::facts::FactKind;
use herdr_threads::sync::facts::payload::{
    ReceiptObligation, SettleBatch, SettleHow, receipt_settle_key,
};
use herdr_threads::sync::facts::receipt::{ReceiptSettled, ReceiptSettledPayload};
use herdr_threads::sync::journal_port::{IngestOutcome, JournalPort};
use herdr_threads::sync::{BatchLimits, LogKey};
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::write_guard;

use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_seat::{Node, budget, registry};
use crate::facts_thread::sync_all;

// ---- shared fixtures --------------------------------------------------------

/// The settlement instant of the kind-level cases (the hand-moved clock's UTC).
pub const SETTLED_AT: i64 = SENT_AT;

/// The native session the world's agent seats are bound under.
pub fn session_of(seat: &SeatId) -> String {
    format!("session-{}", seat.as_str())
}

/// The payload of one settlement.
pub fn settled(
    message: &MessageId,
    seat: &SeatId,
    how: SettleHow,
    batch: &str,
    last_in_thread: bool,
) -> ReceiptSettledPayload {
    ReceiptSettledPayload {
        message: message.clone(),
        seat: seat.clone(),
        how,
        settled_at: SETTLED_AT,
        batch: SettleBatch {
            id: batch.into(),
            last_in_thread,
        },
    }
}

/// The `ack` event key a batch writes in `thread` for `seat`.
pub fn ack_key(prefix: &str, seat: &SeatId, batch: &str, thread: &ThreadId) -> String {
    format!(
        "{prefix}:{}:{}:{}:{}:{}:{}",
        seat.as_str().len(),
        seat.as_str(),
        batch.len(),
        batch,
        thread.as_str().len(),
        thread.as_str()
    )
}

impl Node {
    /// Authors `receipt.settled` on the journal route, the way `settle_pending`
    /// does (the write guard armed by [`Node::run`]).
    pub fn author_settled(&self, payload: &ReceiptSettledPayload) -> Result<(), ApiError> {
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<ReceiptSettled>(tx, self.ctx(), d, payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    /// Authors `receipt.settled` a second time straight through the journal (a
    /// second entry carrying the same payload).
    pub fn author_settled_again(&self, payload: &ReceiptSettledPayload) -> Result<(), ApiError> {
        self.run(|tx, d| journal::author::<ReceiptSettled>(tx, self.ctx(), d, payload).map(|_| ()))
    }

    /// `(state, acked_at, settle_batch)` of a `receipt_state` row.
    pub fn settled_row(
        &self,
        message: &MessageId,
        seat: &SeatId,
    ) -> Option<(String, Option<i64>, Option<String>)> {
        self.query(
            &format!(
                "SELECT state,acked_at,settle_batch FROM receipt_state WHERE message_id='{}' AND seat_id='{}'",
                message.as_str(),
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .into_iter()
        .next()
    }

    /// `(event_key, event_json)` of the `ack` events of a thread, by key.
    pub fn ack_events(&self, thread: &ThreadId) -> Vec<(String, String)> {
        self.query(
            &format!(
                "SELECT event_key,event_json FROM messages WHERE thread_id='{}' AND event_key LIKE 'ack%' ORDER BY event_key",
                thread.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    /// Gives `seat` the cooperative binding a real command resolves (the node
    /// must come from [`Node::open_commands`]).
    pub fn bind_commands(&self, seat: &SeatId) {
        self.bind_commands_as(seat, Harness::Codex);
    }

    pub fn bind_commands_as(&self, seat: &SeatId, harness: Harness) {
        let db = self.raw();
        db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        db.execute(
            "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) \
             VALUES ((SELECT min(host_id) FROM herdr_hosts),'p-'||?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)",
            [seat.as_str()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) \
             VALUES (?1,1,1,'p-'||?1,'b',1,?4,?2,?3,'cooperative_top_level',0,'term-'||?1,'inc',0)",
            rusqlite::params![seat.as_str(), session_of(seat), execution_of(seat), harness.as_str()],
        )
        .unwrap();
    }

    /// The canonical claim for a binding installed by `bind_commands_as`.
    pub fn completion_claim(&self, seat: &SeatId, harness: Harness) -> CallerClaim {
        CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
            instance: self.id.to_string(),
            seat: seat.clone(),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new(session_of(seat)),
            execution: ExecutionId::new(execution_of(seat)),
            target: HostTargetId::new(format!("p-{}", seat.as_str())),
            agent_id: None,
            liveness: Some(LivenessFingerprint::TEST),
        }
    }

    /// The real `ack` (or `ack_displayed`) command of `seat` over `messages`,
    /// the write guard armed on its connection.
    pub fn ack(
        &self,
        seat: &SeatId,
        operation: &str,
        messages: &[&MessageId],
        displayed: bool,
    ) -> Result<CommandResult, ApiError> {
        let request = Ack {
            messages: messages.iter().map(|m| (*m).clone()).collect(),
            operation: OperationId::new(operation),
            claim: self.completion_claim(seat, Harness::Codex),
        };
        let payload = if displayed {
            receipts::ack_displayed_payload(&request)
        } else {
            receipts::ack_payload(&request)
        };
        let mut permit = self.store.issue_cooperative_permit(
            CooperativePermitRequest {
                claim: request.claim.clone(),
                operation: request.operation.clone(),
                obligation: ObligationRef::CheckIn(seat.clone()),
                payload_hash: schema::canonical_digest(&payload).unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )?;
        let mut conn = self.raw();
        let _guard = write_guard::arm(&conn);
        if displayed {
            receipts::ack_displayed(self.ctx(), &mut conn, &budget(), &request, &mut permit)
        } else {
            receipts::ack(self.ctx(), &mut conn, &budget(), &request, &mut permit)
        }
    }
}

fn execution_of(seat: &SeatId) -> String {
    // A UUID-shaped execution ID derived from the seat's tail.
    let tail: String = seat
        .as_str()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .chars()
        .rev()
        .take(12)
        .collect();
    format!("00000000-0000-4000-8000-{tail:0>12}")
}

/// A (owns the thread and sends), B (command-capable; hosts `agent` and
/// `peer`), both seats joined on both nodes.
pub struct RWorld {
    pub a: Node,
    pub b: Node,
    pub thread: ThreadId,
    pub author: SeatId,
    pub agent: SeatId,
    pub peer: SeatId,
}

/// Node seeds `seed` (A) and `seed + 1` (B).
pub fn rworld(seed: u8) -> RWorld {
    rworld_as(seed, Harness::Codex)
}

/// The receipt world with canonical bindings for the requested delivery policy.
pub fn rworld_as(seed: u8, harness: Harness) -> RWorld {
    let reg = registry();
    let a = Node::with(seed, reg);
    let b = Node::open_commands(seed + 1, u64::from(seed + 1), reg);
    let (thread, author) = thread_of(&a, "shared");
    a.joined(&thread, &author, 1).unwrap();
    let (agent, peer) = (b.local_seat(), b.local_seat());
    for seat in [&agent, &peer] {
        b.bind_commands_as(seat, harness);
        b.publish(seat).unwrap();
    }
    sync_all(&[&a, &b]);
    b.joined(&thread, &agent, 1).unwrap();
    b.joined(&thread, &peer, 1).unwrap();
    sync_all(&[&a, &b]);
    RWorld {
        a,
        b,
        thread,
        author,
        agent,
        peer,
    }
}

impl RWorld {
    /// A authors an ordinary message to `agent` (a timed receipt) in `thread`.
    pub fn send_in(&self, thread: &ThreadId) -> MessageId {
        let message = self
            .a
            .stage_prep(thread, "ordinary", &[(&self.agent, FIVE_MIN, true)], &[]);
        let payload = sent(
            &message,
            thread,
            &self.author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![spec(
                &self.agent,
                ReceiptObligation::Ack,
                Some(SENT_AT + FIVE_MIN),
            )],
        );
        self.a.author_sent(&payload).unwrap();
        message
    }

    pub fn send(&self) -> MessageId {
        self.send_in(&self.thread)
    }

    /// Brings A and B level and projects the receipts of `messages` on both.
    pub fn deliver(&self, messages: &[&MessageId]) {
        sync_all(&[&self.a, &self.b]);
        for message in messages {
            self.a.drive_send(message);
            self.b.drive_send(message);
        }
    }
}

// ---- shipping one origin's log ------------------------------------------------

/// Applies pending entries until the store is drained.
pub fn drain(store: &herdr_threads::store::SqliteStore) {
    for _ in 0..10_000 {
        if !store.apply_pending(&budget()).unwrap().has_more {
            return;
        }
    }
    panic!("a store never drained its apply passes");
}

/// Every entry of `origin`'s logs held by `from`, log by log, in counter order.
pub fn origin_entries(from: &Node, origin: NodeUuid) -> Vec<(LogKey, Vec<Envelope>)> {
    from.store
        .vector(&budget())
        .unwrap()
        .0
        .into_iter()
        .filter(|(log, _)| log.node == origin)
        .map(|(log, head)| {
            let mut entries = Vec::new();
            let mut next = 1;
            while next <= head.counter {
                let batch = from
                    .store
                    .serve(&log, next, BatchLimits::default(), &budget())
                    .unwrap();
                next = batch.last().expect("a log ends before its head").counter + 1;
                entries.extend(batch);
            }
            (log, entries)
        })
        .collect()
}

/// Ingests `entries` of `log` into `to` (no apply pass).
pub fn ingest(to: &herdr_threads::store::SqliteStore, log: &LogKey, entries: Vec<Envelope>) {
    for chunk in entries.chunks(100) {
        match to.ingest(log, chunk.to_vec(), &budget()).unwrap() {
            IngestOutcome::Stored { .. } => {}
            other => panic!("ingest of {log:?}: {other:?}"),
        }
    }
}

/// Ingests every log `origin` authored that `from` holds into `to`, then
/// drains `to`'s apply passes.
pub fn ship_origin(from: &Node, origin: NodeUuid, to: &herdr_threads::store::SqliteStore) {
    for (log, entries) in origin_entries(from, origin) {
        ingest(to, &log, entries);
    }
    drain(to);
}

// ---- payload, keys, registration ----------------------------------------------

#[test]
fn round_trip_keys() {
    let w = rworld(150);
    let message = w.send();
    w.deliver(&[&message]);
    let payload = settled(&message, &w.agent, SettleHow::Acked, "op-1", true);

    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        format!(
            r#"{{"message":"{}","seat":"{}","how":"acked","settled_at":{SETTLED_AT},"batch":{{"id":"op-1","last_in_thread":true}}}}"#,
            message.as_str(),
            w.agent.as_str()
        )
    );
    assert_eq!(
        serde_json::from_str::<ReceiptSettledPayload>(&json).unwrap(),
        payload
    );
    // Additive fields default and unknown ones are ignored.
    let bare: ReceiptSettledPayload = serde_json::from_str(&format!(
        r#"{{"message":"{}","seat":"{}","how":"policy","settled_at":1,"future":[1]}}"#,
        message.as_str(),
        w.agent.as_str()
    ))
    .unwrap();
    assert_eq!(bare.batch, SettleBatch::default());
    assert_eq!(bare.how, SettleHow::Policy);

    let key = receipt_settle_key(&message, &w.agent).unwrap();
    assert_eq!(ReceiptSettled::creates(&payload), Some(key.clone()));
    assert_eq!(
        ReceiptSettled::needs(&payload)
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect::<Vec<_>>(),
        vec![message.as_str().to_owned()]
    );

    w.b.author_settled(&payload).unwrap();
    assert_eq!(
        w.b.query(
            "SELECT creates FROM journal WHERE kind='receipt.settled'",
            |r| r.get::<_, String>(0)
        ),
        vec![key.as_str().to_owned()],
        "the journal row creates m/s"
    );
    sync_all(&[&w.a, &w.b]);
    for node in [&w.a, &w.b] {
        assert!(
            node.journal()
                .iter()
                .all(|(_, state, _)| state == "applied")
        );
        assert_eq!(
            node.settled_row(&message, &w.agent),
            Some(("acked".into(), Some(SETTLED_AT), Some("op-1".into())))
        );
        assert_eq!(
            node.ack_events(&w.thread)
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>(),
            vec![ack_key("ack", &w.agent, "op-1", &w.thread)]
        );
    }
}

#[test]
fn mod_delivered_and_policy_write_the_ack_mod_key() {
    let w = rworld(152);
    let (m1, m2) = (w.send(), w.send());
    w.deliver(&[&m1, &m2]);
    w.b.author_settled(&settled(
        &m1,
        &w.agent,
        SettleHow::ModDelivered,
        "op-m",
        true,
    ))
    .unwrap();
    w.b.author_settled(&settled(&m2, &w.agent, SettleHow::Policy, "op-p", true))
        .unwrap();
    sync_all(&[&w.a, &w.b]);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.ack_events(&w.thread)
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>(),
            vec![
                ack_key("ack_mod", &w.agent, "op-m", &w.thread),
                ack_key("ack_mod", &w.agent, "op-p", &w.thread),
            ]
        );
    }
}

#[test]
fn retired_how_materializes_recipient_retired() {
    let w = rworld(154);
    let message = w.send();
    w.deliver(&[&message]);
    w.b.author_settled(&settled(
        &message,
        &w.agent,
        SettleHow::Retired,
        "op-r",
        true,
    ))
    .unwrap();
    sync_all(&[&w.a, &w.b]);
    for node in [&w.a, &w.b] {
        let row = node.settled_row(&message, &w.agent).expect("a row");
        assert_eq!(row.0, "recipient_retired");
        assert_eq!(row.1, None, "never acked");
        assert!(node.ack_events(&w.thread).is_empty(), "no ack event");
    }
}

// ---- idempotence --------------------------------------------------------------

#[test]
fn apply_twice_noop() {
    let w = rworld(156);
    let message = w.send();
    w.deliver(&[&message]);
    let payload = settled(&message, &w.agent, SettleHow::Acked, "op-1", true);
    w.b.author_settled(&payload).unwrap();
    sync_all(&[&w.a, &w.b]);
    let before: Vec<_> = [&w.a, &w.b]
        .iter()
        .map(|n| (n.settled_row(&message, &w.agent), n.ack_events(&w.thread)))
        .collect();
    let revisions = w.a.query(
        "SELECT revision FROM filter_revisions ORDER BY scope_kind,scope_key",
        |r| r.get::<_, i64>(0),
    );

    // A second entry carrying the same payload.
    w.b.author_settled_again(&payload).unwrap();
    sync_all(&[&w.a, &w.b]);
    assert_eq!(w.b.count_kind("receipt.settled"), 2);
    for (node, before) in [&w.a, &w.b].iter().zip(&before) {
        assert!(
            node.journal()
                .iter()
                .all(|(_, state, _)| state == "applied")
        );
        assert_eq!(
            &(
                node.settled_row(&message, &w.agent),
                node.ack_events(&w.thread)
            ),
            before,
            "no second row write or event"
        );
    }
    assert_eq!(
        w.a.query(
            "SELECT revision FROM filter_revisions ORDER BY scope_kind,scope_key",
            |r| r.get::<_, i64>(0)
        ),
        revisions,
        "no second filter bump"
    );
}

#[test]
fn reauthored_second_incarnation_no_second_ack_event() {
    let w = rworld(158);
    let message = w.send();
    w.deliver(&[&message]);
    let payload = settled(&message, &w.agent, SettleHow::Acked, "op-1", true);
    w.b.author_settled(&payload).unwrap();
    sync_all(&[&w.a, &w.b]);
    let events = w.a.ack_events(&w.thread);
    assert_eq!(events.len(), 1);

    // B restarts as a second incarnation and re-authors the same payload.
    w.b.run(|tx, d| journal::begin_incarnation(tx, w.b.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    w.b.author_settled_again(&payload).unwrap();
    sync_all(&[&w.a, &w.b]);
    assert_eq!(w.b.count_kind("receipt.settled"), 2);
    for node in [&w.a, &w.b] {
        assert!(
            node.journal()
                .iter()
                .all(|(_, state, _)| state == "applied")
        );
        assert_eq!(node.ack_events(&w.thread), events, "no second ack event");
        assert_eq!(
            node.settled_row(&message, &w.agent),
            Some(("acked".into(), Some(SETTLED_AT), Some("op-1".into())))
        );
    }
}

// ---- origin authority ----------------------------------------------------------

#[test]
fn foreign_origin_settlement_failed_receipt_pending() {
    let w = rworld(160);
    let c = Node::with(162, registry());
    let message = w.send();
    sync_all(&[&w.a, &w.b, &c]);
    w.a.drive_send(&message);
    w.b.drive_send(&message);

    // C authors a settlement for B's seat; C's own apply passes (local
    // origin), every other node rejects it.
    c.author_settled(&settled(&message, &w.agent, SettleHow::Acked, "op-c", true))
        .unwrap();
    sync_all(&[&w.a, &w.b, &c]);
    for node in [&w.a, &w.b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "receipt.settled" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        let error = failed[0].2.as_deref().unwrap();
        assert!(
            error.contains("is not the authorized author of receipt.settled for")
                && error.contains(w.agent.as_str()),
            "{error}"
        );
        assert_ne!(
            node.settled_row(&message, &w.agent).map(|r| r.0),
            Some("acked".into()),
            "the receipt is not settled"
        );
        assert!(node.ack_events(&w.thread).is_empty(), "no event");
    }

    // The authorized author's own fact passes everywhere.
    w.b.author_settled(&settled(&message, &w.agent, SettleHow::Acked, "op-b", true))
        .unwrap();
    sync_all(&[&w.a, &w.b, &c]);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.settled_row(&message, &w.agent).map(|r| r.0),
            Some("acked".into())
        );
    }
}

// ---- the fork residual -----------------------------------------------------------

/// Two branches of B's log (two incarnations of one node) each settle one
/// receipt with a different `how`. A store keeps whichever branch it applied
/// first (root section 12, "forked-log same-key facts": accepted, asserted
/// here as documented behaviour).
#[test]
fn two_branches_different_how_first_applied() {
    let reg = registry();
    let a = Node::with(164, reg);
    // Two incarnations of node B (same identity, different logs).
    let b1 = Node::open(165, 1, reg, true);
    let b2 = Node::open(165, 2, reg, true);
    assert_eq!(b1.id, b2.id);
    let (c, d) = (Node::with(166, reg), Node::with(167, reg));
    let (thread, author) = thread_of(&a, "a's");
    let seat = b1.mint_seat(0);
    b1.add_local_seat(&seat);
    b2.add_local_seat(&seat);
    b1.publish(&seat).unwrap();
    b2.publish(&seat).unwrap();
    for other in [&b1, &b2, &c, &d] {
        fact_exchange::sync(&a.store, &other.store);
    }
    let message = a.stage_prep(&thread, "ordinary", &[(&seat, FIVE_MIN, true)], &[]);
    a.author_sent(&sent(
        &message,
        &thread,
        &author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(
            &seat,
            ReceiptObligation::Ack,
            Some(SENT_AT + FIVE_MIN),
        )],
    ))
    .unwrap();
    for other in [&b1, &b2, &c, &d] {
        fact_exchange::sync(&a.store, &other.store);
        other.drive_send(&message);
    }

    // Branch 1 acks, branch 2 reports the mod delivery.
    b1.author_settled(&settled(&message, &seat, SettleHow::Acked, "op-1", true))
        .unwrap();
    b2.author_settled(&settled(
        &message,
        &seat,
        SettleHow::ModDelivered,
        "op-2",
        true,
    ))
    .unwrap();
    // C meets branch 1 first, D branch 2 first.
    fact_exchange::sync(&d.store, &b2.store);
    fact_exchange::sync(&c.store, &b1.store);
    fact_exchange::sync(&c.store, &b2.store);
    fact_exchange::sync(&d.store, &b1.store);

    let row = |n: &Node| n.settled_row(&message, &seat).unwrap();
    assert_eq!(
        row(&c),
        ("acked".into(), Some(SETTLED_AT), Some("op-1".into()))
    );
    assert_eq!(
        row(&d),
        ("acked".into(), Some(SETTLED_AT), Some("op-2".into())),
        "each store keeps the branch it applied first"
    );
    let keys = |n: &Node| {
        n.ack_events(&thread)
            .into_iter()
            .map(|(k, _)| k)
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&c), vec![ack_key("ack", &seat, "op-1", &thread)]);
    assert_eq!(keys(&d), vec![ack_key("ack_mod", &seat, "op-2", &thread)]);
    for node in [&c, &d] {
        assert!(
            node.journal()
                .iter()
                .all(|(_, state, _)| state == "applied")
        );
    }
}
