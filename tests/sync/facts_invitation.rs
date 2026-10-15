//! `invitation.created` and `invitation.answered` (fact-kinds design section 6;
//! ht-5fr.4.10): payloads, keys, idempotence, the `already_joined` and
//! `recipient_retired` settlements, the service and operator inviter kinds and
//! origin authority. The multi-store convergence cases are in
//! `fact_convergence_invitation`.
//!
//! The helpers at the top (authoring an invitation fact on a [`Node`], reading
//! its rows) are shared with that file.

use herdr_threads::node::tag_for;
use herdr_threads::protocol::ids::{InvitationId, SeatId, ThreadId, generate_public_id, prefix};
use herdr_threads::protocol::results::ApiError;
use herdr_threads::store::journal;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::invitation::{
    InvitationAnswered, InvitationAnsweredPayload, InvitationCreated, InvitationCreatedPayload,
};
use herdr_threads::sync::facts::payload::{
    AnswerOutcome, Confirmation, InviterKind, ReferencedSeats,
};
use herdr_threads::sync::facts::{FactKind, FactRegistry};

use crate::facts_membership::thread_of;
use crate::facts_seat::{Node, registry};
use crate::facts_thread::sync_all;

// ---- shared fixtures --------------------------------------------------------

/// The `created_at` of every invitation these tests author (the test clock's).
pub const CREATED_AT: i64 = 1_700_000_000_000;
/// The frozen duration of those invitations.
pub const DURATION_MS: i64 = 60_000;
/// An `answered_at` before every deadline above.
pub const ANSWERED_AT: i64 = CREATED_AT + 500;

/// One `invitations` row, as the projection reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationRow {
    pub state: String,
    pub thread: String,
    pub seat: String,
    pub inviter: Option<String>,
    pub kind: String,
    pub origin: String,
    pub invitee_n: i64,
    pub deadline_at: i64,
    pub episode: i64,
    pub accepted_at: Option<i64>,
    pub retired_at: Option<i64>,
}

impl Node {
    /// An invitation ID minted under this node's tag.
    pub fn mint_invitation(&self) -> InvitationId {
        InvitationId::new(generate_public_id(prefix::INVITATION, &tag_for(self.id, 0)))
    }

    /// A timed `require_ack` seat invitation as this node's view would author
    /// it: `invitee_n` is the ordinal this store holds for the invitee.
    pub fn created_payload(
        &self,
        thread: &ThreadId,
        inviter: &SeatId,
        invitee: &SeatId,
    ) -> InvitationCreatedPayload {
        let invitee_n = self
            .membership(thread, invitee)
            .map_or(0, |(_, home_transition, _)| home_transition);
        InvitationCreatedPayload {
            invitation: self.mint_invitation(),
            thread: thread.clone(),
            inviter: Some(inviter.clone()),
            inviter_kind: InviterKind::Seat,
            invitee: invitee.clone(),
            invitee_n: u64::try_from(invitee_n).unwrap(),
            created_at: CREATED_AT,
            duration_ms: DURATION_MS,
            deadline_at: Some(CREATED_AT + DURATION_MS),
            confirmation: Some(Confirmation::RequireAck),
            actor_label: None,
        }
    }

    pub fn author_created(&self, payload: &InvitationCreatedPayload) -> Result<(), ApiError> {
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<InvitationCreated>(tx, self.ctx(), d, payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    /// Authors a seat invitation of `invitee` by `inviter`.
    pub fn invite_fact(
        &self,
        thread: &ThreadId,
        inviter: &SeatId,
        invitee: &SeatId,
    ) -> InvitationId {
        let payload = self.created_payload(thread, inviter, invitee);
        self.author_created(&payload).unwrap();
        payload.invitation
    }

    pub fn author_answered(&self, payload: &InvitationAnsweredPayload) -> Result<(), ApiError> {
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<InvitationAnswered>(tx, self.ctx(), d, payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    /// The accept of `invitation` by `invitee`, with the invitee's next ordinal.
    pub fn accept_payload(
        &self,
        thread: &ThreadId,
        invitation: &InvitationId,
        invitee: &SeatId,
    ) -> InvitationAnsweredPayload {
        let home_transition = self
            .membership(thread, invitee)
            .map_or(0, |(_, home_transition, _)| home_transition);
        InvitationAnsweredPayload {
            invitation: invitation.clone(),
            invitee: invitee.clone(),
            outcome: AnswerOutcome::Accepted,
            reason: None,
            answered_at: ANSWERED_AT,
            n: u64::try_from(home_transition).unwrap() + 1,
        }
    }

    pub fn reject_payload(
        &self,
        invitation: &InvitationId,
        invitee: &SeatId,
    ) -> InvitationAnsweredPayload {
        InvitationAnsweredPayload {
            invitation: invitation.clone(),
            invitee: invitee.clone(),
            outcome: AnswerOutcome::Rejected,
            reason: Some("not now".into()),
            answered_at: ANSWERED_AT,
            n: 0,
        }
    }

    pub fn invitation(&self, id: &InvitationId) -> Option<InvitationRow> {
        self.query(
            &format!(
                "SELECT state,thread_id,seat_id,inviter_seat_id,inviter_kind,origin_node,invitee_n,\
                 deadline_at,episode,accepted_at,retired_at FROM invitations WHERE id='{}'",
                id.as_str()
            ),
            |r| {
                Ok(InvitationRow {
                    state: r.get(0)?,
                    thread: r.get(1)?,
                    seat: r.get(2)?,
                    inviter: r.get(3)?,
                    kind: r.get(4)?,
                    origin: r.get(5)?,
                    invitee_n: r.get(6)?,
                    deadline_at: r.get(7)?,
                    episode: r.get(8)?,
                    accepted_at: r.get(9)?,
                    retired_at: r.get(10)?,
                })
            },
        )
        .into_iter()
        .next()
    }

    pub fn state_of(&self, id: &InvitationId) -> Option<String> {
        self.invitation(id).map(|row| row.state)
    }

    /// The event keys of the invitation family in a thread, oldest first.
    pub fn invitation_events(&self, thread: &ThreadId) -> Vec<String> {
        self.event_keys(thread)
            .into_iter()
            .filter(|k| {
                [
                    "invite:",
                    "accept:",
                    "reject:",
                    "operator_invite:",
                    "service_invite:",
                    "overdue:",
                ]
                .iter()
                .any(|prefix| k.starts_with(prefix))
            })
            .collect()
    }

    /// `(reason_bits, attention_version)` of a seat's attention row.
    pub fn attention(&self, seat: &SeatId) -> Option<(i64, i64)> {
        self.query(
            &format!(
                "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id='{}'",
                seat.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .into_iter()
        .next()
    }

    /// The rejection reason recorded for an invitation.
    pub fn rejection(&self, id: &InvitationId) -> Option<(String, String, i64)> {
        self.query(
            &format!(
                "SELECT reason,actor_seat_id,rejected_at FROM invitation_rejections WHERE invitation_id='{}'",
                id.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .into_iter()
        .next()
    }

    /// The number of overdue warnings on the invitation family of a thread.
    pub fn overdue_warnings(&self) -> i64 {
        self.query(
            "SELECT count(*) FROM messages WHERE kind='warn' AND event_key LIKE 'overdue:invitation:%'",
            |r| r.get(0),
        )[0]
    }
}

/// A thread of `node`'s creator, `node`'s second local seat and that creator.
fn local_pair(node: &Node) -> (ThreadId, SeatId, SeatId) {
    let (thread, creator) = thread_of(node, "t");
    node.joined(&thread, &creator, 1).unwrap();
    let guest = node.local_seat();
    (thread, creator, guest)
}

// ---- payloads, keys, registration ------------------------------------------

#[test]
fn round_trips_keys() {
    let payload = InvitationCreatedPayload {
        invitation: InvitationId::new("iAbc12345678"),
        thread: ThreadId::new("tAbc12345678"),
        inviter: Some(SeatId::new("sInviter0001")),
        inviter_kind: InviterKind::Seat,
        invitee: SeatId::new("sInvitee0001"),
        invitee_n: 2,
        created_at: 1_700_000_000_000,
        duration_ms: 60_000,
        deadline_at: Some(1_700_000_060_000),
        confirmation: Some(Confirmation::RequireAck),
        actor_label: None,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        r#"{"invitation":"iAbc12345678","thread":"tAbc12345678","inviter":"sInviter0001","inviter_kind":"seat","invitee":"sInvitee0001","invitee_n":2,"created_at":1700000000000,"duration_ms":60000,"deadline_at":1700000060000,"confirmation":"require_ack","actor_label":null}"#
    );
    assert_eq!(
        serde_json::from_str::<InvitationCreatedPayload>(&json).unwrap(),
        payload
    );
    // An additive field is ignored; a missing kind is a seat invitation.
    let newer: InvitationCreatedPayload = serde_json::from_str(
        r#"{"invitation":"iAbc12345678","thread":"tAbc12345678","inviter":"sInviter0001","invitee":"sInvitee0001","created_at":1,"duration_ms":5,"future":{"x":1}}"#,
    )
    .unwrap();
    assert_eq!(newer.inviter_kind, InviterKind::Seat);
    assert_eq!(
        (newer.invitee_n, newer.deadline_at, newer.confirmation),
        (0, None, None)
    );

    assert_eq!(InvitationCreated::NAME, "invitation.created");
    assert_eq!(
        InvitationCreated::creates(&payload),
        Some(EntityKey::new("iAbc12345678").unwrap())
    );
    let mut needs = InvitationCreated::needs(&payload);
    needs.sort();
    let mut expected: Vec<_> = ["tAbc12345678", "sInviter0001", "sInvitee0001"]
        .into_iter()
        .map(|k| EntityKey::new(k).unwrap())
        .collect();
    expected.sort();
    assert_eq!(needs, expected, "thread, inviter and invitee");
    assert_eq!(
        payload.referenced_seats(),
        vec![SeatId::new("sInviter0001"), SeatId::new("sInvitee0001")]
    );
    // A service invitation has no inviter seat to depend on or publish.
    let service = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Service,
        ..payload.clone()
    };
    assert_eq!(InvitationCreated::needs(&service).len(), 2);
    assert_eq!(
        service.referenced_seats(),
        vec![SeatId::new("sInvitee0001")]
    );

    let answer = InvitationAnsweredPayload {
        invitation: InvitationId::new("iAbc12345678"),
        invitee: SeatId::new("sInvitee0001"),
        outcome: AnswerOutcome::Accepted,
        reason: None,
        answered_at: 1_700_000_000_500,
        n: 3,
    };
    let json = serde_json::to_string(&answer).unwrap();
    assert_eq!(
        json,
        r#"{"invitation":"iAbc12345678","invitee":"sInvitee0001","outcome":"accepted","reason":null,"answered_at":1700000000500,"n":3}"#
    );
    assert_eq!(
        serde_json::from_str::<InvitationAnsweredPayload>(&json).unwrap(),
        answer
    );
    assert_eq!(InvitationAnswered::NAME, "invitation.answered");
    assert_eq!(
        InvitationAnswered::creates(&answer),
        Some(EntityKey::new("iAbc12345678/answer").unwrap()),
        "the answer's own key"
    );
    assert_eq!(
        InvitationAnswered::needs(&answer),
        vec![EntityKey::new("iAbc12345678").unwrap()]
    );
    assert_eq!(answer.referenced_seats(), vec![SeatId::new("sInvitee0001")]);

    let builtins = FactRegistry::builtins();
    for kind in ["invitation.created", "invitation.answered"] {
        assert!(builtins.lookup(kind, 1).is_some(), "{kind} registered");
    }
}

// ---- idempotence -------------------------------------------------------------

#[test]
fn apply_twice_noop() {
    let node = Node::with(140, registry());
    let (thread, creator, guest) = local_pair(&node);
    let payload = node.created_payload(&thread, &creator, &guest);
    node.author_created(&payload).unwrap();
    let row = node.invitation(&payload.invitation).unwrap();
    assert_eq!(row.state, "pending");
    assert_eq!(
        (row.inviter.as_deref(), row.kind.as_str(), row.invitee_n),
        (Some(creator.as_str()), "seat", 0)
    );
    assert_eq!(row.origin, node.id.to_string());
    assert_eq!(row.deadline_at, CREATED_AT + DURATION_MS);
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("invited".into(), 0, 0))
    );
    assert_eq!(
        node.invitation_events(&thread),
        vec![format!("invite:{}", payload.invitation.as_str())]
    );
    let attention = node.attention(&guest);
    assert!(attention.is_some(), "a local timed invitee is woken once");

    let before = (
        node.membership_projection(),
        node.invitation_events(&thread),
        node.attention(&guest),
    );
    node.author_created(&payload).unwrap();
    assert_eq!(node.count_kind("invitation.created"), 2);
    assert!(
        node.journal()
            .iter()
            .filter(|(k, _, _)| k == "invitation.created")
            .all(|(_, state, _)| state == "applied")
    );
    assert_eq!(
        (
            node.membership_projection(),
            node.invitation_events(&thread),
            node.attention(&guest)
        ),
        before,
        "no second row, event or attention write"
    );

    // The accept, twice.
    let accept = node.accept_payload(&thread, &payload.invitation, &guest);
    node.author_answered(&accept).unwrap();
    let accepted = node.invitation(&payload.invitation).unwrap();
    assert_eq!(accepted.state, "accepted");
    assert_eq!(accepted.accepted_at, Some(ANSWERED_AT));
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("joined".into(), 1, 1)),
        "the answer's ordinal is the seat's"
    );
    assert_eq!(
        accepted.episode, 1,
        "the membership opens the invitation's episode"
    );
    let after_first = (
        node.membership_projection(),
        node.invitation_events(&thread),
        node.intervals(&thread, &guest),
        node.attention(&guest),
    );
    assert_eq!(after_first.2.len(), 1);
    assert!(
        after_first
            .1
            .contains(&format!("accept:{}", payload.invitation.as_str()))
    );
    node.author_answered(&accept).unwrap();
    assert_eq!(node.count_kind("invitation.answered"), 2);
    assert_eq!(
        (
            node.membership_projection(),
            node.invitation_events(&thread),
            node.intervals(&thread, &guest),
            node.attention(&guest)
        ),
        after_first,
        "no second event, interval or attention write"
    );
}

#[test]
fn reauthored_each_kind_noop() {
    let node = Node::with(141, registry());
    let (thread, creator, guest) = local_pair(&node);
    let payload = node.created_payload(&thread, &creator, &guest);
    node.author_created(&payload).unwrap();
    let reject = node.reject_payload(&payload.invitation, &guest);
    node.author_answered(&reject).unwrap();
    assert_eq!(node.rejection(&payload.invitation).unwrap().0, "not now");
    let before = (
        node.membership_projection(),
        node.invitation_events(&thread),
        node.attention(&guest),
        node.rejection(&payload.invitation),
    );
    // A re-authored copy of each kind under a second incarnation of this node.
    node.run(|tx, d| journal::begin_incarnation(tx, node.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    node.run(|tx, d| journal::author::<InvitationCreated>(tx, node.ctx(), d, &payload).map(|_| ()))
        .unwrap();
    node.run(|tx, d| journal::author::<InvitationAnswered>(tx, node.ctx(), d, &reject).map(|_| ()))
        .unwrap();
    for kind in ["invitation.created", "invitation.answered"] {
        assert_eq!(node.count_kind(kind), 2, "{kind}");
    }
    assert!(
        node.journal()
            .iter()
            .filter(|(k, _, _)| k.starts_with("invitation."))
            .all(|(_, state, _)| state == "applied")
    );
    assert_eq!(
        (
            node.membership_projection(),
            node.invitation_events(&thread),
            node.attention(&guest),
            node.rejection(&payload.invitation)
        ),
        before,
        "no second event or attention write"
    );
}

// ---- the settlements ---------------------------------------------------------

#[test]
fn already_joined_wakes_nobody_no_overdue() {
    let node = Node::with(142, registry());
    let (thread, creator, guest) = local_pair(&node);
    // The invitee joined (n = 1) after the inviter's view: invitee_n = 0.
    let stale = node.created_payload(&thread, &creator, &guest);
    node.joined(&thread, &guest, 1).unwrap();
    node.author_created(&stale).unwrap();
    let row = node.invitation(&stale.invitation).unwrap();
    assert_eq!(row.state, "already_joined");
    assert_eq!(
        row.deadline_at,
        CREATED_AT + DURATION_MS,
        "deadline kept as stored"
    );
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("joined".into(), 1, 1)),
        "membership untouched"
    );
    assert!(
        node.invitation_events(&thread).is_empty(),
        "no invite event"
    );
    assert!(node.attention(&guest).is_none(), "nobody woken");

    // The other order: pending first (the invitee is woken for it), then the
    // join settles it without another wake and without an overdue warning.
    let other = node.local_seat();
    let pending = node.created_payload(&thread, &creator, &other);
    node.author_created(&pending).unwrap();
    let woken = node.attention(&other);
    assert!(woken.is_some());
    node.joined(&thread, &other, 1).unwrap();
    assert_eq!(
        node.state_of(&pending.invitation).as_deref(),
        Some("already_joined")
    );
    assert_eq!(node.attention(&other), woken, "the settlement is no wake");

    // Long past both deadlines, still no overdue warning for either.
    node.clock.advance(10 * DURATION_MS as u64);
    assert_eq!(node.overdue_warnings(), 0);
}

#[test]
fn invitation_that_saw_the_join_stays_pending() {
    let node = Node::with(149, registry());
    let (thread, creator, guest) = local_pair(&node);
    node.joined(&thread, &guest, 1).unwrap();
    node.left(&thread, &guest, 2).unwrap();
    // The inviter saw the join and the leave (invitee_n = 2): pending.
    let saw_leave = node.created_payload(&thread, &creator, &guest);
    assert_eq!(saw_leave.invitee_n, 2);
    node.author_created(&saw_leave).unwrap();
    assert_eq!(
        node.state_of(&saw_leave.invitation).as_deref(),
        Some("pending")
    );
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("invited".into(), 2, 1)),
        "the ordinals are the seat's own"
    );
    // The boundary: an inviter that saw only the join (invitee_n equals
    // last_joined_n) is not superseded by it; one that saw nothing (0) is.
    let saw_join = InvitationCreatedPayload {
        invitation: node.mint_invitation(),
        invitee_n: 1,
        ..saw_leave.clone()
    };
    node.author_created(&saw_join).unwrap();
    assert_eq!(
        node.state_of(&saw_join.invitation).as_deref(),
        Some("pending"),
        "last_joined_n above invitee_n is the rule, never equal"
    );
    let saw_nothing = InvitationCreatedPayload {
        invitation: node.mint_invitation(),
        invitee_n: 0,
        ..saw_leave
    };
    node.author_created(&saw_nothing).unwrap();
    assert_eq!(
        node.state_of(&saw_nothing.invitation).as_deref(),
        Some("already_joined")
    );
}

#[test]
fn retired_invitee_settles_recipient_retired_at_apply() {
    let node = Node::with(143, registry());
    let (thread, creator, guest) = local_pair(&node);
    node.publish(&guest).unwrap();
    node.raw()
        .execute(
            "UPDATE seats SET state='retired',retired_at=5,retired_seq=1 WHERE id=?1",
            [guest.as_str()],
        )
        .unwrap();
    let payload = node.created_payload(&thread, &creator, &guest);
    node.author_created(&payload).unwrap();
    let row = node.invitation(&payload.invitation).unwrap();
    assert_eq!(row.state, "recipient_retired");
    assert_eq!(row.retired_at, Some(5), "the seat's own retirement time");
    // The terminal row the retirement job leaves on a node that applied the
    // invitation first (order permutation, fact_order::invite_vs_seat_retired).
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("retired".into(), 0, 0)),
        "a retired membership row, no invited one"
    );
    assert!(node.invitation_events(&thread).is_empty());
    assert!(node.attention(&guest).is_none());
}

#[test]
fn accept_settles_the_invitees_other_pending_invitations() {
    let node = Node::with(144, registry());
    let (thread, creator, guest) = local_pair(&node);
    let first = node.invite_fact(&thread, &creator, &guest);
    let second = node.created_payload(&thread, &creator, &guest);
    // Both were made against the same view (invitee_n = 0).
    node.author_created(&second).unwrap();
    assert_eq!(node.invitation(&second.invitation).unwrap().episode, 2);
    node.author_answered(&node.accept_payload(&thread, &first, &guest))
        .unwrap();
    assert_eq!(node.state_of(&first).as_deref(), Some("accepted"));
    assert_eq!(
        node.state_of(&second.invitation).as_deref(),
        Some("already_joined"),
        "an accept settles the seat's other pending invitations"
    );
    assert_eq!(
        node.membership(&thread, &guest),
        Some(("joined".into(), 1, 1))
    );
}

#[test]
fn reject_records_reason_warn_event_and_keeps_pending_row() {
    let node = Node::with(145, registry());
    let (thread, creator, guest) = local_pair(&node);
    let id = node.invite_fact(&thread, &creator, &guest);
    node.author_answered(&node.reject_payload(&id, &guest))
        .unwrap();
    assert_eq!(
        node.rejection(&id),
        Some(("not now".into(), guest.as_str().into(), ANSWERED_AT))
    );
    assert!(
        node.invitation_events(&thread)
            .contains(&format!("reject:{}", id.as_str()))
    );
    // The invitation row stays pending; the rejection row settles it. A later
    // accept is a no-op.
    assert_eq!(node.state_of(&id).as_deref(), Some("pending"));
    node.author_answered(&node.accept_payload(&thread, &id, &guest))
        .unwrap();
    assert_eq!(node.state_of(&id).as_deref(), Some("pending"));
    assert_ne!(
        node.membership(&thread, &guest).map(|(state, _, _)| state),
        Some("joined".into())
    );
}

// ---- service and operator inviters ------------------------------------------

#[test]
fn service_and_operator_kinds_apply_on_the_signing_node_without_a_seat() {
    let node = Node::with(146, registry());
    let (thread, _creator, guest) = local_pair(&node);
    let service = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Service,
        confirmation: None,
        actor_label: Some("svc-author".into()),
        ..node.created_payload(&thread, &SeatId::new("unused"), &guest)
    };
    node.author_created(&service).unwrap();
    let row = node.invitation(&service.invitation).unwrap();
    assert_eq!(
        (row.state.as_str(), row.inviter, row.kind.as_str()),
        ("pending", None, "service")
    );
    let operator = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Operator,
        confirmation: None,
        actor_label: Some("operator:local-user:501".into()),
        invitation: node.mint_invitation(),
        ..service.clone()
    };
    node.author_created(&operator).unwrap();
    let row = node.invitation(&operator.invitation).unwrap();
    assert_eq!((row.inviter, row.kind.as_str()), (None, "operator"));
    assert_eq!(
        node.invitation_events(&thread),
        vec![
            format!("service_invite:{}:ordinary", service.invitation.as_str()),
            format!("operator_invite:{}", operator.invitation.as_str())
        ],
        "each kind keeps its event shape"
    );

    // A seat invitation must name its inviter.
    let nameless = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Seat,
        invitation: node.mint_invitation(),
        ..service
    };
    node.author_created(&nameless).unwrap_err();
    assert!(node.invitation(&nameless.invitation).is_none());
}

#[test]
fn service_kind_from_another_node_applies_without_a_service_author() {
    let reg = registry();
    let (a, b) = (Node::with(147, reg), Node::with(148, reg));
    let (thread, _creator, _guest) = local_pair(&a);
    let seat = b.local_seat();
    b.publish(&seat).unwrap();
    sync_all(&[&a, &b]);
    // A's service invites B's seat.
    let payload = InvitationCreatedPayload {
        inviter: None,
        inviter_kind: InviterKind::Service,
        confirmation: None,
        actor_label: Some("svc-author".into()),
        ..a.created_payload(&thread, &SeatId::new("unused"), &seat)
    };
    a.author_created(&payload).unwrap();
    sync_all(&[&a, &b]);
    for node in [&a, &b] {
        let row = node.invitation(&payload.invitation).unwrap();
        assert_eq!(row.state, "pending");
        assert_eq!(row.kind, "service");
        assert_eq!(row.origin, a.id.to_string(), "the signing node");
    }
    assert_eq!(
        b.invitation_events(&thread),
        a.invitation_events(&thread),
        "the same derived event on every node"
    );
}

// ---- origin authority --------------------------------------------------------

/// A, B and C with A's thread and creator and B's seat, published everywhere.
fn three_nodes(seed: u8) -> (Node, Node, Node, ThreadId, SeatId, SeatId) {
    let reg = registry();
    let (a, b, c) = (
        Node::with(seed, reg),
        Node::with(seed + 1, reg),
        Node::with(seed + 2, reg),
    );
    let (thread, creator) = thread_of(&a, "a's");
    a.joined(&thread, &creator, 1).unwrap();
    let invitee = b.local_seat();
    b.publish(&invitee).unwrap();
    sync_all(&[&a, &b, &c]);
    (a, b, c, thread, creator, invitee)
}

#[test]
fn foreign_inviter_created_failed() {
    let (a, b, c, thread, creator, invitee) = three_nodes(150);
    assert!(c.seat(&creator).is_some() && c.seat(&invitee).is_some());
    // C authors an invitation whose inviter is A's seat; C's own apply passes
    // (local origin), every other node rejects it.
    let forged = c.created_payload(&thread, &creator, &invitee);
    c.author_created(&forged).unwrap();
    sync_all(&[&a, &b, &c]);
    for node in [&a, &b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "invitation.created" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        let error = failed[0].2.as_deref().unwrap();
        assert!(
            error.contains("is not the authorized author of invitation.created for")
                && error.contains(creator.as_str()),
            "{error}"
        );
        assert!(
            node.invitation(&forged.invitation).is_none(),
            "no invitation row"
        );
        assert!(
            node.membership(&thread, &invitee).is_none(),
            "no membership row"
        );
        assert!(node.invitation_events(&thread).is_empty(), "no event");
    }
    // The inviter's own node authoring it passes everywhere.
    let real = a.invite_fact(&thread, &creator, &invitee);
    sync_all(&[&a, &b, &c]);
    for node in [&a, &b] {
        assert_eq!(node.state_of(&real).as_deref(), Some("pending"));
    }
}

#[test]
fn answer_by_non_home_failed() {
    let (a, b, c, thread, creator, invitee) = three_nodes(155);
    let id = a.invite_fact(&thread, &creator, &invitee);
    sync_all(&[&a, &b, &c]);
    assert_eq!(c.state_of(&id).as_deref(), Some("pending"));
    // C answers for B's seat.
    c.author_answered(&c.accept_payload(&thread, &id, &invitee))
        .unwrap();
    sync_all(&[&a, &b, &c]);
    for node in [&a, &b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "invitation.answered" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        let error = failed[0].2.as_deref().unwrap();
        assert!(
            error.contains("is not the authorized author of invitation.answered for")
                && error.contains(invitee.as_str()),
            "{error}"
        );
        assert_eq!(node.state_of(&id).as_deref(), Some("pending"));
        assert_eq!(
            node.membership(&thread, &invitee),
            Some(("invited".into(), 0, 0)),
            "membership unchanged"
        );
        assert!(
            !node
                .invitation_events(&thread)
                .contains(&format!("accept:{}", id.as_str()))
        );
    }
    // The invitee's own node answers: accepted everywhere that has not failed it.
    b.author_answered(&b.accept_payload(&thread, &id, &invitee))
        .unwrap();
    sync_all(&[&a, &b]);
    for node in [&a, &b] {
        assert_eq!(node.state_of(&id).as_deref(), Some("accepted"));
        assert_eq!(
            node.membership(&thread, &invitee),
            Some(("joined".into(), 1, 1))
        );
    }
}

#[test]
fn answer_naming_a_different_invitee_refused() {
    let (a, b, _c, thread, creator, invitee) = three_nodes(160);
    let id = a.invite_fact(&thread, &creator, &invitee);
    sync_all(&[&a, &b]);
    // B's own other seat names itself as invitee of an invitation addressed to
    // the first: the handler compares with the invitation row.
    let other = b.local_seat();
    let wrong = b.accept_payload(&thread, &id, &other);
    let before = b.journal().len();
    let error = b.author_answered(&wrong).unwrap_err();
    assert!(
        error
            .detail
            .contains("different invitee than the invitation"),
        "{error:?}"
    );
    assert_eq!(b.journal().len(), before, "nothing was authored");
    assert_eq!(b.state_of(&id).as_deref(), Some("pending"));
}
