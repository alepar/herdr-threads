use super::*;
use crate::protocol::pagination::DeliveryRowsPosition;

/// One view's ordered candidate: the next row of binding `?1`'s pending view (`binding_pending`) after
/// `(?3,?4,?5)` = (priority, decision_seq, ordinal), published at or below `?2`. No sort runs.
const SEEK_VIEW_SQL: &str = "SELECT p.priority,p.decision_seq,p.ordinal,r.preparation_id,r.message_id,r.thread_id FROM binding_pending p INDEXED BY binding_pending_order JOIN lazy_recipients r ON r.ordinal=p.ordinal WHERE p.binding_id=?1 AND p.decision_seq<=?2 AND (p.priority,p.decision_seq,p.ordinal)>(?3,?4,?5) ORDER BY p.priority,p.decision_seq,p.ordinal LIMIT 1";
/// [`SEEK_VIEW_SQL`] in one thread (`?6`), on the thread twin of the index.
const SEEK_VIEW_THREAD_SQL: &str = "SELECT p.priority,p.decision_seq,p.ordinal,r.preparation_id,r.message_id,r.thread_id FROM binding_pending p INDEXED BY binding_pending_thread_order JOIN lazy_recipients r ON r.ordinal=p.ordinal WHERE p.binding_id=?1 AND p.thread_id=?6 AND p.decision_seq<=?2 AND (p.priority,p.decision_seq,p.ordinal)>(?3,?4,?5) ORDER BY p.priority,p.decision_seq,p.ordinal LIMIT 1";
/// The seat's legacy seat-pending rows (`?1` seat, `?6` thread or NULL), part of every seat-level view.
const SEEK_LEGACY_SQL: &str = "SELECT r.priority,r.decision_seq,r.ordinal,r.preparation_id,r.message_id,r.thread_id FROM lazy_recipients r INDEXED BY lazy_recipients_legacy_order WHERE r.seat_id=?1 AND r.legacy_pending=1 AND r.decision_seq IS NOT NULL AND r.decision_seq<=?2 AND (r.priority,r.decision_seq,r.ordinal)>(?3,?4,?5) AND (?6 IS NULL OR r.thread_id=?6) ORDER BY r.priority,r.decision_seq,r.ordinal LIMIT 1";

/// The post-seek filters, one `EXISTS` per candidate (`?1` ordinal, `?2` instance, `?3` publication
/// bound) so the seek's own plan stays index-only. The row identity joins are the pending page's.
const SEAT_FILTER_SQL: &str = concat!(
    "SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id WHERE r.ordinal=?1 AND sm.instance_id=?2 AND sm.decision_seq<=?3 AND ",
    seat_unread_sql!(),
    ")"
);
/// `?4` is the reading binding, `?5` the exclusive sequence bound (`i64::MAX` when none).
const BINDING_FILTER_SQL: &str = concat!(
    "SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id JOIN occupant_bindings b ON b.binding_id=?4 AND b.seat_id=r.seat_id WHERE r.ordinal=?1 AND sm.instance_id=?2 AND sm.decision_seq<=?3 AND m.sequence<?5 AND ",
    binding_lazy_unread_sql!(),
    ")"
);

/// Which rows [`seek_pending`] may return after the ordered view seek (a post-seek filter; every
/// rejected row costs one visit).
#[derive(Debug, Clone, Copy)]
pub enum RowFilter<'a> {
    /// Seat-level pending (`seat_unread_sql!`).
    Seat,
    /// The binding's own unread rows (`binding_lazy_unread_sql!`, displayed marks excluded).
    Binding(&'a BindingId),
    /// Binding unread in one thread, below `stop` (`read_cursors::first_ordinary_walk_stop`,
    /// computed once per seek call by the caller, as `pending_page_in` does).
    ThreadBinding {
        binding: &'a BindingId,
        thread: &'a ThreadId,
        stop: i64,
    },
    /// Seat-level pending in one thread.
    Thread(&'a ThreadId),
    /// Private presentation eligibility (`PRIVATE_LAZY_ELIGIBILITY_SQL`): the presentation path's
    /// own test, here so it never walks `lazy_recipients` itself.
    PrivatePresentation(&'a BindingId),
}

impl RowFilter<'_> {
    fn thread(&self) -> Option<&ThreadId> {
        match self {
            RowFilter::ThreadBinding { thread, .. } | RowFilter::Thread(thread) => Some(thread),
            _ => None,
        }
    }
}

/// The outcome of one [`seek_pending`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeekResult {
    /// The first row that passed the filter.
    pub row: Option<(DeliveryRowsPosition, Recipient)>,
    /// View rows examined, accepted or rejected.
    pub visited: usize,
    /// The last position examined (accepted or rejected); resume strictly after it.
    pub position: Option<DeliveryRowsPosition>,
    /// The views hold no further row after `position` within the publication bound.
    pub exhausted: bool,
}

/// DeliveryRowSeek: the next pending published delivery row of `seat` after `after` in
/// (priority, decision_seq, ordinal) order, published at or below `dh`, that passes `filter`,
/// visiting at most `max_visits` index rows.
///
/// Owns: the ordered walk of the reading binding's own pending view (`binding_pending`), or, for a
/// seat-level reader, the min-merge of the connected top-level bindings' views and the seat's legacy
/// rows (thread twins for a thread-scoped filter), the publication bound inside it, and the post-seek
/// predicate. Hides: which views are read, the indexes, the position encoding in the bounds, the
/// displayed-mark exclusion and the private presentation eligibility test. Must not: read a
/// disconnected binding's view for a seat-level reader, read `lazy_recipients.state`, let a caller
/// walk `lazy_recipients` for delivery order itself (the presentation path included), visit an
/// unpublished row, or sort.
pub fn seek_pending(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    filter: RowFilter<'_>,
    after: Option<DeliveryRowsPosition>,
    dh: u64,
    max_visits: usize,
) -> Result<SeekResult, ApiError> {
    let out_of_range = || {
        api_error(
            ErrorCode::StoreCorrupt,
            "delivery row position out of range",
        )
    };
    let dh = i64::try_from(dh)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid lazy publication bound"))?;
    // A bound reader walks only its own view (a top-level binding's legacy rows are in it); a
    // seat-level reader merges the connected top-level views and the seat's legacy rows.
    let (views, legacy): (Vec<String>, bool) = match filter {
        RowFilter::Binding(b)
        | RowFilter::ThreadBinding { binding: b, .. }
        | RowFilter::PrivatePresentation(b) => (vec![b.as_str().to_owned()], false),
        RowFilter::Seat | RowFilter::Thread(_) => (connected_top_level(conn, seat.as_str())?, true),
    };
    let mut position = after;
    let mut visited = 0;
    while visited < max_visits {
        let (p, d, o) = position.map_or((-1, 0, 0), |p| {
            (
                i64::from(p.priority.get()),
                i64::try_from(p.decision_seq).unwrap_or(i64::MAX),
                i64::try_from(p.ordinal).unwrap_or(i64::MAX),
            )
        });
        let read = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        };
        let mut candidate: Option<(i64, i64, i64, String, String, String)> = None;
        let mut consider = |row: Option<(i64, i64, i64, String, String, String)>| {
            if let Some(row) = row
                && candidate
                    .as_ref()
                    .is_none_or(|best| (row.0, row.1, row.2) < (best.0, best.1, best.2))
            {
                candidate = Some(row);
            }
        };
        for view in &views {
            let row = match filter.thread() {
                Some(thread) => conn
                    .prepare_cached(SEEK_VIEW_THREAD_SQL)
                    .map_err(store_error)?
                    .query_row(params![view, dh, p, d, o, thread.as_str()], read),
                None => conn
                    .prepare_cached(SEEK_VIEW_SQL)
                    .map_err(store_error)?
                    .query_row(params![view, dh, p, d, o], read),
            }
            .optional()
            .map_err(store_error)?;
            consider(row);
        }
        if legacy {
            let row = conn
                .prepare_cached(SEEK_LEGACY_SQL)
                .map_err(store_error)?
                .query_row(
                    params![
                        seat.as_str(),
                        dh,
                        p,
                        d,
                        o,
                        filter.thread().map(ThreadId::as_str)
                    ],
                    read,
                )
                .optional()
                .map_err(store_error)?;
            consider(row);
        }
        let Some((priority, decision_seq, ordinal, preparation_id, message, thread)) = candidate
        else {
            return Ok(SeekResult {
                row: None,
                visited,
                position,
                exhausted: true,
            });
        };
        visited += 1;
        let here = DeliveryRowsPosition {
            priority: u8::try_from(priority)
                .ok()
                .and_then(|v| crate::protocol::priority::DeliveryPriority::new(v).ok())
                .ok_or_else(out_of_range)?,
            decision_seq: u64::try_from(decision_seq).map_err(|_| out_of_range())?,
            ordinal: u64::try_from(ordinal).map_err(|_| out_of_range())?,
        };
        position = Some(here);
        let accepted: bool = match filter {
            RowFilter::Seat | RowFilter::Thread(_) => conn
                .prepare_cached(SEAT_FILTER_SQL)
                .map_err(store_error)?
                .query_row(params![ordinal, instance, dh], |r| r.get(0)),
            RowFilter::Binding(binding) => conn
                .prepare_cached(BINDING_FILTER_SQL)
                .map_err(store_error)?
                .query_row(
                    params![ordinal, instance, dh, binding.as_str(), i64::MAX],
                    |r| r.get(0),
                ),
            RowFilter::ThreadBinding { binding, stop, .. } => conn
                .prepare_cached(BINDING_FILTER_SQL)
                .map_err(store_error)?
                .query_row(
                    params![ordinal, instance, dh, binding.as_str(), stop],
                    |r| r.get(0),
                ),
            RowFilter::PrivatePresentation(binding) => conn
                .prepare_cached(crate::store::queries::PRIVATE_LAZY_ELIGIBILITY_SQL)
                .map_err(store_error)?
                .query_row(
                    params![seat.as_str(), message, binding.as_str(), instance, dh],
                    |r| r.get(0),
                ),
        }
        .map_err(store_error)?;
        if accepted {
            return Ok(SeekResult {
                row: Some((
                    here,
                    Recipient {
                        ordinal,
                        preparation_id,
                        message: MessageId::new(message),
                        thread: ThreadId::new(thread),
                        seat: seat.clone(),
                    },
                )),
                visited,
                position,
                exhausted: false,
            });
        }
    }
    Ok(SeekResult {
        row: None,
        visited,
        position,
        exhausted: false,
    })
}

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
            "SELECT decision_seq FROM local_node WHERE id=?1",
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
    conn.query_row("SELECT EXISTS(SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id WHERE sm.preparation_id=?1 AND sm.instance_id=?2 AND sm.message_id=?3 AND sm.thread_id=?4 AND sm.decision_seq<=?5)", params![recipient.preparation_id,instance,recipient.message.as_str(),recipient.thread.as_str(),publication_decision], |r| r.get(0)).map_err(store_error)
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
const BINDING_UNREAD_FROM: &str = "FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=r.message_id";

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

/// Seat-level pending oracle for tests (no production reader; seat-level readers seek the
/// connected views through `seek_pending` / `wake_frontier`): published lazy messages unread for
/// at least one connected top-level binding of the seat, plus legacy pending rows; a seat with no
/// connected top-level binding counts only legacy rows. Returns `(count, max ordinal)` over the
/// newest `cap` rows.
#[cfg(test)]
pub fn seat_lazy_pending(db: &Connection, seat: &SeatId, cap: i64) -> Result<(u64, i64), ApiError> {
    let (count, max): (i64, i64) = db
        .query_row(
            concat!(
                "SELECT count(*),COALESCE(max(ordinal),0) FROM (SELECT r.ordinal FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal DESC LIMIT ?2)"
            ),
            params![seat.as_str(), cap],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((u64::try_from(count).unwrap_or(0), max))
}

#[cfg(test)]
#[path = "../../../tests/store/delivery_row_seek.rs"]
mod delivery_row_seek_tests;
