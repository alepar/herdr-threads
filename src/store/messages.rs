//! Immutable ordinary message creation and recipient snapshots.

use crate::ports::{DurableWorkAdmission, SendPreparationProgress};
use crate::protocol::{
    authority::{MutationPermit, ObligationRef},
    commands::{DeliveryMode, SendMessage},
    ids::{
        MessageId, SeatId, ServiceAuthorId, ThreadId, prefix, send_message_id_for_preparation,
        send_preparation_id_for_message,
    },
    results::{ApiError, CommandResult, ErrorCode},
    time::CallBudget,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use super::{
    attribution,
    connection::{StoreContext, api_error, store_error},
    effective::{self, UnavailableWarningKey},
    materialize::{FactRoute, MaterializeInput},
    schema,
};
use crate::sync::facts::message::{MessageSent, MessageSentPayload};
use crate::sync::facts::payload::{ReceiptObligation, RecipientSpec, receipt_settle_key};
use sha2::{Digest, Sha256};

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
    frozen_duration_ms(request.deadline_millis, limits)
}

/// The receipt duration a send freezes: the explicit deadline or the instance default.
pub(super) fn frozen_duration_ms(
    deadline_millis: Option<u64>,
    limits: MessageLimits,
) -> Result<i64, ApiError> {
    let duration = deadline_millis
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
    let mut payload = json!({"kind":"send_message","thread":request.thread,"body":request.body,
        "invited_recipients":request.invited_recipients,"deadline_millis":request.deadline_millis,"claim":request.claim});
    if let Some(recipient) = &request.direct_recipient {
        payload["direct_recipient"] = json!(recipient);
    }
    if !request.delivery_mode.is_ordinary() {
        payload["delivery_mode"] = json!(request.delivery_mode);
    }
    if request.relays_user {
        payload["relays_user"] = json!(true);
    }
    if let Some(intent) = request.user_intent {
        payload["user_intent"] = json!(intent);
    }
    payload
}

fn validate_delivery_options(request: &SendMessage) -> Result<(), ApiError> {
    if request.delivery_mode == DeliveryMode::Lazy
        && (!request.invited_recipients.is_empty() || request.deadline_millis.is_some())
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy delivery cannot require ACKs or a deadline",
        ));
    }
    Ok(())
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
    prepare_send_step_impl(context, conn, request, limits, budget, admission, None)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_automatic_send_step(
    context: &StoreContext,
    conn: &mut Connection,
    request: &crate::protocol::handoff::AutomaticDeliveryMutation,
    limits: MessageLimits,
    budget: &CallBudget,
    admission: DurableWorkAdmission,
) -> Result<SendPreparationProgress, ApiError> {
    let crate::protocol::handoff::AutomaticDeliveryAction::Send(send) = &request.action else {
        return Err(ApiError::invalid_request("not automatic Send"));
    };
    let requirement = crate::protocol::authority::HandoffRequirement::AutomaticDelivery(Box::new(
        request.clone(),
    ));
    prepare_send_step_impl(
        context,
        conn,
        send,
        limits,
        budget,
        admission,
        Some(&requirement),
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_delivery_send_step(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    limits: MessageLimits,
    budget: &CallBudget,
    admission: DurableWorkAdmission,
    delivery: &crate::protocol::handoff::DeliveryMutation,
) -> Result<SendPreparationProgress, ApiError> {
    let requirement =
        crate::protocol::authority::HandoffRequirement::Delivery(Box::new(delivery.clone()));
    prepare_send_step_impl(
        context,
        conn,
        request,
        limits,
        budget,
        admission,
        Some(&requirement),
    )
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_bootstrap_send_step(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    limits: MessageLimits,
    budget: &CallBudget,
    admission: DurableWorkAdmission,
    namespace: &crate::protocol::handoff::HandoffNamespace,
) -> Result<SendPreparationProgress, ApiError> {
    let requirement = crate::protocol::authority::HandoffRequirement::BootstrapChild {
        namespace: namespace.clone(),
        command: Box::new(crate::protocol::commands::PermitMutation::SendMessage(
            request.clone(),
        )),
    };
    prepare_send_step_impl(
        context,
        conn,
        request,
        limits,
        budget,
        admission,
        Some(&requirement),
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_send_step_impl(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    limits: MessageLimits,
    budget: &CallBudget,
    admission: DurableWorkAdmission,
    delivery: Option<&crate::protocol::authority::HandoffRequirement>,
) -> Result<SendPreparationProgress, ApiError> {
    validate_delivery_options(request)?;
    require_live_budget(context, budget)?;
    let max_units = admission.max_units;
    if max_units == 0 || max_units > 16 || request.invited_recipients.len() > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid send preparation admission",
        ));
    }
    let digest = schema::canonical_digest(&send_payload(request))?;
    let host =
        super::seats::cooperative_claim(conn, context, &request.claim.instance, &request.claim)?;
    let seat = request.claim.seat.clone();
    let scope = format!("seat:{}", seat.as_str());
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    if let Some(delivery) = delivery {
        super::handoff::validate_handoff_requirement(&tx, delivery, true)?;
    } else {
        super::topology_handoff::guard_unscoped_child_phase(
            &tx,
            &request.claim.instance,
            &scope,
            request.operation.as_str(),
        )?;
    }
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
    if let Some(delivery) = delivery {
        if matches!(
            delivery,
            crate::protocol::authority::HandoffRequirement::Delivery(_)
        ) {
            super::seats::cooperative_bound_mapping(&tx, host, &request.claim, None)?;
        }
        super::handoff::validate_handoff_requirement(&tx, delivery, false)?;
    }
    if request.body.is_empty() || request.body.len() > limits.body_bytes.min(MAX_BODY_BYTES) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "message body byte limit",
        ));
    }
    let duration = if request.delivery_mode == DeliveryMode::Lazy {
        0
    } else {
        frozen_duration(request, limits)?
    };
    let (instance, membership_revision, timeline_revision, archived): (String, i64, i64, bool) = tx.query_row(
        "SELECT instance_id,membership_revision,timeline_revision,archived FROM threads WHERE id=?1",
        [request.thread.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(store_error)?.ok_or_else(|| {
        api_error(
            ErrorCode::NotFound,
            format!("thread {} not found", request.thread.as_str()),
        )
    })?;
    super::control::validate_pair_send(&tx, request)?;
    if request.direct_recipient.is_some() {
        super::seats::cooperative_mapping(&tx, host, &request.claim, None)?;
    }
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
        "SELECT (SELECT COALESCE(SUM(lifecycle_revision),0) FROM herdr_hosts WHERE node_id=n.id),n.send_eligibility_revision,n.duration_config_revision FROM local_node n WHERE n.id=?1",
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
            let children:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM prepared_unavailable_warnings WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1) OR EXISTS(SELECT 1 FROM lazy_recipients WHERE preparation_id=?1)",[id.as_str()],|r|r.get(0)).map_err(store_error)?;
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
        } else if (mr, lr, er, cr)
            != (
                membership_revision,
                lifecycle_revision,
                eligibility_revision,
                config_revision,
            )
            || timeline_fence_stale(wn, tr, timeline_revision)
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
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12,'building',?13)",
            params![id,instance,scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str(),membership_revision,lifecycle_revision,eligibility_revision,timeline_revision,config_revision,high_water,if request.delivery_mode.is_ordinary() {"ordinary"} else {"lazy"}]).map_err(store_error)?;
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
        .chain(request.direct_recipient.iter())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let walk = AudienceWalk {
        prep_id: &prep_id,
        instance: &instance,
        thread: &request.thread,
        high_water,
        explicit: &explicit,
        exclude: Some(&seat),
        duration,
    };
    let end = walk_audience(
        &tx,
        context,
        budget,
        max_units,
        &walk,
        &mut cursor,
        &mut count,
        &mut warning_count,
    )?;
    let (visited, complete) = match end {
        WalkEnd::Complete { visited } => (visited, true),
        WalkEnd::More { visited } => (visited, false),
        WalkEnd::Rejected { .. } => {
            discard_preparation(&tx, &prep_id)?;
            tx.commit().map_err(store_error)?;
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "explicit recipient invalid or retired",
            ));
        }
    };
    tx.execute("UPDATE send_preparations SET recipient_cursor=?1,recipient_count=?2,warning_count=?3,status=?4,prepared_at=?6 WHERE id=?5",
        params![cursor,count,warning_count,if complete {"sealed"} else {"building"},prep_id,context.clock().utc_now().0]).map_err(store_error)?;
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

/// Why an explicit recipient was refused during the audience walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecipientRejection {
    Unknown,
    Retired,
    NotMemberOrInvitee,
}

/// Where one audience-walk quantum stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WalkEnd {
    Complete {
        visited: u8,
    },
    More {
        visited: u8,
    },
    Rejected {
        seat: SeatId,
        reason: RecipientRejection,
    },
}

/// The fixed inputs of one preparation's audience walk.
pub(super) struct AudienceWalk<'a> {
    pub prep_id: &'a str,
    pub instance: &'a str,
    pub thread: &'a ThreadId,
    pub high_water: i64,
    pub explicit: &'a [&'a SeatId],
    /// The sender, never its own recipient. `None` for a service author.
    pub exclude: Option<&'a SeatId>,
    pub duration: i64,
}

/// Stages at most `max_units` audience candidates: the joined snapshot up to the
/// interval high-water, then the explicit recipients. The caller owns the
/// transaction and persists the cursor and counters.
// Allowed: the walk updates the caller's three running counters.
#[allow(clippy::too_many_arguments)]
pub(super) fn walk_audience(
    tx: &Transaction<'_>,
    context: &StoreContext,
    budget: &CallBudget,
    max_units: u8,
    walk: &AudienceWalk<'_>,
    cursor: &mut i64,
    count: &mut i64,
    warning_count: &mut i64,
) -> Result<WalkEnd, ApiError> {
    // The 5 ms writer quantum runs on the store's clock, like the snapshot
    // and retirement quanta: an injected clock decides it deterministically.
    let began = context.clock().monotonic_now();
    let mut visited = 0u8;
    while visited < max_units
        && (visited == 0 || context.clock().monotonic_now().0.saturating_sub(began.0) < 5)
        && !budget.is_exhausted(context.clock())
    {
        if *cursor < walk.high_water {
            let row: Option<(i64,String,bool)> = tx.query_row(
                "SELECT mi.ordinal,mi.seat_id,(mi.left_seq IS NULL AND s.retired_seq IS NULL AND s.state!='retired') FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal>?2 AND mi.ordinal<=?3 ORDER BY mi.ordinal LIMIT 1",
                params![walk.thread.as_str(),*cursor,walk.high_water], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).optional().map_err(store_error)?;
            if let Some((ordinal, recipient, joined)) = row {
                *cursor = ordinal;
                visited += 1;
                if joined
                    && walk
                        .exclude
                        .is_none_or(|sender| recipient != sender.as_str())
                {
                    stage_recipient(
                        tx,
                        walk.prep_id,
                        walk.instance,
                        walk.thread,
                        &recipient,
                        walk.duration,
                        count,
                        warning_count,
                    )?;
                }
                continue;
            }
            *cursor = walk.high_water;
        }
        let explicit_index = usize::try_from(*cursor - walk.high_water)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid preparation cursor"))?;
        if explicit_index >= walk.explicit.len() {
            break;
        }
        let recipient = walk.explicit[explicit_index];
        let target: Option<(String, bool)> = tx
            .query_row(
                "SELECT instance_id,state='retired' FROM seats WHERE id=?1",
                [recipient.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        let rejection = match target {
            Some((target_instance, false)) if target_instance == walk.instance => {
                if schema::effective_membership_state(tx, walk.thread, recipient)?
                    .as_ref()
                    .is_none_or(|v| !matches!(v.state.as_str(), "invited" | "joined"))
                {
                    Some(RecipientRejection::NotMemberOrInvitee)
                } else {
                    None
                }
            }
            Some((target_instance, true)) if target_instance == walk.instance => {
                Some(RecipientRejection::Retired)
            }
            _ => Some(RecipientRejection::Unknown),
        };
        if let Some(reason) = rejection {
            return Ok(WalkEnd::Rejected {
                seat: recipient.clone(),
                reason,
            });
        }
        *cursor = cursor.checked_add(1).ok_or_else(|| {
            api_error(ErrorCode::SequenceExhausted, "preparation cursor exhausted")
        })?;
        visited += 1;
        if walk.exclude != Some(recipient) {
            stage_recipient(
                tx,
                walk.prep_id,
                walk.instance,
                walk.thread,
                recipient.as_str(),
                walk.duration,
                count,
                warning_count,
            )?;
        }
    }
    let complete = *cursor
        >= walk.high_water
            + i64::try_from(walk.explicit.len())
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "explicit count overflow"))?;
    Ok(if complete {
        WalkEnd::Complete { visited }
    } else {
        WalkEnd::More { visited }
    })
}

pub(super) fn discard_preparation(tx: &Transaction<'_>, prep_id: &str) -> Result<(), ApiError> {
    let changed = tx.execute("UPDATE send_preparations SET status='discarded' WHERE id=?1 AND origin='local' AND status IN ('building','sealed') AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",[prep_id]).map_err(store_error)?;
    if changed != 0 {
        tx.execute("INSERT OR IGNORE INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'preparation_cleanup',?2,0)",params![format!("work:cleanup:{prep_id}"),prep_id]).map_err(store_error)?;
    }
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
    let changed=tx.execute("UPDATE send_preparations SET status='discarded' WHERE id=?1 AND origin='local' AND status IN ('building','sealed') AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",
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
pub(super) fn stage_recipient(
    tx: &Transaction<'_>,
    prep_id: &str,
    instance: &str,
    thread: &ThreadId,
    recipient: &str,
    duration: i64,
    count: &mut i64,
    warning_count: &mut i64,
) -> Result<(), ApiError> {
    let lazy: bool = tx
        .query_row(
            "SELECT delivery_mode='lazy' FROM send_preparations WHERE id=?1",
            [prep_id],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    // A seat another node hosts: this node holds no delivery state for it and
    // has no availability evidence about it (fact-kinds design section 8).
    let remote_kind: Option<String> = tx
        .query_row(
            "SELECT remote_kind FROM seats WHERE id=?1 AND is_local=0",
            [recipient],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(kind) = remote_kind {
        // A remote lazy recipient has no row here: its own node writes it when
        // the `message.sent` payload (which lists it) applies there.
        if lazy {
            return Ok(());
        }
        // A remote human owes no receipt, like a local one. Any other remote
        // seat owes a timed one: the sender cannot know its availability.
        if kind != "human" {
            let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1 AND seat_id=?2)",params![prep_id,recipient],|r|r.get(0)).map_err(store_error)?;
            if !already {
                *count = count.checked_add(1).ok_or_else(|| {
                    api_error(ErrorCode::SequenceExhausted, "recipient ordinal exhausted")
                })?;
                tx.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES (?1,?2,?3,?4,?5,1,'remote')",
                    params![prep_id,thread.as_str(),recipient,*count,duration]).map_err(store_error)?;
            }
        }
        return Ok(());
    }
    if lazy {
        let already: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM lazy_recipients WHERE preparation_id=?1 AND seat_id=?2)",params![prep_id,recipient],|r|r.get(0)).map_err(store_error)?;
        if already {
            return Ok(());
        }
        let message =
            MessageId::new(send_message_id_for_preparation(prep_id).ok_or_else(|| {
                api_error(ErrorCode::StoreCorrupt, "invalid send preparation ID")
            })?);
        return super::lazy_delivery::stage_recipient(
            tx,
            prep_id,
            &message,
            thread,
            &SeatId::new(recipient),
        );
    }
    // The daemon's open binding is the canonical role. A human still sees the
    // thread's message through membership; no ACK expectation is staged.
    // Keep the separate unavailable-recipient warning below.
    let expects_ack = schema::open_binding_role(tx, &SeatId::new(recipient))? != Some("human");
    let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1 AND seat_id=?2)",params![prep_id,recipient],|r|r.get(0)).map_err(store_error)?;
    if already {
        return Ok(());
    }
    // One availability projection with `ensure_unavailability_episode`.
    let provenance = schema::effective_registered_availability(tx, recipient, Some(instance))?;
    let available = provenance.is_some();
    if expects_ack {
        *count = count.checked_add(1).ok_or_else(|| {
            api_error(ErrorCode::SequenceExhausted, "recipient ordinal exhausted")
        })?;
        tx.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![prep_id,thread.as_str(),recipient,*count,duration,available,provenance]).map_err(store_error)?;
    }
    // C4: a seat whose binding the pending reconciliation pass will carry is
    // not warned about; the carry records availability independently of the send-time deadline.
    if !available
        && !schema::carry_pending(tx, recipient, instance)?
        && schema::effective_membership_state(tx, thread, &SeatId::new(recipient))?
            .as_ref()
            .is_some_and(|v| v.state == "joined")
    {
        let pending_invitation:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id))",params![thread.as_str(),recipient],|r|r.get(0)).map_err(store_error)?;
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
                let source_message = send_message_id_for_preparation(prep_id).ok_or_else(|| {
                    api_error(ErrorCode::StoreCorrupt, "invalid send preparation ID")
                })?;
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

/// Who a published ordinary message is attributed to, and when it was decided.
///
/// Built from the `message.sent` payload (`decision_at` is its `sent_at`) with
/// the attribution `attribution::take` returned, so a local and a replicated
/// publication write the same columns.
pub(super) enum PublicationAuthor<'a> {
    /// A native seat, with its serialized observation.
    Native {
        seat: &'a SeatId,
        observation: &'a str,
        /// The acting binding, recorded as `messages.sender_binding_id` (spec
        /// §2); `None` when no local binding made the row (a remote author).
        binding: Option<&'a str>,
        relays_user: bool,
        user_intent: Option<crate::protocol::summary::UserIntent>,
        decision_at: i64,
    },
    /// The registered service author: no seat, label `herdr-graph`.
    Programmatic {
        author: &'a ServiceAuthorId,
        decision_at: i64,
    },
}

impl PublicationAuthor<'_> {
    fn decision_at(&self) -> i64 {
        match self {
            Self::Native { decision_at, .. } | Self::Programmatic { decision_at, .. } => {
                *decision_at
            }
        }
    }
}

/// What one publication wrote.
pub(super) struct Publication {
    pub message: MessageId,
    pub sequence: i64,
    pub recipient_count: i64,
    /// The frozen receipt duration; 0 when the message has no recipients.
    pub duration_ms: i64,
}

/// Inserts the message, its manifest and ordinary-only `send_attention` job for the sealed
/// preparation `prep_id`, under the apply input `m` (its `decision_seq` and origin
/// node) and `author` (its `decision_at` and attribution). The caller proved
/// authority and revision currency; `suppressed` lists (sorted) the local recipients whose
/// delivery catch-up suppression skipped, stored on the manifest (which is immutable once
/// written). This touches a constant number of rows regardless
/// of audience size, except that a lazy send writes one `wake_work` row per
/// `wake_on: all` recipient (set-based; `nudge` recipients cost no write).
pub(super) fn insert_publication(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    prep_id: &str,
    thread: &ThreadId,
    body: &str,
    author: PublicationAuthor<'_>,
) -> Result<Publication, ApiError> {
    insert_publication_suppressing(tx, m, prep_id, thread, body, author, &[])
}

/// [`insert_publication`] for a publication whose local recipients `suppressed`
/// (sorted) catch-up suppression skipped.
pub(super) fn insert_publication_suppressing(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    prep_id: &str,
    thread: &ThreadId,
    body: &str,
    author: PublicationAuthor<'_>,
    suppressed: &[String],
) -> Result<Publication, ApiError> {
    let utc = author.decision_at();
    let author_role = match &author {
        PublicationAuthor::Native {
            seat,
            relays_user,
            user_intent,
            ..
        } => {
            let author_role = schema::open_binding_role(tx, seat)?;
            // A fact is never rejected from this node's view of its author
            // (fact-kinds review rule 4): the intent check is the sending
            // node's, made against its own binding.
            if m.local_origin
                && user_intent.is_some()
                && !(author_role == Some("human") || (author_role == Some("agent") && *relays_user))
            {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "human input intent requires a human sender or an agent relaying user input",
                ));
            }
            author_role
        }
        PublicationAuthor::Programmatic { .. } => None,
    };
    let (instance,high_water,recipient_count,warning_count,mode):(String,i64,i64,i64,String)=tx.query_row(
        "SELECT instance_id,interval_high_water,recipient_count,warning_count,delivery_mode FROM send_preparations WHERE id=?1",
        [prep_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).map_err(store_error)?;
    let mut duration_ms = 0;
    if recipient_count > 0 {
        duration_ms=tx.query_row("SELECT frozen_duration_ms FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=1",[prep_id],|r|r.get(0)).map_err(store_error)?;
        schema::checked_deadline(crate::protocol::time::UtcMillis(utc), duration_ms)?;
    }
    if warning_count > 0 {
        schema::ensure_unavailability_reopen_ready(tx, prep_id, thread.as_str())?;
    }
    let decision_seq = i64::try_from(m.decision_seq)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "decision sequence exhausted"))?;
    let origin_node = m.origin_node.to_string();
    let base: i64 = tx
        .query_row(
            "SELECT next_sequence FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let advance = warning_count
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted"))?;
    let following = base
        .checked_add(advance)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted"))?;
    if following <= 0 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "timeline sequence exhausted",
        ));
    }
    let id = MessageId::new(
        send_message_id_for_preparation(prep_id)
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid send preparation ID"))?,
    );
    match author {
        PublicationAuthor::Native {
            seat,
            observation,
            binding,
            relays_user,
            user_intent,
            ..
        } => {
            tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,native_observation,body,decision_at,decision_seq,author_role,relays_user,user_intent,delivery_mode,sender_binding_id,origin_node) VALUES (?1,?2,?3,?4,'ordinary',?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![id.as_str(),instance,thread.as_str(),base,seat.as_str(),observation,body,utc,decision_seq,author_role,relays_user as i64,user_intent.map(crate::protocol::summary::UserIntent::as_str),mode,binding,origin_node]).map_err(store_error)?;
        }
        PublicationAuthor::Programmatic { author, .. } => {
            tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_label,body,decision_at,decision_seq,author_kind,author_service_id,author_role,origin_node) VALUES (?1,?2,?3,?4,'ordinary','herdr-graph',?5,?6,?7,'programmatic',?8,'service',?9)",params![id.as_str(),instance,thread.as_str(),base,body,utc,decision_seq,author.as_str(),origin_node]).map_err(store_error)?;
        }
    }
    tx.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count,suppressed_seats) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![prep_id,id.as_str(),instance,thread.as_str(),decision_seq,utc,base,high_water,recipient_count,warning_count,(!suppressed.is_empty()).then(|| suppressed.join("\n"))]).map_err(store_error)?;
    if warning_count > 0 {
        schema::record_published_unavailable_conditions(
            tx,
            prep_id,
            thread.as_str(),
            decision_seq,
            high_water,
            crate::protocol::time::UtcMillis(utc),
        )?;
    }
    tx.execute(
        "UPDATE threads SET next_sequence=?1,updated_at=?2 WHERE id=?3",
        params![following, utc, thread.as_str()],
    )
    .map_err(store_error)?;
    schema::bump_timeline_revision(tx, thread)?;
    schema::bump_filter_revision(tx, &instance, "directory", thread.as_str())?;
    super::delivery_tail::DeliveryTail::message(
        tx,
        Some(m),
        &id,
        prep_id,
        &mode,
        recipient_count,
        warning_count,
    )?;
    Ok(Publication {
        message: id,
        sequence: base,
        recipient_count,
        duration_ms,
    })
}

/// The mismatch every audience comparison reports.
fn audience_mismatch() -> ApiError {
    api_error(
        ErrorCode::StoreCorrupt,
        "staged audience differs from authored audience",
    )
}

/// The thread of a preparation a local publication may adopt: `sealed`, born
/// local, with no manifest yet. `None` for any other state.
fn adoptable_preparation(
    tx: &Transaction<'_>,
    prep_id: &str,
) -> Result<Option<(String, String)>, ApiError> {
    tx.query_row(
        "SELECT thread_id,delivery_mode FROM send_preparations WHERE id=?1 AND status='sealed' AND origin='local' AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",
        [prep_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(store_error)
}

/// Publishes a service author's sealed preparation through the local-only
/// route: the local-adopt branch of `materialize_message` for a message no
/// journal carries (managed threads are never journaled, fact-kinds design
/// section 2). `m` is a [`materialize::local_only`](super::materialize::local_only)
/// input.
pub(super) fn adopt_service_preparation(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    prep_id: &str,
    thread: &ThreadId,
    body: &str,
    author: &ServiceAuthorId,
    sent_at: i64,
) -> Result<Publication, ApiError> {
    if adoptable_preparation(tx, prep_id)?
        .is_none_or(|(prep_thread, _)| prep_thread != thread.as_str())
    {
        return Err(api_error(ErrorCode::Conflict, "send preparation missing"));
    }
    insert_publication(
        tx,
        m,
        prep_id,
        thread,
        body,
        PublicationAuthor::Programmatic {
            author,
            decision_at: sent_at,
        },
    )
}

/// Builds the `message.sent` payload of a sealed preparation (fact-kinds
/// design section 8, author steps 1-2): the complete audience the author
/// fixes, sorted by seat ID with the author excluded.
///
/// - `ack` for each prepared recipient, timed (`sent_at + frozen duration`)
///   when it was eligible at the snapshot;
/// - `none` for every other joined member at the preparation's interval high
///   water and for each explicit recipient, which owe nothing (humans);
/// - for a lazy message, `none` for each staged lazy recipient.
pub(super) fn build_message_sent(
    tx: &Transaction<'_>,
    prep_id: &str,
    request: &SendMessage,
    sent_at: i64,
) -> Result<MessageSentPayload, ApiError> {
    let message = MessageId::new(
        send_message_id_for_preparation(prep_id)
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid send preparation ID"))?,
    );
    let (mode, high_water): (String, i64) = tx
        .query_row(
            "SELECT delivery_mode,interval_high_water FROM send_preparations WHERE id=?1",
            [prep_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let author = &request.claim.seat;
    let mut audience: std::collections::BTreeMap<String, RecipientSpec> = Default::default();
    let none = |seat: &str| RecipientSpec {
        seat: SeatId::new(seat),
        receipt: ReceiptObligation::None,
        deadline_at: None,
    };
    let mut duration_ms = 0;
    if mode == "lazy" {
        let mut stmt = tx
            .prepare("SELECT seat_id FROM lazy_recipients WHERE preparation_id=?1")
            .map_err(store_error)?;
        for seat in stmt
            .query_map([prep_id], |r| r.get::<_, String>(0))
            .map_err(store_error)?
        {
            let seat = seat.map_err(store_error)?;
            audience.insert(seat.clone(), none(&seat));
        }
        // Remote members and explicit remote recipients hold no lazy row on
        // this node but are part of the audience their own node adopts.
        let mut stmt = tx
            .prepare("SELECT mi.seat_id FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal<=?2 AND mi.left_seq IS NULL AND s.retired_seq IS NULL AND s.state!='retired' AND s.is_local=0")
            .map_err(store_error)?;
        for seat in stmt
            .query_map(params![request.thread.as_str(), high_water], |r| {
                r.get::<_, String>(0)
            })
            .map_err(store_error)?
        {
            let seat = seat.map_err(store_error)?;
            audience.insert(seat.clone(), none(&seat));
        }
        for seat in request
            .invited_recipients
            .iter()
            .chain(request.direct_recipient.iter())
        {
            if is_remote_seat(tx, seat.as_str())? {
                audience.insert(seat.as_str().to_owned(), none(seat.as_str()));
            }
        }
    } else {
        let mut stmt = tx
            .prepare("SELECT mi.seat_id FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal<=?2 AND mi.left_seq IS NULL AND s.retired_seq IS NULL AND s.state!='retired'")
            .map_err(store_error)?;
        for seat in stmt
            .query_map(params![request.thread.as_str(), high_water], |r| {
                r.get::<_, String>(0)
            })
            .map_err(store_error)?
        {
            let seat = seat.map_err(store_error)?;
            audience.insert(seat.clone(), none(&seat));
        }
        for seat in request
            .invited_recipients
            .iter()
            .chain(request.direct_recipient.iter())
        {
            audience.insert(seat.as_str().to_owned(), none(seat.as_str()));
        }
        let mut stmt = tx
            .prepare("SELECT seat_id,frozen_duration_ms,eligible_at_snapshot,receipt_ordinal FROM prepared_recipients WHERE preparation_id=?1")
            .map_err(store_error)?;
        for row in stmt
            .query_map([prep_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(store_error)?
        {
            let (seat, frozen, eligible, ordinal) = row.map_err(store_error)?;
            let deadline_at = if eligible {
                Some(schema::checked_deadline(crate::protocol::time::UtcMillis(sent_at), frozen)?.0)
            } else {
                None
            };
            if ordinal == 1 {
                duration_ms = frozen;
            }
            audience.insert(
                seat.clone(),
                RecipientSpec {
                    seat: SeatId::new(seat),
                    receipt: ReceiptObligation::Ack,
                    deadline_at,
                },
            );
        }
    }
    audience.remove(author.as_str());
    Ok(MessageSentPayload {
        message,
        thread: request.thread.clone(),
        author: author.clone(),
        body: request.body.clone(),
        mode: request.delivery_mode,
        nudge: false,
        relays_user: request.relays_user,
        user_intent: request.user_intent,
        sent_at,
        duration_ms,
        recipients: audience.into_values().collect(),
    })
}

/// Whether `seat` is hosted by another node.
fn is_remote_seat(tx: &Transaction<'_>, seat: &str) -> Result<bool, ApiError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND is_local=0)",
        [seat],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Whether the audience staged for `prep_id` is the one `payload` fixed: the
/// same thread and mode, the same ack seats with the same timing, and for a
/// lazy message the same lazy recipients.
fn check_staged_audience(
    tx: &Transaction<'_>,
    prep_id: &str,
    prep_thread: &str,
    prep_mode: &str,
    payload: &MessageSentPayload,
) -> Result<(), ApiError> {
    let mode = if payload.mode.is_ordinary() {
        "ordinary"
    } else {
        "lazy"
    };
    if prep_thread != payload.thread.as_str()
        || prep_mode != mode
        || payload
            .recipients
            .windows(2)
            .any(|w| w[0].seat >= w[1].seat)
        || payload.recipients.iter().any(|r| r.seat == payload.author)
    {
        return Err(audience_mismatch());
    }
    let staged_ack: Vec<(String, i64, bool)> = {
        let mut stmt = tx
            .prepare("SELECT seat_id,frozen_duration_ms,eligible_at_snapshot FROM prepared_recipients WHERE preparation_id=?1 ORDER BY seat_id")
            .map_err(store_error)?;
        stmt.query_map([prep_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
    };
    let authored_ack: Vec<&RecipientSpec> = payload
        .recipients
        .iter()
        .filter(|r| r.receipt == ReceiptObligation::Ack)
        .collect();
    if staged_ack.len() != authored_ack.len() {
        return Err(audience_mismatch());
    }
    for ((seat, frozen, eligible), authored) in staged_ack.iter().zip(&authored_ack) {
        let expected = eligible
            .then(|| payload.sent_at.checked_add(*frozen))
            .flatten();
        if authored.seat.as_str() != seat
            || authored.deadline_at != expected
            || (*eligible && expected.is_none())
        {
            return Err(audience_mismatch());
        }
    }
    let staged_duration: i64 = tx
        .query_row(
            "SELECT COALESCE((SELECT frozen_duration_ms FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=1),0)",
            [prep_id],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if staged_duration != payload.duration_ms {
        return Err(audience_mismatch());
    }
    if mode == "lazy" {
        let staged: Vec<String> = {
            let mut stmt = tx
                .prepare(
                    "SELECT seat_id FROM lazy_recipients WHERE preparation_id=?1 ORDER BY seat_id",
                )
                .map_err(store_error)?;
            stmt.query_map([prep_id], |r| r.get(0))
                .map_err(store_error)?
                .collect::<Result<_, _>>()
                .map_err(store_error)?
        };
        // Remote recipients are authored but staged nowhere on this node.
        let mut authored: Vec<&str> = Vec::new();
        for recipient in &payload.recipients {
            if !is_remote_seat(tx, recipient.seat.as_str())? {
                authored.push(recipient.seat.as_str());
            }
        }
        if staged
            .iter()
            .map(String::as_str)
            .ne(authored.iter().copied())
        {
            return Err(audience_mismatch());
        }
    }
    Ok(())
}

/// Writes the preparation a receiver synthesizes from `payload` (fact-kinds
/// design section 8, step 4) and returns the local recipients whose delivery
/// catch-up suppression skipped (sorted).
///
/// A `replicated` preparation is sealed here (it is `building` only while its
/// rows are inserted) and its manifest is written by the publication that follows. It holds a `prepared_recipients` row per
/// `ack` recipient, local or remote, in payload order, and for a lazy message
/// a `lazy_recipients` row per local recipient that is not suppressed. It
/// never holds `prepared_unavailable_warnings`: those are the sender's
/// derived events.
fn synthesize_preparation(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    prep_id: &str,
    payload: &MessageSentPayload,
) -> Result<Vec<String>, ApiError> {
    let digest = Sha256::digest(
        serde_json::to_vec(payload)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "message.sent payload encode"))?,
    );
    let (high_water, membership, lifecycle, eligibility, timeline, config): (i64, i64, i64, i64, i64, i64) = tx
        .query_row(
            "SELECT COALESCE((SELECT max(ordinal) FROM membership_intervals WHERE thread_id=t.id),0),t.membership_revision,(SELECT COALESCE(SUM(lifecycle_revision),0) FROM herdr_hosts WHERE node_id=n.id),n.send_eligibility_revision,t.timeline_revision,n.duration_config_revision FROM threads t JOIN local_node n ON n.id=t.instance_id WHERE t.id=?1",
            [payload.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .map_err(store_error)?;
    let ack_count = payload
        .recipients
        .iter()
        .filter(|r| r.receipt == ReceiptObligation::Ack)
        .count();
    let ack_count = i64::try_from(ack_count)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "recipient ordinal exhausted"))?;
    let mode = if payload.mode.is_ordinary() {
        "ordinary"
    } else {
        "lazy"
    };
    tx.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_cursor,recipient_count,warning_count,status,prepared_at,delivery_mode,origin) \
         VALUES (?1,(SELECT id FROM local_node),?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?12,?12,0,'building',?13,?14,'replicated')",
        params![
            prep_id,
            format!("replicated:{}", m.origin_node),
            payload.message.as_str(),
            digest.as_slice(),
            payload.thread.as_str(),
            membership,
            lifecycle,
            eligibility,
            timeline,
            config,
            high_water,
            ack_count,
            payload.sent_at,
            mode
        ],
    )
    .map_err(store_error)?;
    let mut suppressed = Vec::new();
    let mut ordinal = 0i64;
    for recipient in &payload.recipients {
        let seat = recipient.seat.as_str();
        let is_local: Option<bool> = tx
            .query_row("SELECT is_local FROM seats WHERE id=?1", [seat], |r| {
                r.get(0)
            })
            .optional()
            .map_err(store_error)?;
        let Some(is_local) = is_local else {
            return Err(api_error(
                ErrorCode::Conflict,
                "message.sent names an unknown seat",
            ));
        };
        if recipient.receipt == ReceiptObligation::Ack {
            ordinal += 1;
            let (frozen, eligible) = match recipient.deadline_at {
                Some(deadline) => (deadline.saturating_sub(payload.sent_at).max(1), 1),
                None => (payload.duration_ms.max(1), 0),
            };
            tx.execute(
                "INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES (?1,?2,?3,?4,?5,?6,'replicated')",
                params![prep_id, payload.thread.as_str(), seat, ordinal, frozen, eligible],
            )
            .map_err(store_error)?;
        }
        if !is_local {
            continue;
        }
        let key = receipt_settle_key(&payload.message, &recipient.seat)
            .map_err(|e| api_error(ErrorCode::InvalidRequest, e.to_string()))?;
        if (m.suppress)(tx, &key)? {
            suppressed.push(seat.to_owned());
        } else if mode == "lazy" {
            super::lazy_delivery::stage_recipient(
                tx,
                prep_id,
                &payload.message,
                &payload.thread,
                &recipient.seat,
            )?;
        }
    }
    // Lazy rows may only be staged while `building`; seal once they exist.
    tx.execute(
        "UPDATE send_preparations SET status='sealed' WHERE id=?1",
        [prep_id],
    )
    .map_err(store_error)?;
    suppressed.sort();
    Ok(suppressed)
}

/// The one materializer of a published ordinary message (`message.sent`;
/// fact-kinds design section 8), shared by the journal handler and the
/// local-only route.
///
/// 1. A no-op when the message exists (a re-authored copy writes nothing).
/// 2. The preparation is the message ID's inverse (`m<tag><8>` to `p<tag><8>`).
/// 3. Local branch: for a local-origin fact whose preparation is `sealed`
///    with no manifest, the staged rows are adopted after checking they equal
///    the authored audience (`StoreCorrupt` "staged audience differs from
///    authored audience" otherwise, which aborts the command).
/// 4. Synthesized branch (a remote origin, or an own old incarnation without
///    a preparation): the preparation is built from the payload
///    ([`synthesize_preparation`]); local recipients whose settlement already
///    exists are listed in `send_manifests.suppressed_seats`.
/// 5. The message is published with the attribution `take` returns.
///
/// Receipts of a recipient already retired at apply are projected by the
/// `send_attention` job (`recipient_retired`, step 6).
pub fn materialize_message(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &MessageSentPayload,
) -> Result<(), ApiError> {
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1)",
            [payload.message.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        return Ok(());
    }
    let prep_id = send_preparation_id_for_message(payload.message.as_str()).ok_or_else(|| {
        api_error(
            ErrorCode::InvalidRequest,
            "message id is not a send message id",
        )
    })?;
    let adopted = if m.local_origin {
        adoptable_preparation(tx, &prep_id)?
    } else {
        None
    };
    let mut suppressed = Vec::new();
    if let Some((prep_thread, prep_mode)) = adopted {
        check_staged_audience(tx, &prep_id, &prep_thread, &prep_mode, payload)?;
    } else {
        let taken: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM send_preparations WHERE id=?1)",
                [&prep_id],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if taken {
            return Err(api_error(
                ErrorCode::Conflict,
                "send preparation exists but is not adoptable",
            ));
        }
        suppressed = synthesize_preparation(tx, m, &prep_id, payload)?;
    }
    let attribution = attribution::take(tx, m, &attribution::send_key(payload.message.as_str()))?;
    insert_publication_suppressing(
        tx,
        m,
        &prep_id,
        &payload.thread,
        &payload.body,
        PublicationAuthor::Native {
            seat: &payload.author,
            observation: &attribution.observation,
            binding: attribution.binding_id.as_deref(),
            relays_user: payload.relays_user,
            user_intent: payload.user_intent,
            decision_at: payload.sent_at,
        },
        &suppressed,
    )?;
    Ok(())
}

/// The route a send takes: the journal for a journaled thread whose creating
/// fact exists, a node-local write for any other.
///
/// Transitional (ht-5fr.4.10): a pair thread `ensure_direct_thread` still
/// inserts directly has no creating fact, so `message.sent` for it would park
/// on its `needs`; until that command authors `thread.direct` such a thread,
/// and a store whose journal never opened (a fixture; the daemon opens it
/// before serving), keep the node-local write.
fn send_route(
    tx: &Transaction<'_>,
    context: &StoreContext,
    thread: &ThreadId,
) -> Result<FactRoute, ApiError> {
    let route = FactRoute::for_thread(tx, thread)?;
    if route == FactRoute::Journal
        && (!context.journal_open() || !super::control::has_creating_fact(tx, thread)?)
    {
        return Ok(FactRoute::LocalOnly);
    }
    Ok(route)
}

/// Publishes only a sealed, revision-current preparation after a fresh caller proof.
/// `current_body_bytes` reads writer-owned effective configuration after the writer
/// wait; configuration updates must use that same lane and remain stable through
/// this transaction. The callback must do bounded memory work without host/file I/O.
/// On a journaled thread the writer authors one `message.sent` carrying the complete
/// audience (fact-kinds design section 8), so it reads one row per recipient and the
/// payload is capped at 448 KiB; the publication itself still writes a constant number
/// of rows regardless of recipient count.
pub fn publish_send(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    permit: &mut MutationPermit,
    budget: &CallBudget,
    current_body_bytes: impl FnOnce() -> usize,
) -> Result<CommandResult, ApiError> {
    validate_delivery_options(request)?;
    require_live_budget(context, budget)?;
    let digest = schema::canonical_digest(&send_payload(request))?;
    super::seats::cooperative_claim(conn, context, &request.claim.instance, &request.claim)?;
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
            super::control::validate_pair_send(tx, request)?;
            if let Some((a, b)) = super::control::direct_pair(tx, &request.thread)? {
                let extra: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients p JOIN send_preparations s ON s.id=p.preparation_id WHERE s.operation_scope=?1 AND s.operation_key=?2 AND p.seat_id NOT IN (?3,?4)) OR EXISTS(SELECT 1 FROM lazy_recipients p JOIN send_preparations s ON s.id=p.preparation_id WHERE s.operation_scope=?1 AND s.operation_key=?2 AND p.seat_id NOT IN (?3,?4))",params![scope,request.operation.as_str(),a.as_str(),b.as_str()],|r|r.get(0)).map_err(store_error)?;
                if extra {
                    return Err(api_error(
                        ErrorCode::StoreCorrupt,
                        "prepared audience outside automatic pair",
                    ));
                }
            }
            type CurrentColumns =
                Option<(String, String, i64, i64, i64, i64, i64, i64, i64, String)>;
            let current:CurrentColumns = tx.query_row(
                "SELECT p.id,p.instance_id,p.captured_membership_revision,p.captured_lifecycle_revision,p.captured_eligibility_revision,p.captured_timeline_revision,p.captured_config_revision,p.recipient_count,p.warning_count,p.status FROM send_preparations p WHERE p.operation_scope=?1 AND p.operation_key=?2 AND p.digest=?3 AND p.thread_id=?4",
                params![scope,request.operation.as_str(),digest.as_slice(),request.thread.as_str()],
                |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?)),
            ).optional().map_err(store_error)?;
            let Some((_, instance, mr, lr, er, tr, cr, _, warnings, status)) = current else {
                return Err(api_error(ErrorCode::Conflict, "send preparation missing"));
            };
            if status != "sealed" {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation incomplete",
                ));
            }
            let live:(i64,i64,i64,i64,i64,bool)=tx.query_row("SELECT t.membership_revision,(SELECT COALESCE(SUM(lifecycle_revision),0) FROM herdr_hosts WHERE node_id=n.id),n.send_eligibility_revision,t.timeline_revision,n.duration_config_revision,t.archived FROM threads t JOIN local_node n ON n.id=t.instance_id WHERE t.id=?1 AND n.id=?2",params![request.thread.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(store_error)?;
            if live.5 {
                return Err(api_error(ErrorCode::Archived, "thread archived"));
            }
            if (mr, lr, er, cr) != (live.0, live.1, live.2, live.4)
                || timeline_fence_stale(warnings, tr, live.3)
            {
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
            // The send-eligibility fence as the deciding writer found it, before the author's
            // own claim resolves below. That resolution (a plain call: enroll, reconnect, touch;
            // the supersede and placeholder hooks are lifecycle-only, the human end is the same
            // seat) changes only the author's seat bindings, and the author is never a staged
            // recipient, so a bump it causes cannot stale the staged recipient set.
            let eligibility_before_author: i64 = tx
                .query_row(
                    "SELECT h.send_eligibility_revision FROM threads t JOIN local_node h ON h.id=t.instance_id WHERE t.id=?1",
                    [request.thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let actor = super::control::decide_accountable(
                context,
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
                "SELECT p.captured_membership_revision=t.membership_revision AND p.captured_lifecycle_revision=(SELECT COALESCE(SUM(lifecycle_revision),0) FROM herdr_hosts WHERE node_id=n.id) AND p.captured_eligibility_revision=?4 AND (p.warning_count=0 OR p.captured_timeline_revision=t.timeline_revision) AND p.captured_config_revision=n.duration_config_revision AND t.archived=0 FROM send_preparations p JOIN threads t ON t.id=p.thread_id JOIN local_node n ON n.id=t.instance_id WHERE p.operation_scope=?1 AND p.operation_key=?2 AND p.digest=?3",
                params![scope,request.operation.as_str(),digest.as_slice(),eligibility_before_author],|r|r.get(0),
            ).map_err(store_error)?;
            if !revisions_match {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "send preparation snapshot changed at decision",
                ));
            }
            let observation = actor.observation(decision.utc.0);
            let prep_id: String = tx
                .query_row(
                    "SELECT id FROM send_preparations WHERE operation_scope=?1 AND operation_key=?2 AND digest=?3",
                    params![scope, request.operation.as_str(), digest.as_slice()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let payload = build_message_sent(tx, &prep_id, request, decision.utc.0)?;
            let route = send_route(tx, context, &request.thread)?;
            if route == FactRoute::Journal {
                payload.validate_for_author()?;
            }
            attribution::stage(
                tx,
                &attribution::send_key(payload.message.as_str()),
                &actor,
                &observation,
            )?;
            route.emit::<MessageSent>(
                tx,
                context,
                decision,
                &payload,
                None,
                materialize_message,
            )?;
            let id = payload.message;
            failpoint!("send.before_commit", context.failpoint_scope());
            Ok(CommandResult::MessageSent(id))
        },
    )?;
    failpoint!("send.after_commit", context.failpoint_scope());
    Ok(result)
}

/// Timeline fence, shared by the preparation restart check and both publish comparisons.
///
/// Protects: the reserved warning sequence span and the unavailable-warning conditions
/// `ensure_unavailability_reopen_ready` re-checks, both of which a timeline append can
/// invalidate. It therefore applies only to a preparation with `warning_count > 0`.
/// Does not protect: the ordinary message sequence, which is read from `threads.next_sequence`
/// at publish, so unrelated timeline traffic (other local senders, remote messages) must not
/// invalidate a preparation without warnings. The audience stays fenced by the membership,
/// lifecycle, eligibility and config revisions, which are compared unconditionally.
/// `service_send` keeps the full fence (managed threads see no remote traffic).
fn timeline_fence_stale(warning_count: i64, captured: i64, live: i64) -> bool {
    warning_count > 0 && captured != live
}

#[cfg(test)]
#[path = "../../tests/store/messages.rs"]
mod messages_tests;

#[cfg(test)]
#[path = "../../tests/store/author_role.rs"]
mod author_role_tests;
