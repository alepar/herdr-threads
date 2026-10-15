//! Fact kinds of the seat entity: `seat.created` and `seat.retired`.
//!
//! `seat.created` publishes a local seat so other nodes list it (fact-kinds
//! design section 4). The handler checks the origin owns the seat ID's tag,
//! then calls the one materializer in `store::seats`. `seat.retired` retires a
//! published seat on every node; its handler checks the origin is the seat's
//! home, then calls `seats::materialize_seat_retired`.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::payload::{ReferencedSeats, SeatKind, seat_key};
use super::{ApplyContext, FactError, FactKind, with_input};
use crate::protocol::ids::SeatId;
use crate::store::materialize::{AuthorRule, require_author};
use crate::store::seats;
use crate::sync::EntityKey;

/// The `seat.created` v1 payload. `display_name` is reserved and always
/// `null` in v1 (seats carry no name); the created time is the envelope HLC.
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
