//! Fact kinds of the seat entity: `seat.created` (`seat.retired` lands in its
//! own leaf task).
//!
//! `seat.created` publishes a local seat so other nodes list it (fact-kinds
//! design section 4). The handler checks the origin owns the seat ID's tag,
//! then calls the one materializer in `store::seats`.

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
