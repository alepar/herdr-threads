//! `store::peer_handoff::stage`, the target's one-transaction staging of a remote handoff
//! (spec §3.2 step 2, §3.3; TRUST-POLICY N15), and the handoff-aware `peer_requests::gate_view`.
//!
//! Two real journal-open stores: S (the sender's node, which homes the author seat A and signs
//! the block) and R (the target, which homes the recipient seat). R holds S's log, so A's seat
//! row and S's key are replicated here the way they are in production.

use herdr_threads::node::{NodeIdentity, tag_for};
use herdr_threads::ports::BootstrapStorePort;
use herdr_threads::protocol::authority::{CallerClaim, CallerRole, Harness};
use herdr_threads::protocol::handoff::BootstrapAttempt;
use herdr_threads::protocol::ids::{
    ExecutionId, HostTargetId, MessageId, NativeSessionId, SeatId, ThreadId, direct_thread_id,
    generate_public_id, prefix, send_message_id_for_preparation,
};
use herdr_threads::protocol::results::{ApiError, CommandResult, ErrorCode};
use herdr_threads::protocol::summary::AuthorRole;
use herdr_threads::store::peer_handoff::{self, GateRecord, Staged};
use herdr_threads::store::{control, peer_requests};
use herdr_threads::sync::frame::{LaunchPlan, LaunchTarget};
use herdr_threads::sync::handoff_block::SenderBlock;
use rusqlite::params;
use uuid::Uuid;

use crate::facts_seat::{Node, budget, registry};
use crate::facts_thread::sync_all;

const REQUEST: &str = "00000000-0000-0000-0000-0000000000a1";
const REQUESTED_AT: i64 = 1_700_000_000_000;
const ACK_AFTER_MS: i64 = 600_000;

struct World {
    sender: Node,
    target: Node,
    sender_seed: u8,
    author: SeatId,
    recipient: SeatId,
    message: MessageId,
}

/// S homes A, R homes the recipient; both seats are replicated everywhere. The `running`
/// request row for [`REQUEST`] is on R, its plan carrying a block signed by S's key. `seed`
/// keeps each test's two nodes distinct.
fn world(seed: u8) -> World {
    let reg = registry();
    let sender_seed = 40 + seed * 2;
    let (sender, target) = (
        Node::with(sender_seed, reg),
        Node::with(sender_seed + 1, reg),
    );
    let author = sender.local_seat();
    let recipient = target.local_seat();
    sender.publish(&author).unwrap();
    target.publish(&recipient).unwrap();
    sync_all(&[&sender, &target]);
    assert!(
        target.seat(&author).is_some(),
        "A reached R through S's log"
    );
    let message = MessageId::new(
        send_message_id_for_preparation(&generate_public_id(
            prefix::SEND_PREPARATION,
            &tag_for(sender.id, 0),
        ))
        .unwrap(),
    );
    let world = World {
        sender,
        target,
        sender_seed,
        author,
        recipient,
        message,
    };
    world.insert_request(REQUEST, "running", true);
    world
}

type Row = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl World {
    fn block(&self) -> SenderBlock {
        let mut block = SenderBlock {
            v: 1,
            request: Uuid::parse_str(REQUEST).unwrap(),
            sender_node: Some(self.sender.id),
            author: Some(self.author.clone()),
            author_role: Some(AuthorRole::Agent),
            message: Some(self.message.clone()),
            body: "take over the migration".into(),
            priority: 0,
            require_ack: true,
            requested_at: REQUESTED_AT,
            ack_after_ms: Some(ACK_AFTER_MS),
            sig: String::new(),
        };
        block
            .sign(&NodeIdentity::from_seed([self.sender_seed; 32]))
            .unwrap();
        block
    }

    fn insert_request(&self, id: &str, state: &str, handoff: bool) {
        let plan = LaunchPlan {
            target: LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "p1".into(),
            },
            kind: "claude".into(),
            message: handoff.then(|| Box::new(self.block())),
            ..LaunchPlan::default()
        };
        self.target
            .raw()
            .execute(
                "INSERT INTO peer_requests(id,node_uuid,kind,plan,state,created_at,updated_at) \
                 VALUES (?1,?2,'pane',?3,?4,1,1)",
                params![
                    id,
                    self.sender.id.to_string(),
                    serde_json::to_string(&plan).unwrap(),
                    state
                ],
            )
            .unwrap();
    }

    fn stage_as(&self, seat: &SeatId, gate: Option<GateRecord<'_>>) -> Result<Staged, ApiError> {
        self.target
            .run(|tx, d| peer_handoff::stage(tx, self.target.ctx(), d, REQUEST, seat, gate))
    }

    fn stage(&self) -> Result<Staged, ApiError> {
        self.stage_as(&self.recipient, None)
    }

    fn pair(&self) -> ThreadId {
        direct_thread_id(&self.author, &self.recipient).unwrap()
    }

    fn row(&self) -> Row {
        self.target
            .query(
                "SELECT state,handoff_thread,handoff_message,seat,gate_created FROM peer_requests \
                 WHERE id='00000000-0000-0000-0000-0000000000a1'",
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .remove(0)
    }
}

fn count(node: &Node, table_where: &str) -> i64 {
    node.query(&format!("SELECT count(*) FROM {table_where}"), |r| r.get(0))[0]
}

#[test]
fn stage_writes_three_facts() {
    let w = world(0);
    let before = w.target.journal_kinds();
    let staged = w.stage().unwrap();
    assert_eq!(staged.thread, w.pair());
    assert_eq!(staged.message, w.message);
    assert!(staged.fresh);
    assert_eq!(staged.sender_node, w.sender.id.to_string());

    let after = w.target.journal_kinds();
    assert_eq!(
        after[before.len()..],
        ["thread.direct", "membership.joined", "message.sent"],
        "exactly the three facts, in order"
    );
    assert_eq!(count(&w.target, "invitations"), 0, "no invitation");
    assert_eq!(
        count(&w.target, "receipt_state WHERE state!='pending'"),
        0,
        "no receipt settlement"
    );

    let (state, thread, message, seat, gate) = w.row();
    assert_eq!(state, "running");
    assert_eq!(thread.as_deref(), Some(w.pair().as_str()));
    assert_eq!(message.as_deref(), Some(w.message.as_str()));
    assert_eq!((seat, gate), (None, None), "no gate record without a gate");

    // The ACK obligation is part of the same commit; the node-local worker projects it to a receipt.
    assert_eq!(
        count(
            &w.target,
            &format!(
                "prepared_recipients WHERE seat_id='{}'",
                w.recipient.as_str()
            )
        ),
        1
    );
    crate::fact_convergence::run_workers(&w.target.store);
    // The materialized message: A's, priority 0, R owing a pending ACK whose deadline is anchored
    // on the sender's requested_at.
    let rows = w.target.query(
        &format!(
            "SELECT m.actor_seat_id,m.priority,m.thread_id,r.seat_id,r.state,r.deadline_at \
             FROM messages m JOIN receipt_state r ON r.message_id=m.id WHERE m.id='{}'",
            w.message.as_str()
        ),
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        },
    );
    assert_eq!(
        rows,
        [(
            w.author.as_str().to_owned(),
            0,
            w.pair().as_str().to_owned(),
            w.recipient.as_str().to_owned(),
            "pending".to_owned(),
            REQUESTED_AT + ACK_AFTER_MS
        )]
    );
    // R joined; A's membership is its home node's to author (N4).
    assert_eq!(
        count(
            &w.target,
            &format!("memberships WHERE thread_id='{}'", w.pair().as_str())
        ),
        1
    );
    assert!(w.target.membership(&w.pair(), &w.recipient).is_some());
    assert_eq!(w.target.membership(&w.pair(), &w.author), None);
}

#[test]
fn stage_is_idempotent() {
    let w = world(1);
    let first = w.stage().unwrap();
    let journal = w.target.journal().len();
    let rows = (
        count(&w.target, "messages"),
        count(&w.target, "memberships"),
    );
    let again = w.stage().unwrap();
    assert_eq!(
        (&again.thread, &again.message),
        (&first.thread, &first.message)
    );
    assert!(!again.fresh, "a repeat reports nothing new to audit");
    assert_eq!(
        w.target.journal().len(),
        journal,
        "a repeat authors nothing"
    );
    assert_eq!(
        rows,
        (
            count(&w.target, "messages"),
            count(&w.target, "memberships")
        )
    );
}

// Kills: an idempotent return that hands the facts of one seat's request to a call naming
// another seat.
#[test]
fn stage_repeat_for_another_seat_is_refused() {
    let w = world(5);
    let first = w.stage().unwrap();
    // The launch recorded its seat.
    w.target
        .raw()
        .execute("UPDATE peer_requests SET seat=?1", [w.recipient.as_str()])
        .unwrap();
    let journal = w.target.journal().len();
    let error = w.stage_as(&w.author, None).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.detail, "peer request names another seat");
    assert_eq!(w.target.journal().len(), journal, "no fact");
    let again = w.stage().unwrap();
    assert_eq!(
        (&again.thread, &again.message, again.fresh),
        (&first.thread, &first.message, false)
    );
}

#[test]
fn stage_refuses_when_not_running() {
    let w = world(2);
    w.target
        .raw()
        .execute("UPDATE peer_requests SET state='failed'", [])
        .unwrap();
    let before = w.target.journal().len();
    let error = w.stage().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(w.target.journal().len(), before, "no fact");
    let (state, thread, message, ..) = w.row();
    assert_eq!((state.as_str(), thread, message), ("failed", None, None));
}

#[test]
fn stage_refuses_when_seat_moved() {
    // A seat that is not the target's: the author's replicated row is not local.
    let w = world(3);
    let before = w.target.journal().len();
    assert_eq!(
        w.stage_as(&w.author, None).unwrap_err().code,
        ErrorCode::Conflict
    );
    // A retired target seat.
    w.target
        .raw()
        .execute(
            "UPDATE seats SET state='retired', retired_at=1 WHERE id=?1",
            [w.recipient.as_str()],
        )
        .unwrap();
    assert_eq!(w.stage().unwrap_err().code, ErrorCode::Conflict);
    // A gate row whose recorded seat differs from the one asked for.
    let w2 = world(4);
    w2.target
        .raw()
        .execute("UPDATE peer_requests SET seat='sSomeoneElse'", [])
        .unwrap();
    let before2 = w2.target.journal().len();
    assert_eq!(w2.stage().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(w.target.journal().len(), before, "no fact");
    assert_eq!(w2.target.journal().len(), before2, "no fact");
    assert_eq!(w2.row().1, None);
}

#[test]
fn stage_refuses_a_row_without_a_block() {
    let w = world(5);
    w.target
        .raw()
        .execute("DELETE FROM peer_requests", [])
        .unwrap();
    w.insert_request(REQUEST, "running", false);
    let before = w.target.journal().len();
    assert_eq!(w.stage().unwrap_err().code, ErrorCode::InvalidRequest);
    assert_eq!(w.target.journal().len(), before);
}

#[test]
fn existing_pair_reused() {
    let w = world(6);
    // The pair already exists, created by the recipient's side, which holds its own join.
    let claim = CallerClaim {
        instance: w.target.id.to_string(),
        host: herdr_threads::test_support::test_host_locator(),
        seat: w.recipient.clone(),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Human,
        native_session: NativeSessionId::new("human"),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("w1:p1"),
        agent_id: None,
        liveness: None,
    };
    let (thread, created) = w
        .target
        .run(|tx, d| {
            control::resolve_direct_pair(tx, w.target.ctx(), d, &claim, &w.author, None, None)
        })
        .unwrap();
    assert!(created);
    assert_eq!(thread, w.pair());
    let before = w.target.journal_kinds();
    let staged = w.stage().unwrap();
    assert_eq!(staged.thread, thread);
    assert_eq!(
        w.target.journal_kinds()[before.len()..],
        ["message.sent"],
        "no second thread.direct, and R already holds its join"
    );
    assert_eq!(
        count(&w.target, "threads WHERE id LIKE 'd%'"),
        1,
        "one pair thread"
    );
}

#[test]
fn crossed_create_same_id() {
    let w = world(7);
    // The sender's node opens the same pair before it hears of the staging.
    let claim = CallerClaim {
        instance: w.sender.id.to_string(),
        host: herdr_threads::test_support::test_host_locator(),
        seat: w.author.clone(),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Human,
        native_session: NativeSessionId::new("human"),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("w1:p1"),
        agent_id: None,
        liveness: None,
    };
    let (sender_thread, created) = w
        .sender
        .run(|tx, d| {
            control::resolve_direct_pair(tx, w.sender.ctx(), d, &claim, &w.recipient, None, None)
        })
        .unwrap();
    assert!(created);
    let staged = w.stage().unwrap();
    assert_eq!(sender_thread, staged.thread, "both sides derive one id");

    sync_all(&[&w.sender, &w.target]);
    for node in [&w.sender, &w.target] {
        assert_eq!(
            node.query("SELECT id FROM threads WHERE id LIKE 'd%'", |r| {
                r.get::<_, String>(0)
            }),
            [staged.thread.as_str()],
            "the crossed creates left one thread on each node"
        );
        assert_eq!(count(node, "direct_threads"), 1);
    }
}

fn created_json() -> String {
    use herdr_threads::ports::{CreatedTarget, HcPeer, HostWitness, Placement};
    use herdr_threads::protocol::ids::{HostBootId, HostCallId, TerminalId};
    serde_json::to_string(&CreatedTarget {
        correlation: HostCallId::new("c"),
        placement: Placement::NewTerminal {
            parent: HostTargetId::new("t1"),
        },
        target: HostTargetId::new("t2"),
        root_pane: None,
        terminal: TerminalId::new("t2"),
        host_incarnation: HostBootId::new("hc-core:core-a:pid=42:start=7:uid=501"),
        witness: HostWitness::HcCore {
            core_id: "core-a".into(),
            peer: HcPeer {
                pid: 42,
                start_seconds: 7,
                uid: 501,
            },
        },
    })
    .unwrap()
}

fn gate_view(w: &World) -> herdr_threads::ports::LaunchGateView {
    let db = w.target.raw();
    peer_requests::gate_view(&db, REQUEST, BootstrapAttempt::first())
        .unwrap()
        .expect("a gate row")
}

#[test]
fn gate_view_waits_for_handoff_message() {
    let w = world(8);
    let created = created_json();
    let db = w.target.raw();
    peer_requests::open_gate(&db, REQUEST).unwrap();
    db.execute(
        "UPDATE peer_requests SET gate_created=?1",
        [created.as_str()],
    )
    .unwrap();
    drop(db);
    assert!(
        !gate_view(&w).send_staged,
        "created, but the handoff message is not staged yet"
    );
    w.stage().unwrap();
    assert!(gate_view(&w).send_staged, "released by the staged message");

    // A launch-only gate row (no block) releases on creation, as before.
    let plain = world(9);
    plain
        .target
        .raw()
        .execute("DELETE FROM peer_requests", [])
        .unwrap();
    plain.insert_request(REQUEST, "running", false);
    let db = plain.target.raw();
    peer_requests::open_gate(&db, REQUEST).unwrap();
    assert!(!gate_view(&plain).send_staged, "not created yet");
    db.execute(
        "UPDATE peer_requests SET gate_created=?1",
        [created.as_str()],
    )
    .unwrap();
    drop(db);
    assert!(gate_view(&plain).send_staged);
}

#[test]
fn stage_with_gate_record_opens_gate_in_same_commit() {
    let w = world(10);
    peer_requests::open_gate(&w.target.raw(), REQUEST).unwrap();
    let created = created_json();
    w.stage_as(
        &w.recipient,
        Some(GateRecord {
            created: &created,
            terminal: "t2",
        }),
    )
    .unwrap();
    let (state, thread, message, seat, gate) = w.row();
    assert_eq!(state, "launched");
    assert!(thread.is_some() && message.is_some());
    assert_eq!(seat.as_deref(), Some(w.recipient.as_str()));
    assert_eq!(gate.as_deref(), Some(created.as_str()));
    assert!(gate_view(&w).send_staged);

    // Without an open gate record the gate half fails and the whole commit goes with it.
    let closed = world(11);
    let before = closed.target.journal().len();
    let error = closed
        .stage_as(
            &closed.recipient,
            Some(GateRecord {
                created: &created,
                terminal: "t2",
            }),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(closed.target.journal().len(), before, "no fact survives");
    assert_eq!(closed.row().1, None);
}

#[test]
fn the_port_serves_a_running_row_and_refuses_a_failed_one() {
    let w = world(12);
    let port: &dyn BootstrapStorePort = &*w.target.store;
    let CommandResult::PeerHandoffStaged { thread, message } = port
        .peer_handoff_stage(REQUEST, &w.recipient, &budget())
        .unwrap()
    else {
        panic!("PeerHandoffStaged expected");
    };
    assert_eq!((thread, message), (w.pair(), w.message.clone()));

    let failed = world(13);
    failed
        .target
        .raw()
        .execute("UPDATE peer_requests SET state='failed'", [])
        .unwrap();
    let before = failed.target.journal().len();
    let port: &dyn BootstrapStorePort = &*failed.target.store;
    let error = port
        .peer_handoff_stage(REQUEST, &failed.recipient, &budget())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(failed.target.journal().len(), before);
}

// Kills: a view that stages as a side effect, drops the frozen prompt or the row's state, or
// answers a launch-only row (whose child must stay the plain launch) or an unknown id.
#[test]
fn the_view_reads_a_handoff_row_and_writes_nothing() {
    let w = world(14);
    w.target
        .raw()
        .execute(
            "UPDATE peer_requests SET handoff_prompt='fix it \u{2014} from: s' WHERE id=?1",
            [REQUEST],
        )
        .unwrap();
    w.insert_request("00000000-0000-0000-0000-0000000000a2", "running", false);
    let before = w.target.journal().len();
    let port: &dyn BootstrapStorePort = &*w.target.store;

    let CommandResult::PeerRequestView(view) = port.peer_request_view(REQUEST, &budget()).unwrap()
    else {
        panic!("PeerRequestView expected");
    };
    assert_eq!(view.state, "running");
    assert_eq!(
        view.handoff_prompt.as_deref(),
        Some("fix it \u{2014} from: s")
    );
    assert_eq!(view.plan.handoff(), Some(&w.block()));

    let launch_only = port
        .peer_request_view("00000000-0000-0000-0000-0000000000a2", &budget())
        .unwrap_err();
    assert_eq!(launch_only.code, ErrorCode::InvalidRequest);
    let unknown = port.peer_request_view("nope", &budget()).unwrap_err();
    assert_eq!(unknown.code, ErrorCode::NotFound);
    assert_eq!(w.target.journal().len(), before, "a read writes no fact");
    assert_eq!(w.row().1, None, "and stages nothing");
}
