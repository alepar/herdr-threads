//! Canonical V2 parent persistence. No host calls or client journal authority.
use crate::{
    ports::BootstrapAttachmentGuard,
    protocol::{
        authority::MutationPermit,
        commands::PermitMutation,
        handoff::*,
        ids::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        connection::{StoreContext, api_error, store_error},
        control, handoff, schema, seats,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};
const LIMIT: usize = 131072;
fn conflict() -> ApiError {
    api_error(
        ErrorCode::Conflict,
        "automatic bootstrap attempt or state changed",
    )
}
fn mismatch() -> ApiError {
    api_error(
        ErrorCode::OperationPayloadMismatch,
        "automatic bootstrap immutable original differs",
    )
}
fn corrupt() -> ApiError {
    api_error(
        ErrorCode::StoreCorrupt,
        "invalid retained automatic bootstrap",
    )
}
fn actor(id: &AutomaticBootstrapIdentity) -> String {
    format!("seat:{}", id.claim.seat.as_str())
}
fn encode<T: Serialize>(v: &T, limit: usize) -> Result<Vec<u8>, ApiError> {
    let b = serde_json::to_vec(v).map_err(|_| corrupt())?;
    if b.len() > limit {
        return Err(ApiError::invalid_request(
            "oversized automatic bootstrap record",
        ));
    }
    Ok(b)
}
fn decode<T: DeserializeOwned + Serialize>(b: &[u8], limit: usize) -> Result<T, ApiError> {
    if b.len() > limit {
        return Err(corrupt());
    }
    let value: serde_json::Value = serde_json::from_slice(b).map_err(|_| corrupt())?;
    let decoded: T = serde_json::from_value(value.clone()).map_err(|_| corrupt())?;
    if value != serde_json::to_value(&decoded).map_err(|_| corrupt())? {
        return Err(corrupt());
    }
    Ok(decoded)
}
fn same<T: Serialize>(a: &T, b: &T) -> Result<bool, ApiError> {
    Ok(encode(a, 2097152)? == encode(b, 2097152)?)
}
pub(crate) fn namespace(
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
) -> Result<(), ApiError> {
    id.validate().map_err(ApiError::invalid_request)?;
    let frozen = &id.plan.payload.handoff.namespace;
    if ns.instance != frozen.instance
        || ns.state_dir.as_os_str() != frozen.state_dir.as_os_str()
        || ns.host_endpoint.as_os_str() != frozen.host_endpoint.as_os_str()
    {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "automatic bootstrap namespace differs",
        ));
    }
    Ok(())
}
fn parent(db: &Connection, id: &AutomaticBootstrapIdentity) -> Result<i64, ApiError> {
    db.query_row("SELECT id FROM automatic_bootstrap_parents WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",params![id.claim.instance,actor(id),id.plan.payload.keys.compound.as_str()],|r|r.get(0)).optional().map_err(store_error)?.ok_or_else(conflict)
}
fn keys(id: &AutomaticBootstrapIdentity) -> Vec<(&'static str, &OperationId)> {
    let p = &id.plan.payload;
    let k = &p.keys;
    let c = &p.handoff.keys;
    vec![
        ("compound", &k.compound),
        ("begin", &k.begin),
        ("resolve", &k.resolve),
        ("attach", &k.attach),
        ("complete", &k.complete),
        ("child", &c.compound),
        ("child_begin", &c.begin),
        ("ensure", &c.ensure),
        ("send", &c.send),
        ("child_complete", &c.complete),
    ]
}
pub(crate) fn current(
    db: &Connection,
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
) -> Result<Option<AutomaticBootstrapResult>, ApiError> {
    namespace(ns, id)?;
    type Row = (
        i64,
        Vec<u8>,
        String,
        u32,
        i64,
        Option<String>,
        String,
        String,
    );
    let row:Option<Row>=db.query_row("SELECT id,substr(original_json,1,524289),state,current_attempt,administrative_revision,thread_id,state_dir,host_endpoint FROM automatic_bootstrap_parents WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3",params![id.claim.instance,actor(id),id.plan.payload.keys.compound.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(store_error)?;
    let Some((parent, bytes, state, attempt, revision, thread, state_dir, endpoint)) = row else {
        return Ok(None);
    };
    let revision = u64::try_from(revision).map_err(|_| corrupt())?;
    let saved: AutomaticBootstrapIdentity = decode(&bytes, 524288)?;
    saved.validate().map_err(|_| corrupt())?;
    if encode(&saved, 524288)? != encode(id, 524288)? {
        return Err(mismatch());
    }
    if Some(state_dir.as_str()) != ns.state_dir.to_str()
        || Some(endpoint.as_str()) != ns.host_endpoint.to_str()
    {
        return Err(corrupt());
    }
    let attempt = BootstrapAttempt::new(attempt).map_err(|_| corrupt())?;
    let (attempt_state,creation):(String,Option<Vec<u8>>)=db.query_row("SELECT state,substr(creation_json,1,131073) FROM automatic_bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",params![parent,attempt.get()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
    let creation: Option<crate::ports::CreatedTab> =
        creation.as_deref().map(|b| decode(b, LIMIT)).transpose()?;
    if let Some(c) = &creation {
        creation_scope(ns, id, c)?;
    }
    let attachment:Option<Vec<u8>>=db.query_row("SELECT substr(attachment_json,1,524289) FROM automatic_bootstrap_attachments WHERE parent_id=?1",[parent],|r|r.get(0)).optional().map_err(store_error)?;
    let attachment: Option<AutomaticBootstrapAttachment> = attachment
        .as_deref()
        .map(|b| decode(b, 524288))
        .transpose()?;
    if let Some(a) = &attachment {
        a.validate(id).map_err(|_| corrupt())?;
        if a.attempt != attempt
            || creation
                .as_ref()
                .is_none_or(|c| !same(c, &a.created).unwrap_or(false))
        {
            return Err(corrupt());
        }
    }
    let report:Option<Vec<u8>>=db.query_row("SELECT substr(report_json,1,2097153) FROM automatic_bootstrap_reports WHERE parent_id=?1",[parent],|r|r.get(0)).optional().map_err(store_error)?;
    let completed: Option<LinkedBootstrapReport> =
        report.as_deref().map(|b| decode(b, 2097152)).transpose()?;
    if let Some(report) = &completed {
        let a = attachment.as_ref().ok_or_else(corrupt)?;
        validate_report(id, a, report)?;
        if thread.as_deref() != Some(report.thread.as_str()) {
            return Err(corrupt());
        }
    }
    for (role, key) in keys(id) {
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_bootstrap_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND parent_id=?4 AND role=?5 AND attempt=0)",params![id.claim.instance,actor(id),key.as_str(),parent,role],|r|r.get(0)).map_err(store_error)?;
        if !valid {
            return Err(corrupt());
        }
    }
    for role in ["reserve", "record", "check", "not_submitted", "unknown"] {
        let key = attempt_key(id, attempt, role)?;
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_bootstrap_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND parent_id=?4 AND role=?5 AND attempt=?6)",params![id.claim.instance,actor(id),key.as_str(),parent,role,attempt.get()],|r|r.get(0)).map_err(store_error)?;
        if !valid {
            return Err(corrupt());
        }
    }
    let recovery_bytes:Option<Vec<u8>>=db.query_row("SELECT substr(result_json,1,2097153) FROM automatic_bootstrap_recoveries WHERE parent_id=?1 ORDER BY revision DESC LIMIT 1",[parent],|r|r.get(0)).optional().map_err(store_error)?;
    let recovery = if let Some(bytes) = recovery_bytes {
        let CommandResult::AutomaticBootstrap(saved) = decode(&bytes, 2097152)? else {
            return Err(corrupt());
        };
        let recovery = saved.recovery.ok_or_else(corrupt)?;
        if !same(&saved.identity, id)?
            || recovery.administrative_revision != revision
            || recovery.operator_provenance
                != format!("operator:local-user:{}", recovery.operator_uid)
        {
            return Err(corrupt());
        }
        recovery.disposition.validate().map_err(|_| corrupt())?;
        recovery.inspection.validate().map_err(|_| corrupt())?;
        Some(recovery)
    } else {
        if revision != 0 {
            return Err(corrupt());
        }
        None
    };
    let state: BootstrapState =
        serde_json::from_value(serde_json::Value::String(state)).map_err(|_| corrupt())?;
    let attempt_state: BootstrapAttemptState =
        serde_json::from_value(serde_json::Value::String(attempt_state)).map_err(|_| corrupt())?;
    let coherent = match state {
        BootstrapState::Prepared => {
            attempt_state == BootstrapAttemptState::Prepared
                && creation.is_none()
                && attachment.is_none()
                && completed.is_none()
        }
        BootstrapState::PossibleCreation => {
            matches!(
                attempt_state,
                BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
            ) && creation.is_none()
                && attachment.is_none()
                && completed.is_none()
        }
        BootstrapState::Created => {
            creation.is_some() && attachment.is_none() && completed.is_none()
        }
        BootstrapState::Attached => attachment.is_some() && completed.is_none(),
        BootstrapState::Completed => completed.is_some() && attachment.is_some(),
        BootstrapState::Cancelled => completed.is_none(),
    };
    if !coherent {
        return Err(corrupt());
    }
    Ok(Some(AutomaticBootstrapResult {
        identity: id.clone(),
        state,
        attempt,
        attempt_state,
        administrative_revision: revision,
        creation,
        attachment,
        thread: thread
            .map(ThreadId::parse)
            .transpose()
            .map_err(|_| corrupt())?,
        completed,
        recovery,
    }))
}
fn status(
    db: &Connection,
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
) -> Result<AutomaticBootstrapResult, ApiError> {
    current(db, ns, id)?.ok_or_else(conflict)
}
fn live(
    db: &Connection,
    id: &AutomaticBootstrapIdentity,
    status: &AutomaticBootstrapResult,
) -> Result<(), ApiError> {
    if matches!(
        status.state,
        BootstrapState::Cancelled | BootstrapState::Completed
    ) {
        return Err(conflict());
    }
    seats::cooperative_mapping(db, &id.claim, None)?;
    if let Some(thread) = &status.thread {
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM threads t JOIN memberships m ON m.thread_id=t.id WHERE t.id=?1 AND t.instance_id=?2 AND t.archived=0 AND m.seat_id=?3 AND m.state='joined')",params![thread.as_str(),id.claim.instance,id.claim.seat.as_str()],|r|r.get(0)).map_err(store_error)?;
        if !valid {
            return Err(api_error(
                ErrorCode::MembershipRequired,
                "automatic bootstrap requires active joined sender",
            ));
        }
    }
    Ok(())
}
fn register_key(
    tx: &Transaction<'_>,
    id: &AutomaticBootstrapIdentity,
    parent: i64,
    role: &str,
    key: &OperationId,
    attempt: u32,
) -> Result<(), ApiError> {
    let used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE actor_scope=?1 AND operation_key=?2) OR EXISTS(SELECT 1 FROM automatic_handoff_keys WHERE instance_id=?3 AND actor_scope=?1 AND operation_key=?2) OR EXISTS(SELECT 1 FROM bootstrap_child_keys WHERE instance_id=?3 AND actor_scope=?1 AND operation_key=?2) OR EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?3 AND actor_scope=?1 AND (compound=?2 OR create_key=?2 OR invite_key=?2 OR send_key=?2))",params![actor(id),key.as_str(),id.claim.instance],|r|r.get(0)).map_err(store_error)?;
    if used {
        return Err(mismatch());
    }
    tx.execute("INSERT INTO automatic_bootstrap_keys(instance_id,actor_scope,operation_key,parent_id,role,attempt) VALUES(?1,?2,?3,?4,?5,?6)",params![id.claim.instance,actor(id),key.as_str(),parent,role,attempt]).map_err(store_error)?;
    Ok(())
}
fn attempt_key(
    id: &AutomaticBootstrapIdentity,
    attempt: BootstrapAttempt,
    phase: &str,
) -> Result<OperationId, ApiError> {
    if phase == "unknown" {
        return Ok(crate::protocol::handoff::automatic_unknown_key(
            &id.plan.payload.keys.compound,
            attempt,
        ));
    }
    attempt
        .operation(&id.plan.payload.keys.compound, phase)
        .map_err(ApiError::invalid_request)
}
fn insert_attempt(
    tx: &Transaction<'_>,
    id: &AutomaticBootstrapIdentity,
    parent: i64,
    attempt: BootstrapAttempt,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO automatic_bootstrap_attempts(parent_id,attempt,state) VALUES(?1,?2,'prepared')",params![parent,attempt.get()]).map_err(store_error)?;
    for phase in ["reserve", "record", "check", "not_submitted", "unknown"] {
        let key = attempt_key(id, attempt, phase)?;
        register_key(tx, id, parent, phase, &key, attempt.get())?;
    }
    Ok(())
}
fn creation_scope(
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
    c: &crate::ports::CreatedTab,
) -> Result<(), ApiError> {
    c.validate().map_err(ApiError::invalid_request)?;
    if c.workspace != id.plan.payload.workspace
        || c.witness.endpoint.as_os_str() != ns.host_endpoint.as_os_str()
    {
        return Err(mismatch());
    }
    encode(c, LIMIT)?;
    Ok(())
}
fn validate_report(
    id: &AutomaticBootstrapIdentity,
    a: &AutomaticBootstrapAttachment,
    report: &LinkedBootstrapReport,
) -> Result<(), ApiError> {
    AutomaticBootstrapMutation {
        identity: id.clone(),
        action: AutomaticBootstrapAction::Complete {
            attachment: Box::new(a.clone()),
            retained: report.clone(),
        },
    }
    .validate()
    .map_err(ApiError::invalid_request)?;
    if id.plan.payload.handoff.policy.is_some() {
        let full_digest = report.report["full_report_digest"]
            .as_str()
            .ok_or_else(mismatch)?;
        let summary =
            crate::protocol::handoff::automatic_launch_summary(&report.report, full_digest)
                .map_err(ApiError::invalid_request)?;
        if summary != report.report {
            return Err(mismatch());
        }
    }
    let AutomaticOriginal::Handoff(child) = &a.child.original else {
        return Err(mismatch());
    };
    let p = &id.plan.payload;
    let argv = report.report.get("argv");
    let valid = if let Some(template) = &p.startup_input {
        crate::cli::handoff::bootstrap_v1::retained_prompts(
            report.thread.as_str(),
            &child.context,
            &id.claim.instance,
        )
        .into_iter()
        .any(|text| {
            template
                .apply(&p.launch.argv, &text)
                .is_ok_and(|(v, _)| argv == Some(&serde_json::json!(v)))
        })
    } else {
        crate::cli::handoff::bootstrap_v1::retained_argv_matches(
            p.launch.harness,
            &p.launch.argv,
            report.thread.as_str(),
            &child.context,
            &id.claim.instance,
            argv,
        )?
    };
    if !valid {
        return Err(mismatch());
    }
    Ok(())
}
pub(crate) fn validate_parent_phase(
    db: &Connection,
    request: &AutomaticBootstrapMutation,
    replay: bool,
) -> Result<(), ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let ns = &request.identity.plan.payload.handoff.namespace;
    if let Some(s) = current(db, ns, &request.identity)? {
        if !replay {
            live(db, &request.identity, &s)?;
        }
    } else if !matches!(request.action, AutomaticBootstrapAction::Begin) {
        return Err(conflict());
    }
    Ok(())
}
pub(crate) fn guard_raw_resolution(
    db: &Connection,
    instance: &str,
    key: &str,
) -> Result<(), ApiError> {
    let registered:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM automatic_bootstrap_keys WHERE instance_id=?1 AND operation_key=?2 AND role='resolve')",params![instance,key],|r|r.get(0)).map_err(store_error)?;
    if registered {
        return Err(mismatch());
    }
    Ok(())
}
/// Select linkage independently of the supplied original; registered keys never fall back.
pub(crate) fn guard_child(
    db: &Connection,
    request: &AutomaticDeliveryMutation,
    replay: bool,
) -> Result<bool, ApiError> {
    let k = &request.plan().payload.keys;
    let mut query=db.prepare("SELECT DISTINCT p.id,substr(p.original_json,1,524289) FROM automatic_bootstrap_keys k JOIN automatic_bootstrap_parents p ON p.id=k.parent_id WHERE k.instance_id=?1 AND k.actor_scope=?2 AND k.operation_key IN (?3,?4,?5,?6,?7) LIMIT 2").map_err(store_error)?;
    let rows = query
        .query_map(
            params![
                request.claim.instance,
                format!("seat:{}", request.claim.seat.as_str()),
                k.compound.as_str(),
                k.begin.as_str(),
                k.ensure.as_str(),
                k.send.as_str(),
                k.complete.as_str()
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    if rows.is_empty() {
        return Ok(false);
    };
    if rows.len() != 1 {
        return Err(mismatch());
    }
    let id: AutomaticBootstrapIdentity = decode(&rows[0].1, 524288)?;
    let ns = &id.plan.payload.handoff.namespace;
    handoff::automatic::validate_namespace(ns, request)?;
    let s = status(db, ns, &id)?;
    let a = s.attachment.as_ref().ok_or_else(conflict)?;
    if a.child
        .canonical_original_bytes()
        .map_err(ApiError::invalid_request)?
        != request
            .canonical_original_bytes()
            .map_err(ApiError::invalid_request)?
    {
        return Err(mismatch());
    }
    if !matches!(
        s.state,
        BootstrapState::Attached | BootstrapState::Completed
    ) {
        return Err(conflict());
    }
    let completed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?1 AND actor_scope=?2 AND compound=?3 AND state='completed')",params![id.claim.instance,actor(&id),k.compound.as_str()],|r|r.get(0)).map_err(store_error)?;
    if replay
        && (completed
            || matches!(
                request.action,
                AutomaticDeliveryAction::Send(_) | AutomaticDeliveryAction::Ensure(_)
            ))
    {
        return Ok(true);
    }
    live(db, &id, &s)?;
    Ok(true)
}
pub(crate) fn attach_thread(
    tx: &Transaction<'_>,
    request: &AutomaticDeliveryMutation,
    thread: &ThreadId,
) -> Result<(), ApiError> {
    if !guard_child(tx, request, false)? {
        return Ok(());
    }
    let p:i64=tx.query_row("SELECT parent_id FROM automatic_bootstrap_keys WHERE instance_id=?1 AND actor_scope=?2 AND operation_key=?3 AND role='ensure'",params![request.claim.instance,format!("seat:{}",request.claim.seat.as_str()),request.plan().payload.keys.ensure.as_str()],|r|r.get(0)).map_err(store_error)?;
    tx.execute(
        "UPDATE automatic_bootstrap_parents SET thread_id=?2 WHERE id=?1",
        params![p, thread.as_str()],
    )
    .map_err(store_error)?;
    Ok(())
}
fn validate_guard(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
    s: &AutomaticBootstrapResult,
    guard: &BootstrapAttachmentGuard,
) -> Result<Option<SeatId>, ApiError> {
    let created = s.creation.as_ref().ok_or_else(conflict)?;
    let ordinary = guard.ordinary();
    let proof = ordinary.structural_proof();
    if ordinary.operation() != &id.plan.payload.keys.resolve
        || guard.workspace() != &created.workspace
        || guard.tab() != &created.tab
        || proof.target() != &created.root_pane
        || proof.terminal() != &created.terminal
        || proof.host_boot() != &created.host_incarnation
        || guard.witness().endpoint.as_os_str() != created.witness.endpoint.as_os_str()
        || guard.witness() != &created.witness
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "automatic bootstrap current creation scope differs",
        ));
    }
    let expected = guard
        .ordinary()
        .admission()
        .lifecycle_revision
        .checked_add(1)
        .ok_or_else(conflict)?;
    let lifecycle: i64 = tx
        .query_row(
            "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
            [&ns.instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if u64::try_from(lifecycle).ok() != Some(expected) {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "automatic bootstrap observation predates lifecycle",
        ));
    }
    let owner = seats::validate_bootstrap_creation(tx, &ns.instance, ordinary)?;
    if s.attachment
        .as_ref()
        .is_some_and(|a| owner.as_ref() != Some(&a.resolved_seat))
    {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "automatic attachment owner changed",
        ));
    }
    Ok(owner)
}
fn resolution_record(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    id: &AutomaticBootstrapIdentity,
    created: &crate::ports::CreatedTab,
) -> Result<Option<CommandResult>, ApiError> {
    let row:Option<(Vec<u8>,String)>=tx.query_row("SELECT digest,substr(result_json,1,131073) FROM operations WHERE actor_scope=?1 AND operation_key=?2",params![format!("service-allocation:{}",ns.instance),id.plan.payload.keys.resolve.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
    let expected =
        schema::canonical_digest(&("resolve_seat", ns.instance.as_str(), &created.root_pane))?;
    row.map(|(digest, json)| {
        if digest != expected {
            return Err(mismatch());
        }
        decode(json.as_bytes(), LIMIT)
    })
    .transpose()
}
pub(crate) fn query(
    db: &Connection,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
) -> Result<CommandResult, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let s = status(db, ns, &request.identity)?;
    match request.action {
        AutomaticBootstrapAction::Status => Ok(CommandResult::AutomaticBootstrap(Box::new(s))),
        AutomaticBootstrapAction::CheckStart => Err(api_error(
            ErrorCode::StaleHostObservation,
            "automatic possible-start needs fresh original creation observation",
        )),
        _ => Err(ApiError::invalid_request("not automatic bootstrap query")),
    }
}
pub(crate) fn check_start(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    guard: &BootstrapAttachmentGuard,
) -> Result<CommandResult, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    if !matches!(request.action, AutomaticBootstrapAction::CheckStart) {
        return Err(ApiError::invalid_request("not possible-start check"));
    }
    let s = status(tx, ns, &request.identity)?;
    validate_guard(tx, ns, &request.identity, &s, guard)?;
    live(tx, &request.identity, &s)?;
    let a = s.attachment.as_ref().ok_or_else(conflict)?;
    let thread = s.thread.as_ref().ok_or_else(conflict)?;
    let child = a.child.send(thread.clone());
    let AutomaticDeliveryAction::Send(send) = &child.action else {
        unreachable!()
    };
    control::validate_pair_send(tx, send)?;
    handoff::automatic::validate_phase(tx, &child, false)?;
    if !handoff::automatic::published_send(tx, &a.child, thread)? {
        return Err(conflict());
    }
    Ok(CommandResult::AutomaticBootstrap(Box::new(s)))
}
fn apply(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    guard: Option<&BootstrapAttachmentGuard>,
    at: crate::store::connection::DecisionInstant,
) -> Result<CommandResult, ApiError> {
    let id = &request.identity;
    let original = current(tx, ns, id)?;
    if matches!(request.action, AutomaticBootstrapAction::Begin) {
        if let Some(s) = original {
            if !matches!(
                s.state,
                BootstrapState::Completed | BootstrapState::Cancelled
            ) {
                live(tx, id, &s)?;
            }
            return Ok(CommandResult::AutomaticBootstrap(Box::new(s)));
        }
        id.validate_admission().map_err(ApiError::invalid_request)?;
        seats::cooperative_mapping(tx, &id.claim, None)?;
        tx.execute("INSERT INTO automatic_bootstrap_parents(instance_id,actor_scope,compound,state_dir,host_endpoint,original_json,state,current_attempt) VALUES(?1,?2,?3,?4,?5,?6,'prepared',1)",params![ns.instance,actor(id),id.plan.payload.keys.compound.as_str(),ns.state_dir.to_str(),ns.host_endpoint.to_str(),encode(id,524288)?]).map_err(store_error)?;
        let p = tx.last_insert_rowid();
        for (role, key) in keys(id) {
            register_key(tx, id, p, role, key, 0)?;
        }
        insert_attempt(tx, id, p, BootstrapAttempt::first())?;
        return Ok(CommandResult::AutomaticBootstrap(Box::new(status(
            tx, ns, id,
        )?)));
    }
    let s = original.ok_or_else(conflict)?;
    let p = parent(tx, id)?;
    match &request.action {
        AutomaticBootstrapAction::Reserve { attempt } => {
            live(tx, id, &s)?;
            if *attempt != s.attempt {
                return Err(conflict());
            }
            let authorization = if s.state == BootstrapState::Prepared {
                tx.execute("UPDATE automatic_bootstrap_attempts SET state='possible_creation',reserved_revision=?3 WHERE parent_id=?1 AND attempt=?2 AND state='prepared'",params![p,attempt.get(),i64::try_from(s.administrative_revision).map_err(|_|conflict())?]).map_err(store_error)?;
                tx.execute(
                    "UPDATE automatic_bootstrap_parents SET state='possible_creation' WHERE id=?1",
                    [p],
                )
                .map_err(store_error)?;
                Some(BootstrapSubmissionAuthorization {
                    compound: id.plan.payload.keys.compound.clone(),
                    attempt: *attempt,
                    reservation: request.operation().map_err(ApiError::invalid_request)?,
                    administrative_revision: s.administrative_revision,
                })
            } else {
                None
            };
            Ok(CommandResult::AutomaticBootstrapReserved(Box::new(
                AutomaticBootstrapReserved {
                    status: status(tx, ns, id)?,
                    authorization,
                },
            )))
        }
        AutomaticBootstrapAction::CheckSubmission {
            attempt,
            administrative_revision,
        } => {
            live(tx, id, &s)?;
            let revision:Option<i64>=tx.query_row("SELECT reserved_revision FROM automatic_bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get()],|r|r.get(0)).optional().map_err(store_error)?.flatten();
            if *attempt != s.attempt
                || s.attempt_state != BootstrapAttemptState::PossibleCreation
                || revision.and_then(|v| u64::try_from(v).ok()) != Some(s.administrative_revision)
                || *administrative_revision != s.administrative_revision
            {
                return Err(conflict());
            }
            Ok(CommandResult::BootstrapSubmissionChecked(
                BootstrapSubmissionChecked {
                    compound: id.plan.payload.keys.compound.clone(),
                    attempt: *attempt,
                    administrative_revision: s.administrative_revision,
                },
            ))
        }
        AutomaticBootstrapAction::RecordCreated { attempt, evidence } => {
            creation_scope(ns, id, evidence)?;
            if *attempt != s.attempt {
                return Err(conflict());
            }
            if let Some(saved) = &s.creation {
                if !same(saved, evidence)? {
                    return Err(mismatch());
                }
                return Ok(CommandResult::AutomaticBootstrap(Box::new(s)));
            }
            live(tx, id, &s)?;
            if !matches!(
                s.attempt_state,
                BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
            ) {
                return Err(conflict());
            }
            tx.execute("UPDATE automatic_bootstrap_attempts SET state='created',creation_json=?3 WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get(),encode(evidence,LIMIT)?]).map_err(store_error)?;
            tx.execute(
                "UPDATE automatic_bootstrap_parents SET state='created' WHERE id=?1",
                [p],
            )
            .map_err(store_error)?;
            Ok(CommandResult::AutomaticBootstrap(Box::new(status(
                tx, ns, id,
            )?)))
        }
        AutomaticBootstrapAction::RecordOutcomeUnknown { attempt } => {
            live(tx, id, &s)?;
            if *attempt != s.attempt
                || !matches!(
                    s.attempt_state,
                    BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
                )
            {
                return Err(conflict());
            }
            tx.execute("UPDATE automatic_bootstrap_attempts SET state='outcome_unknown' WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get()]).map_err(store_error)?;
            Ok(CommandResult::AutomaticBootstrap(Box::new(status(
                tx, ns, id,
            )?)))
        }
        AutomaticBootstrapAction::RecordNotSubmitted { attempt } => {
            live(tx, id, &s)?;
            if *attempt != s.attempt {
                let prior:Option<String>=tx.query_row("SELECT state FROM automatic_bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get()],|r|r.get(0)).optional().map_err(store_error)?;
                return if prior.as_deref() == Some("not_submitted") {
                    Ok(CommandResult::AutomaticBootstrap(Box::new(s)))
                } else {
                    Err(conflict())
                };
            }
            if s.attempt_state != BootstrapAttemptState::PossibleCreation {
                return Err(conflict());
            }
            let next = BootstrapAttempt::new(attempt.get().checked_add(1).ok_or_else(conflict)?)
                .map_err(ApiError::invalid_request)?;
            tx.execute("UPDATE automatic_bootstrap_attempts SET state='not_submitted' WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get()]).map_err(store_error)?;
            insert_attempt(tx, id, p, next)?;
            tx.execute("UPDATE automatic_bootstrap_parents SET state='prepared',current_attempt=?2 WHERE id=?1",params![p,next.get()]).map_err(store_error)?;
            Ok(CommandResult::AutomaticBootstrap(Box::new(status(
                tx, ns, id,
            )?)))
        }
        AutomaticBootstrapAction::Resolve { attempt } => {
            live(tx, id, &s)?;
            if *attempt != s.attempt
                || !matches!(s.state, BootstrapState::Created | BootstrapState::Attached)
            {
                return Err(conflict());
            }
            let guard = guard.ok_or_else(|| {
                api_error(
                    ErrorCode::StaleHostObservation,
                    "automatic resolution needs fresh guard",
                )
            })?;
            let owner = validate_guard(tx, ns, id, &s, guard)?;
            let created = s.creation.as_ref().ok_or_else(conflict)?;
            if let Some(result) = resolution_record(tx, ns, id, created)? {
                return match result {
                    CommandResult::SeatResolved(ref seat) if owner.as_ref() == Some(seat) => {
                        Ok(result)
                    }
                    _ => Err(mismatch()),
                };
            }
            let (result, allocated) = seats::ordinary_resolution_apply(
                tx,
                at,
                &ns.instance,
                &created.root_pane,
                guard.ordinary().structural_proof(),
            )?;
            let CommandResult::SeatResolved(seat) = &result else {
                return Err(corrupt());
            };
            if seats::current_resolution_owner(tx, &ns.instance, guard.ordinary())?.as_ref()
                != Some(seat)
            {
                return Err(conflict());
            }
            let lifecycle: i64 = tx
                .query_row(
                    "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
                    [&ns.instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if u64::try_from(lifecycle).ok()
                != guard
                    .ordinary()
                    .admission()
                    .lifecycle_revision
                    .checked_add(1 + u64::from(allocated))
            {
                return Err(conflict());
            }
            let digest = schema::canonical_digest(&(
                "resolve_seat",
                ns.instance.as_str(),
                &created.root_pane,
            ))?;
            tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES(?1,?2,?3,?4,?5)",params![format!("service-allocation:{}",ns.instance),id.plan.payload.keys.resolve.as_str(),digest.as_slice(),serde_json::to_string(&result).map_err(|_|corrupt())?,at.utc.0]).map_err(store_error)?;
            Ok(result)
        }
        AutomaticBootstrapAction::Attach(a) => {
            a.validate(id).map_err(ApiError::invalid_request)?;
            if s.attachment
                .as_ref()
                .is_some_and(|saved| !same(saved, a.as_ref()).unwrap_or(false))
            {
                return Err(mismatch());
            }
            if s.state == BootstrapState::Completed {
                return Ok(CommandResult::AutomaticBootstrap(Box::new(s)));
            }
            live(tx, id, &s)?;
            if !matches!(s.state, BootstrapState::Created | BootstrapState::Attached)
                || s.attempt != a.attempt
                || s.creation
                    .as_ref()
                    .is_none_or(|c| !same(c, &a.created).unwrap_or(false))
            {
                return Err(conflict());
            }
            let guard = guard.ok_or_else(|| {
                api_error(
                    ErrorCode::StaleHostObservation,
                    "automatic attachment needs fresh guard",
                )
            })?;
            if validate_guard(tx, ns, id, &s, guard)?.as_ref() != Some(&a.resolved_seat) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "automatic attachment recipient differs from current owner",
                ));
            }
            if resolution_record(tx, ns, id, &a.created)?
                != Some(CommandResult::SeatResolved(a.resolved_seat.clone()))
            {
                return Err(mismatch());
            }
            if s.attachment.is_none() {
                tx.execute("INSERT INTO automatic_bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(?1,?2,?3)",params![p,a.attempt.get(),encode(a,524288)?]).map_err(store_error)?;
                tx.execute(
                    "UPDATE automatic_bootstrap_parents SET state='attached' WHERE id=?1",
                    [p],
                )
                .map_err(store_error)?;
            }
            Ok(CommandResult::AutomaticBootstrap(Box::new(status(
                tx, ns, id,
            )?)))
        }
        AutomaticBootstrapAction::Complete { .. } => complete(tx, ns, request, at.utc),
        _ => Err(ApiError::invalid_request(
            "not automatic bootstrap mutation",
        )),
    }
}
pub(crate) fn mutate(
    context: &StoreContext,
    db: &mut Connection,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    permit: MutationPermit,
    guard: Option<&BootstrapAttachmentGuard>,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    namespace(ns, &request.identity)?;
    let mut permit = permit.with_automatic_bootstrap(request);
    let input = crate::store::cooperative_permit_request(&PermitMutation::AutomaticBootstrap(
        Box::new(request.clone()),
    ))?;
    let (claim, issuance, _) = permit.cooperative_metadata();
    let fresh = std::cell::Cell::new(false);
    schema::execute_budgeted_idempotent_transaction_with_constraints(
        context,
        db,
        budget,
        Some(&issuance),
        &actor(&request.identity),
        input.operation.as_str(),
        input.payload_hash,
        |tx| {
            seats::cooperative_instance(tx, &ns.instance, &claim)?;
            current(tx, ns, &request.identity)?;
            Ok(())
        },
        |_| Ok(()),
        |tx, at| {
            control::decide_accountable(
                tx,
                at,
                &mut permit,
                &claim,
                &claim.seat,
                &input.operation,
                &input.obligation,
                &input.payload_hash,
            )?;
            let r = apply(tx, ns, request, guard, at)?;
            fresh.set(true);
            Ok(r)
        },
        |tx, result| {
            if fresh.get() {
                return Ok(result);
            };
            apply(
                tx,
                ns,
                request,
                guard,
                crate::store::connection::DecisionInstant {
                    utc: context.clock().utc_now(),
                    monotonic: context.clock().monotonic_now(),
                },
            )
        },
    )
}

pub(crate) fn recovery_replay(
    db: &Connection,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    actor: &crate::protocol::authority::OperatorActor,
) -> Result<Option<CommandResult>, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let id = &request.identity;
    status(db, ns, id)?;
    let AutomaticBootstrapAction::Recover {
        attempt,
        inspection,
        disposition,
    } = &request.action
    else {
        return Err(ApiError::invalid_request("not automatic recovery"));
    };
    let operation = request.operation().map_err(ApiError::invalid_request)?;
    let p = parent(db, id)?;
    let bytes:Option<Vec<u8>>=db.query_row("SELECT substr(result_json,1,2097153) FROM automatic_bootstrap_recoveries WHERE parent_id=?1 AND operation=?2",params![p,operation.as_str()],|r|r.get(0)).optional().map_err(store_error)?;
    let Some(bytes) = bytes else { return Ok(None) };
    let result: CommandResult = decode(&bytes, 2097152)?;
    let CommandResult::AutomaticBootstrap(saved) = &result else {
        return Err(corrupt());
    };
    let r = saved.recovery.as_ref().ok_or_else(corrupt)?;
    if !same(&saved.identity, id)?
        || r.operation != operation
        || r.attempt != *attempt
        || &r.inspection != inspection
        || !same(&r.disposition, disposition)?
    {
        return Err(corrupt());
    }
    if r.operator_uid != actor.effective_uid() || r.operator_provenance != actor.audit_label() {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "saved automatic recovery belongs to another operator",
        ));
    }
    Ok(Some(result))
}
pub(crate) fn recover(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    actor: &crate::protocol::authority::OperatorActor,
    guard: Option<&BootstrapAttachmentGuard>,
) -> Result<CommandResult, ApiError> {
    if let Some(saved) = recovery_replay(tx, ns, request, actor)? {
        return Ok(saved);
    }
    let id = &request.identity;
    let mut s = status(tx, ns, id)?;
    let p = parent(tx, id)?;
    let AutomaticBootstrapAction::Recover {
        attempt,
        inspection,
        disposition,
    } = &request.action
    else {
        return Err(ApiError::invalid_request("not automatic recovery"));
    };
    if *attempt != s.attempt
        || matches!(
            s.state,
            BootstrapState::Completed | BootstrapState::Cancelled
        )
        || inspection != &s.recovery_inspection().map_err(ApiError::invalid_request)?
    {
        return Err(conflict());
    }
    let kind = if matches!(
        disposition,
        AutomaticBootstrapRecoveryDisposition::Cancelled { .. }
    ) {
        "cancellation"
    } else {
        "recovery"
    };
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM automatic_bootstrap_recoveries WHERE parent_id=?1 AND attempt=?2 AND decision_kind=?3)",params![p,attempt.get(),kind],|r|r.get(0)).map_err(store_error)?;
    if exists {
        return Err(conflict());
    }
    match disposition {
        AutomaticBootstrapRecoveryDisposition::CreatedPane { evidence, .. } => {
            if !matches!(
                s.attempt_state,
                BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
            ) {
                return Err(conflict());
            }
            creation_scope(ns, id, evidence)?;
            s.creation = Some(evidence.as_ref().clone());
            validate_guard(
                tx,
                ns,
                id,
                &s,
                guard.ok_or_else(|| {
                    ApiError::invalid_request("created recovery needs fresh canonical evidence")
                })?,
            )?;
            s.state = BootstrapState::Created;
            s.attempt_state = BootstrapAttemptState::Created;
        }
        AutomaticBootstrapRecoveryDisposition::NotCreated { .. } => {
            if s.creation.is_some() || s.attachment.is_some() {
                return Err(conflict());
            }
            s.attempt = BootstrapAttempt::new(attempt.get().checked_add(1).ok_or_else(conflict)?)
                .map_err(ApiError::invalid_request)?;
            s.state = BootstrapState::Prepared;
            s.attempt_state = BootstrapAttemptState::Prepared;
        }
        AutomaticBootstrapRecoveryDisposition::Cancelled { attached_child, .. } => {
            if !same(
                &s.attachment.as_ref().map(|a| &a.child),
                &attached_child.as_deref(),
            )? {
                return Err(mismatch());
            }
            let k = &id.plan.payload.handoff.keys;
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?1 AND actor_scope=?2 AND state='live' AND (compound IN (?3,?4,?5,?6,?7) OR create_key IN (?3,?4,?5,?6,?7) OR invite_key IN (?3,?4,?5,?6,?7) OR send_key IN (?3,?4,?5,?6,?7)))",params![ns.instance,actor_scope(id),k.compound.as_str(),k.begin.as_str(),k.ensure.as_str(),k.send.as_str(),k.complete.as_str()],|r|r.get(0)).map_err(store_error)?;
            if live {
                return Err(conflict());
            }
            s.state = BootstrapState::Cancelled;
        }
    }
    s.administrative_revision = s
        .administrative_revision
        .checked_add(1)
        .ok_or_else(conflict)?;
    let operation = request.operation().map_err(ApiError::invalid_request)?;
    s.recovery = Some(AutomaticBootstrapRecoveryResult {
        operation: operation.clone(),
        attempt: *attempt,
        inspection: inspection.clone(),
        disposition: disposition.clone(),
        operator_uid: actor.effective_uid(),
        operator_provenance: actor.audit_label(),
        state: s.state,
        administrative_revision: s.administrative_revision,
    });
    let result = CommandResult::AutomaticBootstrap(Box::new(s.clone()));
    let bytes = encode(&result, 2097152)?;
    register_key(tx, id, p, kind, &operation, attempt.get())?;
    match s.state {
        BootstrapState::Prepared => {
            insert_attempt(tx, id, p, s.attempt)?;
        }
        BootstrapState::Created => {
            tx.execute("UPDATE automatic_bootstrap_attempts SET state='created',creation_json=?3 WHERE parent_id=?1 AND attempt=?2",params![p,attempt.get(),encode(s.creation.as_ref().ok_or_else(corrupt)?,LIMIT)?]).map_err(store_error)?;
        }
        BootstrapState::Cancelled => {}
        _ => return Err(corrupt()),
    }
    let state = match s.state {
        BootstrapState::Prepared => "prepared",
        BootstrapState::Created => "created",
        BootstrapState::Cancelled => "cancelled",
        _ => return Err(corrupt()),
    };
    tx.execute("UPDATE automatic_bootstrap_parents SET state=?2,current_attempt=?3,administrative_revision=?4 WHERE id=?1",params![p,state,s.attempt.get(),i64::try_from(s.administrative_revision).map_err(|_|conflict())?]).map_err(store_error)?;
    tx.execute("INSERT INTO automatic_bootstrap_recoveries(parent_id,attempt,decision_kind,operation,revision,result_json) VALUES(?1,?2,?3,?4,?5,?6)",params![p,attempt.get(),kind,operation.as_str(),i64::try_from(s.administrative_revision).map_err(|_|conflict())?,bytes]).map_err(store_error)?;
    status(tx, ns, id)?;
    Ok(result)
}
fn actor_scope(id: &AutomaticBootstrapIdentity) -> String {
    actor(id)
}
pub(crate) fn complete(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &AutomaticBootstrapMutation,
    now: crate::protocol::time::UtcMillis,
) -> Result<CommandResult, ApiError> {
    request.validate().map_err(ApiError::invalid_request)?;
    let id = &request.identity;
    let s = status(tx, ns, id)?;
    let AutomaticBootstrapAction::Complete {
        attachment,
        retained,
    } = &request.action
    else {
        return Err(ApiError::invalid_request("not automatic completion"));
    };
    if s.attachment
        .as_ref()
        .is_none_or(|a| !same(a, attachment.as_ref()).unwrap_or(false))
        || s.thread.as_ref() != Some(&retained.thread)
    {
        return Err(mismatch());
    }
    validate_report(id, attachment, retained)?;
    let child =
        handoff::automatic::current(tx, &attachment.child.complete())?.ok_or_else(conflict)?;
    if child.state != HandoffState::Completed || child.thread.as_ref() != Some(&retained.thread) {
        return Err(conflict());
    }
    if !handoff::automatic::published_send(tx, &attachment.child, &retained.thread)? {
        return Err(conflict());
    }
    let crate::protocol::commands::PermitMutation::CompleteHandoff(inner) =
        handoff::automatic::inner(&attachment.child.complete())?
    else {
        unreachable!()
    };
    let expected = control::cooperative_payload_hash("complete_handoff", &inner)?;
    let complete_operation:Option<(Vec<u8>,String)>=tx.query_row("SELECT digest,substr(result_json,1,131073) FROM operations WHERE actor_scope=?1 AND operation_key=?2",params![actor(id),inner.operation.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
    let Some((digest, result)) = complete_operation else {
        return Err(conflict());
    };
    if digest != expected
        || decode::<CommandResult>(result.as_bytes(), LIMIT)? != CommandResult::Handoff(child)
    {
        return Err(mismatch());
    }
    if let Some(saved) = &s.completed {
        if !same(saved, retained)? {
            return Err(mismatch());
        }
    } else {
        if s.state != BootstrapState::Attached {
            return Err(conflict());
        }
        let p = parent(tx, id)?;
        tx.execute(
            "INSERT INTO automatic_bootstrap_reports(parent_id,report_json) VALUES(?1,?2)",
            params![p, encode(retained, 2097152)?],
        )
        .map_err(store_error)?;
        tx.execute(
            "UPDATE automatic_bootstrap_parents SET state='completed' WHERE id=?1",
            [p],
        )
        .map_err(store_error)?;
    }
    let result = CommandResult::AutomaticBootstrap(Box::new(status(tx, ns, id)?));
    let result_bytes = encode(&result, 2097152)?;
    let operation = request.operation().map_err(ApiError::invalid_request)?;
    let digest = crate::store::cooperative_permit_request(&PermitMutation::AutomaticBootstrap(
        Box::new(request.clone()),
    ))?
    .payload_hash;
    let previous: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![actor(id), operation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((saved, json)) = previous {
        if saved != digest || !same(&decode::<CommandResult>(json.as_bytes(), 2097152)?, &result)? {
            return Err(mismatch());
        }
    } else {
        tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES(?1,?2,?3,?4,?5)",params![actor(id),operation.as_str(),digest.as_slice(),std::str::from_utf8(&result_bytes).map_err(|_|corrupt())?,now.0]).map_err(store_error)?;
    }
    Ok(result)
}
