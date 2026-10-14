use super::*;

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
/// A pending seat receipt within the publication bound: only then may a top-level display
/// ACK settle it. A message another binding already settled is shown without that.
pub(super) fn receipt_pending(
    db: &Connection,
    seat: &SeatId,
    message: &str,
    decision: u64,
) -> Result<bool, ApiError> {
    Ok(
        crate::store::effective::effective_receipt(db, message, seat.as_str())?.is_some_and(|r| {
            r.state == EffectiveReceiptState::Pending
                && r.decision_seq.unwrap_or(0) <= decision as i64
        }),
    )
}
pub(super) struct ContinuedOrdinary {
    pub(super) message: MessageId,
    pub(super) ordinal: i64,
    pub(super) pending: bool,
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
    let body = state.body.as_ref().expect("continued ordinary body");
    let position = state
        .unread
        .ok_or_else(|| invalid("missing unread capture"))?;
    // Recipient, manifest, message, membership and the exact binding cursor.
    *probes += 5;
    let captured: Option<(i64, String, i64, i64, i64)> = db.query_row(
            "SELECT pr.ordinal,m.thread_id,m.sequence,sm.decision_seq,COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=?2 AND c.thread_id=m.thread_id),0) FROM messages m JOIN send_manifests sm ON sm.message_id=m.id AND sm.thread_id=m.thread_id JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.thread_id=m.thread_id AND pr.seat_id=?1 WHERE m.id=?3 AND m.instance_id=?4 AND sm.instance_id=?4 AND m.kind='ordinary' AND m.delivery_mode='ordinary' AND sm.decision_seq<=?5 AND EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=m.thread_id AND mb.seat_id=pr.seat_id AND mb.state='joined')",
            params![state.seat.as_str(),binding.as_str(),body.message.as_str(),instance,state.publication_decision_high_water as i64],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).optional().map_err(store_error)?;
    let Some((ordinal, thread, sequence, decision, displayed)) = captured else {
        return Err(invalid("continued body lacks captured recipient authority"));
    };
    if ordinal <= 0
        || ordinal as u64 <= position.after_ordinal
        || ordinal as u64 > position.high_water_ordinal
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
    let (pending, terminal) = match receipt.state {
        EffectiveReceiptState::Pending => (true, false),
        EffectiveReceiptState::Acknowledged
        | EffectiveReceiptState::RecipientRetired
        | EffectiveReceiptState::NotRequired => (false, true),
    };
    Ok(ContinuedOrdinary {
        message: body.message.clone(),
        ordinal,
        pending,
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
fn invitation_item(
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
#[allow(clippy::too_many_arguments)]
pub(super) fn scan(
    db: &Connection,
    instance: &str,
    state: &mut State,
    current_agent: bool,
    reading: Option<&BindingId>,
    ordinary_only: bool,
    continuation: Option<&ContinuedOrdinary>,
    lazy_scope_probes: &mut usize,
    retired_body: &mut Option<MessageId>,
    scope: Option<&ThreadId>,
) -> Result<(Option<Item>, usize), ApiError> {
    let seat = &state.seat.clone();
    match state.source {
        Source::Unread => {
            let binding = reading.expect("unread walk only on a bound read");
            let position = state.unread.expect("unread walk carries its position");
            let decision = state.publication_decision_high_water;
            // A partial body pins an already validated unread identity (the same rule as lazy).
            if let Some(body) = state.body.clone() {
                let (ordinal, pending) = if ordinary_only {
                    let validated = continuation
                        .filter(|value| value.message == body.message && !value.retire)
                        .ok_or_else(|| invalid("missing validated private body continuation"))?;
                    (validated.ordinal, validated.pending)
                } else {
                    let ordinal =
                        unread_ordinal(db, seat, binding, body.message.as_str(), decision)?
                            .ok_or_else(|| {
                                invalid("continued body is not captured unread content")
                            })?;
                    (
                        ordinal,
                        receipt_pending(db, seat, body.message.as_str(), decision)?,
                    )
                };
                let ack = current_agent && pending;
                let item = message(db, body.message.as_str(), false, ack, state)?;
                state.unread = Some(InboxBatchV2UnreadPosition {
                    after_ordinal: ordinal as u64,
                    ..position
                });
                return Ok((item, 1));
            }
            if ordinary_only {
                let raw: Option<(i64, String)> = db.query_row(
                        "SELECT ordinal,preparation_id FROM prepared_recipients WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                        params![seat.as_str(),position.after_ordinal as i64,position.high_water_ordinal as i64],
                        |r| Ok((r.get(0)?,r.get(1)?)),
                    ).optional().map_err(store_error)?;
                let Some((ordinal, preparation)) = raw else {
                    state.unread = Some(InboxBatchV2UnreadPosition {
                        after_ordinal: position.high_water_ordinal,
                        ..position
                    });
                    return Ok((None, 0));
                };
                state.unread = Some(InboxBatchV2UnreadPosition {
                    after_ordinal: ordinal as u64,
                    ..position
                });
                let id: Option<String> = db.query_row(
                        "SELECT message_id FROM send_manifests WHERE preparation_id=?1 AND instance_id=?2 AND decision_seq<=?3",
                        params![preparation,instance,decision as i64], |r|r.get(0),
                    ).optional().map_err(store_error)?;
                let Some(id) = id else {
                    return Ok((None, 1));
                };
                if unread_ordinal(db, seat, binding, &id, decision)?.is_none() {
                    return Ok((None, 1));
                }
                let ack = current_agent && receipt_pending(db, seat, &id, decision)?;
                return Ok((message(db, &id, false, ack, state)?, 1));
            }
            let row = next_unread(
                db,
                seat,
                binding,
                position.after_ordinal as i64,
                position.high_water_ordinal as i64,
                decision,
            )?;
            let Some((ordinal, id, _)) = row else {
                state.unread = Some(InboxBatchV2UnreadPosition {
                    after_ordinal: position.high_water_ordinal,
                    ..position
                });
                return Ok((None, 0));
            };
            state.unread = Some(InboxBatchV2UnreadPosition {
                after_ordinal: ordinal as u64,
                ..position
            });
            let ack = current_agent && receipt_pending(db, seat, &id, decision)?;
            Ok((message(db, &id, false, ack, state)?, 1))
        }
        Source::Lazy => {
            // A partial body pins an already validated addressed identity.
            // Concurrent completion removes it from the pending index but
            // must not apply its offset to the next pending message.
            if let Some(body) = &state.body {
                let (ordinal, legacy_pending): (i64, i64) = db.query_row(
                        "SELECT ordinal,legacy_pending FROM lazy_recipients WHERE seat_id=?1 AND message_id=?2",
                        params![seat.as_str(), body.message.as_str()],
                        |r| Ok((r.get(0)?,r.get(1)?)),
                    ).map_err(store_error)?;
                if !matches!(legacy_pending, 0 | 1) {
                    return Err(invalid("continued body is not captured addressed content"));
                }
                // Recheck only scope. A captured tail must survive its own
                // cursor progress, but a cursorless reader cannot borrow a
                // later invitation episode for an older publication.
                if let Some(binding) = reading {
                    if ordinary_only {
                        *lazy_scope_probes += 3;
                    }
                    let visible: bool = db
                        .query_row(
                            super::super::LAZY_CONTINUATION_SCOPE_SQL,
                            params![
                                seat.as_str(),
                                body.message.as_str(),
                                binding.as_str(),
                                instance
                            ],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if !visible {
                        // Positive captured publication/address/range and
                        // accumulator validation preceded this scan. Only
                        // modern private tails may retire on scope loss;
                        // public and legacy continuations remain strict.
                        if ordinary_only && legacy_pending == 0 {
                            state.lazy_after_ordinal = ordinal as u64;
                            *retired_body = Some(body.message.clone());
                            state.body = None;
                            // The caller charges this one physical visit.
                            // Keep the captured cutoffs and signal tuple.
                            return Ok((None, 1));
                        }
                        return Err(invalid("continued body is not captured unread content"));
                    }
                }
                let item = message(db, body.message.as_str(), true, false, state)?;
                state.lazy_after_ordinal = ordinal as u64;
                return Ok((item, 1));
            }
            if ordinary_only {
                let binding =
                    reading.ok_or_else(|| invalid("presentation reader has no binding"))?;
                let raw: Option<(i64,String)> = db.query_row(
                        "SELECT ordinal,message_id FROM lazy_recipients WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                        params![seat.as_str(),state.lazy_after_ordinal as i64,state.lazy_high_water_ordinal as i64],
                        |r|Ok((r.get(0)?,r.get(1)?)),
                    ).optional().map_err(store_error)?;
                let Some((ordinal, id)) = raw else {
                    state.lazy_after_ordinal = state.lazy_high_water_ordinal;
                    return Ok((None, 0));
                };
                state.lazy_after_ordinal = ordinal as u64;
                *lazy_scope_probes += 3;
                let visible: bool = db
                    .query_row(
                        super::super::PRIVATE_LAZY_ELIGIBILITY_SQL,
                        params![
                            seat.as_str(),
                            id,
                            binding.as_str(),
                            instance,
                            state.publication_decision_high_water as i64
                        ],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                return Ok((
                    if visible {
                        message(db, &id, true, false, state)?
                    } else {
                        None
                    },
                    1,
                ));
            }
            let pending = crate::store::lazy_delivery::pending_page_in(
                db,
                instance,
                seat,
                reading,
                scope,
                state.lazy_after_ordinal as i64,
                crate::store::lazy_delivery::HighWater {
                    recipient_ordinal: state.lazy_high_water_ordinal as i64,
                    publication_decision: state.publication_decision_high_water as i64,
                },
                1,
            )?;
            let item = pending
                .recipients
                .first()
                .map(|r| message(db, r.message.as_str(), true, false, state))
                .transpose()?
                .flatten();
            state.lazy_after_ordinal = if pending.inspected == 0 {
                state.lazy_high_water_ordinal
            } else {
                pending.last_inspected as u64
            };
            Ok((item, pending.inspected))
        }
        Source::Invitations => {
            let a = &state.attention;
            let step = next_invitation(
                db,
                seat,
                a.invitation_after_ordinal,
                a.invitation_frontier.unwrap().1,
                state.publication_decision_high_water,
            )?;
            let Some(step) = step else {
                state.attention.invitations_done = true;
                return Ok((None, 0));
            };
            state.attention.invitation_after_ordinal = step.ordinal;
            state.attention.invitation_after_seq = step.ordinal;
            Ok((invitation_item(db, seat, step)?, 1))
        }
        Source::Receipts => {
            let step = next_receipt(
                db,
                seat,
                reading,
                batch_receipt_position(&state.attention),
                state.publication_decision_high_water,
            )?;
            let item = step
                .receipt
                .map(|r| message(db, &r.message_id, false, current_agent, state))
                .transpose()?
                .flatten();
            state.attention.receipts_done = step.visited == 0;
            batch_set_receipt_position(&mut state.attention, step.position);
            Ok((item, step.visited))
        }
        Source::Warnings => {
            let a = &state.attention;
            let slice = scan_global_logical_candidates(
                db,
                instance,
                GlobalLogicalKinds::Warnings,
                Some(GlobalLogicalPosition {
                    after_decision_seq: a.physical_warning_after,
                    after_event_offset: a.manifest_warning_after,
                    high_water_decision_seq: a.physical_warning_high_water,
                }),
                2,
            )?;
            state.attention.physical_warning_after = slice.position.after_decision_seq;
            state.attention.manifest_warning_after = slice.position.after_event_offset;
            let item = if let Some(candidate) = slice.candidates.into_iter().next() {
                if let Some(warning) = effective_warning_by_id(db, &candidate.id)?
                    && is_warning_recipient(db, &candidate.id, seat.as_str())?
                    && warning_condition_actionable(db, &warning)?
                    && crate::store::attention::informational_notice_pending_for(
                        db,
                        seat.as_str(),
                        &candidate.id,
                        reading.map(BindingId::as_str),
                    )?
                    .unwrap_or(
                        warning.event_seq > state.attention.latest_warning_offset.unwrap_or(0),
                    )
                {
                    Some(Item::Warning {
                        thread: ThreadId::new(&candidate.thread_id),
                        topic_data: batch_topic(db, &candidate.thread_id)?,
                        informational: !crate::store::attention::warning_wakes_seat(
                            db,
                            seat.as_str(),
                            &candidate.id,
                        )?,
                        warning: MessageId::new(candidate.id),
                        sequence: warning.sequence as u64,
                    })
                } else {
                    None
                }
            } else {
                state.attention.next_manifest_warning = true;
                None
            };
            Ok((item, slice.visited as usize))
        }
    }
}
