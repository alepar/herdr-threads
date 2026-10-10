//! Passive delivery bookkeeping. These primitives do not validate caller claims;
//! completion callers must do so in their canonical deciding transaction (A2).
use super::{
    bindings,
    connection::{api_error, store_error},
    read_cursors,
};
use crate::protocol::{
    commands::DeliveryMode,
    ids::{BindingId, MessageId, SeatId, ThreadId},
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

/// A lazy recipient row `r` (with its message `m`, possibly absent) is unread at seat level when
/// it is legacy pending (carried from before V31) or some connected top-level binding of the seat
/// has a cursor short of the message. Post-V31 delivery never reads `lazy_recipients.state`.
macro_rules! seat_unread_sql {
    () => {
        "(r.legacy_pending=1 OR EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=r.seat_id AND b.ended_at IS NULL AND b.role='top_level' AND b.liveness='connected' AND m.sequence>COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),0)))"
    };
}
pub(crate) use seat_unread_sql;

/// Candidates include staged rows (no message yet): publication is tested after the seek.
pub const PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
    seat_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
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
    let recipient_ordinal = conn
        .query_row(
            concat!(
                "SELECT coalesce(max(r.ordinal),0) FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
                seat_unread_sql!(),
                ")"
            ),
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
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

/// With a `binding`, a published addressed row is pending until that binding's cursor passes
/// it (or, for a top-level binding, until its legacy pending flag clears); without one (a
/// pre-binding caller) the seat-wide `state` column decides, as before V31.
pub fn lazy_completion_state(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    message: &MessageId,
) -> Result<LazyCompletion, ApiError> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT r.state,r.ordinal FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3",
            params![seat.as_str(), message.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, ordinal)) = row else {
        return Ok(LazyCompletion::NotAddressed);
    };
    let pending = match binding {
        Some(binding) => conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN messages m ON m.id=r.message_id WHERE b.binding_id=?1 AND r.ordinal=?2 AND (r.legacy_pending=1 AND b.role='top_level' OR m.sequence>COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),0)))",
                params![binding.as_str(), ordinal],
                |r| r.get::<_, bool>(0),
            )
            .map_err(store_error)?,
        None => state != "displayed",
    };
    Ok(if pending {
        LazyCompletion::Pending
    } else {
        LazyCompletion::Completed
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

/// A lazy message is unread for a binding until its cursor passes the message; a legacy pending
/// row (carried from before V31) is also unread for every top-level binding until displayed.
const BINDING_UNREAD_WHERE: &str = "b.binding_id=?1 AND (m.sequence>COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),0) OR (r.legacy_pending=1 AND b.role='top_level'))";
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

/// Displays completed for `binding`: advances the binding's cursor over the displayed messages
/// and, when the binding is top-level, clears the seat's displayed legacy rows. With `None` (a
/// caller that matches no open binding, a pre-binding client) the display counts for the whole
/// seat as before V31: every open top-level binding advances. Also keeps the historical `state`
/// column. The completion handlers validate the caller and the messages before calling.
pub fn complete_for_binding(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    messages: &[MessageId],
) -> Result<(), ApiError> {
    let mut by_thread: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    let mut displayed = Vec::new();
    for message in messages {
        if !complete_addressed(tx, instance, seat, message)? {
            continue;
        }
        let (thread, sequence): (String, i64) = tx
            .query_row(
                "SELECT thread_id,sequence FROM messages WHERE id=?1",
                [message.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_error)?;
        by_thread.entry(thread).or_default().push(
            u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        );
        displayed.push(message);
    }
    let targets: Vec<BindingId> = match binding {
        Some(binding) => vec![binding.clone()],
        None => bindings::list_open_top_level(tx, seat)?,
    };
    let clears_legacy = match binding {
        Some(binding) => is_top_level(tx, binding)?,
        None => true,
    };
    if clears_legacy {
        for message in displayed {
            tx.execute(
                "UPDATE lazy_recipients SET legacy_pending=0 WHERE seat_id=?1 AND message_id=?2 AND legacy_pending=1",
                params![seat.as_str(), message.as_str()],
            )
            .map_err(store_error)?;
        }
    }
    for target in &targets {
        for (thread, sequences) in &by_thread {
            read_cursors::advance_through_displayed(tx, target, &ThreadId::new(thread), sequences)?;
        }
    }
    // `binding_cursors` is WITHOUT ROWID, so SQLite's update hook never reports it. Touch the
    // displayed rows so the commit still kicks the mod observer (`lazy_recipients` is in
    // `MOD_NOTIFY_TABLES`) when a later binding's cursor passes an already-displayed row.
    for message in messages {
        tx.execute(
            "UPDATE lazy_recipients SET legacy_pending=legacy_pending WHERE seat_id=?1 AND message_id=?2",
            params![seat.as_str(), message.as_str()],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

fn is_top_level(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    Ok(bindings::get(db, binding)?.is_some_and(|b| b.role == bindings::BindingRole::TopLevel))
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

#[cfg(test)]
#[path = "../../tests/store/binding_lazy.rs"]
mod binding_lazy_tests;
