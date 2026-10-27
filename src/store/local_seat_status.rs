//! This node's own seats as `seat_status` publishes them (ht-xg88 phase 2).
//!
//! Responsibility (local_seat_status)
//! - Owns: [`local_seats`], the read of each local, published, nonretired
//!   seat's binding, connection and canonical UI state.
//! - Hides: the binding choice (the open top-level binding, else the latest)
//!   and the effective-observation lookup.
//! - Must not: decide presence for any purpose but the advisory publication;
//!   wakes and availability keep their own rules.
//! - Role: read by `service::seat_publisher`.

use rusqlite::{Connection, OptionalExtension};

use super::connection::store_error;
use crate::ports::HostId;
use crate::protocol::ids::SeatId;
use crate::protocol::results::ApiError;
use crate::sync::frame::MAX_SEAT_STATUS_SEATS;

/// One local seat, as the publisher needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalSeat {
    pub seat: SeatId,
    pub host: Option<HostId>,
    pub target: Option<String>,
    pub terminal: Option<String>,
    pub incarnation: Option<String>,
    /// The harness of the open top-level binding, else of the latest one.
    pub harness: Option<String>,
    /// The seat is active (`bindings::seat_active`).
    pub connected: bool,
    /// The canonical observation's UI state of the seat's target.
    pub ui_state: Option<String>,
    /// The latest binding registration or disconnect (unix ms).
    pub last_transition_at: Option<i64>,
}

/// The advisory presence word of a local seat: `busy` when active and a
/// turn is in progress (the wake path's rule, `effective::turn_in_progress`),
/// `idle` when active otherwise, `offline` when not active.
pub fn presence(seat: &LocalSeat) -> &'static str {
    if !seat.connected {
        "offline"
    } else if seat
        .ui_state
        .as_deref()
        .is_some_and(super::effective::turn_in_progress)
    {
        "busy"
    } else {
        "idle"
    }
}

type SeatRow = (
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Every local, published, nonretired seat, newest first (past the cap the
/// oldest, likeliest idle, are the ones left out), at most
/// [`MAX_SEAT_STATUS_SEATS`].
pub fn local_seats(conn: &Connection) -> Result<Vec<LocalSeat>, ApiError> {
    let rows: Vec<SeatRow> = {
        let mut statement = conn
            .prepare_cached(
                "SELECT id,host_id,target_id,structural_terminal_id,structural_incarnation FROM seats \
                 WHERE is_local=1 AND published=1 AND state<>'retired' ORDER BY ordinal DESC LIMIT ?1",
            )
            .map_err(store_error)?;
        let mapped = statement
            .query_map([MAX_SEAT_STATUS_SEATS as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(store_error)?;
        mapped.collect::<Result<_, _>>().map_err(store_error)?
    };
    rows.into_iter().map(|row| read_one(conn, row)).collect()
}

/// One local seat by id, published or not; `None` for a remote or unknown
/// seat.
pub fn local_seat(conn: &Connection, seat: &str) -> Result<Option<LocalSeat>, ApiError> {
    let row: Option<SeatRow> = conn
        .query_row(
            "SELECT id,host_id,target_id,structural_terminal_id,structural_incarnation FROM seats \
             WHERE id=?1 AND is_local=1",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    row.map(|row| read_one(conn, row)).transpose()
}

fn read_one(conn: &Connection, row: SeatRow) -> Result<LocalSeat, ApiError> {
    let (id, host, target, terminal, incarnation) = row;
    let seat = SeatId::new(id);
    let (harness, last_transition_at): (Option<String>, Option<i64>) = conn
        .prepare_cached(
            "SELECT (SELECT harness FROM occupant_bindings WHERE seat_id=?1 \
               ORDER BY (ended_at IS NULL AND role='top_level') DESC, (ended_at IS NULL) DESC, \
                 registered_at DESC LIMIT 1), \
             (SELECT MAX(max(coalesce(registered_at, 0), coalesce(disconnected_at, 0))) \
               FROM occupant_bindings WHERE seat_id=?1)",
        )
        .and_then(|mut statement| {
            statement.query_row([seat.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
        })
        .map_err(store_error)?;
    // No binding ever registered or disconnected: no transition.
    let last_transition_at = last_transition_at.filter(|at| *at > 0);
    let host = host.map(HostId::from_row);
    let ui_state = match (host, target.as_deref()) {
        (Some(host), Some(target)) => {
            super::effective::effective_observation(conn, host, target)?.map(|o| o.ui_state)
        }
        _ => None,
    };
    Ok(LocalSeat {
        connected: super::bindings::seat_active(conn, &seat)?,
        seat,
        host,
        target,
        terminal,
        incarnation,
        harness,
        ui_state,
        last_transition_at,
    })
}
