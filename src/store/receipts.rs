//! Receipt settlement and availability transitions.

use crate::{
    ports::TimeBasis,
    protocol::{
        authority::{Harness, MutationPermit, ObligationRef},
        commands::Ack,
        ids::{MessageId, SeatId, ThreadId},
        results::{AckResult, ApiError, CommandResult, ErrorCode},
        time::UtcMillis,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    connection::{StoreContext, api_error, store_error},
    effective::{self, EffectiveReceiptState},
    schema::{self, EventInput},
};

pub fn ack_payload(request: &Ack) -> Value {
    json!({"kind":"ack", "messages":request.messages,"claim":request.claim})
}

pub fn ack_displayed_payload(request: &Ack) -> Value {
    json!({"kind":"ack_displayed", "messages":request.messages,"claim":request.claim})
}

pub fn ack(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &Ack,
    permit: &mut MutationPermit,
) -> Result<CommandResult, ApiError> {
    ack_impl(context, conn, budget, request, permit, false)
}

pub fn ack_displayed(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &Ack,
    permit: &mut MutationPermit,
) -> Result<CommandResult, ApiError> {
    ack_impl(context, conn, budget, request, permit, true)
}

fn ack_impl(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &Ack,
    permit: &mut MutationPermit,
    displayed: bool,
) -> Result<CommandResult, ApiError> {
    if request.messages.is_empty() || request.messages.len() > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid ACK batch size",
        ));
    }
    let distinct: BTreeSet<_> = request.messages.iter().collect();
    if distinct.len() != request.messages.len() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "duplicate message in ACK batch",
        ));
    }
    if displayed && request.claim.harness == Harness::Human {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "display ACK requires an agent claim",
        ));
    }
    let digest = schema::canonical_digest(&if displayed {
        ack_displayed_payload(request)
    } else {
        ack_payload(request)
    })?;
    let cooperative = permit.cooperative_metadata();
    let seat = request.claim.seat.clone();
    let scope = format!("seat:{}", seat.as_str());
    let result = schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        request.operation.as_str(),
        digest,
        |tx| {
            let active: bool = tx
                .query_row(
                    "SELECT state!='retired' FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if !active {
                return Err(api_error(ErrorCode::Unauthorized, "seat retired"));
            }
            // Complete validation precedes every warning, receipt or event write.
            for id in &request.messages {
                let kind: Option<String> = tx
                    .query_row(
                        "SELECT kind FROM messages WHERE id=?1",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(store_error)?;
                let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?;
                let eligible = receipt.as_ref().is_some_and(|receipt| {
                    matches!(
                        receipt.state,
                        EffectiveReceiptState::Pending | EffectiveReceiptState::Acknowledged
                    ) || (!displayed
                        && request.claim.harness == Harness::Human
                        && receipt.state == EffectiveReceiptState::NotRequired)
                });
                if kind.as_deref() != Some("ordinary") || !eligible {
                    return Err(api_error(
                        ErrorCode::InvalidRequest,
                        "ACK ID is not an addressed ordinary message",
                    ));
                }
            }
            Ok(())
        },
        |tx, decision| {
            failpoint!("ack.after_decision_sample", context.failpoint_scope());
            let actor = super::control::decide_accountable(
                tx,
                decision,
                permit,
                &request.claim,
                &seat,
                &request.operation,
                &ObligationRef::CheckIn(seat.clone()),
                &digest,
            )?;
            let mut newly = Vec::new();
            let mut prior = Vec::new();
            let mut by_thread: BTreeMap<ThreadId, Vec<MessageId>> = BTreeMap::new();
            for id in &request.messages {
                let thread: String = tx
                    .query_row(
                        "SELECT thread_id FROM messages WHERE id=?1",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared")
                    })?;
                if receipt.state == EffectiveReceiptState::Acknowledged {
                    prior.push(id.clone());
                } else {
                    newly.push(id.clone());
                    by_thread
                        .entry(ThreadId::new(thread))
                        .or_default()
                        .push(id.clone());
                }
            }
            // All late warnings precede settlement and its compact info event.
            for id in &newly {
                let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared")
                    })?;
                if receipt.state == EffectiveReceiptState::Pending
                    && effective_deadline(tx, &receipt)?
                        .is_some_and(|deadline| deadline <= decision.utc.0)
                {
                    schema::record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: id.clone(),
                            seat: seat.clone(),
                        },
                        &TimeBasis::Decision,
                        decision.utc,
                    )?;
                }
            }
            let observation = if displayed {
                let mut value: Value = serde_json::from_str(&actor.observation(decision.utc.0))
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "ACK observation malformed"))?;
                value["action_provenance"] = json!("cooperative_inbox_display");
                value.to_string()
            } else {
                actor.observation(decision.utc.0)
            };
            for id in &newly {
                let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared")
                    })?;
                let manifest: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM send_manifests WHERE message_id=?1)",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                if manifest {
                    tx.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES (?1,?2,'acked',?3,?4,?2,?5,?6,?7) ON CONFLICT(message_id,seat_id) DO UPDATE SET state='acked',ack_actor_seat_id=excluded.ack_actor_seat_id,ack_generation=excluded.ack_generation,ack_observation=excluded.ack_observation,acked_at=excluded.acked_at WHERE receipt_state.state='pending'",
                        params![id.as_str(),seat.as_str(),receipt.available_at,receipt.deadline_at,actor.binding_generation as i64,observation,decision.utc.0]).map_err(store_error)?;
                } else {
                    tx.execute("UPDATE receipts SET state='acked',ack_actor_seat_id=?1,ack_generation=?2,ack_observation=?3,acked_at=?4 WHERE message_id=?5 AND seat_id=?1 AND state='pending'",
                        params![seat.as_str(),actor.binding_generation as i64,observation,decision.utc.0,id.as_str()]).map_err(store_error)?;
                }
            }
            for thread in by_thread.keys() {
                schema::clear_warning_conditions_for_receipts(
                    tx,
                    thread.as_str(),
                    seat.as_str(),
                    decision.utc,
                )?;
            }
            for (thread, ids) in &by_thread {
                let mut payload = json!({"event":"ack", "seat":seat, "messages":ids, "decided_at":decision.utc.0}).to_string();
                if payload.len() > 4096 {
                    let digest = schema::canonical_digest(ids)?;
                    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                    payload = json!({"event":"ack", "seat":seat, "count":ids.len(), "message_ids_sha256":hash, "decided_at":decision.utc.0}).to_string();
                }
                let key = format!(
                    "ack:{}:{}:{}:{}:{}:{}",
                    seat.as_str().len(),
                    seat.as_str(),
                    request.operation.as_str().len(),
                    request.operation.as_str(),
                    thread.as_str().len(),
                    thread.as_str()
                );
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread,
                        key: &key,
                        kind: "info",
                        payload_json: &payload,
                        decision_at: decision.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM threads WHERE id=?1",
                        [thread.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
                schema::bump_filter_revision(tx, &instance, "directory", thread.as_str())?;
                tx.execute("INSERT INTO delivery_observations(seat_id,thread_id,acked) VALUES (?1,?2,?3) ON CONFLICT(seat_id,thread_id) DO UPDATE SET acked=acked+excluded.acked", params![seat.as_str(),thread.as_str(),ids.len() as i64]).map_err(store_error)?;
            }
            failpoint!("ack.before_commit", context.failpoint_scope());
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: newly,
                already_acknowledged: prior,
            }))
        },
    )?;
    failpoint!("ack.after_commit", context.failpoint_scope());
    Ok(result)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReceiptDueCursor {
    pub high_water_ordinal: i64,
    pub after_deadline: Option<i64>,
    pub after_ordinal: i64,
    pub sparse_high_water_rowid: i64,
    pub sparse_after_deadline: Option<i64>,
    pub sparse_after_message: Option<String>,
    pub sparse_after_seat: Option<String>,
    pub next_sparse: bool,
    /// Extension-lapse recheck state (`scan_extension_lapses`). `scan_due`
    /// carries it through every cursor reset; it is not part of the due scan.
    pub extension: ExtensionLapseCursor,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExtensionLapseCursor {
    /// Every row with extension_until <= through was rechecked by a completed walk.
    pub through: Option<i64>,
    /// Keyset position (extension_until, seat_id, thread_id) inside the walk in progress.
    pub after: Option<(i64, String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueScanResult {
    pub warnings: u16,
    pub inspected: u16,
    pub more: bool,
    pub next_ordinal: i64,
}

/// Scans pending, unmarked deadline rows through the partial due index. A
/// timer materializer inserts newly anchored logical receipts into this index.
pub fn scan_due(
    context: &StoreContext,
    conn: &mut Connection,
    limit: u16,
    cursor: &mut ReceiptDueCursor,
) -> Result<DueScanResult, ApiError> {
    if limit == 0 || limit > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid due scan limit",
        ));
    }
    let current = cursor.clone();
    let (result, next) = context.execute_decision(conn,
        |tx| {
            let physical_high = if current.high_water_ordinal == 0 {
                tx.query_row("SELECT COALESCE(MAX(ordinal),0) FROM receipts", [], |r| r.get::<_,i64>(0)).map_err(store_error)?
            } else { current.high_water_ordinal };
            let sparse_high = if current.sparse_high_water_rowid == 0 {
                tx.query_row("SELECT COALESCE(MAX(rowid),0) FROM receipt_state", [], |r| r.get::<_,i64>(0)).map_err(store_error)?
            } else { current.sparse_high_water_rowid };
            let has_sparse:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM receipt_state INDEXED BY receipt_state_required_due WHERE state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL)",[],|r|r.get(0)).map_err(store_error)?;
            let physical_cap = if has_sparse { if current.next_sparse {limit/2} else {limit.div_ceil(2)} } else {limit};
            let mut physical=Vec::new();
            if physical_cap>0 {
                let mut stmt=tx.prepare("SELECT ordinal,message_id,seat_id,deadline_at FROM receipts INDEXED BY receipts_required_due WHERE state='pending' AND ack_required=1 AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND (?1 IS NULL OR deadline_at>?1 OR (deadline_at=?1 AND ordinal>?2)) ORDER BY deadline_at,ordinal LIMIT ?3").map_err(store_error)?;
                physical=stmt.query_map(params![current.after_deadline,current.after_ordinal,i64::from(physical_cap)],|r|Ok((r.get::<_,i64>(0)?,MessageId::new(r.get::<_,String>(1)?),SeatId::new(r.get::<_,String>(2)?),r.get::<_,i64>(3)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)?;
            }
            let sparse_cap=limit-u16::try_from(physical.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"physical due batch too large"))?;
            let mut sparse=Vec::new();
            if sparse_cap>0 {
                let mut stmt=tx.prepare("SELECT rowid,message_id,seat_id,deadline_at FROM receipt_state INDEXED BY receipt_state_required_due WHERE state='pending' AND ack_required=1 AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND (?1 IS NULL OR deadline_at>?1 OR (deadline_at=?1 AND (message_id>?2 OR (message_id=?2 AND seat_id>?3)))) ORDER BY deadline_at,message_id,seat_id LIMIT ?4").map_err(store_error)?;
                sparse=stmt.query_map(params![current.sparse_after_deadline,current.sparse_after_message,current.sparse_after_seat,i64::from(sparse_cap)],|r|Ok((r.get::<_,i64>(0)?,MessageId::new(r.get::<_,String>(1)?),SeatId::new(r.get::<_,String>(2)?),r.get::<_,i64>(3)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)?;
            }
            Ok((physical_high,sparse_high,physical_cap,sparse_cap,physical,sparse))
        },
        |tx, decision, (physical_high,sparse_high,physical_cap,sparse_cap,physical,sparse)| {
            let mut inserted=0u16;
            let inspected=u16::try_from(physical.len()+sparse.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"due batch too large"))?;
            let physical_after=physical.last().map(|v|(v.3,v.0));
            let sparse_after=sparse.last().map(|v|(v.3,v.1.as_str().to_owned(),v.2.as_str().to_owned()));
            let physical_future=physical.iter().any(|v|v.3>decision.utc.0);
            let sparse_future=sparse.iter().any(|v|v.3>decision.utc.0);
            let physical_more=physical_cap>0 && physical.len()==usize::from(physical_cap) && !physical_future;
            let sparse_more=sparse_cap>0 && sparse.len()==usize::from(sparse_cap) && !sparse_future;
            for (source_high,candidates) in [(physical_high,physical),(sparse_high,sparse)] {
                for (ordinal,message,seat,deadline) in candidates {
                    let receipt=effective::effective_receipt(tx,message.as_str(),seat.as_str())?;
                    if ordinal<=source_high && deadline<=decision.utc.0 && match receipt.as_ref() { Some(v) if v.state==EffectiveReceiptState::Pending => effective_deadline(tx,v)?.is_some_and(|at|at<=decision.utc.0), _ => false } {
                        let outcome=schema::record_overdue_if_pending(tx,&ObligationRef::Receipt{message:message.clone(),seat:seat.clone()},&TimeBasis::Decision,decision.utc)?;
                        if outcome.inserted {
                            let (thread,instance):(String,String)=tx.query_row("SELECT m.thread_id,t.instance_id FROM messages m JOIN threads t ON t.id=m.thread_id WHERE m.id=?1",[message.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
                            schema::bump_filter_revision(tx,&instance,"inbox",seat.as_str())?;
                            schema::bump_filter_revision(tx,&instance,"directory",&thread)?;
                            inserted+=1;
                        }
                    }
                }
            }
            let more=physical_more||sparse_more;
            let next=if more {ReceiptDueCursor{
                high_water_ordinal:physical_high,
                after_deadline:physical_after.map(|v|v.0).or(current.after_deadline),
                after_ordinal:physical_after.map(|v|v.1).unwrap_or(current.after_ordinal),
                sparse_high_water_rowid:sparse_high,
                sparse_after_deadline:sparse_after.as_ref().map(|v|v.0).or(current.sparse_after_deadline),
                sparse_after_message:sparse_after.as_ref().map(|v|v.1.clone()).or(current.sparse_after_message.clone()),
                sparse_after_seat:sparse_after.map(|v|v.2).or(current.sparse_after_seat.clone()),
                next_sparse:!current.next_sparse,
                extension:current.extension.clone(),
            }} else {ReceiptDueCursor{extension:current.extension.clone(),..ReceiptDueCursor::default()}};
            Ok((DueScanResult{warnings:inserted,inspected,more,next_ordinal:next.after_ordinal},next))
        })?;
    *cursor = next;
    Ok(result)
}

/// Spec §8: max(frozen deadline, extension_until of the catch-up row for the
/// receipt's (seat, thread)). The (seat, thread) primary key makes that row
/// the latest one. Every overdue, warning, pending-receipt and soft-point
/// comparison goes through this. An ended row's past extension is harmless:
/// it is at or before now, so the receipt is overdue on its own.
pub fn effective_deadline(
    conn: &Connection,
    receipt: &effective::EffectiveReceipt,
) -> Result<Option<i64>, ApiError> {
    effective_deadline_for(
        conn,
        &receipt.seat_id,
        &receipt.thread_id,
        receipt.deadline_at,
    )
}

/// `effective_deadline` for a receipt known only by its (seat, thread, frozen
/// deadline).
pub fn effective_deadline_for(
    conn: &Connection,
    seat: &str,
    thread: &str,
    frozen: Option<i64>,
) -> Result<Option<i64>, ApiError> {
    let Some(frozen) = frozen else {
        return Ok(None);
    };
    let extension: Option<i64> = conn
        .query_row(
            "SELECT extension_until FROM catch_up WHERE seat_id=?1 AND thread_id=?2",
            params![seat, thread],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .flatten();
    Ok(Some(extension.map_or(frozen, |until| frozen.max(until))))
}

fn extension_end(base: UtcMillis, add_ms: u64) -> i64 {
    base.0
        .saturating_add(i64::try_from(add_ms).unwrap_or(i64::MAX))
}

/// Extension hooks the catch-up lifecycle calls (spec §7, §8). These own every
/// `catch_up.extension_until` write. Entry: entered_at + p99, never lowering;
/// the lifecycle skips it on re-entry after a stall with no block stored since.
pub fn extension_on_entry(
    tx: &Transaction<'_>,
    seat: &SeatId,
    thread: &ThreadId,
    entered_at: UtcMillis,
    p99_ms: u64,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET extension_until=MAX(COALESCE(extension_until,0),?3) WHERE seat_id=?1 AND thread_id=?2",
        params![seat.as_str(), thread.as_str(), extension_end(entered_at, p99_ms)],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Progress: now + p99 for every seat in active catch-up on the thread.
pub fn extension_on_progress(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    now: UtcMillis,
    p99_ms: u64,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET extension_until=MAX(COALESCE(extension_until,0),?2) WHERE thread_id=?1 AND state='active'",
        params![thread.as_str(), extension_end(now, p99_ms)],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Exit: the grace window after the row ends is set (spec §7), not raised.
pub fn extension_on_exit(
    tx: &Transaction<'_>,
    seat: &SeatId,
    thread: &ThreadId,
    now: UtcMillis,
    exit_grace_ms: u64,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET extension_until=?3 WHERE seat_id=?1 AND thread_id=?2",
        params![
            seat.as_str(),
            thread.as_str(),
            extension_end(now, exit_grace_ms)
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

const EXTENSION_RECHECK_ROWS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionLapseResult {
    pub warnings: u16,
    pub inspected: u16,
    pub more: bool,
}

/// Rechecks receipts whose extension has lapsed (spec §8). `scan_due` skips a
/// candidate whose effective deadline is still in the future and moves on, so
/// the lapse itself is found here: a bounded walk of `catch_up` through
/// `catch_up_extension_until` over `(through, now]`, re-evaluating each
/// row's pending, unwarned receipts on both receipt tables. Pages are keyed by
/// `(extension_until, seat_id, thread_id)` (`cursor.extension.after`); a short
/// page completes the walk and sets `cursor.extension.through` to `now - 1`, so
/// a completed walk is not repeated and only later lapses are scanned. The
/// `- 1` re-walks rows lapsing in that exact millisecond, which is idempotent
/// because warnings are unique per obligation. `through == None` (first run,
/// restart) walks everything once, paged. A UTC step backwards can leave a
/// later write's `extension_until <= through`; that lapse is still found
/// because `scan_due` re-walks every pending, unwarned, frozen-past receipt on
/// each completed pass and checks its effective deadline.
pub fn scan_extension_lapses(
    context: &StoreContext,
    conn: &mut Connection,
    cursor: &mut ReceiptDueCursor,
) -> Result<ExtensionLapseResult, ApiError> {
    let through = cursor.extension.through;
    let after = cursor.extension.after.clone();
    let (result, next_extension) = context.execute_decision(
        conn,
        |_| Ok(()),
        |tx, decision, ()| {
            let rows: Vec<(i64, String, String)> = {
                let mut stmt = tx
                    .prepare("SELECT extension_until,seat_id,thread_id FROM catch_up WHERE extension_until IS NOT NULL AND extension_until<=?1 AND (?2 IS NULL OR extension_until>?2) AND (?3 IS NULL OR extension_until>?3 OR (extension_until=?3 AND (seat_id>?4 OR (seat_id=?4 AND thread_id>?5)))) ORDER BY extension_until,seat_id,thread_id LIMIT ?6")
                    .map_err(store_error)?;
                stmt.query_map(params![
                    decision.utc.0,
                    through,
                    after.as_ref().map(|k| k.0),
                    after.as_ref().map(|k| k.1.as_str()),
                    after.as_ref().map(|k| k.2.as_str()),
                    EXTENSION_RECHECK_ROWS
                ], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .map_err(store_error)?
                .collect::<Result<_, _>>()
                .map_err(store_error)?
            };
            let mut warnings = 0u16;
            let mut inspected = 0u16;
            for (_, seat, thread) in &rows {
                let mut stmt = tx
                    .prepare("SELECT message_id FROM receipts WHERE seat_id=?1 AND thread_id=?2 AND state='pending' AND ack_required=1 AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND deadline_at<=?3 UNION SELECT s.message_id FROM receipt_state s JOIN messages m ON m.id=s.message_id WHERE s.seat_id=?1 AND m.thread_id=?2 AND s.state='pending' AND s.ack_required=1 AND s.warning_message_id IS NULL AND s.deadline_at IS NOT NULL AND s.deadline_at<=?3")
                    .map_err(store_error)?;
                let messages: Vec<String> = stmt
                    .query_map(params![seat, thread, decision.utc.0], |r| r.get(0))
                    .map_err(store_error)?
                    .collect::<Result<_, _>>()
                    .map_err(store_error)?;
                for message in messages {
                    inspected = inspected.saturating_add(1);
                    let outcome = schema::record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new(message),
                            seat: SeatId::new(seat.clone()),
                        },
                        &TimeBasis::Decision,
                        decision.utc,
                    )?;
                    if outcome.inserted {
                        bump_overdue_filters(tx, seat, thread)?;
                        warnings = warnings.saturating_add(1);
                    }
                }
            }
            let more = rows.len() as i64 == EXTENSION_RECHECK_ROWS;
            let next = if more {
                ExtensionLapseCursor {
                    through,
                    after: rows.last().cloned(),
                }
            } else {
                ExtensionLapseCursor {
                    through: Some(decision.utc.0 - 1),
                    after: None,
                }
            };
            Ok((
                ExtensionLapseResult {
                    warnings,
                    inspected,
                    more,
                },
                next,
            ))
        },
    )?;
    cursor.extension = next_extension;
    Ok(result)
}

fn bump_overdue_filters(tx: &Transaction<'_>, seat: &str, thread: &str) -> Result<(), ApiError> {
    let instance: String = tx
        .query_row(
            "SELECT instance_id FROM threads WHERE id=?1",
            [thread],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    schema::bump_filter_revision(tx, &instance, "inbox", seat)?;
    schema::bump_filter_revision(tx, &instance, "directory", thread)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/store/receipts.rs"]
mod tests;
