//! Transaction-local persistence. No dispatch, permit issuance or host effects.
use super::{MAX_IDENTITY_BYTES, compare_identity, encode_identity};
use crate::{
    ports::CreatedTab,
    protocol::{
        handoff::*,
        ids::{OperationId, ThreadId},
        results::{ApiError, ErrorCode},
        time::UtcMillis,
    },
    store::connection::{api_error, store_error},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

/// Storage envelope limits, independently enforced without truncating data.
/// Identity remains 128 KiB; creation/attachment are also bounded 128 KiB.
/// Recovery includes the identity plus disposition/evidence and operator metadata.
/// Completed adds identity, attachment, the <=1 MiB retained report, repeated
/// launch argv and legacy result. These are storage ceilings, not a promise that
/// every protocol-valid envelope fits; future writers must refuse overflow.
pub const MAX_CREATION_BYTES: usize = 128 * 1024;
pub const MAX_ATTACHMENT_BYTES: usize = 128 * 1024;
pub const MAX_RECOVERY_BYTES: usize = 256 * 1024;
pub const MAX_COMPLETED_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn corrupt() -> ApiError {
    api_error(
        ErrorCode::StoreCorrupt,
        "invalid canonical bootstrap records",
    )
}
pub(super) fn scope(identity: &BootstrapIdentity) -> String {
    format!("seat:{}", identity.claim.seat.as_str())
}
fn parent_keys(identity: &BootstrapIdentity) -> [(&'static str, &OperationId); 10] {
    let payload = &identity.payload;
    let keys = &payload.handoff.keys;
    [
        ("compound", &identity.compound),
        ("begin", &keys.begin),
        ("create", &keys.create),
        ("invite", &keys.invite),
        ("send", &keys.send),
        ("complete", &keys.complete),
        ("handoff", &payload.handoff_key),
        ("resolve", &payload.resolve_key),
        ("attach", &payload.attach_key),
        ("linked_complete", &payload.linked_complete_key),
    ]
}
pub(super) fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: usize,
) -> Result<T, ApiError> {
    if bytes.len() > limit {
        return Err(corrupt());
    }
    let decoded: T = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
    if value != serde_json::to_value(&decoded).map_err(|_| corrupt())? {
        return Err(corrupt());
    }
    Ok(decoded)
}
pub(super) fn same<T: Serialize>(left: &T, right: &T) -> Result<bool, ApiError> {
    Ok(serde_json::to_value(left).map_err(|_| corrupt())?
        == serde_json::to_value(right).map_err(|_| corrupt())?)
}

struct Parent {
    id: i64,
    identity: Vec<u8>,
    digest: String,
    create_key: String,
    original_thread: Option<String>,
    thread: Option<String>,
    state: String,
    attempt: i64,
    attempt_state: Option<String>,
    creation: Option<Vec<u8>>,
    latest_recovery: Option<String>,
    reserve: Option<String>,
    record: Option<String>,
    check: Option<String>,
    terminal_at: Option<i64>,
    administrative_revision: i64,
}

/// Exact indexed canonical lookup. Reconstructs the declared BootstrapResult;
/// a status read never grants reservation or submission permission.
pub fn current(
    db: &Connection,
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
) -> Result<Option<BootstrapResult>, ApiError> {
    encode_identity(canonical, identity)?;
    let row = db.query_row(
        "SELECT p.id,substr(p.identity_json,1,?6),p.digest,p.create_key,p.original_thread,p.thread_id,p.state,p.current_attempt,a.state,substr(a.creation_json,1,?7),p.latest_recovery_operation,a.reserve_key,a.record_key,a.check_key,p.terminal_at,p.administrative_revision FROM bootstrap_handoffs p LEFT JOIN bootstrap_attempts a ON a.parent_id=p.id AND a.attempt=p.current_attempt WHERE p.instance_id=?1 AND p.state_dir=?2 AND p.host_endpoint=?3 AND p.actor_scope=?4 AND p.compound=?5",
        params![canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),scope(identity),identity.compound.as_str(),(MAX_IDENTITY_BYTES+1) as u32,(MAX_CREATION_BYTES+1) as u32],
        |r| Ok(Parent { id:r.get(0)?,identity:r.get(1)?,digest:r.get(2)?,create_key:r.get(3)?,original_thread:r.get(4)?,thread:r.get(5)?,state:r.get(6)?,attempt:r.get(7)?,attempt_state:r.get(8)?,creation:r.get(9)?,latest_recovery:r.get(10)?,reserve:r.get(11)?,record:r.get(12)?,check:r.get(13)?,terminal_at:r.get(14)?,administrative_revision:r.get(15)? }),
    ).optional().map_err(store_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    compare_identity(canonical, &row.identity, identity)?;
    if row.digest != identity.digest
        || row.create_key != identity.payload.handoff.keys.create.as_str()
        || row.original_thread.as_deref()
            != identity
                .payload
                .handoff
                .channel
                .thread()
                .map(ThreadId::as_str)
        || row
            .original_thread
            .as_ref()
            .is_some_and(|t| row.thread.as_ref() != Some(t))
    {
        return Err(corrupt());
    }
    let attempt = BootstrapAttempt::new(u32::try_from(row.attempt).map_err(|_| corrupt())?)
        .map_err(|_| corrupt())?;
    let state = match row.state.as_str() {
        "prepared" => BootstrapState::Prepared,
        "possible_creation" => BootstrapState::PossibleCreation,
        "created" => BootstrapState::Created,
        "attached" => BootstrapState::Attached,
        "completed" => BootstrapState::Completed,
        "cancelled" => BootstrapState::Cancelled,
        _ => return Err(corrupt()),
    };
    if matches!(state, BootstrapState::Completed | BootstrapState::Cancelled)
        != row.terminal_at.is_some()
        || row.administrative_revision < 0
    {
        return Err(corrupt());
    }
    let attempt_state = match row.attempt_state.as_deref() {
        Some("prepared") => BootstrapAttemptState::Prepared,
        Some("possible_creation") => BootstrapAttemptState::PossibleCreation,
        Some("not_submitted") => BootstrapAttemptState::NotSubmitted,
        Some("outcome_unknown") => BootstrapAttemptState::OutcomeUnknown,
        Some("created") => BootstrapAttemptState::Created,
        _ => return Err(corrupt()),
    };
    for (phase, saved) in [
        ("reserve", row.reserve.as_deref()),
        ("record", row.record.as_deref()),
        ("check", row.check.as_deref()),
    ] {
        if saved
            != Some(
                attempt
                    .operation(&identity.compound, phase)
                    .map_err(|_| corrupt())?
                    .as_str(),
            )
        {
            return Err(corrupt());
        }
    }
    check_registry(db, canonical, identity, row.id, attempt)?;
    let creation: Option<CreatedTab> = row
        .creation
        .as_deref()
        .map(|b| decode(b, MAX_CREATION_BYTES))
        .transpose()?;
    if let Some(created) = &creation {
        created.validate().map_err(|_| corrupt())?;
        if created.workspace != identity.payload.workspace
            || created.witness.endpoint.as_os_str() != canonical.host_endpoint.as_os_str()
        {
            return Err(corrupt());
        }
    }
    if (attempt_state == BootstrapAttemptState::Created) != creation.is_some() {
        return Err(corrupt());
    }
    let attachment_row: Option<(u32,Vec<u8>)> = db.query_row(
        "SELECT attempt,substr(attachment_json,1,?2) FROM bootstrap_attachments WHERE parent_id=?1",
        params![row.id,(MAX_ATTACHMENT_BYTES+1) as u32], |r|Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(store_error)?;
    let attachment = attachment_row
        .map(|(saved_attempt, bytes)| {
            let attachment: BootstrapAttachment = decode(&bytes, MAX_ATTACHMENT_BYTES)?;
            attachment.validate(identity).map_err(|_| corrupt())?;
            if i64::from(saved_attempt) != row.attempt
                || attachment.attempt != attempt
                || attachment
                    .handoff
                    .thread
                    .as_ref()
                    .is_some_and(|thread| row.thread.as_deref() != Some(thread.as_str()))
                || creation
                    .as_ref()
                    .is_none_or(|created| !same(created, &attachment.created).unwrap_or(false))
            {
                return Err(corrupt());
            }
            Ok(attachment)
        })
        .transpose()?;
    let report: Option<Vec<u8>> = db
        .query_row(
            "SELECT substr(completed_json,1,?2) FROM bootstrap_reports WHERE parent_id=?1",
            params![row.id, (MAX_COMPLETED_BYTES + 1) as u32],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let completed = report
        .map(|bytes| {
            let result: CompletedBootstrapResult = decode(&bytes, MAX_COMPLETED_BYTES)?;
            compare_identity(canonical, &row.identity, &result.identity).map_err(|_| corrupt())?;
            result.retained.validate().map_err(|_| corrupt())?;
            if attachment
                .as_ref()
                .is_none_or(|a| !same(a, &result.attachment).unwrap_or(false))
                || result.legacy_result.state != HandoffState::Completed
                || result.legacy_result.compound != result.attachment.handoff.compound
                || result.legacy_result.thread.as_ref().map(ThreadId::as_str)
                    != row.thread.as_deref()
                || result.retained.launch != identity.payload.launch
                || row.thread.as_deref() != Some(result.retained.thread.as_str())
                || result.retained.recipient != result.attachment.resolved_seat
                || result.retained.pane != result.attachment.created.root_pane
                || result.retained.terminal != result.attachment.created.terminal
                || result.retained.host_incarnation != result.attachment.created.host_incarnation
            {
                return Err(corrupt());
            }
            Ok(Box::new(result))
        })
        .transpose()?;
    let recovery = row.latest_recovery.as_ref().map(|operation| {
        let saved: Option<(u32,Vec<u8>)> = db.query_row(
            "SELECT attempt,substr(result_json,1,?3) FROM bootstrap_recovery_decisions WHERE parent_id=?1 AND operation=?2",
            params![row.id,operation,(MAX_RECOVERY_BYTES+1) as u32], |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(store_error)?;
        let (saved_attempt,bytes) = saved.ok_or_else(corrupt)?;
        let result: BootstrapRecoveryResult = decode(&bytes,MAX_RECOVERY_BYTES)?;
        compare_identity(canonical,&row.identity,&result.identity).map_err(|_| corrupt())?;
        result.disposition.validate().map_err(|_| corrupt())?;
        if result.attempt.get() != saved_attempt || i64::from(saved_attempt) > row.attempt
            || result.operation.as_str() != operation
            || result.operator_provenance != format!("operator:local-user:{}",result.operator_uid)
        { return Err(corrupt()); }
        let decision = RecoverBootstrap {identity:result.identity.clone(),expected_attempt:result.attempt,operation:result.operation.clone(),disposition:result.disposition.clone()};
        if decision.decision_operation().map_err(|_| corrupt())? != result.operation { return Err(corrupt()); }
        // A decision key excludes the result's creation and snapshot state.
        // Compare against its referenced attempt, never a later current attempt.
        let saved_attempt_record: Option<(String,Option<Vec<u8>>)> = db.query_row(
            "SELECT state,substr(creation_json,1,?3) FROM bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",
            params![row.id,saved_attempt,(MAX_CREATION_BYTES+1) as u32], |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(store_error)?;
        let (saved_state,saved_creation_bytes) = saved_attempt_record.ok_or_else(corrupt)?;
        let saved_creation: Option<CreatedTab> = saved_creation_bytes.as_deref()
            .map(|b| decode(b,MAX_CREATION_BYTES)).transpose()?;
        for created in result.creation.iter().chain(saved_creation.iter()) {
            created.validate().map_err(|_| corrupt())?;
            if created.workspace != identity.payload.workspace
                || created.witness.endpoint.as_os_str() != canonical.host_endpoint.as_os_str()
            { return Err(corrupt()); }
        }
        if !same(&result.creation,&saved_creation)?
            || !matches!(saved_state.as_str(),"prepared"|"possible_creation"|"not_submitted"|"outcome_unknown"|"created")
            || (saved_state=="created") != saved_creation.is_some()
        { return Err(corrupt()); }
        let coherent_recovery = match &result.disposition {
            BootstrapRecoveryDisposition::CreatedPane { evidence,.. } => {
                result.state==BootstrapState::Created && saved_state=="created"
                    && result.attempt==attempt
                    && result.creation.as_ref().is_some_and(|c|same(c,evidence).unwrap_or(false))
            }
            BootstrapRecoveryDisposition::NotCreated { .. } => {
                // Inspection is not proof of non-submission: retain any defined
                // non-Created attempt state, closed before a distinct later attempt.
                result.state==BootstrapState::Prepared && result.creation.is_none()
                    && saved_state!="created" && result.attempt<attempt
            }
            BootstrapRecoveryDisposition::Cancelled { .. } => {
                result.state==BootstrapState::Cancelled && state==BootstrapState::Cancelled
            }
        };
        if !coherent_recovery { return Err(corrupt()); }
        Ok(Box::new(result))
    }).transpose()?;
    let coherent = match state {
        BootstrapState::Prepared => {
            attempt_state == BootstrapAttemptState::Prepared
                && attachment.is_none()
                && completed.is_none()
        }
        BootstrapState::PossibleCreation => {
            matches!(
                attempt_state,
                BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
            ) && attachment.is_none()
                && completed.is_none()
        }
        BootstrapState::Created => {
            creation.is_some() && attachment.is_none() && completed.is_none()
        }
        BootstrapState::Attached => attachment.is_some() && completed.is_none(),
        BootstrapState::Completed => completed.is_some(),
        BootstrapState::Cancelled => {
            completed.is_none()
                && attachment.is_none()
                && recovery.as_ref().is_some_and(|r| {
                    r.state == BootstrapState::Cancelled
                        && r.attempt == attempt
                        && matches!(
                            r.disposition,
                            BootstrapRecoveryDisposition::Cancelled { .. }
                        )
                })
        }
    };
    if !coherent {
        return Err(corrupt());
    }
    Ok(Some(BootstrapResult {
        compound: identity.compound.clone(),
        attempt,
        attempt_state,
        state,
        creation,
        attachment,
        completed,
        recovery,
    }))
}

fn check_registry(
    db: &Connection,
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
    parent: i64,
    attempt: BootstrapAttempt,
) -> Result<(), ApiError> {
    let mut expected = parent_keys(identity)
        .into_iter()
        .map(|(role, key)| (role.to_owned(), 0, key.as_str().to_owned()))
        .collect::<Vec<_>>();
    for phase in ["reserve", "record", "check"] {
        expected.push((
            phase.to_owned(),
            attempt.get(),
            attempt
                .operation(&identity.compound, phase)
                .map_err(|_| corrupt())?
                .as_str()
                .to_owned(),
        ));
    }
    let mut statement=db.prepare("SELECT role,attempt,operation_key FROM bootstrap_child_keys INDEXED BY bootstrap_keys_attempt WHERE parent_id=?1 AND instance_id=?2 AND state_dir=?3 AND host_endpoint=?4 AND actor_scope=?5 AND role!='recovery' AND attempt IN (0,?6) LIMIT 14").map_err(store_error)?;
    let mut saved = statement
        .query_map(
            params![
                parent,
                canonical.instance,
                canonical.state_dir.to_str(),
                canonical.host_endpoint.to_str(),
                scope(identity),
                attempt.get()
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    expected.sort();
    saved.sort();
    if expected != saved {
        return Err(corrupt());
    }
    Ok(())
}

pub(super) fn validate_live(
    db: &Connection,
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
) -> Result<(), ApiError> {
    crate::store::seats::cooperative_mapping(db, &identity.claim, None)?;
    let recorded: Option<Option<String>> = db.query_row(
        "SELECT thread_id FROM bootstrap_handoffs WHERE instance_id=?1 AND state_dir=?2 AND host_endpoint=?3 AND actor_scope=?4 AND compound=?5",
        params![canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),scope(identity),identity.compound.as_str()], |r|r.get(0),
    ).optional().map_err(store_error)?;
    let thread = recorded
        .flatten()
        .map(ThreadId::parse)
        .transpose()
        .map_err(|_| corrupt())?
        .or_else(|| identity.payload.handoff.channel.thread().cloned());
    if let Some(thread) = thread {
        let row: Option<(bool,bool)> = db.query_row(
            "SELECT archived,EXISTS(SELECT 1 FROM memberships m WHERE m.thread_id=t.id AND m.seat_id=?2 AND m.state='joined') FROM threads t WHERE id=?1 AND instance_id=?3",
            params![thread.as_str(),identity.claim.seat.as_str(),identity.claim.instance],|r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(store_error)?;
        match row {
            Some((true, _)) => {
                return Err(api_error(
                    ErrorCode::Archived,
                    "bootstrap thread is archived",
                ));
            }
            Some((false, true)) => {}
            _ => {
                return Err(api_error(
                    ErrorCode::MembershipRequired,
                    "bootstrap requires joined sender in this instance",
                ));
            }
        }
    }
    Ok(())
}
pub(super) fn insertion_error(error: rusqlite::Error) -> ApiError {
    if matches!(error,rusqlite::Error::SqliteFailure(ref failure,_) if failure.code==rusqlite::ErrorCode::ConstraintViolation)
    {
        api_error(
            ErrorCode::Conflict,
            "bootstrap immutable key or record already registered",
        )
    } else {
        store_error(error)
    }
}

/// Begin inside the caller's deciding transaction. LIVE begin/replay uses the
/// existing A2 mapping and membership/archive guards. Exact terminal replay is
/// absorbing after original identity validation. This primitive issues no permit.
/// A helper-local savepoint rolls back all its rows on any late insert refusal,
/// even when the enclosing transaction caller subsequently commits.
pub fn begin_pending(
    tx: &Transaction<'_>,
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
    now: UtcMillis,
) -> Result<BootstrapResult, ApiError> {
    let bytes = encode_identity(canonical, identity)?;
    if let Some(result) = current(tx, canonical, identity)? {
        if !matches!(
            result.state,
            BootstrapState::Completed | BootstrapState::Cancelled
        ) {
            validate_live(tx, canonical, identity)?;
        }
        return Ok(result);
    }
    validate_live(tx, canonical, identity)?;
    tx.execute_batch("SAVEPOINT bootstrap_begin")
        .map_err(store_error)?;
    let result = (|| {
        let original = identity
            .payload
            .handoff
            .channel
            .thread()
            .map(ThreadId::as_str);
        tx.execute("INSERT INTO bootstrap_handoffs(instance_id,state_dir,host_endpoint,actor_scope,compound,digest,identity_json,create_key,original_thread,thread_id,state,current_attempt,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,'prepared',1,?10)",params![canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),scope(identity),identity.compound.as_str(),identity.digest,bytes,identity.payload.handoff.keys.create.as_str(),original,now.0]).map_err(insertion_error)?;
        let parent = tx.last_insert_rowid();
        for (role, key) in parent_keys(identity) {
            tx.execute("INSERT INTO bootstrap_child_keys(parent_id,instance_id,state_dir,host_endpoint,actor_scope,operation_key,role,attempt) VALUES(?1,?2,?3,?4,?5,?6,?7,0)",params![parent,canonical.instance,canonical.state_dir.to_str(),canonical.host_endpoint.to_str(),scope(identity),key.as_str(),role]).map_err(insertion_error)?;
        }
        let attempt = BootstrapAttempt::first();
        tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key) VALUES(?1,1,'prepared',?2,?3,?4)",params![parent,attempt.operation(&identity.compound,"reserve").map_err(|_| corrupt())?.as_str(),attempt.operation(&identity.compound,"record").map_err(|_| corrupt())?.as_str(),attempt.operation(&identity.compound,"check").map_err(|_| corrupt())?.as_str()]).map_err(insertion_error)?;
        current(tx, canonical, identity)?.ok_or_else(corrupt)
    })();
    match result {
        Ok(result) => {
            tx.execute_batch("RELEASE bootstrap_begin")
                .map_err(store_error)?;
            Ok(result)
        }
        Err(error) => {
            tx.execute_batch("ROLLBACK TO bootstrap_begin; RELEASE bootstrap_begin")
                .map_err(store_error)?;
            Err(error)
        }
    }
}
