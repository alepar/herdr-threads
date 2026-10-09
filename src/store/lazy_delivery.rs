//! Passive delivery bookkeeping. These primitives do not validate caller claims;
//! completion callers must do so in their canonical deciding transaction (A2).
use super::connection::{api_error, store_error};
use crate::protocol::{
    commands::DeliveryMode,
    ids::{MessageId, SeatId, ThreadId},
    results::{ApiError, ErrorCode},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Both bounds are frozen in the same read transaction as the traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighWater {
    pub recipient_ordinal: i64,
    pub publication_decision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub ordinal: i64,
    pub preparation_id: String,
    pub message: MessageId,
    pub thread: ThreadId,
    pub seat: SeatId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPage {
    pub recipients: Vec<Recipient>,
    pub high_water: HighWater,
    pub last_inspected: i64,
    pub inspected: usize,
    /// A full work page may require an empty final page to prove exhaustion.
    pub has_more: bool,
}

pub const PENDING_CANDIDATES_SQL: &str = "SELECT ordinal,preparation_id,message_id,thread_id,seat_id FROM lazy_recipients INDEXED BY lazy_recipients_pending_seat_ordinal WHERE seat_id=?1 AND state='pending' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4";
pub const ADDRESSED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3)";
pub const CLEANUP_CANDIDATES_SQL: &str = "SELECT ordinal FROM lazy_recipients INDEXED BY lazy_recipients_preparation_ordinal WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4";

/// Exact canonical ID only; foreign-instance IDs are absent, never inferred.
pub fn recorded_mode(
    conn: &Connection,
    instance: &str,
    message: &MessageId,
) -> Result<Option<DeliveryMode>, ApiError> {
    let mode: Option<String> = conn
        .query_row(
            "SELECT delivery_mode FROM messages WHERE instance_id=?1 AND id=?2",
            params![instance, message.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    mode.map(|mode| match mode.as_str() {
        "ordinary" => Ok(DeliveryMode::Ordinary),
        "lazy" => Ok(DeliveryMode::Lazy),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid recorded delivery mode",
        )),
    })
    .transpose()
}

pub fn capture_high_water(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
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
    let recipient_ordinal = conn.query_row("SELECT coalesce(max(ordinal),0) FROM lazy_recipients INDEXED BY lazy_recipients_pending_seat_ordinal WHERE seat_id=?1 AND state='pending'", [seat.as_str()], |r| r.get(0)).map_err(store_error)?;
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
/// advance the cursor, even when a page returns no visible recipient.
pub fn pending_page(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
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
    let mut stmt = conn.prepare(PENDING_CANDIDATES_SQL).map_err(store_error)?;
    let candidates = stmt
        .query_map(
            params![
                seat.as_str(),
                after,
                high_water.recipient_ordinal,
                work_limit as i64
            ],
            |r| {
                Ok(Recipient {
                    ordinal: r.get(0)?,
                    preparation_id: r.get(1)?,
                    message: MessageId::new(r.get::<_, String>(2)?),
                    thread: ThreadId::new(r.get::<_, String>(3)?),
                    seat: SeatId::new(r.get::<_, String>(4)?),
                })
            },
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
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

/// Canonical caller validation and contiguous display proof belong to the
/// completion handler. Updates only published addressed progress, returning true
/// for both pending and already displayed addressed rows (idempotent eligibility).
pub fn complete_addressed(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    message: &MessageId,
) -> Result<bool, ApiError> {
    let addressed: bool = tx
        .query_row(
            ADDRESSED_SQL,
            params![seat.as_str(), message.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if addressed {
        tx.execute("UPDATE lazy_recipients SET state='displayed' WHERE seat_id=?1 AND message_id=?2 AND state='pending'",params![seat.as_str(),message.as_str()]).map_err(store_error)?;
    }
    Ok(addressed)
}

/// Where a lazy row stands for one seat (read-only helper for mod acks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyCompletion {
    /// Published and addressed to the seat; display not yet completed.
    Pending,
    /// Published, addressed, and already completed as displayed.
    Completed,
    /// Not a published row addressed to the seat in this instance.
    NotAddressed,
}

pub fn lazy_completion_state(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    message: &MessageId,
) -> Result<LazyCompletion, ApiError> {
    let state: Option<String> = conn
        .query_row(
            "SELECT r.state FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3",
            params![seat.as_str(), message.as_str(), instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(match state.as_deref() {
        Some("displayed") => LazyCompletion::Completed,
        Some(_) => LazyCompletion::Pending,
        None => LazyCompletion::NotAddressed,
    })
}

pub fn stage_recipient(
    tx: &Transaction<'_>,
    preparation: &str,
    message: &MessageId,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,?3,?4)",params![preparation,message.as_str(),thread.as_str(),seat.as_str()]).map_err(store_error)?;
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

fn validate_bounds(after: i64, through: i64, limit: usize) -> Result<(), ApiError> {
    if after < 0 || through < after || !(1..=100).contains(&limit) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid lazy work bounds",
        ));
    }
    Ok(())
}
