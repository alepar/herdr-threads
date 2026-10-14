//! Receipt settlement and availability transitions.

use crate::{
    delivery::settle::{self, LazyFact, ModeFact, ReceiptFact, SettleFacts, SettlePlan},
    harness::adapter::DeliveryMechanism,
    ports::TimeBasis,
    protocol::{
        authority::{
            COOPERATIVE_HOOK_DELIVERY_PROVENANCE, COOPERATIVE_MOD_DELIVERY_PROVENANCE, CallerClaim,
            Harness, MutationPermit, ObligationRef,
        },
        commands::{Ack, AckModDelivered},
        delivery::{
            DeliveryConfirm, DeliveryConfirmItem, DeliveryConfirmOutcome, DeliveryConfirmReason,
            DeliveryConfirmReport,
        },
        ids::{BindingId, MessageId, OperationId, SeatId, ThreadId},
        results::{AckResult, ApiError, CommandResult, ErrorCode},
        time::UtcMillis,
        watch::{ModAckItem, ModAckOutcome, ModAckReason, ModAckReport},
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    connection::{DecisionInstant, StoreContext, api_error, store_error},
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

pub fn ack_mod_delivered_payload(request: &AckModDelivered) -> Value {
    json!({"kind":"ack_mod_delivered","via":request.via,"messages":request.messages,"claim":request.claim})
}

/// Reads the stored facts of one requested id (spec D6) and lets the pure
/// [`settle::plan`] decide. Never errors per id.
pub(super) fn plan_delivery_ack(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    id: &MessageId,
) -> Result<SettlePlan, ApiError> {
    let row: Option<(String, i64)> = tx
        .query_row(
            "SELECT kind,length(CAST(coalesce(body,'') AS BLOB)) FROM messages WHERE id=?1 AND instance_id=?2",
            params![id.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let message = row.map(|(kind, body_len)| (kind == "ordinary", body_len as usize));
    // A non-ordinary id is refused before its mode or receipt is read.
    if !message.is_some_and(|(ordinary, _)| ordinary) {
        return Ok(settle::plan(SettleFacts {
            message,
            mode: ModeFact::Receipt(None),
        }));
    }
    // A seat with a delivery row settles it as a row; any other seat by its receipt (D9).
    let mode = match super::lazy_delivery::lazy_completion_state(tx, instance, seat, binding, id)? {
        super::lazy_delivery::LazyCompletion::NotAddressed => ModeFact::Receipt(
            effective::effective_receipt(tx, id.as_str(), seat.as_str())?.map(|receipt| {
                match receipt.state {
                    EffectiveReceiptState::Pending => ReceiptFact::Pending,
                    EffectiveReceiptState::Acknowledged => ReceiptFact::Acknowledged,
                    EffectiveReceiptState::RecipientRetired => ReceiptFact::RecipientRetired,
                    EffectiveReceiptState::NotRequired => ReceiptFact::NotRequired,
                }
            }),
        ),
        super::lazy_delivery::LazyCompletion::Completed => ModeFact::Lazy(LazyFact::Completed),
        super::lazy_delivery::LazyCompletion::Pending => ModeFact::Lazy(LazyFact::Pending),
    };
    Ok(settle::plan(SettleFacts { message, mode }))
}

/// One delivery settlement request, whatever mechanism carried the items
/// (delivery-policy DP3). `provenance` is the A3 constant of the harness's
/// delivery policy; `via` names the delivery path.
pub struct AckPolicyDelivered<'a> {
    pub mechanism: DeliveryMechanism,
    pub provenance: &'static str,
    pub via: &'static str,
    pub messages: &'a [MessageId],
    pub operation: &'a OperationId,
    pub claim: &'a CallerClaim,
    pub digest: [u8; 32],
}

/// Spec D6: settle the mod-delivered ids of the seat one by one. Ordinary
/// pending receipts settle like `ack_displayed` with the receipt's
/// `action_provenance` set to the policy's provenance (the binding's
/// `cooperative_top_level` provenance stays in the observation); lazy rows
/// complete as displayed. Every id gets a result; a refusal leaves it pending.
pub fn ack_mod_delivered(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &AckModDelivered,
    permit: &mut MutationPermit,
) -> Result<CommandResult, ApiError> {
    let digest = schema::canonical_digest(&ack_mod_delivered_payload(request))?;
    ack_policy_delivered(
        context,
        conn,
        budget,
        AckPolicyDelivered {
            mechanism: DeliveryMechanism::PushChannel,
            provenance: crate::harness::registry::builtins()
                .delivery_policy(request.claim.harness.as_str())
                .filter(|policy| policy.mechanism() == DeliveryMechanism::PushChannel)
                .map_or(COOPERATIVE_MOD_DELIVERY_PROVENANCE, |policy| {
                    policy.provenance()
                }),
            via: request.via.as_str(),
            messages: &request.messages,
            operation: &request.operation,
            claim: &request.claim,
            digest,
        },
        permit,
    )
}

pub fn delivery_confirm_payload(request: &DeliveryConfirm) -> Value {
    json!({"kind":"delivery_confirm","messages":request.ids,"claim":request.claim})
}

/// Delivery-policy DP6: settle the ids a hook-pull harness found in its
/// session transcript, one by one, under `cooperative_hook_delivery`. The
/// service has already checked the ids against its offered ledger; the store
/// decides each id against A2 again exactly as `ack_mod_delivered` does.
pub fn delivery_confirm(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: &DeliveryConfirm,
    permit: &mut MutationPermit,
) -> Result<CommandResult, ApiError> {
    let digest = schema::canonical_digest(&delivery_confirm_payload(request))?;
    let result = ack_policy_delivered(
        context,
        conn,
        budget,
        AckPolicyDelivered {
            mechanism: DeliveryMechanism::HookPull,
            provenance: COOPERATIVE_HOOK_DELIVERY_PROVENANCE,
            via: "transcript",
            messages: &request.ids,
            operation: &request.operation,
            claim: &request.claim,
            digest,
        },
        permit,
    )?;
    let CommandResult::ModDeliveryAcked(report) = result else {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "delivery settlement returned another result",
        ));
    };
    Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
        results: report
            .results
            .into_iter()
            .map(|item| DeliveryConfirmItem {
                id: item.id,
                result: match item.result {
                    ModAckOutcome::Settled => DeliveryConfirmOutcome::Settled,
                    ModAckOutcome::AlreadySettled => DeliveryConfirmOutcome::AlreadySettled,
                    ModAckOutcome::RefusedTerminal => DeliveryConfirmOutcome::RefusedTerminal,
                    ModAckOutcome::StaleGeneration => DeliveryConfirmOutcome::StaleGeneration,
                    ModAckOutcome::Retryable => DeliveryConfirmOutcome::Retryable,
                },
                reason: item.reason.map(|reason| match reason {
                    ModAckReason::Unknown => DeliveryConfirmReason::Unknown,
                    ModAckReason::NotAddressed => DeliveryConfirmReason::NotAddressed,
                    ModAckReason::Truncated => DeliveryConfirmReason::Truncated,
                    ModAckReason::NoLiveChannel => DeliveryConfirmReason::NoHookPull,
                    ModAckReason::Busy | ModAckReason::Unreachable => DeliveryConfirmReason::Busy,
                }),
            })
            .collect(),
    }))
}

/// The store's settlement entry point (delivery-policy DP3): the claim must be
/// a top-level claim of a harness whose adapter declares `mechanism`.
pub fn ack_policy_delivered(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    request: AckPolicyDelivered<'_>,
    permit: &mut MutationPermit,
) -> Result<CommandResult, ApiError> {
    if request.messages.is_empty() || request.messages.len() > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid mod delivery ACK batch size",
        ));
    }
    let distinct: BTreeSet<_> = request.messages.iter().collect();
    if distinct.len() != request.messages.len() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "duplicate message in mod delivery ACK batch",
        ));
    }
    if request.claim.role != crate::protocol::authority::CallerRole::TopLevel
        || !crate::delivery::declares(request.claim.harness.as_str(), request.mechanism)
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "delivery ACK requires a top-level claim of a harness with this delivery policy",
        ));
    }
    let digest = request.digest;
    let cooperative = permit.cooperative_metadata();
    let seat = request.claim.seat.clone();
    let instance = request.claim.instance.clone();
    let scope = format!("seat:{}", seat.as_str());
    schema::execute_accountable_transaction(
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
            Ok(())
        },
        |tx, decision| {
            let actor = super::control::decide_accountable(
                tx,
                decision,
                permit,
                request.claim,
                &seat,
                request.operation,
                &ObligationRef::CheckIn(seat.clone()),
                &digest,
            )?;
            let binding = super::bindings::match_claim(
                tx,
                &super::bindings::BindingKey::from_claim(request.claim),
            )?
            .map(|b| b.binding_id);
            let mut plans = Vec::with_capacity(request.messages.len());
            for id in request.messages {
                plans.push(plan_delivery_ack(
                    tx,
                    &instance,
                    &seat,
                    binding.as_ref(),
                    id,
                )?);
            }
            let settle_ids: Vec<MessageId> = request
                .messages
                .iter()
                .zip(&plans)
                .filter(|(_, plan)| matches!(plan, SettlePlan::Settle))
                .map(|(id, _)| id.clone())
                .collect();
            let mut observation: Value =
                serde_json::from_str(&actor.observation(decision.utc.0))
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "ACK observation malformed"))?;
            observation["action_provenance"] = json!(request.provenance);
            observation["via"] = json!(request.via);
            settle_pending(
                tx,
                decision,
                &seat,
                request.operation,
                &settle_ids,
                &observation.to_string(),
                actor.binding_generation,
                &actor.binding_id,
                "ack_mod",
            )?;
            // One display, one cursor walk per thread: lazy rows this display completes, plus
            // the ordinary messages it showed whole (settled now, or already settled elsewhere).
            // A refused id was never displayed and never advances the cursor.
            let mut complete_ids = Vec::new();
            let mut shown_ids = settle_ids;
            let mut results = Vec::with_capacity(plans.len());
            for (id, plan) in request.messages.iter().zip(plans) {
                let (result, reason) = match plan {
                    SettlePlan::Settle => (ModAckOutcome::Settled, None),
                    SettlePlan::Complete => {
                        complete_ids.push(id.clone());
                        (ModAckOutcome::Settled, None)
                    }
                    SettlePlan::Done(result, reason) => {
                        if result == ModAckOutcome::AlreadySettled {
                            shown_ids.push(id.clone());
                        }
                        (result, reason)
                    }
                };
                results.push(ModAckItem {
                    id: id.clone(),
                    result,
                    reason,
                });
            }
            super::lazy_delivery::complete_displayed_for_binding(
                tx,
                &instance,
                &seat,
                binding.as_ref(),
                &complete_ids,
                &shown_ids,
            )?;
            failpoint!("ack.before_commit", context.failpoint_scope());
            Ok(CommandResult::ModDeliveryAcked(ModAckReport { results }))
        },
    )
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
    if displayed && !matches!(request.claim.harness, Harness::Agent(_)) {
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
            if displayed {
                let harness: Option<String> = tx.query_row("SELECT harness FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",[seat.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
                if harness.as_deref().is_some_and(|id| {
                    id != "human" && crate::harness::registry::builtins().agent(id).is_err()
                }) {
                    return Err(api_error(
                        ErrorCode::Unsupported,
                        "stored binding harness has no registered adapter",
                    ));
                }
                let exact: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation WHERE s.id=?1 AND b.generation=?2 AND b.harness=?3 AND b.native_session=?4 AND b.execution_id=?5 AND b.target_id=?6 AND b.ended_at IS NULL AND b.role='top_level' AND b.agent_id='')",
                    params![seat.as_str(),i64::try_from(request.claim.binding_generation).map_err(|_| api_error(ErrorCode::CallerUnverified,"binding generation exceeds storage range"))?,request.claim.harness.as_str(),request.claim.native_session.as_str(),request.claim.execution.as_str(),request.claim.target.as_str()],|r|r.get(0)
                ).map_err(store_error)?;
                if !exact {
                    return Err(api_error(
                        ErrorCode::CallerUnverified,
                        "display ACK requires a connected top-level agent binding of the seat for this exact session",
                    ));
                }
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
            // Any connected top-level binding of the seat may settle (spec §2 ACK). A
            // fingerprinted call has already reconnected its binding in the resolver when it was
            // disconnected by a process exit or migration; a disconnected binding that this call
            // could not reconnect (no fingerprint, or superseded, ended or retired) is refused
            // like a stale-context ACK.
            if !actor.binding_connected {
                return Err(api_error(
                    ErrorCode::CallerUnverified,
                    "a disconnected binding reconnects only on a fingerprinted call after a process exit or migration, not after a supersede or end event",
                ));
            }
            let mut newly = Vec::new();
            let mut prior = Vec::new();
            for id in &request.messages {
                let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared")
                    })?;
                if receipt.state == EffectiveReceiptState::Acknowledged {
                    prior.push(id.clone());
                } else {
                    newly.push(id.clone());
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
            settle_pending(
                tx,
                decision,
                &seat,
                &request.operation,
                &newly,
                &observation,
                actor.binding_generation,
                &actor.binding_id,
                "ack",
            )?;
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

/// Settles validated pending receipts of one seat: the late overdue warning
/// first, then the receipt rows, warning-condition clearing, one compact info
/// event per thread (key prefix `event_prefix`), filter revisions and delivery
/// observations. Shared by `ack`, `ack_displayed` and `ack_mod_delivered`.
// Allowed: shared settlement writes; each argument is one distinct input of the decision.
#[allow(clippy::too_many_arguments)]
fn settle_pending(
    tx: &Transaction<'_>,
    decision: DecisionInstant,
    seat: &SeatId,
    operation: &crate::protocol::ids::OperationId,
    newly: &[MessageId],
    observation: &str,
    binding_generation: u64,
    binding: &BindingId,
    event_prefix: &str,
) -> Result<(), ApiError> {
    let mut by_thread: BTreeMap<ThreadId, Vec<MessageId>> = BTreeMap::new();
    for id in newly {
        let thread: String = tx
            .query_row(
                "SELECT thread_id FROM messages WHERE id=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        by_thread
            .entry(ThreadId::new(thread))
            .or_default()
            .push(id.clone());
    }
    // All late warnings precede settlement and its compact info event.
    for id in newly {
        let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared"))?;
        if receipt.state == EffectiveReceiptState::Pending
            && effective_deadline(tx, &receipt)?.is_some_and(|deadline| deadline <= decision.utc.0)
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
    for id in newly {
        let receipt = effective::effective_receipt(tx, id.as_str(), seat.as_str())?
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "validated receipt disappeared"))?;
        let manifest: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM send_manifests WHERE message_id=?1)",
                [id.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if manifest {
            tx.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at,ack_binding_id) VALUES (?1,?2,'acked',?3,?4,?2,?5,?6,?7,?9) ON CONFLICT(message_id,seat_id) DO UPDATE SET state='acked',deadline_at=CASE WHEN ?8 THEN COALESCE(receipt_state.deadline_at,excluded.deadline_at) ELSE receipt_state.deadline_at END,ack_actor_seat_id=excluded.ack_actor_seat_id,ack_generation=excluded.ack_generation,ack_observation=excluded.ack_observation,acked_at=excluded.acked_at,ack_binding_id=excluded.ack_binding_id WHERE receipt_state.state='pending'",
                params![id.as_str(),seat.as_str(),receipt.available_at,receipt.deadline_at,binding_generation as i64,observation,decision.utc.0,receipt.state==EffectiveReceiptState::Pending,binding.as_str()]).map_err(store_error)?;
        } else {
            // Persist the original projected clock before changing its state.
            // Catch-up's extended deadline is separate; waivers gain no clock.
            let pending_deadline = if receipt.state == EffectiveReceiptState::Pending {
                receipt.deadline_at
            } else {
                None
            };
            tx.execute("UPDATE receipts SET state='acked',deadline_at=COALESCE(deadline_at,?6),ack_actor_seat_id=?1,ack_generation=?2,ack_observation=?3,acked_at=?4,ack_binding_id=?7 WHERE message_id=?5 AND seat_id=?1 AND state='pending'",
                params![seat.as_str(),binding_generation as i64,observation,decision.utc.0,id.as_str(),pending_deadline,binding.as_str()]).map_err(store_error)?;
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
        let mut payload =
            json!({"event":"ack", "seat":seat, "messages":ids, "decided_at":decision.utc.0})
                .to_string();
        if payload.len() > 4096 {
            let digest = schema::canonical_digest(ids)?;
            let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            payload = json!({"event":"ack", "seat":seat, "count":ids.len(), "message_ids_sha256":hash, "decided_at":decision.utc.0}).to_string();
        }
        let key = format!(
            "{event_prefix}:{}:{}:{}:{}:{}:{}",
            seat.as_str().len(),
            seat.as_str(),
            operation.as_str().len(),
            operation.as_str(),
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
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReceiptDueCursor {
    pub high_water_ordinal: i64,
    pub after_deadline: Option<i64>,
    pub after_ordinal: i64,
    pub sparse_high_water_ordinal: i64,
    pub sparse_after_ordinal: i64,
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

pub(crate) const PHYSICAL_DUE_SELECTION: &str = "SELECT ordinal,message_id,seat_id,deadline_at FROM receipts INDEXED BY receipts_required_due_ordinal WHERE state='pending' AND ack_required=1 AND warning_message_id IS NULL AND deadline_at IS NOT NULL AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4";

fn project_receipt_origin(
    tx: &Connection,
    table: &str,
    message: &str,
    seat: &str,
) -> Result<(), ApiError> {
    debug_assert!(matches!(table, "receipts" | "receipt_state"));
    let publication = if table == "receipt_state" {
        "JOIN send_manifests sm ON sm.message_id=r.message_id AND sm.instance_id=m.instance_id AND sm.thread_id=m.thread_id JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=r.seat_id AND pr.thread_id=m.thread_id"
    } else {
        ""
    };
    tx.execute(&format!("INSERT OR IGNORE INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id) SELECT m.instance_id,m.actor_seat_id,c.open_warning_id,m.id FROM {table} r JOIN messages m ON m.id=r.message_id AND (m.author_kind='native' OR m.author_kind IS NULL) AND m.author_service_id IS NULL {publication} JOIN seats s ON s.id=m.actor_seat_id AND s.instance_id=m.instance_id JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id AND c.condition_kind='receipt' AND c.clear_warning_id IS NULL AND c.thread_id=m.thread_id AND c.affected_seat_id=r.seat_id WHERE r.message_id=?1 AND r.seat_id=?2"),params![message,seat]).map_err(store_error)?;
    Ok(())
}

/// All upgrade producers share one allowance. Producer-local Task5 completion
/// remains intact; only this durable combined marker emits the global reset.
fn repair_receipt_discovery(
    tx: &Connection,
    limit: u16,
) -> Result<super::materialization::ReceiptBackfillResult, ApiError> {
    let deadlines = super::materialization::backfill_receipt_deadlines_in(tx, limit)?;
    let (mut physical,physical_high,mut sparse,sparse_high,completed):(i64,i64,i64,i64,bool)=tx.query_row("SELECT physical_after,physical_high_water,sparse_after,sparse_high_water,completed FROM receipt_discovery_backfill WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(store_error)?;
    let (physical_before, sparse_before) = (physical, sparse);
    let mut inspected = deadlines.inspected;
    if !completed {
        for (table, after, high) in [
            ("receipts", &mut physical, physical_high),
            ("receipt_state", &mut sparse, sparse_high),
        ] {
            if inspected == limit || *after == high {
                continue;
            }
            let ordinal = if table == "receipts" {
                "ordinal"
            } else {
                "rowid"
            };
            let rows: Vec<(i64, String, String)> = {
                let mut statement=tx.prepare(&format!("SELECT {ordinal},message_id,seat_id FROM {table} WHERE {ordinal}>?1 AND {ordinal}<=?2 ORDER BY {ordinal} LIMIT ?3")).map_err(store_error)?;
                statement
                    .query_map(params![*after, high, limit - inspected], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .map_err(store_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(store_error)?
            };
            let count = u16::try_from(rows.len()).map_err(|_| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    "receipt discovery repair batch too large",
                )
            })?;
            let short = count < limit - inspected;
            inspected += count;
            for (at, message, seat) in rows {
                project_receipt_origin(tx, table, &message, &seat)?;
                if table == "receipt_state" {
                    tx.execute("INSERT INTO receipt_due_sparse(message_id,seat_id,deadline_at) SELECT message_id,seat_id,deadline_at FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='pending' AND ack_required=1 AND deadline_at IS NOT NULL AND warning_message_id IS NULL ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at",params![message,seat]).map_err(store_error)?;
                }
                *after = at;
            }
            if short {
                *after = high;
            }
        }
    }
    let deadline_complete: bool = tx
        .query_row(
            "SELECT completed FROM receipt_deadline_backfill WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let now_complete = deadline_complete && physical == physical_high && sparse == sparse_high;
    // An unchanged marker is not rewritten: the due scan runs on every
    // deadline tick, and an idle daemon commits nothing.
    if (physical, sparse, now_complete) != (physical_before, sparse_before, completed) {
        tx.execute("UPDATE receipt_discovery_backfill SET physical_after=?1,sparse_after=?2,completed=?3 WHERE singleton=1",params![physical,sparse,now_complete]).map_err(store_error)?;
    }
    Ok(super::materialization::ReceiptBackfillResult {
        inspected,
        more: !now_complete,
        completed: !completed && now_complete,
    })
}

/// Scans pending, unmarked deadline rows through the partial due index.
/// Historical repair and due discovery share the total row allowance and
/// transaction. Repair cannot monopolize ordinary due work at the normal100 cap.
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
            let repair = repair_receipt_discovery(tx,limit.div_ceil(2))?;
            let current = if repair.completed {
                ReceiptDueCursor { extension:current.extension.clone(),..ReceiptDueCursor::default() }
            } else { current };
            let due_limit = limit-repair.inspected;
            let fresh = current.high_water_ordinal == 0 && current.sparse_high_water_ordinal == 0;
            let physical_high = if fresh {
                tx.query_row("SELECT COALESCE(MAX(ordinal),0) FROM receipts", [], |r| r.get::<_,i64>(0)).map_err(store_error)?
            } else { current.high_water_ordinal };
            let sparse_high = if fresh {
                tx.query_row("SELECT COALESCE(MAX(ordinal),0) FROM receipt_due_sparse", [], |r| r.get::<_,i64>(0)).map_err(store_error)?
            } else { current.sparse_high_water_ordinal };
            let physical_page = |after_deadline: Option<i64>, after_ordinal:i64, cap:u16| -> Result<Vec<(i64,MessageId,SeatId,i64)>,ApiError> {
                let mut stmt=tx.prepare(PHYSICAL_DUE_SELECTION).map_err(store_error)?;
                stmt.query_map(params![after_deadline,after_ordinal,physical_high,i64::from(cap)],|r|Ok((r.get::<_,i64>(0)?,MessageId::new(r.get::<_,String>(1)?),SeatId::new(r.get::<_,String>(2)?),r.get::<_,i64>(3)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)
            };
            let physical_cap=due_limit.div_ceil(2);
            let mut physical=physical_page(current.after_deadline,current.after_ordinal,physical_cap)?;
            let sparse_cap=due_limit-u16::try_from(physical.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"physical due batch too large"))?;
            let mut stmt=tx.prepare("SELECT ordinal,message_id,seat_id,deadline_at FROM receipt_due_sparse WHERE ordinal>?1 AND ordinal<=?2 ORDER BY ordinal LIMIT ?3").map_err(store_error)?;
            let sparse=stmt.query_map(params![current.sparse_after_ordinal,sparse_high,i64::from(sparse_cap)],|r|Ok((r.get::<_,i64>(0)?,MessageId::new(r.get::<_,String>(1)?),SeatId::new(r.get::<_,String>(2)?),r.get::<_,i64>(3)?))).map_err(store_error)?.collect::<Result<Vec<_>,_>>().map_err(store_error)?;
            // A short sparse page lends its unused allowance back to physical
            // discovery. Every query uses the captured high water before LIMIT.
            let remaining=due_limit-u16::try_from(physical.len()+sparse.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"due batch too large"))?;
            let physical_full=physical_cap>0 && physical.len()==usize::from(physical_cap);
            let physical_more=if remaining>0 && physical_full {
                let last=physical.last().unwrap();
                let extra=physical_page(Some(last.3),last.0,remaining)?;
                let full=extra.len()==usize::from(remaining);
                physical.extend(extra);
                full
            } else {physical_full};
            let sparse_more=sparse_cap>0 && sparse.len()==usize::from(sparse_cap);
            Ok((repair,current,due_limit,physical_high,sparse_high,physical_more,sparse_more,physical,sparse))
        },
        |tx, decision, (repair,current,due_limit,physical_high,sparse_high,physical_more,sparse_more,physical,sparse)| {
            let mut inserted=0u16;
            let inspected=repair.inspected+u16::try_from(physical.len()+sparse.len()).map_err(|_|api_error(ErrorCode::StoreCorrupt,"due batch too large"))?;
            let physical_after=physical.last().map(|v|(v.3,v.0));
            let sparse_after=sparse.last().map(|v|(v.0,v.3,v.1.as_str().to_owned(),v.2.as_str().to_owned()));
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
            let more=repair.more||physical_more||sparse_more||due_limit==0;
            let next=if more {ReceiptDueCursor{
                high_water_ordinal:physical_high,
                after_deadline:physical_after.map(|v|v.0).or(current.after_deadline),
                after_ordinal:physical_after.map(|v|v.1).unwrap_or(current.after_ordinal),
                sparse_high_water_ordinal:sparse_high,
                sparse_after_ordinal:sparse_after.as_ref().map(|v|v.0).unwrap_or(current.sparse_after_ordinal),
                sparse_after_deadline:sparse_after.as_ref().map(|v|v.1).or(current.sparse_after_deadline),
                sparse_after_message:sparse_after.as_ref().map(|v|v.2.clone()).or(current.sparse_after_message.clone()),
                sparse_after_seat:sparse_after.map(|v|v.3).or(current.sparse_after_seat.clone()),
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
            "SELECT MAX(extension_until) FROM catch_up WHERE seat_id=?1 AND thread_id=?2",
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
    binding: &BindingId,
    thread: &ThreadId,
    entered_at: UtcMillis,
    p99_ms: u64,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET extension_until=MAX(COALESCE(extension_until,0),?3) WHERE binding_id=?1 AND thread_id=?2",
        params![binding.as_str(), thread.as_str(), extension_end(entered_at, p99_ms)],
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
    binding: &BindingId,
    thread: &ThreadId,
    now: UtcMillis,
    exit_grace_ms: u64,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET extension_until=?3 WHERE binding_id=?1 AND thread_id=?2",
        params![
            binding.as_str(),
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

#[cfg(test)]
#[path = "../../tests/store/mod_ack.rs"]
mod mod_ack_tests;
