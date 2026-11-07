//! Fact kinds of the seat entity: `seat.created`, `seat.retired`, `seat.retired_reversible` and
//! `seat.unretired`.
//!
//! `seat.created` publishes a local seat so other nodes list it (fact-kinds
//! design section 4). The handler checks the origin owns the seat ID's tag,
//! then calls the one materializer in `store::seats`. `seat.retired` retires a
//! published seat on every node; its handler checks the origin is the seat's
//! home, then calls `seats::materialize_seat_retired`. On the authoring node's own replay a local row
//! takes the retirement fence's whole transition (open bindings end `seat_retired`), not a bare state change.
//! `seat.retired_reversible` and `seat.unretired` carry a reversible retirement to other nodes (spec 4.3): home-authored
//! like `seat.retired`, no-ops outside the episode their `retired_at` names, and parked by a node that does not know them,
//! so finalization stays the ordinary `seat.retired` every version applies.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{ReferencedSeats, SeatKind, seat_key};
use super::{ApplyContext, FactError, FactKind, WaitFor, with_input};
use crate::protocol::ids::SeatId;
use crate::protocol::results::ErrorCode;
use crate::store::materialize::{AuthorRule, require_author};
use crate::store::seats;
use crate::sync::EntityKey;

/// The `seat.created` v1 payload. `display_name` is the seat's name (`seats.name`) for a
/// named seat and `null` otherwise; a receiver keeps it only when it parses as a
/// `SeatName`. The created time is the envelope HLC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatCreatedPayload {
    pub seat: SeatId,
    pub kind: SeatKind,
    #[serde(default)]
    pub display_name: Option<String>,
}

impl ReferencedSeats for SeatCreatedPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

/// The `seat.created` v1 kind.
pub struct SeatCreated;

impl FactKind for SeatCreated {
    const NAME: &'static str = "seat.created";
    const MAJOR: u32 = 1;
    type Payload = SeatCreatedPayload;

    fn creates(payload: &SeatCreatedPayload) -> Option<EntityKey> {
        seat_key(&payload.seat).ok()
    }

    fn needs(_: &SeatCreatedPayload) -> Vec<EntityKey> {
        Vec::new()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: SeatCreatedPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(tx, m, Self::NAME, &AuthorRule::OwnTag(payload.seat.clone()))?;
            seats::materialize_seat_created(tx, m, &payload)
        })
        // The one wait: an own seat restore with no attached host.
        .map_err(|e| match e {
            FactError::Rejected(err) if err.code == ErrorCode::HostUnavailable => FactError::Wait {
                on: WaitFor::Host,
                error: err,
            },
            other => other,
        })
    }
}

/// Why a seat was retired. Closed set: the pane went away (`closed`) or an
/// operator retired it (`operator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetireReason {
    Closed,
    Operator,
}

/// The `seat.retired` v1 payload. `retired_at` is the cutover in Unix ms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatRetiredPayload {
    pub seat: SeatId,
    pub reason: RetireReason,
    pub retired_at: i64,
}

impl ReferencedSeats for SeatRetiredPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

/// The `seat.retired_reversible` v1 payload (spec 4.3). `retired_at` is the cutover in Unix ms and
/// names the episode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatRetiredReversiblePayload {
    pub seat: SeatId,
    pub reason: RetireReason,
    pub retired_at: i64,
}

impl ReferencedSeats for SeatRetiredReversiblePayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

/// The `seat.unretired` v1 payload (spec 4.3). `retired_at` names the episode it undoes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatUnretiredPayload {
    pub seat: SeatId,
    pub retired_at: i64,
    pub unretired_at: i64,
}

impl ReferencedSeats for SeatUnretiredPayload {
    fn referenced_seats(&self) -> Vec<SeatId> {
        vec![self.seat.clone()]
    }
}

/// The `seat.retired` v1 kind. It is a held kind (`materialize::HELD_KINDS`):
/// `FactRoute::Journal` refuses it for a local seat during the restore-window
/// authoring hold.
pub struct SeatRetired;

impl FactKind for SeatRetired {
    const NAME: &'static str = "seat.retired";
    const MAJOR: u32 = 1;
    type Payload = SeatRetiredPayload;

    fn creates(_: &SeatRetiredPayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &SeatRetiredPayload) -> Vec<EntityKey> {
        seat_key(&payload.seat).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: SeatRetiredPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.seat.clone()),
            )?;
            seats::materialize_seat_retired(tx, m, &payload)
        })
    }
}

/// The `seat.retired_reversible` v1 kind: the seat reads retired with no job. A held kind.
pub struct SeatRetiredReversible;

impl FactKind for SeatRetiredReversible {
    const NAME: &'static str = "seat.retired_reversible";
    const MAJOR: u32 = 1;
    type Payload = SeatRetiredReversiblePayload;

    fn creates(_: &SeatRetiredReversiblePayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &SeatRetiredReversiblePayload) -> Vec<EntityKey> {
        seat_key(&payload.seat).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: SeatRetiredReversiblePayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.seat.clone()),
            )?;
            seats::materialize_seat_retired_reversible(tx, m, &payload)
        })
    }
}

/// The `seat.unretired` v1 kind: undoes the episode `retired_at` names. A held kind.
pub struct SeatUnretired;

impl FactKind for SeatUnretired {
    const NAME: &'static str = "seat.unretired";
    const MAJOR: u32 = 1;
    type Payload = SeatUnretiredPayload;

    fn creates(_: &SeatUnretiredPayload) -> Option<EntityKey> {
        None
    }

    fn needs(payload: &SeatUnretiredPayload) -> Vec<EntityKey> {
        seat_key(&payload.seat).into_iter().collect()
    }

    fn apply(
        tx: &Transaction<'_>,
        cx: &mut ApplyContext<'_>,
        payload: SeatUnretiredPayload,
    ) -> Result<(), FactError> {
        with_input(cx, |m| {
            require_author(
                tx,
                m,
                Self::NAME,
                &AuthorRule::SeatHome(payload.seat.clone()),
            )?;
            seats::materialize_seat_unretired(tx, m, &payload)
        })
    }
}
