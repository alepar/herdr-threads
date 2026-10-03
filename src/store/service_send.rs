//! Service-authored ordinary messages with native receipt obligations.
//!
//! The registered service author posts into a thread it manages. Obligations,
//! deadlines, wake and retirement settlement are the native ones: the audience
//! is the joined snapshot plus explicit invited or joined recipients, staged in
//! bounded quanta and published under the service decision fence. The service
//! itself never appears in a receipt table.
use super::{
    connection::{StoreContext, api_error, store_error},
    materialization,
    messages::{
        self, AudienceWalk, MAX_BODY_BYTES, MessageLimits, PublicationAuthor, RecipientRejection,
        WalkEnd,
    },
    schema, service_controls,
};
use crate::{
    ports::{
        DurableWorkAdmission, ServiceAuthorityGate, ServiceConnectionAuthority,
        ServiceDecisionStartError, ServiceDecisionTransaction,
    },
    protocol::{
        ids::{MessageId, SeatId, ServiceAuthorId, prefix},
        results::{ApiError, ErrorCode, MessageKind},
        service::{ServiceMessageSent, ServiceResult, ServiceSend},
        time::{CallBudget, UtcMillis},
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The same restart allowance as `service_events` notifications.
const MAX_AUDIENCE_RESTARTS: u8 = 8;

/// One preparation quantum's outcome.
// Allowed: a transient step result; ServiceResult carries the v2 receipts inspection
// and boxing it would change the wire-contract type.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendStep {
    More { visited: u8, preparation_id: String },
    Ready { preparation_id: String },
    Committed(ServiceResult),
}

/// A preparation quantum, or the reason the caller must restart or wait.
// Allowed: a transient step result, see `SendStep`.
#[allow(clippy::large_enum_variant)]
pub(crate) enum SendPrepare {
    Step(SendStep),
    /// The audience changed; the named generation was discarded.
    AudienceDrift {
        discarded: String,
    },
    /// A discarded generation of this key still has cleanup to run.
    CleanupPending {
        preparation_id: String,
    },
}

/// A publication attempt.
// Allowed: a transient step result, see `SendStep`.
#[allow(clippy::large_enum_variant)]
pub(crate) enum SendPublish {
    Committed(ServiceResult),
    AudienceDrift { discarded: String },
}

fn scope(instance: &str, author: &ServiceAuthorId) -> String {
    format!("service:{instance}:{}", author.as_str())
}

/// Replay identity: the recipients are an order-insensitive set.
pub fn payload(request: &ServiceSend) -> Value {
    let recipients: Vec<&SeatId> = request
        .recipients
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    json!({"kind":"service_send","thread":request.thread,"body":request.body,
        "recipients":recipients,"deadline_millis":request.deadline_millis})
}

fn replay(
    db: &Connection,
    instance: &str,
    author: &ServiceAuthorId,
    request: &ServiceSend,
    digest: &[u8; 32],
) -> Result<Option<ServiceResult>, ApiError> {
    let prior: Option<(Vec<u8>, String)> = db
        .query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![scope(instance, author), request.operation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    match prior {
        Some((old, result)) if old == digest => {
            serde_json::from_str(&result).map(Some).map_err(|_| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    "stored service send result invalid",
                )
            })
        }
        Some(_) => Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "operation key reused with different payload",
        )),
        None => Ok(None),
    }
}

fn live_budget(context: &StoreContext, budget: &CallBudget) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        return Err(api_error(ErrorCode::Cancelled, "service send cancelled"));
    }
    if budget.deadline_passed(context.clock()) {
        return Err(api_error(
            ErrorCode::DeadlineExceeded,
            "service send budget exhausted",
        ));
    }
    Ok(())
}

fn check_body(request: &ServiceSend, limits: MessageLimits) -> Result<(), ApiError> {
    if request.body.is_empty() || request.body.len() > limits.body_bytes.min(MAX_BODY_BYTES) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "message body byte limit",
        ));
    }
    Ok(())
}

fn begin<'db, 'gate>(
    db: &'db mut Connection,
    gate: &'gate dyn ServiceAuthorityGate,
    connection: &ServiceConnectionAuthority,
) -> Result<ServiceDecisionTransaction<'db, 'gate>, ApiError> {
    let decision =
        ServiceDecisionTransaction::begin(db, gate, connection).map_err(|error| match error {
            ServiceDecisionStartError::Database(error) => store_error(error),
            ServiceDecisionStartError::Authority(error) => error,
        })?;
    if decision.author() != connection.author() {
        return Err(api_error(
            ErrorCode::StaleServiceGeneration,
            "service author changed",
        ));
    }
    Ok(decision)
}

/// The thread's instance, ownership and archive checks shared by both steps.
/// Returns the live revisions `(membership, timeline)`.
fn check_thread(
    tx: &Transaction<'_>,
    connection: &ServiceConnectionAuthority,
    request: &ServiceSend,
) -> Result<(i64, i64), ApiError> {
    let row: Option<(String, i64, i64, bool)> = tx
        .query_row(
            "SELECT instance_id,membership_revision,timeline_revision,archived FROM threads WHERE id=?1",
            [request.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((instance, membership, timeline, archived)) = row else {
        return Err(api_error(ErrorCode::NotFound, "thread not found"));
    };
    if instance != connection.instance() {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "thread belongs to another instance",
        ));
    }
    service_controls::require_owner(tx, &instance, &request.thread, connection.author())?;
    if archived {
        return Err(api_error(ErrorCode::Archived, "thread is archived"));
    }
    Ok((membership, timeline))
}

/// The columns of a prior `send_preparations` row for this key.
type PriorPrep = Option<(String, Vec<u8>, [i64; 5], i64, i64, i64, i64, String)>;

/// One bounded preparation quantum under the service decision fence. A later
/// revision change discards this generation; the orchestrator rebuilds it
/// within the original budget.
// Allowed: one decision quantum takes the store context, connection, gate, request,
// limits, budget and quantum size.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_service_send_step(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceSend,
    limits: MessageLimits,
    budget: &CallBudget,
    max_units: u8,
) -> Result<SendPrepare, ApiError> {
    if max_units == 0 || max_units > 16 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid send preparation admission",
        ));
    }
    live_budget(context, budget)?;
    let digest = schema::canonical_digest(&payload(request))?;
    let decision = begin(db, gate, connection)?;
    let tx = decision.transaction();
    let author = decision.author().clone();
    if let Some(result) = replay(tx, connection.instance(), &author, request, &digest)? {
        decision.rollback().map_err(store_error)?;
        return Ok(SendPrepare::Step(SendStep::Committed(result)));
    }
    let (membership_revision, timeline_revision) = check_thread(tx, connection, request)?;
    check_body(request, limits)?;
    let duration = messages::frozen_duration_ms(request.deadline_millis, limits)?;
    let instance = connection.instance().to_owned();
    let op_scope = scope(&instance, &author);
    let (lifecycle_revision, eligibility_revision, config_revision): (i64, i64, i64) = tx.query_row(
        "SELECT lifecycle_revision,send_eligibility_revision,duration_config_revision FROM host_instances WHERE id=?1",
        [instance.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(store_error)?;
    let live = [
        membership_revision,
        lifecycle_revision,
        eligibility_revision,
        timeline_revision,
        config_revision,
    ];
    let prior_prep: PriorPrep = tx.query_row(
        "SELECT id,digest,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,recipient_high_water,recipient_cursor,recipient_count,warning_count,status FROM send_preparations WHERE instance_id=?1 AND operation_scope=?2 AND operation_key=?3",
        params![instance, op_scope, request.operation.as_str()],
        |r| Ok((
            r.get(0)?,
            r.get(1)?,
            [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
            r.get(10)?,
            r.get(11)?,
        )),
    ).optional().map_err(store_error)?;
    let new_high_water = |tx: &Transaction<'_>| -> Result<i64, ApiError> {
        tx.query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
            [request.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)
    };
    let (prep_id, high_water, mut cursor, mut count, mut warning_count, status) = if let Some((
        id,
        old_digest,
        captured,
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
            let done: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM work_jobs WHERE id=?1 AND kind='preparation_cleanup' AND status='complete')",[job.as_str()],|r|r.get(0)).map_err(store_error)?;
            if !done {
                decision.rollback().map_err(store_error)?;
                return Ok(SendPrepare::CleanupPending { preparation_id: id });
            }
            let children: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM prepared_unavailable_warnings WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",[id.as_str()],|r|r.get(0)).map_err(store_error)?;
            if children {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "completed preparation cleanup retained children",
                ));
            }
            let new_id = crate::store::public_ids::fresh(
                tx,
                prefix::SEND_PREPARATION,
                crate::store::public_ids::SEND_PREPARATION_SLOTS,
            )?;
            let high_water = new_high_water(tx)?;
            tx.execute("UPDATE send_preparations SET id=?1,captured_membership_revision=?2,captured_lifecycle_revision=?3,captured_eligibility_revision=?4,captured_timeline_revision=?5,captured_config_revision=?6,interval_high_water=?7,recipient_high_water=?7,recipient_cursor=0,recipient_count=0,warning_count=0,earliest_lease_deadline=NULL,status='building' WHERE id=?8",
                params![new_id,live[0],live[1],live[2],live[3],live[4],high_water,id]).map_err(store_error)?;
            (new_id, high_water, 0, 0, 0, "building".to_owned())
        } else if captured != live {
            messages::discard_preparation(tx, &id)?;
            decision.commit().map_err(store_error)?;
            return Ok(SendPrepare::AudienceDrift { discarded: id });
        } else {
            (id, hw, cur, n, wn, status)
        }
    } else {
        let id = crate::store::public_ids::fresh(
            tx,
            prefix::SEND_PREPARATION,
            crate::store::public_ids::SEND_PREPARATION_SLOTS,
        )?;
        let high_water = new_high_water(tx)?;
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12,'building')",
            params![id,instance,op_scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str(),live[0],live[1],live[2],live[3],live[4],high_water]).map_err(store_error)?;
        (id, high_water, 0, 0, 0, "building".to_owned())
    };
    if count > 0 {
        let staged_duration: i64 = tx.query_row("SELECT frozen_duration_ms FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=1",[prep_id.as_str()],|r|r.get(0)).map_err(store_error)?;
        if staged_duration != duration {
            messages::discard_preparation(tx, &prep_id)?;
            decision.commit().map_err(store_error)?;
            return Ok(SendPrepare::AudienceDrift { discarded: prep_id });
        }
    }
    if status == "sealed" {
        decision.rollback().map_err(store_error)?;
        return Ok(SendPrepare::Step(SendStep::Ready {
            preparation_id: prep_id,
        }));
    }
    if status != "building" {
        return Err(api_error(
            ErrorCode::Conflict,
            "send preparation unavailable",
        ));
    }
    let explicit: Vec<&SeatId> = request
        .recipients
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let walk = AudienceWalk {
        prep_id: &prep_id,
        instance: &instance,
        thread: &request.thread,
        high_water,
        explicit: &explicit,
        exclude: None,
        duration,
    };
    let end = messages::walk_audience(
        tx,
        context,
        budget,
        max_units,
        &walk,
        &mut cursor,
        &mut count,
        &mut warning_count,
    )?;
    let (visited, complete) = match end {
        WalkEnd::Rejected { seat, reason } => {
            messages::discard_preparation(tx, &prep_id)?;
            decision.commit().map_err(store_error)?;
            return Err(match reason {
                RecipientRejection::Unknown => api_error(
                    ErrorCode::NotFound,
                    format!("recipient seat unknown: {}", seat.as_str()),
                ),
                RecipientRejection::Retired => api_error(
                    ErrorCode::InvalidRequest,
                    format!("recipient retired: {}", seat.as_str()),
                ),
                RecipientRejection::NotMemberOrInvitee => api_error(
                    ErrorCode::InvalidRequest,
                    format!("recipient not a member or invitee: {}", seat.as_str()),
                ),
            });
        }
        WalkEnd::Complete { visited } => (visited, true),
        WalkEnd::More { visited } => (visited, false),
    };
    if complete && count == 0 {
        messages::discard_preparation(tx, &prep_id)?;
        decision.commit().map_err(store_error)?;
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "no receipt recipients",
        ));
    }
    tx.execute("UPDATE send_preparations SET recipient_cursor=?1,recipient_count=?2,warning_count=?3,status=?4,prepared_at=?6 WHERE id=?5",
        params![cursor,count,warning_count,if complete {"sealed"} else {"building"},prep_id,context.clock().utc_now().0]).map_err(store_error)?;
    decision.commit().map_err(store_error)?;
    Ok(SendPrepare::Step(if complete {
        SendStep::Ready {
            preparation_id: prep_id,
        }
    } else {
        SendStep::More {
            visited,
            preparation_id: prep_id,
        }
    }))
}

/// Publishes only a sealed, revision-current preparation. The authority guard
/// is held from after BEGIN IMMEDIATE through commit; the message, its
/// obligations' immutable snapshot and the stored result commit together.
pub(crate) fn publish_service_send(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceSend,
    limits: MessageLimits,
    budget: &CallBudget,
) -> Result<SendPublish, ApiError> {
    live_budget(context, budget)?;
    let digest = schema::canonical_digest(&payload(request))?;
    let decision = begin(db, gate, connection)?;
    let tx = decision.transaction();
    let author = decision.author().clone();
    if let Some(result) = replay(tx, connection.instance(), &author, request, &digest)? {
        decision.rollback().map_err(store_error)?;
        return Ok(SendPublish::Committed(result));
    }
    let instance = connection.instance().to_owned();
    let op_scope = scope(&instance, &author);
    let prep: Option<(String, [i64; 5], String)> = tx.query_row(
        "SELECT id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,status FROM send_preparations WHERE operation_scope=?1 AND operation_key=?2 AND digest=?3 AND thread_id=?4",
        params![op_scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str()],
        |r| Ok((r.get(0)?, [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?], r.get(6)?)),
    ).optional().map_err(store_error)?;
    let Some((prep_id, captured, status)) = prep else {
        return Err(api_error(ErrorCode::Conflict, "send preparation missing"));
    };
    if status != "sealed" {
        return Err(api_error(
            ErrorCode::Conflict,
            "send preparation incomplete",
        ));
    }
    let (membership, timeline) = check_thread(tx, connection, request)?;
    let (lifecycle, eligibility, config): (i64, i64, i64) = tx.query_row(
        "SELECT lifecycle_revision,send_eligibility_revision,duration_config_revision FROM host_instances WHERE id=?1",
        [instance.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(store_error)?;
    if captured != [membership, lifecycle, eligibility, timeline, config] {
        messages::discard_preparation(tx, &prep_id)?;
        decision.commit().map_err(store_error)?;
        return Ok(SendPublish::AudienceDrift { discarded: prep_id });
    }
    check_body(request, limits)?;
    live_budget(context, budget)?;
    let now = context.clock().utc_now();
    let published = messages::insert_publication(
        tx,
        now.0,
        &op_scope,
        request.operation.as_str(),
        &digest,
        &request.thread,
        &request.body,
        PublicationAuthor::Programmatic(&author),
    )?;
    let recipient_count = u64::try_from(published.recipient_count)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative recipient count"))?;
    let receipt_duration_millis = u64::try_from(published.duration_ms)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative receipt duration"))?;
    let sequence = u64::try_from(published.sequence)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?;
    let message: MessageId = published.message;
    let result = ServiceResult::MessageSent(ServiceMessageSent {
        summary: super::service_substrate::service_message_summary(
            message.clone(),
            request.thread.clone(),
            author.clone(),
            MessageKind::Ordinary,
            sequence,
            UtcMillis(now.0),
            &request.body,
            Some(vec![
                "herdr-threads".into(),
                "body".into(),
                message.as_str().into(),
            ]),
        ),
        author: author.clone(),
        recipient_count,
        receipt_duration_millis,
    });
    let encoded = serde_json::to_string(&result).map_err(|_| {
        api_error(
            ErrorCode::StoreCorrupt,
            "service send result encoding failed",
        )
    })?;
    tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES(?1,?2,?3,?4,?5)",
        params![op_scope,request.operation.as_str(),digest.as_slice(),encoded,now.0]).map_err(store_error)?;
    decision.commit().map_err(store_error)?;
    failpoint!("service_send.after_commit", context.failpoint_scope());
    Ok(SendPublish::Committed(result))
}

impl super::SqliteStore {
    /// Service-authored send: prepares, publishes and restarts internally when
    /// the audience changes, all within the caller's original budget.
    pub(super) fn service_send_with_admission(
        &self,
        request: &ServiceSend,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        admission: Option<&crate::service::fair_writer::FairWriter>,
        mut observe: impl FnMut(&SendStep, &StoreContext),
    ) -> Result<ServiceResult, ApiError> {
        if connection.instance() != self.instance {
            return Err(api_error(
                ErrorCode::Unauthorized,
                "service instance mismatch",
            ));
        }
        let limits = self.settings.message_limits;
        let mut restarts = 0u8;
        loop {
            let progress = {
                let _turn = admission
                    .map(|lane| lane.enter_foreground(budget, self.context.clock()))
                    .transpose()?;
                let mut writer = self.writer(budget)?;
                prepare_service_send_step(
                    &self.context,
                    &mut writer,
                    connection,
                    gate,
                    request,
                    limits,
                    budget,
                    16,
                )?
            };
            let step = match progress {
                SendPrepare::AudienceDrift { discarded }
                | SendPrepare::CleanupPending {
                    preparation_id: discarded,
                } => {
                    count_audience_restart(&mut restarts, budget, &self.context)?;
                    self.finish_cleanup(&discarded, budget, admission)?;
                    continue;
                }
                SendPrepare::Step(step) => step,
            };
            observe(&step, &self.context);
            match step {
                SendStep::Committed(result) => return Ok(result),
                SendStep::More { .. } => continue,
                SendStep::Ready { .. } => {}
            }
            let publication = {
                let _turn = admission
                    .map(|lane| lane.enter_foreground(budget, self.context.clock()))
                    .transpose()?;
                let mut writer = self.writer(budget)?;
                publish_service_send(
                    &self.context,
                    &mut writer,
                    connection,
                    gate,
                    request,
                    limits,
                    budget,
                )?
            };
            match publication {
                SendPublish::Committed(result) => return Ok(result),
                SendPublish::AudienceDrift { discarded } => {
                    count_audience_restart(&mut restarts, budget, &self.context)?;
                    self.finish_cleanup(&discarded, budget, admission)?;
                }
            }
        }
    }

    /// Drives one discarded generation's cleanup job to completion, one
    /// bounded writer turn per call, so the rebuild can reuse the key.
    fn finish_cleanup(
        &self,
        preparation_id: &str,
        budget: &CallBudget,
        admission: Option<&crate::service::fair_writer::FairWriter>,
    ) -> Result<(), ApiError> {
        let job = format!("work:cleanup:{preparation_id}");
        loop {
            let _turn = admission
                .map(|lane| lane.enter_foreground(budget, self.context.clock()))
                .transpose()?;
            let mut writer = self.writer(budget)?;
            let status: Option<String> = writer
                .query_row(
                    "SELECT status FROM work_jobs WHERE id=?1",
                    [job.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            match status.as_deref() {
                Some("complete") => return Ok(()),
                Some(_) => {}
                None => {
                    return Err(api_error(
                        ErrorCode::StoreCorrupt,
                        "send preparation cleanup job missing",
                    ));
                }
            }
            let progress = materialization::advance_work(
                &mut writer,
                &job,
                DurableWorkAdmission::new(16)
                    .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?,
                budget,
                self.context.clock(),
            )?;
            if let Some(error) = progress.last_error {
                return Err(api_error(ErrorCode::StoreCorrupt, error));
            }
        }
    }

    #[cfg(test)]
    fn service_send_with_observer(
        &self,
        request: &ServiceSend,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        observe: impl FnMut(&SendStep, &StoreContext),
    ) -> Result<ServiceResult, ApiError> {
        self.service_send_with_admission(request, connection, gate, budget, None, observe)
    }
}

fn count_audience_restart(
    restarts: &mut u8,
    budget: &CallBudget,
    context: &StoreContext,
) -> Result<(), ApiError> {
    live_budget(context, budget)?;
    if *restarts >= MAX_AUDIENCE_RESTARTS {
        return Err(api_error(
            ErrorCode::Conflict,
            "service send audience changed beyond restart allowance",
        ));
    }
    *restarts += 1;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/store/service_send.rs"]
mod tests;
