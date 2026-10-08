//! Transaction-local one-use submission and immutable operator decisions.
//! The daemon must supply canonical namespace/peer UID and the recovery caller
//! must hold the normal operation lock. These functions never call the host.
use super::{current, persistence::*};
use crate::{
    ports::{BootstrapAttachmentGuard, CreateTabOutcome, CreatedTab},
    protocol::{
        handoff::*,
        results::{ApiError, ErrorCode},
        time::UtcMillis,
    },
    store::connection::{api_error, store_error},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

fn conflict() -> ApiError {
    api_error(
        ErrorCode::Conflict,
        "bootstrap attempt or terminal disposition changed",
    )
}
fn invalid(detail: impl Into<String>) -> ApiError {
    api_error(ErrorCode::InvalidRequest, detail)
}
fn bounded<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, ApiError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("invalid bootstrap record"))?;
    if bytes.len() > limit {
        return Err(invalid("oversized bootstrap record"));
    }
    Ok(bytes)
}
fn parent(
    db: &Connection,
    ns: &HandoffNamespace,
    id: &BootstrapIdentity,
) -> Result<(i64, i64), ApiError> {
    db.query_row("SELECT id,administrative_revision FROM bootstrap_handoffs WHERE instance_id=?1 AND state_dir=?2 AND host_endpoint=?3 AND actor_scope=?4 AND compound=?5",params![ns.instance,ns.state_dir.to_str(),ns.host_endpoint.to_str(),scope(id),id.compound.as_str()],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?.ok_or_else(conflict)
}
fn status(
    db: &Connection,
    ns: &HandoffNamespace,
    id: &BootstrapIdentity,
) -> Result<BootstrapResult, ApiError> {
    current(db, ns, id)?.ok_or_else(conflict)
}
fn active(result: &BootstrapResult, attempt: BootstrapAttempt) -> Result<(), ApiError> {
    if result.attempt != attempt
        || matches!(
            result.state,
            BootstrapState::Completed | BootstrapState::Cancelled
        )
    {
        Err(conflict())
    } else {
        Ok(())
    }
}
fn phase(
    id: &BootstrapIdentity,
    attempt: BootstrapAttempt,
    role: &str,
    key: &crate::protocol::ids::OperationId,
) -> Result<(), ApiError> {
    if attempt.operation(&id.compound, role).map_err(invalid)? != *key {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap attempt operation mismatch",
        ));
    }
    Ok(())
}
fn atomic<T>(
    tx: &Transaction<'_>,
    body: impl FnOnce() -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    tx.execute_batch("SAVEPOINT bootstrap_transition")
        .map_err(store_error)?;
    match body() {
        Ok(v) => {
            tx.execute_batch("RELEASE bootstrap_transition")
                .map_err(store_error)?;
            Ok(v)
        }
        Err(e) => {
            tx.execute_batch("ROLLBACK TO bootstrap_transition; RELEASE bootstrap_transition")
                .map_err(store_error)?;
            Err(e)
        }
    }
}
fn next_attempt(
    tx: &Transaction<'_>,
    parent: i64,
    id: &BootstrapIdentity,
    attempt: BootstrapAttempt,
) -> Result<(), ApiError> {
    let n = BootstrapAttempt::new(attempt.get().checked_add(1).ok_or_else(conflict)?)
        .map_err(invalid)?;
    tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(?1,?2,'prepared',?3,?4,?5,?6)",params![parent,n.get(),n.operation(&id.compound,"reserve").map_err(invalid)?.as_str(),n.operation(&id.compound,"record").map_err(invalid)?.as_str(),n.operation(&id.compound,"check").map_err(invalid)?.as_str(),n.operation(&id.compound,"not_submitted").map_err(invalid)?.as_str()]).map_err(insertion_error)?;
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='prepared',current_attempt=?2 WHERE id=?1",
        params![parent, n.get()],
    )
    .map_err(store_error)?;
    Ok(())
}
pub fn reserve_attempt(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &ReserveBootstrapAttempt,
) -> Result<ReserveBootstrapResult, ApiError> {
    phase(
        &request.identity,
        request.expected_attempt,
        "reserve",
        &request.operation,
    )?;
    let result = status(tx, ns, &request.identity)?;
    active(&result, request.expected_attempt)?;
    validate_live(tx, ns, &request.identity)?;
    if result.state != BootstrapState::Prepared {
        return Ok(ReserveBootstrapResult::Replay {
            status: Box::new(result),
        });
    }
    let (parent, revision) = parent(tx, ns, &request.identity)?;
    atomic(tx, || {
        tx.execute("UPDATE bootstrap_attempts SET state='possible_creation',reserved_administrative_revision=?3 WHERE parent_id=?1 AND attempt=?2 AND state='prepared'",params![parent,request.expected_attempt.get(),revision]).map_err(store_error)?;
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='possible_creation' WHERE id=?1",
            [parent],
        )
        .map_err(store_error)?;
        status(tx, ns, &request.identity)?;
        Ok(ReserveBootstrapResult::Authorized {
            authorization: BootstrapSubmissionAuthorization {
                compound: request.identity.compound.clone(),
                attempt: request.expected_attempt,
                reservation: request.operation.clone(),
                administrative_revision: revision as u64,
            },
        })
    })
}
pub fn check_submission(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &CheckBootstrapSubmission,
) -> Result<BootstrapSubmissionChecked, ApiError> {
    phase(
        &request.identity,
        request.expected_attempt,
        "check",
        &request.operation,
    )?;
    let result = status(tx, ns, &request.identity)?;
    active(&result, request.expected_attempt)?;
    validate_live(tx, ns, &request.identity)?;
    // Unknown means the one submission may already have happened; never continue it.
    if result.attempt_state != BootstrapAttemptState::PossibleCreation {
        return Err(conflict());
    }
    let (parent, revision) = parent(tx, ns, &request.identity)?;
    let reserved:Option<i64>=tx.query_row("SELECT reserved_administrative_revision FROM bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",params![parent,request.expected_attempt.get()],|r|r.get(0)).map_err(store_error)?;
    if reserved != Some(revision) || revision as u64 != request.expected_administrative_revision {
        return Err(conflict());
    }
    Ok(BootstrapSubmissionChecked {
        compound: request.identity.compound.clone(),
        attempt: request.expected_attempt,
        administrative_revision: revision as u64,
    })
}
fn creation_bytes(
    ns: &HandoffNamespace,
    id: &BootstrapIdentity,
    evidence: &CreatedTab,
) -> Result<Vec<u8>, ApiError> {
    evidence.validate().map_err(invalid)?;
    if evidence.workspace != id.payload.workspace
        || evidence.witness.endpoint.as_os_str() != ns.host_endpoint.as_os_str()
    {
        return Err(invalid("bootstrap creation namespace mismatch"));
    }
    bounded(evidence, MAX_CREATION_BYTES)
}
pub fn record_created(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &RecordBootstrapCreated,
) -> Result<BootstrapResult, ApiError> {
    phase(
        &request.identity,
        request.expected_attempt,
        "record",
        &request.operation,
    )?;
    let result = status(tx, ns, &request.identity)?;
    active(&result, request.expected_attempt)?;
    let bytes = creation_bytes(ns, &request.identity, &request.evidence)?;
    if let Some(saved) = &result.creation {
        if !same(saved, &request.evidence)? {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "conflicting creation evidence",
            ));
        }
        return Ok(result);
    }
    validate_live(tx, ns, &request.identity)?;
    if !matches!(
        result.attempt_state,
        BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
    ) {
        return Err(conflict());
    }
    let (parent, _) = parent(tx, ns, &request.identity)?;
    atomic(tx, || {
        tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?3 WHERE parent_id=?1 AND attempt=?2",params![parent,request.expected_attempt.get(),bytes]).map_err(store_error)?;
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='created' WHERE id=?1",
            [parent],
        )
        .map_err(store_error)?;
        status(tx, ns, &request.identity)
    })
}
pub fn record_outcome(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    id: &BootstrapIdentity,
    attempt: BootstrapAttempt,
    outcome: &CreateTabOutcome,
) -> Result<BootstrapResult, ApiError> {
    if let CreateTabOutcome::Created(evidence) = outcome {
        return record_created(
            tx,
            ns,
            &RecordBootstrapCreated {
                identity: id.clone(),
                operation: attempt.operation(&id.compound, "record").map_err(invalid)?,
                expected_attempt: attempt,
                evidence: (**evidence).clone(),
            },
        );
    }
    record_failure(
        tx,
        ns,
        id,
        attempt,
        matches!(outcome, CreateTabOutcome::NotSubmitted(_)),
    )
}
/// Additive wire consumer; its producer must be the actual typed zero-byte
/// transport branch. Replay of an older closed attempt never advances again.
pub fn record_not_submitted(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &RecordBootstrapNotSubmitted,
) -> Result<BootstrapResult, ApiError> {
    phase(
        &request.identity,
        request.expected_attempt,
        "not_submitted",
        &request.operation,
    )?;
    record_failure(tx, ns, &request.identity, request.expected_attempt, true)
}
fn record_failure(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    id: &BootstrapIdentity,
    attempt: BootstrapAttempt,
    not_submitted: bool,
) -> Result<BootstrapResult, ApiError> {
    let result = status(tx, ns, id)?;
    if matches!(
        result.state,
        BootstrapState::Completed | BootstrapState::Cancelled
    ) {
        return Err(conflict());
    }
    validate_live(tx, ns, id)?;
    let (parent, _) = parent(tx, ns, id)?;
    if attempt != result.attempt {
        let saved: Option<String> = tx
            .query_row(
                "SELECT state FROM bootstrap_attempts WHERE parent_id=?1 AND attempt=?2",
                params![parent, attempt.get()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if saved.as_deref() == Some("not_submitted") && not_submitted {
            return Ok(result);
        }
        return Err(conflict());
    }
    if !matches!(
        result.attempt_state,
        BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
    ) {
        return Err(conflict());
    }
    if not_submitted && result.attempt_state != BootstrapAttemptState::PossibleCreation {
        return Err(conflict());
    }
    atomic(tx, || {
        if not_submitted {
            tx.execute("UPDATE bootstrap_attempts SET state='not_submitted' WHERE parent_id=?1 AND attempt=?2",params![parent,attempt.get()]).map_err(store_error)?;
            next_attempt(tx, parent, id, attempt)?;
        } else {
            tx.execute("UPDATE bootstrap_attempts SET state='outcome_unknown' WHERE parent_id=?1 AND attempt=?2",params![parent,attempt.get()]).map_err(store_error)?;
        }
        status(tx, ns, id)
    })
}
/// Recovery is a local-account assertion under the caller-held normal operation
/// lock. UID comes from the daemon's authenticated peer, never wire payload.
/// CreatedPane additionally consumes store-admitted fresh structural evidence.
pub fn recover(
    tx: &Transaction<'_>,
    ns: &HandoffNamespace,
    request: &RecoverBootstrap,
    operator_uid: u32,
    now: UtcMillis,
    guard: Option<&BootstrapAttachmentGuard>,
) -> Result<BootstrapRecoveryResult, ApiError> {
    request.disposition.validate().map_err(invalid)?;
    if request.operation != request.decision_operation().map_err(invalid)? {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "bootstrap recovery decision key mismatch",
        ));
    }
    let result = status(tx, ns, &request.identity)?;
    let (parent, revision) = parent(tx, ns, &request.identity)?;
    let decisions = decision_history(
        tx,
        ns,
        &request.identity,
        parent,
        &result,
        request.expected_attempt,
    )?;
    if let Some(saved) = decisions
        .into_iter()
        .find(|r| r.operation == request.operation)
    {
        if !same(&saved.disposition, &request.disposition)? {
            return Err(corrupt());
        }
        return Ok(saved);
    }
    active(&result, request.expected_attempt)?;
    let decision_kind = if matches!(
        request.disposition,
        BootstrapRecoveryDisposition::Cancelled { .. }
    ) {
        "cancellation"
    } else {
        "recovery"
    };
    let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM bootstrap_recovery_decisions WHERE parent_id=?1 AND attempt=?2 AND decision_kind=?3)",params![parent,request.expected_attempt.get(),decision_kind],|r|r.get(0)).map_err(store_error)?;
    if already {
        return Err(conflict());
    }
    let (state, creation) = match &request.disposition {
        BootstrapRecoveryDisposition::CreatedPane {
            evidence,
            structural_reference,
        } => {
            if !matches!(
                result.attempt_state,
                BootstrapAttemptState::PossibleCreation | BootstrapAttemptState::OutcomeUnknown
            ) {
                return Err(conflict());
            }
            creation_bytes(ns, &request.identity, evidence)?;
            let guard = guard
                .ok_or_else(|| invalid("created-pane needs fresh canonical structural evidence"))?;
            let proof = guard.ordinary().structural_proof();
            if guard.workspace() != &evidence.workspace
                || guard.tab() != &evidence.tab
                || guard.ordinary().call_id() != structural_reference
                || guard.ordinary().operation() != &request.identity.payload.resolve_key
                || proof.target() != &evidence.root_pane
                || proof.terminal() != &evidence.terminal
                || proof.host_boot() != &evidence.host_incarnation
            {
                return Err(invalid("created-pane structural evidence mismatch"));
            }
            crate::store::seats::validate_bootstrap_creation(tx, &ns.instance, guard.ordinary())?;
            (BootstrapState::Created, Some(evidence.clone()))
        }
        BootstrapRecoveryDisposition::NotCreated { .. } => {
            if result.creation.is_some() || result.attachment.is_some() {
                return Err(conflict());
            }
            (BootstrapState::Prepared, None)
        }
        BootstrapRecoveryDisposition::Cancelled { child_guard, .. } => {
            let attached = result.attachment.as_ref().map(|a| &a.handoff);
            if !same(&child_guard.attached_child.as_ref(), &attached)? {
                return Err(api_error(
                    ErrorCode::OperationPayloadMismatch,
                    "cancellation exact child mismatch",
                ));
            }
            let keys = &request.identity.payload.handoff.keys;
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?1 AND actor_scope=?2 AND state='live' AND (compound=?3 OR create_key=?4 OR invite_key=?5 OR send_key=?6))",params![ns.instance,scope(&request.identity),request.identity.payload.handoff_key.as_str(),keys.create.as_str(),keys.invite.as_str(),keys.send.as_str()],|r|r.get(0)).map_err(store_error)?;
            if live {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "cancellation cannot release a live legacy child fence or hint",
                ));
            }
            (BootstrapState::Cancelled, result.creation.clone())
        }
    };
    let saved = BootstrapRecoveryResult {
        identity: request.identity.clone(),
        attempt: request.expected_attempt,
        operation: request.operation.clone(),
        disposition: request.disposition.clone(),
        operator_uid,
        operator_provenance: format!("operator:local-user:{operator_uid}"),
        creation: creation.clone(),
        state,
    };
    let bytes = bounded(&saved, MAX_RECOVERY_BYTES)?;
    let next_revision = revision.checked_add(1).ok_or_else(conflict)?;
    atomic(tx, || {
        match state {
            BootstrapState::Prepared => {
                next_attempt(tx, parent, &request.identity, request.expected_attempt)?
            }
            BootstrapState::Created => {
                tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?3 WHERE parent_id=?1 AND attempt=?2",params![parent,request.expected_attempt.get(),creation_bytes(ns,&request.identity,creation.as_ref().ok_or_else(corrupt)?)?]).map_err(store_error)?;
                tx.execute(
                    "UPDATE bootstrap_handoffs SET state='created' WHERE id=?1",
                    [parent],
                )
                .map_err(store_error)?;
            }
            BootstrapState::Cancelled => {}
            _ => return Err(corrupt()),
        }
        tx.execute("INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,result_json,decision_kind) VALUES(?1,?2,?3,?4,?5)",params![parent,request.expected_attempt.get(),request.operation.as_str(),bytes,decision_kind]).map_err(insertion_error)?;
        if state == BootstrapState::Cancelled {
            tx.execute("UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=?4,latest_recovery_operation=?2,administrative_revision=?3 WHERE id=?1",params![parent,request.operation.as_str(),next_revision,now.0]).map_err(store_error)?;
        } else {
            tx.execute("UPDATE bootstrap_handoffs SET latest_recovery_operation=?2,administrative_revision=?3 WHERE id=?1",params![parent,request.operation.as_str(),next_revision]).map_err(store_error)?;
        }
        status(tx, ns, &request.identity)?;
        Ok(saved)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ports::*,
        protocol::{
            handoff::topology_contract_tests::{created, identity},
            ids::*,
            time::MonoInstant,
        },
        store::{schema, seats},
    };

    fn fixture() -> (Connection, BootstrapIdentity, BootstrapAttachmentGuard) {
        fixture_with_tab("w1:t2")
    }
    fn fixture_with_tab(tab: &str) -> (Connection, BootstrapIdentity, BootstrapAttachmentGuard) {
        let db = Connection::open_in_memory().unwrap();
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        let mut id = identity();
        id.payload.handoff.channel = HandoffChannel::New {
            name: None,
            topic: "topic".into(),
            goal: "goal".into(),
        };
        id.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
        id.digest = id.semantic_digest().unwrap();
        let boot = created().host_incarnation;
        db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,observation_sequence,observation_admission_sequence,observation_decided_sequence,lifecycle_revision,recovery_boot,recovery_epoch) VALUES('i',0,?1,1,1,1,1,1,?1,1)",[boot.as_str()]).unwrap();
        db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) VALUES('g','i',?1,1,1,'structural-incarnation',0,0,'published',0,0,0,0)",[boot.as_str()]).unwrap();
        db.execute_batch("UPDATE host_instances SET active_snapshot_id='g',recovery_baseline_generation_id='g'; INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('sender','i','resolved','native','w1:p1',1,0,0);").unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('sender',1,'w1:p1',?1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0)",[boot.as_str()]).unwrap();
        for (pane, terminal, generation) in
            [("w1:p1", "caller-terminal", 0), ("w1:p2", "terminal", 1)]
        {
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i',?1,?2,1,?3,2,'fresh',0,?4,'structural-incarnation','native_current_target',1)",params![pane,boot.as_str(),generation,terminal]).unwrap();
        }
        let observation = HostObservation {
            focused: false,
            target: HostTargetId::new("w1:p2"),
            host_boot: boot.clone(),
            epoch: 1,
            generation: 1,
            observed_at_utc: UtcMillis(1),
            observed_at_mono: MonoInstant(1),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Unknown,
            terminal: Some(TerminalId::new("terminal")),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: "structural-incarnation".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("fresh-structural-call"),
            connection_epoch: 1,
            observation_sequence: 2,
            started_at_mono: MonoInstant(1),
            completed_at_mono: MonoInstant(1),
        };
        let admission = HostObservationAdmission {
            instance: "i".into(),
            sequence: 1,
            expected_active: Some(SnapshotGenerationId::store_issued("g".into())),
            expected_boot: Some(boot),
            expected_epoch: 1,
            lifecycle_revision: 0,
            invalidation_revision: 0,
        };
        let guard = BootstrapAttachmentGuard::try_new(
            &crate::protocol::commands::ResolveSeat {
                target: observation.target.clone(),
                operation: id.payload.resolve_key.clone(),
            },
            BootstrapPaneObservation::try_new(
                observation,
                HostTargetId::new("w1"),
                HostTargetId::new(tab),
            )
            .unwrap(),
            &admission,
        )
        .unwrap();
        (db, id, guard)
    }
    fn request(id: &BootstrapIdentity, guard: &BootstrapAttachmentGuard) -> RecoverBootstrap {
        let mut r = RecoverBootstrap {
            identity: id.clone(),
            expected_attempt: BootstrapAttempt::first(),
            operation: id.compound.clone(),
            disposition: BootstrapRecoveryDisposition::CreatedPane {
                evidence: created(),
                structural_reference: guard.ordinary().call_id().clone(),
            },
        };
        r.operation = r.decision_operation().unwrap();
        r
    }
    fn begin_reserve(tx: &Transaction<'_>, id: &BootstrapIdentity) {
        let ns = &id.payload.handoff.namespace;
        super::super::begin_pending(tx, ns, id, UtcMillis(0)).unwrap();
        reserve_attempt(
            tx,
            ns,
            &ReserveBootstrapAttempt {
                identity: id.clone(),
                operation: BootstrapAttempt::first()
                    .operation(&id.compound, "reserve")
                    .unwrap(),
                expected_attempt: BootstrapAttempt::first(),
            },
        )
        .unwrap();
    }
    #[test]
    fn created_recovery_uses_process_boot_and_fresh_structural_call_without_seat_allocation() {
        let (mut db, id, guard) = fixture();
        let tx = db.transaction().unwrap();
        begin_reserve(&tx, &id);
        assert_ne!(
            guard.ordinary().structural_proof().incarnation(),
            created().host_incarnation.as_str()
        );
        let r = request(&id, &guard);
        let saved = recover(
            &tx,
            &id.payload.handoff.namespace,
            &r,
            501,
            UtcMillis(1),
            Some(&guard),
        )
        .unwrap();
        assert_eq!(saved.state, BootstrapState::Created);
        assert_eq!(saved.creation, Some(created()));
        assert_eq!(
            tx.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        tx.execute_batch("UPDATE host_instances SET lifecycle_revision=5;UPDATE occupant_bindings SET native_session='changed'").unwrap();
        assert_eq!(
            recover(
                &tx,
                &id.payload.handoff.namespace,
                &r,
                501,
                UtcMillis(2),
                None
            )
            .unwrap(),
            saved
        );
    }
    #[test]
    fn created_recovery_refuses_hold_stale_admission_lifecycle_and_structural_mismatch() {
        for (sql, expected) in [
            (
                "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) SELECT 'i','w1:p2',host_boot,1,'held' FROM host_instances",
                ErrorCode::TargetUnresolved,
            ),
            (
                "UPDATE host_instances SET observation_admission_sequence=2",
                ErrorCode::StaleHostObservation,
            ),
            (
                "UPDATE host_instances SET lifecycle_revision=2",
                ErrorCode::TargetUnresolved,
            ),
            (
                "UPDATE observed_targets SET terminal_id='different' WHERE target_id='w1:p2'",
                ErrorCode::StaleHostObservation,
            ),
            (
                "UPDATE observed_targets SET incarnation='different' WHERE target_id='w1:p2'",
                ErrorCode::StaleHostObservation,
            ),
        ] {
            let (mut db, id, guard) = fixture();
            let tx = db.transaction().unwrap();
            begin_reserve(&tx, &id);
            tx.execute_batch(sql).unwrap();
            assert_eq!(
                recover(
                    &tx,
                    &id.payload.handoff.namespace,
                    &request(&id, &guard),
                    501,
                    UtcMillis(1),
                    Some(&guard)
                )
                .unwrap_err()
                .code,
                expected
            );
            assert_eq!(
                status(&tx, &id.payload.handoff.namespace, &id)
                    .unwrap()
                    .state,
                BootstrapState::PossibleCreation
            );
        }
        let (mut db, id, guard) = fixture();
        let tx = db.transaction().unwrap();
        begin_reserve(&tx, &id);
        let mut r = request(&id, &guard);
        if let BootstrapRecoveryDisposition::CreatedPane {
            structural_reference,
            ..
        } = &mut r.disposition
        {
            *structural_reference = HostCallId::new("creation-correlation-is-not-fresh-call");
        }
        r.operation = r.decision_operation().unwrap();
        assert_eq!(
            recover(
                &tx,
                &id.payload.handoff.namespace,
                &r,
                501,
                UtcMillis(1),
                Some(&guard)
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
    }
    #[test]
    fn common_canonical_guard_returns_exact_current_owner_without_mutation() {
        let (mut db, _, guard) = fixture();
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,structural_incarnation_kind,structural_connection_epoch,structural_observation_sequence,created_at) VALUES('peer','i','resolved','native','w1:p2',1,1,'terminal','structural-incarnation',?1,1,'native_current_target',1,2,0)",[created().host_incarnation.as_str()]).unwrap();
        let tx = db.transaction().unwrap();
        assert_eq!(
            seats::validate_bootstrap_creation(&tx, "i", guard.ordinary()).unwrap(),
            Some(SeatId::new("peer"))
        );
        tx.execute_batch("UPDATE seats SET structural_terminal_id='changed' WHERE id='peer'")
            .unwrap();
        assert_eq!(
            seats::validate_bootstrap_creation(&tx, "i", guard.ordinary())
                .unwrap_err()
                .code,
            ErrorCode::TargetUnresolved
        );
    }
    #[test]
    fn created_recovery_then_cancellation_retains_both_immutable_decisions() {
        let (mut db, id, guard) = fixture();
        let tx = db.transaction().unwrap();
        begin_reserve(&tx, &id);
        let created_request = request(&id, &guard);
        let created_decision = recover(
            &tx,
            &id.payload.handoff.namespace,
            &created_request,
            501,
            UtcMillis(1),
            Some(&guard),
        )
        .unwrap();
        let mut cancel = RecoverBootstrap {
            identity: id.clone(),
            expected_attempt: BootstrapAttempt::first(),
            operation: id.compound.clone(),
            disposition: BootstrapRecoveryDisposition::Cancelled {
                reason: "confirmed pane later lost; no downstream child".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                child_guard: BootstrapCancellationGuard {
                    attached_child: None,
                },
            },
        };
        cancel.operation = cancel.decision_operation().unwrap();
        let cancelled = recover(
            &tx,
            &id.payload.handoff.namespace,
            &cancel,
            501,
            UtcMillis(2),
            None,
        )
        .unwrap();
        assert_eq!(cancelled.state, BootstrapState::Cancelled);
        assert_eq!(cancelled.creation, Some(created()));
        assert_eq!(
            recover(
                &tx,
                &id.payload.handoff.namespace,
                &created_request,
                501,
                UtcMillis(3),
                None
            )
            .unwrap(),
            created_decision
        );
        assert_eq!(
            recover(
                &tx,
                &id.payload.handoff.namespace,
                &cancel,
                501,
                UtcMillis(4),
                None
            )
            .unwrap(),
            cancelled
        );
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM bootstrap_recovery_decisions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn created_recovery_refuses_fresh_pane_that_now_belongs_to_another_tab() {
        let (mut db, id, guard) = fixture_with_tab("w1:other-tab");
        let tx = db.transaction().unwrap();
        begin_reserve(&tx, &id);
        assert_eq!(
            recover(
                &tx,
                &id.payload.handoff.namespace,
                &request(&id, &guard),
                501,
                UtcMillis(1),
                Some(&guard)
            )
            .unwrap_err()
            .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            status(&tx, &id.payload.handoff.namespace, &id)
                .unwrap()
                .state,
            BootstrapState::PossibleCreation
        );
    }
}
