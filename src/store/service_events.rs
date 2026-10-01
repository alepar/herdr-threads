//! Attributable service notifications with bounded, revision-checked audience capture.
use super::{
    connection::{StoreContext, api_error, store_error},
    schema,
};
use crate::{
    ports::{
        ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionStartError,
        ServiceDecisionTransaction,
    },
    protocol::{
        ids::{MessageId, ServiceAuthorId, prefix},
        results::{ApiError, ErrorCode, MessageKind, MessageSummary},
        service::{NotificationSeverity, ServiceNotification, ServiceNotify, ServiceResult},
        time::{CallBudget, UtcMillis},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub const MAX_EVENT_BYTES: usize = 4096;
const MAX_AUDIENCE_RESTARTS: u8 = 8;

// Allowed: a transient, module-private step result; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
enum PrepareProgress {
    Step(PreparationStep),
    AudienceDrift,
}

// Allowed: a transient, module-private step result; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
enum PublishProgress {
    Committed(ServiceResult),
    AudienceDrift,
}

impl super::SqliteStore {
    /// Service transport can call this after registration. C2/D2 dispatch
    /// combines it with other service operations at serial fan-in.
    pub fn service_notify(
        &self,
        request: &ServiceNotify,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ApiError> {
        self.service_notify_with_admission(request, connection, gate, budget, None, |_, _| {})
    }

    pub fn service_notify_admitted(
        &self,
        request: &ServiceNotify,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        admission: &crate::service::workers::FairWriter,
    ) -> Result<ServiceResult, ApiError> {
        self.service_notify_with_admission(
            request,
            connection,
            gate,
            budget,
            Some(admission),
            |_, _| {},
        )
    }

    #[cfg(test)]
    fn service_notify_with_observer(
        &self,
        request: &ServiceNotify,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        observe: impl FnMut(&PreparationStep, &StoreContext),
    ) -> Result<ServiceResult, ApiError> {
        self.service_notify_with_admission(request, connection, gate, budget, None, observe)
    }

    fn service_notify_with_admission(
        &self,
        request: &ServiceNotify,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        admission: Option<&crate::service::workers::FairWriter>,
        mut observe: impl FnMut(&PreparationStep, &StoreContext),
    ) -> Result<ServiceResult, ApiError> {
        if connection.instance() != self.instance {
            return Err(api_error(
                ErrorCode::Unauthorized,
                "service instance mismatch",
            ));
        }
        let mut restarts = 0;
        loop {
            let progress = {
                let _turn = admission
                    .map(|lane| lane.enter_foreground(budget, self.context.clock()))
                    .transpose()?;
                let mut writer = self.writer(budget)?;
                prepare_notify_step_internal(
                    &self.context,
                    &mut writer,
                    connection,
                    gate,
                    request,
                    budget,
                    16,
                )?
            };
            let step = match progress {
                PrepareProgress::AudienceDrift => {
                    count_audience_restart(&mut restarts, budget, &self.context)?;
                    continue;
                }
                PrepareProgress::Step(step) => step,
            };
            observe(&step, &self.context);
            match step {
                PreparationStep::Committed(_) => {}
                PreparationStep::More { .. } => continue,
                PreparationStep::Ready { .. } => {}
            }
            let publication = {
                let _turn = admission
                    .map(|lane| lane.enter_foreground(budget, self.context.clock()))
                    .transpose()?;
                let mut writer = self.writer(budget)?;
                publish_notify_internal(
                    &self.context,
                    &mut writer,
                    connection,
                    gate,
                    request,
                    budget,
                )?
            };
            match publication {
                PublishProgress::Committed(result) => return Ok(result),
                PublishProgress::AudienceDrift => {
                    count_audience_restart(&mut restarts, budget, &self.context)?;
                }
            }
        }
    }
}

fn count_audience_restart(
    restarts: &mut u8,
    budget: &CallBudget,
    context: &StoreContext,
) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        return Err(api_error(
            ErrorCode::Cancelled,
            "notification preparation cancelled",
        ));
    }
    if budget.deadline_passed(context.clock()) {
        return Err(api_error(
            ErrorCode::DeadlineExceeded,
            "notification preparation budget exhausted",
        ));
    }
    if *restarts >= MAX_AUDIENCE_RESTARTS {
        return Err(api_error(
            ErrorCode::Conflict,
            "notification audience changed beyond restart allowance",
        ));
    }
    *restarts += 1;
    Ok(())
}

pub fn payload(request: &ServiceNotify) -> Value {
    json!({"kind":"system_notify","thread":request.thread,"severity":request.severity,
        "event_json":request.event_json})
}

fn event(request: &ServiceNotify) -> Result<String, ApiError> {
    let text = json!({"event":"system_notify","data":request.event_json}).to_string();
    if text.len() > MAX_EVENT_BYTES {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "system notification exceeds event byte limit",
        ));
    }
    Ok(text)
}

fn scope(instance: &str, author: &ServiceAuthorId) -> String {
    format!("service:{instance}:{}", author.as_str())
}

fn replay(
    db: &Connection,
    instance: &str,
    author: &ServiceAuthorId,
    request: &ServiceNotify,
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
                    "stored notification result invalid",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparationStep {
    More { visited: u8, preparation_id: String },
    Ready { visited: u8, preparation_id: String },
    Committed(ServiceResult),
}

/// Stages at most sixteen indexed audience candidates. A later revision change
/// discards this generation. The service call internally rebuilds it within
/// its original budget; callers of this step can observe the conflict.
pub fn prepare_notify_step(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceNotify,
    budget: &CallBudget,
    max_units: u8,
) -> Result<PreparationStep, ApiError> {
    match prepare_notify_step_internal(context, db, connection, gate, request, budget, max_units)? {
        PrepareProgress::Step(step) => Ok(step),
        PrepareProgress::AudienceDrift => Err(api_error(
            ErrorCode::Conflict,
            "notification audience changed; retry preparation",
        )),
    }
}

fn prepare_notify_step_internal(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceNotify,
    budget: &CallBudget,
    max_units: u8,
) -> Result<PrepareProgress, ApiError> {
    if max_units == 0 || max_units > 16 || budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid notification preparation admission or budget",
        ));
    }
    let _event = event(request)?;
    let digest = schema::canonical_digest(&payload(request))?;
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
    let tx = decision.transaction();
    if let Some(result) = replay(
        tx,
        connection.instance(),
        connection.author(),
        request,
        &digest,
    )? {
        decision.rollback().map_err(store_error)?;
        return Ok(PrepareProgress::Step(PreparationStep::Committed(result)));
    }
    let (instance,membership_revision,lifecycle_revision): (String,i64,i64) = tx.query_row(
        "SELECT t.instance_id,t.membership_revision,h.lifecycle_revision FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1",
        [request.thread.as_str()],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
        .optional().map_err(store_error)?.ok_or_else(||api_error(ErrorCode::NotFound,"thread not found"))?;
    if instance != connection.instance() {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "thread belongs to another instance",
        ));
    }
    type PriorColumns = Option<(String, Vec<u8>, i64, i64, i64, i64, i64, i64, i64, String)>;
    let prior: PriorColumns = tx.query_row(
        "SELECT id,digest,membership_revision,lifecycle_revision,interval_high_water,requirement_high_water,interval_cursor,requirement_cursor,recipient_count,status FROM service_notification_preparations WHERE instance_id=?1 AND author_id=?2 AND operation_key=?3 ORDER BY ordinal DESC LIMIT 1",
        params![instance,connection.author().as_str(),request.operation.as_str()],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(store_error)?;
    let (
        id,
        interval_high,
        requirement_high,
        mut interval_cursor,
        mut requirement_cursor,
        mut recipient_count,
        status,
    ) = if let Some((id, old, mr, lr, ih, rh, ic, rc, count, status)) = prior {
        if old != digest {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        if status != "discarded" && (mr != membership_revision || lr != lifecycle_revision) {
            discard_preparation(tx, &id)?;
            decision.commit().map_err(store_error)?;
            return Ok(PrepareProgress::AudienceDrift);
        }
        if status == "discarded" {
            new_preparation(
                tx,
                &instance,
                connection.author(),
                request,
                &digest,
                membership_revision,
                lifecycle_revision,
            )?
        } else {
            (id, ih, rh, ic, rc, count, status)
        }
    } else {
        new_preparation(
            tx,
            &instance,
            connection.author(),
            request,
            &digest,
            membership_revision,
            lifecycle_revision,
        )?
    };
    if status == "sealed" {
        decision.rollback().map_err(store_error)?;
        return Ok(PrepareProgress::Step(PreparationStep::Ready {
            visited: 0,
            preparation_id: id,
        }));
    }
    if status != "building" {
        return Err(api_error(
            ErrorCode::Conflict,
            "notification preparation unavailable",
        ));
    }
    let mut visited = 0u8;
    while visited < max_units && !budget.is_exhausted(context.clock()) {
        if interval_cursor < interval_high {
            let row: Option<(i64,String,bool)> = tx.query_row(
                "SELECT mi.ordinal,mi.seat_id,(mi.left_seq IS NULL AND s.retired_seq IS NULL AND s.state!='retired') FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal>?2 AND mi.ordinal<=?3 ORDER BY mi.ordinal LIMIT 1",
                params![request.thread.as_str(),interval_cursor,interval_high],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
            if let Some((ordinal, seat, joined)) = row {
                interval_cursor = ordinal;
                visited += 1;
                if joined && stage(tx, &id, &seat)? {
                    recipient_count = recipient_count.checked_add(1).ok_or_else(|| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "notification recipient count overflow",
                        )
                    })?;
                }
                continue;
            }
            interval_cursor = interval_high;
        }
        if requirement_cursor < requirement_high {
            let row: Option<(i64,String,bool)> = tx.query_row(
                "SELECT r.ordinal,r.seat_id,(s.state!='retired' AND s.retired_seq IS NULL AND r.state='pending') FROM requirement_episodes r JOIN seats s ON s.id=r.seat_id WHERE r.thread_id=?1 AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT 1",
                params![request.thread.as_str(),requirement_cursor,requirement_high],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
            if let Some((ordinal, seat, pending)) = row {
                requirement_cursor = ordinal;
                visited += 1;
                if pending && stage(tx, &id, &seat)? {
                    recipient_count = recipient_count.checked_add(1).ok_or_else(|| {
                        api_error(
                            ErrorCode::StoreCorrupt,
                            "notification recipient count overflow",
                        )
                    })?;
                }
                continue;
            }
            requirement_cursor = requirement_high;
        }
        break;
    }
    let complete = interval_cursor >= interval_high && requirement_cursor >= requirement_high;
    tx.execute("UPDATE service_notification_preparations SET interval_cursor=?1,requirement_cursor=?2,recipient_count=?3,status=?4 WHERE id=?5",
        params![interval_cursor,requirement_cursor,recipient_count,if complete {"sealed"} else {"building"},id]).map_err(store_error)?;
    decision.commit().map_err(store_error)?;
    Ok(PrepareProgress::Step(if complete {
        PreparationStep::Ready {
            visited,
            preparation_id: id,
        }
    } else {
        PreparationStep::More {
            visited,
            preparation_id: id,
        }
    }))
}

fn discard_preparation(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE service_notification_preparations SET status='discarded' WHERE id=?1",
        [id],
    )
    .map_err(store_error)?;
    tx.execute(
        "INSERT OR IGNORE INTO work_jobs(id,kind,subject_id,high_water) VALUES(?1,'preparation_cleanup',?2,0)",
        params![format!("work:service-notify-cleanup:{id}"),format!("service-notify:{id}")],
    ).map_err(store_error)?;
    Ok(())
}

/// The freshly inserted preparation, as `new_preparation` returns it.
type NewPreparation = (String, i64, i64, i64, i64, i64, String);

fn new_preparation(
    tx: &rusqlite::Transaction<'_>,
    instance: &str,
    author: &ServiceAuthorId,
    request: &ServiceNotify,
    digest: &[u8; 32],
    mr: i64,
    lr: i64,
) -> Result<NewPreparation, ApiError> {
    let id = crate::store::public_ids::fresh(
        tx,
        prefix::NOTIFY_PREPARATION,
        crate::store::public_ids::NOTIFY_PREPARATION_SLOTS,
    )?;
    let ih: i64 = tx
        .query_row(
            "SELECT coalesce(max(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
            [request.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let rh: i64 = tx
        .query_row(
            "SELECT coalesce(max(ordinal),0) FROM requirement_episodes WHERE thread_id=?1",
            [request.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    tx.execute("INSERT INTO service_notification_preparations(id,instance_id,author_id,operation_key,digest,thread_id,membership_revision,lifecycle_revision,interval_high_water,requirement_high_water,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'building')",
        params![id,instance,author.as_str(),request.operation.as_str(),digest.as_slice(),request.thread.as_str(),mr,lr,ih,rh]).map_err(store_error)?;
    Ok((id, ih, rh, 0, 0, 0, "building".into()))
}
fn stage(tx: &rusqlite::Transaction<'_>, id: &str, seat: &str) -> Result<bool, ApiError> {
    let inserted = tx.execute("INSERT OR IGNORE INTO service_notification_recipients(preparation_id,seat_id) VALUES(?1,?2)",params![id,seat]).map_err(store_error)?;
    Ok(inserted == 1)
}

/// The authority guard is held from after BEGIN IMMEDIATE through commit.
/// The event is visible immediately with its immutable staged audience.
pub fn publish_notify(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceNotify,
    budget: &CallBudget,
) -> Result<ServiceResult, ApiError> {
    match publish_notify_internal(context, db, connection, gate, request, budget)? {
        PublishProgress::Committed(result) => Ok(result),
        PublishProgress::AudienceDrift => Err(api_error(
            ErrorCode::Conflict,
            "notification audience changed",
        )),
    }
}

fn publish_notify_internal(
    context: &StoreContext,
    db: &mut Connection,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    request: &ServiceNotify,
    budget: &CallBudget,
) -> Result<PublishProgress, ApiError> {
    if budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::Cancelled,
            "notification budget exhausted",
        ));
    }
    let body = event(request)?;
    let digest = schema::canonical_digest(&payload(request))?;
    let decision =
        ServiceDecisionTransaction::begin(db, gate, connection).map_err(|error| match error {
            ServiceDecisionStartError::Database(error) => store_error(error),
            ServiceDecisionStartError::Authority(error) => error,
        })?;
    let tx = decision.transaction();
    if let Some(result) = replay(
        tx,
        connection.instance(),
        decision.author(),
        request,
        &digest,
    )? {
        decision.rollback().map_err(store_error)?;
        return Ok(PublishProgress::Committed(result));
    }
    let prep:Option<(String,i64,i64,i64,String)> = tx.query_row(
        "SELECT id,membership_revision,lifecycle_revision,recipient_count,status FROM service_notification_preparations WHERE instance_id=?1 AND author_id=?2 AND operation_key=?3 AND digest=?4 AND thread_id=?5 ORDER BY ordinal DESC LIMIT 1",
        params![connection.instance(),decision.author().as_str(),request.operation.as_str(),digest.as_slice(),request.thread.as_str()],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(store_error)?;
    let Some((id, mr, lr, count, status)) = prep else {
        return Err(api_error(
            ErrorCode::Conflict,
            "notification preparation missing",
        ));
    };
    if status != "sealed" {
        return Err(api_error(
            ErrorCode::Conflict,
            "notification preparation incomplete",
        ));
    }
    let (instance,current_mr,current_lr,next_sequence): (String,i64,i64,i64)=tx.query_row(
        "SELECT t.instance_id,t.membership_revision,h.lifecycle_revision,t.next_sequence FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1",
        [request.thread.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(store_error)?;
    if instance != connection.instance() {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "thread belongs to another instance",
        ));
    }
    if mr != current_mr || lr != current_lr {
        discard_preparation(tx, &id)?;
        decision.commit().map_err(store_error)?;
        return Ok(PublishProgress::AudienceDrift);
    }
    if budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::Cancelled,
            "notification budget exhausted",
        ));
    }
    let seq = schema::next_decision_seq(tx, connection.instance())?;
    let following = next_sequence
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "timeline exhausted"))?;
    let message = MessageId::new(id.replacen("notify-prep-", "notify-", 1));
    let kind = match request.severity {
        NotificationSeverity::Info => "info",
        NotificationSeverity::Warn => "warn",
    };
    let now = context.clock().utc_now();
    tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,actor_label,event_json,decision_seq,decision_at,author_kind,author_service_id) VALUES(?1,?2,?3,?4,?5,?6,'herdr-graph',?7,?8,?9,'programmatic',?10)",
        params![message.as_str(),instance,request.thread.as_str(),next_sequence,kind,format!("system_notify:{}",message.as_str()),body,seq as i64,now.0,decision.author().as_str()]).map_err(store_error)?;
    tx.execute("INSERT INTO service_notification_publications(preparation_id,message_id,decision_seq,recipient_count) VALUES(?1,?2,?3,?4)",params![id,message.as_str(),seq as i64,count]).map_err(store_error)?;
    tx.execute(
        "UPDATE service_notification_preparations SET status='published' WHERE id=?1",
        [id.as_str()],
    )
    .map_err(store_error)?;
    tx.execute(
        "UPDATE threads SET next_sequence=?1,updated_at=?2 WHERE id=?3",
        params![following, now.0, request.thread.as_str()],
    )
    .map_err(store_error)?;
    schema::bump_timeline_revision(tx, &request.thread)?;
    schema::bump_filter_revision(tx, &instance, "directory", request.thread.as_str())?;
    if matches!(request.severity, NotificationSeverity::Warn) && count > 0 {
        let high: i64 = tx
            .query_row(
                "SELECT max(ordinal) FROM service_notification_recipients WHERE preparation_id=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES(?1,'send_attention',?2,?3)",
            params![format!("work:service-notify:{id}"),format!("service-notify:{id}"),high]).map_err(store_error)?;
    }
    let result = ServiceResult::Notification(ServiceNotification {
        summary: MessageSummary {
            message,
            thread: request.thread.clone(),
            author: None,
            kind: match request.severity {
                NotificationSeverity::Info => MessageKind::Info,
                NotificationSeverity::Warn => MessageKind::Warn,
            },
            sequence: next_sequence as u64,
            created_at: UtcMillis(now.0),
            actor_label: Some("herdr-graph".into()),
            preview_data: body.chars().take(256).collect(),
            preview_omitted: body.chars().count() > 256,
            preview_detail_argv: None,
            event_author: Some(crate::protocol::service::EventAuthor::Programmatic(
                decision.author().clone(),
            )),
        },
        author: decision.author().clone(),
    });
    tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES(?1,?2,?3,?4,?5)",
        params![scope(&instance,decision.author()),request.operation.as_str(),digest.as_slice(),serde_json::to_string(&result).map_err(|_|api_error(ErrorCode::StoreCorrupt,"notification result encoding failed"))?,now.0]).map_err(store_error)?;
    decision.commit().map_err(store_error)?;
    Ok(PublishProgress::Committed(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{DurableWorkAdmission, ServiceDecisionGuard, ServiceWriteTransactionProof};
    use crate::protocol::{
        commands::{Command, DirectoryMembership, DirectoryQuery, HistoryQuery, ParticipantsQuery},
        ids::{OperationId, SeatId, ThreadId},
        pagination::PageRequest,
        results::CommandResult,
        time::{Cancellation, MonoInstant},
    };
    use crate::store::{effective, materialization, queries, service_substrate};
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex, MutexGuard},
    };
    struct FixedClock;
    impl crate::protocol::time::Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            if connection.author() != &self.0 {
                return Err(api_error(ErrorCode::Unauthorized, "wrong service"));
            }
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }
    struct GenerationState {
        next: u64,
        active: Option<ServiceConnectionAuthority>,
    }
    struct GenerationGate(Mutex<GenerationState>);
    struct GenerationGuard<'a> {
        _state: MutexGuard<'a, GenerationState>,
        author: ServiceAuthorId,
    }
    impl ServiceDecisionGuard for GenerationGuard<'_> {
        fn author(&self) -> &ServiceAuthorId {
            &self.author
        }
    }
    impl GenerationGate {
        fn new() -> Self {
            Self(Mutex::new(GenerationState {
                next: 0,
                active: None,
            }))
        }
    }
    impl ServiceAuthorityGate for GenerationGate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            let mut state = self.0.lock().unwrap();
            state.next += 1;
            state.active = Some(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                state.next,
                author.clone(),
            ));
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                state.next,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            let state = self.0.lock().unwrap();
            if state.active.as_ref() != Some(connection) {
                return Err(api_error(
                    ErrorCode::StaleServiceGeneration,
                    "service generation revoked",
                ));
            }
            Ok(Box::new(GenerationGuard {
                _state: state,
                author: connection.author().clone(),
            }))
        }
        fn revoke_exact(&self, connection: &ServiceConnectionAuthority) -> bool {
            let mut state = self.0.lock().unwrap();
            if state.active.as_ref() != Some(connection) {
                return false;
            }
            state.active = None;
            true
        }
    }
    fn fixture() -> (
        StoreContext,
        Connection,
        ServiceConnectionAuthority,
        Gate,
        PathBuf,
    ) {
        let path = std::env::temp_dir().join(format!("service-notify-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let db = context.open_writer().unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0);").unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let author = service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
        tx.commit().unwrap();
        let connection =
            ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author.clone());
        (context, db, connection, Gate(author), path)
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(10_000),
            cancellation: Cancellation::default(),
        }
    }
    fn request(key: &str, severity: NotificationSeverity) -> ServiceNotify {
        ServiceNotify {
            thread: ThreadId::new("t"),
            severity,
            event_json: json!({"text":"notice"}),
            operation: OperationId::new(key),
        }
    }
    fn stage_all(
        context: &StoreContext,
        db: &mut Connection,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        request: &ServiceNotify,
    ) -> usize {
        let mut steps = 0;
        loop {
            steps += 1;
            match prepare_notify_step(context, db, connection, gate, request, &budget(), 16)
                .unwrap()
            {
                PreparationStep::More { visited, .. } => assert!(visited > 0),
                PreparationStep::Ready { .. } => return steps,
                PreparationStep::Committed(_) => panic!("unexpected replay"),
            }
        }
    }
    fn seed_joined(db: &Connection, seat: &str) {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES(?1,'i','resolved','native',1,0)",[seat]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES('t',?1,'joined',1)",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,1)",[seat]).unwrap();
    }
    fn seed_pending(db: &Connection, seat: &str, author: &ServiceAuthorId) {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES(?1,'i','resolved','native',1,0)",[seat]).unwrap();
        let invitation = format!("inv-{seat}");
        let requirement = format!("req-{seat}");
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES(?1,'t',?2,1,'pending',1,1,100,101)",params![invitation,seat]).unwrap();
        db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES(?1,'t',?2,?3,?4,'pending',1,1)",params![requirement,seat,author.as_str(),invitation]).unwrap();
    }
    fn count(db: &Connection, table: &str) -> i64 {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn notification_state(db: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        [
            "service_notification_preparations",
            "service_notification_recipients",
            "service_notification_publications",
            "operations",
            "messages",
            "work_jobs",
            "threads",
            "host_instances",
            "sqlite_sequence",
        ]
        .into_iter()
        .map(|table| {
            let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table.to_owned(), rows)
        })
        .collect()
    }

    #[test]
    fn wrong_current_authority_leaves_no_notification_staging_or_ordinals() {
        let (context, db, connection, _, path) = fixture();
        seed_joined(&db, "seat");
        let before = notification_state(&db);
        drop(db);
        let store =
            crate::store::SqliteStore::new(context, "i", crate::store::StoreSettings::default())
                .unwrap();
        let wrong_gate = Gate(ServiceAuthorId::new("other"));
        assert_eq!(
            store
                .service_notify(
                    &request("denied-staging", NotificationSeverity::Warn),
                    &connection,
                    &wrong_gate,
                    &budget()
                )
                .unwrap_err()
                .code,
            ErrorCode::Unauthorized
        );
        let db = store.context.open_writer().unwrap();
        assert_eq!(notification_state(&db), before);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stale_generation_cannot_stage_and_revocation_stops_after_committed_prefix() {
        let (context, mut db, original, _, path) = fixture();
        for n in 0..20 {
            seed_joined(&db, &format!("seat-{n:02}"));
        }
        let gate = GenerationGate::new();
        let active = gate
            .register("i", "boot", original.author().clone())
            .unwrap();
        let wrong = ServiceConnectionAuthority::new(
            "i".into(),
            "boot".into(),
            99,
            original.author().clone(),
        );
        let request = request("generation-fence", NotificationSeverity::Warn);
        let empty = notification_state(&db);
        assert_eq!(
            prepare_notify_step(&context, &mut db, &wrong, &gate, &request, &budget(), 16)
                .unwrap_err()
                .code,
            ErrorCode::StaleServiceGeneration
        );
        assert_eq!(notification_state(&db), empty);
        let first = prepare_notify_step(&context, &mut db, &active, &gate, &request, &budget(), 16)
            .unwrap();
        assert!(matches!(first, PreparationStep::More { visited: 16, .. }));
        let prefix = notification_state(&db);
        assert!(gate.revoke_exact(&active));
        assert_eq!(
            prepare_notify_step(&context, &mut db, &active, &gate, &request, &budget(), 16)
                .unwrap_err()
                .code,
            ErrorCode::StaleServiceGeneration
        );
        assert_eq!(notification_state(&db), prefix);
        drop(db);
        let store =
            crate::store::SqliteStore::new(context, "i", crate::store::StoreSettings::default())
                .unwrap();
        assert_eq!(
            store
                .service_notify(&request, &active, &gate, &budget())
                .unwrap_err()
                .code,
            ErrorCode::StaleServiceGeneration
        );
        let db = store.context.open_writer().unwrap();
        assert_eq!(notification_state(&db), prefix);
        drop(db);
        let reconnected = gate
            .register("i", "new-boot", original.author().clone())
            .unwrap();
        let result = store
            .service_notify(&request, &reconnected, &gate, &budget())
            .unwrap();
        assert_eq!(
            store
                .service_notify(&request, &reconnected, &gate, &budget())
                .unwrap(),
            result
        );
        let mut changed = request.clone();
        changed.event_json = json!({"text":"changed"});
        assert_eq!(
            store
                .service_notify(&changed, &reconnected, &gate, &budget())
                .unwrap_err()
                .code,
            ErrorCode::OperationPayloadMismatch
        );
        let db = store.context.open_writer().unwrap();
        assert_eq!(count(&db, "service_notification_publications"), 1);
        assert_eq!(count(&db, "messages"), 1);
        assert_eq!(count(&db, "receipts"), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    fn measured_vm_steps<T>(
        db: &mut Connection,
        action: impl FnOnce(&mut Connection) -> T,
    ) -> (T, usize) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        unsafe extern "C" fn count_step(data: *mut std::ffi::c_void) -> i32 {
            let counter = unsafe { &*(data as *const AtomicUsize) };
            counter.fetch_add(1, Ordering::Relaxed);
            0
        }
        struct Reset(*mut rusqlite::ffi::sqlite3);
        impl Drop for Reset {
            fn drop(&mut self) {
                unsafe {
                    rusqlite::ffi::sqlite3_progress_handler(self.0, 0, None, std::ptr::null_mut())
                };
            }
        }
        let counter = AtomicUsize::new(0);
        let handle = unsafe { db.handle() };
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                handle,
                1,
                Some(count_step),
                (&counter as *const AtomicUsize).cast_mut().cast(),
            );
        }
        let reset = Reset(handle);
        let result = action(db);
        drop(reset);
        (result, counter.load(Ordering::Relaxed))
    }

    #[test]
    fn large_audience_late_quantum_and_publication_have_bounded_sqlite_work() {
        fn late_quantum(prior: usize) -> (usize, usize) {
            let (context, mut db, connection, gate, path) = fixture();
            for n in 0..prior + 16 {
                seed_joined(&db, &format!("seat-{n:04}"));
            }
            let request = request("work-probe", NotificationSeverity::Warn);
            let first = prepare_notify_step(
                &context,
                &mut db,
                &connection,
                &gate,
                &request,
                &budget(),
                16,
            )
            .unwrap();
            let PreparationStep::More { preparation_id, .. } = first else {
                panic!("expected first quantum")
            };
            let cursor: i64 = db
                .query_row(
                    "SELECT ordinal FROM membership_intervals WHERE thread_id='t' AND seat_id=?1",
                    [format!("seat-{:04}", prior - 1)],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute("INSERT OR IGNORE INTO service_notification_recipients(preparation_id,seat_id) SELECT ?1,seat_id FROM membership_intervals WHERE thread_id='t' AND ordinal<=?2 ORDER BY ordinal", params![preparation_id,cursor]).unwrap();
            db.execute("UPDATE service_notification_preparations SET interval_cursor=?1,recipient_count=?2 WHERE id=?3", params![cursor,prior as i64,preparation_id]).unwrap();
            let (step, prepare_steps) = measured_vm_steps(&mut db, |db| {
                prepare_notify_step(&context, db, &connection, &gate, &request, &budget(), 16)
                    .unwrap()
            });
            assert!(matches!(step, PreparationStep::Ready { visited: 16, .. }));
            assert_eq!(
                db.query_row(
                    "SELECT recipient_count FROM service_notification_preparations WHERE id=?1",
                    [&preparation_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                (prior + 16) as i64
            );
            let (result, publish_steps) = measured_vm_steps(&mut db, |db| {
                publish_notify(&context, db, &connection, &gate, &request, &budget()).unwrap()
            });
            let ServiceResult::Notification(notice) = result else {
                panic!("expected notification")
            };
            let mut position = None;
            let mut seats = Vec::new();
            loop {
                let page = effective::scan_effective_warning_recipients(
                    &db,
                    notice.summary.message.as_str(),
                    position,
                    100,
                )
                .unwrap();
                seats.extend(page.seats);
                if !page.has_more {
                    break;
                }
                position = Some(page.position);
            }
            let expected: Vec<_> = (0..prior + 16).map(|n| format!("seat-{n:04}")).collect();
            assert_eq!(seats, expected);
            assert_eq!(count(&db, "receipts"), 0);
            drop(db);
            let _ = std::fs::remove_file(path);
            (prepare_steps, publish_steps)
        }
        let small = late_quantum(16);
        let large = late_quantum(4096);
        eprintln!("late notification VM steps: small={small:?}, large={large:?}");
        assert!(
            large.0 <= small.0 + 2_000,
            "preparation work grew with prior audience: {small:?} {large:?}"
        );
        assert!(
            large.1 <= small.1 + 2_000,
            "publication work grew with audience: {small:?} {large:?}"
        );
    }

    #[test]
    fn required_only_history_keeps_high_water_and_late_quantum_bounded() {
        fn required_work(prior: usize) -> (usize, usize, usize, Vec<String>) {
            let (context, mut db, connection, gate, path) = fixture();
            db.execute(
                "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
                [connection.author().as_str()],
            )
            .unwrap();
            for n in 0..prior + 16 {
                seed_pending(&db, &format!("seat-{n:04}"), connection.author());
            }
            let prior_last: i64 = db
                .query_row(
                    "SELECT ordinal FROM requirement_episodes WHERE id=?1",
                    [format!("req-seat-{:04}", prior - 1)],
                    |r| r.get(0),
                )
                .unwrap();
            db.execute("UPDATE requirement_episodes SET state='released',revision=revision+1,released_at=2 WHERE ordinal<=?1 AND ordinal%2=0", [prior_last]).unwrap();
            db.execute("UPDATE requirement_episodes SET state='retired',revision=revision+1,retired_at=2 WHERE ordinal<=?1 AND ordinal%2=1", [prior_last]).unwrap();
            let request = request("required-work", NotificationSeverity::Info);
            let (first, high_water_steps) = measured_vm_steps(&mut db, |db| {
                prepare_notify_step(&context, db, &connection, &gate, &request, &budget(), 1)
                    .unwrap()
            });
            let PreparationStep::More {
                preparation_id,
                visited: 1,
            } = first
            else {
                panic!("expected one retained requirement")
            };
            db.execute(
                "UPDATE service_notification_preparations SET requirement_cursor=?1 WHERE id=?2",
                params![prior_last, preparation_id],
            )
            .unwrap();
            let (late, late_steps) = measured_vm_steps(&mut db, |db| {
                prepare_notify_step(&context, db, &connection, &gate, &request, &budget(), 16)
                    .unwrap()
            });
            assert!(matches!(late, PreparationStep::Ready { visited: 16, .. }));
            assert_eq!(
                db.query_row(
                    "SELECT recipient_count FROM service_notification_preparations WHERE id=?1",
                    [&preparation_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                16
            );
            let high: i64 = db.query_row("SELECT requirement_high_water FROM service_notification_preparations WHERE id=?1", [&preparation_id], |r| r.get(0)).unwrap();
            let (none, no_next_steps) = measured_vm_steps(&mut db, |db| {
                db.query_row(
                "SELECT r.ordinal,r.seat_id,(s.state!='retired' AND s.retired_seq IS NULL AND r.state='pending') FROM requirement_episodes r JOIN seats s ON s.id=r.seat_id WHERE r.thread_id=?1 AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT 1",
                params!["t",high,high], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?))).optional().unwrap()
            });
            assert!(none.is_none());
            let plan: Vec<String> = db.prepare("EXPLAIN QUERY PLAN SELECT r.ordinal,r.seat_id FROM requirement_episodes r JOIN seats s ON s.id=r.seat_id WHERE r.thread_id=?1 AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT 1")
                .unwrap().query_map(params!["t",prior_last,high], |r|r.get(3)).unwrap().collect::<Result<_,_>>().unwrap();
            drop(db);
            let _ = std::fs::remove_file(path);
            (high_water_steps, late_steps, no_next_steps, plan)
        }
        let small = required_work(16);
        let large = required_work(4096);
        eprintln!(
            "required notification VM steps high/late/none: small={small:?}, large={large:?}"
        );
        assert!(
            large.0 <= small.0 + 2_000,
            "high-water work grew with retained history: {small:?} {large:?}"
        );
        assert!(
            large.1 <= small.1 + 2_000,
            "late quantum grew with retained history: {small:?} {large:?}"
        );
        assert!(
            large.2 <= small.2 + 2_000,
            "no-next-row work grew with retained history: {small:?} {large:?}"
        );
        assert!(
            !large.3.iter().any(|line| line.contains("TEMP B-TREE")),
            "{:?}",
            large.3
        );
    }

    #[test]
    fn recipient_count_overflow_rolls_back_the_entire_quantum() {
        let (context, mut db, connection, gate, path) = fixture();
        for n in 0..20 {
            seed_joined(&db, &format!("seat-{n:02}"));
        }
        let request = request("count-overflow", NotificationSeverity::Info);
        let first = prepare_notify_step(
            &context,
            &mut db,
            &connection,
            &gate,
            &request,
            &budget(),
            16,
        )
        .unwrap();
        let PreparationStep::More { preparation_id, .. } = first else {
            panic!("expected first quantum")
        };
        db.execute(
            "UPDATE service_notification_preparations SET recipient_count=?1 WHERE id=?2",
            params![i64::MAX, preparation_id],
        )
        .unwrap();
        let before: (i64, i64) = db
            .query_row(
                "SELECT interval_cursor,recipient_count FROM service_notification_preparations WHERE id=?1",
                [&preparation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            prepare_notify_step(
                &context,
                &mut db,
                &connection,
                &gate,
                &request,
                &budget(),
                16
            )
            .unwrap_err()
            .code,
            ErrorCode::StoreCorrupt
        );
        assert_eq!(count(&db, "service_notification_recipients"), 16);
        assert_eq!(
            db.query_row(
                "SELECT interval_cursor,recipient_count FROM service_notification_preparations WHERE id=?1",
                [&preparation_id],
                |r| Ok((r.get(0)?, r.get(1)?))
            )
            .unwrap(),
            before
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn warn_snapshot_deduplicates_joined_and_pending_and_creates_no_receipts() {
        let (context, mut db, connection, gate, path) = fixture();
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
            [connection.author().as_str()],
        )
        .unwrap();
        seed_joined(&db, "joined");
        seed_joined(&db, "both");
        let invitation = "inv-both";
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES(?1,'t','both',1,'pending',1,1,100,101)",[invitation]).unwrap();
        db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES('req-both','t','both',?1,?2,'pending',1,1)",params![connection.author().as_str(),invitation]).unwrap();
        seed_pending(&db, "pending", connection.author());
        seed_joined(&db, "retired");
        db.execute(
            "UPDATE seats SET state='retired',retired_seq=2,retired_at=2 WHERE id='retired'",
            [],
        )
        .unwrap();
        let request = request("warn-one", NotificationSeverity::Warn);
        stage_all(&context, &mut db, &connection, &gate, &request);
        let result =
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget()).unwrap();
        let ServiceResult::Notification(notice) = result.clone() else {
            panic!()
        };
        assert_eq!(notice.author, *connection.author());
        assert_eq!(
            notice.summary.event_author,
            Some(crate::protocol::service::EventAuthor::Programmatic(
                connection.author().clone()
            ))
        );
        assert_eq!(count(&db, "receipts"), 0);
        assert_eq!(count(&db, "receipt_state"), 0);
        assert_eq!(
            db.query_row(
                "SELECT recipient_count FROM service_notification_publications",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            3
        );
        let mut positions = None;
        let mut seats = Vec::new();
        loop {
            let page = effective::scan_effective_warning_recipients(
                &db,
                notice.summary.message.as_str(),
                positions,
                2,
            )
            .unwrap();
            seats.extend(page.seats);
            if !page.has_more {
                break;
            }
            positions = Some(page.position);
        }
        seats.sort();
        assert_eq!(seats, ["both", "joined", "pending"]);
        assert!(
            effective::is_warning_recipient(&db, notice.summary.message.as_str(), "pending")
                .unwrap()
        );
        assert!(
            effective::warning_condition_actionable(
                &db,
                &effective::effective_warning_by_id(&db, notice.summary.message.as_str())
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        let before_projection =
            effective::scan_effective_seat_attention(&db, "pending", None, 100).unwrap();
        assert!(!before_projection.has_more);
        assert_eq!(
            before_projection.attention.unwrap().latest_warning_seq,
            Some(1)
        );
        let reconnected = ServiceConnectionAuthority::new(
            "i".into(),
            "new-boot".into(),
            2,
            connection.author().clone(),
        );
        let replay =
            publish_notify(&context, &mut db, &reconnected, &gate, &request, &budget()).unwrap();
        assert_eq!(replay, result);
        assert_eq!(count(&db, "messages"), 1);
        let mut changed = request.clone();
        changed.event_json = json!({"text":"changed"});
        assert_eq!(
            publish_notify(&context, &mut db, &connection, &gate, &changed, &budget())
                .unwrap_err()
                .code,
            ErrorCode::OperationPayloadMismatch
        );
        let job = format!(
            "work:service-notify:{}",
            notice
                .summary
                .message
                .as_str()
                .replacen("notify-", "notify-prep-", 1)
        );
        for _ in 0..10 {
            let progress = materialization::advance_work(
                &mut db,
                &job,
                DurableWorkAdmission::new(1).unwrap(),
                &budget(),
                context.clock(),
            )
            .unwrap();
            if !progress.has_more {
                break;
            }
        }
        assert_eq!(count(&db, "warning_recipients"), 3);
        assert_eq!(count(&db, "wake_work"), 3);
        assert_eq!(count(&db, "receipts"), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    /// The seat's digest frontier and warning IDs, read in one transaction.
    fn digest_warnings(
        db: &Connection,
        seat: &str,
        phase: &str,
    ) -> (Vec<String>, Option<(u64, u64)>, u64) {
        db.execute_batch("BEGIN DEFERRED").unwrap();
        let run = crate::store::attention::seat_digest(
            db,
            "i",
            &crate::protocol::ids::SeatId::new(seat),
            &|| Ok(()),
        )
        .unwrap();
        db.execute_batch("COMMIT").unwrap();
        let canonical = {
            let slice = effective::scan_effective_seat_attention(db, seat, None, 100).unwrap();
            assert!(!slice.has_more);
            slice.attention.unwrap().frontier
        };
        assert_eq!(run.frontier, canonical, "{phase}, {seat}");
        (
            run.digest
                .warnings
                .items
                .iter()
                .map(|item| item.id.clone())
                .collect(),
            run.digest.token.warning,
            run.work_steps,
        )
    }

    // Digest fix2 S1: a service `warn` notification reaches a pending-required
    // seat that is not a member (no membership interval) and a joined member.
    // Before the recipient projection work runs, the staged service recipients
    // are the only path to it; afterwards the projected `warning_recipients`
    // (kept in `digest_programmatic_warnings`) are the only path, because the
    // backlog walk no longer names the completed publication. An `info`
    // notice to the same seats never enters the digest. Kills: "service-
    // notification source dropped" (fails before projection) and "projected-
    // recipient source dropped" (fails after projection); each frontier must
    // also equal the wake scheduler's canonical scan.
    #[test]
    fn digest_names_a_warn_notice_through_staged_then_projected_recipients() {
        let (context, mut db, connection, gate, path) = fixture();
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
            [connection.author().as_str()],
        )
        .unwrap();
        seed_joined(&db, "member");
        seed_pending(&db, "req", connection.author());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM membership_intervals WHERE seat_id='req'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let info = request("digest-info", NotificationSeverity::Info);
        stage_all(&context, &mut db, &connection, &gate, &info);
        publish_notify(&context, &mut db, &connection, &gate, &info, &budget()).unwrap();
        let (none, token, _) = digest_warnings(&db, "req", "info only");
        assert!(none.is_empty() && token.is_none(), "{none:?}");
        let request = request("digest-warn", NotificationSeverity::Warn);
        stage_all(&context, &mut db, &connection, &gate, &request);
        let result =
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget()).unwrap();
        let ServiceResult::Notification(notice) = result else {
            panic!()
        };
        let warning = notice.summary.message.as_str().to_owned();
        assert_eq!(count(&db, "warning_recipients"), 0);
        assert_eq!(count(&db, "digest_programmatic_warnings"), 0);
        for seat in ["req", "member"] {
            let (ids, token, _) = digest_warnings(&db, seat, "staged");
            assert_eq!(ids, std::slice::from_ref(&warning), "staged, {seat}");
            assert!(token.is_some(), "{seat}");
        }
        let job: String = db
            .query_row(
                "SELECT id FROM work_jobs WHERE kind='send_attention'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for _ in 0..10 {
            let progress = materialization::advance_work(
                &mut db,
                &job,
                DurableWorkAdmission::new(1).unwrap(),
                &budget(),
                context.clock(),
            )
            .unwrap();
            if !progress.has_more {
                break;
            }
        }
        assert_eq!(
            db.query_row("SELECT status FROM work_jobs WHERE id=?1", [&job], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "complete"
        );
        assert_eq!(count(&db, "warning_recipients"), 2);
        assert_eq!(count(&db, "digest_programmatic_warnings"), 2);
        for seat in ["req", "member"] {
            let (ids, token, steps) = digest_warnings(&db, seat, "projected");
            assert_eq!(ids, std::slice::from_ref(&warning), "projected, {seat}");
            assert!(token.is_some(), "{seat}");
            // No staged recipient row is visited once the projection is done.
            assert!(steps <= 4, "{seat}: {steps}");
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    // Digest fix3 N2: a service-notify projection job whose unit fails is
    // left `failed` with its recipients still unprojected; until it is
    // retried the staged service recipients are the only path to the warning,
    // so the digest must still name it to the pending-required non-member
    // and to the member. Kills: "failed backlog excluded" (the staged-service
    // backlog walk reading only `status IN ('pending')`).
    #[test]
    fn digest_names_a_warn_notice_whose_projection_job_failed() {
        let (context, mut db, connection, gate, path) = fixture();
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
            [connection.author().as_str()],
        )
        .unwrap();
        seed_joined(&db, "member");
        seed_pending(&db, "req", connection.author());
        let request = request("digest-failed", NotificationSeverity::Warn);
        stage_all(&context, &mut db, &connection, &gate, &request);
        let result =
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget()).unwrap();
        let ServiceResult::Notification(notice) = result else {
            panic!()
        };
        let warning = notice.summary.message.as_str().to_owned();
        let job: String = db
            .query_row(
                "SELECT id FROM work_jobs WHERE kind='send_attention'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Every recipient projection unit fails.
        db.execute_batch(
            "CREATE TEMP TRIGGER fail_projection BEFORE INSERT ON warning_recipients BEGIN SELECT RAISE(ABORT,'injected projection failure'); END;",
        )
        .unwrap();
        let progress = materialization::advance_work(
            &mut db,
            &job,
            DurableWorkAdmission::new(1).unwrap(),
            &budget(),
            context.clock(),
        )
        .unwrap();
        assert!(progress.has_more);
        assert!(
            progress
                .last_error
                .as_deref()
                .is_some_and(|e| e.contains("injected projection failure")),
            "{progress:?}"
        );
        assert_eq!(
            db.query_row("SELECT status FROM work_jobs WHERE id=?1", [&job], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "failed"
        );
        assert_eq!(count(&db, "warning_recipients"), 0);
        assert_eq!(count(&db, "digest_programmatic_warnings"), 0);
        for seat in ["req", "member"] {
            let (ids, token, _) = digest_warnings(&db, seat, "failed");
            assert_eq!(ids, std::slice::from_ref(&warning), "failed, {seat}");
            assert!(token.is_some(), "{seat}");
        }
        // The retry projects both recipients and completes the job.
        db.execute_batch("DROP TRIGGER temp.fail_projection")
            .unwrap();
        for _ in 0..10 {
            let progress = materialization::advance_work(
                &mut db,
                &job,
                DurableWorkAdmission::new(1).unwrap(),
                &budget(),
                context.clock(),
            )
            .unwrap();
            if !progress.has_more {
                break;
            }
        }
        assert_eq!(count(&db, "warning_recipients"), 2);
        for seat in ["req", "member"] {
            let (ids, _, _) = digest_warnings(&db, seat, "retried");
            assert_eq!(ids, std::slice::from_ref(&warning), "retried, {seat}");
        }
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn info_archived_large_audience_and_revision_retry() {
        let (context, mut db, connection, gate, path) = fixture();
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1,archived=1 WHERE id='t'",
            [connection.author().as_str()],
        )
        .unwrap();
        for n in 0..205 {
            seed_joined(&db, &format!("seat-{n:03}"));
        }
        seed_pending(&db, "pending-changing", connection.author());
        let request = request("info-one", NotificationSeverity::Info);
        assert!(stage_all(&context, &mut db, &connection, &gate, &request) > 1);
        db.execute("UPDATE requirement_episodes SET state='released',revision=2,released_at=2 WHERE id='req-pending-changing'",[]).unwrap();
        db.execute(
            "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
            [],
        )
        .unwrap();
        assert_eq!(
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget())
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        stage_all(&context, &mut db, &connection, &gate, &request);
        let result =
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget()).unwrap();
        let ServiceResult::Notification(notice) = result else {
            panic!()
        };
        assert_eq!(notice.summary.kind, MessageKind::Info);
        assert_eq!(count(&db, "service_notification_recipients"), 411);
        assert_eq!(
            db.query_row(
                "SELECT recipient_count FROM service_notification_publications",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            205
        );
        let discarded: String = db
            .query_row(
                "SELECT id FROM service_notification_preparations WHERE status='discarded'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cleanup = format!("work:service-notify-cleanup:{discarded}");
        for _ in 0..20 {
            let progress = materialization::advance_work(
                &mut db,
                &cleanup,
                DurableWorkAdmission::new(16).unwrap(),
                &budget(),
                context.clock(),
            )
            .unwrap();
            if !progress.has_more {
                break;
            }
        }
        assert_eq!(count(&db, "service_notification_recipients"), 205);
        assert_eq!(
            db.query_row("SELECT status FROM work_jobs WHERE id=?1", [cleanup], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "complete"
        );
        assert_eq!(count(&db, "work_jobs"), 1);
        assert_eq!(count(&db, "wake_work"), 0);
        assert_eq!(count(&db, "receipts"), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn service_call_reprepares_audience_drift_in_both_windows() {
        for drift_after_seal in [false, true] {
            let (context, db, connection, gate, path) = fixture();
            for n in 0..20 {
                seed_joined(&db, &format!("seat-{n:02}"));
            }
            drop(db);
            let store = crate::store::SqliteStore::new(
                context,
                "i",
                crate::store::StoreSettings::default(),
            )
            .unwrap();
            let request = request("drift", NotificationSeverity::Info);
            let mut changed = false;
            let result = store.service_notify_with_observer(
                &request,
                &connection,
                &gate,
                &budget(),
                |step, context| {
                    if !changed && matches!(step, PreparationStep::Ready { .. }) == drift_after_seal {
                        context.open_writer().unwrap().execute(
                            "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'",
                            [],
                        ).unwrap();
                        changed = true;
                    }
                },
            ).unwrap();
            assert!(changed);
            let ServiceResult::Notification(notice) = result else {
                panic!()
            };
            let db = store.context.open_writer().unwrap();
            assert_eq!(count(&db, "messages"), 1);
            assert_eq!(count(&db, "service_notification_publications"), 1);
            assert_eq!(count(&db, "receipts"), 0);
            assert_eq!(count(&db, "receipt_state"), 0);
            assert_eq!(
                store
                    .service_notify(&request, &connection, &gate, &budget())
                    .unwrap(),
                ServiceResult::Notification(notice)
            );
            assert_eq!(count(&db, "messages"), 1);
            drop(db);
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn service_call_audience_restart_allowance_is_finite() {
        let (context, db, connection, gate, path) = fixture();
        seed_joined(&db, "seat");
        drop(db);
        let store =
            crate::store::SqliteStore::new(context, "i", crate::store::StoreSettings::default())
                .unwrap();
        let mut changes = 0;
        let error = store.service_notify_with_observer(
            &request("repeated-drift", NotificationSeverity::Warn),
            &connection,
            &gate,
            &budget(),
            |step, context| {
                if matches!(step, PreparationStep::Ready { .. }) {
                    context.open_writer().unwrap().execute(
                        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'", [],
                    ).unwrap();
                    changes += 1;
                }
            },
        ).unwrap_err();
        assert_eq!(changes, usize::from(MAX_AUDIENCE_RESTARTS) + 1);
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(error.detail.contains("restart allowance"));
        let db = store.context.open_writer().unwrap();
        assert_eq!(count(&db, "messages"), 0);
        assert_eq!(count(&db, "operations"), 0);
        assert_eq!(count(&db, "receipts"), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn service_call_budget_and_unrelated_errors_do_not_restart() {
        let (context, db, connection, gate, path) = fixture();
        seed_joined(&db, "seat");
        drop(db);
        let store =
            crate::store::SqliteStore::new(context, "i", crate::store::StoreSettings::default())
                .unwrap();
        let call_budget = budget();
        let mut observations = 0;
        let error = store.service_notify_with_observer(
            &request("cancelled-drift", NotificationSeverity::Info),
            &connection,
            &gate,
            &call_budget,
            |step, context| {
                observations += 1;
                if matches!(step, PreparationStep::Ready { .. }) {
                    context.open_writer().unwrap().execute(
                        "UPDATE threads SET membership_revision=membership_revision+1 WHERE id='t'", [],
                    ).unwrap();
                    call_budget.cancellation.cancel();
                }
            },
        ).unwrap_err();
        assert_eq!(observations, 1);
        assert_eq!(error.code, ErrorCode::Cancelled);
        let invalid = ServiceNotify {
            event_json: json!({"text":"x".repeat(MAX_EVENT_BYTES)}),
            ..request("invalid", NotificationSeverity::Info)
        };
        assert_eq!(
            store
                .service_notify(&invalid, &connection, &gate, &budget())
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let wrong_gate = Gate(ServiceAuthorId::new("other"));
        assert_eq!(
            store
                .service_notify(
                    &request("wrong-authority", NotificationSeverity::Info),
                    &connection,
                    &wrong_gate,
                    &budget()
                )
                .unwrap_err()
                .code,
            ErrorCode::Unauthorized
        );
        let mut unrelated_observations = 0;
        let unrelated = store.service_notify_with_observer(
            &request("unrelated-conflict", NotificationSeverity::Info),
            &connection,
            &gate,
            &budget(),
            |step, context| {
                unrelated_observations += 1;
                if let PreparationStep::Ready { preparation_id, .. } = step {
                    context.open_writer().unwrap().execute(
                        "UPDATE service_notification_preparations SET status='discarded' WHERE id=?1",
                        [preparation_id],
                    ).unwrap();
                }
            },
        ).unwrap_err();
        assert_eq!(unrelated.code, ErrorCode::Conflict);
        assert_eq!(unrelated_observations, 1);
        let expired = CallBudget {
            deadline: MonoInstant(1),
            cancellation: Cancellation::default(),
        };
        assert_eq!(
            store
                .service_notify(
                    &request("expired", NotificationSeverity::Info),
                    &connection,
                    &gate,
                    &expired
                )
                .unwrap_err()
                .code,
            ErrorCode::DeadlineExceeded
        );
        let db = store.context.open_writer().unwrap();
        assert_eq!(count(&db, "messages"), 0);
        assert_eq!(count(&db, "operations"), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn mixed_history_directory_and_participants_preserve_author_and_requirement() {
        let (context, mut db, connection, gate, path) = fixture();
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
            [connection.author().as_str()],
        )
        .unwrap();
        seed_joined(&db, "native");
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES('inv-native','t','native',1,'pending',1,1,100,101)",[]).unwrap();
        db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES('req-native','t','native',?1,'inv-native','pending',1,1)",[connection.author().as_str()]).unwrap();
        let request = request("mixed-info", NotificationSeverity::Info);
        stage_all(&context, &mut db, &connection, &gate, &request);
        let notice =
            publish_notify(&context, &mut db, &connection, &gate, &request, &budget()).unwrap();
        let ServiceResult::Notification(notice) = notice else {
            panic!()
        };
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at) VALUES('native-message','i','t',2,'ordinary','native','hello',2,101)",[]).unwrap();
        db.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
            .unwrap();
        db.execute("UPDATE host_instances SET decision_seq=2 WHERE id='i'", [])
            .unwrap();
        let page = PageRequest {
            cursor: None,
            limit: 20,
            max_bytes: 65536,
        };
        let CommandResult::History(history) = queries::query(
            &context,
            "i",
            &Command::History(HistoryQuery {
                thread: ThreadId::new("t"),
                page: page.clone(),
                initial: None,
            }),
            &budget(),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(history.items.len(), 2);
        let native = history
            .items
            .iter()
            .find(|item| item.message.as_str() == "native-message")
            .unwrap();
        let system = history
            .items
            .iter()
            .find(|item| item.message == notice.summary.message)
            .unwrap();
        assert_eq!(native.author, Some(SeatId::new("native")));
        assert_eq!(
            native.event_author,
            Some(crate::protocol::service::EventAuthor::Native(SeatId::new(
                "native"
            )))
        );
        assert_eq!(system.author, None);
        assert_eq!(
            system.event_author,
            Some(crate::protocol::service::EventAuthor::Programmatic(
                connection.author().clone()
            ))
        );
        let CommandResult::Directory(directory) = queries::query(
            &context,
            "i",
            &Command::Directory(DirectoryQuery {
                membership: None,
                membership_filter: DirectoryMembership::All,
                topic_contains: None,
                page: page.clone(),
            }),
            &budget(),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            directory.items[0].managed_owner,
            Some(connection.author().clone())
        );
        let CommandResult::Participants(participants) = queries::query(
            &context,
            "i",
            &Command::Participants(ParticipantsQuery {
                thread: ThreadId::new("t"),
                page,
                caller: None,
            }),
            &budget(),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            participants.items[0].requirement.as_ref().unwrap().state,
            crate::protocol::service::RequirementState::Pending
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
