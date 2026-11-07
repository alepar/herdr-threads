//! Durable target observations, allocation and operator repair.

use crate::{
    ports::HostId,
    protocol::{
        ids::SeatId,
        results::{ApiError, ErrorCode},
    },
    store::{
        bindings,
        connection::{api_error, store_error},
        control,
    },
};
use rusqlite::{Connection, OptionalExtension};

/// The one published local-seat predicate: a seat is local when its home is
/// this node (`is_local = 1`; the `seats_home_node_*` triggers keep the column
/// equal to `home_node = local_node.id`, and a home never changes).
///
/// Responsibility: owns the predicate's spelling ([`LOCAL_SEAT`] for SQL,
/// [`is_local_seat`] for one seat). Hides how locality is stored. Must not be
/// restated: callers never compare `home_node` inline. New locality questions
/// (a remote seat's host, its kind) get a function here.
///
/// Adopting node-local workers (each selects local seats only; a new
/// seat-selecting worker query adds it): observation and reconciliation (this
/// module's snapshot apply, absence closure, allocation, retirement by
/// observation, recovery baseline), liveness sampling
/// (`bindings::liveness_candidates`), wake (`WAKE_SEAT_WALK`, `BATCH_SEATS_SQL`,
/// `wake::reservable_seat_sql`, which the poke scans share), archival
/// (`archival::has_live_remote_member` negates it) and pane listings
/// (`panes`, the picker directory, `seats --panes`). Reads that list members
/// (participants, thread show, directory, `seats`) deliberately include remote
/// seats and render them with state `remote`. Role: the one locality test.
pub(crate) const LOCAL_SEAT: &str = "is_local=1";

/// The Herdr server a local seat sits on (`seats.host_id`; the last server of an
/// unresolved seat). A remote or unknown seat has none.
pub(crate) fn seat_host(conn: &Connection, seat: &SeatId) -> Result<HostId, ApiError> {
    conn.query_row(
        &format!("SELECT host_id FROM seats WHERE id=?1 AND {LOCAL_SEAT}"),
        [seat.as_str()],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)?
    .map(HostId::from_row)
    .ok_or_else(|| api_error(ErrorCode::NotFound, "local seat not found"))
}

/// Whether `seat` exists and is local (see [`LOCAL_SEAT`]); an unknown seat is
/// not local.
pub(crate) fn is_local_seat(conn: &Connection, seat: &SeatId) -> Result<bool, ApiError> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND {LOCAL_SEAT})"),
        [seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

mod continuity;
mod cooperative;
pub(crate) mod key_verdict;
mod observation;
mod operator;
mod publication;
mod resolution;
pub(crate) mod reversible;
mod threshold;

pub use self::continuity::*;
pub use self::cooperative::*;
pub use self::observation::*;
pub use self::operator::*;
pub use self::publication::*;
pub use self::resolution::*;
pub use self::threshold::*;
