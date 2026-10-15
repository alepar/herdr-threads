//! The per-binding inbox stream (ht-54r.7, spec §2 Inbox per binding): each binding of a seat reads
//! its own unread mail past its own cursors; invitations and pending ACK obligations stay
//! seat-wide; a display advances only the displaying binding's cursors.
use super::*;
use crate::ports::StorePort;
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, ObligationRef},
    commands::{Ack, AdvanceCursors, Join, PermitMutation, SendMessage},
    ids::*,
    results::InboxBatchV2Item,
    time::UtcMillis,
};
use crate::store::{bindings, messages, read_cursors};
use crate::test_support::isolation::TestIsolation;
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

const EXEC_A: &str = "00000000-0000-4000-8000-0000000000aa";
const EXEC_B1: &str = "00000000-0000-4000-8000-0000000000b1";
const EXEC_B2: &str = "00000000-0000-4000-8000-0000000000b2";
const EXEC_B3: &str = "00000000-0000-4000-8000-0000000000b3";

/// Sender seat `a`, recipient seat `b` (two sessions `nb1`, `nb2`), thread `t`.
struct Fx {
    iso: TestIsolation,
    context: StoreContext,
    conn: Connection,
    sends: u32,
}

fn insert_binding(
    conn: &Connection,
    seat: &str,
    session: &str,
    execution: &str,
    agent: Option<(&str, &str)>,
) -> BindingId {
    let (role, parent, agent_id) = match agent {
        Some((parent, agent)) => ("subagent", Some(parent.to_owned()), agent),
        None => ("top_level", None, ""),
    };
    let target = format!("p{seat}");
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,role,parent_binding,agent_id) VALUES (?1,1,?2,'b',1,1,'codex',?3,?4,'cooperative_top_level',0,0,?5,'inc',?6,?7,?8)",
        rusqlite::params![seat, target, session, execution, format!("term-{target}"), role, parent, agent_id],
    )
    .unwrap();
    let id = BindingId::new(
        conn.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND native_session=?2 AND agent_id=?3",
            rusqlite::params![seat, session, agent_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    read_cursors::init_for_new_binding(conn, &id).unwrap();
    id
}

fn fx(name: &str) -> Fx {
    let iso = TestIsolation::new(name);
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    for seat in ["a", "b", "c"] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?2,1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", rusqlite::params![seat, format!("p{seat}")]).unwrap();
        conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,1,0,'fresh',?2,'inc','coherent_enumeration',1)", rusqlite::params![format!("p{seat}"), format!("term-p{seat}")]).unwrap();
    }
    conn.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)",
        [],
    )
    .unwrap();
    for seat in ["a", "b"] {
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    insert_binding(&conn, "a", "na", EXEC_A, None);
    insert_binding(&conn, "b", "nb1", EXEC_B1, None);
    insert_binding(&conn, "b", "nb2", EXEC_B2, None);
    Fx {
        iso,
        context,
        conn,
        sends: 0,
    }
}

fn claim(seat: &str, session: &str, execution: &str, agent: Option<&str>) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: if agent.is_some() {
            CallerRole::Subagent
        } else {
            CallerRole::TopLevel
        },
        harness: Harness::Codex,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new(execution),
        target: HostTargetId::new(format!("p{seat}")),
        agent_id: agent.map(AgentId::new),
        liveness: None,
    }
}
fn claim_a() -> CallerClaim {
    claim("a", "na", EXEC_A, None)
}
fn claim_b1() -> CallerClaim {
    claim("b", "nb1", EXEC_B1, None)
}
fn claim_b2() -> CallerClaim {
    claim("b", "nb2", EXEC_B2, None)
}

fn store(fx: &Fx) -> crate::store::SqliteStore {
    crate::store::SqliteStore::new(
        StoreContext::new(fx.iso.path("store.db"), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap()
}

fn send(fx: &mut Fx, body: &str) -> MessageId {
    send_mode(fx, body, crate::protocol::commands::DeliveryMode::Ordinary)
}
fn send_lazy(fx: &mut Fx, body: &str) -> MessageId {
    send_mode(fx, body, crate::protocol::commands::DeliveryMode::Lazy)
}
fn send_mode(
    fx: &mut Fx,
    body: &str,
    delivery_mode: crate::protocol::commands::DeliveryMode,
) -> MessageId {
    send_in(fx, "t", body, delivery_mode)
}
fn send_in(
    fx: &mut Fx,
    thread: &str,
    body: &str,
    delivery_mode: crate::protocol::commands::DeliveryMode,
) -> MessageId {
    fx.sends += 1;
    let request = SendMessage {
        delivery_mode,
        user_intent: None,
        thread: ThreadId::new(thread),
        body: body.into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new(format!("send-{}", fx.sends)),
        claim: claim_a(),
        relays_user: false,
        direct_recipient: None,
    };
    loop {
        let progress = messages::prepare_send_step(
            &fx.context,
            &mut fx.conn,
            &request,
            messages::MessageLimits::default(),
            &budget(),
            crate::ports::DurableWorkAdmission { max_units: 1 },
        )
        .unwrap();
        if matches!(
            progress,
            crate::ports::SendPreparationProgress::Ready { .. }
        ) {
            break;
        }
    }
    let mut permit = store(fx)
        .issue_cooperative_permit(
            crate::ports::CooperativePermitRequest {
                claim: request.claim.clone(),
                operation: request.operation.clone(),
                obligation: ObligationRef::Control(request.thread.clone()),
                payload_hash: crate::store::schema::canonical_digest(&messages::send_payload(
                    &request,
                ))
                .unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::MessageSent(id) = messages::publish_send(
        &fx.context,
        &mut fx.conn,
        &request,
        &mut permit,
        &budget(),
        || 65536,
    )
    .unwrap() else {
        panic!("send")
    };
    id
}

/// What a page lists, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A whole ordinary message and whether the caller may ACK it by displaying it.
    Msg(String, bool),
    /// A lazy (passive) message; it carries no ACK.
    Lazy(String),
    Invitation(String),
}

fn page(fx: &Fx, claim: Option<&CallerClaim>) -> Vec<Row> {
    page_in(fx, claim, None)
}
/// `page`, thread-scoped (`InboxQuery.thread`, ht-br0) when `thread` is set.
fn page_in(fx: &Fx, claim: Option<&CallerClaim>, thread: Option<&str>) -> Vec<Row> {
    let CommandResult::InboxBatchV2(page) = query_with_output(
        &fx.context,
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            thread: thread.map(ThreadId::new),
            claim: claim.cloned(),
            seat: Some(SeatId::new("b")),
            page: PageRequest {
                limit: 50,
                max_bytes: 60_000,
                cursor: None,
            },
        }),
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap() else {
        panic!("v2 page")
    };
    assert!(!page.has_more, "one page holds the fixture's mail");
    page.items
        .into_iter()
        .filter_map(|item| match item {
            InboxBatchV2Item::Message {
                message,
                ack_candidate,
                ..
            } => Some(Row::Msg(
                message.as_str().to_owned(),
                ack_candidate.is_some(),
            )),
            InboxBatchV2Item::LazyMessage { message, .. } => {
                Some(Row::Lazy(message.as_str().to_owned()))
            }
            InboxBatchV2Item::Invitation { thread, .. } => {
                Some(Row::Invitation(thread.as_str().to_owned()))
            }
            _ => None,
        })
        .collect()
}
fn msg(id: &MessageId, ack: bool) -> Row {
    Row::Msg(id.as_str().to_owned(), ack)
}

fn advance(fx: &Fx, claim: &CallerClaim, op: &str, displayed: &[&MessageId]) {
    store(fx)
        .advance_cursors(
            &AdvanceCursors {
                in_turn: false,
                invitations: Vec::new(),
                claim: claim.clone(),
                operation: OperationId::new(op),
                displayed: displayed.iter().map(|m| (*m).clone()).collect(),
            },
            &budget(),
        )
        .unwrap();
}

/// A top-level display ACK (what the CLI sends after flushing the page).
fn ack_displayed(fx: &Fx, claim: &CallerClaim, op: &str, ids: &[&MessageId]) {
    let mutation = PermitMutation::AckDisplayed(Ack {
        messages: ids.iter().map(|m| (*m).clone()).collect(),
        operation: OperationId::new(op),
        claim: claim.clone(),
    });
    let permit = store(fx)
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&mutation).unwrap(),
            &budget(),
        )
        .unwrap();
    store(fx).mutate(mutation, permit, &budget()).unwrap();
}

fn binding_of(fx: &Fx, claim: &CallerClaim) -> BindingId {
    bindings::match_claim(&fx.conn, &bindings::BindingKey::from_claim(claim))
        .unwrap()
        .unwrap()
        .binding_id
}
/// The thread sequence of a message (events such as availability warnings share the counter).
fn seq(fx: &Fx, id: &MessageId) -> u64 {
    fx.conn
        .query_row(
            "SELECT sequence FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get::<_, i64>(0),
        )
        .unwrap() as u64
}
/// The cursor sits at or past `from` and before `before` (it also passes the events that follow
/// a displayed message and are never delivered through a cursor, such as warnings).
fn assert_cursor(fx: &Fx, claim: &CallerClaim, from: &MessageId, before: Option<&MessageId>) {
    let at = cursor(fx, claim).expect("a cursor");
    assert!(at >= seq(fx, from), "cursor {at} is short of {from:?}");
    if let Some(next) = before {
        assert!(at < seq(fx, next), "cursor {at} passed {next:?}");
    }
}
fn cursor(fx: &Fx, claim: &CallerClaim) -> Option<u64> {
    read_cursors::cursor_for(&fx.conn, &binding_of(fx, claim), &ThreadId::new("t")).unwrap()
}

#[test]
fn two_bindings_each_receive_every_message_exactly_once() {
    let mut fx = fx("bi-two-bindings");
    let ids: Vec<MessageId> = (1..=3).map(|n| send(&mut fx, &format!("m{n}"))).collect();
    // Each binding lists all three once (an unread message that is also an ACK obligation is
    // one row, not two), and may ACK them.
    let all = |ack: bool| ids.iter().map(|m| msg(m, ack)).collect::<Vec<_>>();
    assert_eq!(page(&fx, Some(&claim_b1())), all(true));
    assert_eq!(page(&fx, Some(&claim_b2())), all(true));
    // A's display: the top-level ACK settles the seat's obligation, then A's cursor moves.
    ack_displayed(&fx, &claim_b1(), "ack-1", &ids.iter().collect::<Vec<_>>());
    advance(&fx, &claim_b1(), "adv-1", &ids.iter().collect::<Vec<_>>());
    assert_eq!(page(&fx, Some(&claim_b1())), vec![]);
    // B still lists all three, now marked already settled (no ACK candidate).
    assert_eq!(page(&fx, Some(&claim_b2())), all(false));
    advance(&fx, &claim_b2(), "adv-2", &ids.iter().collect::<Vec<_>>());
    assert_eq!(page(&fx, Some(&claim_b1())), vec![]);
    assert_eq!(page(&fx, Some(&claim_b2())), vec![]);
    assert_cursor(&fx, &claim_b1(), &ids[2], None);
    assert_cursor(&fx, &claim_b2(), &ids[2], None);
}

#[test]
fn one_bindings_display_never_hides_mail_from_the_other() {
    let mut fx = fx("bi-interleaved");
    let m1 = send(&mut fx, "one");
    let m2 = send(&mut fx, "two");
    ack_displayed(&fx, &claim_b1(), "ack-1", &[&m1, &m2]);
    advance(&fx, &claim_b1(), "adv-1", &[&m1, &m2]);
    let m3 = send(&mut fx, "three");
    // A sees only the new message; B sees everything it has not displayed itself.
    assert_eq!(page(&fx, Some(&claim_b1())), vec![msg(&m3, true)]);
    assert_eq!(
        page(&fx, Some(&claim_b2())),
        vec![msg(&m1, false), msg(&m2, false), msg(&m3, true)]
    );
    // B displays only the first of its mail: its cursor stops there, A's is untouched.
    advance(&fx, &claim_b2(), "adv-2", &[&m1]);
    assert_cursor(&fx, &claim_b2(), &m1, Some(&m2));
    assert_cursor(&fx, &claim_b1(), &m2, Some(&m3));
    assert_eq!(
        page(&fx, Some(&claim_b2())),
        vec![msg(&m2, false), msg(&m3, true)]
    );
    // Without a claim the page is today's per-seat page, unchanged by the cursors.
    assert_eq!(page(&fx, None), vec![msg(&m3, true)]);
}

#[test]
fn subagent_display_advances_only_its_own_cursor() {
    let mut fx = fx("bi-subagent");
    let parent = binding_of(&fx, &claim_b1());
    insert_binding(
        &fx.conn,
        "b",
        "nb1",
        EXEC_B1,
        Some((parent.as_str(), "ag1")),
    );
    let sub = claim("b", "nb1", EXEC_B1, Some("ag1"));
    let m1 = send(&mut fx, "one");
    let before = page(&fx, Some(&claim_b1()));
    assert_eq!(before, vec![msg(&m1, true)]);
    // The subagent sees the message but is never offered the ACK.
    assert_eq!(page(&fx, Some(&sub)), vec![msg(&m1, false)]);
    advance(&fx, &sub, "adv-sub", &[&m1]);
    assert_cursor(&fx, &sub, &m1, None);
    assert_eq!(cursor(&fx, &claim_b1()), Some(0));
    // The parent's page is unchanged and the obligation is still pending.
    assert_eq!(page(&fx, Some(&claim_b1())), before);
    assert!(
        crate::store::effective::effective_receipt(&fx.conn, m1.as_str(), "b")
            .unwrap()
            .is_some_and(|r| r.state == crate::store::effective::EffectiveReceiptState::Pending)
    );
    // The obligation is seat-wide, so it stays on the subagent's page too.
    assert_eq!(page(&fx, Some(&sub)), vec![msg(&m1, false)]);
}

#[test]
fn pending_obligations_older_than_the_binding_are_listed() {
    let mut fx = fx("bi-older-obligation");
    let m1 = send(&mut fx, "one");
    let m2 = send(&mut fx, "two");
    // One binding displays both (without settling them); a binding enrolled after that starts
    // at the seat maximum: nothing of the mail is unread by it.
    advance(&fx, &claim_b1(), "adv-1", &[&m1, &m2]);
    insert_binding(&fx.conn, "b", "nb3", EXEC_B3, None);
    let late = claim("b", "nb3", EXEC_B3, None);
    assert_cursor(&fx, &late, &m2, None);
    assert!(!read_cursors::has_unread(&fx.conn, &binding_of(&fx, &late)).unwrap());
    assert!(read_cursors::has_unread(&fx.conn, &binding_of(&fx, &claim_b2())).unwrap());
    // Both messages are still the seat's pending obligations: they are on its page, once.
    assert_eq!(page(&fx, Some(&late)), vec![msg(&m1, true), msg(&m2, true)]);
    // Settled elsewhere, they are gone for it (its cursor is already past them).
    ack_displayed(&fx, &claim_b1(), "ack-1", &[&m1, &m2]);
    assert_eq!(page(&fx, Some(&late)), vec![]);
}

#[test]
fn invitations_are_listed_per_seat() {
    let fx = fx("bi-invitations");
    fx.conn
        .execute_batch(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('u','34750f98bd59fcfc946da45aaabe933b','other','goal',0,0);
             INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES ('u','b',1,'invited','invited');
             INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv1','u','b',1,'pending',1,0,300,99999999);",
        )
        .unwrap();
    let want = vec![Row::Invitation("u".into())];
    assert_eq!(page(&fx, Some(&claim_b1())), want);
    assert_eq!(page(&fx, Some(&claim_b2())), want);
    // The invitation is the seat's: it is listed to every binding and to the per-seat page.
    assert_eq!(page(&fx, Some(&claim_b2())), want);
    assert_eq!(page(&fx, Some(&claim_b1())), want);
    assert_eq!(page(&fx, None), want);
}

#[test]
fn unknown_session_inbox_shows_no_backlog_past_the_seat_maximum() {
    let mut fx = fx("bi-unknown");
    let m1 = send(&mut fx, "one");
    let m2 = send(&mut fx, "two");
    ack_displayed(&fx, &claim_b1(), "ack-1", &[&m1, &m2]);
    let stranger = claim("b", "never-seen", EXEC_B3, None);
    // A known binding at 0 lists the settled history; a session the daemon has never seen gets
    // what a fresh binding would: no backlog (everything is settled), nothing enrolled.
    assert_eq!(
        page(&fx, Some(&claim_b2())),
        vec![msg(&m1, false), msg(&m2, false)]
    );
    assert_eq!(page(&fx, Some(&stranger)), vec![]);
    assert!(
        bindings::match_claim(&fx.conn, &bindings::BindingKey::from_claim(&stranger))
            .unwrap()
            .is_none(),
        "a read never enrolls"
    );
    // Seat invitations and a still-pending obligation do reach it.
    let m3 = send(&mut fx, "three");
    fx.conn
        .execute_batch(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('u','34750f98bd59fcfc946da45aaabe933b','other','goal',0,0);
             INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES ('u','b',1,'invited','invited');
             INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv1','u','b',1,'pending',1,0,300,99999999);",
        )
        .unwrap();
    let listed = page(&fx, Some(&stranger));
    assert!(listed.contains(&Row::Invitation("u".into())), "{listed:?}");
    assert!(
        listed
            .iter()
            .any(|r| matches!(r, Row::Msg(id, _) if id == m3.as_str())),
        "{listed:?}"
    );
    assert!(
        !listed
            .iter()
            .any(|r| matches!(r, Row::Msg(id, _) if id == m1.as_str())),
        "{listed:?}"
    );
}

#[test]
fn join_gives_every_open_binding_a_cursor_at_the_join_point() {
    let mut fx = fx("bi-join");
    send(&mut fx, "before the join");
    let head: u64 = fx
        .conn
        .query_row(
            "SELECT max(sequence) FROM messages WHERE thread_id='t'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap() as u64;
    // Seat `c` has two sessions and joins thread `t` through the real join path.
    insert_binding(
        &fx.conn,
        "c",
        "nc1",
        "00000000-0000-4000-8000-0000000000c1",
        None,
    );
    insert_binding(
        &fx.conn,
        "c",
        "nc2",
        "00000000-0000-4000-8000-0000000000c2",
        None,
    );
    let c1 = claim("c", "nc1", "00000000-0000-4000-8000-0000000000c1", None);
    let c2 = claim("c", "nc2", "00000000-0000-4000-8000-0000000000c2", None);
    assert_eq!(cursor(&fx, &c1), None, "not a member yet: no cursor");
    let mutation = PermitMutation::Join(Join {
        thread: ThreadId::new("t"),
        operation: OperationId::new("join-c"),
        claim: c1.clone(),
    });
    let permit = store(&fx)
        .issue_cooperative_permit(
            crate::store::cooperative_permit_request(&mutation).unwrap(),
            &budget(),
        )
        .unwrap();
    store(&fx).mutate(mutation, permit, &budget()).unwrap();
    // Both open bindings, the one that joined and the other session, start at the thread head
    // as it stood when the seat joined (the join event itself comes after).
    assert!(head >= 1);
    assert_eq!(cursor(&fx, &c1), Some(head));
    assert_eq!(cursor(&fx, &c2), Some(head));
    // Leaving keeps the rows (cursor rows are never deleted).
    fx.conn
        .execute("UPDATE memberships SET state='left' WHERE seat_id='c'", [])
        .unwrap();
    assert_eq!(cursor(&fx, &c1), Some(head));
}

/// Spec §2: every binding sees its own full lazy stream; one binding's display never hides a lazy
/// message from another, and a subagent keeps its own cursor.
#[test]
fn bound_lazy_stream_is_per_binding() {
    let mut fx = fx("bi-lazy-per-binding");
    let parent = binding_of(&fx, &claim_b1());
    insert_binding(
        &fx.conn,
        "b",
        "nb1",
        EXEC_B1,
        Some((parent.as_str(), "agent-1")),
    );
    let sub = claim("b", "nb1", EXEC_B1, Some("agent-1"));
    let m = send_lazy(&mut fx, "passive");
    let shows = |fx: &Fx, c: &CallerClaim| {
        page(fx, Some(c))
            .iter()
            .any(|row| matches!(row, Row::Lazy(id) if id == m.as_str()))
    };
    assert!(shows(&fx, &claim_b1()), "nb1 displays the lazy message");
    advance(&fx, &claim_b1(), "lazy-adv-1", &[&m]);
    assert!(
        !shows(&fx, &claim_b1()),
        "nb1 is not shown it again while nb2 is still unread"
    );
    assert!(shows(&fx, &claim_b2()), "nb2 still sees it");
    advance(&fx, &claim_b2(), "lazy-adv-2", &[&m]);
    assert!(shows(&fx, &sub), "the subagent's own cursor is short of it");
    advance(&fx, &sub, "lazy-adv-sub", &[&m]);
    assert!(!shows(&fx, &sub), "the subagent's cursor passed it");
}

/// A binding enrolled after its seat left a thread owns no cursor row for it; a missing row must
/// not read as "everything unread".
#[test]
fn binding_enrolled_after_leave_gets_no_old_lazy_history() {
    let mut fx = fx("bi-lazy-left");
    let m = send_lazy(&mut fx, "old");
    fx.conn
        .execute(
            "UPDATE memberships SET state='left' WHERE thread_id='t' AND seat_id='b'",
            [],
        )
        .unwrap();
    insert_binding(&fx.conn, "b", "nb3", EXEC_B3, None);
    let nb3 = claim("b", "nb3", EXEC_B3, None);
    assert!(
        !page(&fx, Some(&nb3))
            .iter()
            .any(|row| matches!(row, Row::Lazy(id) if id == m.as_str())),
        "a left thread's lazy history is not unread for a new binding"
    );
    // The seat form is "some connected top-level binding finds it unread": nb1 and nb2 keep
    // their cursors from before the leave, so the addressed message stays theirs.
    assert_eq!(
        crate::store::lazy_delivery::seat_lazy_pending(&fx.conn, &SeatId::new("b"), 100)
            .unwrap()
            .0,
        1
    );
    assert!(
        page(&fx, Some(&claim_b1()))
            .iter()
            .any(|row| matches!(row, Row::Lazy(id) if id == m.as_str())),
        "leaving does not erase already addressed delivery"
    );
    assert!(
        !crate::store::lazy_delivery::binding_has_unread_lazy(&fx.conn, &binding_of(&fx, &nb3))
            .unwrap()
    );
}

/// The warning refs of seat b's bound v2 page for `claim`.
fn warning_refs(fx: &Fx, claim: &CallerClaim) -> Vec<String> {
    let CommandResult::InboxBatchV2(page) = query_with_output(
        &fx.context,
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            thread: None,
            claim: Some(claim.clone()),
            seat: Some(SeatId::new("b")),
            page: PageRequest {
                limit: 50,
                max_bytes: 60_000,
                cursor: None,
            },
        }),
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap() else {
        panic!("v2 page")
    };
    page.items
        .into_iter()
        .filter_map(|item| match item {
            InboxBatchV2Item::Warning { warning, .. } => Some(warning.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

// Main review F2 follow-up. Kills: a bound read whose warning cutoff is the seat's newest
// top-level binding's `warning_offer` (nb2) instead of the reader's own (nb1 was already offered
// the warning; nb2 was not), and a subagent cutoff that ignores its parent's non-zero frontier.
#[test]
fn bound_warning_cutoff_is_the_reading_bindings_own_offer_frontier() {
    let fx = fx("bi-warning-cutoff");
    let nb1 = binding_of(&fx, &claim_b1());
    let nb2 = binding_of(&fx, &claim_b2());
    insert_binding(&fx.conn, "b", "nb1", EXEC_B1, Some((nb1.as_str(), "ag1")));
    insert_binding(&fx.conn, "b", "nb2", EXEC_B2, Some((nb2.as_str(), "ag2")));
    let decision: i64 = fx
        .conn
        .query_row(
            "SELECT decision_seq+1 FROM local_node WHERE id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    fx.conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES('inv','t','b',2,'pending',0,?1,300,300)", [decision]).unwrap();
    let warned = decision + 1;
    fx.conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_json,decision_seq,decision_at) VALUES('34750f98bd59fcfc946da45aaabe933b','warning','t',100,'warn','{}',?1,0)", [warned]).unwrap();
    fx.conn.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('warning',?1,'t',100,'b','invitation','inv')", [warned]).unwrap();
    fx.conn
        .execute(
            "UPDATE local_node SET decision_seq=?1 WHERE id='34750f98bd59fcfc946da45aaabe933b'",
            [warned],
        )
        .unwrap();
    // nb1 was offered the warning; nb2 (the seat view: the newest top-level binding) was not.
    for (binding, through) in [(&nb1, warned), (&nb2, warned - 1)] {
        fx.conn
            .execute(
                "INSERT INTO warning_offer(binding_id,seat_id,offered_through_seq) VALUES (?1,'b',?2)",
                rusqlite::params![binding.as_str(), through],
            )
            .unwrap();
    }
    assert_eq!(
        warning_refs(&fx, &claim_b2()),
        ["warning"],
        "nb2 still owes it"
    );
    assert!(
        warning_refs(&fx, &claim_b1()).is_empty(),
        "nb1 was already offered it"
    );
    // A subagent without its own row reads its parent's frontier, not an empty one.
    let sub1 = claim("b", "nb1", EXEC_B1, Some("ag1"));
    let sub2 = claim("b", "nb2", EXEC_B2, Some("ag2"));
    assert!(
        warning_refs(&fx, &sub1).is_empty(),
        "nb1's frontier covers it"
    );
    assert_eq!(warning_refs(&fx, &sub2), ["warning"]);
}

// ht-br0: a thread-scoped page (`InboxQuery.thread`) lists only the reader's unread lazy rows of
// that thread: no lazy row of another thread, no ordinary message, receipt or invitation. The
// unscoped page is unchanged. A scoped request never resumes a continuation cursor.
#[test]
fn thread_scoped_page_lists_only_that_threads_lazy_rows() {
    let mut fx = fx("binding-inbox-thread-scope");
    fx.conn
        .execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','34750f98bd59fcfc946da45aaabe933b','topic2','goal',0,0)",
            [],
        )
        .unwrap();
    for seat in ["a", "b"] {
        fx.conn
            .execute(
                "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t2',?1,'joined')",
                [seat],
            )
            .unwrap();
        fx.conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t2',?1,1,1)", [seat]).unwrap();
    }
    use crate::protocol::commands::DeliveryMode::{Lazy, Ordinary};
    let lazy_t = send_lazy(&mut fx, "lazy in t");
    // The lazy row precedes the ordinary one: the binding-scoped page stops at the first unread
    // ordinary row (`thread_scoped_page_stops_at_the_first_unread_ordinary_row`).
    let lazy_t2 = send_in(&mut fx, "t2", "lazy in t2", Lazy);
    let ordinary_t2 = send_in(&mut fx, "t2", "ordinary in t2", Ordinary);
    let lazy = |id: &MessageId| Row::Lazy(id.as_str().to_owned());
    let b1 = claim_b1();
    assert_eq!(page_in(&fx, Some(&b1), Some("t2")), [lazy(&lazy_t2)]);
    assert_eq!(
        page_in(&fx, None, Some("t2")),
        [lazy(&lazy_t2)],
        "seat form"
    );
    assert_eq!(page_in(&fx, Some(&b1), Some("t")), [lazy(&lazy_t)]);
    let all = page(&fx, Some(&b1));
    for row in [lazy(&lazy_t), lazy(&lazy_t2), msg(&ordinary_t2, true)] {
        assert!(all.contains(&row), "{row:?} in {all:?}");
    }
    let refused = query_with_output(
        &fx.context,
        "34750f98bd59fcfc946da45aaabe933b",
        &Command::InboxBatchV2(InboxQuery {
            thread: Some(ThreadId::new("t2")),
            claim: Some(b1),
            seat: Some(SeatId::new("b")),
            page: PageRequest {
                limit: 50,
                max_bytes: 60_000,
                cursor: Some("v2:anything".into()),
            },
        }),
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidRequest, "{refused:?}");
}

// ht-br0: the binding-scoped thread page stops at the binding's first unread ordinary row (the
// display walk's stop): a lazy row above it could be shown but not passed, so the scoped page
// leaves it to the binding's unscoped stream. The seat form is unchanged.
#[test]
fn thread_scoped_page_stops_at_the_first_unread_ordinary_row() {
    let mut fx = fx("binding-inbox-thread-scope-stop");
    use crate::protocol::commands::DeliveryMode::Ordinary;
    let before = send_lazy(&mut fx, "lazy before");
    let ordinary = send_in(&mut fx, "t", "ordinary between", Ordinary);
    let after = send_lazy(&mut fx, "lazy after");
    let lazy = |id: &MessageId| Row::Lazy(id.as_str().to_owned());
    let b1 = claim_b1();
    assert_eq!(page_in(&fx, Some(&b1), Some("t")), [lazy(&before)]);
    let all = page(&fx, Some(&b1));
    for row in [lazy(&before), msg(&ordinary, true), lazy(&after)] {
        assert!(all.contains(&row), "{row:?} in {all:?}");
    }
}
