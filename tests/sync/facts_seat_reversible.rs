//! `seat.retired_reversible` and `seat.unretired` applied (ht-ma6i.7, spec 4.3): on a peer, in own-log
//! replay, out of episode, for facts that arrive during the window and on a node that parked the
//! kinds. The clock of `Node` stands still, so the facts are authored with chosen times. Authoring
//! from the real transitions is in `tests/store/reversible_facts.rs`.

use herdr_threads::notification::policy::soft_point;
use herdr_threads::protocol::commands::{DeliveryMode, OperatorRetire};
use herdr_threads::protocol::ids::{InvitationId, MessageId, OperationId, SeatId, ThreadId};
use herdr_threads::protocol::results::{CommandResult, ErrorCode};
use herdr_threads::protocol::summary::SummarySettings;
use herdr_threads::store::effective::{self, EffectiveReceiptState};
use herdr_threads::store::materialize::HELD_KINDS;
use herdr_threads::store::receipts::{self, ReceiptDueCursor};
use herdr_threads::store::{control, invitation_due, journal, poke, seats, service_substrate};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::payload::ReceiptObligation;
use herdr_threads::sync::facts::seat::{
    RetireReason, SeatCreated, SeatRetired, SeatRetiredPayload, SeatRetiredReversible,
    SeatRetiredReversiblePayload, SeatUnretired, SeatUnretiredPayload,
};
use herdr_threads::sync::facts::{
    FactKind, FactRegistry, NodeGenesis, NodeRetag, invitation, membership, message, receipt,
    thread,
};
use herdr_threads::test_support::{fact_exchange, operator_actor};
use rusqlite::params;

use crate::fact_convergence_seat::closure_proof;
use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_seat::{Node, budget, registry, retirement_rows};
use crate::facts_thread::sync_all;

/// Past `Node`'s fixed clock and `SENT_AT`, so every obligation a test authors at those times exists by
/// the episode's `unretired_at`.
pub const T1: i64 = 1_700_100_000_000;
const T2: i64 = 1_700_500_000_000;
/// How long a test seat stays frozen.
pub const WINDOW: i64 = 60_000;

pub fn rev(node: &Node, seat: &SeatId, retired_at: i64) {
    node.run(|tx, d| {
        journal::author::<SeatRetiredReversible>(
            tx,
            node.ctx(),
            d,
            &SeatRetiredReversiblePayload {
                seat: seat.clone(),
                reason: RetireReason::Closed,
                retired_at,
            },
        )
        .map(|_| ())
    })
    .unwrap();
}

pub fn unret(node: &Node, seat: &SeatId, retired_at: i64, unretired_at: i64) {
    node.run(|tx, d| {
        journal::author::<SeatUnretired>(
            tx,
            node.ctx(),
            d,
            &SeatUnretiredPayload {
                seat: seat.clone(),
                retired_at,
                unretired_at,
            },
        )
        .map(|_| ())
    })
    .unwrap();
}

fn fin(node: &Node, seat: &SeatId, retired_at: i64) {
    node.run(|tx, d| {
        journal::author::<SeatRetired>(
            tx,
            node.ctx(),
            d,
            &SeatRetiredPayload {
                seat: seat.clone(),
                reason: RetireReason::Closed,
                retired_at,
            },
        )
        .map(|_| ())
    })
    .unwrap();
}

/// A published seat of `home`, with an open binding, known to `others`.
fn published(home: &Node, others: &[&Node]) -> SeatId {
    let seat = home.local_seat();
    home.enroll(&seat, "codex", "session-1").unwrap();
    for other in others {
        fact_exchange::sync(&home.store, &other.store);
    }
    seat
}

/// `(state, retired_at, retired_seq)` of the seat.
fn seat_row(node: &Node, seat: &SeatId) -> (String, Option<i64>, Option<i64>) {
    node.query(
        &format!(
            "SELECT state,retired_at,retired_seq FROM seats WHERE id='{}'",
            seat.as_str()
        ),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .remove(0)
}

/// `(state, cutover_at, reason, closure_boot)` of the reversible row.
fn rev_row(node: &Node, seat: &SeatId) -> Option<(String, i64, String, Option<String>)> {
    node.query(
        &format!(
            "SELECT state,cutover_at,reason,closure_boot FROM reversible_retirements WHERE seat_id='{}'",
            seat.as_str()
        ),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .into_iter()
    .next()
}

fn applied_seq(node: &Node, kind: &str) -> i64 {
    node.query(
        &format!("SELECT max(applied_seq) FROM journal WHERE kind='{kind}'"),
        |r| r.get(0),
    )[0]
}

// ---- kinds ---------------------------------------------------------------------

/// Kills: a kind unregistered, not held, with another name or major, or not needing the seat.
#[test]
fn kinds_are_registered_held_and_need_the_seat() {
    let seat = SeatId::new("sAbc12345678");
    let reversible = SeatRetiredReversiblePayload {
        seat: seat.clone(),
        reason: RetireReason::Closed,
        retired_at: 1,
    };
    let unretired = SeatUnretiredPayload {
        seat,
        retired_at: 1,
        unretired_at: 2,
    };
    assert_eq!(
        (SeatRetiredReversible::NAME, SeatRetiredReversible::MAJOR),
        ("seat.retired_reversible", 1)
    );
    assert_eq!(
        (SeatUnretired::NAME, SeatUnretired::MAJOR),
        ("seat.unretired", 1)
    );
    let key = |keys: Vec<EntityKey>| -> Vec<String> {
        keys.iter().map(|k| k.as_str().to_owned()).collect()
    };
    assert_eq!(SeatRetiredReversible::creates(&reversible), None);
    assert_eq!(SeatUnretired::creates(&unretired), None);
    assert_eq!(
        key(SeatRetiredReversible::needs(&reversible)),
        ["sAbc12345678"]
    );
    assert_eq!(key(SeatUnretired::needs(&unretired)), ["sAbc12345678"]);
    for kind in ["seat.retired_reversible", "seat.unretired"] {
        assert!(FactRegistry::builtins().lookup(kind, 1).is_some(), "{kind}");
        assert!(HELD_KINDS.contains(&kind), "{kind}");
    }
}

/// Kills: a non-home origin retiring or restoring a seat.
#[test]
fn non_home_origin_is_failed() {
    let reg = registry();
    let (a, b, c) = (Node::with(1, reg), Node::with(2, reg), Node::with(3, reg));
    let seat = published(&b, &[&a, &c]);
    rev(&c, &seat, T1);
    unret(&c, &seat, T1, T1 + 1);
    fact_exchange::sync(&c.store, &a.store);
    for (kind, label) in [
        ("seat.retired_reversible", "seat.retired_reversible"),
        ("seat.unretired", "seat.unretired"),
    ] {
        let failures: Vec<_> = a
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == kind && state == "failed")
            .collect();
        assert_eq!(failures.len(), 1, "{:?}", a.journal());
        let error = failures[0].2.as_deref().unwrap();
        assert!(
            error.contains(&format!("is not the authorized author of {label} for"))
                && error.contains(seat.as_str()),
            "{error}"
        );
    }
    assert_eq!(seat_row(&a, &seat).0, "remote");
    assert_eq!(rev_row(&a, &seat), None);
}

// ---- materialization -----------------------------------------------------------

/// Kills: a peer's fact writing a job, keeping the seat live, or a row with invented inputs.
#[test]
fn peer_reversible_reads_retired_with_no_job() {
    let reg = registry();
    let (a, b) = (Node::with(4, reg), Node::with(5, reg));
    let seat = published(&b, &[&a]);
    rev(&b, &seat, T1);
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(
        seat_row(&a, &seat),
        (
            "retired".to_owned(),
            Some(T1),
            Some(applied_seq(&a, "seat.retired_reversible"))
        )
    );
    assert_eq!(
        rev_row(&a, &seat),
        Some(("reversible".to_owned(), T1, "closed".to_owned(), None))
    );
    assert_eq!(retirement_rows(&a), vec![]);
}

/// A peer's copy of what the seat owes: a timed invitation and a timed receipt, authored by `a` for
/// a seat of `b`. Returns them with the receipt deadline.
fn obligations_on_peer(a: &Node, b: &Node, seat: &SeatId) -> (InvitationId, MessageId, i64) {
    let (thread, author) = thread_of(a, "owed");
    a.joined(&thread, &author, 1).unwrap();
    sync_all(&[a, b]);
    b.joined(&thread, seat, 1).unwrap();
    sync_all(&[a, b]);
    let invitation = a.invite_fact(&thread, &author, seat);
    let message = a.stage_prep(&thread, "ordinary", &[(seat, FIVE_MIN, true)], &[]);
    let deadline = SENT_AT + FIVE_MIN;
    a.author_sent(&sent(
        &message,
        &thread,
        &author,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(seat, ReceiptObligation::Ack, Some(deadline))],
    ))
    .unwrap();
    a.drive_send(&message);
    (invitation, message, deadline)
}

fn invitation_row(node: &Node, id: &InvitationId) -> (String, Option<i64>, Option<i64>) {
    node.query(
        &format!(
            "SELECT state,deadline_at,retired_at FROM invitations WHERE id='{}'",
            id.as_str()
        ),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .remove(0)
}

/// `(state, deadline_at, retired_at)` of the physical receipt row.
fn receipt_row(
    node: &Node,
    message: &MessageId,
    seat: &SeatId,
) -> (String, Option<i64>, Option<i64>) {
    node.query(
        &format!(
            "SELECT state,deadline_at,retired_at FROM receipt_state WHERE message_id='{}' AND seat_id='{}'",
            message.as_str(),
            seat.as_str()
        ),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .remove(0)
}

fn effective_state(node: &Node, message: &MessageId, seat: &SeatId) -> EffectiveReceiptState {
    effective::effective_receipt(&node.raw(), message.as_str(), seat.as_str())
        .unwrap()
        .unwrap()
        .state
}

/// Kills: a peer keeping the seat retired, leaving the row, or not shifting its own deadlines.
#[test]
fn peer_unretired_restores_remote_shifts_deadlines_deletes_row() {
    let reg = registry();
    let (a, b) = (Node::with(6, reg), Node::with(7, reg));
    let seat = published(&b, &[&a]);
    let (invitation, message, deadline) = obligations_on_peer(&a, &b, &seat);
    let invitation_deadline = invitation_row(&a, &invitation).1.unwrap();
    rev(&b, &seat, T1);
    sync_all(&[&a, &b]);
    assert_eq!(seat_row(&a, &seat).0, "retired");
    unret(&b, &seat, T1, T1 + WINDOW);
    sync_all(&[&a, &b]);
    assert_eq!(seat_row(&a, &seat), ("remote".to_owned(), None, None));
    assert_eq!(rev_row(&a, &seat), None);
    assert_eq!(
        invitation_row(&a, &invitation).1,
        Some(invitation_deadline + WINDOW)
    );
    assert_eq!(receipt_row(&a, &message, &seat).1, Some(deadline + WINDOW));
    assert_eq!(
        effective_state(&a, &message, &seat),
        EffectiveReceiptState::Pending
    );
    assert_eq!(retirement_rows(&a), vec![]);
}

/// Kills: a final fact on a reversible row writing no job, keeping the row or the fence's sequence.
#[test]
fn final_on_reversible_row_inserts_the_job() {
    let reg = registry();
    let (a, b) = (Node::with(8, reg), Node::with(9, reg));
    let seat = published(&b, &[&a]);
    rev(&b, &seat, T1);
    fact_exchange::sync(&a.store, &b.store);
    let fenced = seat_row(&a, &seat).2.unwrap();
    fin(&b, &seat, T1);
    fact_exchange::sync(&a.store, &b.store);
    let row = seat_row(&a, &seat);
    assert_eq!((row.0.as_str(), row.1), ("retired", Some(T1)));
    assert_eq!(row.2, Some(applied_seq(&a, "seat.retired")));
    assert!(row.2.unwrap() > fenced, "retired_seq advanced");
    assert_eq!(rev_row(&a, &seat), None);
    assert_eq!(
        retirement_rows(&a),
        vec![(seat.as_str().to_owned(), "replicated".to_owned(), T1, None)]
    );
    // The home node's replayed row, with no inputs, ends the same way.
    assert_eq!(rev_row(&b, &seat), None);
    assert_eq!(
        retirement_rows(&b),
        vec![(seat.as_str().to_owned(), "replicated".to_owned(), T1, None)]
    );
}

/// Kills: an episode check that is missing, on either fact, on either kind of row.
#[test]
fn out_of_episode_facts_are_noops() {
    let reg = registry();
    let (a, b) = (Node::with(10, reg), Node::with(11, reg));
    let seat = published(&b, &[&a]);
    rev(&b, &seat, T1);
    unret(&b, &seat, T2, T2 + WINDOW);
    fin(&b, &seat, T2);
    rev(&b, &seat, T2);
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        assert_eq!(seat_row(node, &seat).1, Some(T1), "still the first episode");
        assert_eq!(seat_row(node, &seat).0, "retired");
        let row = rev_row(node, &seat).unwrap();
        assert_eq!((row.0.as_str(), row.1), ("reversible", T1));
        assert_eq!(retirement_rows(node), vec![], "no job");
    }
    assert!(a.journal().iter().all(|(_, state, _)| state == "applied"));
}

/// Kills: a late reversible fact reviving a final retirement into an undoable one.
#[test]
fn reversible_on_finally_retired_row_is_noop() {
    let reg = registry();
    let (a, b) = (Node::with(12, reg), Node::with(13, reg));
    let seat = published(&b, &[&a]);
    fin(&b, &seat, T1);
    fact_exchange::sync(&a.store, &b.store);
    let jobs = retirement_rows(&a);
    assert_eq!(jobs.len(), 1);
    rev(&b, &seat, T1);
    unret(&b, &seat, T1, T1 + WINDOW);
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        assert_eq!(seat_row(node, &seat).0, "retired");
        assert_eq!(seat_row(node, &seat).1, Some(T1));
        assert_eq!(rev_row(node, &seat), None);
        assert_eq!(retirement_rows(node), jobs);
    }
}

/// Kills: a replayed local row skipping what the fence does (open bindings, generation), keeping a
/// stale `unretired` row, or un-retiring to `remote`.
#[test]
fn own_log_replay_takes_the_fences_transition() {
    let reg = registry();
    let b = Node::with(14, reg);
    let seat = published(&b, &[]);
    let generation = |node: &Node| -> i64 {
        node.query(
            &format!("SELECT generation FROM seats WHERE id='{}'", seat.as_str()),
            |r| r.get(0),
        )[0]
    };
    let open_bindings = |node: &Node| -> i64 {
        node.query(
            &format!(
                "SELECT count(*) FROM occupant_bindings WHERE seat_id='{}' AND ended_at IS NULL",
                seat.as_str()
            ),
            |r| r.get(0),
        )[0]
    };
    assert_eq!(open_bindings(&b), 1);
    let before = generation(&b);
    rev(&b, &seat, T1);
    assert_eq!(seat_row(&b, &seat).0, "retired");
    assert_eq!(open_bindings(&b), 0, "bindings end as at the fence");
    assert_eq!(generation(&b), before + 1);
    assert_eq!(
        rev_row(&b, &seat),
        Some(("reversible".to_owned(), T1, "closed".to_owned(), None))
    );
    assert_eq!(retirement_rows(&b), vec![], "no job");

    unret(&b, &seat, T1, T1 + WINDOW);
    assert_eq!(seat_row(&b, &seat), ("unresolved".to_owned(), None, None));
    assert_eq!(generation(&b), before + 2);
    assert_eq!(rev_row(&b, &seat).unwrap().0, "unretired");

    // A later episode replaces the waiting row, and its finalization yields the job.
    rev(&b, &seat, T2);
    let row = rev_row(&b, &seat).unwrap();
    assert_eq!((row.0.as_str(), row.1), ("reversible", T2));
    unret(&b, &seat, T2, T2 + WINDOW);
    fin(&b, &seat, T2 + 2 * WINDOW);
    assert_eq!(seat_row(&b, &seat).0, "retired");
    assert_eq!(rev_row(&b, &seat), None, "the waiting row goes");
    assert_eq!(retirement_rows(&b).len(), 1);
}

// ---- facts applied during the window --------------------------------------------

/// A home node with a joined seat and a seat with no membership, a peer that follows it, and a
/// sender that does not know of the freeze (an older node, or one the fact has not reached yet).
struct Window {
    home: Node,
    peer: Node,
    sender: Node,
    thread: ThreadId,
    author: SeatId,
    seat: SeatId,
    guest: SeatId,
}

fn window(seed: u8) -> Window {
    let reg = registry();
    let home = Node::open_commands(seed, u64::from(seed), reg);
    let peer = Node::open_commands(seed + 1, u64::from(seed + 1), reg);
    let sender = Node::open_commands(seed + 2, u64::from(seed + 2), reg);
    let (thread, author) = thread_of(&sender, "shared");
    sender.joined(&thread, &author, 1).unwrap();
    let (seat, guest) = (home.local_seat(), home.local_seat());
    home.publish(&seat).unwrap();
    home.publish(&guest).unwrap();
    sync_all(&[&home, &peer, &sender]);
    home.joined(&thread, &seat, 1).unwrap();
    sync_all(&[&home, &peer, &sender]);
    Window {
        home,
        peer,
        sender,
        thread,
        author,
        seat,
        guest,
    }
}

impl Window {
    fn nodes(&self) -> [&Node; 3] {
        [&self.home, &self.peer, &self.sender]
    }
    /// The seat freezes on the home node and the peer; the sender has not heard.
    fn freeze(&self) {
        rev(&self.home, &self.seat, T1);
        fact_exchange::sync(&self.home.store, &self.peer.store);
    }
    /// The sender's ACK request to the seat, applied and projected everywhere.
    fn message(&self) -> MessageId {
        let message = self.sender.stage_prep(
            &self.thread,
            "ordinary",
            &[(&self.seat, FIVE_MIN, true)],
            &[],
        );
        self.sender
            .author_sent(&sent(
                &message,
                &self.thread,
                &self.author,
                DeliveryMode::Ordinary,
                FIVE_MIN,
                vec![spec(
                    &self.seat,
                    ReceiptObligation::Ack,
                    Some(SENT_AT + FIVE_MIN),
                )],
            ))
            .unwrap();
        self.spread();
        for node in self.nodes() {
            node.drive_send(&message);
        }
        message
    }
    fn spread(&self) {
        sync_all(&self.nodes());
    }
    fn settle(&self) {
        self.spread();
        for node in self.nodes() {
            node.finish_retirements();
        }
    }
    fn assert_frozen_pending(&self, node: &Node, message: &MessageId) {
        assert_eq!(
            receipt_row(node, message, &self.seat),
            ("pending".to_owned(), Some(SENT_AT + FIVE_MIN), None),
            "no terminal row"
        );
        assert_eq!(
            effective_state(node, message, &self.seat),
            EffectiveReceiptState::RecipientRetired,
            "it still reads retired"
        );
    }
    fn assert_resumed(&self, node: &Node, message: &MessageId) {
        assert_eq!(
            receipt_row(node, message, &self.seat),
            (
                "pending".to_owned(),
                Some(SENT_AT + FIVE_MIN + WINDOW),
                None
            )
        );
        assert_eq!(
            effective_state(node, message, &self.seat),
            EffectiveReceiptState::Pending
        );
    }
    fn assert_settled(&self, node: &Node, message: &MessageId) {
        let row = receipt_row(node, message, &self.seat);
        assert_eq!(
            (row.0.as_str(), row.2),
            ("recipient_retired", Some(T1)),
            "the job settled it"
        );
    }
}

/// Kills: `project_receipt` writing terminal `recipient_retired` for a frozen recipient (nothing
/// could resume it), or a finalization whose fence misses it.
#[test]
fn window_message_resumes_on_unretire_and_settles_on_finalize() {
    let w = window(20);
    w.freeze();
    let message = w.message();
    w.assert_frozen_pending(&w.home, &message);
    w.assert_frozen_pending(&w.peer, &message);
    unret(&w.home, &w.seat, T1, T1 + WINDOW);
    w.spread();
    w.assert_resumed(&w.home, &message);
    w.assert_resumed(&w.peer, &message);

    let w = window(23);
    w.freeze();
    let message = w.message();
    fin(&w.home, &w.seat, T1);
    w.settle();
    w.assert_settled(&w.home, &message);
    w.assert_settled(&w.peer, &message);
}

/// The real fence and the real finalization (a key holder's close, then the operator's retire)
/// around a window message, on the home node and on a peer. Kills: the local job's fence missing a
/// fact applied in the window.
#[test]
fn window_message_settles_through_a_real_close_and_operator_retire() {
    let w = window(26);
    let mut conn = w.home.raw();
    let instance: String = conn
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    let tx = conn.transaction().unwrap();
    let author = service_substrate::ensure_reserved_author(
        &tx,
        &instance,
        herdr_threads::protocol::time::UtcMillis(0),
    )
    .unwrap();
    tx.execute(
        "INSERT INTO continuity_keys(key,owner_author_id,seat_id,reserved_at,bound_at,bound_by) VALUES ('k',?1,?2,1,1,'service_reservation')",
        params![author.as_str(), w.seat.as_str()],
    )
    .unwrap();
    tx.commit().unwrap();
    let proof = closure_proof(&w.home, &w.seat);
    control::begin_retirement(w.home.ctx(), &mut conn, w.seat.clone(), proof).unwrap();
    fact_exchange::sync(&w.home.store, &w.peer.store);
    let retired_at = seat_row(&w.home, &w.seat).1.unwrap();
    assert_eq!(rev_row(&w.peer, &w.seat).unwrap().1, retired_at);
    let message = w.message();
    for node in [&w.home, &w.peer] {
        assert_eq!(receipt_row(node, &message, &w.seat).0, "pending");
    }
    seats::mutate_operator(
        w.home.ctx(),
        &mut conn,
        &instance,
        herdr_threads::ports::OperatorRequest::Retire(OperatorRetire {
            seat: w.seat.clone(),
            operation: OperationId::new("finalize"),
        }),
        operator_actor(),
        None,
    )
    .unwrap();
    w.settle();
    for node in [&w.home, &w.peer] {
        let row = receipt_row(node, &message, &w.seat);
        assert_eq!(
            (row.0.as_str(), row.2),
            ("recipient_retired", Some(retired_at))
        );
    }
}

/// Kills: the invitee apply writing a terminal invitation or a terminal `memberships` row for a
/// frozen seat, so the seat would come back retired from the thread.
#[test]
fn window_invitation_writes_no_terminal_membership() {
    for (seed, finalize) in [(29, false), (32, true)] {
        let w = window(seed);
        rev(&w.home, &w.guest, T1);
        fact_exchange::sync(&w.home.store, &w.peer.store);
        let invitation = w.sender.invite_fact(&w.thread, &w.author, &w.guest);
        w.spread();
        let deadline = invitation_row(&w.home, &invitation).1.unwrap();
        for node in [&w.home, &w.peer] {
            assert_eq!(
                invitation_row(node, &invitation),
                ("pending".to_owned(), Some(deadline), None)
            );
            assert_eq!(node.membership(&w.thread, &w.guest).unwrap().0, "invited");
        }
        if finalize {
            fin(&w.home, &w.guest, T1);
            w.settle();
            for node in [&w.home, &w.peer] {
                let row = invitation_row(node, &invitation);
                assert_eq!((row.0.as_str(), row.2), ("recipient_retired", Some(T1)));
                assert_eq!(node.membership(&w.thread, &w.guest).unwrap().0, "retired");
            }
        } else {
            unret(&w.home, &w.guest, T1, T1 + WINDOW);
            w.spread();
            for node in [&w.home, &w.peer] {
                assert_eq!(
                    invitation_row(node, &invitation),
                    ("pending".to_owned(), Some(deadline + WINDOW), None)
                );
                assert_eq!(node.membership(&w.thread, &w.guest).unwrap().0, "invited");
            }
        }
    }
}

/// A message that reaches the home node during the window from a node that does not know is an
/// ordinary fact: it behaves as the receipt the seat already owed does. Kills: the two diverging.
#[test]
fn old_node_message_stays_frozen_with_the_rest() {
    for (seed, finalize) in [(35, false), (38, true)] {
        let w = window(seed);
        let before = w.message();
        w.freeze();
        let during = w.message();
        for message in [&before, &during] {
            w.assert_frozen_pending(&w.home, message);
        }
        if finalize {
            fin(&w.home, &w.seat, T1);
            w.settle();
            for message in [&before, &during] {
                w.assert_settled(&w.home, message);
            }
        } else {
            unret(&w.home, &w.seat, T1, T1 + WINDOW);
            w.spread();
            for message in [&before, &during] {
                w.assert_resumed(&w.home, message);
            }
        }
    }
}

/// A window message and a window invitation from a node that had not seen the retirement reach
/// the peer during the window and the home node only after the un-retire. Returns the window, the
/// message, the invitation and the invitation's deadline as written during the window.
fn late_window_arrival(seed: u8) -> (Window, MessageId, InvitationId, i64) {
    let w = window(seed);
    w.freeze();
    rev(&w.home, &w.guest, T1);
    fact_exchange::sync(&w.home.store, &w.peer.store);
    let message = w
        .sender
        .stage_prep(&w.thread, "ordinary", &[(&w.seat, FIVE_MIN, true)], &[]);
    w.sender
        .author_sent(&sent(
            &message,
            &w.thread,
            &w.author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![spec(
                &w.seat,
                ReceiptObligation::Ack,
                Some(SENT_AT + FIVE_MIN),
            )],
        ))
        .unwrap();
    let invitation = w.sender.invite_fact(&w.thread, &w.author, &w.guest);
    // Only the peer hears of them while the seat is frozen.
    fact_exchange::sync(&w.sender.store, &w.peer.store);
    w.peer.drive_send(&message);
    w.assert_frozen_pending(&w.peer, &message);
    let invitation_deadline = invitation_row(&w.peer, &invitation).1.unwrap();
    assert!(
        w.home
            .query(
                &format!("SELECT 1 FROM messages WHERE id='{}'", message.as_str()),
                |r| r.get::<_, i64>(0)
            )
            .is_empty(),
        "the home node has not seen it yet"
    );
    unret(&w.home, &w.seat, T1, T1 + WINDOW);
    unret(&w.home, &w.guest, T1, T1 + WINDOW);
    w.spread();
    // The peer ran its send job during the window; the others run theirs now.
    for node in [&w.home, &w.sender] {
        node.drive_send(&message);
    }
    (w, message, invitation, invitation_deadline)
}

/// Kills: the late copy on the home node keeping its unshifted deadline while the copy present at
/// the un-retire moved, or a late copy shifting twice.
#[test]
fn a_window_message_arriving_after_unretire_gets_the_owners_deadline() {
    let (w, message, invitation, invitation_deadline) = late_window_arrival(41);
    // Created at SENT_AT, before the retirement: frozen for the whole window on every node.
    for node in w.nodes() {
        w.assert_resumed(node, &message);
        assert_eq!(
            receipt_deadline(node, &message, &w.seat),
            Some(SENT_AT + FIVE_MIN + WINDOW)
        );
        assert_eq!(
            invitation_row(node, &invitation),
            (
                "pending".to_owned(),
                Some(invitation_deadline + WINDOW),
                None
            )
        );
    }
}

/// Kills: the home node soft-poking the late copy on its unshifted deadline, before the soft
/// point of the deadline every other node's copy carries.
#[test]
fn a_window_message_arriving_after_unretire_is_not_soft_poked_early() {
    let (w, message, _, _) = late_window_arrival(44);
    let owner_deadline = receipt_deadline(&w.peer, &message, &w.seat).unwrap();
    let home = w.home.raw();
    // Reattached, so the seat can be poked at all.
    home.execute(
        "UPDATE seats SET state='resolved',unresolved_reason=NULL,target_id=?1 WHERE id=?1",
        [w.seat.as_str()],
    )
    .unwrap();
    let available_at: i64 = home
        .query_row(
            "SELECT available_at FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            params![message.as_str(), w.seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let settings = SummarySettings {
        soft_fraction: 0.5,
        ..Default::default()
    };
    let soft = soft_point(owner_deadline, owner_deadline - available_at, 0.5);
    let poked = |now: i64| {
        poke::due_pokes_for_seat(&home, &w.seat, now, &settings)
            .unwrap()
            .is_some_and(|due| due.receipts.iter().any(|r| r.message == message))
    };
    assert!(
        !poked(soft - 1),
        "no soft poke before the soft point of the owner's shifted deadline"
    );
    assert!(poked(soft));
}

/// A peer's own thread and author, with `seat` joined and a seat with no membership.
fn peer_view(seed: u8) -> (Node, Node, ThreadId, SeatId, SeatId, SeatId) {
    let reg = registry();
    let peer = Node::open_commands(seed, u64::from(seed), reg);
    let home = Node::open_commands(seed + 1, u64::from(seed + 1), reg);
    let (thread, author) = thread_of(&peer, "peer's");
    peer.bind_commands(&author);
    peer.publish(&author).unwrap();
    peer.joined(&thread, &author, 1).unwrap();
    let (seat, guest) = (home.local_seat(), home.local_seat());
    home.publish(&seat).unwrap();
    home.publish(&guest).unwrap();
    sync_all(&[&home, &peer]);
    home.joined(&thread, &seat, 1).unwrap();
    sync_all(&[&home, &peer]);
    (peer, home, thread, author, seat, guest)
}

/// Kills: a peer's retired sites not seeing the reversible state. The control is the same commands
/// after the un-retire.
#[test]
fn peer_refuses_send_ack_recipient_and_invite_as_retired() {
    let (peer, home, thread, author, seat, guest) = peer_view(41);
    let CommandResult::ThreadCreated(pair) =
        peer.cmd_ensure_direct(&author, &seat, "pair").unwrap()
    else {
        panic!("a direct-pair result");
    };
    sync_all(&[&home, &peer]);
    let direct = |operation: &str| {
        let mut request = peer.prepare_request(&author, &pair, DeliveryMode::Lazy, operation);
        request.direct_recipient = Some(seat.clone());
        request
    };
    rev(&home, &seat, T1);
    rev(&home, &guest, T1);
    sync_all(&[&home, &peer]);

    // Today's refusal of a direct send to a seat with no eligible delivery, the retired one's.
    assert_eq!(
        peer.prepare_steps(&direct("direct")).unwrap_err().code,
        ErrorCode::TargetUnresolved
    );
    for refused in [
        peer.cmd_send_ack(&author, &thread, &[&seat], "ack")
            .unwrap_err(),
        peer.cmd_invite(&author, &thread, &guest, "invite")
            .unwrap_err(),
    ] {
        assert!(refused.detail.contains("retired"), "{refused:?}");
    }

    unret(&home, &seat, T1, T1 + WINDOW);
    unret(&home, &guest, T1, T1 + WINDOW);
    sync_all(&[&home, &peer]);
    peer.prepare_steps(&direct("direct-2")).unwrap();
    peer.cmd_send_ack(&author, &thread, &[&seat], "ack-2")
        .unwrap();
    peer.cmd_invite(&author, &thread, &guest, "invite-2")
        .unwrap();
}

/// Kills: a frozen row firing overdue on a peer or filling its due page. The control is the same
/// scans after the un-retire.
#[test]
fn peer_fires_no_overdue_and_frozen_rows_do_not_starve_its_due_page() {
    let (peer, home, thread, author, frozen, live) = peer_view(43);
    home.joined(&thread, &live, 1).unwrap();
    sync_all(&[&home, &peer]);
    let overdue = |seat: &SeatId| -> MessageId {
        let message = peer.stage_prep(&thread, "ordinary", &[(seat, FIVE_MIN, true)], &[]);
        let sent_at = SENT_AT - 2 * FIVE_MIN;
        let mut payload = sent(
            &message,
            &thread,
            &author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![spec(seat, ReceiptObligation::Ack, Some(sent_at + FIVE_MIN))],
        );
        payload.sent_at = sent_at;
        peer.author_sent(&payload).unwrap();
        peer.drive_send(&message);
        message
    };
    // The frozen seat's rows come first in every order.
    let frozen_message = overdue(&frozen);
    let live_message = overdue(&live);
    let invite = |seat: &SeatId| -> InvitationId {
        let mut payload = peer.created_payload(&thread, &author, seat);
        payload.deadline_at = SENT_AT - FIVE_MIN;
        payload.created_at = payload.deadline_at - payload.duration_ms;
        peer.author_created(&payload).unwrap();
        payload.invitation
    };
    let frozen_invitation = invite(&frozen);
    let live_invitation = invite(&live);
    rev(&home, &frozen, T1);
    sync_all(&[&home, &peer]);

    let mut conn = peer.raw();
    let mut cursor = ReceiptDueCursor::default();
    assert_eq!(
        receipts::scan_due(peer.ctx(), &mut conn, 1, &mut cursor)
            .unwrap()
            .warnings,
        1,
        "the page of one reaches the live row"
    );
    assert_eq!(peer.receipt_overdue(&live_message), 1);
    assert_eq!(peer.receipt_overdue(&frozen_message), 0);
    invitation_due::scan_invitation_due_batch(peer.ctx(), &mut conn, None, 1).unwrap();
    let warned = |id: &InvitationId| -> bool {
        peer.query(
            &format!(
                "SELECT warning_message_id IS NOT NULL FROM invitations WHERE id='{}'",
                id.as_str()
            ),
            |r| r.get(0),
        )[0]
    };
    assert!(warned(&live_invitation));
    assert!(!warned(&frozen_invitation));

    unret(&home, &frozen, T1, T1);
    sync_all(&[&home, &peer]);
    assert_eq!(peer.scan_receipts(), 1, "due again");
    assert_eq!(peer.receipt_overdue(&frozen_message), 1);
    assert!(peer.scan_invitations() >= 1);
    assert!(warned(&frozen_invitation));
}

// ---- mixed versions -------------------------------------------------------------

/// The shipped kinds without the two of spec 4.3: a node of the previous release.
fn registry_without_the_kinds() -> &'static FactRegistry {
    Box::leak(Box::new(
        FactRegistry::builder()
            .with::<NodeGenesis>()
            .with::<NodeRetag>()
            .with::<SeatCreated>()
            .with::<SeatRetired>()
            .with::<thread::ThreadCreated>()
            .with::<thread::ThreadRegister>()
            .with::<thread::ThreadDirect>()
            .with::<invitation::InvitationCreated>()
            .with::<invitation::InvitationAnswered>()
            .with::<membership::MembershipJoined>()
            .with::<membership::MembershipLeft>()
            .with::<message::MessageSent>()
            .with::<receipt::ReceiptSettled>()
            .build(),
    ))
}

#[derive(Clone, Copy)]
enum Step {
    Rev(i64),
    Unret(i64, i64),
    Final(i64),
    /// A timed invitation and an ACK request to the seat, authored at this time.
    ObligationsAt(i64),
}

/// The deadline of `seat`'s receipt as the node's readers see it.
fn receipt_deadline(node: &Node, message: &MessageId, seat: &SeatId) -> Option<i64> {
    effective::effective_receipt(&node.raw(), message.as_str(), seat.as_str())
        .unwrap()
        .unwrap()
        .deadline_at
}

/// What a node that knew the kinds and one that replayed them must agree on. A receipt or
/// invitation deadline is compared only while it can still fire: a seat finally retired has settled
/// it, and a node that parked the un-retire never shifted it.
fn snapshot(
    node: &Node,
    seat: &SeatId,
    invitations: &[InvitationId],
    messages: &[MessageId],
    live: bool,
) -> String {
    let row = seat_row(node, seat);
    let invitations: Vec<_> = invitations
        .iter()
        .map(|id| {
            let invitation = invitation_row(node, id);
            (invitation.0, invitation.2, live.then_some(invitation.1))
        })
        .collect();
    let receipts: Vec<_> = messages
        .iter()
        .map(|message| live.then(|| receipt_deadline(node, message, seat)))
        .collect();
    format!(
        "seat={:?} retired_at={:?} row={:?} jobs={:?} invitations={invitations:?} receipts={receipts:?}",
        row.0,
        row.1,
        rev_row(node, seat),
        retirement_rows(node)
            .into_iter()
            .map(|job| (job.1, job.2))
            .collect::<Vec<_>>(),
    )
}

fn parked(node: &Node) -> usize {
    node.journal()
        .iter()
        .filter(|(_, state, _)| state == "parked_unknown")
        .count()
}

/// Kills: a replay that revives a seat the final fact retired, double-shifts or writes a second job,
/// a node parking the final fact, and a replayed un-retire shifting an obligation created after it.
#[test]
fn a_registry_without_the_kinds_parks_them_and_converges_after_replay() {
    let after = T1 + WINDOW + 1_000;
    let histories: [(&str, &[Step]); 4] = [
        (
            "rev/unret/rev",
            &[Step::Rev(T1), Step::Unret(T1, T1 + WINDOW), Step::Rev(T2)],
        ),
        ("rev/final", &[Step::Rev(T1), Step::Final(T1)]),
        (
            "rev/unret/final",
            &[Step::Rev(T1), Step::Unret(T1, T1 + WINDOW), Step::Final(T2)],
        ),
        (
            "rev/unret/obligation-after",
            &[
                Step::Rev(T1),
                Step::Unret(T1, T1 + WINDOW),
                Step::ObligationsAt(after),
            ],
        ),
    ];
    for (n, (name, history)) in histories.iter().enumerate() {
        let seed = 50 + 3 * n as u8;
        let home = Node::with(seed, FactRegistry::builtins());
        let knows = Node::with(seed + 1, FactRegistry::builtins());
        let old = Node::with(seed + 2, registry_without_the_kinds());
        let (thread, creator) = thread_of(&home, "t");
        home.joined(&thread, &creator, 1).unwrap();
        let seat = published(&home, &[&knows, &old]);
        home.joined(&thread, &seat, 1).unwrap();
        let invitation = home.invite_fact(&thread, &creator, &seat);
        let ack_request = |at: i64| -> MessageId {
            let message = home.stage_prep(&thread, "ordinary", &[(&seat, FIVE_MIN, true)], &[]);
            let mut payload = sent(
                &message,
                &thread,
                &creator,
                DeliveryMode::Ordinary,
                FIVE_MIN,
                vec![spec(&seat, ReceiptObligation::Ack, Some(at + FIVE_MIN))],
            );
            payload.sent_at = at;
            home.author_sent(&payload).unwrap();
            message
        };
        let message = ack_request(SENT_AT);
        sync_all(&[&home, &knows, &old]);
        for node in [&home, &knows, &old] {
            node.drive_send(&message);
        }
        let invitation_deadline = invitation_row(&knows, &invitation).1.unwrap();
        let mut invitations = vec![invitation];
        let mut messages = vec![message];
        let mut live_after = true;
        let mut later = None;
        for step in *history {
            match *step {
                Step::Rev(at) => rev(&home, &seat, at),
                Step::Unret(at, until) => unret(&home, &seat, at, until),
                Step::Final(at) => {
                    fin(&home, &seat, at);
                    live_after = false;
                }
                Step::ObligationsAt(at) => {
                    let mut payload = home.created_payload(&thread, &creator, &seat);
                    payload.created_at = at;
                    payload.deadline_at = at + payload.duration_ms;
                    home.author_created(&payload).unwrap();
                    later = Some((
                        payload.invitation.clone(),
                        payload.deadline_at,
                        ack_request(at),
                        at + FIVE_MIN,
                    ));
                }
            }
        }
        sync_all(&[&home, &knows, &old]);
        if let Some((invitation, _, message, _)) = &later {
            invitations.push(invitation.clone());
            messages.push(message.clone());
            for node in [&home, &knows, &old] {
                node.drive_send(message);
            }
        }
        // What a node that knows the kinds holds, so the comparison below cannot pass on no-ops.
        let (state, retired_at, row, jobs): (&str, Option<i64>, Option<&str>, usize) = match n {
            0 => ("retired", Some(T2), Some("reversible"), 0),
            1 => ("retired", Some(T1), None, 1),
            2 => ("retired", Some(T2), None, 1),
            _ => ("remote", None, None, 0),
        };
        assert_eq!(seat_row(&knows, &seat).0, state, "{name}");
        assert_eq!(seat_row(&knows, &seat).1, retired_at, "{name}");
        assert_eq!(
            rev_row(&knows, &seat).map(|r| r.0).as_deref(),
            row,
            "{name}"
        );
        assert_eq!(retirement_rows(&knows).len(), jobs, "{name}");
        let kinds = history
            .iter()
            .filter(|step| matches!(step, Step::Rev(_) | Step::Unret(..)))
            .count();
        assert_eq!(parked(&old), kinds, "{name}: parked");
        let frozen_or_remote = seat_row(&old, &seat).0;
        if live_after {
            assert_eq!(frozen_or_remote, "remote", "{name}: reads live until final");
        } else {
            assert_eq!(
                frozen_or_remote, "retired",
                "{name}: the final fact applies"
            );
        }
        old.finish_retirements();

        // The upgrade: the same store under the shipped registry.
        let backup = old.snapshot("old-node.db");
        let upgraded =
            Node::open_restored(seed + 2, 9_000, FactRegistry::builtins(), &backup, false);
        loop {
            if !upgraded.store.replay_startup(&budget()).unwrap().has_more {
                break;
            }
        }
        assert_eq!(parked(&upgraded), 0, "{name}: {:?}", upgraded.journal());
        upgraded.finish_retirements();
        knows.finish_retirements();
        assert_eq!(
            snapshot(&upgraded, &seat, &invitations, &messages, live_after),
            snapshot(&knows, &seat, &invitations, &messages, live_after),
            "{name}"
        );
        assert!(
            !upgraded
                .journal()
                .iter()
                .any(|(_, state, _)| state == "failed"),
            "{name}: {:?}",
            upgraded.journal()
        );
        // The bound itself, on the replaying node: what existed moves once, what came later stays.
        if let Some((invitation, invitation_at, message, receipt_at)) = &later {
            for node in [&upgraded, &knows] {
                assert_eq!(
                    invitation_row(node, &invitations[0]).1,
                    Some(invitation_deadline + WINDOW),
                    "{name}: the first invitation shifted once"
                );
                assert_eq!(
                    receipt_deadline(node, &messages[0], &seat),
                    Some(SENT_AT + FIVE_MIN + WINDOW),
                    "{name}: the first receipt shifted once"
                );
                assert_eq!(
                    invitation_row(node, invitation).1,
                    Some(*invitation_at),
                    "{name}: the later invitation unshifted"
                );
                assert_eq!(
                    receipt_deadline(node, message, &seat),
                    Some(*receipt_at),
                    "{name}: the later receipt unshifted"
                );
            }
        }
    }
}
