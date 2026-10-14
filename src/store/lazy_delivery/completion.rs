use super::*;

pub const ADDRESSED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3)";

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

/// Canonical caller validation and contiguous display proof belong to the
/// completion handler. Updates only published addressed progress, returning true
/// for both pending and already displayed addressed rows (idempotent eligibility).
pub fn complete_addressed(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    message: &MessageId,
) -> Result<bool, ApiError> {
    complete_addressed_as(tx, instance, seat, message, true)
}

/// [`complete_addressed`] for a given kind of display. `mark_displayed` is false for a subagent
/// binding's display: the seat-wide `state` stays `pending`, so the wake seek
/// (`lazy_recipients_pending_order`, `state='pending'`) still finds a row the seat's top-level
/// binding has not displayed.
fn complete_addressed_as(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    message: &MessageId,
    mark_displayed: bool,
) -> Result<bool, ApiError> {
    let addressed: bool = tx
        .query_row(
            ADDRESSED_SQL,
            params![seat.as_str(), message.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if addressed && mark_displayed {
        tx.execute("UPDATE lazy_recipients SET state='displayed' WHERE seat_id=?1 AND message_id=?2 AND state='pending'",params![seat.as_str(),message.as_str()]).map_err(store_error)?;
    }
    Ok(addressed)
}

/// A top-level display marks the row displayed seat-wide, unless a connected top-level binding of
/// the seat (the displaying one included, for a row it left above its cursor) still finds it
/// unread. `?1` = seat, `?2` = message; run after the displaying cursors moved.
const MARK_DISPLAYED_SQL: &str = concat!(
    "UPDATE lazy_recipients SET state='displayed' WHERE seat_id=?1 AND message_id=?2 AND state='pending' AND NOT EXISTS(SELECT 1 FROM lazy_recipients r JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND ",
    seat_unread_sql!(),
    ")"
);

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
/// pre-binding caller) the seat-wide `state` column decides, as before V34.
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
                concat!(
                    "SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN messages m ON m.id=r.message_id WHERE b.binding_id=?1 AND r.ordinal=?2 AND ",
                    binding_lazy_unread_sql!(),
                    ")"
                ),
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

/// Displays completed for `binding`: advances the binding's cursor over the displayed messages
/// and, when the binding is top-level, clears the seat's displayed legacy rows. With `None` (a
/// caller that matches no open binding, a pre-binding client) the display counts for the whole
/// seat as before V34: every open top-level binding advances. Also sets `state` to `displayed` (top-level or
/// pre-binding display only; a subagent's display leaves it `pending`),
/// which `read_cursors::init_for_join` reads so a join never re-marks a displayed row. The completion handlers validate the caller and the messages before calling.
pub fn complete_for_binding(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    messages: &[MessageId],
) -> Result<(), ApiError> {
    complete_displayed_for_binding(tx, instance, seat, binding, messages, &[])
}

/// [`complete_for_binding`] for a display that also showed `shown` ordinary messages whole
/// (settled or already settled by the caller): their sequences join `messages`' in the one
/// cursor walk per thread, so the cursor passes both. A `shown` id with no row is skipped.
pub fn complete_displayed_for_binding(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    messages: &[MessageId],
    shown: &[MessageId],
) -> Result<(), ApiError> {
    let clears_legacy = match binding {
        Some(binding) => is_top_level(tx, binding)?,
        None => true,
    };
    let mut by_thread: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    let mut displayed = Vec::new();
    for message in messages {
        // The seat-wide `state` flips only after the cursors moved (below): a sibling top-level
        // binding that has not displayed the row still owes it, and the ordered seek only holds
        // `pending` rows.
        if !complete_addressed_as(tx, instance, seat, message, false)? {
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
    for message in shown {
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT thread_id,sequence FROM messages WHERE id=?1 AND instance_id=?2",
                params![message.as_str(), instance],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        let Some((thread, sequence)) = row else {
            continue;
        };
        by_thread.entry(thread).or_default().push(
            u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        );
    }
    let targets: Vec<BindingId> = match binding {
        Some(binding) => vec![binding.clone()],
        None => bindings::list_open_top_level(tx, seat)?,
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
        let mut reader_changed = false;
        for (thread, sequences) in &by_thread {
            let (_, changed) = read_cursors::advance_through_displayed_with_change(
                tx,
                target,
                &ThreadId::new(thread),
                sequences,
            )?;
            reader_changed |= changed;
        }
        if reader_changed {
            crate::store::presentation_reader::note_binding_reader_change(tx, target)?;
        }
    }
    if clears_legacy {
        for message in messages {
            tx.execute(MARK_DISPLAYED_SQL, params![seat.as_str(), message.as_str()])
                .map_err(store_error)?;
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
