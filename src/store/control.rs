//! Thread and membership controls.

#[cfg(test)]
use crate::protocol::authority::OperatorActor;
use crate::protocol::authority::{MutationPermit, ObligationRef};
#[cfg(test)]
use crate::protocol::commands::{AcceptRequired, OperatorOrphanInvite};
#[cfg(test)]
use crate::protocol::ids::prefix;
use crate::protocol::ids::{SeatId, ThreadId};
use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::effective;
use crate::store::schema;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

mod direct;
mod invitation;
mod membership;
mod retirement;
mod thread;
pub(crate) use direct::direct_pair;
pub(crate) use direct::direct_pair_selection;
pub use direct::ensure_direct_thread;
pub(crate) use direct::guard_pair_member;
pub(crate) use direct::guard_pair_service_owner;
pub(crate) use direct::pending_auto_join;
pub(crate) use direct::validate_pair_send;
pub use direct::{direct_auto_join_payloads, resolve_direct_pair};
pub use invitation::accept;
pub use invitation::accept_required;
pub(crate) use invitation::author_invitation;
pub(crate) use invitation::current_invitation_for_accept;
pub use invitation::invitation_invitee;
pub use invitation::invite;
pub use invitation::materialize_answer;
pub use invitation::materialize_invitation;
pub use invitation::operator_orphan_invite;
pub use invitation::reject;
pub(crate) use invitation::{Placement, insert_invitation};
pub(crate) use membership::author_membership;
pub(crate) use membership::bump_member_directory;
pub use membership::join;
use membership::joined_in_thread;
pub use membership::leave;
pub use membership::{Transition, materialize_membership, settle_superseded_invitations};
pub use retirement::advance_retirement;
pub use retirement::begin_retirement;
pub(crate) use retirement::begin_retirement_authored;
#[cfg(test)]
pub(crate) use retirement::begin_retirement_fence;
pub use thread::NewThread;
pub use thread::archive;
pub(crate) use thread::author_register;
pub use thread::create_thread;
pub use thread::create_thread_in_namespace;
pub use thread::materialize_register;
pub use thread::materialize_thread_created;
pub use thread::materialize_thread_direct;
pub use thread::reopen;
pub use thread::set_thread_name;
pub use thread::set_topic;

pub(super) fn validate_actor(
    tx: &Transaction<'_>,
    seat: &SeatId,
    target: &str,
) -> Result<String, ApiError> {
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
    /// The acting binding (spec §2): receipts record it as `ack_binding_id`, sends as
    /// `sender_binding_id`.
    pub binding_id: crate::protocol::ids::BindingId,
    /// Whether the acting binding was connected when the decision resolved it.
    pub binding_connected: bool,
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
    context: &StoreContext,
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
    let (mapping, resolution) = super::seats::decide_cooperative(
        context,
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
    let binding = resolution.binding();
    // The daemon decides read-only from the resolved binding's role (TRUST-POLICY A2); the
    // claim-shape check in dispatch is only an early refusal.
    require_top_level_binding(binding)?;
    Ok(AccountableActor {
        harness: claim.harness,
        native_session: claim.native_session.clone(),
        execution: claim.execution.clone(),
        host_boot: crate::protocol::ids::HostBootId::new(mapping.boot),
        target_generation: mapping.revision,
        binding_generation: mapping.generation,
        observed_at_utc: at.utc,
        provenance: claim.harness.cooperative_provenance(),
        binding_id: binding.binding_id.clone(),
        binding_connected: binding.liveness == super::bindings::Liveness::Connected,
    })
}

/// The resolved binding must be top-level: a subagent binding is read-only (spec decision 13).
pub(crate) fn require_top_level_binding(
    binding: &super::bindings::BindingRow,
) -> Result<(), ApiError> {
    if binding.role == super::bindings::BindingRole::Subagent {
        return Err(api_error(
            ErrorCode::SubagentReadOnly,
            crate::protocol::authority::SUBAGENT_READ_ONLY_DETAIL,
        ));
    }
    Ok(())
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

/// Complete passive presentation, and negotiated exact native-hook receipt candidates.
pub fn complete_inbox_delivery(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    request: &crate::protocol::commands::CompleteInboxDelivery,
    mut permit: MutationPermit,
) -> Result<CommandResult, ApiError> {
    use crate::protocol::authority::CallerRole;
    let distinct: std::collections::BTreeSet<_> = request.messages.iter().collect();
    let receipt_distinct: std::collections::BTreeSet<_> =
        request.receipt_candidates.iter().collect();
    if request.claim.role == CallerRole::Subagent
        || request.messages.len() + request.receipt_candidates.len() == 0
        || request.messages.len() + request.receipt_candidates.len() > 100
        || distinct.len() != request.messages.len()
        || receipt_distinct.len() != request.receipt_candidates.len()
        || (!request.receipt_candidates.is_empty()
            && request.via != Some(crate::protocol::commands::LazyCompletionVia::HookContext))
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid delivery completion batch or caller",
        ));
    }
    // A hook-pull seat's lazy rows complete only through `DeliveryConfirm` or its own text inbox
    // (TRUST-POLICY A8); the claim's harness is checked against the canonical binding below.
    if request.via == Some(crate::protocol::commands::LazyCompletionVia::HookContext)
        && crate::delivery::declares(
            request.claim.harness.as_str(),
            crate::harness::adapter::DeliveryMechanism::HookPull,
        )
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "a hook-pull seat completes lazy rows through DeliveryConfirm",
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
            if !request.receipt_candidates.is_empty() {
                super::receipts::validate_hook_context(
                    tx,
                    &request.claim,
                    &request.receipt_candidates,
                )?;
            }
            for id in &request.messages {
                if !tx
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
            let actor = decide_accountable(
                context,
                tx,
                decision,
                &mut permit,
                &request.claim,
                seat,
                &request.operation,
                &ObligationRef::CheckIn(seat.clone()),
                &digest,
            )?;
            if !request.receipt_candidates.is_empty() {
                super::receipts::complete_hook_context(
                    context,
                    tx,
                    decision,
                    &actor,
                    &request.claim,
                    &request.operation,
                    &request.receipt_candidates,
                )?;
            }
            // The display counts for the claim's binding; a claim matching no open binding (a
            // pre-binding client) completes for the whole seat, as before V34.
            let binding = super::bindings::match_claim(
                tx,
                &super::bindings::BindingKey::from_claim(&request.claim),
            )?
            .map(|b| b.binding_id);
            super::lazy_delivery::complete_for_binding(
                tx,
                &request.claim.instance,
                seat,
                binding.as_ref(),
                &request.messages,
            )?;
            Ok(CommandResult::InboxDeliveryCompleted(
                request.messages.clone(),
            ))
        },
    )
}

/// Advance the caller binding's own cursors over the messages it fully displayed (spec §2). The
/// claim resolves with the plain trigger (it may enroll); only that binding's cursor moves, so a
/// subagent claim is accepted: no seat state changes and no receipt is settled. Idempotent per
/// operation key. Unjoined threads remain ignored except exact published addressed Lazy
/// pair IDs in an existing own stream or the active direct-invitation episode.
pub fn advance_cursors(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    request: &crate::protocol::commands::AdvanceCursors,
) -> Result<CommandResult, ApiError> {
    let claim = &request.claim;
    let digest = cooperative_payload_hash("advance_cursors", request)?;
    schema::execute_budgeted_idempotent_transaction(
        context,
        conn,
        budget,
        &format!("seat:{}", claim.seat.as_str()),
        request.operation.as_str(),
        digest,
        |tx| super::seats::cooperative_claim(tx, context, &claim.instance, claim).map(|_| ()),
        |tx| super::seats::claimed_mapping(tx, context, &claim.instance, claim, None).map(|_| ()),
        |tx, at| {
            let mapping = super::seats::claimed_mapping(tx, context, &claim.instance, claim, None)?;
            // Own stream (spec §2), allowed for subagent bindings.
            let binding = super::seats::resolve_cooperative(tx, at, claim, None, &mapping)?
                .binding()
                .binding_id
                .clone();
            let mut by_thread: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
            let mut joined_threads: std::collections::BTreeMap<String, bool> = Default::default();
            for id in &request.displayed {
                let row: Option<(String, i64)> = tx
                    .query_row(
                        "SELECT thread_id,sequence FROM messages WHERE id=?1 AND instance_id=?2",
                        params![id.as_str(), claim.instance],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(store_error)?;
                let (thread, sequence) = row.ok_or_else(|| {
                    api_error(ErrorCode::InvalidRequest, "displayed message not found")
                })?;
                let joined = match joined_threads.get(&thread) {
                    Some(joined) => *joined,
                    None => {
                        let joined: bool = tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?1 AND seat_id=?2 AND state='joined')",
                            params![thread, claim.seat.as_str()], |r| r.get(0),
                        ).map_err(store_error)?;
                        joined_threads.insert(thread.clone(), joined);
                        joined
                    }
                };
                // Filter before sequence vectors: a supplied system, ordinary,
                // foreign or old-episode ID cannot bootstrap a cursorless pair.
                if joined
                    || super::lazy_delivery::direct_lazy_display_admitted(
                        tx,
                        &claim.instance,
                        &claim.seat,
                        &binding,
                        id,
                    )?
                {
                    let sequence = u64::try_from(sequence).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative message sequence")
                    })?;
                    by_thread.entry(thread).or_default().push(sequence);
                }
            }
            let mut reader_changed = false;
            for (thread, sequences) in by_thread {
                let (_, changed) = super::read_cursors::advance_through_displayed_with_change(
                    tx,
                    &binding,
                    &ThreadId::new(thread),
                    &sequences,
                )?;
                reader_changed |= changed;
            }
            if reader_changed {
                super::presentation_reader::note_binding_reader_change(tx, &binding)?;
            }
            // An in-turn top-level display of a message whose receipt is still pending, or of a
            // pending invitation, served this occupant (ht-705): the wake candidate owes it no
            // idle wake for that item. Nothing is settled or accepted; the item stays pending.
            let top_level: bool = tx
                .query_row(
                    "SELECT role='top_level' FROM occupant_bindings WHERE binding_id=?1",
                    [binding.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if top_level && request.in_turn {
                for id in &request.displayed {
                    let pending =
                        effective::effective_receipt(tx, id.as_str(), claim.seat.as_str())?
                            .is_some_and(|receipt| {
                                receipt.state == effective::EffectiveReceiptState::Pending
                            });
                    if pending {
                        tx.execute(
                            "INSERT OR IGNORE INTO in_turn_offer(binding_id,seat_id,item_id) VALUES (?1,?2,?3)",
                            params![binding.as_str(), claim.seat.as_str(), id.as_str()],
                        )
                        .map_err(store_error)?;
                    }
                }
                for id in &request.invitations {
                    // An unknown or settled invitation records nothing.
                    tx.execute(
                        "INSERT OR IGNORE INTO in_turn_offer(binding_id,seat_id,item_id) SELECT ?1,?2,id FROM invitations WHERE id=?3 AND seat_id=?2 AND state='pending'",
                        params![binding.as_str(), claim.seat.as_str(), id.as_str()],
                    )
                    .map_err(store_error)?;
                }
            }
            Ok(CommandResult::CursorsAdvanced(request.displayed.clone()))
        },
        |_tx, result| Ok(result),
    )
}

/// Apply a harness lifecycle event to the caller's bindings (spec §2 Enrollment, §3 end events).
/// A subagent start enrolls its binding (the parent first, if needed); a subagent stop or a
/// session end disconnects with reason `liveness_end_event`. Nothing here ends a binding, moves a
/// seat or touches a receipt, so declared subagent claims are accepted. Stop and end events
/// resolve no mapping: an unknown binding is a no-op, and a pane that is already gone must still
/// be able to report its end. Idempotent per operation key.
pub fn binding_event(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &CallBudget,
    request: &crate::protocol::commands::BindingEvent,
) -> Result<CommandResult, ApiError> {
    use crate::protocol::commands::BindingEventKind;
    let claim = &request.claim;
    let digest = cooperative_payload_hash("binding_event", request)?;
    schema::execute_budgeted_idempotent_transaction(
        context,
        conn,
        budget,
        &format!("seat:{}", claim.seat.as_str()),
        request.operation.as_str(),
        digest,
        |tx| super::seats::cooperative_claim(tx, context, &claim.instance, claim).map(|_| ()),
        |tx| match request.kind {
            BindingEventKind::SubagentStart => {
                super::seats::claimed_mapping(tx, context, &claim.instance, claim, None).map(|_| ())
            }
            BindingEventKind::SubagentStop | BindingEventKind::SessionEnd => Ok(()),
        },
        |tx, at| {
            let changed = match request.kind {
                BindingEventKind::SubagentStart => {
                    let mapping =
                        super::seats::claimed_mapping(tx, context, &claim.instance, claim, None)?;
                    let created = match &request.child_session {
                        Some(child) => {
                            super::seats::attach_registered_child(tx, at, claim, child, &mapping)?
                        }
                        // Own stream (spec §2), allowed for subagent bindings: enrolment only.
                        None => matches!(
                            super::seats::resolve_cooperative(tx, at, claim, None, &mapping)?,
                            super::bindings::Resolution::Enrolled { .. }
                        ),
                    };
                    u32::from(created)
                }
                BindingEventKind::SubagentStop => {
                    let key = match &request.child_session {
                        Some(child) => super::bindings::BindingKey::from_claim(
                            &super::bindings::registered_child_claim(claim, child)?,
                        ),
                        None => super::bindings::BindingKey::from_claim(claim),
                    };
                    super::bindings::end_event_subagent(tx, &key, at.utc)?
                }
                BindingEventKind::SessionEnd => super::bindings::end_event_session(
                    tx,
                    &super::bindings::BindingKey::from_claim(claim),
                    at.utc,
                )?,
            };
            Ok(CommandResult::BindingEventRecorded { changed })
        },
        |_tx, result| Ok(result),
    )
}
