//! Remote seats' advisory status (ht-xg88 seat status).
//!
//! Responsibility (remote_seat_status)
//! - Owns: the `remote_seat_origin` and `remote_seat_status` tables:
//!   [`record`] (the coordinator's accepted `seat_status` snapshots), [`get`]
//!   and the freshness rule [`presence_of`].
//! - Hides: the SQL, the whole-snapshot replacement and the home-node guard.
//! - Must not: feed any wake, route, deadline, availability, catch-up, seat
//!   resolution or attribution (TRUST-POLICY C1, N5): every value is a
//!   cooperative hint from the seat's home node, shown as such.
//! - Role: written through `SyncStorePort::record_remote_seat_status`, read
//!   by the seat listings.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::connection::{api_error, store_error};
use super::sync_contacts::{NodeReach, UNREACHABLE_AFTER_MS};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};

/// One seat of an accepted snapshot, on this node's wall clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSeat {
    pub seat: String,
    pub presence: String,
    pub harness: Option<String>,
    pub activity_at: Option<u64>,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    pub pane: Option<String>,
}

/// An accepted snapshot of `origin`'s seats, or (`seats: None`) a refresh of
/// the held one: same content, newer `built_at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSeatUpdate {
    pub origin: NodeUuid,
    pub boot: Uuid,
    pub rev: u64,
    /// When the origin built it, on this node's clock.
    pub built_at: u64,
    pub version: Option<String>,
    /// The last time another boot of `origin` delivered a snapshot.
    pub other_boot_at: Option<u64>,
    pub seats: Option<Vec<RemoteSeat>>,
}

fn ms(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Stores `updates` in order. A snapshot replaces its origin's seat rows; a
/// seat is kept only when this node knows it as a remote seat whose home
/// node is the origin (a consistency guard, N4: only the home node describes
/// its seats). `now` stamps `updated_at`.
pub fn record(
    tx: &Transaction<'_>,
    updates: &[RemoteSeatUpdate],
    now: u64,
) -> Result<(), ApiError> {
    for update in updates {
        let origin = update.origin.to_string();
        tx.execute(
            "INSERT INTO remote_seat_origin(origin,boot,rev,built_at,version,other_boot_at,updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7) \
             ON CONFLICT(origin) DO UPDATE SET boot=excluded.boot, rev=excluded.rev, \
               built_at=excluded.built_at, version=coalesce(excluded.version, version), \
               other_boot_at=excluded.other_boot_at, updated_at=excluded.updated_at",
            params![
                origin,
                update.boot.to_string(),
                ms(update.rev),
                ms(update.built_at),
                update.version.as_deref(),
                update.other_boot_at.map(ms),
                ms(now)
            ],
        )
        .map_err(store_error)?;
        let Some(seats) = &update.seats else {
            continue;
        };
        tx.execute("DELETE FROM remote_seat_status WHERE origin=?1", [&origin])
            .map_err(store_error)?;
        let mut insert = tx
            .prepare_cached(
                "INSERT OR REPLACE INTO remote_seat_status(seat_id,origin,presence,harness,activity_at,workspace,tab,pane) \
                 SELECT ?1,?2,?3,?4,?5,?6,?7,?8 WHERE EXISTS(SELECT 1 FROM seats \
                   WHERE id=?1 AND is_local=0 AND state='remote' AND home_node=?2)",
            )
            .map_err(store_error)?;
        for seat in seats {
            insert
                .execute(params![
                    seat.seat,
                    origin,
                    seat.presence,
                    seat.harness,
                    seat.activity_at.map(ms),
                    seat.workspace,
                    seat.tab,
                    seat.pane
                ])
                .map_err(store_error)?;
        }
    }
    Ok(())
}

/// One remote seat's stored status with its snapshot's origin facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSeatRow {
    pub origin: NodeUuid,
    pub seat: RemoteSeat,
    pub built_at: u64,
    pub other_boot_at: Option<u64>,
}

/// `seat`'s stored status, if its home node published one.
pub fn get(conn: &Connection, seat: &str) -> Result<Option<RemoteSeatRow>, ApiError> {
    type Columns = (
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        Option<i64>,
    );
    let row: Option<Columns> = conn
        .query_row(
            "SELECT s.origin,s.presence,s.harness,s.activity_at,s.workspace,s.tab,s.pane, \
               o.built_at,o.other_boot_at \
             FROM remote_seat_status s JOIN remote_seat_origin o ON o.origin=s.origin \
             WHERE s.seat_id=?1",
            [seat],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let unsigned = |v: i64| u64::try_from(v).unwrap_or(0);
    row.map(
        |(origin, presence, harness, activity, workspace, tab, pane, built, other)| {
            Ok(RemoteSeatRow {
                origin: origin
                    .parse()
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed seat origin"))?,
                seat: RemoteSeat {
                    seat: seat.to_owned(),
                    presence,
                    harness,
                    activity_at: activity.map(unsigned),
                    workspace,
                    tab,
                    pane,
                },
                built_at: unsigned(built),
                other_boot_at: other.map(unsigned),
            })
        },
    )
    .transpose()
}

/// What a remote seat's presence may be shown as (ht-xg88 §5.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemotePresence {
    /// Its home node never published one (no `seat.status.v1`, or not yet).
    Unknown,
    /// Its home node is unreachable: never a stale presence.
    Unreachable { since: Option<u64> },
    /// Two boots of its home node published within the reach window.
    CloneSuspected,
    /// The snapshot is older than the reach window although the node is
    /// reachable (its coordinator stopped publishing).
    Stale { built_at: u64 },
    Fresh {
        presence: String,
        activity_at: Option<u64>,
    },
}

/// The freshness rule, the only place it is decided: a remote seat's
/// presence is shown only as fresh as its home node's status allows.
/// `origin_live`: the coordinator has a live active link to the home node
/// (then its stored reach does not matter).
pub fn presence_of(
    row: Option<&RemoteSeatRow>,
    origin_reach: NodeReach,
    origin_live: bool,
    now: u64,
) -> RemotePresence {
    if !origin_live && let NodeReach::Unreachable { since } = origin_reach {
        return RemotePresence::Unreachable { since };
    }
    let Some(row) = row else {
        return RemotePresence::Unknown;
    };
    let within = |at: u64| now.saturating_sub(at) <= UNREACHABLE_AFTER_MS;
    if row.other_boot_at.is_some_and(within) {
        return RemotePresence::CloneSuspected;
    }
    if !within(row.built_at) {
        return RemotePresence::Stale {
            built_at: row.built_at,
        };
    }
    RemotePresence::Fresh {
        presence: row.seat.presence.clone(),
        activity_at: row.seat.activity_at,
    }
}
