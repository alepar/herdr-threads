//! Fact kinds of the thread entity: `thread.created`, `thread.register`,
//! `thread.direct` (fact-kinds design section 5).
//!
//! - `thread.created` establishes an ordinary thread and seeds its topic and
//!   name registers; the creator's membership is a separate fact.
//! - `thread.register` is a last-writer-wins write of `topic`, `name` or
//!   `archived`; any node may author it.
//! - `thread.direct` establishes a 1:1 thread whose ID is a pure function of
//!   its two seats, so two nodes that open the same pair converge on one
//!   thread.
//!
//! Each handler checks the origin's authority first, then calls its one
//! materializer in `store::control` (`src` holds no SQL here).

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{ReferencedSeats, RegisterField, seat_key, thread_key};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::{SeatId, ThreadId, direct_thread_id};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::control;
use crate::store::materialize::{AuthorRule, require_author};
use crate::sync::EntityKey;

/// The `thread.created` v1 payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadCreatedPayload {
    pub thread: ThreadId,
    pub topic: String,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub name: Option<String>,
    pub creator: SeatId,
}

impl ReferencedSeats for ThreadCreatedPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.creator.clone()]
    }
}

/// The `thread.created` v1 kind.
pub struct ThreadCreated;

impl FactKind for ThreadCreated {
    const NAME: &'static str = "thread.created";
    const MAJOR: u32 = 1;
    type Payload = ThreadCreatedPayload;

    fn creates(payload: &ThreadCreatedPayload) -> Option<EntityKey> {
        thread_key(&payload.thread).ok()
    }

    fn needs(payload: &ThreadCreatedPayload) -> Vec<EntityKey> {
        seat_key(&payload.creator).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: ThreadCreatedPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.creator.clone()),
            )?;
            control::materialize_thread_created(
                tx,
                m,
                &control::NewThread {
                    thread: &payload.thread,
                    topic: &payload.topic,
                    goal: &payload.goal,
                    name: payload.name.as_deref(),
                    creator: Some(&payload.creator),
                    managed_owner: None,
                    journaled: true,
                },
            )
        })
    }
}

/// The `thread.register` v1 payload. `archived` values are `"1"` and `"0"`;
/// a `null` name clears it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRegisterPayload {
    pub thread: ThreadId,
    pub field: RegisterField,
    #[serde(default)]
    pub value: Option<String>,
    /// The seat whose command wrote the register, for the derived event's
    /// `actor_seat`. Absent for a lane decision (automatic archival). Optional
    /// and unchecked: informational, like `thread.direct`'s `creator`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<SeatId>,
}

impl ReferencedSeats for ThreadRegisterPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        Vec::new()
    }
}

/// The `thread.register` v1 kind.
pub struct ThreadRegister;

impl FactKind for ThreadRegister {
    const NAME: &'static str = "thread.register";
    const MAJOR: u32 = 1;
    type Payload = ThreadRegisterPayload;

    fn creates(_: &ThreadRegisterPayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &ThreadRegisterPayload) -> Vec<EntityKey> {
        thread_key(&payload.thread).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: ThreadRegisterPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(tx, m, Self::NAME, &AuthorRule::Any)?;
            control::materialize_register(tx, m, &payload)
        })
    }
}

/// The `thread.direct` v1 payload. `thread` must be
/// `direct_thread_id(seat_lo, seat_hi)` and `seat_lo < seat_hi`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDirectPayload {
    pub thread: ThreadId,
    pub seat_lo: SeatId,
    pub seat_hi: SeatId,
    /// The pair member whose command opened the thread, for the derived
    /// `create` event's actor. Optional and unchecked beyond being one of the
    /// two seats: both members' nodes may author the same thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<SeatId>,
}

impl ReferencedSeats for ThreadDirectPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat_lo.clone(), self.seat_hi.clone()]
    }
}

impl ThreadDirectPayload {
    /// `Rejected` unless the ID is the pair's and the seats are ordered.
    fn check_id(&self) -> Result<(), ApiError> {
        let expected = direct_thread_id(&self.seat_lo, &self.seat_hi).ok();
        if self.seat_lo >= self.seat_hi || expected.as_ref() != Some(&self.thread) {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "thread.direct id does not match its seat pair",
            ));
        }
        Ok(())
    }
}

/// The `thread.direct` v1 kind.
pub struct ThreadDirect;

impl FactKind for ThreadDirect {
    const NAME: &'static str = "thread.direct";
    const MAJOR: u32 = 1;
    type Payload = ThreadDirectPayload;

    fn creates(payload: &ThreadDirectPayload) -> Option<EntityKey> {
        thread_key(&payload.thread).ok()
    }

    fn needs(payload: &ThreadDirectPayload) -> Vec<EntityKey> {
        [&payload.seat_lo, &payload.seat_hi]
            .into_iter()
            .filter_map(|seat| seat_key(seat).ok())
            .collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: ThreadDirectPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            payload.check_id()?;
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::EitherSeatHome(payload.seat_lo.clone(), payload.seat_hi.clone()),
            )?;
            control::materialize_thread_direct(tx, m, &payload)
        })
    }
}
