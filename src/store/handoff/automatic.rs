//! Immutable automatic originals and accountable, atomic channel selection.
use super::{current_legacy, insert, scope};
use crate::{
    protocol::{
        authority::{MutationPermit, ObligationRef},
        commands::PermitMutation,
        handoff::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        connection::{StoreContext, api_error, store_error},
        control, schema, seats,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::protocol::handoff::automatic::MAX_AUTOMATIC_ORIGINAL_BYTES as MAX_ORIGINAL_BYTES;
fn mismatch() -> ApiError {
    api_error(
        ErrorCode::OperationPayloadMismatch,
        "automatic handoff immutable original or phase differs",
    )
}
/// Legacy slots are storage compatibility only: Ensure and an inert completion
/// tombstone. Neither slot grants legacy Create or Invite authority.
fn identity(request: &AutomaticDeliveryMutation) -> HandoffIdentity {
    let plan = request.plan();
    HandoffIdentity {
        claim: request.claim.clone(),
        compound: plan.payload.keys.compound.clone(),
        digest: request.digest.clone(),
        recipient: plan.recipient.clone(),
        create_key: plan.payload.keys.ensure.clone(),
        invite_key: plan.payload.keys.complete.clone(),
        send_key: plan.payload.keys.send.clone(),
        thread: None,
    }
}
fn original_bytes(request: &AutomaticDeliveryMutation) -> Result<Vec<u8>, ApiError> {
    request
        .canonical_original_bytes()
        .map_err(ApiError::invalid_request)
}
pub(crate) fn validate_namespace(
    canonical: &HandoffNamespace,
    request: &AutomaticDeliveryMutation,
) -> Result<(), ApiError> {
    crate::store::verified::validated("automatic-delivery-request", request, |r| r.validate())?;
    original_bytes(request)?;
    let selected = &request.plan().payload.namespace;
    if selected.instance != canonical.instance
        || selected.state_dir.as_os_str() != canonical.state_dir.as_os_str()
        || selected.host_endpoint.as_os_str() != canonical.host_endpoint.as_os_str()
    {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "automatic selected namespace differs",
        ));
    }
    Ok(())
}
pub(crate) fn guard_legacy_identity(
    db: &Connection,
    identity: &HandoffIdentity,
) -> Result<(), ApiError> {
    let automatic: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_handoff_companions WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3) OR EXISTS(SELECT 1 FROM automatic_bootstrap_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND role='child')",params![identity.claim.instance,scope(identity),identity.compound.as_str()],|r|r.get(0)).map_err(store_error)?;
    if automatic {
        return Err(mismatch());
    }
    Ok(())
}
pub(crate) fn guard_unscoped_key(
    db: &Connection,
    instance: &str,
    actor: &str,
    key: &str,
) -> Result<(), ApiError> {
    let automatic: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_handoff_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3) OR EXISTS(SELECT 1 FROM automatic_bootstrap_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3)",params![instance,actor,key],|r|r.get(0)).map_err(store_error)?;
    if automatic {
        return Err(mismatch());
    }
    Ok(())
}
/// Compare the full bounded original before any historical presentation.
pub(crate) fn current(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
) -> Result<Option<HandoffResult>, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    crate::store::topology_handoff::automatic::guard_child(db, request, true)?;
    current_after_parent_guard(db, request)
}
// Both callers validate the request and its linked parent before entering this
// unchanged child reader. Phase validation already uses the requested replay
// mode (at least as strict as true), so it need not hydrate the parent twice.
fn current_after_parent_guard(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
) -> Result<Option<HandoffResult>, ApiError> {
    let id = identity(request);
    let bytes: Option<Vec<u8>> = db.query_row("SELECT substr(original_json,1,?4) FROM automatic_handoff_companions WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",params![id.claim.instance,scope(&id),id.compound.as_str(),(MAX_ORIGINAL_BYTES+1) as u32],|r|r.get(0)).optional().map_err(store_error)?;
    let Some(bytes) = bytes else {
        // An ordinary existing fence can never be adopted as automatic.
        if current_legacy(db, &id)?.is_some() {
            return Err(mismatch());
        }
        return Ok(None);
    };
    if bytes.len() > MAX_ORIGINAL_BYTES {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "oversized automatic original",
        ));
    }
    let saved: AutomaticDeliveryMutation =
        crate::store::verified::remembered("automatic-delivery-original", &[&bytes], || {
            let saved: AutomaticDeliveryMutation = serde_json::from_slice(&bytes)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid automatic original"))?;
            saved
                .validate()
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid automatic original"))?;
            if serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid automatic original"))?
                != serde_json::to_value(&saved)
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid automatic original"))?
            {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "automatic original contains lost fields",
                ));
            }
            Ok(saved)
        })?;
    if original_bytes(&saved)? != original_bytes(request)? {
        return Err(mismatch());
    }
    let keys = &request.plan().payload.keys;
    let exact:bool=db.query_row("SELECT state_dir=?4 AND host_endpoint=?5 AND begin_key=?6 AND ensure_key=?7 AND send_key=?8 AND complete_key=?9 FROM automatic_handoff_companions WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",params![id.claim.instance,scope(&id),id.compound.as_str(),request.plan().payload.namespace.state_dir.to_str(),request.plan().payload.namespace.host_endpoint.to_str(),keys.begin.as_str(),keys.ensure.as_str(),keys.send.as_str(),keys.complete.as_str()],|r|r.get(0)).map_err(store_error)?;
    if !exact {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "automatic companion columns differ",
        ));
    }
    for (role, key) in [
        ("compound", &keys.compound),
        ("begin", &keys.begin),
        ("ensure", &keys.ensure),
        ("send", &keys.send),
        ("complete", &keys.complete),
    ] {
        let registered:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_handoff_keys WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3 AND role=?4 AND operation_key=?5)",params![id.claim.instance,scope(&id),id.compound.as_str(),role,key.as_str()],|r|r.get(0)).map_err(store_error)?;
        if !registered {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "automatic child registration differs",
            ));
        }
    }
    current_legacy(db, &id)?
        .map(Some)
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "automatic fence missing"))
}
/// Registered automatic sends retain the same ordinary service entrypoint.
pub(crate) fn selected_send(
    db: &Connection,
    canonical: &HandoffNamespace,
    send: &crate::protocol::commands::SendMessage,
    replay: bool,
) -> Result<bool, ApiError> {
    let bytes:Option<Vec<u8>>=db.query_row("SELECT substr(c.original_json,1,?4) FROM automatic_handoff_keys k JOIN automatic_handoff_companions c USING(instance_id,actor_scope,compound) WHERE k.instance_id=?1 AND k.actor_scope=?2 AND k.operation_key=?3 AND k.role='send'",params![send.claim.instance,format!("seat:{}",send.claim.seat.as_str()),send.operation.as_str(),(MAX_ORIGINAL_BYTES+1) as u32],|r|r.get(0)).optional().map_err(store_error)?;
    let Some(bytes) = bytes else { return Ok(false) };
    if bytes.len() > MAX_ORIGINAL_BYTES {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "oversized automatic original",
        ));
    }
    let mut request: AutomaticDeliveryMutation = serde_json::from_slice(&bytes)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid automatic original"))?;
    request.action = AutomaticDeliveryAction::Send(send.clone());
    validate_namespace(canonical, &request)?;
    validate_phase(db, &request, replay)?;
    Ok(true)
}
pub(crate) fn validate_phase(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
    replay: bool,
) -> Result<(), ApiError> {
    crate::store::topology_handoff::automatic::guard_child(db, request, replay)?;
    crate::store::verified::validated("automatic-delivery-request", request, |r| r.validate())?;
    let saved = current_after_parent_guard(db, request)?;
    let Some(saved) = saved else {
        return if matches!(request.action, AutomaticDeliveryAction::Begin { .. }) {
            Ok(())
        } else {
            Err(api_error(
                ErrorCode::Conflict,
                "automatic phase requires registered fence",
            ))
        };
    };
    if let AutomaticDeliveryAction::Send(send) = &request.action
        && saved.thread.as_ref() != Some(&send.thread)
    {
        return Err(mismatch());
    }
    if replay
        && (saved.state == HandoffState::Completed
            || matches!(
                request.action,
                AutomaticDeliveryAction::Ensure(_) | AutomaticDeliveryAction::Send(_)
            ))
    {
        return Ok(());
    }
    if saved.state != HandoffState::Live {
        return Err(api_error(
            ErrorCode::Conflict,
            "automatic handoff is completed",
        ));
    }
    super::validate_live(db, &identity(request), saved.thread.as_ref())?;
    seats::eligible_delivery_recipient(db, &request.claim.instance, &request.plan().recipient)
}
pub(crate) fn inner(request: &AutomaticDeliveryMutation) -> Result<PermitMutation, ApiError> {
    crate::store::verified::validated("automatic-delivery-request", request, |r| r.validate())?;
    match &request.action {
        AutomaticDeliveryAction::Begin { operation } => {
            Ok(PermitMutation::BeginHandoff(HandoffMutation {
                identity: identity(request),
                operation: operation.clone(),
            }))
        }
        AutomaticDeliveryAction::Ensure(v) => Ok(PermitMutation::EnsureDirectThread(v.clone())),
        AutomaticDeliveryAction::Send(v) => Ok(PermitMutation::SendMessage(v.clone())),
        AutomaticDeliveryAction::Complete { operation } => {
            Ok(PermitMutation::CompleteHandoff(HandoffMutation {
                identity: identity(request),
                operation: operation.clone(),
            }))
        }
        _ => Err(ApiError::unsupported(
            "automatic phase awaits complete guarded runtime",
        )),
    }
}
pub(crate) fn query(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
) -> Result<CommandResult, ApiError> {
    match request.action {
        AutomaticDeliveryAction::Status => current(db, request)?
            .map(CommandResult::Handoff)
            .ok_or_else(|| api_error(ErrorCode::NotFound, "automatic handoff missing")),
        AutomaticDeliveryAction::Prepare => {
            let saved = current(db, request)?;
            if saved
                .as_ref()
                .is_none_or(|v| v.state != HandoffState::Completed)
            {
                super::validate_live(
                    db,
                    &identity(request),
                    saved.as_ref().and_then(|v| v.thread.as_ref()),
                )?;
                seats::eligible_delivery_recipient(
                    db,
                    &request.claim.instance,
                    &request.plan().recipient,
                )?;
            }
            Ok(CommandResult::SeatResolved(
                request.plan().recipient.clone(),
            ))
        }
        _ => Err(ApiError::invalid_request("not an automatic query")),
    }
}
/// Prove publication of this frozen Send, rather than another message on the pair.
/// Published preparations/manifests are retained by the ordinary engine; its
/// discard, abandonment and retention paths only clean unpublished generations.
pub(crate) fn published_send(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
    thread: &crate::protocol::ids::ThreadId,
) -> Result<bool, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let AutomaticDeliveryAction::Send(send) = request.send(thread.clone()).action else {
        unreachable!()
    };
    let actor = format!("seat:{}", request.claim.seat.as_str());
    let expected = schema::canonical_digest(&crate::store::messages::send_payload(&send))?;
    let row: Option<(Vec<u8>, String)> = db.query_row(
        "SELECT digest,substr(result_json,1,131073) FROM operations WHERE actor_scope=?1 AND operation_key=?2",
        params![actor, send.operation.as_str()], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(store_error)?;
    let Some((digest, json)) = row else {
        return Ok(false);
    };
    if digest != expected {
        return Err(mismatch());
    }
    let corrupt = || {
        api_error(
            ErrorCode::StoreCorrupt,
            "invalid automatic Send publication proof",
        )
    };
    if json.len() > MAX_ORIGINAL_BYTES {
        return Err(corrupt());
    }
    let value: serde_json::Value = serde_json::from_str(&json).map_err(|_| corrupt())?;
    let result: CommandResult = serde_json::from_value(value.clone()).map_err(|_| corrupt())?;
    if serde_json::to_value(&result).map_err(|_| corrupt())? != value {
        return Err(corrupt());
    }
    let CommandResult::MessageSent(message) = result else {
        return Err(corrupt());
    };
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM send_preparations p JOIN send_manifests s ON s.preparation_id=p.id AND s.instance_id=p.instance_id AND s.thread_id=p.thread_id JOIN messages m ON m.id=s.message_id AND m.instance_id=p.instance_id AND m.thread_id=p.thread_id WHERE p.instance_id=?1 AND p.operation_scope=?2 AND p.operation_key=?3 AND p.digest=?4 AND p.thread_id=?5 AND s.message_id=?6 AND m.kind='ordinary' AND m.delivery_mode='ordinary')",
        params![request.claim.instance, actor, send.operation.as_str(), expected.as_slice(), thread.as_str(), message.as_str()], |r| r.get(0),
    ).map_err(store_error)?;
    if !valid {
        return Err(corrupt());
    }
    Ok(true)
}

pub(crate) fn mutate(
    context: &StoreContext,
    db: &mut Connection,
    budget: &CallBudget,
    request: &AutomaticDeliveryMutation,
    permit: MutationPermit,
    installation_default_ms: Option<u64>,
) -> Result<CommandResult, ApiError> {
    let command = inner(request)?;
    let (operation, digest) = match &command {
        PermitMutation::BeginHandoff(v) => (
            &v.operation,
            control::cooperative_payload_hash("begin_handoff", v)?,
        ),
        PermitMutation::EnsureDirectThread(v) => (
            &v.operation,
            control::cooperative_payload_hash("ensure_direct_thread", v)?,
        ),
        PermitMutation::CompleteHandoff(v) => (
            &v.operation,
            control::cooperative_payload_hash("complete_handoff", v)?,
        ),
        _ => unreachable!(),
    };
    let id = identity(request);
    let mut permit = permit.with_automatic_delivery(request);
    let (claim, issuance, _) = permit.cooperative_metadata();
    schema::execute_budgeted_idempotent_transaction_with_constraints(
        context,
        db,
        budget,
        Some(&issuance),
        &scope(&id),
        operation.as_str(),
        digest,
        |tx| {
            seats::cooperative_instance(tx, &claim.instance, &claim)?;
            validate_phase(tx, request, true)
        },
        |_| Ok(()),
        |tx, decision| {
            control::decide_accountable(
                tx,
                decision,
                &mut permit,
                &request.claim,
                &request.claim.seat,
                operation,
                &ObligationRef::CheckIn(request.claim.seat.clone()),
                &digest,
            )?;
            match &request.action {
                AutomaticDeliveryAction::Begin { .. } => {
                    if let Some(saved) = current(tx, request)? {
                        return Ok(CommandResult::Handoff(saved));
                    }
                    request
                        .validate_admission()
                        .map_err(ApiError::invalid_request)?;
                    let linked =
                        crate::store::topology_handoff::automatic::guard_child(tx, request, false)?;
                    if !linked {
                        crate::store::topology_handoff::guard_child_begin(tx, &id)?;
                    }
                    let keys = &request.plan().payload.keys;
                    for key in [
                        &keys.compound,
                        &keys.begin,
                        &keys.ensure,
                        &keys.send,
                        &keys.complete,
                    ] {
                        if !linked {
                            crate::store::topology_handoff::guard_unscoped_child_phase(
                                tx,
                                &request.claim.instance,
                                &scope(&id),
                                key.as_str(),
                            )?;
                        }
                        let used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE actor_scope=?1 AND operation_key=?2) OR EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?3 AND actor_scope=?1 AND (compound=?2 OR create_key=?2 OR invite_key=?2 OR send_key=?2))",params![scope(&id),key.as_str(),request.claim.instance],|r|r.get(0)).map_err(store_error)?;
                        if used {
                            return Err(mismatch());
                        }
                    }
                    super::validate_live(tx, &id, None)?;
                    seats::eligible_delivery_recipient(
                        tx,
                        &request.claim.instance,
                        &request.plan().recipient,
                    )?;
                    insert(
                        tx,
                        &id,
                        None,
                        "cooperative_pending_claim",
                        None,
                        decision.utc,
                    )?;
                    tx.execute("INSERT INTO automatic_handoff_companions(instance_id,actor_scope,compound,state_dir,host_endpoint,original_json,begin_key,ensure_key,send_key,complete_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![request.claim.instance,scope(&id),keys.compound.as_str(),request.plan().payload.namespace.state_dir.to_str(),request.plan().payload.namespace.host_endpoint.to_str(),original_bytes(request)?,keys.begin.as_str(),keys.ensure.as_str(),keys.send.as_str(),keys.complete.as_str()]).map_err(store_error)?;
                    for (role, key) in [
                        ("compound", &keys.compound),
                        ("begin", &keys.begin),
                        ("ensure", &keys.ensure),
                        ("send", &keys.send),
                        ("complete", &keys.complete),
                    ] {
                        tx.execute("INSERT INTO automatic_handoff_keys(instance_id,actor_scope,compound,operation_key,role) VALUES(?1,?2,?3,?4,?5)",params![request.claim.instance,scope(&id),keys.compound.as_str(),key.as_str(),role]).map_err(store_error)?;
                    }
                    Ok(CommandResult::Handoff(current(tx, request)?.ok_or_else(
                        || api_error(ErrorCode::StoreCorrupt, "automatic registration missing"),
                    )?))
                }
                AutomaticDeliveryAction::Ensure(v) => {
                    let (thread, created) = control::resolve_direct_pair(
                        tx,
                        decision,
                        &request.claim,
                        &v.recipient,
                        v.creation_policy.as_ref(),
                        installation_default_ms,
                    )?;
                    super::attach_created_legacy(
                        tx,
                        &request.claim.instance,
                        &scope(&id),
                        operation.as_str(),
                        &thread,
                    )?;
                    crate::store::topology_handoff::automatic::attach_thread(tx, request, &thread)?;
                    let attached = current(tx, request)?.and_then(|v| v.thread);
                    if attached.as_ref() != Some(&thread) {
                        return Err(mismatch());
                    }
                    control::direct_pair_selection(
                        tx,
                        thread,
                        &v.recipient,
                        created,
                        v.creation_policy.is_some(),
                    )
                }
                AutomaticDeliveryAction::Complete { .. } => {
                    let mut saved = current(tx, request)?
                        .ok_or_else(|| api_error(ErrorCode::NotFound, "automatic fence missing"))?;
                    if saved.state == HandoffState::Live {
                        let thread = saved.thread.as_ref().ok_or_else(|| {
                            api_error(
                                ErrorCode::Conflict,
                                "automatic handoff has no attached thread",
                            )
                        })?;
                        if !published_send(tx, request, thread)? {
                            return Err(api_error(
                                ErrorCode::Conflict,
                                "automatic handoff has no published frozen Send",
                            ));
                        }
                        tx.execute("UPDATE channel_handoff_fences SET state='completed',completed_at=?4 WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3 AND state='live'",params![id.claim.instance,scope(&id),id.compound.as_str(),decision.utc.0]).map_err(store_error)?;
                        saved.state = HandoffState::Completed;
                    }
                    Ok(CommandResult::Handoff(saved))
                }
                _ => unreachable!(),
            }
        },
        |tx, result| {
            let saved = current(tx, request)?
                .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "automatic fence missing"))?;
            match (&request.action, result) {
                (
                    AutomaticDeliveryAction::Begin { .. }
                    | AutomaticDeliveryAction::Complete { .. },
                    CommandResult::Handoff(_),
                ) => Ok(CommandResult::Handoff(saved)),
                (AutomaticDeliveryAction::Ensure(_), CommandResult::ThreadCreated(thread))
                    if saved.thread.as_ref() == Some(&thread) =>
                {
                    Ok(CommandResult::ThreadCreated(thread))
                }
                (
                    AutomaticDeliveryAction::Ensure(v),
                    CommandResult::DirectThreadEnsured(actual),
                ) if v.creation_policy.is_some()
                    && saved.thread.as_ref() == Some(&actual.thread)
                    && actual.recipient == v.recipient =>
                {
                    Ok(CommandResult::DirectThreadEnsured(actual))
                }
                _ => Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "automatic result differs from exact attached thread",
                )),
            }
        },
    )
}
