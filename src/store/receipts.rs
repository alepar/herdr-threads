//! Receipt settlement and availability transitions.

use crate::{
    ports::TimeBasis,
    protocol::{
        authority::{DecisionFence, MutationPermit, ObligationRef},
        commands::Ack,
        ids::{MessageId, SeatId, ThreadId},
        results::{AckResult, ApiError, CommandResult, ErrorCode},
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    connection::{DecisionInstant, StoreContext, api_error, store_error},
    effective::{self, EffectiveReceiptState},
    messages::{assert_actor_current, resolve_operation_seat},
    schema::{self, EventInput},
};

pub fn ack_payload(request: &Ack) -> Value {
    if !request.claim.instance.is_empty() {
        return json!({"kind":"ack", "messages":request.messages,"claim":request.claim});
    }
    json!({"kind":"ack", "messages":request.messages})
}

/// The callback supplies a current, local-only fence. `now` always comes from
/// the store's decision sample after validation and lock acquisition.
pub fn ack(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &Ack,
    permit: &mut MutationPermit,
    decision_fence: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<DecisionFence, ApiError>,
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
    let digest = schema::canonical_digest(&ack_payload(request))?;
    let cooperative = permit.cooperative_metadata();
    let seat = if cooperative.is_some() {
        request.claim.seat.clone()
    } else {
        resolve_operation_seat(conn, &request.claim.target, &request.operation)?
    };
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
                if kind.as_deref() != Some("ordinary")
                    || receipt
                        .as_ref()
                        .is_none_or(|r| r.state == EffectiveReceiptState::RecipientRetired)
                {
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
            let actor = if permit.cooperative_claim().is_some() {
                super::control::decide_accountable(
                    tx,
                    decision,
                    permit,
                    &request.claim,
                    &seat,
                    &request.operation,
                    &ObligationRef::CheckIn(seat.clone()),
                    &digest,
                    || decision_fence(tx, decision),
                )?
            } else {
                let mut fence = decision_fence(tx, decision)?;
                fence.now = decision.monotonic;
                let native_actor = permit
                    .consume(
                        &fence,
                        &request.operation,
                        &ObligationRef::CheckIn(seat.clone()),
                        &digest,
                    )
                    .map_err(|why| api_error(ErrorCode::CallerUnverified, why))?;
                if native_actor.seat != seat
                    || native_actor.native_session != request.claim.native_session
                    || native_actor.execution != request.claim.execution
                    || native_actor.harness != request.claim.harness
                {
                    return Err(api_error(
                        ErrorCode::CallerUnverified,
                        "permit actor does not match current claim target",
                    ));
                }
                assert_actor_current(tx, &seat, &request.claim.target, native_actor, &fence)?;
                super::control::AccountableActor::native(native_actor)
            };
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
                if receipt
                    .deadline_at
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
            let observation = actor.observation(decision.utc.0);
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
            let has_sparse:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM receipt_state WHERE state='pending' AND deadline_at IS NOT NULL AND warning_message_id IS NULL)",[],|r|r.get(0)).map_err(store_error)?;
            let physical_cap = if has_sparse { if current.next_sparse {limit/2} else {limit.div_ceil(2)} } else {limit};
            let mut physical=Vec::new();
            if physical_cap>0 {
                let mut stmt=tx.prepare("SELECT ordinal,message_id,seat_id,deadline_at FROM receipts WHERE state='pending' AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND (?1 IS NULL OR deadline_at>?1 OR (deadline_at=?1 AND ordinal>?2)) ORDER BY deadline_at,ordinal LIMIT ?3").map_err(store_error)?;
                physical=stmt.query_map(params![current.after_deadline,current.after_ordinal,i64::from(physical_cap)],|r|Ok((r.get::<_,i64>(0)?,MessageId::new(r.get::<_,String>(1)?),SeatId::new(r.get::<_,String>(2)?),r.get::<_,i64>(3)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)?;
            }
            let sparse_cap=limit-u16::try_from(physical.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"physical due batch too large"))?;
            let mut sparse=Vec::new();
            if sparse_cap>0 {
                let mut stmt=tx.prepare("SELECT rowid,message_id,seat_id,deadline_at FROM receipt_state WHERE state='pending' AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND (?1 IS NULL OR deadline_at>?1 OR (deadline_at=?1 AND (message_id>?2 OR (message_id=?2 AND seat_id>?3)))) ORDER BY deadline_at,message_id,seat_id LIMIT ?4").map_err(store_error)?;
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
                    if ordinal<=source_high && deadline<=decision.utc.0 && receipt.as_ref().is_some_and(|v|v.state==EffectiveReceiptState::Pending && v.deadline_at.is_some_and(|at|at<=decision.utc.0)) {
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
            }} else {ReceiptDueCursor::default()};
            Ok((DueScanResult{warnings:inserted,inspected,more,next_ordinal:next.after_ordinal},next))
        })?;
    *cursor = next;
    Ok(result)
}

#[cfg(test)]
#[path = "../../tests/store/receipts.rs"]
mod tests;
