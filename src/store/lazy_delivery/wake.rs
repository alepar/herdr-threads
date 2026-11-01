use super::*;
use crate::protocol::priority::NudgeThreshold;

/// One seek on binding `?1`'s pending view for priority `?2`, post-filtered by that binding's own unread
/// predicate (the view is a candidate set), in publication order. Staged rows (NULL `decision_seq`) are
/// outside the view and never count.
macro_rules! view_seek_sql {
    ($column:literal, $order:literal) => {
        concat!(
            "SELECT ",
            $column,
            " FROM binding_pending p INDEXED BY binding_pending_order JOIN occupant_bindings b ON b.binding_id=p.binding_id JOIN lazy_recipients r ON r.ordinal=p.ordinal JOIN messages m ON m.id=r.message_id WHERE p.binding_id=?1 AND p.priority=?2 AND ",
            binding_lazy_unread_sql!(),
            " ORDER BY p.decision_seq ",
            $order,
            " LIMIT 1"
        )
    };
}

/// The seat's (`?1`) legacy rows of priority `?2`, unread at seat level regardless of bindings.
macro_rules! legacy_seek_sql {
    ($column:literal, $order:literal) => {
        concat!(
            "SELECT ",
            $column,
            " FROM lazy_recipients r INDEXED BY lazy_recipients_legacy_order JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND r.priority=?2 AND r.legacy_pending=1 AND r.decision_seq IS NOT NULL ORDER BY r.decision_seq ",
            $order,
            " LIMIT 1"
        )
    };
}

/// The seek behind [`wake_frontier`]: the newest pending published row of one priority in one binding's view.
pub(crate) const FRONTIER_SEEK_SQL: &str = view_seek_sql!("p.decision_seq", "DESC");
const LEGACY_FRONTIER_SQL: &str = legacy_seek_sql!("r.decision_seq", "DESC");
const OLDEST_SEEK_SQL: &str = view_seek_sql!("m.decision_at", "ASC");
const LEGACY_OLDEST_SQL: &str = legacy_seek_sql!("m.decision_at", "ASC");
const ANY_SEEK_SQL: &str = view_seek_sql!("1", "ASC");
const LEGACY_ANY_SQL: &str = legacy_seek_sql!("1", "ASC");
/// How many of a seat's newest pending rows a dormant binding's walk reads. A process-exited binding
/// keeps its rows pending and no index orders them by priority, so a seat left in a shell under silent
/// mail would otherwise cost a growing scan per discovery walk. An admitted row is the newest when it
/// is published, so the bound only loses one that this many silent rows have since buried.
pub(crate) const DORMANT_WINDOW: i64 = 4096;
/// The rows behind `DORMANT_PENDING_SQL` and `DORMANT_ANY_SQL`. A dormant binding (`?1`, of seat `?2`;
/// `dormant_top_level`) holds no view, so these are the seat's pending rows (`seat_owes_display_sql!`
/// keeps them pending for it) in the admitted priorities `?3..=?4`.
macro_rules! dormant_rows_sql {
    () => {
        concat!(
            "lazy_recipients r JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?1 WHERE r.ordinal IN (SELECT ordinal FROM lazy_recipients INDEXED BY lazy_recipients_pending_seat_ordinal WHERE seat_id=?2 AND state='pending' ORDER BY ordinal DESC LIMIT ?5) AND r.decision_seq IS NOT NULL AND r.priority BETWEEN ?3 AND ?4 AND ",
            binding_lazy_unread_sql!()
        )
    };
}
/// Their newest publication and oldest publication time.
const DORMANT_PENDING_SQL: &str = concat!(
    "SELECT max(r.decision_seq),min(m.decision_at) FROM ",
    dormant_rows_sql!()
);
/// Whether there are any: the discovery probe stops at the first.
const DORMANT_ANY_SQL: &str = concat!("SELECT EXISTS(SELECT 1 FROM ", dormant_rows_sql!(), ")");

/// The admitted pending rows of the seat's dormant binding, if it has one: `(frontier, oldest)`.
fn dormant_pending(
    conn: &Connection,
    seat: &str,
    threshold: NudgeThreshold,
) -> Result<Option<(i64, i64)>, ApiError> {
    let Some(binding) = dormant_top_level(conn, seat)? else {
        return Ok(None);
    };
    let priorities = admitted(threshold);
    let (frontier, oldest): (Option<i64>, Option<i64>) = conn
        .prepare_cached(DORMANT_PENDING_SQL)
        .map_err(store_error)?
        .query_row(
            params![
                binding,
                seat,
                priorities.start(),
                priorities.end(),
                DORMANT_WINDOW
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok(frontier.zip(oldest))
}

/// [`dormant_pending`] as an existence probe.
fn dormant_has_pending(
    conn: &Connection,
    seat: &str,
    threshold: NudgeThreshold,
) -> Result<bool, ApiError> {
    let Some(binding) = dormant_top_level(conn, seat)? else {
        return Ok(false);
    };
    let priorities = admitted(threshold);
    conn.prepare_cached(DORMANT_ANY_SQL)
        .map_err(store_error)?
        .query_row(
            params![
                binding,
                seat,
                priorities.start(),
                priorities.end(),
                DORMANT_WINDOW
            ],
            |r| r.get(0),
        )
        .map_err(store_error)
}

/// The priorities a threshold admits: `0..=T`, every priority when T is negative.
fn admitted(threshold: NudgeThreshold) -> std::ops::RangeInclusive<i64> {
    let top = if threshold.get() < 0 {
        i64::from(crate::protocol::priority::DeliveryPriority::MAX)
    } else {
        i64::from(threshold.get())
    };
    0..=top
}

fn seek(conn: &Connection, sql: &str, key: &str, priority: i64) -> Result<Option<i64>, ApiError> {
    conn.prepare_cached(sql)
        .map_err(store_error)?
        .query_row(params![key, priority], |r| r.get(0))
        .optional()
        .map_err(store_error)
}

/// The delivery-row wake frontier of a seat at threshold `threshold`: the greatest publication
/// decision sequence (`lazy_recipients.decision_seq`) among the published rows whose priority the
/// threshold admits that are unread for some connected top-level binding (its own pending view) or
/// legacy pending, or `None` when nothing is pending. A disconnected binding's view is never read, so
/// it holds no wake, except the seat's dormant binding (`dormant_top_level`). Publication order only
/// grows, so a row staged early but published late still lies beyond a frontier reserved earlier;
/// the staging ordinal would not. One view seek per connected binding and admitted priority, so a deep backlog of
/// less urgent rows never hides an urgent one; only a seat with a dormant binding also walks its newest pending rows (`DORMANT_WINDOW`).
pub fn wake_frontier(
    conn: &Connection,
    seat: &str,
    threshold: NudgeThreshold,
) -> Result<Option<i64>, ApiError> {
    let mut frontier = None;
    let views = connected_top_level(conn, seat)?;
    for priority in admitted(threshold) {
        for view in &views {
            frontier = frontier.max(seek(conn, FRONTIER_SEEK_SQL, view, priority)?);
        }
        frontier = frontier.max(seek(conn, LEGACY_FRONTIER_SQL, seat, priority)?);
    }
    if let Some((dormant, _)) = dormant_pending(conn, seat, threshold)? {
        frontier = frontier.max(Some(dormant));
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
    let views = connected_top_level(conn, seat)?;
    for priority in admitted(threshold) {
        let found = views
            .iter()
            .map(|view| seek(conn, OLDEST_SEEK_SQL, view, priority))
            .chain([seek(conn, LEGACY_OLDEST_SQL, seat, priority)]);
        for at in found {
            if let Some(at) = at? {
                oldest = Some(oldest.map_or(at, |prior| prior.min(at)));
            }
        }
    }
    if let Some((_, at)) = dormant_pending(conn, seat, threshold)? {
        oldest = Some(oldest.map_or(at, |prior| prior.min(at)));
    }
    Ok(oldest)
}

/// Wake discovery probe: the seat has a published delivery row that is unread at seat level and
/// whose priority its current threshold admits. The canonical frontier decides the wake.
pub fn has_wake_rows(conn: &Connection, seat: &str) -> Result<bool, ApiError> {
    let threshold = crate::store::seats::nudge_threshold(conn, &SeatId::new(seat))?;
    let views = connected_top_level(conn, seat)?;
    for priority in admitted(threshold) {
        for view in &views {
            if seek(conn, ANY_SEEK_SQL, view, priority)?.is_some() {
                return Ok(true);
            }
        }
        if seek(conn, LEGACY_ANY_SQL, seat, priority)?.is_some() {
            return Ok(true);
        }
    }
    dormant_has_pending(conn, seat, threshold)
}
