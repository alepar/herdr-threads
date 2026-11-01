//! Seat resolution: `resolve_*`, bootstrap resolution and ordinary allocation.

use crate::{
    ports::{
        DurableStructuralProof, HostId, OrdinaryResolutionAttempt, OrdinaryResolutionGuard,
        OrdinaryResolutionOutcome, ResolvedTargetCheck, SnapshotGenerationId,
    },
    protocol::{
        authority::MutationPermit,
        ids::{HostBootId, HostTargetId, SeatId, SeatName, prefix},
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        effective::{self, EffectiveObservationSource, EffectiveRecoveryDisposition},
        schema,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::*;

pub(super) fn target_free(
    tx: &Transaction<'_>,
    host: HostId,
    target: &str,
) -> Result<bool, ApiError> {
    tx.query_row("SELECT NOT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved')",
        params![host.get(),target], |r| r.get(0)).map_err(store_error)
}

pub(super) fn observed_matches(
    tx: &Transaction<'_>,
    host: HostId,
    target: &str,
    boot: &HostBootId,
    epoch: u64,
    generation: u64,
) -> Result<bool, ApiError> {
    let epoch = checked_host_number(epoch)?;
    let generation = checked_host_number(generation)?;
    Ok(
        effective::effective_observation(tx, host, target)?.is_some_and(|observation| {
            observation.source == EffectiveObservationSource::NewerCurrentTarget
                && observation.host_boot == boot.as_str()
                && observation.epoch == epoch
                && observation.structural_generation == generation
        }),
    )
}

pub(super) fn structural_proof_matches_current(
    tx: &Transaction<'_>,
    host: HostId,
    proof: &DurableStructuralProof,
) -> Result<bool, ApiError> {
    let epoch = checked_host_number(proof.host_epoch())?;
    let sequence = checked_host_number(proof.observation_sequence())?;
    let generation = checked_host_number(proof.target_generation())?;
    let observation = effective::effective_observation(tx, host, proof.target().as_str())?;
    let source_matches = observation.is_some_and(|observation| {
        observation.source == EffectiveObservationSource::NewerCurrentTarget
            && observation.host_boot == proof.host_boot().as_str()
            && observation.epoch == epoch
            && observation.observation_sequence == sequence
            && observation.structural_generation == generation
            && observation.terminal_id.as_deref() == Some(proof.terminal().as_str())
    });
    if !source_matches {
        return Ok(false);
    }
    let incarnation_matches: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM herdr_hosts h JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' WHERE h.host_id=?1 AND h.host_boot=?2 AND h.host_epoch=?3 AND g.incarnation=?4)",
        params![host.get(), proof.host_boot().as_str(), epoch, proof.incarnation()],
        |r| r.get(0),
    ).map_err(store_error)?;
    Ok(incarnation_matches)
}

fn insert_ordinary_allocation(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    host: HostId,
    proof: &DurableStructuralProof,
) -> Result<CommandResult, ApiError> {
    let seat = SeatId::new(crate::store::public_ids::fresh(
        tx,
        prefix::SEAT,
        &[("seats", "id", prefix::SEAT)],
    )?);
    tx.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,unavailability_episode,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,1,?2,1,?13)",
                params![seat.as_str(),instance,proof.target().as_str(),checked_host_number(proof.target_generation())?,
                    proof.terminal().as_str(),proof.incarnation(),proof.source_spelling(),proof.host_boot().as_str(),
                    checked_host_number(proof.host_epoch())?,checked_host_number(proof.connection_epoch())?,
                    checked_host_number(proof.observation_sequence())?,at.utc.0,host.get()]).map_err(store_error)?;
    tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation) VALUES (?1,?2,?3,'ordinary',?4,?5,?6,?7)",
                params![host.get(),proof.target().as_str(),seat.as_str(),at.utc.0,proof.host_boot().as_str(),checked_host_number(proof.host_epoch())?,checked_host_number(proof.target_generation())?]).map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_host_lifecycle_revision(tx, host)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(CommandResult::SeatResolved(seat))
}

fn ordinary_resolution_digest(
    instance: &str,
    request: &crate::protocol::commands::ResolveSeat,
) -> Result<[u8; 32], ApiError> {
    if request.target.as_str().is_empty() || request.operation.as_str().is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "resolution target and operation are required",
        ));
    }
    schema::canonical_digest(&("resolve_seat", instance, &request.target))
}

fn ordinary_replay(
    conn: &Connection,
    instance: &str,
    request: &crate::protocol::commands::ResolveSeat,
    digest: &[u8; 32],
) -> Result<Option<SeatId>, ApiError> {
    let row: Option<(Vec<u8>, String)> = conn
        .query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![
                format!("service-allocation:{instance}"),
                request.operation.as_str()
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((stored_digest, result)) = row else {
        return Ok(None);
    };
    if stored_digest.as_slice() != digest {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "operation key reused with different payload",
        ));
    }
    match serde_json::from_str::<CommandResult>(&result).map_err(|_| {
        api_error(
            ErrorCode::StoreCorrupt,
            "invalid ordinary resolution replay",
        )
    })? {
        CommandResult::SeatResolved(seat) => Ok(Some(seat)),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "ordinary resolution replay has wrong result type",
        )),
    }
}

fn validate_resolution_evidence(
    tx: &Transaction<'_>,
    instance: &str,
    guard: &OrdinaryResolutionGuard,
) -> Result<(), ApiError> {
    let proof = guard.structural_proof();
    let admission = guard.admission();
    let sequence = checked_host_number(admission.sequence())?;
    let epoch = checked_host_number(proof.host_epoch())?;
    let observed_sequence = checked_host_number(proof.observation_sequence())?;
    let generation = checked_host_number(proof.target_generation())?;
    let connection_epoch = checked_host_number(proof.connection_epoch())?;
    let host = admission.host_id;
    let admitted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM herdr_hosts h JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.host_id=h.host_id AND g.status='published' AND g.staged_targets=g.expected_targets WHERE h.host_id=?8 AND h.node_id=?1 AND h.host_boot=?2 AND h.host_epoch=?3 AND h.observation_admission_sequence=?4 AND h.observation_decided_sequence=?4 AND h.invalidation_revision=?5 AND h.active_snapshot_id=?6 AND g.incarnation=?7 AND g.published_invalidation_revision=h.invalidation_revision)",
        params![instance,proof.host_boot().as_str(),epoch,sequence,checked_host_number(admission.invalidation_revision)?,admission.expected_active.as_ref().map(SnapshotGenerationId::as_str),proof.incarnation(),host.get()],
        |r| r.get(0),
    ).map_err(store_error)?;
    let effective = effective::effective_observation(tx, host, proof.target().as_str())?;
    if admission.instance() != instance
        || !admitted
        || !effective.is_some_and(|row| {
            row.source == EffectiveObservationSource::NewerCurrentTarget
                && row.host_boot == proof.host_boot().as_str()
                && row.epoch == epoch
                && row.structural_generation == generation
                && row.observation_sequence == observed_sequence
                && row.connection_epoch == Some(connection_epoch)
                && row.incarnation.as_deref() == Some(proof.incarnation())
                && row.incarnation_source_kind.as_deref() == Some(proof.source_spelling())
                && row.terminal_id.as_deref() == Some(proof.terminal().as_str())
        })
    {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "ordinary resolution evidence was superseded or invalidated",
        ));
    }
    Ok(())
}

pub(crate) fn current_resolution_owner(
    tx: &Transaction<'_>,
    instance: &str,
    guard: &OrdinaryResolutionGuard,
) -> Result<Option<SeatId>, ApiError> {
    validate_resolution_evidence(tx, instance, guard)?;
    let proof = guard.structural_proof();
    let host = guard.admission().host_id;
    // Ownership is only a locator; independently check active explicit holds.
    let held: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND target_id=?2 AND released_at IS NULL)",
        params![host.get(),proof.target().as_str()], |r| r.get(0),
    ).map_err(store_error)?;
    if held {
        return Err(resolution_recovery_refusal(
            tx,
            host,
            proof.target().as_str(),
            "explicit restore hold",
        )?);
    }
    type OwnerColumns = Option<(
        String,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )>;
    let owner: OwnerColumns = tx.query_row(
        "SELECT id,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,retired_at,retired_seq FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
        params![host.get(),proof.target().as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
    ).optional().map_err(store_error)?;
    let Some((seat, generation, terminal, incarnation, boot, epoch, retired_at, retired_seq)) =
        owner
    else {
        return Ok(None);
    };
    // The same live terminal in the same verified server incarnation bridges
    // a newer connection epoch (daemon restart or reconnect); an owner proven
    // under a newer epoch than this read is not current evidence.
    let proof_epoch = checked_host_number(proof.host_epoch())?;
    if generation != checked_host_number(proof.target_generation())?
        || terminal.as_deref() != Some(proof.terminal().as_str())
        || incarnation.as_deref() != Some(proof.incarnation())
        || boot.as_deref() != Some(proof.host_boot().as_str())
        || epoch.is_none_or(|epoch| epoch > proof_epoch)
        || retired_at.is_some()
        || retired_seq.is_some()
    {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "existing seat needs structural reconciliation",
        ));
    }
    Ok(Some(SeatId::new(seat)))
}

fn resolution_recovery_refusal(
    tx: &Transaction<'_>,
    host: HostId,
    target: &str,
    cause: &str,
) -> Result<ApiError, ApiError> {
    let pane = shlex::try_quote(target)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid target"))?;
    let mut blockers = tx.prepare(&format!("SELECT id FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND state='unresolved' ORDER BY ordinal LIMIT 3")).map_err(store_error)?;
    let seats = blockers
        .query_map([host.get()], |row| row.get::<_, String>(0))
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    let repair = seats.iter().map(|seat| {
            let seat = shlex::try_quote(seat).unwrap_or_else(|_| "SEAT".into());
            format!("inspect `herdr-threads seat inspect {seat}`; rebind if this is its pane: `herdr-threads human seat rebind {seat} --pane {pane} --operator`")
        }).collect::<Vec<_>>().join("; ");
    Ok(api_error(
        ErrorCode::TargetUnresolved,
        format!(
            "{cause} on {pane}; fresh-seat repair: `herdr-threads human seat resolve --pane {pane} --new-seat --operator`; {repair}; `herdr-threads daemon health` lists unresolved blockers (up to 3 shown here); baseline holds lift when no unresolved seats remain"
        ),
    ))
}

fn validate_resolution_allocation(
    tx: &Transaction<'_>,
    host: HostId,
    proof: &DurableStructuralProof,
) -> Result<(), ApiError> {
    // A target appearing after the restore baseline is not a new pane when
    // an unresolved seat still references it. Decide this canonically rather
    // than relying on a client's earlier seat listing to refuse allocation.
    let unresolved: bool = tx.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND target_id=?2 AND state='unresolved')"),
        params![host.get(), proof.target().as_str()], |row| row.get(0),
    ).map_err(store_error)?;
    if unresolved {
        return Err(resolution_recovery_refusal(
            tx,
            host,
            proof.target().as_str(),
            "target has unresolved seat mapping",
        )?);
    }
    let recovery: Option<(Option<String>, Option<i64>)> = tx
        .query_row(
            "SELECT recovery_boot,recovery_epoch FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let disposition = effective::effective_recovery_disposition(tx, host, proof.target().as_str())?;
    let cause = match disposition {
        EffectiveRecoveryDisposition::UnambiguousUnclaimed
        | EffectiveRecoveryDisposition::CreatedAfterBaseline => None,
        EffectiveRecoveryDisposition::BaselineHeld => Some("restore baseline hold"),
        EffectiveRecoveryDisposition::ExplicitHold => Some("explicit restore hold"),
        EffectiveRecoveryDisposition::Unknown => Some("durable recovery baseline unknown"),
        EffectiveRecoveryDisposition::Released => Some("released target has no resolved seat"),
        EffectiveRecoveryDisposition::Owned(_) => Some("target already owned"),
    };
    if let Some(cause) = cause {
        return Err(resolution_recovery_refusal(
            tx,
            host,
            proof.target().as_str(),
            cause,
        )?);
    }
    if !recovery.is_some_and(|(boot, epoch)| {
        boot.as_deref() == Some(proof.host_boot().as_str())
            && epoch.is_some_and(|epoch| epoch >= 0 && epoch <= proof.host_epoch() as i64)
    }) {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "recovery boot/epoch does not match current target evidence; inspect `herdr-threads daemon health` and retry after reconciliation",
        ));
    }
    Ok(())
}

/// Reuse ordinary current-target/hold/recovery guards without allocating or
/// changing a seat. Bootstrap evidence never bypasses restore continuity.
pub(crate) fn validate_bootstrap_creation(
    tx: &Transaction<'_>,
    instance: &str,
    guard: &OrdinaryResolutionGuard,
) -> Result<Option<SeatId>, ApiError> {
    let owner = current_resolution_owner(tx, instance, guard)?;
    if owner.is_none() {
        let lifecycle: i64 = tx
            .query_row(
                "SELECT lifecycle_revision FROM herdr_hosts WHERE host_id=?1",
                [guard.admission().host_id.get()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let published_lifecycle = checked_host_number(guard.admission().lifecycle_revision)?
            .checked_add(1)
            .ok_or_else(|| {
                api_error(
                    ErrorCode::SequenceExhausted,
                    "resolution lifecycle revision exhausted",
                )
            })?;
        if lifecycle != published_lifecycle {
            return Err(api_error(
                ErrorCode::TargetUnresolved,
                "allocation observation predates a lifecycle change",
            ));
        }
        validate_resolution_allocation(tx, guard.admission().host_id, guard.structural_proof())?;
    }
    Ok(owner)
}

pub fn resolve_seat(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    request: crate::protocol::commands::ResolveSeat,
    attempt: OrdinaryResolutionAttempt,
    budget: &CallBudget,
) -> Result<OrdinaryResolutionOutcome, ApiError> {
    snapshot_budget(context, budget)?;
    super::super::topology_handoff::automatic::guard_raw_resolution(
        conn,
        instance,
        request.operation.as_str(),
    )?;
    let digest = ordinary_resolution_digest(instance, &request)?;
    if matches!(attempt, OrdinaryResolutionAttempt::ReplayOnly) {
        return context.execute_decision(
            conn,
            |tx| {
                snapshot_budget(context, budget)?;
                ordinary_replay(tx, instance, &request, &digest)
            },
            |_, _, replay| {
                snapshot_budget(context, budget)?;
                Ok(match replay {
                    Some(seat) => OrdinaryResolutionOutcome::Resolved(seat),
                    None => OrdinaryResolutionOutcome::NeedsObservation,
                })
            },
        );
    }
    let OrdinaryResolutionAttempt::Observed(guard) = attempt else {
        unreachable!()
    };
    resolve_observed(context, conn, instance, &request, &guard, budget)
}

fn resolve_observed(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    request: &crate::protocol::commands::ResolveSeat,
    guard: &OrdinaryResolutionGuard,
    budget: &CallBudget,
) -> Result<OrdinaryResolutionOutcome, ApiError> {
    let digest = ordinary_resolution_digest(instance, request)?;
    // The shared helper performs exact-key replay before this guard validation.
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &format!("service-allocation:{instance}"),
        request.operation.as_str(),
        digest,
        |tx| {
            snapshot_budget(context, budget)?;
            if guard.structural_proof().target() != &request.target
                || guard.operation() != &request.operation
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "resolution guard request mismatch",
                ));
            }
            validate_bootstrap_creation(tx, instance, guard)?;
            Ok(())
        },
        |tx, at| {
            snapshot_budget(context, budget)?;
            ordinary_resolution_apply(
                tx,
                at,
                instance,
                guard.admission().host_id,
                &request.target,
                guard.structural_proof(),
            )
            .map(|(result, _)| result)
        },
    )?;
    match result {
        CommandResult::SeatResolved(seat) => Ok(OrdinaryResolutionOutcome::Resolved(seat)),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "ordinary resolution replay has wrong result type",
        )),
    }
}

/// Bootstrap-only fresh exact scope before allocation, including retained
/// operation replay. Public handler/permit/namespace integration remains19.
pub fn resolve_bootstrap_seat(
    context: &StoreContext,
    conn: &mut Connection,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    request: &crate::protocol::handoff::ResolveBootstrapSeat,
    guard: &crate::ports::BootstrapAttachmentGuard,
    budget: &CallBudget,
) -> Result<SeatId, ApiError> {
    resolve_bootstrap_seat_impl(context, conn, canonical, request, guard, budget, None)
}
/// Permit consumption shares the exact ordinary allocation transaction.
pub(crate) fn resolve_bootstrap_seat_accountable(
    context: &StoreContext,
    conn: &mut Connection,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    request: &crate::protocol::handoff::ResolveBootstrapSeat,
    guard: &crate::ports::BootstrapAttachmentGuard,
    budget: &CallBudget,
    permit: MutationPermit,
) -> Result<SeatId, ApiError> {
    resolve_bootstrap_seat_impl(
        context,
        conn,
        canonical,
        request,
        guard,
        budget,
        Some(permit),
    )
}
fn resolve_bootstrap_seat_impl(
    context: &StoreContext,
    conn: &mut Connection,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    request: &crate::protocol::handoff::ResolveBootstrapSeat,
    guard: &crate::ports::BootstrapAttachmentGuard,
    budget: &CallBudget,
    mut permit: Option<MutationPermit>,
) -> Result<SeatId, ApiError> {
    let input = super::super::cooperative_permit_request(
        &crate::protocol::commands::PermitMutation::ResolveBootstrapSeat(Box::new(request.clone())),
    )?;
    let issuance = permit
        .as_ref()
        .map(|permit| permit.cooperative_metadata().1);
    snapshot_budget(context, budget)?;
    request
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let status = super::super::topology_handoff::current(conn, canonical, &request.identity)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "bootstrap missing"))?;
    let created = status
        .creation
        .ok_or_else(|| api_error(ErrorCode::Conflict, "bootstrap creation missing"))?;
    let ordinary = crate::protocol::commands::ResolveSeat {
        target: created.root_pane,
        host: request.identity.claim.host.clone(),
        operation: request.operation.clone(),
        locator_hint: None,
    };
    let digest = ordinary_resolution_digest(&canonical.instance, &ordinary)?;
    let own_allocation = std::cell::Cell::new(false);
    let validate = |tx: &Transaction<'_>, allocated: bool| -> Result<Option<SeatId>, ApiError> {
        snapshot_budget(context, budget)?;
        let attached = super::super::topology_handoff::validate_bootstrap_resolution(
            tx, canonical, request, guard,
        )?;
        let expected = checked_host_number(guard.ordinary().admission().lifecycle_revision)?
            .checked_add(1)
            .and_then(|n| n.checked_add(i64::from(allocated)))
            .ok_or_else(|| {
                api_error(
                    ErrorCode::SequenceExhausted,
                    "bootstrap resolution lifecycle exhausted",
                )
            })?;
        let lifecycle: i64 = tx
            .query_row(
                "SELECT lifecycle_revision FROM herdr_hosts WHERE host_id=?1",
                [guard.ordinary().admission().host_id.get()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if lifecycle != expected {
            return Err(api_error(
                ErrorCode::StaleHostObservation,
                "bootstrap resolution observation predates a lifecycle change",
            ));
        }
        let owner = current_resolution_owner(tx, &canonical.instance, guard.ordinary())?;
        if attached.is_some() && attached != owner {
            return Err(api_error(
                ErrorCode::TargetUnresolved,
                "bootstrap attached recipient is no longer exact owner",
            ));
        }
        Ok(owner)
    };
    let result = schema::execute_budgeted_idempotent_transaction_with_constraints(
        context,
        conn,
        budget,
        issuance.as_ref(),
        &format!("service-allocation:{}", canonical.instance),
        request.operation.as_str(),
        digest,
        |tx| {
            cooperative_claim(tx, context, &canonical.instance, &request.identity.claim).map(|_| ())
        },
        |tx| {
            if validate(tx, false)?.is_none() {
                validate_resolution_allocation(
                    tx,
                    guard.ordinary().admission().host_id,
                    guard.ordinary().structural_proof(),
                )?;
            }
            Ok(())
        },
        |tx, at| {
            snapshot_budget(context, budget)?;
            if let Some(permit) = permit.as_mut() {
                super::control::decide_accountable(
                    context,
                    tx,
                    at,
                    permit,
                    &input.claim,
                    &input.claim.seat,
                    &input.operation,
                    &input.obligation,
                    &input.payload_hash,
                )?;
            }
            let (result, allocated) = ordinary_resolution_apply(
                tx,
                at,
                &canonical.instance,
                guard.ordinary().admission().host_id,
                &ordinary.target,
                guard.ordinary().structural_proof(),
            )?;
            own_allocation.set(allocated);
            Ok(result)
        },
        |tx, result| {
            let owner = validate(tx, own_allocation.get())?;
            let CommandResult::SeatResolved(seat) = &result else {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "bootstrap resolution result differs",
                ));
            };
            if owner.as_ref() != Some(seat) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "bootstrap resolution result is no longer current owner",
                ));
            }
            Ok(result)
        },
    )?;
    let CommandResult::SeatResolved(seat) = result else {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "bootstrap resolution result differs",
        ));
    };
    Ok(seat)
}

/// Shared unchanged ordinary allocation recipe. The boolean records only this
/// call's successful new seat insert; it is never inferred on replay.
pub(crate) fn ordinary_resolution_apply(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    host: HostId,
    target: &HostTargetId,
    proof: &DurableStructuralProof,
) -> Result<(CommandResult, bool), ApiError> {
    let owner: Option<String> = tx
        .query_row(
            "SELECT id FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
            params![host.get(), target.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(seat) = owner {
        return Ok((CommandResult::SeatResolved(SeatId::new(seat)), false));
    }
    refuse_named_seat_pane(tx, target)?;
    // TRUST-POLICY C1, moved panes: the observed terminal already belongs to a
    // resolved seat at another target in this Herdr boot and incarnation, so
    // the pane moved (a new id after a move to another workspace) and the
    // reconciliation that follows it (`Move`) has not run yet. Allocating here
    // would split the role; the retry after that pass resolves the seat itself.
    let moved: Option<String> = tx
        .query_row(
            &format!("SELECT id FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND state='resolved' AND structural_terminal_id=?2 AND structural_host_boot=?3 AND structural_incarnation=?4 AND target_id IS NOT ?5 LIMIT 1"),
            params![
                host.get(),
                proof.terminal().as_str(),
                proof.host_boot().as_str(),
                proof.incarnation(),
                target.as_str()
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(seat) = moved {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            format!(
                "pane {} moved: seat {seat} still maps its earlier pane until the daemon follows \
                 the move; retry",
                target.as_str()
            ),
        ));
    }
    insert_ordinary_allocation(tx, at, instance, host, proof).map(|result| (result, true))
}

/// N4-paneform (SA9, M10): a pane that a live named seat's open binding hinted
/// (`occupant_bindings.locator_hint`) runs a named agent, so pane-form allocation must not mint a
/// second, ordinary seat for it. The hint is a hint, never authority: it only refuses an
/// allocation, and the refusal names the seat so the caller addresses it by name.
fn refuse_named_seat_pane(tx: &Transaction<'_>, pane: &HostTargetId) -> Result<(), ApiError> {
    let name: Option<String> = tx
        .query_row(
            "SELECT s.name FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
             WHERE b.locator_hint=?1 AND b.ended_at IS NULL AND b.role='top_level' \
             AND s.name IS NOT NULL AND s.state<>'retired' ORDER BY b.ordinal DESC LIMIT 1",
            [pane.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    match name {
        Some(name) => Err(api_error(
            ErrorCode::TargetAlreadyOwned,
            // M10 (57 bytes + fields) after the CLI's `herdr-threads: ` prefix.
            format!(
                "pane {} runs named seat {name}; address it by name",
                pane.as_str()
            ),
        )),
        None => Ok(()),
    }
}

/// The live seat named `name` on the named host `host`, allocated when none exists.
///
/// TRUST-POLICY N (typo-seat accepted limit): a first seat-acting use of a name allocates it, so a
/// misspelled name is a new seat. Reads never reach this function. A live row of any state but
/// `retired` is reused (an unresolved named seat is never split by the next resolve of its name;
/// the unique index `seats_live_name` also covers it). Returns the seat and whether it was created.
pub(crate) fn named_seat_apply(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    host: HostId,
    name: &SeatName,
) -> Result<(SeatId, bool), ApiError> {
    let kind: Option<String> = tx
        .query_row(
            "SELECT kind FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if kind.as_deref() != Some("named") {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "a named seat lives on the named host",
        ));
    }
    let live: Option<(String, String)> = tx
        .query_row(
            &format!("SELECT id,state FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND name=?2 AND state<>'retired'"),
            params![host.get(), name.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((seat, state)) = live {
        let seat = SeatId::new(seat);
        if state == "unresolved" {
            reattach_named_seat(tx, at, instance, host, &seat, name)?;
        }
        return Ok((seat, false));
    }
    let seat = SeatId::new(crate::store::public_ids::fresh(
        tx,
        prefix::SEAT,
        &[("seats", "id", prefix::SEAT)],
    )?);
    tx.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id,name) VALUES (?1,?2,'resolved','native',?3,1,1,?4,1,?2,1,?5,?3)",
        params![seat.as_str(), instance, name.as_str(), at.utc.0, host.get()],
    )
    .map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_host_lifecycle_revision(tx, host)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok((seat, true))
}

/// TRUST-POLICY C1 "named continuity": the resolve of a name whose live seat is `unresolved`
/// reattaches that seat (same id, history and obligations) instead of splitting the role. The
/// decision is recorded in seat history as `cooperative_named_seat`. Retire ends continuity: a
/// retired seat is not live, so its name mints a new seat.
fn reattach_named_seat(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    host: HostId,
    seat: &SeatId,
    name: &SeatName,
) -> Result<(), ApiError> {
    rebind_unresolved_seat(tx, instance, host, seat, name.as_str(), 1, at.utc)?;
    let (boot, epoch): (Option<String>, i64) = tx
        .query_row(
            "SELECT host_boot,host_epoch FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    tx.execute(
        "INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)",
        params![
            host.get(),
            name.as_str(),
            seat.as_str(),
            crate::protocol::authority::COOPERATIVE_CONTINUITY_PROVENANCE,
            at.utc.0,
            boot.unwrap_or_default(),
            epoch,
            crate::protocol::authority::COOPERATIVE_NAMED_SEAT_PROVENANCE,
        ],
    )
    .map_err(store_error)?;
    schema::apply_eligibility_transition(tx, instance, |_| Ok(true))?;
    schema::bump_host_lifecycle_revision(tx, host)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(())
}

/// Longest `locator_hint` recorded (a pane id or `<core>:<tN>` is far shorter).
const LOCATOR_HINT_MAX_BYTES: usize = 128;

/// N4-hint: record the locator a named seat is addressed from on the seat's open top-level
/// binding that carries none yet, and return M9's `(earlier, new)` pair when `hint` is on no
/// earlier row of the seat (ended rows included) while another distinct hint is. The decision
/// reads the persisted rows, so it fires once per (seat, locator) across daemon restarts. A
/// hint is a hint, never authority: nothing but M9 and `seat inspect` reads it. When the seat
/// has no such binding yet (its first check-in has not enrolled one), nothing is recorded and a
/// later command's resolve records it.
fn record_locator_hint(
    tx: &Transaction<'_>,
    seat: &SeatId,
    hint: &str,
) -> Result<Option<(String, String)>, ApiError> {
    if hint.is_empty() || hint.len() > LOCATOR_HINT_MAX_BYTES || hint.chars().any(char::is_control)
    {
        return Ok(None);
    }
    let known: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND locator_hint=?2)",
            params![seat.as_str(), hint],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let unmarked: Option<i64> = tx
        .query_row(
            "SELECT ordinal FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL \
             AND role='top_level' AND locator_hint IS NULL ORDER BY ordinal DESC LIMIT 1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(ordinal) = unmarked else {
        return Ok(None);
    };
    // A known locator is no second locator (no M9), but a session restarted from it still
    // records it on its own binding, so `seat inspect` labels it and the next hint skips it.
    let earlier: Option<String> = tx
        .query_row(
            "SELECT locator_hint FROM occupant_bindings WHERE seat_id=?1 \
             AND locator_hint IS NOT NULL ORDER BY ordinal LIMIT 1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    tx.execute(
        "UPDATE occupant_bindings SET locator_hint=?1 WHERE ordinal=?2",
        params![hint, ordinal],
    )
    .map_err(store_error)?;
    Ok(earlier
        .filter(|_| !known)
        .map(|earlier| (earlier, hint.to_owned())))
}

/// A seat-acting resolve under a name on the named host: [`named_seat_apply`] in one idempotent
/// transaction keyed by the request's operation, so a retried call replays its original result.
pub fn resolve_named_seat(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    host: HostId,
    request: &crate::protocol::commands::ResolveSeat,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    snapshot_budget(context, budget)?;
    let name = SeatName::parse(request.target.as_str())
        .map_err(|detail| api_error(ErrorCode::InvalidRequest, detail))?;
    if request.operation.as_str().is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "resolution operation is required",
        ));
    }
    let digest = schema::canonical_digest(&("resolve_named_seat", instance, &request.target))?;
    schema::execute_idempotent_transaction(
        context,
        conn,
        &format!("service-allocation:{instance}"),
        request.operation.as_str(),
        digest,
        |_| snapshot_budget(context, budget),
        |tx, at| {
            snapshot_budget(context, budget)?;
            let (seat, created) = named_seat_apply(tx, at, instance, host, &name)?;
            let second_locator = match request.locator_hint.as_deref() {
                Some(hint) => record_locator_hint(tx, &seat, hint)?,
                None => None,
            };
            Ok(CommandResult::NamedSeatResolved {
                seat,
                name: name.clone(),
                created,
                second_locator,
            })
        },
    )
}

pub fn check_resolved_target(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    check: ResolvedTargetCheck,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    snapshot_budget(context, budget)?;
    context.execute_decision(
        conn,
        |tx| {
            snapshot_budget(context, budget)?;
            if current_resolution_owner(tx, instance, &check.guard)?.as_ref()
                != Some(&check.expected_seat)
            {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "resolved seat is no longer the target owner",
                ));
            }
            let lifecycle: i64 = tx
                .query_row(
                    "SELECT lifecycle_revision FROM herdr_hosts WHERE host_id=?1",
                    [check.guard.admission().host_id.get()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let published_lifecycle =
                checked_host_number(check.guard.admission().lifecycle_revision)?
                    .checked_add(1)
                    .ok_or_else(|| {
                        api_error(
                            ErrorCode::SequenceExhausted,
                            "resolution lifecycle revision exhausted",
                        )
                    })?;
            if lifecycle != published_lifecycle {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "final observation predates a lifecycle change",
                ));
            }
            Ok(())
        },
        |_, _, ()| snapshot_budget(context, budget),
    )
}

#[cfg(test)]
#[path = "../../../tests/store/named_allocation.rs"]
mod named_allocation_tests;

#[cfg(test)]
#[path = "../../../tests/store/locator_hint.rs"]
mod locator_hint_tests;

#[cfg(test)]
#[path = "../../../tests/store/pane_form_refusal.rs"]
mod pane_form_refusal_tests;
