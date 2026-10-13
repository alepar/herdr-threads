use super::*;

/// Candidates include staged rows (no message yet): publication is tested after the seek.
pub const PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
    seat_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// The binding-keyed twin of `PENDING_CANDIDATES_SQL` (`?5` is the reading binding).
pub const BINDING_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?5 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
    binding_lazy_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// `PENDING_CANDIDATES_SQL` restricted to one thread (`?5`; ht-br0 thread-scoped page).
pub const THREAD_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND r.thread_id=?5 AND (m.id IS NULL OR ",
    seat_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// `BINDING_PENDING_CANDIDATES_SQL` restricted to one thread (`?6`; ht-br0 thread-scoped page)
/// and to messages below `?7`, the binding's first ordinary walk stop in that thread
/// (`read_cursors::first_ordinary_walk_stop`): a lazy row above it could be shown but not passed
/// by the display walk, so the next hook or inbox would show it again.
pub const BINDING_THREAD_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?5 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND r.thread_id=?6 AND (m.id IS NULL OR (m.sequence<?7 AND ",
    binding_lazy_unread_sql!(),
    ")) AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);

/// With a `binding`, the high water counts only rows unread for that binding (staged rows still
/// count); without one, the seat form.
pub fn capture_high_water(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
) -> Result<HighWater, ApiError> {
    let publication_decision = conn
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let belongs: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !belongs {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy recipient instance mismatch",
        ));
    }
    let recipient_ordinal = match binding {
        None => conn
            .query_row(
                concat!(
                    "SELECT coalesce(max(r.ordinal),0) FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
                    seat_unread_sql!(),
                    ")"
                ),
                [seat.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
        Some(binding) => conn
            .query_row(
                concat!(
                    "SELECT coalesce(max(r.ordinal),0) FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?2 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
                    binding_lazy_unread_sql!(),
                    ")"
                ),
                params![seat.as_str(), binding.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    Ok(HighWater {
        recipient_ordinal,
        publication_decision,
    })
}

pub fn publication_eligible(
    conn: &Connection,
    instance: &str,
    recipient: &Recipient,
    publication_decision: i64,
) -> Result<bool, ApiError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id WHERE sm.preparation_id=?1 AND sm.instance_id=?2 AND sm.message_id=?3 AND sm.thread_id=?4 AND sm.decision_seq<=?5 AND m.delivery_mode='lazy')", params![recipient.preparation_id,instance,recipient.message.as_str(),recipient.thread.as_str(),publication_decision], |r| r.get(0)).map_err(store_error)
}

/// Seek candidates first, then test publication. Excluded rows consume work and
/// advance the cursor, even when a page returns no visible recipient. With a `binding` the
/// candidates are that binding's own unread lazy rows; without one, the seat form.
pub fn pending_page(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    after: i64,
    high_water: HighWater,
    work_limit: usize,
) -> Result<PendingPage, ApiError> {
    pending_page_in(
        conn, instance, seat, binding, None, after, high_water, work_limit,
    )
}

/// [`pending_page`], with `thread` restricting the candidates to that one thread (ht-br0
/// thread-scoped page); rows of other threads are neither returned nor counted as work.
#[allow(clippy::too_many_arguments)]
pub fn pending_page_in(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    thread: Option<&ThreadId>,
    after: i64,
    high_water: HighWater,
    work_limit: usize,
) -> Result<PendingPage, ApiError> {
    validate_bounds(after, high_water.recipient_ordinal, work_limit)?;
    if high_water.publication_decision < 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid lazy publication bound",
        ));
    }
    let belongs: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !belongs {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy recipient instance mismatch",
        ));
    }
    let read_row = |r: &rusqlite::Row<'_>| {
        Ok(Recipient {
            ordinal: r.get(0)?,
            preparation_id: r.get(1)?,
            message: MessageId::new(r.get::<_, String>(2)?),
            thread: ThreadId::new(r.get::<_, String>(3)?),
            seat: SeatId::new(r.get::<_, String>(4)?),
        })
    };
    let candidates = match (binding, thread) {
        (None, Some(thread)) => conn
            .prepare(THREAD_PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64,
                    thread.as_str()
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
        (Some(binding), Some(thread)) => {
            let stop = read_cursors::first_ordinary_walk_stop(conn, binding, seat, thread)?
                .map_or(Ok(i64::MAX), i64::try_from)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "message sequence out of range"))?;
            conn.prepare(BINDING_THREAD_PENDING_CANDIDATES_SQL)
                .map_err(store_error)?
                .query_map(
                    params![
                        seat.as_str(),
                        after,
                        high_water.recipient_ordinal,
                        work_limit as i64,
                        binding.as_str(),
                        thread.as_str(),
                        stop
                    ],
                    read_row,
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?
        }
        (None, None) => conn
            .prepare(PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
        (Some(binding), None) => conn
            .prepare(BINDING_PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64,
                    binding.as_str()
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
    };
    let inspected = candidates.len();
    let last_inspected = candidates.last().map_or(after, |r| r.ordinal);
    let mut recipients = Vec::new();
    for recipient in candidates {
        if publication_eligible(conn, instance, &recipient, high_water.publication_decision)? {
            recipients.push(recipient);
        }
    }
    Ok(PendingPage {
        recipients,
        high_water,
        last_inspected,
        inspected,
        has_more: inspected == work_limit && last_inspected < high_water.recipient_ordinal,
    })
}

/// A published lazy message that is unread for one binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyUnread {
    pub ordinal: i64,
    pub message: MessageId,
    pub thread: ThreadId,
    pub sequence: u64,
}

/// A lazy message is unread when its own cursor is short and its established or joined
/// stream permits it, or it is newly published in its current direct invitation episode.
/// The top-level legacy-pending exception remains independent of modern scope.
const BINDING_UNREAD_WHERE: &str = concat!("b.binding_id=?1 AND ", binding_lazy_unread_sql!());
const BINDING_UNREAD_FROM: &str = "FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=r.message_id AND m.delivery_mode='lazy'";

pub fn binding_lazy_unread(
    db: &Connection,
    binding: &BindingId,
    limit: usize,
) -> Result<Vec<LazyUnread>, ApiError> {
    let mut stmt = db
        .prepare(&format!(
            "SELECT r.ordinal,r.message_id,r.thread_id,m.sequence {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE} ORDER BY r.ordinal LIMIT ?2"
        ))
        .map_err(store_error)?;
    stmt.query_map(params![binding.as_str(), limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })
    .map_err(store_error)?
    .map(|row| {
        let (ordinal, message, thread, sequence) = row.map_err(store_error)?;
        Ok(LazyUnread {
            ordinal,
            message: MessageId::new(message),
            thread: ThreadId::new(thread),
            sequence: u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        })
    })
    .collect()
}

/// A published lazy message unread by `binding`, with its publication decision sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedLazyUnread {
    pub ordinal: i64,
    pub message: String,
    pub thread: String,
    pub decision_seq: i64,
}

/// The newest published lazy messages unread by one binding, newest ordinal first: one windowed
/// walk of at most `limit` rows. Published means the manifest is published in `instance` up to
/// the host decision sequence `through` (the same bound `pending_lazy` applies per seat).
pub fn binding_lazy_window(
    db: &Connection,
    instance: &str,
    binding: &BindingId,
    through: i64,
    limit: i64,
) -> Result<Vec<PublishedLazyUnread>, ApiError> {
    let mut stmt = db
        .prepare_cached(&format!(
            "SELECT r.ordinal,r.message_id,r.thread_id,sm.decision_seq {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE} AND sm.instance_id=?3 AND sm.decision_seq<=?4 ORDER BY r.ordinal DESC LIMIT ?2"
        ))
        .map_err(store_error)?;
    stmt.query_map(params![binding.as_str(), limit, instance, through], |r| {
        Ok(PublishedLazyUnread {
            ordinal: r.get(0)?,
            message: r.get(1)?,
            thread: r.get(2)?,
            decision_seq: r.get(3)?,
        })
    })
    .map_err(store_error)?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(store_error)
}

pub fn binding_has_unread_lazy(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    db.query_row(
        &format!("SELECT EXISTS(SELECT 1 {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE})"),
        params![binding.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Seat-level view for seat-keyed consumers (mod fingerprint, kicks, the legacy seat inbox):
/// published lazy messages unread for at least one connected top-level binding of the seat, plus
/// legacy pending rows; a seat with no connected top-level binding counts only legacy rows.
/// Returns `(count, max ordinal)` over the newest `cap` rows.
pub fn seat_lazy_pending(db: &Connection, seat: &SeatId, cap: i64) -> Result<(u64, i64), ApiError> {
    let (count, max): (i64, i64) = db
        .query_row(
            concat!(
                "SELECT count(*),COALESCE(max(ordinal),0) FROM (SELECT r.ordinal FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal DESC LIMIT ?2)"
            ),
            params![seat.as_str(), cap],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((u64::try_from(count).unwrap_or(0), max))
}

/// Bound of the pending-lazy count (a change detector for the mod fingerprint
/// and the wake frontier, not a count anyone presents).
pub const PENDING_SUMMARY_CAP: i64 = 1000;

/// `(count, max ordinal)` of the seat's published pending lazy rows, over the
/// newest `PENDING_SUMMARY_CAP`. Published rows only: a staged row is invisible
/// to the mod and to wake, and its publication (manifest + message) must change
/// both. Shared by `mod_seat_view` and `wake_seat_attention`.
pub fn pending_summary(conn: &Connection, seat: &SeatId) -> Result<(i64, i64), ApiError> {
    // Pending means unread at seat level (`seat_unread_sql!`): post-V34 delivery predicates never
    // read `lazy_recipients.state` (only `init_for_join` does, to skip displayed rows).
    let (count, max) = seat_lazy_pending(conn, seat, PENDING_SUMMARY_CAP)?;
    Ok((i64::try_from(count).unwrap_or(i64::MAX), max))
}
