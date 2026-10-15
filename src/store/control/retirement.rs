//! Seat retirement jobs.

use super::bump_member_directory;
use crate::ports::{
    ClosureEvidence, RETIREMENT_MAX_UNITS, RETIREMENT_WORK_MILLIS, RetirementJob, RetirementPhase,
    RetirementProgress, TimeBasis, WorkAdmission,
};
use crate::protocol::authority::ObligationRef;
use crate::protocol::ids::{InvitationId, MessageId, RetirementJobId, SeatId, ThreadId, prefix};
use crate::protocol::results::{ApiError, BoundedError, ErrorCode, MAX_LAST_ERROR_BYTES};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::effective::{self, EffectiveObservationSource};
use crate::store::schema::{self, EventInput};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub fn begin_retirement(
    context: &StoreContext,
    conn: &mut Connection,
    seat: SeatId,
    proof: ClosureEvidence,
) -> Result<RetirementJob, ApiError> {
    let proof_epoch = i64::try_from(proof.epoch).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "host epoch exceeds SQLite integer range",
        )
    })?;
    let proof_generation = i64::try_from(proof.generation).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "target generation exceeds SQLite integer range",
        )
    })?;
    use rusqlite::TransactionBehavior;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    if let Some((id, cutover, high_water, processed, status)) = tx.query_row(
        "SELECT id,cutover_at,high_water_ordinal,processed_units,status FROM retirements WHERE seat_id=?1",
        [seat.as_str()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?)),
    ).optional().map_err(store_error)? {
        tx.rollback().map_err(store_error)?;
        return Ok(RetirementJob { id: RetirementJobId::new(id), seat, retired_at: UtcMillis(cutover),
            phase: if status == "complete" { RetirementPhase::Complete } else { RetirementPhase::Warnings },
            after_ordinal: None, high_water_ordinal: high_water as u64, processed_units: processed as u64 });
    }
    type SeatRow = Option<(String, String, i64, i64, Option<String>, i64, i64)>;
    let current: SeatRow = tx.query_row(
        "SELECT s.instance_id,s.state,s.generation,s.target_generation,h.host_boot,h.host_epoch,h.host_id \
         FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
         WHERE s.id=?1 AND s.target_id=?2",
        params![seat.as_str(),proof.target.as_str()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).optional().map_err(store_error)?;
    let Some((instance, state, binding_generation, _seat_generation, boot, epoch, host_id)) =
        current
    else {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "closure target not current",
        ));
    };
    let observed = effective::effective_observation(
        &tx,
        crate::ports::HostId::from_row(host_id),
        proof.target.as_str(),
    )?;
    if state == "retired"
        || boot.as_deref() != Some(proof.host_boot.as_str())
        || epoch != proof_epoch
        || observed.as_ref().is_none_or(|observation| {
            observation.source != EffectiveObservationSource::NewerCurrentTarget
                || observation.host_boot != proof.host_boot.as_str()
                || observation.epoch != proof_epoch
                || observation.structural_generation != proof_generation
        })
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "closure proof changed",
        ));
    }
    if binding_generation == i64::MAX {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "seat binding generation exhausted",
        ));
    }
    let result = begin_retirement_fence(
        &tx,
        context.clock().utc_now(),
        seat,
        &instance,
        proof.host_boot.as_str(),
        proof_epoch,
        proof.target.as_str(),
        proof_generation,
    )?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}

/// Complete the already validated constant-size terminal fence in the caller's
/// writer transaction. Snapshot reconciliation uses this without a gap between
/// its active-generation CAS and the retirement decision.
// Allowed: fence inputs mirror the retirement row's columns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_retirement_fence(
    tx: &Transaction<'_>,
    cutover: UtcMillis,
    seat: SeatId,
    instance: &str,
    boot: &str,
    epoch: i64,
    target: &str,
    generation: i64,
) -> Result<RetirementJob, ApiError> {
    let cutover_seq = schema::next_decision_seq(tx, instance)?;
    schema::apply_eligibility_transition(tx, instance, |tx| {
        schema::clear_open_unavailability_for_seat(tx, seat.as_str(), cutover)?;
        let changed = tx.execute("UPDATE seats SET state='retired',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL,generation=generation+1,retired_at=?1,retired_seq=?2 WHERE id=?3 AND state!='retired'",
            params![cutover.0,cutover_seq as i64,seat.as_str()]).map_err(store_error)?;
        // Every open binding disconnects (`seat_retired`, side-effect hooks run) and ends through
        // the sole transition; an already disconnected binding keeps its own reason.
        crate::store::bindings::end_open_bindings(
            tx,
            &seat,
            crate::store::bindings::DisconnectReason::SeatRetired,
            cutover,
        )?;
        crate::store::catch_up::supersede_ended_bindings(tx, &seat, cutover)?;
        tx.execute(
            "DELETE FROM warning_offer WHERE seat_id=?1",
            [seat.as_str()],
        )
        .map_err(store_error)?;
        tx.execute(
            "DELETE FROM in_turn_offer WHERE seat_id=?1",
            [seat.as_str()],
        )
        .map_err(store_error)?;
        Ok(changed == 1)
    })?;
    tx.execute("UPDATE wake_work SET reason_bits=0,reservation_id=NULL,reservation_boot=NULL,binding_generation=NULL WHERE seat_id=?1",[seat.as_str()]).map_err(store_error)?;
    let job = RetirementJobId::new(crate::store::public_ids::fresh(
        tx,
        prefix::RETIREMENT,
        &[("retirements", "id", prefix::RETIREMENT)],
    )?);
    tx.execute("INSERT INTO retirements(id,seat_id,cutover_at,closure_boot,closure_epoch,closure_target,closure_generation) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![job.as_str(),seat.as_str(),cutover.0,boot,epoch,target,generation]).map_err(store_error)?;
    // The retired seat's own server advances its lifecycle; a seat with no server (remote) leaves
    // every local host's lifecycle alone.
    if let Some(host) = tx
        .query_row(
            "SELECT host_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get::<_, Option<i64>>(0),
        )
        .map_err(store_error)?
    {
        schema::bump_host_lifecycle_revision(tx, crate::ports::HostId::from_row(host))?;
    }
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    bump_member_directory(tx, instance, &seat)?;
    Ok(RetirementJob {
        id: job,
        seat,
        retired_at: cutover,
        phase: RetirementPhase::Warnings,
        after_ordinal: None,
        high_water_ordinal: 0,
        processed_units: 0,
    })
}

pub fn advance_retirement(
    context: &StoreContext,
    conn: &mut Connection,
    job: RetirementJobId,
    _admission: WorkAdmission,
    budget: &CallBudget,
) -> Result<RetirementProgress, ApiError> {
    if budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::DeadlineExceeded,
            "retirement work budget exhausted",
        ));
    }
    context.execute_budgeted_decision(conn, budget,
        |tx| {
            type RowColumns = Option<(String,i64,i64,String,i64,i64,i64,String,Option<String>)>;
            let row: RowColumns = tx.query_row(
                "SELECT seat_id,cutover_at,thread_ordinal,phase,obligation_ordinal,high_water_ordinal,processed_units,status,last_error FROM retirements WHERE id=?1",
                [job.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
            ).optional().map_err(store_error)?;
            row.ok_or_else(|| api_error(ErrorCode::NotFound,"retirement job missing"))
        },
        |tx, at, (seat,cutover,mut thread_ordinal,mut phase,mut obligation_ordinal,mut high_water,prior_total,status,prior_error)| {
            if status == "complete" { return Ok(Ok(RetirementProgress { job:job.clone(),processed_this_turn:0,processed_total:prior_total as u64,
                complete:true,warning_history_complete:true,last_error:None })); }
            if high_water == 0 {
                high_water = tx.query_row("SELECT coalesce(max(ordinal),0) FROM memberships WHERE seat_id=?1",[seat.as_str()],|r|r.get(0)).map_err(store_error)?;
            }
            let start = context.clock().monotonic_now();
            let mut units = 0u8;
            let mut warnings = 0i64;
            let mut retired = 0i64;
            let mut audits = 0i64;
            let mut failure: Option<ApiError> = None;
            // The quantum stays atomic: a failed unit rolls back every effect and
            // cursor move of this quantum. Only the bounded diagnostic commits,
            // leaving the job pending and visibly failed until later progress.
            tx.execute_batch("SAVEPOINT retirement_quantum").map_err(store_error)?;
            while (units as usize) < RETIREMENT_MAX_UNITS {
                if budget.is_exhausted(context.clock()) || context.clock().monotonic_now().0.saturating_sub(start.0) >= RETIREMENT_WORK_MILLIS { break; }
                let step = (|| -> Result<(), ApiError> {
                match phase.as_str() {
                    "select_thread" => {
                        let next: Option<i64> = tx.query_row("SELECT ordinal FROM memberships WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                            params![seat,thread_ordinal,high_water],|r|r.get(0)).optional().map_err(store_error)?;
                        if let Some(next) = next { thread_ordinal=next; phase="invitations".into(); obligation_ordinal=0; }
                        else { phase="complete".into(); }
                    }
                    "invitations" | "receipts" => {
                        let thread: String = tx.query_row("SELECT thread_id FROM memberships WHERE ordinal=?1 AND seat_id=?2",
                            params![thread_ordinal,seat],|r|r.get(0)).map_err(store_error)?;
                        if phase == "invitations" {
                            let next: Option<(i64,String)> = tx.query_row("SELECT i.ordinal,i.id FROM invitations i WHERE i.seat_id=?1 AND i.thread_id=?2 AND i.state='pending' AND i.ordinal>?3 AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) ORDER BY i.ordinal LIMIT 1",
                                params![seat,thread,obligation_ordinal],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
                            if let Some((ordinal,id)) = next {
                                let invitation = InvitationId::new(id);
                                let outcome = schema::record_overdue_if_pending(tx,&ObligationRef::Invitation(invitation.clone()),&TimeBasis::Retirement(job.clone()),at.utc)?;
                                warnings += i64::from(outcome.inserted);
                                tx.execute("UPDATE invitations SET state='recipient_retired',retired_at=?1 WHERE id=?2 AND state='pending'",
                                    params![cutover,invitation.as_str()]).map_err(store_error)?;
                                schema::clear_warning_condition_for_invitation(tx, invitation.as_str(), at.utc)?;
                                obligation_ordinal=ordinal; retired+=1;
                            } else { phase="receipts".into(); obligation_ordinal=0; }
                        } else {
                            let next: Option<(i64,String)> = tx.query_row("SELECT ordinal,message_id FROM receipts WHERE seat_id=?1 AND thread_id=?2 AND state='pending' AND ack_required=1 AND ordinal>?3 ORDER BY ordinal LIMIT 1",
                                params![seat,thread,obligation_ordinal],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
                            if let Some((ordinal,id)) = next {
                                let message = MessageId::new(id);
                                let receipt = crate::store::effective::effective_receipt(tx,message.as_str(),&seat)?
                                    .ok_or_else(||api_error(ErrorCode::StoreCorrupt,"physical receipt missing"))?;
                                if receipt.state != crate::store::effective::EffectiveReceiptState::NotRequired {
                                    let outcome = schema::record_overdue_if_pending(tx,&ObligationRef::Receipt { message:message.clone(),seat:SeatId::new(&seat) },&TimeBasis::Retirement(job.clone()),at.utc)?;
                                    warnings += i64::from(outcome.inserted);
                                    tx.execute("UPDATE receipts SET state='recipient_retired',retired_at=?1 WHERE message_id=?2 AND seat_id=?3 AND state='pending'",
                                        params![cutover,message.as_str(),seat]).map_err(store_error)?;
                                    schema::clear_warning_conditions_for_receipts(tx, &thread, &seat, at.utc)?;
                                    retired+=1;
                                }
                                obligation_ordinal=ordinal;
                            } else { phase="logical_receipts".into(); obligation_ordinal=0; }
                        }
                    }
                    "logical_receipts" => {
                        let thread:String=tx.query_row("SELECT thread_id FROM memberships WHERE ordinal=?1 AND seat_id=?2",
                            params![thread_ordinal,seat],|r|r.get(0)).map_err(store_error)?;
                        type NextColumns = Option<(i64,Option<String>,Option<i64>,Option<String>)>;
                        let next:NextColumns=tx.query_row(
                            "SELECT pr.ordinal,sm.message_id,sm.decision_seq,rs.state FROM prepared_recipients pr \
                             LEFT JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id \
                             LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id \
                             WHERE pr.seat_id=?1 AND pr.thread_id=?2 AND pr.ack_required=1 AND pr.ordinal>?3 ORDER BY pr.ordinal LIMIT 1",
                            params![seat,thread,obligation_ordinal],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                            .optional().map_err(store_error)?;
                        if let Some((ordinal,message,send_seq,state))=next {
                            obligation_ordinal=ordinal;
                            let cutover_seq:i64=tx.query_row("SELECT retired_seq FROM seats WHERE id=?1",[&seat],|r|r.get(0)).map_err(store_error)?;
                            if let (Some(message),Some(send_seq))=(message,send_seq)
                                && send_seq<cutover_seq && state.as_deref().is_none_or(|status|status=="pending") {
                                    let message=MessageId::new(message);
                                    let receipt=crate::store::effective::effective_receipt(tx,message.as_str(),&seat)?
                                        .ok_or_else(||api_error(ErrorCode::StoreCorrupt,"published recipient lacks logical receipt"))?;
                                    if receipt.state != crate::store::effective::EffectiveReceiptState::NotRequired {
                                        let outcome=schema::record_overdue_if_pending(tx,&ObligationRef::Receipt {message:message.clone(),seat:SeatId::new(&seat)},
                                            &TimeBasis::Retirement(job.clone()),at.utc)?;
                                        warnings+=i64::from(outcome.inserted);
                                        tx.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,retired_at) VALUES (?1,?2,'recipient_retired',?3,?4,?5) \
                                            ON CONFLICT(message_id,seat_id) DO UPDATE SET state='recipient_retired',available_at=COALESCE(receipt_state.available_at,excluded.available_at),deadline_at=COALESCE(receipt_state.deadline_at,excluded.deadline_at),retired_at=excluded.retired_at WHERE receipt_state.state='pending'",
                                            params![message.as_str(),seat,receipt.available_at,receipt.deadline_at,cutover]).map_err(store_error)?;
                                        schema::clear_warning_conditions_for_receipts(tx, &thread, &seat, at.utc)?;
                                        retired+=1;
                                    }
                                }
                        } else {phase="audit".into();obligation_ordinal=0;}
                    }
                    "audit" => {
                        let thread: String = tx.query_row("SELECT thread_id FROM memberships WHERE ordinal=?1 AND seat_id=?2",
                            params![thread_ordinal,seat],|r|r.get(0)).map_err(store_error)?;
                        let cutover_seq:i64=tx.query_row("SELECT retired_seq FROM seats WHERE id=?1",[&seat],|r|r.get(0)).map_err(store_error)?;
                        tx.execute("UPDATE requirement_episodes SET state='retired',revision=revision+1,retired_at=?1 WHERE thread_id=?2 AND seat_id=?3 AND state IN ('pending','accepted')",
                            params![cutover,thread,seat]).map_err(store_error)?;
                        tx.execute("UPDATE membership_intervals SET left_seq=?1 WHERE thread_id=?2 AND seat_id=?3 AND left_seq IS NULL",
                            params![cutover_seq,thread,seat]).map_err(store_error)?;
                        tx.execute("UPDATE memberships SET state='retired',voluntary_state='retired',retired_at=?1 WHERE ordinal=?2 AND state!='retired'",
                            params![cutover,thread_ordinal]).map_err(store_error)?;
                        let thread = ThreadId::new(thread);
                        schema::bump_membership_revision(tx,&thread)?;
                        let payload = serde_json::json!({"action":"retire_seat","seat":seat,"cutover_at":cutover,"materialized_at":at.utc.0}).to_string();
                        let key = schema::retirement_audit_key(&job,&thread);
                        let (message,inserted) = schema::append_event_once(tx,EventInput { thread:&thread,key:&key,kind:"info",payload_json:&payload,
                            decision_at:at.utc,source_message:None,source_invitation:None })?;
                        tx.execute("INSERT OR IGNORE INTO retirement_audits(job_id,thread_id,message_id) VALUES (?1,?2,?3)",
                            params![job.as_str(),thread.as_str(),message.as_str()]).map_err(store_error)?;
                        audits += i64::from(inserted); phase="select_thread".into(); obligation_ordinal=0;
                    }
                    _ => return Err(api_error(ErrorCode::StoreCorrupt,"invalid retirement phase")),
                }
                Ok(())
                })();
                if let Err(error) = step {
                    failure = Some(error);
                    break;
                }
                units+=1;
                if phase == "complete" { break; }
            }
            if let Some(error) = failure {
                tx.execute_batch("ROLLBACK TO retirement_quantum; RELEASE retirement_quantum").map_err(store_error)?;
                let diagnostic = bounded_retirement_diagnostic(&error.detail);
                // If even the diagnostic cannot be written, nothing commits and
                // the caller still receives the original failure.
                if tx.execute("UPDATE retirements SET last_error=?1 WHERE id=?2 AND status='pending'",
                    params![diagnostic,job.as_str()]).is_err() {
                    return Err(error);
                }
                return Ok(Err(error));
            }
            tx.execute_batch("RELEASE retirement_quantum").map_err(store_error)?;
            let complete = phase == "complete";
            // Committed progress clears a retained diagnostic; an idle quantum
            // leaves it unchanged.
            let progressed = units > 0;
            tx.execute("UPDATE retirements SET thread_ordinal=?1,phase=?2,obligation_ordinal=?3,high_water_ordinal=?4,processed_units=processed_units+?5,warnings_added=warnings_added+?6,obligations_retired=obligations_retired+?7,audits_added=audits_added+?8,status=?9,last_error=CASE WHEN ?11 THEN NULL ELSE last_error END WHERE id=?10",
                params![thread_ordinal,phase,obligation_ordinal,high_water,i64::from(units),warnings,retired,audits,if complete {"complete"} else {"pending"},job.as_str(),progressed]).map_err(store_error)?;
            let last_error = if progressed { None } else { prior_error }
                .map(BoundedError::parse).transpose()
                .map_err(|e| api_error(ErrorCode::StoreCorrupt, e))?;
            Ok(Ok(RetirementProgress { job:job.clone(),processed_this_turn:units,processed_total:(prior_total+i64::from(units)) as u64,
                complete,warning_history_complete:complete,last_error }))
        })?
}

/// Persisted retirement diagnostics keep at most 256 characters and never
/// exceed the 512-byte column and wire bound.
fn bounded_retirement_diagnostic(detail: &str) -> String {
    let mut text: String = detail.chars().take(256).collect();
    if text.len() > MAX_LAST_ERROR_BYTES {
        text.truncate(text.floor_char_boundary(MAX_LAST_ERROR_BYTES));
    }
    text
}
