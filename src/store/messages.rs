//! Immutable ordinary message creation and recipient snapshots.

use crate::ports::{DurableWorkAdmission, SendPreparationProgress};
use crate::protocol::{
    authority::{MutationPermit, ObligationRef},
    commands::SendMessage,
    ids::{MessageId, SeatId, prefix},
    results::{ApiError, CommandResult, ErrorCode},
    time::CallBudget,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use super::{
    connection::{StoreContext, api_error, store_error},
    effective::{self, UnavailableWarningKey},
    schema,
};

fn require_live_budget(context: &StoreContext, budget: &CallBudget) -> Result<(), ApiError> {
    if budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::Cancelled,
            "send request budget exhausted",
        ));
    }
    Ok(())
}

fn frozen_duration(request: &SendMessage, limits: MessageLimits) -> Result<i64, ApiError> {
    let duration = request
        .deadline_millis
        .map(i64::try_from)
        .transpose()
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "duration overflow"))?
        .unwrap_or(limits.receipt_duration_ms);
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "duration must be positive",
        ));
    }
    Ok(duration)
}

pub const DEFAULT_RECEIPT_MILLIS: i64 = 300_000;
pub const MAX_BODY_BYTES: usize = 65_536;

#[derive(Debug, Clone, Copy)]
pub struct MessageLimits {
    pub receipt_duration_ms: i64,
    pub body_bytes: usize,
}
impl Default for MessageLimits {
    fn default() -> Self {
        Self {
            receipt_duration_ms: DEFAULT_RECEIPT_MILLIS,
            body_bytes: MAX_BODY_BYTES,
        }
    }
}

/// The caller claim and transient operation key are excluded from semantic replay identity.
pub fn send_payload(request: &SendMessage) -> Value {
    json!({"kind":"send_message","thread":request.thread,"body":request.body,
        "invited_recipients":request.invited_recipients,"deadline_millis":request.deadline_millis,"claim":request.claim})
}

/// One hidden preparation quantum. The caller yields the writer between steps.
/// Neither a prepared row nor a ready result grants authority to publish.
pub fn prepare_send_step(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    limits: MessageLimits,
    budget: &CallBudget,
    admission: DurableWorkAdmission,
) -> Result<SendPreparationProgress, ApiError> {
    require_live_budget(context, budget)?;
    let max_units = admission.max_units;
    if max_units == 0 || max_units > 16 || request.invited_recipients.len() > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid send preparation admission",
        ));
    }
    let digest = schema::canonical_digest(&send_payload(request))?;
    super::seats::cooperative_instance(conn, &request.claim.instance, &request.claim)?;
    let seat = request.claim.seat.clone();
    let scope = format!("seat:{}", seat.as_str());
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let prior: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![scope, request.operation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((old_digest, json)) = prior {
        if old_digest != digest {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        let result = serde_json::from_str(&json)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "stored send result invalid"))?;
        tx.rollback().map_err(store_error)?;
        return Ok(SendPreparationProgress::Committed(result));
    }
    if request.body.is_empty() || request.body.len() > limits.body_bytes.min(MAX_BODY_BYTES) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "message body byte limit",
        ));
    }
    let duration = frozen_duration(request, limits)?;
    let (instance, membership_revision, timeline_revision, archived): (String, i64, i64, bool) = tx.query_row(
        "SELECT instance_id,membership_revision,timeline_revision,archived FROM threads WHERE id=?1",
        [request.thread.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(store_error)?.ok_or_else(|| {
        api_error(
            ErrorCode::NotFound,
            format!("thread {} not found", request.thread.as_str()),
        )
    })?;
    if archived {
        return Err(api_error(ErrorCode::Archived, "thread is archived"));
    }
    if schema::effective_membership_state(&tx, &request.thread, &seat)?
        .as_ref()
        .is_none_or(|v| v.state != "joined")
    {
        return Err(api_error(ErrorCode::Unauthorized, "sender is not joined"));
    }
    let (lifecycle_revision, eligibility_revision, config_revision): (i64,i64,i64) = tx.query_row(
        "SELECT lifecycle_revision,send_eligibility_revision,duration_config_revision FROM host_instances WHERE id=?1",
        [instance.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(store_error)?;
    type PriorPrepColumns = Option<(
        String,
        Vec<u8>,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        String,
    )>;
    let prior_prep: PriorPrepColumns = tx.query_row(
        "SELECT id,digest,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,recipient_high_water,recipient_cursor,recipient_count,warning_count,status FROM send_preparations WHERE instance_id=?1 AND operation_scope=?2 AND operation_key=?3",
        params![instance,scope,request.operation.as_str()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?)),
    ).optional().map_err(store_error)?;
    let (prep_id, high_water, mut cursor, mut count, mut warning_count, status) = if let Some((
        id,
        old_digest,
        mr,
        lr,
        er,
        tr,
        cr,
        hw,
        cur,
        n,
        wn,
        status,
    )) = prior_prep
    {
        if old_digest != digest {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        if status == "discarded" {
            let job = format!("work:cleanup:{id}");
            let done:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM work_jobs WHERE id=?1 AND kind='preparation_cleanup' AND status='complete')",[job.as_str()],|r|r.get(0)).map_err(store_error)?;
            if !done {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation cleanup pending",
                ));
            }
            let children:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM prepared_unavailable_warnings WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",[id.as_str()],|r|r.get(0)).map_err(store_error)?;
            if children {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "completed preparation cleanup retained children",
                ));
            }
            let new_id = crate::store::public_ids::fresh(
                &tx,
                prefix::SEND_PREPARATION,
                crate::store::public_ids::SEND_PREPARATION_SLOTS,
            )?;
            let high_water: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
                    [request.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            tx.execute("UPDATE send_preparations SET id=?1,captured_membership_revision=?2,captured_lifecycle_revision=?3,captured_eligibility_revision=?4,captured_timeline_revision=?5,captured_config_revision=?6,interval_high_water=?7,recipient_high_water=?7,recipient_cursor=0,recipient_count=0,warning_count=0,earliest_lease_deadline=NULL,status='building' WHERE id=?8",
                params![new_id,membership_revision,lifecycle_revision,eligibility_revision,timeline_revision,config_revision,high_water,id]).map_err(store_error)?;
            (new_id, high_water, 0, 0, 0, "building".to_owned())
        } else if (mr, lr, er, tr, cr)
            != (
                membership_revision,
                lifecycle_revision,
                eligibility_revision,
                timeline_revision,
                config_revision,
            )
        {
            discard_preparation(&tx, &id)?;
            tx.commit().map_err(store_error)?;
            return Err(api_error(
                ErrorCode::Conflict,
                "send preparation snapshot changed; bounded cleanup required",
            ));
        } else {
            (id, hw, cur, n, wn, status)
        }
    } else {
        let id = crate::store::public_ids::fresh(
            &tx,
            prefix::SEND_PREPARATION,
            crate::store::public_ids::SEND_PREPARATION_SLOTS,
        )?;
        let high_water: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
                [request.thread.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12,'building')",
            params![id,instance,scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str(),membership_revision,lifecycle_revision,eligibility_revision,timeline_revision,config_revision,high_water]).map_err(store_error)?;
        (id, high_water, 0, 0, 0, "building".to_owned())
    };
    if count > 0 {
        let staged_duration:i64=tx.query_row("SELECT frozen_duration_ms FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=1",[prep_id.as_str()],|r|r.get(0)).map_err(store_error)?;
        if staged_duration != duration {
            discard_preparation(&tx, &prep_id)?;
            tx.commit().map_err(store_error)?;
            return Err(api_error(
                ErrorCode::Conflict,
                "send preparation duration changed",
            ));
        }
    }
    if status == "sealed" {
        tx.rollback().map_err(store_error)?;
        return Ok(SendPreparationProgress::Ready {
            visited: 0,
            preparation_id: prep_id,
        });
    }
    if status != "building" {
        return Err(api_error(
            ErrorCode::Conflict,
            "send preparation unavailable",
        ));
    }
    let explicit: Vec<_> = request
        .invited_recipients
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let began = std::time::Instant::now();
    let mut visited = 0u8;
    while visited < max_units
        && (visited == 0 || began.elapsed() < std::time::Duration::from_millis(5))
        && !budget.is_exhausted(context.clock())
    {
        if cursor < high_water {
            let row: Option<(i64,String,bool)> = tx.query_row(
                "SELECT mi.ordinal,mi.seat_id,(mi.left_seq IS NULL AND s.retired_seq IS NULL AND s.state!='retired') FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal>?2 AND mi.ordinal<=?3 ORDER BY mi.ordinal LIMIT 1",
                params![request.thread.as_str(),cursor,high_water], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional().map_err(store_error)?;
            if let Some((ordinal, recipient, joined)) = row {
                cursor = ordinal;
                visited += 1;
                if joined && recipient != seat.as_str() {
                    stage_recipient(
                        &tx,
                        &prep_id,
                        &instance,
                        &request.thread,
                        &recipient,
                        duration,
                        &mut count,
                        &mut warning_count,
                    )?;
                }
                continue;
            }
            cursor = high_water;
        }
        let explicit_index = usize::try_from(cursor - high_water)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid preparation cursor"))?;
        if explicit_index >= explicit.len() {
            break;
        }
        let recipient = explicit[explicit_index];
        let target_instance: Option<String> = tx
            .query_row(
                "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
                [recipient.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if target_instance.as_deref() != Some(instance.as_str())
            || schema::effective_membership_state(&tx, &request.thread, recipient)?
                .as_ref()
                .is_none_or(|v| !matches!(v.state.as_str(), "invited" | "joined"))
        {
            discard_preparation(&tx, &prep_id)?;
            tx.commit().map_err(store_error)?;
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "explicit recipient invalid or retired",
            ));
        }
        cursor = cursor.checked_add(1).ok_or_else(|| {
            api_error(ErrorCode::SequenceExhausted, "preparation cursor exhausted")
        })?;
        visited += 1;
        if recipient != &seat {
            stage_recipient(
                &tx,
                &prep_id,
                &instance,
                &request.thread,
                recipient.as_str(),
                duration,
                &mut count,
                &mut warning_count,
            )?;
        }
    }
    let complete = cursor
        >= high_water
            + i64::try_from(explicit.len())
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "explicit count overflow"))?;
    tx.execute("UPDATE send_preparations SET recipient_cursor=?1,recipient_count=?2,warning_count=?3,status=?4 WHERE id=?5",
        params![cursor,count,warning_count,if complete {"sealed"} else {"building"},prep_id]).map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(if complete {
        SendPreparationProgress::Ready {
            visited,
            preparation_id: prep_id,
        }
    } else {
        SendPreparationProgress::More {
            visited,
            preparation_id: prep_id,
        }
    })
}

fn discard_preparation(tx: &Transaction<'_>, prep_id: &str) -> Result<(), ApiError> {
    tx.execute("UPDATE send_preparations SET status='discarded' WHERE id=?1 AND status IN ('building','sealed') AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",[prep_id]).map_err(store_error)?;
    tx.execute("INSERT OR IGNORE INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'preparation_cleanup',?2,0)",params![format!("work:cleanup:{prep_id}"),prep_id]).map_err(store_error)?;
    Ok(())
}

/// Ends one unpublished preparation generation after its request exits. A late
/// request-exit callback cannot discard a successor built for the same key.
pub fn abandon_send_preparation(
    conn: &mut Connection,
    expected_preparation_id: &str,
) -> Result<bool, ApiError> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let changed=tx.execute("UPDATE send_preparations SET status='discarded' WHERE id=?1 AND status IN ('building','sealed') AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",
        [expected_preparation_id]).map_err(store_error)?;
    if changed == 0 {
        tx.rollback().map_err(store_error)?;
        return Ok(false);
    }
    tx.execute("INSERT OR IGNORE INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'preparation_cleanup',?2,0)",
        params![format!("work:cleanup:{expected_preparation_id}"),expected_preparation_id]).map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(true)
}

// Allowed: stages one recipient row and updates the caller's two running counters.
#[allow(clippy::too_many_arguments)]
fn stage_recipient(
    tx: &Transaction<'_>,
    prep_id: &str,
    instance: &str,
    thread: &crate::protocol::ids::ThreadId,
    recipient: &str,
    duration: i64,
    count: &mut i64,
    warning_count: &mut i64,
) -> Result<(), ApiError> {
    let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1 AND seat_id=?2)",params![prep_id,recipient],|r|r.get(0)).map_err(store_error)?;
    if already {
        return Ok(());
    }
    // One availability projection with `ensure_unavailability_episode`.
    let provenance = schema::effective_registered_availability(tx, recipient, Some(instance))?;
    let available = provenance.is_some();
    *count = count
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "recipient ordinal exhausted"))?;
    tx.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![prep_id,thread.as_str(),recipient,*count,duration,available,provenance]).map_err(store_error)?;
    // C4: a seat whose binding the pending reconciliation pass will carry is
    // not warned about; the carry starts its receipt timer.
    if !available
        && !schema::carry_pending(tx, recipient, instance)?
        && schema::effective_membership_state(tx, thread, &SeatId::new(recipient))?
            .as_ref()
            .is_some_and(|v| v.state == "joined")
    {
        let pending_invitation:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id))",params![thread.as_str(),recipient],|r|r.get(0)).map_err(store_error)?;
        if !pending_invitation {
            let episode = schema::ensure_unavailability_episode(tx, &SeatId::new(recipient))?;
            let key = UnavailableWarningKey {
                instance: instance.to_owned(),
                thread_id: thread.as_str().to_owned(),
                affected_seat_id: recipient.to_owned(),
                unavailability_episode: episode,
            };
            if effective::effective_warning_by_key(tx, &key)?.is_none() {
                let encoded = effective::canonical_warning_key(&key)?;
                let warning_id = effective::canonical_warning_id(&key)?;
                let collision:Option<String>=tx.query_row("SELECT warning_key FROM prepared_unavailable_warnings WHERE warning_id=?1 LIMIT 1",[warning_id.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
                if collision.as_ref().is_some_and(|prior| prior != &encoded) {
                    return Err(api_error(
                        ErrorCode::StoreCorrupt,
                        "unavailable warning ID collision",
                    ));
                }
                let source_message = prep_id.replacen("prep-", "msg-", 1);
                let payload=json!({"event":"recipient_unavailable","seat":recipient,"episode":episode,"first_message":source_message}).to_string();
                if payload.len() > 4096 {
                    return Err(api_error(
                        ErrorCode::InvalidRequest,
                        "unavailable warning payload too large",
                    ));
                }
                *warning_count = warning_count.checked_add(1).ok_or_else(|| {
                    api_error(ErrorCode::SequenceExhausted, "warning offset exhausted")
                })?;
                tx.execute("INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![prep_id,encoded,warning_id,recipient,episode as i64,*warning_count,payload]).map_err(store_error)?;
            }
        }
    }
    Ok(())
}

/// Publishes only a sealed, revision-current preparation after a fresh caller proof.
/// `current_body_bytes` reads writer-owned effective configuration after the writer
/// wait; configuration updates must use that same lane and remain stable through
/// this transaction. The callback must do bounded memory work without host/file I/O.
/// The deciding writer touches a constant number of rows regardless of recipient count.
pub fn publish_send(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    permit: &mut MutationPermit,
    budget: &CallBudget,
    current_body_bytes: impl FnOnce() -> usize,
) -> Result<CommandResult, ApiError> {
    require_live_budget(context, budget)?;
    let digest = schema::canonical_digest(&send_payload(request))?;
    super::seats::cooperative_instance(conn, &request.claim.instance, &request.claim)?;
    let seat = request.claim.seat.clone();
    let scope = format!("seat:{}", seat.as_str());
    let cooperative = permit.cooperative_metadata();
    let result = schema::execute_accountable_transaction(
        context,
        conn,
        budget,
        cooperative,
        &scope,
        request.operation.as_str(),
        digest,
        |tx| {
            type CurrentColumns =
                Option<(String, String, i64, i64, i64, i64, i64, i64, i64, String)>;
            let current:CurrentColumns = tx.query_row(
                "SELECT p.id,p.instance_id,p.captured_membership_revision,p.captured_lifecycle_revision,p.captured_eligibility_revision,p.captured_timeline_revision,p.captured_config_revision,p.recipient_count,p.warning_count,p.status FROM send_preparations p WHERE p.operation_scope=?1 AND p.operation_key=?2 AND p.digest=?3 AND p.thread_id=?4",
                params![scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str()],
                |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?)),
            ).optional().map_err(store_error)?;
            let Some((_, instance, mr, lr, er, tr, cr, _, _, status)) = current else {
                return Err(api_error(ErrorCode::Conflict, "send preparation missing"));
            };
            if status != "sealed" {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation incomplete",
                ));
            }
            let live:(i64,i64,i64,i64,i64,bool)=tx.query_row("SELECT t.membership_revision,h.lifecycle_revision,h.send_eligibility_revision,t.timeline_revision,h.duration_config_revision,t.archived FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1 AND h.id=?2",params![request.thread.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(store_error)?;
            if live.5 {
                return Err(api_error(ErrorCode::Archived, "thread archived"));
            }
            if (mr, lr, er, tr, cr) != (live.0, live.1, live.2, live.3, live.4) {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation snapshot changed",
                ));
            }
            if schema::effective_membership_state(tx, &request.thread, &seat)?
                .as_ref()
                .is_none_or(|v| v.state != "joined")
            {
                return Err(api_error(ErrorCode::Unauthorized, "sender is not joined"));
            }
            Ok(())
        },
        |tx, decision| {
            if request.body.is_empty()
                || request.body.len() > current_body_bytes().min(MAX_BODY_BYTES)
            {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "message body byte limit",
                ));
            }
            let actor = super::control::decide_accountable(
                tx,
                decision,
                permit,
                &request.claim,
                &seat,
                &request.operation,
                &ObligationRef::Control(request.thread.clone()),
                &digest,
            )?;
            let revisions_match:bool=tx.query_row(
                "SELECT p.captured_membership_revision=t.membership_revision AND p.captured_lifecycle_revision=h.lifecycle_revision AND p.captured_eligibility_revision=h.send_eligibility_revision AND p.captured_timeline_revision=t.timeline_revision AND p.captured_config_revision=h.duration_config_revision AND t.archived=0 FROM send_preparations p JOIN threads t ON t.id=p.thread_id JOIN host_instances h ON h.id=t.instance_id WHERE p.operation_scope=?1 AND p.operation_key=?2 AND p.digest=?3",
                params![scope,request.operation.as_str(),digest.as_slice()],|r|r.get(0),
            ).map_err(store_error)?;
            if !revisions_match {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation snapshot changed at decision",
                ));
            }
            let (prep_id,instance,high_water,recipient_count,warning_count):(String,String,i64,i64,i64)=tx.query_row(
                "SELECT id,instance_id,interval_high_water,recipient_count,warning_count FROM send_preparations WHERE operation_scope=?1 AND operation_key=?2 AND digest=?3",
                params![scope,request.operation.as_str(),digest.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
            ).map_err(store_error)?;
            if recipient_count > 0 {
                let duration:i64=tx.query_row("SELECT frozen_duration_ms FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=1",[prep_id.as_str()],|r|r.get(0)).map_err(store_error)?;
                schema::checked_deadline(decision.utc, duration)?;
            }
            let decision_seq = schema::next_decision_seq(tx, &instance)?;
            let base: i64 = tx
                .query_row(
                    "SELECT next_sequence FROM threads WHERE id=?1",
                    [request.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let advance = warning_count.checked_add(1).ok_or_else(|| {
                api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted")
            })?;
            let following = base.checked_add(advance).ok_or_else(|| {
                api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted")
            })?;
            if following <= 0 {
                return Err(api_error(
                    ErrorCode::SequenceExhausted,
                    "timeline sequence exhausted",
                ));
            }
            let id = MessageId::new(prep_id.replacen("prep-", "msg-", 1));
            let observation = actor.observation(decision.utc.0);
            tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,native_observation,body,decision_at,decision_seq) VALUES (?1,?2,?3,?4,'ordinary',?5,?6,?7,?8,?9)",params![id.as_str(),instance,request.thread.as_str(),base,seat.as_str(),observation,request.body,decision.utc.0,decision_seq as i64]).map_err(store_error)?;
            tx.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![prep_id,id.as_str(),instance,request.thread.as_str(),decision_seq as i64,decision.utc.0,base,high_water,recipient_count,warning_count]).map_err(store_error)?;
            tx.execute(
                "UPDATE threads SET next_sequence=?1,updated_at=?2 WHERE id=?3",
                params![following, decision.utc.0, request.thread.as_str()],
            )
            .map_err(store_error)?;
            schema::bump_timeline_revision(tx, &request.thread)?;
            schema::bump_filter_revision(tx, &instance, "directory", request.thread.as_str())?;
            let work_high_water = recipient_count
                .checked_add(warning_count)
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| {
                    api_error(ErrorCode::SequenceExhausted, "send work position exhausted")
                })?;
            tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'send_attention',?2,?3)",params![format!("work:send:{}",id.as_str()),prep_id,work_high_water]).map_err(store_error)?;
            failpoint!("send.before_commit", context.failpoint_scope());
            Ok(CommandResult::MessageSent(id))
        },
    )?;
    failpoint!("send.after_commit", context.failpoint_scope());
    Ok(result)
}

#[cfg(test)]
#[path = "../../tests/store/messages.rs"]
mod messages_tests;
