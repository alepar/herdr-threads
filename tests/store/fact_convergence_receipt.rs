//! Two-store convergence of `receipt.settled` (fact-kinds design section 9;
//! ht-5fr.4.12): a recipient's ACK reaches the sender's node with one derived
//! `ack` event per batch and thread, catch-up after a wipe suppresses the
//! delivery of what the seat already settled (with and without the restore
//! window's own-history hold), the hold refuses or skips new settlements, and
//! every log order ends in the same replicated rows.
//!
//! The helpers (a command-capable world, shipping one origin's log into
//! another store) are in `facts_receipt`.

use herdr_threads::ports::StorePort;
use herdr_threads::protocol::authority::Harness;
use herdr_threads::protocol::commands::{
    Ack, AckModDelivered, CompleteInboxDelivery, DeliveryMode, LazyCompletionVia, PermitMutation,
};
use herdr_threads::protocol::delivery::DeliveryConfirm;
use herdr_threads::protocol::ids::{MessageId, OperationId, SeatId};
use herdr_threads::protocol::results::{ApiError, CommandResult, ErrorCode};
use herdr_threads::protocol::watch::ModDeliveryVia;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::sync::facts::payload::ReceiptObligation;
use herdr_threads::sync::facts::seat::SeatRetired;
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::journal_oracle::{
    OracleStore, replicated_projection, replicated_tables,
};

use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_receipt::{
    RWorld, SETTLED_AT, ack_key, drain, ingest, origin_entries, rworld, rworld_as, settled,
    ship_origin,
};
use crate::facts_seat::{Node, budget, hold_off, hold_on, registry, retired_payload};
use crate::facts_thread::sync_all;

impl Node {
    fn lazy_rows(&self) -> i64 {
        self.count("SELECT count(*) FROM lazy_recipients")
    }

    fn pending_kinds(&self, kind: &str) -> usize {
        self.journal()
            .iter()
            .filter(|(k, state, _)| k == kind && state == "pending")
            .count()
    }
}

/// A wiped store of `world`'s node B: same identity, another incarnation.
fn wiped_b(world: &RWorld, seed: u8) -> Node {
    let b2 = Node::open_commands(seed, 999, registry());
    assert_eq!(b2.id, world.b.id, "the same node");
    b2
}

// These calls deliberately enter through StorePort, not `author_settled`: losing the
// receipt fact in any public completion path must break cross-node convergence.
// Service channel/offer admission and output-flush evidence are tested separately.
#[derive(Clone, Copy)]
enum CompletionPath {
    Inbox,
    Mod,
    HookPull,
    HookContext,
}

impl CompletionPath {
    fn harness(self) -> Harness {
        match self {
            Self::Inbox | Self::HookContext => Harness::Codex,
            Self::Mod => Harness::Claude,
            Self::HookPull => Harness::Agent(
                herdr_threads::harness::registry::builtins()
                    .agent("agy")
                    .unwrap(),
            ),
        }
    }

    fn complete(self, w: &RWorld, id: &MessageId, operation: &str) {
        self.try_complete(w, id, operation).unwrap();
    }

    fn try_complete(
        self,
        w: &RWorld,
        id: &MessageId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let claim = w.b.completion_claim(&w.agent, self.harness());
        let operation = OperationId::new(operation);
        let mutation = match self {
            Self::Inbox => PermitMutation::AckDisplayed(Ack {
                messages: vec![id.clone()],
                operation,
                claim,
            }),
            Self::Mod => PermitMutation::AckModDelivered(AckModDelivered {
                row: None,
                via: ModDeliveryVia::Context,
                messages: vec![id.clone()],
                operation,
                claim,
            }),
            Self::HookPull => PermitMutation::DeliveryConfirm(DeliveryConfirm {
                ids: vec![id.clone()],
                operation,
                claim,
            }),
            Self::HookContext => PermitMutation::CompleteInboxDelivery(CompleteInboxDelivery {
                messages: vec![],
                receipt_candidates: vec![id.clone()],
                via: Some(LazyCompletionVia::HookContext),
                operation,
                claim,
            }),
        };
        let permit = w.b.store.issue_cooperative_permit(
            herdr_threads::store::cooperative_permit_request(&mutation)?,
            &budget(),
        )?;
        w.b.store.mutate(mutation, permit, &budget())
    }
}

fn public_completion_replicates_only_shown_ids(path: CompletionPath) {
    let w = rworld_as(230, path.harness());
    let (shown, withheld) = (w.send(), w.send());
    w.deliver(&[&shown, &withheld]);
    path.complete(&w, &shown, "display-1");
    assert_eq!(w.b.settled_row(&shown, &w.agent).unwrap().0, "acked");
    assert_eq!(
        w.a.settled_row(&shown, &w.agent).unwrap().0,
        "pending",
        "the sender remains pending while settlement is offline"
    );
    assert_eq!(w.b.settled_row(&withheld, &w.agent).unwrap().0, "pending");
    assert_eq!(w.b.count_kind("receipt.settled"), 1);

    if matches!(path, CompletionPath::HookContext) {
        let observations: Vec<String> = w.b.query(
            "SELECT ack_observation FROM receipt_state WHERE state='acked'",
            |r| r.get(0),
        );
        assert_eq!(observations.len(), 1);
        let observation: serde_json::Value = serde_json::from_str(&observations[0]).unwrap();
        assert_eq!(observation["action_provenance"], "cooperative_hook_context");
        assert_eq!(observation["via"], "hook_context");
        let facts: Vec<_> = origin_entries(&w.b, w.b.id)
            .into_iter()
            .flat_map(|(_, entries)| entries)
            .filter_map(|entry| entry.as_known().cloned())
            .filter(|entry| entry.kind == "receipt.settled")
            .collect();
        assert_eq!(facts.len(), 1);
        let payload: serde_json::Value = serde_json::from_slice(&facts[0].payload).unwrap();
        assert_eq!(
            payload["how"], "policy",
            "reuse the existing wire settlement kind"
        );
    }

    // Replaying an operation and repeating the display under a fresh operation
    // must not author another fact, including while the sender is offline.
    path.complete(&w, &shown, "display-1");
    path.complete(&w, &shown, "display-2");
    assert_eq!(w.b.count_kind("receipt.settled"), 1);
    ship_origin(&w.b, w.b.id, &w.a.store);
    ship_origin(&w.b, w.b.id, &w.a.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.settled_row(&shown, &w.agent),
            w.b.settled_row(&shown, &w.agent)
        );
        assert_eq!(node.settled_row(&withheld, &w.agent).unwrap().0, "pending");
        assert_eq!(
            node.settled_row(&shown, &w.peer),
            None,
            "display creates no new obligation"
        );
        assert_eq!(node.count_kind("receipt.settled"), 1);
        assert_eq!(node.ack_events(&w.thread).len(), 1);
    }
    assert_eq!(
        w.a.count("SELECT count(*) FROM receipt_due_sparse"),
        1,
        "only the withheld message still has a sender-side deadline"
    );

    // Ship the recipient's REAL signed log first: its settlement and membership
    // depend on the sender's history, so application must wait, then converge.
    let fresh = OracleStore::open([232; 32], registry());
    ship_origin(&w.b, w.b.id, &fresh);
    let pending: i64 = fresh
        .context()
        .open_query(budget())
        .unwrap()
        .query_row(
            "SELECT count(*) FROM journal WHERE apply_state='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pending > 0, "foreign dependencies have not arrived");
    ship_origin(&w.a, w.a.id, &fresh);
    let db = fresh.context().open_query(budget()).unwrap();
    for (id, expected) in [
        (
            &shown,
            herdr_threads::store::effective::EffectiveReceiptState::Acknowledged,
        ),
        (
            &withheld,
            herdr_threads::store::effective::EffectiveReceiptState::Pending,
        ),
    ] {
        let receipt =
            herdr_threads::store::effective::effective_receipt(&db, id.as_str(), w.agent.as_str())
                .unwrap()
                .expect("the message publishes a receipt obligation");
        assert_eq!(receipt.state, expected);
    }
    let unapplied: i64 = db
        .query_row(
            "SELECT count(*) FROM journal WHERE apply_state!='applied'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unapplied, 0, "reordered real logs fully apply");
}

#[test]
fn inbox_display_completion_replicates_only_shown_ids() {
    public_completion_replicates_only_shown_ids(CompletionPath::Inbox);
}

#[test]
fn mod_completion_replicates_only_shown_ids() {
    public_completion_replicates_only_shown_ids(CompletionPath::Mod);
}

#[test]
fn hook_pull_completion_replicates_only_shown_ids() {
    public_completion_replicates_only_shown_ids(CompletionPath::HookPull);
}

#[test]
fn hook_context_completion_replicates_only_shown_ids() {
    public_completion_replicates_only_shown_ids(CompletionPath::HookContext);
}

#[test]
fn hook_context_completion_during_hold_retries_and_replicates_after_release() {
    let w = rworld_as(234, Harness::Codex);
    let message = w.send();
    w.deliver(&[&message]);
    hold_on(&w.b);
    CompletionPath::HookContext
        .try_complete(&w, &message, "held-display")
        .expect_err("a refused receipt completion must not report a successful display commit");
    for node in [&w.a, &w.b] {
        assert_eq!(node.settled_row(&message, &w.agent).unwrap().0, "pending");
        assert_eq!(node.count_kind("receipt.settled"), 0);
    }
    assert_eq!(
        w.b.count("SELECT count(*) FROM fact_attribution_staging"),
        0,
        "a refused display leaves no staged attribution"
    );
    hold_off(&w.b);
    CompletionPath::HookContext.complete(&w, &message, "retry-display");
    ship_origin(&w.b, w.b.id, &w.a.store);
    for node in [&w.a, &w.b] {
        assert_eq!(node.settled_row(&message, &w.agent).unwrap().0, "acked");
        assert_eq!(node.count_kind("receipt.settled"), 1);
        assert_eq!(node.ack_events(&w.thread).len(), 1);
    }
    assert_eq!(w.a.count("SELECT count(*) FROM receipt_due_sparse"), 0);
}

// ---- send, ACK, sync -----------------------------------------------------------

#[test]
fn b_acks_a_message_both_acked_same_time_sparse_due_gone_event_on_a() {
    let w = rworld(170);
    let message = w.send();
    w.deliver(&[&message]);
    assert_eq!(
        w.a.count("SELECT count(*) FROM receipt_due_sparse"),
        1,
        "A watches the receipt's deadline"
    );

    w.b.ack(&w.agent, "op-ack", &[&message], false).unwrap();
    sync_all(&[&w.a, &w.b]);

    let (a_row, b_row) = (
        w.a.settled_row(&message, &w.agent).unwrap(),
        w.b.settled_row(&message, &w.agent).unwrap(),
    );
    assert_eq!(a_row, b_row, "the same state, acked_at and batch");
    assert_eq!(
        a_row,
        ("acked".into(), Some(SETTLED_AT), Some("op-ack".into()))
    );
    assert_eq!(
        w.a.count("SELECT count(*) FROM receipt_due_sparse"),
        0,
        "A's sparse-due row is gone"
    );
    let key = ack_key("ack", &w.agent, "op-ack", &w.thread);
    let (a_events, b_events) = (w.a.ack_events(&w.thread), w.b.ack_events(&w.thread));
    assert_eq!(a_events.len(), 1);
    assert_eq!(a_events[0].0, key, "A's timeline has the ack event");
    assert!(a_events[0].1.contains(message.as_str()));
    assert_eq!(a_events, b_events, "the same event on both nodes");
}

#[test]
fn batch_of_3_in_2_threads_two_ack_events_same_keys() {
    let w = rworld(172);
    let second = w.a.create_thread(&w.author, "second", None).unwrap();
    w.a.joined(&second, &w.author, 1).unwrap();
    sync_all(&[&w.a, &w.b]);
    w.b.joined(&second, &w.agent, 1).unwrap();
    let (m1, m2, m3) = (w.send(), w.send(), w.send_in(&second));
    w.deliver(&[&m1, &m2, &m3]);

    // Request order interleaves the threads.
    w.b.ack(&w.agent, "op-batch", &[&m1, &m3, &m2], false)
        .unwrap();
    sync_all(&[&w.a, &w.b]);

    for node in [&w.a, &w.b] {
        assert_eq!(
            node.count("SELECT count(*) FROM messages WHERE event_key LIKE 'ack%'"),
            2,
            "exactly two ack events"
        );
        for (thread, expect) in [
            (&w.thread, vec![m1.clone(), m2.clone()]),
            (&second, vec![m3.clone()]),
        ] {
            let events = node.ack_events(thread);
            assert_eq!(events.len(), 1, "one event per thread");
            assert_eq!(
                events[0].0,
                ack_key("ack", &w.agent, "op-batch", thread),
                "today's key"
            );
            let mut want: Vec<_> = expect.iter().map(|m| m.as_str().to_owned()).collect();
            want.sort();
            let json: serde_json::Value = serde_json::from_str(&events[0].1).unwrap();
            let got: Vec<String> = json["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(got, want, "the thread's receipts of the batch");
        }
    }
    assert_eq!(
        (w.a.ack_events(&w.thread), w.a.ack_events(&second)),
        (w.b.ack_events(&w.thread), w.b.ack_events(&second)),
        "the same keys and lists on both nodes"
    );
}

#[test]
fn local_ack_event_key_unchanged() {
    let w = rworld(174);
    let message = w.send();
    w.deliver(&[&message]);
    w.b.ack(&w.agent, "op-key", &[&message], false).unwrap();
    // The key a local ACK has always written: prefix, then each of seat,
    // operation and thread as `<len>:<value>`.
    let key = format!(
        "ack:{}:{}:6:op-key:{}:{}",
        w.agent.as_str().len(),
        w.agent.as_str(),
        w.thread.as_str().len(),
        w.thread.as_str()
    );
    let events = w.b.ack_events(&w.thread);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, key);
    let json: serde_json::Value = serde_json::from_str(&events[0].1).unwrap();
    assert_eq!(json["event"], "ack");
    assert_eq!(json["seat"], w.agent.as_str());
    assert_eq!(json["decided_at"], SETTLED_AT);
}

// ---- catch-up after a wipe -------------------------------------------------------

#[test]
fn catch_up_after_wipe_no_delivery_for_settled() {
    let w = rworld(176);
    let message = w.send();
    w.deliver(&[&message]);
    w.b.ack(&w.agent, "op-1", &[&message], false).unwrap();
    sync_all(&[&w.a, &w.b]);

    // B's store is wiped (same seed); it re-ingests its old log and A's.
    let b2 = wiped_b(&w, 177);
    fact_exchange::sync(&w.a.store, &b2.store);

    assert_eq!(
        b2.settled_row(&message, &w.agent),
        Some(("acked".into(), Some(SETTLED_AT), Some("op-1".into()))),
        "the settlement applied after the message"
    );
    assert_eq!(
        b2.suppressed_seats(&message).as_deref(),
        Some(w.agent.as_str())
    );
    assert_eq!(b2.lazy_rows(), 0);
    b2.drive_send(&message);
    assert!(b2.wake_seats().is_empty(), "settled history wakes nobody");
    assert_eq!(
        b2.ack_events(&w.thread),
        w.a.ack_events(&w.thread),
        "the same ack history"
    );
}

#[test]
fn wipe_then_pull_foreign_first_held_then_suppressed() {
    let w = rworld(178);
    let (m1, m2) = (w.send(), w.send());
    w.deliver(&[&m1, &m2]);
    w.b.ack(&w.agent, "op-1", &[&m1], false).unwrap();
    sync_all(&[&w.a, &w.b]);

    // B's store is wiped; A's log arrives first with the hold's `Links` flag
    // set (B's old log advertised beyond B's contiguous).
    let b2 = wiped_b(&w, 179);
    hold_on(&b2);
    ship_origin(&w.a, w.a.id, &b2.store);
    assert_eq!(b2.pending_kinds("message.sent"), 2, "A's messages wait");
    assert_eq!(
        b2.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        0
    );
    ship_origin(&w.b, w.b.id, &b2.store);
    assert_eq!(b2.pending_kinds("message.sent"), 2, "still held");
    assert_eq!(b2.count("SELECT count(*) FROM wake_work"), 0);

    // The old log is in; the flag clears.
    hold_off(&b2);
    drain(&b2.store);
    assert_eq!(b2.pending_kinds("message.sent"), 0);
    assert_eq!(
        b2.settled_row(&m1, &w.agent).map(|r| r.0),
        Some("acked".into())
    );
    assert_eq!(b2.suppressed_seats(&m1).as_deref(), Some(w.agent.as_str()));
    assert_eq!(
        b2.suppressed_seats(&m2),
        None,
        "the unsettled one is offered"
    );
    assert_eq!(b2.lazy_rows(), 0);

    b2.drive_send(&m1);
    assert!(
        b2.wake_seats().is_empty(),
        "no wake_work, so no wake dispatch, for the settled message"
    );
    b2.drive_send(&m2);
    assert_eq!(
        b2.wake_seats(),
        vec![w.agent.as_str().to_owned()],
        "the unsettled message in the same pull is offered"
    );
    assert_eq!(
        b2.settled_row(&m2, &w.agent).map(|r| r.0),
        Some("pending".into())
    );
}

#[test]
fn own_old_unsettled_offered_settled_not() {
    let w = rworld(180);
    // B sends from `agent` to `peer`, both its own seats: two ack messages
    // (the first gets settled) and a lazy one.
    let ordinary = || {
        let message =
            w.b.stage_prep(&w.thread, "ordinary", &[(&w.peer, FIVE_MIN, true)], &[]);
        w.b.author_sent(&sent(
            &message,
            &w.thread,
            &w.agent,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![spec(
                &w.peer,
                ReceiptObligation::Ack,
                Some(SENT_AT + FIVE_MIN),
            )],
        ))
        .unwrap();
        message
    };
    let (m1, m2) = (ordinary(), ordinary());
    let m3 = w.b.stage_prep(&w.thread, "lazy", &[], &[&w.peer]);
    w.b.author_sent(&sent(
        &m3,
        &w.thread,
        &w.agent,
        DeliveryMode::Lazy,
        0,
        vec![spec(&w.peer, ReceiptObligation::None, None)],
    ))
    .unwrap();
    w.b.author_settled(&settled(
        &m1,
        &w.peer,
        herdr_threads::sync::facts::payload::SettleHow::Acked,
        "op-s",
        true,
    ))
    .unwrap();
    assert_eq!(w.b.journal().last().unwrap().0, "receipt.settled");

    // B's store is restored to a state before the sends (here: wiped); the
    // old tail comes back with the hold's flag set, the settlement in a later
    // ingest batch than the messages.
    let b2 = wiped_b(&w, 181);
    hold_on(&b2);
    ship_origin(&w.a, w.a.id, &b2.store);
    for (log, mut entries) in origin_entries(&w.b, w.b.id) {
        let last = entries.split_off(entries.len() - 1);
        ingest(&b2.store, &log, entries);
        drain(&b2.store);
        assert_eq!(
            b2.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
            0,
            "nothing of the tail applies while the flag is set"
        );
        assert_eq!(b2.count("SELECT count(*) FROM wake_work"), 0);
        ingest(&b2.store, &log, last);
        drain(&b2.store);
    }
    assert_eq!(
        b2.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        0,
        "still held after the settlement arrived"
    );

    hold_off(&b2);
    drain(&b2.store);
    assert_eq!(
        b2.settled_row(&m1, &w.peer).map(|r| r.0),
        Some("acked".into())
    );
    assert_eq!(b2.suppressed_seats(&m1).as_deref(), Some(w.peer.as_str()));
    assert_eq!(b2.suppressed_seats(&m2), None);
    assert_eq!(
        b2.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE message_id='{}'",
            m3.as_str()
        )),
        1,
        "the unsettled lazy message writes its lazy row"
    );
    assert_eq!(b2.lazy_rows(), 1, "and nothing else does");
    b2.drive_send(&m1);
    assert!(
        b2.wake_seats().is_empty(),
        "the settled message wakes nobody"
    );
    b2.drive_send(&m2);
    assert_eq!(
        b2.wake_seats(),
        vec![w.peer.as_str().to_owned()],
        "the unsettled message wakes its recipient"
    );
}

// ---- the restore-window authoring hold -------------------------------------------

#[test]
fn display_during_hold_leaves_pending_ack_unavailable() {
    let w = rworld(182);
    let message = w.send();
    w.deliver(&[&message]);
    hold_on(&w.b);

    // A display entry point skips the settlement and does not fail.
    let CommandResult::Acknowledged(result) =
        w.b.ack(&w.agent, "op-display", &[&message], true).unwrap()
    else {
        panic!("an ACK result");
    };
    assert!(result.acknowledged.is_empty(), "nothing was settled");
    assert_eq!(
        w.b.settled_row(&message, &w.agent).map(|r| r.0),
        Some("pending".into())
    );
    assert_eq!(w.b.count_kind("receipt.settled"), 0, "authored nothing");
    assert_eq!(
        w.b.count("SELECT count(*) FROM fact_attribution_staging"),
        0,
        "no staged attribution left behind"
    );

    // An explicit ACK fails, retryable, and writes nothing.
    let error = w.b.ack(&w.agent, "op-ack", &[&message], false).unwrap_err();
    assert_eq!(error.code, ErrorCode::ServiceBusy);
    assert_eq!(error.detail, "own history catching up");
    assert_eq!(w.b.count_kind("receipt.settled"), 0);
    assert_eq!(
        w.b.settled_row(&message, &w.agent).map(|r| r.0),
        Some("pending".into())
    );
    assert!(w.b.ack_events(&w.thread).is_empty());

    // The hold clears and the same ACK succeeds.
    hold_off(&w.b);
    let CommandResult::Acknowledged(result) =
        w.b.ack(&w.agent, "op-ack-2", &[&message], false).unwrap()
    else {
        panic!("an ACK result");
    };
    assert_eq!(result.acknowledged, vec![message.clone()]);
    assert_eq!(w.b.count_kind("receipt.settled"), 1);
    assert_eq!(
        w.b.settled_row(&message, &w.agent).map(|r| r.0),
        Some("acked".into())
    );
}

#[test]
fn after_hold_no_second_settlement_when_old_log_settled() {
    let w = rworld(184);
    let message = w.send();
    w.deliver(&[&message]);

    // B's other incarnation (its old log) settled the receipt; this store is
    // the restored one that has not pulled that tail yet.
    let old = Node::open(185, 7, registry(), true);
    assert_eq!(old.id, w.b.id);
    old.add_local_seat(&w.agent);
    old.publish(&w.agent).unwrap();
    fact_exchange::sync(&w.a.store, &old.store);
    old.drive_send(&message);
    old.author_settled(&settled(
        &message,
        &w.agent,
        herdr_threads::sync::facts::payload::SettleHow::Acked,
        "op-old",
        true,
    ))
    .unwrap();

    hold_on(&w.b);
    w.b.ack(&w.agent, "op-display", &[&message], true).unwrap();
    assert_eq!(w.b.count_kind("receipt.settled"), 0, "nothing authored");
    ship_origin(&old, old.id, &w.b.store);
    assert_eq!(w.b.pending_kinds("receipt.settled"), 1, "held, not applied");
    assert_eq!(
        w.b.settled_row(&message, &w.agent).map(|r| r.0),
        Some("pending".into())
    );

    hold_off(&w.b);
    drain(&w.b.store);
    assert_eq!(
        w.b.settled_row(&message, &w.agent),
        Some(("acked".into(), Some(SETTLED_AT), Some("op-old".into()))),
        "the pulled-back tail settled it"
    );

    // The display now finds the receipt settled and authors nothing.
    let CommandResult::Acknowledged(result) =
        w.b.ack(&w.agent, "op-display-2", &[&message], true)
            .unwrap()
    else {
        panic!("an ACK result");
    };
    assert!(result.acknowledged.is_empty());
    assert_eq!(result.already_acknowledged, vec![message.clone()]);
    assert_eq!(
        w.b.count_kind("receipt.settled"),
        1,
        "no second receipt.settled: only the old log's"
    );
    assert_eq!(w.b.ack_events(&w.thread).len(), 1);
}

// ---- order permutation ------------------------------------------------------------

#[test]
fn order_permutation_settlement_before_retirement_strict() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(190, reg),
        Node::with(191, reg),
        Node::with(192, reg),
    );
    let (thread, author) = thread_of(&a, "shared");
    let (agent_b, agent_c): (SeatId, SeatId) = (b.local_seat(), c.local_seat());
    b.publish(&agent_b).unwrap();
    c.publish(&agent_c).unwrap();
    sync_all(&[&a, &b, &c]);

    let message: MessageId = a.stage_prep(
        &thread,
        "ordinary",
        &[(&agent_b, FIVE_MIN, true), (&agent_c, FIVE_MIN, true)],
        &[],
    );
    a.author_sent(&sent(
        &message,
        &thread,
        &author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![
            spec(&agent_b, ReceiptObligation::Ack, Some(SENT_AT + FIVE_MIN)),
            spec(&agent_c, ReceiptObligation::Ack, Some(SENT_AT + FIVE_MIN)),
        ],
    ))
    .unwrap();
    sync_all(&[&a, &b, &c]);

    // B settles, then retires its seat (same log); C settles concurrently.
    b.author_settled(&settled(
        &message,
        &agent_b,
        herdr_threads::sync::facts::payload::SettleHow::Acked,
        "op-b",
        true,
    ))
    .unwrap();
    b.run(|tx, d| {
        FactRoute::Journal
            .emit::<SeatRetired>(
                tx,
                b.ctx(),
                d,
                &retired_payload(&agent_b),
                None,
                |_, _, _| Ok(()),
            )
            .map(|_| ())
    })
    .unwrap();
    c.author_settled(&settled(
        &message,
        &agent_c,
        herdr_threads::sync::facts::payload::SettleHow::Acked,
        "op-c",
        true,
    ))
    .unwrap();

    let sources = [(&a, a.id), (&b, b.id), (&c, c.id)];
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let specs = replicated_tables();
    let mut first = None;
    for (n, order) in orders.iter().enumerate() {
        let fresh = OracleStore::open([200 + u8::try_from(n).unwrap(); 32], registry());
        for &i in order {
            let (node, id) = sources[i];
            ship_origin(node, id, &fresh);
        }
        let failed: Vec<_> = fresh
            .context()
            .open_query(budget())
            .unwrap()
            .prepare("SELECT kind,apply_state FROM journal WHERE apply_state NOT IN ('applied')")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(failed.is_empty(), "order {order:?}: {failed:?}");
        let state = |sql: String| -> String {
            fresh
                .context()
                .open_query(budget())
                .unwrap()
                .query_row(&sql, [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            state(format!(
                "SELECT state FROM receipt_state WHERE message_id='{}' AND seat_id='{}'",
                message.as_str(),
                agent_b.as_str()
            )),
            "acked",
            "order {order:?}: the settlement beat the retirement"
        );
        assert_eq!(
            state(format!(
                "SELECT state FROM seats WHERE id='{}'",
                agent_b.as_str()
            )),
            "retired"
        );
        assert_eq!(
            state(format!(
                "SELECT state FROM receipt_state WHERE message_id='{}' AND seat_id='{}'",
                message.as_str(),
                agent_c.as_str()
            )),
            "acked"
        );
        let projection = replicated_projection(&fresh, fresh.node(), &specs);
        match &first {
            None => first = Some(projection),
            Some(first) => assert_eq!(
                &projection, first,
                "order {order:?} projects other rows than order {:?}",
                orders[0]
            ),
        }
    }
}
