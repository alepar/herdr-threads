//! Fact kinds of the invitation entity: `invitation.created` and
//! `invitation.answered` (fact-kinds design section 6).
//!
//! - `invitation.created` carries the invitation as the inviter's node decided
//!   it, plus `invitee_n`, the invitee's membership transition ordinal the
//!   inviter saw. Every node materializes it with the same rule
//!   (`control::materialize_invitation`): an invitee that is retired at apply
//!   settles it `recipient_retired`, an invitee whose `last_joined_n` is above
//!   `invitee_n` settles it `already_joined` (the crossed-invite rule), anything
//!   else leaves it `pending` and the invitee `invited`.
//! - `invitation.answered` is the invitee's accept or reject, authored by the
//!   invitee's home node. Its key is `i<id>/answer`, so a node that ingested its
//!   own answer suppresses the invitation's delivery on catch-up.
//!
//! Authority: a seat inviter's invitation is authored by that seat's home node.
//! A service or operator invitation has no seat (`inviter_kind`), so its
//! authority is the signing origin node. An answer is authored by the invitee's
//! home node, which the handler reads from the invitation row.
//!
//! Each handler checks authority first, then calls its one materializer in
//! `store::control` (`src` holds no SQL here).
//!
//! Spec refinement (recorded in the ht-5fr.4.10 report): `invitation.answered`
//! names its `invitee` so `FactRoute` can hold the kind for a local seat and
//! publish it, and so the authority check does not depend on a lookup alone; the
//! handler refuses an answer whose `invitee` differs from the invitation row's.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{
    AnswerOutcome, InviterKind, ReferencedSeats, answer_key, invitation_key, seat_key, thread_key,
};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::{InvitationId, SeatId, ThreadId};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::control;
use crate::store::materialize::{AuthorRule, require_author};
use crate::sync::EntityKey;

/// The longest answer reason in bytes.
pub const MAX_REASON_BYTES: usize = 256;

/// The `invitation.created` v1 payload. Times are origin wall-clock Unix ms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationCreatedPayload {
    pub invitation: InvitationId,
    pub thread: ThreadId,
    /// The inviting seat; `None` for a service or operator invitation.
    #[serde(default)]
    pub inviter: Option<SeatId>,
    /// Who issued it; a `seat` kind names `inviter`.
    #[serde(default)]
    pub inviter_kind: InviterKind,
    pub invitee: SeatId,
    /// The invitee's `memberships.home_transition` for the thread in the
    /// inviter's view (0 without a row).
    #[serde(default)]
    pub invitee_n: u64,
    pub created_at: i64,
    pub duration_ms: i64,
    /// Required: `created_at + duration_ms`. Every invitation is confirming
    /// (main D6: priority 0, wakes the invitee, overdue warning to the
    /// inviter); a missing deadline would silently change that on a receiver.
    pub deadline_at: i64,
    /// The issuing service author's ID or the operator's audit label, for the
    /// derived event of a service or operator invitation.
    #[serde(default)]
    pub actor_label: Option<String>,
}

impl ReferencedSeats for InvitationCreatedPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        self.inviter
            .iter()
            .chain(std::iter::once(&self.invitee))
            .cloned()
            .collect()
    }
}

impl InvitationCreatedPayload {
    /// The seat named as inviter, `Rejected` when a seat invitation has none.
    fn checked_inviter(&self) -> Result<Option<&SeatId>, ApiError> {
        match (self.inviter_kind, &self.inviter) {
            (InviterKind::Seat, None) => Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "invitation.created names no inviter seat",
            )),
            (InviterKind::Seat, Some(seat)) => Ok(Some(seat)),
            _ => Ok(None),
        }
    }

    fn check(&self) -> Result<(), ApiError> {
        if self.duration_ms <= 0 {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "invitation duration must be positive",
            ));
        }
        if self.created_at.checked_add(self.duration_ms) != Some(self.deadline_at) {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "invitation deadline is not created_at plus its duration",
            ));
        }
        self.checked_inviter().map(|_| ())
    }
}

/// The `invitation.created` v1 kind.
pub struct InvitationCreated;

impl FactKind for InvitationCreated {
    const NAME: &'static str = "invitation.created";
    const MAJOR: u32 = 1;
    type Payload = InvitationCreatedPayload;

    fn creates(payload: &InvitationCreatedPayload) -> Option<EntityKey> {
        invitation_key(&payload.invitation).ok()
    }

    fn needs(payload: &InvitationCreatedPayload) -> Vec<EntityKey> {
        let inviter = match payload.inviter_kind {
            InviterKind::Seat => payload.inviter.as_ref().map(seat_key),
            _ => None,
        };
        [
            Some(thread_key(&payload.thread)),
            inviter,
            Some(seat_key(&payload.invitee)),
        ]
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: InvitationCreatedPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            payload.check()?;
            let rule = match payload.checked_inviter()? {
                Some(seat) => AuthorRule::SeatHome(seat.clone()),
                None => AuthorRule::Any,
            };
            require_author(tx, m, Self::NAME, &rule)?;
            control::materialize_invitation(tx, m, &payload)
        })
    }
}

/// The `invitation.answered` v1 payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationAnsweredPayload {
    pub invitation: InvitationId,
    /// The invitee (see the module doc).
    pub invitee: SeatId,
    pub outcome: AnswerOutcome,
    /// A rejection's reason, at most [`MAX_REASON_BYTES`].
    #[serde(default)]
    pub reason: Option<String>,
    pub answered_at: i64,
    /// For an accept, the invitee's membership transition ordinal; 0 for a reject.
    #[serde(default)]
    pub n: u64,
}

impl ReferencedSeats for InvitationAnsweredPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.invitee.clone()]
    }
}

impl InvitationAnsweredPayload {
    fn check(&self) -> Result<(), ApiError> {
        if self
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > MAX_REASON_BYTES)
        {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "invitation answer reason is too long",
            ));
        }
        if self.outcome == AnswerOutcome::Accepted && self.n == 0 {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "accepted answer has membership ordinal 0",
            ));
        }
        if self.outcome == AnswerOutcome::Rejected
            && self.reason.as_deref().is_none_or(str::is_empty)
        {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "rejection needs a reason",
            ));
        }
        Ok(())
    }
}

/// The `invitation.answered` v1 kind.
pub struct InvitationAnswered;

impl FactKind for InvitationAnswered {
    const NAME: &'static str = "invitation.answered";
    const MAJOR: u32 = 1;
    type Payload = InvitationAnsweredPayload;

    fn creates(payload: &InvitationAnsweredPayload) -> Option<EntityKey> {
        answer_key(&payload.invitation).ok()
    }

    fn needs(payload: &InvitationAnsweredPayload) -> Vec<EntityKey> {
        invitation_key(&payload.invitation).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: InvitationAnsweredPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            payload.check()?;
            let row = control::invitation_invitee(tx, &payload.invitation)?;
            if row.as_ref() != Some(&payload.invitee) {
                return Err(ApiError::new(
                    ErrorCode::InvalidRequest,
                    "invitation.answered names a different invitee than the invitation",
                ));
            }
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.invitee.clone()),
            )?;
            control::materialize_answer(tx, m, &payload)
        })
    }
}
