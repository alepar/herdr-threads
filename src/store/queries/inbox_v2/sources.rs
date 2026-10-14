use super::*;
use crate::protocol::pagination::DeliveryRowsPosition;

/// The recipient ordinal of `message` when it is unread by `binding`, within the captured
/// publication bound.
pub(in crate::store::queries) fn unread_ordinal(
    db: &Connection,
    seat: &SeatId,
    binding: &BindingId,
    message: &str,
    decision: u64,
) -> Result<Option<i64>, ApiError> {
    use crate::store::read_cursors::{RECEIVABLE_FROM, RECEIVABLE_WHERE, UNREAD_EXTRA};
    db.query_row(
            &format!("SELECT pr.ordinal FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} {UNREAD_EXTRA} AND m.id=?3 AND sm.decision_seq<=?4"),
            params![seat.as_str(), binding.as_str(), message, decision as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)
}
pub(super) struct ContinuedOrdinary {
    pub(super) message: MessageId,
    pub(super) retire: bool,
}
/// Private presentation continuation only. Positive captured authority is
/// independent of the reader's mutable display cursor; missing authority
/// never proves that a continued body may be retired.
pub(super) fn continued_ordinary(
    db: &Connection,
    instance: &str,
    state: &State,
    binding: &BindingId,
    probes: &mut usize,
) -> Result<ContinuedOrdinary, ApiError> {
    let (_, body) = state.body.as_ref().expect("continued ordinary body");
    let (after_ordinal, high_water_ordinal) = state
        .deadline
        .window_start
        .unread_after
        .zip(state.deadline.high.unread)
        .ok_or_else(|| invalid("missing unread capture"))?;
    // Recipient, manifest, message, membership and the exact binding cursor.
    *probes += 5;
    let captured: Option<(i64, String, i64, i64, i64)> = db.query_row(
            "SELECT pr.ordinal,m.thread_id,m.sequence,sm.decision_seq,COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=?2 AND c.thread_id=m.thread_id),0) FROM messages m JOIN send_manifests sm ON sm.message_id=m.id AND sm.thread_id=m.thread_id JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.thread_id=m.thread_id AND pr.seat_id=?1 WHERE m.id=?3 AND m.instance_id=?4 AND sm.instance_id=?4 AND m.kind='ordinary' AND sm.decision_seq<=?5 AND EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=m.thread_id AND mb.seat_id=pr.seat_id AND mb.state='joined')",
            params![state.seat.as_str(),binding.as_str(),body.message.as_str(),instance,state.publication_decision_high_water as i64],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(store_error)?;
    let Some((ordinal, thread, sequence, decision, displayed)) = captured else {
        return Err(invalid("continued body lacks captured recipient authority"));
    };
    if ordinal <= 0
        || ordinal as u64 <= after_ordinal
        || ordinal as u64 > high_water_ordinal
        || sequence <= 0
        || decision < 0
        || displayed < 0
    {
        return Err(invalid(
            "continued body is outside captured recipient bounds",
        ));
    }
    // The conservative effective-receipt envelope includes sparse misses.
    *probes += 10;
    let receipt =
        crate::store::effective::effective_receipt(db, body.message.as_str(), state.seat.as_str())?
            .ok_or_else(|| invalid("continued body lacks receipt authority"))?;
    if receipt.source != crate::store::effective::ReceiptSource::Manifest
        || receipt.message_id != body.message.as_str()
        || receipt.seat_id != state.seat.as_str()
        || receipt.thread_id != thread
        || receipt.sequence != sequence
        || receipt.source_ordinal != ordinal
        || receipt.decision_seq != Some(decision)
    {
        return Err(invalid(
            "continued body receipt differs from captured authority",
        ));
    }
    let terminal = match receipt.state {
        EffectiveReceiptState::Pending => false,
        EffectiveReceiptState::Acknowledged
        | EffectiveReceiptState::RecipientRetired
        | EffectiveReceiptState::NotRequired => true,
    };
    Ok(ContinuedOrdinary {
        message: body.message.clone(),
        retire: displayed >= sequence && terminal,
    })
}
/// The next unread ordinary row for `binding` after `after` and within `high` and the publication
/// bound: `(recipient ordinal, message id, manifest decision_seq)`.
pub(super) fn next_unread(
    db: &Connection,
    seat: &SeatId,
    binding: &BindingId,
    after: i64,
    high: i64,
    decision: u64,
) -> Result<Option<(i64, String, i64)>, ApiError> {
    use crate::store::read_cursors::{RECEIVABLE_FROM, RECEIVABLE_WHERE, UNREAD_EXTRA};
    db.query_row(
        &format!("SELECT pr.ordinal,m.id,sm.decision_seq FROM {RECEIVABLE_FROM} WHERE {RECEIVABLE_WHERE} {UNREAD_EXTRA} AND pr.ordinal>?3 AND pr.ordinal<=?4 AND sm.decision_seq<=?5 ORDER BY pr.ordinal LIMIT 1"),
        params![seat.as_str(), binding.as_str(), after, high, decision as i64],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
    .map_err(store_error)
}
pub(super) struct InvitationStep {
    pub(super) ordinal: i64,
    pub(super) id: String,
    pub(super) thread: String,
    pub(super) topic: String,
    pub(super) goal: String,
    pub(super) revision: i64,
    pub(super) decision_seq: i64,
    pub(super) pending: bool,
}
/// The next invitation of `seat` after `after` within `high` and the publication bound, with its
/// pending check (not cancelled, rejected or settled).
pub(super) fn next_invitation(
    db: &Connection,
    seat: &SeatId,
    after: i64,
    high: i64,
    decision: u64,
) -> Result<Option<InvitationStep>, ApiError> {
    let row:Option<(i64,String,String,String,String,i64,i64)>=db.query_row("SELECT i.ordinal,i.id,i.thread_id,t.topic,t.goal,t.topic_revision,i.created_decision_seq FROM invitations i JOIN threads t ON t.id=i.thread_id WHERE i.seat_id=?1 AND i.ordinal>?2 AND i.ordinal<=?3 AND i.created_decision_seq<=?4 ORDER BY i.ordinal LIMIT 1",params![seat.as_str(),after,high,decision as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(store_error)?;
    let Some((ordinal, id, thread, topic, goal, revision, decision_seq)) = row else {
        return Ok(None);
    };
    let pending:bool=db.query_row("SELECT state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations WHERE invitation_id=?1) AND NOT EXISTS(SELECT 1 FROM invitation_rejections WHERE invitation_id=?1) FROM invitations WHERE id=?1",[&id],|r|r.get(0)).map_err(store_error)?;
    Ok(Some(InvitationStep {
        ordinal,
        id,
        thread,
        topic,
        goal,
        revision,
        decision_seq,
        pending,
    }))
}
/// The inbox item of a pending invitation step; `None` when it is no longer pending.
pub(super) fn invitation_item(
    db: &Connection,
    seat: &SeatId,
    step: InvitationStep,
) -> Result<Option<Item>, ApiError> {
    if !step.pending {
        return Ok(None);
    }
    let InvitationStep {
        id,
        thread,
        topic,
        goal,
        revision,
        ..
    } = step;
    let required_service =
        crate::store::service_substrate::current_requirement(db, &ThreadId::new(&thread), seat)?
            .filter(|r| {
                r.state == crate::protocol::service::RequirementState::Pending
                    && r.invitation.as_str() == id
            });
    let (confirmation, deadline_at) = invitation_confirmation_metadata(db, &id)?;
    Ok(Some(Item::Invitation {
        thread: ThreadId::new(&thread),
        topic_data: topic,
        goal_data: Some(goal),
        topic_revision: Some(
            u64::try_from(revision)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative topic revision"))?,
        ),
        invitation: crate::protocol::ids::InvitationId::new(id),
        confirmation,
        deadline_at,
        required_service,
    }))
}
pub(super) struct ReceiptStep {
    /// The pending, published, not-also-unread receipt of this step, if any.
    pub(super) receipt: Option<crate::store::effective::EffectiveReceipt>,
    pub(super) position: ReceiptScanPosition,
    /// Candidates examined; 0 means the walk is exhausted.
    pub(super) visited: usize,
}
/// One receipt-walk step from `position`. An obligation that is also unread for `reading` is
/// listed once, by the unread walk.
pub(super) fn next_receipt(
    db: &Connection,
    seat: &SeatId,
    reading: Option<&BindingId>,
    position: ReceiptScanPosition,
    decision: u64,
) -> Result<ReceiptStep, ApiError> {
    let slice = scan_effective_receipts(
        db,
        &ReceiptScanScope::Seat(seat.as_str().into()),
        Some(position),
        1,
    )?;
    let mut pending = slice.items.into_iter().next().filter(|r| {
        r.state == EffectiveReceiptState::Pending && r.decision_seq.unwrap_or(0) <= decision as i64
    });
    if let (Some(receipt), Some(binding)) = (&pending, reading)
        && unread_ordinal(db, seat, binding, &receipt.message_id, decision)?.is_some()
    {
        pending = None;
    }
    Ok(ReceiptStep {
        receipt: pending,
        position: slice.position,
        visited: slice.visited as usize,
    })
}
/// The inbox item of a warning candidate when it is addressed to `seat`, still actionable and
/// not yet offered to the reader (`offered` is the frozen offer cutoff of the cursor).
pub(super) fn warning_item(
    db: &Connection,
    seat: &SeatId,
    reading: Option<&BindingId>,
    candidate: &crate::store::effective::GlobalLogicalCandidate,
    offered: i64,
) -> Result<Option<Item>, ApiError> {
    let Some(warning) = effective_warning_by_id(db, &candidate.id)? else {
        return Ok(None);
    };
    if !is_warning_recipient(db, &candidate.id, seat.as_str())?
        || !warning_condition_actionable(db, &warning)?
        || !crate::store::attention::informational_notice_pending_for(
            db,
            seat.as_str(),
            &candidate.id,
            reading.map(BindingId::as_str),
        )?
        .unwrap_or(warning.event_seq > offered)
    {
        return Ok(None);
    }
    Ok(Some(Item::Warning {
        thread: ThreadId::new(&candidate.thread_id),
        topic_data: batch_topic(db, &candidate.thread_id)?,
        informational: !crate::store::attention::warning_wakes_seat(
            db,
            seat.as_str(),
            &candidate.id,
        )?,
        warning: MessageId::new(candidate.id.clone()),
        sequence: warning.sequence as u64,
    }))
}
/// The delivery-row position of the published lazy `message` addressed to `seat`, when the row
/// exists in this instance within the publication bound `decision`.
pub(super) fn lazy_body_position(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    message: &str,
    decision: u64,
) -> Result<Option<DeliveryRowsPosition>, ApiError> {
    let row: Option<(i64, Option<i64>, i64)> = db
        .query_row(
            "SELECT r.priority,r.decision_seq,r.ordinal FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id AND sm.thread_id=r.thread_id JOIN messages m ON m.id=r.message_id AND m.instance_id=sm.instance_id AND m.thread_id=r.thread_id AND m.decision_seq=sm.decision_seq WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3 AND sm.decision_seq<=?4 AND m.kind='ordinary'",
            params![seat.as_str(), message, instance, decision as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((priority, Some(decision_seq), ordinal)) = row else {
        return Ok(None);
    };
    let out_of_range = || invalid("continued body row is out of range");
    Ok(Some(DeliveryRowsPosition {
        priority: u8::try_from(priority)
            .ok()
            .and_then(|v| crate::protocol::priority::DeliveryPriority::new(v).ok())
            .ok_or_else(out_of_range)?,
        decision_seq: u64::try_from(decision_seq).map_err(|_| out_of_range())?,
        ordinal: u64::try_from(ordinal).map_err(|_| out_of_range())?,
    }))
}
