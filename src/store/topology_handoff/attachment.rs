//! Exact linkage and atomic, retained-report-backed terminal decisions.
//! Public activation and permit/A2 admission remain the daemon integration lane.
use super::{
    current,
    persistence::{corrupt, decode, same, scope, validate_live},
};
use crate::{
    ports::BootstrapAttachmentGuard,
    protocol::{
        handoff::*,
        ids::ThreadId,
        results::{ApiError, CommandResult, ErrorCode},
        time::UtcMillis,
    },
    store::{
        connection::{api_error, store_error},
        handoff, seats,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

fn conflict(detail: &str) -> ApiError {
    api_error(ErrorCode::Conflict, detail)
}
fn parent_id(
    db: &Connection,
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
) -> Result<i64, ApiError> {
    db.query_row("SELECT id FROM bootstrap_handoffs WHERE instance_id=?1 AND state_dir=?2 AND host_endpoint=?3 AND actor_scope=?4 AND compound=?5",params![canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),scope(identity),identity.compound.as_str()],|r|r.get(0)).map_err(store_error)
}
/// Local savepoint protects callers that catch a refusal and commit their outer transaction.
fn atomic<T>(
    tx: &Transaction<'_>,
    apply: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    tx.execute_batch("SAVEPOINT bootstrap_attachment_decision")
        .map_err(store_error)?;
    match apply() {
        Ok(value) => {
            tx.execute_batch("RELEASE bootstrap_attachment_decision")
                .map_err(store_error)?;
            Ok(value)
        }
        Err(error) => {
            tx.execute_batch(
                "ROLLBACK TO bootstrap_attachment_decision; RELEASE bootstrap_attachment_decision",
            )
            .map_err(store_error)?;
            Err(error)
        }
    }
}

fn canonical_parent(
    db: &Connection,
    canonical: &HandoffNamespace,
    actor_scope: &str,
    key: &str,
    role: &str,
) -> Result<Option<(BootstrapIdentity, BootstrapResult)>, ApiError> {
    let saved:Option<Vec<u8>>=db.query_row("SELECT substr(p.identity_json,1,?7) FROM bootstrap_child_keys k JOIN bootstrap_handoffs p ON p.id=k.parent_id WHERE k.instance_id=?1 AND k.state_dir=?2 AND k.host_endpoint=?3 AND k.actor_scope=?4 AND k.operation_key=?5 AND k.role=?6",params![canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),actor_scope,key,role,(super::MAX_IDENTITY_BYTES+1) as u32],|r|r.get(0)).optional().map_err(store_error)?;
    saved
        .map(|bytes| {
            let id: BootstrapIdentity = decode(&bytes, super::MAX_IDENTITY_BYTES)?;
            let status = current(db, canonical, &id)?.ok_or_else(corrupt)?;
            Ok((id, status))
        })
        .transpose()
}

/// Legacy terminal child results remain historical. An unscoped live path can
/// only refuse registered bootstrap keys; retained paths never authorize it.
pub(crate) fn guard_child_begin(db: &Connection, child: &HandoffIdentity) -> Result<(), ApiError> {
    if handoff::current(db, child)?.is_some_and(|c| c.state == HandoffState::Completed) {
        return Ok(());
    }
    let linked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM bootstrap_child_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND role='handoff')",params![child.claim.instance,format!("seat:{}",child.claim.seat.as_str()),child.compound.as_str()],|r|r.get(0)).map_err(store_error)?;
    if linked {
        return Err(conflict(
            "linked child requires daemon-selected bootstrap namespace",
        ));
    }
    Ok(())
}

pub(crate) fn guard_linked_begin(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    child: &HandoffIdentity,
) -> Result<(), ApiError> {
    if canonical.instance != child.claim.instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "bootstrap child namespace differs",
        ));
    }
    let (identity, status) = canonical_parent(
        tx,
        canonical,
        &format!("seat:{}", child.claim.seat.as_str()),
        child.compound.as_str(),
        "handoff",
    )?
    .ok_or_else(|| conflict("bootstrap child missing in selected namespace"))?;
    if status.state == BootstrapState::Cancelled {
        return Err(conflict("bootstrap is cancelled"));
    }
    let a = status
        .attachment
        .as_ref()
        .ok_or_else(|| conflict("bootstrap attachment missing"))?;
    if !same(&a.handoff, child)? {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap child differs",
        ));
    }
    if status.state == BootstrapState::Completed {
        if handoff::current(tx, child)?.is_none_or(|c| c.state != HandoffState::Completed) {
            return Err(corrupt());
        }
    } else {
        if status.state != BootstrapState::Attached {
            return Err(conflict("bootstrap is not attached"));
        }
        validate_live(tx, canonical, &identity)?;
    }
    Ok(())
}

/// Select an existing linked parent through its full canonical namespace.
pub(crate) fn begin_selected_child(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    child: &HandoffIdentity,
    now: UtcMillis,
) -> Result<crate::protocol::handoff::HandoffResult, ApiError> {
    if canonical_parent(
        tx,
        canonical,
        &format!("seat:{}", child.claim.seat.as_str()),
        child.compound.as_str(),
        "handoff",
    )?
    .is_some()
    {
        handoff::begin_linked_pending(tx, canonical, child, now)
    } else {
        // Foreign linked identities are refused by the existing unscoped guard.
        handoff::begin_pending(tx, child, now)
    }
}

/// Missing selected context never grants history or mutation for a registered child.
pub(crate) fn guard_unscoped_child_phase(
    db: &Connection,
    instance: &str,
    actor: &str,
    key: &str,
) -> Result<(), ApiError> {
    let saved: Option<Vec<u8>> = db.query_row("SELECT substr(p.identity_json,1,?4) FROM bootstrap_child_keys k JOIN bootstrap_handoffs p ON p.id=k.parent_id WHERE k.instance_id=?1 AND k.actor_scope=?2 AND k.operation_key=?3 AND k.role IN ('create','invite','send') LIMIT 1", params![instance,actor,key,(super::MAX_IDENTITY_BYTES+1) as u32], |r|r.get(0)).optional().map_err(store_error)?;
    if let Some(bytes) = saved {
        let identity: BootstrapIdentity = decode(&bytes, super::MAX_IDENTITY_BYTES)?;
        current(db, &identity.payload.handoff.namespace, &identity)?.ok_or_else(corrupt)?;
        return Err(conflict(
            "registered linked phase requires daemon-selected namespace",
        ));
    }
    Ok(())
}

/// Only the typed child Begin consumer interprets the shared begin registration.
/// Parent BeginBootstrap uses that same frozen key with its separate canonical lane.
pub(crate) fn guard_unscoped_child_begin(
    db: &Connection,
    command: &HandoffMutation,
) -> Result<(), ApiError> {
    let claim = &command.identity.claim;
    let saved: Option<Vec<u8>> = db.query_row("SELECT substr(p.identity_json,1,?5) FROM bootstrap_child_keys k JOIN bootstrap_handoffs p ON p.id=k.parent_id WHERE k.instance_id=?1 AND k.actor_scope=?2 AND ((k.operation_key=?3 AND k.role='handoff') OR (k.operation_key=?4 AND k.role='begin')) LIMIT 1", params![claim.instance,format!("seat:{}",claim.seat.as_str()),command.identity.compound.as_str(),command.operation.as_str(),(super::MAX_IDENTITY_BYTES+1) as u32], |r|r.get(0)).optional().map_err(store_error)?;
    if let Some(bytes) = saved {
        let identity: BootstrapIdentity = decode(&bytes, super::MAX_IDENTITY_BYTES)?;
        current(db, &identity.payload.handoff.namespace, &identity)?.ok_or_else(corrupt)?;
        return Err(conflict(
            "registered child Begin requires daemon-selected namespace",
        ));
    }
    Ok(())
}

/// A typed live/runtime requirement selects actual retained child registration.
/// Unrelated legacy phases remain unchanged; registered foreign namespaces refuse.
pub(crate) fn validate_selected_child_phase(
    db: &Connection,
    canonical: &HandoffNamespace,
    command: &crate::protocol::commands::PermitMutation,
    replay: bool,
) -> Result<(), ApiError> {
    use crate::protocol::commands::PermitMutation;
    let (role, claim, operation, registered_key, thread) = match command {
        PermitMutation::BeginHandoff(v) => (
            "handoff",
            &v.identity.claim,
            &v.operation,
            &v.identity.compound,
            None,
        ),
        PermitMutation::CreateThread(v) => ("create", &v.claim, &v.operation, &v.operation, None),
        PermitMutation::Invite(v) => (
            "invite",
            &v.claim,
            &v.operation,
            &v.operation,
            Some(&v.thread),
        ),
        PermitMutation::SendMessage(v) => (
            "send",
            &v.claim,
            &v.operation,
            &v.operation,
            Some(&v.thread),
        ),
        _ => {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "not a selected linked phase",
            ));
        }
    };
    if canonical.instance != claim.instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "linked selected instance differs",
        ));
    }
    let actor = format!("seat:{}", claim.seat.as_str());
    let Some((identity, status)) =
        canonical_parent(db, canonical, &actor, registered_key.as_str(), role)?
    else {
        let registered: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM bootstrap_child_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key IN (?3,?4))", params![claim.instance,actor,registered_key.as_str(),operation.as_str()], |r|r.get(0)).map_err(store_error)?;
        return if registered {
            Err(api_error(
                ErrorCode::InstanceMismatch,
                "linked selected namespace or phase differs",
            ))
        } else {
            Ok(())
        };
    };
    if crate::cli::journal::classify_original_claim(&identity.scope, &identity.claim)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid linked original scope"))?
        != crate::cli::journal::OriginalActor::Agent
        || identity.claim != *claim
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "linked phase differs from original agent",
        ));
    }
    let attachment = status
        .attachment
        .as_ref()
        .ok_or_else(|| conflict("linked phase has no attachment"))?;
    let child = handoff::current(db, &attachment.handoff)?;
    if thread.is_some() && child.as_ref().and_then(|c| c.thread.as_ref()) != thread {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "linked phase thread differs",
        ));
    }
    let exact = match command {
        PermitMutation::BeginHandoff(v) => {
            v.operation == identity.payload.handoff.keys.begin
                && same(&attachment.handoff, &v.identity)?
        }
        PermitMutation::CreateThread(v) => {
            matches!(&identity.payload.handoff.channel,HandoffChannel::New{name,topic,goal} if name==&v.name && topic==&v.topic && goal==&v.goal)
        }
        PermitMutation::Invite(v) => {
            v.seat == attachment.resolved_seat && v.deadline_millis.is_none()
        }
        PermitMutation::SendMessage(v) => {
            v.body == identity.payload.handoff.body
                && v.invited_recipients == [attachment.resolved_seat.clone()]
                && v.deadline_millis.is_none()
                && !v.relays_user
                && v.user_intent.is_none()
        }
        _ => false,
    };
    if !exact {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "linked frozen phase payload differs",
        ));
    }
    if status.state == BootstrapState::Completed {
        return if replay
            && child
                .as_ref()
                .is_some_and(|c| c.state == HandoffState::Completed)
        {
            Ok(())
        } else {
            Err(conflict("completed linked phase cannot authorize new work"))
        };
    }
    // Begin can establish the exact attached child; later phases require it.
    if status.state != BootstrapState::Attached
        || child
            .as_ref()
            .is_some_and(|c| c.state != HandoffState::Live)
        || (child.is_none() && !matches!(command, PermitMutation::BeginHandoff(_)))
    {
        return Err(conflict("linked phase is not exact live attachment"));
    }
    validate_live(db, canonical, &identity)?;
    handoff::validate_live(
        db,
        &attachment.handoff,
        child.as_ref().and_then(|c| c.thread.as_ref()),
    )?;
    seats::eligible_delivery_recipient(db, &canonical.instance, &attachment.resolved_seat)
}

pub(crate) fn guard_unscoped_create(
    db: &Connection,
    instance: &str,
    actor_scope: &str,
    key: &str,
) -> Result<(), ApiError> {
    let linked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM bootstrap_child_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND role='create')",params![instance,actor_scope,key],|r|r.get(0)).map_err(store_error)?;
    if linked {
        return Err(conflict(
            "bootstrap CREATE requires daemon-selected namespace",
        ));
    }
    Ok(())
}
pub(crate) fn guard_bare_completion(
    db: &Connection,
    child: &HandoffIdentity,
) -> Result<(), ApiError> {
    guard_child_begin(db, child)
}

/// Bootstrap-only deciding preallocation/replay validation. The sealed guard
/// carries workspace/tab from the SAME published fresh pane response.
pub(crate) fn validate_bootstrap_resolution(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    request: &ResolveBootstrapSeat,
    guard: &BootstrapAttachmentGuard,
) -> Result<Option<crate::protocol::ids::SeatId>, ApiError> {
    request
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let status =
        current(tx, canonical, &request.identity)?.ok_or_else(|| conflict("bootstrap missing"))?;
    if status.attempt != request.expected_attempt
        || !matches!(
            status.state,
            BootstrapState::Created | BootstrapState::Attached
        )
    {
        return Err(conflict(
            "bootstrap is not the active confirmed creation attempt",
        ));
    }
    validate_live(tx, canonical, &request.identity)?;
    let created = status.creation.as_ref().ok_or_else(corrupt)?;
    let ordinary = guard.ordinary();
    let proof = ordinary.structural_proof();
    if ordinary.operation() != &request.operation
        || guard.workspace() != &created.workspace
        || guard.tab() != &created.tab
        || proof.target() != &created.root_pane
        || proof.terminal() != &created.terminal
        || proof.host_boot() != &created.host_incarnation
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "bootstrap current pane scope differs before allocation",
        ));
    }
    Ok(status.attachment.map(|a| a.resolved_seat))
}

/// The ordinary resolver already decided this exact key/result. A fresh guard
/// reconfirms the current owner; no allocation, movement or binding is performed.
pub fn attach_pending(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    command: &AttachBootstrapHandoff,
    guard: &BootstrapAttachmentGuard,
) -> Result<BootstrapResult, ApiError> {
    let id = &command.identity;
    let a = &command.attachment;
    id.validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    a.validate(id)
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    if command.operation != id.payload.attach_key {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap attach operation differs",
        ));
    }
    let status = current(tx, canonical, id)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "bootstrap missing"))?;
    if let Some(saved) = &status.attachment {
        if !same(saved, a)? {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "bootstrap attachment differs",
            ));
        }
        if status.state == BootstrapState::Completed {
            return Ok(status);
        }
    }
    if !matches!(
        status.state,
        BootstrapState::Created | BootstrapState::Attached
    ) || status.attempt != a.attempt
        || status
            .creation
            .as_ref()
            .is_none_or(|c| !same(c, &a.created).unwrap_or(false))
    {
        return Err(conflict("bootstrap is not exact created attempt"));
    }
    validate_live(tx, canonical, id)?;
    let ordinary = guard.ordinary();
    let proof = ordinary.structural_proof();
    if guard.workspace() != &a.created.workspace
        || guard.tab() != &a.created.tab
        || ordinary.operation() != &a.resolve_operation
        || proof.target() != &a.created.root_pane
        || proof.terminal() != &a.created.terminal
        || proof.host_boot() != &a.created.host_incarnation
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "bootstrap current pane scope differs",
        ));
    }
    if crate::store::seats::validate_bootstrap_creation(tx, &canonical.instance, ordinary)?.as_ref()
        != Some(&a.resolved_seat)
    {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "bootstrap recipient is no longer exact owner",
        ));
    }
    let stored:Option<(Vec<u8>,String)>=tx.query_row("SELECT digest,substr(result_json,1,?3) FROM operations WHERE actor_scope=?1 AND operation_key=?2",params![format!("service-allocation:{}",canonical.instance),a.resolve_operation.as_str(),(super::MAX_ATTACHMENT_BYTES+1) as u32],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
    let (digest, json) = stored.ok_or_else(|| conflict("bootstrap resolution missing"))?;
    let expected = crate::store::schema::canonical_digest(&(
        "resolve_seat",
        canonical.instance.as_str(),
        &a.created.root_pane,
    ))?;
    if digest != expected {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap resolution digest differs",
        ));
    }
    let result: CommandResult = decode(json.as_bytes(), super::MAX_ATTACHMENT_BYTES)?;
    if result != CommandResult::SeatResolved(a.resolved_seat.clone()) {
        return Err(conflict("bootstrap resolution result differs"));
    }
    if status.attachment.is_some() {
        return Ok(status);
    }
    let bytes = serde_json::to_vec(a).map_err(|_| corrupt())?;
    if bytes.len() > super::MAX_ATTACHMENT_BYTES {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "oversized bootstrap attachment",
        ));
    }
    atomic(tx, || {
        let parent = parent_id(tx, canonical, id)?;
        tx.execute(
            "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(?1,?2,?3)",
            params![parent, a.attempt.get(), bytes],
        )
        .map_err(store_error)?;
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='attached' WHERE id=?1",
            [parent],
        )
        .map_err(store_error)?;
        current(tx, canonical, id)?.ok_or_else(corrupt)
    })
}

pub(crate) fn validate_create_command(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    command: &crate::protocol::commands::CreateThread,
) -> Result<(), ApiError> {
    if canonical.instance != command.claim.instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "bootstrap CREATE namespace differs",
        ));
    }
    let actor_scope = format!("seat:{}", command.claim.seat.as_str());
    let Some((id, _)) = canonical_parent(
        tx,
        canonical,
        &actor_scope,
        command.operation.as_str(),
        "create",
    )?
    else {
        return guard_unscoped_create(
            tx,
            &canonical.instance,
            &actor_scope,
            command.operation.as_str(),
        );
    };
    if command.claim != id.claim
        || !matches!(&id.payload.handoff.channel,HandoffChannel::New{name,topic,goal} if name==&command.name && topic==&command.topic && goal==&command.goal)
    {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap frozen CREATE payload differs",
        ));
    }
    Ok(())
}

/// CREATE's existing scoped handoff seam invokes this within the same deciding
/// transaction. The link changes only protection, never frozen child identity.
pub fn attach_created(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    instance: &str,
    actor_scope: &str,
    key: &str,
    thread: &ThreadId,
) -> Result<(), ApiError> {
    if canonical.instance != instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "bootstrap CREATE namespace differs",
        ));
    }
    let Some((id, p)) = canonical_parent(tx, canonical, actor_scope, key, "create")? else {
        guard_unscoped_create(tx, instance, actor_scope, key)?;
        return handoff::attach_created_legacy(tx, instance, actor_scope, key, thread);
    };
    let ns = canonical;
    let a = p
        .attachment
        .as_ref()
        .ok_or_else(|| conflict("bootstrap create has no exact child attachment"))?;
    if p.state != BootstrapState::Attached
        || id.payload.handoff.channel.thread().is_some()
        || a.handoff.thread.is_some()
    {
        return Err(conflict("bootstrap create is terminal or not new-thread"));
    }
    let child = handoff::current(tx, &a.handoff)?
        .ok_or_else(|| conflict("bootstrap create has no live child"))?;
    if child.state != HandoffState::Live || child.thread.as_ref().is_some_and(|old| old != thread) {
        return Err(conflict("bootstrap create child contradicts attachment"));
    }
    let belongs: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
            params![thread.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !belongs {
        return Err(conflict(
            "bootstrap created thread belongs to another instance",
        ));
    }
    let prior: Option<String> = tx
        .query_row(
            "SELECT thread_id FROM bootstrap_handoffs WHERE id=?1",
            [parent_id(tx, ns, &id)?],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if prior.as_deref().is_some_and(|old| old != thread.as_str()) {
        return Err(conflict("bootstrap create contradicts attached thread"));
    }
    atomic(tx, || {
        tx.execute(
            "UPDATE bootstrap_handoffs SET thread_id=?2 WHERE id=?1",
            params![parent_id(tx, ns, &id)?, thread.as_str()],
        )
        .map_err(store_error)?;
        handoff::attach_created_legacy(tx, instance, actor_scope, key, thread)
    })
}

fn validate_report(command: &CompleteLinkedBootstrap) -> Result<(), ApiError> {
    let id = &command.identity;
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(
            id.payload
                .handoff
                .namespace
                .state_dir
                .to_string_lossy()
                .into_owned(),
        ),
        host: Some(
            id.payload
                .handoff
                .namespace
                .host_endpoint
                .to_string_lossy()
                .into_owned(),
        ),
    };
    let mut caller = id.payload.launch.argv.clone();
    caller.push(crate::cli::handoff::bootstrap(
        &command.retained.thread,
        &context,
        &id.claim.instance,
    ));
    // Current production SetupHookInspector::installed supplies no owned args.
    let expected =
        crate::harness::launch::compose_native_argv(id.payload.launch.harness, caller, Vec::new())?;
    if command.retained.report.get("argv") != Some(&serde_json::json!(expected)) {
        return Err(conflict("bootstrap retained launch arguments differ"));
    }
    Ok(())
}

/// Stores exact validated historical output and the unchanged legacy completion
/// operation identity, then completes both fences in one rollback-safe decision.
pub fn complete_linked_pending(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    command: &CompleteLinkedBootstrap,
    now: UtcMillis,
) -> Result<CompletedBootstrapResult, ApiError> {
    command
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let id = &command.identity;
    let status = current(tx, canonical, id)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "bootstrap missing"))?;
    if status
        .attachment
        .as_ref()
        .is_none_or(|a| !same(a, &command.attachment).unwrap_or(false))
    {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap retained attachment differs",
        ));
    }
    if let Some(done) = status.completed {
        if handoff::current(tx, &command.attachment.handoff)?.as_ref() != Some(&done.legacy_result)
        {
            return Err(corrupt());
        }
        if !same(&done.retained, &command.retained)? {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "bootstrap retained report differs",
            ));
        }
        verify_operation(
            tx,
            &command.legacy_completion.operation,
            &scope(id),
            crate::store::control::cooperative_payload_hash(
                "complete_handoff",
                &command.legacy_completion,
            )?,
            &CommandResult::Handoff(done.legacy_result.clone()),
        )?;
        verify_operation(
            tx,
            &command.operation,
            &scope(id),
            crate::store::control::cooperative_payload_hash("complete_linked_bootstrap", command)?,
            &CommandResult::LinkedBootstrapCompleted(done.clone()),
        )?;
        return Ok(*done);
    }
    validate_report(command)?;
    if status.state != BootstrapState::Attached {
        return Err(conflict(
            "bootstrap completion requires attached live parent",
        ));
    }
    validate_live(tx, canonical, id)?;
    let child = handoff::current(tx, &command.attachment.handoff)?
        .ok_or_else(|| conflict("bootstrap linked child missing"))?;
    if child.state != HandoffState::Live || child.thread.as_ref() != Some(&command.retained.thread)
    {
        return Err(conflict("bootstrap linked child is not exact live thread"));
    }
    let attached: Option<String> = tx
        .query_row(
            "SELECT thread_id FROM bootstrap_handoffs WHERE id=?1",
            [parent_id(tx, canonical, id)?],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if attached.as_deref() != Some(command.retained.thread.as_str()) {
        return Err(conflict("bootstrap thread differs from retained report"));
    }
    atomic(tx, || {
        let mut legacy = child.clone();
        legacy.state = HandoffState::Completed;
        let done = CompletedBootstrapResult {
            identity: id.clone(),
            attachment: command.attachment.clone(),
            retained: command.retained.clone(),
            legacy_result: legacy,
        };
        let bytes = serde_json::to_vec(&done).map_err(|_| corrupt())?;
        if bytes.len() > super::MAX_COMPLETED_BYTES {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "oversized bootstrap completed result",
            ));
        }
        tx.execute(
            "INSERT INTO bootstrap_reports(parent_id,completed_json) VALUES(?1,?2)",
            params![parent_id(tx, canonical, id)?, bytes],
        )
        .map_err(store_error)?;
        if handoff::complete_linked_child(tx, &command.attachment.handoff, now)?
            != done.legacy_result
        {
            return Err(corrupt());
        }
        retain_operation(
            tx,
            &command.legacy_completion.operation,
            &scope(id),
            crate::store::control::cooperative_payload_hash(
                "complete_handoff",
                &command.legacy_completion,
            )?,
            &CommandResult::Handoff(done.legacy_result.clone()),
            now,
        )?;
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='completed',terminal_at=?2 WHERE id=?1",
            params![parent_id(tx, canonical, id)?, now.0],
        )
        .map_err(store_error)?;
        retain_operation(
            tx,
            &command.operation,
            &scope(id),
            crate::store::control::cooperative_payload_hash("complete_linked_bootstrap", command)?,
            &CommandResult::LinkedBootstrapCompleted(Box::new(done.clone())),
            now,
        )?;
        Ok(done)
    })
}
fn verify_operation(
    tx: &Transaction<'_>,
    key: &crate::protocol::ids::OperationId,
    scope: &str,
    digest: [u8; 32],
    result: &CommandResult,
) -> Result<(), ApiError> {
    let old: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT digest,substr(result_json,1,?3) FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![scope, key.as_str(), (super::MAX_COMPLETED_BYTES+257) as u32],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((d, json)) = old else {
        return Err(corrupt());
    };
    if d != digest {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap completion operation reused",
        ));
    }
    let decoded: CommandResult = decode(json.as_bytes(), super::MAX_COMPLETED_BYTES + 256)?;
    if !same(&decoded, result)? {
        return Err(corrupt());
    }
    Ok(())
}
fn retain_operation(
    tx: &Transaction<'_>,
    key: &crate::protocol::ids::OperationId,
    scope: &str,
    digest: [u8; 32],
    result: &CommandResult,
    now: UtcMillis,
) -> Result<(), ApiError> {
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE actor_scope=?1 AND operation_key=?2)",
            params![scope, key.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        return verify_operation(tx, key, scope, digest, result);
    }
    tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES(?1,?2,?3,?4,?5)",params![scope,key.as_str(),digest.as_slice(),serde_json::to_string(result).map_err(|_|corrupt())?,now.0]).map_err(store_error)?;
    Ok(())
}
