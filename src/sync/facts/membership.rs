//! Fact kinds of the membership entity: `membership.joined` and
//! `membership.left` (fact-kinds design section 7).
//!
//! Both carry `{thread, seat, n, at}`. `n >= 1` is the seat's home-node
//! transition ordinal for that thread (`memberships.home_transition + 1` at
//! authoring): a re-issued or duplicate fact has `n <= home_transition` and
//! is a no-op, whichever incarnation or relay delivered it. Only the seat's
//! home node authors them, and every handler checks that first
//! (`AuthorRule::SeatHome`) before calling its one materializer in
//! `store::control` (`src` holds no SQL here).
//!
//! Fork residual (root section 12, accepted): two branches of the seat's home
//! log (a wipe or restore past the own-history hold's cap) can each author a
//! fact with the same `n`. Each node keeps whichever it applied first (the other
//! is `n <= home_transition`, a no-op), and the seat's next transition
//! (`n + 1`) makes the stores agree again.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{ReferencedSeats, seat_key, thread_key};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::{SeatId, ThreadId};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::control::{self, Transition};
use crate::store::materialize::{AuthorRule, require_author};
use crate::sync::EntityKey;

/// The v1 payload of `membership.joined` and `membership.left`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipPayload {
    pub thread: ThreadId,
    pub seat: SeatId,
    /// The seat's transition ordinal for the thread; at least 1.
    pub n: u64,
    /// Origin wall-clock Unix ms of the transition.
    pub at: i64,
    /// The 1:1 pair creator, set only on a pair seat's auto-join (reconcile
    /// section 2.2). Informational: the fact is authored by `seat`'s home node
    /// and never refused for its actor; the derived join event names it as
    /// `actor_seat` only when it is one of the pair's two seats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<SeatId>,
}

impl ReferencedSeats for MembershipPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

impl MembershipPayload {
    /// `Rejected` for an ordinal of 0, which no home node authors.
    fn check_ordinal(&self) -> Result<(), ApiError> {
        if self.n == 0 {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "membership transition ordinal is 0",
            ));
        }
        Ok(())
    }
}

fn needs(payload: &MembershipPayload) -> Vec<EntityKey> {
    [thread_key(&payload.thread), seat_key(&payload.seat)]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
}

fn apply_transition(
    tx: &Transaction<'_>,
    cx: &ApplyContext<'_>,
    kind: &str,
    transition: Transition,
    payload: &MembershipPayload,
) -> Result<(), FactError> {
    with_input(cx, |m| {
        require_author(tx, m, kind, &AuthorRule::SeatHome(payload.seat.clone()))?;
        payload.check_ordinal()?;
        control::materialize_membership(tx, m, transition, payload)
    })
}

/// The `membership.joined` v1 kind: a join or an accepted invitation.
pub struct MembershipJoined;

impl FactKind for MembershipJoined {
    const NAME: &'static str = "membership.joined";
    const MAJOR: u32 = 1;
    type Payload = MembershipPayload;

    fn creates(_: &MembershipPayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &MembershipPayload) -> Vec<EntityKey> {
        needs(payload)
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: MembershipPayload,
    ) -> Result<(), FactError> {
        apply_transition(tx, cx, Self::NAME, Transition::Joined, &payload)
    }
}

/// The `membership.left` v1 kind.
pub struct MembershipLeft;

impl FactKind for MembershipLeft {
    const NAME: &'static str = "membership.left";
    const MAJOR: u32 = 1;
    type Payload = MembershipPayload;

    fn creates(_: &MembershipPayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &MembershipPayload) -> Vec<EntityKey> {
        needs(payload)
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: MembershipPayload,
    ) -> Result<(), FactError> {
        apply_transition(tx, cx, Self::NAME, Transition::Left, &payload)
    }
}
