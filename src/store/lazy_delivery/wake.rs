use super::*;
use crate::protocol::priority::NudgeThreshold;

/// One seek on the v47 delivery-row index for a single priority: the seat's pending published rows
/// of that priority, post-filtered by the seat-level unread definition (`seat_unread_sql!`), in
/// publication order. Staged rows (NULL `decision_seq`) are outside the index and never count.
macro_rules! priority_seek_sql {
    ($column:literal, $order:literal) => {
        concat!(
            "SELECT ",
            $column,
            " FROM lazy_recipients r INDEXED BY lazy_recipients_pending_order JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND r.priority=?2 AND r.state='pending' AND r.decision_seq IS NOT NULL AND ",
            seat_unread_sql!(),
            " ORDER BY r.decision_seq ",
            $order,
            " LIMIT 1"
        )
    };
}

/// The seek behind [`wake_frontier`]: the newest pending published row of one priority.
pub(crate) const FRONTIER_SEEK_SQL: &str = priority_seek_sql!("r.decision_seq", "DESC");
const OLDEST_SEEK_SQL: &str = priority_seek_sql!("m.decision_at", "ASC");
const ANY_SEEK_SQL: &str = priority_seek_sql!("1", "ASC");

/// The priorities a threshold admits: `0..=T`, every priority when T is negative.
fn admitted(threshold: NudgeThreshold) -> std::ops::RangeInclusive<i64> {
    let top = if threshold.get() < 0 {
        i64::from(crate::protocol::priority::DeliveryPriority::MAX)
    } else {
        i64::from(threshold.get())
    };
    0..=top
}

fn seek(conn: &Connection, sql: &str, seat: &str, priority: i64) -> Result<Option<i64>, ApiError> {
    conn.prepare_cached(sql)
        .map_err(store_error)?
        .query_row(params![seat, priority], |r| r.get(0))
        .optional()
        .map_err(store_error)
}

/// The delivery-row wake frontier of a seat at threshold `threshold`: the greatest publication
/// decision sequence (`lazy_recipients.decision_seq`) among its pending published rows whose
/// priority the threshold admits, or `None` when nothing is pending. Publication order only
/// grows, so a row staged early but published late still lies beyond a frontier reserved earlier;
/// the staging ordinal would not. One index seek per admitted priority, so a deep backlog of
/// less urgent rows never hides an urgent one.
pub fn wake_frontier(
    conn: &Connection,
    seat: &str,
    threshold: NudgeThreshold,
) -> Result<Option<i64>, ApiError> {
    let mut frontier = None;
    for priority in admitted(threshold) {
        frontier = frontier.max(seek(conn, FRONTIER_SEEK_SQL, seat, priority)?);
    }
    Ok(frontier)
}

/// Publication time of the seat's oldest published pending admitted row; the batch window treats
/// it like an ordinary receipt's publication time.
pub fn oldest_pending_at(
    conn: &Connection,
    seat: &str,
    threshold: NudgeThreshold,
) -> Result<Option<i64>, ApiError> {
    let mut oldest: Option<i64> = None;
    for priority in admitted(threshold) {
        if let Some(at) = seek(conn, OLDEST_SEEK_SQL, seat, priority)? {
            oldest = Some(oldest.map_or(at, |prior| prior.min(at)));
        }
    }
    Ok(oldest)
}

/// Wake discovery probe: the seat has a published delivery row that is unread at seat level and
/// whose priority its current threshold admits. The canonical frontier decides the wake.
pub fn has_wake_rows(conn: &Connection, seat: &str) -> Result<bool, ApiError> {
    let threshold = crate::store::seats::nudge_threshold(conn, &SeatId::new(seat))?;
    for priority in admitted(threshold) {
        if seek(conn, ANY_SEEK_SQL, seat, priority)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
