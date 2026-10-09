//! Canonical compound protection and absorbing terminal replay.
use super::connection::{api_error, store_error};
use crate::protocol::{
    handoff::{HandoffIdentity, HandoffResult, HandoffState},
    ids::ThreadId,
    results::{ApiError, CommandResult, ErrorCode},
    time::UtcMillis,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

fn scope(identity: &HandoffIdentity) -> String {
    format!("seat:{}", identity.claim.seat.as_str())
}
fn validate_identity(identity: &HandoffIdentity) -> Result<(), ApiError> {
    if identity.digest.len() != 64
        || !identity
            .digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || identity.claim.instance.is_empty()
        || identity.compound.as_str().is_empty()
        || [
            identity.compound.as_str(),
            identity.create_key.as_str(),
            identity.invite_key.as_str(),
            identity.send_key.as_str(),
        ]
        .iter()
        .any(|key| key.is_empty() || key.len() > 256)
        || identity.create_key == identity.invite_key
        || identity.create_key == identity.send_key
        || identity.invite_key == identity.send_key
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid immutable handoff identity",
        ));
    }
    Ok(())
}

/// Read the CURRENT state only after matching the entire immutable identity.
/// Completed rows grant historical presentation, never live action authority.
pub fn current(
    db: &Connection,
    identity: &HandoffIdentity,
) -> Result<Option<HandoffResult>, ApiError> {
    validate_identity(identity)?;
    type Columns = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    );
    let row:Option<Columns>=db.query_row("SELECT digest,claim_json,recipient,create_key,invite_key,send_key,original_thread,thread_id,state FROM channel_handoff_fences WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",params![identity.claim.instance,scope(identity),identity.compound.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(store_error)?;
    let Some((digest, claim, recipient, create, invite, send, original, thread, state)) = row
    else {
        return Ok(None);
    };
    let saved_claim: crate::protocol::authority::CallerClaim = serde_json::from_str(&claim)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid stored handoff claim"))?;
    if digest != identity.digest
        || saved_claim != identity.claim
        || recipient != identity.recipient.as_str()
        || create != identity.create_key.as_str()
        || invite != identity.invite_key.as_str()
        || send != identity.send_key.as_str()
        || original.as_deref() != identity.thread.as_ref().map(ThreadId::as_str)
    {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "handoff immutable identity differs",
        ));
    }
    Ok(Some(HandoffResult {
        compound: identity.compound.clone(),
        thread: thread.map(ThreadId::new),
        state: if state == "completed" {
            HandoffState::Completed
        } else {
            HandoffState::Live
        },
    }))
}
fn thread_in_instance(
    db: &Connection,
    identity: &HandoffIdentity,
    thread: &ThreadId,
) -> Result<bool, ApiError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
        params![thread.as_str(), identity.claim.instance],
        |r| r.get(0),
    )
    .map_err(store_error)
}
pub(crate) fn validate_live(
    db: &Connection,
    identity: &HandoffIdentity,
    thread: Option<&ThreadId>,
) -> Result<(), ApiError> {
    super::seats::cooperative_mapping(db, &identity.claim, None)?;
    if let Some(thread) = thread {
        let row:Option<(bool,bool)>=db.query_row("SELECT archived,EXISTS(SELECT 1 FROM memberships m WHERE m.thread_id=t.id AND m.seat_id=?2 AND m.state='joined') FROM threads t WHERE id=?1 AND instance_id=?3",params![thread.as_str(),identity.claim.seat.as_str(),identity.claim.instance],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
        match row {
            Some((true, _)) => {
                return Err(api_error(ErrorCode::Archived, "handoff thread is archived"));
            }
            Some((false, true)) => {}
            _ => {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    "handoff requires a joined sender in this instance",
                ));
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_handoff_requirement(
    db: &Connection,
    requirement: &crate::protocol::authority::HandoffRequirement,
    replay: bool,
) -> Result<(), ApiError> {
    match requirement {
        crate::protocol::authority::HandoffRequirement::Delivery(request) => {
            if replay {
                validate_delivery_replay(db, request)
            } else {
                validate_delivery_effect(db, request)
            }
        }
        crate::protocol::authority::HandoffRequirement::BootstrapChild { namespace, command } => {
            super::topology_handoff::validate_selected_child_phase(db, namespace, command, replay)
        }
    }
}
/// Only an exact absorbing fence grants historical presentation without live guards.
pub(crate) fn validate_delivery_replay(
    db: &Connection,
    request: &crate::protocol::handoff::DeliveryMutation,
) -> Result<(), ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let identity = request.identity();
    let saved = current(db, &identity)?;
    if saved
        .as_ref()
        .is_some_and(|v| v.state == HandoffState::Completed)
    {
        return Ok(());
    }
    validate_live(
        db,
        &identity,
        saved.as_ref().and_then(|v| v.thread.as_ref()),
    )?;
    validate_delivery_effect(db, request)
}
/// The explicit envelope selects this guard, never an operation-key prefix.
pub(crate) fn validate_delivery_effect(
    db: &Connection,
    request: &crate::protocol::handoff::DeliveryMutation,
) -> Result<(), ApiError> {
    request
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let identity = request.identity();
    let saved = current(db, &identity)?;
    use crate::protocol::handoff::DeliveryAction;
    if !matches!(request.action, DeliveryAction::Begin(_))
        && saved.as_ref().is_none_or(|r| r.state != HandoffState::Live)
    {
        return Err(api_error(
            ErrorCode::Conflict,
            "delivery effect requires exact live fence",
        ));
    }
    if let Some(saved) = saved {
        if saved.state != HandoffState::Live {
            return Err(api_error(ErrorCode::Conflict, "delivery is completed"));
        }
        let requested = match &request.action {
            DeliveryAction::Invite(v) => Some(&v.thread),
            DeliveryAction::Send(v) => Some(&v.thread),
            _ => None,
        };
        if requested.is_some() && saved.thread.as_ref() != requested {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "delivery phase thread differs from canonical fence",
            ));
        }
    }
    super::seats::eligible_delivery_recipient(db, &identity.claim.instance, &identity.recipient)
}

fn recorded_create(
    db: &Connection,
    identity: &HandoffIdentity,
) -> Result<Option<ThreadId>, ApiError> {
    let recorded: Option<String> = db
        .query_row(
            "SELECT result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![scope(identity), identity.create_key.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(json) = recorded else {
        return Ok(None);
    };
    let result: CommandResult = serde_json::from_str(&json).map_err(|_| {
        api_error(
            ErrorCode::StoreCorrupt,
            "invalid stored handoff create result",
        )
    })?;
    let CommandResult::ThreadCreated(thread) = result else {
        return Err(api_error(
            ErrorCode::Conflict,
            "handoff create key belongs to another operation",
        ));
    };
    if !thread_in_instance(db, identity, &thread)? {
        return Err(api_error(
            ErrorCode::Conflict,
            "handoff create result belongs to another instance",
        ));
    }
    Ok(Some(thread))
}
fn insert(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    thread: Option<&ThreadId>,
    origin: &str,
    source: Option<&str>,
    now: UtcMillis,
) -> Result<(), ApiError> {
    let claim = serde_json::to_string(&identity.claim)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid handoff claim"))?;
    tx.execute("INSERT INTO channel_handoff_fences(instance_id,actor_scope,compound,digest,claim_json,recipient,create_key,invite_key,send_key,original_thread,thread_id,origin,state,created_at,source_identity) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'live',?13,?14)",params![identity.claim.instance,scope(identity),identity.compound.as_str(),identity.digest,claim,identity.recipient.as_str(),identity.create_key.as_str(),identity.invite_key.as_str(),identity.send_key.as_str(),identity.thread.as_ref().map(ThreadId::as_str),thread.map(ThreadId::as_str),origin,now.0,source]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(ref failure,_) if failure.code==rusqlite::ErrorCode::ConstraintViolation){api_error(ErrorCode::Conflict,"handoff child key already registered")}else{store_error(e)})?;
    Ok(())
}

/// Called only inside the accountable Begin deciding transaction. Exact
/// completed replay is deliberately before live binding/membership guards.
pub fn begin_pending(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    super::topology_handoff::guard_child_begin(tx, identity)?;
    begin_pending_impl(tx, identity, now)
}

pub(crate) fn begin_pending_impl(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    if let Some(mut result) = current(tx, identity)? {
        if result.state == HandoffState::Completed {
            return Ok(result);
        }
        if result.thread.is_none() {
            result.thread = recorded_create(tx, identity)?;
        }
        validate_live(tx, identity, result.thread.as_ref())?;
        tx.execute("UPDATE channel_handoff_fences SET origin='cooperative_pending_claim',thread_id=?4 WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3 AND state='live'",params![identity.claim.instance,scope(identity),identity.compound.as_str(),result.thread.as_ref().map(ThreadId::as_str)]).map_err(store_error)?;
        return Ok(result);
    }
    let thread = match &identity.thread {
        Some(thread) => Some(thread.clone()),
        None => recorded_create(tx, identity)?,
    };
    validate_live(tx, identity, thread.as_ref())?;
    insert(
        tx,
        identity,
        thread.as_ref(),
        "cooperative_pending_claim",
        None,
        now,
    )?;
    Ok(HandoffResult {
        compound: identity.compound.clone(),
        thread,
        state: HandoffState::Live,
    })
}

/// Explicit daemon-selected namespace seam for a newly linked child. Public
/// activation must pass its actual InstancePaths, never the retained parent.
pub fn begin_linked_pending(
    tx: &Transaction<'_>,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    identity: &HandoffIdentity,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    super::topology_handoff::guard_linked_begin(tx, canonical, identity)?;
    begin_pending_impl(tx, identity, now)
}

pub fn complete_pending(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    super::topology_handoff::guard_bare_completion(tx, identity)?;
    complete_linked_child(tx, identity, now)
}

/// Internal wrapper seam; the caller validates and completes both linked fences.
pub(crate) fn complete_linked_child(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    let mut result = current(tx, identity)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "handoff fence missing"))?;
    if result.state == HandoffState::Completed {
        return Ok(result);
    }
    super::seats::cooperative_mapping(tx, &identity.claim, None)?;
    if result.thread.is_none() {
        return Err(api_error(
            ErrorCode::Conflict,
            "handoff has no attached thread",
        ));
    }
    tx.execute("UPDATE channel_handoff_fences SET state='completed',completed_at=?4 WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3 AND state='live'",params![identity.claim.instance,scope(identity),identity.compound.as_str(),now.0]).map_err(store_error)?;
    result.state = HandoffState::Completed;
    Ok(result)
}

/// Files are vetoes only. Crucially, completed precedence is checked in the
/// SAME transaction as insertion, even when the filesystem sample is older.
pub fn import_hint(
    tx: &Transaction<'_>,
    identity: &HandoffIdentity,
    progress_thread: Option<&ThreadId>,
    source: &str,
    now: UtcMillis,
) -> Result<HandoffResult, ApiError> {
    if let Some(result) = current(tx, identity)? {
        if progress_thread
            .zip(result.thread.as_ref())
            .is_some_and(|(hint, canonical)| hint != canonical)
        {
            return Err(api_error(
                ErrorCode::Conflict,
                "legacy progress contradicts canonical handoff thread",
            ));
        }
        return Ok(result);
    }
    super::topology_handoff::guard_child_begin(tx, identity)?;
    let created = recorded_create(tx, identity)?;
    let thread = identity
        .thread
        .as_ref()
        .or(progress_thread)
        .or(created.as_ref());
    if progress_thread
        .zip(identity.thread.as_ref())
        .is_some_and(|(a, b)| a != b)
        || created.as_ref().zip(thread).is_some_and(|(a, b)| a != b)
        || thread.is_some_and(|thread| !thread_in_instance(tx, identity, thread).unwrap_or(false))
    {
        return Err(api_error(
            ErrorCode::Conflict,
            "legacy handoff thread is ambiguous or outside instance",
        ));
    }
    // An unresolved new-thread identity deliberately has no thread_id and
    // blocks the whole instance until a validated Begin/Create reconciles it.
    insert(
        tx,
        identity,
        thread,
        "legacy_local_journal_hint",
        Some(source),
        now,
    )?;
    Ok(HandoffResult {
        compound: identity.compound.clone(),
        thread: thread.cloned(),
        state: HandoffState::Live,
    })
}

/// Called by CREATE inside its deciding transaction. Exact scoped keys couple
/// the new channel to protection with no crash window after the channel exists.
pub fn attach_created(
    tx: &Transaction<'_>,
    instance: &str,
    actor_scope: &str,
    create_key: &str,
    thread: &ThreadId,
) -> Result<(), ApiError> {
    super::topology_handoff::guard_unscoped_create(tx, instance, actor_scope, create_key)?;
    attach_created_legacy(tx, instance, actor_scope, create_key, thread)
}
pub(crate) fn attach_created_legacy(
    tx: &Transaction<'_>,
    instance: &str,
    actor_scope: &str,
    create_key: &str,
    thread: &ThreadId,
) -> Result<(), ApiError> {
    let row:Option<(Option<String>,Option<String>,String)>=tx.query_row("SELECT original_thread,thread_id,state FROM channel_handoff_fences WHERE instance_id=?1 AND actor_scope=?2 AND create_key=?3",params![instance,actor_scope,create_key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
    let Some((original, attached, state)) = row else {
        return Ok(());
    };
    if original.is_some()
        || state == "completed"
        || attached
            .as_deref()
            .is_some_and(|old| old != thread.as_str())
    {
        return Err(api_error(
            ErrorCode::Conflict,
            "handoff create key is already terminal or attached",
        ));
    }
    tx.execute("UPDATE channel_handoff_fences SET thread_id=?4 WHERE instance_id=?1 AND actor_scope=?2 AND create_key=?3 AND state='live'",params![instance,actor_scope,create_key,thread.as_str()]).map_err(store_error)?;
    Ok(())
}

pub(crate) fn installed(db: &Connection) -> Result<bool, ApiError> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='channel_handoff_fences')",[],|r|r.get(0)).map_err(store_error)
}

/// Explicit presentation is essential: the shared historical-operation helper
/// skips validate/apply on replay. Live replay rechecks current guards, while
/// completed replay only matches retained immutable identity and instance.
pub fn mutate(
    context: &super::connection::StoreContext,
    db: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    command: &crate::protocol::handoff::HandoffMutation,
    permit: crate::protocol::authority::MutationPermit,
    complete: bool,
) -> Result<CommandResult, ApiError> {
    mutate_impl(context, db, budget, command, permit, complete, None)
}
pub(crate) fn mutate_in_namespace(
    context: &super::connection::StoreContext,
    db: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    command: &crate::protocol::handoff::HandoffMutation,
    permit: crate::protocol::authority::MutationPermit,
    complete: bool,
    canonical: &crate::protocol::handoff::HandoffNamespace,
) -> Result<CommandResult, ApiError> {
    mutate_impl(
        context,
        db,
        budget,
        command,
        permit,
        complete,
        Some(canonical),
    )
}
fn mutate_impl(
    context: &super::connection::StoreContext,
    db: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    command: &crate::protocol::handoff::HandoffMutation,
    mut permit: crate::protocol::authority::MutationPermit,
    complete: bool,
    canonical: Option<&crate::protocol::handoff::HandoffNamespace>,
) -> Result<CommandResult, ApiError> {
    if !installed(db)? {
        return Err(api_error(
            ErrorCode::Unsupported,
            "daemon has no archival handoff contract",
        ));
    }
    let delivery = permit.delivery_requirement().cloned();
    let identity = &command.identity;
    validate_identity(identity)?;
    let (claim, issuance, _) = permit.cooperative_metadata();
    let digest = super::control::cooperative_payload_hash(
        if complete {
            "complete_handoff"
        } else {
            "begin_handoff"
        },
        command,
    )?;
    let obligation =
        crate::protocol::authority::ObligationRef::CheckIn(identity.claim.seat.clone());
    super::schema::execute_budgeted_idempotent_transaction_with_constraints(
        context,
        db,
        budget,
        Some(&issuance),
        &scope(identity),
        command.operation.as_str(),
        digest,
        |tx| super::seats::cooperative_instance(tx, &identity.claim.instance, &identity.claim),
        |_| Ok(()),
        |tx, decision| {
            super::control::decide_accountable(
                tx,
                decision,
                &mut permit,
                &claim,
                &identity.claim.seat,
                &command.operation,
                &obligation,
                &digest,
            )?;
            let result = if complete {
                complete_pending(tx, identity, decision.utc)?
            } else if let Some(canonical) = canonical {
                super::topology_handoff::begin_selected_child(
                    tx,
                    canonical,
                    identity,
                    decision.utc,
                )?
            } else {
                begin_pending(tx, identity, decision.utc)?
            };
            Ok(CommandResult::Handoff(result))
        },
        |tx, _historical| {
            let result = current(tx, identity)?.ok_or_else(|| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    "stored handoff result has no fence",
                )
            })?;
            if result.state == HandoffState::Live {
                if complete {
                    return Err(api_error(
                        ErrorCode::StoreCorrupt,
                        "completed operation has live fence",
                    ));
                }
                if let Some(canonical) = canonical {
                    super::topology_handoff::begin_selected_child(
                        tx,
                        canonical,
                        identity,
                        context.clock().utc_now(),
                    )?;
                }
                if let Some(delivery) = &delivery {
                    validate_delivery_effect(tx, delivery)?;
                }
                validate_live(tx, identity, result.thread.as_ref())?;
            }
            Ok(CommandResult::Handoff(result))
        },
    )
}
