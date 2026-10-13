use super::*;

/// The lazy wake frontier of a `wake_on: all` seat: the greatest publication
/// decision sequence (`send_manifests.decision_seq`) of its pending published
/// lazy rows, or `None` when nothing is pending. Publication order only grows,
/// so a row staged early but published late still lies beyond a frontier
/// reserved earlier; the staging ordinal would not. Same window as
/// [`pending_summary`]: the newest `PENDING_SUMMARY_CAP` pending rows by
/// staging ordinal. A late-published row's ordinal is near the top, because
/// staging and publication are seconds apart.
pub fn wake_frontier(conn: &Connection, seat: &str) -> Result<Option<i64>, ApiError> {
    let (count, max_seq): (i64, i64) = conn
        .query_row(
            concat!(
                "SELECT count(*),COALESCE(max(w.decision_seq),0) FROM (SELECT sm.decision_seq FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal DESC LIMIT ?2) w"
            ),
            params![seat, PENDING_SUMMARY_CAP],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((count > 0).then_some(max_seq))
}

/// Publication time of the seat's oldest published pending lazy row; the batch
/// window treats it like an ordinary receipt's publication time.
pub fn oldest_pending_at(conn: &Connection, seat: &str) -> Result<Option<i64>, ApiError> {
    conn.query_row(
        concat!(
            "SELECT m.decision_at FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal LIMIT 1"
        ),
        [seat],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)
}

/// Wake discovery probe: the seat is `wake_on: all` and has a lazy recipient row that is
/// staged (no message yet) or unread at seat level, within the newest
/// `PENDING_SUMMARY_CAP` rows. The canonical summary decides whether the row is published.
pub fn has_wake_rows(conn: &Connection, seat: &str) -> Result<bool, ApiError> {
    conn.query_row(
        concat!(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND wake_on='all') AND EXISTS(SELECT 1 FROM (SELECT r.ordinal FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
            seat_unread_sql!(),
            ") ORDER BY r.ordinal DESC LIMIT 1000))"
        ),
        [seat],
        |r| r.get(0),
    )
    .map_err(store_error)
}
