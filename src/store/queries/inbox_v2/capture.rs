use super::*;

/// The snapshot's binding fence: the seat's mapping generation and the execution of its most
/// recent open binding (presentation only; a seat holds many bindings, ht-54r.3). A captured
/// cursor stays valid while the seat generation is unchanged and some open binding still holds
/// the captured execution (`fence_holds`), so another session enrolling does not invalidate it.
pub(in crate::store::queries) fn binding(
    db: &Connection,
    seat: &SeatId,
) -> Result<Option<(u64, ExecutionId)>, ApiError> {
    db.query_row("SELECT s.generation,b.execution_id FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id WHERE b.seat_id=?1 AND b.ended_at IS NULL ORDER BY b.ordinal DESC LIMIT 1",[seat.as_str()],|r|Ok((r.get::<_,i64>(0)? as u64,ExecutionId::new(r.get::<_,String>(1)?)))).optional().map_err(store_error)
}
pub(in crate::store::queries) fn fence_holds(
    db: &Connection,
    seat: &SeatId,
    generation: Option<u64>,
    execution: Option<&ExecutionId>,
) -> Result<bool, ApiError> {
    match (generation, execution) {
            (Some(generation), Some(execution)) => db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id WHERE b.seat_id=?1 AND b.ended_at IS NULL AND s.generation=?2 AND b.execution_id=?3)",
                    params![seat.as_str(), generation as i64, execution.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error),
            (None, None) => Ok(binding(db, seat)?.is_none()),
            _ => Ok(false),
        }
}
/// Who is reading (ht-54r.7). `Bound` serves that binding's own stream (spec §2 Inbox, per
/// binding); `Unknown` is a claim whose session has no binding yet: the page a fresh binding
/// would get (no backlog past the seat maximum); `Seat` is today's per-seat page.
pub(super) enum Reader {
    Seat,
    Unknown,
    Bound(BindingId),
}
pub(super) fn reader(
    db: &Connection,
    seat: &SeatId,
    claim: Option<&CallerClaim>,
) -> Result<Reader, ApiError> {
    let Some(claim) = claim.filter(|c| c.harness != crate::protocol::authority::Harness::Human)
    else {
        return Ok(Reader::Seat);
    };
    if claim.seat != *seat {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "inbox claim is for another seat",
        ));
    }
    Ok(
        match crate::store::bindings::match_claim(
            db,
            &crate::store::bindings::BindingKey::from_claim(claim),
        )? {
            Some(binding) => Reader::Bound(binding.binding_id),
            None => Reader::Unknown,
        },
    )
}
pub(super) fn bound(reader: &Reader) -> Option<&BindingId> {
    match reader {
        Reader::Bound(b) => Some(b),
        _ => None,
    }
}
/// The identity a cursor pins: the reading binding's own generation and execution when bound,
/// else the seat's current binding.
pub(super) fn identity(
    db: &Connection,
    seat: &SeatId,
    reader: &Reader,
) -> Result<Option<(u64, ExecutionId)>, ApiError> {
    match reader {
            Reader::Bound(b) => db
                .query_row(
                    "SELECT generation,execution_id FROM occupant_bindings WHERE binding_id=?1 AND ended_at IS NULL",
                    [b.as_str()],
                    |r| Ok((r.get::<_, i64>(0)? as u64, ExecutionId::new(r.get::<_, String>(1)?))),
                )
                .optional()
                .map_err(store_error),
            _ => binding(db, seat),
        }
}
pub(super) fn capture(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    reader: &Reader,
    ordinary_only: bool,
) -> Result<State, ApiError> {
    let high = if ordinary_only {
        crate::store::lazy_delivery::HighWater {
            recipient_ordinal: db
                .query_row(
                    "SELECT coalesce(max(ordinal),0) FROM lazy_recipients WHERE seat_id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?,
            publication_decision: db
                .query_row(
                    "SELECT decision_seq FROM host_instances WHERE id=?1",
                    [instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?,
        }
    } else {
        crate::store::lazy_delivery::capture_high_water(db, instance, seat, bound(reader))?
    };
    let (generation, execution) =
        identity(db, seat, reader)?.map_or((None, None), |(g, e)| (Some(g), Some(e)));
    let max = |table: &str| -> Result<i64, ApiError> {
        db.query_row(
            &format!("SELECT coalesce(max(ordinal),0) FROM {table} WHERE seat_id=?1"),
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)
    };
    // A bound reader's warning cutoff is its own binding's offer frontier (spec §2); a
    // seat-level reader follows the seat's notice binding.
    let offered: i64 = match bound(reader) {
        Some(binding) => db
            .query_row(
                crate::store::attention::BINDING_WARNING_OFFER_SQL,
                [binding.as_str()],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(store_error)?
            .unwrap_or(0),
        None => db
            .query_row(
                &format!(
                    "SELECT o.offered_through_seq FROM warning_offer o WHERE o.binding_id=({})",
                    crate::store::attention::SEAT_NOTICE_BINDING_SQL
                ),
                [seat.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?
            .unwrap_or(0),
    };
    Ok(State {
        seat: seat.clone(),
        binding_generation: generation,
        execution,
        source: Source::Lazy,
        lazy_after_ordinal: 0,
        lazy_high_water_ordinal: high.recipient_ordinal as u64,
        publication_decision_high_water: high.publication_decision as u64,
        body: None,
        unread: matches!(reader, Reader::Bound(_)).then_some(InboxBatchV2UnreadPosition {
            after_ordinal: 0,
            high_water_ordinal: max("prepared_recipients")? as u64,
        }),
        attention: SeatAttentionCursorState {
            invitation_after_seq: 0,
            invitation_after_ordinal: 0,
            invitations_done: ordinary_only,
            has_pending_invitation: false,
            invitation_frontier: Some((
                0,
                if ordinary_only {
                    0
                } else {
                    max("invitations")?
                },
            )),
            receipts: Some(ReceiptAttentionCursorState {
                physical_after: 0,
                manifest_after: 0,
                physical_high_water: max("receipts")?,
                manifest_high_water: max("prepared_recipients")?,
                next_manifest: false,
            }),
            receipts_done: false,
            has_pending_receipt: false,
            receipt_frontier_seq: Some(high.publication_decision),
            physical_warning_after: 0,
            physical_warning_high_water: high.publication_decision,
            manifest_warning_after: 0,
            manifest_warning_high_water: high.publication_decision,
            // The global logical scan needs one phase-complete bit and the
            // frozen informational offer cutoff, not v1 wake token state.
            next_manifest_warning: ordinary_only,
            latest_warning_seq: None,
            latest_warning_offset: Some(offered),
        },
    })
}
