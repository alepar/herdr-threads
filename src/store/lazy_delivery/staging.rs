use super::*;

pub const CLEANUP_CANDIDATES_SQL: &str = "SELECT ordinal FROM lazy_recipients INDEXED BY lazy_recipients_preparation_ordinal WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4";

/// Publication of a send's delivery rows: copy the publication `decision_seq` onto every row of
/// the preparation (set once; the audience was frozen at seal), then give every recipient whose
/// current nudge threshold admits its row's priority the new-attention write so the dispatcher
/// re-judges it. A recipient the threshold does not admit is untouched. Two set-based statements
/// over the preparation's recipients.
pub fn note_published(tx: &Transaction<'_>, preparation: &str) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE lazy_recipients SET decision_seq=(SELECT decision_seq FROM send_manifests WHERE preparation_id=?1) WHERE preparation_id=?1 AND decision_seq IS NULL",
        [preparation],
    )
    .map_err(store_error)?;
    tx.execute(
        &format!(
            "INSERT INTO wake_work(seat_id,reason_bits,attention_version) SELECT DISTINCT r.seat_id,1,1 FROM lazy_recipients r INDEXED BY lazy_recipients_preparation_ordinal JOIN seats s ON s.id=r.seat_id WHERE r.preparation_id=?1 AND {} ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",
            crate::store::seats::THRESHOLD_ADMITS_SQL
        ),
        [preparation],
    )
    .map_err(store_error)?;
    Ok(())
}

pub fn stage_recipient(
    tx: &Transaction<'_>,
    preparation: &str,
    message: &MessageId,
    thread: &ThreadId,
    seat: &SeatId,
    priority: crate::protocol::priority::DeliveryPriority,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES(?1,?2,?3,?4,?5)",params![preparation,message.as_str(),thread.as_str(),seat.as_str(),priority.get()]).map_err(store_error)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPage {
    pub inspected: usize,
    pub deleted: usize,
    pub last_inspected: i64,
    pub has_more: bool,
}

pub fn preparation_high_water(conn: &Connection, preparation: &str) -> Result<i64, ApiError> {
    conn.query_row(
        "SELECT coalesce(max(ordinal),0) FROM lazy_recipients WHERE preparation_id=?1",
        [preparation],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Cleanup a selected preparation in bounded units; publication never permits
/// deletion, and each selected candidate is charged before that decision.
pub fn cleanup_unpublished(
    tx: &Transaction<'_>,
    preparation: &str,
    after: i64,
    through: i64,
    work_limit: usize,
) -> Result<CleanupPage, ApiError> {
    validate_bounds(after, through, work_limit)?;
    let mut stmt = tx.prepare(CLEANUP_CANDIDATES_SQL).map_err(store_error)?;
    let candidates = stmt
        .query_map(
            params![preparation, after, through, work_limit as i64],
            |r| r.get::<_, i64>(0),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    let inspected = candidates.len();
    let last_inspected = candidates.last().copied().unwrap_or(after);
    let published: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",
            [preparation],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut deleted = 0;
    if !published {
        for ordinal in candidates {
            deleted += tx
                .execute("DELETE FROM lazy_recipients WHERE ordinal=?1", [ordinal])
                .map_err(store_error)?;
        }
    }
    Ok(CleanupPage {
        inspected,
        deleted,
        last_inspected,
        has_more: inspected == work_limit && last_inspected < through,
    })
}
