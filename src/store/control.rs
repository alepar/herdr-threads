//! Thread and membership controls.

use crate::{
    ports::{
        ClosureEvidence, RETIREMENT_MAX_UNITS, RETIREMENT_WORK_MILLIS, RetirementJob,
        RetirementPhase, RetirementProgress, TimeBasis, WorkAdmission,
    },
    protocol::{
        authority::{MutationPermit, ObligationRef, OperatorActor},
        commands::{
            Accept, AcceptRequired, CreateThread, Invite, Leave, OperatorOrphanInvite, SetTopic,
            ThreadMutation,
        },
        ids::{InvitationId, MessageId, RetirementJobId, SeatId, ThreadId, prefix},
        results::{ApiError, BoundedError, CommandResult, ErrorCode, MAX_LAST_ERROR_BYTES},
        time::{CallBudget, UtcMillis},
    },
    store::{
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        effective::{self, EffectiveObservationSource},
        schema::{self, EventInput},
    },
};
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
    let current: Option<(String,String,i64,i64,Option<String>,i64)> = tx.query_row(
        "SELECT s.instance_id,s.state,s.generation,s.target_generation,h.host_boot,h.host_epoch \
         FROM seats s JOIN host_instances h ON h.id=s.instance_id \
         WHERE s.id=?1 AND s.target_id=?2",
        params![seat.as_str(),proof.target.as_str()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional().map_err(store_error)?;
    let Some((instance, state, binding_generation, _seat_generation, boot, epoch)) = current else {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "closure target not current",
        ));
    };
    let observed = effective::effective_observation(&tx, &instance, proof.target.as_str())?;
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
        tx.execute(
            "UPDATE occupant_bindings SET ended_at=?1 WHERE seat_id=?2 AND ended_at IS NULL",
            params![cutover.0, seat.as_str()],
        )
        .map_err(store_error)?;
        super::catch_up::supersede_stale(tx, &seat, cutover)?;
        tx.execute(
            "DELETE FROM warning_offer WHERE seat_id=?1",
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
    schema::bump_lifecycle_revision(tx, instance)?;
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

fn validate_actor(tx: &Transaction<'_>, seat: &SeatId, target: &str) -> Result<String, ApiError> {
    let row: Option<(String, String, String)> = tx
        .query_row(
            "SELECT instance_id, state, target_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((instance, state, saved_target)) = row else {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "caller seat missing",
        ));
    };
    if state != "resolved" || saved_target != target {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "caller target not resolved",
        ));
    }
    Ok(instance)
}

fn joined_in_thread(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<bool, ApiError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id \
        WHERE m.thread_id=?1 AND m.seat_id=?2 AND m.state='joined' AND s.state!='retired')",
        params![thread.as_str(), seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

fn bump_member_directory(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
) -> Result<(), ApiError> {
    schema::bump_filter_revision(
        tx,
        instance,
        "directory",
        &format!("member:{}", seat.as_str()),
    )
}

/// Voluntary self-enrollment, distinct from invitation or requirement consent.
pub fn join(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::Join,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("join", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            let instance = validate_actor(tx, &caller, command.claim.target.as_str())?;
            let archived: Option<bool> = tx
                .query_row(
                    "SELECT archived FROM threads WHERE id=?1 AND instance_id=?2",
                    params![command.thread.as_str(), instance],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            match archived {
                None => {
                    return Err(api_error(
                        ErrorCode::NotFound,
                        "thread not found in caller instance",
                    ));
                }
                Some(true) => {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "thread is archived; ask a joined member or service owner to reopen it",
                    ));
                }
                Some(false) => {}
            }
            if let Some(required) =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.state == crate::protocol::service::RequirementState::Pending
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    "pending required invitation; reread thread participants and use accept-required with its exact revision",
                ));
            }
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",
                params![command.thread.as_str(), caller.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if pending {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "pending invitation; use accept THREAD or reject its exact invitation before joining",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            if joined_in_thread(tx, &command.thread, &caller)? {
                return Ok(CommandResult::Joined(command.thread.clone()));
            }
            let episode =
                super::service_controls::next_invitation_episode(tx, &command.thread, &caller)?;
            let instance = &command.claim.instance;
            let seq = schema::next_decision_seq(tx, instance)?;
            tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at) VALUES (?1,?2,?3,'joined','joined',?4) ON CONFLICT(thread_id,seat_id) DO UPDATE SET episode=excluded.episode,state='joined',voluntary_state='joined',joined_at=excluded.joined_at,left_at=NULL",
                params![command.thread.as_str(),caller.as_str(),episode,decision.utc.0]).map_err(store_error)?;
            tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                params![command.thread.as_str(),caller.as_str(),episode,seq as i64]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"join", "seat":caller.as_str(),
                "generation":actor.binding_generation,"observation":actor.provenance})
            .to_string();
            schema::append_attributed_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("join:{}:{}", command.operation.as_str(), caller.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: None,
                },
                seq,
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, instance, "directory", "all")?;
            bump_member_directory(tx, instance, &caller)?;
            schema::bump_filter_revision(tx, instance, "inbox", caller.as_str())?;
            Ok(CommandResult::Joined(command.thread.clone()))
        },
    )
}

pub fn accept(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Accept,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("accept", command)?;
    let invitation: String = conn.query_row("SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) ORDER BY i.episode DESC LIMIT 1",
        params![command.thread.as_str(),caller.as_str()], |r| r.get(0)).optional().map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound,format!("current invitation missing: seat {} has no invitation in thread {}; pass a thread ID listed by herdr-threads inbox",caller.as_str(),command.thread.as_str())))?;
    let invitation = InvitationId::new(invitation);
    let obligation = ObligationRef::Invitation(invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, target)?;
            if let Some(current) =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && current.invitation == invitation
                && current.state == crate::protocol::service::RequirementState::Pending
            {
                return Err(api_error(
                    ErrorCode::StaleRequirementAcceptance,
                    format!(
                        "reread thread participants and accept the current requirement revision: {}",
                        serde_json::to_string(&current)
                            .map_err(|e| api_error(ErrorCode::StoreCorrupt, e.to_string()))?
                    ),
                ));
            }
            let invitation_seat: Option<String> = tx
                .query_row(
                    "SELECT seat_id FROM invitations WHERE id=?1 AND thread_id=?2",
                    params![invitation.as_str(), command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if invitation_seat.as_deref() != Some(caller.as_str()) {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "invitation is not addressed to caller",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let (thread, state): (String, String) = tx
                .query_row(
                    "SELECT thread_id,state FROM invitations WHERE id=?1",
                    [invitation.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            let rejected: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM invitation_rejections WHERE invitation_id=?1)",
                    [invitation.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if rejected {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "invitation was rejected; a fresh invitation is required",
                ));
            }
            if state == "accepted" {
                return Ok(CommandResult::Accepted(invitation.clone().into()));
            }
            if state != "pending" {
                return Err(api_error(ErrorCode::Conflict, "invitation is terminal"));
            }
            let cancelled: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM invitation_cancellations WHERE invitation_id=?1)",
                    [invitation.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if cancelled {
                return Err(api_error(ErrorCode::Conflict, "invitation was cancelled"));
            }
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            tx.execute("UPDATE invitations SET state='accepted', accepted_at=?1,accepted_actor_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5",
                params![decision.utc.0,caller.as_str(),generation,observation,invitation.as_str()]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(tx, invitation.as_str(), decision.utc)?;
            let thread = ThreadId::new(thread);
            let already_joined = joined_in_thread(tx, &thread, &caller)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let event_seq = schema::next_decision_seq(tx, &instance)?;
            // This invitation needs its own acceptance event even when another
            // invitation already joined the seat. The join interval belongs to
            // the first transition into Joined and must keep that frontier.
            if !already_joined {
                let episode: i64 = tx
                    .query_row(
                        "SELECT episode FROM invitations WHERE id=?1",
                        [invitation.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("UPDATE memberships SET state='joined',voluntary_state='joined',episode=?1,joined_at=?2,left_at=NULL WHERE thread_id=?3 AND seat_id=?4",
                    params![episode,decision.utc.0,thread.as_str(),caller.as_str()]).map_err(store_error)?;
                tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                    params![thread.as_str(),caller.as_str(),episode,event_seq as i64]).map_err(store_error)?;
            }
            let payload = serde_json::json!({"action":"accept", "seat":caller.as_str(), "invitation":invitation.as_str()}).to_string();
            schema::append_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &thread,
                    key: &format!("accept:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
                event_seq,
            )?;
            schema::bump_membership_revision(tx, &thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &caller)?;
            schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            Ok(CommandResult::Accepted(invitation.clone().into()))
        },
    )
}

/// Explicit native consent to the exact requirement revision shown to the
/// caller. An ordinary acceptance can never create this provenance.
/// Reject an exact invitation in the same canonical accountable transaction as acceptance.
pub fn reject(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::Reject,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    crate::protocol::commands::validate_rejection_reason(&command.reason)
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("reject", command)?;
    let obligation = ObligationRef::Invitation(command.invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, command.claim.target.as_str())?;
            let recipient: Option<String> = tx
                .query_row(
                    "SELECT seat_id FROM invitations WHERE id=?1 AND thread_id=?2",
                    params![command.invitation.as_str(), command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if recipient.as_deref() != Some(caller.as_str()) {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "invitation is not addressed to caller in this thread",
                ));
            }
            if let Some(required) =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.invitation == command.invitation
                && matches!(
                    required.state,
                    crate::protocol::service::RequirementState::Pending
                        | crate::protocol::service::RequirementState::Accepted
                )
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    format!(
                        "invitation {} is required by {}; ask that service owner to release requirement {} before rejecting; reread thread participants afterward",
                        command.invitation.as_str(),
                        required.issuer.as_str(),
                        required.requirement.as_str(),
                    ),
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            type Saved = (String, i64, String, i64, String);
            let saved: Option<Saved> = tx.query_row(
                "SELECT actor_seat_id,generation,observation,rejected_at,reason FROM invitation_rejections WHERE invitation_id=?1",
                [command.invitation.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).optional().map_err(store_error)?;
            if let Some((seat, generation, observation, at, reason)) = saved {
                if reason != command.reason {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "invitation already rejected with a different reason",
                    ));
                }
                return Ok(CommandResult::Rejected(
                    crate::protocol::results::InvitationRejection {
                        invitation: command.invitation.clone(),
                        actor: SeatId::new(seat),
                        generation: generation as u64,
                        observation,
                        rejected_at: UtcMillis(at),
                        reason,
                    },
                ));
            }
            let pending: bool = tx.query_row(
                "SELECT state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations WHERE invitation_id=?1) AND NOT EXISTS(SELECT 1 FROM invitation_rejections WHERE invitation_id=?1) FROM invitations WHERE id=?1",
                [command.invitation.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if !pending {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "invitation is terminal or cancelled",
                ));
            }
            // Lateness is retained, never described as expiration or acceptance.
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            tx.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,?2,?3,?4,?5,?6)",
                params![command.invitation.as_str(),command.reason,decision.utc.0,caller.as_str(),generation,observation]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(
                tx,
                command.invitation.as_str(),
                decision.utc,
            )?;
            // Only an invited voluntary marker is cleared; no join or leave interval is fabricated.
            // Restore a prior left state only with its recorded closed interval and leave event.
            let prior_left: Option<i64> = tx.query_row(
                "SELECT m.decision_at FROM membership_intervals mi JOIN messages m ON m.thread_id=mi.thread_id AND m.decision_seq=mi.left_seq WHERE mi.thread_id=?1 AND mi.seat_id=?2 AND mi.left_seq IS NOT NULL AND json_extract(m.event_json,'$.action')='leave' AND json_extract(m.event_json,'$.seat')=?2 ORDER BY mi.episode DESC LIMIT 1",
                params![command.thread.as_str(),caller.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            tx.execute("UPDATE memberships SET voluntary_state=?1,left_at=?2 WHERE thread_id=?3 AND seat_id=?4 AND voluntary_state='invited' AND episode=(SELECT episode FROM invitations WHERE id=?5) AND NOT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?3 AND i.seat_id=?4 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",
                params![if prior_left.is_some() { "left" } else { "absent" },prior_left,command.thread.as_str(),caller.as_str(),command.invitation.as_str()]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"reject","seat":caller.as_str(),"invitation":command.invitation.as_str(),"reason":command.reason}).to_string();
            schema::append_attributed_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("reject:{}", command.invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&command.invitation),
                },
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, &command.claim.instance, "directory", "all")?;
            bump_member_directory(tx, &command.claim.instance, &caller)?;
            schema::bump_filter_revision(tx, &command.claim.instance, "inbox", caller.as_str())?;
            Ok(CommandResult::Rejected(
                crate::protocol::results::InvitationRejection {
                    invitation: command.invitation.clone(),
                    actor: caller.clone(),
                    generation: generation as u64,
                    observation,
                    rejected_at: decision.utc,
                    reason: command.reason.clone(),
                },
            ))
        },
    )
}

pub fn accept_required(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &AcceptRequired,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("accept_required", command)?;
    let obligation = ObligationRef::Invitation(command.invitation.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, command.claim.target.as_str())?;
            let current =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                    .ok_or_else(|| {
                        api_error(ErrorCode::NotFound, "effective requirement missing")
                    })?;
            let repeat_accepted = current.state
                == crate::protocol::service::RequirementState::Accepted
                && current.requirement == command.requirement
                && current.invitation == command.invitation
                && current.thread == command.thread
                && current.seat == caller
                && command.claim.seat == caller
                && (command.expected_revision == current.revision
                    || command.expected_revision.checked_add(1) == Some(current.revision));
            if !repeat_accepted {
                command.check_current(&current).map_err(|_| {
                    api_error(
                        ErrorCode::StaleRequirementAcceptance,
                        format!(
                            "reread thread participants and accept the current requirement revision: {}",
                            serde_json::to_string(&current)
                                .unwrap_or_else(|_| "current requirement unavailable".into())
                        ),
                    )
                })?;
            }
            Ok(())
        },
        |tx, decision| {
            if let Some(current) =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && current.state == crate::protocol::service::RequirementState::Accepted
            {
                return Ok(CommandResult::RequiredAccepted(current));
            }
            let actor = decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation = actor.observation(decision.utc.0);
            schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, decision.utc)?;
            tx.execute("UPDATE invitations SET state='accepted',accepted_at=?1,accepted_actor_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending'",
                params![decision.utc.0,caller.as_str(),generation,observation,command.invitation.as_str()]).map_err(store_error)?;
            schema::clear_warning_condition_for_invitation(
                tx,
                command.invitation.as_str(),
                decision.utc,
            )?;
            tx.execute("UPDATE requirement_episodes SET state='accepted',revision=revision+1,accepted_at=?1,accepted_by_seat_id=?2,accepted_generation=?3,accepted_observation=?4 WHERE id=?5 AND state='pending' AND revision=?6",
                params![decision.utc.0,caller.as_str(),generation,observation,command.requirement.as_str(),command.expected_revision as i64]).map_err(store_error)?;
            let joined = joined_in_thread(tx, &command.thread, &caller)?;
            if !joined {
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let seq = schema::next_decision_seq(tx, &instance)?;
                let episode: i64 = tx
                    .query_row(
                        "SELECT episode FROM invitations WHERE id=?1",
                        [command.invitation.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("UPDATE memberships SET state='joined',voluntary_state='joined',episode=?1,joined_at=?2,left_at=NULL WHERE thread_id=?3 AND seat_id=?4",
                    params![episode,decision.utc.0,command.thread.as_str(),caller.as_str()]).map_err(store_error)?;
                tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,?3,?4)",
                    params![command.thread.as_str(),caller.as_str(),episode,seq as i64]).map_err(store_error)?;
            }
            let payload = serde_json::json!({"action":"accept_required","seat":caller.as_str(),
                "requirement":command.requirement.as_str(),"invitation":command.invitation.as_str()}).to_string();
            schema::append_attributed_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("accept_required:{}", command.requirement.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&command.invitation),
                },
                crate::protocol::service::EventAuthor::Native(caller.clone()),
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &caller)?;
            schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            let current =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "accepted requirement disappeared")
                    })?;
            Ok(CommandResult::RequiredAccepted(current))
        },
    )
}

pub fn leave(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Leave,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("leave", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, target)?;
            if let Some(required) =
                super::service_substrate::current_requirement(tx, &command.thread, &caller)?
                && required.state == crate::protocol::service::RequirementState::Accepted
            {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    format!(
                        "thread {} is required by {}; ask the service owner to release requirement {}",
                        command.thread.as_str(),
                        required.issuer.as_str(),
                        required.requirement.as_str()
                    ),
                ));
            }
            let state: Option<String> = tx
                .query_row(
                    "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), caller.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if !matches!(state.as_deref(), Some("joined" | "left")) {
                return Err(api_error(ErrorCode::Conflict, "caller is not joined"));
            }
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let changed = tx.execute("UPDATE memberships SET state='left',voluntary_state='left',left_at=?1 WHERE thread_id=?2 AND seat_id=?3 AND state='joined'", params![decision.utc.0, command.thread.as_str(), caller.as_str()]).map_err(store_error)?;
            if changed != 0 {
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                let left_seq = schema::next_decision_seq(tx, &instance)?;
                tx.execute("UPDATE membership_intervals SET left_seq=?1 WHERE thread_id=?2 AND seat_id=?3 AND left_seq IS NULL",
                    params![left_seq as i64,command.thread.as_str(),caller.as_str()]).map_err(store_error)?;
                let payload =
                    serde_json::json!({"action":"leave", "seat":caller.as_str()}).to_string();
                schema::append_event_once_with_decision_seq(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("leave:{}:{}", command.operation.as_str(), caller.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: decision.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                    left_seq,
                )?;
                schema::bump_membership_revision(tx, &command.thread)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
                bump_member_directory(tx, &instance, &caller)?;
                schema::bump_filter_revision(tx, &instance, "inbox", caller.as_str())?;
            }
            Ok(CommandResult::Left(command.thread.clone()))
        },
    )
}

fn set_archived(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    mut permit: MutationPermit,
    archived: bool,
) -> Result<CommandResult, ApiError> {
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let action = if archived { "archive" } else { "reopen" };
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash(action, command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, target)?;
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if super::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let changed = tx.execute("UPDATE threads SET archived=?1,updated_at=?2,directory_revision=directory_revision+1 WHERE id=?3 AND archived!=?1",
                params![archived, decision.utc.0, command.thread.as_str()]).map_err(store_error)?;
            if changed != 0 {
                let payload =
                    serde_json::json!({"action":action, "actor_seat":caller.as_str()}).to_string();
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("{}:{}", action, command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: decision.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM seats WHERE id=?1",
                        [caller.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            }
            Ok(if archived {
                CommandResult::Archived(command.thread.clone())
            } else {
                CommandResult::Reopened(command.thread.clone())
            })
        },
    )
}

pub fn archive(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    set_archived(context, conn, budget, command, permit, true)
}

pub fn reopen(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &ThreadMutation,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    set_archived(context, conn, budget, command, permit, false)
}

pub fn set_topic(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &SetTopic,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    if command.topic.is_empty() || command.topic.len() > 1024 {
        return Err(api_error(ErrorCode::InvalidRequest, "invalid thread topic"));
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("set_topic", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, target)?;
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if super::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
                ));
            }
            Ok(())
        },
        |tx, at| {
            decide_accountable(
                tx,
                at,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let old: String = tx
                .query_row(
                    "SELECT topic FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if old != command.topic {
                tx.execute("UPDATE threads SET topic=?1,topic_revision=topic_revision+1,updated_at=?2 WHERE id=?3 AND topic_revision<?4",
                    params![command.topic,at.utc.0,command.thread.as_str(),i64::MAX]).map_err(store_error)?;
                let payload=serde_json::json!({"action":"set_topic","actor_seat":caller.as_str(),"topic":command.topic}).to_string();
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("set_topic:{}", command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: at.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM threads WHERE id=?1",
                        [command.thread.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "topic", command.thread.as_str())?;
                schema::bump_filter_revision(tx, &instance, "topic", "all")?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            }
            Ok(CommandResult::TopicChanged(command.thread.clone()))
        },
    )
}

pub fn set_thread_name(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &crate::protocol::commands::SetThreadName,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    if let Some(name) = &command.name {
        crate::protocol::commands::validate_thread_name(name)
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("set_thread_name", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &caller, target)?;
            if !joined_in_thread(tx, &command.thread, &caller)? {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            if super::service_substrate::managed_owner(tx, &command.thread)?.is_some() {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "managed thread control belongs to service owner",
                ));
            }
            Ok(())
        },
        |tx, at| {
            decide_accountable(
                tx,
                at,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT name FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if old != command.name {
                tx.execute(
                    "UPDATE threads SET name=?1,updated_at=?2 WHERE id=?3",
                    params![command.name, at.utc.0, command.thread.as_str()],
                )
                .map_err(store_error)?;
                let payload=serde_json::json!({"action":"set_thread_name","actor_seat":caller.as_str(),"name":command.name}).to_string();
                schema::append_event_once(
                    tx,
                    EventInput {
                        thread: &command.thread,
                        key: &format!("set_thread_name:{}", command.operation.as_str()),
                        kind: "info",
                        payload_json: &payload,
                        decision_at: at.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                let instance: String = tx
                    .query_row(
                        "SELECT instance_id FROM threads WHERE id=?1",
                        [command.thread.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            }
            Ok(CommandResult::ThreadNameChanged(command.thread.clone()))
        },
    )
}

/// A new thread is born with exactly one joined member and a creation audit.
/// The caller-supplied claim only locates the durable seat; the permit is
/// consumed against a decision-time fence before any write.
pub fn create_thread(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    create_thread_impl(context, conn, budget, command, permit, None)
}
/// Bootstrap CREATE seam. `canonical` must come from the daemon's independently
/// selected InstancePaths; this adds no wire mode or runtime admission.
pub fn create_thread_in_namespace(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    permit: MutationPermit,
    canonical: &crate::protocol::handoff::HandoffNamespace,
) -> Result<CommandResult, ApiError> {
    create_thread_impl(context, conn, budget, command, permit, Some(canonical))
}
fn create_thread_impl(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &CreateThread,
    mut permit: MutationPermit,
    canonical: Option<&crate::protocol::handoff::HandoffNamespace>,
) -> Result<CommandResult, ApiError> {
    if command.topic.is_empty() || command.topic.len() > 1024 || command.goal.len() > 1024 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid thread topic or goal",
        ));
    }
    if let Some(name) = &command.name {
        crate::protocol::commands::validate_thread_name(name)
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    }
    if let Some(canonical) = canonical {
        permit = permit.with_bootstrap_child(
            canonical,
            &crate::protocol::commands::PermitMutation::CreateThread(command.clone()),
        );
    }
    let target = command.claim.target.as_str();
    let seat = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", seat.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("create_thread", command)?;
    let obligation = ObligationRef::CheckIn(seat.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            validate_actor(tx, &seat, target)?;
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &seat,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if let Some(canonical) = canonical {
                super::topology_handoff::validate_create_command(tx, canonical, command)?;
            } else {
                super::topology_handoff::guard_unscoped_create(
                    tx,
                    &instance,
                    &scope,
                    command.operation.as_str(),
                )?;
            }
            let thread = ThreadId::new(crate::store::public_ids::fresh(
                tx,
                prefix::THREAD,
                &[("threads", "id", prefix::THREAD)],
            )?);
            let joined_seq = schema::next_decision_seq(tx, &instance)?;
            tx.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at, name) VALUES (?1,?2,?3,?4,?5,?5,?6)",
                params![thread.as_str(), instance, command.topic,command.goal,decision.utc.0,command.name]).map_err(store_error)?;
            // Attach protection in this exact CREATE transaction. Historical
            // pre-fence CREATE results are reconciled by Begin/import instead.
            if super::handoff::installed(tx)? {
                if let Some(canonical) = canonical {
                    super::topology_handoff::attach_created(
                        tx,
                        canonical,
                        &instance,
                        &scope,
                        command.operation.as_str(),
                        &thread,
                    )?;
                } else {
                    super::handoff::attach_created(
                        tx,
                        &instance,
                        &scope,
                        command.operation.as_str(),
                        &thread,
                    )?;
                }
            }
            tx.execute("INSERT INTO memberships(thread_id, seat_id, state, joined_at) VALUES (?1,?2,'joined',?3)",
                params![thread.as_str(), seat.as_str(), decision.utc.0]).map_err(store_error)?;
            tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,?3)",
                params![thread.as_str(),seat.as_str(),joined_seq as i64]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"create_thread", "goal":command.goal,
                "actor_seat":seat.as_str()})
            .to_string();
            schema::append_event_once_with_decision_seq(
                tx,
                EventInput {
                    thread: &thread,
                    key: &format!("create:{}", thread.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: None,
                },
                joined_seq,
            )?;
            schema::bump_membership_revision(tx, &thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
            Ok(CommandResult::ThreadCreated(thread))
        },
    )
}

/// Invite a nonretired seat. A pending episode is immutable, including its
/// deadline, even when the caller requests a different duration later.
pub fn invite(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    command: &Invite,
    mut permit: MutationPermit,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    let duration = command
        .deadline_millis
        .or(installation_default_ms)
        .unwrap_or(300_000);
    let duration = i64::try_from(duration)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let target = command.claim.target.as_str();
    let caller = permit.seat_for_replay_scope().clone();
    let scope = format!("seat:{}", caller.as_str());
    let cooperative = permit.cooperative_metadata();
    let digest = cooperative_payload_hash("invite", command)?;
    let obligation = ObligationRef::Control(command.thread.clone());
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            let instance = validate_actor(tx, &caller, target)?;
            let joined: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id \
                 WHERE m.thread_id=?1 AND m.seat_id=?2 AND m.state='joined' AND s.state!='retired')",
                params![command.thread.as_str(), caller.as_str()], |r| r.get(0),
            ).map_err(store_error)?;
            if !joined {
                return Err(api_error(ErrorCode::Unauthorized, "caller is not joined"));
            }
            let target_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if target_instance.as_deref() != Some(instance.as_str()) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "invite target missing, retired, or outside instance",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &command.claim,
                &caller,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let prior: Option<(i64, String)> = tx
                .query_row(
                    "SELECT episode, state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), command.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            if let Some((_, state)) = &prior
                && state == "joined"
            {
                return Ok(CommandResult::AlreadyJoined(
                    crate::protocol::results::AlreadyJoined {
                        thread: command.thread.clone(),
                        seat: command.seat.clone(),
                    },
                ));
            }
            let pending: Option<String> = tx.query_row(
                "SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
                params![command.thread.as_str(), command.seat.as_str()], |r| r.get(0),
            ).optional().map_err(store_error)?;
            if let Some(id) = pending {
                return Ok(CommandResult::Invitation(InvitationId::new(id)));
            }
            let deadline = schema::checked_deadline(decision.utc, duration)?;
            let episode = super::service_controls::next_invitation_episode(
                tx,
                &command.thread,
                &command.seat,
            )?;
            if prior.is_some() {
                tx.execute("UPDATE memberships SET state='invited', voluntary_state='invited', episode=?1, joined_at=NULL, left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                    params![episode, command.thread.as_str(), command.seat.as_str()]).map_err(store_error)?;
            } else {
                tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                    params![command.thread.as_str(), command.seat.as_str(), episode]).map_err(store_error)?;
            }
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let created_decision_seq = i64::try_from(schema::next_decision_seq(tx, &instance)?)
                .map_err(|_| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "invitation decision sequence exhausted",
                    )
                })?;
            tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?8)",
                params![invitation.as_str(), command.thread.as_str(), command.seat.as_str(), episode, decision.utc.0, created_decision_seq, duration, deadline.0]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"invite", "actor_seat":caller.as_str(), "seat":command.seat.as_str(),
                "invitation":invitation.as_str(), "deadline_at":deadline.0}).to_string();
            schema::append_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("invite:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
            )?;
            schema::bump_membership_revision(tx, &command.thread)?;
            tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1", [command.seat.as_str()]).map_err(store_error)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [caller.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &command.seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", command.seat.as_str())?;
            Ok(CommandResult::Invitation(invitation))
        },
    )
}

pub fn operator_orphan_invite(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    command: &OperatorOrphanInvite,
    actor: OperatorActor,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    let duration = command
        .deadline_millis
        .or(installation_default_ms)
        .unwrap_or(300_000);
    let duration = i64::try_from(duration)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let scope = actor.operation_scope(instance);
    let typed_command = crate::protocol::commands::OperatorCommand::OrphanInvite(command.clone());
    let digest = crate::store::operator::digest(instance, &typed_command)?;
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &scope,
        command.operation.as_str(),
        digest,
        |tx| {
            let thread_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let target_instance: Option<String> = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            if thread_instance.as_deref() != Some(instance)
                || target_instance.as_deref() != Some(instance)
            {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "operator invite target outside thread instance",
                ));
            }
            let joined: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.state='joined' AND s.state!='retired')", [command.thread.as_str()], |r| r.get(0)).map_err(store_error)?;
            if joined {
                return Err(api_error(
                    ErrorCode::ThreadNotOrphaned,
                    "thread still has joined seats",
                ));
            }
            Ok(())
        },
        |tx, decision| {
            let prior: Option<(i64, String)> = tx
                .query_row(
                    "SELECT episode,state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                    params![command.thread.as_str(), command.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            let pending: Option<String> = tx.query_row("SELECT i.id FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
                params![command.thread.as_str(),command.seat.as_str()], |r| r.get(0)).optional().map_err(store_error)?;
            if let Some(id) = pending {
                return Ok(CommandResult::OperatorInvited(InvitationId::new(id)));
            }
            let episode = super::service_controls::next_invitation_episode(
                tx,
                &command.thread,
                &command.seat,
            )?;
            if prior.is_some() {
                tx.execute("UPDATE memberships SET state='invited',voluntary_state='invited',episode=?1,joined_at=NULL,left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                    params![episode,command.thread.as_str(),command.seat.as_str()]).map_err(store_error)?;
            } else {
                tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                    params![command.thread.as_str(),command.seat.as_str(),episode]).map_err(store_error)?;
            }
            let deadline = schema::checked_deadline(decision.utc, duration)?;
            let invitation = InvitationId::new(crate::store::public_ids::fresh(
                tx,
                prefix::INVITATION,
                &[("invitations", "id", prefix::INVITATION)],
            )?);
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM seats WHERE id=?1",
                    [command.seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let created_decision_seq = i64::try_from(schema::next_decision_seq(tx, &instance)?)
                .map_err(|_| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "invitation decision sequence exhausted",
                    )
                })?;
            tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?8)",
                params![invitation.as_str(),command.thread.as_str(),command.seat.as_str(),episode,decision.utc.0,created_decision_seq,duration,deadline.0]).map_err(store_error)?;
            let payload = serde_json::json!({"action":"operator_orphan_invite", "actor":actor.audit_label(),
                "seat":command.seat.as_str(),"invitation":invitation.as_str(),"deadline_at":deadline.0}).to_string();
            schema::append_event_once(
                tx,
                EventInput {
                    thread: &command.thread,
                    key: &format!("operator_invite:{}", invitation.as_str()),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: decision.utc,
                    source_message: None,
                    source_invitation: Some(&invitation),
                },
            )?;
            tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",[command.seat.as_str()]).map_err(store_error)?;
            let instance: String = tx
                .query_row(
                    "SELECT instance_id FROM threads WHERE id=?1",
                    [command.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            schema::bump_membership_revision(tx, &command.thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", "all")?;
            bump_member_directory(tx, &instance, &command.seat)?;
            schema::bump_filter_revision(tx, &instance, "inbox", command.seat.as_str())?;
            Ok(CommandResult::OperatorInvited(invitation))
        },
    )?;
    crate::store::operator::validate_result(&typed_command, &result)?;
    Ok(result)
}

#[cfg(test)]
#[path = "../../tests/store/control.rs"]
mod tests;

/// Migration-free reader for a stored actor observation. Earlier accept rows
/// used Rust debug casing (`"Claude"`) and `native_session`; ACK and send rows
/// used `"claude"` and `session`. Both read back in the one current shape.
/// Non-JSON or non-object values (test fixtures, opaque proofs) and rows
/// already in the current shape are returned unchanged.
pub fn canonical_observation(raw: &str) -> String {
    let Ok(serde_json::Value::Object(mut fields)) = serde_json::from_str::<serde_json::Value>(raw)
    else {
        return raw.to_owned();
    };
    let mut changed = false;
    if !fields.contains_key("session")
        && let Some(session) = fields.remove("native_session")
    {
        fields.insert("session".into(), session);
        changed = true;
    }
    if let Some(serde_json::Value::String(harness)) = fields.get_mut("harness") {
        let lower = harness.to_ascii_lowercase();
        if *harness != lower {
            *harness = lower;
            changed = true;
        }
    }
    if changed {
        serde_json::Value::Object(fields).to_string()
    } else {
        raw.to_owned()
    }
}

/// Audit actor of an accepted cooperative decision. Its fields describe
/// claims and durable local mapping; this type never represents native proof.
pub(crate) struct AccountableActor {
    pub harness: crate::protocol::authority::Harness,
    pub native_session: crate::protocol::ids::NativeSessionId,
    pub execution: crate::protocol::ids::ExecutionId,
    pub host_boot: crate::protocol::ids::HostBootId,
    pub target_generation: u64,
    pub binding_generation: u64,
    pub observed_at_utc: UtcMillis,
    pub provenance: &'static str,
}
impl AccountableActor {
    /// The one durable observation JSON for every accountable decision (send,
    /// ACK, accept, accept-required): wire (snake_case) harness casing and a
    /// `session` field. Rows written before this shape are normalized on read
    /// by [`canonical_observation`]; nothing is migrated.
    pub(crate) fn observation(&self, decided_at: i64) -> String {
        serde_json::json!({"harness":self.harness,"session":self.native_session,
            "execution":self.execution,"host_boot":self.host_boot,
            "binding_generation":self.binding_generation,"target_generation":self.target_generation,
            "provenance":self.provenance,"observed_at":self.observed_at_utc.0,"decided_at":decided_at})
        .to_string()
    }
}

// Allowed: one deciding mutation: transaction, permit, claim and the obligation it proves.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_accountable(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    permit: &mut MutationPermit,
    claim: &crate::protocol::authority::CallerClaim,
    seat: &SeatId,
    operation: &crate::protocol::ids::OperationId,
    obligation: &ObligationRef,
    digest: &[u8; 32],
) -> Result<AccountableActor, ApiError> {
    if claim.seat != *seat {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative actor seat mismatch",
        ));
    }
    if let Some(delivery) = permit.handoff_requirement() {
        super::handoff::validate_handoff_requirement(tx, delivery, false)?;
    } else {
        super::topology_handoff::guard_unscoped_child_phase(
            tx,
            &claim.instance,
            &format!("seat:{}", claim.seat.as_str()),
            operation.as_str(),
        )?;
    }
    let mapping = super::seats::decide_cooperative(
        tx,
        at,
        &claim.instance,
        permit,
        claim,
        None,
        operation,
        obligation,
        digest,
    )?;
    Ok(AccountableActor {
        harness: claim.harness,
        native_session: claim.native_session.clone(),
        execution: claim.execution.clone(),
        host_boot: crate::protocol::ids::HostBootId::new(mapping.boot),
        target_generation: mapping.revision,
        binding_generation: mapping.generation,
        observed_at_utc: at.utc,
        provenance: claim.harness.cooperative_provenance(),
    })
}

/// Complete cooperative control digest.
pub fn cooperative_payload_hash<T: serde::Serialize>(
    kind: &str,
    command: &T,
) -> Result<[u8; 32], ApiError> {
    schema::canonical_digest(&(kind, command))
}

#[cfg(test)]
#[path = "../../tests/store/task48_budget.rs"]
mod task48_budget_tests;

/// Monotonic passive presentation bookkeeping, never receipt attribution.
pub fn complete_inbox_delivery(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    request: &crate::protocol::commands::CompleteInboxDelivery,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    use crate::protocol::authority::CallerRole;
    let distinct: std::collections::BTreeSet<_> = request.messages.iter().collect();
    if request.claim.role == CallerRole::Subagent
        || request.messages.is_empty()
        || request.messages.len() > 100
        || distinct.len() != request.messages.len()
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid delivery completion batch or caller",
        ));
    }
    let digest = cooperative_payload_hash("complete_inbox_delivery", request)?;
    let seat = &request.claim.seat;
    schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        permit.cooperative_metadata(),
        &format!("seat:{}", seat.as_str()),
        request.operation.as_str(),
        digest,
        |tx| {
            for id in &request.messages {
                if super::lazy_delivery::recorded_mode(tx, &request.claim.instance, id)?
                    != Some(crate::protocol::commands::DeliveryMode::Lazy)
                    || !tx
                        .query_row(
                            super::lazy_delivery::ADDRESSED_SQL,
                            params![seat.as_str(), id.as_str(), request.claim.instance],
                            |r| r.get::<_, bool>(0),
                        )
                        .map_err(store_error)?
                {
                    return Err(api_error(
                        ErrorCode::InvalidRequest,
                        "completion ID is not an addressed published lazy message",
                    ));
                }
            }
            Ok(())
        },
        |tx, decision| {
            decide_accountable(
                tx,
                decision,
                &mut permit,
                &request.claim,
                seat,
                &request.operation,
                &ObligationRef::CheckIn(seat.clone()),
                &digest,
            )?;
            for id in &request.messages {
                super::lazy_delivery::complete_addressed(tx, &request.claim.instance, seat, id)?;
            }
            Ok(CommandResult::InboxDeliveryCompleted(
                request.messages.clone(),
            ))
        },
    )
}
